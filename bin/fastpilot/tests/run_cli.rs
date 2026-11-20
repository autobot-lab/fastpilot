//! `fastpilot run` end to end with a **stand-in Claude Code**: `tests/fake-claude/claude` takes
//! the same arguments as `claude -p`, connects to fastpilot over the MCP bridge and scripts the
//! agent's tool calls. So the bridge, the in-process tools, path memory, model selection,
//! retries and the JUnit/HTML output are exercised against real Chrome, with no model and no
//! API key. Skips without Chrome or python3.

use serde_json::Value;

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
    assert!(xml.contains("tests=\"1\" failures=\"0\"") && xml.contains("name=\"login-ok: Login berhasil\""), "{xml}");
    let page = std::fs::read_to_string(&html).expect("html");
    assert!(page.contains("checked Budi on the dashboard") && page.contains("iframe class=\"run\""), "suite html");
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
    assert!(!out.status.success() && text.contains("needs Claude Code installed and logged in"), "{text}");
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
