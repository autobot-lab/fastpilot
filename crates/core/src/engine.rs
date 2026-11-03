//! Flow engine: multi-session registry, auto-wait, lookup retry, stop-early, per-session diffs.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::Instant;

use crate::dsl::substitute;
use crate::snapshot::DEFAULT_MAX_LINES;
use crate::{Driver, Error, Flow, FlowStep, Intent, Platform, Result, Selector, SessionId, Snapshot, Step, diff};

const ACTION_TIMEOUT: Duration = Duration::from_secs(5);
const ASSERT_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(3);

/// Live sessions of one fastpilot process, across platforms (PRD §8.1).
#[derive(Default)]
pub struct SessionRegistry {
    sessions: HashMap<SessionId, Box<dyn Driver>>,
    active: Option<SessionId>,
}

impl SessionRegistry {
    pub fn insert(&mut self, id: SessionId, driver: Box<dyn Driver>) -> Result<()> {
        if self.sessions.contains_key(&id) {
            return Err(Error::Session(format!("session `{id}` already exists")));
        }
        self.sessions.insert(id.clone(), driver);
        self.active = Some(id);
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Option<Box<dyn Driver>> {
        if self.active.as_deref() == Some(id) {
            self.active = None;
        }
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
    async fn launch(&self, platform: Platform) -> Result<Box<dyn Driver>>;
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Initial active session; defaults to the registry's active session.
    pub session: Option<SessionId>,
    pub stop_on_fail: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            session: None,
            stop_on_fail: true,
        }
    }
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
    timeout: Option<Duration>,
    target: Option<(Selector, Intent)>,
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
}

impl Ctx {
    fn record(&self, r: StepRecord) {
        self.records.lock().unwrap_or_else(|p| p.into_inner()).push(r);
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
    };
    let mut baselines: Vec<(SessionId, Option<Snapshot>)> = Vec::new();
    let mut active = opts.session.clone().or_else(|| reg.active().map(str::to_owned));
    let mut index = 0;

    for fs in &flow.steps {
        let ok = match &fs.step {
            Step::StartSession { id, platform, target, reset } => {
                index += 1;
                let t0 = Instant::now();
                if *reset {
                    reg.remove(id);
                }
                let res = start_session(reg, launcher, id, *platform, target.as_deref()).await;
                if res.is_ok() {
                    active = Some(id.clone());
                    baselines.retain(|(s, _)| s != id);
                    baselines.push((id.clone(), None));
                }
                record(&ctx, index, Some(id), &fs.step, t0, res.map(|()| None))
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

    let mut records = ctx.records.into_inner().unwrap_or_else(|p| p.into_inner());
    records.sort_by_key(|r| r.index);
    let failed = records.iter().find(|r| r.error.is_some());
    let mut candidates = Vec::new();
    let mut states = Vec::new();
    for (sid, base) in &baselines {
        let Ok(d) = reg.get(sid) else { continue };
        let snap = match d.snapshot().await {
            Ok(s) => s,
            Err(e) => {
                states.push(format!("[{sid}] snapshot failed: {}: {e}", e.reason()));
                continue;
            }
        };
        let failed_here = failed.filter(|f| f.session.as_deref() == Some(sid.as_str()));
        if let Some((sel, intent)) = failed_here.and_then(|f| f.target.as_ref()) {
            candidates = snap.candidates(sel, *intent);
        }
        let lines = match (failed_here, base) {
            (None, Some(base)) => diff(base, &snap),
            _ => None,
        };
        states.push(match lines {
            Some(l) if l.is_empty() => format!("[{sid}] {}\n  (no changes)", snap.header()),
            Some(l) => format!("[{sid}] {}\n  {}", snap.header(), l.join("\n  ")),
            None => format!("[{sid}] {}", snap.render(None, DEFAULT_MAX_LINES)),
        });
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
) -> Result<()> {
    if reg.contains(id) {
        return Err(Error::Session(format!("session `{id}` already exists")));
    }
    let driver = launcher.launch(platform).await?;
    if let Some(t) = target {
        driver.open(&substitute(t).map_err(Error::Flow)?).await?;
        settle(&*driver).await;
    }
    reg.insert(id.to_string(), driver)
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
    let sel = match step {
        Step::TapOn(s) | Step::InputText { into: s, .. } | Step::UploadFile { into: s, .. } => s,
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
    let node = &snap.resolve(sel, Intent::Find).ok()?.node;
    if !node.name.is_empty() {
        Some(Selector::Role { role: node.role.clone(), name: Some(node.name.clone()) })
    } else {
        node.id.clone().map(Selector::Id)
    }
}

async fn run_one(d: &dyn Driver, sid: &str, step: &Step, index: usize, ctx: &Ctx) -> bool {
    let t0 = Instant::now();
    let stable = match step_ref(step) {
        Some(sel) => durable(d, sel).await,
        None => None,
    };
    let res = exec(d, step).await;
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
    let (timeout, target) = match step {
        Step::TapOn(s) => (Some(ACTION_TIMEOUT), Some((s.clone(), Intent::Tap))),
        Step::InputText { into, .. } | Step::UploadFile { into, .. } => {
            (Some(ACTION_TIMEOUT), Some((into.clone(), Intent::Type)))
        }
        Step::SelectOption { from, .. } => (Some(ACTION_TIMEOUT), Some((from.clone(), Intent::Tap))),
        Step::AssertVisible { selector, timeout } => {
            (Some(timeout.unwrap_or(ASSERT_TIMEOUT)), Some((selector.clone(), Intent::Find)))
        }
        _ => (None, None),
    };
    let (output, error) = match res {
        Ok(o) => (o, None),
        Err(e) => (None, Some(e)),
    };
    ctx.record(StepRecord {
        index,
        session: session.map(str::to_owned),
        label: step.label(),
        elapsed: t0.elapsed(),
        output,
        durable: None,
        // Retry exhaustion is the only case where the timeout is the relevant detail.
        timeout: timeout.filter(|_| error.as_ref().is_some_and(Error::is_retryable)),
        error,
        target,
    });
    ok
}

async fn exec(d: &dyn Driver, step: &Step) -> Result<Option<String>> {
    let subst = |s: &str| substitute(s).map_err(Error::Flow);
    match step {
        Step::OpenUrl(url) => d.open(&subst(url)?).await?,
        Step::TapOn(sel) => retry(ACTION_TIMEOUT, || d.tap(sel)).await?,
        Step::InputText { into, text } => {
            let text = subst(text)?;
            retry(ACTION_TIMEOUT, || d.type_text(into, &text)).await?
        }
        Step::AssertVisible { selector, timeout } => {
            retry(timeout.unwrap_or(ASSERT_TIMEOUT), || async {
                d.snapshot().await?.resolve(selector, Intent::Find).map(|_| ())
            })
            .await?;
            return Ok(None);
        }
        Step::Scroll { direction, within } => retry(ACTION_TIMEOUT, || d.scroll(*direction, within.as_ref())).await?,
        Step::Back => d.back().await?,
        Step::AssertNotVisible { selector, timeout } => {
            let timeout = timeout.unwrap_or(ACTION_TIMEOUT);
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
        Step::PressKey(k) => d.press_key(k).await?,
        Step::SelectOption { from, option } => retry(ACTION_TIMEOUT, || d.select_option(from, option)).await?,
        Step::UploadFile { into, files } => retry(ACTION_TIMEOUT, || d.upload(into, files)).await?,
        Step::LaunchApp(app) => d.launch_app(&subst(app)?).await?,
        Step::Swipe { from, to } => d.swipe(*from, *to).await?,
        Step::EvalScript(js) => {
            let v = d.eval_script(&subst(js)?).await?;
            let mut s = v.to_string();
            if s.len() > 200 {
                let cut = (0..=200).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
                s.truncate(cut);
                s.push('…');
            }
            return Ok(Some(s));
        }
        Step::UseSession(_) | Step::StartSession { .. } | Step::Parallel(_) => {
            return Err(Error::Flow(format!("`{}` is not allowed here", step.label())));
        }
    }
    settle(d).await;
    Ok(None)
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

    struct FakeLauncher;

    #[async_trait]
    impl Launcher for FakeLauncher {
        async fn launch(&self, platform: Platform) -> Result<Box<dyn Driver>> {
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

    #[tokio::test(start_paused = true)]
    async fn login_flow_passes_and_returns_state() {
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

    #[tokio::test(start_paused = true)]
    async fn stops_early_with_reason_and_candidates() {
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
}
