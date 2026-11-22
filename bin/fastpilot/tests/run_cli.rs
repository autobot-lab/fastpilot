//! `fastpilot run` end to end with a **stand-in Claude Code**: `tests/fake-claude/claude` takes
//! the same arguments as `claude -p`, connects to fastpilot over the MCP bridge and scripts the
//! agent's tool calls. So the bridge, the in-process tools, path memory, model selection,
//! retries and the JUnit/HTML output are exercised against real Chrome, with no model and no
//! API key. Skips without Chrome or python3.

use serde_json::{Value, json};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude/claude");

fn python() -> bool {
    std::process::Command::new("python3").arg("--version").output().is_ok_and(|o| o.status.success())
}

/// Invocations of the fake `claude`, oldest first.
fn calls(log: &std::path::Path) -> Vec<Value> {
    let mut files: Vec<_> = std::fs::read_dir(log).map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default();
    files.sort();
    files.iter().filter_map(|p| serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()).collect()
}

fn arg<'a>(call: &'a Value, name: &str) -> Option<&'a str> {
    let argv = call["argv"].as_array()?;
    let i = argv.iter().position(|a| a == name)?;
    argv.get(i + 1)?.as_str()
}

fn run_on(target: &str, dir: &std::path::Path, log: &std::path::Path, env: &[(&str, &str)], extra: &[&str]) -> (bool, String) {
    std::fs::create_dir_all(log).expect("log dir");
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    cmd.args(["run", target])
        .args(extra)
        .current_dir(ROOT)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .env("FASTPILOT_CLAUDE", FAKE)
        .env("FAKE_CLAUDE_LOG", log)
        .env("FASTPILOT_DIR", dir)
        .env("DEMO_PASS", "rahasia123")
        .env("FASTPILOT_VARS", "DEMO_PASS")
        .env("FASTPILOT_RETRY_BUDGET", "1")
        // Never a real adb server: the device pool must not probe the user's emulators (emulator-5554
        // is off limits); Android cases then take their "no device" path.
        .env("FASTPILOT_ADB_SERVER", "127.0.0.1:1")
        .env_remove("ANDROID_SERIAL")
        .env_remove("FASTPILOT_ANDROID_SERIAL");
    // The run's own settings file, not the user's ~/.fastpilot/config.toml (a model chosen there
    // changed which model these tests saw); a test that brings its own HOME uses that one's.
    if env.iter().any(|(k, _)| *k == "HOME") {
        // FastTest (v0.4): never a FASTPILOT_CONFIG from the shell running the tests.
        cmd.env_remove("FASTPILOT_CONFIG");
    } else {
        cmd.env("FASTPILOT_CONFIG", dir.join("config.toml"));
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn headless_run_drives_claude_code() {
    if !python() {
        return eprintln!("skipping: no python3");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-run-{}", std::process::id()));
    let log = dir.join("claude");
    let junit = dir.join("junit.xml");
    let html = dir.join("report.html");
    let (ok, out) = run_on("tests/login.md", &dir, &log, &[], &["--record", "--junit", junit.to_str().unwrap_or(""), "--html", html.to_str().unwrap_or("")]);
    if out.contains("Chrome not found") {
        return eprintln!("skipping: no Chrome");
    }
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.contains("✓ login-ok") && out.contains("claude-opus-5-5") && out.contains("1 passed, 0 failed"), "{out}");
    // V042 (QA Low): a web-only run names only what it uses, no Android or iPhone counts.
    assert!(out.contains("1 tests, 1 workers, Web · Chrome\n") && !out.contains("android") && !out.contains(" ios"), "{out}");
    {
        let c = calls(&log);
        assert_eq!(c.len(), 1, "{c:?}");
        let first = &c[0];
        // Claude Code is told to use only fastpilot, unattended.
        assert_eq!(arg(first, "--model"), Some("claude-opus-5-5"));
        assert_eq!(arg(first, "--tools"), Some(""));
        assert_eq!(arg(first, "--allowedTools"), Some("mcp__fastpilot"));
        assert_eq!(arg(first, "--permission-mode"), Some("dontAsk"));
        assert!(first["argv"].as_array().is_some_and(|a| a.iter().any(|x| x == "--strict-mcp-config")));
        let tools: Vec<&str> = first["tools"].as_array().map(|t| t.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        assert!(tools.contains(&"run_flow") && tools.contains(&"finish") && !tools.contains(&"report"), "{tools:?}");
        // The case text spells out the password: the model gets the variable name instead.
        let prompt = first["prompt"].as_str().unwrap_or("");
        assert!(prompt.contains("a@b.com / ${DEMO_PASS}") && !prompt.contains("rahasia123"), "{prompt}");
        let results: Vec<&str> = first["calls"].as_array().map(|c| c.iter().filter_map(|c| c["text"].as_str()).collect()).unwrap_or_default();
        assert!(results[0].contains("session w started"), "{results:?}");
        assert!(results[1].contains("ok 4/4 steps") && !results[1].contains("rahasia123"), "{results:?}");
    }
    let xml = std::fs::read_to_string(&junit).expect("junit");
    assert!(xml.contains("tests=\"1\" failures=\"0\"") && xml.contains("name=\"login-ok: Login works\""), "{xml}");
    let page = std::fs::read_to_string(&html).expect("html");
    assert!(page.contains("checked Budi on the dashboard") && page.contains("iframe class=\"run\"") && page.contains("sandbox=\"allow-scripts\" srcdoc="), "suite html (run report sandboxed)");
    // --record: the session's screencast is in the run directory and inlined in its report.
    let run_dir = std::fs::read_dir(dir.join("runs")).expect("runs").filter_map(|e| e.ok()).map(|e| e.path()).next().expect("run");
    assert!(run_dir.join("recording-w.html").exists(), "{}", run_dir.display());
    let report = std::fs::read_to_string(run_dir.join("report.html")).expect("report");
    assert!(report.contains("Recording [w]"));
    // E4: the run's steps are on the recording's timeline (one marker per step, flow 1's segment),
    // and the passing flow's final screen is in the report.
    assert!(report.matches("{&quot;flow&quot;:1,&quot;label&quot;:").count() >= 4, "step markers missing");
    assert!(report.contains("&quot;flow&quot;:1,&quot;from&quot;:") && report.contains("▶ flow "), "flow segment missing");
    assert!(run_dir.join("flow-1.jpg").exists() && report.contains("flow 1 (final)"), "final screen of the passing flow");

    // Second run: the passing flow is now path memory, so the fast model replays it.
    let log2 = dir.join("claude2");
    let (ok, out) = run_on("tests/login.md", &dir, &log2, &[], &[]);
    assert!(ok && out.contains("claude-haiku-5-5"), "{out}");
    let c = calls(&log2);
    assert_eq!(arg(&c[0], "--model"), Some("claude-haiku-5-5"));
    assert_eq!(arg(&c[0], "--effort"), Some("low"));
    assert!(c[0]["prompt"].as_str().is_some_and(|p| p.contains("path memory for login-ok")), "{}", c[0]["prompt"]);

    // A failing test: retried once (budget 1), exits non-zero, JUnit carries the failure class.
    let (ok, out) = run_on("tests/login.md", &dir, &dir.join("claude3"), &[("FAKE_CLAUDE_FAIL", "1")], &["--junit", junit.to_str().unwrap_or("")]);
    assert!(!ok && out.contains("✗ login-ok") && out.contains("2 attempts") && out.contains("[product]"), "{out}");
    let xml = std::fs::read_to_string(&junit).expect("junit");
    assert!(xml.contains("failures=\"1\"") && xml.contains("<failure type=\"product\""), "{xml}");

    // Claude Code logged out: an infra failure that says so, not a product bug.
    let (ok, out) = run_on("tests/login.md", &dir, &dir.join("claude4"), &[("FAKE_CLAUDE_NOLOGIN", "1")], &[]);
    assert!(!ok && out.contains("[infra]") && out.contains("Please run /login"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_without_claude_code_says_how_to_get_it() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["run", "tests/login.md"])
        .current_dir(ROOT)
        .env("FASTPILOT_CLAUDE", "/nonexistent/claude")
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && text.contains("fastpilot run needs Claude Code. Claude Code is not installed or does not start"), "{text}");
    assert!(!text.contains("API_KEY"), "{text}");
}

/// `--workers`: tests overlap, run longest-first from recorded durations, and a cross-platform
/// test gets a leased Android device in its session plan (when one is attached).
#[test]
fn parallel_suite_with_stand_in_claude() {
    if !python() {
        return eprintln!("skipping: no python3");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-par-{}", std::process::id()));
    let cases = dir.join("cases");
    std::fs::create_dir_all(&cases).expect("dir");
    let login = std::fs::read_to_string(format!("{ROOT}/tests/login.md")).expect("login case");
    for id in ["par-a", "par-b", "par-c"] {
        std::fs::write(cases.join(format!("{id}.md")), login.replace("id: login-ok", &format!("id: {id}"))).expect("case");
    }
    std::fs::copy(format!("{ROOT}/tests/login-cross-platform.md"), cases.join("par-x.md")).expect("cross");
    let mem = dir.join("fp/memory");
    std::fs::create_dir_all(&mem).expect("mem");
    for (id, ms) in [("par-a", 1000), ("par-b", 5000), ("par-c", 3000), ("login-cross-platform", 100)] {
        let m = serde_json::json!({ "test_id": id, "flow": "", "start": "", "screens": [], "duration_ms": ms, "pass_count": 1, "fail_count": 0, "last_pass": "", "failed_at": [] });
        std::fs::write(mem.join(format!("{id}.json")), m.to_string()).expect("mem");
    }
    let log = dir.join("claude");
    let started = std::time::Instant::now();
    let (ok, out) = run_on(cases.to_str().unwrap_or(""), &dir.join("fp"), &log, &[("FAKE_CLAUDE_DELAY_MS", "300")], &["--workers", "3"]);
    if out.contains("Chrome not found") {
        return eprintln!("skipping: no Chrome");
    }
    println!("{out}");
    // Without an Android device (CI) the cross-platform test fails as infra; the web tests must
    // still overlap and pass.
    let has_android = !out.contains("no healthy android device");
    if has_android {
        assert!(ok && out.contains("4 tests: 4 passed"), "{out}");
    } else {
        assert!(!ok && out.contains("4 tests: 3 passed, 1 failed"), "{out}");
        let cross = out.lines().find(|l| l.contains("login-cross-platform")).unwrap_or("");
        assert!(cross.contains("[infra]"), "{cross}");
    }
    let mut started_at: Vec<(f64, String)> = calls(&log)
        .iter()
        .filter_map(|c| Some((c["start"].as_f64()?, c["prompt"].as_str()?.split('`').nth(1)?.to_owned())))
        .collect();
    started_at.sort_by(|a, b| a.0.total_cmp(&b.0));
    let first: Vec<String> = started_at.into_iter().map(|(_, id)| id).collect();
    // Three workers start the three longest tests first (b 5 s, c 3 s, a 1 s), the 0.1 s one last.
    if has_android {
        assert_eq!(first.last().map(String::as_str), Some("login-cross-platform"), "{first:?}");
    }
    assert!(first.len() >= 3, "{first:?}");
    let mut head = first[..3].to_vec();
    head.sort();
    assert_eq!(head, ["par-a", "par-b", "par-c"], "{first:?}");
    // The 3 web tests run concurrently, not one after another: there is a moment when all three
    // are running (each one started before any of them ended). Measured on the stand-in's own
    // start and end times, so a busy machine makes them slower but not serial (a fixed 15 s limit
    // failed at 18.4 s under load in the v0.4.0 gate).
    let spans: Vec<(f64, f64)> = calls(&log)
        .iter()
        .filter(|c| c["prompt"].as_str().and_then(|p| p.split('`').nth(1)).is_some_and(|id| id.starts_with("par-")))
        .filter_map(|c| Some((c["start"].as_f64()?, c["end"].as_f64()?)))
        .collect();
    assert_eq!(spans.len(), 3, "one finished stand-in run per web test: {spans:?}");
    let last_start = spans.iter().map(|s| s.0).fold(f64::MIN, f64::max);
    let first_end = spans.iter().map(|s| s.1).fold(f64::MAX, f64::min);
    assert!(last_start < first_end, "all three web tests ran at the same time: {spans:?} ({:?} in all)", started.elapsed());
    if has_android {
        let cross = out.lines().find(|l| l.contains("login-cross-platform")).unwrap_or("");
        assert!(cross.contains("Session plan: web: your own Chrome; android:"), "{cross}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stand-in for the GitHub REST API (one PR, `o/r#5`): logs `METHOD path`, keeps the comments.
type Shared<T> = std::sync::Arc<std::sync::Mutex<Vec<T>>>;

fn fake_github() -> (u16, Shared<String>, Shared<Value>) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let comments = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let (log2, comments2) = (log.clone(), comments.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut r = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            let _ = r.read_line(&mut line);
            let mut it = line.split_whitespace();
            let (method, path) = (it.next().unwrap_or("").to_owned(), it.next().unwrap_or("").split('?').next().unwrap_or("").to_owned());
            let (mut len, mut authorized) = (0usize, false);
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                    break;
                }
                let lower = h.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                authorized |= lower.trim() == "authorization: bearer test-token-123";
            }
            let mut body = vec![0u8; len];
            let _ = r.read_exact(&mut body);
            let text = serde_json::from_slice::<Value>(&body).map(|v| v["body"].clone()).unwrap_or_default();
            log2.lock().expect("log").push(format!("{method} {path}"));
            let mut c = comments2.lock().expect("comments");
            let (status, reply) = match (authorized, method.as_str(), path.as_str()) {
                (false, ..) => ("401 Unauthorized", json!({})),
                // Like the Actions token: no /user, comments as github-actions[bot] (GitHub always
                // sends the author; fastpilot only edits a marked comment of its own account).
                (_, "GET", "/user") => ("403 Forbidden", json!({ "message": "Resource not accessible by integration" })),
                (_, "GET", "/repos/o/r/issues/5/comments") => ("200 OK", Value::Array(c.clone())),
                (_, "POST", "/repos/o/r/issues/5/comments") => {
                    c.push(json!({ "id": 77, "body": text, "user": { "login": "github-actions[bot]", "type": "Bot" } }));
                    ("201 Created", json!({ "id": 77 }))
                }
                (_, "PATCH", "/repos/o/r/issues/comments/77") => {
                    c[0]["body"] = text;
                    ("200 OK", json!({ "id": 77 }))
                }
                _ => ("404 Not Found", json!({})),
            };
            drop(c);
            let reply = reply.to_string();
            let mut s = stream;
            let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
        }
    });
    (port, log, comments)
}

/// C6: in GitHub Actions on a pull request, `--report-github` keeps one comment: "running" when the
/// suite starts, the results when it ends. The token only reaches the API, never the output.
#[test]
fn report_github_keeps_one_comment_on_the_pr() {
    let dir = std::env::temp_dir().join(format!("fastpilot-run-gh-{}", std::process::id()));
    let (port, log, comments) = fake_github();
    let api = format!("http://127.0.0.1:{port}");
    let env = [
        ("GITHUB_ACTIONS", "true"),
        ("GITHUB_REPOSITORY", "o/r"),
        ("GITHUB_REF", "refs/pull/5/merge"),
        ("GITHUB_API_URL", api.as_str()),
        ("GITHUB_TOKEN", "test-token-123"),
        ("GITHUB_EVENT_PATH", ""),
    ];
    let (ok, out) = run_on("tests/login.md", &dir, &dir.join("claude"), &env, &["--report-github"]);
    if out.contains("Chrome not found") {
        return eprintln!("skipping: no Chrome");
    }
    assert!(ok, "{out}");
    assert!(!out.contains("test-token-123"), "token in the output:\n{out}");
    assert!(out.contains("report-github: updated o/r#5"), "{out}");
    let calls = log.lock().expect("log").clone();
    assert_eq!(
        calls,
        ["GET /user", "GET /repos/o/r/issues/5/comments", "POST /repos/o/r/issues/5/comments", "GET /user", "GET /repos/o/r/issues/5/comments", "PATCH /repos/o/r/issues/comments/77"]
    );
    let all = comments.lock().expect("comments").clone();
    assert_eq!(all.len(), 1, "one comment: {all:?}");
    let body = all[0]["body"].as_str().unwrap_or("");
    assert!(body.contains("<!-- fastpilot-report -->") && body.contains("✅ 1 passed · 0 failed") && body.contains("login-ok — Login works"), "{body}");
    assert!(!body.contains("rahasia123"), "{body}");

    // Dry run outside Actions: the comment is printed, nothing is sent.
    log.lock().expect("log").clear();
    let (ok, out) = run_on("tests/login.md", &dir, &dir.join("claude2"), &[("GITHUB_API_URL", api.as_str())], &["--report-github=dry"]);
    assert!(ok, "{out}");
    assert!(out.contains("(dry run, not posted)") && out.contains("no pull request") && out.contains("✅ 1 passed"), "{out}");
    assert!(log.lock().expect("log").is_empty(), "dry run must not call the API");
    let _ = std::fs::remove_dir_all(dir);
}

/// A stand-in for an OpenAI-compatible chat-completions API that plays the agent: start the web
/// session, log in with one run_flow, finish. Logs every request body and its Authorization.
type Requests = std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>;

fn fake_openai() -> (u16, Requests) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut r = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            let _ = r.read_line(&mut line);
            let (mut len, mut auth) = (0usize, String::new());
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                    break;
                }
                let lower = h.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if lower.starts_with("authorization:") {
                    auth = h.trim().to_owned();
                }
            }
            let mut body = vec![0u8; len];
            let _ = r.read_exact(&mut body);
            let req: Value = serde_json::from_slice(&body).unwrap_or_default();
            log2.lock().expect("log").push((auth.clone(), req.clone()));
            let tool_call = |name: &str, args: Value| {
                json!({ "choices": [{ "message": { "role": "assistant", "content": null, "tool_calls": [{ "id": format!("call_{name}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] } }], "usage": { "prompt_tokens": 50, "completion_tokens": 10 } })
            };
            let (status, reply) = if !auth.ends_with("Bearer sk-test-provider-999") {
                ("401 Unauthorized", json!({ "error": { "message": "Incorrect API key provided" } }))
            } else if req.get("tools").is_none() {
                // config test-provider: a plain question.
                ("200 OK", json!({ "choices": [{ "message": { "role": "assistant", "content": "OK" } }] }))
            } else {
                let done = req["messages"].as_array().map(|m| m.iter().filter(|m| m["role"] == "tool").count()).unwrap_or(0);
                let flow = "- inputText: { into: Email, text: a@b.com }\n- inputText: { into: Password, text: \"${DEMO_PASS}\" }\n- tapOn: Masuk\n- assertVisible: Budi\n";
                let reply = match done {
                    0 => tool_call("session", json!({ "action": "start", "id": "w", "target": "examples/demo/login.html" })),
                    1 => tool_call("run_flow", json!({ "flow": flow })),
                    _ => tool_call("finish", json!({ "status": "pass", "notes": "checked Budi on the dashboard (provider)" })),
                };
                ("200 OK", reply)
            };
            let reply = reply.to_string();
            let mut s = stream;
            let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
        }
    });
    (port, log)
}

/// B2: with `provider` set, an OpenAI-compatible API drives the test through the same tools and
/// verdict; Claude Code is not started; the key only reaches the API.
#[test]
fn provider_drives_a_run_without_claude_code() {
    let dir = std::env::temp_dir().join(format!("fastpilot-run-provider-{}", std::process::id()));
    let (port, log) = fake_openai();
    let base = format!("http://127.0.0.1:{port}/v1");
    let env = [
        ("FASTPILOT_PROVIDER", "custom"),
        ("FASTPILOT_PROVIDER_BASE_URL", base.as_str()),
        ("FASTPILOT_PROVIDER_MODEL", "fake-model-1"),
        ("FASTPILOT_PROVIDER_API_KEY", "sk-test-provider-999"),
    ];
    let html = dir.join("report.html");
    let (ok, out) = run_on("tests/login.md", &dir, &dir.join("claude"), &env, &["--html", html.to_str().unwrap_or("")]);
    if out.contains("Chrome not found") {
        return eprintln!("skipping: no Chrome");
    }
    assert!(ok, "{out}");
    assert!(out.contains("driver: custom (fake-model-1)") && out.contains("✓ login-ok") && out.contains("1 passed, 0 failed"), "{out}");
    assert!(!out.contains("sk-test-provider-999"), "key in the output:\n{out}");
    assert!(calls(&dir.join("claude")).is_empty(), "Claude Code must not run");
    let requests = log.lock().expect("log").clone();
    assert_eq!(requests.len(), 3, "session, run_flow, finish: {} requests", requests.len());
    let first = &requests[0].1;
    assert_eq!(first["model"], "fake-model-1");
    assert!(first["messages"][0]["content"].as_str().is_some_and(|s| s.contains("fastpilot's QA agent")), "system prompt");
    let tools: Vec<&str> = first["tools"].as_array().map(|t| t.iter().filter_map(|t| t["function"]["name"].as_str()).collect()).unwrap_or_default();
    assert!(tools.contains(&"run_flow") && tools.contains(&"finish") && !tools.contains(&"report") && !tools.contains(&"config"), "{tools:?}");
    // The run_flow result went back to the model masked; the secret never left fastpilot.
    let last = requests[2].1.to_string();
    assert!(last.contains("ok 4/4 steps") && !last.contains("rahasia123"), "{last}");
    let page = std::fs::read_to_string(&html).expect("html");
    assert!(page.contains("checked Budi on the dashboard (provider)") && !page.contains("sk-test-provider-999"), "suite html");

    // `config test-provider`: one small request; a wrong key is refused without being repeated.
    let test = |key: &str| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .args(["config", "test-provider"])
            .env("FASTPILOT_PROVIDER", "custom")
            .env("FASTPILOT_PROVIDER_BASE_URL", &base)
            .env("FASTPILOT_PROVIDER_MODEL", "fake-model-1")
            .env("FASTPILOT_PROVIDER_API_KEY", key)
            .env("FASTPILOT_CONFIG", dir.join("none.toml"))
            .output()
            .expect("run");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    let (ok, out) = test("sk-test-provider-999");
    assert!(ok && out.contains("ok: custom at") && out.contains("\"OK\""), "{out}");
    let (ok, out) = test("sk-wrong-key-123");
    assert!(!ok && out.contains("401") && out.contains("Incorrect API key") && !out.contains("sk-wrong-key-123"), "{out}");
    let _ = std::fs::remove_dir_all(dir);
}

/// QA / Design (Medium): a stored key goes only to the server it was saved for. A custom key saved
/// for one server, with the run pointed at another: the run stops with the reason, the other
/// server gets nothing, and Claude Code is not started in its place.
#[test]
fn provider_key_is_not_sent_to_another_server() {
    let dir = std::env::temp_dir().join(format!("fastpilot-run-keyorigin-{}", std::process::id()));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("home");
    let (port, log) = fake_openai();
    let other = format!("http://127.0.0.1:{port}/v1");
    let fp = |args: &[&str], env: &[(&str, &str)], stdin: &str| {
        use std::io::Write;
        let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
        c.args(args).env("HOME", &home).env("FASTPILOT_KEYSTORE", "file").env_remove("FASTPILOT_PROVIDER_API_KEY").env_remove("FASTPILOT_CONFIG").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        for (k, v) in env {
            c.env(k, v);
        }
        let mut child = c.spawn().expect("spawn");
        child.stdin.take().expect("stdin").write_all(stdin.as_bytes()).expect("write");
        let out = child.wait_with_output().expect("run");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    // The key is saved for https://mine.example (the custom server at that time).
    assert!(fp(&["config", "set", "provider", "custom"], &[], "").0);
    assert!(fp(&["config", "set", "provider_base_url", "https://mine.example/v1"], &[], "").0);
    assert!(fp(&["config", "set", "provider_model", "fake-model-1"], &[], "").0);
    let (ok, out) = fp(&["config", "set-key", "custom"], &[], "sk-test-provider-999\n");
    assert!(ok && !out.contains("sk-test"), "{out}");
    // Pointed at another server (here through the environment): refused before any request.
    let env = [("HOME", home.to_str().unwrap_or("")), ("FASTPILOT_KEYSTORE", "file"), ("FASTPILOT_PROVIDER_BASE_URL", other.as_str())];
    let (ok, out) = run_on("tests/login.md", &dir, &dir.join("claude"), &env, &[]);
    assert!(!ok && out.contains("saved for https://mine.example, not for http://127.0.0.1:"), "{out}");
    assert!(!out.contains("sk-test-provider-999"), "{out}");
    assert!(log.lock().expect("log").is_empty(), "the other server must get nothing");
    assert!(calls(&dir.join("claude")).is_empty(), "no silent fall back to Claude Code");
    let _ = std::fs::remove_dir_all(dir);
}

/// V042 (DEMO_PASS): a test that names a secret this run does not have is a setup problem: it does
/// not start (no Claude Code, no retries), the line says "could not run [setup]", the summary says it
/// is not a pass with the exact next step, the secret's value is never printed, and the exit is 1.
#[test]
fn a_missing_secret_is_a_setup_problem_and_the_test_does_not_start() {
    if !python() {
        return eprintln!("skipping: no python3");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-run-setup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // Allowed, but no value: only the value is missing.
    let log = dir.join("claude-a");
    let (ok, out) = run_on("tests/login.md", &dir, &log, &[("DEMO_PASS", "")], &[]);
    assert!(!ok, "{out}");
    assert!(out.contains("✗ login-ok could not run [setup]: Could not run: needs the secret DEMO_PASS, and it is not set."), "{out}");
    assert!(out.contains("1 tests: 0 passed, 0 failed, 1 could not run, 0 quarantined failures"), "{out}");
    assert!(out.contains("Could not run. The test \"Login works\" needs the secret DEMO_PASS, and it is not set.\n0 of 1 tests ran. This is not a pass."), "{out}");
    assert!(out.contains("• Set it for this window:   export DEMO_PASS=…") && !out.contains("export FASTPILOT_VARS"), "{out}");
    // V051: the plugin case (no .mcp.json; the value comes from Claude Code's environment).
    assert!(out.contains("With the Claude Code plugin: add the name in Settings → Secrets, and export the value before you start Claude Code."), "{out}");
    assert!(out.contains("1 test could not start: a secret it needs is not set") && !out.contains("attempts"), "{out}");
    assert!(calls(&log).is_empty(), "Claude Code never started for a test that cannot run");
    // Neither allowed nor set: both go in the export line, and the dashboard way is named.
    let log = dir.join("claude-b");
    let (ok, out) = run_on("tests/login.md", &dir, &log, &[("DEMO_PASS", ""), ("FASTPILOT_VARS", "")], &[]);
    assert!(!ok && out.contains("export FASTPILOT_VARS=DEMO_PASS DEMO_PASS=…") && out.contains("Settings → Secrets"), "{out}");
    assert!(calls(&log).is_empty());
    // A value without its name in FASTPILOT_VARS: the value is never printed.
    let log = dir.join("claude-c");
    let (ok, out) = run_on("tests/login.md", &dir, &log, &[("FASTPILOT_VARS", "")], &[]);
    assert!(!ok && out.contains("its name is not in FASTPILOT_VARS") && out.contains("export FASTPILOT_VARS=DEMO_PASS\n"), "{out}");
    assert!(!out.contains("rahasia123"), "{out}");
    // In a demo folder: the value is in .mcp.json, which `fastpilot run` does not read; it says so.
    let proj = dir.join("proj");
    std::fs::create_dir_all(proj.join("tests")).expect("proj");
    std::fs::copy(format!("{ROOT}/tests/login.md"), proj.join("tests/login.md")).expect("copy");
    std::fs::write(proj.join(".mcp.json"), json!({ "mcpServers": { "fastpilot": { "command": "fastpilot", "args": ["mcp"], "env": { "FASTPILOT_VARS": "DEMO_PASS", "DEMO_PASS": "rahasia123" } } } }).to_string()).expect("mcp");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["run", "tests/login.md"])
        .current_dir(&proj)
        .env("HOME", &dir)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("DEMO_PASS")
        .env_remove("FASTPILOT_VARS")
        .env("FASTPILOT_CLAUDE", FAKE)
        .env("FAKE_CLAUDE_LOG", dir.join("claude-d"))
        .env("FASTPILOT_DIR", proj.join(".fastpilot"))
        .output()
        .expect("run");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("This folder's .mcp.json has DEMO_PASS for Claude Code and Chat; fastpilot run reads only the environment.") && !text.contains("rahasia123"), "{text}");
    assert!(calls(&dir.join("claude-d")).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// PF2 (v0.6): `--changed` runs the tests related to the code changes first, each with a plain
/// reason, lists the others as "Not run (N)" with how to run them, and remembers which tests failed
/// for those files. Without git it says so and runs everything.
#[test]
fn changed_runs_the_related_tests_first_with_a_reason() {
    if !python() {
        return eprintln!("skipping: no python3");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-run-changed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let proj = dir.join("shop");
    std::fs::create_dir_all(proj.join("tests")).expect("tests");
    std::fs::create_dir_all(proj.join("examples/demo")).expect("demo");
    std::fs::copy(format!("{ROOT}/examples/demo/login.html"), proj.join("examples/demo/login.html")).expect("page");
    std::fs::copy(format!("{ROOT}/tests/login.md"), proj.join("tests/login.md")).expect("login test");
    std::fs::write(proj.join("tests/search.md"), "# Search for shoes\nid: search-shoes\nplatform: web\ntarget: examples/demo/widgets.html\n- Type shoes in the search field\n- Check that results show\n").expect("search test");
    let run = |args: &[&str], log: &str| {
        std::fs::create_dir_all(dir.join(log)).expect("log dir");
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .arg("run")
            .args(args)
            .current_dir(&proj)
            .env("HOME", &dir)
            .env_remove("FASTPILOT_CONFIG")
            .env("FASTPILOT_CLAUDE", FAKE)
            .env("FAKE_CLAUDE_LOG", dir.join(log))
            .env("FASTPILOT_DIR", proj.join(".fastpilot"))
            .env("DEMO_PASS", "rahasia123")
            .env("FASTPILOT_VARS", "DEMO_PASS")
            .env("FASTPILOT_RETRY_BUDGET", "0")
            .output()
            .expect("run");
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
    };
    // Not a git project yet: said plainly, and everything runs.
    let out = run(&["tests", "--changed"], "claude-nogit");
    if out.contains("Chrome not found") {
        return eprintln!("skipping: no Chrome");
    }
    assert!(out.contains("I cannot see what you changed because this folder is not tracked by git. Running all 2 tests instead."), "{out}");
    assert_eq!(calls(&dir.join("claude-nogit")).len(), 2, "{out}");
    // A git project on a branch that changed the login page.
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git").args(args).current_dir(&proj).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").output().expect("git").status.success();
        assert!(ok, "git {args:?}");
    };
    std::fs::write(proj.join(".gitignore"), ".fastpilot/\n").expect("ignore");
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "shop"]);
    let out = run(&["tests", "--changed"], "claude-none");
    assert!(out.contains("I did not find any changes. Running all 2 tests instead."), "{out}");
    git(&["checkout", "-q", "-b", "fix-login"]);
    let page = std::fs::read_to_string(proj.join("examples/demo/login.html")).expect("page");
    std::fs::write(proj.join("examples/demo/login.html"), page.replace("Masuk ke Akun", "Masuk ke akun anda")).expect("change");
    let out = run(&["tests", "--changed"], "claude-changed");
    assert!(out.contains("Tests chosen from your changes · 1 of 2 tests"), "{out}");
    assert!(out.contains("Changed: 1 file (login page) · compared with main") && out.contains("Files: examples/demo/login.html"), "{out}");
    assert!(out.contains("✓ login-ok") && out.contains("    chosen because: you changed the login page"), "{out}");
    assert!(out.contains("Not run (1): they did not look related to your changes. A related-test run is not a full regression.") && out.contains("Run them too:  fastpilot run tests --changed --all-after"), "{out}");
    assert!(out.contains("1 tests: 1 passed"), "the one not run is not counted as passed: {out}");
    let c = calls(&dir.join("claude-changed"));
    assert_eq!(c.len(), 1, "only the related test ran");
    assert!(c[0]["prompt"].as_str().is_some_and(|p| p.contains("login-ok")), "{c:?}");
    // --all-after: the related one first, then the rest.
    let out = run(&["tests", "--changed", "--all-after"], "claude-all");
    assert!(!out.contains("Not run (") && out.contains("2 tests:"), "{out}");
    let c = calls(&dir.join("claude-all"));
    assert_eq!(c.len(), 2);
    assert!(c[0]["prompt"].as_str().is_some_and(|p| p.contains("login-ok")), "related first: {c:?}");
    // A base git does not know, and --all-after alone.
    let out = run(&["tests", "--changed", "no-such-branch"], "claude-bad");
    assert!(out.contains("I cannot compare with \"no-such-branch\": git does not know it. Running all 2 tests instead."), "{out}");
    let out = run(&["tests", "--all-after"], "claude-x");
    assert!(out.contains("--all-after goes with --changed"), "{out}");
    let _ = std::fs::remove_dir_all(dir);
}

/// PF6: every run leaves `outcome.json` for the flaky test doctor: pass or fail, the failed step and
/// how, where it ran and when.
#[test]
fn a_run_leaves_its_outcome_for_the_doctor() {
    if !python() {
        return eprintln!("skipping: no python3");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-run-outcome-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (ok, out) = run_on("tests/login.md", &dir, &dir.join("claude"), &[("FAKE_CLAUDE_FAIL", "1"), ("FASTPILOT_RETRY_BUDGET", "0")], &[]);
    if out.contains("Chrome not found") {
        return eprintln!("skipping: no Chrome");
    }
    assert!(!ok, "{out}");
    let runs: Vec<std::path::PathBuf> = std::fs::read_dir(dir.join("runs")).expect("runs").flatten().map(|e| e.path().join("outcome.json")).filter(|p| p.exists()).collect();
    assert_eq!(runs.len(), 1, "{out}");
    let o: Value = serde_json::from_str(&std::fs::read_to_string(&runs[0]).expect("outcome")).expect("json");
    assert_eq!((o["test_id"].as_str(), o["passed"].as_bool(), o["passed_on_retry"].as_bool()), (Some("login-ok"), Some(false), Some(false)), "{o}");
    // QA v0.6 Low 5: the same "clean" the Memory table counts.
    assert_eq!(o["clean"].as_bool(), Some(false), "{o}");
    assert!(o["failed_step"].as_str().is_some_and(|s| s.starts_with("assertVisible")), "{o}");
    assert_eq!(o["platforms"], json!(["web"]), "{o}");
    assert!(o["started_ms"].as_u64().unwrap_or(0) > 0 && o["finished_ms"].as_u64() >= o["started_ms"].as_u64(), "{o}");
    assert!(!o.to_string().contains("rahasia123"), "no secret values");
    let _ = std::fs::remove_dir_all(dir);
}
