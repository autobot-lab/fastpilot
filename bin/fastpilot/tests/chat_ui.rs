//! C4 Chat in `fastpilot ui`, with a stand-in Claude Code (tests/fake-claude-chat): the bot runs a
//! check over the real fastpilot MCP server, its proposals become cards and change nothing until
//! clicked, a card that would loosen a limit is refused (also when clicked), the bot never gets the
//! `config` tool, and the conversation continues with `--resume`. Skips without Chrome.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude-chat/claude");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| Path::new(p).exists())
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

fn request(port: u16, method: &str, path: &str, cookie: &str, body: &str, origin: bool) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let o = if origin { format!("Origin: http://127.0.0.1:{port}\r\n") } else { String::new() };
    let req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\n{o}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let status = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, raw.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default())
}

fn start_ui(home: &Path) -> Ui {
    start_ui_with(home, &[])
}

fn start_ui_with(home: &Path, env: &[(&str, &str)]) -> Ui {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    cmd.envs(env.iter().copied());
    let mut child = cmd
        .args(["ui", "--no-open"])
        .current_dir(ROOT)
        .env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_BLOCK_ACTIONS")
        .env_remove("FASTPILOT_MAX_STEPS")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env("FASTPILOT_CLAUDE", FAKE)
        .env("FAKE_CLAUDE_LOG", home.join("claude-log"))
        .env("FAKE_CHAT_TESTS", format!("{ROOT}/tests/login.md"))
        .env("FAKE_CHAT_DELAY_MS", "1200")
        .env("FASTPILOT_SCHEDULER", "launchd")
        .env("FASTPILOT_SCHEDULER_DIR", home.join("LaunchAgents"))
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

fn wait_idle(ui: &Ui) -> Value {
    let until = Instant::now() + Duration::from_secs(90);
    loop {
        let (_, body) = request(ui.port, "GET", "/api/chat", &ui.cookie, "", false);
        let v: Value = serde_json::from_str(&body).expect("json");
        if v["busy"] == false {
            return v;
        }
        assert!(Instant::now() < until, "the bot did not finish: {v}");
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn config(home: &Path) -> String {
    std::fs::read_to_string(home.join(".fastpilot/config.toml")).unwrap_or_default()
}

#[test]
fn chat_runs_checks_and_changes_only_what_the_user_clicks() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-chat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let ui = start_ui(&home);
    let post = |path: &str, body: Value| request(ui.port, "POST", path, &ui.cookie, &body.to_string(), true);

    let (st, _) = request(ui.port, "POST", "/api/chat", &ui.cookie, &json!({ "message": "halo" }).to_string(), false);
    assert!(st == 403 || st == 400, "a message needs the page's Origin: {st}");

    let (st, body) = post("/api/chat", json!({ "message": "tes login lalu buat jadwal harian, dan blokir tombol Bayar" }));
    assert_eq!(st, 200, "{body}");
    let (st, body) = post("/api/chat", json!({ "message": "satu lagi" }));
    assert!(st == 409 && body.contains("masih"), "one message at a time: {st} {body}");
    let v = wait_idle(&ui);
    let bot = &v["messages"][1];
    assert_eq!(bot["role"], "bot", "{v}");
    let tools: Vec<&str> = bot["tools"].as_array().expect("tools").iter().filter_map(Value::as_str).collect();
    assert!(tools.contains(&"session · start chat1 web") && tools.contains(&"run_flow · chat1"), "{tools:?}");
    let text = bot["text"].as_str().unwrap_or("");
    assert!(text.contains("tombol Masuk terlihat") && !text.contains("fastpilot-proposal"), "{text}");
    let cards = bot["proposals"].as_array().expect("proposals");
    assert_eq!(cards.len(), 2, "{bot}");
    let (sched, limits) = (&cards[0], &cards[1]);
    assert_eq!((sched["kind"].as_str(), sched["status"].as_str()), (Some("schedule"), Some("pending")), "{sched}");
    assert!(sched["rows"].to_string().contains("Setiap hari 09:00"), "{sched}");
    assert_eq!((limits["kind"].as_str(), limits["status"].as_str()), (Some("limits"), Some("pending")), "{limits}");
    let rows = limits["rows"].to_string();
    assert!(rows.contains("Tombol yang selalu diblokir") && rows.contains("+ Bayar") && rows.contains("Maksimal langkah per test") && rows.contains("30 (bawaan) → 20"), "plain labels, real values: {rows}");
    assert!(!rows.contains("block_actions") && !rows.contains("max_steps"), "no raw keys: {rows}");

    // Nothing changed before a click.
    assert!(!home.join(".fastpilot/schedules.json").exists());
    assert!(!config(&home).contains("block_actions"));

    let (st, body) = post("/api/chat/decide", json!({ "id": limits["id"], "apply": true }));
    assert_eq!(st, 200, "{body}");
    let cfg = config(&home);
    assert!(cfg.contains("block_actions = [\"Bayar\"]") && cfg.contains("max_steps = 20"), "{cfg}");
    let (st, body) = post("/api/chat/decide", json!({ "id": sched["id"], "apply": true }));
    assert!(st == 200 && body.contains("Isi URL webhook"), "{st} {body}");
    let stored = std::fs::read_to_string(home.join(".fastpilot/schedules.json")).expect("saved");
    assert!(stored.contains("\"login-harian\"") && stored.contains("\"webhook\": false"), "{stored}");
    assert!(home.join("LaunchAgents/dev.fastpilot.schedule.login-harian.plist").exists());
    let (st, body) = post("/api/chat/decide", json!({ "id": sched["id"], "apply": true }));
    assert!(st == 422 && body.contains("sudah diputuskan"), "{st} {body}");

    // Looser: refused on the card and when clicked anyway.
    let (st, body) = post("/api/chat", json!({ "message": "longgarkan max_steps" }));
    assert_eq!(st, 200, "{body}");
    let v = wait_idle(&ui);
    let card = &v["messages"][3]["proposals"][0];
    assert_eq!(card["status"], "refused", "{card}");
    assert!(card["note"].as_str().is_some_and(|n| n.contains("Pengaturan › Batasan") && n.contains("Maksimal langkah per test") && n.contains("20 → 500")), "{card}");
    let (st, _) = post("/api/chat/decide", json!({ "id": card["id"], "apply": true }));
    assert_eq!(st, 422);
    assert!(config(&home).contains("max_steps = 20"));

    // What Claude Code got: no config tool, the limits in its context, then --resume.
    let mut calls: Vec<Value> = std::fs::read_dir(home.join("claude-log"))
        .expect("log")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).expect("read")).expect("json"))
        .collect();
    calls.sort_by_key(|c| c["argv"].to_string().contains("--resume"));
    assert_eq!(calls.len(), 2, "{calls:?}");
    let first: Vec<&str> = calls[0]["argv"].as_array().expect("argv").iter().filter_map(Value::as_str).collect();
    let i = first.iter().position(|a| *a == "--disallowedTools").expect("disallowed");
    assert_eq!(first[i + 1], "mcp__fastpilot__config");
    let sys = first.iter().position(|a| *a == "--append-system-prompt").map(|i| first[i + 1]).expect("system prompt");
    assert!(sys.contains("fastpilot-proposal") && sys.contains("block_actions ="), "{sys}");
    let second = calls[1]["argv"].to_string();
    assert!(second.contains("\"--resume\",\"fake-chat-1\""), "{second}");

    // Already so: no card to click (was: `allow_eval_script 0 → false`).
    std::fs::write(home.join(".fastpilot/config.toml"), format!("{}allow_eval_script = false\n", config(&home))).expect("config");
    let (st, body) = post("/api/chat", json!({ "message": "matikan javascript" }));
    assert_eq!(st, 200, "{body}");
    let v = wait_idle(&ui);
    let card = &v["messages"][5]["proposals"][0];
    assert!(card["status"] == "refused" && card["note"].as_str().is_some_and(|n| n.contains("Tidak ada perubahan")), "{card}");

    let (st, body) = post("/api/chat", json!({ "reset": true }));
    assert!(st == 200 && body.contains("\"messages\":[]"), "{body}");
    drop(ui);
    let _ = std::fs::remove_dir_all(home);
}

/// Claude Code not logged in: one message, in Indonesian, with what to do.
#[test]
fn chat_says_once_that_claude_code_needs_a_login() {
    let home = std::env::temp_dir().join(format!("fp-chat-login-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let ui = start_ui_with(&home, &[("FAKE_CHAT_NOLOGIN", "1")]);
    let (st, body) = request(ui.port, "POST", "/api/chat", &ui.cookie, &json!({ "message": "halo" }).to_string(), true);
    assert_eq!(st, 200, "{body}");
    let v = wait_idle(&ui);
    let bot = &v["messages"][1];
    assert_eq!(bot["text"], "", "not repeated as the answer: {bot}");
    let err = bot["error"].as_str().unwrap_or("");
    assert!(err.starts_with("Claude Code belum login") && err.contains("`claude`") && !err.contains("Please run"), "{err}");
    drop(ui);
    let _ = std::fs::remove_dir_all(home);
}
