//! R1 Release watch (v0.4) end to end, against stand-ins: `fastpilot schedule watch` saves a
//! schedule that runs every N minutes (a LaunchAgent with StartInterval, written to a test folder)
//! and takes its "before" run at once. A later run that fails where "before" passed opens one GitHub
//! issue (a fake API); the same failure again sends nothing; when the hours are up the watch removes
//! its scheduler entry and says how it went. Skips without Chrome.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const FAKE_CLAUDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude/claude");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists())
}

/// A fake GitHub API: records each request's first line and body.
fn fake_github() -> (u16, Arc<Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().expect("addr").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut r = BufReader::new(s.try_clone().expect("clone"));
            let (mut first, mut len) = (String::new(), 0usize);
            let _ = r.read_line(&mut first);
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; len];
            let _ = r.read_exact(&mut body);
            log.lock().expect("log").push(format!("{}\n{}", first.trim(), String::from_utf8_lossy(&body)));
            let reply = r#"{"html_url":"https://github.com/acme/shop/issues/1"}"#;
            let mut s = s;
            let _ = s.write_all(format!("HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes());
        }
    });
    (port, seen)
}

#[test]
fn a_release_watch_tells_only_about_a_new_problem_once_and_ends_by_itself() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let entries = home.join("LaunchAgents");
    std::fs::create_dir_all(&entries).expect("dir");
    let fail = home.join("fail-now");
    let (github, issues) = fake_github();
    let run = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .args(args)
            .current_dir(ROOT)
            .env("HOME", &home)
            .env("FASTPILOT_DIR", home.join("project-fastpilot"))
            .env_remove("FASTPILOT_CONFIG")
            .env("FASTPILOT_SCHEDULER", "launchd")
            .env("FASTPILOT_SCHEDULER_DIR", &entries)
            .env("FASTPILOT_NOTIFY_DRY", "1")
            .env("FASTPILOT_GITHUB_API", format!("http://127.0.0.1:{github}"))
            .env("GITHUB_TOKEN", "ghp_test_token")
            .env("FASTPILOT_CLAUDE", FAKE_CLAUDE)
            .env("FAKE_CLAUDE_FAIL_FILE", &fail)
            .env("FASTPILOT_RETRY_BUDGET", "0")
            .env("FASTPILOT_VARS", "DEMO_PASS")
            .env("DEMO_PASS", "rahasia123")
            .env_remove("ANTHROPIC_API_KEY")
            .output()
            .expect("run");
        (out.status.code(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    let store = || -> serde_json::Value { serde_json::from_str(&std::fs::read_to_string(home.join(".fastpilot/schedules.json")).unwrap_or_default()).unwrap_or_default() };
    let summary = |id: &str| store()["last"][id]["summary"].as_str().unwrap_or("").to_owned();

    // Bad input is refused with the reason.
    let (code, out) = run(&["schedule", "watch", &format!("{ROOT}/tests/login.md"), "--every", "7", "--for", "2"]);
    assert!(code != Some(0) && out.contains("5, 10, 15, 20, 30 or 60 minutes"), "{out}");
    let (code, out) = run(&["schedule", "watch", &format!("{ROOT}/tests/login.md"), "--every", "10", "--for", "48"]);
    assert!(code != Some(0) && out.contains("1 to 24 hours"), "{out}");

    // Started: every 10 minutes, the "before" run at once.
    let (code, out) = run(&["schedule", "watch", &format!("{ROOT}/tests/login.md"), "--every", "10", "--for", "2", "--github", "acme/shop", "--name", "Shop release"]);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("every 10 minutes until") && out.contains("\"before\""), "{out}");
    let id = store()["schedules"][0]["id"].as_str().expect("id").to_owned();
    let plist = std::fs::read_to_string(entries.join(format!("dev.fastpilot.schedule.{id}.plist"))).expect("plist");
    assert!(plist.contains("<key>StartInterval</key><integer>600</integer>") && !plist.contains("StartCalendarInterval"), "{plist}");
    let until = Instant::now() + Duration::from_secs(120);
    while !summary(&id).starts_with("Before the release") {
        assert!(Instant::now() < until, "the before run did not finish: {}", store());
        std::thread::sleep(Duration::from_millis(500));
    }
    assert_eq!(store()["schedules"][0]["watch"]["before"], serde_json::json!([]), "nothing failed before");

    // After the release it fails: one issue, the new problem named.
    std::fs::write(&fail, "").expect("fail");
    let (code, out) = run(&["schedule", "run", &id]);
    assert_eq!(code, Some(1), "{out}");
    assert!(summary(&id).starts_with("1 problem new since the release"), "{}", summary(&id));
    {
        let issues = issues.lock().expect("issues");
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].contains("Release watch") && issues[0].contains("a new problem since the release"), "{issues:?}");
    }
    // Still failing: nothing new, nothing sent.
    let (_, out) = run(&["schedule", "run", &id]);
    assert!(summary(&id).starts_with("No new problem since the release"), "{} / {out}", summary(&id));
    assert_eq!(issues.lock().expect("issues").len(), 1, "the same problem is sent once");

    // The hours are up: the watch ends, its scheduler entry is gone, the list says how it went.
    let mut st = store();
    st["schedules"][0]["watch"]["until_ms"] = serde_json::json!(1);
    std::fs::write(home.join(".fastpilot/schedules.json"), st.to_string()).expect("store");
    let (code, out) = run(&["schedule", "run", &id]);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("has ended"), "{out}");
    assert!(!entries.join(format!("dev.fastpilot.schedule.{id}.plist")).exists(), "the scheduler entry is removed");
    assert_eq!(summary(&id), "Release watch ended: 1 new problem since the release (notified)");
    let (_, list) = run(&["schedule", "list"]);
    assert!(list.contains("○") && list.contains("Release watch · ended"), "{list}");
    let _ = std::fs::remove_dir_all(&home);
}
