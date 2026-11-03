//! MCP server (stdio): `session`, `run_flow`, `check_flow`, `snapshot`, `screenshot`, `assert`,
//! `report`, `docs` (PRD §10).

mod docs;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use fastpilot_core::{
    AssertKind, Driver, Error, FlowReport, Launcher, Platform, RunOptions, Selector, SessionRegistry, assert, check_flow,
    flow_to_yaml, redact, run_flow, sessions_used,
};
use fastpilot_memory::{Assertion, Memory, Run};
use fastpilot_web::{Browser, LaunchOptions};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Starts drivers for `session start` and `session: { start: ... }` steps. One Chrome process
/// is launched lazily, shared by every web session, and relaunched if it died.
pub struct PlatformLauncher {
    web: LaunchOptions,
    browser: Mutex<Option<Arc<Browser>>>,
}

impl PlatformLauncher {
    pub fn new(web: LaunchOptions) -> Self {
        Self { web, browser: Mutex::new(None) }
    }

    async fn browser(&self) -> fastpilot_core::Result<Arc<Browser>> {
        let mut slot = self.browser.lock().await;
        match slot.as_ref() {
            Some(b) if !b.is_closed() => Ok(b.clone()),
            _ => {
                let b = Browser::launch(self.web.clone()).await?;
                *slot = Some(b.clone());
                Ok(b)
            }
        }
    }
}

#[async_trait]
impl Launcher for PlatformLauncher {
    async fn launch(&self, platform: Platform) -> fastpilot_core::Result<Box<dyn Driver>> {
        match platform {
            Platform::Web => Ok(Box::new(self.browser().await?.new_page().await?)),
            Platform::Android => {
                let serial = std::env::var("FASTPILOT_ANDROID_SERIAL").ok();
                Ok(Box::new(fastpilot_android::AndroidDriver::start(serial.as_deref()).await?))
            }
            Platform::Ios => {
                let udid = std::env::var("FASTPILOT_IOS_UDID").ok();
                Ok(Box::new(fastpilot_ios::IosDriver::start(udid.as_deref()).await?))
            }
        }
    }
}

struct State {
    registry: Mutex<SessionRegistry>,
    launcher: PlatformLauncher,
    memory: Memory,
    /// Open runs by test id, closed by `report`.
    runs: Mutex<HashMap<String, Run>>,
    /// One lock per session: a flow holds the sessions it touches, so tools on *other*
    /// sessions never wait for it (the registry lock itself is only held briefly).
    busy: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

#[derive(Clone)]
pub struct FastPilot {
    state: Arc<State>,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SessionArgs {
    /// start | stop | list | devices | start_device
    pub action: String,
    /// Session id. start: optional (defaults to web1, android1, ...); stop: defaults to the active session.
    pub id: Option<String>,
    /// web (default) | android | ios
    pub platform: Option<String>,
    /// start: URL / local path (web), package or package/activity (android), bundle id (ios).
    /// The response includes its first snapshot.
    pub target: Option<String>,
    /// web: launch (default, own Chrome) | attach (your running Chrome, see `endpoint`)
    pub mode: Option<String>,
    /// attach: `http://127.0.0.1:9222` or a `ws://` URL; omitted = Chrome M144+ autoConnect
    /// (enable chrome://inspect/#remote-debugging). android/ios: device serial / simulator UDID.
    pub endpoint: Option<String>,
    /// Test case id: the response includes its path-memory hint, if any.
    pub test_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunFlowArgs {
    /// Flow YAML: a list of steps. See `docs` (topic: dsl).
    pub flow: String,
    /// Initial active session (defaults to the last active one).
    pub session: Option<String>,
    /// Stop at the first failed step (default true).
    pub stop_on_fail: Option<bool>,
    /// Test case id: records this flow + evidence into the open run (closed by `report`).
    pub test_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckFlowArgs {
    /// Flow YAML to validate without running it.
    pub flow: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SnapshotArgs {
    pub session: Option<String>,
    /// Find: only nodes whose role or name contains this text (case-insensitive), with their ancestors.
    pub filter: Option<String>,
    /// Lift the 120-line cap.
    pub full: Option<bool>,
    /// Append ` @x,y wxh` boxes.
    pub coords: Option<bool>,
    /// Write the full snapshot to `.fastpilot/snapshots/` and return only its path and size.
    pub file: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotArgs {
    pub session: Option<String>,
    /// `som`: Set-of-Mark — numbered boxes per target (drawn on web) plus a ref -> coordinates list.
    pub annotate: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AssertArgs {
    pub session: Option<String>,
    /// visible | not_visible | text_equals | contains | enabled
    pub kind: String,
    /// DSL selector: `"Dashboard"`, `e7`, `{ role: button, name: Masuk }`, `{ id: total }`.
    pub selector: String,
    /// Expected text for text_equals / contains.
    pub text: Option<String>,
    /// Milliseconds to keep retrying (default 5000).
    pub timeout: Option<u64>,
    /// Record into this test's open run.
    pub test_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReportArgs {
    pub test_id: String,
    /// pass | fail
    pub status: String,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DocsArgs {
    /// dsl | selectors | parallel | examples | errors | memory (default: dsl)
    pub topic: Option<String>,
}

// Every text result passes `redact`, so allowed `${VAR}` values never reach the AI.
fn ok(text: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(redact(&text.into()))]))
}

fn fail(text: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![ContentBlock::text(redact(&text.into()))]))
}

fn err(e: &Error) -> Result<CallToolResult, ErrorData> {
    fail(format!("{}: {e}", e.reason()))
}

impl FastPilot {
    /// Lock sessions in a fixed (sorted) order, so two flows can never deadlock.
    async fn lock_sessions(&self, ids: impl IntoIterator<Item = String>) -> Vec<OwnedMutexGuard<()>> {
        let mut ids: Vec<String> = ids.into_iter().collect();
        ids.sort();
        ids.dedup();
        let locks: Vec<Arc<Mutex<()>>> = {
            let mut busy = self.state.busy.lock().await;
            ids.into_iter().map(|id| busy.entry(id).or_default().clone()).collect()
        };
        let mut guards = Vec::with_capacity(locks.len());
        for l in locks {
            guards.push(l.lock_owned().await);
        }
        guards
    }

    /// The session's driver plus its lock, without holding the registry lock.
    async fn session_driver(&self, id: Option<&str>) -> fastpilot_core::Result<(String, Arc<dyn Driver>, Vec<OwnedMutexGuard<()>>)> {
        let (id, driver) = {
            let reg = self.state.registry.lock().await;
            let (id, _) = reg.pick(id)?;
            let id = id.to_owned();
            let driver = reg.shared(&id).ok_or_else(|| Error::Session(format!("unknown session `{id}`")))?;
            (id, driver)
        };
        let guards = self.lock_sessions([id.clone()]).await;
        Ok((id, driver, guards))
    }

    async fn start_session(&self, args: &SessionArgs) -> fastpilot_core::Result<String> {
        let name = args.platform.as_deref().unwrap_or("web");
        let platform =
            Platform::parse(name).ok_or_else(|| Error::Flow(format!("unknown platform `{name}` (web, android, ios)")))?;
        let id = {
            let reg = self.state.registry.lock().await;
            match &args.id {
                Some(id) if reg.contains(id) => return Err(Error::Session(format!("session `{id}` already exists"))),
                Some(id) => id.clone(),
                None => (1..).map(|n| format!("{platform}{n}")).find(|id| !reg.contains(id)).unwrap_or_default(),
            }
        };
        let _busy = self.lock_sessions([id.clone()]).await;
        let driver: Box<dyn Driver> = match (platform, args.mode.as_deref().unwrap_or("launch")) {
            (Platform::Web, "attach") => {
                let browser = Browser::connect(args.endpoint.as_deref()).await?;
                Box::new(browser.attach_page().await?)
            }
            (Platform::Android, "launch") => Box::new(fastpilot_android::AndroidDriver::start(args.endpoint.as_deref()).await?),
            (Platform::Ios, "launch") => Box::new(fastpilot_ios::IosDriver::start(args.endpoint.as_deref()).await?),
            (_, "launch") => self.state.launcher.launch(platform).await?,
            (_, mode) => return Err(Error::Unsupported(format!("mode `{mode}` (launch, attach)"))),
        };
        let mut text = format!("session {id} started ({platform})");
        if let Some(target) = &args.target {
            driver.open(target).await?;
            let _ = driver.wait_idle(Duration::from_secs(3)).await;
        }
        let mut header = None;
        if args.target.is_some() || args.mode.as_deref() == Some("attach") || args.test_id.is_some() {
            match driver.snapshot().await {
                Ok(s) => {
                    header = Some(s.header());
                    text = format!("{text}\n{}", s.render(None, 120));
                }
                Err(e) => text = format!("{text}\nsnapshot failed: {e}"),
            }
        }
        if let Some(hint) = args.test_id.as_deref().and_then(|t| self.state.memory.hint(t, header.as_deref())) {
            text = format!("{text}\n\n{hint}");
        }
        self.state.registry.lock().await.insert(id, driver)?;
        Ok(text)
    }

    /// Append a `run_flow` to its test's run: durable YAML, steps, failure evidence.
    async fn record(&self, test_id: &str, flow: &fastpilot_core::Flow, report: &FlowReport, reg: &SessionRegistry) -> String {
        let durable = report.records.iter().filter_map(|r| r.durable.clone().map(|d| (r.index, d))).collect();
        let yaml = flow_to_yaml(flow, &durable);
        let failed_session = report.records.iter().find(|r| r.error.is_some()).and_then(|r| r.session.clone());
        let shot = match failed_session.as_deref().map(|s| reg.get(s)) {
            Some(Ok(d)) => d.screenshot().await.ok(),
            _ => None,
        };
        let mut runs = self.state.runs.lock().await;
        let run = match runs.get_mut(test_id) {
            Some(r) => r,
            None => match self.state.memory.begin_run(test_id) {
                Ok(r) => runs.entry(test_id.to_owned()).or_insert(r),
                Err(e) => return format!("\n(run not recorded: {e})"),
            },
        };
        for r in &report.records {
            if r.label.starts_with("assertVisible") || r.label.starts_with("assertNotVisible") {
                run.record_assertion(Assertion {
                    session: r.session.clone().unwrap_or_default(),
                    what: r.label.clone(),
                    pass: r.error.is_none(),
                    detail: r.error.as_ref().map(ToString::to_string).unwrap_or_default(),
                });
            }
        }
        match run.record_flow(yaml, report, shot) {
            Ok(()) => format!("\nrun {} (close with report)", run.id),
            Err(e) => format!("\n(run not recorded: {e})"),
        }
    }
}

#[tool_router]
impl FastPilot {
    pub fn new(web: LaunchOptions) -> Self {
        Self {
            state: Arc::new(State {
                registry: Mutex::new(SessionRegistry::default()),
                launcher: PlatformLauncher::new(web),
                memory: Memory::from_env(),
                runs: Mutex::new(HashMap::new()),
                busy: Mutex::new(HashMap::new()),
            }),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Session lifecycle: start (returns the first snapshot, and the path-memory hint when test_id is given), stop, list, devices, start_device. Web sessions launch their own Chrome or attach to yours (mode: attach).")]
    async fn session(&self, Parameters(args): Parameters<SessionArgs>) -> Result<CallToolResult, ErrorData> {
        match args.action.as_str() {
            "start" => match self.start_session(&args).await {
                Ok(text) => ok(text),
                Err(e) => err(&e),
            },
            "stop" => {
                let id = match args.id.clone() {
                    Some(id) => Some(id),
                    None => self.state.registry.lock().await.active().map(str::to_owned),
                };
                let Some(id) = id else {
                    return fail("no session to stop");
                };
                let _busy = self.lock_sessions([id.clone()]).await;
                match self.state.registry.lock().await.remove(&id) {
                    Some(_) => ok(format!("session {id} stopped")),
                    None => fail(format!("unknown session `{id}`")),
                }
            }
            "list" => {
                let reg = self.state.registry.lock().await;
                let active = reg.active().map(str::to_owned);
                let lines: Vec<String> = reg
                    .ids()
                    .into_iter()
                    .map(|id| if Some(&id) == active.as_ref() { format!("{id} (active)") } else { id })
                    .collect();
                ok(if lines.is_empty() { "no sessions".into() } else { lines.join("\n") })
            }
            "devices" => ok(devices().await),
            "start_device" => match start_device(args.platform.as_deref(), args.endpoint.as_deref()).await {
                Ok(text) => ok(text),
                Err(e) => err(&e),
            },
            other => fail(format!("unknown action `{other}` (start, stop, list, devices, start_device)")),
        }
    }

    #[tool(description = "Run a YAML flow (batch of steps, may span sessions and `parallel:` blocks). Auto-waits, stops at the first failure, and returns per-step timing plus a diff (or full snapshot) per touched session. Pass test_id to record evidence for `report`.")]
    async fn run_flow(&self, Parameters(args): Parameters<RunFlowArgs>) -> Result<CallToolResult, ErrorData> {
        let (flow, before) = {
            let reg = self.state.registry.lock().await;
            let flow = match check_flow(&args.flow, &reg.ids()) {
                Ok(f) => f,
                Err(errors) => return fail(errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")),
            };
            if let Some(s) = args.session.as_deref().filter(|s| !reg.contains(s)) {
                return fail(format!("unknown session `{s}`"));
            }
            (flow, reg.clone())
        };
        let initial = args.session.clone().or_else(|| before.active().map(str::to_owned));
        let _busy = self.lock_sessions(sessions_used(&flow, initial.as_deref())).await;
        let opts = RunOptions {
            session: args.session,
            stop_on_fail: args.stop_on_fail.unwrap_or(true),
            ..RunOptions::default()
        };
        let mut local = before.clone();
        let report = run_flow(&mut local, &self.state.launcher, &flow, &opts).await;
        let mut text = report.to_string();
        if let Some(t) = &args.test_id {
            text.push_str(&self.record(t, &flow, &report, &local).await);
        }
        self.state.registry.lock().await.absorb(&before, local);
        if report.passed() { ok(text) } else { fail(text) }
    }

    #[tool(description = "Validate a flow without running it: syntax, unsupported steps, unknown sessions, parallel rules, missing ${VAR}s. Errors carry line numbers.")]
    async fn check_flow(&self, Parameters(args): Parameters<CheckFlowArgs>) -> Result<CallToolResult, ErrorData> {
        let ids = self.state.registry.lock().await.ids();
        match check_flow(&args.flow, &ids) {
            Ok(f) => ok(format!("ok: {} steps", f.steps.len())),
            Err(errors) => fail(errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")),
        }
    }

    #[tool(description = "Compact text snapshot of a session (one node per line, stable refs eN). Rarely needed: run_flow already returns state.")]
    async fn snapshot(&self, Parameters(args): Parameters<SnapshotArgs>) -> Result<CallToolResult, ErrorData> {
        let (id, driver, _busy) = match self.session_driver(args.session.as_deref()).await {
            Ok(p) => p,
            Err(e) => return err(&e),
        };
        let max = if args.full == Some(true) || args.file == Some(true) { usize::MAX } else { 120 };
        let snap = match driver.snapshot().await {
            Ok(s) => s,
            Err(e) => return err(&e),
        };
        let text = redact(&format!("[{id}] {}", snap.render_with(args.filter.as_deref(), max, args.coords == Some(true))));
        if args.file != Some(true) {
            return ok(text);
        }
        let dir = std::env::var_os("FASTPILOT_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| ".fastpilot".into()).join("snapshots");
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let path = dir.join(format!("{id}-{ms}.txt"));
        match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, &text)) {
            Ok(()) => ok(format!("[{id}] {} — {} lines written to {}", snap.header(), text.lines().count(), path.display())),
            Err(e) => fail(format!("cannot write {}: {e}", path.display())),
        }
    }

    #[tool(description = "JPEG screenshot of a session, for visual fallback and evidence.")]
    async fn screenshot(&self, Parameters(args): Parameters<ScreenshotArgs>) -> Result<CallToolResult, ErrorData> {
        let (_, driver, _busy) = match self.session_driver(args.session.as_deref()).await {
            Ok(p) => p,
            Err(e) => return err(&e),
        };
        let image = |jpeg: Vec<u8>| ContentBlock::image(base64::engine::general_purpose::STANDARD.encode(jpeg), "image/jpeg");
        if args.annotate.as_deref() != Some("som") {
            return match driver.screenshot().await {
                Ok(jpeg) => Ok(CallToolResult::success(vec![image(jpeg)])),
                Err(e) => err(&e),
            };
        }
        let snap = match driver.snapshot().await {
            Ok(s) => s,
            Err(e) => return err(&e),
        };
        let marks: Vec<(String, fastpilot_core::Rect)> = snap
            .entries
            .iter()
            .filter(|e| !matches!(e.node.role.as_str(), "text" | "heading" | "image"))
            .filter_map(|e| e.node.bounds.filter(|b| b.w > 0.0 && b.h > 0.0).map(|b| (e.r#ref.clone(), b)))
            .collect();
        let list: Vec<String> = marks
            .iter()
            .map(|(r, b)| format!("{r} @{:.0},{:.0} {:.0}x{:.0}", b.x, b.y, b.w, b.h))
            .collect();
        match driver.screenshot_annotated(&marks).await {
            Ok(jpeg) => Ok(CallToolResult::success(vec![
                image(jpeg),
                ContentBlock::text(redact(&format!("marks (tapOn: eN, or {{ point: [x, y] }} at the box center):\n{}", list.join("\n")))),
            ])),
            Err(e) => err(&e),
        }
    }

    #[tool(description = "Deterministic assertion with auto-retry: visible | not_visible | text_equals | contains | enabled. Returns pass, or fail with reason and snapshot.")]
    async fn assert(&self, Parameters(args): Parameters<AssertArgs>) -> Result<CallToolResult, ErrorData> {
        let sel = match Selector::parse_yaml(&args.selector) {
            Ok(s) => s,
            Err(e) => return fail(format!("invalid_flow: selector: {e}")),
        };
        let text = args.text.clone().unwrap_or_default();
        let kind = match args.kind.as_str() {
            "visible" => AssertKind::Visible,
            "not_visible" => AssertKind::NotVisible,
            "text_equals" => AssertKind::TextEquals(text),
            "contains" => AssertKind::Contains(text),
            "enabled" => AssertKind::Enabled,
            other => return fail(format!("unknown kind `{other}` (visible, not_visible, text_equals, contains, enabled)")),
        };
        let (id, driver, _busy) = match self.session_driver(args.session.as_deref()).await {
            Ok(p) => p,
            Err(e) => return err(&e),
        };
        let id = id.as_str();
        let timeout = Duration::from_millis(args.timeout.unwrap_or(5000));
        let res = assert(&*driver, &sel, &kind, timeout).await;
        let what = format!("{} {sel}{}", args.kind, args.text.as_deref().map(|t| format!(" {t:?}")).unwrap_or_default());
        if let Some(t) = &args.test_id {
            let mut runs = self.state.runs.lock().await;
            if !runs.contains_key(t)
                && let Ok(r) = self.state.memory.begin_run(t)
            {
                runs.insert(t.clone(), r);
            }
            if let Some(run) = runs.get_mut(t) {
                run.record_assertion(Assertion {
                    session: id.to_owned(),
                    what: what.clone(),
                    pass: res.is_ok(),
                    detail: res.as_ref().err().map(ToString::to_string).unwrap_or_default(),
                });
            }
        }
        match res {
            Ok(()) => ok(format!("pass [{id}] {what}")),
            Err(e) => {
                let snap = driver.snapshot().await.map(|s| s.render(None, 120)).unwrap_or_default();
                fail(format!("fail [{id}] {what}: {}: {e} [{}]\n[{id}] {snap}", e.reason(), e.category()))
            }
        }
    }

    #[tool(description = "Close a test run: writes .fastpilot/runs/<ts>-<test_id>/report.md (+ steps.jsonl, assertions.json, failure screenshots) and, when status is pass, saves the flow as path memory for the next run.")]
    async fn report(&self, Parameters(args): Parameters<ReportArgs>) -> Result<CallToolResult, ErrorData> {
        let passed = match args.status.as_str() {
            "pass" => true,
            "fail" => false,
            other => return fail(format!("unknown status `{other}` (pass, fail)")),
        };
        let Some(run) = self.state.runs.lock().await.remove(&args.test_id) else {
            return fail(format!("no open run for `{}`; pass test_id to run_flow/assert first", args.test_id));
        };
        match run.finish(&self.state.memory, passed, args.notes.as_deref().unwrap_or("")) {
            Ok(path) => {
                let memo = match (passed, self.state.memory.mode) {
                    (_, fastpilot_memory::Mode::Off) => "path memory off",
                    (_, fastpilot_memory::Mode::ReadOnly) => "path memory read-only (not updated)",
                    (true, _) => "path memory updated",
                    (false, _) => "failure recorded; previous hint kept",
                };
                ok(format!("report {} ({memo})", path.display()))
            }
            Err(e) => fail(format!("report failed: {e}")),
        }
    }

    #[tool(description = "fastpilot reference docs. Topics: dsl, selectors, parallel, examples, errors, memory.")]
    async fn docs(&self, Parameters(args): Parameters<DocsArgs>) -> Result<CallToolResult, ErrorData> {
        let topic = args.topic.as_deref().unwrap_or("dsl");
        match docs::topic(topic) {
            Some(md) => ok(md),
            None => fail(format!("unknown topic `{topic}` (dsl, selectors, parallel, examples, errors, memory)")),
        }
    }
}

async fn devices() -> String {
    let mut out = vec!["web: chrome (launch | attach)".to_owned()];
    match fastpilot_android::devices().await {
        Ok((attached, avds)) => {
            out.extend(attached.into_iter().map(|(s, st)| format!("android: {s} ({st})")));
            out.extend(avds.into_iter().map(|a| format!("android avd: {a} (start_device to boot)")));
        }
        Err(e) => out.push(format!("android: unavailable ({e})")),
    }
    match fastpilot_ios::devices().await {
        Ok(sims) => out.extend(
            sims.into_iter()
                .filter(|(_, n, _)| n.starts_with("iPhone") || n.starts_with("iPad"))
                .map(|(u, n, st)| format!("ios: {n} {u} ({st})")),
        ),
        Err(e) => out.push(format!("ios: unavailable ({e})")),
    }
    out.join("\n")
}

/// Boot a device: `platform: android, endpoint: <AVD name>`.
async fn start_device(platform: Option<&str>, name: Option<&str>) -> fastpilot_core::Result<String> {
    match (platform.unwrap_or("android"), name) {
        ("android", Some(avd)) => {
            let serial = fastpilot_android::start_emulator(avd).await?;
            Ok(format!("android {avd} booted as {serial}"))
        }
        ("android", None) => Err(Error::Flow("start_device needs `endpoint: <AVD name>` (see devices)".into())),
        ("ios", Some(sim)) => {
            let udid = fastpilot_ios::boot(sim).await?;
            Ok(format!("ios simulator {sim} booted ({udid})"))
        }
        (p, _) => Err(Error::Unsupported(format!("start_device for {p}"))),
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FastPilot {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("fastpilot", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "fastpilot drives web, Android and iOS for QA. Start a session (with test_id to get a path-memory hint), \
                 batch steps in one run_flow (it auto-waits and returns diffs), then call report. Read docs(topic: dsl) once.",
            )
    }
}

/// Serve MCP over stdin/stdout until the client disconnects or SIGINT/SIGTERM arrives; either
/// way sessions are dropped, so Chrome, agents and runners are shut down cleanly.
pub async fn serve_stdio(web: LaunchOptions) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let service = FastPilot::new(web).serve(rmcp::transport::stdio()).await?;
    tokio::select! {
        r = service.waiting() => { r?; }
        () = shutdown_signal() => tracing::info!("signal received, shutting down"),
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => return std::future::pending().await,
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
