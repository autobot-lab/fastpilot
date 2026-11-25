//! E8 + E6 (v0.7.2): the AI providers fastpilot can use, and which model each agent uses.
//!
//! - **Providers**: Claude Code is built in. OpenAI, OpenRouter and Groq know their own address and
//!   need only a key; Ollama runs on this Mac, needs no key and is found by itself; "other" is any
//!   OpenAI-compatible server at an address the person gives. They are kept in
//!   `~/.fastpilot/providers.json` (owner-only, no key in it). A key goes to the key store, tied to
//!   the provider's own address, and is sent nowhere else.
//! - **Model lists (E6)**: `GET {base}/models` from this process (never from the page), at most 500
//!   plain-text ids. A list that cannot be loaded says so; a model id the person types still works.
//! - **Models per agent**: each agent has Replay and Explore and repair, each "Same as default"
//!   (`None`) or a provider and model. The defaults are the `model_replay` / `model_explore`
//!   settings (a Claude id or `<provider>:<model>`).
//! - **No silent switch**: a run whose provider does not answer stops with a plain line; nothing
//!   goes to another provider without the person's own change.
//!
//! The words are FastUI/UX's TEXT-terminal-mcp.md section 8 (T8.2–T8.6), used as written.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::control::{Done, Fail, Surface, fail, q};

/// A provider people can add.
pub struct Kind {
    /// The typed name (`fastpilot provider add <id>`).
    pub id: &'static str,
    pub name: &'static str,
    /// Its own address; `None`: "other" (the person gives it).
    pub base: Option<&'static str>,
    /// The key store account (the older `custom` for "other").
    pub account: &'static str,
    /// The line in "Add a provider".
    pub line: &'static str,
}

pub const KINDS: &[Kind] = &[
    Kind { id: "openai", name: "OpenAI", base: Some("https://api.openai.com/v1"), account: "openai", line: "api.openai.com · needs an API key" },
    Kind { id: "openrouter", name: "OpenRouter", base: Some("https://openrouter.ai/api/v1"), account: "openrouter", line: "One key, many models" },
    Kind { id: "groq", name: "Groq", base: Some("https://api.groq.com/openai/v1"), account: "groq", line: "Fast hosted models · needs an API key" },
    Kind { id: "ollama", name: "Ollama", base: Some("http://127.0.0.1:11434/v1"), account: "ollama", line: "Runs models on this Mac · no key" },
    Kind { id: "other", name: "Other (OpenAI-compatible)", base: None, account: "custom", line: "Any server with an OpenAI-style address" },
];

/// The typed names, as the errors list them.
pub const TYPED: &str = "openai, openrouter, groq, ollama, other";

/// At most this many model ids per provider (E6).
pub const MAX_MODELS: usize = 500;

/// A provider by its typed name (the older setting value `custom` is "other").
pub fn kind(id: &str) -> Option<&'static Kind> {
    let id = if id == "custom" { "other" } else { id };
    KINDS.iter().find(|k| k.id == id)
}

/// The typed name for a provider name or setting value.
pub fn canonical(id: &str) -> String {
    if id == "custom" { "other".into() } else { id.trim().to_lowercase() }
}

// ---- choices --------------------------------------------------------------------------------

/// A model at a provider (`claude` for Claude Code).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub provider: String,
    pub model: String,
}

impl Choice {
    pub fn claude(model: &str) -> Self {
        Choice { provider: "claude".into(), model: model.into() }
    }
    pub fn is_claude(&self) -> bool {
        self.provider == "claude"
    }
    /// As settings keep it: a Claude id, or `<provider>:<model>`.
    pub fn stored(&self) -> String {
        if self.is_claude() { self.model.clone() } else { format!("{}:{}", self.provider, self.model) }
    }
}

/// Replay or Explore and repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Replay,
    Explore,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Replay => "Replay",
            Role::Explore => "Explore and repair",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Role::Replay => "replay",
            Role::Explore => "explore",
        }
    }
    fn setting(self) -> &'static str {
        match self {
            Role::Replay => "model_replay",
            Role::Explore => "model_explore",
        }
    }
    fn env(self) -> &'static str {
        match self {
            Role::Replay => "FASTPILOT_MODEL_REPLAY",
            Role::Explore => "FASTPILOT_MODEL_EXPLORE",
        }
    }
    fn builtin(self) -> &'static str {
        match self {
            Role::Replay => crate::run::DEFAULT_REPLAY,
            Role::Explore => crate::run::DEFAULT_EXPLORE,
        }
    }
}

/// "claude-sonnet-5-5" -> "Sonnet 5.5"; another id stays as it is.
pub fn claude_name(id: &str) -> String {
    let Some(rest) = id.strip_prefix("claude-") else { return id.to_owned() };
    let parts: Vec<&str> = rest.split('-').collect();
    match parts.as_slice() {
        [family, major, minor] if major.chars().all(|c| c.is_ascii_digit()) && minor.chars().all(|c| c.is_ascii_digit()) => {
            let mut f = family.chars();
            let fam = f.next().map(|c| c.to_uppercase().collect::<String>() + f.as_str()).unwrap_or_default();
            format!("{fam} {major}.{minor}")
        }
        _ => id.to_owned(),
    }
}

/// The model as people read it ("Haiku 5.5", "deepseek/deepseek-v4-flash").
pub fn model_name(c: &Choice) -> String {
    if c.is_claude() { claude_name(&c.model) } else { c.model.clone() }
}

/// The provider as sentences name it: Claude Code, OpenRouter, or the host for "other".
pub fn provider_name(provider: &str) -> String {
    match provider {
        "claude" => "Claude Code".into(),
        "other" | "custom" => host(&base_of_id("other").unwrap_or_default()).unwrap_or_else(|| "Other".into()),
        p => kind(p).map_or_else(|| p.to_owned(), |k| k.name.to_owned()),
    }
}

/// "Haiku 5.5 (Claude Code)", "cb/x (api.inferhub.dev)": the run report's model line.
pub fn label(c: &Choice) -> String {
    format!("{} ({})", model_name(c), provider_name(&c.provider))
}

/// `host[:port]` of an address.
pub fn host(url: &str) -> Option<String> {
    fastpilot_core::config::origin(url).and_then(|o| o.split_once("://").map(|(_, h)| h.to_owned())).filter(|h| !h.is_empty())
}

/// Plain http to this computer only; https anywhere.
fn local(url: &str) -> bool {
    fastpilot_core::policy::remote_host(url).is_none()
}

// ---- the registry ---------------------------------------------------------------------------

/// Connection state, in the dashboard pills' words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Connected,
    NotRunning,
    KeyNotAccepted,
    NotAnswering,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Connected => "Connected",
            State::NotRunning => "Not running",
            State::KeyNotAccepted => "Key not accepted",
            State::NotAnswering => "Not answering",
        }
    }
}

/// One connected provider. No key here: that is in the key store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// Only for "other" (and an Ollama at another port of this computer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    /// Model ids the person typed (Edit, or a list that could not be loaded).
    #[serde(default)]
    pub typed: Vec<String>,
    pub state: State,
    #[serde(default)]
    pub list_failed: bool,
    #[serde(default)]
    pub checked_ms: u64,
    #[serde(default)]
    pub added_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    providers: Vec<Entry>,
    /// Ollama was removed: it is not found by itself any more (until it is added again).
    #[serde(default)]
    ollama_off: bool,
}

fn path() -> PathBuf {
    fastpilot_core::config::path().parent().map(std::path::Path::to_path_buf).unwrap_or_else(std::env::temp_dir).join("providers.json")
}

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn read() -> Store {
    std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write(s: &Store) -> Result<(), String> {
    let p = path();
    let dir = p.parent().map(std::path::Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
    crate::filestore::ensure_dir(&dir, true)?;
    crate::filestore::write_atomic(&dir, "providers.json", serde_json::to_string_pretty(s).unwrap_or_default().as_bytes(), true)
}

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> Result<T, Fail> {
    let _l = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = read();
    let out = f(&mut s);
    write(&s).map_err(|e| fail("failed", e))?;
    Ok(out)
}

pub fn entry(id: &str) -> Option<Entry> {
    let id = canonical(id);
    read().providers.into_iter().find(|e| e.id == id)
}

/// Ollama's address on this computer (`FASTPILOT_OLLAMA_URL` for another port; only this computer).
pub fn ollama_base() -> String {
    std::env::var("FASTPILOT_OLLAMA_URL").ok().map(|u| u.trim().trim_end_matches('/').to_owned()).filter(|u| (u.starts_with("http://") || u.starts_with("https://")) && local(u)).unwrap_or_else(|| "http://127.0.0.1:11434/v1".into())
}

/// Where a provider's requests (and its key) go.
pub fn base_of(e: &Entry) -> String {
    match (e.id.as_str(), &e.base_url) {
        (_, Some(b)) => b.clone(),
        ("ollama", None) => ollama_base(),
        (id, None) => kind(id).and_then(|k| k.base).unwrap_or_default().to_owned(),
    }
}

fn base_of_id(id: &str) -> Option<String> {
    let id = canonical(id);
    let var = |k: &str| fastpilot_core::config::var(k).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    entry(&id)
        .map(|e| base_of(&e))
        .or_else(|| (id == "ollama").then(ollama_base))
        // The older single `provider` setting (B2), with its own address.
        .or_else(|| legacy_provider().filter(|l| *l == id).and_then(|_| var("FASTPILOT_PROVIDER_BASE_URL")))
        .or_else(|| kind(&id).and_then(|k| k.base).map(str::to_owned))
}

/// Whether a provider can be used now without adding it (Claude Code, a connected one, Ollama on
/// this Mac, or the older `provider` setting).
pub fn connected(provider: &str) -> bool {
    let p = canonical(provider);
    p == "claude"
        || p == "ollama" && !read().ollama_off
        || entry(&p).is_some()
        || legacy_provider().is_some_and(|l| l == p)
        // A key saved for a hosted provider before v0.7.2 (`fastpilot config set-key`, D10b agents).
        || kind(&p).is_some_and(|k| k.base.is_some() && k.id != "ollama" && crate::keystore::get_stored(k.account).is_some())
}

/// The older single `provider` setting (B2), when it is not Claude Code.
fn legacy_provider() -> Option<String> {
    fastpilot_core::config::var("FASTPILOT_PROVIDER").ok().map(|v| canonical(&v)).filter(|v| !v.is_empty() && v != "claude")
}

// ---- defaults and per agent ------------------------------------------------------------------

/// A setting value: a Claude id, or `<provider>:<model>`.
fn parse_stored(v: &str) -> Option<Choice> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    if v.starts_with("claude-") {
        return Some(Choice::claude(v));
    }
    let (p, m) = v.split_once(':')?;
    let p = canonical(p);
    (kind(&p).is_some() && !m.trim().is_empty()).then(|| Choice { provider: p, model: m.trim().to_owned() })
}

/// The default model for `role`: the setting, else the older provider setting (B2), else built in.
pub fn default_choice(role: Role) -> Choice {
    if let Some(c) = fastpilot_core::config::var(role.env()).ok().as_deref().and_then(parse_stored) {
        return c;
    }
    let var = |k: &str| fastpilot_core::config::var(k).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    if let (Some(p), Some(m)) = (legacy_provider(), var("FASTPILOT_PROVIDER_MODEL")) {
        return Choice { provider: p, model: m };
    }
    Choice::claude(role.builtin())
}

/// An agent's own choice for `role` (`None`: the default). Before v0.7.2 an agent had one provider
/// and model (D10b) for everything: that stays its choice for both until it is changed here.
pub fn own_choice(a: &crate::agents::Agent, role: Role) -> Option<Choice> {
    if a.models_set {
        return match role {
            Role::Replay => a.replay.clone(),
            Role::Explore => a.explore.clone(),
        };
    }
    match (a.provider.as_deref(), a.model.as_deref()) {
        (None | Some("claude"), Some(m)) => Some(Choice::claude(m)),
        (Some(p), Some(m)) => Some(Choice { provider: canonical(p), model: m.to_owned() }),
        _ => None,
    }
}

/// What a role runs with.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The agent's own choice; `None`: Same as default.
    pub choice: Option<Choice>,
    pub effective: Choice,
    /// One plain line when its own provider is not connected any more.
    pub fallback: Option<String>,
}

pub fn resolve(a: Option<&crate::agents::Agent>, role: Role) -> Resolved {
    let own = a.and_then(|a| own_choice(a, role));
    match own {
        Some(c) if connected(&c.provider) => Resolved { effective: c.clone(), choice: Some(c), fallback: None },
        Some(c) => Resolved { fallback: Some(format!("{} is not connected, so this agent follows the default models.", provider_name(&c.provider))), choice: None, effective: default_choice(role) },
        None => Resolved { choice: None, effective: default_choice(role), fallback: None },
    }
}

/// The agent this process works for (`FASTPILOT_AGENT`), if any.
pub fn run_agent() -> Option<crate::agents::Agent> {
    std::env::var("FASTPILOT_AGENT").ok().and_then(|id| crate::agents::get(&id))
}

// ---- what a model input means ----------------------------------------------------------------

/// A model id as a provider may name it: one line, no spaces, no hidden characters.
fn plain_id(m: &str) -> bool {
    let n = m.chars().count();
    (1..=200).contains(&n) && !m.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c as u32, 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x206f | 0xfeff))
}

/// `opus`, `Sonnet 5.5`, `claude-haiku-5-5` -> the Claude id.
fn claude_alias(t: &str) -> Option<String> {
    let l = t.trim().to_lowercase();
    if crate::agents::CLAUDE_MODELS.iter().any(|(id, ..)| *id == l) {
        return Some(l);
    }
    let l = l.strip_prefix("claude").map(str::trim_start).map(|x| x.trim_start_matches('-')).unwrap_or(&l).to_owned();
    let mut words = l.split([' ', '-']).filter(|w| !w.is_empty());
    let family = words.next()?;
    let version: String = words.collect::<Vec<_>>().join(".");
    crate::agents::CLAUDE_MODELS.iter().map(|(id, ..)| *id).find(|id| {
        let n = claude_name(id).to_lowercase();
        let (f, v) = n.split_once(' ').unwrap_or((n.as_str(), ""));
        f == family && (version.is_empty() || version == v)
    }).map(str::to_owned)
}

/// The model ids a provider lists (and the ones typed for it).
pub fn listed(provider: &str) -> Vec<String> {
    if provider == "claude" {
        return crate::agents::CLAUDE_MODELS.iter().map(|(id, ..)| (*id).to_owned()).collect();
    }
    entry(provider).map(|e| e.models.into_iter().chain(e.typed).collect()).unwrap_or_default()
}

/// The connected providers in list order: Claude Code first.
fn connected_ids() -> Vec<String> {
    let s = read();
    let mut out = vec!["claude".to_owned()];
    for k in KINDS {
        if s.providers.iter().any(|e| e.id == k.id) {
            out.push(k.id.to_owned());
        }
    }
    out
}

/// "Claude Code, OpenRouter, Ollama": the connected providers, for a sentence.
pub fn connected_list() -> String {
    connected_names()
}

fn connected_names() -> String {
    connected_ids().iter().map(|p| provider_name(p)).collect::<Vec<_>>().join(", ")
}

/// A model input read: the choice (`None`: default) and whether the id could not be checked.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub choice: Option<Choice>,
    pub unchecked: bool,
}

fn not_found(typed: &str, s: Surface) -> Fail {
    let all: Vec<String> = connected_ids().iter().flat_map(|p| listed(p)).collect();
    let hint = closest(typed, &all).map(|m| format!("Did you mean {}? ", q(&m))).unwrap_or_default();
    fail(
        "model_not_found",
        match s {
            Surface::Cli => format!("Couldn’t find that model. Connected: {}. {hint}Run fastpilot provider models to see them. Nothing changed.", connected_names()),
            Surface::Mcp => format!("Couldn’t find that model. Connected: {}. {hint}Use `providers models` to see them. Nothing changed.", connected_names()),
        },
    )
}

fn closest(typed: &str, ids: &[String]) -> Option<String> {
    let t = typed.trim().to_lowercase();
    if t.len() < 3 {
        return None;
    }
    let mut hits: Vec<&String> = ids.iter().filter(|m| m.to_lowercase().contains(&t)).collect();
    hits.dedup();
    if hits.len() == 1 {
        return Some(hits[0].clone());
    }
    None
}

pub fn not_connected(provider: &str, s: Surface) -> Fail {
    let k = kind(provider).map_or(provider, |k| k.id);
    let name = provider_name(provider);
    fail(
        "provider_not_connected",
        match s {
            Surface::Cli => format!("{name} is not connected. Add it first: fastpilot provider add {k}. Nothing changed."),
            Surface::Mcp => format!("{name} is not connected. Tell the user to connect it in the dashboard: Settings › AI model › Add a provider, or to run fastpilot provider add {k} in Terminal. Nothing changed."),
        },
    )
}

/// Read a model as the CLI, MCP, the dashboard and Chat take it (T8.0 rule 3).
pub fn parse(input: &str, s: Surface) -> Result<Parsed, Fail> {
    let t = input.trim();
    if t.eq_ignore_ascii_case("default") {
        return Ok(Parsed { choice: None, unchecked: false });
    }
    if let Some(id) = claude_alias(t) {
        return Ok(Parsed { choice: Some(Choice::claude(&id)), unchecked: false });
    }
    // `<provider>:<model>` when the text before the first colon is a provider.
    if let Some((p, m)) = t.split_once(':')
        && (kind(&canonical(p)).is_some() || canonical(p) == "claude")
    {
        let p = canonical(p);
        if p == "claude" {
            return claude_alias(m).map(|id| Parsed { choice: Some(Choice::claude(&id)), unchecked: false }).ok_or_else(|| not_found(m, s));
        }
        if !connected(&p) {
            return Err(not_connected(&p, s));
        }
        let m = m.trim();
        if !plain_id(m) {
            return Err(not_found(m, s));
        }
        let e = entry(&p);
        let list = listed(&p);
        // Ollama found by itself has no kept list: its ids come from the picker (or the person).
        let unchecked = e.as_ref().map_or(p != "ollama", |e| e.list_failed) && !list.iter().any(|x| x == m);
        let unchecked_ok = unchecked || (e.is_none() && p == "ollama");
        if !unchecked_ok && !list.iter().any(|x| x == m) {
            return Err(not_found(m, s));
        }
        return Ok(Parsed { choice: Some(Choice { provider: p, model: m.to_owned() }), unchecked });
    }
    if !plain_id(t) {
        return Err(not_found(t, s));
    }
    let hits: Vec<String> = connected_ids().into_iter().filter(|p| p != "claude" && listed(p).iter().any(|m| m == t)).collect();
    match hits.as_slice() {
        [] => Err(not_found(t, s)),
        [p] => Ok(Parsed { choice: Some(Choice { provider: p.clone(), model: t.to_owned() }), unchecked: false }),
        many => Err(fail(
            "ambiguous_model",
            format!(
                "{} is in {} providers: {}. Say which: {}. Nothing changed.",
                q(t),
                many.len(),
                many.iter().map(|p| provider_name(p)).collect::<Vec<_>>().join(", "),
                many.iter().map(|p| format!("{p}:{t}")).collect::<Vec<_>>().join(" or ")
            ),
        )),
    }
}

// ---- where the data goes (T8.0 rule 5) -------------------------------------------------------

/// The data line and the "not tried" line for a choice (none for Claude Code).
pub fn data_lines(c: &Choice) -> Vec<String> {
    if c.is_claude() {
        return vec![];
    }
    let place = if c.provider == "ollama" {
        "Page text and screenshots from this agent’s tests stay on this Mac.".to_owned()
    } else {
        format!("Page text and screenshots from this agent’s tests go to {}.", host(&base_of_id(&c.provider).unwrap_or_default()).unwrap_or_else(|| provider_name(&c.provider)))
    };
    vec![place, NOT_TRIED.into()]
}

pub const NOT_TRIED: &str = "This model is not tried with fastpilot yet. Run a few tests and check the results before you rely on it.";

/// "Ollama is not running right now. …" when a chosen provider was down at its last check.
fn down_line(c: &Choice) -> Option<String> {
    let e = entry(&c.provider)?;
    match e.state {
        State::NotRunning => Some(format!("{} is not running right now. Runs that need this model stop until it is.", provider_name(&c.provider))),
        State::NotAnswering => Some(format!("{} is not answering right now. Runs that need this model stop until it does.", host(&base_of(&e)).unwrap_or_else(|| provider_name(&c.provider)))),
        _ => None,
    }
}

// ---- changing models (T8.2, T8.6) ------------------------------------------------------------

/// A model change: the result, and the lines that follow it.
#[derive(Debug, Clone, PartialEq)]
pub struct Changed {
    pub done: Done,
    pub lines: Vec<String>,
    /// Something changed (else "… already uses …. Nothing changed.").
    pub changed: bool,
}

fn after_lines(new: &[(Role, Parsed)], effective: &dyn Fn(Role) -> Choice) -> Vec<String> {
    let mut lines = Vec::new();
    for (role, p) in new {
        let c = p.choice.clone().unwrap_or_else(|| effective(*role));
        for l in data_lines(&c) {
            if !lines.contains(&l) {
                lines.push(l);
            }
        }
        if let Some(d) = down_line(&c) {
            lines.push(d);
        }
        if p.unchecked {
            lines.push(format!("The model list of {} could not be loaded, so {} was not checked.", provider_name(&c.provider), q(&c.model)));
        }
    }
    lines.push("It applies from the next run.".into());
    lines
}

/// `fastpilot agent model <agent> --replay X --explore Y`, MCP `agents model`, the dashboard and
/// Chat: change an agent's models. Only that agent changes.
pub fn change_agent(agent: &str, replay: Option<&str>, explore: Option<&str>, s: Surface) -> Result<Changed, Fail> {
    let mut a = crate::control::resolve_agent(agent)?;
    if replay.is_none() && explore.is_none() {
        return Err(no_model_given(s));
    }
    let mut new: Vec<(Role, Parsed)> = Vec::new();
    for (role, v) in [(Role::Replay, replay), (Role::Explore, explore)] {
        if let Some(v) = v {
            new.push((role, parse(v, s)?));
        }
    }
    let before = (own_choice(&a, Role::Replay), own_choice(&a, Role::Explore), a.models_set);
    let effective_before = |r: Role| resolve(Some(&a), r).effective;
    let mut parts: Vec<(Role, String, String)> = Vec::new();
    let mut same: Option<(Role, String)> = None;
    for (role, p) in &new {
        let old_own = resolve(Some(&a), *role).choice;
        let old_eff = effective_before(*role);
        if old_own == p.choice {
            same.get_or_insert((*role, model_name(&old_eff)));
            continue;
        }
        let to = match &p.choice {
            Some(c) => model_name(c),
            None => format!("default ({})", model_name(&default_choice(*role))),
        };
        parts.push((*role, model_name(&old_eff), to));
    }
    if parts.is_empty() {
        let (role, name) = same.unwrap_or((Role::Replay, String::new()));
        return Ok(Changed { done: Done { line: format!("{} in {} already uses {name}. Nothing changed.", role.label(), a.name), undo: None }, lines: vec![], changed: false });
    }
    let line = match parts.as_slice() {
        [(role, from, to)] => format!("Changed {} model in {} · {from} → {to}", role.label(), a.name),
        many => format!("Changed models in {} · {}", a.name, many.iter().map(|(r, f, t)| format!("{}: {f} → {t}", r.label())).collect::<Vec<_>>().join(" · ")),
    };
    let (mut replay_now, mut explore_now) = (before.0.clone(), before.1.clone());
    for (role, p) in &new {
        match role {
            Role::Replay => replay_now.clone_from(&p.choice),
            Role::Explore => explore_now.clone_from(&p.choice),
        }
    }
    let id = a.id.clone();
    a.replay = replay_now;
    a.explore = explore_now;
    a.models_set = true;
    let changed_agent = crate::agents::update(a).map_err(|e| fail("failed", e))?;
    let lines = after_lines(&new, &|r| resolve(Some(&changed_agent), r).effective);
    let undo = crate::undo::record(&line, &format!("models:{id}"), crate::undo::Op::AgentModels { agent: id, replay: before.0, explore: before.1, models_set: before.2 });
    Ok(Changed { done: Done { line, undo }, lines, changed: true })
}

fn no_model_given(s: Surface) -> Fail {
    fail(
        "no_model_given",
        match s {
            Surface::Cli => "Say which model: --replay <model>, --explore <model>, or both. Use “default” to follow the default. Nothing changed.",
            Surface::Mcp => "Say which model: replay, explore, or both. Use “default” to follow the default. Nothing changed.",
        },
    )
}

/// `--default`: change the default for every agent that has not chosen its own.
pub fn change_default(replay: Option<&str>, explore: Option<&str>, s: Surface) -> Result<Changed, Fail> {
    if replay.is_none() && explore.is_none() {
        return Err(no_model_given(s));
    }
    let mut new: Vec<(Role, Parsed)> = Vec::new();
    for (role, v) in [(Role::Replay, replay), (Role::Explore, explore)] {
        if let Some(v) = v {
            new.push((role, parse(v, s)?));
        }
    }
    let raw = |r: Role| fastpilot_core::config::file_values().remove(r.setting());
    let before = (raw(Role::Replay), raw(Role::Explore));
    let agents = crate::agents::list();
    let mut parts: Vec<(Role, String, String, usize)> = Vec::new();
    let mut same: Option<(Role, String)> = None;
    for (role, p) in &new {
        let old = default_choice(*role);
        let new_c = p.choice.clone().unwrap_or_else(|| Choice::claude(role.builtin()));
        if old == new_c {
            same.get_or_insert((*role, model_name(&old)));
            continue;
        }
        let keep = agents.iter().filter(|a| own_choice(a, *role).is_some()).count();
        parts.push((*role, model_name(&old), model_name(&new_c), keep));
    }
    if parts.is_empty() {
        let (role, name) = same.unwrap_or((Role::Replay, String::new()));
        return Ok(Changed { done: Done { line: format!("The default {} model already is {name}. Nothing changed.", role.label()), undo: None }, lines: vec![], changed: false });
    }
    for (role, p) in &new {
        let v = p.choice.as_ref().map(Choice::stored);
        // A free setting: read from the file each time, so this process sees it at once too.
        fastpilot_core::config::write(role.setting(), v.as_deref()).map_err(|e| fail("failed", e))?;
    }
    let keeps = |n: usize| match n {
        0 => String::new(),
        1 => " · 1 agent keeps its own".into(),
        n => format!(" · {n} agents keep their own"),
    };
    let line = match parts.as_slice() {
        [(role, from, to, keep)] => format!("Changed default {} model · {from} → {to}{}", role.label(), keeps(*keep)),
        many => format!("Changed default models · {}", many.iter().map(|(r, f, t, _)| format!("{}: {f} → {t}", r.label())).collect::<Vec<_>>().join(" · ")),
    };
    let lines = after_lines(&new, &default_choice);
    let undo = crate::undo::record(&line, "models:default", crate::undo::Op::DefaultModels { replay: before.0, explore: before.1 });
    Ok(Changed { done: Done { line, undo }, lines, changed: true })
}

/// Undo of a default change: the settings as they were.
pub fn restore_defaults(replay: Option<&str>, explore: Option<&str>) -> Result<(), Fail> {
    for (role, v) in [(Role::Replay, replay), (Role::Explore, explore)] {
        fastpilot_core::config::write(role.setting(), v).map_err(|e| fail("failed", e))?;
    }
    Ok(())
}

/// `fastpilot agent model <agent>`: the two rows.
pub fn show_agent(a: &crate::agents::Agent) -> String {
    let row = |r: Role| {
        let x = resolve(Some(a), r);
        let place = provider_place(&x.effective);
        format!("  {:<20}{} · {place}{}", r.label(), model_name(&x.effective), if x.choice.is_none() { " · default" } else { "" })
    };
    let mut out = vec![format!("{} · models", a.name), row(Role::Replay), row(Role::Explore)];
    for r in [Role::Replay, Role::Explore] {
        if let Some(f) = resolve(Some(a), r).fallback
            && !out.contains(&format!("  {f}"))
        {
            out.push(format!("  {f}"));
        }
    }
    out.join("\n")
}

/// "Claude Code", "api.inferhub.dev", "Ollama".
fn provider_place(c: &Choice) -> String {
    if c.provider == "other" { host(&base_of_id("other").unwrap_or_default()).unwrap_or_else(|| "Other".into()) } else { provider_name(&c.provider) }
}

/// `fastpilot agent model`: the defaults.
pub fn show_defaults() -> String {
    let row = |r: Role| {
        let c = default_choice(r);
        format!("  {:<20}{} · {}", r.label(), model_name(&c), provider_place(&c))
    };
    ["Default models (every agent that has not chosen its own)".to_owned(), row(Role::Replay), row(Role::Explore)].join("\n")
}

/// The dashboard's and MCP's view of one role.
pub fn role_json(a: Option<&crate::agents::Agent>, r: Role) -> serde_json::Value {
    let x = resolve(a, r);
    let c = &x.effective;
    let e = entry(&c.provider);
    let state = if c.is_claude() { State::Connected } else { e.as_ref().map_or(State::Connected, |e| e.state) };
    let lines = data_lines(c);
    serde_json::json!({
        "choice": x.choice.as_ref().map(choice_json),
        "effective": choice_json(c),
        "is_default": x.choice.is_none(),
        "provider_name": provider_name(&c.provider),
        "host": if c.is_claude() || c.provider == "ollama" { None } else { host(&base_of_id(&c.provider).unwrap_or_default()) },
        "state": state,
        "note": lines.first(),
        "warn": lines.get(1),
        "fallback": x.fallback,
    })
}

pub fn choice_json(c: &Choice) -> serde_json::Value {
    serde_json::json!({ "provider": c.provider, "model": c.model, "name": model_name(c) })
}

/// About › Model: "Haiku 5.5 · Sonnet 5.5".
pub fn short_label(a: &crate::agents::Agent) -> serde_json::Value {
    serde_json::json!({ "replay": model_name(&resolve(Some(a), Role::Replay).effective), "explore": model_name(&resolve(Some(a), Role::Explore).effective) })
}

// ---- model lists (E6) ------------------------------------------------------------------------

/// Why a provider could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trouble {
    /// Nobody listens on this computer (Ollama, or another local server).
    NotRunning,
    /// A server elsewhere did not answer.
    NotAnswering,
    KeyNotAccepted,
    /// It answered, but not with a list of models.
    ListFailed,
}

/// The longest a list may be, in bytes (a list of 500 ids is far smaller).
const MAX_LIST_BYTES: usize = 4 << 20;

/// `GET {base}/models` with the key (if any) as a bearer header: at most 500 plain ids. Redirects
/// are not followed (the key would go along).
pub async fn fetch(base: &str, key: Option<&str>, wait: Duration) -> Result<Vec<String>, Trouble> {
    let client = reqwest::Client::builder().timeout(wait).connect_timeout(wait.min(Duration::from_secs(5))).redirect(reqwest::redirect::Policy::none()).build().map_err(|_| Trouble::ListFailed)?;
    let mut req = client.get(format!("{}/models", base.trim_end_matches('/'))).header("User-Agent", concat!("fastpilot/", env!("CARGO_PKG_VERSION")));
    if let Some(k) = key {
        req = req.bearer_auth(k);
    }
    let resp = req.send().await.map_err(|e| if e.is_connect() && local(base) { Trouble::NotRunning } else if e.is_connect() || e.is_timeout() { Trouble::NotAnswering } else { Trouble::ListFailed })?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(Trouble::KeyNotAccepted);
    }
    if !status.is_success() {
        return Err(if status.is_server_error() && status != reqwest::StatusCode::NOT_IMPLEMENTED { Trouble::NotAnswering } else { Trouble::ListFailed });
    }
    let mut resp = resp;
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|_| Trouble::ListFailed)? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_LIST_BYTES {
            return Err(Trouble::ListFailed);
        }
    }
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|_| Trouble::ListFailed)?;
    let items = v["data"].as_array().or_else(|| v["models"].as_array()).or_else(|| v.as_array()).ok_or(Trouble::ListFailed)?;
    let mut ids: Vec<String> = Vec::new();
    for it in items {
        let id = it["id"].as_str().or_else(|| it["name"].as_str()).or_else(|| it.as_str()).unwrap_or("").trim();
        if plain_id(id) && !ids.iter().any(|x| x == id) {
            ids.push(id.to_owned());
            if ids.len() == MAX_MODELS {
                break;
            }
        }
    }
    Ok(ids)
}

fn now_ms() -> u64 {
    fastpilot_core::change::now_ms()
}

/// The key saved for a provider, when it was saved for this address; `Err`: saved for another one
/// (it is not sent).
fn key_for(k: &Kind, base: &str) -> Result<Option<String>, ()> {
    match crate::keystore::get_stored(k.account) {
        None => Ok(None),
        Some((key, Some(o))) if fastpilot_core::config::origin(base).as_deref() == Some(o.as_str()) => Ok(Some(key)),
        // A hosted provider's key saved before keys were bound: only its own address.
        Some((key, None)) if k.base.is_some_and(|b| fastpilot_core::config::origin(b) == fastpilot_core::config::origin(base)) && k.id != "ollama" => Ok(Some(key)),
        Some(_) => Err(()),
    }
}

/// Check a connected provider again and keep its list. Returns the entry as it is now.
pub async fn refresh_entry(id: &str) -> Result<Entry, (Entry, Trouble)> {
    let id = canonical(id);
    let mut e = entry(&id).unwrap_or_else(|| Entry { id: id.clone(), base_url: None, models: vec![], typed: vec![], state: State::Connected, list_failed: false, checked_ms: 0, added_ms: now_ms() });
    let Some(k) = kind(&id) else { return Err((e, Trouble::ListFailed)) };
    let base = base_of(&e);
    let r = match key_for(k, &base) {
        Err(()) => Err(Trouble::KeyNotAccepted),
        Ok(key) => fetch(&base, key.as_deref(), Duration::from_secs(10)).await,
    };
    e.checked_ms = now_ms();
    let out = match r {
        Ok(models) => {
            e.models = models;
            e.state = State::Connected;
            e.list_failed = false;
            Ok(())
        }
        Err(t) => {
            match t {
                Trouble::NotRunning => e.state = State::NotRunning,
                Trouble::NotAnswering => e.state = State::NotAnswering,
                Trouble::KeyNotAccepted => e.state = State::KeyNotAccepted,
                Trouble::ListFailed => {
                    e.state = State::Connected;
                    e.list_failed = true;
                }
            }
            Err(t)
        }
    };
    // Only a provider that is connected is kept (Ollama found by itself stays unlisted).
    if entry(&id).is_some() {
        let saved = e.clone();
        let _ = with_store(|s| {
            if let Some(x) = s.providers.iter_mut().find(|x| x.id == saved.id) {
                *x = saved;
            }
        });
    }
    out.map(|()| e.clone()).map_err(|t| (e, t))
}

/// Ollama on this Mac, looked for by itself (at most `wait`): its models, or `None` when it is not
/// there (or was removed).
pub async fn find_ollama(wait: Duration) -> Option<Result<Vec<String>, Trouble>> {
    if read().ollama_off {
        return None;
    }
    match fetch(&ollama_base(), None, wait).await {
        Err(Trouble::NotRunning | Trouble::NotAnswering) => None,
        r => Some(r),
    }
}

// ---- provider list, add, remove, refresh, models (T8.3) ---------------------------------------

/// One row of `provider list` and `GET /api/providers`.
#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub state: State,
    /// "openrouter.ai · 148 models", "this Mac · found automatically · 3 models".
    pub line: String,
    /// The sentence for a state that is not Connected (and Ollama with no models).
    pub message: Option<String>,
    pub host: Option<String>,
    pub base_url: Option<String>,
    pub models: Vec<String>,
    pub typed: Vec<String>,
    pub list_failed: bool,
    pub auto: bool,
    pub has_key: bool,
    pub checked_ms: u64,
}

fn count(n: usize) -> String {
    if n == 1 { "1 model".into() } else { format!("{n} models") }
}

fn state_message(id: &str, name: &str, host_: &str, st: State) -> Option<String> {
    match st {
        State::Connected => None,
        State::NotRunning => Some(format!("Start {name} on this Mac, then run: fastpilot provider refresh {id}")),
        State::KeyNotAccepted => Some(format!("{host_} did not accept the key. Run: fastpilot provider add {id}")),
        State::NotAnswering => Some(format!("{host_} did not answer. Run: fastpilot provider refresh {id}")),
    }
}

/// The connected providers, Claude Code first; Ollama on this Mac is looked for (≤ 0.5 s).
pub async fn rows() -> Vec<Row> {
    let claude_version = tokio::time::timeout(Duration::from_secs(3), crate::run::claude_ready()).await.ok().and_then(Result::ok).and_then(|l| l.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit())).map(str::to_owned));
    let mut out = vec![Row {
        id: "claude".into(),
        name: "Claude Code".into(),
        state: State::Connected,
        line: format!("uses your login{} · 3 models", claude_version.map(|v| format!(" · version {v}")).unwrap_or_default()),
        message: None,
        host: None,
        base_url: None,
        models: listed("claude"),
        typed: vec![],
        list_failed: false,
        auto: false,
        has_key: false,
        checked_ms: 0,
    }];
    let store = read();
    for k in KINDS {
        let e = store.providers.iter().find(|e| e.id == k.id).cloned();
        let (e, auto) = match (e, k.id) {
            (Some(e), "ollama") => {
                // On this Mac: checked now (cheap), not from the last check.
                let mut e = e;
                match fetch(&base_of(&e), None, Duration::from_millis(500)).await {
                    Ok(m) => {
                        e.models = m;
                        e.state = State::Connected;
                    }
                    Err(Trouble::NotRunning | Trouble::NotAnswering) => e.state = State::NotRunning,
                    Err(_) => e.list_failed = true,
                }
                (e, false)
            }
            (Some(e), _) => (e, false),
            (None, "ollama") => match find_ollama(Duration::from_millis(500)).await {
                Some(Ok(m)) => (Entry { id: "ollama".into(), base_url: None, models: m, typed: vec![], state: State::Connected, list_failed: false, checked_ms: now_ms(), added_ms: 0 }, true),
                Some(Err(_)) => (Entry { id: "ollama".into(), base_url: None, models: vec![], typed: vec![], state: State::Connected, list_failed: true, checked_ms: now_ms(), added_ms: 0 }, true),
                None => continue,
            },
            (None, _) => continue,
        };
        let base = base_of(&e);
        let h = host(&base).unwrap_or_default();
        let name = if k.id == "other" { h.clone() } else { k.name.to_owned() };
        let n = e.models.len() + e.typed.iter().filter(|t| !e.models.contains(t)).count();
        let line = if k.id == "ollama" {
            if e.models.is_empty() && !e.list_failed && e.state == State::Connected {
                "this Mac · no models yet. Download one in Ollama, then run: fastpilot provider refresh ollama".to_owned()
            } else {
                format!("this Mac{} · {}", if auto { " · found automatically" } else { "" }, count(n))
            }
        } else {
            format!("{h} · {}", count(n))
        };
        let message = state_message(k.id, &name, &h, e.state).or_else(|| {
            (e.list_failed && e.state == State::Connected).then(|| format!("Could not load the list of models from {name}. Check the address and key, then run: fastpilot provider refresh {}. A model id you type still works.", k.id))
        });
        out.push(Row {
            id: k.id.into(),
            name,
            state: e.state,
            line,
            message,
            host: (k.id != "ollama").then_some(h),
            base_url: matches!(k.id, "other" | "ollama").then_some(base),
            models: e.models.clone(),
            typed: e.typed.clone(),
            list_failed: e.list_failed,
            auto,
            has_key: crate::keystore::get_stored(k.account).is_some(),
            checked_ms: e.checked_ms,
        });
    }
    out
}

pub fn row_json(r: &Row) -> serde_json::Value {
    let models: Vec<serde_json::Value> =
        r.models.iter().chain(r.typed.iter().filter(|t| !r.models.contains(t))).take(MAX_MODELS).map(|m| serde_json::json!({ "id": m, "name": if r.id == "claude" { claude_name(m) } else { m.clone() } })).collect();
    serde_json::json!({
        "id": r.id,
        "name": r.name,
        "host": if r.id == "ollama" { Some("this Mac".to_owned()) } else { r.host.clone() },
        "base_url": r.base_url,
        "state": r.state,
        "line": if r.id == "claude" { format!("Uses your login{}", r.line.strip_prefix("uses your login").unwrap_or("").split(" · 3 models").next().unwrap_or("")) } else { upper_first(&r.line) },
        "badge": (r.id == "claude").then_some("Installed"),
        "removable": r.id != "claude",
        "has_key": r.has_key,
        "auto": r.auto,
        "local": r.id == "ollama" || r.base_url.as_deref().is_some_and(local),
        "models": models,
        "model_count": models.len(),
        "typed_models": r.typed,
        "models_error": (r.list_failed && r.state == State::Connected).then(|| format!("Could not load the list of models from {}. Check the address and key in Settings, then try again.", r.name)),
        "message": r.message,
        "checked_ms": (r.checked_ms > 0).then_some(r.checked_ms),
    })
}

fn upper_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// The "Add a provider" list.
pub fn addable() -> serde_json::Value {
    let s = read();
    let mut v = vec![serde_json::json!({ "id": "claude", "name": "Claude Code", "line": "Uses your login · already added", "needs_key": false, "needs_address": false, "added": true })];
    for k in KINDS {
        v.push(serde_json::json!({ "id": k.id, "name": k.name, "line": k.line, "needs_key": matches!(k.id, "openai" | "openrouter" | "groq"), "needs_address": k.id == "other", "added": s.providers.iter().any(|e| e.id == k.id) }));
    }
    serde_json::Value::Array(v)
}

/// `provider list` as the terminal prints it.
pub async fn list_text() -> String {
    let rows = rows().await;
    let w = rows.iter().map(|r| r.name.chars().count()).max().unwrap_or(0).max(12) + 2;
    let mut out = vec!["Providers".to_owned()];
    for r in &rows {
        let shown = if r.id == "other" { "Other".to_owned() } else { r.name.clone() };
        let tail = match &r.message {
            Some(m) if r.state != State::Connected => m.clone(),
            _ => r.line.clone(),
        };
        out.push(format!("  {shown:<w$}{:<18}{tail}", r.state.label()).trim_end().to_owned());
    }
    if rows.len() == 1 {
        out.push("Add another provider: fastpilot provider add <provider>".into());
    }
    out.join("\n")
}

fn unknown_provider(typed: &str) -> Fail {
    fail("provider_not_found", format!("No provider called {}. Providers: {TYPED}. Claude Code is always connected.", q(typed.trim())))
}

/// What `add` needs from the person before it runs: the address and whether a key is asked.
pub fn needs_key(id: &str, address: Option<&str>) -> Result<bool, Fail> {
    let id = canonical(id);
    let k = kind(&id).ok_or_else(|| unknown_provider(&id))?;
    Ok(match k.id {
        "openai" | "openrouter" | "groq" => true,
        "ollama" => false,
        _ => address.is_some_and(|a| !local(a)),
    })
}

/// A provider added or changed.
#[derive(Debug, Clone)]
pub struct Added {
    pub done: Done,
    pub lines: Vec<String>,
}

pub fn check_address(id: &str, address: Option<&str>) -> Result<Option<String>, Fail> {
    let Some(a) = address.map(str::trim).filter(|a| !a.is_empty()) else {
        return if id == "other" { Err(fail("bad_address", "“other” needs the server’s address, like https://llm.example.com/v1. Run: fastpilot provider add other <address>")) } else { Ok(None) };
    };
    let rest = ["https://", "http://"].iter().find_map(|s| a.strip_prefix(s));
    let h = rest.map(|r| r.split(['/', '?', '#']).next().unwrap_or(""));
    if h.is_none_or(str::is_empty) || a.contains(char::is_whitespace) || a.contains('@') {
        return Err(fail("bad_address", format!("{} is not a web address. Use a full address like https://llm.example.com/v1. Nothing was saved.", q(a))));
    }
    if a.starts_with("http://") && !local(a) {
        return Err(fail("not_https", "https is required, except to this computer. Use https://… Nothing was saved."));
    }
    let a = a.trim_end_matches('/').to_owned();
    match id {
        "other" => Ok(Some(a)),
        "ollama" if local(&a) => Ok(Some(a)),
        "ollama" => Err(fail("bad_address", format!("{} is not on this Mac. Ollama runs on this Mac: leave the address out. Nothing was saved.", q(&a)))),
        // A named provider talks to its own address only: its key never goes elsewhere.
        _ => {
            let own = kind(id).and_then(|k| k.base).unwrap_or_default();
            if fastpilot_core::config::origin(&a) == fastpilot_core::config::origin(own) {
                Ok(None)
            } else {
                Err(fail("bad_address", format!("{} uses its own address only. For another server, run: fastpilot provider add other {a}. Nothing was saved.", provider_name(id))))
            }
        }
    }
}

fn trouble_fail(id: &str, name: &str, h: &str, t: Trouble) -> Fail {
    match t {
        Trouble::NotRunning if id == "ollama" => fail("provider_not_running", "Ollama is not running on this Mac. Start Ollama, then run the command again. Nothing was saved."),
        Trouble::NotRunning | Trouble::NotAnswering => fail("provider_not_answering", format!("Could not reach {h}. Check your internet connection, then run the command again. Nothing was saved.")),
        Trouble::KeyNotAccepted => fail("key_not_accepted", format!("That key was not accepted by {h}. Check it and run the command again. Nothing was saved.")),
        Trouble::ListFailed => fail("list_failed", format!("Could not load the list of models from {name}. A model id you type still works.")),
    }
}

/// `fastpilot provider add <provider> [<address>]` and the dashboard's Add / Edit: check the
/// provider with the key (the key goes only to its own address), then keep it. `key`: typed by the
/// person (hidden) or pasted in the dashboard; never from a command line or a tool call.
pub async fn add(id: &str, address: Option<&str>, key: Option<String>, model_id: Option<&str>) -> Result<Added, Fail> {
    let id = canonical(id);
    if id == "claude" {
        return Ok(Added { done: Done { line: "Claude Code is already connected. Nothing changed.".into(), undo: None }, lines: vec![] });
    }
    let k = kind(&id).ok_or_else(|| unknown_provider(&id))?;
    let before = entry(&id);
    let address = check_address(&id, address)?;
    let base = match (&address, &before) {
        (Some(a), _) => a.clone(),
        (None, Some(e)) if id == "other" => base_of(e),
        (None, _) if id == "ollama" => ollama_base(),
        (None, _) => k.base.unwrap_or_default().to_owned(),
    };
    let h = host(&base).unwrap_or_default();
    let name = if id == "other" { h.clone() } else { k.name.to_owned() };
    let key = key.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty());
    let needs = needs_key(&id, Some(&base))?;
    let old_key = key_for(k, &base).ok().flatten();
    if let Some(key) = &key
        && crate::keystore::check_key(key).is_err()
    {
        return Err(fail("key_not_accepted", format!("That key was not accepted by {h}. Check it and run the command again. Nothing was saved.")));
    }
    let use_key = key.clone().or_else(|| old_key.clone());
    if needs && use_key.is_none() {
        return Err(fail("no_key", "No key was given. Run this in a terminal and type it when asked, or pipe it in. Nothing was saved."));
    }
    let fetched = fetch(&base, use_key.as_deref(), Duration::from_secs(15)).await;
    let (models, list_failed) = match fetched {
        Ok(m) => (m, false),
        Err(Trouble::ListFailed) => (vec![], true),
        Err(t) => return Err(trouble_fail(&id, &name, &h, t)),
    };
    let same_key = key.is_none() || key == old_key;
    let same_base = before.as_ref().is_none_or(|e| base_of(e) == base);
    if before.is_some() && same_key && same_base && model_id.is_none() {
        // Already connected: the list is fresh now.
        let _ = with_store(|s| {
            if let Some(e) = s.providers.iter_mut().find(|e| e.id == id) {
                e.models.clone_from(&models);
                e.list_failed = list_failed;
                e.state = State::Connected;
                e.checked_ms = now_ms();
            }
        });
        return Ok(Added { done: Done { line: format!("{name} is already connected. Nothing changed."), undo: None }, lines: vec![] });
    }
    if let Some(key) = &key {
        let origin = fastpilot_core::config::origin(&base).ok_or_else(|| fail("bad_address", format!("{} is not a web address. Use a full address like https://llm.example.com/v1. Nothing was saved.", q(&base))))?;
        crate::keystore::set_bound(k.account, key, &origin).map_err(|_| fail("keychain", "Could not save to the Keychain. Nothing was saved. Unlock the Keychain, then run the command again."))?;
    } else if !same_base && !needs {
        // A local server without a key: an old key for another address is not kept.
        if old_key.is_none() {
            crate::keystore::forget(k.account);
        }
    }
    let mut typed: Vec<String> = before.as_ref().map(|e| e.typed.clone()).unwrap_or_default();
    if let Some(m) = model_id.map(str::trim).filter(|m| !m.is_empty()) {
        if !plain_id(m) {
            return Err(fail("model_not_found", format!("{} is not a model id. Nothing was saved.", q(m))));
        }
        if !typed.iter().any(|t| t == m) {
            typed.push(m.to_owned());
        }
    }
    let e = Entry {
        id: id.clone(),
        base_url: address.clone().or_else(|| before.as_ref().and_then(|b| b.base_url.clone())).filter(|_| matches!(id.as_str(), "other" | "ollama")),
        models: models.clone(),
        typed,
        state: State::Connected,
        list_failed,
        checked_ms: now_ms(),
        added_ms: before.as_ref().map_or_else(now_ms, |b| b.added_ms),
    };
    with_store(|s| {
        s.providers.retain(|x| x.id != e.id);
        s.providers.push(e.clone());
        if e.id == "ollama" {
            s.ollama_off = false;
        }
    })?;
    let n = count(models.len() + e.typed.iter().filter(|t| !models.contains(t)).count());
    let mut lines = Vec::new();
    if list_failed {
        lines.push(format!("Could not load the list of models from {name}. A model id you type still works."));
    }
    if before.is_some() {
        let what = if key.is_some() { " · new key saved" } else { "" };
        let found = if list_failed { String::new() } else { format!(" · {n} found") };
        return Ok(Added { done: Done { line: format!("Changed {name}{what}{found}"), undo: None }, lines });
    }
    let line = if id == "ollama" {
        lines.push("Tests that use Ollama keep page text and screenshots on this Mac.".into());
        if models.is_empty() && !list_failed {
            lines = vec!["Download one in Ollama, then run: fastpilot provider refresh ollama".into()];
            "Added Ollama · found on this Mac · no models yet".to_owned()
        } else {
            format!("Added Ollama · found on this Mac · {n}")
        }
    } else {
        lines.push(format!("Tests that use {name} send page text and screenshots to {h}."));
        if list_failed { format!("Added {name}") } else { format!("Added {name} · {n} found") }
    };
    let undo = crate::undo::record(&format!("Added {name}"), &format!("provider:{id}"), crate::undo::Op::RemoveProvider { id: id.clone() });
    Ok(Added { done: Done { line, undo }, lines })
}

/// Undo of "Added …": the provider goes and its key is deleted.
pub fn undo_add(id: &str) -> Result<String, Fail> {
    let had_key = kind(id).is_some_and(|k| crate::keystore::get_stored(k.account).is_some());
    drop_provider(id)?;
    Ok(if had_key { " · its key was deleted".into() } else { String::new() })
}

/// Remove a provider: its key and entry go; agents that used it follow the default. Returns the
/// names of those agents.
fn drop_provider(id: &str) -> Result<Vec<String>, Fail> {
    let id = canonical(id);
    if let Some(k) = kind(&id) {
        crate::keystore::forget(k.account);
    }
    with_store(|s| {
        s.providers.retain(|e| e.id != id);
        if id == "ollama" {
            s.ollama_off = true;
        }
    })?;
    let mut names = Vec::new();
    for mut a in crate::agents::list() {
        let uses = |c: &Option<Choice>| c.as_ref().is_some_and(|c| c.provider == id);
        let (r, e) = (own_choice(&a, Role::Replay), own_choice(&a, Role::Explore));
        let legacy = a.provider.as_deref().map(canonical).is_some_and(|p| p == id);
        if uses(&r) || uses(&e) || legacy {
            a.replay = if uses(&r) { None } else { r };
            a.explore = if uses(&e) { None } else { e };
            a.models_set = true;
            if legacy {
                a.provider = None;
                a.model = None;
            }
            names.push(a.name.clone());
            crate::agents::update(a).map_err(|e| fail("failed", e))?;
        }
    }
    Ok(names)
}

/// `fastpilot provider remove <provider>` (no question, no Undo: its key is deleted).
pub fn remove(id: &str) -> Result<(String, Vec<String>, Vec<String>), Fail> {
    let id = canonical(id);
    if id == "claude" {
        return Err(fail("builtin", "Claude Code is built in and cannot be removed. Nothing changed."));
    }
    let k = kind(&id).ok_or_else(|| fail("provider_not_found", format!("No provider called {}. Providers: {TYPED}.", q(&id))))?;
    let auto_ollama = id == "ollama" && !read().ollama_off;
    if entry(&id).is_none() && !auto_ollama {
        return Err(fail("provider_not_connected", format!("{} is not connected. Nothing changed.", if id == "other" { "Other".to_owned() } else { k.name.to_owned() })));
    }
    let name = provider_name(&id);
    let had_key = crate::keystore::get_stored(k.account).is_some();
    let agents = drop_provider(&id)?;
    let line = if had_key { format!("Removed provider {name} · its key was deleted · this cannot be undone") } else { format!("Removed provider {name} · this cannot be undone") };
    let mut lines = Vec::new();
    match agents.as_slice() {
        [] => {}
        [one] => lines.push(format!("{one} used it and now follows the default models.")),
        many => lines.push(format!("{} and {} used it and now follow the default models.", many[..many.len() - 1].join(", "), many[many.len() - 1])),
    }
    lines.push(format!("To connect it again: fastpilot provider add {}", k.id));
    Ok((line, lines, agents))
}

/// `provider refresh [<provider>]`: each provider checked again, with its model count or why not.
pub async fn refresh(id: Option<&str>) -> Result<Vec<(String, Result<usize, Trouble>)>, Fail> {
    let ids: Vec<String> = match id {
        Some(i) => {
            let i = canonical(i);
            if i == "claude" {
                return Ok(vec![("claude".into(), Ok(3))]);
            }
            kind(&i).ok_or_else(|| unknown_provider(&i))?;
            if (i != "ollama" || read().ollama_off) && entry(&i).is_none() {
                return Err(fail("provider_not_connected", format!("{} is not connected. Add it first: fastpilot provider add {i}. Nothing changed.", provider_name(&i))));
            }
            vec![i]
        }
        None => {
            let mut v: Vec<String> = connected_ids().into_iter().filter(|p| p != "claude").collect();
            if !v.iter().any(|p| p == "ollama") && find_ollama(Duration::from_millis(500)).await.is_some() {
                v.push("ollama".into());
            }
            v
        }
    };
    let mut out = Vec::new();
    for i in ids {
        let r = if i == "ollama" && entry("ollama").is_none() {
            // Found by itself: checked, not kept.
            fetch(&ollama_base(), None, Duration::from_secs(5)).await.map(|m| m.len())
        } else {
            refresh_entry(&i).await.map(|e| e.models.len() + e.typed.iter().filter(|t| !e.models.contains(t)).count()).map_err(|(_, t)| t)
        };
        out.push((i, r));
    }
    Ok(out)
}

/// One refresh result as the terminal (`✓`/`✗`, the bool) or Claude Code says it (T8.3, T8.6).
pub fn refresh_line(id: &str, r: &Result<usize, Trouble>, s: Surface) -> (bool, &'static str, String) {
    let name = provider_name(id);
    let h = if id == "ollama" { "this Mac".to_owned() } else { host(&base_of_id(id).unwrap_or_default()).unwrap_or_else(|| name.clone()) };
    let cli = s == Surface::Cli;
    match r {
        Ok(0) if id == "ollama" => (true, "", "Refreshed Ollama · no models yet. Download one in Ollama, then run this again.".to_owned()),
        Ok(n) => (true, "", format!("Refreshed {name} · {}", count(*n))),
        Err(Trouble::NotRunning) if id == "ollama" => (
            false,
            "provider_not_running",
            if cli { "Ollama is not running on this Mac. Start Ollama, then run: fastpilot provider refresh ollama".into() } else { "Ollama is not running on this Mac. Tell the user to start Ollama, then use refresh.".into() },
        ),
        Err(Trouble::KeyNotAccepted) => (
            false,
            "key_not_accepted",
            if cli { format!("{h} did not accept the key. Run: fastpilot provider add {id} to save a new one.") } else { format!("{h} did not accept the key. Tell the user to save a new one: fastpilot provider add {id}.") },
        ),
        Err(Trouble::NotRunning | Trouble::NotAnswering) => (
            false,
            "provider_not_answering",
            if cli { format!("{h} did not answer. Check your internet connection, then run: fastpilot provider refresh {id}") } else { format!("{h} did not answer. Check the internet connection, then use refresh.") },
        ),
        Err(Trouble::ListFailed) => (
            false,
            "list_failed",
            if cli { format!("Could not load the list of models from {name}. Check the address and key, then run: fastpilot provider refresh {id}. A model id you type still works.") } else { format!("Could not load the list of models from {name}. A model id the user types still works.") },
        ),
    }
}

/// `provider models [<provider>] [--all]`.
pub async fn models_text(id: Option<&str>, all: bool) -> Result<String, Fail> {
    let rows = rows().await;
    let pick: Vec<&Row> = match id {
        Some(i) => {
            let i = canonical(i);
            if i != "claude" {
                kind(&i).ok_or_else(|| unknown_provider(&i))?;
            }
            let r: Vec<&Row> = rows.iter().filter(|r| r.id == i).collect();
            if r.is_empty() {
                return Err(fail("provider_not_connected", format!("{} is not connected. Add it first: fastpilot provider add {i}. Nothing changed.", provider_name(&i))));
            }
            r
        }
        None => rows.iter().collect(),
    };
    let mut out = Vec::new();
    for r in pick {
        if r.state != State::Connected {
            out.push(format!("{}: {}. {}", r.name, r.state.label(), r.message.clone().unwrap_or_default().split(". ").last().unwrap_or("")));
            continue;
        }
        let ids: Vec<String> = r.models.iter().chain(r.typed.iter().filter(|t| !r.models.contains(t))).map(|m| if r.id == "claude" { claude_name(m) } else { m.clone() }).collect();
        let head = match r.id.as_str() {
            "claude" => "Claude Code · uses your login".to_owned(),
            "ollama" => "Ollama · this Mac".to_owned(),
            _ => r.name.clone(),
        };
        if r.id == "ollama" && ids.is_empty() && !r.list_failed {
            out.push("Ollama · this Mac: no models yet. Download one in Ollama, then run fastpilot provider refresh ollama.".into());
            continue;
        }
        if r.list_failed && ids.is_empty() {
            out.push(format!("{head}: could not load the list of models. A model id you type still works."));
            continue;
        }
        let limit = if all { MAX_MODELS } else { 20 };
        if ids.len() > limit {
            out.push(format!("{head} · {} (first {limit} shown; fastpilot provider models {} --all shows the rest)", count(ids.len()), r.id));
            out.push(format!("  {}, …", ids[..limit].join(", ")));
        } else {
            out.push(if r.id == "claude" || r.id == "ollama" { head } else { format!("{head} · {}", count(ids.len())) });
            out.push(format!("  {}", ids.join(", ")));
        }
    }
    Ok(out.join("\n"))
}

// ---- a run that cannot use its model (T8.4) --------------------------------------------------

/// The command that chooses another model, for the stop line.
pub fn choose_other(agent: Option<&crate::agents::Agent>, role: Role, own: bool) -> String {
    match agent {
        Some(a) if own => format!("fastpilot agent model \"{}\" --{} default", a.name, role.key()),
        _ => format!("fastpilot agent model --default --{} default", role.key()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_inputs() {
        assert_eq!(claude_name("claude-sonnet-5-5"), "Sonnet 5.5");
        assert_eq!(claude_name("gpt-4o"), "gpt-4o");
        assert_eq!(claude_alias("haiku").as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(claude_alias("Sonnet 5.5").as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(claude_alias("claude-opus-5-5").as_deref(), Some("claude-opus-5-5"));
        assert_eq!(claude_alias("opus 4.1"), None);
        assert_eq!(claude_alias("deepseek/x"), None);
        assert_eq!(parse_stored("openrouter:deepseek/x"), Some(Choice { provider: "openrouter".into(), model: "deepseek/x".into() }));
        assert_eq!(parse_stored("ollama:llama3.1:8b"), Some(Choice { provider: "ollama".into(), model: "llama3.1:8b".into() }));
        assert_eq!(parse_stored("custom:m"), Some(Choice { provider: "other".into(), model: "m".into() }));
        assert_eq!(parse_stored("evil:m"), None);
        assert!(plain_id("deepseek/deepseek-v4-flash") && !plain_id("a b") && !plain_id("x\u{202e}y") && !plain_id(""));
    }

    /// A fake `/v1/models` that logs the Authorization header of each request.
    fn fake_models(ids: &'static [&'static str]) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://127.0.0.1:{}/v1", l.local_addr().expect("addr").port());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut r = BufReader::new(s.try_clone().expect("clone"));
                let mut auth = String::from("none");
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                        break;
                    }
                    if h.to_ascii_lowercase().starts_with("authorization:") {
                        auth = h.trim().to_owned();
                    }
                }
                seen2.lock().expect("seen").push(auth);
                let body = serde_json::json!({ "data": ids.iter().map(|i| serde_json::json!({ "id": i })).collect::<Vec<_>>() }).to_string();
                let mut s = s;
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (base, seen)
    }

    /// E8 security: a provider's key is sent only to the address it was saved for. Moving "other"
    /// to another server does not take the key along: the new server gets no key, and a run is
    /// refused rather than sending it.
    #[test]
    fn a_key_goes_only_to_its_own_providers_address() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime").block_on(key_goes_only_to_its_own_address());
    }

    async fn key_goes_only_to_its_own_address() {
        let dir = std::env::temp_dir().join(format!("fdd-device-e8-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let old_home = std::env::var_os("HOME");
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("HOME", &dir);
            std::env::set_var("FASTPILOT_KEYSTORE", "file");
            std::env::set_var("FASTPILOT_CONFIG", dir.join(".fastpilot/config.toml"));
            std::env::remove_var("FASTPILOT_PROVIDER_API_KEY");
            std::env::remove_var("FASTPILOT_PROVIDER");
        }
        let (one, seen_one) = fake_models(&["m1", "m2"]);
        let (two, seen_two) = fake_models(&["m3"]);
        let added = add("other", Some(&one), Some("sk-e8-only-one".into()), None).await.expect("add");
        assert!(added.done.line.ends_with("· 2 models found") && added.done.undo.is_some(), "{added:?}");
        assert_eq!(seen_one.lock().expect("seen").as_slice(), ["authorization: Bearer sk-e8-only-one"]);
        assert_eq!(crate::keystore::get_stored("custom").map(|(_, o)| o), Some(fastpilot_core::config::origin(&one)));
        let c = Choice { provider: "other".into(), model: "m1".into() };
        let p = crate::provider::Provider::for_choice(&c, "x").expect("own address");
        assert_eq!(p.base_url, one);
        // Errors and lists never carry the key.
        let rows = rows().await;
        assert!(!format!("{rows:?}").contains("sk-e8") && !rows.iter().any(|r| row_json(r).to_string().contains("sk-e8")));
        // A key saved for another address (an older store, or a changed address) is never sent.
        crate::keystore::set_bound("custom", "sk-e8-elsewhere", "https://elsewhere.example").expect("store");
        let e = crate::provider::Provider::for_choice(&c, "x").expect_err("key saved for another address");
        assert!(e.contains("did not accept the key") && !e.contains("sk-e8"), "{e}");
        crate::keystore::set_bound("custom", "sk-e8-only-one", &fastpilot_core::config::origin(&one).expect("origin")).expect("store");
        // The same provider at another address, without typing the key again: the key stays behind
        // (T8.3: the old key is not kept), and the new server gets none.
        let changed = add("other", Some(&two), None, None).await.expect("change");
        assert!(changed.done.line.starts_with("Changed 127.0.0.1:") && changed.done.undo.is_none(), "{changed:?}");
        assert_eq!(seen_two.lock().expect("seen").as_slice(), ["none"], "the new server got no key");
        let p = crate::provider::Provider::for_choice(&c, "x").expect("no key, local server");
        assert!(p.base_url == two && !format!("{p:?}").contains("sk-e8") && format!("{p:?}").contains("key: None"), "{p:?}");
        assert!(crate::keystore::get_stored("custom").is_none(), "the old key was not kept");
        add("other", Some(&one), Some("sk-e8-only-one".into()), None).await.expect("again");
        // Undo of "Added" deletes the provider and its key.
        assert_eq!(undo_add("other").expect("undo"), " · its key was deleted");
        assert!(crate::keystore::get_stored("custom").is_none() && entry("other").is_none());
        unsafe {
            match old_home {
                Some(h) => std::env::set_var("HOME", h),
                None => std::env::remove_var("HOME"),
            }
            std::env::remove_var("FASTPILOT_KEYSTORE");
            std::env::remove_var("FASTPILOT_CONFIG");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// E6: at most 500 plain ids; hidden characters and duplicates are left out.
    #[tokio::test]
    async fn a_model_list_is_capped_and_plain() {
        use std::io::{BufRead, BufReader, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://127.0.0.1:{}/v1", l.local_addr().expect("addr").port());
        std::thread::spawn(move || {
            if let Ok((s, _)) = l.accept() {
                let mut r = BufReader::new(s.try_clone().expect("clone"));
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                        break;
                    }
                }
                let mut ids: Vec<serde_json::Value> = (0..700).map(|i| serde_json::json!({ "id": format!("m{i}") })).collect();
                ids.insert(0, serde_json::json!({ "id": "evil\u{202e}txt" }));
                ids.insert(1, serde_json::json!({ "id": "m0" }));
                let body = serde_json::json!({ "data": ids }).to_string();
                let mut s = s;
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        let ids = fetch(&base, None, Duration::from_secs(5)).await.expect("list");
        assert_eq!(ids.len(), MAX_MODELS);
        assert_eq!(ids[0], "m0");
        assert!(ids.iter().all(|i| plain_id(i)));
        let gone = format!("http://127.0.0.1:{}/v1", std::net::TcpListener::bind("127.0.0.1:0").expect("b").local_addr().expect("a").port());
        assert_eq!(fetch(&gone, None, Duration::from_secs(2)).await, Err(Trouble::NotRunning));
    }
}
