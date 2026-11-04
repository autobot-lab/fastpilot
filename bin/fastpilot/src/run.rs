//! `fastpilot run <tests dir|file>`: headless CI runner. Each natural-language test case is
//! driven by Claude over the same tools the MCP server serves, in-process (no MCP client), with
//! path-memory hints, retries within a budget, an infra circuit breaker, and JUnit/HTML output.
//!
//! Models (PRD §12.5): a test with a path-memory hint starts on the fast model to replay it;
//! a failed replay, or a test without a hint, runs on the larger model. `--model` pins one.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use std::sync::{Arc, Mutex};

use fastpilot_core::Platform;
use fastpilot_llm::{Claude, MODEL_EXPLORE, MODEL_REPLAY};
use fastpilot_mcp::{Devices, FastPilot};
use fastpilot_memory::{Breaker, Class, RetryBudget};
use fastpilot_web::LaunchOptions;
use serde_json::{Value, json};

use crate::pool::{Device, Pool};

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
async fn run_case(case: &Case, claude: &Claude, opts: &Options, passed_on_retry: bool, devices: &[Device]) -> CaseResult {
    let started = Instant::now();
    let pick = |p: Platform| devices.iter().find(|d| d.platform == p).map(|d| d.id.clone());
    let fp = FastPilot::with_devices(opts.web.clone(), Devices { android: pick(Platform::Android), ios: pick(Platform::Ios) });
    fp.set_record(opts.record);
    let hint = fp.hint(&case.id);
    let mut model = match (&opts.model, &hint) {
        (Some(m), _) => m.clone(),
        (None, Some(_)) => MODEL_REPLAY.to_owned(),
        (None, None) => MODEL_EXPLORE.to_owned(),
    };
    let mut models = vec![model.clone()];
    let mut tools: Vec<Value> = FastPilot::api_tools().into_iter().filter(|t| t["name"] != "report").collect();
    tools.push(finish_tool());
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
async fn attempt(case: &Case, claude: &Claude, opts: &Options, pool: &Pool, retry: bool, avoid: &[String]) -> CaseResult {
    let devices = match pool.lease(&case.platforms(), avoid).await {
        Ok(d) => d,
        Err(e) => return CaseResult::skipped(case, &format!("skipped: {e}")),
    };
    let result = run_case(case, claude, opts, retry, &devices).await;
    pool.release(devices);
    result
}

pub async fn run_with_retries(
    case: &Case,
    claude: &Claude,
    opts: &Options,
    budget: &RetryBudget,
    breaker: &Breaker,
    pool: &Pool,
) -> CaseResult {
    if breaker.is_open() {
        return CaseResult::skipped(case, "skipped: the infra circuit breaker is open (consecutive infra failures)");
    }
    let mut result = attempt(case, claude, opts, pool, false, &[]).await;
    let mut attempts = 1;
    while !result.passed && result.attempts > 0 && result.class != Some(Class::Agent) && budget.take() {
        attempts += 1;
        let mut again = attempt(case, claude, opts, pool, true, &result.devices).await;
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
    let claude = Claude::from_env().ok_or(
        "fastpilot run needs Claude credentials: set ANTHROPIC_API_KEY (or ANTHROPIC_AUTH_TOKEN; ANTHROPIC_BASE_URL for a proxy)",
    )?;
    let pool = Arc::new(Pool::discover().await);
    let order = shard_order(&cases, &fastpilot_memory::Memory::from_env());
    println!("{} tests, {} workers, {}", cases.len(), opts.workers, pool.summary());
    let started = Instant::now();
    let queue = Arc::new(Mutex::new(order.into_iter().collect::<std::collections::VecDeque<_>>()));
    let results = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::new((claude, opts.clone(), RetryBudget::from_env(), Breaker::from_env()));
    let workers: Vec<_> = (0..opts.workers.clamp(1, cases.len()))
        .map(|_| {
            let (queue, results, shared, pool, cases) = (queue.clone(), results.clone(), shared.clone(), pool.clone(), cases.clone());
            tokio::spawn(async move {
                loop {
                    let next = queue.lock().unwrap_or_else(|p| p.into_inner()).pop_front();
                    let Some(i) = next else { break };
                    let (claude, opts, budget, breaker) = &*shared;
                    let r = run_with_retries(&cases[i], claude, opts, budget, breaker, &pool).await;
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
