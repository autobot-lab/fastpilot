//! Flow DSL (PRD §11): Maestro-compatible YAML plus `openUrl`, `evalScript`, `session`, `parallel`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::Duration;

use serde_yaml::{Mapping, Value as Yaml};

use crate::{Direction, Platform, Point, Selector};

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    OpenUrl(String),
    TapOn(Selector),
    InputText { into: Selector, text: String },
    AssertVisible { selector: Selector, timeout: Option<Duration> },
    Scroll { direction: Direction, within: Option<Selector> },
    Back,
    EvalScript(String),
    AssertNotVisible { selector: Selector, timeout: Option<Duration> },
    PressKey(String),
    SelectOption { from: Selector, option: String },
    UploadFile { into: Selector, files: Vec<String> },
    LaunchApp(String),
    Swipe { from: Point, to: Point },
    UseSession(String),
    StartSession { id: String, platform: Platform, target: Option<String>, reset: bool },
    Parallel(Vec<Branch>),
}

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

/// Maestro commands we recognise but do not run yet, so the AI gets a clear message.
const MAESTRO_LATER: &[&str] = &[
    "stopApp", "clearState", "clearKeychain",
    "hideKeyboard", "eraseText", "longPressOn", "doubleTapOn", "scrollUntilVisible",
    "extendedWaitUntil", "waitForAnimationToEnd", "takeScreenshot", "copyTextFrom", "pasteText",
    "runFlow", "runScript", "repeat", "retry", "assertTrue", "openLink", "setLocation", "travel",
    "addMedia", "inputRandomText", "inputRandomEmail", "inputRandomNumber", "inputRandomPersonName",
    "killApp", "setAirplaneMode", "toggleAirplaneMode", "startRecording", "stopRecording",
];

impl Step {
    fn parse(v: &Yaml) -> Result<Self, String> {
        match v {
            Yaml::String(s) => match s.as_str() {
                "back" => Ok(Self::Back),
                "scroll" => Ok(Self::Scroll {
                    direction: Direction::Down,
                    within: None,
                }),
                other => Err(format!("{other}: {}", unknown(other))),
            },
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
                Self::parse_command(cmd, arg).map_err(|e| format!("{cmd}: {e}"))
            }
            _ => Err("a step must be a command string or a `command: args` map".into()),
        }
    }

    fn parse_command(cmd: &str, arg: &Yaml) -> Result<Self, String> {
        Ok(match cmd {
            "openUrl" => Self::OpenUrl(string(arg)?),
            "tapOn" => Self::TapOn(Selector::from_yaml(arg, &[])?),
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
            "pressKey" => Self::PressKey(string(arg)?),
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
                Yaml::Mapping(m) => Self::LaunchApp(string(get(m, "appId").ok_or("missing `appId`")?)?),
                other => Self::LaunchApp(string(other)?),
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
                                Step::parse(s)
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
            Self::OpenUrl(u) => format!("openUrl {u:?}"),
            Self::TapOn(s) => format!("tapOn {s}"),
            Self::InputText { into, text } => format!("inputText {into} {text:?}"),
            Self::AssertVisible { selector, .. } => format!("assertVisible {selector}"),
            Self::Scroll { direction, within: Some(w) } => format!("scroll {direction:?} within {w}"),
            Self::Scroll { direction, within: None } => format!("scroll {direction:?}"),
            Self::Back => "back".into(),
            Self::EvalScript(js) => format!("evalScript {js:?}"),
            Self::AssertNotVisible { selector, .. } => format!("assertNotVisible {selector}"),
            Self::PressKey(k) => format!("pressKey {k}"),
            Self::SelectOption { from, option } => format!("selectOption {from} {option:?}"),
            Self::UploadFile { into, files } => format!("uploadFile {into} {files:?}"),
            Self::LaunchApp(a) => format!("launchApp {a}"),
            Self::Swipe { from, to } => format!("swipe [{}, {}] -> [{}, {}]", from.x, from.y, to.x, to.y),
            Self::UseSession(id) => format!("session {id}"),
            Self::StartSession { id, platform, .. } => format!("session start {id} ({platform})"),
            Self::Parallel(b) => format!("parallel ({} branches)", b.len()),
        }
    }

    fn vars(&self) -> Vec<String> {
        let texts: Vec<&str> = match self {
            Self::OpenUrl(s) | Self::EvalScript(s) => vec![s],
            Self::InputText { text, .. } => vec![text],
            Self::StartSession { target: Some(t), .. } => vec![t],
            _ => vec![],
        };
        texts.into_iter().flat_map(var_names).collect()
    }
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

impl Step {
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
            Self::LaunchApp(a) => one("launchApp", s(a)),
            Self::Swipe { from, to } => one("swipe", map(vec![("from", pt(from)), ("to", pt(to))])),
            Self::UseSession(id) => one("session", s(id)),
            Self::StartSession { id, platform, target, reset } => one(
                "session",
                map(vec![
                    ("start", s(id)),
                    ("platform", s(&platform.to_string())),
                    ("target", target.as_deref().map(s).unwrap_or(Yaml::Null)),
                    ("reset", if *reset { Yaml::Bool(true) } else { Yaml::Null }),
                ]),
            ),
            Self::Parallel(_) => Yaml::Null, // handled above
        }
    }
}

/// Serialize a flow back to DSL YAML, replacing selectors of the given leaf steps (1-based,
/// same numbering as `StepRecord::index`).
pub fn flow_to_yaml(flow: &Flow, durable: &HashMap<usize, Selector>) -> String {
    let mut leaf = 0;
    let seq = Yaml::Sequence(flow.steps.iter().map(|s| s.step.to_yaml(&mut leaf, durable)).collect());
    serde_yaml::to_string(&seq).unwrap_or_default()
}

fn unknown(cmd: &str) -> String {
    if MAESTRO_LATER.contains(&cmd) {
        format!("Maestro command `{cmd}` is not supported yet")
    } else {
        format!(
            "unknown step `{cmd}` (supported: launchApp, openUrl, tapOn, inputText, assertVisible, assertNotVisible, scroll, swipe, back, pressKey, selectOption, uploadFile, evalScript, session, parallel)"
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
    std::env::var("FASTPILOT_VARS")
        .unwrap_or_default()
        .split(',')
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .collect()
}

fn lookup(name: &str) -> Result<String, String> {
    if !allowed().iter().any(|a| a == name) {
        return Err(format!("variable `{name}` is not allowed; list it in FASTPILOT_VARS"));
    }
    std::env::var(name).map_err(|_| format!("environment variable `{name}` is not set"))
}

/// Replace `${NAME}` with allowed environment values of fastpilot's own process.
pub(crate) fn substitute(s: &str) -> Result<String, String> {
    expand(s, lookup)
}

/// Mask every allowed variable's value in text going back to the AI (PRD §15).
pub fn redact(text: &str) -> String {
    let mut out = text.to_owned();
    for name in allowed() {
        if let Ok(v) = std::env::var(&name)
            && v.chars().count() >= 3
        {
            out = out.replace(&v, "***");
        }
    }
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

/// Parse YAML into a flow. Shape errors carry the line of the offending step.
pub fn parse_flow(yaml: &str) -> Result<Flow, Vec<FlowError>> {
    let doc: Yaml = serde_yaml::from_str(yaml).map_err(|e| {
        vec![FlowError {
            line: e.location().map(|l| l.line()),
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
        let line = roots.get(i).map(|l| l + 1);
        match Step::parse(v) {
            Ok(mut step) => {
                if let (Step::Parallel(branches), Some(&start)) = (&mut step, roots.get(i)) {
                    let end = roots.get(i + 1).copied().unwrap_or(lines.len());
                    locate_branches(branches, &lines, start + 1, end);
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

/// Parse + semantic validation without executing anything. `known` are sessions that already
/// exist in the registry.
pub fn check_flow(yaml: &str, known: &[String]) -> Result<Flow, Vec<FlowError>> {
    let flow = parse_flow(yaml)?;
    let mut errors = Vec::new();
    let mut sessions: HashSet<&str> = known.iter().map(String::as_str).collect();
    let mut active = !known.is_empty();
    for fs in &flow.steps {
        let err = |message: String| FlowError { line: fs.line, message };
        for v in fs.step.vars() {
            if let Err(e) = lookup(&v) {
                errors.push(err(e));
            }
        }
        match &fs.step {
            Step::StartSession { id, reset, .. } => {
                if !sessions.insert(id) && !reset {
                    errors.push(err(format!("session `{id}` already exists")));
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
                            other => {
                                for v in other.vars() {
                                    if let Err(e) = lookup(&v) {
                                        errors.push(serr(e));
                                    }
                                }
                            }
                        }
                    }
                }
            }
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

    #[test]
    fn check_flow_reports_semantic_errors() {
        // SAFETY: test-only env mutation; only this test reads these variables.
        unsafe {
            std::env::set_var("FP_TEST_PASS", "x");
            std::env::set_var("FASTPILOT_VARS", "FP_TEST_PASS,FP_DEFINITELY_UNSET");
        }
        let errs = check_flow(LOGIN, &[]).expect_err("web2 is unknown");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].to_string(), "line 7: branch 2: unknown session `web2`");
        assert!(check_flow(LOGIN, &["web2".into()]).is_ok());

        let errs = check_flow("- tapOn: Login\n- hideKeyboard\n- frobnicate: 1\n", &[]).expect_err("bad");
        assert_eq!(errs.len(), 2);
        assert!(errs[0].to_string().starts_with("line 2: hideKeyboard: Maestro command"), "{}", errs[0]);
        assert!(errs[1].to_string().starts_with("line 3: frobnicate: unknown step"));

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
    fn substitutes_env() {
        assert_eq!(var_names("a ${X} b ${Y_2} `${a.b}` $${Z}"), vec!["X", "Y_2"]);
        assert!(substitute("${FP_DEFINITELY_UNSET}").is_err());
        assert!(substitute("${HOME}").expect_err("not allowlisted").contains("FASTPILOT_VARS"));
        assert_eq!(substitute("`${document.title}` $${X} $5"), Ok("`${document.title}` ${X} $5".into()));
        assert!(parse_flow("- inputText: { into: Phone, text: +62812 }").is_err());
    }
}
