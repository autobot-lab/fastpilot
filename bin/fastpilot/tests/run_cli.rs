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
        .env("FASTPILOT_RETRY_BUDGET", "1");
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
    // 3 web tests × 3 tool calls × 300 ms run concurrently, not one after another.
    assert!(started.elapsed() < std::time::Duration::from_secs(15), "{:?}", started.elapsed());
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
