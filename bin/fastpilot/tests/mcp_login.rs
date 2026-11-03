//! End-to-end over MCP stdio: spawns `fastpilot mcp` and drives the demo login page.
//! Skips (passes) when no Chrome is installed.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ()>;

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
        .await
        .expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

const LOGIN: &str = r#"
- session: { start: web1, platform: web, target: "examples/demo/login.html" }
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- tapOn: "Masuk"
- assertVisible: "Budi"
"#;

const PARALLEL: &str = r#"
- session: { start: a, target: "examples/demo/login.html" }
- session: { start: b, target: "examples/demo/login.html" }
- parallel:
    - session: a
      steps:
        - inputText: { into: "Email", text: "a@b.com" }
        - inputText: { into: "Password", text: "${DEMO_PASS}" }
        - tapOn: "Masuk"
        - assertVisible: "Dashboard"
    - session: b
      steps:
        - inputText: { into: "Email", text: "salah@b.com" }
        - inputText: { into: "Password", text: "nope" }
        - tapOn: "Masuk"
        - assertVisible: "Email atau password salah"
"#;

#[tokio::test]
async fn login_and_parallel_sessions_over_mcp() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(root).env("DEMO_PASS", "rahasia123").env("FASTPILOT_VARS", "DEMO_PASS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");

    let tools = client.list_all_tools().await.expect("list tools");
    let mut names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(names, ["assert", "check_flow", "docs", "report", "run_flow", "screenshot", "session", "snapshot"]);

    let (ok, out) = call(&client, "run_flow", json!({ "flow": LOGIN })).await;
    if out.contains("Chrome not found") {
        eprintln!("skipping: {out}");
        return;
    }
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.starts_with("ok 5/5 steps"), "{out}");
    assert!(out.contains("screen \"Dashboard\""), "{out}");
    assert!(!out.contains("rahasia123"), "secret leaked: {out}");

    // Stop early with reason, candidates and the session's snapshot.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- tapOn: \"Hapus Akun\"\n" })).await;
    println!("{out}");
    assert!(!ok);
    assert!(out.starts_with("fail step 1/1 [web1] tapOn \"Hapus Akun\": not_found"), "{out}");
    assert!(out.contains("candidates: e") && out.contains("button \"Keluar\""), "{out}");

    let (ok, out) = call(&client, "run_flow", json!({ "flow": PARALLEL })).await;
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.starts_with("ok 10/10 steps"), "{out}");
    assert!(out.contains("[a] screen \"Dashboard\"") && out.contains("[b] screen \"Login\""), "{out}");

    let (ok, out) = call(&client, "check_flow", json!({ "flow": "- hideKeyboard\n- session: nope\n" })).await;
    assert!(!ok);
    assert!(out.contains("line 1: hideKeyboard: Maestro command"), "{out}");

    let (ok, out) = call(&client, "snapshot", json!({ "session": "b", "filter": "button" })).await;
    assert!(ok && out.contains("button \"Masuk\""), "{out}");

    let res = client
        .call_tool(CallToolRequestParams::new("screenshot").with_arguments(json!({ "session": "a" }).as_object().cloned().unwrap_or_default()))
        .await
        .expect("screenshot");
    assert!(res.content.first().and_then(|c| c.as_image()).is_some_and(|i| i.mime_type == "image/jpeg"));

    let (ok, out) = call(&client, "session", json!({ "action": "list" })).await;
    assert!(ok && out.contains("a\n") && out.contains("web1"), "{out}");

    client.cancel().await.expect("shutdown");
}

const WIDGETS: &str = r#"
- session: { start: w, target: "examples/demo/widgets.html" }
- selectOption: { from: "Negara", option: "Malaysia" }
- assertVisible: "Dipilih: my"
- tapOn: "Buka detail"
- assertVisible: "Detail terbuka"
- tapOn: "Hapus"
- assertVisible: "confirm: Hapus data?"
- assertVisible: "Data dihapus"
- inputText: { into: "Cari", text: "sepatu" }
- pressKey: Enter
- assertVisible: "Hasil untuk: sepatu"
- uploadFile: { into: "Lampiran", files: ["examples/demo/login.html"] }
- assertVisible: "File: login.html"
- tapOn: "Tombol dalam frame"
- assertVisible: "Diklik"
- tapOn: "Tombol shadow"
- assertVisible: "Shadow diklik"
- assertNotVisible: { text: "Detail tertutup", timeout: 500 }
- evalScript: "`${document.title}`"
"#;

#[tokio::test]
async fn web_widgets_over_mcp() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(root);
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": WIDGETS })).await;
    if out.contains("Chrome not found") {
        eprintln!("skipping: {out}");
        return;
    }
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.contains("evalScript \"`${document.title}`\"") && out.contains("= \"Widgets\""), "{out}");
    client.cancel().await.expect("shutdown");
}

fn chrome() -> Option<&'static str> {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
}

#[tokio::test]
async fn assert_report_and_path_memory_over_mcp() {
    if chrome().is_none() {
        return eprintln!("skipping: no Chrome");
    }
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let dir = std::env::temp_dir().join(format!("fastpilot-e2e-mem-{}", std::process::id()));
    let spawn = || {
        let dir = dir.clone();
        tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
            c.arg("mcp").current_dir(root).env("DEMO_PASS", "rahasia123").env("FASTPILOT_VARS", "DEMO_PASS").env("FASTPILOT_DIR", &dir);
        })
    };
    let client: Client = ().serve(TokioChildProcess::new(spawn()).expect("spawn")).await.expect("initialize");
    // Explore with a ref, as an AI would after reading a snapshot: memory must store role+name.
    let (ok, out) = call(&client, "session", json!({ "action": "start", "id": "w", "target": "examples/demo/login.html", "test_id": "login-ok" })).await;
    assert!(ok && !out.contains("path memory"), "{out}");
    let flow = "- inputText: { into: Email, text: a@b.com }\n- inputText: { into: Password, text: \"${DEMO_PASS}\" }\n- tapOn: e5\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow, "test_id": "login-ok" })).await;
    assert!(ok && out.contains("run "), "{out}");
    let (ok, out) = call(&client, "assert", json!({ "kind": "contains", "selector": "{ role: text, name: Budi }", "text": "Bud", "test_id": "login-ok" })).await;
    assert!(ok, "{out}");
    let (ok, out) = call(&client, "assert", json!({ "kind": "text_equals", "selector": "Dashboard", "text": "Nope", "timeout": 300 })).await;
    assert!(!ok && out.contains("assertion_failed") && out.contains("[app]"), "{out}");
    let (ok, out) = call(&client, "report", json!({ "test_id": "login-ok", "status": "pass", "notes": "first run" })).await;
    assert!(ok && out.contains("path memory updated"), "{out}");
    client.cancel().await.expect("shutdown");

    let memo = std::fs::read_to_string(dir.join("memory/login-ok.json")).expect("memory file");
    assert!(memo.contains("role: button") && memo.contains("name: Masuk") && !memo.contains("e5"), "{memo}");
    assert!(memo.contains("${DEMO_PASS}") && !memo.contains("rahasia123"), "{memo}");

    // Next run: the hint comes back and replays in one round-trip.
    let client: Client = ().serve(TokioChildProcess::new(spawn()).expect("spawn")).await.expect("initialize");
    let (ok, out) = call(&client, "session", json!({ "action": "start", "id": "w", "target": "examples/demo/login.html", "test_id": "login-ok" })).await;
    assert!(ok && out.contains("passed 1x"), "{out}");
    let hint = out.split("```yaml\n").nth(1).and_then(|h| h.split("```").next()).expect("yaml hint").to_owned();
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("{hint}- assertVisible: Budi\n"), "test_id": "login-ok" })).await;
    assert!(ok, "{out}");
    let (ok, _) = call(&client, "report", json!({ "test_id": "login-ok", "status": "pass" })).await;
    assert!(ok);
    client.cancel().await.expect("shutdown");
    let runs = std::fs::read_dir(dir.join("runs")).expect("runs").count();
    assert_eq!(runs, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn attach_to_running_chrome_over_mcp() {
    let Some(exe) = chrome() else {
        return eprintln!("skipping: no Chrome");
    };
    let profile = std::env::temp_dir().join(format!("fastpilot-e2e-attach-{}", std::process::id()));
    let mut chrome = tokio::process::Command::new(exe)
        .args(["--headless=new", "--remote-debugging-port=9339", "--no-first-run", "about:blank"])
        .arg(format!("--user-data-dir={}", profile.display()))
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("chrome");
    let mut ready = false;
    for _ in 0..50 {
        if tokio::net::TcpStream::connect("127.0.0.1:9339").await.is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(ready, "chrome debugging port");
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(root);
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = call(&client, "session", json!({ "action": "start", "id": "mine", "mode": "attach", "endpoint": "http://127.0.0.1:9339", "target": "examples/demo/widgets.html" })).await;
    assert!(ok && out.contains("screen \"Widgets\""), "{out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- tapOn: Buka detail\n- assertVisible: Detail terbuka\n" })).await;
    assert!(ok, "{out}");
    let (ok, _) = call(&client, "session", json!({ "action": "stop", "id": "mine" })).await;
    assert!(ok);
    client.cancel().await.expect("shutdown");
    // Detaching must leave the user's tab alive.
    let body = fastpilot_web::http_get("127.0.0.1:9339", "/json/list").await.expect("json list");
    assert!(body.contains("widgets.html"), "{body}");
    let _ = chrome.kill().await;
    let _ = std::fs::remove_dir_all(profile);
}
