//! S1 (v0.7): Map in one click. The Map page's plan comes from the current app (E6): its name for
//! the button, the kind of device that fits it (a website is never mapped on the Android emulator
//! by default, L062-10), its own start, 30 screens and 10 minutes. No app: nothing to map yet.
//! "Skip this page" keeps the map and stops asking about its sign-in screen. Temp HOME, no device.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;

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


fn start_ui_with(home: &Path, extra: &[(&str, String)]) -> Ui {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .envs(extra.iter().map(|(k, v)| (*k, v.as_str())))
        .current_dir(home)
        .env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_ADB_SERVER")
        .env_remove("ANDROID_SDK_ROOT")
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


#[test]
fn the_map_plan_follows_the_current_app() {
    let home = std::env::temp_dir().join(format!("fp-s1-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("fastpilot")).expect("dir");
    let apps = |targets: &str| format!(r#"{{ "current": "shop", "apps": [ {{ "id": "shop", "name": "Shop staging", "targets": {{ {targets} }}, "repo": null, "created_ms": 1 }} ] }}"#);
    let ui = start_ui_with(&home, &[]);
    let own = format!("http://127.0.0.1:{}", ui.port);
    let plan = || serde_json::from_str::<Value>(&request(&ui, "GET", "/api/map", "", &own).1).expect("json")["plan"].clone();

    // No app yet: nothing to map ("Add your app first").
    assert!(plan().is_null());
    // A website (with an Android app too): the website, from its own address, on Chrome.
    std::fs::write(home.join("fastpilot/apps.json"), apps(r#""web": "https://staging.shop.example", "android": "com.shop.app/.Main""#)).expect("apps");
    let p = plan();
    assert_eq!((p["app"]["name"].as_str(), p["platform"].as_str(), p["target"].as_str()), (Some("Shop staging"), Some("web"), Some("https://staging.shop.example")), "{p}");
    assert_eq!((p["screens"].as_u64(), p["minutes"].as_u64()), (Some(30), Some(10)));
    assert_eq!(p["kinds"].as_array().map(Vec::len), Some(2), "the other kind is under More options: {p}");
    // An Android app only: the Android emulator, from the app.
    std::fs::write(home.join("fastpilot/apps.json"), apps(r#""android": "com.shop.app/.Main""#)).expect("apps");
    let p = plan();
    assert_eq!((p["platform"].as_str(), p["target"].as_str()), (Some("android"), Some("com.shop.app/.Main")), "{p}");
    // "Skip this page" needs a map with a sign-in page.
    let (st, body) = request(&ui, "POST", "/api/map", &json!({ "action": "skip-wall" }).to_string(), &own);
    assert!(st == 422 && body.contains("There is no map yet"), "{st} {body}");
    // The Map page asks no question up front: no text field outside More options, one button.
    let (_, js) = request(&ui, "GET", "/assets/map.js", "", &own);
    assert!(js.contains("\"Map \" + p.app.name") && js.contains("UI.moreOptions(") && js.contains("Skip this page") && !js.contains("Project notes\")"), "map.js");
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}
