//! What this fastpilot process is doing right now, for the Live panel of `fastpilot ui` (C7a):
//! its sessions, the steps of the flow running on each, and, for web sessions on fastpilot's own
//! Chrome, where to watch the screen.
//!
//! Written to `~/.fastpilot/live/<pid>.json` (folder 0700, file 0600, replaced atomically) by
//! processes that call [`enable`] (`fastpilot mcp`, the shell daemon, `fastpilot run`), on every
//! change and every 2 s as a heartbeat; a file not refreshed for [`STALE`] belongs to a process
//! that is gone. The file holds the DevTools address of fastpilot's own Chrome: only the owner can
//! read it, and `fastpilot ui` never sends it to the browser, only the frames.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A file older than this is ignored (and cleaned up) by readers.
pub const STALE: Duration = Duration::from_secs(10);
/// Steps kept per flow.
const MAX_STEPS: usize = 300;

/// Where to watch a web session: the browser's DevTools socket and the page target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub ws: String,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pending,
    Running,
    Done,
    Failed,
    Blocked,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveStep {
    pub label: String,
    pub state: StepState,
    /// Why it failed / was blocked, or, before it runs, why a guard rail will refuse it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub will_block: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveFlow {
    pub id: u64,
    pub started_ms: u64,
    pub finished: bool,
    pub steps: Vec<LiveStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSession {
    pub id: String,
    pub platform: String,
    pub started_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<Screen>,
    /// The flow that last ran a step in this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LiveProcess {
    pub pid: u32,
    pub kind: String,
    pub updated_ms: u64,
    pub sessions: Vec<LiveSession>,
    pub flows: Vec<LiveFlow>,
}

struct State {
    path: PathBuf,
    proc: LiveProcess,
    next_flow: u64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// `~/.fastpilot/live` (or `FASTPILOT_LIVE_DIR`).
pub fn dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FASTPILOT_LIVE_DIR").filter(|d| !d.is_empty()) {
        return d.into();
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join(".fastpilot").join("live")
}

/// Start publishing for this process (`kind`: mcp, daemon, run).
pub fn enable(kind: &str) {
    let path = dir().join(format!("{}.json", std::process::id()));
    *lock() = Some(State {
        path,
        proc: LiveProcess { pid: std::process::id(), kind: kind.to_owned(), ..Default::default() },
        next_flow: 1,
    });
    write();
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            if lock().is_none() {
                break;
            }
            write();
        }
    });
}

/// Stop publishing and remove this process' file.
pub fn disable() {
    if let Some(s) = lock().take() {
        let _ = std::fs::remove_file(s.path);
    }
}

fn lock() -> std::sync::MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

fn write() {
    let (path, text) = {
        let mut g = lock();
        let Some(s) = g.as_mut() else { return };
        s.proc.updated_ms = now_ms();
        // Flows nobody looks at any more.
        let shown: Vec<u64> = s.proc.sessions.iter().filter_map(|x| x.flow).collect();
        let last = s.proc.flows.last().map(|f| f.id);
        s.proc.flows.retain(|f| shown.contains(&f.id) || Some(f.id) == last);
        (s.path.clone(), serde_json::to_string(&s.proc).unwrap_or_default())
    };
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = path.with_extension("json.tmp");
    let written = {
        use std::io::Write;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        o.open(&tmp).and_then(|mut f| f.write_all(text.as_bytes())).and_then(|()| std::fs::rename(&tmp, &path))
    };
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn update(f: impl FnOnce(&mut State)) {
    {
        let mut g = lock();
        let Some(s) = g.as_mut() else { return };
        f(s);
    }
    write();
}

pub fn session_started(id: &str, platform: &str, screen: Option<Screen>) {
    update(|s| {
        s.proc.sessions.retain(|x| x.id != id);
        s.proc.sessions.push(LiveSession { id: id.to_owned(), platform: platform.to_owned(), started_ms: now_ms(), screen, flow: None });
    });
}

pub fn session_ended(id: &str) {
    update(|s| s.proc.sessions.retain(|x| x.id != id));
}

/// A flow begins: its leaf steps in order (`(label, will_block, session if known)`). Returns its id.
pub fn flow_started(steps: Vec<(String, Option<String>, Option<String>)>) -> Option<u64> {
    let mut id = None;
    update(|s| {
        let n = s.next_flow;
        s.next_flow += 1;
        id = Some(n);
        let steps = steps
            .into_iter()
            .take(MAX_STEPS)
            .map(|(label, will_block, session)| LiveStep { label, state: StepState::Pending, note: None, will_block, session })
            .collect();
        s.proc.flows.push(LiveFlow { id: n, started_ms: now_ms(), finished: false, steps });
    });
    id
}

/// Step `index` (1-based) of `flow` is running in `session`; `screen` refreshes where to watch it.
pub fn step_started(flow: Option<u64>, index: usize, session: Option<&str>, screen: Option<Screen>) {
    let Some(flow) = flow else { return };
    update(|s| {
        if let Some(step) = s.proc.flows.iter_mut().find(|f| f.id == flow).and_then(|f| f.steps.get_mut(index.wrapping_sub(1))) {
            step.state = StepState::Running;
            if let Some(sid) = session {
                step.session = Some(sid.to_owned());
            }
        }
        if let Some(x) = session.and_then(|sid| s.proc.sessions.iter_mut().find(|x| x.id == sid)) {
            x.flow = Some(flow);
            if screen.is_some() {
                x.screen = screen;
            }
        }
    });
}

/// Step `index` ended: `Ok`, or the error kind and message.
pub fn step_finished(flow: Option<u64>, index: usize, session: Option<&str>, result: Result<(), (bool, String)>) {
    let Some(flow) = flow else { return };
    update(|s| {
        if let Some(step) = s.proc.flows.iter_mut().find(|f| f.id == flow).and_then(|f| f.steps.get_mut(index.wrapping_sub(1))) {
            match result {
                Ok(()) => step.state = StepState::Done,
                Err((blocked, why)) => {
                    step.state = if blocked { StepState::Blocked } else { StepState::Failed };
                    step.note = Some(why.chars().take(300).collect());
                }
            }
            if let Some(sid) = session {
                step.session = Some(sid.to_owned());
            }
        }
        if let Some(x) = session.and_then(|sid| s.proc.sessions.iter_mut().find(|x| x.id == sid)) {
            x.flow = Some(flow);
        }
    });
}

pub fn flow_finished(flow: Option<u64>) {
    let Some(flow) = flow else { return };
    update(|s| {
        if let Some(f) = s.proc.flows.iter_mut().find(|f| f.id == flow) {
            f.finished = true;
            for step in &mut f.steps {
                if matches!(step.state, StepState::Pending | StepState::Running) {
                    step.state = StepState::Skipped;
                }
            }
        }
    });
}

/// Every live process' state, newest first; stale files are removed.
pub fn read_all() -> Vec<LiveProcess> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir()) else { return out };
    let now = now_ms();
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        match serde_json::from_str::<LiveProcess>(&text) {
            Ok(proc) if now.saturating_sub(proc.updated_ms) <= STALE.as_millis() as u64 => out.push(proc),
            _ => {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.updated_ms));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_sessions_and_steps_owner_only() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let d = std::env::temp_dir().join(format!("fp-live-{}", std::process::id()));
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe { std::env::set_var("FASTPILOT_LIVE_DIR", &d) };
        enable("test");
        session_started("w", "web", Some(Screen { ws: "ws://127.0.0.1:1/devtools/browser/x".into(), target: "T1".into() }));
        let f = flow_started(vec![("tapOn \"A\"".into(), None, None), ("tapOn \"Bayar\"".into(), Some("teks berisi \"bayar\"".into()), None)]);
        step_started(f, 1, Some("w"), None);
        step_finished(f, 1, Some("w"), Ok(()));
        step_started(f, 2, Some("w"), None);
        step_finished(f, 2, Some("w"), Err((true, "blocked".into())));
        flow_finished(f);
        let all = read_all();
        let me = all.iter().find(|p| p.pid == std::process::id()).expect("own file");
        assert_eq!(me.sessions[0].flow, f);
        let steps = &me.flows.iter().find(|x| Some(x.id) == f).expect("flow").steps;
        assert_eq!((steps[0].state, steps[1].state), (StepState::Done, StepState::Blocked));
        assert_eq!(steps[1].will_block.as_deref(), Some("teks berisi \"bayar\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file = d.join(format!("{}.json", std::process::id()));
            assert_eq!(std::fs::metadata(&file).expect("file").permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(&d).expect("dir").permissions().mode() & 0o777, 0o700);
        }
        // A stale file (a process that died) is ignored and removed.
        let stale = d.join("1.json");
        std::fs::write(&stale, r#"{"pid":1,"kind":"mcp","updated_ms":1,"sessions":[],"flows":[]}"#).expect("stale");
        assert!(read_all().iter().all(|p| p.pid != 1) && !stale.exists());
        session_ended("w");
        disable();
        assert!(read_all().iter().all(|p| p.pid != std::process::id()));
        unsafe { std::env::remove_var("FASTPILOT_LIVE_DIR") };
        let _ = std::fs::remove_dir_all(d);
    }
}
