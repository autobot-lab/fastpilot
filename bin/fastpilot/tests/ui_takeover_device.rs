//! C7b on devices: the Live panel shows Android and iOS screens (screenshots from the device, only
//! while the stream is read), and "Ambil alih" works there as on the web: the AI's step waits, the
//! user's tap and text reach the app, and the step runs after Kembalikan. Talks to `fastpilot ui`
//! over HTTP like the page does. Skips without a device (Android: emulator/phone + agent-android
//! build; iOS: a Mac with ios-runner/build.sh done).

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// Devices are shared by the tests of this file.
static DEVICE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

struct Ui {
    child: std::process::Child,
    port: u16,
    cookie: String,
}
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn request(port: u16, method: &str, path: &str, cookie: &str, body: &str) -> TcpStream {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let origin = if method == "POST" { format!("Origin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\n") } else { String::new() };
    let req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\n{origin}Content-Length: {}\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).expect("write");
    s
}

fn http(ui: &Ui, method: &str, path: &str, body: &str) -> (u16, String) {
    let mut raw = String::new();
    request(ui.port, method, path, &ui.cookie, body).read_to_string(&mut raw).expect("read");
    let status = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, raw.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default())
}

fn start_ui(live: &std::path::Path) -> Ui {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .env("FASTPILOT_LIVE_DIR", live)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("link");
    let rest = line.trim().strip_prefix("http://127.0.0.1:").expect("link").to_owned();
    let (port, code) = rest.split_once("/?code=").expect("code");
    let port: u16 = port.parse().expect("port");
    let mut raw = String::new();
    request(port, "GET", &format!("/?code={code}"), "", "").read_to_string(&mut raw).expect("read");
    let cookie = raw.lines().find_map(|l| l.strip_prefix("set-cookie: ").or_else(|| l.strip_prefix("Set-Cookie: "))).and_then(|c| c.split(';').next()).expect("cookie").to_owned();
    Ui { child, port, cookie }
}

/// The first JPEG of `/api/live/frames` for the session.
fn first_frame(ui: &Ui, pid: u32, session: &str) -> Vec<u8> {
    let mut s = request(ui.port, "GET", &format!("/api/live/frames?pid={pid}&session={session}&fps=1"), &ui.cookie, "");
    s.set_read_timeout(Some(Duration::from_secs(30))).expect("timeout");
    let mut buf = Vec::new();
    let mut chunk = [0u8; 65536];
    loop {
        let n = s.read(&mut chunk).expect("frames");
        assert!(n > 0, "stream ended: {}", String::from_utf8_lossy(&buf));
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf).into_owned();
        if let Some(at) = text.find("Content-Length: ") {
            let len: usize = text[at + 16..].split("\r\n").next().and_then(|l| l.trim().parse().ok()).expect("length");
            let start = buf.windows(4).skip(at).position(|w| w == b"\r\n\r\n").map(|p| p + at + 4).expect("part head");
            if buf.len() >= start + len {
                return buf[start..start + len].to_vec();
            }
        }
    }
}

/// Take `session` over from the panel, tap the Email field and type into it while the AI's next
/// step waits, then give it back.
async fn take_over_device(client: &Client, session: &str, platform: &str) {
    let live = std::env::temp_dir().join(format!("fp-take-{platform}-{}", std::process::id()));
    let ui = start_ui(&live);
    let (_, sessions) = http(&ui, "GET", "/api/live", "");
    let v: Value = serde_json::from_str(&sessions).expect("json");
    let s = v["sessions"].as_array().expect("sessions").iter().find(|s| s["id"] == session).cloned().unwrap_or_else(|| panic!("{v}"));
    assert_eq!((s["platform"].as_str(), s["watchable"].as_bool(), s["controllable"].as_bool()), (Some(platform), Some(true), Some(true)), "{s}");
    let pid = s["pid"].as_u64().expect("pid") as u32;

    let jpeg = first_frame(&ui, pid, session);
    assert!(jpeg.starts_with(&[0xff, 0xd8]) && jpeg.len() > 5000, "a JPEG of the screen ({} bytes)", jpeg.len());

    // Where Email is, as a fraction of the screen's extent.
    let (ok, out) = call(client, "snapshot", json!({ "session": session, "format": "json" })).await;
    assert!(ok, "{out}");
    let snap: Value = serde_json::from_str(&out).expect("snapshot json");
    let boxes: Vec<(String, [f64; 4])> = snap["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter_map(|e| {
            let b = e["box"].as_array()?;
            Some((e["name"].as_str().unwrap_or("").to_owned(), [b[0].as_f64()?, b[1].as_f64()?, b[2].as_f64()?, b[3].as_f64()?]))
        })
        .collect();
    let (w, h) = boxes.iter().fold((0.0f64, 0.0f64), |(w, h), (_, b)| (w.max(b[0] + b[2]), h.max(b[1] + b[3])));
    let email = boxes.iter().find(|(n, _)| n == "Email").map(|(_, b)| *b).unwrap_or_else(|| panic!("no Email field: {out}"));
    let (fx, fy) = ((email[0] + email[2] / 2.0) / w, (email[1] + email[3] / 2.0) / h);

    let control = |body: Value| {
        let mut b = json!({ "pid": pid, "session": session });
        b.as_object_mut().expect("obj").extend(body.as_object().cloned().unwrap_or_default());
        http(&ui, "POST", "/api/live/control", &b.to_string())
    };
    let (st, body) = control(json!({ "op": "tap", "x": fx, "y": fy }));
    assert_eq!(st, 409, "input before Ambil alih is refused: {body}");
    let (st, body) = control(json!({ "op": "takeover" }));
    assert_eq!(st, 200, "{body}");

    let waiting = call(client, "run_flow", json!({ "flow": "- assertVisible: \"Masuk\"\n", "session": session }));
    tokio::pin!(waiting);
    assert!(tokio::time::timeout(Duration::from_millis(1500), &mut waiting).await.is_err(), "the AI's step waits during the takeover");
    let (st, body) = control(json!({ "op": "tap", "x": fx, "y": fy }));
    assert_eq!(st, 200, "{body}");
    tokio::time::sleep(Duration::from_millis(800)).await;
    let (st, body) = control(json!({ "op": "type", "text": "budi@contoh.id" }));
    assert_eq!(st, 200, "{body}");
    let (st, body) = control(json!({ "op": "release" }));
    assert_eq!(st, 200, "{body}");
    let (ok, out) = tokio::time::timeout(Duration::from_secs(30), waiting).await.expect("runs after Kembalikan");
    assert!(ok, "{out}");
    assert!(out.contains(&format!("the user took over session `{session}`")) && out.contains("1 tap, 1 text input"), "{out}");
    let (ok, out) = call(client, "snapshot", json!({ "session": session, "filter": "Email" })).await;
    assert!(ok && out.contains("budi@contoh.id"), "the user's text is in the Email field: {out}");
    drop(ui);
    let _ = std::fs::remove_dir_all(live);
}

fn mcp(live: &std::path::Path, env: &[(&str, String)]) -> TokioChildProcess {
    let env = env.to_vec();
    let live = live.to_owned();
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_LIVE_DIR", &live);
        for (k, v) in &env {
            c.env(k, v);
        }
    });
    TokioChildProcess::new(cmd).expect("spawn")
}

fn adb() -> String {
    let home = std::env::var("ANDROID_HOME").unwrap_or_else(|_| format!("{}/Library/Android/sdk", std::env::var("HOME").unwrap_or_default()));
    format!("{home}/platform-tools/adb")
}

#[tokio::test(flavor = "multi_thread")]
async fn take_over_android() {
    let _device = DEVICE.lock().await;
    let serial = std::env::var("FASTPILOT_ANDROID_SERIAL").ok().filter(|s| !s.is_empty()).or_else(|| {
        let out = std::process::Command::new(adb()).arg("devices").output().ok()?;
        String::from_utf8_lossy(&out.stdout).lines().skip(1).find_map(|l| l.strip_suffix("\tdevice").map(str::to_owned))
    });
    let apk = format!("{ROOT}/examples/demo-android/out/fastpilot-demo.apk");
    let agent = format!("{ROOT}/agent-android/out/fastpilot-agent.apk");
    let Some(serial) = serial.filter(|_| std::path::Path::new(&apk).exists() && std::path::Path::new(&agent).exists()) else {
        return eprintln!("skipping: no Android device, or agent-android/build.sh not run");
    };
    let install = || std::process::Command::new(adb()).args(["-s", &serial, "install", "-r", &apk]).output().expect("adb");
    let mut out = install();
    // Built in another checkout (another debug key): the installed demo goes first.
    if String::from_utf8_lossy(&out.stderr).contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") {
        let _ = std::process::Command::new(adb()).args(["-s", &serial, "uninstall", "com.fastpilot.demo"]).output();
        out = install();
    }
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let live = std::env::temp_dir().join(format!("fp-take-android-{}", std::process::id()));
    let client: Client = ().serve(mcp(&live, &[("FASTPILOT_ANDROID_SERIAL", serial)])).await.expect("initialize");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- session: { start: and1, platform: android }\n- launchApp: com.fastpilot.demo/.LoginActivity\n- assertVisible: \"Masuk\"\n" })).await;
    assert!(ok, "{out}");
    take_over_device(&client, "and1", "android").await;
    client.cancel().await.expect("shutdown");
}

#[tokio::test(flavor = "multi_thread")]
async fn take_over_ios_simulator() {
    let _device = DEVICE.lock().await;
    let app = format!("{ROOT}/ios-runner/build/Build/Products/Debug-iphonesimulator/FastpilotDemo.app");
    if !cfg!(target_os = "macos") || !std::path::Path::new(&app).exists() {
        return eprintln!("skipping: run ios-runner/build.sh first (macOS)");
    }
    let device = std::env::var("FASTPILOT_IOS_DEVICE").unwrap_or_else(|_| "iPhone 16".into());
    let _ = std::process::Command::new("xcrun").args(["simctl", "boot", &device]).output();
    let _ = std::process::Command::new("xcrun").args(["simctl", "bootstatus", &device, "-b"]).output();
    let out = std::process::Command::new("xcrun").args(["simctl", "install", "booted", &app]).output().expect("simctl");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let live = std::env::temp_dir().join(format!("fp-take-ios-{}", std::process::id()));
    let client: Client = ().serve(mcp(&live, &[])).await.expect("initialize");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- session: { start: ios1, platform: ios }\n- launchApp: com.fastpilot.demo\n- assertVisible: \"Masuk\"\n" })).await;
    assert!(ok, "{out}");
    take_over_device(&client, "ios1", "ios").await;
    client.cancel().await.expect("shutdown");
}
