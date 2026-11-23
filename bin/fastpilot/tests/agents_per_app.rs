//! S4 (v0.7): no agent setup in the main path. Each app has a working tester with no clicks,
//! "<app> tester", with the app's platforms, address and auto-picked skills; a change the user makes
//! in Agent settings keeps working on every app, and one app's name or address never sticks to the
//! next app's tester. "New agent" makes an agent with one focus in one click, from the current app.
//! Temp HOME throughout.

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

fn raw(port: u16, method: &str, path: &str, headers: &[(&str, String)], body: &str) -> (u16, String) {
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
    (status, out)
}

fn start(home: &Path) -> Ui {
    std::fs::create_dir_all(home.join("elsewhere")).expect("home");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(home.join("elsewhere"))
        .env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env("FASTPILOT_KEYSTORE", "file")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("link");
    let rest = line.trim().strip_prefix("http://127.0.0.1:").expect("link").to_owned();
    let (port, code) = rest.split_once("/?code=").expect("code");
    let port: u16 = port.parse().expect("port");
    let (_, head) = raw(port, "GET", &format!("/?code={code}"), &[], "");
    let cookie = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().split(';').next().unwrap_or("").to_owned())).expect("cookie");
    Ui { child, port, cookie }
}

fn body(out: &str) -> Value {
    serde_json::from_str(out.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")).unwrap_or(Value::Null)
}

impl Ui {
    fn get(&self, path: &str) -> Value {
        let (st, out) = raw(self.port, "GET", path, &[("Cookie", self.cookie.clone())], "");
        assert_eq!(st, 200, "{path}: {out}");
        body(&out)
    }
    fn post(&self, path: &str, b: Value) -> Value {
        let h = [("Cookie", self.cookie.clone()), ("Origin", format!("http://127.0.0.1:{}", self.port)), ("Content-Type", "application/json".to_owned())];
        let (st, out) = raw(self.port, "POST", path, &h, &b.to_string());
        assert_eq!(st, 200, "{b}: {out}");
        body(&out)
    }
    fn agents(&self) -> Vec<Value> {
        self.get("/api/agents")["agents"].as_array().cloned().unwrap_or_default().into_iter().map(|x| x["agent"].clone()).collect()
    }
    fn tester(&self) -> Value {
        self.agents().into_iter().find(|a| a["id"] == "default").expect("the default agent")
    }
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

#[test]
fn each_app_has_its_own_tester_and_new_agent_takes_one_click() {
    let home = std::env::temp_dir().join(format!("fp-agents-per-app-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let ui = start(&home);
    ui.post("/api/apps", json!({ "action": "add", "name": "Shop", "targets": { "web": "https://shop.example", "android": "com.shop.app" } }));
    ui.post("/api/apps", json!({ "action": "add", "name": "Blog", "targets": { "web": "https://blog.example" } }));

    // A fresh app has a working agent with no clicks: its name, platforms, address and skills.
    let t = ui.tester();
    assert_eq!(t["name"], "Shop tester", "{t}");
    assert_eq!(strs(&t["devices"]), ["web", "android_emulator"], "{t}");
    assert_eq!(t["app"], "https://shop.example/", "{t}");
    let skills = strs(&t["skills"]);
    assert!(skills.contains(&"choose-technique".to_owned()) && skills.contains(&"mobile-basics".to_owned()) && skills.contains(&"regression".to_owned()), "auto-picked skills: {skills:?}");

    // The other app's tester is its own.
    ui.post("/api/apps", json!({ "action": "use", "id": "blog" }));
    let t = ui.tester();
    assert_eq!((t["name"].as_str(), strs(&t["devices"]), t["app"].as_str()), (Some("Blog tester"), vec!["web".to_owned()], Some("https://blog.example/")), "{t}");

    // Agent settings sends back what it shows; one app's values must not stick to the next app's
    // tester, while the user's own change (an instruction) stays on every app.
    let mut changed = t.clone();
    changed["instructions"] = json!(["Stop at the first failure and report"]);
    ui.post("/api/agents", json!({ "action": "update", "agent": changed }));
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(home.join(".fastpilot/agents.json")).expect("agents.json")).expect("json");
    let d = saved["agents"].as_array().and_then(|a| a.iter().find(|x| x["id"] == "default")).cloned().expect("saved default");
    assert!(!d.to_string().contains("Blog") && !d.to_string().contains("blog.example"), "no app value is saved into the shared agent: {d}");
    ui.post("/api/apps", json!({ "action": "use", "id": "shop" }));
    let t = ui.tester();
    assert_eq!((t["name"].as_str(), t["app"].as_str()), (Some("Shop tester"), Some("https://shop.example/")), "{t}");
    assert_eq!(strs(&t["instructions"]), ["Stop at the first failure and report"], "the user's own change stays: {t}");

    // A name the user picked is kept on every app.
    let mut named = t.clone();
    named["name"] = json!("Lead tester");
    ui.post("/api/agents", json!({ "action": "update", "agent": named }));
    ui.post("/api/apps", json!({ "action": "use", "id": "blog" }));
    assert_eq!(ui.tester()["name"], "Lead tester");
    ui.post("/api/apps", json!({ "action": "use", "id": "shop" }));

    // New agent: three focuses, each one click, everything else from the current app.
    let focuses: Vec<String> = ui.get("/api/agents")["focuses"].as_array().cloned().unwrap_or_default().iter().filter_map(|f| f["label"].as_str().map(str::to_owned)).collect();
    assert_eq!(focuses, ["Regression of everything", "Checkout and payments", "Login and accounts"]);
    let a = ui.post("/api/agents", json!({ "action": "create_focus", "focus": "checkout" }))["agent"].clone();
    assert_eq!((a["name"].as_str(), strs(&a["devices"]), a["app"].as_str()), (Some("Shop checkout"), vec!["web".to_owned(), "android_emulator".to_owned()], Some("https://shop.example/")), "{a}");
    assert!(strs(&a["skills"]).contains(&"safe-checkout".to_owned()), "{a}");
    assert!(a["job"].as_str().unwrap_or("").contains("never pays"), "{a}");
    // A second click opens the same agent instead of failing on the name.
    let again = ui.post("/api/agents", json!({ "action": "create_focus", "focus": "checkout" }))["agent"].clone();
    assert_eq!(again["id"], a["id"]);
    assert_eq!(ui.agents().len(), 2, "no second checkout agent");

    // The form under "Describe it yourself" starts from the app.
    let nd = ui.get("/api/agents")["new_defaults"].clone();
    assert_eq!((nd["app_name"].as_str(), nd["app"].as_str(), strs(&nd["devices"])), (Some("Shop"), Some("https://shop.example/"), vec!["web".to_owned(), "android_emulator".to_owned()]), "{nd}");

    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}
