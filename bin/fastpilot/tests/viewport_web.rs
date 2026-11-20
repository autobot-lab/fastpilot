//! setViewport + setLocale on real Chrome over MCP (issue #5): each step keeps what the other
//! set, in either order. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}

const PROBE: &str = r#"- evalScript: "(navigator.userAgent.includes('iPhone') ? 'iphone' : 'desktop') + ' ' + navigator.language + ' ' + screen.width""#;

#[tokio::test]
async fn locale_and_viewport_keep_each_other() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").current_dir(ROOT);
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let client: Client = client;
    // Viewport first, then locale: the iPhone UA is kept (was: desktop UA).
    let flow = format!("- session: {{ start: a, platform: web, target: examples/demo/login.html }}\n- setViewport: iPhone 15\n{PROBE}\n- setLocale: {{ locale: ar }}\n{PROBE}\n");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.contains("= \"iphone en-US 393\"") || out.contains("= \"iphone en 393\""), "viewport UA: {out}");
    assert!(out.contains("= \"iphone ar 393\""), "UA kept after setLocale: {out}");
    // Locale first, then viewport: the language is kept (was: dropped).
    let flow = format!("- session: {{ start: b, platform: web, target: examples/demo/login.html }}\n- setLocale: {{ locale: ar }}\n- setViewport: iPhone 15\n{PROBE}\n- setViewport: reset\n{PROBE}\n");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.contains("= \"iphone ar 393\""), "language kept after setViewport: {out}");
    assert!(out.contains("= \"desktop ar "), "reset keeps the language: {out}");
    client.cancel().await.expect("shutdown");
}
