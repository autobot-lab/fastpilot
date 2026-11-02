//! MCP server (stdio): `session`, `run_flow`, `check_flow`, `snapshot`, `screenshot`, `docs`.

mod docs;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use fastpilot_core::{
    Driver, Error, Launcher, Platform, RunOptions, SessionRegistry, check_flow, run_flow,
};
use fastpilot_web::{Browser, LaunchOptions};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::{Mutex, OnceCell};

/// Starts drivers for `session start` and `session: { start: ... }` steps. One Chrome process
/// is launched lazily and shared by every web session.
// ponytail: a crashed Chrome stays cached; restart fastpilot. Relaunch-on-close if it bites.
pub struct PlatformLauncher {
    web: LaunchOptions,
    browser: OnceCell<Arc<Browser>>,
}

impl PlatformLauncher {
    pub fn new(web: LaunchOptions) -> Self {
        Self { web, browser: OnceCell::new() }
    }
}

#[async_trait]
impl Launcher for PlatformLauncher {
    async fn launch(&self, platform: Platform) -> fastpilot_core::Result<Box<dyn Driver>> {
        match platform {
            Platform::Web => {
                let browser = self.browser.get_or_try_init(|| Browser::launch(self.web.clone())).await?;
                Ok(Box::new(browser.new_page().await?))
            }
            p => Err(Error::Unsupported(format!(
                "platform `{p}` is not available yet (android: Phase 2, ios: Phase 5)"
            ))),
        }
    }
}

struct State {
    registry: Mutex<SessionRegistry>,
    launcher: PlatformLauncher,
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
    /// Session id. start: optional (defaults to web1, web2, ...); stop: defaults to the active session.
    pub id: Option<String>,
    /// web (default). android / ios arrive in later phases.
    pub platform: Option<String>,
    /// start: URL or local file path to open right away; the response includes its snapshot.
    pub target: Option<String>,
    /// launch (default) | attach (Phase 4)
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunFlowArgs {
    /// Flow YAML: a list of steps. See `docs` (topic: dsl).
    pub flow: String,
    /// Initial active session (defaults to the last active one).
    pub session: Option<String>,
    /// Stop at the first failed step (default true).
    pub stop_on_fail: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckFlowArgs {
    /// Flow YAML to validate without running it.
    pub flow: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SnapshotArgs {
    pub session: Option<String>,
    /// Only nodes whose role or name contains this text (case-insensitive).
    pub filter: Option<String>,
    /// Lift the 120-line cap.
    pub full: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotArgs {
    pub session: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DocsArgs {
    /// dsl | selectors | parallel | examples | errors (default: dsl)
    pub topic: Option<String>,
}

fn ok(text: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn fail(text: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![ContentBlock::text(text)]))
}

fn err(e: &Error) -> Result<CallToolResult, ErrorData> {
    fail(format!("{}: {e}", e.reason()))
}

#[tool_router]
impl FastPilot {
    pub fn new(web: LaunchOptions) -> Self {
        Self {
            state: Arc::new(State {
                registry: Mutex::new(SessionRegistry::default()),
                launcher: PlatformLauncher::new(web),
            }),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Session lifecycle: start (returns the first snapshot when `target` is given), stop, list, devices, start_device.")]
    async fn session(&self, Parameters(args): Parameters<SessionArgs>) -> Result<CallToolResult, ErrorData> {
        let mut reg = self.state.registry.lock().await;
        match args.action.as_str() {
            "start" => {
                if args.mode.as_deref().is_some_and(|m| m != "launch") {
                    return fail("unsupported: only mode `launch` is available (attach arrives in Phase 4)");
                }
                let name = args.platform.as_deref().unwrap_or("web");
                let Some(platform) = Platform::parse(name) else {
                    return fail(format!("unknown platform `{name}` (web, android, ios)"));
                };
                let id = match args.id {
                    Some(id) if reg.contains(&id) => return fail(format!("session `{id}` already exists")),
                    Some(id) => id,
                    None => (1..).map(|n| format!("{platform}{n}")).find(|id| !reg.contains(id)).unwrap_or_default(),
                };
                let driver = match self.state.launcher.launch(platform).await {
                    Ok(d) => d,
                    Err(e) => return err(&e),
                };
                let mut text = format!("session {id} started ({platform})");
                if let Some(target) = &args.target {
                    if let Err(e) = driver.open(target).await {
                        return err(&e);
                    }
                    let _ = driver.wait_idle(Duration::from_secs(3)).await;
                    match driver.snapshot().await {
                        Ok(s) => text = format!("{text}\n{}", s.render(None, 120)),
                        Err(e) => text = format!("{text}\nsnapshot failed: {e}"),
                    }
                }
                if let Err(e) = reg.insert(id, driver) {
                    return err(&e);
                }
                ok(text)
            }
            "stop" => {
                let Some(id) = args.id.or_else(|| reg.active().map(str::to_owned)) else {
                    return fail("no session to stop");
                };
                match reg.remove(&id) {
                    Some(_) => ok(format!("session {id} stopped")),
                    None => fail(format!("unknown session `{id}`")),
                }
            }
            "list" => {
                let active = reg.active().map(str::to_owned);
                let lines: Vec<String> = reg
                    .ids()
                    .into_iter()
                    .map(|id| if Some(&id) == active.as_ref() { format!("{id} (active)") } else { id })
                    .collect();
                ok(if lines.is_empty() { "no sessions".into() } else { lines.join("\n") })
            }
            "devices" => ok("web: chrome (launch mode)\nandroid: Phase 2\nios: Phase 5"),
            "start_device" => fail("unsupported: device management arrives with Android (Phase 2)"),
            other => fail(format!("unknown action `{other}` (start, stop, list, devices, start_device)")),
        }
    }

    #[tool(description = "Run a YAML flow (batch of steps, may span sessions and `parallel:` blocks). Auto-waits, stops at the first failure, and returns per-step timing plus a diff (or full snapshot) per touched session.")]
    async fn run_flow(&self, Parameters(args): Parameters<RunFlowArgs>) -> Result<CallToolResult, ErrorData> {
        let mut reg = self.state.registry.lock().await;
        let flow = match check_flow(&args.flow, &reg.ids()) {
            Ok(f) => f,
            Err(errors) => return fail(errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")),
        };
        if let Some(s) = args.session.as_deref().filter(|s| !reg.contains(s)) {
            return fail(format!("unknown session `{s}`"));
        }
        let opts = RunOptions {
            session: args.session,
            stop_on_fail: args.stop_on_fail.unwrap_or(true),
        };
        let report = run_flow(&mut reg, &self.state.launcher, &flow, &opts).await;
        if report.passed() { ok(report.to_string()) } else { fail(report.to_string()) }
    }

    #[tool(description = "Validate a flow without running it: syntax, unsupported steps, unknown sessions, parallel rules, missing ${ENV} variables. Errors carry line numbers.")]
    async fn check_flow(&self, Parameters(args): Parameters<CheckFlowArgs>) -> Result<CallToolResult, ErrorData> {
        let ids = self.state.registry.lock().await.ids();
        match check_flow(&args.flow, &ids) {
            Ok(f) => ok(format!("ok: {} steps", f.steps.len())),
            Err(errors) => fail(errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")),
        }
    }

    #[tool(description = "Compact text snapshot of a session (one node per line, stable refs eN). Rarely needed: run_flow already returns state.")]
    async fn snapshot(&self, Parameters(args): Parameters<SnapshotArgs>) -> Result<CallToolResult, ErrorData> {
        let reg = self.state.registry.lock().await;
        let (id, driver) = match reg.pick(args.session.as_deref()) {
            Ok(p) => p,
            Err(e) => return err(&e),
        };
        let max = if args.full == Some(true) { usize::MAX } else { 120 };
        match driver.snapshot().await {
            Ok(s) => ok(format!("[{id}] {}", s.render(args.filter.as_deref(), max))),
            Err(e) => err(&e),
        }
    }

    #[tool(description = "JPEG screenshot of a session, for visual fallback and evidence.")]
    async fn screenshot(&self, Parameters(args): Parameters<ScreenshotArgs>) -> Result<CallToolResult, ErrorData> {
        let reg = self.state.registry.lock().await;
        let (_, driver) = match reg.pick(args.session.as_deref()) {
            Ok(p) => p,
            Err(e) => return err(&e),
        };
        match driver.screenshot().await {
            Ok(jpeg) => Ok(CallToolResult::success(vec![ContentBlock::image(
                base64::engine::general_purpose::STANDARD.encode(jpeg),
                "image/jpeg",
            )])),
            Err(e) => err(&e),
        }
    }

    #[tool(description = "fastpilot reference docs. Topics: dsl, selectors, parallel, examples, errors.")]
    async fn docs(&self, Parameters(args): Parameters<DocsArgs>) -> Result<CallToolResult, ErrorData> {
        let topic = args.topic.as_deref().unwrap_or("dsl");
        match docs::topic(topic) {
            Some(md) => ok(md),
            None => fail(format!("unknown topic `{topic}` (dsl, selectors, parallel, examples, errors)")),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FastPilot {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("fastpilot", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "fastpilot drives browsers (Android/iOS later) for QA. Start a session, then batch steps in one \
                 run_flow; it auto-waits and returns diffs, so snapshot is rarely needed. Read docs(topic: dsl) once.",
            )
    }
}

/// Serve MCP over stdin/stdout until the client disconnects.
pub async fn serve_stdio(web: LaunchOptions) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let service = FastPilot::new(web).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
