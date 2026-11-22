//! PF1 (v0.6) App map, end to end in Chrome on the demo shop that `fastpilot demo` writes:
//! - the Map page maps it read-only to at least 6 screens, with the start page's links as edges;
//! - "Delete" (and every blocked or confirm word) is never pressed, nothing is typed or submitted,
//!   and it stops at the sign-in page;
//! - coverage comes from the saved demo tests and their memory: the sign-in screen has a test, and
//!   a failing test still counts as "has a test", shown as failing;
//! - with a test account in the project notes it signs in once, and the password is in no file;
//! - the list view is a real table that works with the keyboard, and "Make a test" opens Chat with
//!   the request written in.
//!
//! Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

type Client = RunningService<RoleClient, ()>;

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists())
}

struct Ui {
    child: std::process::Child,
    port: u16,
    cookie: String,
    link: String,
}
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn request(port: u16, method: &str, path: &str, cookie: &str, body: &str) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).expect("write");
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).expect("read");
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(raw.len(), |i| i + 4);
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    (head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0), raw[split..].to_vec())
}

fn json_of(port: u16, method: &str, path: &str, cookie: &str, body: Value) -> (u16, Value) {
    let (st, b) = request(port, method, path, cookie, &if body.is_null() { String::new() } else { body.to_string() });
    (st, serde_json::from_slice(&b).unwrap_or(json!({ "raw": String::from_utf8_lossy(&b) })))
}

fn start_ui(home: &Path, project: &Path) -> Ui {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(project)
        .env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env_remove("FASTPILOT_BLOCK_ACTIONS")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env("FASTPILOT_VARS", "DEMO_PASS")
        .env("DEMO_PASS", "rahasia123")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("link");
    let link = line.trim().to_owned();
    let rest = link.strip_prefix("http://127.0.0.1:").expect("link").to_owned();
    let (port, code) = rest.split_once("/?code=").expect("code");
    let port: u16 = port.parse().expect("port");
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.write_all(format!("GET /?code={code} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let cookie = raw
        .lines()
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().to_owned()))
        .and_then(|c| c.split(';').next().map(str::to_owned))
        .expect("cookie");
    Ui { child, port, cookie, link }
}

/// Starts a map on the Map page's API and waits until it is done.
fn map(ui: &Ui, target: &str) -> Value {
    let (st, r) = json_of(ui.port, "POST", "/api/map", &ui.cookie, json!({ "action": "start", "target": target, "platform": "web", "screens": 30, "minutes": 5 }));
    assert_eq!(st, 200, "{r}");
    let until = Instant::now() + Duration::from_secs(240);
    loop {
        let (_, v) = json_of(ui.port, "GET", "/api/map", &ui.cookie, Value::Null);
        if v["mapping"] == false && v["map"]["state"].as_str().is_some_and(|s| s != "mapping") {
            return v;
        }
        assert!(Instant::now() < until, "mapping did not finish: {v}");
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn screen<'a>(v: &'a Value, title: &str) -> &'a Value {
    v["map"]["screens"].as_array().into_iter().flatten().find(|s| s["title"] == title).unwrap_or_else(|| panic!("no screen {title}: {v}"))
}

fn status<'a>(v: &'a Value, id: &Value) -> &'a Value {
    v["coverage"]["screens"].as_array().into_iter().flatten().find(|c| c["id"] == *id).expect("coverage")
}

async fn flow(client: &Client, steps: &str) -> (bool, String) {
    let mut args = json!({ "flow": steps }).as_object().cloned().unwrap_or_default();
    if !steps.contains("start: person") {
        args.insert("session".into(), json!("person"));
    }
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new("run_flow").with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

async fn eval(client: &Client, js: &str) -> String {
    let (ok, out) = flow(client, &format!("- evalScript: {}\n", serde_json::to_string(js).expect("js"))).await;
    assert!(ok, "{out}");
    out.lines().find_map(|l| l.rsplit_once(" = ").map(|(_, v)| v.trim().trim_matches('"').to_owned())).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_demo_shop_is_mapped_read_only_with_its_tests() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-map-web-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let project = home.join("shop");
    std::fs::create_dir_all(&home).expect("home");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["demo", "shop"])
        .current_dir(&home)
        .env("HOME", &home)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_ADB_SERVER", "127.0.0.1:1")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("demo");
    assert!(project.join("demo/index.html").is_file() && project.join("tests/login.md").is_file(), "{}", String::from_utf8_lossy(&out.stdout));
    let ui = start_ui(&home, &project);

    // An empty Map page first.
    let (_, v) = json_of(ui.port, "GET", "/api/map", &ui.cookie, Value::Null);
    assert!(v["map"].is_null() && v["mapping"] == false, "{v}");

    // 1. Read-only map of the demo shop.
    let v = map(&ui, "demo/index.html");
    let m = &v["map"];
    let screens = m["screens"].as_array().expect("screens");
    assert!(m["state"] == "finished" && screens.len() >= 6, "at least 6 screens: {m}");
    let start = &screens[0];
    assert_eq!(start["title"], "Demo shop");
    let from_start: Vec<&str> = m["edges"].as_array().expect("edges").iter().filter(|e| e["from"] == start["id"]).filter_map(|e| e["label"].as_str()).collect();
    for label in ["Sign in", "Widgets", "Tickets", "Shipping", "Orders", "PIN", "My account"] {
        assert!(from_start.contains(&label), "edge {label} from the start page: {from_start:?}");
    }
    // The Help page is reached by a harmless button on My account.
    let account = screen(&v, "My account");
    let help = screen(&v, "Help");
    assert!(m["edges"].as_array().expect("edges").iter().any(|e| e["from"] == account["id"] && e["to"] == help["id"] && e["label"] == "Help"), "{m}");
    // Nothing that could change something was pressed: Delete is named as skipped and the
    // "deleted" page it would open is not on the map.
    assert!(m["skipped"].as_array().expect("skipped").iter().any(|k| k["label"] == "Delete" && k["screen"] == "My account" && k["why"] == "asks for confirmation"), "{m}");
    assert!(!screens.iter().any(|s| s["location"].as_str().unwrap_or("").contains("deleted")), "Delete was pressed: {m}");
    // It stopped at the sign-in page: no Dashboard behind it, nothing typed.
    let sign_in = screen(&v, "Masuk ke Akun");
    assert_eq!(sign_in["sign_in"], true);
    assert!(m["wall"] == "Masuk ke Akun" && m["signed_in"] == false && !screens.iter().any(|s| s["title"] == "Dashboard"), "{m}");
    assert!(m["message"].as_str().is_some_and(|t| t.contains("Nothing was typed in or submitted") && t.contains("did not sign in")), "{m}");
    // QA re-test Low 5: the count in the summary is the named list.
    let named = m["skipped"].as_array().expect("skipped").len();
    assert!(m["message"].as_str().is_some_and(|t| t.contains(&format!("{named} buttons and links were not pressed"))), "{named}: {m}");

    // 2. Coverage from the demo tests: tests/login.md starts on the sign-in page.
    let c = status(&v, &sign_in["id"]);
    assert!(c["status"] == "tested" && c["tests"][0]["id"] == "login-ok" && c["tests"][0]["status"] == "not_run", "{c}");
    assert_eq!(status(&v, &start["id"])["status"], "none");
    // A failing run of that test: still "has a test", now shown as failing.
    let location = sign_in["location"].as_str().expect("location");
    let memory = json!({
        "test_id": "login-ok", "flow": "", "start": format!("[s1] screen \"Masuk ke Akun\" ({location})"),
        "screens": [format!("[s1] screen \"Masuk ke Akun\" ({location})")], "duration_ms": 900, "pass_count": 0, "fail_count": 1,
        "last_pass": "", "failed_at": ["2026-11-22T10:00:00Z"], "history": [false], "consecutive_failures": 1, "stale": false, "classes": []
    });
    std::fs::create_dir_all(project.join(".fastpilot/memory")).expect("memory dir");
    std::fs::write(project.join(".fastpilot/memory/login-ok.json"), memory.to_string()).expect("memory");
    let (_, v2) = json_of(ui.port, "GET", "/api/map", &ui.cookie, Value::Null);
    assert_eq!(status(&v2, &sign_in["id"])["status"], "failing");
    assert!(v2["coverage"]["tested"] == 1 && v2["coverage"]["failing"] == 1, "{}", v2["coverage"]);

    // Thumbnails: one per screen; nothing outside the map folder.
    let target = m["target"].as_str().expect("target");
    let (st, jpg) = request(ui.port, "GET", &format!("/api/map/thumb?target={}&id={}", target.replace('/', "%2F"), start["id"].as_str().unwrap_or("")), &ui.cookie, "");
    assert!(st == 200 && jpg.starts_with(&[0xff, 0xd8]), "a JPEG thumbnail: {st}");
    let (st, _) = request(ui.port, "GET", &format!("/api/map/thumb?target={target}&id=..%2F..%2Fx"), &ui.cookie, "");
    assert_eq!(st, 404);

    // 3. With a test account in the project notes it signs in once (the only typing it does).
    let (st, r) = json_of(ui.port, "POST", "/api/notes", &ui.cookie, json!({ "action": "add", "group": "accounts", "text": "Test account for staging: a@b.com, password in ${DEMO_PASS}" }));
    assert_eq!(st, 200, "{r}");
    let v = map(&ui, "demo/index.html");
    let m = &v["map"];
    assert!(m["signed_in"] == true && m["wall"].is_null(), "{m}");
    let dash = screen(&v, "Dashboard");
    assert!(m["edges"].as_array().expect("edges").iter().any(|e| e["to"] == dash["id"] && e["label"] == "Sign in (test account)"), "{m}");
    assert!(m["skipped"].as_array().expect("skipped").iter().any(|k| k["label"] == "Keluar"), "sign out is not pressed: {m}");
    assert!(!m["screens"].as_array().expect("screens").iter().any(|s| s["location"].as_str().unwrap_or("").contains("deleted")));
    // The password is in no map file.
    // The map is kept in fastpilot's own folder, never in the project (Security review, Low 2).
    assert!(!project.join(".fastpilot/map").exists(), "nothing in the project");
    let maps: Vec<_> = std::fs::read_dir(home.join(".fastpilot/projects")).expect("projects").flatten().map(|e| e.path().join("map")).filter(|p| p.is_dir()).collect();
    assert_eq!(maps.len(), 1, "{maps:?}");
    for e in std::fs::read_dir(&maps[0]).expect("map dir").flatten() {
        if e.path().is_file() {
            assert!(!String::from_utf8_lossy(&std::fs::read(e.path()).expect("read")).contains("rahasia123"), "{}", e.path().display());
        }
    }

    // 4. The list view with the keyboard, then "Make a test" opens Chat with the request. A new
    // dashboard, so the browser gets its own one-time link (the map is read from the project).
    drop(ui);
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(&project)
        .env("HOME", &home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut link = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut link).expect("link");
    let ui = Ui { child, port: 0, cookie: String::new(), link: link.trim().to_owned() };
    let person = home.join("person");
    std::fs::create_dir_all(&person).expect("person");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").env("HOME", &person).env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").env("FASTPILOT_LIVE_DIR", person.join("live")).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = flow(&client, &format!("- session: {{ start: person, platform: web, target: \"{}\" }}\n- assertVisible: {{ text: \"Apps\", timeout: 10000 }}\n", ui.link)).await;
    assert!(ok, "{out}");
    eval(&client, "location.hash = '#/map', 'ok'").await;
    let (ok, out) = flow(&client, "- assertVisible: { text: \"screens have a test\", timeout: 8000 }\n- tapOn: List\n- assertVisible: { text: \"Screens, their test status and where they lead\", timeout: 5000 }\n").await;
    assert!(ok, "{out}");
    assert_eq!(eval(&client, "String(document.querySelectorAll('.map-table caption, .map-table th[scope=col], .map-table th[scope=row]').length > 8)").await, "true", "a real table with headers");
    // Tab to the first "Make a test" button and press Enter.
    eval(&client, "(document.querySelector('.map-filters input').focus(), 'ok')").await;
    let mut reached = false;
    for _ in 0..30 {
        let (ok, out) = flow(&client, "- pressKey: Tab\n").await;
        assert!(ok, "{out}");
        if eval(&client, "document.activeElement.textContent").await == "Make a test" {
            reached = true;
            break;
        }
    }
    assert!(reached, "Make a test is reachable with Tab");
    // QA re-test Low 5: the screen panel shows a local file by its name, never the full path.
    eval(&client, "([...document.querySelectorAll('.map-table button')].find((b) => b.textContent === 'Open').click(), 'ok')").await;
    let (ok, out) = flow(&client, "- assertVisible: { text: \"Selected screen\", timeout: 5000 }\n").await;
    assert!(ok, "{out}");
    let panel = eval(&client, "document.querySelector('.map-side').innerText").await;
    assert!(!panel.contains(home.to_string_lossy().trim_start_matches("/private")) && !panel.contains("file://") && panel.contains(".html"), "{panel}");
    eval(&client, "(location.hash = '#/map', 'ok')").await;
    let (ok, out) = flow(&client, "- tapOn: List\n").await;
    assert!(ok, "{out}");
    eval(&client, "(document.querySelector('.map-filters input').focus(), 'ok')").await;
    for _ in 0..30 {
        let (ok, out) = flow(&client, "- pressKey: Tab\n").await;
        assert!(ok, "{out}");
        if eval(&client, "document.activeElement.textContent").await == "Make a test" {
            break;
        }
    }
    let (ok, out) = flow(&client, "- pressKey: Enter\n- assertVisible: { text: \"Ask or tell the fastpilot bot\", timeout: 8000 }\n").await;
    assert!(ok, "{out}");
    let typed = eval(&client, "document.querySelector('textarea').value").await;
    assert!(typed.starts_with("Make a test for the") && typed.contains("Demo shop"), "{typed}");
    let _ = client.cancel().await;
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}
