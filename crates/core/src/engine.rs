//! Flow engine: multi-session registry, auto-wait, lookup retry, stop-early, per-session diffs.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::Instant;

use crate::dsl::{Condition, HttpRequest, OtpSource, Random, ReadOtp, add_secret, looks_secret, substitute, substitute_with};
use crate::snapshot::{DEFAULT_MAX_LINES, shows_nothing_new};
use crate::{Driver, Error, Flow, FlowStep, Intent, Platform, Rect, Result, Selector, SessionId, Snapshot, Step, diff};

/// How long one action (tap, type, select, scroll ...) may take: `action_timeout` seconds
/// (default 5).
fn action_timeout() -> Duration {
    let secs = crate::config::var("FASTPILOT_ACTION_TIMEOUT").ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|s| *s > 0).unwrap_or(5);
    Duration::from_secs(secs)
}

/// Typing on a phone waits for the keyboard too, which slow devices and emulators take seconds to
/// show: three times the action limit there.
fn input_timeout(d: &dyn Driver) -> Duration {
    match d.platform() {
        Platform::Android | Platform::Ios => action_timeout() * 3,
        _ => action_timeout(),
    }
}
const ASSERT_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(3);
const SCROLL_TIMEOUT: Duration = Duration::from_secs(20);
/// `repeat: { while: ... }` without `times` stops here.
const MAX_ITERATIONS: u32 = 1000;

/// Live sessions of one fastpilot process, across platforms (PRD §8.1). Drivers are shared
/// (`Arc`) so a flow can run on a cloned registry while other tools keep using the original;
/// `absorb` merges the flow's session changes back.
#[derive(Default, Clone)]
pub struct SessionRegistry {
    sessions: HashMap<SessionId, Arc<dyn Driver>>,
    active: Option<SessionId>,
}

/// A session id is a plain name: 1–64 letters, digits, `-` or `_`. Files are named after it
/// (`snapshot file: true`), so `../x` must never be one (Security, v0.4).
pub fn check_session_id(id: &str) -> Result<()> {
    let ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(Error::Session(format!("invalid session id `{}`: use 1 to 64 letters, digits, - or _", id.escape_debug())))
    }
}

impl SessionRegistry {
    pub fn insert(&mut self, id: SessionId, driver: Box<dyn Driver>) -> Result<()> {
        check_session_id(&id)?;
        if self.sessions.contains_key(&id) {
            return Err(Error::Session(format!("session `{id}` already exists")));
        }
        let driver: Arc<dyn Driver> = Arc::from(driver);
        crate::live::session_started(&id, &driver.platform().to_string(), driver.live_screen());
        crate::live::attach(&id, &driver);
        self.sessions.insert(id.clone(), driver);
        self.active = Some(id);
        Ok(())
    }

    pub fn shared(&self, id: &str) -> Option<Arc<dyn Driver>> {
        self.sessions.get(id).cloned()
    }

    /// Apply what a flow did on `after` (a clone taken as `before`): sessions it started,
    /// replaced or stopped, and the new active session. Sessions other tools changed meanwhile
    /// are left alone.
    pub fn absorb(&mut self, before: &SessionRegistry, after: SessionRegistry) {
        for (id, d) in &before.sessions {
            let still_same = self.sessions.get(id).is_some_and(|cur| Arc::ptr_eq(cur, d));
            if !after.sessions.contains_key(id) && still_same {
                self.sessions.remove(id);
                crate::live::session_ended(id);
                crate::claim::release(id);
            }
        }
        for (id, d) in after.sessions {
            if !before.sessions.get(&id).is_some_and(|b| Arc::ptr_eq(b, &d)) {
                self.sessions.insert(id, d);
            }
        }
        if let Some(a) = after.active.filter(|a| self.sessions.contains_key(a)) {
            self.active = Some(a);
        }
    }

    /// Close and drop every session: device settings are restored and on-device helpers
    /// stopped before the caller moves on (a Drop fallback would race the process exit).
    pub async fn close_all(&mut self) {
        self.active = None;
        for (id, d) in std::mem::take(&mut self.sessions) {
            crate::live::session_ended(&id);
            crate::claim::release(&id);
            if let Err(e) = d.close().await {
                tracing::warn!(session = %id, error = %e, "session cleanup failed");
            }
        }
    }

    pub fn remove(&mut self, id: &str) -> Option<Arc<dyn Driver>> {
        if self.active.as_deref() == Some(id) {
            self.active = None;
        }
        crate::live::session_ended(id);
        crate::claim::release(id);
        self.sessions.remove(id)
    }

    pub fn get(&self, id: &str) -> Result<&dyn Driver> {
        self.sessions
            .get(id)
            .map(|d| &**d)
            .ok_or_else(|| Error::Session(format!("unknown session `{id}`")))
    }

    pub fn contains(&self, id: &str) -> bool {
        self.sessions.contains_key(id)
    }

    pub fn ids(&self) -> Vec<SessionId> {
        let mut ids: Vec<_> = self.sessions.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// The last active session, or the only one.
    pub fn active(&self) -> Option<&str> {
        match (&self.active, self.sessions.len()) {
            (Some(a), _) => Some(a),
            (None, 1) => self.sessions.keys().next().map(String::as_str),
            _ => None,
        }
    }

    pub fn set_active(&mut self, id: &str) -> Result<()> {
        self.get(id)?;
        self.active = Some(id.to_string());
        Ok(())
    }

    /// `explicit` if given, else the active session.
    pub fn pick(&self, explicit: Option<&str>) -> Result<(&str, &dyn Driver)> {
        let id = explicit
            .or_else(|| self.active())
            .ok_or_else(|| Error::Session("no active session; start one first".into()))?;
        let (id, d) = self
            .sessions
            .get_key_value(id)
            .ok_or_else(|| Error::Session(format!("unknown session `{id}`")))?;
        Ok((id, &**d))
    }
}

/// Creates drivers for `session: { start: ... }`; implemented where the platform crates are known.
#[async_trait]
pub trait Launcher: Send + Sync {
    /// `device`: a `DeviceSpec` (tagged id, plain id or name); `None` = the platform default.
    async fn launch(&self, platform: Platform, device: Option<&str>) -> Result<Box<dyn Driver>>;
    /// `run --reset` / `mcp --reset`: the apps reset so far in this run; `None` without --reset.
    fn fresh(&self) -> Option<std::sync::Arc<crate::FreshApps>> {
        None
    }
}

/// `--reset`: before a mobile session opens an app, the app is reset to a fresh install (data,
/// permissions) and the screen put upright, so every test starts from the same state. Web needs
/// nothing: each session already gets its own browser context (no cookies, no storage). Wiping app
/// data is a guard rail: refused while allow_app_reset = false. Returns what was reset, if anything.
pub async fn reset_app(driver: &dyn Driver, platform: Platform, target: &str) -> Result<Option<String>> {
    if platform == Platform::Web || target.contains("://") {
        return Ok(None);
    }
    if !crate::config::flag("allow_app_reset") {
        return Err(Error::Blocked(
            "--reset wipes the app's data, which is turned off (allow_app_reset = false); only the user can change it in fastpilot settings".into(),
        ));
    }
    let app = target.split('/').next().unwrap_or(target);
    driver.reset_app(app).await.map(|done| Some(format!("reset {app}: {done}")))
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Initial active session; defaults to the registry's active session.
    pub session: Option<SessionId>,
    pub stop_on_fail: bool,
    /// Whole-flow budget; remaining steps fail with `timeout` once it is spent.
    pub deadline: Duration,
}

impl Default for RunOptions {
    fn default() -> Self {
        let secs = crate::config::var("FASTPILOT_FLOW_TIMEOUT").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
        Self {
            session: None,
            stop_on_fail: true,
            deadline: Duration::from_secs(secs),
        }
    }
}

/// Existing sessions a flow will touch, known before running it (for per-session locking).
pub fn sessions_used(flow: &Flow, initial: Option<&str>) -> BTreeSet<SessionId> {
    let mut out: BTreeSet<SessionId> = initial.map(str::to_owned).into_iter().collect();
    for s in &flow.steps {
        match &s.step {
            Step::UseSession(id) | Step::StartSession { id, reset: true, .. } => {
                out.insert(id.clone());
            }
            Step::Parallel(b) => out.extend(b.iter().map(|b| b.session.clone())),
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct StepRecord {
    /// 1-based position among all leaf steps (parallel branches included).
    pub index: usize,
    pub session: Option<SessionId>,
    pub label: String,
    pub elapsed: Duration,
    pub output: Option<String>,
    pub error: Option<Error>,
    /// For steps that targeted a ref: the role+name (or id) it resolved to, for path memory.
    pub durable: Option<Selector>,
    /// This step failed to find what it looked for right after a tap (long press, double
    /// tap) on its session that reported success but left the screen as it was: most likely
    /// the action came too early (an animation swallowed it), not a product bug.
    pub after_no_effect: bool,
    /// Wall-clock start of the step: places it on a session recording's timeline (report.html).
    pub started: std::time::SystemTime,
    timeout: Option<Duration>,
    target: Option<(Selector, Intent)>,
}

impl StepRecord {
    /// The failure category for triage: `timing` when the step failed only because the
    /// previous action had no visible effect, else the error's own category.
    pub fn category(&self) -> Option<&'static str> {
        self.error.as_ref().map(|e| if self.after_no_effect { "timing" } else { e.category() })
    }
}

#[derive(Debug, Clone)]
pub struct FlowReport {
    pub total: usize,
    pub elapsed: Duration,
    pub records: Vec<StepRecord>,
    /// Candidate refs for the first failed step.
    pub candidates: Vec<String>,
    /// One block per touched session: diff, or full snapshot when the screen changed or failed.
    pub states: Vec<String>,
}

impl FlowReport {
    pub fn passed(&self) -> bool {
        self.records.len() == self.total && self.records.iter().all(|r| r.error.is_none())
    }

    fn first_failure(&self) -> Option<&StepRecord> {
        self.records.iter().find(|r| r.error.is_some())
    }
}

impl fmt::Display for FlowReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.elapsed.as_secs_f64();
        match self.first_failure() {
            None if self.passed() => writeln!(f, "ok {}/{} steps {secs:.2}s", self.total, self.total)?,
            None => writeln!(f, "incomplete {}/{} steps {secs:.2}s", self.records.len(), self.total)?,
            Some(r) => {
                let e = r.error.as_ref().map(|e| (e.reason(), e.to_string())).unwrap_or_default();
                let timeout = r.timeout.map(|t| format!(" (timeout {}ms)", t.as_millis())).unwrap_or_default();
                writeln!(f, "fail step {}/{} {}{}: {}: {}{timeout}", r.index, self.total, prefix(&r.session), r.label, e.0, e.1)?;
                if r.after_no_effect {
                    writeln!(f, "note: the previous tap reported success but changed nothing on screen; it most likely came too early (category timing, not product): retry it or add waitForAnimationToEnd")?;
                }
                if let Some(Error::Ambiguous { candidates, .. }) = &r.error {
                    writeln!(f, "candidates: {}", candidates.join(", "))?;
                } else if !self.candidates.is_empty() {
                    writeln!(f, "candidates: {}", self.candidates.join(", "))?;
                }
            }
        }
        for r in &self.records {
            let mark = if r.error.is_some() { "✗" } else { "✓" };
            let out = r.output.as_deref().map(|o| format!(" = {o}")).unwrap_or_default();
            let reason = r.error.as_ref().map(|e| format!(" {}", e.reason())).unwrap_or_default();
            writeln!(f, "  {mark} {} {}{} {}ms{out}{reason}", r.index, prefix(&r.session), r.label, r.elapsed.as_millis())?;
        }
        for s in &self.states {
            writeln!(f, "{s}")?;
        }
        Ok(())
    }
}

fn prefix(session: &Option<SessionId>) -> String {
    session.as_deref().map(|s| format!("[{s}] ")).unwrap_or_default()
}

struct Ctx {
    records: Mutex<Vec<StepRecord>>,
    stop_on_fail: bool,
    deadline: Instant,
    /// Flow-local `${VARS}` (runFlow `env`), visible to every later step.
    vars: Mutex<HashMap<String, String>>,
    /// `copyTextFrom` → `pasteText`.
    clipboard: Mutex<Option<String>>,
    /// Name given to `startRecording`, used by `stopRecording`.
    recording: Mutex<Option<String>>,
    /// Per session: the step index of the last tap-like action and the screen before it,
    /// while only checks followed it (failure triage, see `StepRecord::after_no_effect`).
    last_action: Mutex<HashMap<String, (usize, Snapshot)>>,
    /// This flow in the Live panel (`crate::live`), when the process publishes one.
    live: Option<u64>,
}

fn guard<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Ctx {
    fn record(&self, r: StepRecord) {
        guard(&self.records).push(r);
    }

    fn subst(&self, s: &str) -> Result<String> {
        substitute_with(s, &guard(&self.vars)).map_err(Error::Flow)
    }
}

fn leaf_count(steps: &[FlowStep]) -> usize {
    steps
        .iter()
        .map(|s| match &s.step {
            Step::Parallel(b) => b.iter().map(|b| b.steps.len()).sum(),
            _ => 1,
        })
        .sum()
}

/// Run a flow against the registry. Never panics on driver errors: every failure ends up in
/// the report with its reason and the state of the sessions involved.
pub async fn run_flow(
    reg: &mut SessionRegistry,
    launcher: &dyn Launcher,
    flow: &Flow,
    opts: &RunOptions,
) -> FlowReport {
    let started = Instant::now();
    let ctx = Ctx {
        records: Mutex::new(Vec::new()),
        stop_on_fail: opts.stop_on_fail,
        deadline: started + opts.deadline,
        vars: Mutex::new(HashMap::new()),
        clipboard: Mutex::new(None),
        recording: Mutex::new(None),
        last_action: Mutex::new(HashMap::new()),
        live: crate::live::flow_started(live_plan(flow)),
    };
    let mut baselines: Vec<(SessionId, Option<Snapshot>)> = Vec::new();
    let mut active = opts.session.clone().or_else(|| reg.active().map(str::to_owned));
    let mut index = 0;

    for fs in &flow.steps {
        if Instant::now() >= ctx.deadline {
            index += 1;
            let msg = format!("flow exceeded its {}s budget (FASTPILOT_FLOW_TIMEOUT)", opts.deadline.as_secs());
            record(&ctx, index, active.as_deref(), &fs.step, Instant::now(), Err(Error::Timeout(msg)));
            break;
        }
        let ok = match &fs.step {
            Step::StartSession { id, platform, target, reset, device } => {
                index += 1;
                let t0 = Instant::now();
                if *reset {
                    reg.remove(id);
                }
                crate::live::step_started(ctx.live, index, Some(id), None);
                let res = start_session(reg, launcher, id, *platform, target.as_deref(), device.as_deref()).await;
                if res.is_ok() {
                    active = Some(id.clone());
                    baselines.retain(|(s, _)| s != id);
                    baselines.push((id.clone(), None));
                }
                record(&ctx, index, Some(id), &fs.step, t0, res)
            }
            Step::UseSession(id) => {
                index += 1;
                let res = if reg.contains(id) {
                    active = Some(id.clone());
                    Ok(None)
                } else {
                    Err(Error::Session(format!("unknown session `{id}`")))
                };
                record(&ctx, index, Some(id), &fs.step, Instant::now(), res)
            }
            Step::Parallel(branches) => {
                for b in branches {
                    ensure_baseline(reg, &mut baselines, &b.session).await;
                }
                let reg: &SessionRegistry = reg;
                let mut first = index;
                let futs: Vec<_> = branches
                    .iter()
                    .map(|b| {
                        let start = first;
                        first += b.steps.len();
                        run_branch(reg, &b.session, &b.steps, start, &ctx)
                    })
                    .collect();
                index = first;
                if ctx.stop_on_fail {
                    // Fail-fast: the first failing branch cancels the others (PRD §8.1).
                    futures_util::future::try_join_all(futs).await.is_ok()
                } else {
                    futures_util::future::join_all(futs).await.iter().all(Result::is_ok)
                }
            }
            step if step.sessionless() => {
                index += 1;
                let t0 = Instant::now();
                let res = sessionless(step, &ctx, None).await;
                record(&ctx, index, active.as_deref(), step, t0, res)
            }
            step => {
                index += 1;
                match active.clone() {
                    None => record(
                        &ctx,
                        index,
                        None,
                        step,
                        Instant::now(),
                        Err(Error::Session("no active session; add `session: { start: ... }` first".into())),
                    ),
                    Some(sid) => {
                        ensure_baseline(reg, &mut baselines, &sid).await;
                        match reg.get(&sid) {
                            Ok(d) => run_one(d, &sid, step, index, &ctx).await,
                            Err(e) => record(&ctx, index, Some(&sid), step, Instant::now(), Err(e)),
                        }
                    }
                }
            }
        };
        if !ok && opts.stop_on_fail {
            break;
        }
    }
    if let Some(a) = &active {
        let _ = reg.set_active(a);
    }
    crate::live::flow_finished(ctx.live);

    let mut records = ctx.records.into_inner().unwrap_or_else(|p| p.into_inner());
    records.sort_by_key(|r| r.index);
    let failed = records.iter().find(|r| r.error.is_some());
    let mut candidates = Vec::new();
    let mut states = Vec::new();
    let mut root_cause = None;
    let mut no_effect = None;
    for (sid, base) in &baselines {
        let Ok(d) = reg.get(sid) else { continue };
        let snap = match d.snapshot().await {
            Ok(s) => s,
            Err(e) => {
                // The app died or hung: that, not the timeout it caused, is why the step failed.
                if matches!(e, Error::AppCrashed(_) | Error::AppNotResponding(_))
                    && let Some(f) = failed.filter(|f| f.session.as_deref() == Some(sid.as_str()))
                {
                    root_cause = Some((f.index, e.clone()));
                }
                states.push(format!("[{sid}] snapshot failed: {}: {e}", e.reason()));
                continue;
            }
        };
        let failed_here = failed.filter(|f| f.session.as_deref() == Some(sid.as_str()));
        if let Some(f) = failed_here
            && matches!(f.error, Some(Error::NotFound(_) | Error::Assertion(_)))
            && let Some((at, before)) = guard(&ctx.last_action).get(sid.as_str())
            && *at < f.index
            && shows_nothing_new(before, &snap)
        {
            no_effect = Some(f.index);
        }
        if let Some((sel, intent)) = failed_here.and_then(|f| f.target.as_ref()) {
            candidates = snap.candidates(sel, *intent);
        }
        let lines = match (failed_here, base) {
            (None, Some(base)) => diff(base, &snap),
            _ => None,
        };
        let mut block = match lines {
            Some(l) if l.is_empty() => format!("[{sid}] {}\n  (no changes)", snap.header()),
            Some(l) => format!("[{sid}] {}\n  {}", snap.header(), l.join("\n  ")),
            None => format!("[{sid}] {}", snap.render(None, DEFAULT_MAX_LINES)),
        };
        if failed_here.is_some() {
            let diag = d.diagnostics().await;
            if !diag.is_empty() {
                block.push_str(&format!("\n[{sid}] recent errors:\n  {}", diag.join("\n  ")));
            }
        }
        states.push(block);
    }
    if let Some((index, e)) = root_cause
        && let Some(r) = records.iter_mut().find(|r| r.index == index && r.error.is_some())
    {
        r.error = Some(e);
    }
    if let Some(index) = no_effect
        && let Some(r) = records.iter_mut().find(|r| r.index == index && r.error.is_some())
    {
        r.after_no_effect = true;
    }
    FlowReport {
        total: leaf_count(&flow.steps),
        elapsed: started.elapsed(),
        records,
        candidates,
        states,
    }
}

async fn start_session(
    reg: &mut SessionRegistry,
    launcher: &dyn Launcher,
    id: &str,
    platform: Platform,
    target: Option<&str>,
    device: Option<&str>,
) -> Result<Option<String>> {
    check_session_id(id)?;
    if reg.contains(id) {
        return Err(Error::Session(format!("session `{id}` already exists")));
    }
    // D10d: one test per device, at most `max_parallel` at once; otherwise wait in the queue.
    crate::claim::acquire(id, platform, device).await?;
    let driver = launcher.launch(platform, device).await.inspect_err(|_| crate::claim::release(id))?;
    let mut done = None;
    if let Some(t) = target {
        let t = substitute(t).map_err(Error::Flow)?;
        if crate::dsl::holds_app_secret(&t) {
            let at = (platform == Platform::Web).then_some(t.as_str());
            crate::secrets::allowed_at(at, platform).map_err(Error::Flow)?;
        }
        if platform != Platform::Web
            && let Some(f) = launcher.fresh()
            && f.claim(&t)
        {
            done = reset_app(&*driver, platform, &t).await?;
        }
        driver.open(&t).await?;
        settle(&*driver).await;
    }
    reg.insert(id.to_string(), driver)?;
    Ok(done)
}

async fn ensure_baseline(reg: &SessionRegistry, baselines: &mut Vec<(SessionId, Option<Snapshot>)>, sid: &str) {
    if baselines.iter().any(|(s, _)| s == sid) {
        return;
    }
    let snap = match reg.get(sid) {
        Ok(d) => d.snapshot().await.ok(),
        Err(_) => None,
    };
    baselines.push((sid.to_string(), snap));
}

async fn run_branch(
    reg: &SessionRegistry,
    sid: &str,
    steps: &[FlowStep],
    before: usize,
    ctx: &Ctx,
) -> std::result::Result<(), ()> {
    let d = match reg.get(sid) {
        Ok(d) => d,
        Err(e) => {
            if let Some(s) = steps.first() {
                record(ctx, before + 1, Some(sid), &s.step, Instant::now(), Err(e));
            }
            return Err(());
        }
    };
    let mut failed = false;
    for (i, s) in steps.iter().enumerate() {
        if Instant::now() >= ctx.deadline {
            let msg = "flow exceeded its time budget (FASTPILOT_FLOW_TIMEOUT)".to_owned();
            record(ctx, before + i + 1, Some(sid), &s.step, Instant::now(), Err(Error::Timeout(msg)));
            return Err(());
        }
        if !run_one(d, sid, &s.step, before + i + 1, ctx).await {
            failed = true;
            if ctx.stop_on_fail {
                break;
            }
        }
    }
    if failed { Err(()) } else { Ok(()) }
}

fn step_ref(step: &Step) -> Option<&Selector> {
    let sel = match step.unconfirmed() {
        Step::TapOn(s) | Step::InputText { into: s, .. } | Step::UploadFile { into: s, .. } => s,
        Step::LongPressOn(s) | Step::DoubleTapOn(s) | Step::CopyTextFrom(s) => s,
        Step::SelectOption { from: s, .. } => s,
        Step::AssertVisible { selector: s, .. } | Step::AssertNotVisible { selector: s, .. } => s,
        Step::Scroll { within: Some(s), .. } => s,
        _ => return None,
    };
    matches!(sel, Selector::Ref(_)).then_some(sel)
}

/// Refs are only valid in this session; path memory needs what they pointed at.
async fn durable(d: &dyn Driver, sel: &Selector) -> Option<Selector> {
    let snap = d.snapshot().await.ok()?;
    let entry = snap.resolve(sel, Intent::Find).ok()?;
    let node = &entry.node;
    if !node.name.is_empty() {
        return Some(Selector::Role { role: node.role.clone(), name: Some(node.name.clone()) });
    }
    // An id several elements share replays as an action only with its index.
    let by_id = Selector::Id(node.id.clone()?);
    let same = snap.matching(&by_id);
    match same.iter().position(|e| e.r#ref == entry.r#ref) {
        Some(index) if same.len() > 1 => Some(Selector::Nth { of: Box::new(by_id), index }),
        _ => Some(by_id),
    }
}

/// Taps whose effect the failure triage checks.
fn tap_like(step: &Step) -> bool {
    matches!(step.unconfirmed(), Step::TapOn(_) | Step::LongPressOn(_) | Step::DoubleTapOn(_))
}

/// Steps that only look at the screen: they keep the last action as "previous".
fn check_only(step: &Step) -> bool {
    matches!(
        step,
        Step::AssertVisible { .. } | Step::AssertNotVisible { .. } | Step::WaitForAnimationToEnd(_) | Step::TakeScreenshot(_) | Step::AssertScreenshot { .. }
    )
}

async fn run_one(d: &dyn Driver, sid: &str, step: &Step, index: usize, ctx: &Ctx) -> bool {
    // The user has this session taken over in `fastpilot ui`: wait until it is given back.
    crate::live::hold(sid).await;
    let t0 = Instant::now();
    crate::live::step_started(ctx.live, index, Some(sid), d.live_screen());
    let stable = match step_ref(step) {
        Some(sel) => durable(d, sel).await,
        None => None,
    };
    let before = if tap_like(step) { d.snapshot().await.ok() } else { None };
    let res = exec(d, step, ctx).await;
    match (before, &res) {
        (Some(b), Ok(_)) => {
            guard(&ctx.last_action).insert(sid.to_owned(), (index, b));
        }
        _ if check_only(step) => {}
        _ => {
            guard(&ctx.last_action).remove(sid);
        }
    }
    if let Err(e) = &res {
        tracing::debug!(session = sid, step = %step.label(), error = %e, "step failed");
    }
    let ok = record(ctx, index, Some(sid), step, t0, res);
    if stable.is_some() {
        let mut records = ctx.records.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(r) = records.iter_mut().rev().find(|r| r.index == index) {
            r.durable = stable;
        }
    }
    ok
}

fn record(ctx: &Ctx, index: usize, session: Option<&str>, step: &Step, t0: Instant, res: Result<Option<String>>) -> bool {
    let ok = res.is_ok();
    let (timeout, target) = match step.unconfirmed() {
        Step::TapOn(s) => (Some(action_timeout()), Some((s.clone(), Intent::Tap))),
        Step::InputText { into, .. } | Step::UploadFile { into, .. } => {
            (Some(action_timeout()), Some((into.clone(), Intent::Type)))
        }
        Step::SelectOption { from, .. } | Step::LongPressOn(from) | Step::DoubleTapOn(from) => {
            (Some(action_timeout()), Some((from.clone(), Intent::Tap)))
        }
        Step::CopyTextFrom(s) => (Some(action_timeout()), Some((s.clone(), Intent::Find))),
        Step::ScrollUntilVisible { selector, timeout, .. } => {
            (Some(timeout.unwrap_or(SCROLL_TIMEOUT)), Some((selector.clone(), Intent::Find)))
        }
        Step::AssertVisible { selector, timeout } => {
            (Some(timeout.unwrap_or(ASSERT_TIMEOUT)), Some((selector.clone(), Intent::Find)))
        }
        _ => (None, None),
    };
    let (output, error) = match res {
        Ok(o) => (o, None),
        Err(e) => (None, Some(e)),
    };
    crate::live::step_finished(ctx.live, index, session, error.as_ref().map_or(Ok(()), |e| Err((matches!(e, Error::Blocked(_)), e.to_string()))));
    ctx.record(StepRecord {
        index,
        session: session.map(str::to_owned),
        label: step.label(),
        elapsed: t0.elapsed(),
        output,
        durable: None,
        after_no_effect: false,
        started: std::time::SystemTime::now() - t0.elapsed(),
        // Retry exhaustion is the only case where the timeout is the relevant detail.
        timeout: timeout.filter(|_| error.as_ref().is_some_and(Error::is_retryable)),
        error,
        target,
    });
    ok
}

type StepFuture<'a> = std::pin::Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>>;

/// Boxed: blocks (runFlow, repeat, retry) run their steps through `exec` again.
fn exec<'a>(d: &'a dyn Driver, step: &'a Step, ctx: &'a Ctx) -> StepFuture<'a> {
    exec_with(d, step, ctx, false)
}

/// Boxes of `ignore` selectors in screenshot pixels. Snapshot boxes are in CSS pixels (web) or
/// points (iOS) or pixels (Android); the scale comes from the viewport when the driver knows it,
/// else from the widest box on screen, rounded to a whole device scale.
async fn masks_for(d: &dyn Driver, ignore: &[Selector], shot: &[u8]) -> Result<Vec<crate::visual::Mask>> {
    let img_w = image::load_from_memory(shot).map(|i| i.width()).map_err(|e| Error::Driver(format!("screenshot: {e}")))? as f64;
    let snap = d.snapshot().await?;
    let screen_w = match d.viewport().await {
        Some(v) if v.w > 0.0 => v.w,
        _ => snap.entries.iter().filter_map(|e| e.node.bounds.map(|b| b.x + b.w)).fold(0.0, f64::max),
    };
    let scale = if screen_w > 0.0 { (img_w / screen_w).round().max(1.0) } else { 1.0 };
    let (ox, oy) = d.viewport().await.map(|v| (v.x, v.y)).unwrap_or((0.0, 0.0));
    let mut out = Vec::new();
    for sel in ignore {
        let e = snap.resolve(sel, Intent::Find)?;
        let b = e.node.bounds.ok_or_else(|| Error::NotActionable(format!("{sel} has no box to ignore")))?;
        let px = |v: f64| (v * scale).max(0.0) as u32;
        out.push(crate::visual::Mask { x: px(b.x - ox), y: px(b.y - oy), w: px(b.w) + 2, h: px(b.h) + 2 });
    }
    Ok(out)
}

/// The flow's leaf steps for the Live panel, numbered like `run_flow` numbers them, each with
/// what a guard rail will say about it (judged from the step alone, before it runs).
fn live_plan(flow: &Flow) -> Vec<(String, Option<String>, Option<String>)> {
    let mut out = Vec::new();
    for fs in &flow.steps {
        match &fs.step {
            Step::Parallel(branches) => {
                for b in branches {
                    out.extend(b.steps.iter().map(|s| (s.step.label(), will_block(&s.step), Some(b.session.clone()))));
                }
            }
            step => out.push((step.label(), will_block(step), None)),
        }
    }
    out
}

/// What the guard rails will refuse about `step`, in the words of the Live panel.
fn will_block(step: &Step) -> Option<String> {
    let flag_off = |key: &str| !crate::config::flag(key);
    let (inner, confirmed) = match step {
        Step::Confirmed(inner) => (&**inner, true),
        other => (other, false),
    };
    match inner {
        Step::TapOn(sel) | Step::LongPressOn(sel) | Step::DoubleTapOn(sel) => {
            let (Selector::Text(name) | Selector::Role { name: Some(name), .. }) = sel else { return None };
            if let Some(hit) = crate::policy::block_match(name, &crate::policy::block_rules()) {
                return Some(format!("will be blocked: the text contains \"{}\" ({})", hit.word, block_source(&hit)));
            }
            match crate::policy::risky_match(name, &crate::policy::risky_words()) {
                Some(w) if !confirmed => Some(format!("will stop: needs confirm: true (\"{w}\", confirm_actions)")),
                _ => None,
            }
        }
        Step::OpenUrl(url) if !url.contains("${") && !crate::policy::url_allowed(url, &crate::policy::allowed_domains()) => {
            Some("will be blocked: the domain is outside allowed_domains".into())
        }
        Step::EvalScript(_) if flag_off("allow_eval_script") => Some("will be blocked: evalScript is turned off (allow_eval_script)".into()),
        Step::EvalScript(_) if !crate::policy::blocked_words().is_empty() && !crate::policy::eval_unlocked() => {
            Some("will be blocked: evalScript is off while the block list is on (turn on \"Run any JavaScript\" in Limits)".into())
        }
        Step::ClearState(_) | Step::ClearKeychain if flag_off("allow_app_reset") => Some("will be blocked: clearing app data is turned off (allow_app_reset)".into()),
        Step::InstallApp(_) | Step::UninstallApp(_) if flag_off("allow_app_install") => {
            Some("will be blocked: installing or removing apps is turned off (allow_app_install)".into())
        }
        _ => None,
    }
}

/// FASTPILOT_CONFIRM_ACTIONS: a tap on an element named like a risky action ("Hapus akun",
/// "Bayar") needs `confirm: true` in the step.
async fn guard_risky(d: &dyn Driver, sel: &Selector, confirmed: bool) -> Result<()> {
    guard_risky_with(d, sel, confirmed, &crate::policy::risky_words()).await
}

async fn guard_risky_with(d: &dyn Driver, sel: &Selector, confirmed: bool, words: &[String]) -> Result<()> {
    guard_tap(d, sel, confirmed, words, &crate::policy::block_rules()).await
}

/// The selector, with the element's name when it says more (`e3 ("Bayar")`), not twice
/// (`"Bayar sekarang"`, not `"Bayar sekarang" ("Bayar sekarang")`).
fn labelled(sel: &Selector, name: &str) -> String {
    match sel {
        Selector::Text(t) | Selector::Role { name: Some(t), .. } if t.trim().eq_ignore_ascii_case(name.trim()) => sel.to_string(),
        _ if name.is_empty() => sel.to_string(),
        _ => format!("{sel} ({})", crate::quote(name)),
    }
}

/// `block_actions` (always, confirmed or not), then `confirm_actions` (unless confirmed).
async fn guard_tap(d: &dyn Driver, sel: &Selector, confirmed: bool, words: &[String], blocked: &[String]) -> Result<()> {
    if (confirmed || words.is_empty()) && blocked.is_empty() {
        return Ok(());
    }
    if let Selector::Point(p) = sel {
        // A tap by coordinates is checked against every element under it; when that cannot be told,
        // the rules win (fail closed).
        let names = point_targets(d, *p).await.ok_or_else(|| {
            Error::Blocked(format!(
                "{sel}: cannot tell which element is at this point (no snapshot, or no element covers it) while blocked or risky words are set (Limits & permissions); tap it by text or ref"
            ))
        })?;
        return judge_tap(sel, &names, confirmed, words, blocked);
    }
    // The step's own text first: refused at once, without waiting for an element that may never
    // come (`tapOn: Keluar` with "Keluar" blocked).
    let own = match sel {
        Selector::Text(t) | Selector::Role { name: Some(t), .. } => Some(t.clone()),
        _ => None,
    };
    if let Some(t) = &own {
        judge_tap(sel, std::slice::from_ref(t), confirmed, words, blocked)?;
    }
    // Then the element the tap will hit, waited for like the tap waits: checked once before it was
    // on screen, `{ id: pay }` passed and the tap's retry then hit "Bayar" when it appeared.
    let name = std::sync::Mutex::new(None::<String>);
    let no_snapshot = std::sync::Mutex::new(false);
    let found = retry(action_timeout(), || async {
        let snap = d.snapshot().await;
        *no_snapshot.lock().unwrap_or_else(|p| p.into_inner()) = snap.is_err();
        let snap = snap?;
        let e = snap.resolve(sel, Intent::Tap)?;
        *name.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.node.name.clone());
        Ok(())
    })
    .await;
    let no_snapshot = *no_snapshot.lock().unwrap_or_else(|p| p.into_inner());
    match found {
        Ok(()) => judge_tap(sel, &[name.into_inner().unwrap_or_else(|p| p.into_inner()).unwrap_or_default()], confirmed, words, blocked),
        // No snapshot: a text selector was judged by its text above; a ref, id or role cannot be
        // told apart, so the rules win (fail closed), like a point.
        Err(_) if no_snapshot && own.is_some() => Ok(()),
        Err(_) if no_snapshot => Err(Error::Blocked(format!(
            "{sel}: cannot tell which element this is (no snapshot) while blocked or risky words are set (Limits & permissions); tap it by text"
        ))),
        // Not on screen (or ambiguous) within the step's time: the tap would fail the same way.
        Err(e) => Err(e),
    }
}

/// `block_actions` (always), then `confirm_actions` (unless confirmed), against `names`.
fn judge_tap(sel: &Selector, names: &[String], confirmed: bool, words: &[String], blocked: &[String]) -> Result<()> {
    for name in names {
        if let Some(hit) = crate::policy::block_match(name, blocked) {
            return Err(Error::Blocked(format!("{} contains \"{}\" ({}). {BLOCKED_HOW}", labelled(sel, name), hit.word, block_source(&hit))));
        }
    }
    if confirmed {
        return Ok(());
    }
    for name in names {
        if let Some(w) = crate::policy::risky_match(name, words) {
            return Err(Error::Blocked(format!(
                "{} looks like a risky action (\"{w}\", confirm_actions): if it is intended, \
                 write the step with `confirm: true`, e.g. tapOn: {{ text: \"{name}\", confirm: true }}",
                labelled(sel, name)
            )));
        }
    }
    Ok(())
}

/// The names of every element whose box covers page point `p` (a button, the dialog it sits in, the
/// nameless icon inside it): a rule matching any of them applies. `None` when it cannot be told: no
/// snapshot, or no element covers the point.
async fn point_targets(d: &dyn Driver, p: crate::Point) -> Option<Vec<String>> {
    let snap = d.snapshot().await.ok()?;
    let covering: Vec<String> = snap.entries.iter().filter(|e| e.node.bounds.is_some_and(|b| b.contains(p))).map(|e| e.node.name.clone()).collect();
    (!covering.is_empty()).then_some(covering)
}

/// `{ point: [x, y] }` is a page position, the one `snapshot coords` shows. The driver taps screen
/// positions, so it gets the spot's position on screen (minus the scroll offset); a point that is
/// not on screen is refused rather than tapped somewhere else. Also returns the element the tap
/// lands on (the deepest one covering the point), for the step report. Other selectors pass through.
async fn point_on_screen(d: &dyn Driver, sel: &Selector) -> Result<(Selector, Option<String>)> {
    let Selector::Point(p) = sel else { return Ok((sel.clone(), None)) };
    let hit = d.snapshot().await.ok().and_then(|snap| {
        snap.entries
            .iter()
            .filter(|e| e.node.bounds.is_some_and(|b| b.contains(*p)))
            .max_by_key(|e| e.depth)
            .map(|e| format!("on {} {} {}", e.r#ref, e.node.role, crate::quote(&e.node.name)))
    });
    let Some(v) = d.viewport().await else { return Ok((sel.clone(), hit)) };
    let on_screen = crate::Point { x: p.x - v.x, y: p.y - v.y };
    if on_screen.x < 0.0 || on_screen.y < 0.0 || on_screen.x >= v.w || on_screen.y >= v.h {
        return Err(Error::NotActionable(format!(
            "{sel} is not on screen (page coordinates, as `snapshot coords` shows them; the screen shows x {:.0}..{:.0}, y {:.0}..{:.0}): scroll it into view first, or tap by ref",
            v.x,
            v.x + v.w,
            v.y,
            v.y + v.h
        )));
    }
    Ok((Selector::Point(on_screen), hit))
}

/// Enter (and Space on a button) acts like a tap: on the focused button, or, from a form field, on
/// whichever button submits the form, which the snapshot cannot tell. So with the focus in a field
/// (or unknown), a key that may submit is refused while any button on screen is blocked
/// (block_actions) or risky (confirm_actions, unless confirmed). No snapshot while a rule is on:
/// refused (fail closed). Other keys are not checked.
async fn guard_key(d: &dyn Driver, key: &str, confirmed: bool, words: &[String], blocked: &[String]) -> Result<()> {
    let submits = matches!(key.to_ascii_lowercase().as_str(), "enter" | "return" | "space" | " ");
    if !submits || ((confirmed || words.is_empty()) && blocked.is_empty()) {
        return Ok(());
    }
    let snap = d.snapshot().await.map_err(|_| {
        Error::Blocked(format!(
            "pressKey {key}: cannot tell what it would press (no snapshot) while blocked or risky words are set (Limits & permissions); tap the button by text or ref"
        ))
    })?;
    let focused = snap.entries.iter().find(|e| e.node.focused);
    let on_button = focused.is_some_and(|e| !crate::snapshot::EDITABLE.contains(&e.node.role.as_str()));
    let candidates: Vec<&str> = match focused {
        Some(e) if on_button => vec![e.node.name.as_str()],
        _ => snap.entries.iter().filter(|e| crate::snapshot::INTERACTIVE.contains(&e.node.role.as_str())).map(|e| e.node.name.as_str()).collect(),
    };
    let what = if on_button { "presses" } else { "may submit the form to" };
    for name in &candidates {
        if let Some(hit) = crate::policy::block_match(name, blocked) {
            return Err(Error::Blocked(format!("pressKey {key} {what} \"{name}\", which contains \"{}\" ({}). {BLOCKED_HOW}", hit.word, block_source(&hit))));
        }
    }
    if confirmed {
        return Ok(());
    }
    for name in &candidates {
        if let Some(w) = crate::policy::risky_match(name, words) {
            return Err(Error::Blocked(format!(
                "pressKey {key} {what} \"{name}\", a risky action (\"{w}\", confirm_actions): if it is intended, write the step as pressKey: {{ key: {key}, confirm: true }}, or tap the right button by its text"
            )));
        }
    }
    Ok(())
}

/// What the agent (and the report) is told after a block_actions refusal.
const BLOCKED_HOW: &str =
    "Do not retry or work around it (`confirm: true` does not lift it). The user can change it in fastpilot ui › Settings › Limits & permissions.";

/// In plain words, never the setting's name (V042: the Pay answer showed `block_actions`).
fn block_source(hit: &crate::policy::BlockHit) -> &'static str {
    if hit.builtin { "the built-in blocked list" } else { "your blocked words" }
}

/// A script can click anything, which block_actions cannot see: no evalScript while it is set,
/// unless the user allowed it on purpose (`allow_eval_script = true`, checked by the caller).
fn guard_script(blocked: &[String]) -> Result<()> {
    if blocked.is_empty() {
        return Ok(());
    }
    Err(Error::Blocked(
        "evalScript is off while buttons are blocked (the built-in blocked list too): a script could press a blocked button. Do not work around it. The user can allow scripts in fastpilot ui › Settings › Limits & permissions (Run any JavaScript) or with `fastpilot config set allow_eval_script true`.".into(),
    ))
}

/// The `allow_*` permissions and `upload_dirs` (settings, pinned at start like every guard rail).
fn permitted(d: &dyn Driver, step: &Step) -> Result<()> {
    let refuse = |what: &str, key: &str| Err(Error::Blocked(format!("{what} is turned off ({key} = false); only the user can change it in fastpilot settings")));
    let mobile = matches!(d.platform(), Platform::Android | Platform::Ios);
    match step {
        Step::EvalScript(_) if !crate::config::flag("allow_eval_script") => refuse("evalScript", "allow_eval_script"),
        Step::ClearState(_) | Step::ClearKeychain if !crate::config::flag("allow_app_reset") => refuse("wiping app data (clearState / clearKeychain)", "allow_app_reset"),
        Step::InstallApp(_) | Step::UninstallApp(_) if !crate::config::flag("allow_app_install") => refuse("installing or uninstalling apps", "allow_app_install"),
        Step::SetNetwork(_) | Step::SetLocation { .. } | Step::SetPermissions { .. } | Step::SetLocale { .. }
            if mobile && !crate::config::flag("allow_device_settings") =>
        {
            refuse("changing device settings (network, location, permissions, locale)", "allow_device_settings")
        }
        Step::UploadFile { files, .. } => {
            let dirs = crate::policy::upload_dirs();
            match files.iter().find(|f| !crate::policy::upload_allowed(std::path::Path::new(f), &dirs)) {
                Some(f) => Err(Error::Blocked(format!(
                    "uploadFile `{f}` is outside the allowed upload folders (upload_dirs: {})",
                    dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
                ))),
                None => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

fn exec_with<'a>(d: &'a dyn Driver, step: &'a Step, ctx: &'a Ctx, confirmed: bool) -> StepFuture<'a> {
    Box::pin(async move {
        let subst = |s: &str| ctx.subst(s);
        permitted(d, step)?;
        // S-1 (E2): a step that uses a password runs only on its app's own site (an address it
        // opens is checked as the place it goes; anything else, the page that is open now). Steps
        // that hold others are checked step by step inside.
        if !matches!(step, Step::Confirmed(_) | Step::RunFlow { .. } | Step::Repeat { .. } | Step::Retry { .. } | Step::Parallel(_)) && holds_app_secret(ctx, &format!("{step:?}")) {
            // A script can send what it reads anywhere: never with an app's password (L1).
            if matches!(step, Step::EvalScript(_)) {
                return Err(Error::Flow("a password of this app is never put into evalScript: a script could send it anywhere".into()));
            }
            let at = match step {
                Step::OpenUrl(u) => Some(subst(u)?),
                // Where it is typed: the field's own document, also inside an iframe (M1).
                Step::InputText { into, .. } if d.platform() == Platform::Web => d.url_of(into).await,
                _ => d.current_url().await,
            };
            crate::secrets::allowed_at(at.as_deref(), d.platform()).map_err(Error::Flow)?;
            // D07-L3: on a phone, only into the app itself, never a browser or another app in front.
            if let Some(own) = crate::secrets::own_app(d.platform()) {
                let front = d.foreground_is(&own).await;
                crate::secrets::allowed_in_front(&own, front).map_err(Error::Flow)?;
            }
        }
        match step {
            Step::Confirmed(inner) => return exec_with(d, inner, ctx, true).await,
            Step::OpenUrl(url) => d.open(&subst(url)?).await?,
            Step::TapOn(sel) => {
                guard_risky(d, sel, confirmed).await?;
                let (sel, hit) = point_on_screen(d, sel).await?;
                retry(action_timeout(), || d.tap(&sel)).await?;
                if hit.is_some() {
                    return Ok(hit);
                }
            }
            Step::InputText { into, text } => {
                let text = subst(text)?;
                retry(input_timeout(d), || d.type_text(into, &text)).await?
            }
            Step::AssertVisible { selector, timeout } => {
                retry(timeout.unwrap_or(ASSERT_TIMEOUT), || async {
                    d.snapshot().await?.resolve(selector, Intent::Find).map(|_| ())
                })
                .await?;
                return Ok(None);
            }
            Step::Scroll { direction, within } => retry(action_timeout(), || d.scroll(*direction, within.as_ref())).await?,
            Step::Back => d.back().await?,
            Step::AssertNotVisible { selector, timeout } => {
                let timeout = timeout.unwrap_or(action_timeout());
                let deadline = Instant::now() + timeout;
                loop {
                    match d.snapshot().await?.resolve(selector, Intent::Find) {
                        Err(Error::NotFound(_)) => return Ok(None),
                        _ if Instant::now() >= deadline => {
                            return Err(Error::Timeout(format!("{selector} still visible after {}ms", timeout.as_millis())));
                        }
                        _ => tokio::time::sleep(Duration::from_millis(100)).await,
                    }
                }
            }
            Step::PressKey(k) => {
                guard_key(d, k, confirmed, &crate::policy::risky_words(), &crate::policy::block_rules()).await?;
                d.press_key(k).await?
            }
            Step::SelectOption { from, option } => retry(action_timeout(), || d.select_option(from, option)).await?,
            Step::UploadFile { into, files } => retry(action_timeout(), || d.upload(into, files)).await?,
            Step::LaunchApp { app, launch } => {
                let mut l = crate::Launch::default();
                for (k, v) in &launch.arguments {
                    let v = match v {
                        crate::LaunchArg::Str(t) => crate::LaunchArg::Str(subst(t)?),
                        crate::LaunchArg::List(items) => crate::LaunchArg::List(items.iter().map(|t| subst(t)).collect::<Result<_>>()?),
                        other => other.clone(),
                    };
                    l.arguments.push((k.clone(), v));
                }
                for (k, v) in &launch.env {
                    if v == crate::dsl::MASKED {
                        return Err(Error::Flow(format!(
                            "launchApp env `{k}` was hidden when this flow was recorded; give its value again (secrets as ${{NAME}} from FASTPILOT_VARS)"
                        )));
                    }
                    l.env.push((k.clone(), subst(v)?));
                }
                d.launch_app_with(&subst(app)?, &l).await?
            }
            Step::Swipe { from, to } => d.swipe(*from, *to).await?,
            Step::ShipCheck { links, cookies } => {
                let r = crate::ship::run(d, *links, *cookies).await?;
                return Ok(Some(r.text()));
            }
            Step::EvalScript(js) => {
                if !crate::policy::eval_unlocked() {
                    guard_script(&crate::policy::block_rules())?;
                }
                let v = d.eval_script(&subst(js)?).await?;
                return Ok(Some(clip(v.to_string())));
            }
            Step::UseSession(_) | Step::StartSession { .. } | Step::Parallel(_) => {
                return Err(Error::Flow(format!("`{}` is not allowed here", step.label())));
            }
            Step::LongPressOn(sel) => {
                guard_risky(d, sel, confirmed).await?;
                let (sel, hit) = point_on_screen(d, sel).await?;
                retry(action_timeout(), || d.long_press(&sel)).await?;
                if hit.is_some() {
                    return Ok(hit);
                }
            }
            Step::DoubleTapOn(sel) => {
                guard_risky(d, sel, confirmed).await?;
                let (sel, hit) = point_on_screen(d, sel).await?;
                retry(action_timeout(), || d.double_tap(&sel)).await?;
                if hit.is_some() {
                    return Ok(hit);
                }
            }
            Step::ScrollUntilVisible { selector, direction, timeout } => {
                let timeout = timeout.unwrap_or(SCROLL_TIMEOUT);
                let deadline = Instant::now() + timeout;
                let mut scrolls = 0;
                loop {
                    let snap = d.snapshot().await?;
                    if let Ok(e) = snap.resolve(selector, Intent::Find)
                        && in_view(e.node.bounds, d.viewport().await)
                    {
                        return Ok(Some(format!("{scrolls} scrolls")));
                    }
                    if Instant::now() >= deadline {
                        return Err(Error::NotFound(format!(
                            "{selector} not visible after {scrolls} scrolls ({}ms)",
                            timeout.as_millis()
                        )));
                    }
                    d.scroll(*direction, None).await?;
                    settle(d).await;
                    scrolls += 1;
                }
            }
            Step::HideKeyboard => d.hide_keyboard().await?,
            Step::EraseText(n) => d.erase_text(*n).await?,
            Step::StopApp(app) => d.stop_app(app.as_deref().map(subst).transpose()?.as_deref()).await?,
            Step::ClearState(app) => d.clear_state(app.as_deref().map(subst).transpose()?.as_deref()).await?,
            Step::ClearKeychain => d.clear_keychain().await?,
            Step::InstallApp(path) => d.install_app(&subst(path)?).await?,
            Step::UninstallApp(app) => d.uninstall_app(&subst(app)?).await?,
            Step::CopyTextFrom(sel) => {
                retry(action_timeout(), || async {
                    let snap = d.snapshot().await?;
                    let n = &snap.resolve(sel, Intent::Find)?.node;
                    if n.secure {
                        return Err(Error::NotActionable(format!("{sel} is a secure field; its text cannot be copied")));
                    }
                    let text = n.value.clone().filter(|v| !v.is_empty()).unwrap_or_else(|| n.name.clone());
                    *guard(&ctx.clipboard) = Some(text);
                    Ok(())
                })
                .await?;
                return Ok(guard(&ctx.clipboard).clone().map(clip));
            }
            Step::PasteText => {
                let text = guard(&ctx.clipboard)
                    .clone()
                    .ok_or_else(|| Error::Flow("nothing copied yet; use copyTextFrom first".into()))?;
                retry(input_timeout(d), || d.type_text(&Selector::Focused, &text)).await?
            }
            Step::WaitForAnimationToEnd(t) => {
                if let Err(e) = d.wait_idle(t.unwrap_or(IDLE_TIMEOUT)).await {
                    tracing::debug!(error = %e, "still animating, continuing");
                }
                return Ok(None);
            }
            Step::TakeScreenshot(name) => {
                let jpeg = d.screenshot().await?;
                return save_screenshot(name, &jpeg).map(Some);
            }
            Step::AssertScreenshot { name, threshold, max_diff, ignore, update } => {
                settle(d).await;
                let shot = d.screenshot().await?;
                let masks = if ignore.is_empty() { Vec::new() } else { masks_for(d, ignore, &shot).await? };
                let (t, m) = (threshold.unwrap_or(crate::visual::THRESHOLD), max_diff.unwrap_or(crate::visual::MAX_DIFF));
                return match crate::visual::compare(name, &shot, &masks, t, m, *update).map_err(Error::Flow)? {
                    crate::visual::Outcome::Stored(p) => Ok(Some(format!("baseline stored: {}", p.display()))),
                    crate::visual::Outcome::Same(f) => Ok(Some(format!("matches baseline ({:.2}% of pixels changed)", f * 100.0))),
                    crate::visual::Outcome::Changed { fraction, limit, diff } => Err(Error::Assertion(format!(
                        "screenshot differs from baseline {name}: {:.2}% of pixels changed (limit {:.2}%); diff image {} \
                         (red = changed); if the change is intended, rerun with update: true",
                        fraction * 100.0,
                        limit * 100.0,
                        diff.display()
                    ))),
                };
            }
            Step::SaveState(name) => {
                let state = d.save_state().await?;
                let (path, encrypted) = crate::state::write(name, "web", &state).map_err(Error::Flow)?;
                let how = if encrypted { "encrypted" } else { "not encrypted: set FASTPILOT_STATE_KEY to encrypt" };
                return Ok(Some(format!("{} ({how})", path.display())));
            }
            Step::LoadState(name) => {
                let doc = crate::state::read(name).map_err(Error::Flow)?;
                d.load_state(&doc["state"]).await?;
                settle(d).await;
                let hours = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|n| n.as_secs().saturating_sub(doc["saved_at"].as_u64().unwrap_or(0)) / 3600)
                    .unwrap_or(0);
                return Ok(Some(format!("{name} (saved {hours} h ago)")));
            }
            Step::InputRandom { kind, length } => {
                let value = random(*kind, *length);
                retry(input_timeout(d), || d.type_text(&Selector::Focused, &value)).await?;
                settle(d).await;
                return Ok(Some(value));
            }
            Step::RunFlow { when, env, steps, .. } => {
                if let Some(c) = when
                    && !holds(d, c).await?
                {
                    return Ok(Some(format!("skipped ({c} is false)")));
                }
                for (k, v) in env {
                    let v = subst(v)?;
                    guard(&ctx.vars).insert(k.clone(), v);
                }
                block(d, steps, ctx).await?;
                return Ok(None);
            }
            Step::Repeat { times, while_, steps } => {
                let mut n = 0;
                loop {
                    if times.is_some_and(|t| n >= t) {
                        break;
                    }
                    if let Some(c) = while_
                        && !holds(d, c).await?
                    {
                        break;
                    }
                    if Instant::now() >= ctx.deadline {
                        return Err(Error::Timeout(format!("flow time budget spent after {n} iterations")));
                    }
                    if times.is_none() && n >= MAX_ITERATIONS {
                        return Err(Error::Flow(format!("repeat while: still true after {MAX_ITERATIONS} iterations")));
                    }
                    block(d, steps, ctx).await.map_err(|e| e.context(&format!("iteration {}", n + 1)))?;
                    n += 1;
                }
                return Ok(Some(format!("{n} iterations")));
            }
            Step::HttpRequest(_) | Step::ReadOtp(_) => return sessionless(step, ctx, Some(d)).await,
            Step::StartRecording(name) => {
                check_name(name)?;
                d.start_recording().await?;
                *guard(&ctx.recording) = Some(name.clone());
                return Ok(None);
            }
            Step::SetLocation { latitude, longitude } => d.set_location(*latitude, *longitude).await?,
            Step::SetViewport { viewport, .. } => d.set_viewport(viewport.as_ref()).await?,
            Step::SetPermissions { app, permissions } => {
                d.set_permissions(app.as_deref().map(subst).transpose()?.as_deref(), permissions).await?
            }
            Step::SetNetwork(n) => d.set_network(n).await?,
            Step::SetLocale { locale, timezone } => d.set_locale(locale.as_deref(), timezone.as_deref()).await?,
            Step::PushNotification { app, title, body, payload } => {
                d.push_notification(app.as_deref().map(subst).transpose()?.as_deref(), &subst(title)?, &subst(body)?, payload.as_ref()).await?
            }
            Step::StopRecording => {
                let name = guard(&ctx.recording).take().unwrap_or_else(|| "recording".into());
                let rec = d.stop_recording().await?;
                let (ext, bytes) = rec.file(&name);
                return save_file("recordings", &format!("{name}.{ext}"), &bytes).map(Some);
            }
            Step::Retry { max, steps } => {
                let mut attempt = 0;
                loop {
                    match block(d, steps, ctx).await {
                        Ok(()) => return Ok(Some(format!("passed on attempt {}", attempt + 1))),
                        Err(e) if attempt < *max && Instant::now() < ctx.deadline => {
                            tracing::debug!(attempt, error = %e, "retry block failed, running it again");
                            attempt += 1;
                        }
                        Err(e) => return Err(e.context(&format!("attempt {}", attempt + 1))),
                    }
                }
            }
        }
        settle(d).await;
        Ok(None)
    })
}

/// Steps that talk to services instead of a screen (test data, one-time codes).
async fn sessionless(step: &Step, ctx: &Ctx, d: Option<&dyn Driver>) -> Result<Option<String>> {
    match step {
        Step::HttpRequest(h) => http_request(h, ctx).await,
        Step::ReadOtp(o) => read_otp(o, ctx, d).await,
        other => Err(Error::Flow(format!("`{}` needs a session", other.label()))),
    }
}

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| reqwest::Client::builder().timeout(Duration::from_secs(30)).build().unwrap_or_default())
}

fn subst_json(v: &serde_json::Value, ctx: &Ctx) -> Result<serde_json::Value> {
    use serde_json::Value as J;
    Ok(match v {
        J::String(s) => J::String(ctx.subst(s)?),
        J::Array(a) => J::Array(a.iter().map(|x| subst_json(x, ctx)).collect::<Result<_>>()?),
        J::Object(o) => J::Object(o.iter().map(|(k, x)| Ok((k.clone(), subst_json(x, ctx)?))).collect::<Result<_>>()?),
        other => other.clone(),
    })
}

/// `a.b.0.c` into a JSON value; strings come back unquoted.
fn json_path(v: &serde_json::Value, path: &str) -> Option<String> {
    let mut cur = v;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        cur = match part.parse::<usize>() {
            Ok(i) if cur.is_array() => cur.get(i)?,
            _ => cur.get(part)?,
        };
    }
    Some(match cur {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => return None,
        other => other.to_string(),
    })
}

/// Set a flow variable; secret-looking (or listed) ones are redacted from every output.
fn set_var(ctx: &Ctx, name: &str, value: String, secret: bool) {
    if secret || looks_secret(name) {
        add_secret(&value);
    }
    guard(&ctx.vars).insert(name.to_owned(), value);
}

/// S-1: would this step's text, its variables filled in, carry a password saved for the app?
/// (A variable that cannot be filled fails the step itself, with its own message.)
fn holds_app_secret(ctx: &Ctx, text: &str) -> bool {
    // As written (the value itself, L2), then with its variables filled in.
    crate::dsl::holds_app_secret(text) || (!crate::dsl::var_names(text).is_empty() && ctx.subst(text).is_ok_and(|t| crate::dsl::holds_app_secret(&t)))
}

async fn http_request(h: &HttpRequest, ctx: &Ctx) -> Result<Option<String>> {
    let url = ctx.subst(&h.url)?;
    // S-1 (E2): a password goes only to its app's own site.
    if holds_app_secret(ctx, &format!("{h:?}")) {
        crate::secrets::allowed_at(Some(&url), Platform::Web).map_err(Error::Flow)?;
    }
    let method = reqwest::Method::from_bytes(h.method.as_bytes()).map_err(|e| Error::Flow(e.to_string()))?;
    let mut req = http().request(method, &url);
    for (k, v) in &h.headers {
        req = req.header(k.as_str(), ctx.subst(v)?);
    }
    if let Some(j) = &h.json {
        req = req.json(&subst_json(j, ctx)?);
    }
    let what = format!("{} {}", h.method, h.url);
    let resp = req.send().await.map_err(|e| Error::Driver(format!("{what}: {e}")))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| Error::Driver(format!("{what}: {e}")))?;
    let expected = h.expect.map_or((200..300).contains(&status), |e| e == status);
    if !expected {
        let want = h.expect.map_or("2xx".to_owned(), |e| e.to_string());
        let msg = format!("{what} returned {status}, expected {want}: {}", clip(body));
        return Err(if status >= 500 { Error::Driver(msg) } else { Error::Assertion(msg) });
    }
    if h.capture.is_empty() {
        return Ok(Some(status.to_string()));
    }
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| Error::Assertion(format!("{what}: response is not JSON ({e}), cannot capture")))?;
    for (name, path) in &h.capture {
        let value = json_path(&json, path).ok_or_else(|| Error::Assertion(format!("{what}: response has no `{path}` for {name}")))?;
        set_var(ctx, name, value, h.secret.contains(name));
    }
    let names: Vec<&str> = h.capture.iter().map(|(n, _)| n.as_str()).collect();
    Ok(Some(format!("{status}; captured {}", names.join(", "))))
}

/// First one-time code in `text` (after `after`, if given): a digit run of exactly `length`,
/// else of 4 to 8 digits.
fn otp_in(text: &str, after: Option<&str>, length: Option<usize>) -> Option<String> {
    // ASCII lowering keeps byte offsets valid for slicing the original.
    let start = match after {
        Some(a) => text.to_ascii_lowercase().find(&a.to_ascii_lowercase())? + a.len(),
        None => 0,
    };
    let rest = &text[start..];
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let j = (i..bytes.len()).find(|&k| !bytes[k].is_ascii_digit()).unwrap_or(bytes.len());
            let ok = match length {
                Some(n) => j - i == n,
                None => (4..=8).contains(&(j - i)),
            };
            if ok {
                return Some(rest[i..j].to_owned());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

async fn get_json(url: &str) -> Result<serde_json::Value> {
    let resp = http().get(url).send().await.map_err(|e| Error::Driver(format!("GET {url}: {e}")))?;
    resp.json().await.map_err(|e| Error::Driver(format!("GET {url}: {e}")))
}

/// Candidate texts, newest first.
async fn otp_texts(o: &ReadOtp, ctx: &Ctx, d: Option<&dyn Driver>) -> Result<Vec<String>> {
    let base = o.url.as_deref().map(|u| ctx.subst(u)).transpose()?.unwrap_or_default();
    let base = base.trim_end_matches('/');
    let to = o.to.as_deref().map(|t| ctx.subst(t)).transpose()?.map(|t| t.to_ascii_lowercase());
    let addressed = |addrs: Vec<String>| to.as_ref().is_none_or(|t| addrs.iter().any(|a| a.to_ascii_lowercase().contains(t.as_str())));
    Ok(match o.source {
        // https://mailpit.axllent.org/docs/api-v1/ — newest first.
        OtpSource::Mailpit => {
            let list = get_json(&format!("{base}/api/v1/messages?limit=20")).await?;
            let mut out = Vec::new();
            for m in list["messages"].as_array().into_iter().flatten() {
                let addrs = m["To"].as_array().into_iter().flatten().filter_map(|a| a["Address"].as_str().map(str::to_owned)).collect();
                if !addressed(addrs) {
                    continue;
                }
                let Some(id) = m["ID"].as_str() else { continue };
                let full = get_json(&format!("{base}/api/v1/message/{id}")).await?;
                let subject = m["Subject"].as_str().unwrap_or("");
                out.push(format!("{subject}
{}", full["Text"].as_str().or(full["HTML"].as_str()).unwrap_or("")));
                break;
            }
            out
        }
        // MailHog API v2 — newest first; bodies may be quoted-printable (soft breaks removed).
        OtpSource::Mailhog => {
            let list = get_json(&format!("{base}/api/v2/messages?limit=20")).await?;
            list["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|m| {
                    let to = m["Content"]["Headers"]["To"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_owned));
                    addressed(to.collect())
                })
                .map(|m| {
                    let subject = m["Content"]["Headers"]["Subject"][0].as_str().unwrap_or("");
                    format!("{subject}
{}", m["Content"]["Body"].as_str().unwrap_or("").replace("=
", "").replace("=
", ""))
                })
                .take(1)
                .collect()
        }
        OtpSource::Http => {
            let resp = http().get(base).send().await.map_err(|e| Error::Driver(format!("GET {base}: {e}")))?;
            vec![resp.text().await.map_err(|e| Error::Driver(format!("GET {base}: {e}")))?]
        }
        OtpSource::Sms => {
            let d = d.ok_or_else(|| Error::Flow("readOtp from sms needs an android session".into()))?;
            d.read_sms().await?.into_iter().take(1).collect()
        }
    })
}

/// Poll the source until a code shows up; it becomes `${into}` (always redacted).
async fn read_otp(o: &ReadOtp, ctx: &Ctx, d: Option<&dyn Driver>) -> Result<Option<String>> {
    let timeout = o.timeout.unwrap_or(Duration::from_secs(30));
    let deadline = Instant::now() + timeout;
    loop {
        for text in otp_texts(o, ctx, d).await? {
            if let Some(code) = otp_in(&text, o.after.as_deref(), o.length) {
                set_var(ctx, &o.into, code, true);
                return Ok(Some(format!("code from {} in ${{{}}}", o.source.name(), o.into)));
            }
        }
        if Instant::now() >= deadline {
            return Err(Error::NotFound(format!("no one-time code from {} within {}ms", o.source.name(), timeout.as_millis())));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Steps of a block, in order; a failure names the step inside the block.
async fn block(d: &dyn Driver, steps: &[FlowStep], ctx: &Ctx) -> Result<()> {
    for (i, s) in steps.iter().enumerate() {
        if Instant::now() >= ctx.deadline {
            return Err(Error::Timeout("flow exceeded its time budget (FASTPILOT_FLOW_TIMEOUT)".into()));
        }
        exec(d, &s.step, ctx).await.map_err(|e| e.context(&format!("step {} {}", i + 1, s.step.label())))?;
    }
    Ok(())
}

/// `when` / `while`, checked once on the current screen.
async fn holds(d: &dyn Driver, c: &Condition) -> Result<bool> {
    let found = |sel: &Selector, snap: &Snapshot| match snap.resolve(sel, Intent::Find) {
        Ok(_) | Err(Error::Ambiguous { .. }) => Ok(true),
        Err(Error::NotFound(_)) => Ok(false),
        Err(e) => Err(e),
    };
    Ok(match c {
        Condition::Visible(sel) => found(sel, &d.snapshot().await?)?,
        Condition::NotVisible(sel) => !found(sel, &d.snapshot().await?)?,
        Condition::Platform(p) => d.platform() == *p,
    })
}

/// A node counts as visible when its center is in the viewport (web snapshots include
/// off-screen nodes; mobile ones do not, so no viewport means visible).
fn in_view(node: Option<Rect>, viewport: Option<Rect>) -> bool {
    match (node, viewport) {
        (Some(b), Some(v)) => v.contains(b.center()),
        _ => true,
    }
}

fn clip(mut s: String) -> String {
    if s.len() > 200 {
        let cut = (0..=200).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
        s.truncate(cut);
        s.push('…');
    }
    s
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) || name.contains("..") {
        return Err(Error::Flow(format!("name `{name}`: use letters, digits, '-', '_' and '.'")));
    }
    Ok(())
}

/// `.fastpilot/<dir>/<file>` (or under `FASTPILOT_DIR`); returns the path.
fn save_file(dir: &str, file: &str, bytes: &[u8]) -> Result<String> {
    check_name(file)?;
    let dir = crate::config::var_os("FASTPILOT_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| ".fastpilot".into()).join(dir);
    let path = dir.join(file);
    std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&path, bytes))
        .map_err(|e| Error::Flow(format!("cannot write {}: {e}", path.display())))?;
    crate::config::keep_out_of_git();
    Ok(path.display().to_string())
}

/// `takeScreenshot: name` → `.fastpilot/screenshots/<name>.jpg` (or `FASTPILOT_DIR`).
fn save_screenshot(name: &str, jpeg: &[u8]) -> Result<String> {
    let stem = name.trim_end_matches(".jpg").trim_end_matches(".png");
    check_name(stem)?;
    save_file("screenshots", &format!("{stem}.jpg"), jpeg)
}

/// Test data for `inputRandom*` (xorshift; not for secrets).
fn random(kind: Random, length: usize) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let mut x = STATE.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed) ^ seed | 1;
    let mut next = move |n: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % n
    };
    let letters = |next: &mut dyn FnMut(u64) -> u64, n: usize| (0..n).map(|_| (b'a' + next(26) as u8) as char).collect::<String>();
    match kind {
        Random::Text => letters(&mut next, length),
        Random::Number => (0..length).map(|i| char::from(b'0' + if i == 0 { 1 + next(9) } else { next(10) } as u8)).collect(),
        Random::Email => format!("{}@example.com", letters(&mut next, length)),
        Random::PersonName => {
            const FIRST: &[&str] = &["Budi", "Siti", "Agus", "Dewi", "Rina", "Andi", "Maya", "Joko"];
            const LAST: &[&str] = &["Santoso", "Wijaya", "Lestari", "Pratama", "Saputra", "Hidayat"];
            format!("{} {}", FIRST[next(FIRST.len() as u64) as usize], LAST[next(LAST.len() as u64) as usize])
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssertKind {
    Visible,
    NotVisible,
    TextEquals(String),
    Contains(String),
    Enabled,
}

/// Property assertion with auto-retry until `timeout` (PRD §10 `assert`). Deterministic:
/// evaluated on the snapshot, never by a model.
pub async fn assert(d: &dyn Driver, sel: &Selector, kind: &AssertKind, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let snap = d.snapshot().await?;
        let res = match (kind, snap.resolve(sel, Intent::Find)) {
            (AssertKind::NotVisible, Err(Error::NotFound(_))) => Ok(()),
            (AssertKind::NotVisible, _) => Err(Error::Assertion(format!("{sel} is still visible"))),
            (_, Err(e)) => Err(e),
            (AssertKind::Visible, Ok(_)) => Ok(()),
            (AssertKind::Enabled, Ok(e)) if e.node.disabled => Err(Error::Assertion(format!("{sel} is disabled"))),
            (AssertKind::Enabled, Ok(_)) => Ok(()),
            (AssertKind::TextEquals(want) | AssertKind::Contains(want), Ok(e)) => {
                let actual = match &e.node.value {
                    Some(v) if !e.node.secure && !v.is_empty() => v.as_str(),
                    _ => e.node.name.as_str(),
                };
                let pass = match kind {
                    AssertKind::TextEquals(_) => actual == want,
                    _ => actual.contains(want.as_str()),
                };
                if pass {
                    Ok(())
                } else {
                    let shown = if e.node.secure { "(secure)" } else { actual };
                    Err(Error::Assertion(format!("{sel} text is {shown:?}, expected {want:?}")))
                }
            }
        };
        match res {
            Ok(()) => return Ok(()),
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// Post-action auto-wait. Not fatal: a page that never goes idle (polling) still gets asserted on.
async fn settle(d: &dyn Driver) {
    if let Err(e) = d.wait_idle(IDLE_TIMEOUT).await {
        tracing::debug!(error = %e, "not idle, continuing");
    }
}

/// Retry lookup/actionability errors with 50→400 ms backoff until `timeout` (PRD §11.2).
async fn retry<F, Fut>(timeout: Duration, mut attempt: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let deadline = Instant::now() + timeout;
    let mut delay = Duration::from_millis(50);
    loop {
        let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(100));
        let res = tokio::time::timeout(left, attempt())
            .await
            .unwrap_or_else(|_| Err(Error::Timeout(format!("no response within {}ms", timeout.as_millis()))));
        match res {
            Err(e) if e.is_retryable() && Instant::now() + delay < deadline => {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_millis(400));
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_plain_names() {
        for ok in ["web1", "and1", "shop-1_a", "A", &"x".repeat(64)] {
            assert!(check_session_id(ok).is_ok(), "{ok}");
        }
        for bad in ["", "../x", "a/b", "..", ".", "a b", "x\ny", "é", &"x".repeat(65)] {
            assert!(check_session_id(bad).is_err_and(|e| e.to_string().contains("invalid session id")), "{bad:?}");
        }
    }
    use crate::{Direction, Node, Point, RefRegistry, Value, parse_flow};

    /// Login page that switches to a dashboard once "Masuk" is tapped with a filled email.
    #[derive(Default)]
    struct Fake {
        state: Mutex<(String, bool)>,
        refs: Mutex<RefRegistry>,
    }

    impl Fake {
        fn state(&self) -> std::sync::MutexGuard<'_, (String, bool)> {
            self.state.lock().unwrap_or_else(|p| p.into_inner())
        }
    }

    fn node(role: &str, name: &str) -> Node {
        Node { role: role.into(), name: name.into(), ..Node::default() }
    }

    #[async_trait]
    impl Driver for Fake {
        async fn open(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn snapshot(&self) -> Result<Snapshot> {
            let (email, logged_in) = self.state().clone();
            let nodes = if logged_in {
                vec![node("heading", "Dashboard"), node("text", "Budi")]
            } else {
                let mut f = node("textfield", "Email");
                f.value = Some(email);
                vec![f, node("button", "Masuk"), node("button", "Daftar")]
            };
            let title = if logged_in { "Dashboard" } else { "Login" };
            let mut refs = self.refs.lock().unwrap_or_else(|p| p.into_inner());
            Ok(Snapshot::build(title, "fake://", nodes, &mut refs))
        }
        async fn tap(&self, sel: &Selector) -> Result<()> {
            let snap = self.snapshot().await?;
            let e = snap.resolve(sel, Intent::Tap)?;
            let mut st = self.state();
            if e.node.name == "Masuk" && !st.0.is_empty() {
                st.1 = true;
            }
            Ok(())
        }
        async fn type_text(&self, sel: &Selector, text: &str) -> Result<()> {
            self.snapshot().await?.resolve(sel, Intent::Type)?;
            self.state().0 = text.into();
            Ok(())
        }
        async fn scroll(&self, _: Direction, _: Option<&Selector>) -> Result<()> {
            Ok(())
        }
        async fn swipe(&self, _: Point, _: Point) -> Result<()> {
            Err(Error::Unsupported("swipe".into()))
        }
        async fn back(&self) -> Result<()> {
            Ok(())
        }
        async fn screenshot(&self) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        async fn wait_idle(&self, _: Duration) -> Result<()> {
            Ok(())
        }
        async fn eval_script(&self, _: &str) -> Result<Value> {
            Ok(Value::String("Login".into()))
        }
    }

    /// A page scrolled down 100 px (viewport: page y 100..400): a "Bayar sekarang" button with a
    /// nameless icon in it, a "Batal" button, and a "Konfirmasi Pembayaran" dialog holding a "Tutup"
    /// button. `fail` makes every snapshot fail.
    struct PointPage {
        fail: bool,
        refs: Mutex<RefRegistry>,
        /// Snapshots left before the pay button (`id: pay`) is on screen.
        late: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl Driver for PointPage {
        async fn open(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn snapshot(&self) -> Result<Snapshot> {
            if self.fail {
                return Err(Error::Driver("snapshot failed".into()));
            }
            let at = |n: Node, x: f64, y: f64, w: f64, h: f64| Node { bounds: Some(Rect { x, y, w, h }), ..n };
            let icon = at(node("image", ""), 12.0, 128.0, 16.0, 16.0);
            let pay = Node { children: vec![icon], id: Some("pay".into()), ..at(node("button", "Bayar sekarang"), 8.0, 123.0, 120.0, 30.0) };
            let late = self.late.fetch_update(std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst, |n| n.checked_sub(1)).is_ok();
            let pay = if late { at(node("group", ""), 8.0, 123.0, 120.0, 30.0) } else { pay };
            let close = at(node("button", "Tutup"), 20.0, 300.0, 80.0, 30.0);
            let dialog = Node { children: vec![close], ..at(node("dialog", "Konfirmasi Pembayaran"), 0.0, 250.0, 300.0, 120.0) };
            let page = Node { children: vec![pay, at(node("button", "Batal"), 200.0, 123.0, 80.0, 30.0), dialog], ..at(node("group", ""), 0.0, 0.0, 400.0, 1200.0) };
            let mut refs = self.refs.lock().unwrap_or_else(|p| p.into_inner());
            Ok(Snapshot::build("Checkout", "fake://", vec![page], &mut refs))
        }
        async fn viewport(&self) -> Option<Rect> {
            Some(Rect { x: 0.0, y: 100.0, w: 400.0, h: 300.0 })
        }
        async fn tap(&self, _: &Selector) -> Result<()> {
            Ok(())
        }
        async fn type_text(&self, _: &Selector, _: &str) -> Result<()> {
            Ok(())
        }
        async fn scroll(&self, _: Direction, _: Option<&Selector>) -> Result<()> {
            Ok(())
        }
        async fn swipe(&self, _: Point, _: Point) -> Result<()> {
            Ok(())
        }
        async fn back(&self) -> Result<()> {
            Ok(())
        }
        async fn screenshot(&self) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        async fn wait_idle(&self, _: Duration) -> Result<()> {
            Ok(())
        }
        async fn eval_script(&self, _: &str) -> Result<Value> {
            Ok(Value::Null)
        }
    }

    /// Security review (High) and P2 1b: `tapOn: { point }` obeys block_actions and confirm_actions,
    /// for every element covering the point, and fails closed when nothing is known there.
    #[tokio::test]
    async fn point_taps_obey_block_and_confirm_rules() {
        let page = PointPage { fail: false, refs: Mutex::default(), late: Default::default() };
        // Points are page coordinates, as `snapshot coords` shows them: (60, 133) is "Bayar sekarang".
        let pt = |x: f64, y: f64| Selector::Point(Point { x, y });
        let bayar = ["bayar".to_owned()];
        let e = guard_tap(&page, &pt(60.0, 133.0), true, &[], &bayar).await.expect_err("blocked by its text");
        assert!(matches!(&e, Error::Blocked(m) if m.contains("Bayar sekarang") && (m.contains("the built-in blocked list") || m.contains("your blocked words")) && !m.contains("block_actions")), "{e}");
        // The nameless icon inside the button is covered by the button.
        assert!(guard_tap(&page, &pt(15.0, 130.0), true, &[], &bayar).await.is_err(), "icon in the pay button");
        // confirm_actions: refused unless confirmed.
        assert!(guard_tap(&page, &pt(60.0, 133.0), false, &bayar, &[]).await.is_err());
        assert!(guard_tap(&page, &pt(60.0, 133.0), true, &bayar, &[]).await.is_ok());
        // Another button: allowed.
        assert!(guard_tap(&page, &pt(220.0, 133.0), false, &bayar, &bayar).await.is_ok(), "Batal");
        // Every element covering the point counts, not only the deepest named one: "Tutup" sits in
        // the "Konfirmasi Pembayaran" dialog, so a point on it is blocked too (fail closed).
        let e = guard_tap(&page, &pt(40.0, 310.0), true, &[], &bayar).await.expect_err("inside the pay dialog");
        assert!(matches!(&e, Error::Blocked(m) if m.contains("Konfirmasi Pembayaran")), "{e}");
        assert!(guard_tap(&page, &pt(40.0, 310.0), true, &[], &[]).await.is_ok(), "no rules");
        // Nothing known at the point, or no snapshot, while a rule is on: refused (fail closed).
        let e = guard_tap(&page, &pt(390.0, 1190.0), false, &[], &bayar).await.expect_err("outside every element");
        assert!(matches!(&e, Error::Blocked(m) if m.contains("cannot tell")), "{e}");
        let broken = PointPage { fail: true, refs: Mutex::default(), late: Default::default() };
        assert!(guard_tap(&broken, &pt(60.0, 133.0), false, &[], &bayar).await.is_err(), "no snapshot");
        // No rule: nothing to check.
        assert!(guard_tap(&broken, &pt(60.0, 133.0), false, &[], &[]).await.is_ok());
    }

    /// v0.3.1 (Design/Bug Fixer Low): a tap by ref, id or nameless role was let through when the
    /// snapshot failed, and was checked only once: `{ id: pay }` before the button was on screen
    /// passed, then the tap's retry hit "Bayar sekarang". Now the guard waits for the element like
    /// the tap does, and refuses when it cannot see it at all.
    #[tokio::test(start_paused = true)]
    async fn taps_by_id_or_ref_obey_rules_when_late_or_unseen() {
        let bayar = ["bayar".to_owned()];
        let pay = Selector::Id("pay".into());
        let page = PointPage { fail: false, refs: Mutex::default(), late: 3.into() };
        let e = guard_tap(&page, &pay, true, &[], &bayar).await.expect_err("late pay button");
        assert!(matches!(&e, Error::Blocked(m) if m.contains("Bayar sekarang") && (m.contains("the built-in blocked list") || m.contains("your blocked words")) && !m.contains("block_actions")), "{e}");
        let page = PointPage { fail: false, refs: Mutex::default(), late: 3.into() };
        assert!(guard_tap(&page, &pay, false, &bayar, &[]).await.is_err(), "confirm_actions, late button");
        assert!(guard_tap(&page, &Selector::Id("nope".into()), false, &[], &bayar).await.is_err(), "never on screen");
        let broken = PointPage { fail: true, refs: Mutex::default(), late: Default::default() };
        for sel in [Selector::Ref("e2".into()), pay.clone(), Selector::Role { role: "button".into(), name: None }] {
            let e = guard_tap(&broken, &sel, false, &[], &bayar).await.expect_err("no snapshot");
            assert!(matches!(&e, Error::Blocked(m) if m.contains("cannot tell")), "{sel}: {e}");
        }
        // A blocked text that never comes on screen is refused at once, by its own text (main after
        // #58 waited for it and said "not found": ui_live_keyboard, `tapOn: Keluar`).
        let page = PointPage { fail: false, refs: Mutex::default(), late: Default::default() };
        let t0 = tokio::time::Instant::now();
        let e = guard_tap(&page, &Selector::Text("Keluar".into()), false, &[], &["keluar".to_owned()]).await.expect_err("blocked text");
        assert!(matches!(&e, Error::Blocked(m) if m.contains("Keluar") && m.contains("your blocked words") && !m.contains("block_actions")), "{e}");
        assert!(t0.elapsed() < Duration::from_millis(50), "refused without waiting");
        // A text selector without a snapshot is judged by its text, as before.
        assert!(guard_tap(&broken, &Selector::Text("Batal".into()), false, &[], &bayar).await.is_ok());
        assert!(guard_tap(&broken, &Selector::Text("Bayar".into()), false, &[], &bayar).await.is_err());
        // Another button, or no rules: allowed.
        let page = PointPage { fail: false, refs: Mutex::default(), late: Default::default() };
        assert!(guard_tap(&page, &Selector::Role { role: "button".into(), name: Some("Batal".into()) }, false, &bayar, &bayar).await.is_ok());
        assert!(guard_tap(&broken, &pay, false, &[], &[]).await.is_ok(), "no rules");
    }

    /// FastTest F1: after a scroll, `snapshot coords` gave page positions but `tapOn: { point }` was
    /// sent as a screen position, so the tap landed elsewhere and still reported ✓. A point is a page
    /// position now: sent to the driver as the screen position of that spot, and refused when it is
    /// not on screen.
    #[tokio::test]
    async fn point_taps_use_the_coordinates_snapshot_shows() {
        let page = PointPage { fail: false, refs: Mutex::default(), late: Default::default() };
        let pt = |x: f64, y: f64| Selector::Point(Point { x, y });
        // "Bayar sekarang" at page (60, 133) is on screen at (60, 33) (scrolled 100 px).
        let (on_screen, hit) = point_on_screen(&page, &pt(60.0, 133.0)).await.expect("on screen");
        assert_eq!(on_screen, pt(60.0, 33.0));
        assert!(hit.as_deref().is_some_and(|h| h.contains("button \"Bayar sekarang\"")), "{hit:?}");
        // Above or below the screen: an error, never a tap somewhere else.
        for off in [pt(60.0, 50.0), pt(60.0, 900.0)] {
            let e = point_on_screen(&page, &off).await.expect_err("off screen");
            assert!(matches!(&e, Error::NotActionable(m) if m.contains("not on screen")), "{e}");
        }
        // Other selectors pass through.
        let (same, hit) = point_on_screen(&page, &Selector::Text("Batal".into())).await.expect("text");
        assert!(same == Selector::Text("Batal".into()) && hit.is_none());
    }

    /// A screen of `nodes` (snapshot fails when `None`); records the keys pressed and scripts run.
    struct KeyPage {
        nodes: Option<Vec<Node>>,
        refs: Mutex<RefRegistry>,
        done: Mutex<Vec<String>>,
    }

    impl KeyPage {
        fn new(nodes: Option<Vec<Node>>) -> Self {
            Self { nodes, refs: Mutex::default(), done: Mutex::default() }
        }
    }

    #[async_trait]
    impl Driver for KeyPage {
        async fn open(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn snapshot(&self) -> Result<Snapshot> {
            let nodes = self.nodes.clone().ok_or_else(|| Error::Driver("snapshot failed".into()))?;
            let mut refs = self.refs.lock().unwrap_or_else(|p| p.into_inner());
            Ok(Snapshot::build("Form", "fake://", nodes, &mut refs))
        }
        async fn tap(&self, _: &Selector) -> Result<()> {
            Ok(())
        }
        async fn type_text(&self, _: &Selector, _: &str) -> Result<()> {
            Ok(())
        }
        async fn press_key(&self, key: &str) -> Result<()> {
            self.done.lock().unwrap_or_else(|p| p.into_inner()).push(format!("key {key}"));
            Ok(())
        }
        async fn scroll(&self, _: Direction, _: Option<&Selector>) -> Result<()> {
            Ok(())
        }
        async fn swipe(&self, _: Point, _: Point) -> Result<()> {
            Ok(())
        }
        async fn back(&self) -> Result<()> {
            Ok(())
        }
        async fn screenshot(&self) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        async fn wait_idle(&self, _: Duration) -> Result<()> {
            Ok(())
        }
        async fn eval_script(&self, js: &str) -> Result<Value> {
            self.done.lock().unwrap_or_else(|p| p.into_inner()).push(format!("eval {js}"));
            Ok(Value::Null)
        }
    }

    /// Security Low #5 / P2 1c: Enter in a form field submits it like a tap on its button, and a
    /// script can click anything: block_actions (and confirm_actions for keys) apply to both.
    #[tokio::test]
    async fn enter_and_scripts_obey_block_actions() {
        let bayar = ["bayar".to_owned()];
        let focused = |n: Node| Node { focused: true, ..n };
        let form = |field: Node| vec![field, node("button", "Bayar sekarang"), node("button", "Batal")];
        // Enter in a field of a form with a blocked button: refused, also with confirm.
        let page = KeyPage::new(Some(form(focused(node("textfield", "Nomor kartu")))));
        for confirmed in [false, true] {
            let e = guard_key(&page, "Enter", confirmed, &[], &bayar).await.expect_err("Enter may submit to Bayar");
            assert!(matches!(&e, Error::Blocked(m) if m.contains("Bayar sekarang") && (m.contains("the built-in blocked list") || m.contains("your blocked words")) && !m.contains("block_actions")), "{e}");
        }
        // Enter / Space on the blocked button itself.
        let on_button = KeyPage::new(Some(vec![focused(node("button", "Bayar sekarang"))]));
        assert!(guard_key(&on_button, "Enter", true, &[], &bayar).await.is_err());
        assert!(guard_key(&on_button, "space", true, &[], &bayar).await.is_err());
        // confirm_actions: lifted by confirm.
        assert!(guard_key(&page, "Enter", false, &bayar, &[]).await.is_err());
        assert!(guard_key(&page, "Enter", true, &bayar, &[]).await.is_ok());
        // Focus on an allowed button: Enter is that button.
        let on_batal = KeyPage::new(Some(form(focused(node("button", "Batal")))));
        assert!(guard_key(&on_batal, "Enter", false, &bayar, &bayar).await.is_ok());
        // No blocked button on screen, other keys, or no rules: allowed.
        let search = KeyPage::new(Some(vec![focused(node("textfield", "Cari")), node("button", "Cari")]));
        assert!(guard_key(&search, "Enter", false, &bayar, &bayar).await.is_ok());
        assert!(guard_key(&page, "Tab", false, &bayar, &bayar).await.is_ok());
        assert!(guard_key(&page, "Enter", false, &[], &[]).await.is_ok());
        // No snapshot while a rule is on: refused (fail closed).
        let blind = KeyPage::new(None);
        assert!(guard_key(&blind, "Enter", false, &[], &bayar).await.is_err());
        // evalScript: refused while block_actions is set, allowed otherwise.
        let e = guard_script(&bayar).expect_err("script while block_actions");
        assert!(matches!(&e, Error::Blocked(m) if m.contains("evalScript") && m.contains("buttons are blocked") && !m.contains("block_actions")), "{e}");
        assert!(guard_script(&[]).is_ok());
        assert!(page.done.lock().unwrap_or_else(|p| p.into_inner()).is_empty(), "guards never press or run anything");
    }

    /// Typing takes `typing`: a phone whose keyboard is slow to come up. Platform `platform`.
    struct SlowKeyboard {
        platform: Platform,
        typing: Duration,
    }

    #[async_trait]
    impl Driver for SlowKeyboard {
        fn platform(&self) -> Platform {
            self.platform
        }
        async fn open(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn snapshot(&self) -> Result<Snapshot> {
            Ok(Snapshot::build("Login", "fake://", vec![node("textfield", "Email")], &mut RefRegistry::default()))
        }
        async fn tap(&self, _: &Selector) -> Result<()> {
            Ok(())
        }
        async fn type_text(&self, _: &Selector, _: &str) -> Result<()> {
            tokio::time::sleep(self.typing).await;
            Ok(())
        }
        async fn scroll(&self, _: Direction, _: Option<&Selector>) -> Result<()> {
            Ok(())
        }
        async fn swipe(&self, _: Point, _: Point) -> Result<()> {
            Ok(())
        }
        async fn back(&self) -> Result<()> {
            Ok(())
        }
        async fn screenshot(&self) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        async fn wait_idle(&self, _: Duration) -> Result<()> {
            Ok(())
        }
        async fn eval_script(&self, _: &str) -> Result<Value> {
            Ok(Value::Null)
        }
    }

    /// FastTest (v0.2.0, Android API 35): the keyboard took 1.7–7.8 s to come up and every action
    /// had a fixed 5 s limit, so inputText timed out. The limit is now `action_timeout` (seconds),
    /// and inputText on a phone gets three times that for the keyboard.
    #[test]
    fn input_text_waits_for_a_slow_keyboard() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
        rt.block_on(slow_keyboard_cases());
    }

    async fn slow_keyboard_cases() {
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::remove_var("FASTPILOT_ACTION_TIMEOUT");
            std::env::remove_var("FASTPILOT_CONFIG");
        }
        let typing = Duration::from_millis(1500);
        let step = Step::InputText { into: Selector::Text("Email".into()), text: "a@b.c".into() };
        let android = SlowKeyboard { platform: Platform::Android, typing };
        let web = SlowKeyboard { platform: Platform::Web, typing };
        // The environment variable people set: FASTPILOT_ACTION_TIMEOUT.
        unsafe { std::env::set_var("FASTPILOT_ACTION_TIMEOUT", "1") };
        assert_eq!(action_timeout(), Duration::from_secs(1), "FASTPILOT_ACTION_TIMEOUT");
        // A phone: 1.5 s of keyboard fits in 3 × 1 s.
        assert!(type_with_timeout(&android, &step).await.is_ok(), "android keyboard");
        // A web page has no keyboard to wait for: the action limit applies as it is.
        let e = type_with_timeout(&web, &step).await.expect_err("web 1 s limit");
        assert!(matches!(e, Error::Timeout(_)), "{e}");
        unsafe { std::env::remove_var("FASTPILOT_ACTION_TIMEOUT") };
        // The setting in the config file (`fastpilot config set action_timeout 2`, the UI).
        let dir = std::env::temp_dir().join(format!("fastpilot-action-timeout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("config.toml");
        std::fs::write(&file, "action_timeout = 2\n").expect("write");
        unsafe { std::env::set_var("FASTPILOT_CONFIG", &file) };
        assert_eq!(action_timeout(), Duration::from_secs(2), "action_timeout from the file");
        assert!(type_with_timeout(&web, &step).await.is_ok(), "action_timeout 2 fits 1.5 s");
        unsafe { std::env::remove_var("FASTPILOT_CONFIG") };
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(action_timeout(), Duration::from_secs(5), "default");
    }

    /// One step through the real executor.
    async fn type_with_timeout(d: &dyn Driver, step: &Step) -> Result<()> {
        let ctx = Ctx {
            records: Mutex::new(Vec::new()),
            stop_on_fail: true,
            deadline: Instant::now() + Duration::from_secs(60),
            vars: Mutex::new(HashMap::new()),
            clipboard: Mutex::new(None),
            recording: Mutex::new(None),
            last_action: Mutex::new(HashMap::new()),
            live: None,
        };
        exec_with(d, step, &ctx, false).await.map(|_| ())
    }

    /// D07-L3: a stand-in Android phone with a Password field; `front` is the package in front.
    struct Phone {
        front: Mutex<Option<String>>,
        typed: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Driver for Phone {
        fn platform(&self) -> Platform {
            Platform::Android
        }
        async fn foreground_is(&self, app: &str) -> Option<bool> {
            self.front.lock().unwrap_or_else(|p| p.into_inner()).as_deref().map(|f| f == app)
        }
        async fn open(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn snapshot(&self) -> Result<Snapshot> {
            Ok(Snapshot::build("Login", "", vec![node("textfield", "Password")], &mut RefRegistry::default()))
        }
        async fn tap(&self, _: &Selector) -> Result<()> {
            Ok(())
        }
        async fn type_text(&self, _: &Selector, text: &str) -> Result<()> {
            self.typed.lock().unwrap_or_else(|p| p.into_inner()).push(text.to_owned());
            Ok(())
        }
        async fn scroll(&self, _: Direction, _: Option<&Selector>) -> Result<()> {
            Ok(())
        }
        async fn swipe(&self, _: Point, _: Point) -> Result<()> {
            Ok(())
        }
        async fn back(&self) -> Result<()> {
            Ok(())
        }
        async fn screenshot(&self) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        async fn wait_idle(&self, _: Duration) -> Result<()> {
            Ok(())
        }
        async fn eval_script(&self, _: &str) -> Result<Value> {
            Ok(Value::Null)
        }
    }

    /// D07-L3 (Security L3 on #190): on a phone, an app's saved password is typed only while that
    /// app is in front. A browser (or any other app) in front, or a phone that cannot tell, gets
    /// nothing; the app itself gets it. Before, "the app has an Android app" was enough.
    #[tokio::test]
    // The environment and the process-wide app scope stay as set for the whole test.
    #[allow(clippy::await_holding_lock)]
    async fn a_saved_password_goes_only_into_the_app_in_front() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let home = std::env::temp_dir().join(format!("fp-d07-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("home");
        let old_home = std::env::var_os("HOME");
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("FASTPILOT_KEYSTORE", "file");
            std::env::set_var("FASTPILOT_VARS", "D07_PASS");
            std::env::remove_var("D07_PASS");
        }
        crate::secrets::set("shop", "D07_PASS", "s3cret-d07").expect("save");
        crate::secrets::set_scope(Some(crate::secrets::Scope { app: "shop".into(), web: None, android: Some("com.shop.app".into()), ios: None }));
        let step = Step::InputText { into: Selector::Text("Password".into()), text: "${D07_PASS}".into() };
        let phone = Phone { front: Mutex::new(Some("com.android.chrome".into())), typed: Mutex::new(Vec::new()) };
        let browser = type_with_timeout(&phone, &step).await;
        assert!(browser.as_ref().is_err_and(|e| e.to_string().contains("only typed into the app itself (com.shop.app)")), "{browser:?}");
        *phone.front.lock().unwrap_or_else(|p| p.into_inner()) = None;
        assert!(type_with_timeout(&phone, &step).await.is_err_and(|e| e.to_string().contains("could not tell")));
        assert!(phone.typed.lock().unwrap_or_else(|p| p.into_inner()).is_empty(), "nothing typed into the browser");
        *phone.front.lock().unwrap_or_else(|p| p.into_inner()) = Some("com.shop.app".into());
        type_with_timeout(&phone, &step).await.expect("the app itself");
        assert_eq!(*phone.typed.lock().unwrap_or_else(|p| p.into_inner()), vec!["s3cret-d07".to_owned()]);
        crate::secrets::set_scope(None);
        // SAFETY: as above.
        unsafe {
            match old_home {
                Some(h) => std::env::set_var("HOME", h),
                None => std::env::remove_var("HOME"),
            }
            for k in ["FASTPILOT_KEYSTORE", "FASTPILOT_VARS"] {
                std::env::remove_var(k);
            }
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Records the order of calls; any platform.
    struct Recorder {
        platform: Platform,
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Driver for Recorder {
        fn platform(&self) -> Platform {
            self.platform
        }
        async fn open(&self, target: &str) -> Result<()> {
            self.calls.lock().unwrap_or_else(|p| p.into_inner()).push(format!("open {target}"));
            Ok(())
        }
        async fn reset_app(&self, app: &str) -> Result<String> {
            self.calls.lock().unwrap_or_else(|p| p.into_inner()).push(format!("reset {app}"));
            Ok("data and permissions cleared".into())
        }
        async fn snapshot(&self) -> Result<Snapshot> {
            Ok(Snapshot::build("App", "fake://", vec![node("text", "Halo")], &mut RefRegistry::default()))
        }
        async fn tap(&self, _: &Selector) -> Result<()> {
            Ok(())
        }
        async fn type_text(&self, _: &Selector, _: &str) -> Result<()> {
            Ok(())
        }
        async fn scroll(&self, _: Direction, _: Option<&Selector>) -> Result<()> {
            Ok(())
        }
        async fn swipe(&self, _: Point, _: Point) -> Result<()> {
            Ok(())
        }
        async fn back(&self) -> Result<()> {
            Ok(())
        }
        async fn screenshot(&self) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        async fn wait_idle(&self, _: Duration) -> Result<()> {
            Ok(())
        }
        async fn eval_script(&self, _: &str) -> Result<Value> {
            Ok(Value::Null)
        }
    }

    struct ResetLauncher {
        fresh: Option<Arc<crate::FreshApps>>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Launcher for ResetLauncher {
        async fn launch(&self, platform: Platform, _device: Option<&str>) -> Result<Box<dyn Driver>> {
            Ok(Box::new(Recorder { platform, calls: self.calls.clone() }))
        }
        fn fresh(&self) -> Option<Arc<crate::FreshApps>> {
            self.fresh.clone()
        }
    }

    /// E3: with `--reset`, a mobile session resets its app (data, permissions, orientation) before
    /// opening it, once per session start; web sessions and runs without `--reset` do not; wiping
    /// app data obeys allow_app_reset.
    #[test]
    fn reset_runs_before_a_mobile_app_opens() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
        rt.block_on(async {
            let flow = |yaml: &str| parse_flow(yaml).expect("flow");
            let go = |fresh: bool, yaml: &'static str| async move {
                let calls = Arc::new(Mutex::new(Vec::new()));
                let launcher = ResetLauncher { fresh: fresh.then(|| Arc::new(crate::FreshApps::default())), calls: calls.clone() };
                let report = run_flow(&mut SessionRegistry::default(), &launcher, &flow(yaml), &RunOptions::default()).await;
                let calls = calls.lock().unwrap_or_else(|p| p.into_inner()).clone();
                (report, calls)
            };
            // SAFETY: test-only env mutation, serialized by ENV_LOCK.
            unsafe { std::env::remove_var("FASTPILOT_ALLOW_APP_RESET") };
            let (report, calls) = go(true, "- session: { start: a, platform: android, target: com.x/.Main }\n").await;
            assert!(report.passed(), "{report}");
            assert_eq!(calls, ["reset com.x", "open com.x/.Main"]);
            assert!(report.to_string().contains("reset com.x: data and permissions cleared"), "{report}");
            // Without --reset, or a web session: nothing is reset.
            let (_, calls) = go(false, "- session: { start: a, platform: android, target: com.x }\n").await;
            assert_eq!(calls, ["open com.x"]);
            let (_, calls) = go(true, "- session: { start: w, platform: web, target: \"https://example.com\" }\n").await;
            assert_eq!(calls, ["open https://example.com"]);
            // allow_app_reset = false: the session does not start, nothing is wiped or opened.
            unsafe { std::env::set_var("FASTPILOT_ALLOW_APP_RESET", "false") };
            let (report, calls) = go(true, "- session: { start: a, platform: ios, target: com.x }\n").await;
            assert!(!report.passed() && report.to_string().contains("allow_app_reset"), "{report}");
            assert!(calls.is_empty(), "{calls:?}");
            unsafe { std::env::remove_var("FASTPILOT_ALLOW_APP_RESET") };
        });
    }

    /// FastTest F1: `--reset` resets each app once per run, whichever comes first: a session with
    /// the app as target or its first launchApp (the drivers claim from the same set). A second
    /// session on the same app in the same run is not reset again; another app is.
    #[test]
    fn reset_claims_each_app_once_per_run() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let f = crate::FreshApps::default();
        assert!(f.claim("com.x/.Main") && !f.claim("com.x") && !f.claim("com.x/.Other"), "one reset per app");
        assert!(f.claim("com.y"), "another app");
        assert!(!f.claim("https://example.com") && !f.claim(""), "not apps");
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
        rt.block_on(async {
            // SAFETY: test-only env mutation, serialized by ENV_LOCK.
            unsafe { std::env::remove_var("FASTPILOT_ALLOW_APP_RESET") };
            let calls = Arc::new(Mutex::new(Vec::new()));
            let launcher = ResetLauncher { fresh: Some(Arc::new(crate::FreshApps::default())), calls: calls.clone() };
            let mut reg = SessionRegistry::default();
            for yaml in ["- session: { start: a, platform: android, target: com.x }\n", "- session: { start: b, platform: android, target: com.x/.Main }\n"] {
                let report = run_flow(&mut reg, &launcher, &parse_flow(yaml).expect("flow"), &RunOptions::default()).await;
                assert!(report.passed(), "{report}");
            }
            // The launchApp path claims from the same set (here: com.x is already reset).
            assert!(!launcher.fresh.as_ref().expect("fresh").claim("com.x"));
            assert_eq!(*calls.lock().unwrap_or_else(|p| p.into_inner()), ["reset com.x", "open com.x", "open com.x/.Main"]);
        });
    }

    struct FakeLauncher;

    #[async_trait]
    impl Launcher for FakeLauncher {
        async fn launch(&self, platform: Platform, _device: Option<&str>) -> Result<Box<dyn Driver>> {
            match platform {
                Platform::Web => Ok(Box::new(Fake::default())),
                p => Err(Error::Unsupported(format!("{p}"))),
            }
        }
    }

    async fn run(reg: &mut SessionRegistry, yaml: &str) -> FlowReport {
        let flow = parse_flow(yaml).expect("valid flow");
        run_flow(reg, &FakeLauncher, &flow, &RunOptions::default()).await
    }

    /// The flow runs evalScript, which the built-in block_actions list turns off (F5): this test
    /// allows scripts for itself (`allow_eval_script = true`, option C), under ENV_LOCK.
    #[test]
    fn login_flow_passes_and_returns_state() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe { std::env::set_var("FASTPILOT_ALLOW_EVAL_SCRIPT", "true") };
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build().expect("runtime");
        rt.block_on(login_flow_cases());
        unsafe { std::env::remove_var("FASTPILOT_ALLOW_EVAL_SCRIPT") };
    }

    /// F5 (v0.3.2): with nothing set (a new install), "Bayar sekarang" is refused by the built-in
    /// list with the message the agent and the Live panel read; "Hapus filter" only needs
    /// `confirm: true`; emptied on purpose, both go through.
    #[test]
    fn built_in_limits_on_a_new_install() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("fastpilot-f5-{}", std::process::id()));
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("FASTPILOT_CONFIG", dir.join("none.toml"));
            std::env::remove_var("FASTPILOT_BLOCK_ACTIONS");
            std::env::remove_var("FASTPILOT_CONFIRM_ACTIONS");
        }
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build().expect("runtime");
        rt.block_on(async {
            let page = PointPage { fail: false, refs: Mutex::default(), late: Default::default() };
            let pay = Selector::Text("Bayar sekarang".into());
            let e = guard_risky(&page, &pay, true).await.expect_err("built-in block");
            assert_eq!(
                e.to_string(),
                "\"Bayar sekarang\" contains \"bayar\" (the built-in blocked list). Do not retry or work around it (`confirm: true` does not lift it). The user can change it in fastpilot ui › Settings › Limits & permissions."
            );
            assert_eq!(e.reason(), "blocked");
            assert_eq!(will_block(&Step::TapOn(pay.clone())).as_deref(), Some("will be blocked: the text contains \"bayar\" (the built-in blocked list)"));
            // A plain delete: confirm only.
            let e = guard_tap(&page, &Selector::Text("Hapus filter".into()), false, &crate::policy::risky_words(), &crate::policy::block_rules()).await.expect_err("confirm");
            assert!(matches!(&e, Error::Blocked(m) if m.contains("confirm_actions") && m.contains("confirm: true")), "{e}");
            let hapus = Selector::Text("Hapus filter".into());
            assert!(judge_tap(&hapus, &["Hapus filter".into()], true, &crate::policy::risky_words(), &crate::policy::block_rules()).is_ok(), "confirmed");
            let e = guard_script(&crate::policy::block_rules()).expect_err("evalScript off while the built-in list is on");
            assert!(e.to_string().contains("allow_eval_script true") && e.to_string().contains("Run any JavaScript"), "{e}");
            assert!(!crate::policy::eval_unlocked());
            // Security Low 3: an empty variable (an MCP entry an agent wrote) does not empty the list.
            // T1-T3 (Security Low 1): nor does a value with no item in it, like "," or " , ".
            for empty in ["", ",", " , ", ",,"] {
                unsafe { std::env::set_var("FASTPILOT_BLOCK_ACTIONS", empty) };
                assert_eq!(crate::policy::blocked_words().len(), crate::policy::DEFAULT_BLOCKED.len(), "{empty:?}");
            }
            unsafe { std::env::remove_var("FASTPILOT_BLOCK_ACTIONS") };
            // Option C: the user's `allow_eval_script = true` unlocks scripts.
            unsafe { std::env::set_var("FASTPILOT_ALLOW_EVAL_SCRIPT", "true") };
            assert!(crate::policy::eval_unlocked());
            unsafe { std::env::remove_var("FASTPILOT_ALLOW_EVAL_SCRIPT") };
        });
        // Emptying the file under a running process loosens: ignored until it restarts.
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("none.toml"), "block_actions = []\nconfirm_actions = []\n").expect("write");
        assert_eq!(crate::policy::blocked_words().len(), crate::policy::DEFAULT_BLOCKED.len(), "pinned at start");
        // Emptied on purpose, from the start (a restarted process): nothing is refused.
        std::fs::write(dir.join("empty.toml"), "block_actions = []\nconfirm_actions = []\n").expect("write");
        unsafe { std::env::set_var("FASTPILOT_CONFIG", dir.join("empty.toml")) };
        assert!(crate::policy::blocked_words().is_empty() && crate::policy::risky_words().is_empty());
        rt.block_on(async {
            let page = PointPage { fail: false, refs: Mutex::default(), late: Default::default() };
            assert!(guard_risky(&page, &Selector::Text("Bayar sekarang".into()), false).await.is_ok());
        });
        unsafe { std::env::remove_var("FASTPILOT_CONFIG") };
        let _ = std::fs::remove_dir_all(dir);
    }

    async fn login_flow_cases() {
        let mut reg = SessionRegistry::default();
        let report = run(
            &mut reg,
            "- session: { start: web1, target: login }\n- evalScript: document.title\n- inputText: { into: Email, text: a@b.com }\n- tapOn: Masuk\n- assertVisible: Budi\n",
        )
        .await;
        let out = report.to_string();
        assert!(report.passed(), "{out}");
        assert!(out.starts_with("ok 5/5 steps"), "{out}");
        assert!(out.contains("evalScript \"document.title\" 0ms = \"Login\""), "{out}");
        assert!(out.contains("[web1] screen \"Dashboard\" (fake://)\ne4 heading \"Dashboard\""), "{out}");
        assert_eq!(reg.active(), Some("web1"));
    }

    /// Its text has the action limit in it ("timeout 5000ms"): FASTPILOT_ACTION_TIMEOUT, which
    /// another test sets, must not change under it (ENV_LOCK).
    #[test]
    fn stops_early_with_reason_and_candidates() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build().expect("runtime");
        rt.block_on(stops_early_cases());
    }

    async fn stops_early_cases() {
        let mut reg = SessionRegistry::default();
        let report = run(&mut reg, "- session: { start: web1 }\n- tapOn: Login\n- tapOn: Masuk\n").await;
        let out = report.to_string();
        assert!(!report.passed());
        assert_eq!(report.records.len(), 2, "{out}");
        assert!(
            out.starts_with("fail step 2/3 [web1] tapOn \"Login\": not_found: no element matches \"Login\" (timeout 5000ms)\ncandidates: e1 textfield \"Email\" value=\"\", e2 button \"Masuk\", e3 button \"Daftar\""),
            "{out}"
        );
    }

    #[test]
    fn finds_one_time_codes_and_json_paths() {
        assert_eq!(otp_in("Kode OTP Anda: 482913. Berlaku 5 menit.", None, None).as_deref(), Some("482913"));
        assert_eq!(otp_in("Order 12 — code 2026 1234567890 ok 7788", Some("code"), Some(4)).as_deref(), Some("2026"));
        assert_eq!(otp_in("Tanggal 03/11/2026, kode: 5521", Some("kode"), None).as_deref(), Some("5521"));
        assert_eq!(otp_in("no digits", None, None), None);
        let j = serde_json::json!({ "data": { "token": "t0k", "items": [{ "id": 7 }] } });
        assert_eq!(json_path(&j, "data.token").as_deref(), Some("t0k"));
        assert_eq!(json_path(&j, "data.items.0.id").as_deref(), Some("7"));
        assert_eq!(json_path(&j, "data.nope"), None);
    }

    #[tokio::test(start_paused = true)]
    async fn blocks_conditions_and_clipboard() {
        let mut reg = SessionRegistry::default();
        let flow = r#"
- session: { start: web1 }
- runFlow:
    env: { WHO: budi@x.id }
    commands:
      - inputText: { into: Email, text: "${WHO}" }
- copyTextFrom: Email
- runFlow: { when: { visible: "Nope" }, commands: [ tapOn: Nope ] }
- runFlow: { when: { platform: web }, commands: [ tapOn: Daftar ] }
- repeat: { times: 3, commands: [ tapOn: Daftar ] }
- repeat: { while: { visible: "Masuk" }, commands: [ tapOn: Masuk ] }
- retry: { maxRetries: 1, commands: [ assertVisible: Budi ] }
"#;
        let report = run(&mut reg, flow).await;
        let out = report.to_string();
        assert!(report.passed(), "{out}");
        assert!(out.contains("copyTextFrom \"Email\" 0ms = budi@x.id"), "{out}");
        assert!(out.contains("= skipped (visible \"Nope\" is false)"), "{out}");
        assert!(out.contains("repeat 3x (1 steps) 0ms = 3 iterations"), "{out}");
        assert!(out.contains("repeat while visible \"Masuk\" (1 steps) 0ms = 1 iterations"), "{out}");

        let report = run(&mut reg, "- retry: { maxRetries: 2, commands: [ tapOn: Logout ] }\n").await;
        let out = report.to_string();
        assert!(out.contains("not_found: attempt 3: step 1 tapOn \"Logout\": no element matches"), "{out}");
    }

    /// Issue #7: a check that fails after a tap which changed nothing is `timing`, not `product`.
    #[tokio::test(start_paused = true)]
    async fn check_after_a_tap_without_effect_is_timing() {
        let mut reg = SessionRegistry::default();
        // "Masuk" with an empty email does nothing in the fake app.
        let report = run(&mut reg, "- session: { start: w }\n- tapOn: Masuk\n- assertVisible: { text: Budi, timeout: 300 }\n").await;
        let failed = report.records.iter().find(|r| r.error.is_some()).expect("failed step");
        assert!(failed.after_no_effect, "{report}");
        assert_eq!(failed.category(), Some("timing"));
        assert!(report.to_string().contains("category timing, not product"), "{report}");
        // The tap worked (the screen changed) and the check still fails: product.
        let report = run(&mut reg, "- inputText: { into: Email, text: a@b.com }\n- tapOn: Masuk\n- assertVisible: { text: Nope, timeout: 300 }\n").await;
        let failed = report.records.iter().find(|r| r.error.is_some()).expect("failed step");
        assert!(!failed.after_no_effect, "{report}");
        assert_eq!(failed.category(), Some("product"));
        // Another action came after the no-op tap: the tap is not the previous action.
        let mut reg = SessionRegistry::default();
        let report = run(&mut reg, "- session: { start: w }\n- tapOn: Daftar\n- inputText: { into: Email, text: x }\n- assertVisible: { text: Nope, timeout: 300 }\n").await;
        let failed = report.records.iter().find(|r| r.error.is_some()).expect("failed step");
        assert_eq!(failed.category(), Some("product"), "{report}");
    }

    /// FASTPILOT_CONFIRM_ACTIONS: risky taps need `confirm: true` (the env path is covered by
    /// tests/policy_web.rs; here the word list is passed, so parallel tests are unaffected).
    #[tokio::test(start_paused = true)]
    async fn risky_taps_need_confirmation() {
        let d = Fake::default();
        let words = vec!["daftar".to_owned()];
        let daftar = Selector::Text("Daftar".into());
        let e = guard_risky_with(&d, &daftar, false, &words).await.expect_err("blocked");
        assert_eq!(e.reason(), "blocked");
        assert_eq!(e.category(), "agent");
        assert!(e.to_string().contains("confirm: true"), "{e}");
        assert!(guard_risky_with(&d, &daftar, true, &words).await.is_ok());
        assert!(guard_risky_with(&d, &Selector::Text("Masuk".into()), false, &words).await.is_ok());
        assert!(guard_risky_with(&d, &daftar, false, &[]).await.is_ok());
        // `confirm: true` runs the step and says so; it survives in path memory.
        let mut reg = SessionRegistry::default();
        let report = run(&mut reg, "- session: { start: w }\n- tapOn: { text: Daftar, confirm: true }\n").await;
        assert!(report.passed() && report.to_string().contains("tapOn \"Daftar\" (confirmed)"), "{report}");
        let flow = crate::parse_flow("- tapOn: { ref: e3, confirm: true }\n- doubleTapOn: { text: X, confirm: true }\n").expect("flow");
        let yaml = crate::flow_to_yaml(&flow, &std::collections::HashMap::new());
        assert!(yaml.contains("confirm: true") && crate::parse_flow(&yaml).expect("again").steps.len() == 2, "{yaml}");
        assert!(crate::parse_flow("- tapOn: { text: X, confirm: maybe }\n").is_err());
        // v0.3.2: Enter that may press a confirm_actions button can be confirmed too (the refusal
        // says so); it round-trips as `{ key: Enter, confirm: true }`.
        let flow = crate::parse_flow("- pressKey: { key: Enter, confirm: true }\n- pressKey: Tab\n").expect("pressKey confirm");
        assert!(matches!(&flow.steps[0].step, Step::Confirmed(inner) if matches!(&**inner, Step::PressKey(k) if k == "Enter")));
        let yaml = crate::flow_to_yaml(&flow, &std::collections::HashMap::new());
        assert!(yaml.contains("key: Enter") && yaml.contains("confirm: true") && crate::parse_flow(&yaml).is_ok(), "{yaml}");
        assert!(crate::parse_flow("- pressKey: { confirm: true }\n").is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn parallel_branches_run_and_diff_per_session() {
        let mut reg = SessionRegistry::default();
        run(&mut reg, "- session: { start: a }\n- session: { start: b }\n").await;
        let report = run(
            &mut reg,
            "- parallel:\n  - session: a\n    steps:\n      - inputText: { into: Email, text: x@y.z }\n  - session: b\n    steps:\n      - inputText: { into: Email, text: q@r.s }\n      - tapOn: Masuk\n",
        )
        .await;
        let out = report.to_string();
        assert!(report.passed(), "{out}");
        assert!(out.contains("[a] screen \"Login\" (fake://)\n  ~ e1 textfield \"Email\" value=\"\" -> value=\"x@y.z\""), "{out}");
        assert!(out.contains("[b] screen \"Dashboard\""), "{out}");

        let report = run(
            &mut reg,
            "- parallel:\n  - session: a\n    steps:\n      - assertVisible: { text: Nope, timeout: 300 }\n  - session: b\n    steps:\n      - assertVisible: Budi\n",
        )
        .await;
        assert!(report.to_string().starts_with("fail step 1/2 [a] assertVisible \"Nope\": not_found"), "{report}");
    }

    /// E1: a web session refuses launch extras with a reason; a masked env value from a recorded
    /// flow is not passed to the app as `***`.
    #[tokio::test(start_paused = true)]
    async fn launch_app_extras_need_a_mobile_app_and_real_values() {
        let mut reg = SessionRegistry::default();
        let report = run(&mut reg, "- session: { start: web1, target: login }\n- launchApp: { appId: com.x, arguments: { debug: true } }\n").await;
        assert!(!report.passed() && report.to_string().contains("for Android and iOS apps"), "{report}");
        let report = run(&mut reg, "- session: web1\n- launchApp: { appId: com.x, env: { API_TOKEN: \"***\" } }\n").await;
        assert!(!report.passed() && report.to_string().contains("was hidden when this flow was recorded"), "{report}");
    }
}
