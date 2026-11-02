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
        c.arg("mcp").current_dir(root).env("DEMO_PASS", "rahasia123");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");

    let tools = client.list_all_tools().await.expect("list tools");
    let mut names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(names, ["check_flow", "docs", "run_flow", "screenshot", "session", "snapshot"]);

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

    let (ok, out) = call(&client, "check_flow", json!({ "flow": "- launchApp: com.x\n- session: nope\n" })).await;
    assert!(!ok);
    assert!(out.contains("line 1: launchApp: Maestro command"), "{out}");

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
