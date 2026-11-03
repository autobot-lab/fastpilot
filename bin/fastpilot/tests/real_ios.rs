//! Physical iPhone, opt-in: set FASTPILOT_IOS_REAL_UDID (see `fastpilot ios doctor`). Skipped
//! otherwise. Uses only built-in apps (Settings, Safari), so nothing is installed beyond the
//! runner (free Apple ID: 3 apps per device).

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

#[tokio::test]
async fn real_iphone() {
    let Ok(udid) = std::env::var("FASTPILOT_IOS_REAL_UDID") else {
        return eprintln!("skipping: set FASTPILOT_IOS_REAL_UDID (fastpilot ios doctor lists paired iPhones)");
    };
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT);
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = call(&client, "session", json!({ "action": "devices" })).await;
    assert!(ok && out.contains(&format!("ios-device:{udid}")), "{out}");

    let flow = format!(
        "- session: {{ start: phone, platform: ios, device: \"ios-device:{udid}\" }}\n- launchApp: com.apple.Preferences\n- assertVisible: {{ text: \"General\", timeout: 15000 }}\n- tapOn: \"General\"\n- assertVisible: \"About\"\n- back\n- openUrl: \"https://example.com\"\n- assertVisible: {{ text: \"Example Domain\", timeout: 20000 }}\n"
    );
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok, "{out}");
    let (ok, out) = call(&client, "screenshot", json!({ "session": "phone", "annotate": "som" })).await;
    assert!(ok, "{out}");
    // simctl-only steps say so instead of failing obscurely.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- clearKeychain\n", "session": "phone" })).await;
    assert!(!ok && out.contains("unsupported"), "{out}");
    let (ok, out) = call(&client, "session", json!({ "action": "stop", "id": "phone" })).await;
    assert!(ok, "{out}");
    client.cancel().await.expect("shutdown");
}
