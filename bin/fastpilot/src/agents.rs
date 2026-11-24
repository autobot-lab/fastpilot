//! D10a: agents. An agent is a tester with its own Chat, schedules, instructions, skills and AI
//! model, like an agent in OpenBot. They live in `~/.fastpilot/agents.json` (owner-only). Someone
//! who used fastpilot before gets one agent, "My tester" (`default`), which holds the existing
//! Chat, schedules and history.
//!
//! What an agent can change is bounded: its instructions are the user's own words for the bot (they
//! never loosen a limit: guard rails are enforced by fastpilot, not by the prompt), and its extra
//! blocked words only add to the global list.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

type Res<T> = Result<T, String>;

pub const DEFAULT: &str = "default";

/// Where an agent tests, as people name it.
pub const DEVICES: &[(&str, &str)] = &[
    ("web", "Web"),
    ("android_emulator", "Android emulator"),
    ("ios_simulator", "iPhone simulator"),
    ("android_phone", "Android phone (physical)"),
    ("iphone", "iPhone (physical)"),
];

/// D10b: who drives an agent (`provider` setting values), as people name them.
pub const PROVIDERS: &[(&str, &str)] = &[("claude", "Claude Code"), ("openai", "OpenAI"), ("openrouter", "OpenRouter"), ("groq", "Groq"), ("ollama", "Ollama (local)"), ("custom", "Other")];

/// Claude models to pick from (id, name, what it is good for).
pub const CLAUDE_MODELS: &[(&str, &str, &str)] = &[
    ("claude-sonnet-5-5", "Sonnet", "balanced"),
    ("claude-opus-5-5", "Opus", "most careful · uses more quota"),
    ("claude-haiku-5-5", "Haiku", "fast & cheap · good for regressions"),
];

/// "Provider · Model" as the page shows it everywhere, and whether that model has been tried
/// with fastpilot (only Claude models have).
pub fn model_label(a: &Agent) -> (String, bool) {
    let provider = a.provider.as_deref().unwrap_or("claude");
    let pname = PROVIDERS.iter().find(|(k, _)| *k == provider).map(|(_, n)| *n).unwrap_or(provider);
    if provider == "claude" {
        let id = a.model.clone().unwrap_or_else(crate::run::model_explore);
        let name = CLAUDE_MODELS.iter().find(|(m, _, _)| *m == id).map(|(_, n, _)| (*n).to_owned()).unwrap_or(id);
        (format!("{pname} · {name}"), true)
    } else {
        (format!("{pname} · {}", a.model.as_deref().unwrap_or("?")), false)
    }
}

/// Instructions to click into an agent (v7 #s4, "Suggestions").
pub const SUGGESTED_INSTRUCTIONS: &[&str] = &[
    "Stop at the first failure and report",
    "Retry once before calling it a failure",
    "Never use real customer data",
    "Keep reports short: what passed, what failed, why",
    "Ask me before changing a saved test",
    "On a failure, attach a screenshot and say which step broke",
    "Test in English and Indonesian",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    /// Who it is: its job in one or two sentences.
    #[serde(default)]
    pub job: String,
    /// Where it tests ([`DEVICES`] keys).
    #[serde(default)]
    pub devices: Vec<String>,
    /// The site or app it tests, in the user's words (a URL, a package, "Shop (staging)").
    #[serde(default)]
    pub app: String,
    /// How it should work, in the user's words.
    #[serde(default)]
    pub instructions: Vec<String>,
    /// Skills it uses (D10c); empty = all installed skills.
    #[serde(default)]
    pub skills: Vec<String>,
    /// D10b: who drives it (`claude`, `openai`, ...) and the model; `None` = the global settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// More blocked words for this agent only (on top of the global list; stricter only).
    #[serde(default)]
    pub blocked: Vec<String>,
    /// Its Chat conversations in History (ids), newest last.
    #[serde(default)]
    pub conversations: Vec<String>,
    #[serde(default)]
    pub created_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    agents: Vec<Agent>,
}

fn path() -> PathBuf {
    fastpilot_core::config::path().parent().map(std::path::Path::to_path_buf).unwrap_or_else(std::env::temp_dir).join("agents.json")
}

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn default_agent() -> Agent {
    Agent {
        id: DEFAULT.into(),
        name: "My tester".into(),
        job: "Tests this project.".into(),
        devices: vec!["web".into()],
        created_ms: fastpilot_core::change::now_ms(),
        ..Default::default()
    }
}

fn read() -> Store {
    let mut s: Store = std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    // Every install has the default agent: the one that holds what was there before agents.
    if !s.agents.iter().any(|a| a.id == DEFAULT) {
        s.agents.insert(0, default_agent());
    }
    s
}

fn write(s: &Store) -> Res<()> {
    use std::io::Write;
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension(format!("json.{}.tmp", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(&tmp)
        .and_then(|mut f| f.write_all(serde_json::to_string_pretty(s).unwrap_or_default().as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, &p))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("The agents could not be saved: {e}")
        })
}

pub fn list() -> Vec<Agent> {
    let _l = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut agents = read().agents;
    drop(_l);
    if let (Some(app), Some(d)) = (worked_on(), agents.iter_mut().find(|a| a.id == DEFAULT)) {
        *d = for_app(d.clone(), &app);
    }
    agents
}

/// S4 (v0.7): the app being worked on, which the default agent tests (none while `FASTPILOT_DIR`
/// pins a folder: then "here" is that folder, not an app).
fn worked_on() -> Option<crate::apps::App> {
    if std::env::var_os("FASTPILOT_DIR").is_some() {
        return None;
    }
    crate::apps::current()
}

/// Where each platform of an app is tested, as [`DEVICES`] keys.
fn app_devices(app: &crate::apps::App) -> Vec<String> {
    let d: Vec<String> = app.kinds().into_iter().map(|k| match k { "android" => "android_emulator", "ios" => "ios_simulator", _ => "web" }.to_owned()).collect();
    if d.is_empty() { vec!["web".into()] } else { d }
}

/// The app's start point in the user's terms: its address, else its Android or iPhone app id.
fn app_address(app: &crate::apps::App) -> String {
    app.targets.web.clone().or_else(|| app.targets.android.clone()).or_else(|| app.targets.ios.clone()).unwrap_or_default()
}

/// v0.7.1 FC5: the app an agent tests: the one its `app` names (an app id, or the app's address or
/// app id), when it is in the list. The default agent follows the current app until it has its own.
pub fn app_of(a: &Agent) -> Option<crate::apps::App> {
    if a.app.is_empty() {
        return None;
    }
    crate::apps::list().into_iter().find(|x| x.id == a.app || [&x.targets.web, &x.targets.android, &x.targets.ios].iter().any(|t| t.as_deref() == Some(a.app.as_str())))
}

/// v0.7.1 FC5: agent `agent` tests app `app` (an app id) from now on: an app added from its Chat.
pub fn bind_app(agent: &str, app: &str) -> Res<()> {
    let _l = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = read();
    let a = s.agents.iter_mut().find(|x| x.id == agent).ok_or("This agent is not there any more.")?;
    a.app = app.to_owned();
    write(&s)
}

/// The skills picked for an agent: the technique chooser plus the suggested ones.
fn auto_skills(job: &str, app: &str, devices: &[String]) -> Vec<String> {
    std::iter::once(crate::skills_catalog::CHOOSER.to_owned()).chain(crate::skills_catalog::suggest(job, app, devices).into_iter().map(|(id, _)| id)).collect()
}

/// "<app> <what>", within the 40 characters a name may have (the app name is shortened).
fn app_named(app: &str, what: &str) -> String {
    let room = 40usize.saturating_sub(what.chars().count() + 1);
    let short: String = app.chars().take(room).collect();
    format!("{} {what}", short.trim_end()).trim().to_owned()
}

/// S4: the default agent as each app sees it, "<app> tester", with the app's platforms, address and
/// auto-picked skills. Only fields the user never changed (still the built-in value) are filled, and
/// [`update`] turns them back into the built-in value, so one app's name or address is never saved
/// into the agent the next app sees.
fn for_app(mut a: Agent, app: &crate::apps::App) -> Agent {
    // v0.7.1 FC5: the default agent keeps its name ("My tester"); its app shows under the name.
    let built = default_agent();
    if a.job == built.job {
        a.job = format!("Tests {}.", app.name);
    }
    if a.app.is_empty() {
        a.app = app_address(app);
    }
    if a.devices == built.devices {
        a.devices = app_devices(app);
    }
    if a.skills.is_empty() {
        a.skills = auto_skills(&a.job, &a.app, &a.devices);
    }
    a
}

/// The reverse of [`for_app`]: values that are only the app's go back to the built-in ones.
fn without_app(mut a: Agent, app: &crate::apps::App) -> Agent {
    let built = default_agent();
    let shown = for_app(built.clone(), app);
    if a.skills == auto_skills(&a.job, &a.app, &a.devices) {
        a.skills = vec![];
    }
    if a.name == shown.name {
        a.name = built.name;
    }
    if a.job == shown.job {
        a.job = built.job;
    }
    if a.app == shown.app {
        a.app = String::new();
    }
    if a.devices == shown.devices {
        a.devices = built.devices;
    }
    a
}

/// S4: what "New agent" offers in one click: (key, button text, name, job). The name and job say the
/// app; where it tests and its skills come from the app too.
pub const FOCUSES: &[(&str, &str, &str, &str)] = &[
    ("regression", "Regression of everything", "regression", "Runs every saved test of {app} and says what broke since the last run."),
    ("checkout", "Checkout and payments", "checkout", "Keeps the cart, checkout and payment of {app} working, and never pays."),
    ("login", "Login and accounts", "login", "Keeps sign-in, sign-up and the account pages of {app} working."),
];

/// S4: a new agent with one focus, the rest from the current app. Clicking the same focus again
/// opens the agent made the first time.
pub fn create_focus(key: &str) -> Res<Agent> {
    let (_, _, what, job) = FOCUSES.iter().find(|(k, ..)| *k == key).ok_or("Unknown focus")?;
    let app = worked_on();
    let name = match &app {
        Some(a) => app_named(&a.name, what),
        None => format!("{}{} tester", what[..1].to_uppercase(), &what[1..]),
    };
    if let Some(a) = list().into_iter().find(|a| a.name.eq_ignore_ascii_case(&name)) {
        return Ok(a);
    }
    let job = job.replace("{app}", app.as_ref().map_or("this app", |a| a.name.as_str()));
    let (address, devices) = app.as_ref().map_or((String::new(), vec!["web".to_owned()]), |a| (app_address(a), app_devices(a)));
    let skills = auto_skills(&job, &address, &devices);
    create(Agent { name, job, app: address, devices, skills, ..Default::default() })
}

/// S4: what a Chat sentence leaves out ("an agent for checkout"), taken from the current app: its
/// address, its platforms and the skills that fit.
pub fn prefill(mut a: Agent) -> Agent {
    if let Some(app) = worked_on() {
        if a.app.trim().is_empty() {
            a.app = app_address(&app);
        }
        if a.devices.is_empty() {
            a.devices = app_devices(&app);
        }
    }
    if a.skills.is_empty() && !a.job.trim().is_empty() {
        let devices = if a.devices.is_empty() { vec!["web".to_owned()] } else { a.devices.clone() };
        a.skills = auto_skills(&a.job, &a.app, &devices);
    }
    a
}

/// What the "Describe it yourself" form starts with: the current app's address and platforms.
pub fn new_defaults() -> serde_json::Value {
    let app = worked_on();
    serde_json::json!({
        "app_name": app.as_ref().map(|a| a.name.clone()),
        "app": app.as_ref().map(app_address).unwrap_or_default(),
        "devices": app.as_ref().map_or(vec!["web".to_owned()], app_devices),
    })
}

pub fn get(id: &str) -> Option<Agent> {
    list().into_iter().find(|a| a.id == id)
}

/// The agent for `id`, or the default one (an unknown or deleted id).
pub fn get_or_default(id: &str) -> Agent {
    get(id).unwrap_or_else(|| get(DEFAULT).unwrap_or_else(default_agent))
}

/// Security (D10a finding 1): an agent's own words only add blocks. `!pay` would allow Pay and
/// `default` is the built-in list, so neither is a word an agent may have.
fn blocking_word(w: &str) -> bool {
    let w = w.trim();
    !w.starts_with('!') && !w.eq_ignore_ascii_case("default") && !w.contains(',')
}

/// Characters that show nothing or change how text reads (bidi, zero-width, tags, variation
/// selectors): refused in what an agent tells the AI (Security, D6 notes).
fn hidden(c: char) -> bool {
    c.is_control() || matches!(c as u32, 0x00ad | 0x034f | 0x061c | 0x180e | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x206f | 0xfe00..=0xfe0f | 0xfeff | 0xe0000..=0xe0fff)
}

fn line(what: &str, t: &str, max: usize, required: bool) -> Res<String> {
    let t = t.trim();
    if required && t.is_empty() {
        return Err(format!("{what} is needed"));
    }
    if t.chars().count() > max {
        return Err(format!("{what} can be at most {max} characters"));
    }
    if t.chars().any(hidden) {
        return Err(format!("{what} has hidden characters (copied from a page or a document?); type it again"));
    }
    if fastpilot_core::redact(t) != t {
        return Err(format!("{what} seems to hold a secret; write it as ${{NAME}} and keep the value in Settings › Secrets"));
    }
    Ok(t.to_owned())
}

/// Checked and tidied: every field within bounds, nothing hidden, no secret values.
pub fn check(mut a: Agent) -> Res<Agent> {
    a.name = line("A name", &a.name, 40, true)?;
    a.job = line("What it does", &a.job, 300, true)?;
    a.app = line("The app or site", &a.app, 300, false)?;
    a.devices.retain(|d| DEVICES.iter().any(|(k, _)| k == d));
    a.devices.dedup();
    if a.devices.is_empty() {
        return Err("Pick where it tests: web, an emulator, a simulator or a phone".into());
    }
    if a.instructions.len() > 15 {
        return Err("An agent has at most 15 instructions".into());
    }
    a.instructions = a.instructions.iter().map(|i| line("An instruction", i, 200, true)).collect::<Res<Vec<_>>>()?;
    for w in &a.blocked {
        let n = w.trim().chars().count();
        if !(3..=40).contains(&n) || w.chars().any(hidden) {
            return Err(format!("A blocked word needs 3 to 40 plain characters (\"{w}\")"));
        }
        if !blocking_word(w) {
            return Err(format!("\"{}\" is not a word to block: an agent can only block more, never allow a blocked button", w.trim()));
        }
    }
    a.blocked = a.blocked.iter().map(|w| w.trim().to_owned()).collect();
    // D10b: a known provider, and a model it can take (Claude ids for Claude Code).
    match (a.provider.as_deref().map(str::trim).filter(|p| !p.is_empty()), a.model.as_deref().map(str::trim).filter(|m| !m.is_empty())) {
        (None | Some("claude"), m) => {
            if let Some(e) = m.and_then(|m| fastpilot_core::config::invalid("model", m)) {
                return Err(format!("Claude Code takes a Claude model ({e})"));
            }
            a.provider = None;
            a.model = m.map(str::to_owned);
        }
        (Some(p), m) => {
            if !PROVIDERS.iter().any(|(k, _)| *k == p) {
                return Err(format!("Unknown provider \"{p}\""));
            }
            let Some(m) = m else { return Err("Pick the model at this provider".into()) };
            if m.chars().count() > 100 || m.contains(char::is_whitespace) || m.chars().any(hidden) {
                return Err(format!("\"{m}\" is not a model id"));
            }
            a.provider = Some(p.to_owned());
            a.model = Some(m.to_owned());
        }
    }
    a.skills.dedup();
    if let Some(s) = a.skills.iter().find(|s| !crate::skills_catalog::known(s)) {
        return Err(format!("fastpilot has no skill called \"{s}\""));
    }
    Ok(a)
}

fn slug(name: &str, taken: &[String]) -> String {
    let base: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let mut base = base.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    base.truncate(30);
    let base = if base.is_empty() || base == DEFAULT { "agent".to_owned() } else { base.trim_end_matches('-').to_owned() };
    (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(|id| !taken.contains(id)).unwrap_or(base)
}

/// A new agent (its id from its name). Returns it.
pub fn create(a: Agent) -> Res<Agent> {
    let mut a = check(a)?;
    let _l = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = read();
    if s.agents.len() >= 30 {
        return Err("fastpilot keeps at most 30 agents".into());
    }
    if s.agents.iter().any(|x| x.name.eq_ignore_ascii_case(&a.name)) {
        return Err(format!("An agent called \"{}\" exists already", a.name));
    }
    let taken: Vec<String> = s.agents.iter().map(|x| x.id.clone()).collect();
    a.id = slug(&a.name, &taken);
    a.conversations.clear();
    a.created_ms = fastpilot_core::change::now_ms();
    s.agents.push(a.clone());
    write(&s)?;
    Ok(a)
}

/// Change an agent's fields (not its id, conversations or creation time).
pub fn update(a: Agent) -> Res<Agent> {
    let mut a = check(a)?;
    if a.id == DEFAULT
        && let Some(app) = worked_on()
    {
        a = without_app(a, &app);
    }
    let _l = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = read();
    if s.agents.iter().any(|x| x.id != a.id && x.name.eq_ignore_ascii_case(&a.name)) {
        return Err(format!("An agent called \"{}\" exists already", a.name));
    }
    let Some(old) = s.agents.iter_mut().find(|x| x.id == a.id) else { return Err("That agent is not there any more".into()) };
    let keep = (old.conversations.clone(), old.created_ms);
    *old = Agent { conversations: keep.0, created_ms: keep.1, ..a };
    let out = old.clone();
    write(&s)?;
    Ok(out)
}

/// Delete an agent; its schedules and conversations go to the default agent. The default agent
/// stays.
pub fn delete(id: &str) -> Res<()> {
    if id == DEFAULT {
        return Err("The first agent cannot be deleted; rename it instead".into());
    }
    let _l = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = read();
    let Some(i) = s.agents.iter().position(|x| x.id == id) else { return Err("That agent is not there any more".into()) };
    let gone = s.agents.remove(i);
    if let Some(d) = s.agents.iter_mut().find(|x| x.id == DEFAULT) {
        d.conversations.extend(gone.conversations);
    }
    write(&s)?;
    drop(_l);
    crate::schedule::move_agent(id, DEFAULT)
}

/// Remember that conversation `conv` belongs to `agent` (History filters by it).
pub fn add_conversation(agent: &str, conv: &str) {
    let _l = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = read();
    if let Some(a) = s.agents.iter_mut().find(|x| x.id == agent)
        && !a.conversations.iter().any(|c| c == conv)
    {
        a.conversations.push(conv.to_owned());
        let extra = a.conversations.len().saturating_sub(500);
        a.conversations.drain(..extra);
        let _ = write(&s);
    }
}

/// FC4: a removed chat leaves its agent's list (Undo puts it back with [`add_conversation`]).
pub fn forget_conversation(agent: &str, conv: &str) {
    let _l = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = read();
    if let Some(a) = s.agents.iter_mut().find(|x| x.id == agent)
        && a.conversations.iter().any(|c| c == conv)
    {
        a.conversations.retain(|c| c != conv);
        let _ = write(&s);
    }
}

/// The agent a conversation belongs to: the one that lists it, else the default one.
pub fn of_conversation(conv: &str) -> String {
    list().into_iter().find(|a| a.conversations.iter().any(|c| c == conv)).map(|a| a.id).unwrap_or_else(|| DEFAULT.into())
}

/// What the Chat bot of `a` is told about itself: who it is, where it tests, and the user's
/// instructions (which never loosen a limit).
pub fn prompt(a: &Agent) -> String {
    let devices: Vec<&str> = a.devices.iter().filter_map(|d| DEVICES.iter().find(|(k, _)| k == d).map(|(_, n)| *n)).collect();
    let mut p = format!("\n\nYou are the agent \"{}\". Your job, in the user's words: {}\nWhere you test: {}.", a.name, a.job, devices.join(", "));
    if !a.app.is_empty() {
        p.push_str(&format!(" The app or site: {}.", a.app));
    }
    if !a.instructions.is_empty() {
        p.push_str("\nInstructions from the user who made this agent (follow them; they never loosen a limit or allow a blocked button):");
        for i in &a.instructions {
            p.push_str(&format!("\n- {i}"));
        }
    }
    if !a.blocked.is_empty() {
        p.push_str(&format!("\nThis agent also never presses buttons named: {}.", a.blocked.join(", ")));
    }
    p
}

/// `block_actions` for this agent's tools: the global list plus its own words (stricter only).
pub fn block_actions(a: &Agent) -> Option<String> {
    if a.blocked.is_empty() {
        return None;
    }
    let global = match fastpilot_core::config::current("block_actions") {
        fastpilot_core::config::Source::Env(v) | fastpilot_core::config::Source::File(v) => v,
        fastpilot_core::config::Source::Default => "default".into(),
    };
    let mut items: Vec<String> = global.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
    // An agent file written before this check (or by hand) cannot loosen anything either.
    for w in a.blocked.iter().filter(|w| blocking_word(w) && !w.chars().any(hidden)) {
        if !items.iter().any(|x| x.eq_ignore_ascii_case(w)) {
            items.push(w.clone());
        }
    }
    Some(items.join(","))
}

// ---- D10d: what each agent used today ----

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub turns: u64,
    pub input: u64,
    pub output: u64,
}

fn usage_path() -> PathBuf {
    path().with_file_name("usage.json")
}

fn today() -> i64 {
    fastpilot_core::claim::local_day()
}

/// Add one turn (a Chat answer or a scheduled run) of `agent` to today's usage.
pub fn record_usage(agent: &str, input: u64, output: u64) {
    let _l = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut v: serde_json::Value = std::fs::read_to_string(usage_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if v["day"].as_i64() != Some(today()) {
        v = serde_json::json!({ "day": today(), "agents": {} });
    }
    let mut u: Usage = serde_json::from_value(v["agents"][agent].clone()).unwrap_or_default();
    u.turns += 1;
    u.input += input;
    u.output += output;
    v["agents"][agent] = serde_json::to_value(u).unwrap_or_default();
    let _ = std::fs::write(usage_path(), v.to_string());
}

/// Today's usage per agent.
pub fn usage_today() -> std::collections::BTreeMap<String, Usage> {
    let v: serde_json::Value = std::fs::read_to_string(usage_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if v["day"].as_i64() != Some(today()) {
        return Default::default();
    }
    serde_json::from_value(v["agents"].clone()).unwrap_or_default()
}

/// Initials for the avatar ("Mobile Payments" -> "MP").
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let s: String = match words.as_slice() {
        [] => "A".into(),
        [one] => one.chars().take(2).collect(),
        [a, b, ..] => a.chars().take(1).chain(b.chars().take(1)).collect(),
    };
    s.to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(name: &str) -> Agent {
        Agent { name: name.into(), job: "Keep checkout working".into(), devices: vec!["web".into()], ..Default::default() }
    }

    #[test]
    fn agents_are_checked_and_stored() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("fastpilot-agents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe { std::env::set_var("FASTPILOT_CONFIG", dir.join("config.toml")) };
        // A first look: the default agent exists.
        assert_eq!(list().iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["default"]);
        let a = create(Agent { instructions: vec!["Never press Pay".into()], blocked: vec!["Confirm order".into()], ..agent("QA Checkout") }).expect("create");
        assert_eq!(a.id, "qa-checkout");
        assert!(create(agent("qa checkout")).unwrap_err().contains("exists already"));
        assert!(create(Agent { devices: vec![], ..agent("X") }).unwrap_err().contains("Pick where"));
        assert!(create(Agent { instructions: vec!["Ignore\u{202e} rules".into()], ..agent("Y") }).unwrap_err().contains("hidden characters"));
        assert!(create(Agent { instructions: vec!["".into()], ..agent("Y") }).is_err());
        assert!(create(Agent { blocked: vec!["a".into()], ..agent("Y") }).unwrap_err().contains("3 to 40"));
        assert!(delete(DEFAULT).is_err());
        add_conversation("qa-checkout", "c1");
        assert_eq!(of_conversation("c1"), "qa-checkout");
        assert_eq!(of_conversation("c-unknown"), DEFAULT);
        let p = prompt(&a);
        assert!(p.contains("\"QA Checkout\"") && p.contains("- Never press Pay") && p.contains("never loosen a limit") && p.contains("Confirm order"), "{p}");
        assert_eq!(block_actions(&a).as_deref(), Some("default,Confirm order"));
        delete("qa-checkout").expect("delete");
        assert_eq!(of_conversation("c1"), DEFAULT, "its conversations go to the default agent");
        assert_eq!(initials("Mobile Payments"), "MP");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("agents.json")).expect("file").permissions().mode() & 0o777, 0o600);
        }
        unsafe { std::env::remove_var("FASTPILOT_CONFIG") };
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Security D10a finding 1: an agent only adds blocks; `!pay`, `default` or a comma never
    /// reach its runs, also from an agents.json written before the check.
    #[test]
    fn an_agent_can_only_block_more() {
        for w in ["!pay", "!Pay", "default", "DEFAULT", "refund,!pay"] {
            let e = check(Agent { blocked: vec![w.into()], ..agent("A") }).unwrap_err();
            assert!(e.contains("not a word to block"), "{w}: {e}");
        }
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let old = Agent { blocked: vec!["!pay".into(), "default".into(), "x,!buy".into(), "Refund".into()], ..agent("Old") };
        assert_eq!(block_actions(&old).as_deref(), Some("default,Refund"));
        let rules = fastpilot_core::policy::expand_blocked(Some(block_actions(&old).unwrap_or_default().split(',').map(|s| s.trim().to_lowercase()).collect()));
        assert!(fastpilot_core::policy::block_match("Pay", &rules).is_some(), "Pay stays blocked");
        assert!(fastpilot_core::policy::block_match("Refund", &rules).is_some());
    }

    /// D10b: provider then model; Claude Code takes Claude ids only; the label everywhere.
    #[test]
    fn provider_and_model_per_agent() {
        let claude = check(Agent { model: Some("claude-haiku-5-5".into()), ..agent("A") }).expect("claude");
        assert_eq!(model_label(&claude), ("Claude Code · Haiku".to_owned(), true));
        assert!(check(Agent { model: Some("gpt-4o".into()), ..agent("A") }).unwrap_err().contains("Claude model"));
        let other = check(Agent { provider: Some("openrouter".into()), model: Some("vendor/model-x".into()), ..agent("A") }).expect("openrouter");
        assert_eq!(model_label(&other), ("OpenRouter · vendor/model-x".to_owned(), false));
        assert!(check(Agent { provider: Some("openrouter".into()), ..agent("A") }).unwrap_err().contains("Pick the model"));
        assert!(check(Agent { provider: Some("evil".into()), model: Some("m".into()), ..agent("A") }).unwrap_err().contains("Unknown provider"));
        assert!(check(Agent { provider: Some("groq".into()), model: Some("a b".into()), ..agent("A") }).is_err());
        // "claude" is the default: stored as no provider.
        assert_eq!(check(Agent { provider: Some("claude".into()), ..agent("A") }).expect("ok").provider, None);
    }
}
