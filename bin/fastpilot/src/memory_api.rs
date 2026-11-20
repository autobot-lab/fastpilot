//! The Memory page of `fastpilot ui` (D6, v0.4): what fastpilot has learned about each test of this
//! project, in plain words, and the two things a person can do about it:
//! - **Set aside** (quarantine): the test still runs and is shown, but its failure does not turn
//!   the run red. **Bring back** undoes it.
//! - **Forget**: drop the remembered path and statistics; the next run works the steps out again.
//!   The test file is not touched.
//!
//! Only the page's own buttons change anything (the dashboard's guard: cookie, JSON from our own
//! origin); the Chat bot cannot.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use fastpilot_memory::{Memory, Mode, PathMemory, Quarantine};
use serde::Serialize;
use serde_json::{Value, json};

fn refuse(status: StatusCode, why: &str) -> Response {
    let mut r = (status, format!("fastpilot ui: {why}\n")).into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    r
}

/// One row: a test and what is known about it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    /// `web`, `android`, … from the test file (empty when the file is gone).
    pub platform: String,
    /// `reliable`, `unstable`, `failing`, `set_aside` or `new`.
    pub status: &'static str,
    /// What fastpilot knows, in one sentence.
    pub summary: String,
    /// Steps of the remembered path.
    pub steps: usize,
    /// Screens of the remembered path, in order ("Login" → "Dashboard").
    pub path: Vec<String>,
    /// When it last passed (ms since 1970, UTC), if ever.
    pub last_worked_ms: Option<u64>,
    /// Failures in the last runs, and how many runs that is (up to 10).
    pub recent_fails: usize,
    pub recent_runs: usize,
    pub set_aside_since_ms: Option<u64>,
    /// fastpilot set it aside by itself (it flips between pass and fail).
    pub set_aside_auto: bool,
    /// The test file is gone; only its memory is left.
    pub orphan: bool,
}

/// `2026-11-17 10:00:00` (UTC, as the memory files store it) → ms since 1970.
fn stamp_ms(s: &str) -> Option<u64> {
    let (date, time) = s.trim().split_once(' ')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|x| x.parse::<i64>().ok());
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(((days * 86_400) + hh * 3600 + mm * 60 + ss) * 1000).ok()
}

/// `2026-11-17 10:00:00` → `17 Nov 2026`.
fn day_month(s: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let date = s.split(' ').next().unwrap_or(s);
    let p: Vec<&str> = date.split('-').collect();
    match (p.first(), p.get(1).and_then(|m| m.parse::<usize>().ok()).filter(|m| (1..=12).contains(m)), p.get(2).and_then(|d| d.parse::<u32>().ok())) {
        (Some(y), Some(m), Some(d)) => format!("{d} {} {y}", MONTHS[m - 1]),
        _ => date.to_owned(),
    }
}

/// `screen "Login" (https://…)` → `Login`.
fn screen_name(header: &str) -> String {
    header
        .split_once('"')
        .and_then(|(_, rest)| rest.rsplit_once('"').map(|(t, _)| t.to_owned()))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| header.trim().to_owned())
}

fn steps_of(flow: &str) -> usize {
    flow.lines().filter(|l| l.trim_start().starts_with("- ")).count()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The row for one test (`m`: its memory, `q`: set aside).
pub fn row(id: &str, title: &str, platform: &str, m: Option<&PathMemory>, q: Option<&Quarantine>, orphan: bool) -> Row {
    let recent: Vec<bool> = m.map(|m| m.history.iter().rev().take(10).copied().collect()).unwrap_or_default();
    let recent_fails = recent.iter().filter(|p| !**p).count();
    let steps = m.map_or(0, |m| steps_of(&m.flow));
    let remembers = if steps > 0 { format!("Remembers the last path that worked ({}).", plural(steps, "step", "steps")) } else { String::new() };
    let (status, summary) = if let Some(q) = q {
        // The stamp is UTC; the person reads their own day (FastTest M3: "20 Nov" next to "Today").
        let since = stamp_ms(&q.since).map_or_else(|| day_month(&q.since), fastpilot_core::claim::local_date);
        let who = if q.auto { "Set aside by fastpilot because it passes and fails without a change" } else { "Set aside" };
        ("set_aside", format!("{who} since {since}. It still runs, but does not turn a run red."))
    } else if m.is_none_or(|m| m.history.is_empty() && m.flow.is_empty()) {
        ("new", "Nothing remembered yet. It remembers after the first pass.".to_owned())
    } else if m.is_some_and(|m| m.stale || (m.consecutive_failures >= 3)) {
        let n = m.map_or(0, |m| m.consecutive_failures);
        ("failing", format!("Failed the last {n} runs. The remembered path is not used until it passes again."))
    } else if recent_fails > 0 && recent_fails < recent.len() {
        ("unstable", format!("{remembers} Failed {recent_fails} of the last {} runs.", recent.len()).trim().to_owned())
    } else if recent_fails > 0 && steps == 0 {
        ("failing", format!("Failed the last {}. Nothing remembered yet: it remembers after the first pass.", plural(recent_fails, "run", "runs")))
    } else {
        ("reliable", if remembers.is_empty() { "Passed its recent runs.".to_owned() } else { remembers })
    };
    Row {
        id: id.to_owned(),
        title: title.to_owned(),
        platform: platform.to_owned(),
        status,
        summary,
        steps,
        path: m.map(|m| m.screens.iter().map(|s| screen_name(s)).collect()).unwrap_or_default(),
        last_worked_ms: m.and_then(|m| stamp_ms(&m.last_pass)),
        recent_fails,
        recent_runs: recent.len(),
        set_aside_since_ms: q.and_then(|q| stamp_ms(&q.since)),
        set_aside_auto: q.is_some_and(|q| q.auto),
        orphan,
    }
}

/// Every test of the project (`tests/*.md`) and every test something is remembered for.
pub fn rows(memory: &Memory) -> Vec<Row> {
    let cases = crate::run::load(&crate::chat::tests_dir()).unwrap_or_default();
    let remembered = memory.remembered();
    let quarantined = memory.quarantined();
    let mut out: Vec<Row> = cases
        .iter()
        .map(|c| row(&c.id, &c.title, &c.platform, remembered.iter().find(|m| m.test_id == c.id), quarantined.get(&c.id), false))
        .collect();
    for m in &remembered {
        if !cases.iter().any(|c| c.id == m.test_id) {
            out.push(row(&m.test_id, &m.test_id, "", Some(m), quarantined.get(&m.test_id), true));
        }
    }
    for (id, q) in &quarantined {
        if !out.iter().any(|r| &r.id == id) {
            out.push(row(id, id, "", None, Some(q), true));
        }
    }
    out
}

/// `GET /api/memory`.
pub async fn list() -> Response {
    let memory = Memory::from_env();
    let mode = match memory.mode {
        Mode::ReadWrite => "on",
        Mode::ReadOnly => "read-only",
        Mode::Off => "off",
    };
    // Off: nothing is read or written, so the page says so instead of showing every test as new.
    Json(json!({ "tests": rows(&memory), "mode": mode })).into_response()
}

/// `POST /api/memory`: `{action: set_aside|bring_back|forget, id}`.
pub async fn change(Json(body): Json<Value>) -> Response {
    let memory = Memory::from_env();
    let id = body["id"].as_str().unwrap_or("");
    let done = match body["action"].as_str().unwrap_or("") {
        "set_aside" => memory.set_quarantine(id, Some("set aside on the Memory page")).map(|()| "Set aside: it still runs, but a failure does not turn the run red."),
        "bring_back" => memory.set_quarantine(id, None).map(|()| "Brought back: its failures count again."),
        "forget" => memory.forget(id).map(|had| if had { "Forgotten: the next run works the steps out again." } else { "There was nothing to forget." }),
        _ => return refuse(StatusCode::BAD_REQUEST, "Unknown Memory action."),
    };
    match done {
        Ok(text) => Json(json!({ "ok": text, "tests": rows(&memory) })).into_response(),
        Err(e) => refuse(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

// ---- project notes (D6 stage 2) ------------------------------------------------------------

fn notes_state(notes: &fastpilot_memory::notes::Notes) -> Value {
    use fastpilot_memory::notes::{GROUPS, MAX_CHARS, MAX_NOTES, MAX_TOTAL};
    let l = notes.read();
    json!({
        "notes": l.notes,
        "damaged": l.damaged,
        "groups": GROUPS.iter().map(|(k, l)| json!({ "key": k, "label": l })).collect::<Vec<_>>(),
        "limits": { "notes": MAX_NOTES, "chars": MAX_CHARS, "total": MAX_TOTAL },
        "used": notes.total(),
    })
}

/// `GET /api/notes`.
pub async fn notes_list() -> Response {
    Json(notes_state(&fastpilot_memory::notes::Notes::from_env())).into_response()
}

/// `POST /api/notes`: `{action: add, group, text}`, `{action: edit, id, group, text}`,
/// `{action: remove, id}`. Only the page's own form sends these: a note is the person's own
/// context for every agent, so nothing else (the Chat bot, a tested page) can add one.
pub async fn notes_change(Json(body): Json<Value>) -> Response {
    let notes = fastpilot_memory::notes::Notes::from_env();
    let s = |k: &str| body[k].as_str().unwrap_or("").to_owned();
    // Secrets this process does not have in its environment: the values the project's .mcp.json
    // gives fastpilot (FASTPILOT_VARS names in its env), as `fastpilot demo` writes them.
    if matches!(s("action").as_str(), "add" | "edit") && mcp_secret_values().iter().any(|v| s("text").contains(v.as_str())) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "kind": "secret", "error": "This contains the value of a saved secret. Notes are not hidden, so it was not saved. Save it as a secret instead and write only its name." })),
        )
            .into_response();
    }
    let done = match s("action").as_str() {
        "add" => notes.add(&s("group"), &s("text")).map(|_| "Note saved. Agents use it from the next run."),
        "edit" => notes.edit(&s("id"), &s("group"), &s("text")).map(|_| "Note saved. Agents use it from the next run."),
        "remove" => notes.remove(&s("id")).map(|()| "Note removed. Agents stop using it from the next run."),
        _ => return refuse(StatusCode::BAD_REQUEST, "Unknown notes action."),
    };
    match done {
        Ok(text) => {
            let mut v = notes_state(&notes);
            v["ok"] = json!(text);
            Json(v).into_response()
        }
        // The page shows the matching state (it looks like a password, the limit, …).
        Err(r) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": r.message, "kind": r.kind }))).into_response(),
    }
}

/// Secret values in the project's `.mcp.json` (fastpilot server entry, FASTPILOT_VARS names).
fn mcp_secret_values() -> Vec<String> {
    let Some(v) = std::fs::read_to_string(".mcp.json").ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return vec![] };
    let env = &v["mcpServers"]["fastpilot"]["env"];
    let names = env["FASTPILOT_VARS"].as_str().unwrap_or("");
    names.split(',').map(str::trim).filter(|n| !n.is_empty()).filter_map(|n| env[n].as_str()).filter(|v| v.chars().count() >= 3).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(history: &[bool], flow: &str) -> PathMemory {
        PathMemory {
            test_id: "t".into(),
            flow: flow.into(),
            screens: vec!["screen \"Shop\" (https://s.example)".into(), "screen \"Cart\" (https://s.example/cart)".into()],
            history: history.to_vec(),
            consecutive_failures: history.iter().rev().take_while(|p| !**p).count() as u32,
            last_pass: "2026-11-20 07:02:00".into(),
            ..PathMemory::default()
        }
    }

    #[test]
    fn statuses_and_sentences() {
        let flow = "- tapOn: Shop\n- tapOn: Add\n- inputText: { into: Voucher, text: SAVE10 }\n- assertVisible: Discount\n";
        let r = row("t", "Checkout", "web", Some(&mem(&[true, true, true], flow)), None, false);
        assert_eq!((r.status, r.summary.as_str(), r.steps), ("reliable", "Remembers the last path that worked (4 steps).", 4));
        assert_eq!(r.path, ["Shop", "Cart"]);
        assert_eq!(r.last_worked_ms, Some(1_795_158_120_000), "2026-11-20 07:02:00 UTC");
        let r = row("t", "Checkout", "web", Some(&mem(&[true, false, true, true, false, true, true, true, true, false, true], flow)), None, false);
        assert_eq!((r.status, r.recent_fails, r.recent_runs), ("unstable", 3, 10));
        assert_eq!(r.summary, "Remembers the last path that worked (4 steps). Failed 3 of the last 10 runs.");
        let r = row("t", "Checkout", "web", Some(&mem(&[true, false, false, false], flow)), None, false);
        assert_eq!(r.status, "failing", "{}", r.summary);
        assert!(r.summary.starts_with("Failed the last 3 runs."));
        let r = row("t", "Notifications", "android", None, None, false);
        assert_eq!((r.status, r.summary.as_str()), ("new", "Nothing remembered yet. It remembers after the first pass."));
        let q = Quarantine { reason: "x".into(), since: "2026-11-17 10:00:00".into(), auto: false };
        let r = row("t", "Admin export", "web", Some(&mem(&[false, true], flow)), Some(&q), false);
        assert_eq!(r.status, "set_aside");
        assert_eq!(r.summary, "Set aside since 17 Nov 2026. It still runs, but does not turn a run red.");
        assert!(r.set_aside_since_ms.is_some());
        assert_eq!(stamp_ms("1970-01-01 00:00:01"), Some(1000));
        assert_eq!(stamp_ms("garbage"), None);
        assert_eq!(screen_name("screen \"Login\" (https://x)"), "Login");
    }
}
