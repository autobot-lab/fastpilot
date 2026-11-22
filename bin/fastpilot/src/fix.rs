//! D2 (v0.4): the Fix buttons of the Device page. fastpilot sets up what its tests need, only after
//! a person clicks and confirms on that page (never from Chat or an MCP tool: nothing else can
//! start a fix), and shows the steps and a live log while it works.
//!
//! - **Chrome**: install Google Chrome with Homebrew (macOS, when Chrome is missing).
//! - **Android** (v0.4.1, DV1: what is installed comes first): when no device runs, the emulators
//!   already on this computer are listed, each with **Start** (it starts that one on a free port,
//!   never 5554, and waits until it has booted; its saved state is not changed). Adding
//!   fastpilot's own test phone, `fastpilot_api35` (download the Android tools and an Android 15
//!   image when missing, create the phone, start it without a window), is the second choice, or
//!   the only one when no emulator is installed. A running device is used as it is. Nothing is
//!   ever stopped or sent to another emulator or phone.
//! - **iPhone**: the simulators already installed are listed, and the person picks the one tests
//!   use (`ios_udid`). The test runner is a one-time setup (`fastpilot setup ios`, with its log).
//!
//! The plan is made on the server and made again when the person confirms; the page only names
//! the area. One fix runs at a time; Stop ends it.

use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

pub const AVD: &str = "fastpilot_api35";
const LOG_LINES: usize = 300;

fn abi() -> &'static str {
    if cfg!(target_arch = "aarch64") { "arm64-v8a" } else { "x86_64" }
}

fn image() -> String {
    format!("system-images;android-35;google_apis;{}", abi())
}

/// `program` in a PATH folder. Only absolute folders count: `.` or an empty entry would run a
/// `brew` or `sdkmanager` that a cloned project put in its own folder (Security re-review, Low).
fn on_path(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).filter(|d| d.is_absolute()).map(|d| d.join(program)).find(|f| f.is_file()))
}

/// A tool of the Android SDK, or `None` when it is not installed.
fn sdk(rel: &str, name: &str) -> Option<PathBuf> {
    let p = fastpilot_android::sdk_tool(rel, name);
    let p = PathBuf::from(&p);
    if p.is_absolute() { p.is_file().then_some(p) } else { on_path(name) }
}

/// Where the SDK is (or will be): ANDROID_HOME / ANDROID_SDK_ROOT, else the usual place.
fn sdk_root() -> PathBuf {
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(v) = std::env::var_os(var).filter(|v| !v.is_empty()) {
            return v.into();
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if cfg!(target_os = "macos") { home.join("Library/Android/sdk") } else { home.join("Android/Sdk") }
}

/// sdkmanager / avdmanager: in the SDK, else where Homebrew's command-line tools put them.
fn cmdline_tool(name: &str) -> Option<PathBuf> {
    sdk(&format!("cmdline-tools/latest/bin/{name}"), name).or_else(|| {
        ["/opt/homebrew", "/usr/local"].iter().map(|p| Path::new(p).join("share/android-commandlinetools/cmdline-tools/latest/bin").join(name)).find(|p| p.is_file())
    })
}

#[derive(Debug, Clone)]
enum Action {
    /// Run a program; `answer` is typed on every question it asks (licences).
    Run { program: String, args: Vec<String>, answer: Option<&'static str>, minutes: u64 },
    /// A command-line tool that may only exist after an earlier step.
    Tool { name: &'static str, args: Vec<String>, answer: Option<&'static str>, minutes: u64 },
    /// Start this emulator (an AVD on this computer) and wait until it has booted.
    StartAvd(String),
}

#[derive(Debug, Clone)]
struct Step {
    label: String,
    action: Action,
}

#[derive(Debug, Clone)]
struct Plan {
    area: &'static str,
    title: String,
    confirm: String,
    button: &'static str,
    steps: Vec<Step>,
    /// What the page says while it runs, and when it is done.
    doing: String,
    done: String,
}

/// One thing to pick in an offer: an installed emulator to start, or a simulator for tests.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Choice {
    pub id: String,
    pub name: String,
    /// What the button does: `start` an installed emulator, `use` a device or simulator for tests.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub act: &'static str,
    /// `running`, `in use for tests`, … (`None`: nothing to say).
    pub note: Option<String>,
    /// The button; `None` when there is nothing to do (it is already the one in use).
    pub button: Option<&'static str>,
    /// Shown before it starts (emulators); `None`: no confirmation needed (picking a simulator).
    pub confirm: Option<String>,
}

/// What the page shows for an area: a fix fastpilot can run, or what the person can do.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Offer {
    pub area: &'static str,
    pub title: String,
    pub detail: String,
    /// The button; `None` when fastpilot cannot do it (the detail says how).
    pub button: Option<&'static str>,
    pub confirm: Option<String>,
    pub steps: Vec<String>,
    /// Installed emulators / simulators to pick from (DV1).
    pub choices: Vec<Choice>,
    /// The button is the second choice (the list comes first).
    pub secondary: bool,
    /// Nothing is missing; the offer is only there to pick from.
    pub ready: bool,
}

impl Offer {
    fn from_plan(p: &Plan, detail: impl Into<String>) -> Self {
        Offer {
            area: p.area,
            title: p.title.clone(),
            detail: detail.into(),
            button: Some(p.button),
            confirm: Some(p.confirm.clone()),
            steps: p.steps.iter().map(|s| s.label.clone()).collect(),
            choices: vec![],
            secondary: false,
            ready: false,
        }
    }
    fn note(area: &'static str, title: &str, detail: impl Into<String>) -> Self {
        Offer { area, title: title.into(), detail: detail.into(), button: None, confirm: None, steps: vec![], choices: vec![], secondary: false, ready: false }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StepView {
    pub label: String,
    /// `waiting`, `running`, `done`, `failed` or `stopped`.
    pub state: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Job {
    pub area: &'static str,
    pub title: String,
    /// What the page says while it runs.
    pub doing: String,
    /// `running`, `done`, `failed` or `stopped`.
    pub state: &'static str,
    pub steps: Vec<StepView>,
    pub log: Vec<String>,
    pub message: Option<String>,
}

struct Running {
    job: Job,
    stop: std::sync::Arc<tokio::sync::Notify>,
    stopping: bool,
}

static JOB: LazyLock<Mutex<Option<Running>>> = LazyLock::new(|| Mutex::new(None));

fn job() -> std::sync::MutexGuard<'static, Option<Running>> {
    JOB.lock().unwrap_or_else(|p| p.into_inner())
}

fn log(line: &str) {
    let line = fastpilot_core::redact(line.trim_end());
    if line.is_empty() {
        return;
    }
    if let Some(r) = job().as_mut() {
        r.job.log.push(line);
        let extra = r.job.log.len().saturating_sub(LOG_LINES);
        r.job.log.drain(..extra);
    }
}

fn set_step(i: usize, state: &'static str) {
    if let Some(r) = job().as_mut()
        && let Some(s) = r.job.steps.get_mut(i)
    {
        s.state = state;
    }
}

/// `adb devices` through the SDK's adb (the same server the tests use).
async fn adb_devices(adb: &Path) -> Vec<(String, String)> {
    let out = tokio::time::timeout(Duration::from_secs(20), tokio::process::Command::new(adb).arg("devices").stdin(std::process::Stdio::null()).output()).await;
    let Ok(Ok(o)) = out else { return vec![] };
    String::from_utf8_lossy(&o.stdout).lines().skip(1).filter_map(|l| l.split_once('\t').map(|(s, st)| (s.trim().to_owned(), st.trim().to_owned()))).collect()
}

/// Where fastpilot keeps the port its test phone was started on (in its own folder).
fn port_file() -> PathBuf {
    fastpilot_core::config::path().parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir).join("android-test-phone.port")
}

/// The test phone, if it runs: only the port fastpilot started it on is asked (`emu avd name`);
/// other emulators and phones are never contacted.
async fn running_phone(adb: &Path) -> Option<String> {
    let port: u16 = std::fs::read_to_string(port_file()).ok()?.trim().parse().ok().filter(|p| *p != 5554)?;
    let serial = format!("emulator-{port}");
    if !adb_devices(adb).await.iter().any(|(s, st)| s == &serial && st == "device") {
        return None;
    }
    let o = tokio::process::Command::new(adb).args(["-s", &serial, "emu", "avd", "name"]).stdin(std::process::Stdio::null()).output();
    match tokio::time::timeout(Duration::from_secs(10), o).await {
        Ok(Ok(o)) if String::from_utf8_lossy(&o.stdout).lines().next().map(str::trim) == Some(AVD) => Some(serial),
        _ => None,
    }
}

async fn chrome_offer() -> Option<Offer> {
    if fastpilot_web::chrome_endpoint().is_some() || fastpilot_web::find_chrome().is_some() {
        return None;
    }
    Some(match chrome_plan() {
        Some(p) => Offer::from_plan(&p, "Not installed. Needed for web tests."),
        None => Offer::note("chrome", "Chrome", "Not installed. Download it from google.com/chrome, then press Check again."),
    })
}

fn chrome_plan() -> Option<Plan> {
    let brew = on_path("brew").filter(|_| cfg!(target_os = "macos"))?;
    Some(Plan {
        area: "chrome",
        title: "Chrome".into(),
        confirm: "fastpilot will install Google Chrome with Homebrew. Homebrew downloads it (a few hundred MB); it takes a few minutes. Nothing else is changed.".into(),
        button: "Install Chrome",
        steps: vec![Step {
            label: "Install Google Chrome".into(),
            action: Action::Run { program: brew.to_string_lossy().into_owned(), args: ["install", "--cask", "google-chrome"].map(String::from).to_vec(), answer: None, minutes: 30 },
        }],
        doing: "Installing Chrome".into(),
        done: "Chrome is installed. Web tests can use it now.".into(),
    })
}

/// The Android steps still needed for fastpilot's test phone; `Ok(None)`: it is running.
async fn android_plan() -> Result<Option<Plan>, String> {
    if fastpilot_android::configured_server().is_some() {
        return Err("Android devices come from another computer (FASTPILOT_ADB_SERVER); set up the emulator there.".into());
    }
    let root = sdk_root();
    let mut steps = Vec::new();
    let sdkmanager = cmdline_tool("sdkmanager");
    if sdkmanager.is_none() {
        let Some(brew) = on_path("brew").filter(|_| cfg!(target_os = "macos")) else {
            return Err("The Android tools are not installed. Install Android Studio from developer.android.com/studio, then press Check again.".into());
        };
        steps.push(Step {
            label: "Download the Android tools".into(),
            action: Action::Run { program: brew.to_string_lossy().into_owned(), args: ["install", "--cask", "android-commandlinetools"].map(String::from).to_vec(), answer: None, minutes: 30 },
        });
    }
    let image_dir = root.join("system-images/android-35/google_apis").join(abi());
    let emulator = sdk("emulator/emulator", "emulator");
    let adb = sdk("platform-tools/adb", "adb");
    let download = !image_dir.is_dir() || emulator.is_none() || adb.is_none();
    if download {
        steps.push(Step {
            label: "Download Android 15 for the test phone".into(),
            action: Action::Tool {
                name: "sdkmanager",
                args: vec![format!("--sdk_root={}", root.display()), "--install".into(), "platform-tools".into(), "emulator".into(), image()],
                answer: Some("y"),
                minutes: 90,
            },
        });
    }
    let exists = match &emulator {
        Some(e) => tokio::process::Command::new(e).arg("-list-avds").stdin(std::process::Stdio::null()).output().await.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim() == AVD)),
        None => false,
    };
    if !exists {
        steps.push(Step {
            label: "Create the test phone".into(),
            action: Action::Tool { name: "avdmanager", args: ["create", "avd", "-n", AVD, "-k", &image(), "-d", "pixel_6"].map(String::from).to_vec(), answer: Some("no"), minutes: 10 },
        });
    }
    if steps.is_empty()
        && let Some(adb) = &adb
        && running_phone(adb).await.is_some()
    {
        return Ok(None);
    }
    steps.push(Step { label: "Start it and check it works".into(), action: Action::StartAvd(AVD.into()) });
    let size = if download { "download about 1.5 GB and " } else { "" };
    let licence = if download { " Downloading Android means accepting Google's Android SDK licence." } else { "" };
    let time = if download { "about 10 minutes" } else { "a few minutes" };
    Ok(Some(Plan {
        area: "android",
        title: "Android emulator".into(),
        confirm: format!(
            "fastpilot will {size}{} a test phone called \"{AVD}\". It takes {time}. Your own phones and emulators are not touched.{licence}",
            if exists { "start" } else { "create and start" }
        ),
        button: if exists { "Start the test phone" } else { "Set up an emulator" },
        steps,
        doing: "Setting up your emulator".into(),
        done: "The test phone is running. Android tests can use it now.".into(),
    }))
}

/// An AVD name as `emulator -list-avds` prints it; anything else is never passed on.
fn avd_name_ok(n: &str) -> bool {
    !n.is_empty() && n.len() <= 100 && n.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// The emulators (AVDs) on this computer, by name. Only lists them: nothing is started.
async fn installed_avds(emulator: &Path) -> Vec<String> {
    let o = tokio::process::Command::new(emulator).arg("-list-avds").stdin(std::process::Stdio::null()).output();
    let Ok(Ok(o)) = tokio::time::timeout(Duration::from_secs(20), o).await else { return vec![] };
    String::from_utf8_lossy(&o.stdout).lines().map(str::trim).filter(|l| avd_name_ok(l)).map(str::to_owned).collect()
}

/// A device is running (an emulator or a phone): tests use it, there is nothing to start.
async fn device_running(adb: Option<&Path>) -> bool {
    match adb {
        Some(adb) => adb_devices(adb).await.iter().any(|(_, st)| st == "device"),
        None => false,
    }
}

/// Start one of the person's own emulators (DV1).
async fn avd_plan(name: &str) -> Result<Plan, String> {
    if fastpilot_android::configured_server().is_some() {
        return Err("Android devices come from another computer (FASTPILOT_ADB_SERVER); start the emulator there.".into());
    }
    let adb = sdk("platform-tools/adb", "adb").ok_or("adb is not installed")?;
    let emulator = sdk("emulator/emulator", "emulator").ok_or("the Android emulator is not installed")?;
    if device_running(Some(&adb)).await {
        return Err("An Android device is already running; tests use it. Nothing was started.".into());
    }
    if !avd_name_ok(name) || !installed_avds(&emulator).await.iter().any(|a| a == name) {
        return Err(format!("There is no emulator called \"{name}\" on this computer."));
    }
    Ok(Plan {
        area: "android",
        title: format!("Android emulator {name}"),
        confirm: avd_confirm(name),
        button: "Start",
        steps: vec![Step { label: format!("Start {name} and check it works"), action: Action::StartAvd(name.into()) }],
        doing: format!("Starting {name}"),
        done: format!("{name} is running. Android tests use it now."),
    })
}

fn avd_confirm(name: &str) -> String {
    if name == AVD {
        return format!("fastpilot will start its test phone \"{AVD}\". It takes a few minutes. Your own phones and emulators are not touched.");
    }
    format!("fastpilot will start your emulator \"{name}\" on a free port and wait until it is ready (usually 1–2 minutes). Its saved state is not changed, and other emulators and phones are not touched.")
}

async fn android_offer() -> Option<Offer> {
    if let Some(server) = fastpilot_android::configured_server() {
        let why = server.err().unwrap_or_else(|| "Android devices come from another computer (FASTPILOT_ADB_SERVER); start the emulator there.".into());
        return Some(Offer::note("android", "Android emulator", why));
    }
    let adb = sdk("platform-tools/adb", "adb");
    if device_running(adb.as_deref()).await {
        return None;
    }
    let avds = match sdk("emulator/emulator", "emulator") {
        Some(e) => installed_avds(&e).await,
        None => vec![],
    };
    let plan = android_plan().await;
    if avds.is_empty() {
        return match plan {
            Ok(Some(p)) => Some(Offer::from_plan(&p, "No emulator is installed yet. Needed for Android tests on this computer.")),
            Ok(None) => None,
            Err(why) => Some(Offer::note("android", "Android emulator", why)),
        };
    }
    // DV1: the emulators already here first; a new test phone only as the second choice.
    let choices = avds.iter().map(|a| Choice { id: a.clone(), name: a.clone(), act: "start", note: (a == AVD).then(|| "fastpilot's test phone".into()), button: Some("Start"), confirm: Some(avd_confirm(a)) }).collect();
    let mut offer = Offer::note("android", "Android emulator", "No emulator is running. Start one of the emulators on this computer; tests use it.");
    offer.choices = choices;
    if let Ok(Some(p)) = plan.as_ref()
        && !avds.iter().any(|a| a == AVD)
    {
        offer.button = Some("Add a new test phone");
        offer.confirm = Some(p.confirm.clone());
        offer.steps = p.steps.iter().map(|s| s.label.clone()).collect();
        offer.secondary = true;
    }
    Some(offer)
}

/// iPhone simulators on this computer (`simctl`), and the runner as a one-time setup (DV1).
async fn ios_offer() -> Option<Offer> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let sims: Vec<(String, String, String)> = fastpilot_ios::devices().await.ok()?.into_iter().filter(|(_, n, _)| n.starts_with("iPhone") || n.starts_with("iPad")).collect();
    let chosen = fastpilot_core::config::var("FASTPILOT_IOS_UDID").ok().filter(|u| !u.trim().is_empty());
    let booted = sims.iter().find(|(_, _, st)| st == "Booted").map(|(u, _, _)| u.clone());
    let in_use = chosen.clone().filter(|c| sims.iter().any(|(u, _, _)| u == c)).or(booted);
    let mut choices: Vec<Choice> = sims
        .iter()
        .map(|(udid, name, st)| {
            let mut notes = Vec::new();
            if st == "Booted" {
                notes.push("running");
            }
            if in_use.as_deref() == Some(udid.as_str()) {
                notes.push(if chosen.is_some() { "you picked it for tests" } else { "tests use it" });
            }
            // DC4: the simulator already in use has no "Use for tests".
            let picked = chosen.as_deref() == Some(udid.as_str()) || in_use.as_deref() == Some(udid.as_str());
            Choice { id: udid.clone(), name: name.clone(), act: "use", note: (!notes.is_empty()).then(|| notes.join(" · ")), button: (!picked).then_some("Use for tests"), confirm: None }
        })
        .collect();
    if chosen.is_some() {
        choices.push(Choice { id: String::new(), name: "Any running simulator".into(), act: "use", note: None, button: Some("Use for tests"), confirm: None });
    }
    let runner = fastpilot_ios::runner::xctestrun(fastpilot_ios::runner::Target::Simulator).is_some();
    let mut offer = match ios_plan().filter(|_| !runner) {
        Some(p) => Offer::from_plan(&p, IOS_SETUP),
        None if sims.is_empty() => Offer::note("ios", "iPhone simulator", "No iPhone simulator is installed. Xcode › Settings › Components: install an iOS simulator."),
        None if runner => {
            let mut o = Offer::note("ios", "iPhone simulator", "Pick the simulator tests use. Without a pick, they use the one that is running (or start an iPhone 16).");
            o.ready = true;
            o
        }
        None => Offer::note("ios", "iPhone simulator", "iPhone testing needs a one-time setup (about 5 minutes) with Xcode. Install Xcode from the App Store, then press Check again."),
    };
    offer.choices = choices;
    Some(offer)
}

/// What the doctor, init and the Device page say while the runner is not built yet.
pub const IOS_SETUP: &str = "iPhone testing needs a one-time setup (about 5 minutes)";

/// Build the iPhone test runner (`fastpilot setup ios --yes`, its output in the log). `None`
/// without Xcode.
fn ios_plan() -> Option<Plan> {
    if !cfg!(target_os = "macos") || !std::process::Command::new("xcodebuild").arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success()) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    Some(Plan {
        area: "ios",
        title: "iPhone testing".into(),
        confirm: "fastpilot will build a small test runner with your Xcode. It takes about 5 minutes, once. It runs inside your simulators to tap and read the screen; your simulators and Xcode are not changed.".into(),
        button: "Set up iPhone testing",
        steps: vec![Step {
            label: "Build the iPhone test runner with your Xcode".into(),
            action: Action::Run { program: exe.to_string_lossy().into_owned(), args: ["setup", "ios", "--yes"].map(String::from).to_vec(), answer: None, minutes: 30 },
        }],
        doing: "Setting up iPhone testing".into(),
        done: "iPhone testing is set up. Tests use your simulators now.".into(),
    })
}

/// Tests use this simulator (`ios_udid`; empty: any running one). Only a simulator `simctl` lists.
pub async fn pick_simulator(udid: &str) -> Result<String, String> {
    if udid.is_empty() {
        fastpilot_core::config::write("ios_udid", None).map_err(|e| e.to_string())?;
        return Ok("Tests use the simulator that is running.".into());
    }
    let sims = fastpilot_ios::devices().await.map_err(|e| e.to_string())?;
    let (_, name, _) = sims.iter().find(|(u, n, _)| u == udid && (n.starts_with("iPhone") || n.starts_with("iPad"))).ok_or("There is no such iPhone simulator on this computer.")?;
    fastpilot_core::config::write("ios_udid", Some(udid)).map_err(|e| e.to_string())?;
    Ok(format!("Tests use {name} now."))
}

// ---- S10 + DC4 (v0.7): one line per kind of device, one device picked for the person ----

/// One line of the Device page: the device tests use for one kind (website, Android, iPhone), or
/// what is missing and the one button that fixes it. The full list is behind Change.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Line {
    /// `web`, `android` or `ios`.
    pub kind: &'static str,
    /// The kind, never an ID: "Web · Chrome", "Android emulator", "iPhone simulator".
    pub name: &'static str,
    pub ready: bool,
    /// "Small_Phone_API_35 · in use", "No emulator running", "Needs one-time setup", ...
    pub detail: String,
    /// The button when it is not ready ("Start the Android emulator"); never an ID.
    pub button: Option<String>,
    /// Shown before the button acts (the page asks first).
    pub confirm: Option<String>,
    /// The body the page posts for the button (`confirmed: true` added after the person confirms).
    pub start: Option<serde_json::Value>,
    /// Every device of this kind on the computer, for Change.
    pub choices: Vec<Choice>,
}

/// A person-only setting (`device_autostart`, `android_avd`) as it is now: the environment, else the
/// file. Not through `config::var`, which pins non-free settings for the life of the process (the
/// dashboard must see the answer the person just gave).
fn person_setting(key: &str) -> Option<String> {
    std::env::var(format!("FASTPILOT_{}", key.to_uppercase())).ok().or_else(|| fastpilot_core::config::file_values().remove(key)).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// "Start it automatically when a test needs it?": the person's answer, `None` until asked.
pub fn autostart() -> Option<bool> {
    match person_setting("device_autostart")?.to_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}

/// AVD names of the emulators of this SDK running on this computer, by port, read from the process list
/// (`-avd NAME ... -port P`). Nothing is sent to any emulator or phone.
fn running_avd_names() -> std::collections::HashMap<u16, String> {
    let mut by_port = std::collections::HashMap::new();
    let Ok(out) = std::process::Command::new("ps").args(["-Ao", "command"]).stdin(std::process::Stdio::null()).output() else { return by_port };
    // Only emulators of this SDK (its emulator or qemu binary): the ones adb here lists.
    let root = sdk_root().display().to_string();
    for line in String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.contains(&root)) {
        let words: Vec<&str> = line.split_whitespace().collect();
        let after = |flag: &str| words.windows(2).find(|w| w[0] == flag).map(|w| w[1]);
        if let (Some(avd), Some(port)) = (after("-avd"), after("-port").and_then(|p| p.parse::<u16>().ok()))
            && avd_name_ok(avd)
        {
            by_port.insert(port, avd.to_owned());
        }
    }
    by_port
}

/// How a running Android device is named for the person: its emulator's name, or "Android phone
/// (physical)"; the serial only when nothing better is known.
fn android_name(serial: &str, names: &std::collections::HashMap<u16, String>) -> String {
    match serial.strip_prefix("emulator-").and_then(|p| p.parse::<u16>().ok()) {
        Some(port) => names.get(&port).cloned().unwrap_or_else(|| "Emulator".into()),
        None => "Android phone (physical)".into(),
    }
}

/// The installed emulator a test starts: `android_avd`, else fastpilot's test phone, else the first.
fn default_avd(avds: &[String]) -> Option<String> {
    let picked = person_setting("android_avd");
    picked.filter(|p| avds.contains(p)).or_else(|| avds.iter().find(|a| *a == AVD).cloned()).or_else(|| avds.first().cloned())
}

/// The simulator a test boots when none runs: the picked one, else `ios_device`, else iPhone 16.
fn default_simulator() -> String {
    ["FASTPILOT_IOS_UDID", "FASTPILOT_IOS_DEVICE"]
        .into_iter()
        .find_map(|k| fastpilot_core::config::var(k).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "iPhone 16".into())
}

fn web_line(offers: &[Offer]) -> Line {
    let offer = offers.iter().find(|o| o.area == "chrome");
    Line {
        kind: "web",
        name: "Web · Chrome",
        ready: offer.is_none(),
        detail: match offer {
            None => "Ready".into(),
            Some(o) if o.button.is_some() => "Chrome not found".into(),
            Some(o) => o.detail.clone(),
        },
        button: offer.and_then(|o| o.button).map(str::to_owned),
        confirm: offer.and_then(|o| o.confirm.clone()),
        start: offer.filter(|o| o.button.is_some()).map(|_| serde_json::json!({ "action": "start", "area": "chrome" })),
        choices: vec![],
    }
}

async fn android_line(offers: &[Offer]) -> Line {
    let mut line = Line { kind: "android", name: "Android emulator", ready: false, detail: String::new(), button: None, confirm: None, start: None, choices: vec![] };
    if let Some(server) = fastpilot_android::configured_server() {
        line.detail = server.err().unwrap_or_else(|| "Android devices come from another computer (FASTPILOT_ADB_SERVER).".into());
        return line;
    }
    let adb = sdk("platform-tools/adb", "adb");
    let avds = match sdk("emulator/emulator", "emulator") {
        Some(e) => installed_avds(&e).await,
        None => vec![],
    };
    let running: Vec<String> = match &adb {
        Some(adb) => adb_devices(adb).await.into_iter().filter(|(_, st)| st == "device").map(|(s, _)| s).collect(),
        None => vec![],
    };
    let names = running_avd_names();
    let chosen = fastpilot_core::config::var("FASTPILOT_ANDROID_SERIAL").ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let in_use = chosen.clone().filter(|c| running.contains(c)).or_else(|| running.first().cloned());
    for serial in &running {
        let used = in_use.as_deref() == Some(serial.as_str());
        line.choices.push(Choice {
            id: serial.clone(),
            name: android_name(serial, &names),
            act: "use",
            note: Some(if used { "in use".into() } else { "running".into() }),
            button: (!used).then_some("Use for tests"),
            confirm: None,
        });
    }
    // Only the emulators adb lists as running count as running (the process list may show others).
    let running_avds: Vec<String> = running.iter().map(|r| android_name(r, &names)).collect();
    for a in avds.iter().filter(|a| !running_avds.contains(a)) {
        line.choices.push(Choice { id: a.clone(), name: a.clone(), act: "start", note: (a == AVD).then(|| "fastpilot's test phone".into()), button: Some("Start"), confirm: Some(avd_confirm(a)) });
    }
    if let Some(serial) = &in_use {
        line.ready = true;
        let more = running.len() - 1;
        line.detail = format!("{} · in use{}", android_name(serial, &names), if more > 0 { format!(" · {more} more running") } else { String::new() });
        return line;
    }
    if let Some(avd) = default_avd(&avds) {
        line.detail = "No emulator running".into();
        line.button = Some("Start the Android emulator".into());
        line.confirm = Some(avd_confirm(&avd));
        line.start = Some(serde_json::json!({ "action": "start", "area": "android", "avd": avd }));
    } else if let Some(o) = offers.iter().find(|o| o.area == "android") {
        line.detail = if o.button.is_some() { "No emulator installed".into() } else { o.detail.clone() };
        line.button = o.button.map(str::to_owned);
        line.confirm = o.confirm.clone();
        line.start = o.button.map(|_| serde_json::json!({ "action": "start", "area": "android" }));
    } else {
        line.detail = "No emulator installed. Install Android Studio, then press Check again.".into();
    }
    line
}

async fn ios_line(offers: &[Offer]) -> Option<Line> {
    let offer = offers.iter().find(|o| o.area == "ios")?;
    let mut line = Line { kind: "ios", name: "iPhone simulator", ready: false, detail: String::new(), button: None, confirm: None, start: None, choices: offer.choices.clone() };
    let runner = fastpilot_ios::runner::xctestrun(fastpilot_ios::runner::Target::Simulator).is_some();
    if !runner {
        line.detail = "Needs one-time setup".into();
        line.button = offer.button.map(str::to_owned);
        line.confirm = offer.confirm.clone();
        line.start = offer.button.map(|_| serde_json::json!({ "action": "start", "area": "ios" }));
        if offer.button.is_none() {
            line.detail = offer.detail.clone();
        }
        return Some(line);
    }
    let sims = fastpilot_ios::devices().await.unwrap_or_default();
    let chosen = fastpilot_core::config::var("FASTPILOT_IOS_UDID").ok().filter(|u| !u.trim().is_empty());
    let booted = |u: &str| sims.iter().any(|(x, _, st)| x == u && st == "Booted");
    let in_use = chosen.clone().filter(|c| booted(c)).or_else(|| sims.iter().find(|(_, n, st)| st == "Booted" && (n.starts_with("iPhone") || n.starts_with("iPad"))).map(|(u, _, _)| u.clone()));
    match in_use.and_then(|u| sims.iter().find(|(x, _, _)| *x == u).cloned()) {
        Some((_, name, _)) => {
            line.ready = true;
            line.detail = format!("{name} · in use");
        }
        None if sims.is_empty() => line.detail = offer.detail.clone(),
        None => {
            line.detail = "No simulator running".into();
            line.button = Some("Start the iPhone simulator".into());
            line.confirm = Some(format!("fastpilot will start the iPhone simulator \"{}\". It takes about a minute. Other simulators are not changed.", default_simulator()));
            line.start = Some(serde_json::json!({ "action": "boot-simulator" }));
        }
    }
    Some(line)
}

/// The Device page's lines: website, Android, iPhone (macOS), in that order.
pub async fn lines(offers: &[Offer]) -> Vec<Line> {
    let mut lines = vec![web_line(offers), android_line(offers).await];
    lines.extend(ios_line(offers).await);
    lines
}

/// Tests use this running Android device (`android_serial`). Only a device adb lists.
async fn pick_android(serial: &str) -> Result<String, String> {
    let adb = sdk("platform-tools/adb", "adb").ok_or("adb is not installed")?;
    if !adb_devices(&adb).await.iter().any(|(s, st)| s == serial && st == "device") {
        return Err("That Android device is not running.".into());
    }
    fastpilot_core::config::write("android_serial", Some(serial)).map_err(|e| e.to_string())?;
    let name = android_name(serial, &running_avd_names());
    Ok(format!("Tests use {name} now."))
}

/// Boot the simulator tests use (`simctl boot`), when none runs.
async fn boot_simulator() -> Result<String, String> {
    let target = default_simulator();
    let udid = fastpilot_ios::boot(&target).await.map_err(|e| e.to_string())?;
    let name = fastpilot_ios::devices().await.ok().and_then(|v| v.into_iter().find(|(u, _, _)| *u == udid).map(|(_, n, _)| n)).unwrap_or(target);
    Ok(format!("{name} is running. iPhone tests use it now."))
}

/// What `fastpilot run` does about a stopped device, given the person's answer and whether it can
/// ask now (a Terminal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Autostart {
    Start,
    Ask,
    /// Not started: "No", or nobody to ask (the Device page asks).
    Leave,
}

pub fn decide(answer: Option<bool>, can_ask: bool) -> Autostart {
    match (answer, can_ask) {
        (Some(true), _) => Autostart::Start,
        (Some(false), _) => Autostart::Leave,
        (None, true) => Autostart::Ask,
        (None, false) => Autostart::Leave,
    }
}

/// The one-time question in Terminal; the answer is saved. Empty or "y" is yes.
pub fn ask_once(kind: &str, input: &mut dyn std::io::BufRead, out: &mut dyn std::io::Write) -> bool {
    let _ = write!(out, "\nThe {kind} is not running. Start it automatically when a test needs it? [Y/n] ");
    let _ = out.flush();
    let mut answer = String::new();
    let _ = input.read_line(&mut answer);
    let yes = !matches!(answer.trim().to_lowercase().as_str(), "n" | "no");
    match fastpilot_core::config::write("device_autostart", Some(if yes { "true" } else { "false" })) {
        Ok(_) => {
            let _ = writeln!(out, "  Saved. Change it any time on the Device page.");
        }
        Err(e) => {
            let _ = writeln!(out, "  Not saved ({e}); asked again next time.");
        }
    }
    yes
}

fn can_ask() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal() && std::env::var_os("FASTPILOT_SCHEDULED_AS").is_none()
}

/// S10: before `fastpilot run` looks for devices, start a stopped Android emulator or iPhone
/// simulator the tests need, when the person said so (asked once). The emulator is one of the
/// person's installed ones, on a free port from 5556 (never 5554), and keeps running afterwards.
/// Returns lines for the run's output.
pub async fn start_for_run(need: &[fastpilot_core::Platform]) -> Vec<String> {
    use fastpilot_core::Platform;
    let mut notes = Vec::new();
    let mut answer = autostart();
    fn decide_now(answer: &mut Option<bool>, kind: &str) -> bool {
        match decide(*answer, can_ask()) {
            Autostart::Start => true,
            Autostart::Leave => false,
            Autostart::Ask => {
                let yes = ask_once(kind, &mut std::io::stdin().lock(), &mut std::io::stderr());
                *answer = Some(yes);
                yes
            }
        }
    }
    if need.contains(&Platform::Android)
        && fastpilot_android::configured_server().is_none()
        && fastpilot_core::config::var("FASTPILOT_ANDROID_SERIAL").ok().is_none_or(|s| s.trim().is_empty())
        && let (Some(adb), Some(emulator)) = (sdk("platform-tools/adb", "adb"), sdk("emulator/emulator", "emulator"))
        && !device_running(Some(&adb)).await
        && let Some(avd) = default_avd(&installed_avds(&emulator).await)
    {
        if decide_now(&mut answer, "Android emulator") {
            eprintln!("Starting the Android emulator ({avd}) for this run…");
            notes.push(match start("android", Some(&avd)).await {
                Ok(_) => wait_for_job("the Android emulator").await,
                Err(e) => format!("Could not start the Android emulator: {e}"),
            });
        } else if answer.is_none() {
            notes.push("The Android emulator is not running. Start it on the Device page, where you can also say whether tests may start it themselves.".into());
        }
    }
    if need.contains(&Platform::Ios)
        && cfg!(target_os = "macos")
        && fastpilot_ios::runner::xctestrun(fastpilot_ios::runner::Target::Simulator).is_some()
        && let Ok(sims) = fastpilot_ios::devices().await
        && !sims.is_empty()
        && !sims.iter().any(|(_, n, st)| st == "Booted" && (n.starts_with("iPhone") || n.starts_with("iPad")))
    {
        if decide_now(&mut answer, "iPhone simulator") {
            eprintln!("Starting the iPhone simulator for this run…");
            notes.push(boot_simulator().await.unwrap_or_else(|e| format!("Could not start the iPhone simulator: {e}")));
        } else if answer.is_none() {
            notes.push("The iPhone simulator is not running. Start it on the Device page, where you can also say whether tests may start it themselves.".into());
        }
    }
    notes
}

/// Wait for the fix that started a device, with a progress line every 15 s (start_phone gives up
/// after 8 minutes).
async fn wait_for_job(what: &str) -> String {
    let began = std::time::Instant::now();
    let mut said = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (state, message) = job().as_ref().map(|r| (r.job.state, r.job.message.clone())).unwrap_or(("failed", None));
        match state {
            "running" => {
                let secs = began.elapsed().as_secs();
                if secs >= said + 15 {
                    said = secs - secs % 15;
                    eprintln!("  still starting {what} ({secs} s)");
                }
            }
            "done" => return message.unwrap_or_else(|| format!("Started {what}.")),
            _ => return format!("Could not start {what}: {}", message.unwrap_or_default()),
        }
    }
}

/// What the Device page can offer now, and the fix that runs or last ran.
pub async fn state() -> (Vec<Offer>, Option<Job>) {
    let mut offers = Vec::new();
    offers.extend(chrome_offer().await);
    offers.extend(android_offer().await);
    offers.extend(ios_offer().await);
    (offers, job().as_ref().map(|r| r.job.clone()))
}

/// Start the fix for `area` (`chrome`, `android` or `ios`) after the person confirmed on the
/// page; `avd`: start that installed emulator instead of setting up the test phone.
pub async fn start(area: &str, avd: Option<&str>) -> Result<Job, String> {
    if job().as_ref().is_some_and(|r| r.job.state == "running") {
        return Err("A fix is already running. Wait for it, or press Stop.".into());
    }
    let plan = match area {
        "chrome" => {
            if fastpilot_web::find_chrome().is_some() {
                return Err("Chrome is already installed.".into());
            }
            chrome_plan().ok_or("fastpilot cannot install Chrome here; download it from google.com/chrome.")?
        }
        "android" => match avd {
            Some(name) => {
                let plan = avd_plan(name).await?;
                let _ = fastpilot_core::config::write("android_avd", Some(name));
                plan
            }
            None => android_plan().await?.ok_or("fastpilot's test phone is already running.")?,
        },
        "ios" => {
            if fastpilot_ios::runner::xctestrun(fastpilot_ios::runner::Target::Simulator).is_some() {
                return Err("iPhone testing is already set up.".into());
            }
            ios_plan().ok_or("iPhone testing needs Xcode on a Mac. Install Xcode from the App Store first.")?
        }
        _ => return Err("Unknown fix.".into()),
    };
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    let view = Job {
        area: plan.area,
        title: plan.title.clone(),
        doing: plan.doing.clone(),
        state: "running",
        steps: plan.steps.iter().map(|s| StepView { label: s.label.clone(), state: "waiting" }).collect(),
        log: vec![],
        message: None,
    };
    {
        let mut j = job();
        if j.as_ref().is_some_and(|r| r.job.state == "running") {
            return Err("A fix is already running. Wait for it, or press Stop.".into());
        }
        *j = Some(Running { job: view.clone(), stop: stop.clone(), stopping: false });
    }
    tokio::spawn(run(plan, stop));
    Ok(view)
}

/// Stop the running fix (the program it runs is ended; a test phone it started is shut down).
pub fn stop() -> Result<(), String> {
    let mut j = job();
    match j.as_mut() {
        Some(r) if r.job.state == "running" => {
            r.stopping = true;
            r.stop.notify_waiters();
            r.stop.notify_one();
            Ok(())
        }
        _ => Err("No fix is running.".into()),
    }
}

fn stopping() -> bool {
    job().as_ref().is_some_and(|r| r.stopping)
}

fn finish(state: &'static str, message: Option<String>) {
    if let Some(r) = job().as_mut() {
        r.job.state = state;
        r.job.message = message;
    }
}

async fn run(plan: Plan, stop: std::sync::Arc<tokio::sync::Notify>) {
    let root = sdk_root();
    for (i, step) in plan.steps.iter().enumerate() {
        set_step(i, "running");
        log(&format!("— {}", step.label));
        let result = match &step.action {
            Action::Run { program, args, answer, minutes } => run_program(Path::new(program), args, *answer, *minutes, &root, &stop).await,
            Action::Tool { name, args, answer, minutes } => match cmdline_tool(name) {
                Some(p) => run_program(&p, args, *answer, *minutes, &root, &stop).await,
                None => Err(format!("{name} is not installed")),
            },
            Action::StartAvd(name) => start_phone(&root, &stop, name).await,
        };
        match result {
            Ok(()) => set_step(i, "done"),
            Err(e) if stopping() => {
                set_step(i, "stopped");
                log(&format!("stopped: {e}"));
                return finish("stopped", Some("Stopped. What was done so far stays; set it up again to continue.".into()));
            }
            Err(e) => {
                set_step(i, "failed");
                log(&format!("failed: {e}"));
                return finish("failed", Some(format!("Could not finish \"{}\": {e}. What was done so far stays; Try again continues from there.", step.label)));
            }
        }
    }
    finish("done", Some(plan.done.clone()));
}

async fn run_program(program: &Path, args: &[String], answer: Option<&'static str>, minutes: u64, root: &Path, stop: &tokio::sync::Notify) -> Result<(), String> {
    log(&format!("$ {} {}", program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), args.join(" ")));
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .env("ANDROID_SDK_ROOT", root)
        .env("ANDROID_HOME", root)
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .stdin(if answer.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", program.display()))?;
    if let (Some(a), Some(mut stdin)) = (answer, child.stdin.take()) {
        tokio::spawn(async move {
            for _ in 0..50 {
                if stdin.write_all(format!("{a}\n").as_bytes()).await.is_err() {
                    break;
                }
            }
        });
    }
    let out = child.stdout.take().map(|o| tokio::spawn(read_lines(o)));
    let err = child.stderr.take().map(|o| tokio::spawn(read_lines(o)));
    let status = tokio::select! {
        s = child.wait() => s.map_err(|e| e.to_string()),
        () = stop.notified() => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err("stopped by the user".into())
        }
        () = tokio::time::sleep(Duration::from_secs(minutes * 60)) => {
            let _ = child.start_kill();
            Err(format!("no result after {minutes} minutes"))
        }
    };
    for h in [out, err].into_iter().flatten() {
        let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
    }
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{} ended with {s}", program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
        Err(e) => Err(e),
    }
}

async fn read_lines<R: tokio::io::AsyncRead + Unpin>(r: R) {
    let mut lines = tokio::io::BufReader::new(r).lines();
    while let Ok(Some(l)) = lines.next_line().await {
        // sdkmanager redraws a progress bar with \r: keep the last state of the line.
        log(l.rsplit('\r').next().unwrap_or(&l));
    }
}

/// A free emulator port for the test phone: even, 5556–5584, not used by any device, never 5554.
async fn free_port(adb: &Path) -> Option<u16> {
    let used: Vec<String> = adb_devices(adb).await.into_iter().map(|(s, _)| s).collect();
    (5556..=5584u16).step_by(2).find(|p| {
        !used.iter().any(|s| s == &format!("emulator-{p}"))
            && std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok()
            && std::net::TcpListener::bind(("127.0.0.1", *p + 1)).is_ok()
    })
}

/// Start the AVD `name` on a free port and wait until it has booted. fastpilot's own test phone
/// runs without a window (its port is noted); the person's own emulator opens as usual, and its
/// saved state is not written (`-no-snapshot-save`).
async fn start_phone(root: &Path, stop: &tokio::sync::Notify, name: &str) -> Result<(), String> {
    let adb = sdk("platform-tools/adb", "adb").ok_or("adb is not installed")?;
    let emulator = sdk("emulator/emulator", "emulator").ok_or("the Android emulator is not installed")?;
    let own = name == AVD;
    if own && let Some(serial) = running_phone(&adb).await {
        log(&format!("the test phone is already running ({serial})"));
        return Ok(());
    }
    let port = free_port(&adb).await.ok_or("no free emulator port between 5556 and 5584")?;
    let serial = format!("emulator-{port}");
    if own {
        if let Some(dir) = port_file().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(port_file(), port.to_string()).map_err(|e| format!("could not note the test phone's port: {e}"))?;
    }
    let p = port.to_string();
    let args: Vec<String> = if own {
        ["-avd", AVD, "-port", &p, "-no-window", "-no-audio", "-no-snapshot-save", "-no-boot-anim"].map(String::from).to_vec()
    } else {
        ["-avd", name, "-port", &p, "-no-snapshot-save", "-no-boot-anim"].map(String::from).to_vec()
    };
    log(&format!("$ emulator {}", args.join(" ")));
    // It keeps running after this page and fastpilot stop: not killed on drop.
    let mut child = tokio::process::Command::new(&emulator)
        .args(&args)
        .env("ANDROID_SDK_ROOT", root)
        .env("ANDROID_HOME", root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start the emulator: {e}"))?;
    let booted = async {
        let mut said = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if let Ok(Some(s)) = child.try_wait() {
                return Err(format!("the emulator stopped ({s})"));
            }
            let o = tokio::process::Command::new(&adb).args(["-s", &serial, "shell", "getprop", "sys.boot_completed"]).stdin(std::process::Stdio::null()).output();
            if let Ok(Ok(o)) = tokio::time::timeout(Duration::from_secs(10), o).await
                && String::from_utf8_lossy(&o.stdout).trim() == "1"
            {
                return Ok(());
            }
            said += 1;
            if said.is_multiple_of(10) {
                log("still starting…");
            }
        }
    };
    let result = tokio::select! {
        r = booted => r,
        () = stop.notified() => Err("stopped by the user".into()),
        () = tokio::time::sleep(Duration::from_secs(8 * 60)) => Err("it did not finish starting within 8 minutes".into()),
    };
    match result {
        Ok(()) => {
            log(&format!("{} is running ({serial})", if own { "the test phone" } else { name }));
            Ok(())
        }
        Err(e) => {
            // Only the emulator this fix started is shut down.
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            Err(e)
        }
    }
}

// ---- the page's API: only the Device page's buttons (dashboard guard: cookie, own origin, JSON) ----

fn refuse(status: axum::http::StatusCode, why: &str) -> axum::response::Response {
    use axum::response::IntoResponse;
    (status, axum::Json(serde_json::json!({ "error": why }))).into_response()
}

/// `GET /api/fix`: what can be fixed, and the fix that runs or last ran.
pub async fn api_state() -> axum::response::Response {
    use axum::response::IntoResponse;
    let (offers, job) = state().await;
    let lines = lines(&offers).await;
    // The one-time question is asked only when there is a device fastpilot could start.
    let startable = lines.iter().any(|l| l.kind != "web" && (l.choices.iter().any(|c| c.act == "start") || l.start.as_ref().is_some_and(|s| s["action"] == "boot-simulator" || s["avd"].is_string())));
    let autostart = autostart();
    axum::Json(serde_json::json!({ "offers": offers, "job": job, "lines": lines, "autostart": autostart, "ask_autostart": autostart.is_none() && startable })).into_response()
}

/// `POST /api/fix`: `{action: start, area, confirmed: true}` after the page's confirmation, or
/// `{action: stop}`.
pub async fn api_change(axum::Json(body): axum::Json<serde_json::Value>) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    match body["action"].as_str().unwrap_or("") {
        "start" if body["confirmed"] != true => refuse(StatusCode::BAD_REQUEST, "Confirm the fix on the Device page first."),
        "start" => match start(body["area"].as_str().unwrap_or(""), body["avd"].as_str()).await {
            Ok(job) => axum::Json(serde_json::json!({ "job": job })).into_response(),
            Err(e) => refuse(StatusCode::CONFLICT, &e),
        },
        // Which simulator tests use: a setting, not a fix (nothing is installed or started).
        "pick-simulator" => match pick_simulator(body["udid"].as_str().unwrap_or("")).await {
            Ok(note) => axum::Json(serde_json::json!({ "ok": note })).into_response(),
            Err(e) => refuse(StatusCode::CONFLICT, &e),
        },
        // S10: "Start it automatically when a test needs it?" Yes / No (a person's click on this page).
        "autostart" => match body["on"].as_bool() {
            Some(on) => match fastpilot_core::config::write("device_autostart", Some(if on { "true" } else { "false" })) {
                Ok(_) => axum::Json(serde_json::json!({ "ok": if on { "Tests start a stopped emulator or simulator when they need it." } else { "Tests do not start devices. Use Start on this page." } })).into_response(),
                Err(e) => refuse(StatusCode::CONFLICT, &e),
            },
            None => refuse(StatusCode::BAD_REQUEST, "Say yes or no."),
        },
        "pick-android" => match pick_android(body["serial"].as_str().unwrap_or("")).await {
            Ok(note) => axum::Json(serde_json::json!({ "ok": note })).into_response(),
            Err(e) => refuse(StatusCode::CONFLICT, &e),
        },
        "boot-simulator" if body["confirmed"] != true => refuse(StatusCode::BAD_REQUEST, "Confirm on the Device page first."),
        "boot-simulator" => match boot_simulator().await {
            Ok(note) => axum::Json(serde_json::json!({ "ok": note })).into_response(),
            Err(e) => refuse(StatusCode::CONFLICT, &e),
        },
        "stop" => match stop() {
            Ok(()) => axum::Json(serde_json::json!({ "ok": "Stopping…" })).into_response(),
            Err(e) => refuse(StatusCode::CONFLICT, &e),
        },
        _ => refuse(StatusCode::BAD_REQUEST, "Unknown fix action."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S10: the one-time question. Yes starts, No leaves it (the Device page has Start), and with
    /// nobody to ask (a schedule, the dashboard) nothing starts until the person answers.
    #[test]
    fn a_stopped_device_is_started_only_when_the_person_said_so() {
        assert_eq!(decide(Some(true), false), Autostart::Start);
        assert_eq!(decide(Some(false), true), Autostart::Leave);
        assert_eq!(decide(None, true), Autostart::Ask);
        assert_eq!(decide(None, false), Autostart::Leave);
    }

    /// S10 "Done when": with the emulator stopped and the question unanswered, `fastpilot run` in
    /// Terminal asks once, saves the answer and starts the installed emulator (on a free port, never
    /// 5554); the next run does not ask again. Stand-in SDK scripts; nothing real is started.
    #[tokio::test]
    // The environment must stay as set for the whole test.
    #[allow(clippy::await_holding_lock)]
    async fn a_run_asks_once_then_starts_the_installed_emulator() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("fp-s10-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sdk = dir.join("sdk");
        let script = |rel: &str, text: &str| {
            let p = sdk.join(rel);
            std::fs::create_dir_all(p.parent().expect("dir")).expect("dir");
            std::fs::write(&p, text).expect("script");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        };
        let s = sdk.display().to_string();
        script("platform-tools/adb", &format!("#!/bin/bash\necho \"adb $*\" >> {s}/calls.log\nif [ \"$1\" = devices ]; then echo 'List of devices attached'; for f in {s}/running-*; do [ -e \"$f\" ] && printf 'emulator-%s\\tdevice\\n' \"${{f##*-}}\"; done; fi\nif [ \"$4 $5\" = \"getprop sys.boot_completed\" ]; then echo 1; fi\nexit 0\n"));
        script("emulator/emulator", &format!("#!/bin/bash\necho \"emulator $*\" >> {s}/calls.log\nif [ \"$1\" = -list-avds ]; then echo Pixel_User; exit 0; fi\nport=$4\ntouch {s}/running-$port\nsleep 60\n"));
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("FASTPILOT_CONFIG", dir.join("config.toml"));
            std::env::set_var("ANDROID_HOME", &sdk);
            for k in ["ANDROID_SDK_ROOT", "FASTPILOT_ADB_SERVER", "FASTPILOT_ANDROID_SERIAL", "ANDROID_SERIAL", "FASTPILOT_DEVICE_AUTOSTART", "FASTPILOT_ANDROID_AVD"] {
                std::env::remove_var(k);
            }
        }
        // Asked once in Terminal: Enter is yes, and it is saved.
        let mut out = Vec::new();
        assert!(ask_once("Android emulator", &mut std::io::Cursor::new(b"\n".to_vec()), &mut out));
        let said = String::from_utf8_lossy(&out);
        assert!(said.contains("The Android emulator is not running. Start it automatically when a test needs it? [Y/n]") && said.contains("Saved."), "{said}");
        assert_eq!(autostart(), Some(true));
        assert_eq!(decide(autostart(), true), Autostart::Start, "not asked again");
        // The run starts the installed emulator, then has a device.
        let notes = start_for_run(&[fastpilot_core::Platform::Web, fastpilot_core::Platform::Android]).await;
        assert_eq!(notes, vec!["Pixel_User is running. Android tests use it now.".to_owned()]);
        let calls = std::fs::read_to_string(sdk.join("calls.log")).unwrap_or_default();
        let start = calls.lines().find(|l| l.starts_with("emulator -avd")).unwrap_or_else(|| panic!("{calls}"));
        let port: u16 = start.split("-port ").nth(1).and_then(|r| r.split(' ').next()).and_then(|p| p.parse().ok()).expect("port");
        assert!(port != 5554 && (5556..=5584).contains(&port), "{start}");
        // Running now: a second run starts nothing.
        let again = start_for_run(&[fastpilot_core::Platform::Android]).await;
        assert!(again.is_empty(), "{again:?} {}", std::fs::read_to_string(sdk.join("calls.log")).unwrap_or_default());
        assert_eq!(calls.matches("emulator -avd").count(), std::fs::read_to_string(sdk.join("calls.log")).unwrap_or_default().matches("emulator -avd").count());
        // "No": nothing starts, and the run says nothing more (the Device page has Start).
        let _ = std::process::Command::new("pkill").args(["-f", &format!("{s}/emulator/emulator")]).status();
        for e in std::fs::read_dir(&sdk).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with("running-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
        // (The file was written within the same second; the variable is the same setting.)
        // SAFETY: as above.
        unsafe { std::env::set_var("FASTPILOT_DEVICE_AUTOSTART", "false") };
        let before = std::fs::read_to_string(sdk.join("calls.log")).unwrap_or_default().matches("emulator -avd").count();
        assert!(start_for_run(&[fastpilot_core::Platform::Android]).await.is_empty());
        assert_eq!(std::fs::read_to_string(sdk.join("calls.log")).unwrap_or_default().matches("emulator -avd").count(), before, "No starts nothing");
        // SAFETY: as above.
        unsafe {
            for k in ["FASTPILOT_CONFIG", "ANDROID_HOME", "FASTPILOT_DEVICE_AUTOSTART"] {
                std::env::remove_var(k);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Security re-review (Low): `.` or an empty PATH entry never finds a program (the tests run
    /// in the crate folder, which has a `Cargo.toml`).
    #[test]
    fn only_absolute_path_folders_count() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        assert!(Path::new("Cargo.toml").is_file(), "run from the crate folder");
        let here = std::env::current_dir().expect("cwd");
        let old = std::env::var_os("PATH");
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe { std::env::set_var("PATH", ".:") };
        let relative = on_path("Cargo.toml");
        unsafe { std::env::set_var("PATH", format!(".::{}", here.display())) };
        let absolute = on_path("Cargo.toml");
        match old {
            Some(p) => unsafe { std::env::set_var("PATH", p) },
            None => unsafe { std::env::remove_var("PATH") },
        }
        assert_eq!(relative, None, "relative and empty entries do not count");
        assert_eq!(absolute, Some(here.join("Cargo.toml")), "an absolute folder does");
    }
    #[test]
    fn the_test_phone_never_uses_port_5554() {
        assert!((5556..=5584u16).step_by(2).all(|p| p != 5554 && p.is_multiple_of(2)));
        assert_eq!(super::AVD, "fastpilot_api35");
    }
}
