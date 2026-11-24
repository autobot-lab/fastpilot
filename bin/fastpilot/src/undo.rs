//! v0.7.1 FC6: every change made from Chat, Claude Code or the terminal can be undone, and removing
//! keeps the thing for 7 days ("Recently removed"). Two stores in `~/fastpilot/undo.json`
//! (owner-only): the changes, newest last, each with how to reverse it; and what was removed, with
//! everything needed to bring it back (an app with the schedules that were on; a schedule with its
//! last result and its webhook).
//!
//! The Undo rules (HANDOFF FC6, TEXT §4): only the newest change of one thing can be undone; an older
//! one says "Changed later". A restore returns the state the thing had ("Restored schedule … (paused)").
//! The words are TEXT-terminal-mcp.md's, used as written.

use serde::{Deserialize, Serialize};

use crate::apps::{self, App};
use crate::control::{Fail, Surface, fail, q};
use crate::schedule::{self, LastRun, Schedule, When};

/// How long a removed thing is kept.
pub const KEEP_MS: u64 = 7 * DAY_MS;
const DAY_MS: u64 = 24 * 3600 * 1000;

/// How to reverse one change.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// "Added …": remove the app again (it goes to Recently removed).
    RemoveApp { id: String },
    /// "Removed app …": bring it back.
    RestoreApp { id: String },
    /// "Created schedule …": remove it again (it goes to Recently removed).
    RemoveSchedule { id: String },
    /// "Removed schedule …": bring it back.
    RestoreSchedule { id: String },
    /// "Paused …" / "Resumed …": back on or off.
    SetSchedule { id: String, on: bool },
    /// "Changed schedule …": its timing and name before.
    SetWhen { id: String, name: String, when: When },
    /// An agent's skills before the change.
    Skills { agent: String, skills: Vec<String> },
    /// "Remembered …": the note goes.
    RemoveNote { id: String },
    /// "Turned on/off release watch …": the other way.
    Watch { tests: String, on: bool },
    /// "Created agent …": the agent goes.
    RemoveAgent { id: String },
    /// v0.7.1 FC4: "Chat removed": bring it back.
    RestoreChat { id: String },
    /// v0.7.1 FC-STOP: "Removed agent …": bring it back.
    RestoreAgent { id: String },
    /// v0.7.1 FC5: "Added … as a new agent": the agent goes, and the app when it was added then.
    RemoveNewAgent { agent: String, app: Option<String> },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Change {
    /// Four characters, for `fastpilot undo <id>` and `history undo`.
    pub id: String,
    pub at_ms: u64,
    /// The result line as the person saw it ("Created schedule “Every night” · …").
    pub line: String,
    /// The thing it changed ("app:shop", "schedule:every-night", "skills:default", …).
    pub item: String,
    pub op: Op,
    #[serde(default)]
    pub undone: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Removed {
    /// `app` or `schedule`.
    pub kind: String,
    pub id: String,
    pub name: String,
    pub removed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<App>,
    /// The app's schedules that were on (turned off while it is removed).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedules_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Schedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<LastRun>,
    /// The schedule's webhook address (kept like the schedule keeps it: owner-only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook: Option<String>,
    /// FC4: a removed chat as History kept it, and the agent it belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// FC-STOP: a removed agent as it was, and the schedules that worked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_full: Option<crate::agents::Agent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedule_ids: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct Store {
    #[serde(default)]
    changes: Vec<Change>,
    #[serde(default)]
    removed: Vec<Removed>,
}

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn now() -> u64 {
    schedule::now_ms()
}

/// Read, run `f`, and save, under one lock. Removed things older than 7 days and changes older than
/// 30 days are dropped on the way.
fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> Result<T, Fail> {
    let root = apps::root();
    let _held = crate::filestore::hold(&LOCK, &root, ".undo.lock", true);
    let mut s: Store = std::fs::read_to_string(root.join("undo.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let t = now();
    s.removed.retain(|r| t.saturating_sub(r.removed_ms) <= KEEP_MS);
    s.changes.retain(|c| t.saturating_sub(c.at_ms) <= 30 * DAY_MS);
    let n = s.changes.len();
    if n > 300 {
        s.changes.drain(..n - 300);
    }
    let out = f(&mut s);
    crate::filestore::write_atomic(&root, "undo.json", serde_json::to_string_pretty(&s).unwrap_or_default().as_bytes(), true)?;
    Ok(out)
}

/// Keep one change; returns its id.
pub fn record(line: &str, item: &str, op: Op) -> Option<String> {
    with_store(|s| {
        let id = (0..)
            .map(|n: u64| format!("{:04x}", (crate::filestore::random_u64().wrapping_add(n)) & 0xffff))
            .find(|id| !s.changes.iter().any(|c| &c.id == id))
            .unwrap_or_default();
        s.changes.push(Change { id: id.clone(), at_ms: now(), line: line.to_owned(), item: item.to_owned(), op, undone: false });
        id
    })
    .ok()
}

/// Is change `c` older than another change of the same thing?
fn changed_later(s: &Store, c: &Change) -> bool {
    let at = s.changes.iter().position(|y| y.id == c.id).unwrap_or(usize::MAX);
    s.changes.iter().skip(at.saturating_add(1)).any(|x| !x.undone && x.item == c.item)
}

/// Of these change ids, the ones whose thing changed later (no Undo on their line).
pub fn later(ids: &[String]) -> Vec<String> {
    with_store(|s| ids.iter().filter(|id| s.changes.iter().find(|c| &c.id == *id).is_some_and(|c| !c.undone && changed_later(s, c))).cloned().collect()).unwrap_or_default()
}

/// Undo change `id` (the newest change when `None`). Returns the line for the person.
pub fn undo(id: Option<&str>, by: Surface) -> Result<String, Fail> {
    let found = with_store(|s| {
        let c = match id {
            Some(id) => s.changes.iter().find(|c| c.id == id.trim()).cloned(),
            None => s.changes.iter().rev().find(|c| !c.undone).cloned(),
        };
        c.map(|c| (changed_later(s, &c), c))
    })?;
    let (later, c) = match (found, id) {
        (Some(x), _) => x,
        (None, None) => return Err(fail("nothing_to_undo", "Nothing to undo.")),
        (None, Some(id)) => {
            return Err(fail(
                "not_found",
                match by {
                    Surface::Cli => format!("No change with id {}. Run fastpilot undo --list to see them.", q(id.trim())),
                    Surface::Mcp => format!("No change with id {}. Use history removed to see what can be brought back.", q(id.trim())),
                },
            ));
        }
    };
    if c.undone {
        return Err(fail("nothing_to_undo", "Nothing to undo."));
    }
    if later {
        return Err(fail(
            "changed_later",
            match by {
                Surface::Cli => format!("Can't undo {}: it changed later. Change it back yourself, or look in Recently removed: fastpilot undo --list.", q(&c.line)),
                Surface::Mcp => format!("Can't undo {}: it changed later. Change it back yourself, or use action “removed” to see what can be brought back.", q(&c.line)),
            },
        ));
    }
    let undone = format!("Undone · {}", c.line);
    let line = match &c.op {
        Op::RemoveApp { id } => {
            let app = apps::get(id).ok_or_else(|| fail("failed", format!("{} is not there any more.", q(&c.line))))?;
            remove_app(&app, &format!("Removed app {}", q(&app.name)), false)?;
            undone
        }
        Op::RestoreApp { id } => restore_app(id).map_err(|e| if e.code == "expired" { expired(&c) } else { e })?,
        Op::RemoveSchedule { id } => {
            let s = schedule::load().schedules.into_iter().find(|x| &x.id == id).ok_or_else(|| fail("failed", format!("{} is not there any more.", q(&c.line))))?;
            remove_schedule(&s, "", false)?;
            undone
        }
        Op::RestoreSchedule { id } => restore_schedule(id).map_err(|e| if e.code == "expired" { expired(&c) } else { e })?,
        Op::SetSchedule { id, on } => {
            schedule::set_enabled(id, *on)?;
            undone
        }
        Op::SetWhen { id, name, when } => {
            let mut s = schedule::load().schedules.into_iter().find(|x| &x.id == id).ok_or_else(|| fail("failed", format!("{} is not there any more.", q(&c.line))))?;
            s.name.clone_from(name);
            s.when = when.clone();
            schedule::save(s, None)?;
            undone
        }
        Op::Skills { agent, skills } => {
            let mut a = crate::agents::get(agent).ok_or_else(|| fail("failed", format!("{} is not there any more.", q(&c.line))))?;
            a.skills.clone_from(skills);
            crate::agents::update(a)?;
            undone
        }
        Op::RemoveNote { id } => {
            fastpilot_memory::notes::Notes::from_env().remove(id).map_err(|r| fail("failed", r.message))?;
            undone
        }
        Op::Watch { tests, on } => {
            schedule::release_watch(tests, *on)?;
            undone
        }
        Op::RemoveAgent { id } => {
            crate::agents::delete(id)?;
            undone
        }
        Op::RemoveNewAgent { agent, app } => {
            crate::agents::delete(agent)?;
            if let Some(a) = app.as_deref().and_then(apps::get) {
                remove_app(&a, "", false)?;
            }
            undone
        }
        Op::RestoreAgent { id } => restore_agent(id).map_err(|e| if e.code == "expired" { expired(&c) } else { e })?,
        Op::RestoreChat { id } => restore_chat(id).map_err(|e| if e.code == "expired" { expired(&c) } else { e })?,
    };
    with_store(|s| {
        if let Some(x) = s.changes.iter_mut().find(|x| x.id == c.id) {
            x.undone = true;
        }
    })?;
    Ok(line)
}

fn expired(c: &Change) -> Fail {
    let days = now().saturating_sub(c.at_ms) / DAY_MS;
    let name = c.line.split('“').nth(1).and_then(|r| r.split('”').next()).unwrap_or(&c.line);
    fail("expired", format!("{} was removed {} ago and is no longer kept.", q(name), if days == 1 { "1 day".to_owned() } else { format!("{days} days") }))
}

/// Remove an app for 7 days: its schedules that were on are turned off, the app leaves the list,
/// and its folder (tests, history, map, memory) and its passwords stay where they are. `journal`:
/// keep the change (an Undo line); an Undo of "Added" does not.
pub fn remove_app(app: &App, line: &str, journal: bool) -> Result<Option<String>, Fail> {
    let on: Vec<String> = crate::control::app_schedules(app).into_iter().filter(|s| s.enabled).map(|s| s.id).collect();
    for id in &on {
        schedule::set_enabled(id, false)?;
    }
    with_store(|s| {
        s.removed.retain(|r| !(r.kind == "app" && r.id == app.id));
        s.removed.push(Removed { kind: "app".into(), id: app.id.clone(), name: app.name.clone(), removed_ms: now(), app: Some(app.clone()), schedules_on: on.clone(), schedule: None, last: None, webhook: None, chat: None, agent: None, agent_full: None, schedule_ids: vec![] });
    })?;
    apps::remove(&app.id)?;
    Ok(if journal { record(line, &format!("app:{}", app.id), Op::RestoreApp { id: app.id.clone() }) } else { None })
}

/// Bring a removed app back, with the schedules that were on.
pub fn restore_app(id: &str) -> Result<String, Fail> {
    let r = with_store(|s| {
        let i = s.removed.iter().position(|r| r.kind == "app" && r.id == id)?;
        Some(s.removed.remove(i))
    })?
    .ok_or_else(|| fail("expired", "It is no longer kept."))?;
    let app = r.app.ok_or_else(|| fail("failed", "Nothing to bring back."))?;
    let app = apps::restore(app)?;
    for sid in &r.schedules_on {
        if schedule::load().schedules.iter().any(|x| &x.id == sid) {
            schedule::set_enabled(sid, true)?;
        }
    }
    settled(&format!("app:{id}"))?;
    Ok(format!("Restored app {} · tests, history, passwords and schedules are back", q(&app.name)))
}

/// Remove a schedule for 7 days: off the system scheduler and the list, with its last result and
/// its webhook kept.
pub fn remove_schedule(sched: &Schedule, line: &str, journal: bool) -> Result<Option<String>, Fail> {
    let last = schedule::load().last.get(&sched.id).cloned();
    let webhook = std::fs::read_to_string(schedule::secret_path(&sched.id)).ok().map(|w| w.trim().to_owned()).filter(|w| !w.is_empty());
    with_store(|s| {
        s.removed.retain(|r| !(r.kind == "schedule" && r.id == sched.id));
        s.removed.push(Removed { kind: "schedule".into(), id: sched.id.clone(), name: crate::control::schedule_title(sched), removed_ms: now(), app: None, schedules_on: vec![], schedule: Some(sched.clone()), last, webhook, chat: None, agent: None, agent_full: None, schedule_ids: vec![] });
    })?;
    schedule::delete(&sched.id)?;
    Ok(if journal { record(line, &format!("schedule:{}", sched.id), Op::RestoreSchedule { id: sched.id.clone() }) } else { None })
}

/// Bring a removed schedule back in the state it had (on, or paused).
pub fn restore_schedule(id: &str) -> Result<String, Fail> {
    let r = with_store(|s| {
        let i = s.removed.iter().position(|r| r.kind == "schedule" && r.id == id)?;
        Some(s.removed.remove(i))
    })?
    .ok_or_else(|| fail("expired", "It is no longer kept."))?;
    let sched = r.schedule.ok_or_else(|| fail("failed", "Nothing to bring back."))?;
    let paused = !sched.enabled;
    let title = crate::control::schedule_title(&sched);
    schedule::restore(sched, r.last, r.webhook.as_deref())?;
    settled(&format!("schedule:{id}"))?;
    Ok(format!("Restored schedule {}{}", q(&title), if paused { " (paused)" } else { "" }))
}

/// FC4: remove a chat for 7 days: it leaves History and the agent's list, and is kept here as it was.
pub fn remove_chat(id: &str, agent: &str, doc: serde_json::Value) -> Result<Option<String>, Fail> {
    let title = doc["title"].as_str().filter(|t| !t.is_empty()).unwrap_or("New chat").to_owned();
    with_store(|s| {
        s.removed.retain(|r| !(r.kind == "chat" && r.id == id));
        s.removed.push(Removed { kind: "chat".into(), id: id.to_owned(), name: title.clone(), removed_ms: now(), app: None, schedules_on: vec![], schedule: None, last: None, webhook: None, chat: Some(doc), agent: Some(agent.to_owned()), agent_full: None, schedule_ids: vec![] });
    })?;
    fastpilot_memory::history::History::from_env().delete(&[], &[id.to_owned()]);
    crate::agents::forget_conversation(agent, id);
    Ok(record(&format!("Removed chat {}", q(&title)), &format!("chat:{id}"), Op::RestoreChat { id: id.to_owned() }))
}

/// FC-STOP: remove an agent for 7 days (About › Remove agent). Its chats and schedules go to the
/// default agent meanwhile (as before); Undo brings it back with them.
pub fn remove_agent(a: &crate::agents::Agent) -> Result<(String, Option<String>), Fail> {
    let raw = crate::agents::list().into_iter().find(|x| x.id == a.id).ok_or_else(|| fail("failed", "That agent is not there any more."))?;
    let ids: Vec<String> = schedule::load().schedules.into_iter().filter(|s| s.agent.as_deref() == Some(a.id.as_str())).map(|s| s.id).collect();
    crate::agents::delete(&a.id)?;
    with_store(|s| {
        s.removed.retain(|r| !(r.kind == "agent" && r.id == a.id));
        s.removed.push(Removed { kind: "agent".into(), id: a.id.clone(), name: a.name.clone(), removed_ms: now(), app: None, schedules_on: vec![], schedule: None, last: None, webhook: None, chat: None, agent: None, agent_full: Some(raw), schedule_ids: ids });
    })?;
    let line = format!("Removed agent {} · kept 7 days", q(&a.name));
    let undo = record(&line, &format!("agent:{}", a.id), Op::RestoreAgent { id: a.id.clone() });
    Ok((line, undo))
}

/// Bring a removed agent back, with its chats and schedules.
pub fn restore_agent(id: &str) -> Result<String, Fail> {
    let r = with_store(|s| {
        let i = s.removed.iter().position(|r| r.kind == "agent" && r.id == id)?;
        Some(s.removed.remove(i))
    })?
    .ok_or_else(|| fail("expired", "It is no longer kept."))?;
    let a = r.agent_full.ok_or_else(|| fail("failed", "Nothing to bring back."))?;
    let a = crate::agents::restore(a)?;
    schedule::give_back(&r.schedule_ids, &a.id)?;
    settled(&format!("agent:{id}"))?;
    Ok(format!("Restored agent {}", q(&a.name)))
}

/// Bring a removed chat back to its agent, with all its messages.
pub fn restore_chat(id: &str) -> Result<String, Fail> {
    let r = with_store(|s| {
        let i = s.removed.iter().position(|r| r.kind == "chat" && r.id == id)?;
        Some(s.removed.remove(i))
    })?
    .ok_or_else(|| fail("expired", "It is no longer kept."))?;
    let doc = r.chat.ok_or_else(|| fail("failed", "Nothing to bring back."))?;
    let agent = r.agent.filter(|a| crate::agents::get(a).is_some()).unwrap_or_else(|| crate::agents::DEFAULT.into());
    fastpilot_memory::history::History::from_env().restore_chat(id, &doc).map_err(|e| fail("failed", e.to_string()))?;
    crate::agents::add_conversation(&agent, id);
    settled(&format!("chat:{id}"))?;
    Ok(format!("Restored chat {}", q(&r.name)))
}

/// A restore from Recently removed: the "Removed …" change of that thing is done with.
fn settled(item: &str) -> Result<(), Fail> {
    with_store(|s| {
        for c in s.changes.iter_mut().filter(|c| c.item == item && matches!(c.op, Op::RestoreApp { .. } | Op::RestoreSchedule { .. } | Op::RestoreChat { .. } | Op::RestoreAgent { .. })) {
            c.undone = true;
        }
    })
}

/// What was removed in the last 7 days, newest first.
pub fn removed() -> Vec<Removed> {
    let mut list = with_store(|s| s.removed.clone()).unwrap_or_default();
    list.sort_by_key(|r| std::cmp::Reverse(r.removed_ms));
    list
}

/// Days a removed thing is still kept: "7 days left" on the day it was removed.
pub fn days_left(r: &Removed) -> u64 {
    7u64.saturating_sub(now().saturating_sub(r.removed_ms) / DAY_MS).max(1)
}

/// "App “Old demo”", "Schedule “All tests”".
pub fn removed_label(r: &Removed) -> String {
    format!("{} {}", match r.kind.as_str() { "app" => "App", "chat" => "Chat", "agent" => "Agent", _ => "Schedule" }, q(&r.name))
}

/// The recent changes that can still be undone or were changed later, newest first: (change,
/// changed later).
pub fn recent() -> Vec<(Change, bool)> {
    with_store(|s| s.changes.iter().rev().filter(|c| !c.undone && now().saturating_sub(c.at_ms) <= KEEP_MS).take(10).map(|c| (c.clone(), changed_later(s, c))).collect()).unwrap_or_default()
}

/// "today", "yesterday", "3 days ago".
pub fn ago(ms: u64) -> String {
    match now().saturating_sub(ms) / DAY_MS {
        0 => "today".into(),
        1 => "yesterday".into(),
        n => format!("{n} days ago"),
    }
}

/// `fastpilot undo --list`.
pub fn list_text() -> String {
    let changes = recent();
    let gone = removed();
    if changes.is_empty() && gone.is_empty() {
        return "Nothing recent to undo or bring back.".into();
    }
    let mut out = Vec::new();
    if !changes.is_empty() {
        out.push("You can undo".to_owned());
        let w = changes.iter().map(|(c, _)| c.line.chars().count()).max().unwrap_or(0) + 4;
        for (c, later) in &changes {
            out.push(format!("  {:<7}{:<w$}{}{}", c.id, c.line, ago(c.at_ms), if *later { " · Changed later" } else { "" }));
        }
    }
    if !gone.is_empty() {
        out.push("Recently removed (kept 7 days)".to_owned());
        let w = gone.iter().map(|r| removed_label(r).chars().count()).max().unwrap_or(0) + 4;
        for r in &gone {
            let left = days_left(r);
            out.push(format!("  {:<w$}{}", removed_label(r), if left == 1 { "1 day left".to_owned() } else { format!("{left} days left") }));
        }
    }
    out.join("\n")
}

pub const UNDO_USAGE: &str = "fastpilot undo — undo a change

Usage: fastpilot undo [<id>] [--list]

  (no id)     Undo the newest change.
  <id>        Undo this change. Only the newest change of the same thing can be undone.
  --list      Show recent changes and what was removed in the last 7 days.";

/// `fastpilot undo [id] [--list]`.
pub fn cli(args: &[String]) -> Result<(), Fail> {
    if args.iter().any(|a| a == "--list") {
        println!("{}", list_text());
        return Ok(());
    }
    match undo(args.iter().find(|a| !a.starts_with("--")).map(String::as_str), Surface::Cli) {
        Ok(line) => {
            println!("✓ {line}");
            Ok(())
        }
        Err(f) if f.code == "nothing_to_undo" => {
            println!("{}", f.msg);
            Ok(())
        }
        Err(f) => Err(f),
    }
}
