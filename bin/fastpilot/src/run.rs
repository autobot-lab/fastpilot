//! `fastpilot run <tests dir|file>`: headless runner. Each natural-language test case is driven
//! by the user's own Claude Code (`claude -p`, their login and plan; fastpilot calls no model API
//! and needs no API key). Claude Code reaches this process's tools over MCP: a per-test server on
//! 127.0.0.1 behind `fastpilot mcp-bridge` (the stdio command Claude Code spawns), so path-memory
//! hints, evidence and the `finish` verdict stay in-process. Retries within a budget, an infra
//! circuit breaker, and JUnit/HTML output.
//!
//! Models (PRD §12.5): a test with a path-memory hint starts on the fast model to replay it;
//! a failed replay, or a test without a hint, runs on the larger model. `--model` pins one.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use std::sync::{Arc, Mutex};

use fastpilot_core::Platform;
use fastpilot_mcp::{Devices, FastPilot};
use fastpilot_memory::{Breaker, Class, RetryBudget};
use fastpilot_web::LaunchOptions;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::pool::{Device, Pool};

/// Exploration / judgment (PRD §12.5: the larger model); `model_explore` setting overrides.
pub fn model_explore() -> String {
    fastpilot_core::config::var("FASTPILOT_MODEL_EXPLORE").ok().filter(|m| !m.is_empty()).unwrap_or_else(|| "claude-opus-5-5".into())
}
/// Replaying a path-memory hint (PRD §12.5: the fast model); `model_replay` setting overrides.
pub fn model_replay() -> String {
    fastpilot_core::config::var("FASTPILOT_MODEL_REPLAY").ok().filter(|m| !m.is_empty()).unwrap_or_else(|| "claude-haiku-5-5".into())
}
/// One Claude Code invocation may take this long before it is stopped.
const CASE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

const SYSTEM: &str = "You are fastpilot's QA agent running one test case unattended in CI. Nobody is watching and \
nobody can answer questions: decide for yourself, and always finish.

How to work:
- Start a session for the test's platform and target with the `session` tool (action: start).
- If a path-memory hint is given, send it as one run_flow first; explore only from the step that fails.
- Batch steps in run_flow: it auto-waits and returns per-step results plus a diff or snapshot. Use snapshot or \
screenshot only when the returned state is not enough. Read docs(topic: dsl) once if unsure about a step.
- Verify every expectation in the test case with assertVisible or the assert tool; do not assume.
- `${NAME}` variables are masked as *** in results; keep using the ${NAME} form in flows.
- When done, call `finish` exactly once: status pass only if every expectation was verified, otherwise fail, with \
notes saying what was checked or what went wrong (expected vs actual).
- Only the fastpilot tools are available; do not ask for others.";

#[derive(Clone)]
pub struct Options {
    pub model: Option<String>,
    pub max_steps: usize,
    pub junit: Option<PathBuf>,
    pub html: Option<PathBuf>,
    pub web: LaunchOptions,
    /// Tests run at once (each with its own Chrome and leased devices).
    pub workers: usize,
    /// Record every session (web screencast, Android screenrecord, iOS simctl) into the run.
    pub record: bool,
}

/// One natural-language test case (`tests/*.md`: `# Title`, `id:`, `platform:`, steps).
#[derive(Debug, Clone)]
pub struct Case {
    pub id: String,
    pub title: String,
    pub platform: String,
    pub text: String,
    pub path: PathBuf,
}

impl Case {
    fn parse(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let field = |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().to_owned()));
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Self {
            id: field("id:").unwrap_or(stem),
            title: text.lines().find_map(|l| l.strip_prefix("# ").map(str::to_owned)).unwrap_or_default(),
            platform: field("platform:").unwrap_or_else(|| "web".into()),
            text,
            path: path.to_owned(),
        })
    }

    /// Platforms the test uses (`platform:` and `sessions:` lines); web when none is named.
    pub fn platforms(&self) -> Vec<Platform> {
        let lines: String = self
            .text
            .lines()
            .filter(|l| l.trim_start().starts_with("platform:") || l.trim_start().starts_with("sessions:"))
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        let mut out: Vec<Platform> = [Platform::Web, Platform::Android, Platform::Ios]
            .into_iter()
            .filter(|p| lines.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == p.to_string()))
            .collect();
        if out.is_empty() {
            out.push(Platform::Web);
        }
        out
    }
}

/// Test cases under `path` (a `.md` file, or every `.md` directly in a directory, sorted).
pub fn load(path: &Path) -> Result<Vec<Case>, String> {
    if path.is_file() {
        return Ok(vec![Case::parse(path)?]);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md") && p.is_file())
        .collect();
    files.sort();
    files.iter().map(|p| Case::parse(p)).collect()
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
}

#[derive(Debug, Clone)]
pub struct CaseResult {
    pub case: Case,
    pub passed: bool,
    pub class: Option<Class>,
    pub notes: String,
    pub turns: usize,
    pub elapsed: Duration,
    pub models: Vec<String>,
    pub usage: Usage,
    pub quarantined: bool,
    pub attempts: usize,
    /// Tool calls and their (redacted, clipped) results, for the reports.
    pub transcript: Vec<String>,
    pub report: Option<PathBuf>,
    /// Devices of the last attempt.
    pub devices: Vec<String>,
}

impl CaseResult {
    fn skipped(case: &Case, notes: &str) -> Self {
        Self {
            case: case.clone(),
            passed: false,
            class: Some(Class::Infra),
            notes: notes.to_owned(),
            turns: 0,
            elapsed: Duration::ZERO,
            models: Vec::new(),
            usage: Usage::default(),
            quarantined: false,
            attempts: 0,
            transcript: Vec::new(),
            report: None,
            devices: Vec::new(),
        }
    }
}

fn finish_tool() -> Value {
    json!({
        "name": "finish",
        "description": "Call exactly once when the test case is done: pass only if every expectation was verified.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["pass", "fail"] },
                "notes": { "type": "string", "description": "What was verified, or expected vs actual for a failure." }
            },
            "required": ["status", "notes"],
            "additionalProperties": false
        }
    })
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_owned() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

/// Claude Code's CLI: `FASTPILOT_CLAUDE`, else `claude` on PATH.
pub fn claude_bin() -> String {
    fastpilot_core::config::var("FASTPILOT_CLAUDE").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "claude".into())
}

/// What the tools of one test case share with Claude Code's calls.
struct Agent {
    fp: FastPilot,
    case_id: String,
    max_steps: usize,
    verdict: Mutex<Option<(bool, String)>>,
    transcript: Mutex<Vec<String>>,
    steps: std::sync::atomic::AtomicUsize,
    replay_failed: std::sync::atomic::AtomicBool,
}

impl Agent {
    /// MCP `tools/list`: the server's tools (without `report`: the runner closes the run) plus `finish`.
    fn tools() -> Vec<Value> {
        let mut tools: Vec<Value> = FastPilot::api_tools()
            .into_iter()
            .filter(|t| t["name"] != "report" && t["name"] != "config")
            .map(|t| json!({ "name": t["name"], "description": t["description"], "inputSchema": t["input_schema"] }))
            .collect();
        tools.push(finish_tool());
        tools
    }

    async fn call(&self, name: &str, mut input: Value) -> Value {
        let text = |t: String, err: bool| json!({ "content": [{ "type": "text", "text": t }], "isError": err });
        if name == "config" {
            return text("settings are not changed during an unattended run".into(), true);
        }
        if name == "finish" {
            let pass = input["status"] == "pass";
            let notes = input["notes"].as_str().unwrap_or("").to_owned();
            lock(&self.transcript).push(format!("finish {}: {}", if pass { "pass" } else { "fail" }, fastpilot_core::redact(&notes)));
            *lock(&self.verdict) = Some((pass, notes));
            return text("recorded; the test is over, stop here".into(), false);
        }
        if lock(&self.verdict).is_some() {
            return text("the test already has a verdict (finish was called); stop here".into(), true);
        }
        let n = self.steps.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        if n > self.max_steps {
            return text(format!("step budget of {} tool calls is used up: call finish now", self.max_steps), true);
        }
        // Evidence always lands in this test's run, whether or not the model passed test_id.
        let start = name == "session" && input["action"] == "start";
        if (start || name == "run_flow" || name == "assert") && input.get("test_id").is_none_or(Value::is_null) {
            input["test_id"] = json!(self.case_id);
        }
        let (ok, out) = self.fp.call_json(name, input.clone()).await;
        let joined: String = out.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");
        lock(&self.transcript).push(format!("{} {} {}\n{}", if ok { "✓" } else { "✗" }, name, clip(&input.to_string(), 300), clip(&joined, 1500)));
        if !ok && name == "run_flow" {
            self.replay_failed.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        json!({ "content": out, "isError": !ok })
    }

    /// One JSON-RPC message from Claude Code; `None` for notifications.
    async fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned()?;
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fastpilot", "version": env!("CARGO_PKG_VERSION") },
            }),
            "ping" => json!({}),
            "tools/list" => json!({ "tools": Self::tools() }),
            "tools/call" => {
                let p = &msg["params"];
                self.call(p["name"].as_str().unwrap_or(""), p["arguments"].clone()).await
            }
            other => return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("method not found: {other}") } })),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Serve `agent` as an MCP server (newline-delimited JSON-RPC) to every connection that first
/// sends `token`. Returns the address.
async fn serve(agent: Arc<Agent>, token: String) -> std::io::Result<(String, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?.to_string();
    let task = tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let (agent, token) = (agent.clone(), token.clone());
            tokio::spawn(async move {
                let (r, mut w) = sock.into_split();
                let mut lines = BufReader::new(r).lines();
                if lines.next_line().await.ok().flatten().as_deref() != Some(token.as_str()) {
                    return;
                }
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                    if let Some(reply) = agent.handle(&msg).await
                        && (w.write_all(format!("{reply}\n").as_bytes()).await.is_err() || w.flush().await.is_err())
                    {
                        break;
                    }
                }
            });
        }
    });
    Ok((addr, task))
}

/// `fastpilot mcp-bridge <addr> <token>`: the stdio MCP server Claude Code spawns; it relays
/// stdin/stdout to the runner's in-process server for one test case.
pub async fn bridge(addr: &str, token: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let sock = tokio::net::TcpStream::connect(addr).await?;
    let (mut r, mut w) = sock.into_split();
    w.write_all(format!("{token}\n").as_bytes()).await?;
    let up = async {
        let _ = tokio::io::copy(&mut tokio::io::stdin(), &mut w).await;
        let _ = w.shutdown().await;
    };
    let down = async {
        let _ = tokio::io::copy(&mut r, &mut tokio::io::stdout()).await;
    };
    tokio::select! {
        () = up => {}
        () = down => {}
    }
    Ok(())
}

/// What one `claude -p` invocation reported.
struct Session {
    turns: usize,
    usage: Usage,
    /// Set when Claude Code itself failed (not installed, not logged in, crashed, timed out).
    failure: Option<String>,
}

/// Run Claude Code once on `prompt` with only this test's fastpilot tools.
async fn claude_session(prompt: &str, model: &str, addr: &str, token: &str) -> Session {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "fastpilot".into());
    let config = json!({ "mcpServers": { "fastpilot": { "type": "stdio", "command": exe, "args": ["mcp-bridge", addr, token] } } });
    let effort = if model == model_replay() { "low" } else { "medium" };
    let child = tokio::process::Command::new(claude_bin())
        .args(["-p", "--output-format", "json", "--model", model, "--effort", effort])
        .args(["--append-system-prompt", SYSTEM])
        .args(["--mcp-config", &config.to_string(), "--strict-mcp-config"])
        // Only fastpilot: no shell, file or web tools for an unattended agent.
        .args(["--tools", "", "--allowedTools", "mcp__fastpilot", "--permission-mode", "dontAsk", "--no-session-persistence"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return Session { turns: 0, usage: Usage::default(), failure: Some(format!("cannot start Claude Code (`{}`): {e}", claude_bin())) },
    };
    // The prompt goes on stdin: variadic options (--tools, --mcp-config) would swallow it.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes()).await;
    }
    let out = match tokio::time::timeout(CASE_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Session { turns: 0, usage: Usage::default(), failure: Some(format!("Claude Code: {e}")) },
        Err(_) => return Session { turns: 0, usage: Usage::default(), failure: Some(format!("Claude Code did not finish within {} min", CASE_TIMEOUT.as_secs() / 60)) },
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let result: Value = stdout.lines().rev().find_map(|l| serde_json::from_str(l).ok()).unwrap_or_default();
    let u = &result["usage"];
    let usage = Usage {
        input: u["input_tokens"].as_u64().unwrap_or(0),
        output: u["output_tokens"].as_u64().unwrap_or(0),
        cache_read: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
    };
    let failed = !out.status.success() || result["is_error"] == true;
    let failure = failed.then(|| {
        let why = result["result"].as_str().map(str::to_owned).unwrap_or_else(|| format!("{}{}", stdout.trim(), String::from_utf8_lossy(&out.stderr).trim()));
        format!("Claude Code failed ({}): {}", out.status, clip(why.trim(), 400))
    });
    Session { turns: result["num_turns"].as_u64().unwrap_or(0) as usize, usage, failure }
}

/// Drive one test case to a verdict.
async fn run_case(case: &Case, opts: &Options, passed_on_retry: bool, devices: &[Device]) -> CaseResult {
    let started = Instant::now();
    let pick = |p: Platform| devices.iter().find(|d| d.platform == p).map(|d| d.id.clone());
    let fp = FastPilot::with_devices(opts.web.clone(), Devices { android: pick(Platform::Android), ios: pick(Platform::Ios) });
    fp.set_record(opts.record);
    let hint = fp.hint(&case.id);
    let mut model = match (&opts.model, &hint) {
        (Some(m), _) => m.clone(),
        (None, Some(_)) => model_replay(),
        (None, None) => model_explore(),
    };
    let mut models = vec![model.clone()];
    let plan: Vec<String> = case
        .platforms()
        .iter()
        .map(|p| match pick(*p) {
            Some(id) => format!("{p}: device {id} (assigned; start sessions without an endpoint)"),
            None => format!("{p}: {}", if *p == Platform::Web { "your own Chrome" } else { "default device" }),
        })
        .collect();
    let intro = format!(
        "Test case `{}` (file {}):\n\n{}\n\nSession plan: {}.\n\n{}",
        case.id,
        case.path.display(),
        fastpilot_core::to_vars(case.text.trim()),
        plan.join("; "),
        hint.as_deref().unwrap_or("No path memory for this test yet: explore.")
    );
    let agent = Arc::new(Agent {
        fp: fp.clone(),
        case_id: case.id.clone(),
        max_steps: opts.max_steps,
        verdict: Mutex::new(None),
        transcript: Mutex::new(Vec::new()),
        steps: std::sync::atomic::AtomicUsize::new(0),
        replay_failed: std::sync::atomic::AtomicBool::new(false),
    });
    // Only this test's Claude Code may connect: a random token it passes to the bridge.
    let token: String = (0..2)
        .map(|_| {
            use std::hash::{BuildHasher, Hasher};
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(started.elapsed().as_nanos());
            format!("{:016x}", h.finish())
        })
        .collect();
    let mut usage = Usage::default();
    let mut turns = 0;
    let mut forced_class = None;
    let mut failure = None;
    match serve(agent.clone(), token.clone()).await {
        Err(e) => failure = Some(format!("cannot serve the tools to Claude Code: {e}")),
        Ok((addr, server)) => {
            let mut prompt = intro;
            loop {
                let s = claude_session(&prompt, &model, &addr, &token).await;
                turns += s.turns;
                usage.input += s.usage.input;
                usage.output += s.usage.output;
                usage.cache_read += s.usage.cache_read;
                let verdict = lock(&agent.verdict).clone();
                if verdict.is_none() && s.failure.is_some() {
                    failure = s.failure;
                    break;
                }
                // The hint no longer applies: hand over to the larger model, sessions as they are.
                let replay_failed = agent.replay_failed.load(std::sync::atomic::Ordering::SeqCst);
                if replay_failed && !verdict.as_ref().is_some_and(|v| v.0) && opts.model.is_none() && model == model_replay() {
                    model = model_explore();
                    models.push(model.clone());
                    lock(&agent.transcript).push(format!("replay failed: switching to {model}"));
                    *lock(&agent.verdict) = None;
                    prompt = format!(
                        "{prompt}\n\nThe path-memory replay failed. The sessions are still open where it stopped: take a \
                         snapshot, explore from there, and call finish."
                    );
                    continue;
                }
                break;
            }
            server.abort();
        }
    }
    let mut transcript = std::mem::take(&mut *lock(&agent.transcript));
    let verdict = lock(&agent.verdict).clone();
    let (passed, notes) = match (verdict, failure) {
        (Some(v), _) => v,
        (None, Some(f)) => {
            transcript.push(f.clone());
            forced_class = Some(Class::Infra);
            (false, f)
        }
        (None, None) => {
            forced_class = Some(Class::Agent);
            (false, "the agent ended without calling finish".to_owned())
        }
    };
    let (class, report) = match fp.finish_test(&case.id, passed, &notes, passed_on_retry).await {
        Ok(f) => (forced_class.or(f.class), Some(f.report)),
        Err(e) => {
            transcript.push(format!("report failed: {e}"));
            (forced_class.or((!passed).then_some(Class::Product)), None)
        }
    };
    // Restore phone settings before the device goes back to the pool (or the process exits).
    fp.close_all().await;
    CaseResult {
        quarantined: fp.quarantined(&case.id).is_some(),
        case: case.clone(),
        passed,
        class,
        notes: fastpilot_core::redact(&notes),
        turns,
        elapsed: started.elapsed(),
        models,
        usage,
        attempts: 1,
        transcript,
        report,
        devices: devices.iter().map(|d| d.id.clone()).collect(),
    }
}

/// Run a test case with the suite's retry budget and breaker: a failure is rerun while the
/// budget lasts; a rerun that passes makes the test flaky.
/// Lease devices, run, release; `avoid` steers a retry to other devices.
async fn attempt(case: &Case, opts: &Options, pool: &Pool, retry: bool, avoid: &[String]) -> CaseResult {
    let devices = match pool.lease(&case.platforms(), avoid).await {
        Ok(d) => d,
        Err(e) => return CaseResult::skipped(case, &format!("skipped: {e}")),
    };
    let result = run_case(case, opts, retry, &devices).await;
    pool.release(devices);
    result
}

pub async fn run_with_retries(
    case: &Case,
    opts: &Options,
    budget: &RetryBudget,
    breaker: &Breaker,
    pool: &Pool,
) -> CaseResult {
    if breaker.is_open() {
        return CaseResult::skipped(case, "skipped: the infra circuit breaker is open (consecutive infra failures)");
    }
    let mut result = attempt(case, opts, pool, false, &[]).await;
    let mut attempts = 1;
    while !result.passed && result.attempts > 0 && result.class != Some(Class::Agent) && budget.take() {
        attempts += 1;
        let mut again = attempt(case, opts, pool, true, &result.devices).await;
        let earlier = std::mem::take(&mut result.transcript);
        again.transcript.splice(0..0, earlier.into_iter().chain(std::iter::once(format!("— attempt {attempts} —"))));
        again.usage.input += result.usage.input;
        again.usage.output += result.usage.output;
        again.usage.cache_read += result.usage.cache_read;
        again.elapsed += result.elapsed;
        result = again;
    }
    result.attempts = attempts;
    breaker.record(if result.passed { None } else { result.class });
    result
}

pub fn summary_line(r: &CaseResult) -> String {
    let mark = match (r.passed, r.quarantined) {
        (true, _) if r.class == Some(Class::Flaky) => "~",
        (true, _) => "✓",
        (false, true) => "q",
        (false, false) => "✗",
    };
    let class = r.class.map(|c| format!(" [{}]", c.as_str())).unwrap_or_default();
    let tries = if r.attempts > 1 { format!(", {} attempts", r.attempts) } else { String::new() };
    let on = if r.devices.is_empty() { String::new() } else { format!(" on {}", r.devices.join("+")) };
    format!(
        "{mark} {} {:.1}s, {} turns{tries}{on}, {}{class}: {}",
        r.case.id,
        r.elapsed.as_secs_f64(),
        r.turns,
        r.models.join(" → "),
        clip(&r.notes, 200)
    )
}

/// The whole suite: exit status is failure when any test outside quarantine failed.
pub async fn run(target: &Path, opts: Options) -> Result<(), Box<dyn Error + Send + Sync>> {
    let cases = load(target)?;
    if cases.is_empty() {
        return Err(format!("no test cases (*.md) in {}", target.display()).into());
    }
    // The user's own Claude Code does the thinking: no API key, their login and plan.
    let version = tokio::process::Command::new(claude_bin()).arg("--version").output().await;
    if !version.as_ref().is_ok_and(|o| o.status.success()) {
        return Err(format!(
            "fastpilot run needs Claude Code installed and logged in (`{}` not found or not working; \
             install it from https://claude.com/claude-code, run `claude` once to log in, or set FASTPILOT_CLAUDE)",
            claude_bin()
        )
        .into());
    }
    let pool = Arc::new(Pool::discover().await);
    let order = shard_order(&cases, &fastpilot_memory::Memory::from_env());
    println!("{} tests, {} workers, {}", cases.len(), opts.workers, pool.summary());
    let started = Instant::now();
    let queue = Arc::new(Mutex::new(order.into_iter().collect::<std::collections::VecDeque<_>>()));
    let results = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::new((opts.clone(), RetryBudget::from_env(), Breaker::from_env()));
    let workers: Vec<_> = (0..opts.workers.clamp(1, cases.len()))
        .map(|_| {
            let (queue, results, shared, pool, cases) = (queue.clone(), results.clone(), shared.clone(), pool.clone(), cases.clone());
            tokio::spawn(async move {
                loop {
                    let next = queue.lock().unwrap_or_else(|p| p.into_inner()).pop_front();
                    let Some(i) = next else { break };
                    let (opts, budget, breaker) = &*shared;
                    let r = run_with_retries(&cases[i], opts, budget, breaker, &pool).await;
                    println!("{}", summary_line(&r));
                    results.lock().unwrap_or_else(|p| p.into_inner()).push((i, r));
                }
            })
        })
        .collect();
    for w in workers {
        w.await?;
    }
    let mut results = std::mem::take(&mut *results.lock().unwrap_or_else(|p| p.into_inner()));
    results.sort_by_key(|(i, _)| *i);
    let results: Vec<CaseResult> = results.into_iter().map(|(_, r)| r).collect();
    finish_suite(&results, started.elapsed(), &opts)
}

/// Longest first (LPT scheduling) by the last passing duration; tests never seen first, since
/// their cost is unknown.
pub fn shard_order(cases: &[Case], memory: &fastpilot_memory::Memory) -> Vec<usize> {
    let mut order: Vec<(usize, u64)> = cases
        .iter()
        .enumerate()
        .map(|(i, c)| (i, memory.load(&c.id).map(|m| m.duration_ms).filter(|d| *d > 0).unwrap_or(u64::MAX)))
        .collect();
    order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    order.into_iter().map(|(i, _)| i).collect()
}

pub fn finish_suite(results: &[CaseResult], elapsed: Duration, opts: &Options) -> Result<(), Box<dyn Error + Send + Sync>> {
    let failed = results.iter().filter(|r| !r.passed && !r.quarantined).count();
    let quarantined = results.iter().filter(|r| !r.passed && r.quarantined).count();
    let tokens: (u64, u64, u64) =
        results.iter().fold((0, 0, 0), |a, r| (a.0 + r.usage.input, a.1 + r.usage.output, a.2 + r.usage.cache_read));
    println!(
        "\n{} tests: {} passed, {failed} failed, {quarantined} quarantined failures, in {:.1}s; tokens in {} (cache read {}) out {}",
        results.len(),
        results.iter().filter(|r| r.passed).count(),
        elapsed.as_secs_f64(),
        tokens.0,
        tokens.2,
        tokens.1
    );
    if let Some(p) = &opts.junit {
        std::fs::write(p, crate::report::junit(results, elapsed))?;
        println!("junit: {}", p.display());
    }
    if let Some(p) = &opts.html {
        std::fs::write(p, crate::report::html(results, elapsed))?;
        println!("html: {}", p.display());
    }
    if failed > 0 {
        return Err(format!("{failed} test(s) failed").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platforms_and_longest_first_order() {
        let dir = std::env::temp_dir().join(format!("fastpilot-shard-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("cases")).expect("dir");
        for (id, extra) in [("a", "platform: web"), ("b", "sessions: web (x), android (y)"), ("c", "platform: ios")] {
            std::fs::write(dir.join(format!("cases/{id}.md")), format!("# {id}\nid: {id}\n{extra}\n- step\n")).expect("case");
        }
        let cases = load(&dir.join("cases")).expect("load");
        let ids: Vec<_> = cases.iter().map(|c| (c.id.as_str(), c.platforms())).collect();
        assert_eq!(ids, vec![("a", vec![Platform::Web]), ("b", vec![Platform::Web, Platform::Android]), ("c", vec![Platform::Ios])]);
        let mem = fastpilot_memory::Memory::new(dir.join("fp"), fastpilot_memory::Mode::ReadWrite);
        std::fs::create_dir_all(dir.join("fp/memory")).expect("dir");
        for (id, ms) in [("a", 1000), ("c", 9000)] {
            std::fs::write(dir.join(format!("fp/memory/{id}.json")), json!({ "test_id": id, "flow": "", "start": "", "screens": [], "duration_ms": ms, "pass_count": 1, "fail_count": 0, "last_pass": "", "failed_at": [] }).to_string()).expect("mem");
        }
        // b has no recorded duration (unknown: first), then c (9 s), then a (1 s).
        assert_eq!(shard_order(&cases, &mem), vec![1, 2, 0]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
