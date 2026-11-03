//! `fastpilot run <tests dir|file>`: headless CI runner. Each natural-language test case is
//! driven by Claude over the same tools the MCP server serves, in-process (no MCP client), with
//! path-memory hints, retries within a budget, an infra circuit breaker, and JUnit/HTML output.
//!
//! Models (PRD §12.5): a test with a path-memory hint starts on the fast model to replay it;
//! a failed replay, or a test without a hint, runs on the larger model. `--model` pins one.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fastpilot_llm::{Claude, MODEL_EXPLORE, MODEL_REPLAY};
use fastpilot_mcp::FastPilot;
use fastpilot_memory::{Breaker, Class, RetryBudget};
use fastpilot_web::LaunchOptions;
use serde_json::{Value, json};

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
notes saying what was checked or what went wrong (expected vs actual).";

const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

#[derive(Clone)]
pub struct Options {
    pub model: Option<String>,
    pub max_steps: usize,
    pub junit: Option<PathBuf>,
    pub html: Option<PathBuf>,
    pub web: LaunchOptions,
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
        }
    }
}

fn finish_tool() -> Value {
    json!({
        "name": "finish",
        "description": "Call exactly once when the test case is done: pass only if every expectation was verified.",
        "strict": true,
        "input_schema": {
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

/// MCP content blocks → Claude tool_result content.
fn to_claude(content: Vec<Value>) -> Vec<Value> {
    content
        .into_iter()
        .map(|c| match c["type"].as_str() {
            Some("image") => json!({
                "type": "image",
                "source": { "type": "base64", "media_type": c["mimeType"].as_str().unwrap_or("image/jpeg"), "data": c["data"] }
            }),
            _ => json!({ "type": "text", "text": c["text"].as_str().unwrap_or("") }),
        })
        .collect()
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_owned() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

/// Larger models take the server-side refusal fallback (Haiku has none).
fn request(model: &str, tools: &[Value], messages: &[Value]) -> (Value, Vec<&'static str>) {
    let replay = model.contains("haiku");
    let mut body = json!({
        "model": model,
        "max_tokens": 16000,
        "system": SYSTEM,
        "tools": tools,
        "messages": messages,
        // Auto-caching: the stable prefix (tools, system, earlier turns) is reused every turn.
        "cache_control": { "type": "ephemeral" },
        "output_config": { "effort": if replay { "low" } else { "medium" } },
    });
    let mut betas = Vec::new();
    if !replay {
        body["fallbacks"] = json!("default");
        betas.push(FALLBACK_BETA);
    }
    (body, betas)
}

/// Drive one test case to a verdict.
async fn run_case(case: &Case, claude: &Claude, opts: &Options, passed_on_retry: bool) -> CaseResult {
    let started = Instant::now();
    let fp = FastPilot::new(opts.web.clone());
    let hint = fp.hint(&case.id);
    let mut model = match (&opts.model, &hint) {
        (Some(m), _) => m.clone(),
        (None, Some(_)) => MODEL_REPLAY.to_owned(),
        (None, None) => MODEL_EXPLORE.to_owned(),
    };
    let mut models = vec![model.clone()];
    let mut tools: Vec<Value> = FastPilot::api_tools().into_iter().filter(|t| t["name"] != "report").collect();
    tools.push(finish_tool());
    let intro = format!(
        "Test case `{}` (file {}):\n\n{}\n\n{}",
        case.id,
        case.path.display(),
        fastpilot_core::to_vars(case.text.trim()),
        hint.as_deref().unwrap_or("No path memory for this test yet: explore.")
    );
    let mut messages = vec![json!({ "role": "user", "content": intro })];
    let mut usage = Usage::default();
    let mut transcript = Vec::new();
    let mut verdict: Option<(bool, String)> = None;
    let mut turns = 0;
    let mut nudges = 0;
    let mut forced_class = None;

    while turns < opts.max_steps && verdict.is_none() {
        turns += 1;
        let (body, betas) = request(&model, &tools, &messages);
        let resp = match claude.messages(&body, &betas).await {
            Ok(r) => r,
            Err(e) => {
                transcript.push(format!("model error: {e}"));
                forced_class = Some(Class::Infra);
                verdict = Some((false, format!("model request failed: {e}")));
                break;
            }
        };
        let u = &resp["usage"];
        usage.input += u["input_tokens"].as_u64().unwrap_or(0);
        usage.output += u["output_tokens"].as_u64().unwrap_or(0);
        usage.cache_read += u["cache_read_input_tokens"].as_u64().unwrap_or(0);
        let stop = resp["stop_reason"].as_str().unwrap_or("").to_owned();
        let content = resp["content"].as_array().cloned().unwrap_or_default();
        if stop == "refusal" {
            let why = resp["stop_details"]["category"].as_str().unwrap_or("unspecified");
            forced_class = Some(Class::Agent);
            verdict = Some((false, format!("the model declined the request (refusal: {why})")));
            break;
        }
        // The whole assistant turn goes back, thinking blocks included.
        messages.push(json!({ "role": "assistant", "content": content.clone() }));
        let calls: Vec<&Value> = content.iter().filter(|b| b["type"] == "tool_use").collect();
        if stop == "max_tokens" && !calls.is_empty() {
            forced_class = Some(Class::Agent);
            verdict = Some((false, "model output was cut off at max_tokens mid tool call".into()));
            break;
        }
        if calls.is_empty() {
            // `auto` does not guarantee a tool call: ask for the verdict, a bounded number of times.
            nudges += 1;
            if nudges > 2 {
                break;
            }
            messages.push(json!({ "role": "user", "content": "Continue the test, then call finish with pass or fail." }));
            continue;
        }
        let mut results = Vec::new();
        let mut replay_failed = false;
        for call in calls {
            let id = call["id"].clone();
            let name = call["name"].as_str().unwrap_or("");
            let mut input = call["input"].clone();
            if name == "finish" {
                let pass = input["status"] == "pass";
                let notes = input["notes"].as_str().unwrap_or("").to_owned();
                transcript.push(format!("finish {}: {}", if pass { "pass" } else { "fail" }, fastpilot_core::redact(&notes)));
                verdict = Some((pass, notes));
                results.push(json!({ "type": "tool_result", "tool_use_id": id, "content": "recorded" }));
                continue;
            }
            // Evidence always lands in this test's run, whether or not the model passed test_id.
            let start = name == "session" && input["action"] == "start";
            if (start || name == "run_flow" || name == "assert") && input.get("test_id").is_none_or(Value::is_null) {
                input["test_id"] = json!(case.id);
            }
            let (ok, out) = fp.call_json(name, input.clone()).await;
            let text: String = out.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");
            transcript.push(format!("{} {} {}\n{}", if ok { "✓" } else { "✗" }, name, clip(&input.to_string(), 300), clip(&text, 1500)));
            replay_failed |= !ok && name == "run_flow";
            results.push(json!({ "type": "tool_result", "tool_use_id": id, "content": to_claude(out), "is_error": !ok }));
        }
        messages.push(json!({ "role": "user", "content": results }));
        // The hint no longer applies: hand over to the larger model.
        if replay_failed && opts.model.is_none() && model == MODEL_REPLAY {
            model = MODEL_EXPLORE.to_owned();
            models.push(model.clone());
            transcript.push(format!("replay failed: switching to {model}"));
        }
    }

    let (passed, notes) = verdict.unwrap_or_else(|| {
        forced_class = Some(Class::Agent);
        (false, format!("the agent did not call finish within {} turns", opts.max_steps))
    });
    let (class, report) = match fp.finish_test(&case.id, passed, &notes, passed_on_retry).await {
        Ok(f) => (forced_class.or(f.class), Some(f.report)),
        Err(e) => {
            transcript.push(format!("report failed: {e}"));
            (forced_class.or((!passed).then_some(Class::Product)), None)
        }
    };
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
    }
}

/// Run a test case with the suite's retry budget and breaker: a failure is rerun while the
/// budget lasts; a rerun that passes makes the test flaky.
pub async fn run_with_retries(case: &Case, claude: &Claude, opts: &Options, budget: &RetryBudget, breaker: &Breaker) -> CaseResult {
    if breaker.is_open() {
        return CaseResult::skipped(case, "skipped: the infra circuit breaker is open (consecutive infra failures)");
    }
    let mut result = run_case(case, claude, opts, false).await;
    let mut attempts = 1;
    while !result.passed && result.class != Some(Class::Agent) && budget.take() {
        attempts += 1;
        let mut again = run_case(case, claude, opts, true).await;
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
    format!(
        "{mark} {} {:.1}s, {} turns{tries}, {}{class}: {}",
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
    let claude = Claude::from_env().ok_or(
        "fastpilot run needs Claude credentials: set ANTHROPIC_API_KEY (or ANTHROPIC_AUTH_TOKEN; ANTHROPIC_BASE_URL for a proxy)",
    )?;
    let budget = RetryBudget::from_env();
    let breaker = Breaker::from_env();
    let started = Instant::now();
    let mut results = Vec::new();
    for case in &cases {
        let r = run_with_retries(case, &claude, &opts, &budget, &breaker).await;
        println!("{}", summary_line(&r));
        results.push(r);
    }
    finish_suite(&results, started.elapsed(), &opts)
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
