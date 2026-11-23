//! E6a (v0.7): the apps being tested, for this user, without a project folder, through the
//! dashboard's API on a real `fastpilot ui` in a temp HOME. `~/fastpilot/apps.json` holds them; one
//! app has a website, Android and iPhone version under `targets`; its id never changes; removing it
//! keeps its folder; a page cannot point an app at a folder; parallel changes lose nothing. The old
//! integrations list (Jira) moves to `/api/integrations`.

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

fn raw(port: u16, method: &str, path: &str, headers: &[(&str, String)], body: &str) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).expect("write");
    let mut out = String::new();
    s.read_to_string(&mut out).expect("read");
    let status = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let (head, body) = out.split_once("\r\n\r\n").map(|(h, b)| (h.to_owned(), b.to_owned())).unwrap_or_default();
    (status, head, body)
}

fn start(home: &Path) -> Ui {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(home)
        .env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_KEYSTORE", "file")
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
    let (_, head, _) = raw(port, "GET", &format!("/?code={code}"), &[], "");
    let cookie = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().split(';').next().unwrap_or("").to_owned())).expect("cookie");
    Ui { child, port, cookie }
}

impl Ui {
    fn get(&self, path: &str) -> Value {
        let (st, _, body) = raw(self.port, "GET", path, &[("Cookie", self.cookie.clone())], "");
        assert_eq!(st, 200, "{path}: {body}");
        serde_json::from_str(&body).expect("json")
    }
    fn post(&self, body: Value) -> (u16, Value) {
        let h = [("Cookie", self.cookie.clone()), ("Origin", format!("http://127.0.0.1:{}", self.port)), ("Content-Type", "application/json".to_owned())];
        let (st, _, text) = raw(self.port, "POST", "/api/apps", &h, &body.to_string());
        (st, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
}

#[test]
fn apps_are_kept_for_the_user_and_changed_only_through_the_page() {
    let home = std::env::temp_dir().join(format!("fp-apps-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");
    let ui = start(&home);

    assert_eq!(ui.get("/api/apps"), json!({ "current": null, "apps": [] }), "a new user has no app yet");

    // Add: a website and an Android app under one app; the first one becomes current.
    let (st, r) = ui.post(json!({ "action": "add", "name": "Shop staging", "targets": { "web": " https://staging.shop.example ", "android": "com.shop.app" }, "repo": "/etc" }));
    assert_eq!(st, 200, "{r}");
    let shop = &r["app"];
    assert_eq!((shop["id"].as_str(), shop["kinds"].clone(), shop["targets"].clone(), shop["repo"].clone()),
        (Some("shop-staging"), json!(["web", "android"]), json!({ "web": "https://staging.shop.example/", "android": "com.shop.app" }), Value::Null), "a page cannot set a repo folder: {r}");
    assert!(home.join("fastpilot/apps/shop-staging").is_dir(), "its own folder in ~/fastpilot");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.join("fastpilot/apps/shop-staging")).expect("dir").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "an app's data is owner-only");
    }
    assert_eq!(ui.get("/api/apps")["current"], "shop-staging");

    // Refused: the same name, no platform, an address that is not a website, a made-up app id.
    for (bad, why) in [
        (json!({ "action": "add", "name": "shop STAGING", "targets": { "web": "https://x.example" } }), "already an app called"),
        (json!({ "action": "add", "name": "Empty", "targets": {} }), "add a website address"),
        (json!({ "action": "add", "name": "Script", "targets": { "web": "javascript:alert(1)" } }), "not a website address"),
        (json!({ "action": "add", "name": "Phone", "targets": { "ios": "../../etc" } }), "not an iPhone app id"),
        (json!({ "action": "add", "name": "Login", "targets": { "web": "https://user:pw@shop.example" } }), "leave the user name and password out"),
        (json!({ "action": "add", "name": "Hidden", "targets": { "web": "https://shop.example%2f@evil.example" } }), "leave the user name and password out"),
        (json!({ "action": "add", "name": "Share", "targets": { "web": "file://evil.example/share" } }), "not a website address"),
    ] {
        let (st, r) = ui.post(bad.clone());
        assert!(st == 400 && r.to_string().contains(why), "{bad} → {st} {r}");
    }

    // A second app; "use" switches; update keeps the id; remove keeps the folder.
    let (_, r) = ui.post(json!({ "action": "add", "name": "Android demo", "targets": { "android": "com.fastpilot.demo" } }));
    assert_eq!((r["app"]["id"].as_str(), ui.get("/api/apps")["current"].as_str()), (Some("android-demo"), Some("shop-staging")), "adding does not switch: {r}");
    let (st, _) = ui.post(json!({ "action": "use", "id": "android-demo" }));
    assert_eq!((st, ui.get("/api/apps")["current"].as_str()), (200, Some("android-demo")));
    let (st, r) = ui.post(json!({ "action": "update", "id": "shop-staging", "name": "Shop live", "targets": { "web": "https://shop.example", "ios": "com.shop.app" } }));
    assert_eq!((st, r["app"]["id"].as_str(), r["app"]["name"].as_str(), r["app"]["kinds"].clone()), (200, Some("shop-staging"), Some("Shop live"), json!(["web", "ios"])), "{r}");
    let (st, _) = ui.post(json!({ "action": "remove", "id": "android-demo" }));
    assert_eq!(st, 200);
    let v = ui.get("/api/apps");
    assert_eq!((v["current"].as_str(), v["apps"].as_array().map(Vec::len)), (Some("shop-staging"), Some(1)), "the remaining app becomes current: {v}");
    assert!(home.join("fastpilot/apps/android-demo").is_dir(), "removing an app keeps its folder");
    assert_eq!(ui.post(json!({ "action": "use", "id": "android-demo" })).0, 400, "a removed app cannot be used");
    // A new app with the same name never gets the removed one's id, folder or (E2) passwords.
    let (_, r) = ui.post(json!({ "action": "add", "name": "Android demo", "targets": { "web": "https://other.example" } }));
    assert_eq!(r["app"]["id"], "android-demo-2", "{r}");
    let (_, r) = ui.post(json!({ "action": "remove", "id": "android-demo-2" }));
    assert_eq!(r["removed"], "android-demo-2");

    // Design M1 (seamless-v13): the address alone is enough; the name comes from it.
    let (st, r) = ui.post(json!({ "action": "add", "targets": { "web": "http://127.0.0.1:8080/login" } }));
    assert_eq!((st, r["app"]["name"].as_str()), (200, Some("127.0.0.1")), "{r}");
    let (_, r2) = ui.post(json!({ "action": "add", "name": "  ", "targets": { "web": "http://127.0.0.1:9090/" } }));
    assert_eq!(r2["app"]["name"], "127.0.0.1 (2)", "{r2}");
    for a in [&r, &r2] {
        assert_eq!(ui.post(json!({ "action": "remove", "id": a["app"]["id"] })).0, 200);
    }

    // Only the page may change apps: no Origin, or JSON from another site, is refused.
    let (st, _, _) = raw(ui.port, "POST", "/api/apps", &[("Cookie", ui.cookie.clone()), ("Content-Type", "application/json".to_owned())], &json!({ "action": "remove", "id": "shop-staging" }).to_string());
    assert_eq!(st, 403);
    let (st, _, _) = raw(ui.port, "POST", "/api/apps", &[("Cookie", ui.cookie.clone()), ("Origin", "http://evil.example".to_owned()), ("Content-Type", "application/json".to_owned())], &json!({ "action": "remove", "id": "shop-staging" }).to_string());
    assert_eq!(st, 403);
    assert_eq!(ui.get("/api/apps")["apps"].as_array().map(Vec::len), Some(1), "nothing removed");

    // Parallel adds (two browser tabs, Claude Code): none is lost.
    let (port, cookie) = (ui.port, ui.cookie.clone());
    let adds: Vec<_> = (0..12)
        .map(|i| {
            let cookie = cookie.clone();
            std::thread::spawn(move || {
                let h = [("Cookie", cookie), ("Origin", format!("http://127.0.0.1:{port}")), ("Content-Type", "application/json".to_owned())];
                raw(port, "POST", "/api/apps", &h, &json!({ "action": "add", "name": format!("Tab {i}"), "targets": { "web": format!("https://tab{i}.example") } }).to_string()).0
            })
        })
        .collect();
    assert!(adds.into_iter().all(|t| t.join().expect("thread") == 200));
    assert_eq!(ui.get("/api/apps")["apps"].as_array().map(Vec::len), Some(13), "every parallel add is kept");

    // Kept across a restart; the integrations list (Jira) moved to its own address.
    drop(ui);
    let ui = start(&home);
    let v = ui.get("/api/apps");
    assert_eq!((v["current"].as_str(), v["apps"][0]["name"].as_str(), v["apps"].as_array().map(Vec::len)), (Some("shop-staging"), Some("Shop live"), Some(13)), "{v}");
    let i = ui.get("/api/integrations");
    assert!(i.get("jira").is_some() && i.get("token_kept").is_some(), "{i}");
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(home.join("fastpilot/apps.json")).expect("apps.json")).expect("json");
    assert_eq!(saved["apps"][0]["targets"], json!({ "web": "https://shop.example/", "ios": "com.shop.app" }));
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}
