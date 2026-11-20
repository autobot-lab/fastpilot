//! C3 Jadwal end to end, against stand-ins: the page saves a schedule (its OS entry is written to
//! a test folder instead of being loaded into launchd), the webhook URL is kept secret, a test
//! notification and a failing scheduled run reach a fake Slack webhook and a fake GitHub API, and
//! turning it off / deleting removes the entry. Bad input is refused with a reason.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const FAKE_CLAUDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude/claude");

/// A tiny HTTP server that records `METHOD path\nheaders\n\nbody` of each request.
fn fake_server(reply: &'static str) -> (u16, Arc<Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().expect("addr").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut r = BufReader::new(s.try_clone().expect("clone"));
            let mut head = String::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                head.push_str(&line);
            }
            let mut body = vec![0u8; len];
            let _ = r.read_exact(&mut body);
            log.lock().expect("log").push(format!("{head}\n{}", String::from_utf8_lossy(&body)));
            let mut s = s;
            let _ = s.write_all(format!("HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes());
        }
    });
    (port, seen)
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
    let extra = if origin { format!("Origin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\n") } else { "Content-Type: application/json\r\n".into() };
    let req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\n{extra}Content-Length: {}\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let status = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, raw.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default())
}

fn env_for(cmd: &mut std::process::Command, home: &Path, entries: &Path, github: u16) {
    cmd.env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_SCHEDULER", "launchd")
        .env("FASTPILOT_SCHEDULER_DIR", entries)
        .env("FASTPILOT_NOTIFY_DRY", "1")
        .env("FASTPILOT_GITHUB_API", format!("http://127.0.0.1:{github}"))
        .env("GITHUB_TOKEN", "ghp_test_token")
        .env("FASTPILOT_CLAUDE", FAKE_CLAUDE)
        .env("FAKE_CLAUDE_FAIL", "1")
        .env("FAKE_CLAUDE_LOG", home.join("claude-log"))
        .env("FASTPILOT_RETRY_BUDGET", "0")
        .env_remove("ANTHROPIC_API_KEY");
}

fn start_ui(home: &Path, entries: &Path, github: u16) -> Ui {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    cmd.args(["ui", "--no-open"]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    env_for(&mut cmd, home, entries, github);
    let mut child = cmd.spawn().expect("ui");
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
fn schedules_from_the_page_to_the_os_entry_and_notifications() {
    let home: PathBuf = std::env::temp_dir().join(format!("fp-sched-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let entries = home.join("LaunchAgents");
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let (hook_port, hook) = fake_server("ok");
    let (gh_port, gh) = fake_server(r#"{"html_url":"https://github.com/acme/shop/issues/7"}"#);
    let ui = start_ui(&home, &entries, gh_port);
    let post = |body: &str| request(ui.port, "POST", "/api/schedules", &ui.cookie, body, true);
    let tests = format!("{ROOT}/tests/login.md");
    let hook_url = format!("http://127.0.0.1:{hook_port}/services/T000/B000/rahasia");

    // Refused, with a reason the page shows.
    let bad = |schedule: serde_json::Value, webhook: Option<&str>, why: &str| {
        let (st, body) = post(&serde_json::json!({ "action": "save", "schedule": schedule, "webhook": webhook }).to_string());
        assert!(st == 422 && body.contains(why), "{st} {body}");
    };
    let base = serde_json::json!({ "id": "", "name": "Login harian", "tests": tests, "when": { "every": "weekday", "time": "08:30" }, "notify": { "webhook": true, "os": true, "github": "acme/shop" }, "enabled": true });
    bad(base.clone(), Some("http://hooks.example.com/x"), "https://");
    bad(serde_json::json!({ "id": "", "name": "x", "tests": "tests/login.md", "when": { "every": "day", "time": "08:30" }, "notify": {}, "enabled": true }), None, "path lengkap");
    bad(serde_json::json!({ "id": "", "name": "x", "tests": tests, "when": { "every": "day", "time": "25:00" }, "notify": {}, "enabled": true }), None, "HH:MM");
    bad(serde_json::json!({ "id": "", "name": "x", "tests": tests, "when": { "every": "day", "time": "08:00" }, "notify": { "github": "acme/../x" }, "enabled": true }), None, "pemilik/nama");
    bad(base.clone(), None, "URL webhook");
    // Changes need the page's own Origin.
    let (st, _) = request(ui.port, "POST", "/api/schedules", &ui.cookie, &serde_json::json!({ "action": "save", "schedule": base }).to_string(), false);
    assert!(st == 403 || st == 400, "no Origin: {st}");

    // Saved: the LaunchAgent holds only the id; the webhook URL only in an owner-only secret file.
    let (st, body) = post(&serde_json::json!({ "action": "save", "schedule": base, "webhook": hook_url }).to_string());
    assert_eq!(st, 200, "{body}");
    assert!(body.contains("\"id\":\"login-harian\""), "{body}");
    let plist = std::fs::read_to_string(entries.join("dev.fastpilot.schedule.login-harian.plist")).expect("LaunchAgent written");
    assert!(plist.contains("<string>schedule</string><string>run</string><string>login-harian</string>") && plist.matches("<key>Weekday</key>").count() == 5, "{plist}");
    assert!(!plist.contains("login.md") && !plist.contains("rahasia"), "{plist}");
    if cfg!(target_os = "macos") {
        let lint = std::process::Command::new("plutil").arg("-lint").arg(entries.join("dev.fastpilot.schedule.login-harian.plist")).output().expect("plutil");
        assert!(lint.status.success(), "{}", String::from_utf8_lossy(&lint.stdout));
    }
    let stored = std::fs::read_to_string(home.join(".fastpilot/schedules.json")).expect("schedules.json");
    assert!(!stored.contains("rahasia") && !stored.contains(&hook_port.to_string()), "the URL is not in schedules.json: {stored}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let secret = home.join(".fastpilot/secrets/webhook-login-harian");
        assert_eq!(std::fs::metadata(&secret).expect("secret").permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(home.join(".fastpilot/schedules.json")).expect("json").permissions().mode() & 0o777, 0o600);
    }
    let (st, list) = request(ui.port, "GET", "/api/schedules", &ui.cookie, "", false);
    assert_eq!(st, 200);
    assert!(list.contains(&format!("http://127.0.0.1:{hook_port}/•••")) && !list.contains("rahasia") && list.contains("Hari kerja 08:30"), "{list}");

    // Tes notifikasi: Slack-style text to the webhook, an issue with the token.
    let (st, body) = post(r#"{"action":"test","id":"login-harian"}"#);
    assert!(st == 200 && body.contains("webhook: terkirim") && body.contains("issues/7"), "{st} {body}");
    let first = hook.lock().expect("hook")[0].clone();
    assert!(first.starts_with("POST /services/T000/B000/rahasia") && first.contains("\"text\"") && first.contains("tes notifikasi"), "{first}");
    let issue = gh.lock().expect("gh")[0].clone();
    assert!(issue.starts_with("POST /repos/acme/shop/issues") && issue.to_lowercase().contains("authorization: bearer ghp_test_token"), "{issue}");

    // The OS scheduler's command: a failing run notifies and is kept as the last run.
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    cmd.args(["schedule", "run", "login-harian"]).current_dir("/");
    env_for(&mut cmd, &home, &entries, gh_port);
    let out = cmd.output().expect("schedule run");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "{text}");
    let failed = hook.lock().expect("hook").last().cloned().unwrap_or_default();
    assert!(failed.contains("gagal") && failed.contains("Login harian") && failed.contains("login-ok"), "{failed}");
    assert_eq!(gh.lock().expect("gh").len(), 2, "an issue for the failure");
    let (_, list) = request(ui.port, "GET", "/api/schedules", &ui.cookie, "", false);
    let v: serde_json::Value = serde_json::from_str(&list).expect("json");
    let last = &v["schedules"][0]["last"];
    assert_eq!(last["passed"], false, "{v}");
    assert!(last["summary"].as_str().is_some_and(|s| s.contains("1 failed") && s.contains("login-ok")), "{last}");
    assert!(last["notified"].as_array().is_some_and(|n| n.len() == 3), "{last}");

    // Off: the entry goes; delete: the schedule and its secret go.
    let (st, _) = post(r#"{"action":"disable","id":"login-harian"}"#);
    assert_eq!(st, 200);
    assert!(!entries.join("dev.fastpilot.schedule.login-harian.plist").exists());
    let (st, _) = post(r#"{"action":"delete","id":"login-harian"}"#);
    assert_eq!(st, 200);
    assert!(!home.join(".fastpilot/secrets/webhook-login-harian").exists());
    assert!(!std::fs::read_to_string(home.join(".fastpilot/schedules.json")).expect("json").contains("login-harian"));
    drop(ui);
    let _ = std::fs::remove_dir_all(home);
}
