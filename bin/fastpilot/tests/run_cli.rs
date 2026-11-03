//! `fastpilot run` end to end with a **stubbed model**: a local server speaks the Messages API
//! (`ANTHROPIC_BASE_URL`) and scripts the agent's tool calls, so the loop, the in-process tools,
//! path memory, model selection, retries and the JUnit/HTML output are exercised against real
//! Chrome without an API key. Skips without Chrome.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

async fn read_request(s: &mut tokio::net::TcpStream) -> Option<(String, Value)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 65536];
    loop {
        let n = s.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf).into_owned();
        if let Some(i) = text.find("\r\n\r\n") {
            let head = text[..i].to_ascii_lowercase();
            let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
            if buf.len() >= i + 4 + len {
                let body = serde_json::from_slice(&buf[i + 4..i + 4 + len]).ok()?;
                return Some((head, body));
            }
        }
    }
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({ "id": "msg", "type": "message", "role": "assistant", "stop_reason": "tool_use",
            "usage": { "input_tokens": 100, "output_tokens": 20, "cache_read_input_tokens": 50 },
            "content": [{ "type": "text", "text": "ok" }, { "type": "tool_use", "id": id, "name": name, "input": input }] })
}

/// The scripted agent: its next move depends on how many tool results it has seen. A test
/// whose session plan names a device only reports what it was given.
fn agent(body: &Value, fail: bool) -> Value {
    let results = body["messages"].as_array().map_or(0, |m| m.iter().filter(|m| m["role"] == "user").count()) - 1;
    let intro = body["messages"][0]["content"].as_str().unwrap_or("");
    if let Some(plan) = intro.lines().find(|l| l.starts_with("Session plan:")).filter(|p| p.contains("android")) {
        return tool_use("t9", "finish", json!({ "status": "pass", "notes": plan }));
    }
    match results {
        0 => tool_use("t1", "session", json!({ "action": "start", "id": "w", "target": "examples/demo/login.html" })),
        1 => tool_use("t2", "run_flow", json!({ "flow": format!(
            "- inputText: {{ into: Email, text: a@b.com }}\n- inputText: {{ into: Password, text: \"${{DEMO_PASS}}\" }}\n- tapOn: Masuk\n- assertVisible: {}\n",
            if fail { "{ text: Nope, timeout: 500 }" } else { "Budi" }) })),
        _ => {
            let last = body["messages"].as_array().and_then(|m| m.last()).cloned().unwrap_or_default();
            let failed = last["content"][0]["is_error"] == true;
            tool_use("t3", "finish", json!({ "status": if failed { "fail" } else { "pass" }, "notes": "checked Budi on the dashboard" }))
        }
    }
}

async fn fake_claude(seen: Arc<Mutex<Vec<(String, Value)>>>, fail: bool) -> u16 {
    fake_claude_delay(seen, fail, 0).await
}

/// Each reply waits `delay_ms`, so parallel tests visibly overlap.
async fn fake_claude_delay(seen: Arc<Mutex<Vec<(String, Value)>>>, fail: bool, delay_ms: u64) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                while let Some((head, body)) = read_request(&mut s).await {
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    let reply = agent(&body, fail).to_string();
                    seen.lock().unwrap().push((head, body));
                    let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{reply}", reply.len());
                    if s.write_all(resp.as_bytes()).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

fn run(port: u16, dir: &std::path::Path, extra: &[&str]) -> (bool, String) {
    run_on("tests/login.md", port, dir, extra)
}

fn run_on(target: &str, port: u16, dir: &std::path::Path, extra: &[&str]) -> (bool, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["run", target])
        .args(extra)
        .current_dir(ROOT)
        .env("ANTHROPIC_API_KEY", "test-key")
        .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}"))
        .env("FASTPILOT_DIR", dir)
        .env("DEMO_PASS", "rahasia123")
        .env("FASTPILOT_VARS", "DEMO_PASS")
        .env("FASTPILOT_RETRY_BUDGET", "1")
        .output()
        .expect("run");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

// Multi-threaded: the CLI is awaited synchronously while the fake model serves on the runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headless_run_with_stubbed_model() {
    let dir = std::env::temp_dir().join(format!("fastpilot-run-{}", std::process::id()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let port = fake_claude(seen.clone(), false).await;

    let junit = dir.join("junit.xml");
    let html = dir.join("report.html");
    let (ok, out) = run(port, &dir, &["--record", "--junit", junit.to_str().unwrap_or(""), "--html", html.to_str().unwrap_or("")]);
    if out.contains("Chrome not found") {
        return eprintln!("skipping: no Chrome");
    }
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.contains("✓ login-ok") && out.contains("claude-opus-5-5") && out.contains("1 passed, 0 failed"), "{out}");
    {
        let seen = seen.lock().unwrap();
        let (head, first) = &seen[0];
        assert!(head.contains("x-api-key: test-key") && head.contains("anthropic-version: 2026-10-21"), "{head}");
        assert!(head.contains("anthropic-beta: server-side-fallback-2026-07-01") && first["fallbacks"] == "default", "{head}");
        assert_eq!(first["model"], "claude-opus-5-5");
        assert_eq!(first["cache_control"]["type"], "ephemeral");
        let tools: Vec<&str> = first["tools"].as_array().map(|t| t.iter().filter_map(|t| t["name"].as_str()).collect()).unwrap_or_default();
        assert!(tools.contains(&"run_flow") && tools.contains(&"finish") && !tools.contains(&"report"), "{tools:?}");
        // Tool results go back as one user message with the matching tool_use_id; secrets stay masked.
        let second = &seen[1].1["messages"];
        assert_eq!(second[2]["content"][0]["tool_use_id"], "t1");
        assert!(second[2]["content"][0]["content"][0]["text"].as_str().is_some_and(|t| t.contains("session w started")));
        let third = seen[2].1.to_string();
        assert!(third.contains("ok 4/4 steps") && !third.contains("rahasia123"), "{third}");
        // The case text spells out the password: the model gets the variable name instead.
        assert!(third.contains("a@b.com / ${DEMO_PASS}"), "{third}");
    }
    let xml = std::fs::read_to_string(&junit).expect("junit");
    assert!(xml.contains("tests=\"1\" failures=\"0\"") && xml.contains("name=\"login-ok: Login berhasil\""), "{xml}");
    let page = std::fs::read_to_string(&html).expect("html");
    assert!(page.contains("checked Budi on the dashboard") && page.contains("iframe class=\"run\""), "suite html");
    // --record: the session's screencast is in the run directory and inlined in its report.
    let run_dir = std::fs::read_dir(dir.join("runs")).expect("runs").filter_map(|e| e.ok()).map(|e| e.path()).next().expect("run");
    assert!(run_dir.join("recording-w.html").exists(), "{}", run_dir.display());
    assert!(std::fs::read_to_string(run_dir.join("report.html")).expect("report").contains("Recording [w]"));

    // Second run: the passing flow is now path memory, so the fast model replays it.
    seen.lock().unwrap().clear();
    let (ok, out) = run(port, &dir, &[]);
    assert!(ok && out.contains("claude-haiku-5-5"), "{out}");
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].1["model"], "claude-haiku-5-5");
        assert!(seen[0].1.get("fallbacks").is_none() && !seen[0].0.contains("anthropic-beta"), "{}", seen[0].0);
        assert!(seen[0].1["messages"][0]["content"].as_str().is_some_and(|c| c.contains("path memory for login-ok")));
    }

    // A failing test: retried once (budget 1), exits non-zero, JUnit carries the failure class.
    let port = fake_claude(Arc::new(Mutex::new(Vec::new())), true).await;
    let (ok, out) = run(port, &dir, &["--junit", junit.to_str().unwrap_or("")]);
    assert!(!ok && out.contains("✗ login-ok") && out.contains("2 attempts") && out.contains("[product]"), "{out}");
    let xml = std::fs::read_to_string(&junit).expect("junit");
    assert!(xml.contains("failures=\"1\"") && xml.contains("<failure type=\"product\""), "{xml}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--workers`: tests overlap, run longest-first from recorded durations, and a cross-platform
/// test gets a leased Android device in its session plan (when one is attached).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_suite_with_stubbed_model() {
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
        let m = json!({ "test_id": id, "flow": "", "start": "", "screens": [], "duration_ms": ms, "pass_count": 1, "fail_count": 0, "last_pass": "", "failed_at": [] });
        std::fs::write(mem.join(format!("{id}.json")), m.to_string()).expect("mem");
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let port = fake_claude_delay(seen.clone(), false, 300).await;
    let started = std::time::Instant::now();
    let (ok, out) = run_on(cases.to_str().unwrap_or(""), port, &dir.join("fp"), &["--workers", "3"]);
    if out.contains("Chrome not found") {
        return eprintln!("skipping: no Chrome");
    }
    println!("{out}");
    assert!(ok && out.contains("4 tests: 4 passed"), "{out}");
    let seen = seen.lock().unwrap();
    let first: Vec<String> = seen
        .iter()
        .filter(|(_, b)| b["messages"].as_array().is_some_and(|m| m.len() == 1))
        .filter_map(|(_, b)| b["messages"][0]["content"].as_str().and_then(|c| c.split('`').nth(1)).map(str::to_owned))
        .collect();
    // Three workers start the three longest tests first (b 5 s, c 3 s, a 1 s), the 0.1 s one last.
    assert_eq!(first.last().map(String::as_str), Some("login-cross-platform"), "{first:?}");
    let mut head = first[..3].to_vec();
    head.sort();
    assert_eq!(head, ["par-a", "par-b", "par-c"], "{first:?}");
    // 3 web tests × 3 model turns × 300 ms run concurrently, not one after another.
    assert!(started.elapsed() < std::time::Duration::from_secs(12), "{:?}", started.elapsed());
    let cross = out.lines().find(|l| l.contains("login-cross-platform")).unwrap_or("");
    assert!(cross.contains("Session plan: web: your own Chrome; android:"), "{cross}");
    let _ = std::fs::remove_dir_all(&dir);
}
