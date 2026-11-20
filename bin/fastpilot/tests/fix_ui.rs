//! D2 (v0.4): the Fix button of the Device page end to end, against a stand-in Android SDK (small
//! scripts for adb, emulator, sdkmanager and avdmanager that log every call): nothing is
//! downloaded and no real emulator starts. Checked: the offer and its confirmation text, a fix
//! starts only when confirmed and only from the page, the steps and the live log, the test phone
//! is fastpilot's own on a port other than 5554, the person's emulator-5554 is never contacted,
//! Stop ends the fix and the emulator it started, and the AI has no tool that could run a fix.

use rmcp::ServiceExt;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

const ADB: &str = r#"#!/bin/bash
echo "adb $*" >> "$FAKE_SDK/calls.log"
case "$1" in
  version) echo "Android Debug Bridge version 1.0.41";;
  devices)
    echo "List of devices attached"
    printf 'emulator-5554\tdevice\n'
    for f in "$FAKE_SDK"/running-*; do [ -e "$f" ] && printf 'emulator-%s\tdevice\n' "${f##*-}"; done;;
  -s)
    port="${2#emulator-}"; shift 2
    case "$*" in
      "emu avd name") [ -e "$FAKE_SDK/running-$port" ] && { echo fastpilot_api35; echo OK; } || { echo Pixel_User; echo OK; };;
      "shell getprop sys.boot_completed") [ -e "$FAKE_SDK/booted-$port" ] && echo 1 || echo;;
    esac;;
esac
exit 0
"#;

const EMULATOR: &str = r#"#!/bin/bash
echo "emulator $*" >> "$FAKE_SDK/calls.log"
if [ "$1" = "-list-avds" ]; then echo Pixel_User; [ -e "$FAKE_SDK/avd-created" ] && echo fastpilot_api35; exit 0; fi
port=""; prev=""
for a in "$@"; do [ "$prev" = "-port" ] && port="$a"; prev="$a"; done
touch "$FAKE_SDK/running-$port"
trap 'rm -f "$FAKE_SDK/running-$port" "$FAKE_SDK/booted-$port"; echo "emulator $port stopped" >> "$FAKE_SDK/calls.log"; exit 0' TERM INT
[ -e "$FAKE_SDK/no-boot" ] || { sleep 2; touch "$FAKE_SDK/booted-$port"; }
for i in $(seq 1 120); do sleep 0.5; done
rm -f "$FAKE_SDK/running-$port" "$FAKE_SDK/booted-$port"
"#;

const SDKMANAGER: &str = r#"#!/bin/bash
echo "sdkmanager $*" >> "$FAKE_SDK/calls.log"
n=0; while read -t 2 line; do n=$((n+1)); done
echo "sdkmanager answered $n" >> "$FAKE_SDK/calls.log"
echo "License accepted"
printf '[=======                ] 30%% Downloading\r[=======================] 100%% Downloading\n'
mkdir -p "$FAKE_SDK/system-images/android-35/google_apis/arm64-v8a" "$FAKE_SDK/system-images/android-35/google_apis/x86_64"
"#;

const AVDMANAGER: &str = r#"#!/bin/bash
echo "avdmanager $*" >> "$FAKE_SDK/calls.log"
while read -t 2 line; do :; done
touch "$FAKE_SDK/avd-created"
echo "AVD created"
"#;

fn fake_sdk(dir: &Path) {
    for (rel, text) in [("platform-tools/adb", ADB), ("emulator/emulator", EMULATOR), ("cmdline-tools/latest/bin/sdkmanager", SDKMANAGER), ("cmdline-tools/latest/bin/avdmanager", AVDMANAGER)] {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().expect("dir")).expect("dir");
        std::fs::write(&p, text).expect("script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
    }
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

fn request(ui: &Ui, method: &str, path: &str, body: &str, origin: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", ui.port)).expect("connect");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
        port = ui.port,
        cookie = ui.cookie
    );
    s.write_all(req.as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let status = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, raw.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default())
}

fn start_ui(home: &Path, sdk: &Path) -> Ui {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(home)
        .env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_ADB_SERVER")
        .env_remove("ANDROID_SDK_ROOT")
        .env("ANDROID_HOME", sdk)
        .env("FAKE_SDK", sdk)
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("link");
    let rest = line.trim().strip_prefix("http://127.0.0.1:").expect("link").to_owned();
    let (port, code) = rest.split_once("/?code=").expect("code");
    let port: u16 = port.parse().expect("port");
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.write_all(format!("GET /?code={code} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let cookie = raw.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().to_owned())).and_then(|c| c.split(';').next().map(str::to_owned)).expect("cookie");
    Ui { child, port, cookie }
}

fn android(v: &Value) -> Option<Value> {
    v["offers"].as_array().and_then(|o| o.iter().find(|x| x["area"] == "android")).cloned()
}

fn wait_job(ui: &Ui, own: &str, until: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let (_, body) = request(ui, "GET", "/api/fix", "", own);
        let v: Value = serde_json::from_str(&body).expect("json");
        if until(&v["job"]) {
            return v;
        }
        assert!(Instant::now() < end, "the fix did not get there: {v}");
        std::thread::sleep(Duration::from_millis(300));
    }
}

#[test]
fn the_fix_button_sets_up_fastpilots_own_test_phone_only_after_the_person_confirms() {
    let home = std::env::temp_dir().join(format!("fp-fix-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let sdk = home.join("sdk");
    fake_sdk(&sdk);
    let ui = start_ui(&home, &sdk);
    let own = format!("http://127.0.0.1:{}", ui.port);
    let post = |body: Value, origin: &str| {
        let (st, text) = request(&ui, "POST", "/api/fix", &body.to_string(), origin);
        (st, serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text)))
    };

    // The offer: three steps, with size, time, the licence and "your own … not touched".
    let (st, body) = request(&ui, "GET", "/api/fix", "", &own);
    assert_eq!(st, 200, "{body}");
    let offer = android(&serde_json::from_str(&body).expect("json")).expect("android offer");
    assert_eq!(offer["button"], "Set up an emulator");
    assert_eq!(offer["steps"], json!(["Download Android 15 for the test phone", "Create the test phone", "Start it and check it works"]));
    let confirm = offer["confirm"].as_str().unwrap_or("");
    assert!(confirm.contains("about 1.5 GB") && confirm.contains("about 10 minutes") && confirm.contains("Your own phones and emulators are not touched") && confirm.contains("licence"), "{confirm}");

    // Only after confirming, only from the page itself.
    let (st, r) = post(json!({ "action": "start", "area": "android" }), &own);
    assert_eq!(st, 400, "{r}");
    let (st, _) = post(json!({ "action": "start", "area": "android", "confirmed": true }), "http://evil.example");
    assert_ne!(st, 200);
    let (st, _) = post(json!({ "action": "stop" }), &own);
    assert_eq!(st, 409, "nothing to stop");
    assert!(!sdk.join("calls.log").exists() || !std::fs::read_to_string(sdk.join("calls.log")).unwrap_or_default().contains("sdkmanager"), "nothing ran yet");

    let (st, r) = post(json!({ "action": "start", "area": "android", "confirmed": true }), &own);
    assert_eq!((st, r["job"]["state"].as_str()), (200, Some("running")), "{r}");
    let (st, r) = post(json!({ "action": "start", "area": "android", "confirmed": true }), &own);
    assert_eq!(st, 409, "one fix at a time: {r}");
    let v = wait_job(&ui, &own, |j| j["state"] != "running");
    let job = &v["job"];
    assert_eq!(job["state"], "done", "{v}");
    assert!(job["steps"].as_array().is_some_and(|s| s.iter().all(|x| x["state"] == "done")), "{job}");
    let log = job["log"].as_array().map(|l| l.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")).unwrap_or_default();
    assert!(log.contains("$ sdkmanager") && log.contains("100% Downloading") && log.contains("AVD created") && log.contains("the test phone is running"), "{log}");
    assert_eq!(job["message"], "The test phone is running. Android tests can use it now.");

    let calls = std::fs::read_to_string(sdk.join("calls.log")).expect("calls");
    let sdkmanager = calls.lines().find(|l| l.starts_with("sdkmanager --sdk_root=")).expect("sdkmanager");
    assert!(sdkmanager.contains("--install platform-tools emulator system-images;android-35;google_apis;"), "{sdkmanager}");
    assert!(calls.lines().any(|l| l.starts_with("sdkmanager answered ") && l != "sdkmanager answered 0"), "the licence is answered: {calls}");
    assert!(calls.contains("avdmanager create avd -n fastpilot_api35 -k system-images;android-35;google_apis;"), "{calls}");
    let start = calls.lines().find(|l| l.starts_with("emulator -avd")).expect("emulator start");
    assert!(start.starts_with("emulator -avd fastpilot_api35 -port ") && start.contains("-no-window"), "only fastpilot's own phone: {start}");
    let port: u16 = start.split("-port ").nth(1).and_then(|r| r.split(' ').next()).and_then(|p| p.parse().ok()).expect("port");
    assert!(port != 5554 && port.is_multiple_of(2) && (5556..=5584).contains(&port), "{port}");
    assert!(!calls.contains("emulator-5554"), "the person's emulator-5554 is never contacted: {calls}");
    assert!(!calls.contains("-avd Pixel_User"), "never another emulator: {calls}");

    // Running now: no offer any more.
    let (_, body) = request(&ui, "GET", "/api/fix", "", &own);
    assert!(android(&serde_json::from_str(&body).expect("json")).is_none(), "{body}");

    // Stop: the phone does not boot this time; Stop ends the step and the emulator it started.
    let _ = std::process::Command::new("pkill").args(["-f", &format!("{}/emulator/emulator", sdk.display())]).status();
    let end = Instant::now() + Duration::from_secs(10);
    while std::fs::read_dir(&sdk).expect("sdk").flatten().any(|e| e.file_name().to_string_lossy().starts_with("running-")) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(200));
    }
    std::fs::write(sdk.join("no-boot"), "").expect("no-boot");
    let (_, body) = request(&ui, "GET", "/api/fix", "", &own);
    let again = android(&serde_json::from_str(&body).expect("json")).expect("offer again");
    assert_eq!((again["button"].as_str(), again["steps"].clone()), (Some("Start the test phone"), json!(["Start it and check it works"])), "image and phone exist: {again}");
    let (st, _) = post(json!({ "action": "start", "area": "android", "confirmed": true }), &own);
    assert_eq!(st, 200);
    wait_job(&ui, &own, |_| std::fs::read_dir(&sdk).map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("running-"))).unwrap_or(false));
    let (st, _) = post(json!({ "action": "stop" }), &own);
    assert_eq!(st, 200);
    let v = wait_job(&ui, &own, |j| j["state"] != "running");
    assert_eq!(v["job"]["state"], "stopped", "{v}");
    assert_eq!(v["job"]["steps"][0]["state"], "stopped");
    let end = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_to_string(sdk.join("calls.log")).unwrap_or_default().matches(" stopped").any(|_| true) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(std::fs::read_to_string(sdk.join("calls.log")).unwrap_or_default().contains("stopped"), "the emulator this fix started is shut down");

    // The page part.
    let (st, js) = request(&ui, "GET", "/assets/fix.js", "", &own);
    assert!(st == 200 && js.contains("PAGES.deviceFix") && js.contains("You cannot start them from Chat, so the AI cannot install things."));
    let code: Vec<&str> = js.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("//")).collect();
    assert!(code.first() == Some(&"(() => {") && code.last() == Some(&"})();") && !js.contains("style: "), "wrapped, no inline styles");
    drop(ui);
    let _ = std::process::Command::new("pkill").args(["-f", &format!("{}/emulator/emulator", sdk.display())]).status();
    let _ = std::fs::remove_dir_all(&home);
}

/// The AI cannot run a fix: no MCP tool installs or sets up anything (Chat and Claude Code only
/// have these tools).
#[tokio::test]
async fn no_ai_tool_can_start_a_fix() {
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(std::env::temp_dir());
    });
    let client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let tools: Vec<String> = client.list_all_tools().await.expect("tools").into_iter().map(|t| t.name.to_string()).collect();
    for t in &tools {
        assert!(!["fix", "setup", "install"].iter().any(|w| t.contains(w)), "{t}");
    }
    client.cancel().await.expect("shutdown");
}
