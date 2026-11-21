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
    let choices = avds.iter().map(|a| Choice { id: a.clone(), name: a.clone(), note: (a == AVD).then(|| "fastpilot's test phone".into()), button: Some("Start"), confirm: Some(avd_confirm(a)) }).collect();
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
            let picked = chosen.as_deref() == Some(udid.as_str());
            Choice { id: udid.clone(), name: name.clone(), note: (!notes.is_empty()).then(|| notes.join(" · ")), button: (!picked).then_some("Use for tests"), confirm: None }
        })
        .collect();
    if chosen.is_some() {
        choices.push(Choice { id: String::new(), name: "Any running simulator".into(), note: None, button: Some("Use for tests"), confirm: None });
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
            Some(name) => avd_plan(name).await?,
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
    axum::Json(serde_json::json!({ "offers": offers, "job": job })).into_response()
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
