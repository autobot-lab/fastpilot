//! Android driver (PRD §7.4): adb server protocol + the on-device agent (UiAutomation inside an
//! instrumentation APK, embedded in this binary). WebView content is driven over CDP via
//! `webview_devtools_remote_<pid>` and merged into the native snapshot.

mod adb;
mod agent;
mod system;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use fastpilot_core::{Direction, Driver, Error as CoreError, Intent, Node, Point, Rect, RefRegistry, Result, Selector, Snapshot, Value};
use fastpilot_web::{Browser, WebPage, http_get};
use serde_json::json;
use tokio::net::TcpStream;

pub use adb::Adb;
pub use agent::Agent;
pub use system::Policy;
use system::Kind;

const AGENT_APK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fastpilot-agent.apk"));
const AGENT_PKG: &str = "com.fastpilot.agent";
const AGENT_PORT: u16 = 7912;
/// Other UiAutomation holders: only one may exist per device.
const RIVALS: &[&str] = &["com.github.uiautomator", "com.github.uiautomator.test", "io.appium.uiautomator2.server", "io.appium.uiautomator2.server.test"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("adb: {0}")]
    Adb(String),
    #[error("agent: {0}")]
    Agent(String),
    #[error("io: {0}")]
    Io(String),
}

impl Error {
    fn io(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<Error> for CoreError {
    fn from(e: Error) -> Self {
        match &e {
            Error::Agent(m) if m.contains("no focused text field") => Self::NotActionable(m.clone()),
            _ => Self::Driver(e.to_string()),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// FNV-1a, to tell whether the installed agent is the embedded one.
fn checksum(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Shell arguments come from the AI: allow only what package names, components and URLs need.
fn shell_safe(s: &str) -> Result<&str> {
    let ok = !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "._/:-?=&%#+~@,$".contains(c));
    if ok { Ok(s) } else { Err(CoreError::Flow(format!("unsafe characters in `{s}`"))) }
}

/// `(serial, state)` of attached devices; available AVDs.
pub async fn devices() -> std::result::Result<(Vec<(String, String)>, Vec<String>), Error> {
    let attached = Adb::new().await?.devices().await?;
    let out = tokio::process::Command::new(adb::sdk_tool("emulator/emulator", "emulator"))
        .arg("-list-avds")
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(str::to_owned).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default();
    Ok((attached, out))
}

/// Boot an AVD headless and wait for `sys.boot_completed`; returns its serial.
pub async fn start_emulator(avd: &str) -> std::result::Result<String, Error> {
    let adb = Adb::new().await?;
    let before: Vec<String> = adb.devices().await?.into_iter().map(|(s, _)| s).collect();
    tokio::process::Command::new(adb::sdk_tool("emulator/emulator", "emulator"))
        .args(["-avd", avd, "-no-window", "-no-audio", "-no-boot-anim", "-no-snapshot-save", "-gpu", "swiftshader_indirect"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error::Adb(format!("cannot start emulator: {e}")))?;
    for _ in 0..240 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        for (serial, state) in adb.devices().await? {
            if before.contains(&serial) || state != "device" {
                continue;
            }
            if adb.shell(&serial, "getprop sys.boot_completed").await?.trim() == "1" {
                return Ok(serial);
            }
        }
    }
    Err(Error::Adb(format!("emulator {avd} did not boot in 120s")))
}

struct Web {
    socket: String,
    page: WebPage,
}

pub struct AndroidDriver {
    adb: Adb,
    serial: String,
    agent: Arc<Agent>,
    size: (f64, f64),
    /// `am instrument -w` stream: the agent lives as long as this stays open.
    _instrument: TcpStream,
    refs: Mutex<RefRegistry>,
    web: tokio::sync::Mutex<Option<Web>>,
    /// The screen was seen idle after the last action (tap verification), so the engine's
    /// post-action settle can skip a second idle wait.
    settled: std::sync::atomic::AtomicBool,
    /// Package of the app under test (set by open/launchApp), to detect it crashing.
    app: Mutex<Option<String>>,
    check_crash: std::sync::atomic::AtomicBool,
    policy: Policy,
    /// System dialogs handled on the flow's behalf, newest last (reported with failures).
    notes: Mutex<Vec<String>>,
}

impl AndroidDriver {
    /// Connect to `serial` (or the only attached device), install/refresh the agent, start it.
    pub async fn start(serial: Option<&str>) -> std::result::Result<Self, Error> {
        if AGENT_APK.is_empty() {
            return Err(Error::Agent("agent APK not embedded: run agent-android/build.sh, then rebuild fastpilot".into()));
        }
        let adb = Adb::new().await?;
        let ready: Vec<String> = adb.devices().await?.into_iter().filter(|(_, st)| st == "device").map(|(s, _)| s).collect();
        let serial = match serial {
            Some(s) if ready.iter().any(|r| r == s) => s.to_owned(),
            Some(s) => return Err(Error::Adb(format!("device `{s}` not attached (have: {})", ready.join(", ")))),
            None => match ready.as_slice() {
                [one] => one.clone(),
                [] => return Err(Error::Adb("no Android device; boot one with session start_device".into())),
                many => return Err(Error::Adb(format!("several devices, pass endpoint: {}", many.join(", ")))),
            },
        };
        let sum = checksum(AGENT_APK);
        let marker = "/data/local/tmp/fastpilot-agent.sum";
        let installed = adb.shell(&serial, &format!("pm path {AGENT_PKG} >/dev/null && cat {marker}")).await?;
        if installed.trim() != sum {
            tracing::info!(serial, "installing fastpilot agent");
            adb.install(&serial, AGENT_APK).await?;
            adb.shell(&serial, &format!("echo {sum} > {marker}")).await?;
        }
        let stop: Vec<String> = std::iter::once(AGENT_PKG).chain(RIVALS.iter().copied()).map(|p| format!("am force-stop {p}")).collect();
        adb.shell(&serial, &stop.join("; ")).await?;
        let stream = adb
            .shell_stream(&serial, &format!("am instrument -w -e port {AGENT_PORT} {AGENT_PKG}/.Agent"))
            .await?;
        let (instrument, _) = adb::wait_line(stream, "FASTPILOT_AGENT_READY", Duration::from_secs(20)).await?;
        let port = adb.forward(&serial, &format!("tcp:{AGENT_PORT}")).await?;
        let agent = Arc::new(Agent::connect(port).await?);
        let hello = agent.call("hello", json!({})).await?;
        let size = (hello["width"].as_f64().unwrap_or(1080.0), hello["height"].as_f64().unwrap_or(1920.0));
        // Animations are the main source of idle-wait flakiness (Android testing guide).
        let _ = agent
            .call(
                "shell",
                json!({ "command": "settings put global window_animation_scale 0; settings put global transition_animation_scale 0; settings put global animator_duration_scale 0" }),
            )
            .await;
        Ok(Self {
            adb,
            serial,
            agent,
            size,
            _instrument: instrument,
            refs: Mutex::default(),
            web: tokio::sync::Mutex::new(None),
            settled: std::sync::atomic::AtomicBool::new(false),
            app: Mutex::new(None),
            check_crash: std::sync::atomic::AtomicBool::new(false),
            policy: Policy::from_env(),
            notes: Mutex::default(),
        })
    }

    /// Input was sent: the next idle wait must really wait, and the next snapshot checks
    /// whether the app crashed meanwhile.
    fn acted(&self) {
        self.settled.store(false, std::sync::atomic::Ordering::SeqCst);
        self.check_crash.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn note(&self, text: String) {
        tracing::info!("{text}");
        let mut n = lock(&self.notes);
        n.push(text);
        let extra = n.len().saturating_sub(20);
        n.drain(..extra);
    }

    /// Raw agent snapshot plus the core snapshot. A crash or ANR of the app under test fails
    /// with `app_crashed` / `app_not_responding` (the dialog is closed so the device recovers).
    async fn screen(&self) -> Result<(Value, Snapshot)> {
        let v = self.agent.call("snapshot", json!({})).await?;
        let dialog = system::dialog(&v);
        if let Some(d) = dialog.as_ref().filter(|d| d.kind != Kind::Permission) {
            if let Some(p) = d.dismiss(self.policy) {
                let _ = self.tap_at(p).await;
            }
            *lock(&self.app) = None;
            return Err(match d.kind {
                Kind::Anr => CoreError::AppNotResponding(format!("{} (dialog closed)", d.text)),
                _ => CoreError::AppCrashed(format!("{} (dialog closed)", d.text)),
            });
        }
        let pkg = s(&v, "package").to_owned();
        let app = lock(&self.app).clone();
        let acted = self.check_crash.swap(false, std::sync::atomic::Ordering::SeqCst);
        if let Some(app) = app.filter(|a| (acted || *a != pkg) && dialog.is_none()) {
            // Recent Android versions close (and may restart) a crashed app without a dialog.
            let log = self.shell("logcat -b crash -d").await.unwrap_or_default();
            if let Some(why) = system::crash_in_log(&log, &app) {
                let _ = self.shell("logcat -b crash -c").await;
                *lock(&self.app) = None;
                return Err(CoreError::AppCrashed(format!("{app} crashed: {why}")));
            }
        }
        let snap = self.build(&v, &pkg).await;
        Ok((v, snap))
    }

    async fn build(&self, v: &Value, pkg: &str) -> Snapshot {
        let mut roots: Vec<Node> = v["nodes"].as_array().into_iter().flatten().map(convert).collect();
        if let Some(wv) = find_webview(&mut roots) {
            let mut slot = self.web.lock().await;
            if let Some(page) = self.webview(pkg, &mut slot).await {
                match page.raw_nodes().await {
                    // Web nodes keep their CDP handles: actions on them go through CDP.
                    Ok((_, _, nodes)) => wv.children = nodes,
                    Err(_) => *slot = None,
                }
            }
        }
        let title = if s(v, "title").is_empty() { pkg.to_owned() } else { s(v, "title").to_owned() };
        let mut refs = lock(&self.refs);
        Snapshot::build(title, format!("android, {pkg}"), roots, &mut refs)
    }

    /// Resolve a tap/type target, first clearing what stands in its way: a permission prompt
    /// the target is not part of (per `FASTPILOT_PERMISSIONS`), or the keyboard covering it.
    async fn target(&self, sel: &Selector, intent: Intent) -> Result<(Snapshot, Node)> {
        for _ in 0..3 {
            let (raw, snap) = self.screen().await?;
            let node = snap.resolve(sel, intent).map(|e| e.node.clone());
            if let Some(d) = system::dialog(&raw) {
                let ours = node.as_ref().is_ok_and(|n| n.bounds.is_some_and(|b| d.owns(&b)));
                if let (false, Some(p)) = (ours, d.dismiss(self.policy)) {
                    self.note(format!("permission prompt {:?} dismissed ({:?}) before {sel}", d.text, self.policy));
                    self.tap_at(p).await?;
                    let _ = self.agent.call("idle", json!({ "quietMs": 100, "timeoutMs": 1500 })).await;
                    continue;
                }
            }
            // A target under the keyboard may also drop out of the tree (not visible to user).
            let ime = system::ime(&raw);
            if ime.is_some() && matches!(node, Err(CoreError::NotFound(_))) {
                self.note(format!("{sel} not on screen with the keyboard up; keyboard hidden"));
                self.hide_keyboard().await?;
                continue;
            }
            let node = node?;
            if let (Some(ime), Some(b)) = (ime, node.bounds)
                && ime.contains(b.center())
            {
                self.note(format!("keyboard covered {sel}; hidden"));
                self.hide_keyboard().await?;
                continue;
            }
            return Ok((snap, node));
        }
        let snap = self.snapshot().await?;
        let node = snap.resolve(sel, intent)?.node.clone();
        Ok((snap, node))
    }

    /// Back closes the IME without navigating; only sent while the keyboard is up.
    async fn hide_keyboard(&self) -> Result<()> {
        let v = self.agent.call("snapshot", json!({})).await?;
        if system::ime(&v).is_some() {
            self.acted();
            self.agent.call("key", json!({ "code": 4 })).await?;
            let _ = self.agent.call("idle", json!({ "quietMs": 100, "timeoutMs": 1500 })).await;
        }
        Ok(())
    }

    /// The app under test is `pkg` from now on; forget crashes of earlier runs.
    async fn track(&self, target: &str) {
        if target.contains("://") {
            return;
        }
        let pkg = target.split('/').next().unwrap_or(target).to_owned();
        let _ = self.shell("logcat -b crash -c").await;
        *lock(&self.app) = Some(pkg);
    }

    async fn shell(&self, cmd: &str) -> Result<String> {
        let v = self.agent.call("shell", json!({ "command": cmd })).await?;
        Ok(v["out"].as_str().unwrap_or("").to_owned())
    }

    /// CDP page of the WebView in `pkg`'s process, cached while the socket stays the same.
    async fn webview<'a>(&self, pkg: &str, slot: &'a mut Option<Web>) -> Option<&'a WebPage> {
        let pid = self.shell(&format!("pidof {}", shell_safe(pkg).ok()?)).await.ok()?;
        let socket = format!("webview_devtools_remote_{}", pid.split_whitespace().next()?);
        if slot.as_ref().is_some_and(|w| w.socket == socket) {
            return slot.as_ref().map(|w| &w.page);
        }
        let port = self.adb.forward(&self.serial, &format!("localabstract:{socket}")).await.ok()?;
        let list: Value = serde_json::from_str(&http_get(&format!("127.0.0.1:{port}"), "/json/list").await.ok()?).ok()?;
        let pages = list.as_array()?;
        let page = pages
            .iter()
            .find(|p| p["type"] == "page" && p["description"].as_str().is_some_and(|d| d.contains("\"visible\":true")))
            .or_else(|| pages.iter().find(|p| p["type"] == "page"))?;
        let ws = page["webSocketDebuggerUrl"].as_str()?;
        let id = page["id"].as_str()?;
        let page = Browser::connect_page(ws, id).await.ok()?;
        *slot = Some(Web { socket, page });
        slot.as_ref().map(|w| &w.page)
    }

    async fn web_page(&self) -> Result<tokio::sync::MappedMutexGuard<'_, WebPage>> {
        let pkg = self.agent.call("snapshot", json!({})).await?["package"].as_str().unwrap_or("").to_owned();
        let mut slot = self.web.lock().await;
        if self.webview(&pkg, &mut slot).await.is_none() {
            return Err(CoreError::Unsupported("no debuggable WebView on screen (needs setWebContentsDebuggingEnabled)".into()));
        }
        tokio::sync::MutexGuard::try_map(slot, |s| s.as_mut().map(|w| &mut w.page))
            .map_err(|_| CoreError::Unsupported("no WebView".into()))
    }

    async fn resolve(&self, sel: &Selector, intent: Intent) -> Result<Node> {
        Ok(self.target(sel, intent).await?.1)
    }

    async fn tap_at(&self, p: Point) -> Result<()> {
        self.acted();
        self.agent.call("tap", json!({ "x": p.x, "y": p.y })).await?;
        Ok(())
    }

    /// Actionability for native nodes: enabled, non-empty box on screen.
    fn center(&self, n: &Node) -> Result<Point> {
        if n.disabled {
            return Err(CoreError::NotActionable("disabled".into()));
        }
        match n.bounds {
            Some(b) if b.w > 0.0 && b.h > 0.0 => Ok(b.center()),
            _ => Err(CoreError::NotActionable("not visible".into())),
        }
    }
}

impl Drop for AndroidDriver {
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
        if !n.name.is_empty() && matches!(n.role.as_str(), "text" | "button" | "link") {
            out.push(n.name.clone());
        } else {
            text_of(&n.children, out);
        }
    }
}

/// Agent JSON node -> core node with canonical roles.
fn convert(v: &Value) -> Node {
    let cls = s(v, "cls");
    let short = cls.rsplit('.').next().unwrap_or(cls);
    let flag = |k: &str| v[k].as_bool() == Some(true);
    let (text, desc, hint) = (s(v, "text"), s(v, "desc"), s(v, "hint"));
    let id = s(v, "id").rsplit('/').next().filter(|i| !i.is_empty()).map(str::to_owned);
    let children: Vec<Node> = v["children"].as_array().into_iter().flatten().map(convert).collect();
    let editable = flag("editable") || short.contains("EditText");
    let role = if editable {
        "textfield"
    } else if short.contains("CheckBox") {
        "checkbox"
    } else if short.contains("Switch") || short.contains("ToggleButton") {
        "switch"
    } else if short.contains("RadioButton") {
        "radio"
    } else if short.contains("Spinner") {
        "combobox"
    } else if short.contains("SeekBar") {
        "slider"
    } else if short.contains("WebView") {
        "webview"
    } else if short.contains("Button") || short.contains("Chip") || (flag("clickable") && (!text.is_empty() || !desc.is_empty())) {
        "button"
    } else if flag("clickable") {
        "clickable"
    } else if short.contains("ImageView") {
        "image"
    } else if !text.is_empty() || !desc.is_empty() {
        "text"
    } else {
        "none"
    };
    let first = |opts: &[&str]| opts.iter().find(|o| !o.is_empty()).map(|o| (*o).to_owned()).unwrap_or_default();
    let mut name = match role {
        "textfield" => first(&[hint, desc, id.as_deref().unwrap_or("")]),
        _ => first(&[text, desc]),
    };
    if role == "clickable" && name.is_empty() {
        let mut parts = Vec::new();
        text_of(&children, &mut parts);
        name = parts.join(" ");
    }
    let b = v["bounds"].as_array().map(|b| [0, 1, 2, 3].map(|i| b.get(i).and_then(Value::as_f64).unwrap_or(0.0)));
    Node {
        role: role.to_owned(),
        name,
        value: editable.then(|| text.to_owned()),
        id,
        secure: flag("password"),
        disabled: !flag("enabled"),
        checked: flag("checked"),
        selected: flag("selected"),
        focused: flag("focused"),
        bounds: b.map(|[l, t, r, btm]| Rect { x: l, y: t, w: r - l, h: btm - t }),
        children,
        ..Node::default()
    }
}

fn find_webview(nodes: &mut [Node]) -> Option<&mut Node> {
    for n in nodes {
        if n.role == "webview" {
            return Some(n);
        }
        if let Some(w) = find_webview(&mut n.children) {
            return Some(w);
        }
    }
    None
}

fn keycode(key: &str) -> Option<u32> {
    Some(match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => 66,
        "tab" => 61,
        "escape" | "esc" => 111,
        "backspace" => 67,
        "delete" => 112,
        "space" => 62,
        "arrowup" | "up" => 19,
        "arrowdown" | "down" => 20,
        "arrowleft" | "left" => 21,
        "arrowright" | "right" => 22,
        "home" => 122,
        "end" => 123,
        "pageup" => 92,
        "pagedown" => 93,
        "back" => 4,
        _ => return None,
    })
}

#[async_trait]
impl Driver for AndroidDriver {
    /// `package`, `package/activity`, or a URL (opened with an ACTION_VIEW intent).
    async fn open(&self, target: &str) -> Result<()> {
        let target = shell_safe(target)?;
        let cmd = if target.contains("://") {
            format!("am start -W -a android.intent.action.VIEW -d '{target}'")
        } else if target.contains('/') {
            format!("am start -W -n {target}")
        } else {
            let out = self.shell(&format!("cmd package resolve-activity --brief -c android.intent.category.LAUNCHER {target}")).await?;
            let comp = out.lines().last().unwrap_or("").trim().to_owned();
            if !comp.contains('/') {
                return Err(CoreError::NotFound(format!("no launcher activity for {target}")));
            }
            format!("am start -W -n {comp}")
        };
        let out = self.shell(&cmd).await?;
        if out.contains("Error") {
            return Err(CoreError::Driver(out.trim().to_owned()));
        }
        self.track(target).await;
        Ok(())
    }

    async fn snapshot(&self) -> Result<Snapshot> {
        Ok(self.screen().await?.1)
    }

    async fn tap(&self, sel: &Selector) -> Result<()> {
        if let Selector::Point(p) = sel {
            return self.tap_at(*p).await;
        }
        let (snap, node) = self.target(sel, Intent::Tap).await?;
        if let Some(h) = node.handle {
            return self.web_page().await?.tap_handle(h).await;
        }
        let p = self.center(&node)?;
        self.tap_at(p).await?;
        // Tap verification (as Maestro does): a tap that changed nothing on screen was most
        // likely swallowed by an animation or a re-layout; retry it once.
        let idle = self.agent.call("idle", json!({ "quietMs": 100, "timeoutMs": 800 })).await;
        let before = snap.render(None, usize::MAX);
        let after = match self.snapshot().await {
            Err(e @ (CoreError::AppCrashed(_) | CoreError::AppNotResponding(_))) => return Err(e),
            other => other,
        };
        if after.is_ok_and(|after| after.render(None, usize::MAX) == before) {
            tracing::debug!(%sel, "tap had no visible effect; retrying once");
            self.tap_at(p).await?;
        } else if idle.is_ok_and(|v| v["idle"].as_bool() == Some(true)) {
            self.settled.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }

    async fn type_text(&self, sel: &Selector, text: &str) -> Result<()> {
        match sel {
            Selector::Focused => {}
            Selector::Point(p) => self.tap_at(*p).await?,
            _ => {
                let node = self.resolve(sel, Intent::Type).await?;
                if let Some(h) = node.handle {
                    return self.web_page().await?.type_handle(h, text).await;
                }
                if !node.focused {
                    self.tap_at(self.center(&node)?).await?;
                }
            }
        }
        self.acted();
        self.agent.call("text", json!({ "text": text })).await?;
        Ok(())
    }

    async fn scroll(&self, dir: Direction, sel: Option<&Selector>) -> Result<()> {
        let area = match sel {
            Some(s) => self.resolve(s, Intent::Find).await?.bounds.unwrap_or_default(),
            None => Rect { x: 0.0, y: 0.0, w: self.size.0, h: self.size.1 },
        };
        let c = area.center();
        let (dx, dy) = (area.w * 0.3, area.h * 0.3);
        // Finger moves against the content direction.
        let (from, to) = match dir {
            Direction::Down => (Point { x: c.x, y: c.y + dy }, Point { x: c.x, y: c.y - dy }),
            Direction::Up => (Point { x: c.x, y: c.y - dy }, Point { x: c.x, y: c.y + dy }),
            Direction::Right => (Point { x: c.x + dx, y: c.y }, Point { x: c.x - dx, y: c.y }),
            Direction::Left => (Point { x: c.x - dx, y: c.y }, Point { x: c.x + dx, y: c.y }),
        };
        self.swipe(from, to).await
    }

    async fn swipe(&self, from: Point, to: Point) -> Result<()> {
        self.acted();
        self.agent.call("swipe", json!({ "x1": from.x, "y1": from.y, "x2": to.x, "y2": to.y, "ms": 300 })).await?;
        Ok(())
    }

    async fn back(&self) -> Result<()> {
        self.acted();
        self.agent.call("key", json!({ "code": 4 })).await?;
        Ok(())
    }

    async fn screenshot(&self) -> Result<Vec<u8>> {
        let v = self.agent.call("screenshot", json!({ "quality": 80 })).await?;
        base64::engine::general_purpose::STANDARD
            .decode(s(&v, "jpeg"))
            .map_err(|e| CoreError::Driver(format!("screenshot decode: {e}")))
    }

    /// Bounded settle (research Tier 3): 100 ms of accessibility-event quiet, capped at 1.5 s.
    async fn wait_idle(&self, timeout: Duration) -> Result<()> {
        if self.settled.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        let cap = timeout.min(Duration::from_millis(1500)).as_millis() as u64;
        let v = self.agent.call("idle", json!({ "quietMs": 100, "timeoutMs": cap })).await?;
        if v["idle"].as_bool() == Some(true) {
            Ok(())
        } else {
            Err(CoreError::Timeout(format!("screen not idle after {cap}ms")))
        }
    }

    async fn eval_script(&self, js: &str) -> Result<Value> {
        self.web_page().await?.eval_script(js).await
    }

    async fn diagnostics(&self) -> Vec<String> {
        lock(&self.notes).clone()
    }

    async fn press_key(&self, key: &str) -> Result<()> {
        self.acted();
        let code = keycode(key).ok_or_else(|| CoreError::Unsupported(format!("unknown key `{key}`")))?;
        self.agent.call("key", json!({ "code": code })).await?;
        Ok(())
    }

    /// Maestro semantics: stop the app, then start it fresh.
    async fn launch_app(&self, app: &str) -> Result<()> {
        let pkg = shell_safe(app)?.split('/').next().unwrap_or(app);
        self.shell(&format!("am force-stop {pkg}")).await?;
        *self.web.lock().await = None;
        self.open(app).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_agent_nodes() {
        let v = json!({ "cls": "android.widget.FrameLayout", "bounds": [0, 0, 720, 1280], "enabled": true, "children": [
            { "cls": "android.widget.EditText", "hint": "Email", "id": "com.x:id/email", "editable": true, "enabled": true, "bounds": [10, 100, 710, 160] },
            { "cls": "android.widget.EditText", "hint": "Password", "password": true, "editable": true, "enabled": true, "bounds": [10, 180, 710, 240] },
            { "cls": "android.widget.Button", "text": "Masuk", "clickable": true, "bounds": [10, 260, 710, 320] },
            { "cls": "android.widget.LinearLayout", "clickable": true, "enabled": true, "bounds": [0, 400, 720, 480],
              "children": [{ "cls": "android.widget.TextView", "text": "Lupa password?", "enabled": true, "bounds": [10, 410, 300, 470] }] }
        ]});
        let mut refs = RefRegistry::default();
        let snap = Snapshot::build("Login", "android, com.x", vec![convert(&v)], &mut refs);
        assert_eq!(
            snap.render(None, 120),
            "screen \"Login\" (android, com.x)\ne1 textfield \"Email\" value=\"\"\ne2 textfield \"Password\" secure\ne3 button \"Masuk\" disabled\ne4 clickable \"Lupa password?\""
        );
        let email = snap.resolve(&Selector::Id("email".into()), Intent::Type).map(|e| e.node.bounds);
        assert_eq!(email.ok().flatten().map(|b| b.center()), Some(Point { x: 360.0, y: 130.0 }));
        assert!(shell_safe("com.x/.Main").is_ok() && shell_safe("x; rm -rf /").is_err());
    }
}
