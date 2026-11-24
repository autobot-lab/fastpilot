//! v0.7.1 FC6: removing is kept 7 days and Undo is real. Removing an app (from the terminal or the
//! dashboard) and undoing it brings back its tests, its history, its password and its schedule
//! (the system scheduler entry too). Only the newest change of one thing can be undone ("it changed
//! later"); a schedule comes back in the state it had ("(paused)"); after 7 days it is no longer
//! kept. The words are FastUI/UX's (TEXT-terminal-mcp.md §2.5, §4).

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn home() -> PathBuf {
    let h = std::env::temp_dir().join(format!("fp-undo-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&h);
    std::fs::create_dir_all(&h).expect("home");
    std::fs::canonicalize(&h).expect("home")
}

fn cmd(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    c.current_dir(home)
        .env("HOME", home)
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_SCHEDULER", "launchd")
        .env("FASTPILOT_SCHEDULER_DIR", home.join("agents"))
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env("FASTPILOT_ADB_SERVER", "127.0.0.1:1")
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env_remove("FASTPILOT_VARS");
    c
}

/// `fastpilot ARGS`: (exit code, output, the Undo id it printed).
fn fp(home: &Path, args: &[&str], input: &str) -> (i32, String, Option<String>) {
    let mut c = cmd(home).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("fastpilot");
    c.stdin.take().expect("stdin").write_all(input.as_bytes()).expect("input");
    let o = c.wait_with_output().expect("wait");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let id = text.lines().find_map(|l| l.strip_prefix("  Undo: fastpilot undo ").map(str::to_owned));
    (o.status.code().unwrap_or(-1), text, id)
}

fn apps_json(home: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(home.join("fastpilot/apps.json")).expect("apps.json")).expect("json")
}

fn schedules(home: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(home.join(".fastpilot/schedules.json")).unwrap_or_else(|_| "{}".into())).unwrap_or(Value::Null)
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

impl Ui {
    fn start(home: &Path) -> Ui {
        let mut child = cmd(home).args(["ui", "--no-open"]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("ui");
        let mut line = String::new();
        BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("link");
        let rest = line.trim().strip_prefix("http://127.0.0.1:").expect("link").to_owned();
        let (port, code) = rest.split_once("/?code=").expect("code");
        let port: u16 = port.parse().expect("port");
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        s.write_all(format!("GET /?code={code} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes()).expect("write");
        let mut head = String::new();
        s.read_to_string(&mut head).expect("read");
        let cookie = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().split(';').next().unwrap_or("").to_owned())).expect("cookie");
        Ui { child, port, cookie }
    }
    fn req(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
        let b = body.map(|b| b.to_string()).unwrap_or_default();
        let origin = if method == "POST" { format!("Origin: http://127.0.0.1:{}\r\nContent-Type: application/json\r\n", self.port) } else { String::new() };
        s.write_all(format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nCookie: {}\r\n{origin}Connection: close\r\nContent-Length: {}\r\n\r\n{b}", self.port, self.cookie, b.len()).as_bytes()).expect("write");
        let mut out = String::new();
        s.read_to_string(&mut out).expect("read");
        let st = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let text = out.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default();
        (st, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
}

#[test]
fn removing_is_kept_7_days_and_undo_brings_everything_back() {
    let home = home();
    assert_eq!(fp(&home, &["undo"], "").1, "Nothing to undo.\n");
    assert_eq!(fp(&home, &["undo", "--list"], "").1, "Nothing recent to undo or bring back.\n");
    let (code, out, _) = fp(&home, &["undo", "9z9z"], "");
    assert_eq!((code, out.as_str()), (1, "✗ No change with id “9z9z”. Run fastpilot undo --list to see them.\n"));

    // An app with a saved test, a run in its history, a password and a schedule that is on.
    assert_eq!(fp(&home, &["app", "add", "https://old.example"], "").0, 0);
    let dir = home.join("fastpilot/apps/old");
    std::fs::create_dir_all(dir.join("tests")).expect("tests");
    std::fs::write(dir.join("tests/login.md"), "# Check the login\nid: login\nplatform: web\ntarget: https://old.example\n- Open the page\n").expect("test");
    let history = fastpilot_memory::history::History::new(dir.join(".fastpilot"));
    let suite: fastpilot_memory::history::Suite = serde_json::from_value(json!({ "id": history.new_suite_id(), "title": "Run all tests", "platforms": ["web"], "started_ms": 1, "finished_ms": 2, "passed": 1, "failed": 0 })).expect("suite");
    history.record_suite(&suite).expect("history");
    assert_eq!(fp(&home, &["secret", "set", "TEST_PASSWORD"], "old-pass-1\n").0, 0);
    assert_eq!(fp(&home, &["schedule", "add", "--nightly"], "").0, 0);
    let entries = || std::fs::read_dir(home.join("agents")).map(|d| d.count()).unwrap_or(0);
    assert_eq!(entries(), 1);

    // Remove: no question, kept 7 days; its schedule stops running; its folder stays.
    let (code, out, id) = fp(&home, &["app", "remove", "Old"], "");
    let id = id.expect("an Undo id");
    assert_eq!((code, out.lines().next()), (0, Some("✓ Removed app “Old” · 1 test, 1 run, 1 password, 1 schedule · kept 7 days")), "{out}");
    assert_eq!(id.len(), 4);
    assert!(apps_json(&home)["apps"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(entries(), 0, "its schedule does not run while the app is removed");
    assert!(dir.join("tests/login.md").is_file());
    let (_, list, _) = fp(&home, &["undo", "--list"], "");
    assert!(list.starts_with(&format!("You can undo\n  {id}   Removed app “Old” · 1 test, 1 run, 1 password, 1 schedule · kept 7 days    today\n")) && list.ends_with("Recently removed (kept 7 days)\n  App “Old”    7 days left\n"), "{list}");

    // Undo: the app, its tests, history, password and schedule are back.
    let (code, out, _) = fp(&home, &["undo", &id], "");
    assert_eq!((code, out.as_str()), (0, "✓ Restored app “Old” · tests, history, passwords and schedules are back\n"));
    assert_eq!((apps_json(&home)["apps"][0]["name"].as_str(), apps_json(&home)["current"].as_str()), (Some("Old"), Some("old")));
    assert_eq!(fp(&home, &["secret", "list"], "").1, "Passwords for Old (names only, values are never shown)\n  TEST_PASSWORD\n");
    assert_eq!(history.runs().len(), 1);
    assert_eq!(entries(), 1, "its schedule runs again");
    assert_eq!(fp(&home, &["undo", &id], "").1, "Nothing to undo.\n");

    // Only the newest change of one thing: an older one says it changed later.
    let (_, _, paused) = fp(&home, &["schedule", "pause", "Every night"], "");
    let (_, _, resumed) = fp(&home, &["schedule", "resume", "Every night"], "");
    let (code, out, _) = fp(&home, &["undo", paused.as_deref().expect("id")], "");
    assert_eq!((code, out.as_str()), (1, "✗ Can't undo “Paused schedule “Every night” · its history is kept”: it changed later. Change it back yourself, or look in Recently removed: fastpilot undo --list.\n"));
    let list = fp(&home, &["undo", "--list"], "").1;
    assert!(list.lines().any(|l| l.contains("Paused schedule “Every night” · its history is kept") && l.ends_with("today · Changed later")), "{list}");
    assert_eq!(fp(&home, &["undo", resumed.as_deref().expect("id")], "").1, "✓ Undone · Resumed schedule “Every night” · next run 02:00\n");
    assert_eq!(entries(), 0, "paused again");

    // A paused schedule that is removed comes back paused; Undo with no id is the newest change.
    let (_, out, _) = fp(&home, &["schedule", "remove", "Every night"], "");
    assert!(out.starts_with("✓ Removed schedule “Every night” · kept 7 days\n  Undo: fastpilot undo "), "{out}");
    assert!(schedules(&home)["schedules"].as_array().is_none_or(Vec::is_empty));
    assert_eq!(fp(&home, &["undo"], "").1, "✓ Restored schedule “Every night” (paused)\n");
    assert_eq!((schedules(&home)["schedules"][0]["enabled"].as_bool(), entries()), (Some(false), 0));

    // After 7 days it is no longer kept.
    let (_, _, gone) = fp(&home, &["schedule", "remove", "Every night"], "");
    let path = home.join("fastpilot/undo.json");
    let mut store: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("undo.json")).expect("json");
    let eight_days_ago = store["removed"][0]["removed_ms"].as_u64().expect("ms") - 8 * 24 * 3600 * 1000;
    store["removed"][0]["removed_ms"] = json!(eight_days_ago);
    for c in store["changes"].as_array_mut().expect("changes") {
        if c["id"] == gone.as_deref().expect("id") {
            c["at_ms"] = json!(eight_days_ago);
        }
    }
    std::fs::write(&path, store.to_string()).expect("write");
    let (code, out, _) = fp(&home, &["undo", gone.as_deref().expect("id")], "");
    assert_eq!((code, out.as_str()), (1, "✗ “Every night” was removed 8 days ago and is no longer kept.\n"));

    // The dashboard: removing on its page is kept too, and History › Recently removed restores it.
    let ui = Ui::start(&home);
    let (st, r) = ui.req("POST", "/api/apps", Some(json!({ "action": "remove", "id": "old" })));
    assert!(st == 200 && r["message"].as_str().is_some_and(|m| m.starts_with("Removed app “Old”")) && r["undo"].as_str().is_some_and(|u| u.len() == 4), "{r}");
    let (_, list) = ui.req("GET", "/api/removed", None);
    assert_eq!((list["items"][0]["label"].as_str(), list["items"][0]["days_left"].as_u64()), (Some("App “Old”"), Some(7)), "{list}");
    let (st, r) = ui.req("POST", "/api/removed", Some(json!({ "action": "restore", "kind": "app", "id": "old" })));
    assert_eq!((st, r["message"].as_str()), (200, Some("Restored app “Old” · tests, history, passwords and schedules are back")), "{r}");
    assert_eq!(apps_json(&home)["apps"][0]["name"], "Old");
    let (_, list) = ui.req("GET", "/api/removed", None);
    assert!(list["items"].as_array().is_some_and(Vec::is_empty), "{list}");
    let (st, r) = ui.req("POST", "/api/undo", Some(json!({ "id": "9z9z" })));
    assert_eq!((st, r["code"].as_str()), (422, Some("not_found")), "{r}");
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}
