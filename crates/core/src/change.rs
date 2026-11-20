//! D9 "All settings through chat": one settings change as data, described in fastpilot's own words,
//! and the rules every way of asking for it follows: Chat cards in `fastpilot ui`, the `config`
//! tool's pending requests, and `fastpilot config approve` (Security review D9, M1–M15).
//!
//! - A change is `(key, before, after)` with the values as the config file holds them (`None` =
//!   not set). Its card text comes only from the templates here, never from the AI (M3).
//! - [`level`]: a bounded tightening applies at once (M7), other tightenings and model changes need
//!   a click, anything else loosens a limit and needs the user (red card, or a pending request).
//! - [`check`] refuses what never goes through chat or the AI (M8), and the dangerous values of M11.
//! - [`provenance`]: a loosening must be asked for in the user's own words this turn (M1).
//! - [`Pending`]: the `config` tool's requests for the user, structured and short-lived (M13).
//! - [`audit`]: an append-only log of what was asked and what happened (M14). It is evidence for an
//!   honest user, not a lock: a program running as the same user can edit any of these files.
//!
//! Nothing here touches a running test: guard rails are pinned when a process starts (M15), so a
//! change applies from the next run (and the next Chat conversation, M6).

use serde::{Deserialize, Serialize};

use crate::config::{self, Rule};
use crate::policy;

/// Loosening cards and pending requests expire after this long (M5, M13).
pub const EXPIRY_MS: u64 = 10 * 60 * 1000;
/// At most this many pending requests (M13) and open loosening cards (M4).
pub const MAX_PENDING: usize = 5;

/// Never changed through chat or the AI: they redirect all test content or run programs (M8).
pub const NEVER: &[&str] = &["provider", "provider_base_url", "chrome", "claude", "dir", "ios_runner", "adb_server", "chrome_endpoint"];

/// The model settings a chat may change (to a validated id, M8).
pub const MODELS: &[&str] = &["model", "model_replay", "model_explore", "provider_model"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub key: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// How a change is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Stricter and bounded: applied at once, with Undo (M7).
    Applied,
    /// Stricter, or another kind of change (the AI model): nothing happens before a click.
    Ok,
    /// Loosens a limit: only the user, on a red card or with `fastpilot config approve`.
    Loosen,
}

/// What a card or the terminal shows, all from fastpilot's own templates (M3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Card {
    pub title: String,
    pub now: String,
    pub after: String,
    /// One sentence: what becomes possible (or stops being possible).
    pub consequence: String,
    pub level: Level,
    /// The word the user types to confirm (M11): `Pay`, a host, a folder name, a secret name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typed: Option<String>,
    /// More the user should know (a secret that looks like a key, whether it exists, ...).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// An edit to one setting, as the AI asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Add one item to a list.
    Add(String),
    /// Remove one item from a list (a built-in blocked word is allowed again with `!word`).
    Remove(String),
    /// Set a single value.
    Set(String),
    /// Back to the default.
    Unset,
}

/// The value in the config file now; an environment variable wins over the file and is not
/// changed from here.
pub fn raw(key: &str) -> Result<Option<String>, String> {
    let name = format!("FASTPILOT_{}", key.to_uppercase());
    if std::env::var_os(&name).is_some() {
        return Err(format!("{} is set by an environment variable ({name}) where fastpilot runs, so it is changed there", label(key)));
    }
    Ok(config::file_values().remove(key))
}

/// List items as written; a list that is not set starts from its built-in content.
fn items(key: &str, raw: Option<&str>) -> Vec<String> {
    match raw {
        None if key == "block_actions" => vec!["default".into()],
        None if key == "confirm_actions" => policy::DEFAULT_CONFIRM.iter().map(|w| (*w).to_owned()).collect(),
        None => vec![],
        Some(v) => v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect(),
    }
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// The value after `op` on `before`.
pub fn apply(key: &str, before: Option<&str>, op: &Op) -> Result<Option<String>, String> {
    let list = config::LISTS.contains(&key);
    if let Op::Add(item) | Op::Remove(item) = op
        && list
        && item.contains(',')
    {
        return Err("Add or remove one item at a time (no commas).".into());
    }
    match op {
        Op::Add(_) | Op::Remove(_) if !list => Err(format!("{} is not a list: set its value instead", label(key))),
        Op::Set(_) if list => Err(format!("{} is a list: add or remove one item at a time", label(key))),
        Op::Add(item) => {
            let item = item.trim();
            // Security (D9): "!pay" or "default" are not words; allowing a built-in word is the
            // "Allow Pay" change (remove), with its typed word.
            if matches!(key, "block_actions" | "confirm_actions") && (item.starts_with('!') || same(item, "default")) {
                return Err(format!("\"{item}\" is not a word to block or confirm. To allow a blocked button, ask to allow it by name; the built-in list itself is changed only in Settings."));
            }
            let mut v = items(key, before);
            if key == "block_actions" && let Some(i) = v.iter().position(|x| x.strip_prefix('!').is_some_and(|w| same(w, item))) {
                // A built-in word allowed earlier is blocked again.
                v.remove(i);
            } else if v.iter().any(|x| same(x, item)) {
                return Err(format!("\"{item}\" is already in {}", label(key)));
            } else {
                v.push(item.to_owned());
            }
            Ok(Some(v.join(",")))
        }
        Op::Remove(item) => {
            let item = item.trim();
            if key == "block_actions" && same(item, "default") {
                return Err(DEFAULT_ONLY_IN_SETTINGS.into());
            }
            let mut v = items(key, before);
            let had = v.len();
            v.retain(|x| !same(x, item));
            if key == "block_actions" && policy::is_builtin_blocked(item) && v.iter().any(|x| x == "default") {
                if v.iter().any(|x| x.strip_prefix('!').is_some_and(|w| same(w, item))) {
                    return Err(format!("\"{item}\" is already allowed"));
                }
                v.push(format!("!{}", policy::normalize(item)));
            } else if v.len() == had {
                return Err(format!("\"{item}\" is not in {}", label(key)));
            }
            Ok(Some(v.join(",")))
        }
        Op::Set(value) => Ok(Some(value.trim().to_owned())),
        Op::Unset => Ok(None),
    }
}

/// The setting as people know it.
pub fn label(key: &str) -> &'static str {
    match key {
        "allowed_domains" => "the sites tests may open",
        "confirm_actions" => "the buttons that need a confirmation",
        "block_actions" => "the buttons that are always blocked",
        "vars" => "the secrets tests may use",
        "state_expire_days" => "how long a saved login is kept",
        "allow_app_install" => "installing apps",
        "allow_app_reset" => "wiping app data",
        "allow_physical_devices" => "using real phones",
        "allow_device_settings" => "changing device settings",
        "allow_eval_script" => "running JavaScript on tested pages",
        "allow_takeover" => "taking over a test",
        "upload_dirs" => "the folders tests may upload from",
        "max_steps" => "the steps per test",
        "test_timeout" => "the time per test",
        "model" => "the AI model for every test",
        "model_replay" => "the AI model for repeat runs and regressions",
        "model_explore" => "the AI model for new tests",
        "provider_model" => "the AI model at your provider",
        _ => "this setting",
    }
}

/// Security (D9 finding 2): every built-in block at once is never dropped from chat or a request.
const DEFAULT_ONLY_IN_SETTINGS: &str = "The built-in list (payment, deleting an account and the like) can only be switched off by you, in Settings. To allow one button, ask to allow it by name.";

fn home() -> String {
    std::env::var("HOME").unwrap_or_default().trim_end_matches('/').to_owned()
}

/// Whether the change may be made from chat or the AI at all, and its values are safe (M8, M11).
pub fn check(c: &Change) -> Result<(), String> {
    let s = config::setting(&c.key).ok_or_else(|| format!("fastpilot has no setting called \"{}\"", c.key))?;
    if NEVER.contains(&c.key.as_str()) || s.rule == Rule::HumanOnly {
        return Err("This setting can only be changed by you, in Settings or with `fastpilot config set`: it decides where tests send what they see, or which program runs.".into());
    }
    if s.rule == Rule::Free && !MODELS.contains(&c.key.as_str()) {
        return Err("Change this one in Settings.".into());
    }
    if c.key == "allow_takeover" && !config::tightens(&c.key, c.before.as_deref(), c.after.as_deref()) {
        return Err("Taking over a test can only be switched back on in Settings.".into());
    }
    if c.key == "provider_model" && config::var("FASTPILOT_PROVIDER").ok().is_none_or(|p| p.is_empty() || p == "claude") {
        return Err("Tests run with Claude Code, so there is no provider model to change; pick a Claude model instead.".into());
    }
    if let Some(e) = c.after.as_deref().and_then(|v| config::invalid(&c.key, v)) {
        return Err(e);
    }
    let after = items(&c.key, c.after.as_deref());
    let before = items(&c.key, c.before.as_deref());
    let added: Vec<&String> = after.iter().filter(|a| !before.iter().any(|b| same(a, b))).collect();
    if c.key == "block_actions" {
        // Security (D9 findings 1, 2, 7): the built-in list stays, and the only `!word` is a
        // built-in word allowed one at a time, as `apply` writes it (lowercase, normalised).
        if before.iter().any(|b| b == "default") && !after.iter().any(|a| a == "default") {
            return Err(DEFAULT_ONLY_IN_SETTINGS.into());
        }
        for w in added.iter().filter_map(|w| w.strip_prefix('!')) {
            if !(policy::is_builtin_blocked(w) && policy::normalize(w) == w && after.iter().any(|a| a == "default")) {
                return Err(format!("\"!{w}\" is not a built-in word that can be allowed. To allow a blocked button, ask to allow it by name."));
            }
        }
    }
    if c.key == "confirm_actions" && added.iter().any(|w| w.starts_with('!')) {
        return Err("A word to confirm cannot start with \"!\".".into());
    }
    if matches!(c.key.as_str(), "block_actions" | "confirm_actions") {
        for w in added.iter().filter(|w| !w.starts_with('!') && w.as_str() != "default") {
            let n = w.chars().count();
            if !(3..=40).contains(&n) {
                return Err(format!("A word to block or confirm needs 3 to 40 characters (\"{w}\" has {n}): a shorter one would match almost every button."));
            }
        }
        if after.len() > 100 {
            return Err("That list would have more than 100 words.".into());
        }
    }
    if c.key == "allowed_domains" {
        for h in &added {
            if let Some(base) = h.strip_prefix("*.")
                && !base.contains('.')
            {
                return Err(format!("\"{h}\" would allow every site under .{base}; name the site, e.g. *.example.{base}"));
            }
        }
    }
    if c.key == "upload_dirs" {
        let home = home();
        for d in &added {
            let d = d.trim_end_matches('/');
            let full = match d.strip_prefix("~") {
                Some(rest) => format!("{home}{rest}"),
                None => d.to_owned(),
            };
            let bad = full.is_empty()
                || !full.starts_with('/')
                || full == home
                || full.starts_with(&format!("{home}/.ssh"))
                || full.split('/').any(|p| p == "Library" || p == ".ssh" || p == "..");
            if bad {
                return Err(format!("Uploads cannot come from \"{d}\": pick a folder of test files, not the whole disk, your home folder, ~/.ssh or Library."));
            }
        }
    }
    // A switch already in that position (`allow_*` default to on; `allow_eval_script` has a third,
    // unset state), or a list or value that stays the same.
    let switch = |v: Option<&str>| match v {
        None if c.key == "allow_eval_script" => 1,
        v if off(v) => 0,
        _ => 2,
    };
    let same_switch = c.key.starts_with("allow_") && switch(c.before.as_deref()) == switch(c.after.as_deref());
    if same_switch || (before == after && c.before.is_some() == c.after.is_some()) {
        return Err("Nothing to change: it is already so.".into());
    }
    Ok(())
}

/// Turned off (false) for a switch.
fn off(v: Option<&str>) -> bool {
    v.is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "0" | "false" | "no" | "off"))
}

/// How the change is decided (M7: only bounded tightenings apply without a click).
pub fn level(c: &Change) -> Level {
    if MODELS.contains(&c.key.as_str()) {
        return Level::Ok;
    }
    if !config::tightens(&c.key, c.before.as_deref(), c.after.as_deref()) {
        return Level::Loosen;
    }
    let before = items(&c.key, c.before.as_deref());
    let after = items(&c.key, c.after.as_deref());
    let one_more_word = matches!(c.key.as_str(), "block_actions" | "confirm_actions") && after.len() == before.len() + 1 && before.iter().all(|b| after.contains(b));
    let switched_off = c.key.starts_with("allow_") && off(c.after.as_deref());
    // A built-in word allowed earlier, blocked again (`!pay` dropped).
    let reblock = c.key == "block_actions" && before.len() == after.len() + 1 && after.iter().all(|a| before.contains(a)) && before.iter().any(|b| b.starts_with('!') && !after.contains(b));
    if one_more_word || switched_off || reblock { Level::Applied } else { Level::Ok }
}

/// Probe texts for the built-in rules: each word alone, followed by "now", and the phrases with a
/// word in between ("Delete my account").
fn probes() -> Vec<String> {
    let mut p: Vec<String> = Vec::new();
    for w in policy::DEFAULT_BLOCKED {
        p.push((*w).to_owned());
        p.push(format!("{w} now"));
        if let Some((a, b)) = w.split_once(' ') {
            p.push(format!("{a} my {b}"));
            p.push(format!("{a} your {b}"));
        }
    }
    p
}

/// Probes blocked under `before` but not under `after` (`block_actions`).
pub fn unblocked(before: Option<&str>, after: Option<&str>) -> Vec<String> {
    let rules = |v: Option<&str>| policy::expand_blocked(v.map(|v| v.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()));
    let (b, a) = (rules(before), rules(after));
    probes().into_iter().filter(|p| policy::block_match(p, &b).is_some() && policy::block_match(p, &a).is_none()).collect()
}

/// A built-in word allowed by this change (`!word` added), if that is what it does.
fn allowed_word(c: &Change) -> Option<String> {
    let before = items(&c.key, c.before.as_deref());
    items(&c.key, c.after.as_deref()).into_iter().filter(|a| !before.contains(a)).find_map(|a| a.strip_prefix('!').map(policy::normalize))
}

fn title_case(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn shown(key: &str, v: Option<&str>) -> String {
    match (key, v.map(str::trim)) {
        ("block_actions", v) => {
            let words: Vec<String> = policy::expand_blocked(v.map(|v| v.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect())).iter().map(|w| w.trim().to_owned()).collect();
            if words.is_empty() { "nothing is blocked".into() } else { format!("{} blocked: {}", words.len(), words.join(", ")) }
        }
        ("confirm_actions", v) => {
            let words = items(key, v);
            if words.is_empty() { "nothing needs a confirmation".into() } else { words.join(", ") }
        }
        ("allowed_domains", None | Some("")) => "any site".into(),
        ("upload_dirs", None | Some("")) => "any folder".into(),
        ("vars", None | Some("")) => "none".into(),
        ("max_steps", None) => "30 steps (default)".into(),
        ("max_steps", Some(n)) => format!("{n} steps"),
        ("test_timeout", None | Some("0")) => "no limit".into(),
        ("test_timeout", Some(n)) => format!("{n} minutes"),
        ("state_expire_days", None) => "30 days (default)".into(),
        ("state_expire_days", Some(n)) => format!("{n} days"),
        (k, v) if k.starts_with("allow_") => {
            if k == "allow_eval_script" && v.is_none() {
                "off while buttons are blocked (default)".into()
            } else if off(v) {
                "not allowed".into()
            } else {
                "allowed".into()
            }
        }
        (k, None) if MODELS.contains(&k) => "the default".into(),
        (_, Some(v)) => items(key, Some(v)).join(", "),
        (_, None) => "not set".into(),
    }
}

const PAYING: &[&str] = &["bayar", "pay", "beli", "buy", "checkout", "purchase", "transfer", "buat pesanan", "place order"];

/// Credential-like secret names get a warning (M10).
pub fn credential_like(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    ["TOKEN", "KEY", "SECRET"].iter().any(|w| n.contains(w)) || ["AWS_", "ANTHROPIC_", "GITHUB_"].iter().any(|p| n.starts_with(p))
}

/// The card for `c`, from fixed templates and the validated values only (M3).
pub fn card(c: &Change) -> Card {
    let level = level(c);
    let (b, a) = (c.before.as_deref(), c.after.as_deref());
    let key = c.key.as_str();
    let mut typed = None;
    let mut notes = Vec::new();
    let before_items = items(key, b);
    let after_items = items(key, a);
    let added: Vec<String> = after_items.iter().filter(|x| !before_items.iter().any(|y| same(x, y))).cloned().collect();
    let removed: Vec<String> = before_items.iter().filter(|x| !after_items.iter().any(|y| same(x, y))).cloned().collect();
    let first = |v: &Vec<String>| v.first().cloned().unwrap_or_default();
    let (title, consequence) = match key {
        "block_actions" => match allowed_word(c) {
            Some(w) => {
                let w_ = title_case(&w);
                typed = Some(w_.clone());
                let gone = unblocked(b, a);
                notes.push(format!("No longer blocked: {}", gone.join(", ")));
                let what = if PAYING.contains(&w.as_str()) {
                    format!("The agent will be able to press {w_} and finish real purchases. Use this only on a test shop with test payment details.")
                } else {
                    format!("The agent will be able to press buttons named \"{w_}\", for example to delete an account.")
                };
                (format!("Allow the {w_} button"), what)
            }
            None if level != Level::Loosen && removed.iter().any(|r| r.starts_with('!')) => {
                let w = removed.iter().find_map(|r| r.strip_prefix('!')).map(title_case).unwrap_or_default();
                (format!("Block the {w} button again"), format!("Tests will not press {w} again."))
            }
            None if level == Level::Loosen => (format!("Stop blocking \"{}\"", first(&removed)), format!("Tests will be able to press buttons with \"{}\" in their name.", first(&removed))),
            None => ("Block a word".into(), "Tests will not press anything that contains this word.".into()),
        },
        "confirm_actions" if level == Level::Loosen => (
            format!("Stop asking before \"{}\"", first(&removed)),
            format!("Tests will press buttons named like \"{}\" without asking first.", first(&removed)),
        ),
        "confirm_actions" => (format!("Ask before pressing \"{}\"", first(&added)), "A test presses such a button only when its steps say so.".into()),
        "allowed_domains" if level == Level::Loosen && after_items.is_empty() => {
            typed = Some("any".into());
            ("Allow any site".into(), "Tests will be able to open every website, not only the ones on the list.".into())
        }
        "allowed_domains" if level == Level::Loosen => {
            let h = first(&added);
            typed = Some(h.clone());
            if h.contains("xn--") {
                notes.push(format!("{h} is an international site name written in its ASCII form; check it is the site you mean."));
            }
            (format!("Allow {h}"), format!("Tests will be able to open {h}."))
        }
        "allowed_domains" => ("Only open these sites".into(), "Tests will not open other sites; a test that goes elsewhere fails.".into()),
        "upload_dirs" if level == Level::Loosen && after_items.is_empty() => {
            typed = Some("any".into());
            ("Allow uploads from any folder".into(), "Tests will be able to upload any file on this computer to the tested site.".into())
        }
        "upload_dirs" if level == Level::Loosen => {
            let d = first(&added);
            let name = d.trim_end_matches('/').rsplit('/').next().unwrap_or(&d).to_owned();
            typed = Some(name);
            (format!("Allow uploads from {d}"), format!("Tests will be able to upload files from {d} to the tested site."))
        }
        "upload_dirs" => ("Limit uploads to these folders".into(), "Tests will upload files only from these folders.".into()),
        "vars" if level == Level::Loosen => {
            let n = first(&added);
            typed = Some(n.clone());
            let exists = if std::env::var_os(&n).is_some() { "yes" } else { "no (set it where fastpilot runs, in Settings › Secrets)" };
            notes.push(format!("Exists in this computer's environment: {exists}"));
            if credential_like(&n) {
                notes.push(format!("{n} looks like a key or token: tests could type it into any page they open."));
            }
            (format!("Let tests use the secret {n}"), format!("Tests will be able to type its value as ${{{n}}}. The value is never shown in chat or reports."))
        }
        "vars" => (format!("Stop tests using {}", first(&removed)), "Tests that type it will fail.".into()),
        k if k.starts_with("allow_") => {
            let what = match k {
                "allow_app_install" => "install and remove apps",
                "allow_app_reset" => "wipe app data",
                "allow_physical_devices" => "use real phones",
                "allow_device_settings" => "change device settings (network, location, permissions, language)",
                "allow_eval_script" => "run their own JavaScript on tested pages; a script can press buttons that are blocked",
                _ => "take over",
            };
            if level == Level::Loosen {
                (format!("Allow tests to {}", what.split(';').next().unwrap_or(what)), format!("Tests will be able to {what}."))
            } else {
                (format!("Turn off {}", label(k)), format!("Tests will not be able to {}.", what.split(';').next().unwrap_or(what)))
            }
        }
        "max_steps" | "test_timeout" | "state_expire_days" => {
            if level == Level::Loosen {
                (format!("Raise {}", label(key)), "Tests may run longer or keep a login longer before fastpilot stops them.".into())
            } else {
                (format!("Lower {}", label(key)), "A test that needs more is stopped and fails.".into())
            }
        }
        k if MODELS.contains(&k) => ("Change the AI model".into(), format!("Used for {} from the next run.", label(k).trim_start_matches("the AI model for ").trim_start_matches("the AI model at "))),
        _ => (format!("Change {}", label(key)), "Used from the next run.".into()),
    };
    let (now, after) = match (key, allowed_word(c)) {
        ("block_actions", Some(w)) => (format!("{} is blocked", title_case(&w)), format!("{} is allowed", title_case(&w))),
        ("vars", _) if level == Level::Loosen => (format!("Tests cannot read {}", first(&added)), format!("Tests type it as ${{{}}}", first(&added))),
        _ => (shown(key, b), shown(key, a)),
    };
    Card { title, now, after, consequence, level, typed, notes }
}

// ---- M1: provenance ---------------------------------------------------------------------------

const VERBS: &[&str] = &[
    "allow", "unblock", "enable", "permit", "let", "turn on", "switch on", "stop blocking", "don't block", "do not block", "remove", "raise",
    "increase", "add", "use", "stop asking", "izinkan", "bolehkan", "aktifkan", "buka", "hapus", "naikkan", "tambah", "pakai",
];

/// Whether the user asked for this loosening in their own message (M1): the new value and its topic
/// appear in it, as words, after normalising, together with a word that asks for it ("allow",
/// "let", ...). "Why is Pay blocked?" names Pay but asks for nothing, so it does not count.
pub fn provenance(c: &Change, message: &str) -> bool {
    let m = format!(" {} ", policy::normalize(message).replace(|ch: char| !ch.is_alphanumeric() && !"._-/~$".contains(ch), " "));
    let m = m.split_whitespace().collect::<Vec<_>>().join(" ");
    let m = format!(" {m} ");
    let has = |w: &str| {
        let w = policy::normalize(w);
        !w.is_empty() && m.contains(&format!(" {w} "))
    };
    let asks = VERBS.iter().any(|v| has(v));
    if !asks {
        return false;
    }
    let before = items(&c.key, c.before.as_deref());
    let after = items(&c.key, c.after.as_deref());
    let added: Vec<String> = after.iter().filter(|x| !before.iter().any(|y| same(x, y))).cloned().collect();
    let removed: Vec<String> = before.iter().filter(|x| !after.iter().any(|y| same(x, y))).cloned().collect();
    let any_of = |words: &[&str]| words.iter().any(|w| has(w));
    match c.key.as_str() {
        "block_actions" => match allowed_word(c) {
            Some(w) => has(&w),
            None => removed.iter().all(|w| has(w)),
        },
        "confirm_actions" => removed.iter().all(|w| has(w)),
        "allowed_domains" if after.is_empty() => any_of(&["any", "all", "every"]) && any_of(&["site", "sites", "website", "websites", "domain", "domains"]),
        "allowed_domains" => added.iter().all(|h| has(h.trim_start_matches("*."))),
        "upload_dirs" if after.is_empty() => any_of(&["any", "all", "every"]) && any_of(&["folder", "folders", "file", "files"]),
        "upload_dirs" => added.iter().all(|d| has(d.trim_end_matches('/'))),
        "vars" => added.iter().all(|n| has(n)) && any_of(&["secret", "secrets", "variable", "var", "env", "environment", "password", "token", "key"]),
        "allow_app_install" => any_of(&["install", "installing", "uninstall"]),
        "allow_app_reset" => any_of(&["reset", "wipe", "clear", "clearstate"]),
        "allow_physical_devices" => any_of(&["physical", "real phone", "real phones", "real device", "real devices", "phone", "phones"]),
        "allow_device_settings" => any_of(&["device settings", "settings", "network", "location", "permissions", "language", "locale"]),
        "allow_eval_script" => any_of(&["javascript", "script", "scripts", "evalscript", "js"]),
        "max_steps" => c.after.as_deref().is_some_and(has) && any_of(&["step", "steps"]),
        "test_timeout" => (c.after.as_deref().is_some_and(has) || any_of(&["no limit", "unlimited"])) && any_of(&["minute", "minutes", "time", "timeout"]),
        "state_expire_days" => c.after.as_deref().is_some_and(has) && any_of(&["day", "days", "login", "logins"]),
        _ => false,
    }
}

// ---- files: pending requests (M13) and the audit log (M14) ------------------------------------

fn dir() -> std::path::PathBuf {
    config::path().parent().map(std::path::Path::to_path_buf).unwrap_or_else(std::env::temp_dir)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Random hex (`bytes` from the OS): card and request ids nobody can guess (M5).
pub fn random_id(bytes: usize) -> String {
    use std::io::Read;
    let mut buf = vec![0u8; bytes];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).is_err() {
        // No /dev/urandom: time and pid, still not sequential.
        let seed = format!("{}{}", now_ms(), std::process::id());
        buf = seed.bytes().cycle().take(bytes).collect();
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_private(path: &std::path::Path, text: &str, append: bool) -> Result<(), String> {
    use std::io::Write;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    }
    let mut o = std::fs::OpenOptions::new();
    if append {
        o.append(true).create(true);
    } else {
        o.write(true).create(true).truncate(true);
    }
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path).and_then(|mut f| f.write_all(text.as_bytes())).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// A request the AI filed through the `config` tool for the user to approve (M13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub id: String,
    pub key: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub created_ms: u64,
}

pub fn pending_path() -> std::path::PathBuf {
    dir().join("pending-approvals.json")
}

/// The pending requests, read as untrusted input: a file too big, unreadable entries, unknown or
/// refused settings, changes that do not loosen anything and expired requests are all dropped.
pub fn load_pending() -> Vec<Pending> {
    let path = pending_path();
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 64 * 1024 {
        return vec![];
    }
    let Ok(text) = std::fs::read_to_string(&path) else { return vec![] };
    let list: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
    let now = now_ms();
    let mut out: Vec<Pending> = Vec::new();
    for v in list {
        let Ok(p) = serde_json::from_value::<Pending>(v) else { continue };
        let short = |s: &Option<String>| s.as_ref().is_none_or(|s| s.len() <= 2000);
        let ok = p.id.len() <= 64
            && p.id.chars().all(|c| c.is_ascii_hexdigit())
            && short(&p.before)
            && short(&p.after)
            && now.saturating_sub(p.created_ms) < EXPIRY_MS
            && p.created_ms <= now + 60_000;
        let c = Change { key: p.key.clone(), before: p.before.clone(), after: p.after.clone() };
        if ok && check(&c).is_ok() && level(&c) == Level::Loosen && !out.iter().any(|x| x.key == p.key && x.after == p.after) {
            out.push(p);
        }
    }
    out.truncate(MAX_PENDING);
    out
}

fn save_pending(list: &[Pending]) -> Result<(), String> {
    write_private(&pending_path(), &serde_json::to_string_pretty(list).map_err(|e| e.to_string())?, false)
}

/// File a request (only a loosening that may be asked for at all). Returns its card.
pub fn add_pending(c: &Change) -> Result<Card, String> {
    check(c)?;
    if level(c) != Level::Loosen {
        return Err("only a change that loosens a limit waits for approval".into());
    }
    let mut list = load_pending();
    if !list.iter().any(|p| p.key == c.key && p.after == c.after) {
        if list.len() >= MAX_PENDING {
            return Err(format!("{MAX_PENDING} requests are already waiting for the user; nothing more is saved until they decide"));
        }
        list.push(Pending { id: random_id(8), key: c.key.clone(), before: c.before.clone(), after: c.after.clone(), created_ms: now_ms() });
        save_pending(&list)?;
    }
    audit(&c.key, c.before.as_deref(), c.after.as_deref(), "terminal", "requested");
    Ok(card(c))
}

pub fn remove_pending(id: &str) -> Result<(), String> {
    let list: Vec<Pending> = load_pending().into_iter().filter(|p| p.id != id).collect();
    save_pending(&list)
}

pub fn audit_path() -> std::path::PathBuf {
    dir().join("audit.jsonl")
}

/// One line in the activity log (M14): what changed and what happened, never a secret's value or
/// the AI's words. Values are setting values (secret *names* for `vars`).
pub fn audit(key: &str, before: Option<&str>, after: Option<&str>, channel: &str, result: &str) {
    let line = serde_json::json!({ "time_ms": now_ms(), "setting": key, "before": before, "after": after, "channel": channel, "result": result });
    if let Err(e) = write_private(&audit_path(), &format!("{line}\n"), true) {
        tracing::warn!(error = %e, "the activity log could not be written");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(key: &str, before: Option<&str>, after: Option<&str>) -> Change {
        Change { key: key.into(), before: before.map(Into::into), after: after.map(Into::into) }
    }

    #[test]
    fn allowing_pay_changes_exactly_pay() {
        let after = apply("block_actions", None, &Op::Remove("Pay".into())).expect("allow");
        assert_eq!(after.as_deref(), Some("default,!pay"));
        let c = ch("block_actions", None, after.as_deref());
        assert_eq!(level(&c), Level::Loosen);
        // M9: only Pay is unblocked; every other built-in rule (phrases included) stays.
        let gone = unblocked(None, after.as_deref());
        assert_eq!(gone, vec!["pay".to_owned(), "pay now".to_owned()]);
        let card = card(&c);
        assert_eq!((card.title.as_str(), card.now.as_str(), card.after.as_str(), card.typed.as_deref()), ("Allow the Pay button", "Pay is blocked", "Pay is allowed", Some("Pay")));
        assert!(card.consequence.contains("finish real purchases"));
        // Blocked again: a tightening that applies at once.
        let again = apply("block_actions", after.as_deref(), &Op::Add("pay".into())).expect("again");
        assert_eq!(again.as_deref(), Some("default"));
        assert_eq!(level(&ch("block_actions", after.as_deref(), again.as_deref())), Level::Applied);
        // `!` only for built-in words.
        assert!(config::invalid("block_actions", "default,!refund").is_some());
    }

    #[test]
    fn auto_apply_is_bounded() {
        let add = |w: &str| ch("block_actions", None, apply("block_actions", None, &Op::Add(w.into())).expect("add").as_deref());
        assert_eq!(level(&add("refund")), Level::Applied);
        assert!(check(&add("a")).unwrap_err().contains("3 to 40"));
        assert!(check(&add(&"x".repeat(41))).is_err());
        assert_eq!(level(&ch("allow_eval_script", None, Some("false"))), Level::Applied);
        // Tightenings that can break or redirect tests need a click.
        assert_eq!(level(&ch("allowed_domains", None, Some("shop.example.com"))), Level::Ok);
        assert_eq!(level(&ch("max_steps", None, Some("20"))), Level::Ok);
        assert_eq!(level(&ch("model_replay", None, Some("claude-haiku-5-5"))), Level::Ok);
        assert_eq!(level(&ch("max_steps", Some("20"), Some("500"))), Level::Loosen);
    }

    #[test]
    fn some_changes_never_go_through_chat() {
        for (k, v) in [("provider", "ollama"), ("provider_base_url", "https://x.example"), ("chrome", "/bin/sh"), ("adb_server", "1.2.3.4:5"), ("chrome_endpoint", "ws://x"), ("dir", "/tmp"), ("headful", "true")] {
            assert!(check(&ch(k, None, Some(v))).is_err(), "{k}");
        }
        assert!(check(&ch("allow_takeover", Some("false"), Some("true"))).is_err());
        assert!(check(&ch("allow_takeover", None, Some("false"))).is_ok());
        assert!(check(&ch("model", None, Some("gpt-4o"))).is_err());
        // M11
        assert!(check(&ch("allowed_domains", Some("shop.example.com"), Some("shop.example.com,*.com"))).is_err());
        assert!(check(&ch("allowed_domains", Some("shop.example.com"), Some("shop.example.com,*.staging.example.com"))).is_ok());
        let home = home();
        for d in ["/", home.as_str(), "~/.ssh", "~/Library/Keys", "/Users/x/../y"] {
            assert!(check(&ch("upload_dirs", Some("/tmp/a"), Some(&format!("/tmp/a,{d}")))).is_err(), "{d}");
        }
        assert!(check(&ch("upload_dirs", Some("/tmp/a"), Some("/tmp/a,/tmp/fixtures"))).is_ok());
    }

    #[test]
    fn provenance_needs_the_users_own_words() {
        let pay = ch("block_actions", None, Some("default,!pay"));
        assert!(provenance(&pay, "Allow the Pay button for this project"));
        assert!(provenance(&pay, "tolong izinkan tombol Pay"));
        assert!(!provenance(&pay, "why is Pay blocked?"));
        assert!(!provenance(&pay, "allow the checkout tests to run"));
        let host = ch("allowed_domains", Some("shop.example.com"), Some("shop.example.com,evil.example"));
        assert!(!provenance(&host, "allow example.com"));
        assert!(provenance(&host, "please allow evil.example too"));
        let js = ch("allow_eval_script", None, Some("true"));
        assert!(provenance(&js, "turn on JavaScript"));
        assert!(!provenance(&js, "what is JavaScript?"));
        let var = ch("vars", None, Some("GITHUB_TOKEN"));
        assert!(provenance(&var, "add the secret GITHUB_TOKEN"));
        assert!(!provenance(&var, "add GITHUB_TOKEN"));
        let card = card(&var);
        assert_eq!(card.typed.as_deref(), Some("GITHUB_TOKEN"));
        assert!(card.notes.iter().any(|n| n.contains("looks like a key or token")), "{card:?}");
    }

    /// Security test 7 (probe property): whatever the chat or the AI asks, a change that is not a
    /// loosening never unblocks a built-in phrase, and allowing one built-in word unblocks only that
    /// word's probes.
    #[test]
    fn no_accepted_change_unblocks_more_than_it_says() {
        let starts: Vec<Option<String>> = vec![None, Some("default".into()), Some("default,refund".into()), Some("default,!buy".into())];
        let words = ["refund", "keluar", "pay", "buy", "delete account", "place order", "hapus akun", "transfer"];
        let mut seen = 0;
        for before in &starts {
            for w in words {
                for op in [Op::Add(w.into()), Op::Remove(w.into())] {
                    let Ok(after) = apply("block_actions", before.as_deref(), &op) else { continue };
                    let c = Change { key: "block_actions".into(), before: before.clone(), after: after.clone() };
                    if check(&c).is_err() {
                        continue;
                    }
                    seen += 1;
                    let gone = unblocked(before.as_deref(), after.as_deref());
                    match level(&c) {
                        Level::Loosen => match allowed_word(&c) {
                            Some(a) => assert!(!gone.is_empty() && gone.iter().all(|p| policy::block_match(p, &[format!(" {a} ")]).is_some()), "{c:?}: {gone:?}"),
                            None => assert!(gone.is_empty(), "{c:?}: {gone:?}"),
                        },
                        _ => assert!(gone.is_empty(), "a stricter change unblocked {gone:?}: {c:?}"),
                    }
                }
            }
        }
        assert!(seen > 30, "{seen}");
    }

    #[test]
    fn chat_never_drops_the_built_in_list_or_adds_an_allow_word() {
        // Security D9 finding 1: "!pay" is not a word to add (block or confirm list).
        for key in ["block_actions", "confirm_actions"] {
            assert!(apply(key, None, &Op::Add("!pay".into())).is_err(), "{key}");
            assert!(apply(key, None, &Op::Add("default".into())).is_err(), "{key}");
            assert!(apply(key, None, &Op::Add("refund,!pay".into())).is_err(), "{key}: a comma makes two items");
        }
        // Finding 2: removing `default` is refused, through apply and through check (a pending
        // request carries before/after only).
        assert!(apply("block_actions", None, &Op::Remove("default".into())).is_err());
        assert!(apply("block_actions", Some("default,refund"), &Op::Remove("DEFAULT".into())).is_err());
        for (before, after) in [(None, Some("refund")), (Some("default,!pay"), Some("!pay")), (Some("default,refund"), Some("refund"))] {
            let c = ch("block_actions", before, after);
            let e = check(&c).expect_err(&format!("{before:?} -> {after:?}"));
            assert!(e.contains("only be switched off by you, in Settings"), "{e}");
        }
        // Only the canonical allow word of a built-in word passes, and only with the list kept.
        for after in ["default,!PAY", "default,!Pay", "default,!refund", "default,!payment-x", "!pay"] {
            assert!(check(&ch("block_actions", Some("default"), Some(after))).is_err(), "{after}");
        }
        assert!(check(&ch("block_actions", None, Some("default,!pay"))).is_ok());
        // "Allow Pay" (remove) still works and still needs the typed word.
        let after = apply("block_actions", None, &Op::Remove("PAY".into())).expect("allow");
        assert_eq!(after.as_deref(), Some("default,!pay"));
        let c = ch("block_actions", None, after.as_deref());
        check(&c).expect("allow pay");
        let k = card(&c);
        assert_eq!(k.typed.as_deref(), Some("Pay"));
        assert!(k.consequence.contains("finish real purchases"), "{}", k.consequence);
        // A user who already switched the list off in Settings can still block a word from chat.
        assert!(check(&ch("block_actions", Some("refund"), Some("refund,wipe"))).is_ok());
    }
}
