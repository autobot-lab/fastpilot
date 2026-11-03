//! iOS simulator end-to-end over MCP (Phase 5). Needs Xcode and `ios-runner/build.sh` artifacts;
//! skips otherwise. Boots "iPhone 16" (or FASTPILOT_IOS_DEVICE) if no simulator is running.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ()>;

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn demo_app() -> Option<String> {
    let app = format!("{ROOT}/ios-runner/build/Build/Products/Debug-iphonesimulator/FastpilotDemo.app");
    let built = std::path::Path::new(&app).exists()
        && std::fs::read_dir(format!("{ROOT}/ios-runner/build/Build/Products"))
            .ok()?
            .any(|e| e.ok().is_some_and(|e| e.path().extension().is_some_and(|x| x == "xctestrun")));
    built.then_some(app)
}

const IOS_LOGIN: &str = r#"
- session: { start: ios1, platform: ios }
- launchApp: com.fastpilot.demo
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- tapOn: "Masuk"
- assertVisible: "Selamat datang, Budi"
- tapOn: "Keluar"
- assertVisible: "Masuk ke Akun"
"#;

#[tokio::test]
async fn ios_login_over_mcp() {
    let Some(app) = demo_app() else {
        return eprintln!("skipping: run ios-runner/build.sh first");
    };
    let device = std::env::var("FASTPILOT_IOS_DEVICE").unwrap_or_else(|_| "iPhone 16".into());
    let _ = std::process::Command::new("xcrun").args(["simctl", "boot", &device]).output();
    let _ = std::process::Command::new("xcrun").args(["simctl", "bootstatus", &device, "-b"]).output();
    let out = std::process::Command::new("xcrun").args(["simctl", "install", "booted", &app]).output().expect("simctl");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("DEMO_PASS", "rahasia123").env("FASTPILOT_VARS", "DEMO_PASS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": IOS_LOGIN })).await;
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.contains("(ios, com.fastpilot.demo)") && !out.contains("rahasia123"), "{out}");
    let (ok, out) = call(&client, "screenshot", json!({ "session": "ios1" })).await;
    assert!(ok, "{out}");
    client.cancel().await.expect("shutdown");
}
