//! iOS driver (PRD §7.5), simulator first. Instead of Appium/WebDriverAgent it runs fastpilot's
//! own minimal XCUITest runner (`ios-runner/`, built from source with Xcode) and speaks the same
//! framed-JSON protocol as the Android agent. The simulator shares the host network, so no
//! port forwarding is needed.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use fastpilot_android::{Agent, Policy};
use fastpilot_core::{
    Direction, Driver, Error as CoreError, Intent, Node, Platform, Point, Rect, RefRegistry, Result, Selector, Snapshot, Value,
};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("simctl: {0}")]
    Simctl(String),
    #[error("runner: {0}")]
    Runner(String),
    #[error(transparent)]
    Agent(#[from] fastpilot_android::Error),
}

impl From<Error> for CoreError {
    fn from(e: Error) -> Self {
        match &e {
            Error::Agent(fastpilot_android::Error::Agent(m)) if m.contains("no focused text field") => {
                Self::NotActionable(m.clone())
            }
            _ => Self::Driver(e.to_string()),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

async fn simctl(args: &[&str]) -> std::result::Result<String, Error> {
    let out = Command::new("xcrun")
        .arg("simctl")
        .args(args)
        .output()
        .await
        .map_err(|e| Error::Simctl(format!("xcrun simctl: {e}")))?;
    if !out.status.success() {
        return Err(Error::Simctl(format!("{}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `(udid, name, state)` of available iPhone/iPad simulators.
pub async fn devices() -> std::result::Result<Vec<(String, String, String)>, Error> {
    let v: Value = serde_json::from_str(&simctl(&["list", "devices", "available", "-j"]).await?)
        .map_err(|e| Error::Simctl(e.to_string()))?;
    let mut out = Vec::new();
    for list in v["devices"].as_object().into_iter().flat_map(|m| m.values()) {
        for d in list.as_array().into_iter().flatten() {
            let get = |k: &str| d[k].as_str().unwrap_or("").to_owned();
            out.push((get("udid"), get("name"), get("state")));
        }
    }
    Ok(out)
}

/// Boot a simulator by name or UDID and wait until it is ready; returns its UDID.
pub async fn boot(name_or_udid: &str) -> std::result::Result<String, Error> {
    let all = devices().await?;
    let (udid, _, state) = all
        .iter()
        .find(|(u, n, _)| u == name_or_udid || n == name_or_udid)
        .ok_or_else(|| Error::Simctl(format!("no simulator `{name_or_udid}`")))?;
    if state != "Booted" {
        simctl(&["boot", udid]).await?;
    }
    simctl(&["bootstatus", udid, "-b"]).await?;
    Ok(udid.clone())
}

fn xctestrun() -> Option<PathBuf> {
    let dir = std::env::var_os("FASTPILOT_IOS_RUNNER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ios-runner/build")));
    let products = dir.join("Build/Products");
    std::fs::read_dir(products)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x == "xctestrun"))
}

/// A SpringBoard alert over the app (permission prompts and the like), PRD §11.2 rule 6.
#[derive(Debug, Default)]
struct Alert {
    text: String,
    buttons: Vec<(String, Point)>,
    boxes: Vec<Rect>,
}

fn frame(v: &Value) -> Option<Rect> {
    let b = v["frame"].as_array()?;
    let n = |i: usize| b.get(i).and_then(Value::as_f64).unwrap_or(0.0);
    Some(Rect { x: n(0), y: n(1), w: n(2), h: n(3) })
}

fn walk_alert(v: &Value, a: &mut Alert) {
    if let Some(f) = frame(v) {
        a.boxes.push(f);
        match s(v, "type") {
            "button" => a.buttons.push((s(v, "label").to_owned(), f.center())),
            "staticText" if !s(v, "label").is_empty() => {
                if !a.text.is_empty() {
                    a.text.push_str(" / ");
                }
                a.text.push_str(s(v, "label"));
            }
            _ => {}
        }
    }
    for c in v["children"].as_array().into_iter().flatten() {
        walk_alert(c, a);
    }
}

fn system_alert(snapshot: &Value) -> Option<Alert> {
    let root = snapshot["nodes"].as_array()?.iter().find(|n| n["system"].as_bool() == Some(true))?;
    let mut a = Alert::default();
    walk_alert(root, &mut a);
    Some(a)
}

impl Alert {
    /// Labels are localized; English ones first, then the usual position (deny is the
    /// "Don't …" / "Not Now" button).
    fn dismiss(&self, policy: Policy) -> Option<Point> {
        let find = |pred: &dyn Fn(&str) -> bool| self.buttons.iter().find(|(l, _)| pred(l)).map(|(_, p)| *p);
        match policy {
            Policy::Keep => None,
            Policy::Deny => find(&|l| l.starts_with("Don") || l.starts_with("Not Now") || l.starts_with("Ask App Not")),
            Policy::Allow => ["Allow While Using App", "Allow", "OK", "Allow Once"].iter().find_map(|w| find(&|l| l == *w)),
        }
    }

    fn owns(&self, b: &Rect) -> bool {
        self.boxes.iter().any(|x| x == b)
    }
}

fn keyboard(v: &Value) -> Option<Rect> {
    if s(v, "type") == "keyboard" {
        return frame(v);
    }
    v.get("children")
        .or_else(|| v.get("nodes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(keyboard)
}

pub struct IosDriver {
    udid: String,
    agent: std::sync::Arc<Agent>,
    size: (f64, f64),
    /// App under test (open/launchApp); a crash shows up as it no longer running.
    bundle: Mutex<String>,
    refs: Mutex<RefRegistry>,
    policy: Policy,
    notes: Mutex<Vec<String>>,
    /// `simctl io recordVideo` while recording, and its output file.
    recording: tokio::sync::Mutex<Option<(Child, PathBuf)>>,
    _runner: Child,
}

impl IosDriver {
    /// Start the XCUITest runner on `udid` (or the booted simulator, booting "iPhone 16" if none).
    pub async fn start(udid: Option<&str>) -> std::result::Result<Self, Error> {
        let udid = match udid {
            Some(u) => boot(u).await?,
            None => match devices().await?.into_iter().find(|(_, _, st)| st == "Booted") {
                Some((u, _, _)) => u,
                None => boot(&std::env::var("FASTPILOT_IOS_DEVICE").unwrap_or_else(|_| "iPhone 16".into())).await?,
            },
        };
        let xctestrun = xctestrun().ok_or(Error::Runner("runner not built: run ios-runner/build.sh".into()))?;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .map_err(|e| Error::Runner(e.to_string()))?;
        let mut child = Command::new("xcodebuild")
            .arg("test-without-building")
            .arg("-xctestrun")
            .arg(&xctestrun)
            .args(["-destination", &format!("id={udid}")])
            .env("TEST_RUNNER_FASTPILOT_PORT", port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::Runner(format!("xcodebuild: {e}")))?;
        let stdout = child.stdout.take().ok_or(Error::Runner("no stdout".into()))?;
        let mut lines = BufReader::new(stdout).lines();
        let mut tail = Vec::new();
        let ready = tokio::time::timeout(Duration::from_secs(180), async {
            while let Ok(Some(l)) = lines.next_line().await {
                if l.contains("FASTPILOT_RUNNER_READY") {
                    return true;
                }
                tail.push(l);
                if tail.len() > 20 {
                    tail.remove(0);
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        if !ready {
            return Err(Error::Runner(format!("runner did not start: {}", tail.join(" | "))));
        }
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let agent = std::sync::Arc::new(Agent::connect(port).await?);
        let hello = agent.call("hello", json!({})).await?;
        let size = (hello["width"].as_f64().unwrap_or(390.0), hello["height"].as_f64().unwrap_or(844.0));
        Ok(Self {
            udid,
            agent,
            size,
            bundle: Mutex::new(String::new()),
            refs: Mutex::default(),
            policy: Policy::from_env(),
            notes: Mutex::default(),
            recording: tokio::sync::Mutex::new(None),
            _runner: child,
        })
    }

    fn app_or(&self, app: Option<&str>) -> Result<String> {
        match app {
            Some(a) => Ok(a.to_owned()),
            None => Some(lock(&self.bundle).clone())
                .filter(|b| !b.is_empty())
                .ok_or_else(|| CoreError::Flow("no app under test yet; pass an appId or launchApp first".into())),
        }
    }

    /// Stopped on purpose: not running is no longer a crash.
    fn forget(&self, bundle: &str) {
        let mut b = lock(&self.bundle);
        if *b == bundle {
            b.clear();
        }
    }

    async fn point_of(&self, sel: &Selector) -> Result<Point> {
        match sel {
            Selector::Point(p) => Ok(*p),
            _ => Self::center(&self.resolve(sel, Intent::Tap).await?),
        }
    }

    fn note(&self, text: String) {
        tracing::info!("{text}");
        let mut n = lock(&self.notes);
        n.push(text);
        let extra = n.len().saturating_sub(20);
        n.drain(..extra);
    }

    /// Raw runner snapshot plus the core snapshot; `app_crashed` when the app under test died.
    async fn screen(&self) -> Result<(Value, Snapshot)> {
        let v = self.call("snapshot", json!({})).await?;
        let bundle = s(&v, "bundleId").to_owned();
        // XCUIApplication.State.notRunning
        if v["state"].as_i64() == Some(1) && *lock(&self.bundle) == bundle && !bundle.is_empty() {
            lock(&self.bundle).clear();
            return Err(CoreError::AppCrashed(format!("{bundle} is no longer running (crashed)")));
        }
        let roots: Vec<Node> = v["nodes"].as_array().into_iter().flatten().filter_map(convert).collect();
        let title = if s(&v, "title").is_empty() { bundle.clone() } else { s(&v, "title").to_owned() };
        let snap = Snapshot::build(title, format!("ios, {bundle}"), roots, &mut lock(&self.refs));
        Ok((v, snap))
    }

    /// Like the Android driver: clear an unrelated permission alert or a covering keyboard first.
    async fn target(&self, sel: &Selector, intent: Intent) -> Result<Node> {
        for _ in 0..3 {
            let (raw, snap) = self.screen().await?;
            let node = snap.resolve(sel, intent).map(|e| e.node.clone());
            if let Some(a) = system_alert(&raw) {
                let ours = node.as_ref().is_ok_and(|n| n.bounds.is_some_and(|b| a.owns(&b)));
                if let (false, Some(p)) = (ours, a.dismiss(self.policy)) {
                    self.note(format!("system alert {:?} dismissed ({:?}) before {sel}", a.text, self.policy));
                    self.tap_at(p).await?;
                    continue;
                }
            }
            let kb = keyboard(&raw);
            let covered = match (&node, kb) {
                (Err(CoreError::NotFound(_)), Some(_)) => true,
                (Ok(n), Some(k)) => n.bounds.is_some_and(|b| k.contains(b.center())),
                _ => false,
            };
            if covered {
                self.note(format!("keyboard covered {sel}; hidden"));
                self.call("hideKeyboard", json!({})).await?;
                continue;
            }
            return node;
        }
        Ok(self.snapshot().await?.resolve(sel, intent)?.node.clone())
    }

    async fn call(&self, cmd: &str, args: Value) -> Result<Value> {
        Ok(self.agent.call(cmd, args).await.map_err(Error::from)?)
    }

    async fn resolve(&self, sel: &Selector, intent: Intent) -> Result<Node> {
        self.target(sel, intent).await
    }

    async fn tap_at(&self, p: Point) -> Result<()> {
        self.call("tap", json!({ "x": p.x, "y": p.y })).await.map(|_| ())
    }

    fn center(n: &Node) -> Result<Point> {
        if n.disabled {
            return Err(CoreError::NotActionable("disabled".into()));
        }
        match n.bounds {
            Some(b) if b.w > 0.0 && b.h > 0.0 => Ok(b.center()),
            _ => Err(CoreError::NotActionable("not visible".into())),
        }
    }
}

impl Drop for IosDriver {
    fn drop(&mut self) {
        let agent = self.agent.clone();
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = agent.call("quit", json!({})).await;
            });
        }
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn text_of(nodes: &[Node], out: &mut Vec<String>) {
    for n in nodes {
        if !n.name.is_empty() && n.role == "text" {
            out.push(n.name.clone());
        }
        text_of(&n.children, out);
    }
}

/// Runner JSON node -> core node with canonical roles. `None` drops the subtree (keyboard).
fn convert(v: &Value) -> Option<Node> {
    let ty = s(v, "type");
    if ty == "keyboard" {
        return None;
    }
    let flag = |k: &str| v[k].as_bool() == Some(true);
    let (label, id, value, placeholder) = (s(v, "label"), s(v, "id"), s(v, "value"), s(v, "placeholder"));
    let children: Vec<Node> = v["children"].as_array().into_iter().flatten().filter_map(convert).collect();
    let role = match ty {
        "button" => "button",
        "staticText" => "text",
        "textField" | "secureTextField" | "searchField" | "textView" => "textfield",
        "switch" | "toggle" => "switch",
        "link" => "link",
        "image" | "icon" => "image",
        "cell" => "clickable",
        "checkBox" => "checkbox",
        "slider" => "slider",
        "picker" | "pickerWheel" => "combobox",
        "tab" => "tab",
        "alert" | "sheet" => "dialog",
        _ => "none",
    };
    let first = |opts: &[&str]| opts.iter().find(|o| !o.is_empty()).map(|o| (*o).to_owned()).unwrap_or_default();
    let mut name = match role {
        "textfield" => first(&[placeholder, label, id]),
        "button" | "switch" | "link" | "checkbox" => first(&[label, id]),
        _ => label.to_owned(),
    };
    if role == "clickable" && name.is_empty() {
        let mut parts = Vec::new();
        text_of(&children, &mut parts);
        name = parts.join(" ");
    }
    let secure = ty == "secureTextField";
    let f = v["frame"].as_array().map(|b| [0, 1, 2, 3].map(|i| b.get(i).and_then(Value::as_f64).unwrap_or(0.0)));
    Some(Node {
        value: (role == "textfield" && !secure).then(|| if value == placeholder { String::new() } else { value.to_owned() }),
        checked: role == "switch" && value == "1",
        role: role.to_owned(),
        name,
        id: (!id.is_empty()).then(|| id.to_owned()),
        secure,
        disabled: !flag("enabled"),
        selected: flag("selected"),
        focused: flag("focused"),
        bounds: f.map(|[x, y, w, h]| Rect { x, y, w, h }),
        children,
        ..Node::default()
    })
}

#[async_trait]
impl Driver for IosDriver {
    /// Bundle id (activates the app) or URL (opened with `simctl openurl`).
    async fn open(&self, target: &str) -> Result<()> {
        if target.contains("://") {
            simctl(&["openurl", &self.udid, target]).await?;
            return Ok(());
        }
        self.call("activate", json!({ "bundleId": target })).await?;
        *lock(&self.bundle) = target.to_owned();
        Ok(())
    }

    async fn snapshot(&self) -> Result<Snapshot> {
        Ok(self.screen().await?.1)
    }

    async fn diagnostics(&self) -> Vec<String> {
        lock(&self.notes).clone()
    }

    /// Set-of-Mark drawn on the device (ios): boxes and refs in screen coordinates.
    async fn screenshot_annotated(&self, marks: &[(String, Rect)]) -> Result<Vec<u8>> {
        let list: Vec<Value> = marks.iter().map(|(r, b)| json!({ "r": r, "x": b.x, "y": b.y, "w": b.w, "h": b.h })).collect();
        let v = self.call("screenshot", json!({ "quality": 80, "marks": list })).await?;
        if v["marked"].as_u64() != Some(marks.len() as u64) {
            return Err(CoreError::Driver(format!("agent drew {} of {} marks (rebuild the agent)", v["marked"], marks.len())));
        }
        base64::engine::general_purpose::STANDARD
            .decode(s(&v, "jpeg"))
            .map_err(|e| CoreError::Driver(format!("screenshot decode: {e}")))
    }

    fn platform(&self) -> Platform {
        Platform::Ios
    }

    async fn start_recording(&self) -> Result<()> {
        let mut slot = self.recording.lock().await;
        if slot.is_some() {
            return Err(CoreError::Flow("already recording".into()));
        }
        let path = std::env::temp_dir().join(format!("fastpilot-{}-{}.mp4", self.udid, std::process::id()));
        let child = Command::new("xcrun")
            .args(["simctl", "io", &self.udid, "recordVideo", "--codec", "h264", "--force"])
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| CoreError::Driver(format!("simctl recordVideo: {e}")))?;
        // recordVideo needs a moment before the first frame is captured.
        tokio::time::sleep(Duration::from_millis(500)).await;
        *slot = Some((child, path));
        Ok(())
    }

    async fn stop_recording(&self) -> Result<fastpilot_core::Recording> {
        let (mut child, path) = self.recording.lock().await.take().ok_or_else(|| CoreError::Flow("not recording".into()))?;
        // SIGINT makes simctl finalize the file.
        if let Some(pid) = child.id() {
            let _ = Command::new("kill").args(["-INT", &pid.to_string()]).status().await;
        }
        let _ = tokio::time::timeout(Duration::from_secs(15), child.wait()).await;
        let data = std::fs::read(&path).map_err(|e| CoreError::Driver(format!("recording {}: {e}", path.display())))?;
        let _ = std::fs::remove_file(&path);
        Ok(fastpilot_core::Recording::Video { mime: "video/mp4", data })
    }

    async fn long_press(&self, sel: &Selector) -> Result<()> {
        let p = self.point_of(sel).await?;
        self.call("longPress", json!({ "x": p.x, "y": p.y, "ms": 800 })).await.map(|_| ())
    }

    async fn double_tap(&self, sel: &Selector) -> Result<()> {
        let p = self.point_of(sel).await?;
        self.call("doubleTap", json!({ "x": p.x, "y": p.y })).await.map(|_| ())
    }

    async fn hide_keyboard(&self) -> Result<()> {
        self.call("hideKeyboard", json!({})).await.map(|_| ())
    }

    async fn erase_text(&self, count: Option<usize>) -> Result<()> {
        let args = match count {
            Some(n) => json!({ "count": n }),
            None => json!({}),
        };
        self.call("erase", args).await.map(|_| ())
    }

    async fn stop_app(&self, app: Option<&str>) -> Result<()> {
        let bundle = self.app_or(app)?;
        self.call("terminate", json!({ "bundleId": bundle })).await?;
        self.forget(&bundle);
        Ok(())
    }

    /// The simulator keeps app data in a host directory: terminate, then empty it (keeps
    /// the top-level folders, which the app expects to exist).
    async fn clear_state(&self, app: Option<&str>) -> Result<()> {
        let bundle = self.app_or(app)?;
        self.stop_app(Some(&bundle)).await?;
        let dir = simctl(&["get_app_container", &self.udid, &bundle, "data"]).await?;
        let dir = PathBuf::from(dir.trim());
        let wipe = |d: &std::path::Path| -> std::io::Result<()> {
            for top in std::fs::read_dir(d)?.flatten() {
                if !top.path().is_dir() {
                    continue;
                }
                for e in std::fs::read_dir(top.path())?.flatten() {
                    let p = e.path();
                    if p.is_dir() { std::fs::remove_dir_all(&p)? } else { std::fs::remove_file(&p)? }
                }
            }
            Ok(())
        };
        wipe(&dir).map_err(|e| CoreError::Driver(format!("clear {}: {e}", dir.display())))
    }

    async fn clear_keychain(&self) -> Result<()> {
        simctl(&["keychain", &self.udid, "reset"]).await?;
        Ok(())
    }

    async fn tap(&self, sel: &Selector) -> Result<()> {
        if let Selector::Point(p) = sel {
            return self.tap_at(*p).await;
        }
        let node = self.resolve(sel, Intent::Tap).await?;
        self.tap_at(Self::center(&node)?).await
    }

    async fn type_text(&self, sel: &Selector, text: &str) -> Result<()> {
        match sel {
            Selector::Focused => {}
            Selector::Point(p) => self.tap_at(*p).await?,
            _ => {
                let node = self.resolve(sel, Intent::Type).await?;
                if !node.focused {
                    self.tap_at(Self::center(&node)?).await?;
                }
            }
        }
        self.call("text", json!({ "text": text })).await.map(|_| ())
    }

    async fn scroll(&self, dir: Direction, sel: Option<&Selector>) -> Result<()> {
        let area = match sel {
            Some(s) => self.resolve(s, Intent::Find).await?.bounds.unwrap_or_default(),
            None => Rect { x: 0.0, y: 0.0, w: self.size.0, h: self.size.1 },
        };
        let c = area.center();
        let (dx, dy) = (area.w * 0.3, area.h * 0.3);
        let (from, to) = match dir {
            Direction::Down => (Point { x: c.x, y: c.y + dy }, Point { x: c.x, y: c.y - dy }),
            Direction::Up => (Point { x: c.x, y: c.y - dy }, Point { x: c.x, y: c.y + dy }),
            Direction::Right => (Point { x: c.x + dx, y: c.y }, Point { x: c.x - dx, y: c.y }),
            Direction::Left => (Point { x: c.x - dx, y: c.y }, Point { x: c.x + dx, y: c.y }),
        };
        self.swipe(from, to).await
    }

    async fn swipe(&self, from: Point, to: Point) -> Result<()> {
        self.call("swipe", json!({ "x1": from.x, "y1": from.y, "x2": to.x, "y2": to.y, "ms": 300 })).await.map(|_| ())
    }

    async fn back(&self) -> Result<()> {
        match self.call("back", json!({})).await {
            Err(CoreError::Driver(m)) if m.contains("no back button") => Err(CoreError::NotActionable(m)),
            other => other.map(|_| ()),
        }
    }

    async fn screenshot(&self) -> Result<Vec<u8>> {
        let v = self.call("screenshot", json!({ "quality": 80 })).await?;
        base64::engine::general_purpose::STANDARD
            .decode(s(&v, "jpeg"))
            .map_err(|e| CoreError::Driver(format!("screenshot decode: {e}")))
    }

    async fn wait_idle(&self, _timeout: Duration) -> Result<()> {
        self.call("idle", json!({})).await.map(|_| ())
    }

    async fn eval_script(&self, _js: &str) -> Result<Value> {
        Err(CoreError::Unsupported("evalScript on iOS (WKWebView) is not supported yet".into()))
    }

    async fn press_key(&self, key: &str) -> Result<()> {
        let name = match key.to_ascii_lowercase().as_str() {
            "enter" | "return" => "return",
            "backspace" | "delete" => "delete",
            "tab" => "tab",
            "escape" | "esc" => "escape",
            "space" => "space",
            other => return Err(CoreError::Unsupported(format!("unknown key `{other}` on iOS"))),
        };
        self.call("key", json!({ "name": name })).await.map(|_| ())
    }

    /// Maestro semantics: terminate and relaunch.
    async fn launch_app(&self, app: &str) -> Result<()> {
        self.call("launch", json!({ "bundleId": app })).await?;
        *lock(&self.bundle) = app.to_owned();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_runner_nodes() {
        let v = json!({ "type": "application", "label": "Demo", "frame": [0, 0, 393, 852], "enabled": true, "children": [
            { "type": "navigationBar", "id": "Login", "frame": [0, 54, 393, 44], "enabled": true,
              "children": [{ "type": "staticText", "label": "Login", "frame": [170, 60, 50, 30], "enabled": true }] },
            { "type": "textField", "placeholder": "Email", "value": "Email", "id": "email", "frame": [20, 200, 353, 34], "enabled": true },
            { "type": "secureTextField", "placeholder": "Password", "id": "password", "frame": [20, 250, 353, 34], "enabled": true },
            { "type": "switch", "label": "Ingat saya", "value": "0", "frame": [20, 300, 51, 31], "enabled": true },
            { "type": "button", "label": "Masuk", "frame": [20, 350, 353, 44], "enabled": true },
            { "type": "keyboard", "frame": [0, 500, 393, 300], "children": [{ "type": "key", "label": "q" }] }
        ]});
        assert_eq!(keyboard(&json!({ "nodes": [v.clone()] })), Some(Rect { x: 0.0, y: 500.0, w: 393.0, h: 300.0 }));
        let alert = json!({ "nodes": [v.clone(), { "type": "alert", "system": true, "label": "Allow?", "frame": [40, 300, 300, 200], "enabled": true, "children": [
            { "type": "staticText", "label": "Allow “Demo” to use your location?", "frame": [50, 310, 280, 40] },
            { "type": "button", "label": "Allow Once", "frame": [40, 400, 300, 40], "enabled": true },
            { "type": "button", "label": "Allow While Using App", "frame": [40, 440, 300, 40], "enabled": true },
            { "type": "button", "label": "Don’t Allow", "frame": [40, 480, 300, 40], "enabled": true }
        ]}]});
        let a = system_alert(&alert).expect("alert");
        assert!(a.text.starts_with("Allow “Demo”"), "{}", a.text);
        assert_eq!(a.dismiss(Policy::Deny), Some(Point { x: 190.0, y: 500.0 }));
        assert_eq!(a.dismiss(Policy::Allow), Some(Point { x: 190.0, y: 460.0 }));
        assert!(system_alert(&json!({ "nodes": [v.clone()] })).is_none());
        let mut refs = RefRegistry::default();
        let snap = Snapshot::build("Login", "ios, com.fastpilot.demo", convert(&v).into_iter().collect(), &mut refs);
        assert_eq!(
            snap.render(None, 120),
            "screen \"Login\" (ios, com.fastpilot.demo)\ne1 text \"Login\"\ne2 textfield \"Email\" value=\"\"\ne3 textfield \"Password\" secure\ne4 switch \"Ingat saya\"\ne5 button \"Masuk\""
        );
    }
}
