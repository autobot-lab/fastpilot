//! D6 (v0.4) stage 1: the Memory page's API end to end, in a temp project (its own tests/ and
//! .fastpilot): one row per test in words (Reliable, Unstable, Failing, Set aside, New), Set aside
//! / Bring back, and Forget (the remembered path goes, the test file stays). Only the page's own
//! requests change anything.

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

fn start_ui(home: &Path, project: &Path) -> Ui {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(project)
        .env("HOME", home)
        .env("FASTPILOT_DIR", project.join(".fastpilot"))
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_MEMORY")
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

fn memory(dir: &Path, id: &str, history: &[bool]) {
    let fails_at_end = history.iter().rev().take_while(|p| !**p).count();
    let m = json!({
        "test_id": id, "flow": "- tapOn: Shop\n- tapOn: Add\n- assertVisible: Cart\n", "start": "https://shop.example",
        "screens": ["screen \"Shop\" (https://shop.example)", "screen \"Cart\" (https://shop.example/cart)"],
        "duration_ms": 1200, "pass_count": history.iter().filter(|p| **p).count(), "fail_count": history.iter().filter(|p| !**p).count(),
        "last_pass": "2026-11-20 07:02:00", "failed_at": [], "history": history, "consecutive_failures": fails_at_end, "stale": fails_at_end >= 3, "classes": []
    });
    std::fs::write(dir.join(format!("memory/{id}.json")), m.to_string()).expect("memory");
}

fn row<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["tests"].as_array().and_then(|t| t.iter().find(|r| r["id"] == id)).unwrap_or_else(|| panic!("{id} in {v}"))
}

#[test]
fn memory_page_lists_tests_in_words_and_changes_only_on_the_users_click() {
    let home = std::env::temp_dir().join(format!("fp-memory-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let project = home.join("shop");
    let fp = project.join(".fastpilot");
    std::fs::create_dir_all(project.join("tests")).expect("tests");
    std::fs::create_dir_all(fp.join("memory")).expect("memory");
    for (id, title, platform) in [("voucher", "Checkout with voucher SAVE10", "web"), ("login", "Login with one-time code", "web"), ("export", "Admin export", "web"), ("notify", "Notifications on Android", "android"), ("broken", "Broken search", "web")] {
        std::fs::write(project.join(format!("tests/{id}.md")), format!("# {title}\nid: {id}\nplatform: {platform}\ntarget: https://shop.example\n- Open the shop\n")).expect("test");
    }
    memory(&fp, "voucher", &[true, false, true, true, false, true, true, true, true, false, true]);
    memory(&fp, "login", &[true, true, true]);
    memory(&fp, "export", &[true, false]);
    memory(&fp, "broken", &[true, false, false, false]);
    memory(&fp, "old-test", &[true]);
    std::fs::write(fp.join("quarantine.json"), json!({ "export": { "reason": "manual", "since": "2026-11-17 10:00:00", "auto": false } }).to_string()).expect("q");

    let ui = start_ui(&home, &project);
    let own = format!("http://127.0.0.1:{}", ui.port);
    let (st, body) = request(&ui, "GET", "/api/memory", "", &own);
    assert_eq!(st, 200, "{body}");
    let v: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["mode"], "on");
    let voucher = row(&v, "voucher");
    assert_eq!((voucher["status"].as_str(), voucher["title"].as_str()), (Some("unstable"), Some("Checkout with voucher SAVE10")));
    assert_eq!(voucher["summary"], "Remembers the last path that worked (3 steps). Failed 3 of the last 10 runs.");
    assert_eq!(voucher["path"], json!(["Shop", "Cart"]));
    assert_eq!(row(&v, "login")["status"], "reliable");
    assert_eq!(row(&v, "export")["status"], "set_aside");
    assert_eq!(row(&v, "export")["summary"], "Set aside since 17 Nov 2026. It still runs, but does not turn a run red.");
    assert_eq!(row(&v, "notify")["status"], "new");
    assert_eq!(row(&v, "broken")["status"], "failing");
    let orphan = row(&v, "old-test");
    assert_eq!((orphan["orphan"].as_bool(), orphan["platform"].as_str()), (Some(true), Some("")), "memory of a test whose file is gone");

    // Set aside, then bring back: quarantine.json follows.
    let change = |action: &str, id: &str| {
        let (st, body) = request(&ui, "POST", "/api/memory", &json!({ "action": action, "id": id }).to_string(), &own);
        (st, serde_json::from_str::<Value>(&body).unwrap_or(Value::String(body)))
    };
    let (st, r) = change("set_aside", "voucher");
    assert_eq!(st, 200, "{r}");
    assert_eq!(row(&r, "voucher")["status"], "set_aside");
    let q = std::fs::read_to_string(fp.join("quarantine.json")).expect("q");
    assert!(q.contains("\"voucher\"") && q.contains("set aside on the Memory page"), "{q}");
    let (_, r) = change("bring_back", "voucher");
    assert_eq!(row(&r, "voucher")["status"], "unstable");
    assert!(!std::fs::read_to_string(fp.join("quarantine.json")).expect("q").contains("\"voucher\""));

    // Forget: the remembered path goes, the test file stays.
    let test_file = std::fs::read_to_string(project.join("tests/login.md")).expect("test");
    let (st, r) = change("forget", "login");
    assert_eq!(st, 200, "{r}");
    assert_eq!(r["ok"], "Forgotten: the next run works the steps out again.");
    assert_eq!(row(&r, "login")["status"], "new");
    assert!(!fp.join("memory/login.json").exists());
    assert_eq!(std::fs::read_to_string(project.join("tests/login.md")).expect("test"), test_file, "the test itself is not changed");
    let (_, r) = change("forget", "login");
    assert_eq!(r["ok"], "There was nothing to forget.");

    // Ids, never paths; unknown actions; and only the page's own origin.
    let (st, _) = change("forget", "../tests/login");
    assert_eq!(st, 400);
    let (st, _) = change("wipe", "voucher");
    assert_eq!(st, 400);
    let (st, _) = request(&ui, "POST", "/api/memory", &json!({ "action": "forget", "id": "voucher" }).to_string(), "http://evil.example");
    assert_ne!(st, 200);
    assert!(fp.join("memory/voucher.json").exists(), "nothing changed from another origin");

    // The page and its script.
    let (st, page) = request(&ui, "GET", "/", "", &own);
    assert!(st == 200 && page.contains("#/memory") && page.contains("/assets/memory.js"));
    let (st, js) = request(&ui, "GET", "/assets/memory.js", "", &own);
    assert!(st == 200 && js.contains("PAGES.memory") && js.contains("Set aside") && js.contains("Forget this"));
    let code: Vec<&str> = js.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("//")).collect();
    assert!(code.first() == Some(&"(() => {") && code.last() == Some(&"})();"), "memory.js is wrapped in one function");
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}
