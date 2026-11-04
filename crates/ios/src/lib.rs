//! iOS driver (PRD §7.5), simulator first. Instead of Appium/WebDriverAgent it runs fastpilot's
//! own minimal XCUITest runner (`ios-runner/`, built from source with Xcode) and speaks the same
//! framed-JSON protocol as the Android agent. The simulator shares the host network, so no
//! port forwarding is needed.

pub mod device;
pub mod runner;
pub mod usbmux;
pub mod webinspector;

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

/// Simulators (iPhone/iPad) and paired physical devices, tagged, for `devices` and selection.
pub async fn device_infos() -> Vec<fastpilot_core::DeviceInfo> {
    use fastpilot_core::{DeviceInfo, DeviceKind};
    let mut out: Vec<DeviceInfo> = devices()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, n, _)| n.starts_with("iPhone") || n.starts_with("iPad"))
        .map(|(id, name, state)| DeviceInfo { id, kind: DeviceKind::Simulator, name, state, detail: vec![] })
        .collect();
    let profiles = device::runner_profiles(&device::device_build_dir());
    for d in device::list().await {
        let mut detail = vec![format!("iOS {}", d.os_version), format!("tunnel {}", d.tunnel)];
        match profiles.iter().filter(|p| p.devices.is_empty() || p.devices.contains(&d.udid)).map(device::days_left).min() {
            Some(days) => detail.push(format!("runner profile {days}d left")),
            None => detail.push("runner not built for it (fastpilot ios doctor)".into()),
        }
        out.push(DeviceInfo { id: d.udid, kind: DeviceKind::IosDevice, name: d.name, state: d.pairing, detail });
    }
    out
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
    runner::xctestrun(runner::Target::Simulator)
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
    /// `setLocale`: applied as launch arguments / environment on every launchApp.
    locale: Mutex<(Option<String>, Option<String>)>,
    /// The runner process when this driver started it (kept alive for the next session).
    _runner: Option<std::sync::Arc<tokio::sync::Mutex<Child>>>,
    /// A physical device (xcodebuild + usbmux/tunnel): simctl features are unavailable.
    device: bool,
    /// usbmux port forward to the device's runner, while this session lives.
    forward: Option<Forward>,
    /// This process's claim on the physical device (one session per phone).
    busy: Mutex<Option<Busy>>,
    web: tokio::sync::Mutex<Web>,
}

/// Web Inspector session on the frontmost app's WKWebView (by bundle id); a failed attempt is
/// remembered for 3 s so snapshots of screens without inspectable web content stay fast.
#[derive(Default)]
struct Web {
    session: Option<(String, webinspector::Session)>,
    failed: Option<(std::time::Instant, String, webinspector::Error)>,
}

fn web_error(e: webinspector::Error) -> CoreError {
    match e {
        webinspector::Error::NotInspectable(m) => CoreError::Unsupported(m),
        e => CoreError::Driver(e.to_string()),
    }
}

/// DOM walker for the snapshot: visible elements with canonical roles, names and flags, bounds
/// in layout-viewport CSS px. Elements get a numeric handle (`window.__fastpilot.els`) that
/// taps and typing use. Flags: d disabled, k checked, s selected, e expanded, P pressed,
/// f focused, q required, o readonly, x invalid, p password.
const DOM_JS: &str = r#"(() => {
  const W = window.__fastpilot || (window.__fastpilot = { n: 0, els: new Map() });
  W.els.clear();
  const ref = typeof WeakRef === 'function' ? e => new WeakRef(e) : e => ({ deref: () => e });
  const handle = el => { if (!el.__fpId) el.__fpId = ++W.n; W.els.set(el.__fpId, ref(el)); return el.__fpId; };
  const clean = s => (s || '').replace(/\s+/g, ' ').trim().slice(0, 200);
  const SKIP = /^(SCRIPT|STYLE|NOSCRIPT|TEMPLATE|HEAD|META|LINK|TITLE)$/;
  const ARIA = { textbox: 'textfield', searchbox: 'textfield', img: 'image', menuitemcheckbox: 'menuitem', menuitemradio: 'menuitem', presentation: '', none: '', generic: '' };
  const LEAF = /^(button|link|heading|option|tab|menuitem|textfield|combobox|listbox|image|checkbox|radio|switch|slider|spinbutton)$/;
  let focused = document.activeElement;
  while (focused && focused.shadowRoot && focused.shadowRoot.activeElement) focused = focused.shadowRoot.activeElement;
  let budget = 3000;
  const role = (el, cursor, parentCursor) => {
    const r = (el.getAttribute('role') || '').split(' ')[0];
    if (r) return r in ARIA ? ARIA[r] : r;
    const t = el.tagName, type = (el.getAttribute('type') || '').toLowerCase();
    if (t === 'A') return el.hasAttribute('href') ? 'link' : '';
    if (t === 'BUTTON' || t === 'SUMMARY') return 'button';
    if (t === 'INPUT') {
      if (type === 'hidden') return null;
      if (type === 'checkbox') return 'checkbox';
      if (type === 'radio') return 'radio';
      if (type === 'range') return 'slider';
      if (/^(button|submit|reset|image)$/.test(type)) return 'button';
      return 'textfield';
    }
    if (t === 'TEXTAREA') return 'textfield';
    if (t === 'SELECT') return el.multiple ? 'listbox' : 'combobox';
    if (t === 'OPTION') return 'option';
    if (t === 'IMG') return 'image';
    if (/^H[1-6]$/.test(t)) return 'heading';
    if (t === 'DIALOG') return 'dialog';
    if (el.isContentEditable && !(el.parentElement && el.parentElement.isContentEditable)) return 'textfield';
    if (cursor === 'pointer' && parentCursor !== 'pointer') return 'clickable';
    return '';
  };
  const byIds = ids => ids.split(/\s+/).map(i => { const e = document.getElementById(i); return e ? e.textContent : ''; }).join(' ');
  const name = (el, r) => {
    const a = el.getAttribute('aria-label');
    if (a && a.trim()) return clean(a);
    const lb = el.getAttribute('aria-labelledby');
    if (lb) { const s = clean(byIds(lb)); if (s) return s; }
    if (/^(textfield|combobox|listbox|checkbox|radio|slider|spinbutton|switch)$/.test(r)) {
      const l = el.labels && el.labels[0];
      if (l) { const s = clean(l.innerText); if (s) return s; }
      return clean(el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('name'));
    }
    if (r === 'image') return clean(el.getAttribute('alt') || el.getAttribute('title'));
    if (el.tagName === 'INPUT') return clean(el.value || el.getAttribute('alt'));
    if (/^(button|link|heading|option|tab|menuitem)$/.test(r)) return clean(el.innerText || el.textContent || el.getAttribute('title'));
    return clean(el.getAttribute('title'));
  };
  const flags = el => {
    const aria = k => el.getAttribute('aria-' + k) === 'true';
    let f = '';
    if (el.disabled || aria('disabled')) f += 'd';
    if (el.checked || aria('checked')) f += 'k';
    if (aria('selected') || (el.tagName === 'OPTION' && el.selected)) f += 's';
    if (aria('expanded')) f += 'e';
    if (aria('pressed')) f += 'P';
    if (el === focused) f += 'f';
    if (el.required || aria('required')) f += 'q';
    if (el.readOnly || aria('readonly')) f += 'o';
    if (aria('invalid')) f += 'x';
    if (el.type === 'password') f += 'p';
    return f;
  };
  const kids = n => {
    if (n.shadowRoot) return n.shadowRoot.childNodes;
    if (n.tagName === 'SLOT') { const a = n.assignedNodes({ flatten: true }); return a.length ? a : n.childNodes; }
    return n.childNodes;
  };
  const walk = (node, ox, oy, cursor, visible, out) => {
    for (const c of kids(node)) {
      if (budget <= 0) return;
      if (c.nodeType === 3) {
        const t = clean(c.data);
        if (!t || !visible) continue;
        const rg = document.createRange();
        rg.selectNodeContents(c);
        const b = rg.getBoundingClientRect();
        if (b.width > 0 && b.height > 0) { budget--; out.push({ r: 'text', n: t, b: [b.left + ox, b.top + oy, b.width, b.height] }); }
        continue;
      }
      if (c.nodeType !== 1 || SKIP.test(c.tagName)) continue;
      const el = c, st = getComputedStyle(el);
      if (st.display === 'none' || el.getAttribute('aria-hidden') === 'true') continue;
      const box = el.getBoundingClientRect();
      if (el.tagName === 'IFRAME') {
        try { const d = el.contentDocument; if (d && d.body) walk(d.body, ox + box.left, oy + box.top, 'auto', visible, out); } catch (e) {}
        continue;
      }
      const r = role(el, st.cursor, cursor);
      if (r === null) continue;
      const shown = st.visibility === 'visible' && +st.opacity > 0;
      let n = null;
      if (r && shown && box.width > 0 && box.height > 0) {
        budget--;
        n = { r, n: name(el, r), h: handle(el), b: [box.left + ox, box.top + oy, box.width, box.height], f: flags(el) };
        if (el.id) n.i = el.id; else if (el.getAttribute('data-testid')) n.i = el.getAttribute('data-testid');
        if (r === 'textfield') n.v = 'value' in el ? String(el.value) : clean(el.innerText);
        else if (r === 'combobox' && el.selectedOptions) n.v = clean(el.selectedOptions[0] && el.selectedOptions[0].text);
        out.push(n);
        if (LEAF.test(r)) continue;
        n.c = [];
      }
      walk(el, ox, oy, st.cursor, shown, n ? n.c : out);
      if (n && !n.c.length) delete n.c;
    }
  };
  const out = [];
  walk(document.body || document.documentElement, 0, 0, 'auto', true, out);
  const vv = window.visualViewport;
  return JSON.stringify({ vx: vv ? vv.offsetLeft : 0, vy: vv ? vv.offsetTop : 0, vw: vv ? vv.width : innerWidth, nodes: out });
})()"#;

/// Tap (`el.click()`) or type (`insertText`, else the native value setter plus input/change
/// events, as React expects) on a snapshot handle.
/// ponytail: DOM events, not touches; a page listening only to touchstart needs a native tap.
const ACT_JS: &str = r#"((h, op, text) => {
  const W = window.__fastpilot, w = W && W.els.get(h), el = w && w.deref();
  if (!el || !el.isConnected) return 'gone';
  el.scrollIntoView({ block: 'center', inline: 'center' });
  if (el.disabled) return 'disabled';
  if (el.focus) el.focus({ preventScroll: true });
  if (op === 'tap') { el.click(); return 'ok'; }
  if (document.execCommand('insertText', false, text)) return 'ok';
  if (!('value' in el)) { el.textContent += text; el.dispatchEvent(new Event('input', { bubbles: true })); return 'ok'; }
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, (el.value || '') + text);
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return 'ok';
})"#;

/// DOM walker node -> core node, CSS px mapped into the WKWebView's screen frame.
fn web_node(v: &Value, to_screen: &dyn Fn([f64; 4]) -> Rect) -> Node {
    let f = s(v, "f");
    let has = |c: char| f.contains(c);
    let b = v["b"].as_array().map(|b| [0, 1, 2, 3].map(|i| b.get(i).and_then(Value::as_f64).unwrap_or(0.0)));
    Node {
        role: s(v, "r").to_owned(),
        name: s(v, "n").to_owned(),
        value: v["v"].as_str().filter(|_| !has('p')).map(str::to_owned),
        id: v["i"].as_str().filter(|i| !i.is_empty()).map(str::to_owned),
        secure: has('p'),
        disabled: has('d'),
        checked: has('k'),
        selected: has('s'),
        expanded: has('e'),
        pressed: has('P'),
        invalid: has('x'),
        required: has('q'),
        readonly: has('o'),
        focused: has('f'),
        handle: v["h"].as_i64(),
        bounds: b.map(to_screen),
        children: v["c"].as_array().into_iter().flatten().map(|c| web_node(c, to_screen)).collect(),
    }
}

/// The page's DOM as core nodes placed in `frame` (the WKWebView on screen).
/// ponytail: the content inset (a translucent navigation bar over the web view) is not
/// subtracted; bounds may sit a bar's height high. Actions use handles, not bounds.
async fn dom_nodes(page: &webinspector::Session, frame: Option<Rect>) -> Result<Vec<Node>> {
    let r = page.call("Runtime.evaluate", json!({ "expression": DOM_JS, "returnByValue": true })).await.map_err(web_error)?;
    if r["wasThrown"] == true {
        return Err(CoreError::Driver(format!("DOM snapshot: {}", s(&r["result"], "description"))));
    }
    let v: Value = serde_json::from_str(s(&r["result"], "value")).map_err(|e| CoreError::Driver(format!("DOM snapshot: {e}")))?;
    let f = frame.unwrap_or_default();
    let vw = v["vw"].as_f64().filter(|w| *w > 0.0).unwrap_or(f.w);
    let scale = if vw > 0.0 && f.w > 0.0 { f.w / vw } else { 1.0 };
    let (vx, vy) = (v["vx"].as_f64().unwrap_or(0.0), v["vy"].as_f64().unwrap_or(0.0));
    let to_screen = |[x, y, w, h]: [f64; 4]| Rect { x: f.x + (x - vx) * scale, y: f.y + (y - vy) * scale, w: w * scale, h: h * scale };
    Ok(v["nodes"].as_array().into_iter().flatten().map(|n| web_node(n, &to_screen)).collect())
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

/// UDIDs of paired physical iPhones/iPads (`devicectl`).
pub async fn physical_devices() -> Vec<String> {
    device::list().await.into_iter().map(|d| d.udid).collect()
}

/// 128-bit token for the device runner (first request must carry it).
fn token() -> std::result::Result<String, Error> {
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b))
        .map_err(|e| Error::Runner(format!("random token: {e}")))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// `devicectl device info details`: the CoreDevice tunnel address (IPv6), when connected.
async fn tunnel_address(udid: &str) -> Option<std::net::IpAddr> {
    let v = device::devicectl_json(&["device", "info", "details", "--device", udid]).await.ok()?;
    v["result"]["connectionProperties"]["tunnelIPAddress"].as_str()?.parse().ok()
}

/// Runners started by this process, by simulator: a session ending leaves its runner running
/// (it exits by itself when idle), so the next session on that simulator skips xcodebuild.
fn runners() -> &'static Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<Child>>>> {
    static R: std::sync::OnceLock<Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<Child>>>>> = std::sync::OnceLock::new();
    R.get_or_init(Mutex::default)
}

/// Fixed port per simulator, so a runner can be found again (22100–22599).
fn runner_port(udid: &str) -> u16 {
    let h = udid.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    22100 + (h % 500) as u16
}

/// A usbmux forward that stops when dropped: session end and every failed start alike.
struct Forward(tokio::task::JoinHandle<()>);

impl Drop for Forward {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Physical devices with a session in this process. The device runner's token lives only in
/// its session, so a second session would start another xcodebuild on the same port.
fn busy_devices() -> &'static Mutex<std::collections::HashSet<String>> {
    static B: std::sync::OnceLock<Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    B.get_or_init(Mutex::default)
}

struct Busy(String);

impl Busy {
    fn claim(udid: &str) -> std::result::Result<Self, Error> {
        if !lock(busy_devices()).insert(udid.to_owned()) {
            return Err(Error::Runner(format!("{udid} already has a session in this fastpilot: stop it first (one session per physical device)")));
        }
        Ok(Self(udid.to_owned()))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        lock(busy_devices()).remove(&self.0);
    }
}

impl IosDriver {
    /// Start the XCUITest runner on `udid` (or the booted simulator, booting "iPhone 16" if none).
    /// `udid`: a device spec (`ios-device:<udid>`, `simulator:<udid>`, a plain UDID or name).
    /// A physical device is used only when named; without a spec the booted simulator.
    pub async fn start(udid: Option<&str>) -> std::result::Result<Self, Error> {
        use fastpilot_core::DeviceKind;
        let spec = udid.map(fastpilot_core::DeviceSpec::parse).transpose().map_err(Error::Runner)?;
        let mut target = spec.as_ref().map(|s| s.id.clone());
        if let Some(spec) = &spec {
            // A simulator UDID or name (what the pool leases) needs no devicectl round trip.
            let known_sim = match spec.kind {
                Some(DeviceKind::Simulator) => true,
                Some(_) => false,
                None => devices().await?.iter().any(|(u, n, _)| *u == spec.id || *n == spec.id),
            };
            if !known_sim {
                let all = device_infos().await;
                let d = fastpilot_core::select(&all, Some(spec), fastpilot_core::Platform::Ios).map_err(|e| Error::Runner(e.to_string()))?;
                if d.kind == DeviceKind::IosDevice {
                    return Self::start_device(&d.id).await;
                }
                target = Some(d.id.clone());
            }
        }
        let udid = match target {
            Some(u) => boot(&u).await?,
            None => match devices().await?.into_iter().find(|(_, _, st)| st == "Booted") {
                Some((u, _, _)) => u,
                None => boot(&std::env::var("FASTPILOT_IOS_DEVICE").unwrap_or_else(|_| "iPhone 16".into())).await?,
            },
        };
        let port = runner_port(&udid);
        if let Some(agent) = Agent::try_connect(port).await {
            tracing::info!(udid, port, "reusing the running XCUITest runner");
            let runner = lock(runners()).get(&udid).cloned();
            return Self::connected(udid, std::sync::Arc::new(agent), runner).await;
        }
        let xctestrun = xctestrun().ok_or(Error::Runner("runner not built: run ios-runner/build.sh".into()))?;
        let mut child = Command::new("xcodebuild")
            .arg("test-without-building")
            .arg("-xctestrun")
            .arg(&xctestrun)
            .args(["-destination", &format!("id={udid}")])
            .env("TEST_RUNNER_FASTPILOT_PORT", port.to_string())
            .env("TEST_RUNNER_FASTPILOT_IDLE_EXIT", std::env::var("FASTPILOT_IOS_RUNNER_IDLE").unwrap_or_else(|_| "300".into()))
            .env("TEST_RUNNER_FASTPILOT_QUIESCENCE", std::env::var("FASTPILOT_IOS_QUIESCENCE").unwrap_or_default())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(false)
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
        let child = std::sync::Arc::new(tokio::sync::Mutex::new(child));
        lock(runners()).insert(udid.clone(), child.clone());
        Self::connected(udid, agent, Some(child)).await
    }

    /// Physical device: preflight, the device build of the runner, xcodebuild with a token,
    /// then readiness by polling `hello` (raced against xcodebuild exiting, 240 s) over usbmux
    /// (default; the runner binds loopback) or the CoreDevice tunnel
    /// (`FASTPILOT_IOS_TRANSPORT=tunnel`; the runner binds every interface, token-guarded).
    /// Every early return kills xcodebuild (kill_on_drop) and stops the forward (`Forward`).
    async fn start_device(udid: &str) -> std::result::Result<Self, Error> {
        let claim = Busy::claim(udid)?;
        let facts = device::list().await.into_iter().find(|d| d.udid == udid).ok_or_else(|| Error::Runner(format!("no paired device {udid} (fastpilot ios doctor)")))?;
        let blocking: Vec<String> = device::preflight(&facts, &device::host().await, std::time::SystemTime::now())
            .into_iter()
            .filter(|p| p.blocking)
            .map(|p| format!("{} (fix: {})", p.what, p.fix))
            .collect();
        if !blocking.is_empty() {
            return Err(Error::Runner(format!("{udid} cannot run tests yet: {}", blocking.join("; "))));
        }
        let xctestrun = runner::xctestrun(runner::Target::Device).ok_or_else(|| {
            Error::Runner(format!("runner not built for devices: ios-runner/build.sh --device {udid}"))
        })?;
        let tunnel = std::env::var("FASTPILOT_IOS_TRANSPORT").is_ok_and(|t| t == "tunnel");
        let port = runner_port(udid);
        let token = token()?;
        let env = [
            ("FASTPILOT_PORT".to_owned(), port.to_string()),
            ("FASTPILOT_BIND".to_owned(), if tunnel { "any" } else { "loopback" }.to_owned()),
            ("FASTPILOT_TOKEN".to_owned(), token.clone()),
            ("FASTPILOT_IDLE_EXIT".to_owned(), std::env::var("FASTPILOT_IOS_RUNNER_IDLE").unwrap_or_else(|_| "300".into())),
            ("FASTPILOT_QUIESCENCE".to_owned(), std::env::var("FASTPILOT_IOS_QUIESCENCE").unwrap_or_default()),
        ];
        let mut spawned = runner::spawn(&xctestrun, udid, &env).map_err(|e| Error::Runner(format!("xcodebuild: {e}")))?;
        let started = std::time::Instant::now();
        let mut forward: Option<(u16, Forward)> = None;
        let mut last = "nothing tried yet".to_owned();
        loop {
            if let Ok(Some(status)) = spawned.child.try_wait() {
                let tail = spawned.tail_text();
                let hint = runner::classify(&tail).map(|h| format!(" — {h}")).unwrap_or_default();
                return Err(Error::Runner(format!("xcodebuild exited ({status}) before the runner answered{hint}: {tail}")));
            }
            if started.elapsed() > Duration::from_secs(240) {
                return Err(Error::Runner(format!("runner on {udid} did not answer within 240 s (last: {last}): {}", spawned.tail_text())));
            }
            if !tunnel && forward.is_none() {
                match usbmux::forward(udid, port).await {
                    Ok((local, h)) => forward = Some((local, Forward(h))),
                    Err(e) => last = e.to_string(),
                }
            }
            if let Some((local, _)) = &forward {
                let ep = fastpilot_android::Endpoint { token: Some(token.clone()), ..fastpilot_android::Endpoint::local(*local) };
                match Agent::connect_to(&ep, 1).await {
                    Ok(agent) => {
                        tracing::info!(udid, "runner reached over usbmux");
                        return Self::on_device(udid, agent, spawned.child, forward.map(|(_, f)| f), claim).await;
                    }
                    Err(e) => last = format!("runner over usbmux: {e}"),
                }
            }
            if tunnel {
                match tunnel_address(udid).await {
                    Some(addr) => {
                        let ep = fastpilot_android::Endpoint { host: addr, port, token: Some(token.clone()) };
                        match Agent::connect_to(&ep, 1).await {
                            Ok(agent) => {
                                tracing::info!(udid, %addr, "runner reached over the CoreDevice tunnel");
                                return Self::on_device(udid, agent, spawned.child, None, claim).await;
                            }
                            Err(e) => last = format!("runner over the tunnel ({addr}): {e}"),
                        }
                    }
                    None => last = "no CoreDevice tunnel address yet (devicectl device info details)".into(),
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// The child stays out of `runners()`: a device runner is not reused (its token dies with
    /// the session); dropping the driver kills it.
    async fn on_device(udid: &str, agent: Agent, child: Child, forward: Option<Forward>, claim: Busy) -> std::result::Result<Self, Error> {
        let child = std::sync::Arc::new(tokio::sync::Mutex::new(child));
        let mut d = Self::connected_as(udid.to_owned(), std::sync::Arc::new(agent), Some(child), true).await?;
        d.forward = forward;
        *lock(&d.busy) = Some(claim);
        Ok(d)
    }

    async fn connected(
        udid: String,
        agent: std::sync::Arc<Agent>,
        runner: Option<std::sync::Arc<tokio::sync::Mutex<Child>>>,
    ) -> std::result::Result<Self, Error> {
        Self::connected_as(udid, agent, runner, false).await
    }

    async fn connected_as(
        udid: String,
        agent: std::sync::Arc<Agent>,
        runner: Option<std::sync::Arc<tokio::sync::Mutex<Child>>>,
        device: bool,
    ) -> std::result::Result<Self, Error> {
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
            locale: Mutex::default(),
            _runner: runner,
            device,
            forward: None,
            busy: Mutex::new(None),
            web: tokio::sync::Mutex::default(),
        })
    }

    /// Device `clearState`: terminate, empty the app's data container with devicectl (works for
    /// development-signed apps only), reset its permissions.
    async fn clear_device_state(&self, app: Option<&str>) -> Result<()> {
        let bundle = self.app_or(app)?;
        self.call("terminate", json!({ "bundleId": bundle })).await?;
        let empty = std::env::temp_dir().join(format!("fastpilot-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).map_err(|e| CoreError::Driver(e.to_string()))?;
        for dest in ["Documents", "Library", "tmp"] {
            let out = Command::new("xcrun")
                .args(["devicectl", "device", "copy", "to", "--device", &self.udid, "--domain-type", "appDataContainer", "--domain-identifier", &bundle])
                .arg("--source")
                .arg(&empty)
                .args(["--destination", dest, "--remove-existing-content", "true"])
                .output()
                .await
                .map_err(|e| CoreError::Driver(format!("devicectl: {e}")))?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                return Err(CoreError::Unsupported(format!(
                    "clearState {bundle} on a device needs a development-signed app (get-task-allow): {}",
                    err.trim()
                )));
            }
        }
        let _ = std::fs::remove_dir_all(&empty);
        self.call("resetPermissions", json!({ "bundleId": bundle })).await?;
        self.forget(&bundle);
        Ok(())
    }

    /// Simulator-only features (simctl) on a real device.
    fn sim_only(&self, what: &str) -> Result<()> {
        if self.device {
            return Err(CoreError::Unsupported(format!("{what} needs the iOS simulator (simctl); not available on a real device")));
        }
        Ok(())
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
        let mut roots: Vec<Node> = v["nodes"].as_array().into_iter().flatten().filter_map(convert).collect();
        if let Some(wv) = find_webview(&mut roots) {
            // Web nodes keep their DOM handles: taps and typing on them go through the inspector.
            let mut web = self.web.lock().await;
            if let Ok(page) = self.webview(&bundle, &mut web).await {
                match dom_nodes(page, wv.bounds).await {
                    Ok(nodes) if !nodes.is_empty() => wv.children = nodes,
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!(error = %e, "WKWebView snapshot failed; native accessibility tree kept");
                        web.session = None;
                    }
                }
            }
        }
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

    /// Inspector session on `bundle`'s WKWebView (cached; reopened after it closed).
    async fn webview<'a>(&self, bundle: &str, web: &'a mut Web) -> Result<&'a webinspector::Session> {
        if !web.session.as_ref().is_some_and(|(b, s)| b == bundle && s.alive()) {
            web.session = None;
            if let Some((at, b, e)) = &web.failed
                && b == bundle
                && at.elapsed() < Duration::from_secs(3)
            {
                return Err(web_error(e.clone()));
            }
            let io = if self.device { webinspector::device_stream(&self.udid).await } else { webinspector::simulator_stream(&self.udid).await };
            match async { webinspector::Session::open(io?, bundle).await }.await {
                Ok(s) => {
                    web.session = Some((bundle.to_owned(), s));
                    web.failed = None;
                }
                Err(e) => {
                    web.failed = Some((std::time::Instant::now(), bundle.to_owned(), e.clone()));
                    return Err(web_error(e));
                }
            }
        }
        web.session.as_ref().map(|(_, s)| s).ok_or_else(|| CoreError::Unsupported("no WKWebView session".into()))
    }

    /// The inspector session in use, else one on the app under test (or the frontmost app).
    async fn web_page(&self) -> Result<tokio::sync::MappedMutexGuard<'_, webinspector::Session>> {
        let mut web = self.web.lock().await;
        if !web.session.as_ref().is_some_and(|(_, s)| s.alive()) {
            let mut bundle = lock(&self.bundle).clone();
            if bundle.is_empty() {
                bundle = s(&self.call("snapshot", json!({})).await?, "bundleId").to_owned();
            }
            self.webview(&bundle, &mut web).await?;
        }
        tokio::sync::MutexGuard::try_map(web, |w| w.session.as_mut().map(|(_, s)| s))
            .map_err(|_| CoreError::Unsupported("no WKWebView session".into()))
    }

    async fn web_act(&self, handle: i64, op: &str, text: &str) -> Result<()> {
        let js = format!("{ACT_JS}({handle}, {}, {})", json!(op), json!(text));
        let page = self.web_page().await?;
        let r = page.call("Runtime.evaluate", json!({ "expression": js, "returnByValue": true, "emulateUserGesture": true })).await.map_err(web_error)?;
        match s(&r["result"], "value") {
            "ok" => Ok(()),
            "disabled" => Err(CoreError::NotActionable("disabled".into())),
            "gone" => Err(CoreError::NotFound("the web element is gone (the page changed); take a new snapshot".into())),
            _ => Err(CoreError::Script(s(&r["result"], "description").to_owned())),
        }
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

// No `quit` on drop: the runner stays up for the next session and exits when idle
// (FASTPILOT_IOS_RUNNER_IDLE seconds, default 300).

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
        "webView" => "webview",
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
            if self.device {
                // Runner first (iOS 16.4+); else devicectl opens it in Safari or the app.
                if self.call("openUrl", json!({ "url": target })).await.is_err() {
                    let app = if target.starts_with("http") { "com.apple.mobilesafari".to_owned() } else { lock(&self.bundle).clone() };
                    let out = Command::new("xcrun")
                        .args(["devicectl", "device", "process", "launch", "--device", &self.udid, "--payload-url", target, &app])
                        .output()
                        .await
                        .map_err(|e| CoreError::Driver(format!("devicectl: {e}")))?;
                    if !out.status.success() {
                        return Err(CoreError::Driver(format!("open {target}: {}", String::from_utf8_lossy(&out.stderr).trim())));
                    }
                }
            } else {
                simctl(&["openurl", &self.udid, target]).await?;
            }
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

    /// Physical device: quit the runner, stop xcodebuild and the usbmux forward, free the
    /// device. A simulator runner stays up for the next session (it exits when idle).
    async fn close(&self) -> Result<()> {
        if !self.device {
            return Ok(());
        }
        let _ = tokio::time::timeout(Duration::from_secs(3), self.agent.call("quit", json!({}))).await;
        if let Some(f) = &self.forward {
            f.0.abort();
        }
        if let Some(r) = &self._runner {
            let mut child = r.lock().await;
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
        }
        lock(&self.busy).take();
        Ok(())
    }

    async fn set_location(&self, lat: f64, lon: f64) -> Result<()> {
        if self.device {
            return Err(CoreError::Unsupported(
                "setLocation on a physical iPhone: use Xcode's location simulation (Debug → Simulate Location); simctl location is simulator-only".into(),
            ));
        } else {
            simctl(&["location", &self.udid, "set", &format!("{lat},{lon}")]).await?;
        }
        Ok(())
    }

    /// `simctl privacy` (TCC services; notifications are not among them).
    async fn set_permissions(&self, app: Option<&str>, perms: &[(String, fastpilot_core::Grant)]) -> Result<()> {
        if self.device {
            // A device has no simctl privacy: only "unset" (back to asking) is possible.
            let bundle = self.app_or(app)?;
            let mut resources = Vec::new();
            for (name, grant) in perms {
                if *grant != fastpilot_core::Grant::Unset {
                    return Err(CoreError::Unsupported(format!(
                        "setPermissions {name}: allow/deny needs the simulator; on a device use `unset` and answer the prompt (or let FASTPILOT_PERMISSIONS do it)"
                    )));
                }
                resources.push(name.clone());
            }
            self.call("resetPermissions", json!({ "bundleId": bundle, "resources": resources })).await?;
            return Ok(());
        }
        use fastpilot_core::Grant;
        let bundle = self.app_or(app)?;
        for (name, grant) in perms {
            let service = match name.as_str() {
                "notifications" => return Err(CoreError::Unsupported("notification permission cannot be set on the simulator (simctl privacy has no such service)".into())),
                "location" => "location",
                other => other,
            };
            let action = match grant {
                Grant::Allow => "grant",
                Grant::Deny => "revoke",
                Grant::Unset => "reset",
            };
            simctl(&["privacy", &self.udid, action, service, &bundle]).await.map_err(|e| CoreError::Flow(format!("permission `{name}`: {e}")))?;
        }
        Ok(())
    }

    async fn set_network(&self, _n: &fastpilot_core::Network) -> Result<()> {
        Err(CoreError::Unsupported(
            "the iOS simulator uses the Mac's network: throttle with Network Link Conditioner, go offline on the host".into(),
        ))
    }

    /// Takes effect from the next launch; the app under test is relaunched now.
    async fn set_locale(&self, locale: Option<&str>, timezone: Option<&str>) -> Result<()> {
        {
            let mut l = lock(&self.locale);
            if let Some(x) = locale {
                l.0 = Some(x.to_owned());
            }
            if let Some(x) = timezone {
                l.1 = Some(x.to_owned());
            }
        }
        let bundle = lock(&self.bundle).clone();
        if !bundle.is_empty() {
            self.launch_app(&bundle).await?;
        }
        Ok(())
    }

    /// `simctl push`: an APNs payload delivered to the app as a remote notification.
    async fn push_notification(&self, app: Option<&str>, title: &str, body: &str, payload: Option<&Value>) -> Result<()> {
        self.sim_only("pushNotification")?;
        let bundle = self.app_or(app)?;
        let mut p = payload.cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
        p["aps"]["alert"] = json!({ "title": title, "body": body });
        let path = std::env::temp_dir().join(format!("fastpilot-push-{}.json", std::process::id()));
        std::fs::write(&path, p.to_string()).map_err(|e| CoreError::Driver(e.to_string()))?;
        let res = simctl(&["push", &self.udid, &bundle, &path.display().to_string()]).await;
        let _ = std::fs::remove_file(&path);
        res?;
        Ok(())
    }

    async fn start_recording(&self) -> Result<()> {
        self.sim_only("recording")?;
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
        if self.device {
            return self.clear_device_state(app).await;
        }
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
        if self.device {
            return Err(CoreError::Unsupported(
                "iOS has no API to clear the keychain on a device: have the app clear it at launch (a test flag) or log out through the UI".into(),
            ));
        }
        simctl(&["keychain", &self.udid, "reset"]).await?;
        Ok(())
    }

    async fn tap(&self, sel: &Selector) -> Result<()> {
        if let Selector::Point(p) = sel {
            return self.tap_at(*p).await;
        }
        let node = self.resolve(sel, Intent::Tap).await?;
        if let Some(h) = node.handle {
            return self.web_act(h, "tap", "").await;
        }
        self.tap_at(Self::center(&node)?).await
    }

    async fn type_text(&self, sel: &Selector, text: &str) -> Result<()> {
        match sel {
            Selector::Focused => {}
            Selector::Point(p) => self.tap_at(*p).await?,
            _ => {
                let node = self.resolve(sel, Intent::Type).await?;
                if let Some(h) = node.handle {
                    return self.web_act(h, "type", text).await;
                }
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

    /// The runner waits for two identical snapshots (XCTest's own animation wait is off).
    async fn wait_idle(&self, timeout: Duration) -> Result<()> {
        let cap = timeout.min(Duration::from_millis(1500)).as_millis() as u64;
        let v = self.call("idle", json!({ "timeoutMs": cap })).await?;
        if v["idle"].as_bool() == Some(false) {
            return Err(CoreError::Timeout(format!("screen not idle after {cap}ms")));
        }
        Ok(())
    }

    /// In the WKWebView, through the Web Inspector: by reference first, so a Promise is awaited
    /// and any other object returned by value (as the web driver does).
    async fn eval_script(&self, js: &str) -> Result<Value> {
        let guard = self.web_page().await?;
        let page: &webinspector::Session = &guard;
        let call = |m: &'static str, p: Value| async move { page.call(m, p).await.map_err(web_error) };
        let mut r = call("Runtime.evaluate", json!({ "expression": js, "objectGroup": "fastpilot", "emulateUserGesture": true })).await?;
        if r["wasThrown"] != true
            && let Some(id) = r["result"]["objectId"].as_str().map(str::to_owned)
        {
            r = if r["result"]["className"] == "Promise" || r["result"]["subtype"] == "promise" {
                call("Runtime.awaitPromise", json!({ "promiseObjectId": id, "returnByValue": true })).await?
            } else {
                call("Runtime.callFunctionOn", json!({ "objectId": id, "functionDeclaration": "function() { return this; }", "returnByValue": true })).await?
            };
            let _ = call("Runtime.releaseObjectGroup", json!({ "objectGroup": "fastpilot" })).await;
        }
        if r["wasThrown"] == true {
            let msg = r["result"]["description"].as_str().unwrap_or("script error");
            return Err(CoreError::Script(msg.to_owned()));
        }
        Ok(r["result"]["value"].take())
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

    /// Maestro semantics: terminate and relaunch (with the `setLocale` launch settings).
    async fn launch_app(&self, app: &str) -> Result<()> {
        let (locale, tz) = lock(&self.locale).clone();
        let mut args = Vec::new();
        if let Some(l) = &locale {
            let lang = l.split(['-', '_']).next().unwrap_or(l);
            args.extend(["-AppleLanguages".to_owned(), format!("({lang})"), "-AppleLocale".to_owned(), l.replace('-', "_")]);
        }
        let env: serde_json::Map<String, Value> = tz.into_iter().map(|t| ("TZ".to_owned(), json!(t))).collect();
        self.call("launch", json!({ "bundleId": app, "args": args, "env": env })).await?;
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

    #[test]
    fn one_session_per_physical_device() {
        let first = Busy::claim("00008110-TEST").expect("free");
        assert!(Busy::claim("00008110-TEST").is_err(), "a second session is refused");
        assert!(Busy::claim("00008110-OTHER").is_ok());
        drop(first);
        assert!(Busy::claim("00008110-TEST").is_ok(), "freed when the session ends");
    }

    #[test]
    fn maps_dom_nodes_into_the_webview_frame() {
        let v = json!({ "r": "textfield", "n": "Kata sandi", "v": "rahasia", "h": 7, "f": "pfq", "b": [10, 20, 100, 30],
                        "c": [{ "r": "text", "n": "x", "b": [0, 0, 1, 1] }] });
        let frame = Rect { x: 0.0, y: 100.0, w: 390.0, h: 600.0 };
        // Page zoomed out to a 780 px wide layout: half scale.
        let to_screen = |[x, y, w, h]: [f64; 4]| Rect { x: frame.x + x * 0.5, y: frame.y + y * 0.5, w: w * 0.5, h: h * 0.5 };
        let n = web_node(&v, &to_screen);
        assert_eq!((n.role.as_str(), n.handle, n.value.as_deref()), ("textfield", Some(7), None), "password values stay out");
        assert!(n.secure && n.focused && n.required && !n.disabled);
        assert_eq!(n.bounds, Some(Rect { x: 5.0, y: 110.0, w: 50.0, h: 15.0 }));
        assert_eq!(n.children.len(), 1);
        let mut roots = vec![convert(&json!({ "type": "other", "frame": [0, 0, 390, 844], "children": [{ "type": "webView", "frame": [0, 100, 390, 600] }] })).expect("node")];
        assert_eq!(find_webview(&mut roots).and_then(|w| w.bounds), Some(frame));
    }
}
