//! Flow DSL (PRD §11): Maestro-compatible YAML plus `openUrl`, `evalScript`, `session`, `parallel`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_yaml::{Mapping, Value as Yaml};

use crate::{Direction, Grant, Network, Platform, Point, Selector, Viewport};

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    OpenUrl(String),
    TapOn(Selector),
    /// A tap-like step written with `confirm: true`: allowed past FASTPILOT_CONFIRM_ACTIONS.
    Confirmed(Box<Step>),
    InputText { into: Selector, text: String },
    AssertVisible { selector: Selector, timeout: Option<Duration> },
    Scroll { direction: Direction, within: Option<Selector> },
    Back,
    EvalScript(String),
    AssertNotVisible { selector: Selector, timeout: Option<Duration> },
    PressKey(String),
    SelectOption { from: Selector, option: String },
    UploadFile { into: Selector, files: Vec<String> },
    /// `launchApp: appId` or `{ appId, arguments: {k: v}, env: {K: V} }`.
    LaunchApp { app: String, launch: crate::Launch },
    Swipe { from: Point, to: Point },
    UseSession(String),
    StartSession { id: String, platform: Platform, target: Option<String>, reset: bool, device: Option<String> },
    Parallel(Vec<Branch>),
    LongPressOn(Selector),
    DoubleTapOn(Selector),
    ScrollUntilVisible { selector: Selector, direction: Direction, timeout: Option<Duration> },
    HideKeyboard,
    /// `None`: erase everything in the focused field.
    EraseText(Option<usize>),
    /// `None`: the app under test.
    StopApp(Option<String>),
    ClearState(Option<String>),
    ClearKeychain,
    /// `installApp: path/to/app.apk` (android) or `.app` (ios simulator).
    InstallApp(String),
    /// `uninstallApp: com.example.app`.
    UninstallApp(String),
    CopyTextFrom(Selector),
    PasteText,
    WaitForAnimationToEnd(Option<Duration>),
    TakeScreenshot(String),
    /// `assertScreenshot: name` (visual regression against `.fastpilot/baselines/<name>.png`).
    AssertScreenshot { name: String, threshold: Option<f64>, max_diff: Option<f64>, ignore: Vec<Selector>, update: bool },
    /// `saveState: name` → `.fastpilot/states/<name>.json` (web: cookies + localStorage).
    SaveState(String),
    /// `loadState: name`: restore a saved login state.
    LoadState(String),
    InputRandom { kind: Random, length: usize },
    /// A sub-flow: `file` (kept for round-trips) or inline `commands`, optionally conditional,
    /// with `env` variables visible inside it.
    RunFlow { file: Option<String>, when: Option<Condition>, env: Vec<(String, String)>, steps: Vec<FlowStep> },
    Repeat { times: Option<u32>, while_: Option<Condition>, steps: Vec<FlowStep> },
    Retry { max: u32, steps: Vec<FlowStep> },
    /// Test-data setup/teardown over HTTP; needs no session.
    HttpRequest(Box<HttpRequest>),
    /// Wait for a one-time code and store it in a variable.
    ReadOtp(Box<ReadOtp>),
    /// Screen recording of the active session, saved as `.fastpilot/recordings/<name>.*`.
    StartRecording(String),
    StopRecording,
    SetLocation { latitude: f64, longitude: f64 },
    /// Web device emulation; `None` = back to the normal window. `name` keeps the preset.
    SetViewport { name: Option<String>, viewport: Option<Viewport> },
    SetPermissions { app: Option<String>, permissions: Vec<(String, Grant)> },
    SetNetwork(Network),
    SetLocale { locale: Option<String>, timezone: Option<String> },
    PushNotification { app: Option<String>, title: String, body: String, payload: Option<serde_json::Value> },
    /// R2: the health of the open web page (console, requests, links, 3 sizes, accessibility);
    /// `links`: how many same-site links are asked at most (0: none).
    ShipCheck { links: usize, cookies: bool },
}

fn grant_name(g: Grant) -> &'static str {
    match g {
        Grant::Allow => "allow",
        Grant::Deny => "deny",
        Grant::Unset => "unset",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// JSON body; `${VARS}` are replaced inside its strings.
    pub json: Option<serde_json::Value>,
    /// Expected status; default: any 2xx.
    pub expect: Option<u16>,
    /// `VAR: dotted.path` into the JSON response (`items.0.id`).
    pub capture: Vec<(String, String)>,
    /// Captured names to redact in addition to the secret-looking ones.
    pub secret: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtpSource {
    Mailpit,
    Mailhog,
    /// Any endpoint whose body contains the code.
    Http,
    /// The device's SMS inbox (Android).
    Sms,
}

impl OtpSource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mailpit => "mailpit",
            Self::Mailhog => "mailhog",
            Self::Http => "http",
            Self::Sms => "sms",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadOtp {
    pub source: OtpSource,
    pub url: Option<String>,
    /// Mail: recipient address to look for.
    pub to: Option<String>,
    /// Only text after this (case-insensitive), e.g. "kode".
    pub after: Option<String>,
    /// Exact number of digits; default 4 to 8.
    pub length: Option<usize>,
    pub into: String,
    pub timeout: Option<Duration>,
}

/// Captured variables with these in their name are redacted like `FASTPILOT_VARS`.
const SECRET_HINTS: &[&str] = &["TOKEN", "SECRET", "PASSWORD", "PASS", "KEY", "OTP", "COOKIE", "SESSION", "AUTH"];

pub(crate) fn looks_secret(name: &str) -> bool {
    SECRET_HINTS.iter().any(|h| name.contains(h))
}

/// Values captured at runtime that must never reach the AI (process-wide, like FASTPILOT_VARS).
static SECRETS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn add_secret(value: &str) {
    if value.chars().count() >= 3 {
        let mut s = SECRETS.lock().unwrap_or_else(|p| p.into_inner());
        if !s.iter().any(|v| v == value) {
            s.push(value.to_owned());
            let extra = s.len().saturating_sub(1000);
            s.drain(..extra);
        }
    }
}

fn json_of(v: &Yaml) -> Result<serde_json::Value, String> {
    serde_json::to_value(v).map_err(|e| format!("json: {e}"))
}

fn var_name(v: &Yaml) -> Result<String, String> {
    let k = v.as_str().ok_or("variable names are strings")?.to_owned();
    if is_var(&k) { Ok(k) } else { Err(format!("variable `{k}` must be UPPER_SNAKE_CASE")) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Random {
    Text,
    Number,
    Email,
    PersonName,
}

impl Random {
    fn command(self) -> &'static str {
        match self {
            Self::Text => "inputRandomText",
            Self::Number => "inputRandomNumber",
            Self::Email => "inputRandomEmail",
            Self::PersonName => "inputRandomPersonName",
        }
    }
}

/// `when:` / `while:` condition, checked once against the current screen.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    Visible(Selector),
    NotVisible(Selector),
    Platform(Platform),
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Visible(s) => write!(f, "visible {s}"),
            Self::NotVisible(s) => write!(f, "notVisible {s}"),
            Self::Platform(p) => write!(f, "platform {p}"),
        }
    }
}

impl Condition {
    fn parse(v: &Yaml) -> Result<Self, String> {
        let m = v.as_mapping().ok_or("expects { visible | notVisible | platform }")?;
        if let Some(s) = get(m, "visible") {
            return Ok(Self::Visible(Selector::from_yaml(s, &[])?));
        }
        if let Some(s) = get(m, "notVisible") {
            return Ok(Self::NotVisible(Selector::from_yaml(s, &[])?));
        }
        if let Some(p) = get(m, "platform") {
            let p = string(p)?;
            return Platform::parse(&p).map(Self::Platform).ok_or(format!("unknown platform `{p}` (web, android, ios)"));
        }
        // `true:` is a boolean key in YAML.
        if get(m, "true").is_some() || m.get(Yaml::Bool(true)).is_some() {
            return Err("JavaScript conditions (`true:`) are not supported; use visible, notVisible or platform".into());
        }
        Err("expects { visible | notVisible | platform }".into())
    }

    fn to_yaml(&self) -> Yaml {
        match self {
            Self::Visible(s) => map(vec![("visible", s.to_yaml())]),
            Self::NotVisible(s) => map(vec![("notVisible", s.to_yaml())]),
            Self::Platform(p) => map(vec![("platform", Yaml::String(p.to_string()))]),
        }
    }
}

/// Where relative `runFlow` files resolve, and how deep sub-flows already nest.
#[derive(Debug, Clone)]
struct Cx {
    base: PathBuf,
    depth: usize,
}

const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    pub session: String,
    pub steps: Vec<FlowStep>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowStep {
    /// 1-based source line, when it could be located.
    pub line: Option<usize>,
    pub step: Step,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Flow {
    pub steps: Vec<FlowStep>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowError {
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for FlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "line {l}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Maestro commands we recognise but do not run, so the AI gets a clear message.
const MAESTRO_LATER: &[&str] = &[
    "runScript", "assertTrue", "assertWithAI", "assertNoDefectsWithAI", "extractTextWithAI", "evalScriptWithAI",
    "travel", "addMedia", "toggleAirplaneMode",
];

fn null_or<T>(arg: &Yaml, f: impl FnOnce(&Yaml) -> Result<T, String>) -> Result<Option<T>, String> {
    if arg.is_null() { Ok(None) } else { f(arg).map(Some) }
}

fn millis(v: &Yaml) -> Result<Duration, String> {
    v.as_u64().map(Duration::from_millis).ok_or_else(|| "`timeout` is milliseconds".to_owned())
}

/// App id argument: absent, `stopApp: com.x`, or `{ appId: com.x }`.
fn app_arg(arg: &Yaml) -> Result<Option<String>, String> {
    match arg {
        Yaml::Mapping(m) => get(m, "appId").map(string).transpose(),
        other => null_or(other, string),
    }
}

fn steps_of(v: &Yaml, cx: &Cx) -> Result<Vec<FlowStep>, String> {
    let seq = v.as_sequence().ok_or("`commands` must be a list of steps")?;
    seq.iter()
        .enumerate()
        .map(|(i, s)| Step::parse(s, cx).map(|step| FlowStep { line: None, step }).map_err(|e| format!("command {}: {e}", i + 1)))
        .collect()
}

/// Steps of a sub-flow file (relative to the including flow's directory).
fn load(path: &str, cx: &Cx) -> Result<Vec<FlowStep>, String> {
    if cx.depth >= MAX_DEPTH {
        return Err(format!("sub-flows nest deeper than {MAX_DEPTH} levels (a cycle?)"));
    }
    if !(path.ends_with(".yaml") || path.ends_with(".yml")) {
        return Err(format!("`{path}`: sub-flows are .yaml / .yml files"));
    }
    let full = cx.base.join(path);
    let text = std::fs::read_to_string(&full).map_err(|e| format!("{}: {e}", full.display()))?;
    let sub = Cx { base: full.parent().map(Path::to_path_buf).unwrap_or_default(), depth: cx.depth + 1 };
    parse_with(&text, &sub)
        .map(|f| f.steps)
        .map_err(|errs| format!("{path}: {}", errs.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")))
}

impl Step {
    fn parse(v: &Yaml, cx: &Cx) -> Result<Self, String> {
        match v {
            Yaml::String(s) => Self::parse_command(s, &Yaml::Null, cx).map_err(|e| format!("{s}: {e}")),
            Yaml::Mapping(m) => {
                let mut it = m.iter();
                let (Some((k, arg)), None) = (it.next(), it.next()) else {
                    let keys: Vec<_> = m.keys().filter_map(Yaml::as_str).collect();
                    return Err(format!(
                        "a step has exactly one command, found: {}",
                        keys.join(", ")
                    ));
                };
                let cmd = k.as_str().ok_or("step command must be a string")?;
                Self::parse_command(cmd, arg, cx).map_err(|e| format!("{cmd}: {e}"))
            }
            _ => Err("a step must be a command string or a `command: args` map".into()),
        }
    }

    fn parse_command(cmd: &str, arg: &Yaml, cx: &Cx) -> Result<Self, String> {
        let none = || -> Result<(), String> { if arg.is_null() { Ok(()) } else { Err("takes no arguments".into()) } };
        Ok(match cmd {
            "openUrl" => Self::OpenUrl(string(arg)?),
            "openLink" => match arg {
                Yaml::Mapping(m) => Self::OpenUrl(string(get(m, "link").ok_or("missing `link`")?)?),
                other => Self::OpenUrl(string(other)?),
            },
            "longPressOn" => confirmable(arg, Self::LongPressOn(Selector::from_yaml(arg, &["confirm"])?))?,
            "doubleTapOn" => confirmable(arg, Self::DoubleTapOn(Selector::from_yaml(arg, &["confirm"])?))?,
            "scrollUntilVisible" => match arg {
                Yaml::Mapping(m) if get(m, "element").is_some() => Self::ScrollUntilVisible {
                    selector: Selector::from_yaml(get(m, "element").ok_or("missing `element`")?, &[])?,
                    direction: get(m, "direction").map(direction).transpose()?.unwrap_or(Direction::Down),
                    timeout: get(m, "timeout").map(millis).transpose()?,
                },
                other => Self::ScrollUntilVisible { selector: Selector::from_yaml(other, &[])?, direction: Direction::Down, timeout: None },
            },
            "hideKeyboard" => {
                none()?;
                Self::HideKeyboard
            }
            "eraseText" => Self::EraseText(match arg {
                Yaml::Mapping(m) => get(m, "charactersToErase").and_then(Yaml::as_u64).map(|n| n as usize),
                other => null_or(other, |v| v.as_u64().map(|n| n as usize).ok_or("expects a number of characters".into()))?,
            }),
            "stopApp" | "killApp" => Self::StopApp(app_arg(arg)?),
            "clearState" => Self::ClearState(app_arg(arg)?),
            "installApp" => Self::InstallApp(string(arg)?),
            "uninstallApp" => Self::UninstallApp(app_arg(arg)?.ok_or("expects the app id: `uninstallApp: com.example.app`")?),
            "clearKeychain" => {
                none()?;
                Self::ClearKeychain
            }
            "copyTextFrom" => Self::CopyTextFrom(Selector::from_yaml(arg, &[])?),
            "pasteText" => {
                none()?;
                Self::PasteText
            }
            "waitForAnimationToEnd" => Self::WaitForAnimationToEnd(match arg {
                Yaml::Mapping(m) => get(m, "timeout").map(millis).transpose()?,
                other => null_or(other, millis)?,
            }),
            "extendedWaitUntil" => {
                let m = arg.as_mapping().ok_or("expects { visible | notVisible: <selector>, timeout }")?;
                let timeout = get(m, "timeout").map(millis).transpose()?;
                match (get(m, "visible"), get(m, "notVisible")) {
                    (Some(s), None) => Self::AssertVisible { selector: Selector::from_yaml(s, &[])?, timeout },
                    (None, Some(s)) => Self::AssertNotVisible { selector: Selector::from_yaml(s, &[])?, timeout },
                    _ => return Err("expects exactly one of `visible` / `notVisible`".into()),
                }
            }
            "assertScreenshot" => match arg {
                Yaml::Mapping(m) => {
                    let frac = |k: &str| -> Result<Option<f64>, String> {
                        match get(m, k) {
                            None => Ok(None),
                            Some(v) => v.as_f64().filter(|f| (0.0..=1.0).contains(f)).map(Some).ok_or(format!("`{k}` is a number from 0 to 1")),
                        }
                    };
                    Self::AssertScreenshot {
                        name: string(get(m, "name").ok_or("missing `name`")?)?,
                        threshold: frac("threshold")?,
                        max_diff: frac("maxDiff")?,
                        ignore: match get(m, "ignore") {
                            None => Vec::new(),
                            Some(Yaml::Sequence(v)) => v.iter().map(|s| Selector::from_yaml(s, &[])).collect::<Result<_, _>>()?,
                            Some(one) => vec![Selector::from_yaml(one, &[])?],
                        },
                        update: get(m, "update").and_then(Yaml::as_bool).unwrap_or(false),
                    }
                }
                other => Self::AssertScreenshot { name: string(other)?, threshold: None, max_diff: None, ignore: Vec::new(), update: false },
            },
            "saveState" => Self::SaveState(string(arg)?),
            "loadState" => Self::LoadState(string(arg)?),
            "takeScreenshot" => Self::TakeScreenshot(match arg {
                Yaml::Mapping(m) => string(get(m, "path").ok_or("missing `path`")?)?,
                other => string(other)?,
            }),
            "inputRandomText" | "inputRandomNumber" | "inputRandomEmail" | "inputRandomPersonName" => {
                let kind = match cmd {
                    "inputRandomText" => Random::Text,
                    "inputRandomNumber" => Random::Number,
                    "inputRandomEmail" => Random::Email,
                    _ => Random::PersonName,
                };
                let length = match arg {
                    Yaml::Mapping(m) => get(m, "length").and_then(Yaml::as_u64).unwrap_or(8) as usize,
                    _ => 8,
                };
                Self::InputRandom { kind, length: length.clamp(1, 200) }
            }
            "runFlow" => match arg {
                Yaml::Mapping(m) => {
                    let when = get(m, "when").map(Condition::parse).transpose().map_err(|e| format!("when: {e}"))?;
                    let env = match get(m, "env") {
                        Some(Yaml::Mapping(e)) => e
                            .iter()
                            .map(|(k, v)| {
                                let k = k.as_str().ok_or("env names are strings")?.to_owned();
                                if !is_var(&k) {
                                    return Err(format!("env name `{k}` must be UPPER_SNAKE_CASE"));
                                }
                                let v = match v {
                                    Yaml::Number(n) => n.to_string(),
                                    other => string(other)?,
                                };
                                Ok((k, v))
                            })
                            .collect::<Result<_, String>>()?,
                        Some(_) => return Err("`env` must be a map".into()),
                        None => Vec::new(),
                    };
                    let (file, steps) = match (get(m, "file"), get(m, "commands")) {
                        (Some(f), None) => {
                            let f = string(f)?;
                            let steps = load(&f, cx)?;
                            (Some(f), steps)
                        }
                        (None, Some(c)) => (None, steps_of(c, cx)?),
                        _ => return Err("expects exactly one of `file` / `commands`".into()),
                    };
                    Self::RunFlow { file, when, env, steps }
                }
                other => {
                    let f = string(other)?;
                    Self::RunFlow { steps: load(&f, cx)?, file: Some(f), when: None, env: Vec::new() }
                }
            },
            "repeat" => {
                let m = arg.as_mapping().ok_or("expects { times | while, commands }")?;
                let times = get(m, "times")
                    .map(|t| match t {
                        Yaml::Number(n) => n.as_u64().map(|n| n as u32).ok_or("`times` is a count".to_owned()),
                        Yaml::String(s) => s.parse().map_err(|_| "`times` is a count".to_owned()),
                        _ => Err("`times` is a count".to_owned()),
                    })
                    .transpose()?;
                let while_ = get(m, "while").map(Condition::parse).transpose().map_err(|e| format!("while: {e}"))?;
                if times.is_none() && while_.is_none() {
                    return Err("needs `times` and/or `while`".into());
                }
                Self::Repeat { times, while_, steps: steps_of(get(m, "commands").ok_or("missing `commands`")?, cx)? }
            }
            "setViewport" => match arg {
                Yaml::String(p) => match Viewport::preset(p) {
                    Some(viewport) => Self::SetViewport { name: Some(p.clone()), viewport },
                    None => return Err(format!("unknown device `{p}` (iPhone 15, iPhone SE, iPhone Pro Max, Pixel 8, Galaxy S23, iPad, desktop)")),
                },
                Yaml::Mapping(m) => {
                    if let Some(p) = get(m, "device") {
                        let p = string(p)?;
                        let viewport = Viewport::preset(&p).ok_or(format!("unknown device `{p}`"))?;
                        Self::SetViewport { name: Some(p), viewport }
                    } else {
                        let n = |k: &str| get(m, k).and_then(Yaml::as_u64).map(|v| v as u32);
                        let width = n("width").ok_or("missing `width` (or use `device: iPhone 15`)")?;
                        let height = n("height").ok_or("missing `height`")?;
                        if !(100..=10_000).contains(&width) || !(100..=10_000).contains(&height) {
                            return Err("width/height must be 100..10000 CSS pixels".into());
                        }
                        let mobile = get(m, "mobile").and_then(Yaml::as_bool).unwrap_or(false);
                        Self::SetViewport {
                            name: None,
                            viewport: Some(Viewport {
                                width,
                                height,
                                scale: get(m, "deviceScaleFactor").and_then(Yaml::as_f64).unwrap_or(1.0),
                                mobile,
                                touch: get(m, "touch").and_then(Yaml::as_bool).unwrap_or(mobile),
                                user_agent: get(m, "userAgent").map(string).transpose()?,
                            }),
                        }
                    }
                }
                _ => return Err("expects a device name (`iPhone 15`, `desktop`) or { width, height, deviceScaleFactor, mobile, touch, userAgent }".into()),
            },
            "setLocation" => {
                let m = arg.as_mapping().ok_or("expects { latitude, longitude }")?;
                let num = |k: &str| -> Result<f64, String> {
                    match get(m, k).ok_or(format!("missing `{k}`"))? {
                        Yaml::Number(n) => n.as_f64().ok_or(format!("`{k}` is a number")),
                        Yaml::String(s) => s.parse().map_err(|_| format!("`{k}` is a number")),
                        _ => Err(format!("`{k}` is a number")),
                    }
                };
                let (latitude, longitude) = (num("latitude")?, num("longitude")?);
                if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
                    return Err("latitude must be within ±90 and longitude within ±180".into());
                }
                Self::SetLocation { latitude, longitude }
            }
            "setPermissions" => {
                let m = arg.as_mapping().ok_or("expects { appId?, permissions: { camera: allow, ... } }")?;
                let perms = get(m, "permissions").and_then(Yaml::as_mapping).ok_or("missing `permissions` map")?;
                let permissions = perms
                    .iter()
                    .map(|(k, v)| {
                        let name = k.as_str().ok_or("permission names are strings")?.to_owned();
                        let g = match v.as_str() {
                            Some("allow") => Grant::Allow,
                            Some("deny") => Grant::Deny,
                            Some("unset") => Grant::Unset,
                            _ => return Err(format!("`{name}`: allow, deny or unset")),
                        };
                        Ok((name, g))
                    })
                    .collect::<Result<_, String>>()?;
                Self::SetPermissions { app: get(m, "appId").map(string).transpose()?, permissions }
            }
            "setNetwork" => Self::SetNetwork(match arg {
                Yaml::String(s) if s == "offline" => Network { offline: true, ..Network::default() },
                Yaml::String(s) if s == "online" => Network::default(),
                Yaml::Mapping(m) => {
                    let n = |k: &str| get(m, k).and_then(Yaml::as_u64);
                    Network {
                        offline: get(m, "offline").and_then(Yaml::as_bool).unwrap_or(false),
                        latency_ms: n("latency"),
                        download_kbps: n("download"),
                        upload_kbps: n("upload"),
                    }
                }
                _ => return Err("expects offline, online, or { latency: ms, download: kbps, upload: kbps }".into()),
            }),
            "setAirplaneMode" => Self::SetNetwork(match arg.as_str() {
                Some("enabled") => Network { offline: true, ..Network::default() },
                Some("disabled") => Network::default(),
                _ => return Err("expects enabled or disabled".into()),
            }),
            "setLocale" => {
                let m = arg.as_mapping().ok_or("expects { locale: id-ID, timezone: Asia/Jakarta }")?;
                let locale = get(m, "locale").map(string).transpose()?;
                let timezone = get(m, "timezone").map(string).transpose()?;
                if locale.is_none() && timezone.is_none() {
                    return Err("needs `locale` and/or `timezone`".into());
                }
                Self::SetLocale { locale, timezone }
            }
            "pushNotification" => {
                let m = arg.as_mapping().ok_or("expects { appId?, title, body, payload? }")?;
                Self::PushNotification {
                    app: get(m, "appId").map(string).transpose()?,
                    title: string(get(m, "title").ok_or("missing `title`")?)?,
                    body: get(m, "body").map(string).transpose()?.unwrap_or_default(),
                    payload: get(m, "payload").map(json_of).transpose()?,
                }
            }
            "startRecording" => Self::StartRecording(match arg {
                Yaml::Mapping(m) => string(get(m, "path").ok_or("missing `path`")?)?,
                Yaml::Null => "recording".into(),
                other => string(other)?,
            }),
            "stopRecording" => {
                none()?;
                Self::StopRecording
            }
            "shipCheck" => Self::ShipCheck {
                links: match arg {
                    Yaml::Null => crate::ship::DEFAULT_LINKS,
                    Yaml::Mapping(m) => get(m, "links").map(|v| v.as_u64().map(|n| n as usize).ok_or("`links` is a number")).transpose()?.unwrap_or(crate::ship::DEFAULT_LINKS),
                    other => other.as_u64().map(|n| n as usize).ok_or("expects nothing, or { links: N, cookies: true }")?,
                }
                .min(crate::ship::MAX_LINKS),
                cookies: match arg {
                    Yaml::Mapping(m) => get(m, "cookies").map(|v| v.as_bool().ok_or("`cookies` is true or false")).transpose()?.unwrap_or(false),
                    _ => false,
                },
            },
            "httpRequest" => {
                let m = arg.as_mapping().ok_or("expects { method, url, headers, json, expect, capture, secret }")?;
                let method = get(m, "method").map(string).transpose()?.unwrap_or_else(|| "GET".into()).to_uppercase();
                if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"].contains(&method.as_str()) {
                    return Err(format!("unknown method `{method}`"));
                }
                let pairs = |key: &str, value: &dyn Fn(&Yaml) -> Result<String, String>| -> Result<Vec<(String, String)>, String> {
                    match get(m, key) {
                        None => Ok(Vec::new()),
                        Some(Yaml::Mapping(h)) => h
                            .iter()
                            .map(|(k, v)| Ok((k.as_str().ok_or(format!("`{key}` keys are strings"))?.to_owned(), value(v)?)))
                            .collect(),
                        Some(_) => Err(format!("`{key}` must be a map")),
                    }
                };
                let capture = pairs("capture", &|v| string(v))?;
                for (k, _) in &capture {
                    var_name(&Yaml::String(k.clone()))?;
                }
                let secret = match get(m, "secret") {
                    Some(Yaml::Sequence(v)) => v.iter().map(var_name).collect::<Result<_, _>>()?,
                    Some(_) => return Err("`secret` is a list of captured names".into()),
                    None => Vec::new(),
                };
                Self::HttpRequest(Box::new(HttpRequest {
                    method,
                    url: string(get(m, "url").ok_or("missing `url`")?)?,
                    headers: pairs("headers", &|v| match v {
                        Yaml::Number(n) => Ok(n.to_string()),
                        other => string(other),
                    })?,
                    json: get(m, "json").map(json_of).transpose()?,
                    expect: get(m, "expect").map(|e| e.as_u64().map(|n| n as u16).ok_or("`expect` is a status code")).transpose()?,
                    capture,
                    secret,
                }))
            }
            "readOtp" => {
                let m = arg.as_mapping().ok_or("expects { from: mailpit | mailhog | http | sms, url, to, after, length, into, timeout }")?;
                let source = match string(get(m, "from").ok_or("missing `from`")?)?.as_str() {
                    "mailpit" => OtpSource::Mailpit,
                    "mailhog" => OtpSource::Mailhog,
                    "http" => OtpSource::Http,
                    "sms" => OtpSource::Sms,
                    other => return Err(format!("unknown source `{other}` (mailpit, mailhog, http, sms)")),
                };
                let url = get(m, "url").map(string).transpose()?;
                if url.is_none() && source != OtpSource::Sms {
                    return Err("missing `url` (e.g. http://localhost:8025 for Mailpit)".into());
                }
                Self::ReadOtp(Box::new(ReadOtp {
                    source,
                    url,
                    to: get(m, "to").map(string).transpose()?,
                    after: get(m, "after").map(string).transpose()?,
                    length: get(m, "length").and_then(Yaml::as_u64).map(|n| n as usize),
                    into: get(m, "into").map(var_name).transpose()?.unwrap_or_else(|| "OTP".into()),
                    timeout: get(m, "timeout").map(millis).transpose()?,
                }))
            }
            "retry" => {
                let m = arg.as_mapping().ok_or("expects { maxRetries, commands | file }")?;
                let max = get(m, "maxRetries").and_then(Yaml::as_u64).unwrap_or(1).min(10) as u32;
                let steps = match (get(m, "commands"), get(m, "file")) {
                    (Some(c), None) => steps_of(c, cx)?,
                    (None, Some(f)) => load(&string(f)?, cx)?,
                    _ => return Err("expects exactly one of `commands` / `file`".into()),
                };
                Self::Retry { max, steps }
            }
            "tapOn" => confirmable(arg, Self::TapOn(Selector::from_yaml(arg, &["confirm"])?))?,
            "inputText" => match arg {
                Yaml::Mapping(m) => Self::InputText {
                    into: match get(m, "into") {
                        Some(sel) => Selector::from_yaml(sel, &[])?,
                        None => Selector::Focused,
                    },
                    text: string(get(m, "text").ok_or("missing `text`")?)?,
                },
                other => Self::InputText {
                    into: Selector::Focused,
                    text: string(other)?,
                },
            },
            "assertVisible" => Self::AssertVisible {
                selector: Selector::from_yaml(arg, &["timeout"])?,
                timeout: match arg {
                    Yaml::Mapping(m) => get(m, "timeout")
                        .map(|t| t.as_u64().map(Duration::from_millis).ok_or("`timeout` is milliseconds"))
                        .transpose()?,
                    _ => None,
                },
            },
            "scroll" => match arg {
                Yaml::Null => Self::Scroll {
                    direction: Direction::Down,
                    within: None,
                },
                Yaml::Mapping(m) => Self::Scroll {
                    direction: get(m, "direction").map(direction).transpose()?.unwrap_or(Direction::Down),
                    within: get(m, "within").map(|s| Selector::from_yaml(s, &[])).transpose()?,
                },
                other => Self::Scroll {
                    direction: direction(other)?,
                    within: None,
                },
            },
            "back" => match arg {
                Yaml::Null => Self::Back,
                _ => return Err("takes no arguments".into()),
            },
            "evalScript" => Self::EvalScript(string(arg)?),
            "assertNotVisible" => Self::AssertNotVisible {
                selector: Selector::from_yaml(arg, &["timeout"])?,
                timeout: match arg {
                    Yaml::Mapping(m) => get(m, "timeout")
                        .map(|t| t.as_u64().map(Duration::from_millis).ok_or("`timeout` is milliseconds"))
                        .transpose()?,
                    _ => None,
                },
            },
            // `{ key: Enter, confirm: true }`: Enter or Space that may press a confirm_actions button,
            // meant (the refusal says to write it; v0.3.2 lists hapus/delete/remove by default).
            "pressKey" => match arg.as_mapping() {
                Some(m) => {
                    let key = string(get(m, "key").ok_or("expects pressKey: Enter or { key: Enter, confirm: true }")?)?;
                    confirmable(arg, Self::PressKey(key))?
                }
                None => Self::PressKey(string(arg)?),
            },
            "selectOption" => {
                let m = arg.as_mapping().ok_or("expects { from: <selector>, option: \"...\" }")?;
                Self::SelectOption {
                    from: Selector::from_yaml(get(m, "from").ok_or("missing `from`")?, &[])?,
                    option: string(get(m, "option").ok_or("missing `option`")?)?,
                }
            }
            "uploadFile" => {
                let m = arg.as_mapping().ok_or("expects { into: <selector>, files: [paths] }")?;
                let files = match get(m, "files").ok_or("missing `files`")? {
                    Yaml::Sequence(s) => s.iter().map(string).collect::<Result<_, _>>()?,
                    other => vec![string(other)?],
                };
                Self::UploadFile { into: Selector::from_yaml(get(m, "into").ok_or("missing `into`")?, &[])?, files }
            }
            "launchApp" => match arg {
                Yaml::Mapping(m) => Self::LaunchApp { app: string(get(m, "appId").ok_or("missing `appId`")?)?, launch: launch_of(m)? },
                other => Self::LaunchApp { app: string(other)?, launch: crate::Launch::default() },
            },
            "swipe" => {
                let m = arg.as_mapping().ok_or("expects { from: [x, y], to: [x, y] }")?;
                let pt = |k: &str| -> Result<Point, String> {
                    let v = get(m, k).ok_or(format!("missing `{k}`"))?;
                    match Selector::from_yaml(&Yaml::Mapping([(Yaml::String("point".into()), v.clone())].into_iter().collect()), &[])? {
                        Selector::Point(p) => Ok(p),
                        _ => Err(format!("`{k}` must be [x, y]")),
                    }
                };
                Self::Swipe { from: pt("from")?, to: pt("to")? }
            }
            "session" => match arg {
                Yaml::Mapping(m) => Self::StartSession {
                    id: string(get(m, "start").ok_or("use `session: <id>` or `session: { start: <id>, platform, target }`")?)?,
                    platform: match get(m, "platform") {
                        Some(p) => {
                            let p = string(p)?;
                            Platform::parse(&p).ok_or(format!("unknown platform `{p}` (web, android, ios)"))?
                        }
                        None => Platform::Web,
                    },
                    target: get(m, "target").map(string).transpose()?,
                    reset: get(m, "reset").and_then(Yaml::as_bool).unwrap_or(false),
                    device: get(m, "device").map(string).transpose()?,
                },
                other => Self::UseSession(string(other)?),
            },
            "parallel" => {
                let seq = arg.as_sequence().ok_or("expects a list of { session, steps }")?;
                let branches = seq
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        let m = b.as_mapping().ok_or(format!("branch {}: expects {{ session, steps }}", i + 1))?;
                        let session = string(get(m, "session").ok_or(format!("branch {}: missing `session`", i + 1))?)?;
                        let steps = get(m, "steps")
                            .and_then(Yaml::as_sequence)
                            .ok_or(format!("branch {}: missing `steps` list", i + 1))?
                            .iter()
                            .enumerate()
                            .map(|(j, s)| {
                                Step::parse(s, cx)
                                    .map(|step| FlowStep { line: None, step })
                                    .map_err(|e| format!("branch {} step {}: {e}", i + 1, j + 1))
                            })
                            .collect::<Result<_, _>>()?;
                        Ok(Branch { session, steps })
                    })
                    .collect::<Result<_, String>>()?;
                Self::Parallel(branches)
            }
            other => return Err(unknown(other)),
        })
    }

    /// Context-free label used in reports (`${VARS}` stay unresolved, so secrets never echo back).
    pub fn label(&self) -> String {
        match self {
            Self::OpenUrl(u) => format!("openUrl {}", crate::quote(u)),
            Self::TapOn(s) => format!("tapOn {s}"),
            Self::Confirmed(inner) => format!("{} (confirmed)", inner.label()),
            Self::InputText { into, text } => format!("inputText {into} {}", crate::quote(text)),
            Self::AssertVisible { selector, .. } => format!("assertVisible {selector}"),
            Self::Scroll { direction, within: Some(w) } => format!("scroll {direction:?} within {w}"),
            Self::Scroll { direction, within: None } => format!("scroll {direction:?}"),
            Self::Back => "back".into(),
            Self::EvalScript(js) => format!("evalScript {}", crate::quote(js)),
            Self::AssertNotVisible { selector, .. } => format!("assertNotVisible {selector}"),
            Self::PressKey(k) => format!("pressKey {k}"),
            Self::SelectOption { from, option } => format!("selectOption {from} {}", crate::quote(option)),
            Self::UploadFile { into, files } => format!("uploadFile {into} {files:?}"),
            Self::LaunchApp { app, launch } => {
                let mut label = format!("launchApp {app}");
                if !launch.arguments.is_empty() {
                    let args: Vec<String> = launch.arguments.iter().map(|(k, v)| format!("{k}={}", arg_text(v))).collect();
                    label.push_str(&format!(" arguments {{{}}}", args.join(", ")));
                }
                if !launch.env.is_empty() {
                    // Values never shown: env is where apps take tokens and keys.
                    let env: Vec<String> = launch.env.iter().map(|(k, _)| format!("{k}=***")).collect();
                    label.push_str(&format!(" env {{{}}}", env.join(", ")));
                }
                label
            }
            Self::Swipe { from, to } => format!("swipe [{}, {}] -> [{}, {}]", from.x, from.y, to.x, to.y),
            Self::UseSession(id) => format!("session {id}"),
            Self::StartSession { id, platform, .. } => format!("session start {id} ({platform})"),
            Self::Parallel(b) => format!("parallel ({} branches)", b.len()),
            Self::LongPressOn(s) => format!("longPressOn {s}"),
            Self::DoubleTapOn(s) => format!("doubleTapOn {s}"),
            Self::ScrollUntilVisible { selector, direction, .. } => format!("scrollUntilVisible {selector} {direction:?}"),
            Self::HideKeyboard => "hideKeyboard".into(),
            Self::EraseText(Some(n)) => format!("eraseText {n}"),
            Self::EraseText(None) => "eraseText".into(),
            Self::StopApp(a) => format!("stopApp{}", a.as_deref().map(|a| format!(" {a}")).unwrap_or_default()),
            Self::ClearState(a) => format!("clearState{}", a.as_deref().map(|a| format!(" {a}")).unwrap_or_default()),
            Self::ClearKeychain => "clearKeychain".into(),
            Self::InstallApp(p) => format!("installApp {}", crate::quote(p)),
            Self::UninstallApp(a) => format!("uninstallApp {a}"),
            Self::CopyTextFrom(s) => format!("copyTextFrom {s}"),
            Self::PasteText => "pasteText".into(),
            Self::WaitForAnimationToEnd(_) => "waitForAnimationToEnd".into(),
            Self::TakeScreenshot(n) => format!("takeScreenshot {n}"),
            Self::AssertScreenshot { name, .. } => format!("assertScreenshot {name}"),
            Self::SaveState(n) => format!("saveState {n}"),
            Self::LoadState(n) => format!("loadState {n}"),
            Self::InputRandom { kind, .. } => kind.command().into(),
            Self::RunFlow { file, when, steps, .. } => {
                let what = file.clone().unwrap_or_else(|| format!("({} steps)", steps.len()));
                let when = when.as_ref().map(|c| format!(" when {c}")).unwrap_or_default();
                format!("runFlow {what}{when}")
            }
            Self::Repeat { times, while_, steps } => {
                let times = times.map(|t| format!(" {t}x")).unwrap_or_default();
                let cond = while_.as_ref().map(|c| format!(" while {c}")).unwrap_or_default();
                format!("repeat{times}{cond} ({} steps)", steps.len())
            }
            Self::Retry { max, steps } => format!("retry {max}x ({} steps)", steps.len()),
            Self::HttpRequest(h) => format!("httpRequest {} {}", h.method, h.url),
            Self::ReadOtp(o) => format!("readOtp {} -> {}", o.source.name(), o.into),
            Self::StartRecording(n) => format!("startRecording {n}"),
            Self::StopRecording => "stopRecording".into(),
            Self::ShipCheck { links, cookies: false } if *links == crate::ship::DEFAULT_LINKS => "shipCheck".into(),
            Self::ShipCheck { links, cookies } => format!("shipCheck links {links}{}", if *cookies { " signed in" } else { "" }),
            Self::SetLocation { latitude, longitude } => format!("setLocation {latitude},{longitude}"),
            Self::SetViewport { name: Some(n), .. } => format!("setViewport {n}"),
            Self::SetViewport { viewport: Some(v), .. } => format!("setViewport {}x{}@{}", v.width, v.height, v.scale),
            Self::SetViewport { .. } => "setViewport desktop".into(),
            Self::SetPermissions { permissions, .. } => {
                let p: Vec<String> = permissions.iter().map(|(n, g)| format!("{n}={}", grant_name(*g))).collect();
                format!("setPermissions {}", p.join(" "))
            }
            Self::SetNetwork(n) if n.offline => "setNetwork offline".into(),
            Self::SetNetwork(n) if *n == Network::default() => "setNetwork online".into(),
            Self::SetNetwork(n) => format!(
                "setNetwork latency={}ms down={}kbps up={}kbps",
                n.latency_ms.unwrap_or(0),
                n.download_kbps.map_or("-".into(), |v| v.to_string()),
                n.upload_kbps.map_or("-".into(), |v| v.to_string())
            ),
            Self::SetLocale { locale, timezone } => {
                format!("setLocale {} {}", locale.as_deref().unwrap_or("-"), timezone.as_deref().unwrap_or("-"))
            }
            Self::PushNotification { title, .. } => format!("pushNotification {}", crate::quote(title)),
        }
    }

    /// Steps that do not drive a screen (run before any session is started).
    pub fn sessionless(&self) -> bool {
        match self {
            Self::HttpRequest(_) => true,
            Self::ReadOtp(o) => o.source != OtpSource::Sms,
            _ => false,
        }
    }

    /// Steps nested in a block (runFlow, repeat, retry).
    pub fn children(&self) -> &[FlowStep] {
        match self {
            Self::RunFlow { steps, .. } | Self::Repeat { steps, .. } | Self::Retry { steps, .. } => steps,
            _ => &[],
        }
    }

    /// `${VARS}` this step itself reads (not those of nested steps).
    fn vars(&self) -> Vec<String> {
        let texts: Vec<&str> = match self {
            Self::OpenUrl(s) | Self::EvalScript(s) => vec![s],
            Self::InputText { text, .. } => vec![text],
            Self::StartSession { target: Some(t), .. } => vec![t],
            Self::StopApp(Some(a)) | Self::ClearState(Some(a)) | Self::InstallApp(a) | Self::UninstallApp(a) => vec![a],
            Self::LaunchApp { app, launch } => std::iter::once(app.as_str())
                .chain(launch.env.iter().map(|(_, v)| v.as_str()))
                .chain(launch.arguments.iter().flat_map(|(_, v)| match v {
                    crate::LaunchArg::Str(t) => vec![t.as_str()],
                    crate::LaunchArg::List(l) => l.iter().map(String::as_str).collect(),
                    _ => vec![],
                }))
                .collect(),
            Self::RunFlow { env, .. } => env.iter().map(|(_, v)| v.as_str()).collect(),
            Self::HttpRequest(h) => {
                let mut names: Vec<String> = std::iter::once(h.url.as_str())
                    .chain(h.headers.iter().map(|(_, v)| v.as_str()))
                    .flat_map(var_names)
                    .collect();
                if let Some(j) = &h.json {
                    names.extend(var_names(&j.to_string()));
                }
                return names;
            }
            Self::ReadOtp(o) => o.url.iter().chain(&o.to).map(String::as_str).collect(),
            _ => vec![],
        };
        texts.into_iter().flat_map(var_names).collect()
    }

    /// Variables this step defines for the steps after it / inside it.
    fn defines(&self) -> Vec<String> {
        match self {
            Self::RunFlow { env, .. } => env.iter().map(|(k, _)| k.clone()).collect(),
            Self::HttpRequest(h) => h.capture.iter().map(|(k, _)| k.clone()).collect(),
            Self::ReadOtp(o) => vec![o.into.clone()],
            _ => vec![],
        }
    }
}

/// What a literal `launchApp` env value becomes in recorded flows.
pub(crate) const MASKED: &str = "***";

fn arg_text(v: &crate::LaunchArg) -> String {
    match v {
        crate::LaunchArg::Str(t) => crate::quote(t),
        crate::LaunchArg::Int(n) => n.to_string(),
        crate::LaunchArg::Bool(b) => b.to_string(),
        crate::LaunchArg::List(l) => format!("[{}]", l.iter().map(|t| crate::quote(t)).collect::<Vec<_>>().join(", ")),
    }
}

/// `arguments` and `env` of a `launchApp` mapping.
fn launch_of(m: &Mapping) -> Result<crate::Launch, String> {
    let mut launch = crate::Launch::default();
    match get(m, "arguments") {
        Some(Yaml::Mapping(a)) => {
            for (k, v) in a {
                let k = k.as_str().ok_or("argument names are strings")?.to_owned();
                if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
                    return Err(format!("argument name `{k}` must be letters, digits, `.`, `_` or `-`"));
                }
                let v = match v {
                    Yaml::Bool(b) => crate::LaunchArg::Bool(*b),
                    Yaml::Number(n) => crate::LaunchArg::Int(n.as_i64().ok_or(format!("argument `{k}`: whole numbers only (write \"{n}\" for text)"))?),
                    Yaml::Sequence(items) => crate::LaunchArg::List(
                        items.iter().map(|i| match i {
                            Yaml::Number(n) => Ok(n.to_string()),
                            Yaml::Bool(b) => Ok(b.to_string()),
                            other => string(other),
                        }).collect::<Result<_, _>>().map_err(|e| format!("argument `{k}`: {e}"))?,
                    ),
                    other => crate::LaunchArg::Str(string(other).map_err(|e| format!("argument `{k}`: {e}"))?),
                };
                launch.arguments.push((k, v));
            }
        }
        Some(Yaml::Null) | None => {}
        Some(_) => return Err("`arguments` must be a map (name: value)".into()),
    }
    match get(m, "env") {
        Some(Yaml::Mapping(e)) => {
            for (k, v) in e {
                let k = k.as_str().ok_or("env names are strings")?.to_owned();
                if let Some(why) = crate::launch_env_refused(&k) {
                    return Err(why);
                }
                let v = match v {
                    Yaml::Number(n) => n.to_string(),
                    Yaml::Bool(b) => b.to_string(),
                    other => string(other).map_err(|e| format!("env `{k}`: {e}"))?,
                };
                launch.env.push((k, v));
            }
        }
        Some(Yaml::Null) | None => {}
        Some(_) => return Err("`env` must be a map (NAME: value)".into()),
    }
    if let Some(k) = m.keys().filter_map(Yaml::as_str).find(|k| !["appId", "arguments", "env"].contains(k)) {
        return Err(format!("unknown launchApp field `{k}` (appId, arguments, env)"));
    }
    Ok(launch)
}

fn map(pairs: Vec<(&str, Yaml)>) -> Yaml {
    Yaml::Mapping(
        pairs
            .into_iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| (Yaml::String(k.into()), v))
            .collect(),
    )
}

fn with_timeout(sel: Yaml, timeout: Option<Duration>) -> Yaml {
    let Some(t) = timeout else { return sel };
    let t = Yaml::Number((t.as_millis() as u64).into());
    match sel {
        Yaml::Mapping(mut m) => {
            m.insert(Yaml::String("timeout".into()), t);
            Yaml::Mapping(m)
        }
        other => map(vec![("text", other), ("timeout", t)]),
    }
}

/// `confirm: true` in a tap-like step's selector mapping wraps it in `Step::Confirmed`.
fn confirmable(arg: &Yaml, step: Step) -> Result<Step, String> {
    match arg.as_mapping().and_then(|m| get(m, "confirm")) {
        None | Some(Yaml::Bool(false)) => Ok(step),
        Some(Yaml::Bool(true)) => Ok(Step::Confirmed(Box::new(step))),
        Some(_) => Err("`confirm` is true or false".into()),
    }
}

impl Step {
    /// The step itself, without a `confirm: true` wrapper.
    pub fn unconfirmed(&self) -> &Step {
        match self {
            Self::Confirmed(inner) => inner.unconfirmed(),
            other => other,
        }
    }

    /// DSL YAML for this step; `durable` replaces the selector of leaf `n` (refs -> role+name).
    fn to_yaml(&self, leaf: &mut usize, durable: &HashMap<usize, Selector>) -> Yaml {
        let s = |x: &str| Yaml::String(x.to_owned());
        let one = |k: &str, v: Yaml| map(vec![(k, v)]);
        if let Self::Parallel(branches) = self {
            let b = branches
                .iter()
                .map(|b| {
                    let steps = b.steps.iter().map(|st| st.step.to_yaml(leaf, durable)).collect();
                    map(vec![("session", s(&b.session)), ("steps", Yaml::Sequence(steps))])
                })
                .collect();
            return one("parallel", Yaml::Sequence(b));
        }
        *leaf += 1;
        let n = *leaf;
        let sel = |x: &Selector| durable.get(&n).unwrap_or(x).to_yaml();
        let pt = |p: &Point| Yaml::Sequence(vec![p.x.into(), p.y.into()]);
        match self {
            Self::OpenUrl(u) => one("openUrl", s(u)),
            Self::TapOn(x) => one("tapOn", sel(x)),
            Self::Confirmed(inner) => {
                // `{ tapOn: <sel> }` → `{ tapOn: { ...sel, confirm: true } }`
                let Yaml::Mapping(m) = inner.to_yaml(leaf, durable) else { return Yaml::Null };
                let pairs: Vec<(Yaml, Yaml)> = m
                    .into_iter()
                    .map(|(k, v)| {
                        let field = if k.as_str() == Some("pressKey") { "key" } else { "text" };
                        let mut v = match v {
                            Yaml::Mapping(v) => v,
                            other => [(Yaml::String(field.into()), other)].into_iter().collect(),
                        };
                        v.insert(Yaml::String("confirm".into()), Yaml::Bool(true));
                        (k, Yaml::Mapping(v))
                    })
                    .collect();
                Yaml::Mapping(pairs.into_iter().collect())
            }
            Self::InputText { into: Selector::Focused, text } => one("inputText", s(text)),
            Self::InputText { into, text } => one("inputText", map(vec![("into", sel(into)), ("text", s(text))])),
            Self::AssertVisible { selector, timeout } => one("assertVisible", with_timeout(sel(selector), *timeout)),
            Self::AssertNotVisible { selector, timeout } => one("assertNotVisible", with_timeout(sel(selector), *timeout)),
            Self::Scroll { direction, within } => one(
                "scroll",
                map(vec![
                    ("direction", s(&format!("{direction:?}").to_lowercase())),
                    ("within", within.as_ref().map(sel).unwrap_or(Yaml::Null)),
                ]),
            ),
            Self::Back => s("back"),
            Self::EvalScript(js) => one("evalScript", s(js)),
            Self::PressKey(k) => one("pressKey", s(k)),
            Self::SelectOption { from, option } => one("selectOption", map(vec![("from", sel(from)), ("option", s(option))])),
            Self::UploadFile { into, files } => one(
                "uploadFile",
                map(vec![("into", sel(into)), ("files", Yaml::Sequence(files.iter().map(|f| s(f)).collect()))]),
            ),
            Self::LaunchApp { app, launch } if launch.is_empty() => one("launchApp", s(app)),
            Self::LaunchApp { app, launch } => {
                let arg = |v: &crate::LaunchArg| match v {
                    crate::LaunchArg::Str(t) => s(t),
                    crate::LaunchArg::Int(n) => Yaml::Number((*n).into()),
                    crate::LaunchArg::Bool(b) => Yaml::Bool(*b),
                    crate::LaunchArg::List(l) => Yaml::Sequence(l.iter().map(|t| s(t)).collect()),
                };
                let arguments = launch.arguments.iter().map(|(k, v)| (s(k), arg(v))).collect::<Mapping>();
                // Recorded runs and hints keep `${NAME}` references but never literal env values.
                let env = launch.env.iter().map(|(k, v)| (s(k), s(if v.contains("${") { v } else { MASKED }))).collect::<Mapping>();
                one(
                    "launchApp",
                    map(vec![
                        ("appId", s(app)),
                        ("arguments", if arguments.is_empty() { Yaml::Null } else { Yaml::Mapping(arguments) }),
                        ("env", if env.is_empty() { Yaml::Null } else { Yaml::Mapping(env) }),
                    ]),
                )
            }
            Self::Swipe { from, to } => one("swipe", map(vec![("from", pt(from)), ("to", pt(to))])),
            Self::UseSession(id) => one("session", s(id)),
            Self::StartSession { id, platform, target, reset, device } => one(
                "session",
                map(vec![
                    ("start", s(id)),
                    ("platform", s(&platform.to_string())),
                    ("target", target.as_deref().map(s).unwrap_or(Yaml::Null)),
                    ("reset", if *reset { Yaml::Bool(true) } else { Yaml::Null }),
                    ("device", device.as_deref().map(s).unwrap_or(Yaml::Null)),
                ]),
            ),
            Self::Parallel(_) => Yaml::Null, // handled above
            Self::LongPressOn(x) => one("longPressOn", sel(x)),
            Self::DoubleTapOn(x) => one("doubleTapOn", sel(x)),
            Self::ScrollUntilVisible { selector, direction, timeout } => one(
                "scrollUntilVisible",
                map(vec![
                    ("element", sel(selector)),
                    ("direction", s(&format!("{direction:?}").to_lowercase())),
                    ("timeout", timeout.map(|t| Yaml::Number((t.as_millis() as u64).into())).unwrap_or(Yaml::Null)),
                ]),
            ),
            Self::HideKeyboard => s("hideKeyboard"),
            Self::EraseText(Some(n)) => one("eraseText", Yaml::Number((*n as u64).into())),
            Self::EraseText(None) => s("eraseText"),
            Self::StopApp(Some(a)) => one("stopApp", s(a)),
            Self::StopApp(None) => s("stopApp"),
            Self::ClearState(Some(a)) => one("clearState", s(a)),
            Self::ClearState(None) => s("clearState"),
            Self::InstallApp(p) => one("installApp", s(p)),
            Self::UninstallApp(a) => one("uninstallApp", s(a)),
            Self::ClearKeychain => s("clearKeychain"),
            Self::CopyTextFrom(x) => one("copyTextFrom", sel(x)),
            Self::PasteText => s("pasteText"),
            Self::WaitForAnimationToEnd(Some(t)) => one("waitForAnimationToEnd", map(vec![("timeout", Yaml::Number((t.as_millis() as u64).into()))])),
            Self::WaitForAnimationToEnd(None) => s("waitForAnimationToEnd"),
            Self::TakeScreenshot(n) => one("takeScreenshot", s(n)),
            Self::AssertScreenshot { name, threshold, max_diff, ignore, update } => {
                let mut pairs = vec![("name", s(name))];
                if let Some(t) = threshold {
                    pairs.push(("threshold", Yaml::Number((*t).into())));
                }
                if let Some(t) = max_diff {
                    pairs.push(("maxDiff", Yaml::Number((*t).into())));
                }
                if !ignore.is_empty() {
                    pairs.push(("ignore", Yaml::Sequence(ignore.iter().map(Selector::to_yaml).collect())));
                }
                if *update {
                    pairs.push(("update", Yaml::Bool(true)));
                }
                one("assertScreenshot", map(pairs))
            }
            Self::SaveState(n) => one("saveState", s(n)),
            Self::LoadState(n) => one("loadState", s(n)),
            Self::InputRandom { kind, length } => one(kind.command(), map(vec![("length", Yaml::Number((*length as u64).into()))])),
            Self::RunFlow { file, when, env, steps } => {
                let env = if env.is_empty() {
                    Yaml::Null
                } else {
                    Yaml::Mapping(env.iter().map(|(k, v)| (s(k), s(v))).collect())
                };
                match (file, when, &env) {
                    (Some(f), None, Yaml::Null) => one("runFlow", s(f)),
                    _ => one(
                        "runFlow",
                        map(vec![
                            ("when", when.as_ref().map(Condition::to_yaml).unwrap_or(Yaml::Null)),
                            ("env", env),
                            ("file", file.as_deref().map(s).unwrap_or(Yaml::Null)),
                            ("commands", if file.is_some() { Yaml::Null } else { block(steps) }),
                        ]),
                    ),
                }
            }
            Self::Repeat { times, while_, steps } => one(
                "repeat",
                map(vec![
                    ("times", times.map(|t| Yaml::Number(u64::from(t).into())).unwrap_or(Yaml::Null)),
                    ("while", while_.as_ref().map(Condition::to_yaml).unwrap_or(Yaml::Null)),
                    ("commands", block(steps)),
                ]),
            ),
            Self::Retry { max, steps } => {
                one("retry", map(vec![("maxRetries", Yaml::Number(u64::from(*max).into())), ("commands", block(steps))]))
            }
            Self::StartRecording(n) => one("startRecording", s(n)),
            Self::StopRecording => s("stopRecording"),
            Self::ShipCheck { links, cookies: false } if *links == crate::ship::DEFAULT_LINKS => s("shipCheck"),
            Self::ShipCheck { links, cookies } => one("shipCheck", map(vec![("links", Yaml::Number((*links as u64).into())), ("cookies", Yaml::Bool(*cookies))])),
            Self::SetLocation { latitude, longitude } => {
                one("setLocation", map(vec![("latitude", (*latitude).into()), ("longitude", (*longitude).into())]))
            }
            Self::SetViewport { name: Some(n), .. } => one("setViewport", s(n)),
            Self::SetViewport { viewport: Some(v), .. } => one(
                "setViewport",
                map(vec![
                    ("width", Yaml::Number(u64::from(v.width).into())),
                    ("height", Yaml::Number(u64::from(v.height).into())),
                    ("deviceScaleFactor", v.scale.into()),
                    ("mobile", Yaml::Bool(v.mobile)),
                    ("touch", Yaml::Bool(v.touch)),
                    ("userAgent", v.user_agent.as_deref().map(s).unwrap_or(Yaml::Null)),
                ]),
            ),
            Self::SetViewport { .. } => one("setViewport", s("desktop")),
            Self::SetPermissions { app, permissions } => one(
                "setPermissions",
                map(vec![
                    ("appId", app.as_deref().map(s).unwrap_or(Yaml::Null)),
                    ("permissions", Yaml::Mapping(permissions.iter().map(|(n, g)| (s(n), s(grant_name(*g)))).collect())),
                ]),
            ),
            Self::SetNetwork(n) if n.offline => one("setNetwork", s("offline")),
            Self::SetNetwork(n) if *n == Network::default() => one("setNetwork", s("online")),
            Self::SetNetwork(n) => {
                let v = |x: Option<u64>| x.map(|v| Yaml::Number(v.into())).unwrap_or(Yaml::Null);
                one("setNetwork", map(vec![("latency", v(n.latency_ms)), ("download", v(n.download_kbps)), ("upload", v(n.upload_kbps))]))
            }
            Self::SetLocale { locale, timezone } => one(
                "setLocale",
                map(vec![
                    ("locale", locale.as_deref().map(s).unwrap_or(Yaml::Null)),
                    ("timezone", timezone.as_deref().map(s).unwrap_or(Yaml::Null)),
                ]),
            ),
            Self::PushNotification { app, title, body, payload } => one(
                "pushNotification",
                map(vec![
                    ("appId", app.as_deref().map(s).unwrap_or(Yaml::Null)),
                    ("title", s(title)),
                    ("body", s(body)),
                    ("payload", payload.as_ref().and_then(|p| serde_yaml::to_value(p).ok()).unwrap_or(Yaml::Null)),
                ]),
            ),
            Self::HttpRequest(h) => {
                let kv = |v: &[(String, String)]| {
                    if v.is_empty() { Yaml::Null } else { Yaml::Mapping(v.iter().map(|(k, x)| (s(k), s(x))).collect()) }
                };
                one(
                    "httpRequest",
                    map(vec![
                        ("method", s(&h.method)),
                        ("url", s(&h.url)),
                        ("headers", kv(&h.headers)),
                        ("json", h.json.as_ref().and_then(|j| serde_yaml::to_value(j).ok()).unwrap_or(Yaml::Null)),
                        ("expect", h.expect.map(|e| Yaml::Number(u64::from(e).into())).unwrap_or(Yaml::Null)),
                        ("capture", kv(&h.capture)),
                        (
                            "secret",
                            if h.secret.is_empty() { Yaml::Null } else { Yaml::Sequence(h.secret.iter().map(|x| s(x)).collect()) },
                        ),
                    ]),
                )
            }
            Self::ReadOtp(o) => {
                let opt = |v: &Option<String>| v.as_deref().map(s).unwrap_or(Yaml::Null);
                one(
                    "readOtp",
                    map(vec![
                        ("from", s(o.source.name())),
                        ("url", opt(&o.url)),
                        ("to", opt(&o.to)),
                        ("after", opt(&o.after)),
                        ("length", o.length.map(|n| Yaml::Number((n as u64).into())).unwrap_or(Yaml::Null)),
                        ("into", s(&o.into)),
                        ("timeout", o.timeout.map(|t| Yaml::Number((t.as_millis() as u64).into())).unwrap_or(Yaml::Null)),
                    ]),
                )
            }
        }
    }
}

/// Nested steps keep their own selectors (path memory only rewrites top-level refs).
fn block(steps: &[FlowStep]) -> Yaml {
    let mut leaf = 0;
    Yaml::Sequence(steps.iter().map(|st| st.step.to_yaml(&mut leaf, &HashMap::new())).collect())
}

/// Serialize a flow back to DSL YAML, replacing selectors of the given leaf steps (1-based,
/// same numbering as `StepRecord::index`).
pub fn flow_to_yaml(flow: &Flow, durable: &HashMap<usize, Selector>) -> String {
    let mut leaf = 0;
    let seq = Yaml::Sequence(flow.steps.iter().map(|s| s.step.to_yaml(&mut leaf, durable)).collect());
    serde_yaml::to_string(&seq).unwrap_or_default()
}

/// QA B8: the names an AI guesses first (Playwright/Cypress/Selenium habits), and fastpilot's step.
const GUESSES: &[(&[&str], &str)] = &[
    (&["fill", "type", "typeText", "enterText", "setText", "input", "write"], "inputText: { into: \"Email\", text: \"a@b.com\" }"),
    (&["tap", "click", "press", "clickOn", "tapText", "select"], "tapOn: \"Login\""),
    (&["goto", "navigate", "open", "visit", "go", "url", "openURL", "navigateTo"], "openUrl: https://example.com"),
    (&["expect", "see", "assert", "assertText", "check", "verify", "waitFor", "wait", "waitForText"], "assertVisible: \"Dashboard\" (or { text: \"Dashboard\", timeout: 10000 })"),
    (&["key", "keyPress", "pressEnter"], "pressKey: Enter"),
    (&["screenshot"], "takeScreenshot: name"),
];

fn unknown(cmd: &str) -> String {
    if let Some((_, step)) = GUESSES.iter().find(|(names, _)| names.iter().any(|n| n.eq_ignore_ascii_case(cmd))) {
        return format!("unknown step `{cmd}`: in fastpilot write `{step}`");
    }
    if MAESTRO_LATER.contains(&cmd) {
        format!("Maestro command `{cmd}` is not supported yet")
    } else {
        format!(
            "unknown step `{cmd}` (supported: launchApp, openUrl, openLink, tapOn, longPressOn, doubleTapOn, inputText, eraseText, \
             hideKeyboard, copyTextFrom, pasteText, assertVisible, assertNotVisible, extendedWaitUntil, scroll, \
             scrollUntilVisible, swipe, back, pressKey, selectOption, uploadFile, evalScript, stopApp, killApp, \
             clearState, clearKeychain, installApp, uninstallApp, waitForAnimationToEnd, takeScreenshot, inputRandom*, runFlow, repeat, \
             retry, session, parallel)"
        )
    }
}

fn get<'a>(m: &'a Mapping, key: &str) -> Option<&'a Yaml> {
    m.get(Yaml::String(key.into()))
}

fn string(v: &Yaml) -> Result<String, String> {
    match v {
        Yaml::String(s) => Ok(s.clone()),
        Yaml::Number(_) => Err(crate::selector::QUOTE.into()),
        _ => Err("expects a string".into()),
    }
}

fn direction(v: &Yaml) -> Result<Direction, String> {
    let s = string(v)?;
    Direction::parse(&s).ok_or(format!("unknown direction `{s}` (up, down, left, right)"))
}

/// Single pass over `${NAME}` (NAME = `[A-Z_][A-Z0-9_]*`); `$${` is a literal `${`, and
/// anything else (JS template literals like `${a.b}`) is left untouched.
fn expand(s: &str, mut var: impl FnMut(&str) -> Result<String, String>) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(after) = tail.strip_prefix("$${") {
            out.push_str("${");
            rest = after;
            continue;
        }
        let name = tail.strip_prefix("${").and_then(|t| t.find('}').map(|end| &t[..end]));
        match name.filter(|n| is_var(n)) {
            Some(n) => {
                out.push_str(&var(n)?);
                rest = &tail[n.len() + 3..];
            }
            None => {
                out.push('$');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// E1: a `${NAME}` that holds a one-time code (it changes on every sign-in, so it is never a saved
/// secret): the name says so (`LOGIN_OTP`, `SMS_CODE`, `PIN`), or `text`, the step it is in, does
/// ("Type the one-time code", "Isi kode OTP").
pub fn is_code(name: &str, text: &str) -> bool {
    let parts: Vec<&str> = name.split('_').filter(|p| !p.is_empty()).collect();
    let by_name = parts.last().is_some_and(|p| ["OTP", "TOTP", "CODE", "PIN", "MFA", "2FA"].contains(p)) || parts.first() == Some(&"OTP");
    let words: Vec<String> = text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    let says = |phrase: &[&str]| words.windows(phrase.len()).any(|w| w.iter().zip(phrase).all(|(a, b)| a == b));
    // Security #229 L2: a step's words never make a password, token or key a code.
    let secret_name = ["PASS", "TOKEN", "SECRET", "KEY"].iter().any(|w| name.contains(w));
    by_name || !secret_name && [&["otp"][..], &["one", "time", "code"], &["onetime", "code"], &["verification", "code"], &["kode"], &["2fa"]].iter().any(|p| says(p))
}

/// E1: what a run says when it reaches a one-time code it cannot get (never "setup needed: the
/// test needs the password", which makes a password card).
pub fn code_needed(name: &str) -> String {
    format!(
        "one-time code needed: the step needs the one-time code {name}, sent when signing in. It is never a saved password. A person types it while the test runs in the dashboard, or the test reads it from a test inbox or the emulator's SMS (readOtp); nothing was typed"
    )
}

fn is_var(n: &str) -> bool {
    let mut chars = n.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

pub(crate) fn var_names(s: &str) -> Vec<String> {
    let mut names = Vec::new();
    let _ = expand(s, |n| {
        names.push(n.to_owned());
        Ok(String::new())
    });
    names
}

/// Variables a flow may read: only those listed in `FASTPILOT_VARS` (comma separated), so a
/// flow (or a prompt-injected AI) cannot pull arbitrary secrets out of the process environment.
fn allowed() -> Vec<String> {
    crate::config::var("FASTPILOT_VARS")
        .unwrap_or_default()
        .split(',')
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .collect()
}

fn lookup(name: &str) -> Result<String, String> {
    // QA #5980 bug 1: a value the person saved for the current app (Secrets, the inline password
    // dialog, Map's sign-in) is allowed by being saved; the list (FASTPILOT_VARS) is for values
    // from the environment.
    let saved = crate::secrets::lookup(name);
    if saved.is_none() && !allowed().iter().any(|a| a == name) {
        if is_code(name, "") {
            return Err(code_needed(name));
        }
        return Err(format!(
            "setup needed: the test needs the password {name} and it is not saved for this app. The user sets it once in the dashboard (ask for it in Chat, or Settings › Secrets), or lists it in FASTPILOT_VARS and gives it in the environment fastpilot starts in; nothing was typed"
        ));
    }
    // E2: the environment first (CI, power users), then the current app's saved value. A saved
    // value is masked in output from now on, like an environment one.
    if allowed().iter().any(|a| a == name)
        && let Ok(v) = std::env::var(name)
    {
        return Ok(v);
    }
    match saved {
        Some(v) => {
            add_secret(&v);
            let mut kept = APP_SECRETS.lock().unwrap_or_else(|p| p.into_inner());
            if !kept.contains(&v) {
                kept.push(v.clone());
            }
            Ok(v)
        }
        None if is_code(name, "") => Err(code_needed(name)),
        None => Err(format!(
            "setup needed: the test needs the password {name} and it is not set for this app. The user sets it once in the dashboard (Settings › Secrets › {name} › Set value), or sets it in the environment fastpilot starts in; nothing was typed"
        )),
    }
}

/// Values read from the current app's key store (E2), whatever local name they are copied to
/// later (`runFlow` env, a captured variable): S-1 follows the value, not its name.
static APP_SECRETS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// S-1: does this (already substituted) text hold a password saved for the app? A value from the
/// environment fastpilot starts in is not one: it keeps the older rules (allowed domains).
pub(crate) fn holds_app_secret(text: &str) -> bool {
    APP_SECRETS.lock().unwrap_or_else(|p| p.into_inner()).iter().any(|v| !v.is_empty() && text.contains(v.as_str()))
}

/// Replace `${NAME}` with allowed environment values of fastpilot's own process.
pub(crate) fn substitute(s: &str) -> Result<String, String> {
    expand(s, lookup)
}

/// Like `substitute`, flow-local variables (runFlow `env`, captures) first.
pub(crate) fn substitute_with(s: &str, local: &HashMap<String, String>) -> Result<String, String> {
    expand(s, |n| local.get(n).cloned().map_or_else(|| lookup(n), Ok))
}

/// Replace allowed variables' values with their `${NAME}` (prompts written by people, e.g. a
/// test case that spells out a password: the model sees and uses the variable instead).
pub fn to_vars(text: &str) -> String {
    let mut out = text.to_owned();
    for name in allowed() {
        if let Ok(v) = std::env::var(&name)
            && v.chars().count() >= 3
        {
            out = out.replace(&v, &format!("${{{name}}}"));
        }
    }
    out
}

/// Mask every allowed variable's value in text going back to the AI (PRD §15).
pub fn redact(text: &str) -> String {
    let mut secrets: Vec<String> = allowed()
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .filter(|v| v.chars().count() >= 3)
        .collect();
    secrets.extend(SECRETS.lock().unwrap_or_else(|p| p.into_inner()).iter().cloned());
    redact_with(text, &secrets)
}

/// A clipped secret (`sk-test-FAKE0123…`) is masked when at least this many of its first
/// characters are shown.
const MIN_CLIPPED_PREFIX: usize = 6;

/// Every way a secret can appear in output: as is, escaped by `quote` (snapshots), by JSON
/// (reports, tool arguments), by HTML (reports), and whitespace-collapsed (snapshot names).
fn secret_forms(v: &str) -> Vec<String> {
    let inner = |q: String| q[1..q.len() - 1].to_owned();
    let mut forms = vec![
        v.to_owned(),
        inner(crate::snapshot::quote(v)),
        serde_json::to_string(v).map(inner).unwrap_or_default(),
        crate::recording::escape_html(v),
        v.split_whitespace().collect::<Vec<_>>().join(" "),
    ];
    forms.retain(|f| f.chars().count() >= 3);
    // Longest first, so an escaped form is masked whole rather than around its raw core.
    forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
    forms.dedup();
    forms
}

/// `redact` over an explicit list of secret values.
pub(crate) fn redact_with(text: &str, secrets: &[String]) -> String {
    let forms: Vec<String> = secrets.iter().flat_map(|v| secret_forms(v)).collect();
    let mut out = text.to_owned();
    for f in &forms {
        if out.contains(f.as_str()) {
            out = out.replace(f.as_str(), "***");
        }
    }
    // Output clipped with `…` may show only the start of a secret: mask that prefix too.
    if out.contains('…') {
        out = mask_clipped(&out, &forms);
    }
    out
}

/// Replace `<prefix of a secret>…` with `***…`.
fn mask_clipped(text: &str, forms: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('…') {
        let head = &rest[..i];
        let cut = forms
            .iter()
            .filter_map(|f| {
                // Longest prefix of `f` (whole chars, at least MIN_CLIPPED_PREFIX) ending `head`.
                f.char_indices()
                    .map(|(j, c)| j + c.len_utf8())
                    .skip(MIN_CLIPPED_PREFIX - 1)
                    .filter(|&end| head.ends_with(&f[..end]))
                    .max()
            })
            .max();
        match cut {
            Some(n) => {
                out.push_str(&head[..head.len() - n]);
                out.push_str("***");
            }
            None => out.push_str(head),
        }
        out.push('…');
        rest = &rest[i + '…'.len_utf8()..];
    }
    out.push_str(rest);
    out
}

/// Line numbers of the top-level `- ` items in `lines[from..to]`.
fn item_lines(lines: &[&str], from: usize, to: usize) -> Vec<usize> {
    let items: Vec<(usize, usize)> = lines[from..to]
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let trimmed = l.trim_start();
            (trimmed == "-" || trimmed.starts_with("- ")).then(|| (from + i, l.len() - trimmed.len()))
        })
        .collect();
    let Some(min) = items.iter().map(|&(_, indent)| indent).min() else {
        return Vec::new();
    };
    items.into_iter().filter(|&(_, i)| i == min).map(|(l, _)| l).collect()
}

/// Maestro flow files may start with a config document (`appId: ...` then `---`); the steps
/// are the last document. Returns the steps text and how many lines precede it.
fn body(yaml: &str) -> (&str, usize) {
    let mut offset = 0;
    let mut start = 0;
    for line in yaml.split_inclusive('\n') {
        if line.trim_end() == "---" {
            start = offset + line.len();
        }
        offset += line.len();
    }
    let before = yaml[..start].lines().count();
    (&yaml[start..], before)
}

/// Parse YAML into a flow. Shape errors carry the line of the offending step; relative
/// `runFlow` files resolve against the working directory.
pub fn parse_flow(yaml: &str) -> Result<Flow, Vec<FlowError>> {
    parse_with(yaml, &Cx { base: PathBuf::new(), depth: 0 })
}

/// `parse_flow` for a flow read from `path`: its sub-flows resolve next to it.
pub fn parse_flow_file(yaml: &str, path: &Path) -> Result<Flow, Vec<FlowError>> {
    parse_with(yaml, &Cx { base: path.parent().map(Path::to_path_buf).unwrap_or_default(), depth: 0 })
}

fn parse_with(yaml: &str, cx: &Cx) -> Result<Flow, Vec<FlowError>> {
    let (yaml, skipped) = body(yaml);
    let shift = |l: usize| l + skipped;
    let doc: Yaml = serde_yaml::from_str(yaml).map_err(|e| {
        vec![FlowError {
            line: e.location().map(|l| shift(l.line())),
            message: format!("YAML: {e}"),
        }]
    })?;
    let seq = match doc {
        Yaml::Sequence(s) => s,
        Yaml::Null => Vec::new(),
        _ => {
            return Err(vec![FlowError {
                line: Some(1),
                message: "a flow is a YAML list of steps".into(),
            }]);
        }
    };
    let lines: Vec<&str> = yaml.lines().collect();
    let roots = item_lines(&lines, 0, lines.len());
    let roots = if roots.len() == seq.len() { roots } else { Vec::new() };

    let mut steps = Vec::new();
    let mut errors = Vec::new();
    for (i, v) in seq.iter().enumerate() {
        let line = roots.get(i).map(|l| shift(l + 1));
        match Step::parse(v, cx) {
            Ok(mut step) => {
                if let (Step::Parallel(branches), Some(&start)) = (&mut step, roots.get(i)) {
                    let end = roots.get(i + 1).copied().unwrap_or(lines.len());
                    locate_branches(branches, &lines, start + 1, end);
                    for b in branches.iter_mut() {
                        b.steps.iter_mut().for_each(|s| s.line = s.line.map(shift));
                    }
                }
                steps.push(FlowStep { line, step });
            }
            Err(message) => errors.push(FlowError { line, message }),
        }
    }
    if errors.is_empty() { Ok(Flow { steps }) } else { Err(errors) }
}

fn locate_branches(branches: &mut [Branch], lines: &[&str], from: usize, to: usize) {
    let heads = item_lines(lines, from, to);
    if heads.len() != branches.len() {
        return;
    }
    for (i, b) in branches.iter_mut().enumerate() {
        let end = heads.get(i + 1).copied().unwrap_or(to);
        let items = item_lines(lines, heads[i] + 1, end);
        if items.len() == b.steps.len() {
            for (s, l) in b.steps.iter_mut().zip(items) {
                s.line = Some(l + 1);
            }
        }
    }
}

/// `${VARS}` of a step and everything nested in it must be allowlisted or defined by the flow
/// (runFlow `env`); blocks may not contain session or parallel steps.
fn check_step(step: &Step, defined: &mut HashSet<String>, out: &mut dyn FnMut(String)) {
    for v in step.vars() {
        if !defined.contains(&v)
            && let Err(e) = lookup(&v)
        {
            out(e);
        }
    }
    defined.extend(step.defines());
    for c in step.children() {
        match &c.step {
            Step::Parallel(_) | Step::UseSession(_) | Step::StartSession { .. } => {
                out(format!("`{}` is not allowed inside runFlow/repeat/retry", c.step.label()));
            }
            other => check_step(other, defined, out),
        }
    }
}

/// Parse + semantic validation without executing anything. `known` are sessions that already
/// exist in the registry.
pub fn check_flow(yaml: &str, known: &[String]) -> Result<Flow, Vec<FlowError>> {
    check_parsed(parse_flow(yaml)?, known)
}

/// `check_flow` for a flow file (sub-flows resolve next to it).
pub fn check_flow_file(yaml: &str, path: &Path, known: &[String]) -> Result<Flow, Vec<FlowError>> {
    check_parsed(parse_flow_file(yaml, path)?, known)
}

fn check_parsed(flow: Flow, known: &[String]) -> Result<Flow, Vec<FlowError>> {
    let mut errors = Vec::new();
    let mut sessions: HashSet<&str> = known.iter().map(String::as_str).collect();
    let mut active = !known.is_empty();
    let mut defined = HashSet::new();
    for fs in &flow.steps {
        let err = |message: String| FlowError { line: fs.line, message };
        check_step(&fs.step, &mut defined, &mut |m| errors.push(err(m)));
        match &fs.step {
            Step::StartSession { id, reset, device, platform, .. } => {
                if !sessions.insert(id) && !reset {
                    errors.push(err(format!("session `{id}` already exists")));
                }
                if let Some(d) = device
                    && let Err(e) = crate::DeviceSpec::parse(d).and_then(|spec| spec.check_platform(*platform))
                {
                    errors.push(err(e));
                }
                active = true;
            }
            Step::UseSession(id) => {
                if !sessions.contains(id.as_str()) {
                    errors.push(err(format!("unknown session `{id}`; start it first")));
                }
                active = true;
            }
            Step::Parallel(branches) => {
                let mut used = HashSet::new();
                for (i, b) in branches.iter().enumerate() {
                    let n = i + 1;
                    if !sessions.contains(b.session.as_str()) {
                        errors.push(err(format!("branch {n}: unknown session `{}`", b.session)));
                    }
                    if !used.insert(&b.session) {
                        errors.push(err(format!("branch {n}: session `{}` is used by two branches", b.session)));
                    }
                    if b.steps.is_empty() {
                        errors.push(err(format!("branch {n}: no steps")));
                    }
                    for s in &b.steps {
                        let serr = |m: String| FlowError { line: s.line.or(fs.line), message: format!("branch {n}: {m}") };
                        match &s.step {
                            Step::Parallel(_) => errors.push(serr("nested `parallel` is not allowed".into())),
                            Step::UseSession(_) | Step::StartSession { .. } => errors.push(serr(
                                "a branch is bound to one session; `session` steps are not allowed inside".into(),
                            )),
                            other => check_step(other, &mut defined, &mut |m| errors.push(serr(m))),
                        }
                    }
                }
            }
            step if step.sessionless() => {}
            _ if !active => {
                errors.push(err(
                    "no active session: add `- session: { start: web1, platform: web, target: ... }` first".into(),
                ));
                active = true; // report once
            }
            _ => {}
        }
    }
    if errors.is_empty() { Ok(flow) } else { Err(errors) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN: &str = r#"
- session: { start: web1, platform: web, target: "examples/demo/login.html" }
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${FP_TEST_PASS}" }
- tapOn: "Masuk"
- assertVisible: { text: "Dashboard", timeout: 3000 }
- parallel:
    - session: web1
      steps:
        - tapOn: { role: button, name: "Keluar" }
        - scroll: down
    - session: web2
      steps:
        - back
- evalScript: "document.title"
"#;

    #[test]
    fn parses_steps_with_lines() {
        let flow = parse_flow(LOGIN).expect("valid flow");
        assert_eq!(flow.steps.len(), 7);
        assert_eq!(flow.steps[0].line, Some(2));
        assert_eq!(
            flow.steps[4].step,
            Step::AssertVisible {
                selector: Selector::Text("Dashboard".into()),
                timeout: Some(Duration::from_millis(3000)),
            }
        );
        let Step::Parallel(b) = &flow.steps[5].step else { panic!("parallel") };
        assert_eq!(b[0].steps[1].line, Some(11));
        assert_eq!(b[1].steps[0].step, Step::Back);
        assert_eq!(b[1].steps[0].line, Some(14));
    }

    /// Issues #2 and #3: secrets must be masked however they were clipped or escaped.
    #[test]
    fn redacts_clipped_and_escaped_secrets() {
        let long = "sk-test-FAKE0123456789abcdefghijklmnopqrstuvwxyzABCDEFGH".to_owned();
        let quoted = "pa\"ss\\word99".to_owned();
        let multi = "line1\nline2\tend".to_owned();
        let arabic = "كلمة\"سر\\٢٠".to_owned();
        let secrets = [long.clone(), quoted.clone(), multi.clone(), arabic.clone()];
        let r = |t: &str| redact_with(t, &secrets);
        // Raw and clipped (40 chars + `…`, as snapshots render a value).
        assert_eq!(r(&format!("token {long} end")), "token *** end");
        let clipped: String = long.chars().take(40).collect();
        assert_eq!(r(&format!("value=\"{clipped}…\"")), "value=\"***…\"");
        // A clipped value with the secret after other text.
        assert_eq!(r("value=\"Bearer sk-test-FAKE01…\""), "value=\"Bearer ***…\"");
        // Too short a prefix, or unrelated clipped text, stays.
        assert_eq!(r("see sk-te… and Sesi Anda telah…"), "see sk-te… and Sesi Anda telah…");
        // Escaped by `quote` (snapshots), JSON (reports) and HTML (report pages).
        for secret in [&quoted, &multi, &arabic] {
            let q = crate::snapshot::quote(secret);
            assert_eq!(r(&format!("value={q}")), "value=\"***\"", "quote form of {secret:?}");
            let j = serde_json::to_string(secret).expect("json");
            assert_eq!(r(&format!("{{\"text\":{j}}}")), "{\"text\":\"***\"}", "json form of {secret:?}");
            assert!(!r(&crate::recording::escape_html(secret)).contains("ss"), "html form of {secret:?}");
            assert_eq!(r(secret), "***");
        }
        assert_eq!(r("value=\"pa\\\"ss\\\\word99\""), "value=\"***\"");
        // Whitespace collapsed, as in snapshot names.
        assert_eq!(r("text \"line1 line2 end\""), "text \"***\"");
        // Escaped and clipped at once.
        let q = crate::snapshot::quote(&quoted);
        assert_eq!(r(&format!("{}…", &q[..q.len() - 4])), "\"***…");
    }

    #[test]
    fn snapshots_mask_secrets_before_clipping_and_quoting() {
        let long = "sk-test-FAKEsnap0123456789abcdefghijklmnopqrstuvwxyz".to_owned();
        let quoted = "pa\"ss\\snap99".to_owned();
        add_secret(&long);
        add_secret(&quoted);
        let field = |name: &str, v: &str| crate::Node { role: "textfield".into(), name: name.into(), value: Some(v.into()), ..Default::default() };
        let mut refs = crate::RefRegistry::default();
        let snap = crate::Snapshot::build("t", "u", vec![field("API Token", &long), field("Kata sandi", &quoted), field("Auth", &format!("Bearer {long}"))], &mut refs);
        let text = snap.render(None, 120);
        assert!(text.contains("textfield \"API Token\" value=\"***\""), "{text}");
        assert!(text.contains("textfield \"Kata sandi\" value=\"***\""), "{text}");
        assert!(text.contains("textfield \"Auth\" value=\"Bearer ***\""), "{text}");
        assert!(!text.contains("FAKEsnap") && !text.contains("snap99"), "{text}");
        let next = crate::Snapshot::build("t", "u", vec![field("API Token", ""), field("Kata sandi", ""), field("Auth", "")], &mut refs);
        let d = crate::diff(&snap, &next).expect("small diff").join("\n");
        assert!(!d.contains("FAKEsnap") && !d.contains("snap99"), "{d}");
    }

    #[test]
    fn check_flow_reports_semantic_errors() {
        // FASTPILOT_VARS is also read and changed by the config tests: hold ENV_LOCK, restore after.
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("FP_TEST_PASS", "x");
            std::env::set_var("FASTPILOT_VARS", "FP_TEST_PASS,FP_DEFINITELY_UNSET");
        }
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                // SAFETY: still under ENV_LOCK (dropped before `_env`).
                unsafe {
                    std::env::remove_var("FP_TEST_PASS");
                    std::env::remove_var("FASTPILOT_VARS");
                }
            }
        }
        let _restore = Restore;
        let errs = check_flow(LOGIN, &[]).expect_err("web2 is unknown");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].to_string(), "line 7: branch 2: unknown session `web2`");
        assert!(check_flow(LOGIN, &["web2".into()]).is_ok());

        let errs = check_flow("- tapOn: Login\n- assertTrue: x\n- frobnicate: 1\n", &[]).expect_err("bad");
        assert_eq!(errs.len(), 2);
        assert!(errs[0].to_string().starts_with("line 2: assertTrue: Maestro command"), "{}", errs[0]);
        assert!(errs[1].to_string().starts_with("line 3: frobnicate: unknown step"));
        // QA B8: the names an AI guesses first point at fastpilot's step.
        for (guess, step) in [("fill", "inputText: { into:"), ("type", "inputText:"), ("tap", "tapOn:"), ("click", "tapOn:"), ("goto", "openUrl:"), ("expect", "assertVisible:")] {
            let errs = check_flow(&format!("- {guess}: \"x\"\n"), &[]).expect_err(guess);
            assert!(errs[0].to_string().contains(&format!("unknown step `{guess}`: in fastpilot write `{step}")), "{guess}: {}", errs[0]);
        }

        let errs = check_flow("- tapOn: Login\n", &[]).expect_err("no session");
        assert!(errs[0].message.starts_with("no active session"));

        let errs = check_flow("- openUrl: \"${FP_DEFINITELY_UNSET}\"\n", &["s".into()]).expect_err("unset var");
        assert!(errs[0].message.contains("FP_DEFINITELY_UNSET"));

        assert!(parse_flow("- tapOn: [1").is_err());
    }

    #[test]
    fn round_trips_to_yaml_with_durable_selectors() {
        let flow = parse_flow(LOGIN).expect("valid flow");
        let durable = HashMap::from([(4, Selector::Role { role: "button".into(), name: Some("Masuk".into()) })]);
        let again = parse_flow(&flow_to_yaml(&flow, &durable)).expect("re-parses");
        let steps = |f: &Flow| f.steps.iter().map(|s| s.step.clone()).collect::<Vec<_>>();
        let mut expected = steps(&flow);
        expected[3] = Step::TapOn(Selector::Role { role: "button".into(), name: Some("Masuk".into()) });
        let strip = |v: Vec<Step>| -> Vec<Step> {
            v.into_iter()
                .map(|s| match s {
                    Step::Parallel(b) => Step::Parallel(
                        b.into_iter()
                            .map(|mut b| {
                                b.steps.iter_mut().for_each(|s| s.line = None);
                                b
                            })
                            .collect(),
                    ),
                    other => other,
                })
                .collect()
        };
        assert_eq!(strip(steps(&again)), strip(expected));
    }

    #[test]
    fn parses_maestro_blocks_and_subflows() {
        let dir = std::env::temp_dir().join(format!("fastpilot-dsl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        // A Maestro file: config document, then the steps.
        std::fs::write(dir.join("login.yaml"), "appId: com.x\n---\n- inputText: { into: Email, text: \"${USER_EMAIL}\" }\n- tapOn: Masuk\n")
            .expect("write");
        let main = dir.join("main.yaml");
        let yaml = r#"
- session: { start: w, target: x }
- runFlow: { file: login.yaml, env: { USER_EMAIL: a@b.com } }
- runFlow:
    when: { visible: "Promo" }
    commands:
      - tapOn: "Tutup"
- repeat: { times: 2, commands: [ scrollUntilVisible: { element: "Footer", direction: down, timeout: 3000 } ] }
- retry: { maxRetries: 2, commands: [ longPressOn: "Item", doubleTapOn: { id: img } ] }
- eraseText: 3
- hideKeyboard
- copyTextFrom: { id: total }
- pasteText
- openLink: "myapp://home"
- stopApp
- clearState: com.x
- inputRandomEmail
- extendedWaitUntil: { visible: "Done", timeout: 5000 }
"#;
        let flow = check_flow_file(yaml, &main, &[]).expect("valid");
        let Step::RunFlow { file, env, steps, .. } = &flow.steps[1].step else { panic!("runFlow") };
        assert_eq!((file.as_deref(), env.len(), steps.len()), (Some("login.yaml"), 1, 2));
        assert_eq!(steps[1].line, Some(4), "lines count the config document");
        assert_eq!(flow.steps[2].step.label(), "runFlow (1 steps) when visible \"Promo\"");
        assert_eq!(flow.steps[3].step.label(), "repeat 2x (1 steps)");
        assert_eq!(flow.steps[9].step, Step::OpenUrl("myapp://home".into()));
        assert_eq!(flow.steps[13].step, Step::AssertVisible { selector: Selector::Text("Done".into()), timeout: Some(Duration::from_secs(5)) });

        let again = parse_flow_file(&flow_to_yaml(&flow, &HashMap::new()), &main).expect("re-parses");
        let steps = |f: &Flow| f.steps.iter().map(|s| s.step.label()).collect::<Vec<_>>();
        assert_eq!(steps(&again), steps(&flow));

        // Unknown var inside a block is still caught; env-defined ones are fine.
        let errs = check_flow_file("- session: { start: w }\n- repeat: { times: 1, commands: [ inputText: \"${NOPE_X}\" ] }\n", &main, &[])
            .expect_err("unknown var");
        assert!(errs[0].message.contains("NOPE_X"), "{}", errs[0]);
        let errs = check_flow_file("- runFlow: { file: missing.yaml }\n", &main, &[]).expect_err("missing");
        assert!(errs[0].message.contains("missing.yaml"), "{}", errs[0]);
        let errs = check_flow("- retry: { commands: [ session: w ] }\n", &["w".into()]).expect_err("session in block");
        assert!(errs[0].message.contains("not allowed inside"), "{}", errs[0]);
        assert!(check_flow("- runFlow: { when: { true: \"1\" }, commands: [ back ] }\n", &[]).is_err());
        std::fs::write(dir.join("loop.yaml"), "- runFlow: loop.yaml\n").expect("write");
        assert!(parse_flow_file("- runFlow: loop.yaml\n", &main).expect_err("cycle")[0].message.contains("deeper"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// E1: one-time codes are run-time values, by their name or by the words of their step.
    #[test]
    fn tells_a_one_time_code_from_a_password() {
        for (name, step) in [
            ("MOTMAINA_OTP", "Isi kode OTP ${MOTMAINA_OTP} di kolom 5 digit"),
            ("SMS", "Isi kode OTP ${SMS} di kolom 5 digit"),
            ("X", "Type the one-time code ${X}"),
            ("X", "Enter the verification code ${X}"),
            ("LOGIN_CODE", ""),
            ("PIN", ""),
            ("OTP_EMAIL", ""),
        ] {
            assert!(is_code(name, step), "{name} in {step:?}");
        }
        for (name, step) in [("TEST_PASSWORD", "Type ${TEST_PASSWORD} and the verification code"), ("API_TOKEN", "Isi kode ${API_TOKEN}"), ("DEMO_PASS", "Sign in with ${DEMO_PASS}"), ("CODENAME", ""), ("PINTU", "Open the footprint page"), ("TEST_PASSWORD", "Type ${TEST_PASSWORD}"), ("PROMO", "Enter the voucher ${PROMO}")] {
            assert!(!is_code(name, step), "{name} in {step:?}");
        }
    }

    #[test]
    fn substitutes_env() {
        assert_eq!(var_names("a ${X} b ${Y_2} `${a.b}` $${Z}"), vec!["X", "Y_2"]);
        assert!(substitute("${FP_DEFINITELY_UNSET}").is_err());
        assert!(substitute("${HOME}").expect_err("not allowlisted").contains("FASTPILOT_VARS"));
        // V051: what a plugin user (no .mcp.json) sees in Claude Code says where the name and the value go.
        let e = substitute("${HOME}").expect_err("not allowlisted");
        assert!(e.contains("ask for it in Chat, or Settings › Secrets") && e.contains("nothing was typed"), "{e}");
        assert_eq!(substitute("`${document.title}` $${X} $5"), Ok("`${document.title}` ${X} $5".into()));
        // E1: a one-time code nobody can save is not reported as a missing password.
        let e = substitute("${MOTMAINA_OTP}").expect_err("no code");
        assert!(e.starts_with("one-time code needed:") && !e.contains("setup needed") && e.contains("nothing was typed"), "{e}");
        assert!(parse_flow("- inputText: { into: Phone, text: +62812 }").is_err());
    }

    /// E1: `launchApp` with `arguments` and `env`; the old string form still works.
    #[test]
    fn launch_app_arguments_and_env() {
        use crate::{Launch, LaunchArg};
        let flow = parse_flow(
            "- launchApp: com.x\n- launchApp:\n    appId: com.x/.Main\n    arguments: { server: \"https://staging\", retries: 3, debug: true, tags: [a, b] }\n    env: { API_TOKEN: s3cr3t-literal, REF: \"${FP_LAUNCH_SECRET}\" }\n",
        )
        .expect("parses");
        assert_eq!(flow.steps[0].step, Step::LaunchApp { app: "com.x".into(), launch: Launch::default() });
        let Step::LaunchApp { app, launch } = &flow.steps[1].step else { panic!("not launchApp") };
        assert_eq!(app, "com.x/.Main");
        assert_eq!(
            launch.arguments,
            [
                ("server".into(), LaunchArg::Str("https://staging".into())),
                ("retries".into(), LaunchArg::Int(3)),
                ("debug".into(), LaunchArg::Bool(true)),
                ("tags".into(), LaunchArg::List(vec!["a".into(), "b".into()])),
            ]
        );
        // Env values never in the step label (output, report) ...
        let label = flow.steps[1].step.label();
        assert!(label.contains("env {API_TOKEN=***, REF=***}") && !label.contains("s3cr3t") && label.contains("retries=3"), "{label}");
        // ... nor as literals in the recorded flow; `${NAME}` references stay replayable.
        let yaml = flow_to_yaml(&flow, &HashMap::new());
        assert!(!yaml.contains("s3cr3t") && yaml.contains("API_TOKEN: '***'") && yaml.contains("REF: ${FP_LAUNCH_SECRET}"), "{yaml}");
        assert!(yaml.contains("- launchApp: com.x\n"), "old form kept: {yaml}");
        let again = parse_flow(&yaml).expect("recorded flow parses");
        let Step::LaunchApp { launch: replay, .. } = &again.steps[1].step else { panic!("not launchApp") };
        assert_eq!(replay.arguments, launch.arguments);

        // `${NAME}` in env is checked like any variable.
        let errs = check_flow("- session: { start: a, platform: android }\n- launchApp: { appId: com.x, env: { T: \"${FP_NOT_ALLOWED_X}\" } }\n", &[])
            .expect_err("unknown variable");
        assert!(errs.iter().any(|e| e.message.contains("FP_NOT_ALLOWED_X")), "{errs:?}");

        // Names that change how the app is loaded, or reach into fastpilot, are refused.
        for bad in ["DYLD_INSERT_LIBRARIES", "LD_PRELOAD", "FASTPILOT_ALLOWED_DOMAINS", "fastpilot_vars", "MallocStackLogging", "A-B"] {
            let err = parse_flow(&format!("- launchApp: {{ appId: com.x, env: {{ {bad}: \"1\" }} }}\n")).expect_err(bad);
            assert!(err.iter().any(|e| e.message.contains(bad)), "{bad}: {err:?}");
        }
        for (yaml, want) in [
            ("{ appId: com.x, arguments: { ratio: 1.5 } }", "whole numbers"),
            ("{ appId: com.x, arguments: [a] }", "must be a map"),
            ("{ appId: com.x, arguments: { \"a b\": 1 } }", "argument name"),
            ("{ appId: com.x, clearState: true }", "unknown launchApp field `clearState`"),
        ] {
            let err = parse_flow(&format!("- launchApp: {yaml}\n")).expect_err(yaml);
            assert!(err.iter().any(|e| e.message.contains(want)), "{yaml}: {err:?}");
        }
    }
}
