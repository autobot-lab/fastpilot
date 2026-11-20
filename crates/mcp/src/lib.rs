//! MCP server (stdio): `session`, `run_flow`, `check_flow`, `snapshot`, `screenshot`, `assert`,
//! `report`, `docs` (PRD §10).

mod docs;
pub mod skills;

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
    devices: Devices,
    /// `--reset`: the apps reset so far (each once, at its session start or first launchApp).
    fresh: std::sync::Mutex<Option<std::sync::Arc<fastpilot_core::FreshApps>>>,
}

/// Devices this server's sessions use when a start names none (suite runner: the leased
/// devices of one test). Unset fields fall back to `FASTPILOT_ANDROID_SERIAL` / `FASTPILOT_IOS_UDID`.
#[derive(Debug, Clone, Default)]
pub struct Devices {
    pub android: Option<String>,
    pub ios: Option<String>,
}

impl Devices {
    fn android(&self) -> Option<String> {
        self.android.clone().or_else(|| fastpilot_core::config::var("FASTPILOT_ANDROID_SERIAL").ok())
    }

    fn ios(&self) -> Option<String> {
        self.ios.clone().or_else(|| fastpilot_core::config::var("FASTPILOT_IOS_UDID").ok())
    }
}

impl PlatformLauncher {
    pub fn new(web: LaunchOptions) -> Self {
        Self::with_devices(web, Devices::default())
    }

    pub fn with_devices(web: LaunchOptions, devices: Devices) -> Self {
        Self { web, browser: Mutex::new(None), devices, fresh: std::sync::Mutex::new(None) }
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
    async fn launch(&self, platform: Platform, device: Option<&str>) -> fastpilot_core::Result<Box<dyn Driver>> {
        let spec = device.map(fastpilot_core::DeviceSpec::parse).transpose().map_err(Error::Flow)?;
        if let Some(s) = &spec {
            s.check_platform(platform).map_err(Error::Flow)?;
        }
        let pick = |fallback: Option<String>| spec.as_ref().map(ToString::to_string).or(fallback);
        let driver: Box<dyn Driver> = match platform {
            Platform::Web => Box::new(self.browser().await?.new_page().await?),
            Platform::Android => Box::new(fastpilot_android::AndroidDriver::start(pick(self.devices.android()).as_deref()).await?),
            Platform::Ios => Box::new(fastpilot_ios::IosDriver::start(pick(self.devices.ios()).as_deref()).await?),
        };
        // `session start` without a target, then `launchApp`: the first launch resets the app.
        if let Some(f) = self.fresh() {
            driver.reset_on_first_launch(f);
        }
        Ok(driver)
    }

    fn fresh(&self) -> Option<std::sync::Arc<fastpilot_core::FreshApps>> {
        self.fresh.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

struct State {
    /// D6: the project notes, read once when this server starts (pinned for its whole life).
    notes: Option<String>,
    registry: Mutex<SessionRegistry>,
    launcher: PlatformLauncher,
    memory: Memory,
    /// Open runs by test id, closed by `report`.
    runs: Mutex<HashMap<String, Run>>,
    /// One lock per session: a flow holds the sessions it touches, so tools on *other*
    /// sessions never wait for it (the registry lock itself is only held briefly).
    busy: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Record every session from its start; recordings land in the test's run (`run --record`).
    record: std::sync::atomic::AtomicBool,
    /// When each session's recording started, to put the steps on its timeline in the report.
    recording_started: std::sync::Mutex<HashMap<String, std::time::SystemTime>>,
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
    /// attach: `http://127.0.0.1:9222` or a `ws://` URL; `extension` (or `extension:<port>`) = the
    /// tab you attached with the fastpilot Chrome extension; omitted = Chrome M144+ autoConnect
    /// (enable chrome://inspect/#remote-debugging). android: device serial; ios: simulator UDID
    /// or a real device UDID (needs go-ios, see docs topic devices).
    pub endpoint: Option<String>,
    /// Test case id: the response includes its path-memory hint, if any.
    pub test_id: Option<String>,
    /// android/ios: which device (see action devices): `ios-device:<udid>`, `simulator:<udid>`,
    /// `emulator:<serial>`, `android-device:<serial>`, or a plain id / name. Wins over endpoint.
    pub device: Option<String>,
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
    /// Only interactive nodes (buttons, links, fields), flattened; refs unchanged.
    pub interactive: Option<bool>,
    /// `json`: entries as JSON (ref, role, name, value, flags, box) instead of text.
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotArgs {
    pub session: Option<String>,
    /// `som`: Set-of-Mark — a box labelled with its ref per target (web, android, ios) plus a ref -> coordinates list.
    pub annotate: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AssertArgs {
    pub session: Option<String>,
    /// visible | not_visible | text_equals | contains | enabled
    pub kind: String,
    /// DSL selector: `"Dashboard"`, `e7`, `{ role: button, name: "Sign in" }`, `{ id: total }`.
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
    /// true: quarantine this test (notes = reason); false: release it. Quarantined tests still
    /// run and are reported, but do not fail a suite.
    pub quarantine: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConfigArgs {
    /// list (default) | get | set | unset
    pub action: Option<String>,
    /// Setting key, e.g. allowed_domains, confirm_actions, headful, model_replay (see list).
    pub key: Option<String>,
    /// set: the new value; lists are comma separated (`shop.example.com, *.staging.example.com`).
    pub value: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StateArgs {
    /// save | load | list | delete
    pub action: String,
    /// State name (save, load, delete): `.fastpilot/states/<name>.json`.
    pub name: Option<String>,
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DocsArgs {
    /// dsl | selectors | parallel | examples | errors | memory | devices | skills | skill/<name> (default: dsl)
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

    /// Drop sessions whose browser is gone (fastpilot's Chrome crashed or was closed), so the
    /// same name can be started again and a new Chrome is launched. Returns their names.
    async fn prune(&self) -> Vec<String> {
        let gone: Vec<(String, Arc<dyn Driver>)> = {
            let mut reg = self.state.registry.lock().await;
            let dead: Vec<String> = reg.ids().into_iter().filter(|id| reg.shared(id).is_some_and(|d| d.is_gone())).collect();
            dead.into_iter().filter_map(|id| reg.remove(&id).map(|d| (id, d))).collect()
        };
        for (id, d) in &gone {
            tracing::warn!(session = %id, "browser gone; session dropped");
            let _ = d.close().await;
        }
        gone.into_iter().map(|(id, _)| id).collect()
    }

    /// Why a session that was just dropped by [`Self::prune`] is not there any more.
    fn ended(gone: &[String]) -> Option<String> {
        (!gone.is_empty()).then(|| {
            let names = gone.iter().map(|g| format!("`{g}`")).collect::<Vec<_>>().join(", ");
            format!(
                "note: session {names} ended: fastpilot's Chrome stopped (crashed or was closed). Start it again with `session start` (shell: `fastpilot open <url>`); a new Chrome is launched."
            )
        })
    }

    /// The session's driver plus its lock, without holding the registry lock.
    async fn session_driver(&self, id: Option<&str>) -> fastpilot_core::Result<(String, Arc<dyn Driver>, Vec<OwnedMutexGuard<()>>)> {
        let gone = self.prune().await;
        if let Some(note) = Self::ended(&gone)
            && (id.is_none_or(|i| gone.iter().any(|g| g == i)))
        {
            return Err(Error::Session(note));
        }
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
            // No id (or an empty one): web1, android1, …; one given is checked before anything starts.
            match args.id.as_deref().filter(|i| !i.is_empty()) {
                Some(id) if reg.contains(id) => return Err(Error::Session(format!("session `{id}` already exists"))),
                Some(id) => {
                    fastpilot_core::check_session_id(id)?;
                    id.to_owned()
                }
                None => (1..).map(|n| format!("{platform}{n}")).find(|id| !reg.contains(id)).unwrap_or_default(),
            }
        };
        let _busy = self.lock_sessions([id.clone()]).await;
        // D10d: one test per device, at most `max_parallel` at once; otherwise wait in the queue.
        let device = args.device.clone().or_else(|| args.endpoint.clone()).filter(|_| platform != Platform::Web);
        fastpilot_core::claim::acquire(&id, platform, device.as_deref()).await?;
        let started = self.start_driver(platform, args).await;
        let driver = match started {
            Ok(d) => d,
            Err(e) => {
                fastpilot_core::claim::release(&id);
                return Err(e);
            }
        };
        self.after_start(id, platform, args, driver).await
    }

    async fn start_driver(&self, platform: Platform, args: &SessionArgs) -> fastpilot_core::Result<Box<dyn Driver>> {
        Ok(match (platform, args.mode.as_deref().unwrap_or("launch")) {
            // `extension` / `extension:9231`: the user's tab through the fastpilot extension.
            (Platform::Web, "attach") if args.endpoint.as_deref().is_some_and(|e| e.starts_with("extension")) => {
                let port = args.endpoint.as_deref().and_then(|e| e.strip_prefix("extension:")).and_then(|p| p.parse().ok());
                Box::new(fastpilot_web::extension_page(port).await?)
            }
            (Platform::Web, "attach") => {
                let browser = Browser::connect(args.endpoint.as_deref()).await?;
                Box::new(browser.attach_page().await?)
            }
            (Platform::Android | Platform::Ios, "launch") => {
                let device = args.device.clone().or_else(|| args.endpoint.clone());
                self.state.launcher.launch(platform, device.as_deref()).await?
            }
            (_, "launch") => self.state.launcher.launch(platform, None).await?,
            (_, mode) => return Err(Error::Unsupported(format!("mode `{mode}` (launch, attach)"))),
        })
    }

    async fn after_start(&self, id: String, platform: Platform, args: &SessionArgs, driver: Box<dyn Driver>) -> fastpilot_core::Result<String> {
        let mut text = format!("session {id} started ({platform})");
        if let Some(target) = &args.target {
            if platform != Platform::Web
                && self.state.launcher.fresh().is_some_and(|f| f.claim(target))
                && let Some(done) = fastpilot_core::reset_app(&*driver, platform, target).await?
            {
                text = format!("{text}\n{done}");
            }
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
        if self.state.record.load(std::sync::atomic::Ordering::Relaxed) {
            let started = std::time::SystemTime::now();
            match driver.start_recording().await {
                Ok(()) => {
                    self.state.recording_started.lock().unwrap_or_else(|p| p.into_inner()).insert(id.clone(), started);
                }
                Err(e) => text = format!("{text}\nrecording not started: {e}"),
            }
        }
        if let Some(hint) = args.test_id.as_deref().and_then(|t| self.state.memory.hint(t, header.as_deref())) {
            text = format!("{text}\n\n{hint}");
        }
        self.state.registry.lock().await.insert(id, driver)?;
        Ok(text)
    }

    /// Tool definitions as the Claude Messages API takes them (the same tools MCP serves).
    pub fn api_tools() -> Vec<serde_json::Value> {
        Self::tool_router()
            .list_all()
            .into_iter()
            .map(|t| {
                let mut schema = (*t.input_schema).clone();
                schema.remove("$schema");
                serde_json::json!({
                    "name": t.name,
                    "description": t.description.as_deref().unwrap_or(""),
                    "input_schema": serde_json::Value::Object(schema),
                })
            })
            .collect()
    }

    /// Call a tool in-process (headless runner): `(ok, content blocks as MCP JSON)`.
    pub async fn call_json(&self, name: &str, args: serde_json::Value) -> (bool, Vec<serde_json::Value>) {
        fn parse<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<Parameters<T>, String> {
            serde_json::from_value(v).map(Parameters).map_err(|e| format!("invalid arguments: {e}"))
        }
        let res = match name {
            "session" => match parse(args) { Ok(a) => self.session(a).await, Err(e) => fail(e) },
            "run_flow" => match parse(args) { Ok(a) => self.run_flow(a).await, Err(e) => fail(e) },
            "check_flow" => match parse(args) { Ok(a) => self.check_flow(a).await, Err(e) => fail(e) },
            "snapshot" => match parse(args) { Ok(a) => self.snapshot(a).await, Err(e) => fail(e) },
            "screenshot" => match parse(args) { Ok(a) => self.screenshot(a).await, Err(e) => fail(e) },
            "assert" => match parse(args) { Ok(a) => self.assert(a).await, Err(e) => fail(e) },
            "report" => match parse(args) { Ok(a) => self.report(a).await, Err(e) => fail(e) },
            "docs" => match parse(args) { Ok(a) => self.docs(a).await, Err(e) => fail(e) },
            "state" => match parse(args) { Ok(a) => self.state(a).await, Err(e) => fail(e) },
            "config" => match parse(args) { Ok(a) => self.config(a).await, Err(e) => fail(e) },
            other => fail(format!("unknown tool `{other}`")),
        };
        match res {
            Ok(r) => (r.is_error != Some(true), r.content.iter().filter_map(|c| serde_json::to_value(c).ok()).collect()),
            Err(e) => (false, vec![serde_json::json!({ "type": "text", "text": e.to_string() })]),
        }
    }

    /// Session ids and the active one (shell CLI); sessions whose browser is gone are dropped.
    pub async fn session_ids(&self) -> (Vec<String>, Option<String>) {
        self.prune().await;
        let reg = self.state.registry.lock().await;
        (reg.ids(), reg.active().map(str::to_owned))
    }

    /// Recent page / app errors of a session (console errors, failed requests, crashes).
    pub async fn diagnostics(&self, session: Option<&str>) -> Result<Vec<String>, String> {
        let (_, driver, _busy) = self.session_driver(session).await.map_err(|e| e.to_string())?;
        Ok(driver.diagnostics().await.into_iter().map(|d| redact(&d)).collect())
    }

    /// Close every session (restores device settings) before the process exits.
    pub async fn close_all(&self) {
        let drivers: Vec<Arc<dyn Driver>> = {
            let mut reg = self.state.registry.lock().await;
            reg.ids().into_iter().filter_map(|id| reg.remove(&id)).collect()
        };
        for d in drivers {
            if let Err(e) = d.close().await {
                tracing::warn!(error = %e, "session cleanup failed");
            }
        }
    }

    /// `--reset`: every mobile session resets its app (data, permissions, orientation) before
    /// opening it.
    pub fn set_reset(&self, on: bool) {
        *self.state.launcher.fresh.lock().unwrap_or_else(|p| p.into_inner()) = on.then(|| std::sync::Arc::new(fastpilot_core::FreshApps::default()));
    }

    /// Record every session from its start (headless `run --record`).
    pub fn set_record(&self, on: bool) {
        self.state.record.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Final screen of every session, plus recordings when recording is on, into the run.
    async fn collect_evidence(&self, run: &mut Run) {
        let sessions: Vec<(String, Arc<dyn Driver>)> = {
            let reg = self.state.registry.lock().await;
            reg.ids().into_iter().filter_map(|id| reg.shared(&id).map(|d| (id, d))).collect()
        };
        for (id, d) in sessions {
            let _busy = self.lock_sessions([id.clone()]).await;
            if let Ok(jpeg) = d.screenshot().await {
                run.add_screenshot(&format!("{id} (final)"), jpeg);
            }
            if self.state.record.load(std::sync::atomic::Ordering::Relaxed) {
                match d.stop_recording().await {
                    Ok(rec) => {
                        let started = self.state.recording_started.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
                        if let Err(e) = run.add_recording(&id, rec, started) {
                            tracing::warn!(session = id, error = %e, "recording not saved");
                        }
                    }
                    Err(e) => tracing::debug!(session = id, error = %e, "no recording"),
                }
            }
        }
    }

    /// Close a test's run (opening an empty one if nothing was recorded).
    pub async fn finish_test(
        &self,
        test_id: &str,
        passed: bool,
        notes: &str,
        passed_on_retry: bool,
        could_not_run: bool,
    ) -> Result<fastpilot_memory::Finished, String> {
        let run = self.state.runs.lock().await.remove(test_id);
        let mut run = match run {
            Some(r) => r,
            None => self.state.memory.begin_run(test_id).map_err(|e| e.to_string())?,
        };
        if passed_on_retry {
            run.passed_on_retry();
        }
        if could_not_run {
            run.could_not_run();
        }
        self.collect_evidence(&mut run).await;
        run.finish(&self.state.memory, passed, notes).map_err(|e| e.to_string())
    }

    /// Path-memory hint for a test (headless runner: picks the replay model).
    pub fn hint(&self, test_id: &str) -> Option<String> {
        self.state.memory.hint(test_id, None)
    }

    pub fn quarantined(&self, test_id: &str) -> Option<fastpilot_memory::Quarantine> {
        self.state.memory.quarantined().remove(test_id)
    }

    /// Append a `run_flow` to its test's run: durable YAML, steps, failure evidence.
    async fn record(&self, test_id: &str, flow: &fastpilot_core::Flow, report: &FlowReport, reg: &SessionRegistry, takeovers: &[String]) -> String {
        let durable = report.records.iter().filter_map(|r| r.durable.clone().map(|d| (r.index, d))).collect();
        let yaml = flow_to_yaml(flow, &durable);
        // The failed step's screen, or for a passing flow the screen it ended on.
        let failed_session = report.records.iter().find(|r| r.error.is_some()).and_then(|r| r.session.clone());
        let shot_session = failed_session.or_else(|| report.records.iter().rev().find_map(|r| r.session.clone()));
        let shot = match shot_session.as_deref().map(|s| reg.get(s)) {
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
        for t in takeovers {
            run.record_takeover(t);
        }
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
        Self::with_devices(web, Devices::default())
    }

    /// A server whose sessions default to `devices` (one test of a parallel suite).
    pub fn with_devices(web: LaunchOptions, devices: Devices) -> Self {
        Self {
            state: Arc::new(State {
                registry: Mutex::new(SessionRegistry::default()),
                launcher: PlatformLauncher::with_devices(web, devices),
                memory: Memory::from_env(),
                runs: Mutex::new(HashMap::new()),
                busy: Mutex::new(HashMap::new()),
                record: std::sync::atomic::AtomicBool::new(false),
                recording_started: std::sync::Mutex::new(HashMap::new()),
                notes: fastpilot_memory::notes::Notes::from_env().for_agents(),
            }),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Session lifecycle: start (returns the first snapshot, and the path-memory hint when test_id is given), stop, list, devices, start_device. Web sessions launch their own Chrome or attach to yours (mode: attach).")]
    async fn session(&self, Parameters(args): Parameters<SessionArgs>) -> Result<CallToolResult, ErrorData> {
        let gone = self.prune().await;
        let note = Self::ended(&gone).map(|n| format!("{n}\n")).unwrap_or_default();
        match args.action.as_str() {
            "start" => match self.start_session(&args).await {
                Ok(text) => ok(format!("{note}{text}")),
                Err(e) => fail(format!("{note}{}: {e}", e.reason())),
            },
            "stop" if args.id.as_ref().is_some_and(|id| gone.contains(id)) => ok(note),
            "stop" => {
                let id = match args.id.clone() {
                    Some(id) => Some(id),
                    None => self.state.registry.lock().await.active().map(str::to_owned),
                };
                let Some(id) = id else {
                    return fail("no session to stop");
                };
                let _busy = self.lock_sessions([id.clone()]).await;
                let removed = self.state.registry.lock().await.remove(&id);
                match removed {
                    Some(d) => match d.close().await {
                        Ok(()) => ok(format!("session {id} stopped")),
                        Err(e) => ok(format!("session {id} stopped (cleanup: {e})")),
                    },
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
        let note = Self::ended(&self.prune().await).map(|n| format!("{n}\n")).unwrap_or_default();
        let (flow, before) = {
            let reg = self.state.registry.lock().await;
            let flow = match check_flow(&args.flow, &reg.ids()) {
                Ok(f) => f,
                Err(errors) => return fail(format!("{note}{}", errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))),
            };
            if let Some(s) = args.session.as_deref().filter(|s| !reg.contains(s)) {
                return fail(format!("{note}unknown session `{s}`"));
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
        // The user took a session over in `fastpilot ui` (Ambil alih) and gave it back.
        let used: Vec<String> = report.records.iter().filter_map(|r| r.session.clone()).collect();
        let taken = fastpilot_core::live::take_notes(&used);
        let taken_text: String = taken.iter().map(|n| format!("note: {n}\n")).collect();
        let mut text = format!("{note}{taken_text}{report}");
        if let Some(t) = &args.test_id {
            text.push_str(&self.record(t, &flow, &report, &local, &taken).await);
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
        let snap = if args.interactive == Some(true) { snap.interactive_only() } else { snap };
        if args.format.as_deref() == Some("json") {
            let mut v = snap.to_json();
            v["session"] = serde_json::json!(id);
            return ok(v.to_string());
        }
        let text = redact(&format!("[{id}] {}", snap.render_with(args.filter.as_deref(), max, args.coords == Some(true))));
        if args.file != Some(true) {
            return ok(text);
        }
        let dir = fastpilot_core::config::var_os("FASTPILOT_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| ".fastpilot".into()).join("snapshots");
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        // The id is a checked plain name already; the file name is made safe anyway.
        let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        let path = dir.join(format!("{safe}-{ms}.txt"));
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

    #[tool(description = "Close a test run: writes .fastpilot/runs/<ts>-<test_id>/report.md and a self-contained report.html (steps, timings, diffs, screenshots, recordings) plus steps.jsonl and assertions.json, and, when status is pass, saves the flow as path memory for the next run.")]
    async fn report(&self, Parameters(args): Parameters<ReportArgs>) -> Result<CallToolResult, ErrorData> {
        let passed = match args.status.as_str() {
            "pass" => true,
            "fail" => false,
            other => return fail(format!("unknown status `{other}` (pass, fail)")),
        };
        let Some(mut run) = self.state.runs.lock().await.remove(&args.test_id) else {
            return fail(format!("no open run for `{}`; pass test_id to run_flow/assert first", args.test_id));
        };
        self.collect_evidence(&mut run).await;
        if let Some(q) = args.quarantine {
            let reason = args.notes.as_deref().filter(|n| !n.is_empty()).unwrap_or("quarantined by the agent");
            if let Err(e) = self.state.memory.set_quarantine(&args.test_id, q.then_some(reason)) {
                return fail(format!("quarantine failed: {e}"));
            }
        }
        match run.finish(&self.state.memory, passed, args.notes.as_deref().unwrap_or("")) {
            Ok(done) => {
                let memo = match (passed, self.state.memory.mode) {
                    (_, fastpilot_memory::Mode::Off) => "path memory off",
                    (_, fastpilot_memory::Mode::ReadOnly) => "path memory read-only (not updated)",
                    (true, _) => "path memory updated",
                    (false, _) => "failure recorded; previous hint kept",
                };
                let mut text = format!("report {} ({memo})", done.report.display());
                if let Some(c) = done.class {
                    text.push_str(&format!("\nfailure class: {}", c.as_str()));
                }
                if let Some(st) = done.stats {
                    text.push_str(&format!("\nhistory: {st}"));
                }
                if let Some(q) = done.quarantine {
                    text.push_str(&format!("\nQUARANTINED since {}: {}", q.since, q.reason));
                }
                ok(text)
            }
            Err(e) => fail(format!("report failed: {e}")),
        }
    }

    #[tool(description = "fastpilot settings (~/.fastpilot/config.toml; environment variables win): list, get, set, unset. Changes apply to the next action, no restart. Guard rails (allowed_domains, confirm_actions, block_actions, vars, state_expire_days, allow_* permissions, upload_dirs, max_steps, test_timeout) can only be made stricter here (a running fastpilot also ignores a looser value written to the file until it restarts); program paths and locations (chrome, claude, dir, ios_runner) only by the user with `fastpilot config set` or the file.")]
    async fn config(&self, Parameters(args): Parameters<ConfigArgs>) -> Result<CallToolResult, ErrorData> {
        use fastpilot_core::config::{self, Rule, Source};
        let action = args.action.as_deref().unwrap_or("list");
        if action == "list" {
            return ok(config::describe());
        }
        let Some(key) = args.key.as_deref().map(str::trim).filter(|k| !k.is_empty()).map(str::to_lowercase) else {
            return fail("`key` is required (see action: list)");
        };
        let Some(setting) = config::setting(&key) else {
            return fail(format!("unknown setting `{key}`; see action: list"));
        };
        let current = match config::effective(&key) {
            Source::Env(v) | Source::File(v) => Some(v),
            Source::Default => None,
        };
        if action == "get" {
            return ok(format!("{key} = {}", current.as_deref().unwrap_or("— (default)")));
        }
        let value = match action {
            "set" => match args.value.as_deref() {
                Some(v) => Some(v),
                None => return fail("set needs `value`"),
            },
            "unset" => None,
            other => return fail(format!("unknown action `{other}` (list, get, set, unset)")),
        };
        match setting.rule {
            Rule::HumanOnly => {
                return fail(format!(
                    "`{key}` is a program path or location: only the user changes it (`fastpilot config set {key} <value>` or {})",
                    config::path().display()
                ));
            }
            Rule::TightenOnly if !config::tightens(&key, current.as_deref(), value) => {
                // D9, M13: the AI cannot loosen a guard rail. It may file a structured request that
                // only the user approves, in a terminal (`fastpilot config approve`). Never reported as
                // approved here.
                use fastpilot_core::change;
                let request = change::raw(&key).and_then(|before| {
                    let c = change::Change { key: key.clone(), before, after: value.map(str::to_owned) };
                    change::add_pending(&c)
                });
                // block_actions: words are added on top of the built-in list, which stays `default`.
                let add = if key == "block_actions" { "\nTo add words instead, keep the built-in list: value \"default,<word>\"; to allow one built-in word only: \"default,!<word>\"." } else { "" };
                return match request {
                    Ok(card) => fail(format!(
                        "I can't loosen a limit myself. I saved a request for the user to approve:\n  {}  LOOSENS A LIMIT\n  Now:   {}\n  After: {}\nTo approve it, the user opens a terminal and runs:\n  fastpilot config approve\nNothing has changed. Tell the user; do not try another way.{add}",
                        card.title, card.now, card.after
                    )),
                    Err(e) => fail(format!("`{key}` is a guard rail and this change would loosen it, so it needs the user. It was not saved as a request: {e}. Nothing has changed.{add}")),
                };
            }
            _ => {}
        }
        if let Source::Env(v) = config::effective(&key) {
            return fail(format!("`{key}` is set by the environment (FASTPILOT_{} = {v}), which wins over the file: change it there", key.to_uppercase()));
        }
        let before = config::file_values().remove(&key);
        match config::write(&key, value) {
            Ok(p) => ok({
                fastpilot_core::change::audit(&key, before.as_deref(), value, "terminal", "applied");
                format!(
                "{key} {} in {} (applies to the next action)",
                value.map_or("removed".to_owned(), |v| format!("= {v}")),
                p.display()
            )
            }),
            Err(e) => fail(e),
        }
    }

    #[tool(description = "Saved login state (web: cookies + localStorage), so a test can skip the login: save | load | list | delete. Files are owner-only, expire after FASTPILOT_STATE_EXPIRE_DAYS (30) and are encrypted when FASTPILOT_STATE_KEY is set. Same as the saveState / loadState steps.")]
    async fn state(&self, Parameters(args): Parameters<StateArgs>) -> Result<CallToolResult, ErrorData> {
        use fastpilot_core::state;
        let name = || args.name.as_deref().filter(|n| !n.is_empty()).ok_or("`name` is required");
        match args.action.as_str() {
            "list" => {
                let all = state::list();
                if all.is_empty() {
                    return ok(format!("no saved states in {}", state::dir().display()));
                }
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let lines: Vec<String> = all
                    .iter()
                    .map(|(n, at, enc)| format!("{n}  saved {} h ago{}", now.saturating_sub(*at) / 3600, if *enc { ", encrypted" } else { "" }))
                    .collect();
                ok(lines.join("\n"))
            }
            "delete" => match name().map_err(str::to_owned).and_then(state::delete) {
                Ok(p) => ok(format!("deleted {}", p.display())),
                Err(e) => fail(e),
            },
            action @ ("save" | "load") => {
                let n = match name() {
                    Ok(n) => n.to_owned(),
                    Err(e) => return fail(e),
                };
                let step = if action == "save" { "saveState" } else { "loadState" };
                let flow = format!("- {step}: {}\n", serde_json::to_string(&n).unwrap_or_default());
                self.run_flow(Parameters(RunFlowArgs { flow, session: args.session.clone(), stop_on_fail: None, test_id: None })).await
            }
            other => fail(format!("unknown action `{other}` (save, load, list, delete)")),
        }
    }

    #[tool(description = "fastpilot reference docs. Topics: dsl, selectors, parallel, examples, errors, memory, devices, skills (the testing skills: regression, bug-report, test techniques, ...), skill/<name> (one skill to follow).")]
    async fn docs(&self, Parameters(args): Parameters<DocsArgs>) -> Result<CallToolResult, ErrorData> {
        let topic = args.topic.as_deref().unwrap_or("dsl");
        if topic == "skills" {
            return ok(skills::index());
        }
        if let Some(name) = topic.strip_prefix("skill/") {
            return match skills::get(name) {
                Some(text) => ok(text),
                None => fail(format!("unknown skill `{name}`; docs(topic: \"skills\") lists them")),
            };
        }
        match docs::topic(topic) {
            Some(md) => ok(md),
            None => fail(format!("unknown topic `{topic}` (dsl, selectors, parallel, examples, errors, memory, devices, skills, skill/<name>)")),
        }
    }
}

async fn devices() -> String {
    let mut out = vec!["web: chrome (launch | attach)".to_owned()];
    match fastpilot_android::device::list().await {
        // Every state is listed, with what to do when the phone needs the user.
        Ok(list) => out.extend(list.iter().map(fastpilot_core::DeviceInfo::line)),
        Err(e) => out.push(format!("android: unavailable ({e})")),
    }
    if let Ok((_, avds)) = fastpilot_android::devices().await {
        out.extend(avds.into_iter().map(|a| format!("android avd: {a} (start_device to boot)")));
    }
    // Simulators and paired iPhones (physical ones are used only when named: device: ios-device:<udid>).
    out.extend(fastpilot_ios::device_infos().await.iter().map(fastpilot_core::DeviceInfo::line));
    out.join("\n")
}

/// Boot a device: `platform: android, endpoint: <AVD name>`.
async fn start_device(platform: Option<&str>, name: Option<&str>) -> fastpilot_core::Result<String> {
    if let Some(spec) = name.and_then(|n| fastpilot_core::DeviceSpec::parse(n).ok())
        && spec.kind.is_some_and(fastpilot_core::DeviceKind::physical)
    {
        return Err(Error::Unsupported("physical devices cannot be started; connect and unlock one".into()));
    }
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
            .with_instructions(instructions(self.state.notes.as_deref()))
    }
}

/// What the client's model is told at the start: how to use fastpilot, then the project's notes
/// (D6) when there are any, marked as the project owner's own context.
pub fn instructions(notes: Option<&str>) -> String {
    let base = "fastpilot drives web, Android and iOS for QA. Start a session (with test_id to get a path-memory hint), \
                batch steps in one run_flow (it auto-waits and returns diffs), then call report. Read docs(topic: dsl) once.";
    match notes {
        Some(n) => format!("{base}\n\n{n}"),
        None => base.to_owned(),
    }
}

/// Serve MCP over stdin/stdout until the client disconnects or SIGINT/SIGTERM arrives; either
/// way sessions are dropped, so Chrome, agents and runners are shut down cleanly.
pub async fn serve_stdio(web: LaunchOptions, reset: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = FastPilot::new(web);
    server.set_reset(reset);
    let service = server.clone().serve(rmcp::transport::stdio()).await?;
    tokio::select! {
        r = service.waiting() => { r?; }
        () = shutdown_signal() => tracing::info!("signal received, shutting down"),
    }
    server.close_all().await;
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
