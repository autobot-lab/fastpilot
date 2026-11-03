//! One web session: a page (inside its own BrowserContext when fastpilot launched Chrome),
//! implementing the core `Driver`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use fastpilot_core::{Direction, Driver, Error as CoreError, Intent, Node, Point, Rect, RefRegistry, Result, Selector, Snapshot, Value};
use serde_json::json;
use tokio::sync::{broadcast::error::RecvError, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::{Browser, Error};

const NAV_TIMEOUT: Duration = Duration::from_secs(30);
const GROUP: &str = "fastpilot";

/// Actionability (Playwright-style): attached, visible, stable for 2 frames, enabled, receives
/// events at its center (shadow-DOM aware). Coordinates are translated to the top-level
/// viewport through same-origin frames. Optionally focuses + selects content for typing.
const ACTION_JS: &str = r#"async function(focus) {
  const el = this.nodeType === 1 ? this : this.parentElement;
  if (!el || !el.isConnected) return { ok: false, reason: 'detached' };
  const win = el.ownerDocument.defaultView;
  if (el.scrollIntoViewIfNeeded) el.scrollIntoViewIfNeeded(true); else el.scrollIntoView({ block: 'center', inline: 'center' });
  const frame = () => new Promise(r => { const t = setTimeout(r, 34); win.requestAnimationFrame(() => { clearTimeout(t); r(); }); });
  await frame();
  const a = el.getBoundingClientRect();
  await frame();
  const b = el.getBoundingClientRect();
  if (a.x !== b.x || a.y !== b.y || a.width !== b.width || a.height !== b.height) return { ok: false, reason: 'not stable (animating)' };
  if (b.width === 0 || b.height === 0 || win.getComputedStyle(el).visibility !== 'visible') return { ok: false, reason: 'not visible' };
  if (el.disabled || el.closest('[aria-disabled="true"],fieldset[disabled]')) return { ok: false, reason: 'disabled' };
  let x = b.x + b.width / 2, y = b.y + b.height / 2;
  const root = el.getRootNode();
  const hit = (root.elementFromPoint ? root : el.ownerDocument).elementFromPoint(x, y);
  const inside = (n, anc) => { for (; n; n = n.parentNode || n.host) if (n === anc) return true; return false; };
  if (!hit || !(inside(hit, el) || inside(el, hit) || hit.control === el)) return { ok: false, reason: 'covered by <' + (hit ? hit.tagName.toLowerCase() + (hit.id ? '#' + hit.id : '') : 'nothing') + '>' };
  if (focus) {
    if (!el.isContentEditable && !/^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)) return { ok: false, reason: 'not editable' };
    if (el.readOnly) return { ok: false, reason: 'read-only' };
    el.focus();
    if (typeof el.select === 'function') el.select();
    else if (el.isContentEditable) { const r = el.ownerDocument.createRange(); r.selectNodeContents(el); const s = win.getSelection(); s.removeAllRanges(); s.addRange(r); }
  }
  try {
    for (let w = win; w.frameElement; w = w.parent) {
      const f = w.frameElement, r = f.getBoundingClientRect(), s = w.parent.getComputedStyle(f);
      x += r.left + parseFloat(s.borderLeftWidth) + parseFloat(s.paddingLeft);
      y += r.top + parseFloat(s.borderTopWidth) + parseFloat(s.paddingTop);
    }
  } catch (e) {}
  return { ok: true, x, y };
}"#;

/// "DOM quiet": 80 ms without mutations and no finite animation running (infinite spinners are
/// ignored). Gives up after 1 s; the caller bounds it further.
const QUIET_JS: &str = r#"new Promise(r => {
  if (document.readyState === 'loading') return r(false);
  const start = performance.now();
  let last = start;
  const o = new MutationObserver(() => { last = performance.now(); });
  o.observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  const busy = () => document.getAnimations().some(a => a.playState === 'running' && a.effect && a.effect.getComputedTiming().endTime !== Infinity);
  const tick = () => {
    const now = performance.now();
    if (now - last >= 80 && !busy()) { o.disconnect(); r(true); }
    else if (now - start > 1000) { o.disconnect(); r(false); }
    else setTimeout(tick, 20);
  };
  setTimeout(tick, 20);
})"#;

const SELECT_JS: &str = r#"function(label) {
  const el = this.nodeType === 1 ? this : this.parentElement;
  const sel = el && (el.tagName === 'SELECT' ? el : el.closest('select'));
  if (!sel) return { ok: false, kind: 'unsupported', reason: 'not a native <select>; tapOn the option instead' };
  if (sel.disabled) return { ok: false, reason: 'disabled' };
  const opts = [...sel.options];
  const opt = opts.find(o => o.label === label || o.text.trim() === label || o.value === label)
    || opts.find(o => o.text.toLowerCase().includes(label.toLowerCase()));
  if (!opt) return { ok: false, kind: 'not_found', reason: 'no option ' + JSON.stringify(label) + ' (options: ' + opts.slice(0, 10).map(o => o.text.trim()).join(', ') + ')' };
  sel.value = opt.value;
  opt.selected = true;
  sel.dispatchEvent(new Event('input', { bubbles: true }));
  sel.dispatchEvent(new Event('change', { bubbles: true }));
  return { ok: true };
}"#;

#[derive(Debug, Clone, Default)]
struct Nav {
    /// Bumped on every main-frame load and same-document navigation.
    count: u64,
    /// loaderId of the last main-frame `load`.
    loader: String,
}

#[derive(Default)]
struct Live {
    /// requestId -> (start time, url), for "requests triggered by the last action".
    inflight: HashMap<String, (Instant, String)>,
    /// Main frame started loading and has not stopped yet.
    loading_since: Option<Instant>,
    /// Dialogs, popups, downloads since the last action; shown as `alert` nodes.
    notes: Vec<String>,
    /// Recent console errors / exceptions / failed requests, newest last.
    diag: VecDeque<String>,
    downloads: HashMap<String, String>,
}

/// The page currently driven: the session's own tab, or a popup it opened.
#[derive(Debug, Clone)]
struct Tab {
    /// `None` on a page-level endpoint (Android WebView), where commands need no session.
    session: Option<String>,
    target: String,
}

/// Out-of-process (cross-site) iframe attached to the current tab.
#[derive(Debug, Clone)]
struct Frame {
    session: String,
    target: String,
    /// The `<iframe>` element in the parent page, learned at snapshot time.
    owner: Option<i64>,
}

struct Shared {
    live: Mutex<Live>,
    tab: Mutex<Tab>,
    /// Tabs to return to when popups close, with their OOPIFs (still attached meanwhile).
    openers: Mutex<Vec<(Tab, HashMap<u32, Frame>)>>,
    frames: Mutex<HashMap<u32, Frame>>,
    next_frame: AtomicU32,
    downloads_dir: Option<PathBuf>,
}

/// Handles of OOPIF nodes carry their frame id above this bit; main-page handles are plain
/// backendNodeIds.
const FRAME_SHIFT: u32 = 40;

/// A remote object and the session it lives in.
struct Obj {
    session: Option<String>,
    id: String,
    /// `<iframe>` owner for OOPIF nodes: their coordinates are frame-local.
    owner: Option<i64>,
}

pub struct WebPage {
    browser: Arc<Browser>,
    /// Set when fastpilot created the BrowserContext and must dispose of it.
    context: Option<String>,
    shared: Arc<Shared>,
    refs: Mutex<RefRegistry>,
    nav: watch::Receiver<Nav>,
    last_action: Mutex<Instant>,
    events: JoinHandle<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn field(v: &Value, key: &'static str) -> std::result::Result<String, Error> {
    v[key].as_str().map(str::to_owned).ok_or(Error::Unexpected(key))
}

fn clip(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    match line.char_indices().nth(200) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line.to_owned(),
    }
}

#[derive(Default)]
struct DomInfo {
    id: Option<String>,
    secure: bool,
    pointer: bool,
    /// opacity:0 / visibility:hidden — drawn as nothing, though still in the AX tree.
    hidden: bool,
    bounds: Option<Rect>,
}

/// Domains every driven tab (own page, popup) needs.
async fn enable(cdp: &crate::cdp::Cdp, session: Option<&str>) -> std::result::Result<(), Error> {
    cdp.call(session, "Page.enable", json!({})).await?;
    cdp.call(session, "Page.setLifecycleEventsEnabled", json!({ "enabled": true })).await?;
    cdp.call(session, "Network.enable", json!({})).await?;
    // Best-effort: absent on some endpoints (WebView) or experimental.
    let _ = cdp.call(session, "Runtime.enable", json!({})).await;
    let _ = cdp.call(session, "Page.setInterceptFileChooserDialog", json!({ "enabled": true })).await;
    let _ = cdp.call(session, "Emulation.setFocusEmulationEnabled", json!({ "enabled": true })).await;
    let _ = cdp
        .call(session, "Target.setAutoAttach", json!({ "autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true }))
        .await;
    Ok(())
}

impl WebPage {
    pub(crate) async fn create(browser: Arc<Browser>) -> std::result::Result<Self, Error> {
        let cdp = browser.cdp.clone();
        let ctx = cdp.call(None, "Target.createBrowserContext", json!({ "disposeOnDetach": true })).await?;
        let context = field(&ctx, "browserContextId")?;
        let attach = async {
            let t = cdp
                .call(None, "Target.createTarget", json!({ "url": "about:blank", "browserContextId": context }))
                .await?;
            let target = field(&t, "targetId")?;
            let a = cdp.call(None, "Target.attachToTarget", json!({ "targetId": target, "flatten": true })).await?;
            Ok::<_, Error>((target, field(&a, "sessionId")?))
        };
        let (target, session) = match attach.await {
            Ok(v) => v,
            Err(e) => {
                cdp.send(None, "Target.disposeBrowserContext", json!({ "browserContextId": context }));
                return Err(e);
            }
        };
        let dir = std::env::temp_dir().join(format!("fastpilot-downloads-{}", std::process::id()));
        let _ = cdp
            .call(
                None,
                "Browser.setDownloadBehavior",
                json!({ "behavior": "allow", "browserContextId": context, "downloadPath": dir.display().to_string(), "eventsEnabled": true }),
            )
            .await;
        let page = Self::init(browser, Some(session), target, Some(context), Some(dir)).await?;
        // Deterministic layout for sessions we own (headless and headful agree).
        let _ = page
            .call(
                "Emulation.setDeviceMetricsOverride",
                json!({ "width": 1280, "height": 800, "deviceScaleFactor": 1, "mobile": false }),
            )
            .await;
        Ok(page)
    }

    pub(crate) async fn init(
        browser: Arc<Browser>,
        session: Option<String>,
        target: String,
        context: Option<String>,
        downloads_dir: Option<PathBuf>,
    ) -> std::result::Result<Self, Error> {
        let shared = Arc::new(Shared {
            live: Mutex::default(),
            tab: Mutex::new(Tab { session: session.clone(), target }),
            openers: Mutex::default(),
            frames: Mutex::default(),
            next_frame: AtomicU32::new(1),
            downloads_dir,
        });
        let (nav_tx, nav) = watch::channel(Nav::default());
        let rx = browser.cdp.subscribe();
        let events = tokio::spawn(event_loop(rx, shared.clone(), browser.cdp.clone(), nav_tx));
        let page = Self {
            browser,
            context,
            shared,
            refs: Mutex::default(),
            nav,
            last_action: Mutex::new(Instant::now()),
            events,
        };
        enable(&page.browser.cdp, session.as_deref()).await?;
        Ok(page)
    }

    fn tab(&self) -> Tab {
        lock(&self.shared.tab).clone()
    }

    async fn call(&self, method: &str, params: Value) -> std::result::Result<Value, Error> {
        self.browser.cdp.call(self.tab().session.as_deref(), method, params).await
    }

    async fn call_in(&self, session: Option<&str>, method: &str, params: Value) -> std::result::Result<Value, Error> {
        self.browser.cdp.call(session, method, params).await
    }

    fn mark_action(&self) {
        *lock(&self.last_action) = Instant::now();
        lock(&self.shared.live).notes.clear();
    }

    async fn wait_nav(&self, what: &str, done: impl Fn(&Nav) -> bool) -> Result<()> {
        let mut nav = self.nav.clone();
        match tokio::time::timeout(NAV_TIMEOUT, nav.wait_for(|n| done(n))).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(_)) => Err(Error::Closed.into()),
            Err(_) => Err(CoreError::Timeout(format!("{what} did not finish loading in {}s", NAV_TIMEOUT.as_secs()))),
        }
    }

    /// Input that must not be cut in half by a cancelled future (mouse down without up):
    /// runs on its own task, so dropping the caller still completes the sequence.
    async fn dispatch(&self, events: Vec<(&'static str, Value)>) -> Result<()> {
        self.mark_action();
        let (cdp, sid) = (self.browser.cdp.clone(), self.tab().session);
        let task = tokio::spawn(async move {
            for (method, params) in events {
                if method == PAUSE {
                    tokio::time::sleep(Duration::from_millis(params["ms"].as_u64().unwrap_or(0))).await;
                    continue;
                }
                cdp.call(sid.as_deref(), method, params).await?;
            }
            Ok::<(), Error>(())
        });
        // Never retried: part of the input may already have reached the page.
        match task.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(CoreError::Driver(e.to_string())),
            Err(e) => Err(CoreError::Driver(e.to_string())),
        }
    }

    async fn click(&self, x: f64, y: f64) -> Result<()> {
        self.dispatch(mouse(x, y, 1, 0)).await
    }

    /// Element point for `sel`, after actionability checks.
    async fn point_of(&self, sel: &Selector) -> Result<(f64, f64)> {
        if let Selector::Point(p) = sel {
            return Ok((p.x, p.y));
        }
        let obj = self.object_for(sel, Intent::Tap).await?;
        self.actionable(&obj, false).await
    }

    async fn resolve(&self, sel: &Selector, intent: Intent) -> Result<i64> {
        let snap = self.snapshot().await?;
        snap.resolve(sel, intent)?
            .node
            .handle
            .ok_or_else(|| CoreError::NotFound(format!("{sel} has no DOM node")))
    }

    async fn object_for(&self, sel: &Selector, intent: Intent) -> Result<Obj> {
        let handle = self.resolve(sel, intent).await?;
        self.object_of(handle).await
    }

    /// (session, backendNodeId, iframe owner) for a snapshot handle.
    fn locate(&self, handle: i64) -> Result<(Option<String>, i64, Option<i64>)> {
        let fid = (handle >> FRAME_SHIFT) as u32;
        let backend = handle & ((1 << FRAME_SHIFT) - 1);
        if fid == 0 {
            return Ok((self.tab().session, backend, None));
        }
        let frames = lock(&self.shared.frames);
        let f = frames.get(&fid).ok_or_else(|| CoreError::NotFound("its iframe was removed".into()))?;
        Ok((Some(f.session.clone()), backend, f.owner))
    }

    async fn object_of(&self, handle: i64) -> Result<Obj> {
        let (session, backend, owner) = self.locate(handle)?;
        let obj = self
            .call_in(session.as_deref(), "DOM.resolveNode", json!({ "backendNodeId": backend, "objectGroup": GROUP }))
            .await
            .map_err(|_| CoreError::NotFound("element was removed from the page".into()))?;
        let id = obj["object"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| CoreError::NotFound("element was removed from the page".into()))?;
        Ok(Obj { session, id, owner })
    }

    async fn call_on(&self, obj: &Obj, function: &str, arg: Value) -> Result<Value> {
        let r = self
            .call_in(
                obj.session.as_deref(),
                "Runtime.callFunctionOn",
                json!({
                    "objectId": obj.id,
                    "functionDeclaration": function,
                    "arguments": [{ "value": arg }],
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
            )
            .await;
        self.browser.cdp.send(obj.session.as_deref(), "Runtime.releaseObjectGroup", json!({ "objectGroup": GROUP }));
        let mut r = r?;
        if let Some(ex) = r.get("exceptionDetails") {
            return Err(CoreError::NotActionable(ex["text"].as_str().unwrap_or("script error").to_owned()));
        }
        Ok(r["result"]["value"].take())
    }

    /// Top-level viewport point to click, after actionability checks.
    async fn actionable(&self, obj: &Obj, focus: bool) -> Result<(f64, f64)> {
        if let Some(owner) = obj.owner {
            let _ = self.call("DOM.scrollIntoViewIfNeeded", json!({ "backendNodeId": owner })).await;
        }
        let v = self.call_on(obj, ACTION_JS, json!(focus)).await?;
        let (x, y) = match (v["ok"].as_bool(), v["x"].as_f64(), v["y"].as_f64()) {
            (Some(true), Some(x), Some(y)) => (x, y),
            _ => {
                return match v["reason"].as_str() {
                    Some("detached") => Err(CoreError::NotFound("element was removed from the page".into())),
                    reason => Err(CoreError::NotActionable(reason.unwrap_or("unknown").to_owned())),
                };
            }
        };
        let Some(owner) = obj.owner else { return Ok((x, y)) };
        // OOPIF: frame-local point + the iframe's content box in the page.
        let model = self.call("DOM.getBoxModel", json!({ "backendNodeId": owner })).await?;
        let quad = &model["model"]["content"];
        Ok((x + quad[0].as_f64().unwrap_or(0.0), y + quad[1].as_f64().unwrap_or(0.0)))
    }

    async fn dom_quiet(&self) -> bool {
        let r = self
            .call("Runtime.evaluate", json!({ "expression": QUIET_JS, "awaitPromise": true, "returnByValue": true }))
            .await;
        r.is_ok_and(|v| v["result"]["value"].as_bool() == Some(true))
    }

    fn busy(&self, since: Instant) -> bool {
        let l = lock(&self.shared.live);
        l.loading_since.is_some_and(|t| t >= since) || l.inflight.values().any(|(t, _)| *t >= since)
    }

    async fn location(&self) -> std::result::Result<(String, String), Error> {
        let tab = self.tab();
        if tab.session.is_some() {
            let info = self.browser.cdp.call(None, "Target.getTargetInfo", json!({ "targetId": tab.target })).await?;
            let i = &info["targetInfo"];
            return Ok((i["title"].as_str().unwrap_or("").to_owned(), i["url"].as_str().unwrap_or("").to_owned()));
        }
        let r = self
            .call("Runtime.evaluate", json!({ "expression": "[document.title, location.href]", "returnByValue": true }))
            .await?;
        let v = &r["result"]["value"];
        Ok((v[0].as_str().unwrap_or("").to_owned(), v[1].as_str().unwrap_or("").to_owned()))
    }

    /// AX roots of OOPIFs (handles namespaced by frame id), keyed by their `<iframe>` element.
    async fn oop_frames(&self) -> HashMap<i64, Vec<Node>> {
        fn encode(nodes: &mut [Node], fid: u32) {
            for n in nodes {
                n.handle = n.handle.map(|h| (i64::from(fid) << FRAME_SHIFT) | h);
                encode(&mut n.children, fid);
            }
        }
        let frames: Vec<(u32, Frame)> = lock(&self.shared.frames).iter().map(|(k, v)| (*k, v.clone())).collect();
        let mut out = HashMap::new();
        for (fid, f) in frames {
            let (owner, ax, dom) = tokio::join!(
                self.call("DOM.getFrameOwner", json!({ "frameId": f.target })),
                self.call_in(Some(&f.session), "Accessibility.getFullAXTree", json!({})),
                self.call_in(Some(&f.session), "DOMSnapshot.captureSnapshot", json!({ "computedStyles": STYLES })),
            );
            let (Ok(owner), Ok(ax)) = (owner, ax) else { continue };
            let Some(owner) = owner["backendNodeId"].as_i64() else { continue };
            if let Some(slot) = lock(&self.shared.frames).get_mut(&fid) {
                slot.owner = Some(owner);
            }
            let dom = dom.map(|d| dom_info(&d, false)).unwrap_or_default();
            let mut roots = ax["nodes"].as_array().map(|n| ax_tree(n, &dom, &HashMap::new())).unwrap_or_default();
            encode(&mut roots, fid);
            out.insert(owner, roots);
        }
        out
    }

    /// AX roots of same-process child frames, keyed by the iframe element's backendNodeId.
    async fn frames(&self, tree: &Value, dom: &HashMap<i64, DomInfo>, mut out: HashMap<i64, Vec<Node>>) -> HashMap<i64, Vec<Node>> {
        fn collect(t: &Value, out: &mut Vec<String>) {
            for c in t["childFrames"].as_array().into_iter().flatten() {
                collect(c, out); // children first, so nested frames are spliced bottom-up
                if let Some(id) = c["frame"]["id"].as_str() {
                    out.push(id.to_owned());
                }
            }
        }
        let mut ids = Vec::new();
        collect(&tree["frameTree"], &mut ids);
        for fid in ids {
            // Out-of-process frames fail here; `oop_frames` covers them.
            let (Ok(owner), Ok(ax)) = (
                self.call("DOM.getFrameOwner", json!({ "frameId": fid })).await,
                self.call("Accessibility.getFullAXTree", json!({ "frameId": fid })).await,
            ) else {
                continue;
            };
            if let (Some(owner), Some(nodes)) = (owner["backendNodeId"].as_i64(), ax["nodes"].as_array()) {
                let roots = ax_tree(nodes, dom, &out);
                out.insert(owner, roots);
            }
        }
        out
    }
}

/// Styles read in the DOMSnapshot, in this order.
const STYLES: [&str; 4] = ["cursor", "-webkit-text-security", "opacity", "visibility"];

async fn event_loop(
    mut rx: tokio::sync::broadcast::Receiver<Arc<crate::cdp::Event>>,
    shared: Arc<Shared>,
    cdp: Arc<crate::cdp::Cdp>,
    nav_tx: watch::Sender<Nav>,
) {
    let note = |s: String| lock(&shared.live).notes.push(s);
    let diag = |s: String| {
        let mut l = lock(&shared.live);
        l.diag.push_back(clip(&s));
        if l.diag.len() > 10 {
            l.diag.pop_front();
        }
    };
    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(RecvError::Lagged(_)) => {
                // Lost events: forget pending work rather than waiting on it forever.
                let mut l = lock(&shared.live);
                l.inflight.clear();
                l.loading_since = None;
                nav_tx.send_modify(|n| n.count += 1);
                continue;
            }
            Err(RecvError::Closed) => break,
        };
        let tab = lock(&shared.tab).clone();
        let p = &ev.params;
        // Browser-level events (only on browser endpoints, where the tab has a session).
        if ev.session.is_none() && tab.session.is_some() {
            match ev.method.as_str() {
                "Target.targetCreated" if p["targetInfo"]["openerId"].as_str() == Some(tab.target.as_str()) && p["targetInfo"]["type"] == "page" => {
                    let Some(target) = p["targetInfo"]["targetId"].as_str().map(str::to_owned) else { continue };
                    let Ok(a) = cdp.call(None, "Target.attachToTarget", json!({ "targetId": target, "flatten": true })).await else { continue };
                    let Some(session) = a["sessionId"].as_str().map(str::to_owned) else { continue };
                    if enable(&cdp, Some(&session)).await.is_ok() {
                        let frames = std::mem::take(&mut *lock(&shared.frames));
                        lock(&shared.openers).push((tab.clone(), frames));
                        *lock(&shared.tab) = Tab { session: Some(session), target };
                        let url = p["targetInfo"]["url"].as_str().unwrap_or("");
                        note(if url.is_empty() { "popup opened".into() } else { format!("popup opened: {url}") });
                        nav_tx.send_modify(|n| n.count += 1);
                    }
                }
                "Target.targetDestroyed" if p["targetId"].as_str() == Some(tab.target.as_str()) => {
                    let back = lock(&shared.openers).pop();
                    if let Some((opener, frames)) = back {
                        *lock(&shared.tab) = opener;
                        *lock(&shared.frames) = frames;
                        note("popup closed".into());
                        nav_tx.send_modify(|n| n.count += 1);
                    }
                }
                "Browser.downloadWillBegin" if p["frameId"].as_str() == Some(tab.target.as_str()) => {
                    let name = p["suggestedFilename"].as_str().unwrap_or("").to_owned();
                    if let Some(guid) = p["guid"].as_str() {
                        lock(&shared.live).downloads.insert(guid.to_owned(), name.clone());
                    }
                    note(format!("download started: {name}"));
                }
                "Browser.downloadProgress" if p["state"] == "completed" || p["state"] == "canceled" => {
                    let name = p["guid"].as_str().and_then(|g| lock(&shared.live).downloads.remove(g));
                    if let Some(name) = name {
                        let dir = shared.downloads_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default();
                        let bytes = p["receivedBytes"].as_u64().unwrap_or(0);
                        note(format!("download {}: {name} ({bytes} bytes) in {dir}", p["state"].as_str().unwrap_or("")));
                    }
                }
                _ => {}
            }
            continue;
        }
        if ev.session != tab.session {
            continue;
        }
        let main_frame = p["frameId"].as_str() == Some(tab.target.as_str());
        match ev.method.as_str() {
            "Network.requestWillBeSent" => {
                // Long-lived channels never "finish"; they must not block idleness.
                let long_lived = matches!(p["type"].as_str(), Some("EventSource" | "WebSocket" | "Ping"));
                if let (false, Some(id)) = (long_lived, p["requestId"].as_str()) {
                    let url = p["request"]["url"].as_str().unwrap_or("").to_owned();
                    lock(&shared.live).inflight.insert(id.to_owned(), (Instant::now(), url));
                }
            }
            "Network.loadingFinished" => {
                if let Some(id) = p["requestId"].as_str() {
                    lock(&shared.live).inflight.remove(id);
                }
            }
            "Network.loadingFailed" => {
                let url = p["requestId"].as_str().and_then(|id| lock(&shared.live).inflight.remove(id)).map(|(_, u)| u);
                let err = p["errorText"].as_str().unwrap_or("");
                if p["canceled"].as_bool() != Some(true) && err != "net::ERR_ABORTED" {
                    diag(format!("request failed: {err} {}", url.unwrap_or_default()));
                }
            }
            "Network.responseReceived" => {
                let status = p["response"]["status"].as_u64().unwrap_or(0);
                let url = p["response"]["url"].as_str().unwrap_or("");
                if status >= 400 && !url.ends_with("/favicon.ico") {
                    diag(format!("HTTP {status} {url}"));
                }
            }
            "Runtime.exceptionThrown" => {
                let d = &p["exceptionDetails"];
                diag(format!("exception: {}", d["exception"]["description"].as_str().or(d["text"].as_str()).unwrap_or("")));
            }
            "Runtime.consoleAPICalled" if matches!(p["type"].as_str(), Some("error" | "assert")) => {
                let a = &p["args"][0];
                let text = a["value"].as_str().map(str::to_owned).or_else(|| a["description"].as_str().map(str::to_owned)).unwrap_or_default();
                diag(format!("console.error: {text}"));
            }
            "Page.frameStartedLoading" if main_frame => lock(&shared.live).loading_since = Some(Instant::now()),
            "Page.frameStoppedLoading" if main_frame => lock(&shared.live).loading_since = None,
            "Page.lifecycleEvent" if main_frame && p["name"] == "load" => {
                let loader = p["loaderId"].as_str().unwrap_or("").to_owned();
                nav_tx.send_modify(|n| {
                    n.count += 1;
                    n.loader = loader;
                });
            }
            "Page.navigatedWithinDocument" if main_frame => nav_tx.send_modify(|n| n.count += 1),
            "Page.javascriptDialogOpening" => {
                let kind = p["type"].as_str().unwrap_or("dialog");
                note(format!("{kind}: {}", p["message"].as_str().unwrap_or("")));
                // Policy: accept, so a flow never hangs on alert/confirm/beforeunload.
                let prompt = p["defaultPrompt"].as_str().unwrap_or("");
                cdp.send(tab.session.as_deref(), "Page.handleJavaScriptDialog", json!({ "accept": true, "promptText": prompt }));
            }
            "Page.fileChooserOpened" => note("file chooser opened (use uploadFile)".into()),
            "Target.attachedToTarget" if p["targetInfo"]["type"] == "iframe" => {
                if let (Some(session), Some(target)) = (p["sessionId"].as_str(), p["targetInfo"]["targetId"].as_str()) {
                    let id = shared.next_frame.fetch_add(1, Ordering::Relaxed);
                    lock(&shared.frames).insert(id, Frame { session: session.to_owned(), target: target.to_owned(), owner: None });
                }
            }
            "Target.detachedFromTarget" => {
                if let Some(session) = p["sessionId"].as_str() {
                    lock(&shared.frames).retain(|_, f| f.session != session);
                }
            }
            _ => {}
        }
    }
}

impl WebPage {
    /// Title, URL and unpruned nodes (handles = backendNodeIds); used by `snapshot` and by
    /// hosts that embed this page in a larger tree (Android WebView).
    pub async fn raw_nodes(&self) -> Result<(String, String, Vec<Node>)> {
        let (ax, loc, dom, tree) = tokio::join!(
            self.call("Accessibility.getFullAXTree", json!({})),
            self.location(),
            self.call("DOMSnapshot.captureSnapshot", json!({ "computedStyles": STYLES })),
            self.call("Page.getFrameTree", json!({})),
        );
        let (ax, (title, url), dom, tree) = (ax?, loc?, dom?, tree?);
        let dom = dom_info(&dom, true);
        let oop = self.oop_frames().await;
        let frames = self.frames(&tree, &dom, oop).await;
        let mut roots = ax["nodes"].as_array().map(|n| ax_tree(n, &dom, &frames)).unwrap_or_default();
        roots.extend(lock(&self.shared.live).notes.iter().map(|note| Node {
            role: "alert".into(),
            name: note.clone(),
            ..Node::default()
        }));
        Ok((title, url, roots))
    }

    /// Tap a node by backendNodeId with full actionability checks.
    pub async fn tap_handle(&self, handle: i64) -> Result<()> {
        let obj = self.object_of(handle).await?;
        let (x, y) = self.actionable(&obj, false).await?;
        self.click(x, y).await
    }

    /// Focus a node by backendNodeId, replace its content with `text`.
    pub async fn type_handle(&self, handle: i64, text: &str) -> Result<()> {
        let obj = self.object_of(handle).await?;
        self.actionable(&obj, true).await?;
        self.insert(text).await
    }

    async fn insert(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            let key = |kind: &str| {
                ("Input.dispatchKeyEvent", json!({ "type": kind, "key": "Backspace", "code": "Backspace", "windowsVirtualKeyCode": 8 }))
            };
            self.dispatch(vec![key("keyDown"), key("keyUp")]).await
        } else {
            self.dispatch(vec![("Input.insertText", json!({ "text": text }))]).await
        }
    }
}

impl Drop for WebPage {
    fn drop(&mut self) {
        self.events.abort();
        let tab = self.tab();
        match (&self.context, &tab.session) {
            (Some(ctx), _) => {
                self.browser.cdp.send(None, "Target.disposeBrowserContext", json!({ "browserContextId": ctx }));
            }
            // Attached to someone else's tab: just let go of it.
            (None, Some(sid)) => self.browser.cdp.send(None, "Target.detachFromTarget", json!({ "sessionId": sid })),
            (None, None) => {}
        }
    }
}

/// Known schemes pass through; existing paths become `file://`; local hosts get `http://`.
fn to_url(target: &str) -> String {
    let lower = target.to_ascii_lowercase();
    if ["http://", "https://", "file://", "about:", "data:"].iter().any(|s| lower.starts_with(s)) {
        return target.to_owned();
    }
    if let Ok(p) = Path::new(target).canonicalize() {
        return format!("file://{}", p.display());
    }
    let host = lower.split(['/', ':']).next().unwrap_or("");
    let has_port = lower.split('/').next().is_some_and(|h| h.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()));
    if host == "localhost" || host.ends_with(".localhost") || host.starts_with("127.") || has_port {
        format!("http://{target}")
    } else {
        format!("https://{target}")
    }
}

/// Element ids, secure inputs, `cursor:pointer`, invisibility and boxes per backendNodeId, from
/// one DOMSnapshot (covers shadow roots and same-process frames, which a `querySelectorAll`
/// would miss). Boxes are kept for the first document only when `main_bounds` (frame
/// documents use their own coordinates).
fn dom_info(snap: &Value, main_bounds: bool) -> HashMap<i64, DomInfo> {
    let strings: Vec<&str> = snap["strings"].as_array().into_iter().flatten().map(|s| s.as_str().unwrap_or("")).collect();
    let s = |i: &Value| i.as_u64().and_then(|i| strings.get(i as usize).copied()).unwrap_or("");
    let mut out: HashMap<i64, DomInfo> = HashMap::new();
    for (di, doc) in snap["documents"].as_array().into_iter().flatten().enumerate() {
        let nodes = &doc["nodes"];
        let Some(backend) = nodes["backendNodeId"].as_array() else { continue };
        let names = nodes["nodeName"].as_array();
        let attrs = nodes["attributes"].as_array();
        for (i, b) in backend.iter().enumerate() {
            let Some(b) = b.as_i64() else { continue };
            let (mut id, mut ty, mut ac) = (None, "", "");
            for pair in attrs.and_then(|a| a.get(i)).and_then(Value::as_array).into_iter().flatten().collect::<Vec<_>>().chunks(2) {
                if let [k, v] = pair {
                    match s(k) {
                        "id" => id = Some(s(v).to_owned()),
                        "type" => ty = s(v),
                        "autocomplete" => ac = s(v),
                        _ => {}
                    }
                }
            }
            let input = names.and_then(|n| n.get(i)).map(&s) == Some("INPUT");
            let secure = input
                && (ty.eq_ignore_ascii_case("password")
                    || ac.split_whitespace().any(|t| {
                        matches!(t, "current-password" | "new-password" | "one-time-code" | "cc-number" | "cc-csc")
                    }));
            if id.is_some() || secure {
                out.insert(b, DomInfo { id, secure, ..DomInfo::default() });
            }
        }
        let layout = &doc["layout"];
        let (Some(index), Some(styles)) = (layout["nodeIndex"].as_array(), layout["styles"].as_array()) else { continue };
        let rects = layout["bounds"].as_array();
        for (li, ni) in index.iter().enumerate() {
            let Some(b) = ni.as_u64().and_then(|n| backend.get(n as usize)).and_then(Value::as_i64) else { continue };
            let style = styles.get(li).and_then(Value::as_array);
            let at = |k: usize| style.and_then(|st| st.get(k)).map(&s).unwrap_or("");
            let (pointer, masked) = (at(0) == "pointer", !matches!(at(1), "" | "none"));
            let hidden = at(2) == "0" || matches!(at(3), "hidden" | "collapse");
            if !(pointer || masked || hidden || di == 0 || !main_bounds) {
                continue;
            }
            let e = out.entry(b).or_default();
            e.pointer |= pointer;
            e.secure |= masked;
            e.hidden |= hidden;
            if di > 0 && main_bounds {
                continue;
            }
            if let Some([x, y, w, h]) = rects.and_then(|r| r.get(li)).and_then(Value::as_array).map(|r| {
                [0, 1, 2, 3].map(|i| r.get(i).and_then(Value::as_f64).unwrap_or(0.0))
            }) {
                e.bounds = Some(Rect { x, y, w, h });
            }
        }
    }
    out
}

/// CDP AX tree -> core nodes, with roles mapped onto the canonical vocabulary.
fn ax_tree(nodes: &[Value], dom: &HashMap<i64, DomInfo>, frames: &HashMap<i64, Vec<Node>>) -> Vec<Node> {
    let by_id: HashMap<&str, &Value> = nodes.iter().filter_map(|n| Some((n["nodeId"].as_str()?, n))).collect();
    let ctx = AxCtx { by_id, dom, frames };
    nodes.first().and_then(|root| ax_node(root, &ctx, 0, false)).into_iter().collect()
}

struct AxCtx<'a> {
    by_id: HashMap<&'a str, &'a Value>,
    dom: &'a HashMap<i64, DomInfo>,
    frames: &'a HashMap<i64, Vec<Node>>,
}

const HIDDEN: &[&str] = &["ariaHiddenElement", "ariaHiddenSubtree", "notRendered", "notVisible", "inertElement", "inertSubtree"];

fn text_of(nodes: &[Node], out: &mut Vec<String>) {
    for n in nodes {
        if n.role == "text" && !n.name.is_empty() {
            out.push(n.name.clone());
        }
        text_of(&n.children, out);
    }
}

fn ax_node(n: &Value, ctx: &AxCtx, depth: usize, pointer_above: bool) -> Option<Node> {
    let raw_role = n["role"]["value"].as_str().unwrap_or("");
    if depth > 256 || matches!(raw_role, "InlineTextBox" | "LineBreak") {
        return None;
    }
    let handle = n["backendDOMNodeId"].as_i64();
    let info = handle.and_then(|h| ctx.dom.get(&h));
    // opacity:0 / visibility:hidden subtrees are not visible, whatever the AX tree says.
    if info.is_some_and(|i| i.hidden) {
        return None;
    }
    let pointer = info.is_some_and(|i| i.pointer);
    let mut children: Vec<Node> = n["childIds"]
        .as_array()
        .map(|ids| {
            ids.iter()
                .filter_map(|id| ctx.by_id.get(id.as_str()?))
                .filter_map(|c| ax_node(c, ctx, depth + 1, pointer_above || pointer))
                .collect()
        })
        .unwrap_or_default();
    if let Some(roots) = handle.and_then(|h| ctx.frames.get(&h)) {
        children.extend(roots.iter().cloned());
    }
    let ignored = n["ignored"].as_bool() == Some(true);
    let hidden = n["ignoredReasons"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|r| r["name"].as_str().is_some_and(|name| HIDDEN.contains(&name)));
    let ax_str = |v: &Value| match &v["value"] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let mut name = ax_str(&n["name"]);
    let mut role = match raw_role {
        _ if hidden => "none".to_owned(),
        "textbox" | "searchbox" => "textfield".to_owned(),
        "StaticText" => "text".to_owned(),
        "img" => "image".to_owned(),
        r => r.to_ascii_lowercase(),
    };
    // Role-less elements that look clickable (cursor:pointer), outermost only.
    if !hidden && pointer && !pointer_above && matches!(role.as_str(), "generic" | "none" | "group" | "listitem" | "paragraph" | "cell") {
        role = "clickable".to_owned();
        if name.is_empty() {
            let mut parts = Vec::new();
            text_of(&children, &mut parts);
            name = parts.join(" ");
        }
    } else if ignored && role != "none" && !hidden {
        role = "none".to_owned();
    }
    let value = ax_str(&n["value"]);
    let mut node = Node {
        value: (role == "textfield" || role == "combobox" || !value.is_empty()).then_some(value),
        role,
        name,
        id: info.and_then(|i| i.id.clone()),
        secure: info.is_some_and(|i| i.secure),
        bounds: info.and_then(|i| i.bounds),
        handle,
        children,
        ..Node::default()
    };
    for p in n["properties"].as_array().into_iter().flatten() {
        let v = &p["value"]["value"];
        let on = v.as_bool() == Some(true) || v.as_str() == Some("true");
        match p["name"].as_str() {
            Some("disabled") => node.disabled = on,
            Some("checked") => node.checked = on,
            Some("selected") => node.selected = on,
            Some("expanded") => node.expanded = on,
            Some("pressed") => node.pressed = on,
            Some("invalid") => node.invalid = v.as_str().is_some_and(|s| s != "false"),
            Some("required") => node.required = on,
            Some("readonly") => node.readonly = on && node.role == "textfield",
            Some("focused") => node.focused = on,
            _ => {}
        }
    }
    Some(node)
}

/// (key, code, windowsVirtualKeyCode, text) for `pressKey`.
/// Pseudo-method in a `dispatch` list: sleep `ms` inside the same uncancellable task.
const PAUSE: &str = "fastpilot.pause";

/// Mouse move, then `clicks` press/release pairs (clickCount 1, 2, ...), with `hold_ms`
/// between each press and its release.
fn mouse(x: f64, y: f64, clicks: u8, hold_ms: u64) -> Vec<(&'static str, Value)> {
    let ev = |kind: &str, button: &str, buttons: u8, count: u8| {
        ("Input.dispatchMouseEvent", json!({ "type": kind, "x": x, "y": y, "button": button, "buttons": buttons, "clickCount": count }))
    };
    let mut out = vec![ev("mouseMoved", "none", 0, 0)];
    for n in 1..=clicks {
        out.push(ev("mousePressed", "left", 1, n));
        if hold_ms > 0 {
            out.push((PAUSE, json!({ "ms": hold_ms })));
        }
        out.push(ev("mouseReleased", "left", 0, n));
    }
    out
}

fn key_info(key: &str) -> Option<(&'static str, &'static str, u32, &'static str)> {
    Some(match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => ("Enter", "Enter", 13, "\r"),
        "tab" => ("Tab", "Tab", 9, ""),
        "escape" | "esc" => ("Escape", "Escape", 27, ""),
        "backspace" => ("Backspace", "Backspace", 8, ""),
        "delete" => ("Delete", "Delete", 46, ""),
        "space" => (" ", "Space", 32, " "),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38, ""),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40, ""),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37, ""),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39, ""),
        "home" => ("Home", "Home", 36, ""),
        "end" => ("End", "End", 35, ""),
        "pageup" => ("PageUp", "PageUp", 33, ""),
        "pagedown" => ("PageDown", "PageDown", 34, ""),
        _ => return None,
    })
}

#[async_trait]
impl Driver for WebPage {
    async fn open(&self, target: &str) -> Result<()> {
        let url = to_url(target);
        let before = self.nav.borrow().count;
        self.mark_action();
        let r = self.call("Page.navigate", json!({ "url": url })).await?;
        if let Some(err) = r["errorText"].as_str().filter(|e| !e.is_empty()) {
            return Err(CoreError::Driver(format!("navigation to {url} failed: {err}")));
        }
        match r["loaderId"].as_str() {
            // New document: wait for *its* load, not a late one from the previous page.
            Some(loader) => self.wait_nav(&url, |n| n.loader == loader).await,
            None => self.wait_nav(&url, |n| n.count > before).await,
        }
    }

    async fn snapshot(&self) -> Result<Snapshot> {
        let (title, url, roots) = self.raw_nodes().await?;
        let mut refs = lock(&self.refs);
        Ok(Snapshot::build(title, url, roots, &mut refs))
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
        self.insert(text).await
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
        self.dispatch(vec![(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy }),
        )])
        .await
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
        let before = self.nav.borrow().count;
        self.mark_action();
        self.call("Page.navigateToHistoryEntry", json!({ "entryId": prev })).await?;
        self.wait_nav("back navigation", |n| n.count > before).await
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
            let left = deadline.saturating_duration_since(Instant::now());
            // Re-check after the quiet window: a click-initiated navigation may only start then.
            if !self.busy(since)
                && tokio::time::timeout(left, self.dom_quiet()).await == Ok(true)
                && !self.busy(since)
            {
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
            .call(
                "Runtime.evaluate",
                json!({ "expression": js, "returnByValue": true, "awaitPromise": true, "replMode": true }),
            )
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            let msg = ex["exception"]["description"].as_str().or(ex["text"].as_str()).unwrap_or("script error");
            return Err(CoreError::Script(msg.to_owned()));
        }
        Ok(r["result"]["value"].take())
    }

    async fn press_key(&self, key: &str) -> Result<()> {
        let (k, code, vk, text) = key_info(key).ok_or_else(|| {
            CoreError::Unsupported(format!("unknown key `{key}` (Enter, Tab, Escape, Backspace, Delete, Space, Arrow*, Home, End, PageUp, PageDown)"))
        })?;
        let down = if text.is_empty() {
            json!({ "type": "rawKeyDown", "key": k, "code": code, "windowsVirtualKeyCode": vk })
        } else {
            json!({ "type": "keyDown", "key": k, "code": code, "windowsVirtualKeyCode": vk, "text": text })
        };
        let up = json!({ "type": "keyUp", "key": k, "code": code, "windowsVirtualKeyCode": vk });
        self.dispatch(vec![("Input.dispatchKeyEvent", down), ("Input.dispatchKeyEvent", up)]).await
    }

    async fn select_option(&self, sel: &Selector, option: &str) -> Result<()> {
        let obj = self.object_for(sel, Intent::Tap).await?;
        self.mark_action();
        let v = self.call_on(&obj, SELECT_JS, json!(option)).await?;
        if v["ok"].as_bool() == Some(true) {
            return Ok(());
        }
        let reason = v["reason"].as_str().unwrap_or("unknown").to_owned();
        Err(match v["kind"].as_str() {
            Some("unsupported") => CoreError::Unsupported(reason),
            Some("not_found") => CoreError::NotFound(reason),
            _ => CoreError::NotActionable(reason),
        })
    }

    async fn upload(&self, sel: &Selector, files: &[String]) -> Result<()> {
        let paths = files
            .iter()
            .map(|f| {
                Path::new(f)
                    .canonicalize()
                    .map(|p| p.display().to_string())
                    .map_err(|e| CoreError::Flow(format!("file {f}: {e}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let handle = self.resolve(sel, Intent::Tap).await?;
        let (session, backend, _) = self.locate(handle)?;
        self.mark_action();
        self.call_in(session.as_deref(), "DOM.setFileInputFiles", json!({ "files": paths, "backendNodeId": backend })).await?;
        Ok(())
    }

    async fn diagnostics(&self) -> Vec<String> {
        lock(&self.shared.live).diag.iter().cloned().collect()
    }

    async fn long_press(&self, sel: &Selector) -> Result<()> {
        let (x, y) = self.point_of(sel).await?;
        self.dispatch(mouse(x, y, 1, 800)).await
    }

    async fn double_tap(&self, sel: &Selector) -> Result<()> {
        let (x, y) = self.point_of(sel).await?;
        self.dispatch(mouse(x, y, 2, 0)).await
    }

    async fn hide_keyboard(&self) -> Result<()> {
        self.eval_script("document.activeElement && document.activeElement.blur(); true").await.map(|_| ())
    }

    /// Real Backspace key events (frameworks see input events); all = select, then one.
    async fn erase_text(&self, count: Option<usize>) -> Result<()> {
        let n = match count {
            Some(n) => n.min(1000),
            None => {
                let selected = self
                    .eval_script("(() => { const e = document.activeElement; if (!e || e === document.body) return false; if (e.select) e.select(); else document.execCommand('selectAll'); return true; })()")
                    .await?;
                if selected != Value::Bool(true) {
                    return Err(CoreError::NotActionable("no focused field to erase".into()));
                }
                1
            }
        };
        let key = |kind: &str| ("Input.dispatchKeyEvent", json!({ "type": kind, "key": "Backspace", "code": "Backspace", "windowsVirtualKeyCode": 8 }));
        self.dispatch((0..n).flat_map(|_| [key("rawKeyDown"), key("keyUp")]).collect()).await
    }

    /// Cookies of this session's browser context plus every storage of the current origin.
    async fn clear_state(&self, _app: Option<&str>) -> Result<()> {
        self.call("Network.clearBrowserCookies", json!({})).await?;
        let origin = self.eval_script("location.origin").await?;
        if let Some(o) = origin.as_str().filter(|o| o.starts_with("http")) {
            self.call("Storage.clearDataForOrigin", json!({ "origin": o, "storageTypes": "all" })).await?;
        } else {
            self.eval_script("try { localStorage.clear(); sessionStorage.clear(); } catch (e) {} true").await?;
        }
        Ok(())
    }

    async fn viewport(&self) -> Option<Rect> {
        let m = self.call("Page.getLayoutMetrics", json!({})).await.ok()?;
        let v = &m["cssVisualViewport"];
        Some(Rect {
            x: v["pageX"].as_f64()?,
            y: v["pageY"].as_f64()?,
            w: v["clientWidth"].as_f64()?,
            h: v["clientHeight"].as_f64()?,
        })
    }

    async fn screenshot_annotated(&self, marks: &[(String, Rect)]) -> Result<Vec<u8>> {
        let marks: Vec<Value> = marks.iter().map(|(r, b)| json!({ "r": r, "x": b.x, "y": b.y, "w": b.w, "h": b.h })).collect();
        let draw = format!(
            "(() => {{ const root = document.createElement('div'); root.id = '__fastpilot_som';
              root.style = 'position:absolute;left:0;top:0;pointer-events:none;z-index:2147483647';
              for (const m of {}) {{ const b = document.createElement('div');
                b.style = `position:absolute;left:${{m.x}}px;top:${{m.y}}px;width:${{m.w}}px;height:${{m.h}}px;outline:2px solid #e00`;
                const l = document.createElement('span'); l.textContent = m.r;
                l.style = 'position:absolute;left:0;top:-14px;background:#e00;color:#fff;font:11px monospace;padding:0 2px';
                b.appendChild(l); root.appendChild(b); }}
              document.documentElement.appendChild(root); }})()",
            Value::Array(marks)
        );
        self.call("Runtime.evaluate", json!({ "expression": draw })).await?;
        let shot = self.screenshot().await;
        let _ = self.call("Runtime.evaluate", json!({ "expression": "document.getElementById('__fastpilot_som')?.remove()" })).await;
        shot
    }
}
