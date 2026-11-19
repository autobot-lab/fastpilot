//! Android driver (PRD §7.4): adb server protocol + the on-device agent (UiAutomation inside an
//! instrumentation APK, embedded in this binary). WebView content is driven over CDP via
//! `webview_devtools_remote_<pid>` and merged into the native snapshot.

mod adb;
mod agent;
pub mod device;
mod system;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use fastpilot_core::{
    Direction, Driver, Error as CoreError, Intent, Launch, LaunchArg, Node, Platform, Point, Rect, RefRegistry, Result, Selector, Snapshot, Value,
};
use fastpilot_web::{Browser, WebPage, http_get};
use serde_json::json;
use tokio::net::TcpStream;

pub use adb::{Adb, sdk_tool};

/// Whether this build carries the Android agent APK (release builds do).
pub fn agent_embedded() -> bool {
    !AGENT_APK.is_empty()
}
pub use agent::{Agent, Endpoint};
pub use system::Policy;
use system::Kind;

const AGENT_APK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fastpilot-agent.apk"));
const AGENT_PKG: &str = "com.fastpilot.agent";
/// Roles that own the keyboard while focused.
const EDITABLE: &[&str] = &["textfield", "combobox", "spinbutton"];
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
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Offline(String),
    /// Screen locked with a PIN/pattern fastpilot cannot (and must not) enter.
    #[error("{0}")]
    Locked(String),
    /// The OEM blocked installing the agent (MIUI "Install via USB", Samsung Auto Blocker, ...).
    #[error("{0}")]
    InstallBlocked(String),
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
            // MIUI/HyperOS refuses injected input until its security toggle is on.
            Error::Agent(m) if m.contains("INJECT_EVENTS") => Self::Session(format!("{m}: {}", device::XIAOMI_HINT)),
            // The device needs the user: an environment problem, not a product bug.
            Error::Unauthorized(m) | Error::Offline(m) | Error::Locked(m) => Self::Session(m.clone()),
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
/// One word for the device shell, whatever it contains.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// `am start` extras: `--es` text, `--ei`/`--el` whole numbers, `--ez` booleans, `--esa` text lists.
/// `am` splits lists on `,` and keeps the backslash of `\,`, so an item with a comma cannot arrive
/// as written: refused rather than changed.
fn intent_extras(args: &[(String, LaunchArg)]) -> Result<String> {
    args.iter()
        .map(|(k, v)| {
            let q = sh_quote(k);
            Ok(match v {
                LaunchArg::Str(s) => format!("--es {q} {}", sh_quote(s)),
                LaunchArg::Int(n) if i32::try_from(*n).is_ok() => format!("--ei {q} {n}"),
                LaunchArg::Int(n) => format!("--el {q} {n}"),
                LaunchArg::Bool(b) => format!("--ez {q} {b}"),
                LaunchArg::List(items) => {
                    if let Some(i) = items.iter().find(|i| i.contains(',')) {
                        return Err(CoreError::Unsupported(format!(
                            "launchApp argument `{k}`: list item {} has a comma, which Android's `am start` cannot pass in a list; pass it as a separate text argument",
                            fastpilot_core::quote(i)
                        )));
                    }
                    format!("--esa {q} {}", sh_quote(&items.join(",")))
                }
            })
        })
        .collect::<Result<Vec<_>>>()
        .map(|v| v.join(" "))
}

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
    /// `screenrecord` shell stream while recording.
    recording: tokio::sync::Mutex<Option<TcpStream>>,
    /// close() ran: Drop must not restore or quit again.
    closed: std::sync::atomic::AtomicBool,
}

/// Screen on and keyguard dismissed before the agent starts; a PIN/pattern lock stays the
/// user's to open (never automated, never passed through env).
async fn wake(adb: &Adb, serial: &str) -> std::result::Result<(), Error> {
    if !device::awake(&adb.shell(serial, "dumpsys power").await?) {
        adb.shell(serial, "input keyevent 224").await?; // KEYCODE_WAKEUP
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let _ = adb.shell(serial, "wm dismiss-keyguard").await;
    // Slow phones (MIUI) take a while to drop the keyguard: poll for about 3 s.
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        if !device::locked(&adb.shell(serial, "dumpsys window").await?) {
            return Ok(());
        }
    }
    Err(Error::Locked(format!(
        "{serial} is locked: unlock it; a PIN cannot be automated, so use Stay awake and a Swipe/None lock for test phones"
    )))
}

/// Install the agent; a key mismatch (another machine's debug keystore) is fixed once by
/// uninstalling the agent package, OEM blocks come back with their fix.
async fn install_agent(adb: &Adb, serial: &str) -> std::result::Result<(), Error> {
    let first = match adb.install(serial, AGENT_APK).await {
        Ok(()) => return Ok(()),
        Err(e) => e.to_string(),
    };
    match device::install_failure(&first) {
        device::InstallFailure::Incompatible => {
            tracing::info!(serial, "agent signed with another key: reinstalling");
            adb.shell(serial, &format!("pm uninstall {AGENT_PKG}")).await?;
            adb.install(serial, AGENT_APK).await
        }
        device::InstallFailure::UserRestricted => Err(Error::InstallBlocked(format!("installing the agent was blocked: {}", device::XIAOMI_HINT))),
        device::InstallFailure::Confirm => Err(Error::InstallBlocked("confirm the install on the phone (tap Install), then retry".into())),
        device::InstallFailure::Other => {
            let focus = adb.shell(serial, "dumpsys window | grep mCurrentFocus").await.unwrap_or_default();
            if device::install_failure(&focus) == device::InstallFailure::Confirm {
                return Err(Error::InstallBlocked("confirm the install on the phone (tap Install), then retry".into()));
            }
            Err(Error::InstallBlocked(format!("{first} ({}; {})", device::XIAOMI_HINT, device::SAMSUNG_HINT)))
        }
    }
}

const RECORDING: &str = "/data/local/tmp/fastpilot-recording.mp4";

impl AndroidDriver {
    /// Connect to `serial` (or the only attached device), install/refresh the agent, start it.
    pub async fn start(serial: Option<&str>) -> std::result::Result<Self, Error> {
        if AGENT_APK.is_empty() {
            return Err(Error::Agent("agent APK not embedded: run agent-android/build.sh, then rebuild fastpilot".into()));
        }
        let adb = Adb::new().await?;
        let serial = device::resolve(serial).await?;
        wake(&adb, &serial).await?;
        let sum = checksum(AGENT_APK);
        let marker = "/data/local/tmp/fastpilot-agent.sum";
        let installed = adb.shell(&serial, &format!("pm path {AGENT_PKG} >/dev/null && cat {marker}")).await?;
        if installed.trim() != sum {
            tracing::info!(serial, "installing fastpilot agent");
            install_agent(&adb, &serial).await?;
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
        // Animations are the main source of idle-wait flakiness (Android testing guide). The
        // user's values are saved first and restored by close().
        if let Err(e) = adb.shell(&serial, &device::prepare_script()).await {
            tracing::warn!(serial, error = %e, "could not save/disable animations");
        }
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
            recording: tokio::sync::Mutex::new(None),
            closed: std::sync::atomic::AtomicBool::new(false),
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
                self.dismiss_ime().await?;
                continue;
            }
            let node = node?;
            if let (Some(ime), Some(b)) = (ime, node.bounds)
                && ime.contains(b.center())
            {
                self.note(format!("keyboard covered {sel}; hidden"));
                self.dismiss_ime().await?;
                continue;
            }
            return Ok((snap, node));
        }
        let snap = self.snapshot().await?;
        let node = snap.resolve(sel, intent)?.node.clone();
        Ok((snap, node))
    }

    /// Back closes the IME without navigating, but only while a text field of the screen has
    /// focus. Right after a screen change the old screen's keyboard is still reported while it
    /// closes; Back then would leave the new screen (issue #8: Dashboard → Login), so wait for
    /// it to go instead.
    async fn dismiss_ime(&self) -> Result<()> {
        let (v, snap) = self.screen().await?;
        if system::ime(&v).is_none() {
            return Ok(());
        }
        if snap.entries.iter().any(|e| e.node.focused && EDITABLE.contains(&e.node.role.as_str())) {
            self.acted();
            self.agent.call("key", json!({ "code": 4 })).await?;
        } else {
            tracing::debug!("keyboard without a focused field (closing); waiting instead of Back");
        }
        let _ = self.agent.call("idle", json!({ "quietMs": 100, "timeoutMs": 1500 })).await;
        Ok(())
    }

    /// Emulator console command through `adb emu` (which handles the console auth token).
    async fn console(&self, args: &[&str]) -> Result<()> {
        if !self.serial.starts_with("emulator-") {
            return Err(CoreError::Unsupported(format!("`{}` needs an emulator (its console); {} is a device", args.join(" "), self.serial)));
        }
        let out = tokio::process::Command::new(adb::sdk_tool("platform-tools/adb", "adb"))
            .args(["-s", &self.serial, "emu"])
            .args(args)
            .output()
            .await
            .map_err(|e| CoreError::Driver(format!("adb emu: {e}")))?;
        let text = String::from_utf8_lossy(&out.stdout);
        if text.lines().any(|l| l.starts_with("OK")) {
            Ok(())
        } else {
            Err(CoreError::Driver(format!("emulator console `{}`: {}", args.join(" "), text.trim())))
        }
    }

    /// `app` (a package, or package/activity), else the app under test.
    fn app_or(&self, app: Option<&str>) -> Result<String> {
        match app {
            Some(a) => Ok(a.split('/').next().unwrap_or(a).to_owned()),
            None => lock(&self.app).clone().ok_or_else(|| CoreError::Flow("no app under test yet; pass an appId or launchApp first".into())),
        }
    }

    /// The app was stopped on purpose: it no longer running is not a crash.
    fn forget(&self, pkg: &str) {
        let mut app = lock(&self.app);
        if app.as_deref() == Some(pkg) {
            *app = None;
        }
    }

    /// The app under test is `pkg` from now on; forget crashes of earlier runs.
    /// `package/activity` as given, or the launcher activity of `package`.
    async fn component(&self, target: &str) -> Result<String> {
        if target.contains('/') {
            return Ok(target.to_owned());
        }
        let out = self.shell(&format!("cmd package resolve-activity --brief -c android.intent.category.LAUNCHER {target}")).await?;
        let comp = out.lines().last().unwrap_or("").trim().to_owned();
        if !comp.contains('/') {
            return Err(CoreError::NotFound(format!("no launcher activity for {target}")));
        }
        Ok(comp)
    }

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
    /// Fallback when close() was not called: restore settings and stop the agent.
    fn drop(&mut self) {
        if self.closed.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let (agent, adb, serial) = (self.agent.clone(), self.adb.clone(), self.serial.clone());
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = adb.shell(&serial, &device::restore_script()).await;
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

/// Runtime permissions behind a generic name; `android.permission.*` names pass through.
fn permissions(name: &str) -> Vec<String> {
    let short: &[&str] = match name {
        "camera" => &["CAMERA"],
        "microphone" => &["RECORD_AUDIO"],
        "location" => &["ACCESS_FINE_LOCATION", "ACCESS_COARSE_LOCATION"],
        "notifications" => &["POST_NOTIFICATIONS"],
        "contacts" => &["READ_CONTACTS", "WRITE_CONTACTS"],
        "photos" | "storage" => &["READ_MEDIA_IMAGES", "READ_MEDIA_VIDEO"],
        "calendar" => &["READ_CALENDAR", "WRITE_CALENDAR"],
        "sms" => &["READ_SMS", "RECEIVE_SMS"],
        other => return vec![other.to_owned()],
    };
    short.iter().map(|p| format!("android.permission.{p}")).collect()
}

/// Single-quoted for `sh -c` (the adb shell service runs one).
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
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
        } else {
            format!("am start -W -n {}", self.component(target).await?)
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
        let after = match self.screen().await {
            Err(e @ (CoreError::AppCrashed(_) | CoreError::AppNotResponding(_))) => return Err(e),
            other => other,
        };
        // The keyboard of a field typed into just before came up under the finger (issue #8):
        // the tap went to the keyboard. Look the target up again (that hides the keyboard).
        if let Ok((raw, _)) = &after
            && !EDITABLE.contains(&node.role.as_str())
            && system::ime(raw).is_some_and(|k| k.contains(p))
        {
            self.note(format!("keyboard came up under the tap on {sel}; tapped again"));
            let (_, node) = self.target(sel, Intent::Tap).await?;
            self.tap_at(self.center(&node)?).await?;
            return Ok(());
        }
        let after = after.map(|(_, s)| s);
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

    /// Set-of-Mark drawn on the device (android): boxes and refs in screen coordinates.
    async fn screenshot_annotated(&self, marks: &[(String, Rect)]) -> Result<Vec<u8>> {
        let list: Vec<Value> = marks.iter().map(|(r, b)| json!({ "r": r, "x": b.x, "y": b.y, "w": b.w, "h": b.h })).collect();
        let v = self.agent.call("screenshot", json!({ "quality": 80, "marks": list })).await?;
        if v["marked"].as_u64() != Some(marks.len() as u64) {
            return Err(CoreError::Driver(format!("agent drew {} of {} marks (rebuild the agent)", v["marked"], marks.len())));
        }
        base64::engine::general_purpose::STANDARD
            .decode(s(&v, "jpeg"))
            .map_err(|e| CoreError::Driver(format!("screenshot decode: {e}")))
    }

    fn platform(&self) -> Platform {
        Platform::Android
    }

    /// Restore the user's animation scales and stay-awake setting, then stop the agent.
    async fn close(&self) -> Result<()> {
        if self.closed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        let restored = self.adb.shell(&self.serial, &device::restore_script()).await;
        let _ = self.agent.call("quit", json!({})).await;
        restored.map(|_| ()).map_err(Into::into)
    }

    /// Emulator console `geo fix` (real devices have no console to fake GPS through).
    async fn set_location(&self, lat: f64, lon: f64) -> Result<()> {
        self.console(&["geo", "fix", &lon.to_string(), &lat.to_string()]).await
    }

    async fn set_permissions(&self, app: Option<&str>, perms: &[(String, fastpilot_core::Grant)]) -> Result<()> {
        use fastpilot_core::Grant;
        let pkg = self.app_or(app)?;
        let pkg = shell_safe(&pkg)?;
        for (name, grant) in perms {
            for p in permissions(name) {
                let p = shell_safe(&p)?;
                let cmd = match grant {
                    Grant::Allow => format!("pm grant {pkg} {p}"),
                    Grant::Deny => format!("pm revoke {pkg} {p}; pm set-permission-flags {pkg} {p} user-set user-fixed"),
                    Grant::Unset => format!("pm revoke {pkg} {p}; pm clear-permission-flags {pkg} {p} user-set user-fixed"),
                };
                let out = self.adb.shell(&self.serial, &cmd).await?;
                if out.contains("Exception") || out.contains("Unknown permission") || out.contains("not a changeable") {
                    return Err(CoreError::Flow(format!("permission `{name}` ({p}): {}", out.trim())));
                }
            }
        }
        Ok(())
    }

    /// Offline = airplane mode; throttling via the emulator console (emulators only).
    async fn set_network(&self, n: &fastpilot_core::Network) -> Result<()> {
        let mode = if n.offline { "enable" } else { "disable" };
        self.adb.shell(&self.serial, &format!("cmd connectivity airplane-mode {mode}")).await?;
        let throttled = n.latency_ms.is_some() || n.download_kbps.is_some() || n.upload_kbps.is_some();
        if throttled || !n.offline {
            let speed = match (n.upload_kbps, n.download_kbps) {
                (None, None) => "full".to_owned(),
                (up, down) => format!("{}:{}", up.unwrap_or(0), down.unwrap_or(0)),
            };
            let delay = n.latency_ms.map_or("none".to_owned(), |l| l.to_string());
            let res = async {
                self.console(&["network", "speed", &speed]).await?;
                self.console(&["network", "delay", &delay]).await
            }
            .await;
            if throttled {
                res?;
            }
        }
        Ok(())
    }

    /// App locale (`cmd locale`, the app under test) and system timezone (`cmd alarm`).
    async fn set_locale(&self, locale: Option<&str>, timezone: Option<&str>) -> Result<()> {
        if let Some(l) = locale {
            let pkg = self.app_or(None)?;
            let out = self.adb.shell(&self.serial, &format!("cmd locale set-app-locales {} --locales {}", shell_safe(&pkg)?, shell_safe(l)?)).await?;
            if !out.trim().is_empty() {
                return Err(CoreError::Flow(format!("locale `{l}`: {}", out.trim())));
            }
        }
        if let Some(tz) = timezone {
            let out = self.adb.shell(&self.serial, &format!("cmd alarm set-timezone {}", shell_safe(tz)?)).await?;
            if out.contains("rror") || out.contains("nvalid") {
                return Err(CoreError::Flow(format!("timezone `{tz}`: {}", out.trim())));
            }
        }
        Ok(())
    }

    /// Posts a notification from the shell (real push needs the app's server and FCM).
    async fn push_notification(&self, _app: Option<&str>, title: &str, body: &str, _payload: Option<&Value>) -> Result<()> {
        let cmd = format!("cmd notification post -S bigtext -t {} fastpilot {}", quote(title), quote(if body.is_empty() { title } else { body }));
        let out = self.adb.shell(&self.serial, &cmd).await?;
        if !out.contains("posting") {
            return Err(CoreError::Driver(format!("notification: {}", out.trim())));
        }
        Ok(())
    }

    /// `screenrecord` (3 minutes at most, the tool's own limit).
    async fn start_recording(&self) -> Result<()> {
        let mut slot = self.recording.lock().await;
        if slot.is_some() {
            return Err(CoreError::Flow("already recording".into()));
        }
        let cmd = format!("screenrecord --time-limit 180 --bit-rate 2000000 {RECORDING}");
        *slot = Some(self.adb.shell_stream(&self.serial, &cmd).await?);
        Ok(())
    }

    async fn stop_recording(&self) -> Result<fastpilot_core::Recording> {
        use tokio::io::AsyncReadExt;
        let mut stream = self.recording.lock().await.take().ok_or_else(|| CoreError::Flow("not recording".into()))?;
        // SIGINT lets screenrecord finish the mp4; its shell stream closes once it has.
        self.shell("pkill -INT screenrecord").await?;
        let mut sink = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut sink)).await;
        let data = self.adb.pull(&self.serial, RECORDING).await?;
        let _ = self.shell(&format!("rm -f {RECORDING}")).await;
        Ok(fastpilot_core::Recording::Video { mime: "video/mp4", data })
    }

    /// The SMS provider sorts the inbox newest first; a body may span several lines.
    async fn read_sms(&self) -> Result<Vec<String>> {
        let out = self.shell("content query --uri content://sms/inbox --projection body").await?;
        let mut bodies: Vec<String> = Vec::new();
        for line in out.lines() {
            match line.strip_prefix("Row: ").and_then(|r| r.split_once(" body=")) {
                Some((_, body)) => bodies.push(body.to_owned()),
                None => {
                    if let Some(last) = bodies.last_mut() {
                        last.push('\n');
                        last.push_str(line);
                    }
                }
            }
        }
        Ok(bodies)
    }

    async fn long_press(&self, sel: &Selector) -> Result<()> {
        let p = match sel {
            Selector::Point(p) => *p,
            _ => self.center(&self.resolve(sel, Intent::Tap).await?)?,
        };
        self.acted();
        // A swipe that does not move is a press held for `ms`.
        self.agent.call("swipe", json!({ "x1": p.x, "y1": p.y, "x2": p.x, "y2": p.y, "ms": 800 })).await?;
        Ok(())
    }

    async fn double_tap(&self, sel: &Selector) -> Result<()> {
        let p = match sel {
            Selector::Point(p) => *p,
            _ => self.center(&self.resolve(sel, Intent::Tap).await?)?,
        };
        self.tap_at(p).await?;
        self.tap_at(p).await
    }

    async fn hide_keyboard(&self) -> Result<()> {
        self.dismiss_ime().await
    }

    async fn erase_text(&self, count: Option<usize>) -> Result<()> {
        let snap = self.snapshot().await?;
        let field = snap.resolve(&Selector::Focused, Intent::Type)?.node.clone();
        let text = field.value.unwrap_or_default();
        let keep = match count {
            Some(n) => text.chars().count().saturating_sub(n),
            None => 0,
        };
        self.acted();
        self.agent.call("text", json!({ "text": text.chars().take(keep).collect::<String>() })).await?;
        Ok(())
    }

    async fn stop_app(&self, app: Option<&str>) -> Result<()> {
        let pkg = self.app_or(app)?;
        self.shell(&format!("am force-stop {}", shell_safe(&pkg)?)).await?;
        self.forget(&pkg);
        Ok(())
    }

    /// `pm clear`: data, caches and runtime permissions; the app is stopped too.
    async fn clear_state(&self, app: Option<&str>) -> Result<()> {
        let pkg = self.app_or(app)?;
        let out = self.shell(&format!("pm clear {}", shell_safe(&pkg)?)).await?;
        if !out.contains("Success") {
            return Err(CoreError::Driver(format!("pm clear {pkg}: {}", out.trim())));
        }
        self.forget(&pkg);
        *self.web.lock().await = None;
        Ok(())
    }

    async fn press_key(&self, key: &str) -> Result<()> {
        self.acted();
        let code = keycode(key).ok_or_else(|| CoreError::Unsupported(format!("unknown key `{key}`")))?;
        self.agent.call("key", json!({ "code": code })).await?;
        Ok(())
    }

    /// Maestro semantics: stop the app, then start it fresh.
    async fn launch_app(&self, app: &str) -> Result<()> {
        self.launch_app_with(app, &Launch::default()).await
    }

    /// `arguments` become intent extras; Android apps get no process environment from
    /// `am start`, so `env` is refused with the way to do it instead.
    async fn launch_app_with(&self, app: &str, launch: &Launch) -> Result<()> {
        if let Some((k, _)) = launch.env.first() {
            return Err(CoreError::Unsupported(format!(
                "launchApp `env` ({k}) is iOS only: an Android app gets no environment from `am start`; pass it as `arguments` (intent extras)"
            )));
        }
        let app = shell_safe(app)?;
        let pkg = app.split('/').next().unwrap_or(app);
        self.shell(&format!("am force-stop {pkg}")).await?;
        *self.web.lock().await = None;
        if launch.arguments.is_empty() {
            return self.open(app).await;
        }
        // Through adb's shell (`sh` on the device), not the agent's: UiAutomation's
        // executeShellCommand splits on spaces and would ignore the quoting of the extras.
        let extras = intent_extras(&launch.arguments)?;
        let cmd = format!("am start -W -n {} {extras}", self.component(app).await?);
        let out = self.adb.shell(&self.serial, &cmd).await.map_err(|e| CoreError::Driver(e.to_string()))?;
        if out.contains("Error") {
            return Err(CoreError::Driver(out.trim().to_owned()));
        }
        self.track(app).await;
        Ok(())
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

    #[test]
    fn launch_arguments_become_quoted_intent_extras() {
        let args = vec![
            ("server".to_owned(), LaunchArg::Str("https://staging.x/api?a=1&b='2' ; reboot".into())),
            ("retries".to_owned(), LaunchArg::Int(3)),
            ("big".to_owned(), LaunchArg::Int(5_000_000_000)),
            ("debug".to_owned(), LaunchArg::Bool(true)),
            ("tags".to_owned(), LaunchArg::List(vec!["a".into(), "b c".into()])),
        ];
        assert_eq!(
            intent_extras(&args).expect("extras"),
            r"--es 'server' 'https://staging.x/api?a=1&b='\''2'\'' ; reboot' --ei 'retries' 3 --el 'big' 5000000000 --ez 'debug' true --esa 'tags' 'a,b c'"
        );
        let comma = [("tags".to_owned(), LaunchArg::List(vec!["b,c".into()]))];
        assert!(intent_extras(&comma).is_err_and(|e| e.to_string().contains("has a comma")));
    }
}
