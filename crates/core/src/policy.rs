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
//!   `confirm: true` or not ("Bayar" also blocks "Lanjut ke Pembayaran"). Not set at all (a new
//!   install): the built-in list [`DEFAULT_BLOCKED`] (payment, account deletion), matched as whole
//!   words; `default` in the list stands for it too. Emptied on purpose (`block_actions = []`): off.
//! - `confirm_actions` not set at all: [`DEFAULT_CONFIRM`] (hapus, delete, remove).
//! - `allow_*` permissions (default on) and `upload_dirs`: see [`crate::config::SETTINGS`].

/// Built-in risky words for `FASTPILOT_CONFIRM_ACTIONS=default` (or `1`).
const DEFAULT_RISKY: &[&str] = &[
    "delete", "hapus", "remove", "pay", "bayar", "purchase", "buy", "beli", "checkout", "transfer", "place order", "buat pesanan",
    "unsubscribe", "deactivate", "nonaktifkan",
];

/// Built-in `block_actions` (v0.3.2): used while the key is not set, or for the word `default` in
/// it. Matched as whole words ("pay" blocks "Pay now", not "Paypal").
pub const DEFAULT_BLOCKED: &[&str] =
    &["bayar", "pay", "beli", "buy", "checkout", "purchase", "transfer", "buat pesanan", "place order", "hapus akun", "delete account"];

/// Built-in `confirm_actions` while the key is not set (v0.3.2): plain deletes need `confirm: true`
/// ("Hapus filter" is not blocked, only confirmed).
pub const DEFAULT_CONFIRM: &[&str] = &["hapus", "delete", "remove"];

/// A configured list: `None` when the setting is not set, `Some(empty)` when it was emptied on
/// purpose. Emptied means the config file (`[]`: the user, through `fastpilot ui` or `fastpilot
/// config`): an empty environment variable does not count (Security Low 3: an MCP server entry an
/// agent wrote must not switch the built-in lists off).
fn configured(var: &str) -> Option<Vec<String>> {
    crate::config::var_unless_empty_env(var).ok().map(|v| v.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect())
}

/// Text as the rules compare it: lower case, zero-width characters and soft hyphens removed,
/// fullwidth letters and digits as plain ones, every kind of space as one space (Security Low 4:
/// "Ｐａｙ", "Pa\u{200b}y" and "Pay\u{a0}now" are "pay", "pay now").
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if invisible(c) {
            continue;
        }
        let c = match c {
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
            c if c.is_whitespace() => ' ',
            c => c,
        };
        for l in c.to_lowercase() {
            out.push(look_alike(l));
        }
    }
    out.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Characters that show nothing (or only change how the next one looks): format and bidi
/// controls, joiners, variation selectors, tag characters, Hangul fillers, combining marks
/// ("Pa\u{202e}y", "Pa\u{e0041}y", "Pa\u{301}y" are "pay"). T1-T3: they slipped through phrase matching.
fn invisible(c: char) -> bool {
    matches!(c as u32,
        0x00ad | 0x034f | 0x061c | 0x115f | 0x1160 | 0x17b4 | 0x17b5 | 0x180b..=0x180f | 0x200b..=0x200f
        | 0x202a..=0x202e | 0x2060..=0x206f | 0x3164 | 0xfe00..=0xfe0f | 0xfeff | 0xffa0 | 0x1bca0..=0x1bca3
        | 0xe0000..=0xe0fff
        // combining marks: accents typed as separate characters
        | 0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x20d0..=0x20ff | 0xfe20..=0xfe2f)
}

/// A lower-case letter that looks like a Latin one, as that Latin letter: Cyrillic and Greek
/// look-alikes, accented Latin letters, mathematical and circled letters ("Раy", "Ρay", "pày",
/// "𝐩𝐚𝐲", "ⓟⓐⓨ" are "pay"). T1-T3: homoglyphs slipped through phrase matching.
fn look_alike(c: char) -> char {
    const TABLE: &[(&str, char)] = &[
        ("аàáâãäåāăąɑα", 'a'),
        ("ьвβ", 'b'),
        ("сçćĉċčϲ", 'c'),
        ("ԁďđ", 'd'),
        ("еèéêëēĕėęěε", 'e'),
        ("ĝğġģɡ", 'g'),
        ("һĥħ", 'h'),
        ("іїìíîïĩīĭįıι", 'i'),
        ("јĵ", 'j'),
        ("κķ", 'k'),
        ("ĺļľŀł", 'l'),
        ("ñńņňŉη", 'n'),
        ("оòóôõöøōŏőοσ", 'o'),
        ("рρ", 'p'),
        ("ŕŗř", 'r'),
        ("ѕśŝşšș", 's'),
        ("ţťŧțτ", 't'),
        ("ùúûüũūŭůűųυ", 'u'),
        ("νѵ", 'v'),
        ("ŵω", 'w'),
        ("хχ", 'x'),
        ("уýÿŷ", 'y'),
        ("źżž", 'z'),
    ];
    let n = c as u32;
    // Mathematical alphanumeric letters (bold, italic, script, ...): A-Z a-z repeating.
    if (0x1d400..=0x1d6a3).contains(&n) {
        let i = (n - 0x1d400) % 52;
        return char::from_u32(if i < 26 { 'a' as u32 + i } else { 'a' as u32 + i - 26 }).unwrap_or(c);
    }
    if (0x1d7ce..=0x1d7ff).contains(&n) {
        return char::from_u32('0' as u32 + (n - 0x1d7ce) % 10).unwrap_or(c);
    }
    // Circled letters ⓐ-ⓩ (Ⓐ-Ⓩ lower-case to these first).
    if (0x24d0..=0x24e9).contains(&n) {
        return char::from_u32('a' as u32 + n - 0x24d0).unwrap_or(c);
    }
    TABLE.iter().find(|(set, _)| set.contains(c)).map_or(c, |(_, l)| *l)
}

/// The words of `text` (letters and digits), normalized.
fn tokens(text: &str) -> Vec<String> {
    normalize(text).split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_owned).collect()
}

/// Whether `phrase` occurs in `name` as whole words, in order, with at most `gap` other words
/// between two of its words ("delete account" in "Delete my account" with a gap of 1).
fn phrase_in(name: &[String], phrase: &[String], gap: usize) -> bool {
    let Some(first) = phrase.first() else { return false };
    (0..name.len()).filter(|&i| &name[i] == first).any(|start| {
        let mut at = start;
        phrase[1..].iter().all(|w| match name.iter().enumerate().skip(at + 1).take(gap + 1).find(|(_, t)| *t == w) {
            Some((i, _)) => {
                at = i;
                true
            }
            None => false,
        })
    })
}

/// `block_actions` items as the engine matches them ([`block_rules`]): `default` (or nothing set)
/// becomes the built-in words, wrapped in spaces (` pay `) = whole words only; the user's own
/// words stay as they are and match inside text.
///
/// `!word` (D9, M9) allows one built-in word again and changes nothing else: `default,!pay` keeps
/// every built-in rule except " pay " (and a plain `pay` of the user's own), so allowing Pay can
/// never rewrite the rest of the list into plain words.
pub fn expand_blocked(items: Option<Vec<String>>) -> Vec<String> {
    let items = items.unwrap_or_else(|| vec!["default".to_owned()]);
    let allowed: Vec<String> = items.iter().filter_map(|w| w.strip_prefix('!')).map(|w| w.trim().to_lowercase()).collect();
    let mut out: Vec<String> = Vec::new();
    for w in items.into_iter().filter(|w| !w.starts_with('!')) {
        if w == "default" {
            out.extend(DEFAULT_BLOCKED.iter().filter(|d| !allowed.iter().any(|a| a == *d)).map(|d| format!(" {d} ")));
        } else if !allowed.iter().any(|a| *a == w.trim().to_lowercase()) {
            out.push(w);
        }
    }
    out.dedup();
    out
}

/// Whether `word` is one of the built-in blocked words ([`DEFAULT_BLOCKED`]).
pub fn is_builtin_blocked(word: &str) -> bool {
    let w = normalize(word);
    DEFAULT_BLOCKED.iter().any(|d| *d == w)
}

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

/// Words that always block a tap (`block_actions`), as people read them (built-in ones included).
pub fn blocked_words() -> Vec<String> {
    block_rules().into_iter().map(|w| w.trim().to_owned()).collect()
}

/// [`blocked_words`] as [`block_match`] takes them: built-in ones wrapped in spaces (whole words).
pub fn block_rules() -> Vec<String> {
    expand_blocked(configured("FASTPILOT_BLOCK_ACTIONS"))
}

/// Option C (v0.3.2): while any word is blocked, `evalScript` is off unless the user switched it on
/// on purpose (`allow_eval_script = true`, in the file or the environment); a script could press a
/// blocked button. An agent cannot set it (see `config::tightens`).
pub fn eval_unlocked() -> bool {
    crate::config::var("FASTPILOT_ALLOW_EVAL_SCRIPT").is_ok_and(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

/// A blocked word found in an element's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockHit {
    pub word: String,
    /// From the built-in list (not set, or `default`).
    pub builtin: bool,
}

/// The blocked word `name` contains, case-insensitive and normalized ([`normalize`]): the user's
/// words anywhere in the text, the built-in ones as whole words, a phrase with up to one word
/// between its words ("Delete my account", "Place your order").
pub fn block_match(name: &str, words: &[String]) -> Option<BlockHit> {
    let text = normalize(name);
    let name_tokens = tokens(name);
    words.iter().find_map(|w| {
        let whole = w.len() > 2 && w.starts_with(' ') && w.ends_with(' ');
        let hit = if whole { phrase_in(&name_tokens, &tokens(w), 1) } else { !w.trim().is_empty() && text.contains(normalize(w).as_str()) };
        hit.then(|| BlockHit { word: w.trim().to_owned(), builtin: whole })
    })
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

/// Risky words / phrases; empty = no confirmation needed; not set = [`DEFAULT_CONFIRM`].
pub fn risky_words() -> Vec<String> {
    expand_risky(configured("FASTPILOT_CONFIRM_ACTIONS"))
}

/// `confirm_actions` items as the engine matches them (`None`: not set).
pub fn expand_risky(words: Option<Vec<String>>) -> Vec<String> {
    let Some(words) = words else {
        return DEFAULT_CONFIRM.iter().map(|w| (*w).to_owned()).collect();
    };
    if words.iter().any(|w| w == "default" || w == "1" || w == "true") {
        let mut all: Vec<String> = DEFAULT_RISKY.iter().map(|w| (*w).to_owned()).collect();
        all.extend(words.into_iter().filter(|w| !matches!(w.as_str(), "default" | "1" | "true")));
        return all;
    }
    words
}

/// The risky word `name` contains, matched as whole words ("pay" matches "Pay now", not "Paypal"),
/// after [`normalize`].
pub fn risky_match(name: &str, words: &[String]) -> Option<String> {
    let name = tokens(name);
    words.iter().find(|w| phrase_in(&name, &tokens(w), 0)).cloned()
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

/// `text` (an error about `endpoint`) with the endpoint's `user:password@` replaced by `***@`.
pub fn redact_userinfo_in(text: &str, endpoint: &str) -> String {
    let rest = endpoint.split_once("://").map_or(endpoint, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    match authority.rsplit_once('@') {
        Some((userinfo, _)) if !userinfo.is_empty() => text.replace(&format!("{userinfo}@"), "***@"),
        _ => text.to_owned(),
    }
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
        assert_eq!(redact_userinfo_in("CDP websocket: u:pw@host:9222/json: refused", "http://u:pw@host:9222"), "CDP websocket: ***@host:9222/json: refused");
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
        let word = |n: &str, w: &[String]| block_match(n, w).map(|h| h.word);
        assert_eq!(word("Lanjut ke Pembayaran", &w).as_deref(), Some("bayar"));
        assert_eq!(word("BAYAR", &w).as_deref(), Some("bayar"));
        assert_eq!(word("Batal", &w), None);
    }

    /// F5 (v0.3.2): built-in words match whole words only, and say they are built in; the user's
    /// own words still match inside text.
    #[test]
    fn built_in_block_words_match_whole_words() {
        let all = expand_blocked(None);
        assert_eq!(all.len(), DEFAULT_BLOCKED.len());
        let hit = block_match("Bayar sekarang", &all).expect("bayar");
        assert_eq!(hit, BlockHit { word: "bayar".into(), builtin: true });
        for (name, word) in [("Pay now", "pay"), ("CHECKOUT", "checkout"), ("Buat Pesanan", "buat pesanan"), ("Hapus akun saya", "hapus akun"), ("Delete account", "delete account")] {
            assert_eq!(block_match(name, &all).map(|h| h.word).as_deref(), Some(word), "{name}");
        }
        // Security Low 4: hidden characters, fullwidth letters, odd spaces, a word in between.
        for (name, word) in [
            ("Pa\u{200b}y now", "pay"),
            ("Ｐａｙ ｎｏｗ", "pay"),
            ("Bayar\u{a0}sekarang", "bayar"),
            ("CHECK\u{00ad}OUT", "checkout"),
            ("Delete my account", "delete account"),
            ("Hapus  akun   saya", "hapus akun"),
            ("Place your order", "place order"),
        ] {
            assert_eq!(block_match(name, &all).map(|h| h.word).as_deref(), Some(word), "{name:?}");
        }
        assert_eq!(block_match("Delete this old account", &all), None, "two words between: not the phrase");
        // T1-T3: invisible characters and look-alike letters do not hide a phrase either.
        for name in [
            "Pa\u{202e}y now",            // bidi override
            "Pa\u{e0041}y now",           // tag character
            "Pay\u{fe0f} now",            // variation selector
            "Pa\u{301}y now",             // combining accent
            "P\u{3164}ay now",            // Hangul filler
            "Pаy now",                    // Cyrillic а
            "РАY NOW",                    // Cyrillic Р and А
            "Ρay now",                    // Greek capital rho
            "Pày now",                    // accented a
            "𝐏𝐚𝐲 now",                    // mathematical bold
            "ⓅⓐⓎ now",                    // circled
            "Dеlete my аccount",          // Cyrillic е and а in a phrase
        ] {
            assert!(block_match(name, &all).is_some(), "{name:?} must be blocked");
        }
        assert_eq!(normalize("Ⓒhеckout"), "checkout");
        assert_eq!(block_match("Masuk", &all), None, "plain words stay plain");
        assert_eq!(normalize(" Ｈｅｌｌｏ\u{3000}\u{200b}World "), "hello world");
        for name in ["Paypal", "Lanjut ke Pembayaran", "Hapus filter", "Remove from cart", "Transferan", "Masuk"] {
            assert_eq!(block_match(name, &all), None, "{name}");
        }
        // `default` next to the user's own words: both kinds.
        let mixed = expand_blocked(Some(vec!["default".into(), "keluar".into()]));
        assert_eq!(block_match("Keluarkan", &mixed), Some(BlockHit { word: "keluar".into(), builtin: false }));
        assert!(block_match("Kelu\u{200b}arkan", &mixed).is_some(), "the user's words are normalized too");
        assert!(block_match("Pay now", &mixed).is_some_and(|h| h.builtin));
        // Emptied on purpose: nothing is blocked.
        assert!(expand_blocked(Some(vec![])).is_empty());
        assert_eq!(expand_risky(None), ["hapus", "delete", "remove"]);
        assert!(expand_risky(Some(vec![])).is_empty());
        assert!(risky_match("Hapus filter", &expand_risky(None)).is_some());
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
