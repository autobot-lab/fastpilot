//! Android end-to-end over MCP (Phase 2 + WebView from Phase 4). Needs the emulator named in
//! FASTPILOT_ANDROID_SERIAL and `agent-android/build.sh` artifacts; fails without them, saying
//! why (FASTPILOT_SKIP_ANDROID=1 skips on purpose).

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

#[path = "support/android_device.rs"]
mod android_device;
use android_device::adb;

/// The device under test: FASTPILOT_ANDROID_SERIAL / ANDROID_SERIAL, never guessed. Without a
/// usable one the test fails with the reason; `None` only with FASTPILOT_SKIP_ANDROID=1.
fn serial() -> Option<String> {
    android_device::device()
}

/// `adb -s <serial> ...`, so a second attached phone never receives the commands.
fn adb_s(args: &[&str]) -> std::process::Command {
    let mut c = std::process::Command::new(adb());
    if let Some(s) = serial() {
        c.args(["-s", &s]);
    }
    c.args(args);
    c
}

/// Screen size in pixels from `wm size` (override first).
fn screen() -> (u32, u32) {
    let text = adb_shell("wm size");
    let line = text.lines().rev().find(|l| l.contains("size:")).unwrap_or("");
    let (w, h) = line.rsplit(' ').next().and_then(|d| d.split_once('x')).unwrap_or(("720", "1280"));
    (w.trim().parse().unwrap_or(720), h.trim().parse().unwrap_or(1280))
}

/// Installs the demo app; `None` only when the Android tests are skipped on purpose. No device or
/// no built demo app fails the test, saying what to do.
fn prepare() -> Option<()> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    serial()?;
    let apk = format!("{root}/examples/demo-android/out/fastpilot-demo.apk");
    android_device::need_file(&apk, "the demo app");
    android_device::need_file(&format!("{root}/agent-android/out/fastpilot-agent.apk"), "the fastpilot agent app");
    let install = || adb_s(&["install", "-r", &apk]).output().ok();
    let mut out = install()?;
    // Built on another machine or checkout (another debug keystore): the installed copy has
    // to go first, as the driver does for its agent APK.
    if !out.status.success() && needs_uninstall(&format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))) {
        let _ = adb_s(&["uninstall", "com.fastpilot.demo"]).output();
        out = install()?;
    }
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    Some(())
}

/// `adb install` output that an uninstall fixes: another signing key, or a newer version.
fn needs_uninstall(out: &str) -> bool {
    out.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") || out.contains("INSTALL_FAILED_VERSION_DOWNGRADE")
}

#[test]
fn reinstalls_on_a_signature_mismatch() {
    assert!(needs_uninstall("adb: failed to install x.apk: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Existing package com.fastpilot.demo signatures do not match newer version; ignoring!]"));
    assert!(!needs_uninstall("Failure [INSTALL_FAILED_INSUFFICIENT_STORAGE]"));
}

/// These tests press "Bayar" and run evalScript in the demo's WebView: since v0.3.2 the built-in
/// limits refuse both, so they run with the lists emptied on purpose (a config file of their own).
fn no_limits() -> std::path::PathBuf {
    let file = std::env::temp_dir().join(format!("fp-android-limits-{}.toml", std::process::id()));
    std::fs::write(&file, "block_actions = []\nconfirm_actions = []\n").expect("config");
    file
}

/// Live-session files of the servers these tests start: a temp folder, never the user's
/// `~/.fastpilot/live` (a test that ends mid-run must leave nothing there).
fn live_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("fastpilot-android-live-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("live dir");
    d
}

fn server() -> TokioChildProcess {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp")
            .current_dir(root)
            .env("FASTPILOT_LIVE_DIR", live_dir())
            .env("FASTPILOT_CONFIG", no_limits())
            .env("DEMO_PASS", "rahasia123")
            .env("FASTPILOT_VARS", "DEMO_PASS")
            .env("FASTPILOT_ANDROID_SERIAL", serial().unwrap_or_default());
    });
    TokioChildProcess::new(cmd).expect("spawn")
}

// Every launch of the demo app is followed by a wait for its first screen: a launch on a busy
// emulator can take longer than the 5 s the next step has (F6); the tests check behaviour, not
// launch speed.
const ANDROID_LOGIN: &str = r#"
- session: { start: and1, platform: android }
- launchApp: com.fastpilot.demo/.LoginActivity
- assertVisible: { text: "Masuk ke Akun", timeout: 30000 }
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
- assertVisible: { text: "Masuk ke Akun", timeout: 30000 }
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
        return;
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
    let out = adb_s(&["shell", cmd]).output().expect("adb");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const LOGIN_STEPS: &str = r#"
- launchApp: com.fastpilot.demo/.LoginActivity
- assertVisible: { text: "Masuk ke Akun", timeout: 30000 }
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
        return;
    }
    adb_shell("pm revoke com.fastpilot.demo android.permission.CAMERA; pm clear-permission-flags com.fastpilot.demo android.permission.CAMERA user-set user-fixed");
    let client: Client = ().serve(server()).await.expect("initialize");
    let (ok, out) = call(&client, "session", json!({ "action": "start", "id": "dlg", "platform": "android" })).await;
    assert!(ok, "{out}");

    // Keyboard: "Lupa password?" sits under the IME once the email field is focused.
    let keyboard = "- launchApp: com.fastpilot.demo/.LoginActivity\n- assertVisible: { text: \"Masuk ke Akun\", timeout: 30000 }\n- inputText: { into: \"Email\", text: \"a@b.com\" }\n- tapOn: \"Lupa password?\"\n- assertVisible: \"Cek email Anda\"\n";
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
    let (w, h) = screen();
    let (x, y1, y2) = (w / 2, h * 7 / 10, h * 4 / 10);
    let flow = format!("{LOGIN_STEPS}- tapOn: \"Freeze\"\n- swipe: {{ from: [{x}, {y1}], to: [{x}, {y2}] }}\n- assertVisible: {{ text: \"Tidak ada\", timeout: 25000 }}\n");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow, "stop_on_fail": false })).await;
    println!("{out}");
    assert!(!ok && out.contains("app_not_responding"), "{out}");
    client.cancel().await.expect("shutdown");
    // The demo app's main thread is still asleep (up to 30 s): stop it, so the next test does not
    // start on a frozen app (F6: the test after this one failed 1 in 3, alone 0 in 3).
    adb_shell("am force-stop com.fastpilot.demo");
}

/// Maestro-compatible steps on a device (the web e2e covers the remaining ones).
#[tokio::test]
async fn android_maestro_steps() {
    let _device = DEVICE.lock().await; // before prepare: reinstalling under a running test breaks it
    if prepare().is_none() {
        return;
    }
    let client: Client = ().serve(server()).await.expect("initialize");
    let flow = r#"
- session: { start: m1, platform: android }
- startRecording: android-rec
- launchApp: com.fastpilot.demo/.LoginActivity
- assertVisible: { text: "Masuk ke Akun", timeout: 30000 }
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
    let (ok, _) = call(&client, "run_flow", json!({ "flow": "- launchApp: com.fastpilot.demo/.LoginActivity\n- assertVisible: { text: \"Masuk ke Akun\", timeout: 30000 }\n" })).await;
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
        return;
    }
    let code = format!("{}", 100_000 + std::process::id() % 900_000);
    let sent = adb_s(&["emu", "sms", "send", "5551234", &format!("Kode masuk Anda {code}")]).output().expect("adb emu");
    if !String::from_utf8_lossy(&sent.stdout).contains("OK") {
        return eprintln!("SKIPPED on purpose: {} is a phone; reading an SMS needs an emulator console", serial().unwrap_or_default());
    }
    let client: Client = ().serve(server()).await.expect("initialize");
    let flow = "- session: { start: sms1, platform: android }\n- launchApp: com.fastpilot.demo/.LoginActivity\n- assertVisible: { text: \"Masuk ke Akun\", timeout: 30000 }\n- readOtp: { from: sms, after: \"kode\", into: SMS_OTP, timeout: 10000 }\n- inputText: { into: \"Email\", text: \"${SMS_OTP}\" }\n- copyTextFrom: \"Email\"\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok && out.contains("code from sms in ${SMS_OTP}"), "{out}");
    let published = std::fs::read_dir(live_dir()).expect("live dir").flatten().any(|e| e.path().extension().is_some_and(|x| x == "json"));
    assert!(published, "the server publishes its live session in the test's own folder");
    assert!(!out.contains(&code) && out.contains("copyTextFrom \"Email\"") && out.contains("= ***"), "{out}");
    client.cancel().await.expect("shutdown");
}

/// Device control on the emulator: permissions, GPS, airplane mode, timezone, notification.
#[tokio::test]
async fn android_device_control() {
    let _device = DEVICE.lock().await;
    if prepare().is_none() {
        return;
    }
    let tz = adb_shell("getprop persist.sys.timezone").trim().to_owned();
    let client: Client = ().serve(server()).await.expect("initialize");
    let flow = r#"
- session: { start: dc, platform: android }
- launchApp: com.fastpilot.demo/.LoginActivity
- assertVisible: { text: "Masuk ke Akun", timeout: 30000 }
- setPermissions: { permissions: { camera: allow } }
- setLocation: { latitude: -6.2088, longitude: 106.8456 }
- setLocale: { timezone: Asia/Jakarta }
- setNetwork: offline
- pushNotification: { title: "Pesanan dikirim", body: "Paket Anda dalam perjalanan" }
"#;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    let airplane = adb_shell("settings get global airplane_mode_on");
    let (ok2, out2) = call(&client, "run_flow", json!({ "flow": "- setNetwork: online\n" })).await;
    if !tz.is_empty() {
        adb_shell(&format!("cmd alarm set-timezone {tz}"));
    }
    assert!(ok, "{out}");
    assert!(ok2, "{out2}");
    assert_eq!(airplane.trim(), "1");
    assert_eq!(adb_shell("settings get global airplane_mode_on").trim(), "0");
    assert!(adb_shell("dumpsys package com.fastpilot.demo").contains("android.permission.CAMERA: granted=true"));
    assert!(adb_shell("dumpsys notification --noredact").contains("Pesanan dikirim"));
    client.cancel().await.expect("shutdown");
}

/// E1: `launchApp` arguments arrive as typed intent extras (the demo lists them); `env` is
/// refused on Android with the way to do it.
#[tokio::test]
async fn android_launch_app_arguments() {
    let _device = DEVICE.lock().await;
    if prepare().is_none() {
        return;
    }
    let client: Client = ().serve(server()).await.expect("initialize");
    let flow = r#"
- session: { start: la, platform: android }
- launchApp:
    appId: com.fastpilot.demo/.LoginActivity
    arguments: { server: "https://staging.contoh.id/api?a=1&b='2'", retries: 3, debug: true, tags: [satu, "dua tiga"] }
- assertVisible: "extra debug=true"
- assertVisible: "extra retries=3"
- assertVisible: "extra server=https://staging.contoh.id/api?a=1&b='2'"
- assertVisible: "extra tags=satu|dua tiga"
- launchApp: com.fastpilot.demo/.LoginActivity
- assertVisible: { text: "Masuk ke Akun", timeout: 30000 }
- assertNotVisible: { text: "extra debug=true", timeout: 3000 }
"#;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok, "{out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- launchApp: { appId: com.fastpilot.demo, env: { API_TOKEN: abc } }\n", "session": "la" })).await;
    assert!(!ok && out.contains("iOS only") && !out.contains("abc"), "{out}");
    client.cancel().await.expect("shutdown");
}

/// A TCP relay in front of `target` (stands in for an SSH tunnel); counts connections.
async fn tunnel(target: &'static str) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = count.clone();
    tokio::spawn(async move {
        while let Ok((mut a, _)) = listener.accept().await {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::spawn(async move {
                if let Ok(mut b) = tokio::net::TcpStream::connect(target).await {
                    let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
                }
            });
        }
    });
    (port, count)
}

/// D2: devices behind a configured adb server (here through a relay, as through an SSH tunnel):
/// the agent and the WebView are reached through the server itself, not through a port the
/// server would open on its own machine.
#[tokio::test(flavor = "multi_thread")]
async fn android_through_a_configured_adb_server() {
    let _device = DEVICE.lock().await;
    if prepare().is_none() {
        return;
    }
    let (port, connections) = tunnel("127.0.0.1:5037").await;
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp")
            .current_dir(root)
            .env("FASTPILOT_LIVE_DIR", live_dir())
            .env("DEMO_PASS", "rahasia123")
            .env("FASTPILOT_VARS", "DEMO_PASS")
            .env("FASTPILOT_ADB_SERVER", format!("127.0.0.1:{port}"))
            .env("FASTPILOT_CONFIG", no_limits())
            .env("FASTPILOT_ANDROID_SERIAL", serial().unwrap_or_default());
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": ANDROID_LOGIN })).await;
    println!("{out}");
    assert!(ok, "{out}");
    assert!(out.contains("= \"Pembayaran\""), "WebView reached through the server: {out}");
    assert!(connections.load(std::sync::atomic::Ordering::SeqCst) > 3, "traffic did not go through the configured server");
    // The emulator console is only on the emulator's machine.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- setLocation: { latitude: -6.2, longitude: 106.8 }\n", "session": "and1" })).await;
    assert!(!ok && out.contains("FASTPILOT_ADB_SERVER"), "{out}");
    client.cancel().await.expect("shutdown");

    // Nothing listening there: a clear error, and no local adb server is started in its place.
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(root).env("FASTPILOT_LIVE_DIR", live_dir()).env("FASTPILOT_ADB_SERVER", "127.0.0.1:1");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- session: { start: x, platform: android }\n" })).await;
    assert!(!ok && out.contains("cannot reach the adb server at 127.0.0.1:1"), "{out}");
    client.cancel().await.expect("shutdown");
}
