//! Flow DSL (PRD §11): Maestro-compatible YAML plus `openUrl`, `evalScript`, `session`, `parallel`.

use std::collections::HashSet;
use std::fmt;
use std::time::Duration;

use serde_yaml::{Mapping, Value as Yaml};

use crate::{Direction, Platform, Selector};

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    OpenUrl(String),
    TapOn(Selector),
    InputText { into: Selector, text: String },
    AssertVisible { selector: Selector, timeout: Option<Duration> },
    Scroll { direction: Direction, within: Option<Selector> },
    Back,
    EvalScript(String),
    UseSession(String),
    StartSession { id: String, platform: Platform, target: Option<String> },
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
    "launchApp", "assertNotVisible", "swipe", "stopApp", "clearState", "clearKeychain",
    "hideKeyboard", "eraseText", "pressKey", "longPressOn", "doubleTapOn", "scrollUntilVisible",
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
                other => Err(unknown(other)),
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
            Self::UseSession(id) => format!("session {id}"),
            Self::StartSession { id, platform, .. } => format!("session start {id} ({platform})"),
            Self::Parallel(b) => format!("parallel ({} branches)", b.len()),
        }
    }

    fn vars(&self) -> Vec<&str> {
        let texts: Vec<&str> = match self {
            Self::OpenUrl(s) | Self::EvalScript(s) => vec![s],
            Self::InputText { text, .. } => vec![text],
            Self::StartSession { target: Some(t), .. } => vec![t],
            _ => vec![],
        };
        texts.into_iter().flat_map(var_names).collect()
    }
}

fn unknown(cmd: &str) -> String {
    if MAESTRO_LATER.contains(&cmd) {
        format!("Maestro command `{cmd}` is not supported yet (Phase 1: web only)")
    } else {
        format!(
            "unknown step `{cmd}` (supported: openUrl, tapOn, inputText, assertVisible, scroll, back, evalScript, session, parallel)"
        )
    }
}

fn get<'a>(m: &'a Mapping, key: &str) -> Option<&'a Yaml> {
    m.get(Yaml::String(key.into()))
}

fn string(v: &Yaml) -> Result<String, String> {
    match v {
        Yaml::String(s) => Ok(s.clone()),
        Yaml::Number(n) => Ok(n.to_string()),
        Yaml::Bool(b) => Ok(b.to_string()),
        _ => Err("expects a string".into()),
    }
}

fn direction(v: &Yaml) -> Result<Direction, String> {
    let s = string(v)?;
    Direction::parse(&s).ok_or(format!("unknown direction `{s}` (up, down, left, right)"))
}

/// `${NAME}` references in a string.
pub(crate) fn var_names(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else { break };
        out.push(&after[..end]);
        rest = &after[end + 1..];
    }
    out
}

/// Replace `${NAME}` with the environment value of fastpilot's own process.
pub(crate) fn substitute(s: &str) -> Result<String, String> {
    let mut out = s.to_string();
    for name in var_names(s) {
        let val = std::env::var(name).map_err(|_| format!("environment variable `{name}` is not set"))?;
        out = out.replace(&format!("${{{name}}}"), &val);
    }
    Ok(out)
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
            if std::env::var(v).is_err() {
                errors.push(err(format!("environment variable `{v}` is not set")));
            }
        }
        match &fs.step {
            Step::StartSession { id, .. } => {
                if !sessions.insert(id) {
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
                                    if std::env::var(v).is_err() {
                                        errors.push(serr(format!("environment variable `{v}` is not set")));
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
        // SAFETY: test-only env mutation; no other test reads this variable.
        unsafe { std::env::set_var("FP_TEST_PASS", "x") };
        let errs = check_flow(LOGIN, &[]).expect_err("web2 is unknown");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].to_string(), "line 7: branch 2: unknown session `web2`");
        assert!(check_flow(LOGIN, &["web2".into()]).is_ok());

        let errs = check_flow("- tapOn: Login\n- launchApp: com.x\n- frobnicate: 1\n", &[]).expect_err("bad");
        assert_eq!(errs.len(), 2);
        assert!(errs[0].to_string().starts_with("line 2: launchApp: Maestro command"));
        assert!(errs[1].to_string().starts_with("line 3: frobnicate: unknown step"));

        let errs = check_flow("- tapOn: Login\n", &[]).expect_err("no session");
        assert!(errs[0].message.starts_with("no active session"));

        let errs = check_flow("- openUrl: \"${FP_DEFINITELY_UNSET}\"\n", &["s".into()]).expect_err("unset var");
        assert!(errs[0].message.contains("FP_DEFINITELY_UNSET"));

        assert!(parse_flow("- tapOn: [1").is_err());
    }

    #[test]
    fn substitutes_env() {
        assert_eq!(var_names("a ${X} b ${Y_2}"), vec!["X", "Y_2"]);
        assert!(substitute("${FP_DEFINITELY_UNSET}").is_err());
        assert_eq!(substitute("plain"), Ok("plain".into()));
    }
}
