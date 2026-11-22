//! E3 (v0.7): browsers the person chose to remember for the dashboard.
//!
//! - The browser keeps a random key in the dashboard page's `localStorage` (bound to this exact
//!   origin, port included). A cookie would not be: cookies on 127.0.0.1 go to every port
//!   (Security, v0.7 review). The key is traded for a short session cookie (`ui.rs`).
//! - This file keeps only the SHA-256 of each key, a short label ("Chrome on this Mac") and two times,
//!   in `~/.fastpilot/ui/browsers.json` (folder 0700, file 0600). Never the key.
//! - A key works for 30 days after its last use and never longer than 90 days after it was made.
//!   Forget and Forget all remove the entries; the next visit needs a new link.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Unused this long, a remembered browser is forgotten.
pub const IDLE_MS: u64 = 30 * 86_400_000;
/// However often it is used, a remembered browser is forgotten this long after it was remembered.
pub const MAX_MS: u64 = 90 * 86_400_000;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Browser {
    pub id: String,
    pub label: String,
    /// SHA-256 of the key, hex.
    hash: String,
    pub created_ms: u64,
    pub last_ms: u64,
}

impl Browser {
    /// When it stops working without a new link: 30 days after the last use, at most 90 after it
    /// was remembered.
    pub fn until_ms(&self) -> u64 {
        (self.last_ms + IDLE_MS).min(self.created_ms + MAX_MS)
    }
    fn alive(&self, now: u64) -> bool {
        now < self.until_ms()
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Store {
    browsers: Vec<Browser>,
}

/// `~/.fastpilot/ui`: the dashboard's own state for this user (not a project's).
pub fn dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join(".fastpilot").join("ui")
}

fn path() -> PathBuf {
    dir().join("browsers.json")
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn hash(key: &str) -> String {
    ring::digest::digest(&ring::digest::SHA256, key.as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn load() -> Store {
    std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Owner-only folder and file, written whole (a temp file renamed into place).
fn save(s: &Store) -> Result<(), String> {
    let d = dir();
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(&d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = d.join(format!(".browsers.{}.tmp", std::process::id()));
    {
        use std::io::Write;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&tmp).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        f.write_all(serde_json::to_string_pretty(s).unwrap_or_default().as_bytes()).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path()).map_err(|e| e.to_string())
}

/// The remembered browsers that still work, newest first; expired ones are dropped from the file.
pub fn list() -> Vec<Browser> {
    let now = now_ms();
    let mut s = load();
    let before = s.browsers.len();
    s.browsers.retain(|b| b.alive(now));
    if s.browsers.len() != before {
        let _ = save(&s);
    }
    s.browsers.sort_by_key(|b| std::cmp::Reverse(b.created_ms));
    s.browsers
}

/// Remember a browser: a new random key (returned once, to be kept by that browser only) and its
/// entry. `key` comes from the caller so it can use the same random source as the session.
pub fn remember(key: &str, label: &str, id: &str) -> Result<Browser, String> {
    let now = now_ms();
    let mut s = load();
    s.browsers.retain(|b| b.alive(now));
    let b = Browser { id: id.to_owned(), label: label.chars().take(60).collect(), hash: hash(key), created_ms: now, last_ms: now };
    s.browsers.push(b.clone());
    save(&s)?;
    Ok(b)
}

/// The browser this key belongs to, if it is remembered and not expired; its last use moves to now.
pub fn unlock(key: &str) -> Option<Browser> {
    if key.len() != 64 || !key.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let now = now_ms();
    let h = hash(key);
    let mut s = load();
    s.browsers.retain(|b| b.alive(now));
    let found = s.browsers.iter_mut().find(|b| crate::ui::same(&b.hash, &h)).map(|b| {
        b.last_ms = now;
        b.clone()
    });
    let _ = save(&s);
    found
}

/// Forget one browser, or all of them (`None`). Returns the ids that were removed.
pub fn forget(id: Option<&str>) -> Result<Vec<String>, String> {
    let mut s = load();
    let gone: Vec<String> = s.browsers.iter().filter(|b| id.is_none_or(|i| b.id == i)).map(|b| b.id.clone()).collect();
    s.browsers.retain(|b| !gone.contains(&b.id));
    save(&s)?;
    Ok(gone)
}

/// "Chrome on this Mac": the browser and system from the User-Agent, nothing else of it is kept.
pub fn label(user_agent: &str) -> String {
    let browser = if user_agent.contains("Edg/") {
        "Edge"
    } else if user_agent.contains("Firefox/") {
        "Firefox"
    } else if user_agent.contains("Chrome/") || user_agent.contains("Chromium/") {
        "Chrome"
    } else if user_agent.contains("Safari/") {
        "Safari"
    } else {
        "A browser"
    };
    let os = if user_agent.contains("Mac OS X") || user_agent.contains("Macintosh") {
        "this Mac"
    } else if user_agent.contains("Linux") {
        "Linux"
    } else if user_agent.contains("Windows") {
        "Windows"
    } else {
        "this computer"
    };
    format!("{browser} on {os}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remembered_browser_ends_after_30_idle_days_or_90_days() {
        let b = Browser { id: "a".into(), label: "x".into(), hash: String::new(), created_ms: 1_000, last_ms: 1_000 };
        assert_eq!(b.until_ms(), 1_000 + IDLE_MS);
        let used = Browser { last_ms: 1_000 + 80 * 86_400_000, ..b.clone() };
        assert_eq!(used.until_ms(), 1_000 + MAX_MS, "used on day 80: still ends on day 90");
        assert!(!used.alive(1_000 + MAX_MS) && used.alive(1_000 + MAX_MS - 1));
    }

    #[test]
    fn labels_say_only_the_browser_and_system() {
        assert_eq!(label("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36"), "Chrome on this Mac");
        assert_eq!(label("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_1) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.1 Safari/605.1.15"), "Safari on this Mac");
        assert_eq!(label("Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0"), "Firefox on Linux");
        assert_eq!(label(""), "A browser on this computer");
    }
}
