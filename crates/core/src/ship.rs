//! R2 Ship check (v0.4): the health of one web page before a release, with honest "not checked".
//!
//! `- shipCheck` (or `{ links: 20 }`) on a web session looks at the page that is open now:
//! - **Console errors**: exceptions and `console.error` the page logged since the last report;
//! - **Failed requests**: HTTP errors and requests that did not finish;
//! - **Broken links**: up to `links` links to the page's own site, asked with `HEAD` from the page,
//!   five at a time, redirects not followed, without cookies unless `cookies: true` (servers treat
//!   HEAD like GET). The page only lists its links: which are asked is decided here. Links to other
//!   sites are counted, not opened. Links whose text or address names a risky action (the block and
//!   confirm lists, sign out, delete, unsubscribe, ...) are never requested; signed in, neither are
//!   links with a query string or a word for a change;
//! - **Layout** at three screen sizes (phone 390, tablet 768, desktop 1280): content wider than
//!   the screen. The size is put back afterwards;
//! - **Accessibility basics**: page language and title, images without `alt`, fields, buttons
//!   and links without a name, repeated ids. Colour contrast and keyboard use: not checked.
//!
//! It only reads the page and asks `HEAD` of same-site links; it never clicks, types or submits.
//! Every text from the page is one line and clipped in the report ([`clean`]).
//! The verdict follows the release-check skill: a problem with the page itself (it did not load,
//! or its own script crashed) is DO NOT SHIP; other problems are SHIP WITH FIXES; a check that
//! could not run makes it INCONCLUSIVE; nothing missing and nothing wrong is SHIP. Missing
//! evidence is never a pass.

use serde::Serialize;
use serde_json::{Value, json};

use crate::{Driver, Error, Platform, Result, Viewport};

/// How many same-site links are asked at most, by default.
pub const DEFAULT_LINKS: usize = 20;
/// The most a test may ask for.
pub const MAX_LINKS: usize = 50;

/// What a check found, in the words of the S8 card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Checked,
    Problem,
    CouldNotRun,
    NotChecked,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            Self::Checked => "checked",
            Self::Problem => "problem",
            Self::CouldNotRun => "could not run",
            Self::NotChecked => "not checked",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    /// PF4: missing evidence here never makes the verdict INCONCLUSIVE (speed is extra evidence).
    #[serde(skip)]
    pub optional: bool,
    pub name: &'static str,
    pub state: State,
    /// One line: "No errors", "2 of 14 links are broken".
    pub summary: String,
    /// Up to five examples.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
    /// A problem with the page itself (it did not load, its own script crashed).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub major: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verdict {
    #[serde(rename = "SHIP")]
    Ship,
    #[serde(rename = "SHIP WITH FIXES")]
    ShipWithFixes,
    #[serde(rename = "DO NOT SHIP")]
    DoNotShip,
    #[serde(rename = "INCONCLUSIVE")]
    Inconclusive,
}

impl Verdict {
    pub fn word(self) -> &'static str {
        match self {
            Self::Ship => "SHIP",
            Self::ShipWithFixes => "SHIP WITH FIXES",
            Self::DoNotShip => "DO NOT SHIP",
            Self::Inconclusive => "INCONCLUSIVE",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub url: String,
    pub checks: Vec<Check>,
    pub verdict: Verdict,
    pub why: String,
}

/// The release-check rules over the checks of one or more pages.
pub fn verdict(checks: &[&Check]) -> (Verdict, String) {
    let major: Vec<&&Check> = checks.iter().filter(|c| c.state == State::Problem && c.major).collect();
    let problems: Vec<&&Check> = checks.iter().filter(|c| c.state == State::Problem).collect();
    let missing: Vec<&&Check> = checks.iter().filter(|c| !c.optional && matches!(c.state, State::CouldNotRun | State::NotChecked)).collect();
    let names = |v: &[&&Check]| {
        let mut n: Vec<&str> = v.iter().map(|c| c.name).collect();
        n.dedup();
        n.join(", ").to_lowercase()
    };
    if !major.is_empty() {
        return (Verdict::DoNotShip, major.iter().map(|c| c.summary.as_str()).collect::<Vec<_>>().join("; "));
    }
    if !missing.is_empty() {
        let found = if problems.is_empty() { String::new() } else { format!(" Found so far: {}.", names(&problems)) };
        return (Verdict::Inconclusive, format!("Important evidence is missing: {} could not be checked.{found}", names(&missing)));
    }
    if !problems.is_empty() {
        let n = problems.len();
        return (Verdict::ShipWithFixes, format!("{n} minor problem{} ({}); the page loads and its own script runs.", if n == 1 { "" } else { "s" }, names(&problems)));
    }
    (Verdict::Ship, "No console errors, failed requests, broken links, layout or accessibility problems were found.".into())
}

/// Words that also keep a link from being asked when it is asked signed in (`cookies: true`): a
/// GET that changes something (Security, PR #100: servers treat HEAD like GET).
const SIGNED_IN_WORDS: &[&str] = &[
    "reset", "clear", "empty", "archive", "accept", "approve", "confirm", "activate", "disable", "enable", "ban", "block", "revoke",
    "publish", "send", "submit", "refund", "void", "invite", "join", "verify", "reject", "close", "wipe", "purge", "token", "pay",
    "order", "buy", "checkout", "transfer", "add", "follow", "like", "vote", "subscribe", "toggle", "set", "update", "edit",
];

/// Words for a change that are never asked, signed in or not (Security, PR #100 re-check): a
/// one-time link works without cookies too.
const CHANGE_WORDS: &[&str] = &["reset", "clear", "wipe", "purge", "archive", "ban", "revoke", "verify", "accept", "invite", "activate", "confirm"];
/// Query parameters that carry a one-time secret.
const SECRET_PARAMS: &[&str] = &["token", "key", "sig", "signature", "code", "otp", "secret"];

/// Words that keep a link from being asked: the block and confirm lists, and leaving or deleting.
fn risky_words() -> Vec<String> {
    let mut w: Vec<String> = crate::policy::blocked_words().into_iter().chain(crate::policy::risky_words()).map(|w| w.trim().to_lowercase()).filter(|w| w.len() >= 3).collect();
    for x in [
        "logout", "log-out", "log out", "log_out", "signout", "sign-out", "sign out", "sign_out", "logoff", "log-off", "signoff", "sign-off", "disconnect",
        "deauth", "delete", "remove", "unsubscribe", "destroy", "cancel", "keluar", "hapus",
    ] {
        w.push(x.into());
    }
    w.sort();
    w.dedup();
    w
}

/// The page's own address and how it loaded (the main document's status, when the browser knows).
const PAGE_JS: &str = r#"(() => {
  const nav = performance.getEntriesByType('navigation')[0];
  return { url: location.href, origin: location.origin, status: nav && nav.responseStatus ? nav.responseStatus : 0,
           width: innerWidth, height: innerHeight, scale: devicePixelRatio };
})()"#;

/// The page's links, as the browser resolves them (at most 500). Which ones may be asked is
/// decided in Rust, not in the page (a page can change the browser's own functions).
const LINKS_JS: &str = r#"(() => [...document.querySelectorAll('a[href]')].slice(0, 500).map((a) => {
  let u = ''; try { u = new URL(a.getAttribute('href'), location.href).href; } catch (e) {}
  return [u, (a.innerText || '').slice(0, 200), (a.getAttribute('aria-label') || '').slice(0, 200)];
}))()"#;

/// Asks the approved links with HEAD, five at a time, redirects not followed. `cookies`: with the
/// session's cookies (signed in), else none.
const HEAD_JS: &str = r#"(async (urls, cookies) => {
  const out = [];
  const todo = urls.slice();
  const one = async () => {
    for (let u = todo.shift(); u !== undefined; u = todo.shift()) {
      const stop = new AbortController(), t = setTimeout(() => stop.abort(), 8000);
      try {
        const r = await fetch(u, { method: 'HEAD', credentials: cookies ? 'same-origin' : 'omit', redirect: 'manual', cache: 'no-store', signal: stop.signal });
        out.push([u, r.type === 'opaqueredirect' ? 0 : r.status]);
      } catch (e) { out.push([u, -1]); }
      finally { clearTimeout(t); }
    }
  };
  await Promise.all([one(), one(), one(), one(), one()]);
  return out;
})"#;

/// Content wider than the screen, with up to three of the widest elements.
const LAYOUT_JS: &str = r#"(() => {
  const w = document.documentElement.clientWidth;
  const over = Math.max(document.documentElement.scrollWidth, document.body ? document.body.scrollWidth : 0) - w;
  const name = (e) => e.tagName.toLowerCase() + (e.id ? '#' + e.id : '') + (typeof e.className === 'string' && e.className.trim() ? '.' + e.className.trim().split(/\s+/)[0] : '');
  const wide = [];
  if (over > 1) for (const e of document.body.querySelectorAll('*')) {
    const r = e.getBoundingClientRect();
    if (r.width > 0 && r.height > 0 && r.right > w + 1 && getComputedStyle(e).position !== 'fixed') wide.push([r.right - w, name(e)]);
  }
  wide.sort((a, b) => b[0] - a[0]);
  const meta = document.querySelector('meta[name=viewport]');
  const fits = !!meta && /width\s*=\s*device-width/i.test(meta.getAttribute('content') || '');
  return { width: w, over: Math.max(0, Math.round(over)), wide: [...new Set(wide.map((x) => x[1]))].slice(0, 3), fits };
})()"#;

/// Accessibility basics a script can see.
const A11Y_JS: &str = r#"(() => {
  const vis = (e) => { const r = e.getBoundingClientRect(); const s = getComputedStyle(e); return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none'; };
  const named = (e) => (e.getAttribute('aria-label') || '').trim() || (e.getAttribute('aria-labelledby') || '').trim() || (e.getAttribute('title') || '').trim();
  const text = (e) => (e.innerText || '').trim() || [...e.querySelectorAll('img[alt]')].map((i) => i.alt.trim()).join('');
  const short = (e) => e.tagName.toLowerCase() + (e.id ? '#' + e.id : '') + (e.getAttribute('name') ? '[name=' + e.getAttribute('name') + ']' : '') + (e.getAttribute('src') ? ' ' + e.getAttribute('src').split('/').pop().slice(0, 40) : '');
  const issues = [];
  if (!(document.documentElement.getAttribute('lang') || '').trim()) issues.push(['The page has no language (html lang)', []]);
  if (!document.title.trim()) issues.push(['The page has no title', []]);
  const img = [...document.querySelectorAll('img')].filter((i) => vis(i) && !i.hasAttribute('alt') && i.getAttribute('role') !== 'presentation');
  if (img.length) issues.push([img.length + ' image' + (img.length > 1 ? 's have' : ' has') + ' no alt text', img.map(short)]);
  const fields = [...document.querySelectorAll('input, select, textarea')].filter((f) => vis(f) && !['hidden', 'submit', 'button', 'reset', 'image'].includes((f.type || '').toLowerCase())
    && !named(f) && !(f.id && document.querySelector('label[for="' + CSS.escape(f.id) + '"]')) && !f.closest('label') && !(f.getAttribute('placeholder') || '').trim());
  if (fields.length) issues.push([fields.length + ' form field' + (fields.length > 1 ? 's have' : ' has') + ' no label', fields.map(short)]);
  const btns = [...document.querySelectorAll('button, a[href], [role=button], input[type=submit], input[type=button]')].filter((b) => vis(b) && !named(b) && !text(b) && !(b.value || '').trim());
  if (btns.length) issues.push([btns.length + ' button' + (btns.length > 1 ? 's or links have' : ' or link has') + ' no name', btns.map(short)]);
  const ids = {};
  for (const e of document.querySelectorAll('[id]')) ids[e.id] = (ids[e.id] || 0) + 1;
  const dup = Object.keys(ids).filter((k) => ids[k] > 1);
  if (dup.length) issues.push([dup.length + ' id' + (dup.length > 1 ? 's are' : ' is') + ' used more than once', dup.map((d) => '#' + d)]);
  return issues.map(([what, ex]) => ({ what, examples: ex.slice(0, 3) }));
})()"#;

const SIZES: &[(&str, u32, u32, bool)] = &[("phone", 390, 844, true), ("tablet", 768, 1024, true), ("desktop", 1280, 800, false)];

fn check(name: &'static str, state: State, summary: impl Into<String>, details: Vec<String>) -> Check {
    Check { name, state, summary: summary.into(), details: details.into_iter().take(5).collect(), major: false, optional: false }
}

// ---- PF4: speed, compared with the check before -------------------------------------------

/// Speed is measured on one window size and the normal connection, every time.
pub const SPEED_VIEWPORT: (u32, u32) = (1280, 800);
const CONNECTION: &str = "normal";

/// One measurement as stored in `~/.fastpilot/projects/<project>/ship/<host>.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, serde::Deserialize)]
pub struct Sample {
    pub at_ms: u64,
    pub connection: String,
    pub viewport: String,
    #[serde(flatten)]
    pub speed: crate::Speed,
    /// What was flagged against the earlier check, in words (shown only; Release watch recomputes).
    #[serde(default)]
    pub flags: Vec<String>,
}

/// Leaving, deleting or a change in the address: the page is not opened again to measure it.
const NO_REOPEN_WORDS: &[&str] = &[
    "logout", "log-out", "log_out", "signout", "sign-out", "sign_out", "logoff", "deauth", "disconnect", "delete", "remove", "unsubscribe", "destroy", "cancel", "keluar", "hapus",
];

/// Parts of a parameter name that say it carries a code, key or session.
const SECRET_NAME_PARTS: &[&str] = &["token", "key", "sig", "code", "otp", "secret", "auth", "session", "pass", "nonce", "ticket", "jwt"];

/// A query or fragment parameter name (decoded) that may carry a one-time code or key.
fn secret_name(name: &str) -> bool {
    let n = pct_decode(name).to_ascii_lowercase();
    SECRET_PARAMS.contains(&n.as_str()) || SECRET_NAME_PARTS.iter().any(|p| n.contains(p)) || n.ends_with("_id")
}

/// A path segment that looks like a random one-time id (`/magic/AbCdEf123456`).
fn random_segment(seg: &str) -> bool {
    let seg = pct_decode(seg);
    if !seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return false;
    }
    let alnum = seg.chars().filter(char::is_ascii_alphanumeric).count();
    let digits = seg.chars().any(|c| c.is_ascii_digit());
    let letters = seg.chars().any(|c| c.is_ascii_alphabetic());
    let mixed_case = seg.chars().any(|c| c.is_ascii_uppercase()) && seg.chars().any(|c| c.is_ascii_lowercase());
    digits && letters && (alnum >= 16 || (alnum >= 12 && mixed_case))
}

/// The address after the host: (path, query, fragment). `?` and `#` count anywhere, also with
/// no `/` before them.
fn address_parts(url: &str) -> (&str, &str, &str) {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = &rest[rest.find(['/', '?', '#']).unwrap_or(rest.len())..];
    let (rest, frag) = rest.split_once('#').unwrap_or((rest, ""));
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    (path, query, frag)
}

/// Why the page's own address is not opened again for a speed measurement, if it is not.
pub fn reopen_refusal(url: &str) -> Option<&'static str> {
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("file://")) {
        return Some("it is not a web address");
    }
    let (path, query, frag) = address_parts(url);
    let params = query.split(['&', ';']).chain(frag.trim_start_matches(['/', '!', '?']).split(['&', ';', '?']));
    if params.filter_map(|kv| kv.split_once('=').map(|(k, _)| k)).any(secret_name) || path.split('/').any(random_segment) {
        return Some("its address carries a one-time code or key");
    }
    let words = pct_decode(&format!("{path}?{query}#{frag}")).to_lowercase();
    if CHANGE_WORDS.iter().chain(NO_REOPEN_WORDS).any(|w| words.contains(w)) {
        return Some("its address looks like a link that changes something");
    }
    None
}

/// Text kept in the speed file or shown: test variables masked (`dsl::redact`), any
/// `user:password@` replaced by `***@`, and parameter values replaced by `***`: every value when
/// `all` (page errors and failed requests), else those whose name may carry a code or key.
fn scrub(text: &str, all: bool) -> String {
    let text = crate::dsl::redact(text);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(i) = rest.find("://") {
        out.push_str(&rest[..i + 3]);
        rest = &rest[i + 3..];
        let end = rest.find(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace()).unwrap_or(rest.len());
        match rest[..end].rfind('@') {
            Some(at) => {
                out.push_str("***");
                out.push_str(&rest[at..end]);
            }
            None => out.push_str(&rest[..end]),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    let bytes = out.as_bytes();
    let mut masked = String::with_capacity(out.len());
    let (mut last, mut i) = (0, 0);
    while i < bytes.len() {
        if !matches!(bytes[i], b'?' | b'&' | b';' | b'#') {
            i += 1;
            continue;
        }
        let name_end = (i + 1..bytes.len()).find(|&j| !(bytes[j].is_ascii_alphanumeric() || matches!(bytes[j], b'_' | b'-' | b'.' | b'%' | b'[' | b']'))).unwrap_or(bytes.len());
        if name_end == i + 1 || bytes.get(name_end) != Some(&b'=') || !(all || secret_name(&out[i + 1..name_end])) {
            i += 1;
            continue;
        }
        let start = name_end + 1;
        let end = out[start..].find(|c: char| matches!(c, '&' | ';' | '#' | '"' | '\'' | ')' | ',' | '<' | '>') || c.is_whitespace()).map_or(out.len(), |e| start + e);
        masked.push_str(&out[last..start]);
        masked.push_str("***");
        last = end;
        i = end;
    }
    masked.push_str(&out[last..]);
    masked
}

/// Where a project's speed measurements are kept: in fastpilot's own folder on this computer
/// (`~/.fastpilot/projects/<sha256 of the project folder>/ship`), never in the project, so a
/// cloned project cannot bring its own "before".
pub fn speed_dir(project: &std::path::Path) -> std::path::PathBuf {
    let real = std::fs::canonicalize(project).unwrap_or_else(|_| project.to_owned());
    let hex: String = ring::digest::digest(&ring::digest::SHA256, real.to_string_lossy().as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    let home = crate::config::path().parent().map(std::path::Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
    home.join("projects").join(hex).join("ship")
}

/// The file for a page's measurements (`shop.example.json`, `local-files.json`) of the project in
/// the current folder.
pub fn speed_file(url: &str) -> std::path::PathBuf {
    let host = url.split_once("://").filter(|(s, _)| *s != "file").map(|(_, r)| r.split(['/', '?', '#']).next().unwrap_or("").rsplit('@').next().unwrap_or("").to_lowercase()).unwrap_or_default();
    let host: String = host.chars().take(100).map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    let name = if host.is_empty() || host.chars().all(|c| c == '.') { "local-files".to_owned() } else { host };
    speed_dir(&std::env::current_dir().unwrap_or_default()).join(format!("{name}.json"))
}

const MAX_SPEED_FILE: u64 = 1_000_000;
const MAX_SAMPLES: usize = 100;

/// The samples in a file: at most 1 MB and the last 100; never through a link.
pub fn read_samples(path: &std::path::Path) -> Vec<Sample> {
    let Ok(m) = std::fs::symlink_metadata(path) else { return vec![] };
    if !m.is_file() || m.len() > MAX_SPEED_FILE {
        return vec![];
    }
    let mut all: Vec<Sample> = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let extra = all.len().saturating_sub(MAX_SAMPLES);
    all.drain(..extra);
    all
}

pub fn load_samples(url: &str) -> Vec<Sample> {
    read_samples(&speed_file(url))
}

/// Written as a new owner-only file and renamed into place; never through a link.
fn save_sample(s: &Sample) -> std::io::Result<()> {
    save_to(&speed_file(&s.speed.url), s)
}

fn save_to(path: &std::path::Path, s: &Sample) -> std::io::Result<()> {
    let path = path.to_path_buf();
    let mut all = read_samples(&path);
    all.push(s.clone());
    let extra = all.len().saturating_sub(MAX_SAMPLES);
    all.drain(..extra);
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)?;
    if std::fs::symlink_metadata(dir)?.file_type().is_symlink() || std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
        return Err(std::io::Error::other("the speed file or its folder is a link"));
    }
    let tmp = path.with_extension(format!("tmp-{}-{}", std::process::id(), s.at_ms));
    {
        use std::io::Write;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&tmp)?;
        f.write_all(serde_json::to_string_pretty(&all).unwrap_or_default().as_bytes())?;
    }
    std::fs::rename(&tmp, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// PF4 Release watch: the pages measured in this run (`since_ms` to now) that are slower than
/// before the watch began (`watch_started_ms`), as (`Speed · /path`, what is flagged). The flags
/// are worked out again from the numbers, never taken from the file.
pub fn watch_problems(project: &std::path::Path, watch_started_ms: Option<u64>, since_ms: u64, now_ms: u64) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(speed_dir(project)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    for path in files.into_iter().take(200) {
        let samples = read_samples(&path);
        for s in samples.iter().filter(|s| (since_ms..=now_ms).contains(&s.at_ms)) {
            let Some(flag) = compare(s, baseline(&samples, s, watch_started_ms)).into_iter().find_map(|r| r.flagged) else { continue };
            let (page, _, _) = address_parts(&s.speed.url);
            let id = format!("Speed · /{}", clean(page.trim_start_matches('/'), 80));
            if !out.iter().any(|(x, _)| *x == id) {
                out.push((id, clean(&flag, 160)));
            }
        }
    }
    out
}

/// The check to compare with: the latest of the same page, connection and window size. During a
/// Release watch (`FASTPILOT_SPEED_BASELINE_MS`): the latest from before the watch began, else the
/// watch's own first ("before") check.
pub fn baseline<'a>(samples: &'a [Sample], now: &Sample, before_ms: Option<u64>) -> Option<&'a Sample> {
    let same = || samples.iter().filter(|s| s.speed.url == now.speed.url && s.connection == now.connection && s.viewport == now.viewport && s.at_ms < now.at_ms);
    match before_ms {
        None => same().max_by_key(|s| s.at_ms),
        Some(b) => same().filter(|s| s.at_ms <= b).max_by_key(|s| s.at_ms).or_else(|| same().filter(|s| s.at_ms > b).min_by_key(|s| s.at_ms)),
    }
}

/// One value and its change in words ("2.8 s ▲ 41% slower", "no change", "not measured").
#[derive(Debug, Clone, PartialEq)]
pub struct SpeedRow {
    pub label: &'static str,
    pub value: String,
    pub change: String,
    pub flagged: Option<String>,
}

fn secs(ms: u64) -> String {
    if ms < 1000 { format!("{:.2} s", ms as f64 / 1000.0) } else { format!("{:.1} s", ms as f64 / 1000.0) }
}

fn mb(bytes: u64) -> String {
    if bytes >= 1_000_000 { format!("{:.1} MB", bytes as f64 / 1_000_000.0) } else { format!("{} KB", bytes.div_ceil(1000)) }
}

/// The rows of the Speed section, with what is flagged: more than 20% and more than 300 ms slower,
/// layout jumps up by more than 0.1, a new page error, a new failed request. Within 10%: no change.
pub fn compare(now: &Sample, before: Option<&Sample>) -> Vec<SpeedRow> {
    let time = |label: &'static str, n: Option<u64>, b: Option<Option<u64>>| -> SpeedRow {
        let Some(n) = n else { return SpeedRow { label, value: "not measured".into(), change: String::new(), flagged: None } };
        let (change, flagged) = match b {
            None => ("no earlier value".to_owned(), None),
            Some(None) => ("no earlier value".to_owned(), None),
            Some(Some(b)) => {
                let pct = if b == 0 { 0.0 } else { (n as f64 - b as f64) / b as f64 * 100.0 };
                if pct.abs() <= 10.0 {
                    ("no change".into(), None)
                } else if pct > 0.0 {
                    let flag = (pct > 20.0 && n.saturating_sub(b) > 300).then(|| format!("{label} {}, up from {} ({:.0}% slower)", secs(n), secs(b), pct));
                    (format!("▲ {:.0}% slower", pct), flag)
                } else {
                    (format!("▼ {:.0}% faster", -pct), None)
                }
            }
        };
        SpeedRow { label, value: secs(n), change, flagged }
    };
    let b = before.map(|b| &b.speed);
    let mut rows = vec![time("Loads in", now.speed.load_ms, b.map(|b| b.load_ms)), time("Main content appears", now.speed.lcp_ms, b.map(|b| b.lcp_ms))];
    rows.push(match (now.speed.cls, b.map(|b| b.cls)) {
        (None, _) => SpeedRow { label: "Layout jumps", value: "not measured".into(), change: String::new(), flagged: None },
        (Some(n), None | Some(None)) => SpeedRow { label: "Layout jumps", value: format!("{n:.2}"), change: "no earlier value".into(), flagged: None },
        (Some(n), Some(Some(o))) => {
            let up = n - o;
            let flagged = (up > 0.1).then(|| format!("Layout jumps {n:.2}, up from {o:.2}"));
            SpeedRow { label: "Layout jumps", value: format!("{n:.2}"), change: if up.abs() < 0.01 { "no change".into() } else if up > 0.0 { format!("▲ {up:.2} more") } else { format!("▼ {:.2} less", -up) }, flagged }
        }
    });
    let req = format!("{} · {}", now.speed.requests, mb(now.speed.bytes));
    rows.push(match b {
        None => SpeedRow { label: "Requests", value: req, change: "no earlier value".into(), flagged: None },
        Some(b) => {
            let d = now.speed.requests as i64 - b.requests as i64;
            let small = (d.unsigned_abs() as f64) <= (b.requests as f64 * 0.1).max(1.0);
            SpeedRow { label: "Requests", value: req, change: if small { "no change".into() } else if d > 0 { format!("▲ {d} more") } else { format!("▼ {} fewer", -d) }, flagged: None }
        }
    });
    let new_items = |now: &[String], before: &[String]| -> Vec<String> { now.iter().filter(|e| !before.contains(e)).cloned().collect() };
    let (errs, fails) = match b {
        Some(b) => (new_items(&now.speed.errors, &b.errors), new_items(&now.speed.failed, &b.failed)),
        None => (vec![], vec![]),
    };
    rows.push(SpeedRow {
        label: "Page errors",
        value: now.speed.errors.len().to_string(),
        change: match b {
            None => "no earlier value".into(),
            Some(_) if errs.is_empty() => "no change".into(),
            Some(_) => format!("▲ {} new", errs.len()),
        },
        flagged: (!errs.is_empty()).then(|| format!("{} new page {}: {}", errs.len(), if errs.len() == 1 { "error" } else { "errors" }, clean(&errs[0], 120))),
    });
    if !fails.is_empty() {
        rows.push(SpeedRow { label: "Failed requests", value: now.speed.failed.len().to_string(), change: format!("▲ {} new", fails.len()), flagged: Some(format!("{} new failed {}: {}", fails.len(), if fails.len() == 1 { "request" } else { "requests" }, clean(&fails[0], 120))) });
    }
    rows
}

/// PF4: measure, compare with the check before, store, and say it as the Speed check. Never a
/// guessed value; speed alone gives at most SHIP WITH FIXES; not measured never blocks the verdict.
pub async fn speed_check(d: &dyn Driver, url: &str) -> Check {
    let c = |state: State, summary: String, details: Vec<String>| Check { optional: true, ..check("Speed", state, summary, details) };
    if let Some(why) = reopen_refusal(url) {
        return c(State::NotChecked, format!("Not measured: {why}, so it is not opened again"), vec![]);
    }
    let (w, h) = SPEED_VIEWPORT;
    let v = Viewport { width: w, height: h, scale: 1.0, mobile: false, touch: false, user_agent: None };
    if let Err(e) = d.set_viewport(Some(&v)).await {
        return c(State::CouldNotRun, format!("Not measured: {}", clean(&e.to_string(), 200)), vec![]);
    }
    let speed = match d.measure_speed().await {
        Ok(s) => crate::Speed { url: scrub(&s.url, false), errors: s.errors.iter().map(|e| scrub(e, true)).collect(), failed: s.failed.iter().map(|f| scrub(f, true)).collect(), ..s },
        Err(e) => return c(State::NotChecked, format!("Not measured: {}", clean(&e.to_string(), 200)), vec![]),
    };
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let mut now = Sample { at_ms: now_ms, connection: CONNECTION.into(), viewport: format!("{w}x{h}"), speed, flags: vec![] };
    let samples = load_samples(&now.speed.url);
    let before_ms = crate::config::var("FASTPILOT_SPEED_BASELINE_MS").ok().and_then(|v| v.trim().parse().ok());
    let before = baseline(&samples, &now, before_ms).cloned();
    let rows = compare(&now, before.as_ref());
    now.flags = rows.iter().filter_map(|r| r.flagged.clone()).collect();
    let _ = save_sample(&now);
    let line = rows.iter().map(|r| if r.change.is_empty() { format!("{} {}", r.label, r.value) } else { format!("{} {} ({})", r.label, r.value, r.change) }).collect::<Vec<_>>().join(" · ");
    let basis = match &before {
        Some(b) => format!("Compared with the check of {}, {} · normal connection · {w}×{h}", crate::claim::local_date(b.at_ms), crate::claim::local_hm(b.at_ms)),
        None => "This is the first speed measurement. There is nothing to compare with yet. The next check will show what changed.".into(),
    };
    let mut details = vec![basis];
    details.extend(now.flags.iter().cloned());
    details.push(format!(
        "Details: LCP {}, CLS {} (layout shifts summed). Flagged when a page is more than 20% and more than 0.3 s slower, layout jumps grow by more than 0.1, or a new page error or failed request appears.",
        now.speed.lcp_ms.map_or("not measured".into(), |m| format!("{m} ms")),
        now.speed.cls.map_or("not measured".into(), |x| format!("{x:.3}"))
    ));
    if now.flags.is_empty() { c(State::Checked, line, details) } else { c(State::Problem, line, details) }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Page text in the report (Security, PR #100): control characters (new lines, terminal escapes,
/// bidi marks) become spaces, and it is clipped, so a page cannot forge report lines or flood it.
pub fn clean(text: &str, max: usize) -> String {
    let t: String = text
        .chars()
        // Invisible characters (tags, variation selectors, format controls) are dropped: they
        // could spell words the person cannot see but the bot reads.
        .filter(|c| !matches!(*c as u32, 0xe0000..=0xe0fff | 0xfe00..=0xfe0f | 0x2060..=0x206f | 0x00ad | 0x034f | 0x180b..=0x180f | 0x115f | 0x1160 | 0x3164 | 0xffa0))
        .map(|c| if c.is_control() || matches!(c as u32, 0x200b..=0x200f | 0x202a..=0x202e | 0x2066..=0x2069 | 0xfeff) { ' ' } else { c })
        .collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > max { format!("{}…", t.chars().take(max).collect::<String>()) } else { t }
}

/// `%xx` decoded where it is valid; the rest kept as it is (a stray `%` is common: "/sale/50%").
fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Which collected links may be asked: the page's own origin only, never one whose text or
/// address names a risky action; signed in (`cookies`), also never one with a query or a word for
/// a change. Decided here, in Rust, from what the page reported.
#[derive(Debug, Default, PartialEq)]
pub struct LinkPlan {
    pub ask: Vec<(String, String)>,
    pub external: usize,
    pub risky: usize,
    pub more: usize,
}

pub fn plan_links(collected: &Value, origin: &str, max: usize, cookies: bool) -> LinkPlan {
    let mut plan = LinkPlan::default();
    let risky = risky_words();
    let mut seen = std::collections::HashSet::new();
    let own = |u: &str| u == origin || u.starts_with(&format!("{origin}/"));
    for l in collected.as_array().into_iter().flatten() {
        let url = l[0].as_str().unwrap_or("");
        let url = url.split('#').next().unwrap_or("");
        if !(url.starts_with("http://") || url.starts_with("https://")) || url.chars().any(|c| c.is_whitespace() || c.is_control()) || !seen.insert(url.to_owned()) {
            continue;
        }
        if origin.is_empty() || !own(url) {
            plan.external += 1;
            continue;
        }
        let rest = &url[origin.len()..];
        let words = format!("{} {} {}", l[1].as_str().unwrap_or(""), l[2].as_str().unwrap_or(""), pct_decode(rest)).to_lowercase();
        let signed_in = cookies && (rest.contains('?') || SIGNED_IN_WORDS.iter().any(|w| words.contains(w)));
        let query = rest.split_once('?').map_or("", |(_, q)| q).to_lowercase();
        let secret = query.split('&').any(|kv| SECRET_PARAMS.contains(&kv.split('=').next().unwrap_or("")));
        if signed_in || secret || CHANGE_WORDS.iter().any(|w| words.contains(w)) || risky.iter().any(|w| words.contains(w.as_str())) {
            plan.risky += 1;
            continue;
        }
        let text = if l[1].as_str().unwrap_or("").trim().is_empty() { l[2].as_str().unwrap_or("") } else { l[1].as_str().unwrap_or("") };
        plan.ask.push((url.to_owned(), clean(text, 60)));
    }
    plan.more = plan.ask.len().saturating_sub(max);
    plan.ask.truncate(max);
    plan
}

/// Run every check on the page open in `d`. `cookies`: ask links signed in (with the session's
/// cookies); off by default, because servers treat HEAD like GET.
pub async fn run(d: &dyn Driver, links: usize, cookies: bool) -> Result<Report> {
    if d.platform() != Platform::Web {
        return Err(Error::Unsupported("shipCheck is for web pages (open the page in a web session)".into()));
    }
    // Errors seen before the check, and those during it (resizing can raise them too).
    let mut diag = d.diagnostics().await;
    let page = d.eval_script(PAGE_JS).await?;
    let url = clean(page["url"].as_str().unwrap_or(""), 200);
    let origin = page["origin"].as_str().unwrap_or("").to_owned();
    let mut checks = Vec::new();

    // Links: collected by the page, chosen here, asked with HEAD.
    let mut asked: Vec<String> = Vec::new();
    let link_check = if links == 0 {
        check("Broken links", State::NotChecked, "Not checked (links: 0)", vec![])
    } else {
        match d.eval_script(LINKS_JS).await {
            Ok(v) => {
                let plan = plan_links(&v, &origin, links.min(MAX_LINKS), cookies);
                let urls: Vec<&String> = plan.ask.iter().map(|(u, _)| u).collect();
                asked = plan.ask.iter().map(|(u, _)| u.clone()).collect();
                match d.eval_script(&format!("{HEAD_JS}({}, {cookies})", json!(urls))).await {
                    Ok(answers) => links_check(&plan, &answers, cookies),
                    Err(e) => check("Broken links", State::CouldNotRun, format!("Could not run: {}", clean(&e.to_string(), 200)), vec![]),
                }
            }
            Err(e) => check("Broken links", State::CouldNotRun, format!("Could not run: {}", clean(&e.to_string(), 200)), vec![]),
        }
    };

    // Layout at three sizes; the size is put back afterwards (clamped: the page reports it).
    let num = |k: &str, d: f64| page[k].as_f64().filter(|x| x.is_finite()).unwrap_or(d);
    let (bw, bh) = (num("width", 1280.0).clamp(200.0, 4000.0) as u32, num("height", 800.0).clamp(200.0, 4000.0) as u32);
    let before = Viewport { width: bw, height: bh, scale: num("scale", 1.0).clamp(0.5, 5.0), mobile: bw < 800, touch: bw < 800, user_agent: None };
    let mut wide = Vec::new();
    let mut failed = None;
    for (name, w, h, mobile) in SIZES {
        let v = Viewport { width: *w, height: *h, scale: 1.0, mobile: *mobile, touch: *mobile, user_agent: None };
        let res = async {
            d.set_viewport(Some(&v)).await?;
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            d.eval_script(LAYOUT_JS).await
        }
        .await;
        match res {
            // No mobile viewport: a phone lays the page out at desktop width and shows it zoomed out.
            Ok(l) if *mobile && l["fits"] == false && l["width"].as_u64().unwrap_or(0) > u64::from(*w) + 1 => {
                wide.push(format!("{name} ({w} px): laid out {} px wide and shown zoomed out (no mobile viewport: <meta name=viewport content=\"width=device-width\">)", l["width"].as_u64().unwrap_or(0).min(100_000)));
            }
            Ok(l) if l["over"].as_u64().unwrap_or(0) > 1 => {
                let what: Vec<String> = l["wide"].as_array().into_iter().flatten().take(3).filter_map(|x| x.as_str().map(|x| clean(x, 80))).collect();
                wide.push(format!("{name} ({w} px): content is {} px wider than the screen{}", l["over"].as_u64().unwrap_or(0).min(100_000), if what.is_empty() { String::new() } else { format!(" ({})", what.join(", ")) }));
            }
            Ok(_) => {}
            Err(e) => {
                failed = Some(format!("{name}: {}", clean(&e.to_string(), 200)));
                break;
            }
        }
    }
    let restored = d.set_viewport(Some(&before)).await;
    let layout = match (failed, wide.is_empty()) {
        (Some(e), _) => check("Layout at 3 sizes", State::CouldNotRun, format!("Could not run: {e}"), wide),
        (None, true) => check("Layout at 3 sizes", State::Checked, "Phone 390, tablet 768 and desktop 1280 px: nothing wider than the screen", vec![]),
        (None, false) => check("Layout at 3 sizes", State::Problem, format!("Wider than the screen at {}", plural(wide.len(), "size", "sizes")), wide),
    };
    restored?;

    // Accessibility basics.
    let a11y = match d.eval_script(A11Y_JS).await {
        Ok(v) => {
            let issues: Vec<(String, Vec<String>)> = v
                .as_array()
                .into_iter()
                .flatten()
                .take(10)
                .map(|i| (clean(i["what"].as_str().unwrap_or(""), 120), i["examples"].as_array().into_iter().flatten().take(3).filter_map(|x| x.as_str().map(|x| clean(x, 80))).collect()))
                .collect();
            if issues.is_empty() {
                check("Accessibility basics", State::Checked, "Language, title, alt text, labels, names and ids are fine", vec![])
            } else {
                let details = issues.iter().map(|(w, ex)| if ex.is_empty() { w.clone() } else { format!("{w}: {}", ex.join(", ")) }).collect();
                check("Accessibility basics", State::Problem, plural(issues.len(), "problem", "problems"), details)
            }
        }
        Err(e) => check("Accessibility basics", State::CouldNotRun, format!("Could not run: {}", clean(&e.to_string(), 200)), vec![]),
    };

    // Console errors and failed requests: before and during the check, the page's own first.
    // fastpilot's own HEAD requests are not the page's failed requests.
    diag.extend(d.diagnostics().await.into_iter().filter(|l| !asked.iter().any(|u| l.ends_with(&format!(" {u}")))));
    let diag: Vec<String> = diag.iter().map(|l| clean(l, 200)).collect();
    let (console, requests): (Vec<String>, Vec<String>) = diag.into_iter().partition(|l| l.starts_with("exception:") || l.starts_with("console.error:"));
    let exceptions = console.iter().filter(|l| l.starts_with("exception:")).count();
    let mut c = if console.is_empty() {
        check("Console errors", State::Checked, "No errors", vec![])
    } else {
        let crash = if exceptions > 0 { format!(", {exceptions} uncaught: the page's own script crashed") } else { String::new() };
        check("Console errors", State::Problem, format!("{}{crash}", plural(console.len(), "error", "errors")), console)
    };
    c.major = exceptions > 0;
    checks.push(c);
    let status = page["status"].as_u64().unwrap_or(0).min(999);
    let mut r = if status >= 400 {
        check("Failed requests", State::Problem, format!("The page itself answered HTTP {status}"), requests)
    } else if requests.is_empty() {
        check("Failed requests", State::Checked, "No failed requests", vec![])
    } else {
        check("Failed requests", State::Problem, plural(requests.len(), "request failed", "requests failed"), requests)
    };
    r.major = status >= 400;
    checks.push(r);
    checks.push(link_check);
    checks.push(layout);
    checks.push(a11y);
    // PF4: speed last, from a fresh load measured through CDP only, so the other checks above are
    // about the page as it was. The window size is put back, and what the reload itself reported
    // is not left for the next step.
    let speed = speed_check(d, &url).await;
    let _ = d.set_viewport(Some(&before)).await;
    let _ = d.diagnostics().await;
    checks.push(speed);

    let (verdict, why) = verdict(&checks.iter().collect::<Vec<_>>());
    Ok(Report { url, checks, verdict, why })
}

fn links_check(plan: &LinkPlan, answers: &Value, cookies: bool) -> Check {
    let text_of = |u: &str| plan.ask.iter().find(|(x, _)| x == u).map(|(_, t)| t.clone()).unwrap_or_default();
    let name = |u: &str| {
        let t = text_of(u);
        let u = clean(u, 200);
        if t.is_empty() { u } else { format!("\"{t}\" {u}") }
    };
    let (mut ok, mut broken, mut unchecked) = (0usize, Vec::new(), Vec::new());
    for a in answers.as_array().into_iter().flatten() {
        let u = a[0].as_str().unwrap_or("");
        // Only answers for links fastpilot asked count.
        if !plan.ask.iter().any(|(x, _)| x == u) {
            continue;
        }
        match a[1].as_i64().unwrap_or(-1) {
            0 => ok += 1,
            401 | 403 => unchecked.push(format!("{}: needs sign-in{}", name(u), if cookies { "" } else { " (shipCheck { cookies: true } asks signed in)" })),
            405 | 501 => unchecked.push(format!("{}: the server does not answer HEAD", name(u))),
            s if s >= 400 => broken.push(format!("{} → HTTP {s}", name(u))),
            s if s > 0 => ok += 1,
            _ => unchecked.push(format!("{}: could not be reached from the page", name(u))),
        }
    }
    let asked = ok + broken.len() + unchecked.len();
    let mut notes = Vec::new();
    if plan.external > 0 {
        notes.push(format!("{} to other sites not opened", plural(plan.external, "link", "links")));
    }
    if plan.risky > 0 {
        notes.push(format!("{} that could sign out, delete, pay or change something not opened", plural(plan.risky, "link", "links")));
    }
    if plan.more > 0 {
        notes.push(format!("{} more not asked (limit)", plan.more));
    }
    let tail = if notes.is_empty() { String::new() } else { format!("; {}", notes.join("; ")) };
    if !broken.is_empty() {
        return check("Broken links", State::Problem, format!("{} of {asked} links are broken{tail}", broken.len()), broken);
    }
    if asked == 0 {
        return check("Broken links", State::NotChecked, format!("No links to this site were asked{tail}"), vec![]);
    }
    if unchecked.len() == asked {
        return check("Broken links", State::CouldNotRun, format!("None of the {asked} links could be checked{tail}"), unchecked);
    }
    let details = unchecked.into_iter().map(|u| format!("not checked: {u}")).collect();
    check("Broken links", State::Checked, format!("{ok} of {asked} links answer{tail}"), details)
}

impl Report {
    /// The step's output and the terminal's evidence block (S8 T1 layout).
    pub fn text(&self) -> String {
        let mut s = format!("Ship check · {}\nVerdict: {}\nWhy: {}\nEvidence\n", self.url, self.verdict.word(), self.why);
        for c in &self.checks {
            let mark = match c.state {
                State::Checked => "✓",
                State::Problem => "✗",
                State::CouldNotRun => "!",
                State::NotChecked => "–",
            };
            s.push_str(&format!("{mark} {:<22}{} ({})\n", c.name, c.summary, c.state.word()));
            for d in &c.details {
                s.push_str(&format!("    {d}\n"));
            }
        }
        s.push_str("Not checked: colour contrast, keyboard use, the look of pages (no saved reference), anything behind sign-in or other clicks.\n");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &'static str, state: State, major: bool) -> Check {
        Check { name, state, summary: format!("{name} summary"), details: vec![], major, optional: false }
    }

    #[test]
    fn the_verdict_follows_the_release_check_rules() {
        let ok = c("Console errors", State::Checked, false);
        let minor = c("Broken links", State::Problem, false);
        let major = c("Console errors", State::Problem, true);
        let missing = c("Layout at 3 sizes", State::CouldNotRun, false);
        let skipped = c("Broken links", State::NotChecked, false);
        assert_eq!(verdict(&[&ok]).0, Verdict::Ship);
        assert_eq!(verdict(&[&ok, &minor]).0, Verdict::ShipWithFixes);
        assert_eq!(verdict(&[&major, &minor, &missing]).0, Verdict::DoNotShip, "a crash wins over missing evidence");
        let (v, why) = verdict(&[&ok, &minor, &missing]);
        assert_eq!(v, Verdict::Inconclusive, "missing evidence is never a pass");
        assert!(why.contains("layout at 3 sizes could not be checked") && why.contains("Found so far: broken links"), "{why}");
        assert_eq!(verdict(&[&ok, &skipped]).0, Verdict::Inconclusive, "not checked is not a pass either");
        assert_eq!(serde_json::to_value(Verdict::ShipWithFixes).expect("json"), "SHIP WITH FIXES");
    }

    fn sample(at: u64, load: Option<u64>, lcp: Option<u64>, cls: Option<f64>, req: u64, errors: &[&str]) -> Sample {
        Sample {
            at_ms: at,
            connection: "normal".into(),
            viewport: "1280x800".into(),
            speed: crate::Speed { url: "https://shop.test/checkout".into(), load_ms: load, lcp_ms: lcp, cls, requests: req, bytes: 2_100_000, errors: errors.iter().map(|e| (*e).to_owned()).collect(), failed: vec![] },
            flags: vec![],
        }
    }

    /// PF4: flagged only when more than 20% and more than 300 ms slower, layout jumps up by more
    /// than 0.1, or a new page error; within 10% is "no change"; never a guessed value.
    #[test]
    fn speed_changes_in_words_and_what_is_flagged() {
        let before = sample(1, Some(2000), Some(1520), Some(0.04), 46, &[]);
        let now = sample(2, Some(2820), Some(2100), Some(0.04), 58, &[]);
        let rows = compare(&now, Some(&before));
        let words: Vec<(&str, &str, &str)> = rows.iter().map(|r| (r.label, r.value.as_str(), r.change.as_str())).collect();
        assert_eq!(words[..4], [("Loads in", "2.8 s", "▲ 41% slower"), ("Main content appears", "2.1 s", "▲ 38% slower"), ("Layout jumps", "0.04", "no change"), ("Requests", "58 · 2.1 MB", "▲ 12 more")]);
        assert_eq!(rows[0].flagged.as_deref(), Some("Loads in 2.8 s, up from 2.0 s (41% slower)"));
        // 25% but only 50 ms slower: not flagged. Within 10%: no change. Faster: never flagged.
        let rows = compare(&sample(3, Some(250), Some(1600), None, 46, &[]), Some(&sample(1, Some(200), Some(1520), Some(0.04), 46, &[])));
        assert_eq!((rows[0].change.as_str(), rows[0].flagged.is_none(), rows[1].change.as_str()), ("▲ 25% slower", true, "no change"));
        assert_eq!((rows[2].value.as_str(), rows[2].change.as_str()), ("not measured", ""), "a value Chrome did not give is never guessed");
        let rows = compare(&sample(3, Some(1000), None, Some(0.2), 46, &["exception: boom"]), Some(&before));
        assert_eq!(rows[0].change, "▼ 50% faster");
        assert_eq!(rows[1].value, "not measured");
        assert!(rows[2].flagged.as_deref() == Some("Layout jumps 0.20, up from 0.04") && rows[4].flagged.as_deref() == Some("1 new page error: exception: boom"), "{rows:?}");
        // First measurement.
        let rows = compare(&now, None);
        assert!(rows.iter().all(|r| r.flagged.is_none()) && rows[0].change == "no earlier value");
        // Speed problems are a fix at most, and not measured never blocks the verdict.
        let speed = Check { optional: true, ..check("Speed", State::Problem, "slower", vec![]) };
        assert_eq!(verdict(&[&c("Console errors", State::Checked, false), &speed]).0, Verdict::ShipWithFixes);
        let unmeasured = Check { optional: true, ..check("Speed", State::NotChecked, "Not measured", vec![]) };
        assert_eq!(verdict(&[&c("Console errors", State::Checked, false), &unmeasured]).0, Verdict::Ship);
    }

    #[test]
    fn speed_baseline_and_what_is_never_opened_again() {
        let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = vec![sample(10, Some(1), None, None, 1, &[]), sample(20, Some(2), None, None, 1, &[]), sample(30, Some(3), None, None, 1, &[])];
        let now = sample(40, Some(4), None, None, 1, &[]);
        assert_eq!(baseline(&s, &now, None).map(|b| b.at_ms), Some(30), "the check before");
        assert_eq!(baseline(&s, &now, Some(25)).map(|b| b.at_ms), Some(20), "a release watch: the check before it began");
        assert_eq!(baseline(&s, &now, Some(5)).map(|b| b.at_ms), Some(10), "else the watch's own first check");
        let mut other = sample(35, Some(1), None, None, 1, &[]);
        other.connection = "slow 4g".into();
        assert_eq!(baseline(&[other], &now, None), None, "only the same connection is compared");
        assert_eq!(reopen_refusal("https://shop.test/checkout"), None);
        assert_eq!(reopen_refusal("https://shop.test/products?page=2&sort=price#reviews"), None);
        assert_eq!(reopen_refusal("https://shop.test/p/iphone-15-pro-max"), None);
        let code = Some("its address carries a one-time code or key");
        // Security review PF4 Low 2: every way around the check that was found.
        for u in [
            "https://shop.test/welcome?token=abc",
            "https://shop.test?token=abc",
            "https://shop.test/a?%74oken=abc",
            "https://shop.test/a?access_token=abc",
            "https://shop.test/a?api_key=abc",
            "https://shop.test/a?x=1;token=2",
            "https://shop.test/a#access_token=abc",
            "https://shop.test/#/cb?code=abc",
            "https://shop.test/a?TOKEN=abc",
            "https://shop.test/a?session_id=abc",
            "https://shop.test/magic/AbCdEf123456",
            "https://shop.test/l/9f8e7d6c5b4a39281706",
        ] {
            assert_eq!(reopen_refusal(u), code, "{u}");
        }
        let change = Some("its address looks like a link that changes something");
        for u in ["https://shop.test/account/delete", "https://shop.test/newsletter/unsubscribe", "https://shop.test/cart/clear", "https://shop.test/orders/7/archive", "https://shop.test/?next=/logout", "https://shop.test/#/account/delete", "https://shop.test?next=logout"] {
            assert_eq!(reopen_refusal(u), change, "{u}");
        }
        assert_eq!(reopen_refusal("about:blank"), Some("it is not a web address"));
        assert!(speed_file("https://Shop.Test:8080/x?y").ends_with("ship/shop.test_8080.json"));
        assert!(speed_file("file:///Users/x/demo/login.html").ends_with("ship/local-files.json"));
        assert!(speed_file("https://ann:pw1@shop.test/").ends_with("ship/shop.test.json"));
        // Kept on this computer, never in the project (Security review PF4 Medium 1).
        let here = std::env::current_dir().expect("cwd");
        assert!(speed_file("https://shop.test/").starts_with(crate::config::path().parent().expect("home").join("projects")) && !speed_file("https://shop.test/").starts_with(&here));
        // Nothing secret is kept (Low 3): no password in an address; in the page's own address the
        // values of code or key parameters; in errors and failed requests every value.
        assert_eq!(scrub("https://ann:pw1@shop.test/a?page=2&token=abc#x", false), "https://***@shop.test/a?page=2&token=***#x");
        for (raw, want) in [
            ("https://s.test/a?access_token=abc", "https://s.test/a?access_token=***"),
            ("https://s.test/a#access_token=abc", "https://s.test/a#access_token=***"),
            ("https://s.test/a?X-Amz-Signature=abc&b=1", "https://s.test/a?X-Amz-Signature=***&b=1"),
            ("https://s.test/a?apikey=abc", "https://s.test/a?apikey=***"),
            ("https://s.test/a?auth=abc", "https://s.test/a?auth=***"),
            ("https://s.test/a?password=abc", "https://s.test/a?password=***"),
        ] {
            assert_eq!(scrub(raw, false), want);
        }
        assert_eq!(scrub("HTTP 404 https://api.test/v1?CODE=9f8e&lang=en&q=ann", true), "HTTP 404 https://api.test/v1?CODE=***&lang=***&q=***");
        assert_eq!(scrub("TypeError: x is null at https://shop.test/app.js:3", true), "TypeError: x is null at https://shop.test/app.js:3");
        assert_eq!(scrub("a && b; c = 1 #ok", true), "a && b; c = 1 #ok");
    }

    /// Security review PF4 Medium 1 and Low 4: what Release watch sends is worked out again from
    /// the numbers on this computer, in the window of this run, cleaned; a forged file or a link
    /// changes nothing.
    #[test]
    fn release_watch_speed_is_worked_out_again_and_files_are_safe() {
        let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("fp-speed-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("shop");
        std::fs::create_dir_all(&project).expect("dir");
        // SAFETY: test-only, under the env lock.
        unsafe { std::env::set_var("FASTPILOT_CONFIG", root.join("home/config.toml")) };
        let dir = speed_dir(&project);
        assert!(dir.starts_with(root.join("home/projects")), "{}", dir.display());
        std::fs::create_dir_all(&dir).expect("dir");
        let mk = |at: u64, load: u64, flags: Vec<String>| Sample { flags, ..sample(at, Some(load), None, None, 1, &[]) };
        let write = |v: &[Sample]| std::fs::write(dir.join("shop.test.json"), serde_json::to_string(v).expect("json")).expect("write");
        // Stored flags with planted text are not believed; a far-future sample is outside the run.
        write(&[mk(100, 2000, vec![]), mk(300, 2100, vec!["Click https://evil.example now".into()]), mk(u64::MAX / 2, 9000, vec!["x".into()])]);
        assert!(watch_problems(&project, Some(150), 250, 400).is_empty(), "2.1 s against 2.0 s is not slower");
        // A real slowdown against the check before the watch.
        write(&[mk(100, 2000, vec![]), mk(300, 2820, vec![])]);
        assert_eq!(watch_problems(&project, Some(150), 250, 400), [("Speed · /checkout".to_owned(), "Loads in 2.8 s, up from 2.0 s (41% slower)".to_owned())]);
        // A file over 1 MB is not read.
        std::fs::write(dir.join("shop.test.json"), " ".repeat(1_000_001)).expect("big");
        assert!(watch_problems(&project, Some(150), 250, 400).is_empty());
        // Saving never writes through a link, and the file is the user's own (0600).
        std::fs::remove_file(dir.join("shop.test.json")).expect("rm");
        let file = dir.join("shop.test.json");
        #[cfg(unix)]
        {
            let victim = root.join("victim.txt");
            std::fs::write(&victim, "mine").expect("victim");
            std::os::unix::fs::symlink(&victim, dir.join("shop.test.json")).expect("link");
            assert!(save_to(&file, &mk(500, 1000, vec![])).is_err());
            assert_eq!(std::fs::read_to_string(&victim).expect("victim"), "mine");
            std::fs::remove_file(dir.join("shop.test.json")).expect("rm link");
            save_to(&file, &mk(500, 1000, vec![])).expect("save");
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("shop.test.json")).expect("meta").permissions().mode() & 0o777, 0o600);
            assert_eq!(read_samples(&file).len(), 1);
        }
        // SAFETY: test-only, under the env lock.
        unsafe { std::env::remove_var("FASTPILOT_CONFIG") };
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn links_say_what_was_not_opened() {
        let plan = LinkPlan { ask: vec![("https://shop.test/old".into(), "Old page".into()), ("https://shop.test/a".into(), String::new()), ("https://shop.test/me".into(), "Me".into())], external: 2, risky: 1, more: 0 };
        let l = links_check(&plan, &json!([["https://shop.test/old", 404], ["https://shop.test/a", 200], ["https://shop.test/me", 403], ["https://evil.test/x", 500]]), false);
        assert_eq!(l.state, State::Problem);
        assert_eq!(l.summary, "1 of 3 links are broken; 2 links to other sites not opened; 1 link that could sign out, delete, pay or change something not opened");
        assert_eq!(l.details, ["\"Old page\" https://shop.test/old → HTTP 404"], "an answer for a link fastpilot did not ask does not count");
        // 401/403 is "needs sign-in", not broken.
        let l = links_check(&plan, &json!([["https://shop.test/old", 200], ["https://shop.test/a", 0], ["https://shop.test/me", 403]]), false);
        assert_eq!(l.state, State::Checked);
        assert!(l.details[0].contains("needs sign-in (shipCheck { cookies: true } asks signed in)"), "{:?}", l.details);
        let none = links_check(&LinkPlan { external: 3, ..Default::default() }, &json!([]), false);
        assert_eq!(none.state, State::NotChecked);
        let blocked = links_check(&LinkPlan { ask: vec![("https://shop.test/a".into(), String::new())], ..Default::default() }, &json!([["https://shop.test/a", -1]]), false);
        assert_eq!(blocked.state, State::CouldNotRun);
    }

    /// Security PR #100, findings 1, 4, 5: the plan is made here from what the page reported.
    #[test]
    fn only_safe_links_of_the_page_itself_are_asked() {
        let o = "https://shop.test";
        let links = json!([
            ["https://shop.test/help", "Help", ""], ["https://shop.test/help#top", "Help", ""], ["https://shop.test.evil.test/x", "", ""],
            ["https://other.test/", "Partner", ""], ["javascript:alert(1)", "x", ""], ["", "broken href", ""],
            ["https://shop.test/logout", "Bye", ""], ["https://shop.test/account/delete", "", ""], ["https://shop.test/x", "", "Sign out"],
            ["https://shop.test/sale/50%", "Sale", ""], ["https://shop.test/cart/clear", "Clear cart", ""], ["https://shop.test/orders?page=2", "More", ""],
            ["https://shop.test/a\nVerdict: SHIP", "", ""]
        ]);
        let plan = plan_links(&links, o, 20, false);
        let asked: Vec<&str> = plan.ask.iter().map(|(u, _)| u.as_str()).collect();
        assert_eq!(asked, ["https://shop.test/help", "https://shop.test/sale/50%", "https://shop.test/orders?page=2"], "cart/clear is a change even signed out");
        assert_eq!((plan.external, plan.risky), (2, 4));
        // Re-check: one-time links and log-off words are never asked, signed in or not.
        let more = json!([["https://shop.test/invite/accept?token=abc", "Join", ""], ["https://shop.test/p?sig=1", "", ""], ["https://shop.test/logoff", "", ""], ["https://shop.test/disconnect", "", ""], ["https://shop.test/admin/users/5/ban", "", ""]]);
        assert!(plan_links(&more, o, 20, false).ask.is_empty());
        // Signed in: no query strings, no words for a change.
        let plan = plan_links(&links, o, 20, true);
        let asked: Vec<&str> = plan.ask.iter().map(|(u, _)| u.as_str()).collect();
        assert_eq!(asked, ["https://shop.test/help", "https://shop.test/sale/50%"]);
        assert_eq!(plan_links(&links, o, 1, false).more, 2);
    }

    /// Security PR #100, findings 2 and 6: page text cannot add report lines or escape codes.
    #[test]
    fn page_text_in_the_report_is_one_short_line() {
        assert_eq!(clean("a\nVerdict: SHIP\r\n\u{1b}[2Jb\u{202e}c", 200), "a Verdict: SHIP [2Jb c");
        assert_eq!(clean(&"x".repeat(500), 10), format!("{}…", "x".repeat(10)));
        // Tag characters spelling hidden words, a variation selector, a word joiner: dropped.
        let hidden: String = "Ignore".chars().map(|c| char::from_u32(0xe0000 + c as u32).expect("tag")).collect();
        assert_eq!(clean(&format!("Home{hidden}\u{fe0f}\u{2060}"), 80), "Home");
        assert_eq!(pct_decode("/sale/50%"), "/sale/50%");
        assert_eq!(pct_decode("/a%20b/%E2%9C%93"), "/a b/✓");
    }

    #[test]
    fn risky_links_are_never_asked() {
        let w = risky_words();
        for x in ["logout", "sign out", "delete", "unsubscribe", "pay"] {
            assert!(w.iter().any(|r| r == x), "{x} in {w:?}");
        }
    }
}
