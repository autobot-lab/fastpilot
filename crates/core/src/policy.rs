//! Guard rails for an agent acting on real systems (opt-in, read from the environment):
//!
//! - `FASTPILOT_ALLOWED_DOMAINS=shop.example.com,*.staging.example.com,localhost`: web sessions
//!   may only load pages from these hosts (openUrl, links, form posts, redirects, popups; other
//!   resources such as CDNs and APIs still load). Local files and `about:`/`data:` pages stay
//!   allowed.
//! - `FASTPILOT_CONFIRM_ACTIONS=default` (or a comma list of words / phrases): a tap, long press or
//!   double tap on an element whose name contains one of them ("Hapus", "Bayar", "Delete
//!   account", ...) fails with `blocked` unless the step says `confirm: true`.

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

#[cfg(test)]
mod tests {
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
    fn risky_names() {
        let w: Vec<String> = ["pay", "hapus", "delete account"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(risky_match("Pay now", &w).as_deref(), Some("pay"));
        assert_eq!(risky_match("Hapus Akun", &w).as_deref(), Some("hapus"));
        assert_eq!(risky_match("Delete account permanently", &w).as_deref(), Some("delete account"));
        assert_eq!(risky_match("Paypal", &w), None);
        assert_eq!(risky_match("Delete", &w), None);
    }
}
