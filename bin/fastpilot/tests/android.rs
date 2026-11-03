//! Android end-to-end over MCP (Phase 2 + WebView from Phase 4). Needs a booted device/emulator
//! (`emulator -avd ...` or `session start_device`) and `agent-android/build.sh` artifacts;
//! skips otherwise.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ()>;

/// One UiAutomation agent per device: tests that drive the device take turns.
static DEVICE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let _device = DEVICE.lock().await; // before prepare: reinstalling under a running test breaks it
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

fn adb_shell(cmd: &str) -> String {
    let out = std::process::Command::new(adb()).args(["shell", cmd]).output().expect("adb");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const LOGIN_STEPS: &str = r#"
- launchApp: com.fastpilot.demo/.LoginActivity
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- tapOn: "Masuk"
- assertVisible: "Selamat datang, Budi"
"#;

/// PRD §11.2 rule 6: permission prompts the flow does not touch are dismissed, a covering
/// keyboard is hidden, a crash / ANR fails the flow with a structured reason.
#[tokio::test]
async fn android_system_dialogs() {
    let _device = DEVICE.lock().await; // before prepare: reinstalling under a running test breaks it
    if prepare().is_none() {
        return eprintln!("skipping: no Android device or demo APK");
    }
    adb_shell("pm revoke com.fastpilot.demo android.permission.CAMERA; pm clear-permission-flags com.fastpilot.demo android.permission.CAMERA user-set user-fixed");
    let client: Client = ().serve(server()).await.expect("initialize");
    let (ok, out) = call(&client, "session", json!({ "action": "start", "id": "dlg", "platform": "android" })).await;
    assert!(ok, "{out}");

    // Keyboard: "Lupa password?" sits under the IME once the email field is focused.
    let keyboard = "- launchApp: com.fastpilot.demo/.LoginActivity\n- inputText: { into: \"Email\", text: \"a@b.com\" }\n- tapOn: \"Lupa password?\"\n- assertVisible: \"Cek email Anda\"\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": keyboard })).await;
    println!("{out}");
    assert!(ok, "{out}");

    // Permission prompt, unrelated to the next tap: denied, then the tap goes through.
    let flow = format!("{LOGIN_STEPS}- tapOn: \"Izin kamera\"\n- assertVisible: {{ id: permission_deny_button }}\n- tapOn: \"Bayar\"\n- assertVisible: \"Total Rp 150.000\"\n");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok, "{out}");
    assert!(adb_shell("dumpsys package com.fastpilot.demo").contains("android.permission.CAMERA: granted=false"));

    let flow = format!("{LOGIN_STEPS}- tapOn: \"Crash\"\n- assertVisible: {{ text: \"Tidak ada\", timeout: 10000 }}\n");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(!ok && out.contains("app_crashed"), "{out}");

    // An ANR needs pending input: the swipe is queued behind the blocked main thread.
    let flow = format!("{LOGIN_STEPS}- tapOn: \"Freeze\"\n- swipe: {{ from: [360, 900], to: [360, 500] }}\n- assertVisible: {{ text: \"Tidak ada\", timeout: 25000 }}\n");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow, "stop_on_fail": false })).await;
    println!("{out}");
    assert!(!ok && out.contains("app_not_responding"), "{out}");
    client.cancel().await.expect("shutdown");
}

/// Maestro-compatible steps on a device (the web e2e covers the remaining ones).
#[tokio::test]
async fn android_maestro_steps() {
    let _device = DEVICE.lock().await; // before prepare: reinstalling under a running test breaks it
    if prepare().is_none() {
        return eprintln!("skipping: no Android device or demo APK");
    }
    let client: Client = ().serve(server()).await.expect("initialize");
    let flow = r#"
- session: { start: m1, platform: android }
- startRecording: android-rec
- launchApp: com.fastpilot.demo/.LoginActivity
- inputText: { into: "Email", text: "a@b.comXY" }
- eraseText: 2
- copyTextFrom: "Email"
- tapOn: "Password"
- pasteText
- eraseText
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- hideKeyboard
- runFlow: { when: { platform: android }, commands: [ tapOn: "Masuk" ] }
- assertVisible: "Selamat datang, Budi"
- longPressOn: "Selamat datang, Budi"
- assertVisible: "Ditekan lama"
- doubleTapOn: "Ketukan: 0"
- assertVisible: "Ketukan: 2"
- scrollUntilVisible: "Riwayat 30"
- stopApp
- assertNotVisible: "Riwayat 30"
- stopRecording
- clearState: com.fastpilot.demo
"#;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.contains("copyTextFrom \"Email\"") && out.contains("= a@b.com"), "{out}");
    let mp4 = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.fastpilot/recordings/android-rec.mp4"));
    let video = std::fs::read(mp4).expect("android recording");
    assert!(video.len() > 10_000 && &video[4..8] == b"ftyp", "not an mp4: {} bytes", video.len());

    // Set-of-Mark on the device: the agent confirms it drew every listed box.
    let (ok, _) = call(&client, "run_flow", json!({ "flow": "- launchApp: com.fastpilot.demo/.LoginActivity\n" })).await;
    assert!(ok);
    let res: CallToolResult = client
        .call_tool(CallToolRequestParams::new("screenshot").with_arguments(json!({ "annotate": "som" }).as_object().cloned().unwrap_or_default()))
        .await
        .expect("screenshot");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    assert!(res.is_error != Some(true), "{text}");
    let image = res.content.iter().find_map(|c| c.as_image()).expect("annotated image");
    assert!(text.contains(" @"), "{text}");
    if let Ok(out) = std::env::var("FASTPILOT_SOM_OUT") {
        use base64::Engine;
        let jpeg = base64::engine::general_purpose::STANDARD.decode(&image.data).expect("base64");
        std::fs::write(out, jpeg).expect("write");
    }
    client.cancel().await.expect("shutdown");
}

/// One-time code from the device's SMS inbox (emulator console `sms send`).
#[tokio::test]
async fn android_reads_otp_from_sms() {
    let _device = DEVICE.lock().await;
    if prepare().is_none() {
        return eprintln!("skipping: no Android device or demo APK");
    }
    let code = format!("{}", 100_000 + std::process::id() % 900_000);
    let sent = std::process::Command::new(adb()).args(["emu", "sms", "send", "5551234", &format!("Kode masuk Anda {code}")]).output().expect("adb emu");
    if !String::from_utf8_lossy(&sent.stdout).contains("OK") {
        return eprintln!("skipping: not an emulator (no console to send an SMS)");
    }
    let client: Client = ().serve(server()).await.expect("initialize");
    let flow = "- session: { start: sms1, platform: android }\n- launchApp: com.fastpilot.demo/.LoginActivity\n- readOtp: { from: sms, after: \"kode\", into: SMS_OTP, timeout: 10000 }\n- inputText: { into: \"Email\", text: \"${SMS_OTP}\" }\n- copyTextFrom: \"Email\"\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok && out.contains("code from sms in ${SMS_OTP}"), "{out}");
    assert!(!out.contains(&code) && out.contains("copyTextFrom \"Email\"") && out.contains("= ***"), "{out}");
    client.cancel().await.expect("shutdown");
}
