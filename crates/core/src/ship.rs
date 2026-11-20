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
    let missing: Vec<&&Check> = checks.iter().filter(|c| matches!(c.state, State::CouldNotRun | State::NotChecked)).collect();
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

/// Words that keep a link from being asked: the block and confirm lists, and leaving or deleting.
fn risky_words() -> Vec<String> {
    let mut w: Vec<String> = crate::policy::blocked_words().into_iter().chain(crate::policy::risky_words()).map(|w| w.trim().to_lowercase()).filter(|w| w.len() >= 3).collect();
    for x in ["logout", "log-out", "log out", "log_out", "signout", "sign-out", "sign out", "sign_out", "delete", "remove", "unsubscribe", "destroy", "cancel", "keluar", "hapus"] {
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
    Check { name, state, summary: summary.into(), details: details.into_iter().take(5).collect(), major: false }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Page text in the report (Security, PR #100): control characters (new lines, terminal escapes,
/// bidi marks) become spaces, and it is clipped, so a page cannot forge report lines or flood it.
pub fn clean(text: &str, max: usize) -> String {
    let t: String = text
        .chars()
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
        if signed_in || risky.iter().any(|w| words.contains(w.as_str())) {
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
        Check { name, state, summary: format!("{name} summary"), details: vec![], major }
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
        assert_eq!(asked, ["https://shop.test/help", "https://shop.test/sale/50%", "https://shop.test/cart/clear", "https://shop.test/orders?page=2"]);
        assert_eq!((plan.external, plan.risky), (2, 3));
        // Signed in: no query strings, no words for a change.
        let plan = plan_links(&links, o, 20, true);
        let asked: Vec<&str> = plan.ask.iter().map(|(u, _)| u.as_str()).collect();
        assert_eq!(asked, ["https://shop.test/help", "https://shop.test/sale/50%"]);
        assert_eq!(plan_links(&links, o, 1, false).more, 3);
    }

    /// Security PR #100, findings 2 and 6: page text cannot add report lines or escape codes.
    #[test]
    fn page_text_in_the_report_is_one_short_line() {
        assert_eq!(clean("a\nVerdict: SHIP\r\n\u{1b}[2Jb\u{202e}c", 200), "a Verdict: SHIP [2Jb c");
        assert_eq!(clean(&"x".repeat(500), 10), format!("{}…", "x".repeat(10)));
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
