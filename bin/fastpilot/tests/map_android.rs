//! PF1 (v0.6) App map of the Android demo app (com.fastpilot.demo), on the emulator the device
//! suites are told to use (FASTPILOT_ANDROID_SERIAL, see support/android_device.rs: never guessed,
//! so never the user's own emulator). It signs in once with the test account from the project
//! notes, maps the Dashboard, never presses Bayar or Keluar, and leaves the app (a system
//! permission dialog) alone.

#[path = "support/android_device.rs"]
mod android_device;

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn call(port: u16, cookie: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
    let body = if body.is_null() { String::new() } else { body.to_string() };
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let st = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (st, raw.split_once("\r\n\r\n").and_then(|(_, b)| serde_json::from_str(b).ok()).unwrap_or(Value::Null))
}

#[test]
fn the_android_demo_app_is_mapped_on_the_chosen_emulator() {
    let Some(serial) = android_device::device() else { return };
    // A known start: the demo app signed out, as the device suites leave it after a reset.
    let out = std::process::Command::new(android_device::adb()).args(["-s", &serial, "shell", "pm", "clear", "com.fastpilot.demo"]).output().expect("adb");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Success"), "the demo app is installed on {serial}: {out:?}");
    let home = std::env::temp_dir().join(format!("fp-map-android-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let project = home.join("shop");
    std::fs::create_dir_all(&project).expect("project");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(&project)
        .env("HOME", &home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env("FASTPILOT_ANDROID_SERIAL", &serial)
        .env("FASTPILOT_VARS", "DEMO_PASS")
        .env("DEMO_PASS", "rahasia123")
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

    let (st, r) = call(port, &cookie, "POST", "/api/notes", json!({ "action": "add", "group": "accounts", "text": "Demo app account: a@b.com, password in ${DEMO_PASS}" }));
    assert_eq!(st, 200, "{r}");
    let (st, r) = call(port, &cookie, "POST", "/api/map", json!({ "action": "start", "target": "com.fastpilot.demo/.LoginActivity", "platform": "android", "screens": 10, "minutes": 5 }));
    assert_eq!(st, 200, "{r}");
    let until = Instant::now() + Duration::from_secs(360);
    let v = loop {
        let (_, v) = call(port, &cookie, "GET", "/api/map", Value::Null);
        if v["mapping"] == false && v["map"]["state"].as_str().is_some_and(|s| s != "mapping") {
            break v;
        }
        assert!(Instant::now() < until, "mapping did not finish: {v}");
        std::thread::sleep(Duration::from_secs(1));
    };
    let _ = child.kill();
    let _ = child.wait();
    let m = &v["map"];
    assert_eq!(m["platform"], "android", "{m}");
    assert!(m["state"] == "finished" || m["state"] == "limit", "{m}");
    let titles: Vec<&str> = m["screens"].as_array().expect("screens").iter().filter_map(|s| s["title"].as_str()).collect();
    assert!(titles.len() >= 2 && titles.contains(&"Dashboard"), "{titles:?}");
    assert!(m["screens"].as_array().expect("screens").iter().all(|s| s["location"].as_str().is_some_and(|l| l.contains("com.fastpilot.demo"))), "only the app's own screens: {m}");
    let skipped: Vec<&str> = m["skipped"].as_array().expect("skipped").iter().filter_map(|k| k["label"].as_str()).collect();
    assert!(skipped.contains(&"Bayar") && skipped.contains(&"Keluar"), "Bayar and Keluar are never pressed: {skipped:?}");
    assert_eq!(m["signed_in"], true, "{m}");
    // Where the device is: still in the demo app, on the emulator it was told to use.
    let out = std::process::Command::new(android_device::adb()).args(["-s", &serial, "shell", "dumpsys", "window"]).output().expect("adb");
    let focus = String::from_utf8_lossy(&out.stdout).lines().find(|l| l.contains("mCurrentFocus")).unwrap_or("").to_owned();
    assert!(focus.contains("com.fastpilot.demo"), "{focus}");
    let _ = std::fs::remove_dir_all(&home);
}
