//! Physical Android phone, opt-in: set FASTPILOT_ANDROID_REAL_SERIAL to the phone's serial
//! (Xiaomi/Redmi/POCO, Samsung, Pixel). Skipped otherwise. Drives Settings (always installed),
//! the demo app when its APK is built, and checks that close restores the animation scales.

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

fn adb(serial: &str, args: &[&str]) -> String {
    let home = std::env::var("ANDROID_HOME").unwrap_or_else(|_| format!("{}/Library/Android/sdk", std::env::var("HOME").unwrap_or_default()));
    let out = std::process::Command::new(format!("{home}/platform-tools/adb")).args(["-s", serial]).args(args).output().expect("adb");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test]
async fn real_android_phone() {
    let Ok(serial) = std::env::var("FASTPILOT_ANDROID_REAL_SERIAL") else {
        return eprintln!("skipping: set FASTPILOT_ANDROID_REAL_SERIAL to a physical phone's serial");
    };
    let before = adb(&serial, &["shell", "settings get global animator_duration_scale"]);
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT);
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = call(&client, "session", json!({ "action": "devices" })).await;
    assert!(ok && out.contains(&format!("android-device:{serial}")), "{out}");

    let flow = format!(
        "- session: {{ start: phone, platform: android, device: \"android-device:{serial}\" }}\n- launchApp: com.android.settings\n- scrollUntilVisible: {{ element: {{ role: text }}, direction: down, timeout: 5000 }}\n- back\n"
    );
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok, "{out}");
    let (ok, out) = call(&client, "screenshot", json!({ "session": "phone", "annotate": "som" })).await;
    assert!(ok, "{out}");

    let apk = format!("{ROOT}/examples/demo-android/out/fastpilot-demo.apk");
    if std::path::Path::new(&apk).exists() {
        adb(&serial, &["install", "-r", &apk]);
        let flow = "- launchApp: com.fastpilot.demo/.LoginActivity\n- inputText: { into: \"Email\", text: \"a@b.com\" }\n- inputText: { into: \"Password\", text: \"rahasia123\" }\n- tapOn: \"Masuk\"\n- assertVisible: \"Selamat datang, Budi\"\n";
        let (ok, out) = call(&client, "run_flow", json!({ "flow": flow, "session": "phone" })).await;
        println!("{out}");
        assert!(ok, "{out}");
    }

    let (ok, out) = call(&client, "session", json!({ "action": "stop", "id": "phone" })).await;
    assert!(ok, "{out}");
    assert_eq!(adb(&serial, &["shell", "settings get global animator_duration_scale"]), before, "animation scale restored");
    assert!(adb(&serial, &["shell", "ls /data/local/tmp/fastpilot-anim.bak 2>/dev/null"]).trim().is_empty(), "backup removed");
    client.cancel().await.expect("shutdown");
}
