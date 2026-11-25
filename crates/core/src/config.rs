//! Settings: environment variables, then `~/.fastpilot/config.toml` (or `FASTPILOT_CONFIG`),
//! then built-in defaults. Every `FASTPILOT_*` setting is read through [`var`], so a value written
//! to the file (by `fastpilot config set` or the `config` tool) applies to the next read, without a
//! restart. Guard rails ([`Rule::TightenOnly`]) are the exception: a running process only follows a
//! file change that makes them stricter; a looser value (or a file that no longer parses) applies
//! from the next start. Keys in the file are the variable names without the prefix, lowercase:
//!
//! ```toml
//! allowed_domains = ["shop.example.com", "*.staging.example.com"]
//! confirm_actions = "default"
//! headful = true
//! model_replay = "claude-haiku-5-5"
//! ```
//!
//! Secrets never come from the file (`FASTPILOT_STATE_KEY`): set them in the environment.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

/// A documented setting: file key, what it does, and who may change it through the `config` tool.
pub struct Setting {
    pub key: &'static str,
    pub help: &'static str,
    pub rule: Rule,
}

/// What the `config` MCP tool (an AI agent) may do with a setting. People change anything with
/// `fastpilot config set` or by editing the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Any value.
    Free,
    /// Only stricter: a guard rail an agent must not loosen on itself.
    TightenOnly,
    /// Not from the tool: a program path or a location (an agent could point it anywhere).
    HumanOnly,
}

pub const SETTINGS: &[Setting] = &[
    Setting { key: "allowed_domains", help: "web sessions only load pages from these hosts (`*.example.com` for subdomains); empty = any", rule: Rule::TightenOnly },
    Setting { key: "confirm_actions", help: "taps on elements named like these need `confirm: true`; not set: hapus, delete, remove (built-in); `default` = delete, pay, buy, transfer, ...; [] = none", rule: Rule::TightenOnly },
    Setting { key: "vars", help: "environment variables flows may read as ${NAME}; their values are masked as ***", rule: Rule::TightenOnly },
    Setting { key: "state_expire_days", help: "saved login state is refused after this many days (default 30)", rule: Rule::TightenOnly },
    Setting { key: "block_actions", help: "taps on elements whose text contains one of these words are always refused, even with `confirm: true` (case-insensitive); not set: the built-in list (bayar, pay, beli, buy, checkout, purchase, transfer, buat pesanan, place order, hapus akun, delete account; whole words), also `default` in the list; [] = none", rule: Rule::TightenOnly },
    Setting { key: "allow_app_install", help: "installApp / uninstallApp steps may run (default true)", rule: Rule::TightenOnly },
    Setting { key: "allow_app_reset", help: "clearState / clearKeychain may wipe app data (default true)", rule: Rule::TightenOnly },
    Setting { key: "allow_physical_devices", help: "sessions may use real phones; false = emulators and simulators only (default true)", rule: Rule::TightenOnly },
    Setting { key: "allow_device_settings", help: "android/ios steps may change device settings: setNetwork, setLocation, setPermissions, setLocale (default true)", rule: Rule::TightenOnly },
    Setting { key: "allow_eval_script", help: "evalScript steps may run JavaScript in pages; not set: off while block_actions is on (a script could press a blocked button, or read a password typed into the page), on otherwise; `true` = on even then (people only)", rule: Rule::TightenOnly },
    Setting { key: "allow_takeover", help: "the user may take sessions over in `fastpilot ui` (Take over); off for agents with a shell (default true)", rule: Rule::TightenOnly },
    Setting { key: "upload_dirs", help: "uploadFile only takes files inside these folders; empty = any", rule: Rule::TightenOnly },
    Setting { key: "max_steps", help: "`fastpilot run`: tool calls the AI may make per test (default 30)", rule: Rule::TightenOnly },
    Setting { key: "test_timeout", help: "`fastpilot run`: minutes one test may take before it is stopped and failed (default: 30)", rule: Rule::TightenOnly },
    Setting { key: "headful", help: "show the browser window (default false: headless, never in the way)", rule: Rule::Free },
    Setting { key: "model", help: "`fastpilot run`: pin one Claude model for every test", rule: Rule::Free },
    Setting { key: "model_replay", help: "`fastpilot run`: model that replays path memory (default claude-haiku-5-5)", rule: Rule::Free },
    Setting { key: "model_explore", help: "`fastpilot run`: model that explores (default claude-sonnet-5-5)", rule: Rule::Free },
    Setting { key: "flow_timeout", help: "seconds one run_flow may take (default 300)", rule: Rule::Free },
    Setting { key: "action_timeout", help: "seconds one action (tap, type, select, scroll) may take (default 5); typing on a phone gets three times that for the keyboard", rule: Rule::Free },
    Setting { key: "retry_budget", help: "`fastpilot run`: reruns of failed tests per suite (default 3)", rule: Rule::Free },
    Setting { key: "breaker", help: "`fastpilot run`: consecutive infra failures before the rest is skipped (default 3)", rule: Rule::Free },
    Setting { key: "android_serial", help: "default Android device or emulator serial", rule: Rule::Free },
    Setting { key: "ios_device", help: "simulator booted when none runs (default iPhone 16)", rule: Rule::Free },
    Setting { key: "ios_udid", help: "default iOS simulator or device UDID", rule: Rule::Free },
    Setting { key: "device_autostart", help: "start a stopped Android emulator or iPhone simulator when a test needs it: true or false; not set: asked once (Device page, or `fastpilot run` in Terminal)", rule: Rule::HumanOnly },
    Setting { key: "android_avd", help: "the installed Android emulator started when a test needs one (default: fastpilot's test phone, else the first installed)", rule: Rule::HumanOnly },
    Setting { key: "memory", help: "path memory: rw (default), readonly, off", rule: Rule::Free },
    Setting { key: "max_parallel", help: "tests that may run at once on this computer, for all agents together (1-20, default 3); one device always runs one test", rule: Rule::Free },
    Setting { key: "daemon_idle", help: "shell CLI daemon exits after this many idle seconds (default 1800)", rule: Rule::Free },
    Setting { key: "chrome", help: "Chrome/Chromium executable", rule: Rule::HumanOnly },
    Setting { key: "claude", help: "`fastpilot run`: the claude CLI to drive", rule: Rule::HumanOnly },
    Setting { key: "provider", help: "`fastpilot run`: who drives the tests: claude (Claude Code, default), or an OpenAI-compatible API: openai, openrouter, groq, ollama, custom (key: `fastpilot config set-key <provider>`)", rule: Rule::HumanOnly },
    Setting { key: "provider_base_url", help: "OpenAI-compatible API base URL (default per provider; required for custom), e.g. https://api.openai.com/v1", rule: Rule::HumanOnly },
    Setting { key: "provider_model", help: "model id at the OpenAI-compatible provider (required when provider is not claude)", rule: Rule::Free },
    Setting { key: "dir", help: "where runs, memory, states and baselines go (default ./.fastpilot)", rule: Rule::HumanOnly },
    Setting { key: "ios_runner", help: "iOS runner build folder (default ios-runner/build)", rule: Rule::HumanOnly },
    Setting { key: "adb_server", help: "adb server for Android devices on another machine, host:port (e.g. 127.0.0.1:15037 through an SSH tunnel)", rule: Rule::HumanOnly },
    Setting { key: "chrome_endpoint", help: "remote Chrome for web sessions: http://host:port or ws://... (through an SSH tunnel; never expose the port)", rule: Rule::HumanOnly },
];

/// Never read from the file.
const SECRETS: &[&str] = &["FASTPILOT_STATE_KEY", "FASTPILOT_TOKEN", "FASTPILOT_RELAY_TOKEN", "FASTPILOT_PROVIDER_API_KEY"];

/// The preset base URL of `provider`, if it has one.
pub fn preset_base(provider: &str) -> Option<&'static str> {
    PROVIDERS.iter().find(|(p, _)| *p == provider).and_then(|(_, b)| *b)
}

/// `scheme://host[:port]` of a URL, lowercase: where a request (and a key) goes.
pub fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    (!host_port.is_empty()).then(|| format!("{}://{}", scheme.to_ascii_lowercase(), host_port.to_ascii_lowercase()))
}

/// `provider` values and their default base URL (`None`: Claude Code, or set provider_base_url).
pub const PROVIDERS: &[(&str, Option<&str>)] = &[
    ("claude", None),
    ("openai", Some("https://api.openai.com/v1")),
    ("openrouter", Some("https://openrouter.ai/api/v1")),
    ("groq", Some("https://api.groq.com/openai/v1")),
    ("ollama", Some("http://127.0.0.1:11434/v1")),
    ("custom", None),
];

/// The project's fastpilot folder: `FASTPILOT_DIR`, else `.fastpilot` in the current folder.
pub fn project_dir() -> PathBuf {
    var_os("FASTPILOT_DIR").map(PathBuf::from).unwrap_or_else(|| ".fastpilot".into())
}

/// Security (Low 9): chat transcripts, runs and saved logins live in the project's `.fastpilot`,
/// and git does not keep their 0600 permissions. Once that folder exists, it gets a `.gitignore`
/// with `*`, never over a file or a link someone put there, and only in a folder named
/// `.fastpilot` (a custom `FASTPILOT_DIR` may be a folder the user keeps in git on purpose).
pub fn keep_out_of_git() {
    ignore_in(&project_dir());
}

/// Everything is private by default (new kinds of files too); visual baselines are made to be
/// shared, so they stay, without a run's actual and diff images (Security re-check F).
const IGNORE: &str = "# fastpilot: test runs, chats and saved logins of this computer; not for git.\n*\n!.gitignore\n# Visual baselines are shared with the team (not this computer's actual and diff images).\n!baselines/\n!baselines/*.png\nbaselines/*.actual.png\nbaselines/*.diff.png\n";

fn ignore_in(d: &std::path::Path) {
    // A real folder, not a link a cloned repo planted (Security re-check F).
    if d.file_name().is_none_or(|n| n != ".fastpilot") || !d.symlink_metadata().is_ok_and(|m| m.is_dir()) {
        return;
    }
    let p = d.join(".gitignore");
    if p.symlink_metadata().is_ok() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
        use std::io::Write;
        let _ = f.write_all(IGNORE.as_bytes());
    }
}

pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("FASTPILOT_CONFIG").filter(|p| !p.is_empty()) {
        return p.into();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join(".fastpilot").join("config.toml")
}

type Cache = Option<(PathBuf, Option<SystemTime>, BTreeMap<String, String>)>;
static CACHE: Mutex<Cache> = Mutex::new(None);

fn render(v: &toml::Value) -> Option<String> {
    Some(match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => (if *b { "1" } else { "0" }).to_owned(),
        toml::Value::Array(a) => a.iter().filter_map(render).collect::<Vec<_>>().join(","),
        _ => return None,
    })
}

/// The file is there but does not parse: commands refuse to start (see [`check`]) rather than run
/// with the guard rails silently off.
pub fn check() -> Result<(), String> {
    let p = path();
    let Ok(text) = std::fs::read_to_string(&p) else { return Ok(()) };
    let table = text.parse::<toml::Table>().map_err(|e| {
        format!("{} is not valid TOML, so its settings (guard rails included) would be ignored: fix or delete it\n{e}", p.display())
    })?;
    // Values fastpilot would misread (a hand-edited `flow_timeout = "x"`, `allowed_domains = 5`)
    // stop it just the same, with the key and the reason.
    for (k, v) in &table {
        let Some(s) = setting(k) else { continue };
        let why = if LISTS.contains(&s.key) {
            match v {
                toml::Value::Array(items) if items.iter().all(toml::Value::is_str) => {
                    let joined: Vec<&str> = items.iter().filter_map(toml::Value::as_str).collect();
                    invalid(s.key, &joined.join(","))
                }
                toml::Value::String(text) => invalid(s.key, text),
                _ => Some(format!("`{k}` takes a list of text, e.g. {k} = [\"a\", \"b\"]")),
            }
        } else if FLAGS.contains(&s.key) {
            // A quoted "yes" / "true" would be read as text, not as on: only a TOML boolean.
            (!v.is_bool()).then(|| format!("`{k}` takes true or false, without quotes (`{k} = true`)"))
        } else {
            match render(v) {
                Some(text) if matches!(v, toml::Value::Array(_)) => Some(format!("`{k}` takes one value, not a list (`{text}`)")),
                Some(text) => invalid(s.key, &text),
                None => Some(format!("`{k}` has a value fastpilot cannot read")),
            }
        };
        if let Some(why) = why {
            return Err(format!("{}: {why}; fix or delete it", p.display()));
        }
    }
    Ok(())
}

/// The file's values (key → value as an env string), re-read when the file changes. A file that
/// does not parse counts as empty, with a warning.
pub fn file_values() -> BTreeMap<String, String> {
    let p = path();
    let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cp, cm, values)) = cache.as_ref()
        && *cp == p
        && *cm == mtime
    {
        return values.clone();
    }
    let values: BTreeMap<String, String> = match std::fs::read_to_string(&p) {
        Ok(text) => match text.parse::<toml::Table>() {
            Ok(t) => t.iter().filter_map(|(k, v)| render(v).map(|s| (k.to_lowercase(), s))).collect(),
            Err(e) => {
                tracing::warn!(path = %p.display(), error = %e, "config file ignored: not valid TOML");
                BTreeMap::new()
            }
        },
        Err(_) => BTreeMap::new(),
    };
    *cache = Some((p, mtime, values.clone()));
    values
}

fn key_of(name: &str) -> Option<String> {
    name.strip_prefix("FASTPILOT_").map(str::to_lowercase)
}

/// Guard-rail values in force in this process, per file: the first read pins the value, and later
/// file changes only replace it when they are stricter ([`tightens`]). A shell, an edited file or
/// `fastpilot config set` can loosen them for the next start, never under a running session.
static GUARD: Mutex<BTreeMap<(PathBuf, String), Option<String>>> = Mutex::new(BTreeMap::new());

fn from_file(key: &str) -> Option<String> {
    let value = file_values().remove(key);
    let Some(rule) = setting(key).map(|s| s.rule).filter(|r| *r != Rule::Free) else {
        return value;
    };
    let mut guard = GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let pinned = guard.entry((path(), key.to_owned())).or_insert_with(|| value.clone());
    if *pinned != value {
        // Program paths and endpoints (`HumanOnly`): what the process started with, until it restarts;
        // an edited file must not send the next session (and the secrets typed in it) elsewhere.
        if rule == Rule::HumanOnly {
            tracing::warn!(key, "config file changes a program path or endpoint: ignored until fastpilot restarts");
        } else if tightens(key, pinned.as_deref(), value.as_deref()) {
            *pinned = value;
        } else {
            tracing::warn!(key, "config file loosens a guard rail: ignored until fastpilot restarts");
        }
    }
    pinned.clone()
}

/// Pin every guard rail now. Long-running processes (`fastpilot mcp`, the shell daemon, `fastpilot
/// run`) call this at start, so the file as it was then is the loosest they ever apply: loosening
/// it before a rail's first use (an agent with a shell, between reading a page and the first risky
/// tap) does not count either.
pub fn pin_guard_rails() {
    for s in SETTINGS.iter().filter(|s| s.rule != Rule::Free) {
        let _ = from_file(s.key);
    }
}

/// Like `std::env::var` for a `FASTPILOT_*` name, falling back to the config file.
pub fn var(name: &str) -> Result<String, std::env::VarError> {
    match std::env::var(name) {
        Ok(v) => Ok(v),
        Err(std::env::VarError::NotPresent) if !SECRETS.contains(&name) => key_of(name)
            .and_then(|k| from_file(&k))
            .ok_or(std::env::VarError::NotPresent),
        Err(e) => Err(e),
    }
}

/// Like [`var`], but an empty environment variable counts as not set: the built-in guard-rail lists
/// are only emptied in the config file, by the user (Security Low 3: an empty
/// `FASTPILOT_BLOCK_ACTIONS` in an MCP server entry an agent wrote must not switch them off).
pub fn var_unless_empty_env(name: &str) -> Result<String, std::env::VarError> {
    match std::env::var(name) {
        // No item in it (empty, spaces, or only commas like ","): not set, so it cannot empty a
        // built-in list either (Security Low 1).
        Ok(v) if v.split(',').any(|item| !item.trim().is_empty()) => Ok(v),
        Ok(_) | Err(std::env::VarError::NotPresent) => key_of(name).and_then(|k| from_file(&k)).ok_or(std::env::VarError::NotPresent),
        Err(e) => Err(e),
    }
}

/// Like `std::env::var_os`, falling back to the config file.
pub fn var_os(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).or_else(|| var(name).ok().map(Into::into))
}

/// Where a setting's current value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Env(String),
    File(String),
    Default,
}

pub fn effective(key: &str) -> Source {
    let name = format!("FASTPILOT_{}", key.to_uppercase());
    match std::env::var(&name) {
        Ok(v) => Source::Env(v),
        Err(_) => match from_file(key) {
            Some(v) => Source::File(v),
            None => Source::Default,
        },
    }
}

/// Like [`effective`], but the file as it is now: what the next run will apply, also for guard
/// rails this process has pinned (`fastpilot ui` shows and edits these).
pub fn current(key: &str) -> Source {
    let name = format!("FASTPILOT_{}", key.to_uppercase());
    match std::env::var(&name) {
        Ok(v) => Source::Env(v),
        Err(_) => match file_values().remove(key) {
            Some(v) => Source::File(v),
            None => Source::Default,
        },
    }
}

pub fn setting(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key)
}

/// List items; hosts and words compare lowercase, variable names (`vars`) and folders keep their case.
fn items_of(key: &str, v: &str) -> Vec<String> {
    let fold = |s: &str| if matches!(key, "vars" | "upload_dirs") { s.to_owned() } else { s.to_lowercase() };
    v.split(',').map(|s| fold(s.trim())).filter(|s| !s.is_empty()).collect()
}

/// Settings whose value is a list.
pub const LISTS: &[&str] = &["allowed_domains", "confirm_actions", "vars", "block_actions", "upload_dirs"];

/// Settings whose value is a whole number.
const NUMBERS: &[&str] = &["state_expire_days", "flow_timeout", "action_timeout", "retry_budget", "breaker", "daemon_idle", "max_steps", "test_timeout", "max_parallel"];

/// Settings whose value is true / false.
pub const FLAGS: &[&str] =
    &["headful", "allow_app_install", "allow_app_reset", "allow_physical_devices", "allow_device_settings", "allow_eval_script", "allow_takeover"];

/// A true/false setting (`allow_*` default to true, the others to false).
pub fn flag(key: &str) -> bool {
    let name = format!("FASTPILOT_{}", key.to_uppercase());
    match var(&name).ok().map(|v| v.trim().to_lowercase()) {
        Some(v) if matches!(v.as_str(), "1" | "true" | "yes" | "on") => true,
        Some(v) if matches!(v.as_str(), "0" | "false" | "no" | "off") => false,
        _ => key.starts_with("allow_"),
    }
}

const BOOLS: &[&str] = &["true", "false", "on", "off", "yes", "no", "1", "0"];
const MEMORY_MODES: &[&str] = &["rw", "readonly", "off"];

/// Why `value` is not valid for `key`, if it is not.
pub fn invalid(key: &str, value: &str) -> Option<String> {
    let v = value.trim();
    if key == "block_actions" {
        for w in v.split(',').map(str::trim).filter_map(|w| w.strip_prefix('!')) {
            if !crate::policy::is_builtin_blocked(w) {
                return Some(format!("`block_actions`: `!{w}` allows a built-in word again, and `{w}` is not one of them"));
            }
        }
    }
    if NUMBERS.contains(&key) && v.parse::<u64>().is_err() {
        let unit = match key {
            "retry_budget" | "breaker" => "runs",
            "max_parallel" => "tests",
            "max_steps" => "steps",
            "test_timeout" => "minutes",
            "state_expire_days" => "days",
            _ => "seconds",
        };
        return Some(format!("`{key}` takes a whole number of {unit}, not `{value}`"));
    }
    if FLAGS.contains(&key) && !BOOLS.contains(&v.to_lowercase().as_str()) {
        return Some(format!("`{key}` takes true or false, not `{value}`"));
    }
    if key == "device_autostart" && !matches!(v.to_lowercase().as_str(), "true" | "false") {
        return Some(format!("`device_autostart` takes true or false, not `{value}`"));
    }
    if key == "headful" && !matches!(v.to_lowercase().as_str(), "true" | "false") {
        return Some(format!("`headful` takes true or false, not `{value}`"));
    }
    if key == "vars" {
        for name in v.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !valid {
                return Some(format!("`vars`: `{name}` is not an environment variable name (letters, digits and _, not starting with a digit)"));
            }
            if name.to_ascii_uppercase().starts_with("FASTPILOT_") {
                return Some(format!("`vars`: `{name}` is fastpilot's own setting or secret; flows may not read FASTPILOT_* variables"));
            }
        }
    }
    if key == "allowed_domains" {
        for host in v.split(',').map(str::trim).filter(|h| !h.is_empty()) {
            let bare = host.strip_prefix("*.").unwrap_or(host);
            let valid = !bare.is_empty()
                && !bare.starts_with('.')
                && !bare.ends_with('.')
                && !bare.contains("..")
                && bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
            if !valid {
                return Some(format!(
                    "`allowed_domains`: `{host}` is not a host; write only the host (example.com, *.example.com), without https://, a path or a port"
                ));
            }
        }
    }
    if key == "memory" && !MEMORY_MODES.contains(&v) {
        return Some(format!("`memory` takes rw, readonly or off, not `{value}`"));
    }
    if key == "max_parallel" && !v.parse::<usize>().is_ok_and(|n| (1..=crate::claim::MAX_PARALLEL).contains(&n)) {
        return Some(format!("`max_parallel` takes 1 to {} tests, not `{value}`", crate::claim::MAX_PARALLEL));
    }
    if key == "max_steps" && v == "0" {
        return Some("`max_steps` takes at least 1 step".into());
    }
    if key == "action_timeout" && v == "0" {
        return Some("`action_timeout` takes at least 1 second".into());
    }
    if key == "adb_server" {
        let ok = v.rsplit_once(':').is_some_and(|(h, p)| !h.is_empty() && !h.contains('/') && p.parse::<u16>().is_ok_and(|p| p > 0));
        if !ok {
            return Some(format!("`adb_server` takes host:port, e.g. 127.0.0.1:15037, not `{value}`"));
        }
    }
    if key == "chrome_endpoint" {
        let host = ["http://", "https://", "ws://", "wss://"].iter().find_map(|s| v.strip_prefix(s)).map(|r| r.split('/').next().unwrap_or(""));
        if host.is_none_or(str::is_empty) {
            return Some(format!("`chrome_endpoint` takes http://host:port or ws://host:port/devtools/browser/..., not `{value}`"));
        }
    }
    if key == "provider" && !PROVIDERS.iter().any(|(p, _)| *p == v) {
        let names: Vec<&str> = PROVIDERS.iter().map(|(p, _)| *p).collect();
        return Some(format!("`provider` takes one of {}, not `{value}`", names.join(", ")));
    }
    if key == "provider_base_url" {
        let rest = ["https://", "http://"].iter().find_map(|s| v.strip_prefix(s));
        let host = rest.map(|r| r.split(['/', '?', '#']).next().unwrap_or(""));
        if host.is_none_or(str::is_empty) || v.contains(char::is_whitespace) || v.contains('@') {
            return Some(format!("`provider_base_url` takes an http(s) URL without credentials, e.g. https://api.openai.com/v1, not `{value}`"));
        }
        // Prompts carry screen contents: plain http only to this machine (e.g. Ollama).
        if v.starts_with("http://") && crate::policy::remote_host(v).is_some() {
            return Some(format!("`provider_base_url`: plain http to another machine would send test screens unencrypted; use https, not `{value}`"));
        }
    }
    if key == "provider_model" && (v.is_empty() || v.contains(char::is_whitespace)) {
        return Some(format!("`provider_model` takes a model id, not `{value}`"));
    }
    let claude_id = |v: &str| v.starts_with("claude-") && v.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.');
    // E8 (v0.7.2): a default model may be at another provider: `<provider>:<model id>`.
    let at_provider = |v: &str| {
        v.split_once(':').is_some_and(|(p, m)| {
            matches!(p, "openai" | "openrouter" | "groq" | "ollama" | "other" | "custom") && (1..=200).contains(&m.chars().count()) && !m.chars().any(|c| c.is_whitespace() || c.is_control())
        })
    };
    if matches!(key, "model_replay" | "model_explore") && !claude_id(v) && !at_provider(v) {
        return Some(format!("`{key}` takes a Claude model id such as claude-haiku-5-5, or <provider>:<model id> such as openrouter:deepseek/deepseek-v4-flash, not `{value}`"));
    }
    if key == "model" && !claude_id(v) {
        return Some(format!("`{key}` takes a Claude model id such as claude-haiku-5-5, not `{value}`"));
    }
    None
}

/// Whether `new` is at least as strict as `old` for a guard-rail setting (`None` = unset).
pub fn tightens(key: &str, old: Option<&str>, new: Option<&str>) -> bool {
    // Not set = the built-in lists (v0.3.2), so going from them to an empty list loosens.
    if key == "block_actions" {
        let words = |v: Option<&str>| crate::policy::expand_blocked(v.map(|v| items_of(key, v)));
        // Only `default` (the built-in rules: whole words, a phrase with a word in between) covers a
        // built-in word; the same word written out is a plain word and drops the phrase rules
        // ("Delete my account", "Place your order"), which loosens (Security Medium, v0.3.2).
        let n = words(new);
        return words(old).iter().all(|w| n.contains(w));
    }
    if key == "confirm_actions" {
        let words = |v: Option<&str>| crate::policy::expand_risky(v.map(|v| items_of(key, v)));
        let n = words(new);
        return words(old).iter().all(|w| n.contains(w));
    }
    let items = |v: &str| items_of(key, v);
    let (o, n) = (old.map(items).unwrap_or_default(), new.map(items).unwrap_or_default());
    match key {
        // Empty means "any host": any list is stricter than none, and a list may only shrink.
        "allowed_domains" => !n.is_empty() && (o.is_empty() || n.iter().all(|d| o.contains(d))),
        // Fewer readable variables, never more.
        "vars" => n.iter().all(|v| o.contains(v)),
        "state_expire_days" => {
            let days = |v: Option<&str>| v.and_then(|d| d.trim().parse::<u64>().ok());
            matches!((days(old).or(Some(30)), days(new)), (Some(a), Some(b)) if b <= a)
        }
        // Like allowed_domains: a list, and it may only shrink.
        "upload_dirs" => !n.is_empty() && (o.is_empty() || n.iter().all(|d| o.contains(d))),
        // Option C (v0.3.2): not set = off while block_actions is on; `true` written out switches
        // it on even then. So false < not set < true, and an agent may only go down.
        "allow_eval_script" => {
            let level = |v: Option<&str>| match v.map(|v| v.trim().to_lowercase()) {
                None => 1,
                Some(v) if matches!(v.as_str(), "0" | "false" | "no" | "off") => 0,
                Some(_) => 2,
            };
            level(new) <= level(old)
        }
        // A permission may be taken away, not given back.
        k if k.starts_with("allow_") => {
            let on = |v: Option<&str>| v.map(|v| !matches!(v.trim().to_lowercase().as_str(), "0" | "false" | "no" | "off")).unwrap_or(true);
            !on(new) || on(old)
        }
        "max_steps" => {
            let n_ = |v: Option<&str>| v.and_then(|d| d.trim().parse::<u64>().ok());
            matches!((n_(old).or(Some(30)), n_(new)), (Some(a), Some(b)) if b <= a)
        }
        // No value (or 0) = the built-in 30 minutes `fastpilot run` uses (L062-11), not "no limit".
        "test_timeout" => {
            let m = |v: Option<&str>| v.and_then(|d| d.trim().parse::<u64>().ok()).filter(|m| *m > 0).unwrap_or(30);
            m(new) <= m(old)
        }
        _ => false,
    }
}

/// Parse a command-line or tool value into TOML: lists for list settings, booleans and numbers
/// where they look like one, else a string.
fn to_toml(key: &str, value: &str) -> toml::Value {
    if LISTS.contains(&key) {
        // Stored as written (hosts lowercase); comparisons fold case where it does not matter.
        let items = value.split(',').map(str::trim).filter(|s| !s.is_empty());
        let items: Vec<String> = if key == "allowed_domains" { items.map(str::to_lowercase).collect() } else { items.map(str::to_owned).collect() };
        return toml::Value::Array(items.into_iter().map(toml::Value::String).collect());
    }
    let v = value.trim();
    if FLAGS.contains(&key) {
        return toml::Value::Boolean(matches!(v.to_lowercase().as_str(), "true" | "on" | "yes" | "1"));
    }
    if NUMBERS.contains(&key) {
        return v.parse::<i64>().map(toml::Value::Integer).unwrap_or_else(|_| toml::Value::String(v.to_owned()));
    }
    // Everything else is text (`memory = "off"`, not a boolean).
    toml::Value::String(v.to_owned())
}

/// Write `key = value` to the file (`None` removes it), keeping the other keys and comments-free
/// layout. Returns the file path.
pub fn write(key: &str, value: Option<&str>) -> Result<PathBuf, String> {
    let s = setting(key).ok_or_else(|| {
        format!("unknown setting `{key}`; known: {}", SETTINGS.iter().map(|s| s.key).collect::<Vec<_>>().join(", "))
    })?;
    if let Some(e) = value.and_then(|v| invalid(s.key, v)) {
        return Err(e);
    }
    let p = path();
    let mut table: toml::Table = match std::fs::read_to_string(&p) {
        Ok(text) => text.parse().map_err(|e| format!("{} is not valid TOML: {e}", p.display()))?,
        Err(_) => toml::Table::new(),
    };
    match value {
        Some(v) => {
            table.insert(s.key.to_owned(), to_toml(s.key, v));
        }
        None => {
            table.remove(s.key);
        }
    }
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let text = format!("# fastpilot settings (`fastpilot config`); environment variables win.\n{}", toml::to_string(&table).map_err(|e| e.to_string())?);
    // Owner-only, and atomic: a reader never sees a half-written file.
    let tmp = p.with_extension(format!("toml.{}.tmp", std::process::id()));
    let written = {
        use std::io::Write;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        o.open(&tmp).and_then(|mut f| f.write_all(text.as_bytes())).and_then(|()| std::fs::rename(&tmp, &p))
    };
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("cannot write {}: {e}", p.display()));
    }
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(p)
}

/// A value as people read it: a switch as true / false (the file stores TOML booleans, read back
/// as 1 / 0; `allow_eval_script = true` showed as "1"), an empty list as "".
pub fn shown(key: &str, v: &str) -> String {
    if FLAGS.contains(&key) {
        return match v.trim().to_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => "true".to_owned(),
            "0" | "false" | "no" | "off" => "false".to_owned(),
            _ => v.to_owned(),
        };
    }
    if v.is_empty() { "\"\"".to_owned() } else { v.to_owned() }
}

/// Every documented setting with its effective value and source, one per line.
pub fn describe() -> String {
    let mut lines = vec![format!("settings ({}; environment variables win)", path().display())];
    for s in SETTINGS {
        let shown = |v: &str| shown(s.key, v);
        let value = match effective(s.key) {
            Source::Env(v) => format!("{} (env FASTPILOT_{})", shown(&v), s.key.to_uppercase()),
            Source::File(v) => format!("{} (file)", shown(&v)),
            Source::Default => match s.key {
                "block_actions" => format!("{} (default)", crate::policy::DEFAULT_BLOCKED.join(", ")),
                "confirm_actions" => format!("{} (default)", crate::policy::DEFAULT_CONFIRM.join(", ")),
                "allow_eval_script" => "off while block_actions is on, else on (default)".to_owned(),
                _ => "— (default)".to_owned(),
            },
        };
        let who = match s.rule {
            Rule::Free => "",
            Rule::TightenOnly => " [agent: stricter only]",
            Rule::HumanOnly => " [people only]",
        };
        lines.push(format!("{:<18} {value}{who}\n{:<18} {}", s.key, "", s.help));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_folder_stays_out_of_git() {
        let root = std::env::temp_dir().join(format!("fastpilot-ignore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = root.join(".fastpilot");
        ignore_in(&d);
        assert!(!d.exists(), "a missing folder is not made");
        std::fs::create_dir_all(&d).expect("dir");
        ignore_in(&d);
        assert!(std::fs::read_to_string(d.join(".gitignore")).expect("written").lines().any(|l| l == "*"));
        // What git makes of it: private files ignored, baselines kept (when git is here).
        if std::process::Command::new("git").arg("init").arg("-q").arg(&root).status().is_ok_and(|s| s.success()) {
            for (f, ignored) in [("history/chats/c.json", true), ("states/login.json", true), ("baselines/home.png", false), ("baselines/home.diff.png", true), (".gitignore", false)] {
                let st = std::process::Command::new("git").arg("-C").arg(&root).args(["check-ignore", "-q", "--no-index"]).arg(format!(".fastpilot/{f}")).status().expect("git");
                assert_eq!(st.success(), ignored, "{f}");
            }
        }
        // Never over the user's own file, and never in a folder with another name.
        std::fs::write(d.join(".gitignore"), "mine\n").expect("own");
        ignore_in(&d);
        assert_eq!(std::fs::read_to_string(d.join(".gitignore")).expect("kept"), "mine\n");
        let other = root.join("tests");
        std::fs::create_dir_all(&other).expect("dir");
        ignore_in(&other);
        assert!(!other.join(".gitignore").exists());
        #[cfg(unix)]
        {
            let linked = root.join("l/.fastpilot");
            std::fs::create_dir_all(&linked).expect("dir");
            std::os::unix::fs::symlink(root.join("victim"), linked.join(".gitignore")).expect("link");
            ignore_in(&linked);
            assert!(!root.join("victim").exists(), "a link is not followed");
            // A `.fastpilot` that is itself a link: nothing is written in its target.
            std::fs::create_dir_all(root.join("target")).expect("dir");
            let planted = root.join("p");
            std::fs::create_dir_all(&planted).expect("dir");
            std::os::unix::fs::symlink(root.join("target"), planted.join(".fastpilot")).expect("link");
            ignore_in(&planted.join(".fastpilot"));
            assert!(!root.join("target/.gitignore").exists(), "a linked .fastpilot is left alone");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn env_wins_file_fills_in_secrets_never_from_file() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("fastpilot-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("config.toml");
        std::fs::write(&file, "allowed_domains = [\"a.com\", \"*.b.com\"]\nheadful = true\nflow_timeout = 60\nstate_key = \"nope\"\n").expect("write");
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("FASTPILOT_CONFIG", &file);
            std::env::remove_var("FASTPILOT_ALLOWED_DOMAINS");
            std::env::remove_var("FASTPILOT_HEADFUL");
            std::env::set_var("FASTPILOT_FLOW_TIMEOUT", "120");
            std::env::remove_var("FASTPILOT_STATE_KEY");
        }
        assert_eq!(var("FASTPILOT_ALLOWED_DOMAINS").as_deref(), Ok("a.com,*.b.com"));
        assert_eq!(var("FASTPILOT_HEADFUL").as_deref(), Ok("1"));
        assert_eq!(var("FASTPILOT_FLOW_TIMEOUT").as_deref(), Ok("120"), "env wins");
        assert!(var("FASTPILOT_STATE_KEY").is_err(), "secrets only from the environment");
        assert_eq!(effective("flow_timeout"), Source::Env("120".into()));
        assert_eq!(effective("headful"), Source::File("1".into()));
        assert_eq!(effective("model"), Source::Default);

        // Writing applies to the next read, keeps the other keys, and removes on unset.
        std::thread::sleep(std::time::Duration::from_millis(10));
        write("confirm_actions", Some("default, hapus akun")).expect("set");
        assert_eq!(var("FASTPILOT_CONFIRM_ACTIONS").as_deref(), Ok("default,hapus akun"));
        assert_eq!(var("FASTPILOT_ALLOWED_DOMAINS").as_deref(), Ok("a.com,*.b.com"));
        write("headful", None).expect("unset");
        assert!(var("FASTPILOT_HEADFUL").is_err());
        assert!(write("frobnicate", Some("1")).expect_err("unknown").contains("unknown setting"));
        assert!(describe().contains("confirm_actions") && describe().contains("[agent: stricter only]"));

        // A broken file: `check` refuses it; in a running process the guard rails stay as they were.
        std::fs::write(&file, "allowed_domains = [").expect("write");
        assert!(check().expect_err("broken").contains("not valid TOML"));
        assert_eq!(var("FASTPILOT_ALLOWED_DOMAINS").as_deref(), Ok("a.com,*.b.com"), "not switched off");
        unsafe {
            std::env::remove_var("FASTPILOT_CONFIG");
            std::env::remove_var("FASTPILOT_FLOW_TIMEOUT");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// P0 review: a guard rail loosened in the file (by a shell, an editor or `config set`) does not
    /// apply to the running process; a stricter value does. Values are checked and kept as typed.
    #[test]
    fn guard_rails_only_tighten_while_running() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("fastpilot-config-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("config.toml");
        let _ = std::fs::remove_file(&file);
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("FASTPILOT_CONFIG", &file);
            std::env::remove_var("FASTPILOT_ALLOWED_DOMAINS");
            std::env::remove_var("FASTPILOT_VARS");
            std::env::remove_var("FASTPILOT_HEADFUL");
        }
        write("allowed_domains", Some("a.com,b.com")).expect("set");
        assert_eq!(var("FASTPILOT_ALLOWED_DOMAINS").as_deref(), Ok("a.com,b.com"));
        // Looser in the file: still the old value here, for `get` and the `config` tool too.
        std::fs::write(&file, "headful = true\n").expect("write");
        assert_eq!(var("FASTPILOT_ALLOWED_DOMAINS").as_deref(), Ok("a.com,b.com"), "emptied file");
        assert_eq!(var("FASTPILOT_HEADFUL").as_deref(), Ok("1"), "other settings stay live");
        std::fs::write(&file, "allowed_domains = [\"a.com\", \"evil.com\"]\n").expect("write");
        assert_eq!(var("FASTPILOT_ALLOWED_DOMAINS").as_deref(), Ok("a.com,b.com"), "wider list");
        assert_eq!(effective("allowed_domains"), Source::File("a.com,b.com".into()));
        // Stricter: applies at once.
        write("allowed_domains", Some("a.com")).expect("set");
        assert_eq!(var("FASTPILOT_ALLOWED_DOMAINS").as_deref(), Ok("a.com"));

        // `vars` keeps the case of variable names (`${DEMO_PASS}` is looked up as written). Checked
        // in the file: other tests read FASTPILOT_VARS without ENV_LOCK, and a read that lands
        // while FASTPILOT_CONFIG points here pins `vars` unset, which this wider list cannot loosen.
        write("vars", Some("DEMO_PASS, Api_Key")).expect("set");
        assert_eq!(file_values().get("vars").map(String::as_str), Some("DEMO_PASS,Api_Key"));
        // Numbers are numbers.
        for key in NUMBERS {
            assert!(write(key, Some("abc")).expect_err("not a number").contains("whole number"), "{key}");
        }
        assert!(write("flow_timeout", Some("1.5")).is_err() && write("flow_timeout", Some("90")).is_ok());
        // Owner-only file, no temporary file left behind.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(std::fs::read_dir(&dir).expect("dir").count(), 1, "only config.toml");
        unsafe {
            std::env::remove_var("FASTPILOT_CONFIG");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Security review (Medium): program paths and endpoints (`chrome`, `chrome_endpoint`,
    /// `adb_server`, ...) were read live, so an agent editing the file sent the next session of a
    /// running MCP server / daemon to another Chrome. Pinned at start like the guard rails.
    #[test]
    fn human_only_settings_are_pinned_at_start() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("fastpilot-config-human-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("config.toml");
        let human: Vec<&str> = SETTINGS.iter().filter(|s| s.rule == Rule::HumanOnly).map(|s| s.key).collect();
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("FASTPILOT_CONFIG", &file);
            for k in &human {
                std::env::remove_var(format!("FASTPILOT_{}", k.to_uppercase()));
            }
        }
        std::fs::write(&file, "chrome_endpoint = \"http://127.0.0.1:9222\"\nchrome = \"/opt/chrome\"\n").expect("write");
        pin_guard_rails();
        // Changed, added or removed in the file afterwards: this process keeps what it started with.
        std::fs::write(&file, "chrome_endpoint = \"ws://evil.example:9222\"\nadb_server = \"evil.example:5037\"\n").expect("write");
        assert_eq!(var("FASTPILOT_CHROME_ENDPOINT").as_deref(), Ok("http://127.0.0.1:9222"));
        assert!(var("FASTPILOT_ADB_SERVER").is_err(), "added after start");
        assert_eq!(var("FASTPILOT_CHROME").as_deref(), Ok("/opt/chrome"), "removed after start");
        assert_eq!(effective("chrome_endpoint"), Source::File("http://127.0.0.1:9222".into()));
        // Free settings stay live.
        std::fs::write(&file, "headful = true\n").expect("write");
        assert_eq!(var("FASTPILOT_HEADFUL").as_deref(), Ok("1"));
        unsafe {
            std::env::remove_var("FASTPILOT_CONFIG");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FastTest P0: the rails are pinned when the process starts, not at first use, so a file
    /// loosened between start and the first risky step does not apply. Stricter still applies.
    #[test]
    fn guard_rails_are_pinned_at_start() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("fastpilot-config-pin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("config.toml");
        let _ = std::fs::remove_file(&file);
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("FASTPILOT_CONFIG", &file);
            for k in ["FASTPILOT_ALLOWED_DOMAINS", "FASTPILOT_CONFIRM_ACTIONS", "FASTPILOT_VARS", "FASTPILOT_STATE_EXPIRE_DAYS"] {
                std::env::remove_var(k);
            }
        }
        std::fs::write(&file, "allowed_domains = [\"localhost\"]\nconfirm_actions = \"default\"\nvars = [\"DEMO_PASS\"]\nstate_expire_days = 7\n").expect("write");
        pin_guard_rails();
        // Loosened before any rail was read: nothing changes in this process.
        std::fs::write(&file, "").expect("empty");
        assert_eq!(var("FASTPILOT_ALLOWED_DOMAINS").as_deref(), Ok("localhost"));
        assert_eq!(var("FASTPILOT_CONFIRM_ACTIONS").as_deref(), Ok("default"));
        assert_eq!(var("FASTPILOT_STATE_EXPIRE_DAYS").as_deref(), Ok("7"));
        // Fewer readable variables is stricter, so dropping `vars` applies.
        assert!(var("FASTPILOT_VARS").is_err());
        // Stricter applies at once.
        std::fs::write(&file, "allowed_domains = [\"localhost\"]\nconfirm_actions = \"default,hapus\"\nvars = []\nstate_expire_days = 1\n").expect("write");
        assert_eq!(var("FASTPILOT_CONFIRM_ACTIONS").as_deref(), Ok("default,hapus"));
        assert_eq!(var("FASTPILOT_STATE_EXPIRE_DAYS").as_deref(), Ok("1"));
        assert_eq!(var("FASTPILOT_VARS").as_deref(), Ok(""));
        unsafe {
            std::env::remove_var("FASTPILOT_CONFIG");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// FastTest P0: `headful` and `memory` only take values fastpilot understands, and `memory off`
    /// stays the word `off` (a boolean would read as "rw").
    #[test]
    fn headful_and_memory_values_are_checked() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("fastpilot-config-values-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("config.toml");
        let _ = std::fs::remove_file(&file);
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("FASTPILOT_CONFIG", &file);
            std::env::remove_var("FASTPILOT_HEADFUL");
            std::env::remove_var("FASTPILOT_MEMORY");
        }
        assert!(write("headful", Some("maybe")).expect_err("not a bool").contains("true or false"));
        assert!(write("memory", Some("banana")).expect_err("not a mode").contains("rw, readonly or off"));
        for (v, want) in [("true", "1"), ("FALSE", "0"), ("True", "1")] {
            write("headful", Some(v)).expect("bool");
            assert_eq!(var("FASTPILOT_HEADFUL").as_deref(), Ok(want), "headful {v}");
        }
        // QA: headful takes true or false only (`yes`, `1`, `on` were taken as text or guessed).
        for v in ["yes", "1", "on", "maybe"] {
            assert!(write("headful", Some(v)).expect_err(v).contains("true or false"), "headful {v}");
        }
        for mode in MEMORY_MODES {
            write("memory", Some(mode)).expect("mode");
            assert_eq!(var("FASTPILOT_MEMORY").as_deref(), Ok(*mode));
        }
        unsafe {
            std::env::remove_var("FASTPILOT_CONFIG");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tighten_only_rules() {
        // allowed_domains: from any host to a list, or a smaller list; never wider or empty.
        assert!(tightens("allowed_domains", None, Some("a.com")));
        assert!(tightens("allowed_domains", Some("a.com,b.com"), Some("a.com")));
        assert!(!tightens("allowed_domains", Some("a.com"), Some("a.com,evil.com")));
        assert!(!tightens("allowed_domains", Some("a.com"), None));
        // confirm_actions: more words only.
        assert!(tightens("confirm_actions", Some("default"), Some("default,hapus")));
        assert!(!tightens("confirm_actions", Some("default,hapus"), Some("hapus")));
        assert!(!tightens("confirm_actions", Some("default"), None));
        // vars: fewer only.
        assert!(tightens("vars", Some("A,B"), Some("A")));
        assert!(!tightens("vars", Some("A"), Some("A,HOME_TOKEN")));
        assert!(!tightens("vars", Some("DEMO_PASS"), Some("demo_pass")), "another variable");
        // state_expire_days: shorter only (default 30).
        assert!(tightens("state_expire_days", None, Some("7")));
        assert!(!tightens("state_expire_days", Some("7"), Some("90")));
    }

    /// QA #6: values that would be misread are refused, from `config set` and from the file.
    #[test]
    fn loose_values_are_refused() {
        for (k, v) in [
            ("max_steps", "0"),
            ("adb_server", "notaserver"),
            ("adb_server", "host:99999"),
            ("chrome_endpoint", "evil.example:9222"),
            ("chrome_endpoint", "http://"),
            ("model_explore", "not-a-model"),
            ("model", "gpt-4o"),
            ("model_replay", "evil:m"),
            ("model_replay", "openrouter:a b"),
            ("model", "openrouter:m"),
        ] {
            assert!(invalid(k, v).is_some(), "{k} = {v} accepted");
        }
        for (k, v) in [("max_steps", "1"), ("adb_server", "127.0.0.1:15037"), ("chrome_endpoint", "http://127.0.0.1:19222"), ("chrome_endpoint", "ws://h:1/devtools/browser/x"), ("model_replay", "claude-haiku-5-5"), ("model_explore", "openrouter:deepseek/deepseek-v4-flash"), ("model_replay", "ollama:llama3.1:8b")] {
            assert!(invalid(k, v).is_none(), "{k} = {v} refused");
        }
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("fastpilot-config-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("config.toml");
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe { std::env::set_var("FASTPILOT_CONFIG", &file) };
        for (text, want) in [
            ("flow_timeout = \"x\"\n", "flow_timeout"),
            ("headful = \"maybe\"\n", "headful"),
            ("allowed_domains = 5\n", "list of text"),
            ("max_steps = [1]\n", "one value"),
        ] {
            std::fs::write(&file, text).expect("write");
            let e = check().expect_err(text);
            assert!(e.contains(want) && e.contains("fix or delete it"), "{text}: {e}");
        }
        // QA: a quoted flag is read as text ("yes" was silently headless), list items are checked.
        for (text, want) in [
            ("headful = \"yes\"\n", "without quotes"),
            ("allow_eval_script = \"false\"\n", "without quotes"),
            ("allowed_domains = [\"https://example.net/path\"]\n", "not a host"),
            ("vars = [\"FASTPILOT_STATE_KEY\"]\n", "FASTPILOT_*"),
        ] {
            std::fs::write(&file, text).expect("write");
            let e = check().expect_err(text);
            assert!(e.contains(want), "{text}: {e}");
        }
        std::fs::write(&file, "headful = true\nallowed_domains = \"a.com\"\nflow_timeout = 60\nunknown_key = 1\n").expect("write");
        assert!(check().is_ok());
        unsafe { std::env::remove_var("FASTPILOT_CONFIG") };
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn origins_and_presets() {
        assert_eq!(origin("https://API.example.com:8443/v1?x=1").as_deref(), Some("https://api.example.com:8443"));
        assert_eq!(origin("http://u:p@127.0.0.1:9/v1").as_deref(), Some("http://127.0.0.1:9"));
        assert_eq!(origin("not a url"), None);
        assert_eq!(preset_base("groq"), Some("https://api.groq.com/openai/v1"));
        assert_eq!(preset_base("custom"), None);
    }

    /// B2: provider settings are checked, and the provider's base URL defaults per provider.
    #[test]
    fn provider_settings_are_checked() {
        for (k, v) in [("provider", "openai"), ("provider", "ollama"), ("provider_base_url", "https://openrouter.ai/api/v1"), ("provider_base_url", "http://127.0.0.1:11434/v1"), ("provider_model", "gpt-x")] {
            assert_eq!(invalid(k, v), None, "{k} = {v}");
        }
        for (k, v, want) in [
            ("provider", "anthropic-api", "one of"),
            ("provider_base_url", "api.openai.com/v1", "http(s) URL"),
            ("provider_base_url", "https://user:pw@example.com/v1", "without credentials"),
            ("provider_base_url", "http://example.com/v1", "unencrypted"),
            ("provider_model", "two words", "model id"),
        ] {
            assert!(invalid(k, v).is_some_and(|e| e.contains(want)), "{k} = {v}: {:?}", invalid(k, v));
        }
        assert!(setting("provider").is_some_and(|s| s.rule == Rule::HumanOnly) && setting("provider_base_url").is_some_and(|s| s.rule == Rule::HumanOnly));
        assert!(SECRETS.contains(&"FASTPILOT_PROVIDER_API_KEY"), "the key never comes from the file");
    }

    /// QA #6: names fastpilot would never match, or must not hand to a flow, are refused when set.
    #[test]
    fn vars_and_domains_are_checked() {
        for (k, v, want) in [
            ("vars", "FASTPILOT_STATE_KEY", "FASTPILOT_*"),
            ("vars", "DEMO_PASS,fastpilot_token", "FASTPILOT_*"),
            ("vars", "bad-name!", "not an environment variable name"),
            ("vars", "9LIVES", "not an environment variable name"),
            ("allowed_domains", "https://example.net/path", "not a host"),
            ("allowed_domains", "example.net/path", "not a host"),
            ("allowed_domains", "example.net:8080", "not a host"),
            ("allowed_domains", "bad domain", "not a host"),
            ("allowed_domains", "*.", "not a host"),
        ] {
            assert!(invalid(k, v).is_some_and(|e| e.contains(want)), "{k} = {v}: {:?}", invalid(k, v));
        }
        for (k, v) in [("vars", "DEMO_PASS, _X1"), ("allowed_domains", "example.net, *.staging.example.net, localhost, 127.0.0.1")] {
            assert_eq!(invalid(k, v), None, "{k} = {v}");
        }
    }

    /// C2: the new guard rails only get stricter through the agent's `config` tool.
    #[test]
    fn new_guard_rails_tighten_only() {
        assert!(tightens("block_actions", Some(""), Some("Bayar")));
        assert!(tightens("block_actions", Some("bayar"), Some("Bayar,Hapus")), "case does not matter");
        assert!(!tightens("block_actions", Some("bayar"), None), "the built-in `bayar` is whole words only");
        // F5 (v0.3.2): not set = the built-in list. The agent may add to it, never empty it or
        // replace it with fewer words; restoring it after the user emptied the list is stricter.
        assert!(!tightens("block_actions", None, Some("")), "built-in → empty loosens");
        assert!(!tightens("block_actions", None, Some("Bayar")), "built-in → fewer words loosens");
        assert!(tightens("block_actions", None, Some("default,Keluar")));
        // Security Medium (v0.3.2 round 3): the same words written out are plain words, without the
        // built-in phrase rules ("Delete my account", "Place your order"): that loosens.
        assert!(!tightens("block_actions", None, Some(&crate::policy::DEFAULT_BLOCKED.join(","))), "the same words, written out, loosen");
        assert!(!tightens("block_actions", Some("default"), Some(&format!("{},keluar", crate::policy::DEFAULT_BLOCKED.join(",")))));
        assert!(tightens("block_actions", None, Some(&format!("default,{}", crate::policy::DEFAULT_BLOCKED.join(",")))), "default kept: fine");
        assert!(tightens("block_actions", Some(""), None), "back to the built-in list");
        assert!(!tightens("block_actions", Some("default,keluar"), None), "drops the user's word");
        assert!(!tightens("confirm_actions", None, Some("")));
        assert!(tightens("confirm_actions", None, Some("hapus,delete,remove,keluar")));
        assert!(tightens("confirm_actions", Some(""), None));
        assert!(!tightens("confirm_actions", None, Some("hapus")));
        // Option C: `allow_eval_script = true` unlocks evalScript while block_actions is on, so an
        // agent may not write it (only the user), nor go from false back to not set.
        assert!(!tightens("allow_eval_script", None, Some("true")), "agent cannot unlock evalScript");
        assert!(tightens("allow_eval_script", Some("true"), None) && tightens("allow_eval_script", None, Some("false")));
        assert!(!tightens("allow_eval_script", Some("false"), None));
        for k in ["allow_app_install", "allow_app_reset", "allow_physical_devices", "allow_device_settings", "allow_eval_script"] {
            assert!(tightens(k, None, Some("false")), "{k}: switch off");
            assert!(!tightens(k, Some("false"), Some("true")), "{k}: switch back on");
            assert!(!tightens(k, Some("false"), None), "{k}: unset = on");
            assert!(setting(k).is_some_and(|s| s.rule == Rule::TightenOnly), "{k}");
        }
        assert!(tightens("upload_dirs", None, Some("/a")));
        assert!(tightens("upload_dirs", Some("/a,/b"), Some("/a")));
        assert!(!tightens("upload_dirs", Some("/a"), Some("/a,/etc")));
        assert!(!tightens("upload_dirs", Some("/a"), None));
        assert!(tightens("max_steps", None, Some("20")) && !tightens("max_steps", Some("20"), Some("40")) && !tightens("max_steps", None, Some("31")));
        assert!(tightens("test_timeout", None, Some("10")) && tightens("test_timeout", Some("10"), Some("5")));
        assert!(!tightens("test_timeout", Some("10"), Some("30")) && !tightens("test_timeout", Some("10"), None));
        // L062-11: an empty value is the built-in 30 minutes, so 60 is looser, not stricter.
        assert!(!tightens("test_timeout", None, Some("60")) && !tightens("test_timeout", None, Some("31")) && tightens("test_timeout", None, Some("30")) && tightens("test_timeout", Some("60"), None));
        let help = SETTINGS.iter().find(|s| s.key == "test_timeout").map(|s| s.help).unwrap_or("");
        assert!(help.ends_with("(default: 30)") && !help.contains("no limit"), "{help}");
        // Values are checked; words keep their case in the file.
        assert!(invalid("allow_eval_script", "maybe").is_some() && invalid("allow_eval_script", "off").is_none());
        assert!(invalid("test_timeout", "10m").is_some());
        assert_eq!(to_toml("block_actions", "Bayar, Lanjut"), toml::Value::Array(vec!["Bayar".into(), "Lanjut".into()]));
        assert_eq!(to_toml("allowed_domains", "Toko.COM"), toml::Value::Array(vec!["toko.com".into()]));
        assert_eq!(to_toml("allow_eval_script", "off"), toml::Value::Boolean(false));
    }

    /// Security Medium (v0.3.2 round 3): whatever `tightens` lets the agent write for block_actions
    /// still blocks every text the old list blocked (built-in phrases with a gap, odd spacing,
    /// fullwidth, zero-width).
    #[test]
    fn block_actions_changes_the_agent_may_make_block_at_least_as_much() {
        use crate::policy::{block_match, expand_blocked, DEFAULT_BLOCKED};
        let probes = [
            "Bayar sekarang", "Pay now", "Ｐａｙ now", "Pa\u{200b}y now", "Delete my account", "Delete account", "Place your order",
            "Place order", "Hapus akun saya", "Hapus  akun", "Buat pesanan", "Checkout", "Transfer dana", "Keluar", "Paypal", "Masuk",
        ];
        let all = DEFAULT_BLOCKED.join(",");
        let candidates = [
            None, Some(""), Some("default"), Some("default,keluar"), Some(all.as_str()), Some("bayar,pay,checkout"),
            Some("delete account,place order"), Some("keluar"),
        ];
        let blocked = |v: Option<&str>, t: &str| block_match(t, &expand_blocked(v.map(|v| items_of("block_actions", v)))).is_some();
        for old in candidates {
            for new in candidates {
                if !tightens("block_actions", old, new) {
                    continue;
                }
                for t in probes {
                    assert!(!blocked(old, t) || blocked(new, t), "{old:?} -> {new:?} was accepted, but {t:?} is no longer blocked");
                }
            }
        }
    }


    /// T1-T3: a switch in the file read back as "1" / "0" and was shown that way.
    #[test]
    fn switches_show_as_true_or_false() {
        assert_eq!(shown("allow_eval_script", "1"), "true");
        assert_eq!(shown("allow_eval_script", "0"), "false");
        assert_eq!(shown("headful", "1"), "true");
        assert_eq!(shown("max_steps", "1"), "1", "numbers stay numbers");
        assert_eq!(shown("block_actions", ""), "\"\"");
    }

}
