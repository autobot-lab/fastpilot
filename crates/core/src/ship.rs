//! R2 Ship check (v0.4): the health of one web page before a release, with honest "not checked".
//!
//! `- shipCheck` (or `{ links: 20 }`) on a web session looks at the page that is open now:
//! - **Console errors**: exceptions and `console.error` the page logged since the last report;
//! - **Failed requests**: HTTP errors and requests that did not finish;
//! - **Broken links**: up to `links` links to the page's own site, asked with `HEAD` from the page
//!   (its cookies), redirects not followed. Links to other sites are counted, not opened. Links whose
//!   text or address names a risky action (the block and confirm lists, sign out, delete,
//!   unsubscribe, ...) are never requested;
//! - **Layout** at three screen sizes (phone 390, tablet 768, desktop 1280): content wider than
//!   the screen. The size is put back afterwards;
//! - **Accessibility basics**: page language and title, images without `alt`, fields, buttons
//!   and links without a name, repeated ids. Colour contrast and keyboard use: not checked.
//!
//! It only reads the page and asks `HEAD` of same-site links; it never clicks, types or submits.
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

/// Same-site links, asked with HEAD (never risky ones); other sites counted only.
const LINKS_JS: &str = r#"(async (max, risky) => {
  const out = { checked: 0, broken: [], unchecked: [], external: 0, risky: 0, more: 0 };
  const seen = new Set(), todo = [];
  for (const a of document.querySelectorAll('a[href]')) {
    let u; try { u = new URL(a.getAttribute('href'), location.href); } catch (e) { continue; }
    if (u.protocol !== 'http:' && u.protocol !== 'https:') continue;
    u.hash = '';
    if (seen.has(u.href)) continue;
    seen.add(u.href);
    if (u.origin !== location.origin) { out.external++; continue; }
    const text = ((a.innerText || '') + ' ' + (a.getAttribute('aria-label') || '') + ' ' + decodeURIComponent(u.pathname + u.search)).toLowerCase();
    if (risky.some((w) => text.includes(w))) { out.risky++; continue; }
    todo.push({ url: u.href, text: (a.innerText || a.getAttribute('aria-label') || '').trim().replace(/\s+/g, ' ').slice(0, 60) });
  }
  out.more = Math.max(0, todo.length - max);
  await Promise.all(todo.slice(0, max).map(async (l) => {
    const stop = new AbortController(), t = setTimeout(() => stop.abort(), 8000);
    try {
      // Redirects are not followed: they could lead outside allowed_domains. A redirect answers.
      const r = await fetch(l.url, { method: 'HEAD', credentials: 'same-origin', redirect: 'manual', cache: 'no-store', signal: stop.signal });
      if (r.type === 'opaqueredirect') out.checked++;
      else if (r.status === 405 || r.status === 501) out.unchecked.push({ ...l, why: 'the server does not answer HEAD' });
      else if (r.status >= 400) out.broken.push({ ...l, status: r.status });
      else out.checked++;
    } catch (e) { out.unchecked.push({ ...l, why: 'could not be reached from the page' }); }
    finally { clearTimeout(t); }
  }));
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

/// Run every check on the page open in `d`.
pub async fn run(d: &dyn Driver, links: usize) -> Result<Report> {
    if d.platform() != Platform::Web {
        return Err(Error::Unsupported("shipCheck is for web pages (open the page in a web session)".into()));
    }
    let page = d.eval_script(PAGE_JS).await?;
    let url = page["url"].as_str().unwrap_or("").to_owned();
    let mut checks = Vec::new();

    // Console errors and failed requests seen since the last report (the page's own first).
    let diag = d.diagnostics().await;
    let (console, requests): (Vec<String>, Vec<String>) = diag.into_iter().partition(|l| l.starts_with("exception:") || l.starts_with("console.error:"));
    let exceptions = console.iter().filter(|l| l.starts_with("exception:")).count();
    let mut c = if console.is_empty() {
        check("Console errors", State::Checked, "No errors", vec![])
    } else {
        let crash = if exceptions > 0 { format!(", {exceptions} uncaught: the page's own script crashed") } else { String::new() };
        check("Console errors", State::Problem, format!("{}{crash}", plural(console.len(), "error", "errors")), console.clone())
    };
    c.major = exceptions > 0;
    checks.push(c);
    let status = page["status"].as_u64().unwrap_or(0);
    let mut r = if status >= 400 {
        check("Failed requests", State::Problem, format!("The page itself answered HTTP {status}"), requests.clone())
    } else if requests.is_empty() {
        check("Failed requests", State::Checked, "No failed requests", vec![])
    } else {
        check("Failed requests", State::Problem, plural(requests.len(), "request failed", "requests failed"), requests.clone())
    };
    r.major = status >= 400;
    checks.push(r);

    // Links.
    if links == 0 {
        checks.push(check("Broken links", State::NotChecked, "Not checked (links: 0)", vec![]));
    } else {
        let js = format!("{LINKS_JS}({}, {})", links.min(MAX_LINKS), json!(risky_words()));
        checks.push(match d.eval_script(&js).await {
            Ok(v) => links_check(&v),
            Err(e) => check("Broken links", State::CouldNotRun, format!("Could not run: {e}"), vec![]),
        });
    }

    // Layout at three sizes; the size is put back afterwards.
    let before = Viewport {
        width: page["width"].as_u64().unwrap_or(1280) as u32,
        height: page["height"].as_u64().unwrap_or(800) as u32,
        scale: page["scale"].as_f64().unwrap_or(1.0),
        mobile: page["width"].as_u64().unwrap_or(1280) < 800,
        touch: page["width"].as_u64().unwrap_or(1280) < 800,
        user_agent: None,
    };
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
                wide.push(format!("{name} ({w} px): laid out {} px wide and shown zoomed out (no mobile viewport: <meta name=viewport content=\"width=device-width\">)", l["width"]));
            }
            Ok(l) if l["over"].as_u64().unwrap_or(0) > 1 => {
                let what: Vec<String> = l["wide"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect();
                wide.push(format!("{name} ({w} px): content is {} px wider than the screen{}", l["over"], if what.is_empty() { String::new() } else { format!(" ({})", what.join(", ")) }));
            }
            Ok(_) => {}
            Err(e) => {
                failed = Some(format!("{name}: {e}"));
                break;
            }
        }
    }
    let restored = d.set_viewport(Some(&before)).await;
    checks.push(match (failed, wide.is_empty()) {
        (Some(e), _) => check("Layout at 3 sizes", State::CouldNotRun, format!("Could not run: {e}"), wide),
        (None, true) => check("Layout at 3 sizes", State::Checked, "Phone 390, tablet 768 and desktop 1280 px: nothing wider than the screen", vec![]),
        (None, false) => check("Layout at 3 sizes", State::Problem, format!("Wider than the screen at {}", plural(wide.len(), "size", "sizes")), wide),
    });
    restored?;

    // Accessibility basics.
    checks.push(match d.eval_script(A11Y_JS).await {
        Ok(v) => {
            let issues: Vec<(String, Vec<String>)> = v
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| (i["what"].as_str().unwrap_or("").to_owned(), i["examples"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect()))
                .collect();
            if issues.is_empty() {
                check("Accessibility basics", State::Checked, "Language, title, alt text, labels, names and ids are fine", vec![])
            } else {
                let details = issues.iter().map(|(w, ex)| if ex.is_empty() { w.clone() } else { format!("{w}: {}", ex.join(", ")) }).collect();
                check("Accessibility basics", State::Problem, plural(issues.len(), "problem", "problems"), details)
            }
        }
        Err(e) => check("Accessibility basics", State::CouldNotRun, format!("Could not run: {e}"), vec![]),
    });

    let (verdict, why) = verdict(&checks.iter().collect::<Vec<_>>());
    Ok(Report { url, checks, verdict, why })
}

fn links_check(v: &Value) -> Check {
    let n = |k: &str| v[k].as_u64().unwrap_or(0) as usize;
    let list = |k: &str| v[k].as_array().cloned().unwrap_or_default();
    let broken = list("broken");
    let unchecked = list("unchecked");
    let asked = n("checked") + broken.len() + unchecked.len();
    let mut notes = Vec::new();
    if n("external") > 0 {
        notes.push(format!("{} to other sites not opened", plural(n("external"), "link", "links")));
    }
    if n("risky") > 0 {
        notes.push(format!("{} that sign out, delete or pay not opened", plural(n("risky"), "link", "links")));
    }
    if n("more") > 0 {
        notes.push(format!("{} more not asked (limit)", n("more")));
    }
    let tail = if notes.is_empty() { String::new() } else { format!("; {}", notes.join("; ")) };
    let name = |l: &Value| {
        let t = l["text"].as_str().unwrap_or("");
        let u = l["url"].as_str().unwrap_or("");
        if t.is_empty() { u.to_owned() } else { format!("\"{t}\" {u}") }
    };
    if !broken.is_empty() {
        let details = broken.iter().map(|l| format!("{} → HTTP {}", name(l), l["status"])).collect();
        return check("Broken links", State::Problem, format!("{} of {asked} links are broken{tail}", broken.len()), details);
    }
    if asked == 0 {
        return check("Broken links", State::NotChecked, format!("No links to this site on the page{tail}"), vec![]);
    }
    if !unchecked.is_empty() && unchecked.len() == asked {
        let details = unchecked.iter().map(|l| format!("{}: {}", name(l), l["why"].as_str().unwrap_or(""))).collect();
        return check("Broken links", State::CouldNotRun, format!("None of the {asked} links could be asked{tail}"), details);
    }
    let details = unchecked.iter().map(|l| format!("not checked: {}: {}", name(l), l["why"].as_str().unwrap_or(""))).collect();
    check("Broken links", State::Checked, format!("{} of {asked} links answer{tail}", n("checked")), details)
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
        let v = json!({ "checked": 3, "broken": [{ "url": "https://shop.test/old", "text": "Old page", "status": 404 }], "unchecked": [], "external": 2, "risky": 1, "more": 0 });
        let l = links_check(&v);
        assert_eq!(l.state, State::Problem);
        assert_eq!(l.summary, "1 of 4 links are broken; 2 links to other sites not opened; 1 link that sign out, delete or pay not opened");
        assert_eq!(l.details, ["\"Old page\" https://shop.test/old → HTTP 404"]);
        let none = links_check(&json!({ "checked": 0, "broken": [], "unchecked": [], "external": 3, "risky": 0, "more": 0 }));
        assert_eq!(none.state, State::NotChecked);
        let blocked = links_check(&json!({ "checked": 0, "broken": [], "unchecked": [{ "url": "https://shop.test/a", "text": "", "why": "could not be reached from the page" }], "external": 0, "risky": 0, "more": 0 }));
        assert_eq!(blocked.state, State::CouldNotRun);
    }

    #[test]
    fn risky_links_are_never_asked() {
        let w = risky_words();
        for x in ["logout", "sign out", "delete", "unsubscribe", "pay"] {
            assert!(w.iter().any(|r| r == x), "{x} in {w:?}");
        }
    }
}
