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
    Setting { key: "confirm_actions", help: "taps on elements named like these need `confirm: true` (`default` = delete, pay, buy, transfer, ...)", rule: Rule::TightenOnly },
    Setting { key: "vars", help: "environment variables flows may read as ${NAME}; their values are masked as ***", rule: Rule::TightenOnly },
    Setting { key: "state_expire_days", help: "saved login state is refused after this many days (default 30)", rule: Rule::TightenOnly },
    Setting { key: "headful", help: "show the browser window (default false: headless, never in the way)", rule: Rule::Free },
    Setting { key: "model", help: "`fastpilot run`: pin one Claude model for every test", rule: Rule::Free },
    Setting { key: "model_replay", help: "`fastpilot run`: model that replays path memory (default claude-haiku-5-5)", rule: Rule::Free },
    Setting { key: "model_explore", help: "`fastpilot run`: model that explores (default claude-opus-5-5)", rule: Rule::Free },
    Setting { key: "flow_timeout", help: "seconds one run_flow may take (default 300)", rule: Rule::Free },
    Setting { key: "retry_budget", help: "`fastpilot run`: reruns of failed tests per suite (default 3)", rule: Rule::Free },
    Setting { key: "breaker", help: "`fastpilot run`: consecutive infra failures before the rest is skipped (default 3)", rule: Rule::Free },
    Setting { key: "android_serial", help: "default Android device or emulator serial", rule: Rule::Free },
    Setting { key: "ios_device", help: "simulator booted when none runs (default iPhone 16)", rule: Rule::Free },
    Setting { key: "ios_udid", help: "default iOS simulator or device UDID", rule: Rule::Free },
    Setting { key: "memory", help: "path memory: rw (default), readonly, off", rule: Rule::Free },
    Setting { key: "daemon_idle", help: "shell CLI daemon exits after this many idle seconds (default 1800)", rule: Rule::Free },
    Setting { key: "chrome", help: "Chrome/Chromium executable", rule: Rule::HumanOnly },
    Setting { key: "claude", help: "`fastpilot run`: the claude CLI to drive", rule: Rule::HumanOnly },
    Setting { key: "dir", help: "where runs, memory, states and baselines go (default ./.fastpilot)", rule: Rule::HumanOnly },
    Setting { key: "ios_runner", help: "iOS runner build folder (default ios-runner/build)", rule: Rule::HumanOnly },
];

/// Never read from the file.
const SECRETS: &[&str] = &["FASTPILOT_STATE_KEY", "FASTPILOT_TOKEN", "FASTPILOT_RELAY_TOKEN"];

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
    match std::fs::read_to_string(&p) {
        Ok(text) => text.parse::<toml::Table>().map(|_| ()).map_err(|e| {
            format!("{} is not valid TOML, so its settings (guard rails included) would be ignored: fix or delete it\n{e}", p.display())
        }),
        Err(_) => Ok(()),
    }
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
    if setting(key).is_none_or(|s| s.rule != Rule::TightenOnly) {
        return value;
    }
    let mut guard = GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let pinned = guard.entry((path(), key.to_owned())).or_insert_with(|| value.clone());
    if *pinned != value {
        if tightens(key, pinned.as_deref(), value.as_deref()) {
            *pinned = value;
        } else {
            tracing::warn!(key, "config file loosens a guard rail: ignored until fastpilot restarts");
        }
    }
    pinned.clone()
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

pub fn setting(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key)
}

/// List items; hosts and words compare lowercase, variable names (`vars`) keep their case.
fn items_of(key: &str, v: &str) -> Vec<String> {
    let fold = |s: &str| if key == "vars" { s.to_owned() } else { s.to_lowercase() };
    v.split(',').map(|s| fold(s.trim())).filter(|s| !s.is_empty()).collect()
}

/// Settings whose value is a whole number.
const NUMBERS: &[&str] = &["state_expire_days", "flow_timeout", "retry_budget", "breaker", "daemon_idle"];

/// Whether `new` is at least as strict as `old` for a guard-rail setting (`None` = unset).
pub fn tightens(key: &str, old: Option<&str>, new: Option<&str>) -> bool {
    let items = |v: &str| items_of(key, v);
    let (o, n) = (old.map(items).unwrap_or_default(), new.map(items).unwrap_or_default());
    match key {
        // Empty means "any host": any list is stricter than none, and a list may only shrink.
        "allowed_domains" => !n.is_empty() && (o.is_empty() || n.iter().all(|d| o.contains(d))),
        // More words to confirm, never fewer.
        "confirm_actions" => o.iter().all(|w| n.contains(w)),
        // Fewer readable variables, never more.
        "vars" => n.iter().all(|v| o.contains(v)),
        "state_expire_days" => {
            let days = |v: Option<&str>| v.and_then(|d| d.trim().parse::<u64>().ok());
            matches!((days(old).or(Some(30)), days(new)), (Some(a), Some(b)) if b <= a)
        }
        _ => false,
    }
}

/// Parse a command-line or tool value into TOML: lists for list settings, booleans and numbers
/// where they look like one, else a string.
fn to_toml(key: &str, value: &str) -> toml::Value {
    if matches!(key, "allowed_domains" | "confirm_actions" | "vars") {
        return toml::Value::Array(items_of(key, value).into_iter().map(toml::Value::String).collect());
    }
    match value.trim() {
        "true" | "on" | "yes" => toml::Value::Boolean(true),
        "false" | "off" | "no" => toml::Value::Boolean(false),
        v => v.parse::<i64>().map(toml::Value::Integer).unwrap_or_else(|_| toml::Value::String(v.to_owned())),
    }
}

/// Write `key = value` to the file (`None` removes it), keeping the other keys and comments-free
/// layout. Returns the file path.
pub fn write(key: &str, value: Option<&str>) -> Result<PathBuf, String> {
    let s = setting(key).ok_or_else(|| {
        format!("unknown setting `{key}`; known: {}", SETTINGS.iter().map(|s| s.key).collect::<Vec<_>>().join(", "))
    })?;
    if let Some(v) = value
        && NUMBERS.contains(&s.key)
        && v.trim().parse::<u64>().is_err()
    {
        return Err(format!("`{}` takes a whole number of {}, not `{v}`", s.key, if s.key == "retry_budget" || s.key == "breaker" { "runs" } else if s.key == "state_expire_days" { "days" } else { "seconds" }));
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

/// Every documented setting with its effective value and source, one per line.
pub fn describe() -> String {
    let mut lines = vec![format!("settings ({}; environment variables win)", path().display())];
    for s in SETTINGS {
        let shown = |v: &str| if v.is_empty() { "\"\"".to_owned() } else { v.to_owned() };
        let value = match effective(s.key) {
            Source::Env(v) => format!("{} (env FASTPILOT_{})", shown(&v), s.key.to_uppercase()),
            Source::File(v) => format!("{} (file)", shown(&v)),
            Source::Default => "— (default)".to_owned(),
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

        // `vars` keeps the case of variable names (`${DEMO_PASS}` is looked up as written).
        write("vars", Some("DEMO_PASS, Api_Key")).expect("set");
        assert_eq!(var("FASTPILOT_VARS").as_deref(), Ok("DEMO_PASS,Api_Key"));
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
}
