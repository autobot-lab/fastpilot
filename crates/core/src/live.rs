//! What this fastpilot process is doing right now, for the Live panel of `fastpilot ui` (C7a):
//! its sessions, the steps of the flow running on each, and, for web sessions on fastpilot's own
//! Chrome, where to watch the screen.
//!
//! Written to `~/.fastpilot/live/<pid>.json` (folder 0700, file 0600, replaced atomically) by
//! processes that call [`enable`] (`fastpilot mcp`, the shell daemon, `fastpilot run`), on every
//! change and every 2 s as a heartbeat; a file not refreshed for [`STALE`] belongs to a process
//! that is gone. The file holds the DevTools address of fastpilot's own Chrome: only the owner can
//! read it, and `fastpilot ui` never sends it to the browser, only the frames.
//!
//! "Ambil alih" (C7b): each publishing process also listens on `<pid>.sock` in the same 0700 folder
//! (Unix only). `fastpilot ui` asks it for screen frames of Android and iOS sessions (only while the
//! panel shows them) and, after the user takes a session over, sends the user's taps, swipes, text
//! and keys. While a session is taken over its automatic steps wait ([`hold`]); giving it back
//! leaves a note ([`take_notes`]) for the AI's next result and the run record. Guard rails are not
//! applied to the user's own input; they still apply to every automatic step.
//!
//! Who may use the socket: only a `fastpilot ui` process running the same program file as this
//! process (the peer's pid and executable path, checked on every connection); a script, an agent, or
//! another program merely named `fastpilot` is turned away. `allow_takeover = false` (a limit, pinned
//! at start) switches takeover off altogether. A takeover hands
//! out a random token, kept only in the memory of this process and of that `fastpilot ui`; input
//! and Kembalikan need it. A takeover ends by itself when the page stops sending keep-alives
//! ([`IDLE`], e.g. the tab was closed) or after [`MAX_TAKEOVER`], so an AI step never waits forever.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    /// Since when (ms) the user has this session taken over in `fastpilot ui`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taken_over_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LiveProcess {
    pub pid: u32,
    pub kind: String,
    /// `allow_takeover` as this process pinned it at start (false: no Ambil alih here).
    #[serde(default = "takeover_default")]
    pub takeover: bool,
    pub updated_ms: u64,
    /// v0.7.1 (Security L1): when this record was made, so a stop can tell a reused pid from this
    /// process (its start must not be later than this).
    #[serde(default)]
    pub started_ms: u64,
    /// D10a: the agent this process works for (`FASTPILOT_AGENT`, set by the dashboard's Chat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// V042: the schedule that started this process (`FASTPILOT_SCHEDULED_AS`, set by `schedule run`),
    /// so Live says "Scheduled · …" instead of an agent's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled: Option<String>,
    pub sessions: Vec<LiveSession>,
    pub flows: Vec<LiveFlow>,
    /// PF5: tests of this run that run once per platform, with each platform's progress.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub side_by_side: Vec<SideRun>,
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
    #[cfg(unix)]
    control::listen(path.with_extension("sock"));
    *lock() = Some(State {
        path,
        proc: LiveProcess {
            pid: std::process::id(),
            kind: kind.to_owned(),
            takeover: crate::config::flag("allow_takeover"),
            started_ms: now_ms(),
            agent: std::env::var("FASTPILOT_AGENT").ok().filter(|a| !a.is_empty() && a.len() <= 40 && a.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')),
            scheduled: std::env::var("FASTPILOT_SCHEDULED_AS").ok().map(|t| t.chars().filter(|c| !c.is_control()).take(80).collect::<String>()).filter(|t| !t.trim().is_empty()),
            ..Default::default()
        },
        next_flow: 1,
    });
    write();
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            if lock().is_none() {
                break;
            }
            expire_takeovers();
            write();
        }
    });
}

/// Stop publishing and remove this process' file.
pub fn disable() {
    if let Some(s) = lock().take() {
        let _ = std::fs::remove_file(s.path.with_extension("sock"));
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
        // What a step says can hold a password (an agent may write the value itself): masked like
        // every other output before it reaches the file the dashboard reads.
        let mut shown = s.proc.clone();
        let r = crate::dsl::redact;
        for st in shown.flows.iter_mut().flat_map(|f| f.steps.iter_mut()) {
            st.label = r(&st.label);
            st.note = st.note.as_deref().map(r);
            st.will_block = st.will_block.as_deref().map(r);
        }
        for t in shown.side_by_side.iter_mut().flat_map(|x| x.tiles.iter_mut()) {
            t.label = r(&t.label);
            t.note = r(&t.note);
        }
        (s.path.clone(), serde_json::to_string(&shown).unwrap_or_default())
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
        s.proc.sessions.push(LiveSession { id: id.to_owned(), platform: platform.to_owned(), started_ms: now_ms(), screen, flow: None, taken_over_ms: None });
    });
}

pub fn session_ended(id: &str) {
    lock_map(&DRIVERS).remove(id);
    give_back(id, "the session ended");
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

// ---- PF5: side-by-side runs ---------------------------------------------------------------

/// Where one platform of a side-by-side test is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileState {
    /// In the queue, or waiting for its device ("one device runs one test at a time").
    Waiting,
    Running,
    Passed,
    Failed,
    /// Never started: no device for it, or something it needs is not set up (`note` says what).
    NotRun,
}

/// One platform's tile in the Live summary row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SideTile {
    /// `web`, `android` or `ios`.
    pub platform: String,
    /// "Web · Chrome", "Android emulator", "iPhone simulator", ...
    pub name: String,
    pub state: TileState,
    /// The last numbered step it reported (0: none yet) of `steps`.
    pub step: usize,
    pub steps: usize,
    /// That step's words from the test (test text: shown as text only).
    #[serde(default)]
    pub label: String,
    /// Why it waits or did not run.
    #[serde(default)]
    pub note: String,
}

/// A test running once per platform (PF5): its summary row on the Live page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SideRun {
    pub id: String,
    pub title: String,
    pub started_ms: u64,
    pub tiles: Vec<SideTile>,
}

/// A side-by-side test starts: one tile per platform.
pub fn side_started(id: &str, title: &str, tiles: Vec<SideTile>) {
    update(|s| {
        s.proc.side_by_side.retain(|r| r.id != id);
        s.proc.side_by_side.push(SideRun { id: id.to_owned(), title: title.to_owned(), started_ms: now_ms(), tiles });
    });
}

/// Change the tile of `platform` in side-by-side test `id`.
pub fn side_tile(id: &str, platform: &str, f: impl FnOnce(&mut SideTile)) {
    update(|s| {
        if let Some(t) = s.proc.side_by_side.iter_mut().find(|r| r.id == id).and_then(|r| r.tiles.iter_mut().find(|t| t.platform == platform)) {
            f(t);
        }
    });
}

// ---- Ambil alih (C7b) ---------------------------------------------------------------------

/// Drivers of this process' sessions, for frames and the user's input.
static DRIVERS: LazyLock<Mutex<HashMap<String, Weak<dyn crate::Driver>>>> = LazyLock::new(Default::default);
/// Sessions the user has taken over, with what they did.
static TAKEN: LazyLock<Mutex<HashMap<String, Takeover>>> = LazyLock::new(Default::default);
/// Takeovers that ended `(session, note)`, for the next result that uses the session, and its run record.
static NOTES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

#[derive(Debug, Default, Clone, Copy)]
struct Inputs {
    taps: u32,
    swipes: u32,
    texts: u32,
    keys: u32,
}

struct Takeover {
    since: Instant,
    /// Last input or keep-alive from the page.
    seen: Instant,
    token: String,
    inputs: Inputs,
}

/// A takeover without input or keep-alive from the page for this long ends by itself
/// (`FASTPILOT_TAKEOVER_IDLE_SECS`, for tests).
pub const IDLE: Duration = Duration::from_secs(45);
/// No takeover lasts longer than this.
pub const MAX_TAKEOVER: Duration = Duration::from_secs(30 * 60);

fn idle_limit() -> Duration {
    std::env::var("FASTPILOT_TAKEOVER_IDLE_SECS").ok().and_then(|s| s.parse().ok()).map(Duration::from_secs).unwrap_or(IDLE)
}

/// 128 random bits, hex.
fn token() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        // No /dev/urandom: time and address entropy, still unknown to other processes.
        let seed = format!("{:?}{:p}{}", Instant::now(), &b, std::process::id());
        for (i, c) in seed.bytes().enumerate() {
            b[i % 16] ^= c.rotate_left((i % 7) as u32);
        }
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// End takeovers whose page went quiet or that ran too long.
pub fn expire_takeovers() {
    let idle = idle_limit();
    let stale: Vec<(String, &'static str)> = lock_map(&TAKEN)
        .iter()
        .filter_map(|(id, t)| {
            if t.since.elapsed() > MAX_TAKEOVER {
                Some((id.clone(), "it ended by itself after 30 minutes"))
            } else if t.seen.elapsed() > idle {
                Some((id.clone(), "it ended by itself because the fastpilot ui page stopped answering (closed tab?)"))
            } else {
                None
            }
        })
        .collect();
    for (id, why) in stale {
        give_back(&id, why);
    }
}

fn lock_map<T>(m: &'static LazyLock<Mutex<T>>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The session's driver, so `fastpilot ui` can show and (after a takeover) steer it.
pub fn attach(id: &str, driver: &Arc<dyn crate::Driver>) {
    lock_map(&DRIVERS).insert(id.to_owned(), Arc::downgrade(driver));
}

fn driver(id: &str) -> Option<Arc<dyn crate::Driver>> {
    lock_map(&DRIVERS).get(id).and_then(Weak::upgrade)
}

/// Whether the user has `session` taken over right now.
pub fn is_taken_over(session: &str) -> bool {
    expire_takeovers();
    lock_map(&TAKEN).contains_key(session)
}

/// Before an automatic step on `session`: wait while the user has it taken over.
pub async fn hold(session: &str) {
    while is_taken_over(session) {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

fn takeover_default() -> bool {
    true
}

/// Start a takeover; its token is the only way to steer or end it.
fn take_over(id: &str) -> Result<String, String> {
    // A limit like the others: pinned at start, so turning it on later needs a restart.
    if !crate::config::flag("allow_takeover") {
        return Err("Take over is turned off in Limits (allow_takeover = false)".into());
    }
    if driver(id).is_none() {
        return Err(format!("session `{id}` is gone"));
    }
    expire_takeovers();
    let token = {
        let mut taken = lock_map(&TAKEN);
        if taken.contains_key(id) {
            return Err("this session is taken over on another page".into());
        }
        let token = token();
        taken.insert(id.to_owned(), Takeover { since: Instant::now(), seen: Instant::now(), token: token.clone(), inputs: Inputs::default() });
        token
    };
    let since = now_ms();
    update(|s| {
        if let Some(x) = s.proc.sessions.iter_mut().find(|x| x.id == id) {
            x.taken_over_ms.get_or_insert(since);
        }
    });
    Ok(token)
}

/// End a takeover (Kembalikan, or the session ended) and leave a note of it.
fn give_back(id: &str, how: &str) {
    let Some(t) = lock_map(&TAKEN).remove(id) else { return };
    let i = t.inputs;
    let mut did: Vec<String> = Vec::new();
    for (n, what) in [(i.taps, "tap"), (i.swipes, "swipe"), (i.texts, "text input"), (i.keys, "key")] {
        if n > 0 {
            did.push(format!("{n} {what}{}", if n == 1 { "" } else { "s" }));
        }
    }
    let did = if did.is_empty() { "no input".to_owned() } else { did.join(", ") };
    let note = format!(
        "the user took over session `{id}` in fastpilot ui for {}s ({did}); {how}. Automatic steps waited meanwhile, and the screen may differ from what the flow expects.",
        t.since.elapsed().as_secs()
    );
    let mut notes = NOTES.lock().unwrap_or_else(|p| p.into_inner());
    // Notes of sessions nobody runs again are not kept forever.
    if notes.len() >= 50 {
        notes.remove(0);
    }
    notes.push((id.to_owned(), note));
    drop(notes);
    update(|s| {
        if let Some(x) = s.proc.sessions.iter_mut().find(|x| x.id == id) {
            x.taken_over_ms = None;
        }
    });
}

/// Takeovers of `sessions` that ended since they were last asked for (oldest first).
pub fn take_notes(sessions: &[String]) -> Vec<String> {
    let mut notes = NOTES.lock().unwrap_or_else(|p| p.into_inner());
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *notes).into_iter().partition(|(s, _)| sessions.contains(s));
    *notes = rest;
    mine.into_iter().map(|(_, n)| n).collect()
}

/// The screen size in the coordinates `tap`/`swipe` points use: the visible area (web), or the
/// extent of the screen's elements (Android pixels, iOS points).
async fn screen_size(d: &dyn crate::Driver) -> Option<(f64, f64)> {
    if let Some(v) = d.viewport().await {
        return Some((v.w, v.h));
    }
    let snap = d.snapshot().await.ok()?;
    let (w, h) = snap.entries.iter().filter_map(|e| e.node.bounds).fold((0.0f64, 0.0f64), |(w, h), b| (w.max(b.x + b.w), h.max(b.y + b.h)));
    (w > 0.0 && h > 0.0).then_some((w, h))
}

/// Keys the panel may send.
const KEYS: &[&str] = &["Enter", "Tab", "Escape", "Backspace", "Delete", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown", "Space"];

/// One request from `fastpilot ui` (a JSON line): `op` is frame, takeover, or, with the takeover's
/// `token`, keep, release, tap, swipe, type, key or back; `x`/`y` (and `x2`/`y2`) are fractions
/// (0–1) of the shown screen.
pub async fn command(req: &serde_json::Value) -> serde_json::Value {
    use serde_json::json;
    let op = req["op"].as_str().unwrap_or("");
    let Some(id) = req["session"].as_str() else { return json!({ "ok": false, "error": "session is required" }) };
    let Some(d) = driver(id) else { return json!({ "ok": false, "error": format!("session `{id}` is gone") }) };
    let frac = |k: &str| req[k].as_f64().filter(|v| (0.0..=1.0).contains(v));
    let result: Result<serde_json::Value, String> = async {
        match op {
            "frame" => {
                let jpeg = d.screenshot().await.map_err(|e| e.to_string())?;
                use base64::Engine;
                return Ok(json!({ "jpeg": base64::engine::general_purpose::STANDARD.encode(jpeg) }));
            }
            "takeover" => return take_over(id).map(|token| json!({ "token": token })),
            _ => {}
        }
        // Everything else belongs to one takeover: its token, while it lasts.
        expire_takeovers();
        {
            let mut taken = lock_map(&TAKEN);
            let Some(t) = taken.get_mut(id) else { return Err("take over this session first".into()) };
            let given = req["token"].as_str().unwrap_or("");
            if given.len() != t.token.len() || !given.bytes().zip(t.token.bytes()).fold(true, |ok, (a, b)| ok & (a == b)) {
                return Err("this page did not take over this session".into());
            }
            t.seen = Instant::now();
        }
        match op {
            "release" => {
                give_back(id, "then gave it back");
                return Ok(json!({}));
            }
            "keep" => return Ok(json!({})),
            _ => {}
        }
        let count = |f: fn(&mut Inputs)| {
            if let Some(t) = lock_map(&TAKEN).get_mut(id) {
                f(&mut t.inputs);
            }
        };
        match op {
            "tap" | "swipe" => {
                let (Some(x), Some(y)) = (frac("x"), frac("y")) else { return Err("x and y must be 0–1".into()) };
                let (w, h) = screen_size(&*d).await.ok_or("the screen size is unknown")?;
                let at = crate::Point { x: x * w, y: y * h };
                if op == "tap" {
                    d.tap(&crate::Selector::Point(at)).await.map_err(|e| e.to_string())?;
                    count(|i| i.taps += 1);
                } else {
                    let (Some(x2), Some(y2)) = (frac("x2"), frac("y2")) else { return Err("x2 and y2 must be 0–1".into()) };
                    d.swipe(at, crate::Point { x: x2 * w, y: y2 * h }).await.map_err(|e| e.to_string())?;
                    count(|i| i.swipes += 1);
                }
            }
            "type" => {
                let text = req["text"].as_str().filter(|t| !t.is_empty() && t.chars().count() <= 500).ok_or("teks 1–500 karakter")?;
                d.type_text(&crate::Selector::Focused, text).await.map_err(|e| e.to_string())?;
                count(|i| i.texts += 1);
            }
            "key" => {
                let key = req["key"].as_str().filter(|k| KEYS.contains(k)).ok_or("unknown key")?;
                d.press_key(key).await.map_err(|e| e.to_string())?;
                count(|i| i.keys += 1);
            }
            "back" => {
                d.back().await.map_err(|e| e.to_string())?;
                count(|i| i.keys += 1);
            }
            other => return Err(format!("unknown command `{other}`")),
        }
        Ok(json!({}))
    }
    .await;
    match result {
        Ok(mut v) => {
            v["ok"] = json!(true);
            v
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// The control socket: one JSON request per line, one JSON answer per line. Only this user can
/// reach it (the folder is 0700, the socket 0600), and only a `fastpilot ui` process is served.
#[cfg(unix)]
mod control {
    use std::path::PathBuf;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    pub fn listen(path: PathBuf) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let _ = std::fs::remove_file(&path);
        let listener = {
            let _runtime = rt.enter();
            match tokio::net::UnixListener::bind(&path) {
                Ok(l) => l,
                Err(e) => return tracing::debug!(error = %e, "live control socket not available"),
            }
        };
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        rt.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream));
            }
        });
    }

    /// The process at the other end is `fastpilot ui` (its pid from the socket, its command line from
    /// the system). `FASTPILOT_LIVE_ANY_PEER=1` lifts this for tests of the socket itself.
    async fn from_ui(stream: &tokio::net::UnixStream) -> bool {
        if std::env::var_os("FASTPILOT_LIVE_ANY_PEER").is_some_and(|v| v == "1") {
            return true;
        }
        let Some(pid) = stream.peer_cred().ok().and_then(|c| c.pid()) else { return false };
        // The same program file as this process (not just one named `fastpilot`) ...
        let same = match (exe_of(pid), std::env::current_exe()) {
            (Some(peer), Ok(me)) => same_file(&peer, &me),
            _ => false,
        };
        if !same {
            return false;
        }
        // ... started as `fastpilot ui`.
        let out = tokio::process::Command::new("ps").args(["-ww", "-o", "args=", "-p", &pid.to_string()]).output().await;
        let Ok(out) = out else { return false };
        super::is_ui_command(&String::from_utf8_lossy(&out.stdout))
    }

    fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
        match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    /// The program file a process runs.
    #[cfg(target_os = "linux")]
    fn exe_of(pid: i32) -> Option<std::path::PathBuf> {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok()
    }

    #[cfg(target_os = "macos")]
    fn exe_of(pid: i32) -> Option<std::path::PathBuf> {
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: the buffer is PROC_PIDPATHINFO_MAXSIZE bytes, as proc_pidpath requires.
        let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        (n > 0).then(|| {
            buf.truncate(n as usize);
            std::path::PathBuf::from(String::from_utf8_lossy(&buf).into_owned())
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn exe_of(_pid: i32) -> Option<std::path::PathBuf> {
        None
    }

    async fn serve(stream: tokio::net::UnixStream) {
        if !from_ui(&stream).await {
            let (r, mut w) = stream.into_split();
            let why = format!(
                "only fastpilot ui may use this socket: start `fastpilot ui` from the same installation as this fastpilot ({})",
                std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
            );
            // Wait (briefly) for the request before answering, and close the writing side cleanly.
            // Answering and closing at once raced a client that had not written yet: its write then
            // failed (broken pipe), and `fastpilot ui` showed "that fastpilot process does not take
            // control" instead of this reason (T1).
            let mut first = String::new();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), BufReader::new(r.take(64 * 1024)).read_line(&mut first)).await;
            let _ = w.write_all(format!("{}\n", serde_json::json!({ "ok": false, "error": why })).as_bytes()).await;
            let _ = w.shutdown().await;
            return;
        }
        let (r, mut w) = stream.into_split();
        // Requests are small: a connection reads at most 64 KB.
        let mut lines = BufReader::new(r.take(64 * 1024)).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let answer = match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(req) => super::command(&req).await,
                Err(_) => serde_json::json!({ "ok": false, "error": "not JSON" }),
            };
            if w.write_all(format!("{answer}\n").as_bytes()).await.is_err() {
                break;
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// T1 (v0.3.1 finding): a refused client that wrote its request a moment after connecting
        /// (a busy machine) found the connection already closed, so its write failed and the page
        /// showed a misleading "that process does not take control". Every refused client, quick
        /// or slow, now writes its request and reads the refusal line.
        #[test]
        fn a_refused_client_always_reads_the_reason() {
            let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            // SAFETY: test-only env mutation, serialized by ENV_LOCK.
            unsafe { std::env::remove_var("FASTPILOT_LIVE_ANY_PEER") };
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime");
            rt.block_on(async {
                let path = std::env::temp_dir().join(format!("fp-ctl-{}.sock", std::process::id()));
                let _ = std::fs::remove_file(&path);
                let listener = tokio::net::UnixListener::bind(&path).expect("bind");
                tokio::spawn(async move {
                    while let Ok((stream, _)) = listener.accept().await {
                        tokio::spawn(serve(stream));
                    }
                });
                for i in 0..20 {
                    let mut s = tokio::net::UnixStream::connect(&path).await.expect("connect");
                    // Every other client is slow to write, as on a loaded machine.
                    if i % 2 == 1 {
                        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    }
                    let wrote = s.write_all(b"{\"op\":\"sessions\"}\n").await;
                    let mut line = String::new();
                    let read = BufReader::new(s).read_line(&mut line).await;
                    assert!(wrote.is_ok() && read.is_ok() && line.contains("only fastpilot ui may use this socket"), "try {i}: wrote {wrote:?}, read {read:?}, line {line:?}");
                }
                let _ = std::fs::remove_file(&path);
            });
        }
    }
}

/// `ps` args of a `fastpilot ui` process: the program is named `fastpilot` and its first argument
/// is `ui`. (A path with spaces is split by `ps`; the program is then found by its name.)
fn is_ui_command(args: &str) -> bool {
    let words: Vec<&str> = args.split_whitespace().collect();
    for i in 0..words.len() {
        // The program: the first word, or (a path with spaces) the first words up to `.../fastpilot`.
        let program = words[..=i].join(" ");
        if i > 0 && !program.starts_with('/') {
            return false;
        }
        if program.rsplit('/').next() == Some("fastpilot") {
            return words.get(i + 1) == Some(&"ui");
        }
    }
    false
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
        let f = flow_started(vec![("tapOn \"A\"".into(), None, None), ("tapOn \"Bayar\"".into(), Some("the text contains \"bayar\"".into()), None)]);
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
        assert_eq!(steps[1].will_block.as_deref(), Some("the text contains \"bayar\""));
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
        // PF5: a side-by-side test publishes one tile per platform, each with its own state.
        let tile = |p: &str, name: &str| SideTile { platform: p.into(), name: name.into(), state: TileState::Waiting, step: 0, steps: 5, label: String::new(), note: String::new() };
        side_started("login-all", "Login", vec![tile("web", "Web · Chrome"), tile("ios", "iPhone simulator")]);
        side_tile("login-all", "web", |t| {
            t.state = TileState::Running;
            t.step = 3;
            t.label = "Enter the email".into();
        });
        side_tile("login-all", "ios", |t| t.note = "after Android finishes".into());
        side_tile("nope", "web", |t| t.step = 9);
        let me = read_all().into_iter().find(|p| p.pid == std::process::id()).expect("own file");
        let run = &me.side_by_side[0];
        assert_eq!((run.id.as_str(), run.tiles.len()), ("login-all", 2));
        assert_eq!((run.tiles[0].state, run.tiles[0].step, run.tiles[0].label.as_str()), (TileState::Running, 3, "Enter the email"));
        assert_eq!((run.tiles[1].state, run.tiles[1].note.as_str()), (TileState::Waiting, "after Android finishes"));
        session_ended("w");
        disable();
        assert!(read_all().iter().all(|p| p.pid != std::process::id()));
        unsafe { std::env::remove_var("FASTPILOT_LIVE_DIR") };
        let _ = std::fs::remove_dir_all(d);
    }
}

#[cfg(test)]
mod takeover_tests {
    #[test]
    fn only_a_fastpilot_ui_command_counts() {
        assert!(super::is_ui_command("/usr/local/bin/fastpilot ui --no-open --port 7777\n"));
        assert!(super::is_ui_command("/Users/x/node_modules/fastpilot/vendor/fastpilot ui"));
        assert!(super::is_ui_command("/Users/my dir/fastpilot ui --no-open"));
        for no in ["/usr/local/bin/fastpilot mcp", "python3 x.py fastpilot ui", "/bin/sh -c fastpilot", "fastpilot-ui ui", "/opt/notfastpilot ui", ""] {
            assert!(!super::is_ui_command(no), "{no}");
        }
        assert_eq!(super::token().len(), 32);
        assert_ne!(super::token(), super::token());
    }
}
