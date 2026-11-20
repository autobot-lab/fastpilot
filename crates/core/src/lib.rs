//! fastpilot core: the platform-agnostic driver contract, selectors, snapshots,
//! the flow DSL and the flow engine. Drivers (web, android, ios) live in their own crates.

pub mod change;
pub mod claim;
pub mod config;
pub mod live;
mod device;
mod dsl;
mod engine;
mod error;
mod recording;
mod selector;
pub mod policy;
pub mod ship;
mod snapshot;
pub mod state;
pub mod visual;

/// Tests that change process-wide environment variables (FASTPILOT_DIR) take turns.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

use std::fmt;
use std::time::Duration;

/// `--reset`: the apps already reset in this run (a test of `run --reset`, or the whole
/// `mcp --reset` process). Each app is reset once, by whichever comes first: a session started
/// with the app as its target, or the first `launchApp` of it.
#[derive(Debug, Default)]
pub struct FreshApps {
    done: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl FreshApps {
    /// Whether `app` (`com.x` or `com.x/.Main`) still has to be reset now; marks it reset.
    pub fn claim(&self, app: &str) -> bool {
        if app.contains("://") {
            return false;
        }
        let id = app.split('/').next().unwrap_or(app).trim().to_owned();
        !id.is_empty() && self.done.lock().unwrap_or_else(|p| p.into_inner()).insert(id)
    }
}

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use dsl::{
    Branch, Condition, Flow, FlowError, FlowStep, HttpRequest, OtpSource, Random, ReadOtp, Step, check_flow, check_flow_file, flow_to_yaml, parse_flow,
    parse_flow_file, redact, to_vars,
};
pub use recording::{Mark, Span, timeline};
pub use engine::{AssertKind, FlowReport, Launcher, RunOptions, SessionRegistry, StepRecord, assert, check_session_id, reset_app, run_flow, sessions_used};
pub use device::{DeviceInfo, DeviceKind, DeviceSpec, SelectError, select};
pub use error::{Error, Result};
pub use recording::{escape_html, filmstrip};
pub use selector::Selector;
pub use snapshot::{Entry, Intent, Node, Rect, RefRegistry, Snapshot, diff, quote, shows_nothing_new};

pub type Value = serde_json::Value;
pub type SessionId = String;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Web,
    Android,
    Ios,
}

impl Platform {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "web" => Some(Self::Web),
            "android" => Some(Self::Android),
            "ios" => Some(Self::Ios),
            _ => None,
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Web => "web",
            Self::Android => "android",
            Self::Ios => "ios",
        })
    }
}

/// What a recording produced: an encoded video (mobile), or timed JPEG frames (web screencast).
#[derive(Debug, Clone)]
pub enum Recording {
    Video { mime: &'static str, data: Vec<u8> },
    /// `(seconds since the recording started, jpeg)`.
    Frames(Vec<(f64, Vec<u8>)>),
}

/// `setPermissions` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    Allow,
    Deny,
    /// Back to "ask" (the next request shows the prompt again).
    Unset,
}

/// `setViewport` (web): a device preset or explicit metrics; `None` from `preset("desktop")`
/// means back to the normal window.
#[derive(Debug, Clone, PartialEq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub mobile: bool,
    pub touch: bool,
    pub user_agent: Option<String>,
}

const IOS_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const ANDROID_UA: &str = "Mozilla/5.0 (Linux; Android 15; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36";
const IPAD_UA: &str = "Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";

impl Viewport {
    /// Device presets (CSS pixels): iPhone 15/16, iPhone SE, Pixel 8, Galaxy S23, iPad.
    pub fn preset(name: &str) -> Option<Option<Self>> {
        let m = |width, height, scale, ua: &str| Some(Some(Self { width, height, scale, mobile: true, touch: true, user_agent: Some(ua.into()) }));
        match name.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
            "iphone" | "iphone15" | "iphone16" | "iphone15pro" | "iphone16pro" => m(393, 852, 3.0, IOS_UA),
            "iphonese" => m(375, 667, 2.0, IOS_UA),
            "iphonepromax" | "iphone16promax" | "iphone15promax" => m(430, 932, 3.0, IOS_UA),
            "pixel" | "pixel8" | "pixel7" => m(412, 915, 2.625, ANDROID_UA),
            "galaxy" | "galaxys23" | "samsung" => m(360, 780, 3.0, ANDROID_UA),
            "ipad" | "ipadair" => m(820, 1180, 2.0, IPAD_UA),
            "desktop" | "reset" | "none" => Some(None),
            _ => None,
        }
    }
}

/// A `launchApp` argument value: Android intent extra (`--es`/`--ei`/`--el`/`--ez`/`--esa`), iOS
/// launch argument `-key value` (read with `UserDefaults`).
#[derive(Debug, Clone, PartialEq)]
pub enum LaunchArg {
    Str(String),
    Int(i64),
    Bool(bool),
    List(Vec<String>),
}

/// `launchApp` extras: `arguments` (Android and iOS) and `env` (iOS process environment).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Launch {
    pub arguments: Vec<(String, LaunchArg)>,
    pub env: Vec<(String, String)>,
}

impl Launch {
    pub fn is_empty(&self) -> bool {
        self.arguments.is_empty() && self.env.is_empty()
    }
}

/// Why an environment variable name may not be passed to an app under test: it would change how
/// the app (or its loader) runs rather than what it reads, or reach into fastpilot itself.
pub fn launch_env_refused(name: &str) -> Option<String> {
    let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Some(format!("env name `{name}` must be letters, digits and _"));
    }
    let upper = name.to_ascii_uppercase();
    ["DYLD_", "LD_", "FASTPILOT_", "__XPC_", "MALLOC", "OBJC_", "CFNETWORK_", "NSZOMBIE", "XPC_"]
        .iter()
        .find(|p| upper.starts_with(*p))
        .map(|p| format!("env name `{name}` is refused: `{p}*` variables change how the app or the system loads it"))
}

/// `setNetwork`: offline, or online with optional throttling.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Network {
    pub offline: bool,
    pub latency_ms: Option<u64>,
    pub download_kbps: Option<u64>,
    pub upload_kbps: Option<u64>,
}

/// Shared by every platform (PRD §7.1).
#[async_trait]
pub trait Driver: Send + Sync {
    async fn open(&self, target: &str) -> Result<()>; // URL | package/activity | bundleId
    async fn snapshot(&self) -> Result<Snapshot>; // normalized a11y tree + refs
    async fn tap(&self, sel: &Selector) -> Result<()>;
    async fn type_text(&self, sel: &Selector, text: &str) -> Result<()>;
    async fn scroll(&self, dir: Direction, sel: Option<&Selector>) -> Result<()>;
    async fn swipe(&self, from: Point, to: Point) -> Result<()>;
    async fn back(&self) -> Result<()>;
    async fn screenshot(&self) -> Result<Vec<u8>>; // JPEG
    async fn wait_idle(&self, timeout: Duration) -> Result<()>;
    async fn eval_script(&self, js: &str) -> Result<Value>; // web/WebView only; others Unsupported

    // Beyond §7.1: optional capabilities; drivers that lack them report `unsupported`.
    /// Key name: Enter, Tab, Escape, Backspace, Delete, ArrowUp/Down/Left/Right, Home, End, PageUp, PageDown, Space.
    async fn press_key(&self, key: &str) -> Result<()> {
        Err(Error::Unsupported(format!("pressKey {key} is not supported by this driver")))
    }
    async fn select_option(&self, _sel: &Selector, _option: &str) -> Result<()> {
        Err(Error::Unsupported("selectOption is not supported by this driver".into()))
    }
    async fn upload(&self, _sel: &Selector, _files: &[String]) -> Result<()> {
        Err(Error::Unsupported("uploadFile is not supported by this driver".into()))
    }
    /// Screenshot with numbered boxes (Set-of-Mark) drawn at `marks` (ref, box); drivers that
    /// cannot draw return a plain screenshot.
    async fn screenshot_annotated(&self, _marks: &[(String, Rect)]) -> Result<Vec<u8>> {
        self.screenshot().await
    }
    /// Console errors, failed requests and similar, newest last (attached to failures).
    async fn diagnostics(&self) -> Vec<String> {
        Vec::new()
    }
    /// Maestro `launchApp`: package / package/activity / bundleId.
    async fn launch_app(&self, app: &str) -> Result<()> {
        self.open(app).await
    }
    /// The browser or device behind this session is gone for good (Chrome crashed or was
    /// closed): the session cannot recover and should be dropped.
    fn is_gone(&self) -> bool {
        false
    }
    /// Where `fastpilot ui` can watch this session (web sessions on fastpilot's own Chrome).
    fn live_screen(&self) -> Option<live::Screen> {
        None
    }
    /// `installApp`: put an app (apk / .app) on the device.
    async fn install_app(&self, _path: &str) -> Result<()> {
        Err(Error::Unsupported("installApp is for Android devices and iOS simulators".into()))
    }
    /// `uninstallApp`: remove an app from the device.
    async fn uninstall_app(&self, _app: &str) -> Result<()> {
        Err(Error::Unsupported("uninstallApp is for Android devices and iOS simulators".into()))
    }
    /// `launchApp` with `arguments` / `env` (values already substituted).
    async fn launch_app_with(&self, app: &str, launch: &Launch) -> Result<()> {
        if launch.is_empty() {
            return self.launch_app(app).await;
        }
        Err(Error::Unsupported("launchApp `arguments` / `env` are for Android and iOS apps".into()))
    }
    /// Which platform this driver drives (for `when: { platform: ... }`).
    fn platform(&self) -> Platform {
        Platform::Web
    }
    async fn long_press(&self, _sel: &Selector) -> Result<()> {
        Err(Error::Unsupported("longPressOn is not supported by this driver".into()))
    }
    async fn double_tap(&self, _sel: &Selector) -> Result<()> {
        Err(Error::Unsupported("doubleTapOn is not supported by this driver".into()))
    }
    async fn hide_keyboard(&self) -> Result<()> {
        Err(Error::Unsupported("hideKeyboard is not supported by this driver".into()))
    }
    /// Delete `count` characters before the cursor of the focused field (`None`: all of them).
    async fn erase_text(&self, _count: Option<usize>) -> Result<()> {
        Err(Error::Unsupported("eraseText is not supported by this driver".into()))
    }
    /// Stop the app (`None`: the app under test). Mobile only.
    async fn stop_app(&self, _app: Option<&str>) -> Result<()> {
        Err(Error::Unsupported("stopApp is mobile-only".into()))
    }
    /// Wipe the app's data (web: cookies and storage of the session's origin).
    async fn clear_state(&self, _app: Option<&str>) -> Result<()> {
        Err(Error::Unsupported("clearState is not supported by this driver".into()))
    }
    /// `--reset`: the app as if freshly installed (data, permissions) and the screen upright.
    /// Returns what was reset, for the session's report. Mobile only.
    async fn reset_app(&self, _app: &str) -> Result<String> {
        Err(Error::Unsupported("--reset is for android and ios sessions".into()))
    }
    /// `--reset`: reset each app on its first `launchApp` (claimed in `apps`). Mobile drivers.
    fn reset_on_first_launch(&self, _apps: std::sync::Arc<FreshApps>) {}
    /// Visible area in snapshot coordinates, when the snapshot includes off-screen nodes (web).
    async fn viewport(&self) -> Option<Rect> {
        None
    }
    /// SMS inbox bodies, newest first (for one-time codes).
    async fn read_sms(&self) -> Result<Vec<String>> {
        Err(Error::Unsupported("reading SMS is Android-only".into()))
    }
    /// Start recording the screen (web: CDP screencast; android: screenrecord; ios: simctl).
    async fn start_recording(&self) -> Result<()> {
        Err(Error::Unsupported("recording is not supported by this driver".into()))
    }
    async fn stop_recording(&self) -> Result<Recording> {
        Err(Error::Unsupported("recording is not supported by this driver".into()))
    }
    /// `None`: back to the normal window size.
    async fn set_viewport(&self, _v: Option<&Viewport>) -> Result<()> {
        Err(Error::Unsupported("setViewport is web-only (mobile sessions already run on a device)".into()))
    }
    async fn set_location(&self, _lat: f64, _lon: f64) -> Result<()> {
        Err(Error::Unsupported("setLocation is not supported by this driver".into()))
    }
    /// Permission names: camera, microphone, location, notifications, contacts, photos,
    /// clipboard (web), calendar; a platform-specific name passes through.
    async fn set_permissions(&self, _app: Option<&str>, _perms: &[(String, Grant)]) -> Result<()> {
        Err(Error::Unsupported("setPermissions is not supported by this driver".into()))
    }
    async fn set_network(&self, _n: &Network) -> Result<()> {
        Err(Error::Unsupported("setNetwork is not supported by this driver".into()))
    }
    /// Locale like `id-ID`, IANA timezone like `Asia/Jakarta`.
    async fn set_locale(&self, _locale: Option<&str>, _timezone: Option<&str>) -> Result<()> {
        Err(Error::Unsupported("setLocale is not supported by this driver".into()))
    }
    async fn push_notification(&self, _app: Option<&str>, _title: &str, _body: &str, _payload: Option<&Value>) -> Result<()> {
        Err(Error::Unsupported("pushNotification is mobile-only".into()))
    }
    /// End of the session: undo what the driver changed on the device (animation scales,
    /// stay-awake) and stop on-device helpers. Called on `session stop` and at shutdown.
    async fn close(&self) -> Result<()> {
        Ok(())
    }
    /// Login state to save (`saveState`): web returns cookies and the current origin's
    /// localStorage. Other platforms: Unsupported.
    async fn save_state(&self) -> Result<Value> {
        Err(Error::Unsupported("saveState / loadState (saved login state) is web-only".into()))
    }
    /// Restore what `save_state` returned (`loadState`).
    async fn load_state(&self, _state: &Value) -> Result<()> {
        Err(Error::Unsupported("saveState / loadState (saved login state) is web-only".into()))
    }
    async fn clear_keychain(&self) -> Result<()> {
        Err(Error::Unsupported("clearKeychain is iOS-only".into()))
    }
}

#[cfg(test)]
mod viewport_tests {
    use super::*;

    #[test]
    fn presets_and_parsing() {
        let p = Viewport::preset("iPhone 15").flatten().expect("preset");
        assert_eq!((p.width, p.height, p.mobile, p.touch), (393, 852, true, true));
        assert_eq!(Viewport::preset("desktop"), Some(None));
        assert_eq!(Viewport::preset("nokia 3310"), None);
        let flow = parse_flow("- setViewport: Pixel 8\n- setViewport: { width: 800, height: 600, deviceScaleFactor: 2 }\n- setViewport: desktop\n").expect("flow");
        assert_eq!(flow.steps[0].step.label(), "setViewport Pixel 8");
        assert_eq!(flow.steps[1].step.label(), "setViewport 800x600@2");
        assert_eq!(flow.steps[2].step.label(), "setViewport desktop");
        assert!(parse_flow("- setViewport: { width: 5 }\n").is_err());
    }
}
