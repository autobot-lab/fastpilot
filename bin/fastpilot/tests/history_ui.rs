//! D7 (v0.4): the History page's API end to end. `fastpilot run` (with the stand-in Claude Code of
//! tests/fake-claude) records each suite with its test runs and report; a Chat conversation is
//! saved after the answer; the page lists both, opens a report and a conversation, deletes one,
//! several or all, and auto-deletes after N days. Everything stays in a temp project folder.
//! Skips without Chrome or python3.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const FAKE_RUN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude/claude");
const FAKE_CHAT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude-chat/claude");

fn ready() -> bool {
    let chrome = ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists());
    chrome && std::process::Command::new("python3").arg("--version").output().is_ok_and(|o| o.status.success())
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

fn request(ui: &Ui, method: &str, path: &str, body: &str) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", ui.port)).expect("connect");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
        port = ui.port,
        cookie = ui.cookie
    );
    s.write_all(req.as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let status = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let (head, body) = raw.split_once("\r\n\r\n").map(|(h, b)| (h.to_owned(), b.to_owned())).unwrap_or_default();
    (status, head, body)
}

fn get(ui: &Ui, path: &str) -> Value {
    let (st, _, body) = request(ui, "GET", path, "");
    assert_eq!(st, 200, "{path}: {body}");
    serde_json::from_str(&body).expect("json")
}

fn post(ui: &Ui, body: Value) -> (u16, Value) {
    let (st, _, text) = request(ui, "POST", "/api/history", &body.to_string());
    (st, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn start_ui(home: &Path, fp: &Path) -> Ui {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(ROOT)
        .env("HOME", home)
        .env("FASTPILOT_DIR", fp)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env("FASTPILOT_CLAUDE", FAKE_CHAT)
        .env("FAKE_CLAUDE_LOG", home.join("chat-log"))
        .env("FAKE_CHAT_TESTS", format!("{ROOT}/tests/login.md"))
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

/// `fastpilot run tests/login.md` with the stand-in Claude Code, recording into `fp`.
fn run_suite(home: &Path, fp: &Path, log: &str, env: &[(&str, &str)]) -> String {
    std::fs::create_dir_all(home.join(log)).expect("log dir");
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    cmd.args(["run", "tests/login.md"])
        .current_dir(ROOT)
        .env("HOME", home)
        .env("FASTPILOT_CONFIG", home.join("config.toml"))
        .env("FASTPILOT_DIR", fp)
        .env("FASTPILOT_CLAUDE", FAKE_RUN)
        .env("FAKE_CLAUDE_LOG", home.join(log))
        .env("DEMO_PASS", "rahasia123")
        .env("FASTPILOT_VARS", "DEMO_PASS")
        .env("FASTPILOT_RETRY_BUDGET", "1")
        .env_remove("ANTHROPIC_API_KEY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

fn run_folders(fp: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(fp.join("runs")).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

#[test]
fn history_lists_opens_and_deletes_runs_and_conversations() {
    if !ready() {
        return eprintln!("skipping: no Chrome or python3");
    }
    let home = std::env::temp_dir().join(format!("fp-history-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");
    let fp = home.join("project-fastpilot");

    // Two suites: one passing, one failing after a retry (two run folders, one suite).
    let out = run_suite(&home, &fp, "claude1", &[("FASTPILOT_RUN_TITLE", "Login every 2 hours")]);
    assert!(out.contains("1 passed, 0 failed"), "{out}");
    let out = run_suite(&home, &fp, "claude2", &[("FAKE_CLAUDE_FAIL", "1")]);
    assert!(out.contains("2 attempts"), "{out}");
    assert_eq!(run_folders(&fp).len(), 3, "1 + 2 attempts");
    // A single test run outside any suite (as Claude Code's `report` tool writes one).
    let solo = fp.join("runs/20261120-090000-solo-check");
    std::fs::create_dir_all(&solo).expect("solo");
    std::fs::write(solo.join("report.md"), "# solo-check — pass\n").expect("md");
    std::fs::write(solo.join("report.html"), "<p>solo report</p>").expect("html");

    let ui = start_ui(&home, &fp);
    let h = get(&ui, "/api/history");
    let runs = h["runs"].as_array().expect("runs").clone();
    assert_eq!(runs.len(), 3, "two suites and the single run; a suite's runs are inside it: {h}");
    let named = runs.iter().find(|r| r["title"] == "Login every 2 hours").expect("schedule-style title");
    assert_eq!((named["tests"].as_u64(), named["passed"].as_u64(), named["has_report"].as_bool()), (Some(1), Some(1), Some(true)), "{named}");
    assert_eq!(named["platforms"], json!(["web"]));
    let failed = runs.iter().find(|r| r["failed"] == 1).expect("failed suite");
    assert_eq!(failed["title"], "Login berhasil", "one test: its title");
    assert!(runs.iter().any(|r| r["id"] == "run:20261120-090000-solo-check" && r["passed"] == 1));
    assert_eq!(h["keep_days"], Value::Null, "auto-delete is off until the user picks it");

    // Open: the suite's report, sandboxed; never a path from the request.
    let id = named["id"].as_str().expect("id").to_owned();
    let (st, head, body) = request(&ui, "GET", &format!("/api/history/report?id={id}"), "");
    assert_eq!(st, 200);
    assert!(head.to_lowercase().contains("content-security-policy: sandbox") && body.contains("Login berhasil"), "{head}");
    for bad in ["run:..%2Fsecret", "suite:..%2F..%2Fx", "run:", "x"] {
        let (st, _, _) = request(&ui, "GET", &format!("/api/history/report?id={bad}"), "");
        assert_eq!(st, 404, "{bad}");
    }

    // A Chat conversation is kept after the answer.
    let (st, _, _) = request(&ui, "POST", "/api/chat", &json!({ "message": "test the login page" }).to_string());
    assert_eq!(st, 200);
    let until = Instant::now() + Duration::from_secs(60);
    while get(&ui, "/api/chat")["busy"] != false {
        assert!(Instant::now() < until, "the bot did not finish");
        std::thread::sleep(Duration::from_millis(300));
    }
    let chats = get(&ui, "/api/history")["chats"].clone();
    assert_eq!((chats.as_array().map(Vec::len), chats[0]["title"].as_str()), (Some(1), Some("test the login page")), "{chats}");
    let cid = chats[0]["id"].as_str().expect("chat id").to_owned();
    let conv = get(&ui, &format!("/api/history/chat?id={cid}"));
    assert_eq!(conv["messages"][0]["text"], "test the login page");
    assert!(conv["messages"].as_array().is_some_and(|m| m.len() >= 2), "{conv}");

    // Delete one suite: its run folders and report go; the rest stays.
    let failed_id = failed["id"].as_str().expect("id").to_owned();
    let before = run_folders(&fp).len();
    let (st, r) = post(&ui, json!({ "action": "delete", "runs": [failed_id], "chats": [] }));
    assert_eq!((st, r["deleted"]["runs"].as_u64()), (200, Some(1)), "{r}");
    assert_eq!(run_folders(&fp).len(), before - 2, "both attempts of the failed test");
    assert_eq!(get(&ui, "/api/history")["runs"].as_array().map(Vec::len), Some(2));
    // A delete only comes from the page itself (JSON from our own origin).
    let mut s = TcpStream::connect(("127.0.0.1", ui.port)).expect("connect");
    let body = json!({ "action": "delete_all" }).to_string();
    s.write_all(format!("POST /api/history HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nCookie: {}\r\nOrigin: http://evil.example\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", ui.port, ui.cookie, body.len()).as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    assert!(!raw.starts_with("HTTP/1.1 200"), "{raw}");
    assert_eq!(get(&ui, "/api/history")["runs"].as_array().map(Vec::len), Some(2), "nothing deleted");

    // Auto-delete: a bad value is refused; 30 days keeps today's items; delete all empties it.
    let (st, _) = post(&ui, json!({ "action": "keep_days", "days": "soon" }));
    assert_eq!(st, 400);
    let (st, r) = post(&ui, json!({ "action": "keep_days", "days": 30 }));
    assert_eq!((st, r["keep_days"].as_u64(), r["deleted"]["runs"].as_u64()), (200, Some(30), Some(0)), "{r}");
    // An item from 40 days ago goes as soon as History is read.
    let old = fp.join("runs/20261001-090000-old");
    std::fs::create_dir_all(&old).expect("old");
    std::fs::write(old.join("report.md"), "# old — pass\n").expect("md");
    let forty_days_ago = std::time::SystemTime::now() - Duration::from_secs(40 * 86_400);
    std::fs::File::options().write(true).open(old.join("report.md")).and_then(|f| f.set_modified(forty_days_ago)).expect("mtime");
    std::fs::File::open(&old).and_then(|f| f.set_modified(forty_days_ago)).expect("dir mtime");
    let h = get(&ui, "/api/history");
    assert!(!old.exists() && h["runs"].as_array().map(Vec::len) == Some(2), "{h}");
    let (st, r) = post(&ui, json!({ "action": "delete_all" }));
    assert_eq!((st, r["deleted"]["runs"].as_u64(), r["deleted"]["chats"].as_u64()), (200, Some(2), Some(1)), "{r}");
    let h = get(&ui, "/api/history");
    assert_eq!((h["runs"].as_array().map(Vec::len), h["chats"].as_array().map(Vec::len)), (Some(0), Some(0)));
    assert!(run_folders(&fp).is_empty(), "every run folder is gone");
    // Tests stay: the case file the suites ran is untouched.
    assert!(Path::new(ROOT).join("tests/login.md").exists());
    // Chat starts over: the deleted conversation is not saved again.
    assert_eq!(get(&ui, "/api/chat")["messages"].as_array().map(Vec::len), Some(0));

    // The page and its script are served.
    let (st, _, page) = request(&ui, "GET", "/", "");
    assert!(st == 200 && page.contains("#/history") && page.contains("/assets/history.js"));
    let (st, _, js) = request(&ui, "GET", "/assets/history.js", "");
    assert!(st == 200 && js.contains("PAGES.history") && js.contains("This cannot be undone. Your tests and schedules stay."));
    // Its names stay inside one function: a top-level name that app.js also has (PLATFORM did)
    // stops the whole script from loading.
    let code: Vec<&str> = js.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("//")).collect();
    assert!(code.first() == Some(&"(() => {") && code.last() == Some(&"})();"), "history.js is wrapped in one function");
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}
