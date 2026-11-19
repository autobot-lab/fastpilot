//! Guard rails for an agent acting on real systems (opt-in, read from the environment):
//!
//! - `FASTPILOT_ALLOWED_DOMAINS=shop.example.com,*.staging.example.com,localhost`: web sessions
//!   may only load pages from these hosts (openUrl, links, form posts, redirects, popups; other
//!   resources such as CDNs and APIs still load). Local files and `about:`/`data:` pages stay
//!   allowed.
//! - `FASTPILOT_CONFIRM_ACTIONS=default` (or a comma list of words / phrases): a tap, long press or
//!   double tap on an element whose name contains one of them ("Hapus", "Bayar", "Delete
//!   account", ...) fails with `blocked` unless the step says `confirm: true`.

//! - `block_actions`: a tap on an element whose text contains one of these words is always refused,
//!   `confirm: true` or not ("Bayar" also blocks "Lanjut ke Pembayaran").
//! - `allow_*` permissions (default on) and `upload_dirs`: see [`crate::config::SETTINGS`].

/// Built-in risky words for `FASTPILOT_CONFIRM_ACTIONS=default` (or `1`).
const DEFAULT_RISKY: &[&str] = &[
    "delete", "hapus", "remove", "pay", "bayar", "purchase", "buy", "beli", "checkout", "transfer", "place order", "buat pesanan",
    "unsubscribe", "deactivate", "nonaktifkan",
];

fn list(var: &str) -> Vec<String> {
    crate::config::var(var)
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Allowed hosts; empty = no restriction.
pub fn allowed_domains() -> Vec<String> {
    list("FASTPILOT_ALLOWED_DOMAINS")
}

/// Whether `url` may be loaded under `allowed` (empty list: everything).
pub fn url_allowed(url: &str, allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true;
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        // about:blank, data:, javascript: — nothing leaves the machine.
        return true;
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https" | "ws" | "wss") {
        return true;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    }
    .to_lowercase();
    allowed.iter().any(|d| match d.strip_prefix("*.") {
        Some(base) => host == base || host.ends_with(&format!(".{base}")),
        None => host == *d,
    })
}

/// Words that always block a tap (`block_actions`).
pub fn blocked_words() -> Vec<String> {
    list("FASTPILOT_BLOCK_ACTIONS")
}

/// The blocked word `name` contains (substring, case-insensitive).
pub fn block_match(name: &str, words: &[String]) -> Option<String> {
    let name = name.to_lowercase();
    words.iter().find(|w| name.contains(w.as_str())).cloned()
}

/// Folders `uploadFile` may take files from (`upload_dirs`); empty = any. `~/` is the home folder.
pub fn upload_dirs() -> Vec<std::path::PathBuf> {
    crate::config::var("FASTPILOT_UPLOAD_DIRS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|d| match d.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default().join(rest),
            None => std::path::PathBuf::from(d),
        })
        .collect()
}

/// Whether `file` lies inside one of `dirs` (both resolved, so `..` and links cannot leave them).
pub fn upload_allowed(file: &std::path::Path, dirs: &[std::path::PathBuf]) -> bool {
    if dirs.is_empty() {
        return true;
    }
    let Ok(f) = file.canonicalize() else { return false };
    dirs.iter().filter_map(|d| d.canonicalize().ok()).any(|d| f.starts_with(d))
}

/// Risky words / phrases; empty = no confirmation needed.
pub fn risky_words() -> Vec<String> {
    let words = list("FASTPILOT_CONFIRM_ACTIONS");
    if words.iter().any(|w| w == "default" || w == "1" || w == "true") {
        let mut all: Vec<String> = DEFAULT_RISKY.iter().map(|w| (*w).to_owned()).collect();
        all.extend(words.into_iter().filter(|w| !matches!(w.as_str(), "default" | "1" | "true")));
        return all;
    }
    words
}

/// The risky word `name` contains, matched as whole words ("pay" matches "Pay now", not "Paypal").
pub fn risky_match(name: &str, words: &[String]) -> Option<String> {
    let tokens: Vec<String> = name.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_owned).collect();
    let joined = format!(" {} ", tokens.join(" "));
    words.iter().find(|w| joined.contains(&format!(" {w} "))).cloned()
}

/// The host of an endpoint (`ws://user:pw@host:9222/x`, `http://[::1]:9222`, `host:5037`) when it
/// is not this machine: Chrome DevTools and adb have no password and no encryption, so anything
/// else should be an SSH tunnel to 127.0.0.1.
pub fn remote_host(endpoint: &str) -> Option<String> {
    let rest = endpoint.split_once("://").map_or(endpoint, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = match host_port.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host_port.rsplit_once(':').map_or(host_port, |(h, _)| h),
    }
    .to_ascii_lowercase();
    let loopback = host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    (!loopback).then_some(host)
}

/// `endpoint` with any `user:password@` replaced by `***@`, for logs and `doctor`.
pub fn redact_userinfo(endpoint: &str) -> String {
    let (scheme, rest) = endpoint.split_once("://").map_or(("", endpoint), |(s, r)| (s, r));
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let authority = match authority.rsplit_once('@') {
        Some((_, host)) => format!("***@{host}"),
        None => authority.to_owned(),
    };
    if scheme.is_empty() { format!("{authority}{tail}") } else { format!("{scheme}://{authority}{tail}") }
}

#[cfg(test)]
mod tests {

    /// Security review (Low): a remote Chrome / adb endpoint gets a warning unless it is this machine.
    #[test]
    fn remote_endpoints_are_told_apart_from_loopback() {
        for local in ["http://127.0.0.1:9222", "ws://localhost:9222/devtools/browser/x", "127.0.0.1:15037", "http://[::1]:9222", "127.1.2.3:5037"] {
            assert_eq!(remote_host(local), None, "{local}");
        }
        assert_eq!(remote_host("ws://evil.example:9222").as_deref(), Some("evil.example"));
        assert_eq!(remote_host("http://u:pw@192.168.1.5:9222/json").as_deref(), Some("192.168.1.5"));
        assert_eq!(remote_host("10.0.0.2:5037").as_deref(), Some("10.0.0.2"));
        assert_eq!(remote_host("http://127.0.0.1.evil.example:9222").as_deref(), Some("127.0.0.1.evil.example"));
        assert_eq!(redact_userinfo("http://u:pw@host:9222/json?x=1"), "http://***@host:9222/json?x=1");
        assert_eq!(redact_userinfo("host:5037"), "host:5037");
    }

    use super::*;

    #[test]
    fn domains() {
        let allowed: Vec<String> = ["shop.example.com", "*.staging.example.com", "localhost"].iter().map(|s| (*s).to_owned()).collect();
        assert!(url_allowed("https://shop.example.com/cart?x=1", &allowed));
        assert!(url_allowed("http://localhost:8080/", &allowed));
        assert!(url_allowed("https://a.b.staging.example.com/", &allowed));
        assert!(url_allowed("https://staging.example.com", &allowed));
        assert!(!url_allowed("https://evil.com/", &allowed));
        assert!(!url_allowed("https://shop.example.com.evil.com/", &allowed));
        assert!(!url_allowed("https://user@evil.com/", &allowed));
        assert!(!url_allowed("http://notlocalhost/", &allowed));
        assert!(url_allowed("file:///tmp/a.html", &allowed) && url_allowed("about:blank", &allowed) && url_allowed("data:text/html,hi", &allowed));
        assert!(url_allowed("https://anything/", &[]));
    }

    #[test]
    fn blocked_words_match_inside_text() {
        let w: Vec<String> = vec!["bayar".into()];
        assert_eq!(block_match("Lanjut ke Pembayaran", &w).as_deref(), Some("bayar"));
        assert_eq!(block_match("BAYAR", &w).as_deref(), Some("bayar"));
        assert_eq!(block_match("Batal", &w), None);
    }

    #[test]
    fn uploads_stay_in_their_folders() {
        let dir = std::env::temp_dir().join(format!("fp-upload-{}", std::process::id()));
        let inside = dir.join("ok");
        std::fs::create_dir_all(&inside).expect("dir");
        std::fs::write(inside.join("a.pdf"), b"x").expect("file");
        std::fs::write(dir.join("secret.txt"), b"x").expect("file");
        let dirs = vec![inside.clone()];
        assert!(upload_allowed(&inside.join("a.pdf"), &dirs));
        assert!(!upload_allowed(&inside.join("../secret.txt"), &dirs), "`..` leaves the folder");
        assert!(!upload_allowed(&dir.join("secret.txt"), &dirs));
        assert!(!upload_allowed(&inside.join("missing.pdf"), &dirs));
        assert!(upload_allowed(&dir.join("secret.txt"), &[]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn risky_names() {
        let w: Vec<String> = ["pay", "hapus", "delete account"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(risky_match("Pay now", &w).as_deref(), Some("pay"));
        assert_eq!(risky_match("Hapus Akun", &w).as_deref(), Some("hapus"));
        assert_eq!(risky_match("Delete account permanently", &w).as_deref(), Some("delete account"));
        assert_eq!(risky_match("Paypal", &w), None);
        assert_eq!(risky_match("Delete", &w), None);
    }
}
