//! S8 (v0.7): "Run on all platforms" from the dashboard. `GET /api/run-all?test=` says which
//! platforms of the current app the saved test would run on, with an "Add … app" button for each
//! missing one; `POST` starts `fastpilot run <test> --all-platforms`. Temp HOME, a stand-in Claude
//! that cannot start (so nothing real runs), no device.

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
fn run_on_all_platforms_names_the_platforms_and_the_missing_ones() {
    let home = std::env::temp_dir().join(format!("fp-s8-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let tests = home.join("fastpilot/apps/demo/tests");
    std::fs::create_dir_all(&tests).expect("dir");
    let apps = |targets: &str| format!(r#"{{ "current": "demo", "apps": [ {{ "id": "demo", "name": "Demo shop", "targets": {{ {targets} }}, "repo": null, "created_ms": 1 }} ] }}"#);
    std::fs::write(home.join("fastpilot/apps.json"), apps(r#""web": "https://staging.shop.example", "android": "com.shop.app""#)).expect("apps");
    std::fs::write(tests.join("login-saved.md"), "# Login\nid: login-saved\ntarget: https://staging.shop.example\n- Open the login page\n").expect("test");
    let ui = start_ui_with(&home, &[("FASTPILOT_CLAUDE", "/nonexistent/claude".into())]);
    let own = format!("http://127.0.0.1:{}", ui.port);

    let (st, body) = request(&ui, "GET", "/api/run-all?test=login-saved", "", &own);
    assert_eq!(st, 200, "{body}");
    let v: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["platforms"], json!(["Web · Chrome", "Android emulator"]), "{v}");
    assert_eq!(v["missing"], json!([{ "kind": "ios", "button": "Add iPhone app", "href": "#/apps/add?app=demo&kind=ios" }]), "a missing platform is an add button: {v}");
    for bad in ["nope", "..%2Fsecret", "Login-Saved"] {
        let (st, _) = request(&ui, "GET", &format!("/api/run-all?test={bad}"), "", &own);
        assert_eq!(st, 404, "{bad}");
    }
    let (st, body) = request(&ui, "POST", "/api/run-all", &json!({ "test": "login-saved" }).to_string(), &own);
    assert_eq!(st, 200, "{body}");
    assert!(body.contains("Running on Web · Chrome and Android emulator: watch it in Live."), "{body}");
    // It is `fastpilot run <saved test> --all-platforms` (here it stops at once: no Claude Code).
    let log = home.join("fastpilot/logs/run-all-login-saved.log");
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !std::fs::read_to_string(&log).unwrap_or_default().contains("Claude Code") && std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(std::fs::read_to_string(&log).unwrap_or_default().contains("Claude Code"), "the run started");
    // One platform only: nothing to compare, said plainly; the add buttons stay.
    std::fs::write(home.join("fastpilot/apps.json"), apps(r#""web": "https://staging.shop.example""#)).expect("apps");
    let (st, body) = request(&ui, "POST", "/api/run-all", &json!({ "test": "login-saved" }).to_string(), &own);
    assert!(st == 422 && body.contains("This app runs only on Web · Chrome. Add another kind of app to compare."), "{st} {body}");
    let v: Value = serde_json::from_str(&request(&ui, "GET", "/api/run-all?test=login-saved", "", &own).1).expect("json");
    assert_eq!(v["missing"].as_array().map(|m| m.iter().filter_map(|x| x["button"].as_str()).collect::<Vec<_>>()), Some(vec!["Add Android app", "Add iPhone app"]), "{v}");
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}
