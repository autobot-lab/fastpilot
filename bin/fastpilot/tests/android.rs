//! Android end-to-end over MCP (Phase 2 + WebView from Phase 4). Needs a booted device/emulator
//! (`emulator -avd ...` or `session start_device`) and `agent-android/build.sh` artifacts;
//! skips otherwise.

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

fn adb() -> String {
    let home = std::env::var("ANDROID_HOME").unwrap_or_else(|_| format!("{}/Library/Android/sdk", std::env::var("HOME").unwrap_or_default()));
    format!("{home}/platform-tools/adb")
}

/// Installs the demo app; `None` when no device is attached.
fn prepare() -> Option<()> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let devices = std::process::Command::new(adb()).arg("devices").output().ok()?;
    let listed = String::from_utf8_lossy(&devices.stdout);
    if !listed.lines().skip(1).any(|l| l.ends_with("\tdevice")) {
        return None;
    }
    let apk = format!("{root}/examples/demo-android/out/fastpilot-demo.apk");
    std::path::Path::new(&apk).exists().then_some(())?;
    let out = std::process::Command::new(adb()).args(["install", "-r", &apk]).output().ok()?;
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    Some(())
}

fn server() -> TokioChildProcess {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(root).env("DEMO_PASS", "rahasia123").env("FASTPILOT_VARS", "DEMO_PASS");
    });
    TokioChildProcess::new(cmd).expect("spawn")
}

const ANDROID_LOGIN: &str = r#"
- session: { start: and1, platform: android }
- launchApp: com.fastpilot.demo/.LoginActivity
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- tapOn: "Masuk"
- assertVisible: "Selamat datang, Budi"
- tapOn: "Bayar"
- assertVisible: "Total Rp 150.000"
- evalScript: "document.querySelector('h1').textContent"
- tapOn: "Bayar sekarang"
- assertVisible: "Pembayaran berhasil"
- back
- assertVisible: "Dashboard"
"#;

const CROSS_PLATFORM: &str = r#"
- session: { start: web1, platform: web, target: "examples/demo/login.html" }
- session: { start: and1, platform: android }
- session: and1
- launchApp: com.fastpilot.demo/.LoginActivity
- parallel:
    - session: web1
      steps:
        - inputText: { into: "Email", text: "a@b.com" }
        - inputText: { into: "Password", text: "${DEMO_PASS}" }
        - tapOn: "Masuk"
        - assertVisible: "Budi"
    - session: and1
      steps:
        - inputText: { into: "Email", text: "a@b.com" }
        - inputText: { into: "Password", text: "${DEMO_PASS}" }
        - tapOn: "Masuk"
        - assertVisible: "Selamat datang, Budi"
"#;

#[tokio::test]
async fn android_login_webview_and_cross_platform_over_mcp() {
    if prepare().is_none() {
        return eprintln!("skipping: no Android device or demo APK");
    }
    let client: Client = ().serve(server()).await.expect("initialize");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": ANDROID_LOGIN })).await;
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.contains("= \"Pembayaran\""), "{out}");
    assert!(!out.contains("rahasia123"), "{out}");
    let (ok, out) = call(&client, "snapshot", json!({ "session": "and1" })).await;
    assert!(ok && out.contains("android, com.fastpilot.demo"), "{out}");
    let (ok, _) = call(&client, "session", json!({ "action": "stop", "id": "and1" })).await;
    assert!(ok);

    let (ok, out) = call(&client, "run_flow", json!({ "flow": CROSS_PLATFORM })).await;
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.contains("[web1] screen \"Dashboard\"") && out.contains("[and1] screen"), "{out}");
    client.cancel().await.expect("shutdown");
}
