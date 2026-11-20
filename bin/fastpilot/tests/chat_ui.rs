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
    start_ui_in(home, Path::new(ROOT), env)
}

fn start_ui_in(home: &Path, cwd: &Path, env: &[(&str, &str)]) -> Ui {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    cmd.envs(env.iter().copied());
    let mut child = cmd
        .args(["ui", "--no-open"])
        .current_dir(cwd)
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
    assert!(sched["rows"].to_string().contains("daily 09:00"), "{sched}");
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
    // Added to the built-in list (v0.3.2), not in place of it.
    assert!(cfg.contains("block_actions = [\"default\", \"Bayar\"]") && cfg.contains("max_steps = 20"), "{cfg}");
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

/// D5: "every 2 hours, 08:00–20:00" from Chat: a card first, then on click the OS entry with the
/// seven times; an N outside 1–24 is refused on the card.
#[test]
fn chat_schedules_every_n_hours_after_a_click() {
    let home = std::env::temp_dir().join(format!("fp-chat-hours-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let ui = start_ui_with(&home, &[("FAKE_CHAT_DELAY_MS", "0")]);
    let post = |path: &str, body: Value| request(ui.port, "POST", path, &ui.cookie, &body.to_string(), true);
    let (st, body) = post("/api/chat", json!({ "message": "run the checkout tests every 2 hours, 8 to 8, and also every 30 hours" }));
    assert_eq!(st, 200, "{body}");
    let v = wait_idle(&ui);
    let cards = v["messages"][1]["proposals"].as_array().cloned().expect("cards");
    assert_eq!(cards.len(), 2, "{v}");
    assert_eq!((cards[0]["kind"].as_str(), cards[0]["status"].as_str()), (Some("schedule"), Some("pending")), "{}", cards[0]);
    assert!(cards[0]["rows"].to_string().contains("every 2 hours, 08:00–20:00"), "{}", cards[0]);
    assert!(cards[1]["status"] == "refused" && cards[1]["note"].as_str().is_some_and(|n| n.contains("1 to 24")), "{}", cards[1]);
    let plist = home.join("LaunchAgents/dev.fastpilot.schedule.checkout-every-2-hours.plist");
    assert!(!plist.exists(), "nothing before the click");
    let (st, body) = post("/api/chat/decide", json!({ "id": cards[0]["id"], "apply": true }));
    assert_eq!(st, 200, "{body}");
    let text = std::fs::read_to_string(&plist).expect("OS entry");
    assert_eq!(text.matches("<key>Hour</key>").count(), 7, "{text}");
    assert!(text.contains("<key>Hour</key><integer>8</integer>") && text.contains("<key>Hour</key><integer>20</integer>"), "{text}");
    drop(ui);
    let _ = std::fs::remove_dir_all(home);
}

/// D1: one sentence in Chat -> a "Save test & schedule" card -> nothing written until Save &
/// schedule -> the file (in the project's tests folder, written by the server from the card's data),
/// the schedule and its OS entry, and a first run that passes and shows in Live. The same name is not
/// overwritten; dangerous file names are refused and nothing lands outside the tests folder.
#[test]
fn chat_saves_a_test_and_its_schedule_only_after_a_click() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-chat-d1-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let project = home.join("shop");
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    std::fs::create_dir_all(project.join("examples/demo")).expect("project");
    for f in std::fs::read_dir(format!("{ROOT}/examples/demo")).expect("demo").flatten() {
        if f.path().is_file() {
            std::fs::copy(f.path(), project.join("examples/demo").join(f.file_name())).expect("copy");
        }
    }
    let ui = start_ui_in(&home, &project, &[("FAKE_CHAT_DELAY_MS", "0"), ("FAKE_CLAUDE_DELAY_MS", "600"), ("DEMO_PASS", "rahasia123"), ("FASTPILOT_VARS", "DEMO_PASS"), ("FASTPILOT_RETRY_BUDGET", "0"), ("FASTPILOT_NOTIFY_DRY", "1")]);
    let post = |path: &str, body: Value| request(ui.port, "POST", path, &ui.cookie, &body.to_string(), true);
    let get = |path: &str| -> Value { serde_json::from_str(&request(ui.port, "GET", path, &ui.cookie, "", false).1).expect("json") };

    let (st, body) = post("/api/chat", json!({ "message": "write a test for the demo login, save it and run it every 2 hours" }));
    assert_eq!(st, 200, "{body}");
    let v = wait_idle(&ui);
    let card = v["messages"][1]["proposals"][0].clone();
    assert_eq!((card["kind"].as_str(), card["status"].as_str(), card["schedules"].as_bool()), (Some("test"), Some("pending"), Some(true)), "{card}");
    assert_eq!(card["title"], "Login with the demo account", "the title, not a path: {card}");
    assert_eq!(card["steps"].as_array().map(Vec::len), Some(2), "{card}");
    assert!(card["rows"].to_string().contains("every 2 hours, 08:00–20:00") && card["rows"].to_string().contains("notify on this computer"), "{card}");
    assert!(card["detail"].as_str().is_some_and(|d| d.contains("tests/login-with-the-demo-account.md")), "the path only in the details: {card}");
    // Nothing is written before the click, whatever the bot said.
    assert!(!project.join("tests").exists() && !home.join(".fastpilot/schedules.json").exists());

    let (st, body) = post("/api/chat/decide", json!({ "id": card["id"], "apply": true, "mode": "schedule" }));
    assert_eq!(st, 200, "{body}");
    assert!(body.contains("Saved and scheduled") && body.contains("first run has started"), "{body}");
    let file = project.join("tests/login-with-the-demo-account.md");
    let text = std::fs::read_to_string(&file).expect("test file");
    assert_eq!(text, "# Login with the demo account\nid: login-with-the-demo-account\nplatform: web\ntarget: examples/demo/login.html\n- Open the login page and sign in as a@b.com with ${DEMO_PASS}\n- Check that the Dashboard shows Budi\n");
    assert!(!text.contains("rahasia123"));
    let stored = std::fs::read_to_string(home.join(".fastpilot/schedules.json")).expect("schedule");
    assert!(stored.contains(&file.to_string_lossy().into_owned()) && stored.contains("\"hours\": 2"), "{stored}");
    let plist = std::fs::read_to_string(home.join("LaunchAgents/dev.fastpilot.schedule.login-with-the-demo-account.plist")).expect("OS entry");
    assert_eq!(plist.matches("<key>Hour</key>").count(), 7, "{plist}");

    // The first run: seen in Live while it runs, then passed.
    let (mut in_live, mut last) = (false, Value::Null);
    let until = Instant::now() + Duration::from_secs(120);
    while Instant::now() < until {
        let live = get("/api/live");
        in_live |= live["sessions"].as_array().is_some_and(|s| !s.is_empty());
        let sched = get("/api/schedules");
        let row = &sched["schedules"][0];
        if row["running"] == false && row["last"]["passed"].is_boolean() {
            last = row["last"].clone();
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(last["passed"], true, "first run: {last}");
    assert_eq!((last["ok"].as_u64(), last["total"].as_u64()), (Some(1), Some(1)), "{last}");
    assert!(in_live, "the run's session showed in Live");
    // Run now from the saved card.
    let v = get("/api/chat");
    assert_eq!(v["messages"][1]["proposals"][0]["status"], "applied");
    let (st, body) = post("/api/chat/run", json!({ "id": card["id"] }));
    assert!(st == 200 && body.contains("Running now"), "{st} {body}");

    // The same test again: not overwritten.
    let (st, _) = post("/api/chat", json!({ "message": "save it again" }));
    assert_eq!(st, 200);
    let v = wait_idle(&ui);
    let again = v["messages"][3]["proposals"][0].clone();
    assert!(again["note"].as_str().is_some_and(|n| n.contains("already saved")), "{again}");
    let (st, body) = post("/api/chat/decide", json!({ "id": again["id"], "apply": true, "mode": "test" }));
    assert!(st == 422 && body.contains("already saved"), "{st} {body}");
    assert_eq!(std::fs::read_to_string(&file).expect("file"), text, "unchanged");
    // ... saved under another name with Edit; a dangerous name typed there is refused too.
    let (st, body) = post("/api/chat/decide", json!({ "id": again["id"], "apply": true, "mode": "test", "edit": { "id": "../escape" } }));
    assert!(st == 422 && body.contains("file name"), "{st} {body}");
    let (st, body) = post("/api/chat/decide", json!({ "id": again["id"], "apply": true, "mode": "test", "edit": { "title": "Login, second account", "id": "" } }));
    assert_eq!(st, 200, "{body}");
    assert!(project.join("tests/login-second-account.md").exists());

    // Dangerous file names from the bot: refused cards; nothing outside the tests folder.
    let (st, _) = post("/api/chat", json!({ "message": "save tests named ../ and /tmp" }));
    assert_eq!(st, 200);
    let v = wait_idle(&ui);
    let bad = v["messages"][5]["proposals"].as_array().cloned().unwrap_or_default();
    assert_eq!(bad.len(), 2, "{v}");
    for b in &bad {
        assert_eq!(b["status"], "refused", "{b}");
        let (st, _) = post("/api/chat/decide", json!({ "id": b["id"], "apply": true, "mode": "test" }));
        assert_eq!(st, 422);
    }
    assert!(!home.join("outside.md").exists() && !Path::new("/tmp/fastpilot-evil.md").exists());
    let names: Vec<String> = std::fs::read_dir(project.join("tests")).expect("tests").flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names.len(), 2, "{names:?}");

    // The bot never had a file-writing tool: only fastpilot's MCP tools, without `config`.
    let call = std::fs::read_dir(home.join("claude-log")).expect("log").flatten().map(|e| std::fs::read_to_string(e.path()).unwrap_or_default()).find(|t| t.contains("fastpilot bot")).expect("chat call");
    let call: Value = serde_json::from_str(&call).expect("json");
    let argv: Vec<&str> = call["argv"].as_array().expect("argv").iter().filter_map(Value::as_str).collect();
    let at = |flag: &str| argv.iter().position(|a| *a == flag).map(|i| argv[i + 1]);
    assert_eq!((at("--tools"), at("--allowedTools"), at("--disallowedTools")), (Some(""), Some("mcp__fastpilot"), Some("mcp__fastpilot__config")), "{argv:?}");
    drop(ui);
    let _ = std::fs::remove_dir_all(home);
}

/// D1: `fastpilot ui` started in the home folder (not a project): the card says where the test
/// would be saved before anything is written.
#[test]
fn chat_says_where_a_test_goes_outside_a_project() {
    let home = std::env::temp_dir().join(format!("fp-chat-d1-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let ui = start_ui_in(&home, &home, &[("FAKE_CHAT_DELAY_MS", "0")]);
    let (st, body) = request(ui.port, "POST", "/api/chat", &ui.cookie, &json!({ "message": "write a test for the demo login and save it" }).to_string(), true);
    assert_eq!(st, 200, "{body}");
    let v = wait_idle(&ui);
    let card = &v["messages"][1]["proposals"][0];
    assert_eq!((card["status"].as_str(), card["schedules"].as_bool()), (Some("pending"), None), "{card}");
    let note = card["note"].as_str().unwrap_or("");
    assert!(note.contains("not a project folder") && note.contains(&home.join("tests").to_string_lossy().into_owned()), "{note}");
    assert!(!home.join("tests").exists(), "nothing before the click");
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
