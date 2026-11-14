//! Snapshot regressions on real Chrome over MCP. Skips without Chrome.
//! Issue #1: plain-text keypad keys must stay separate, tappable nodes.

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

#[tokio::test]
async fn pin_pad_keys_are_separate_nodes() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-snapshot-web-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_DIR", dir.join(".fastpilot"));
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let client: Client = client;
    let flow = r#"
- session: { start: w, platform: web, target: examples/demo/pin.html }
- tapOn: "1"
- tapOn: "5"
- assertVisible: "PIN: 15"
- assertVisible: "4.8 (11)"
- assertVisible: "Toko Budi"
"#;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok, "{out}");
    let (ok, snap) = call(&client, "snapshot", json!({ "session": "w", "full": true })).await;
    assert!(ok, "{snap}");
    assert!(!snap.contains("PIN: 15 1 2"), "label merged with the keys: {snap}");
    for k in ["\"1\"", "\"2\"", "\"0\""] {
        assert!(snap.contains(&format!("text {k}")), "key {k} is not its own node: {snap}");
    }
    assert!(!snap.contains("Rating 4.8"), "{snap}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(dir);
}
