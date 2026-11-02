//! One web session: a page inside its own BrowserContext, implementing the core `Driver`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use fastpilot_core::{Direction, Driver, Error as CoreError, Intent, Node, Point, RefRegistry, Result, Selector, Snapshot, Value};
use serde_json::json;
use tokio::sync::{broadcast::error::RecvError, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::{Browser, Error};

const NAV_TIMEOUT: Duration = Duration::from_secs(30);
const GROUP: &str = "fastpilot";

/// Actionability (Playwright-style): attached, visible, stable for 2 frames, enabled, receives
/// events at its center. Optionally focuses + selects the content for typing.
const ACTION_JS: &str = r#"async function(focus) {
  const el = this.nodeType === 1 ? this : this.parentElement;
  if (!el || !el.isConnected) return { ok: false, reason: 'detached' };
  if (el.scrollIntoViewIfNeeded) el.scrollIntoViewIfNeeded(true); else el.scrollIntoView({ block: 'center', inline: 'center' });
  const frame = () => new Promise(r => { const t = setTimeout(r, 34); requestAnimationFrame(() => { clearTimeout(t); r(); }); });
  await frame();
  const a = el.getBoundingClientRect();
  await frame();
  const b = el.getBoundingClientRect();
  if (a.x !== b.x || a.y !== b.y || a.width !== b.width || a.height !== b.height) return { ok: false, reason: 'not stable (animating)' };
  if (b.width === 0 || b.height === 0 || getComputedStyle(el).visibility !== 'visible') return { ok: false, reason: 'not visible' };
  if (el.disabled || el.closest('[aria-disabled="true"],fieldset[disabled]')) return { ok: false, reason: 'disabled' };
  const x = b.x + b.width / 2, y = b.y + b.height / 2;
  const hit = document.elementFromPoint(x, y);
  const ok = hit && (hit === el || el.contains(hit) || hit.contains(el) || hit.control === el);
  if (!ok) return { ok: false, reason: 'covered by <' + (hit ? hit.tagName.toLowerCase() + (hit.id ? '#' + hit.id : '') : 'nothing') + '>' };
  if (focus) {
    if (!el.isContentEditable && !/^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)) return { ok: false, reason: 'not editable' };
    if (el.readOnly) return { ok: false, reason: 'read-only' };
    el.focus();
    if (typeof el.select === 'function') el.select();
    else if (el.isContentEditable) { const r = document.createRange(); r.selectNodeContents(el); const s = getSelection(); s.removeAllRanges(); s.addRange(r); }
  }
  return { ok: true, x, y };
}"#;

/// "DOM quiet": no mutation during two animation frames (with a timer fallback for hidden pages).
const QUIET_JS: &str = r#"new Promise(r => {
  if (document.readyState === 'loading') return r(false);
  let changed = false;
  const o = new MutationObserver(() => { changed = true; });
  o.observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  const done = () => { o.disconnect(); r(!changed); };
  const t = setTimeout(done, 50);
  requestAnimationFrame(() => requestAnimationFrame(() => { clearTimeout(t); done(); }));
})"#;

pub struct WebPage {
    browser: Arc<Browser>,
    session: String,
    target: String,
    context: String,
    refs: Mutex<RefRegistry>,
    /// requestId -> start time, for "requests triggered by the last action".
    inflight: Arc<Mutex<HashMap<String, Instant>>>,
    /// Bumped on every load / same-document navigation.
    nav: watch::Receiver<u64>,
    last_action: Mutex<Instant>,
    events: JoinHandle<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn field(v: &Value, key: &'static str) -> std::result::Result<String, Error> {
    v[key].as_str().map(str::to_owned).ok_or(Error::Unexpected(key))
}

impl WebPage {
    pub(crate) async fn create(browser: Arc<Browser>) -> std::result::Result<Self, Error> {
        let cdp = browser.cdp.clone();
        let ctx = cdp.call(None, "Target.createBrowserContext", json!({ "disposeOnDetach": true })).await?;
        let context = field(&ctx, "browserContextId")?;
        let t = cdp
            .call(None, "Target.createTarget", json!({ "url": "about:blank", "browserContextId": context }))
            .await?;
        let target = field(&t, "targetId")?;
        let a = cdp.call(None, "Target.attachToTarget", json!({ "targetId": target, "flatten": true })).await?;
        let session = field(&a, "sessionId")?;

        let inflight: Arc<Mutex<HashMap<String, Instant>>> = Arc::default();
        let (nav_tx, nav) = watch::channel(0u64);
        let mut rx = cdp.subscribe();
        let (sid, net) = (session.clone(), inflight.clone());
        let events = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) if ev.session.as_deref() == Some(sid.as_str()) => {
                        let id = ev.params["requestId"].as_str();
                        match (ev.method.as_str(), id) {
                            ("Network.requestWillBeSent", Some(id)) => {
                                lock(&net).insert(id.to_owned(), Instant::now());
                            }
                            ("Network.loadingFinished" | "Network.loadingFailed", Some(id)) => {
                                lock(&net).remove(id);
                            }
                            ("Page.loadEventFired" | "Page.navigatedWithinDocument", _) => {
                                nav_tx.send_modify(|n| *n += 1);
                            }
                            _ => {}
                        }
                    }
                    Ok(_) => {}
                    Err(RecvError::Lagged(_)) => lock(&net).clear(),
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let page = Self {
            browser,
            session,
            target,
            context,
            refs: Mutex::default(),
            inflight,
            nav,
            last_action: Mutex::new(Instant::now()),
            events,
        };
        page.call("Page.enable", json!({})).await?;
        page.call("Network.enable", json!({})).await?;
        // Keeps rAF/focus behaving in background windows; best-effort (experimental domain).
        let _ = page.call("Emulation.setFocusEmulationEnabled", json!({ "enabled": true })).await;
        Ok(page)
    }

    async fn call(&self, method: &str, params: Value) -> std::result::Result<Value, Error> {
        self.browser.cdp.call(Some(&self.session), method, params).await
    }

    fn mark_action(&self) {
        *lock(&self.last_action) = Instant::now();
    }

    async fn wait_nav(&self, before: u64, what: &str) -> Result<()> {
        let mut nav = self.nav.clone();
        match tokio::time::timeout(NAV_TIMEOUT, nav.wait_for(|n| *n > before)).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(_)) => Err(Error::Closed.into()),
            Err(_) => Err(CoreError::Timeout(format!("{what} did not finish loading in {}s", NAV_TIMEOUT.as_secs()))),
        }
    }

    /// Remote object for a selector. DOM ids go straight to the DOM; everything else is
    /// resolved against a fresh snapshot (refs, text, role+name, focus).
    async fn object_for(&self, sel: &Selector, intent: Intent) -> Result<String> {
        let obj = match sel {
            Selector::Id(id) => {
                let expr = format!("document.getElementById({})", json!(id));
                self.call("Runtime.evaluate", json!({ "expression": expr, "objectGroup": GROUP })).await?
            }
            _ => {
                let snap = self.snapshot().await?;
                let handle = snap.resolve(sel, intent)?.node.handle;
                let handle = handle.ok_or_else(|| CoreError::NotFound(format!("{sel} has no DOM node")))?;
                self.call("DOM.resolveNode", json!({ "backendNodeId": handle, "objectGroup": GROUP }))
                    .await
                    .map_err(|_| CoreError::NotFound(format!("{sel} was removed from the page")))?
            }
        };
        let id = match sel {
            Selector::Id(_) => obj["result"]["objectId"].as_str(),
            _ => obj["object"]["objectId"].as_str(),
        };
        id.map(str::to_owned).ok_or_else(|| CoreError::NotFound(format!("no element matches {sel}")))
    }

    async fn actionable(&self, object: &str, focus: bool) -> Result<(f64, f64)> {
        let r = self
            .call(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object,
                    "functionDeclaration": ACTION_JS,
                    "arguments": [{ "value": focus }],
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
            )
            .await;
        self.browser.cdp.send(Some(&self.session), "Runtime.releaseObjectGroup", json!({ "objectGroup": GROUP }));
        let r = r?;
        if let Some(ex) = r.get("exceptionDetails") {
            return Err(CoreError::NotActionable(ex["text"].as_str().unwrap_or("script error").to_owned()));
        }
        let v = &r["result"]["value"];
        match (v["ok"].as_bool(), v["x"].as_f64(), v["y"].as_f64()) {
            (Some(true), Some(x), Some(y)) => Ok((x, y)),
            _ => match v["reason"].as_str() {
                Some("detached") => Err(CoreError::NotFound("element was removed from the page".into())),
                reason => Err(CoreError::NotActionable(reason.unwrap_or("unknown").to_owned())),
            },
        }
    }

    async fn click(&self, x: f64, y: f64) -> Result<()> {
        self.mark_action();
        for (kind, button, buttons) in [("mouseMoved", "none", 0), ("mousePressed", "left", 1), ("mouseReleased", "left", 0)] {
            self.call(
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": x, "y": y, "button": button, "buttons": buttons, "clickCount": 1 }),
            )
            .await?;
        }
        Ok(())
    }

    /// backendNodeIds of password inputs: the AX tree does not mark them, and `secure` must
    /// hold even while the field is empty.
    async fn password_nodes(&self) -> std::result::Result<Vec<i64>, Error> {
        let doc = self.call("DOM.getDocument", json!({ "depth": 0 })).await?;
        let root = doc["root"]["nodeId"].as_i64().ok_or(Error::Unexpected("root"))?;
        let found = self
            .call("DOM.querySelectorAll", json!({ "nodeId": root, "selector": "input[type=password]" }))
            .await?;
        let mut out = Vec::new();
        for id in found["nodeIds"].as_array().into_iter().flatten() {
            let d = self.call("DOM.describeNode", json!({ "nodeId": id })).await?;
            out.extend(d["node"]["backendNodeId"].as_i64());
        }
        Ok(out)
    }

    async fn dom_quiet(&self) -> bool {
        let r = self
            .call("Runtime.evaluate", json!({ "expression": QUIET_JS, "awaitPromise": true, "returnByValue": true }))
            .await;
        r.is_ok_and(|v| v["result"]["value"].as_bool() == Some(true))
    }
}

impl Drop for WebPage {
    fn drop(&mut self) {
        self.events.abort();
        self.browser
            .cdp
            .send(None, "Target.disposeBrowserContext", json!({ "browserContextId": self.context }));
    }
}

/// Relative or absolute paths that exist become `file://` URLs.
fn to_url(target: &str) -> String {
    if target.contains(':') {
        return target.to_owned();
    }
    match Path::new(target).canonicalize() {
        Ok(p) => format!("file://{}", p.display()),
        Err(_) => format!("https://{target}"),
    }
}

/// CDP AX tree -> core nodes, with roles mapped onto the canonical vocabulary.
fn ax_tree(nodes: &[Value], secure: &[i64]) -> Vec<Node> {
    let by_id: HashMap<&str, &Value> = nodes.iter().filter_map(|n| Some((n["nodeId"].as_str()?, n))).collect();
    nodes.first().and_then(|root| ax_node(root, &by_id, secure, 0)).into_iter().collect()
}

fn ax_node(n: &Value, by_id: &HashMap<&str, &Value>, secure: &[i64], depth: usize) -> Option<Node> {
    let raw_role = n["role"]["value"].as_str().unwrap_or("");
    if depth > 256 || matches!(raw_role, "InlineTextBox" | "LineBreak") {
        return None;
    }
    let children = n["childIds"]
        .as_array()
        .map(|ids| {
            ids.iter()
                .filter_map(|id| by_id.get(id.as_str()?))
                .filter_map(|c| ax_node(c, by_id, secure, depth + 1))
                .collect()
        })
        .unwrap_or_default();
    let role = if n["ignored"].as_bool() == Some(true) {
        "none".to_owned()
    } else {
        match raw_role {
            "textbox" | "searchbox" => "textfield".to_owned(),
            "StaticText" => "text".to_owned(),
            "img" => "image".to_owned(),
            r => r.to_ascii_lowercase(),
        }
    };
    let ax_str = |v: &Value| match &v["value"] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let mut node = Node {
        name: ax_str(&n["name"]),
        handle: n["backendDOMNodeId"].as_i64(),
        secure: n["backendDOMNodeId"].as_i64().is_some_and(|h| secure.contains(&h)),
        children,
        ..Node::default()
    };
    let value = ax_str(&n["value"]);
    node.value = (role == "textfield" || role == "combobox" || !value.is_empty()).then_some(value);
    node.role = role;
    for p in n["properties"].as_array().into_iter().flatten() {
        let v = &p["value"]["value"];
        let on = v.as_bool() == Some(true) || v.as_str() == Some("true");
        match p["name"].as_str() {
            Some("disabled") => node.disabled = on,
            Some("checked") => node.checked = on,
            Some("selected") => node.selected = on,
            Some("expanded") => node.expanded = on,
            Some("focused") => node.focused = on,
            _ => {}
        }
    }
    Some(node)
}

#[async_trait]
impl Driver for WebPage {
    async fn open(&self, target: &str) -> Result<()> {
        let url = to_url(target);
        let before = *self.nav.borrow();
        self.mark_action();
        let r = self.call("Page.navigate", json!({ "url": url })).await?;
        if let Some(err) = r["errorText"].as_str().filter(|e| !e.is_empty()) {
            return Err(CoreError::Driver(format!("navigation to {url} failed: {err}")));
        }
        self.wait_nav(before, &url).await
    }

    async fn snapshot(&self) -> Result<Snapshot> {
        let (ax, info, secure) = tokio::join!(
            self.call("Accessibility.getFullAXTree", json!({})),
            self.browser.cdp.call(None, "Target.getTargetInfo", json!({ "targetId": self.target })),
            self.password_nodes(),
        );
        let (ax, info, secure) = (ax?, info?, secure?);
        let roots = ax["nodes"].as_array().map(|n| ax_tree(n, &secure)).unwrap_or_default();
        let info = &info["targetInfo"];
        let mut refs = lock(&self.refs);
        Ok(Snapshot::build(
            info["title"].as_str().unwrap_or(""),
            info["url"].as_str().unwrap_or(""),
            roots,
            &mut refs,
        ))
    }

    async fn tap(&self, sel: &Selector) -> Result<()> {
        if let Selector::Point(p) = sel {
            return self.click(p.x, p.y).await;
        }
        let obj = self.object_for(sel, Intent::Tap).await?;
        let (x, y) = self.actionable(&obj, false).await?;
        self.click(x, y).await
    }

    async fn type_text(&self, sel: &Selector, text: &str) -> Result<()> {
        if let Selector::Point(p) = sel {
            self.click(p.x, p.y).await?;
        } else {
            let obj = self.object_for(sel, Intent::Type).await?;
            self.actionable(&obj, true).await?;
        }
        self.mark_action();
        if text.is_empty() {
            for kind in ["keyDown", "keyUp"] {
                self.call(
                    "Input.dispatchKeyEvent",
                    json!({ "type": kind, "key": "Backspace", "code": "Backspace", "windowsVirtualKeyCode": 8 }),
                )
                .await?;
            }
        } else {
            self.call("Input.insertText", json!({ "text": text })).await?;
        }
        Ok(())
    }

    async fn scroll(&self, dir: Direction, sel: Option<&Selector>) -> Result<()> {
        let (x, y) = match sel {
            Some(s) => {
                let obj = self.object_for(s, Intent::Find).await?;
                self.actionable(&obj, false).await?
            }
            None => {
                let m = self.call("Page.getLayoutMetrics", json!({})).await?;
                let vp = &m["cssLayoutViewport"];
                (
                    vp["clientWidth"].as_f64().unwrap_or(800.0) / 2.0,
                    vp["clientHeight"].as_f64().unwrap_or(600.0) / 2.0,
                )
            }
        };
        let (dx, dy) = match dir {
            Direction::Down => (0, 400),
            Direction::Up => (0, -400),
            Direction::Right => (400, 0),
            Direction::Left => (-400, 0),
        };
        self.mark_action();
        self.call(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy }),
        )
        .await?;
        Ok(())
    }

    async fn swipe(&self, _from: Point, _to: Point) -> Result<()> {
        Err(CoreError::Unsupported("swipe is mobile-only; use scroll on web".into()))
    }

    async fn back(&self) -> Result<()> {
        let h = self.call("Page.getNavigationHistory", json!({})).await?;
        let idx = h["currentIndex"].as_u64().unwrap_or(0) as usize;
        let prev = idx
            .checked_sub(1)
            .and_then(|i| h["entries"].get(i))
            .and_then(|e| e["id"].as_i64())
            .ok_or_else(|| CoreError::NotActionable("no previous page in history".into()))?;
        let before = *self.nav.borrow();
        self.mark_action();
        self.call("Page.navigateToHistoryEntry", json!({ "entryId": prev })).await?;
        self.wait_nav(before, "back navigation").await
    }

    async fn screenshot(&self) -> Result<Vec<u8>> {
        let r = self.call("Page.captureScreenshot", json!({ "format": "jpeg", "quality": 80 })).await?;
        let data = r["data"].as_str().ok_or(Error::Unexpected("data"))?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| CoreError::Driver(format!("screenshot decode: {e}")))
    }

    async fn wait_idle(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let since = *lock(&self.last_action);
        loop {
            let busy = lock(&self.inflight).values().any(|t| *t >= since);
            if !busy && self.dom_quiet().await {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(CoreError::Timeout(format!("page not idle after {}ms", timeout.as_millis())));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn eval_script(&self, js: &str) -> Result<Value> {
        let mut r = self
            .call("Runtime.evaluate", json!({ "expression": js, "returnByValue": true, "awaitPromise": true }))
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            let msg = ex["exception"]["description"].as_str().or(ex["text"].as_str()).unwrap_or("script error");
            return Err(CoreError::Script(msg.to_owned()));
        }
        Ok(r["result"]["value"].take())
    }
}
