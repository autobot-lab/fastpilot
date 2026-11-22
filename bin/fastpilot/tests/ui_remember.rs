//! E3 (v0.7): "Remember this browser" for the dashboard, end to end on real `fastpilot ui`
//! processes in a temp HOME.
//! - Only a session from the printed link can remember a browser; the key goes back once and the
//!   server keeps only its SHA-256, in `~/.fastpilot/ui/browsers.json` (folder 0700, file 0600).
//! - The key opens a new session after a restart, on the same port (bookmark), without a new link;
//!   a wrong key, another Origin or Host, a forgotten browser, 30 idle days and 90 days are refused.
//! - Without a session only the small sign-in page and what it needs are served (HTTP 401 page).
//! - In Chrome: the banner asks first; after "Remember this browser" a restarted dashboard opens by
//!   itself; Settings › Security lists it as "This browser"; Forget signs it out.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const CHROME_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36";

struct Ui {
    child: std::process::Child,
    port: u16,
    code: String,
}
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(home: &Path) -> Ui {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(home)
        .env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("link");
    let rest = line.trim().strip_prefix("http://127.0.0.1:").expect("link").to_owned();
    let (port, code) = rest.split_once("/?code=").expect("code");
    Ui { child, port: port.parse().expect("port"), code: code.to_owned() }
}

struct Answer {
    status: u16,
    head: String,
    body: String,
}
impl Answer {
    fn cookie(&self) -> Option<String> {
        self.head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().split(';').next().unwrap_or("").to_owned()))
    }
}

fn http(port: u16, method: &str, path: &str, headers: &[(&str, String)], body: &str) -> Answer {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        req.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let status = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let (head, body) = raw.split_once("\r\n\r\n").map(|(h, b)| (h.to_owned(), b.to_owned())).unwrap_or_default();
    Answer { status, head, body }
}

fn post(port: u16, path: &str, cookie: Option<&str>, body: Value) -> Answer {
    let mut h = vec![("Origin", format!("http://127.0.0.1:{port}")), ("Content-Type", "application/json".to_owned()), ("User-Agent", CHROME_UA.to_owned())];
    if let Some(c) = cookie {
        h.push(("Cookie", c.to_owned()));
    }
    http(port, "POST", path, &h, &body.to_string())
}

fn signed_in(port: u16, cookie: &str) -> bool {
    http(port, "GET", "/api/settings", &[("Cookie", cookie.to_owned())], "").status == 200
}

fn file(home: &Path) -> std::path::PathBuf {
    home.join(".fastpilot/ui/browsers.json")
}

#[test]
fn a_remembered_browser_opens_the_dashboard_without_a_new_link() {
    let home = std::env::temp_dir().join(format!("fp-remember-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");

    let ui = start(&home);
    let p = ui.port;
    let link = http(p, "GET", &format!("/?code={}", ui.code), &[], "");
    let session = link.cookie().expect("session cookie for the link");
    assert!(link.head.contains("HttpOnly") && link.head.contains("SameSite=Strict") && !link.head.to_lowercase().contains("max-age") && !link.head.to_lowercase().contains("expires"), "a session cookie, gone when the browser closes: {}", link.head);
    let b: Value = serde_json::from_str(&http(p, "GET", "/api/browsers", &[("Cookie", session.clone())], "").body).expect("json");
    assert_eq!((b["current"].clone(), b["browsers"].as_array().map(Vec::len)), (Value::Null, Some(0)), "nothing remembered yet: {b}");

    // Remember: only with a session; the key comes back once; the file keeps its hash only.
    assert_eq!(post(p, "/api/browsers", None, json!({ "action": "remember" })).status, 401, "no session, no key");
    let r = post(p, "/api/browsers", Some(&session), json!({ "action": "remember" }));
    assert_eq!(r.status, 200, "{}", r.body);
    let r: Value = serde_json::from_str(&r.body).expect("json");
    let key = r["key"].as_str().expect("key").to_owned();
    assert_eq!((key.len(), r["label"].as_str()), (64, Some("Chrome on this Mac")), "{r}");
    let again: Value = serde_json::from_str(&post(p, "/api/browsers", Some(&session), json!({ "action": "remember" })).body).expect("json");
    assert!(again["key"].is_null() && again["already"] == true, "a key is handed out once per browser: {again}");
    let saved = std::fs::read_to_string(file(&home)).expect("browsers.json");
    assert!(!saved.contains(&key) && saved.contains("\"hash\""), "only the hash is kept: {saved}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
        assert_eq!((mode(&file(&home)), mode(&home.join(".fastpilot/ui"))), (0o600, 0o700), "owner only");
    }
    let listed: Value = serde_json::from_str(&http(p, "GET", "/api/browsers", &[("Cookie", session.clone())], "").body).expect("json");
    assert_eq!(listed["current"], r["id"], "this browser: {listed}");
    assert!(!listed.to_string().contains(&key) && !listed.to_string().contains("hash"), "the list never shows a key or hash: {listed}");

    // Without a session: the sign-in page (401) and only what it needs.
    let page = http(p, "GET", "/", &[], "");
    assert!(page.status == 401 && page.body.contains("/assets/unlock.js") && page.body.contains("This browser is not signed in"), "{} {}", page.status, page.body);
    assert!(page.head.to_lowercase().contains("content-security-policy: default-src 'self'"), "{}", page.head);
    assert_eq!(http(p, "GET", "/assets/unlock.js", &[], "").status, 200);
    assert_eq!(http(p, "GET", "/assets/app.css", &[], "").status, 200);
    for closed in ["/assets/app.js", "/api/settings", "/api/browsers"] {
        assert_eq!(http(p, "GET", closed, &[], "").status, 401, "{closed}");
    }

    // A restart: the same port, the old session is gone, the key opens a new one.
    drop(ui);
    let ui = start(&home);
    assert_eq!(ui.port, p, "the port is kept for the bookmark");
    assert!(!signed_in(p, &session), "a session ends with its dashboard");
    let wrong = post(p, "/api/unlock", None, json!({ "key": "0".repeat(64) }));
    assert!(wrong.status == 401 && wrong.cookie().is_none(), "{}", wrong.body);
    let mut h = vec![("Content-Type", "application/json".to_owned())];
    assert_eq!(http(p, "POST", "/api/unlock", &h, &json!({ "key": key }).to_string()).status, 403, "no Origin: not from our page");
    h.push(("Origin", "http://127.0.0.1:1".to_owned()));
    assert_eq!(http(p, "POST", "/api/unlock", &h, &json!({ "key": key }).to_string()).status, 403, "another port's page");
    h[1] = ("Origin", format!("http://127.0.0.1:{p}"));
    h.push(("Host", format!("evil.example:{p}")));
    assert_ne!(http(p, "POST", "/api/unlock", &h, &json!({ "key": key }).to_string()).status, 200, "DNS rebinding: wrong Host");
    let ok = post(p, "/api/unlock", None, json!({ "key": key }));
    assert_eq!(ok.status, 200, "{}", ok.body);
    let second = ok.cookie().expect("new session");
    assert!(signed_in(p, &second), "the bookmark opens without a new link");
    let listed: Value = serde_json::from_str(&http(p, "GET", "/api/browsers", &[("Cookie", second.clone())], "").body).expect("json");
    assert_eq!(listed["current"], r["id"]);

    // Forget: the key stops working and its session ends now.
    let out = post(p, "/api/browsers", Some(&second), json!({ "action": "forget", "id": r["id"] }));
    let out: Value = serde_json::from_str(&out.body).expect("json");
    assert_eq!((out["forgotten"].as_u64(), out["signed_out"].as_bool()), (Some(1), Some(true)), "{out}");
    assert!(!signed_in(p, &second), "signed out at once");
    assert_eq!(post(p, "/api/unlock", None, json!({ "key": key })).status, 401, "a forgotten browser needs a new link");

    // 30 idle days, or 90 days in all, end it.
    let fresh = http(p, "GET", &format!("/?code={}", ui.code), &[], "").cookie().expect("session");
    let day = 86_400_000u64;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("time").as_millis() as u64;
    for (created, last, works) in [(now - 31 * day, now - 31 * day, false), (now - 91 * day, now - day, false), (now - 60 * day, now - 29 * day, true)] {
        let r: Value = serde_json::from_str(&post(p, "/api/browsers", Some(&fresh), json!({ "action": "remember" })).body).expect("json");
        let k = r["key"].as_str().map(str::to_owned).unwrap_or_default();
        let mut s: Value = serde_json::from_str(&std::fs::read_to_string(file(&home)).expect("file")).expect("json");
        for b in s["browsers"].as_array_mut().expect("list") {
            if b["id"] == r["id"] {
                b["created_ms"] = json!(created);
                b["last_ms"] = json!(last);
            }
        }
        std::fs::write(file(&home), s.to_string()).expect("write");
        assert_eq!(post(p, "/api/unlock", None, json!({ "key": k })).status == 200, works, "created {} days ago, used {} days ago", (now - created) / day, (now - last) / day);
        // The next one is a new browser (taken out of the file, so this session may remember again;
        // Forget would sign this session out, as it should).
        std::fs::write(file(&home), json!({ "browsers": [] }).to_string()).expect("clear");
    }
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists())
}

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

async fn eval(client: &Client, js: &str) -> String {
    let flow = format!("- evalScript: {}\n", serde_json::to_string(js).expect("js"));
    let (ok, out) = call(client, "run_flow", json!({ "flow": flow, "session": "viewer" })).await;
    assert!(ok, "{js}: {out}");
    out.lines().find_map(|l| l.rsplit_once(" = ").map(|(_, v)| v.trim().to_owned())).unwrap_or_default()
}

async fn see(client: &Client, text: &str) {
    let flow = format!("- assertVisible: {{ text: {}, timeout: 10000 }}\n", serde_json::to_string(text).expect("text"));
    let (ok, out) = call(client, "run_flow", json!({ "flow": flow, "session": "viewer" })).await;
    assert!(ok, "{text}: {out}");
}

async fn click(client: &Client, label: &str) {
    eval(client, &format!("([...document.querySelectorAll('button')].find((b) => b.textContent.trim() === {label:?}).click(), 'ok')")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chrome_remembers_after_asking_and_forgets_on_request() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-remember-chrome-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("viewer")).expect("home");
    let ui = start(&home);
    let p = ui.port;
    let viewer = home.join("viewer");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").current_dir(ROOT).env("HOME", &viewer).env("FASTPILOT_LIVE_DIR", viewer.join("live")).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let c = &client;
    let flow = format!("- session: {{ start: viewer, platform: web, target: \"http://127.0.0.1:{p}/?code={}\" }}\n", ui.code);
    let (ok, out) = call(c, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");

    // Asked first, in a banner.
    see(c, "Remember this browser?").await;
    assert_eq!(eval(c, "String(localStorage.getItem('fastpilot.browser'))").await, "\"null\"", "nothing kept before the click");
    click(c, "Remember this browser").await;
    see(c, "Bookmark 127.0.0.1").await;
    assert_eq!(eval(c, "String((localStorage.getItem('fastpilot.browser') || '').length)").await, "\"64\"", "the key is in this page's storage");
    assert_eq!(eval(c, "String(document.cookie)").await, "\"\"", "the session cookie is HttpOnly");

    // The dashboard restarts (a new process, no session): the bookmark opens it by itself.
    drop(ui);
    let ui = start(&home);
    assert_eq!(ui.port, p);
    eval(c, &format!("(location.href = 'http://127.0.0.1:{p}/#/settings/security', 'ok')")).await;
    see(c, "This browser").await;
    let row = eval(c, "document.querySelector('tbody tr th').textContent").await;
    assert_eq!(row, "\"Chrome on this Mac This browser\"");
    assert_eq!(eval(c, "String(!!document.querySelector('.remember'))").await, "\"false\"", "no second question");

    // Forget this browser: asks, then signs out at once.
    click(c, "Forget").await;
    see(c, "Forget this browser?").await;
    click(c, "Forget and sign out").await;
    see(c, "This browser is not signed in").await;
    assert_eq!(eval(c, "String(localStorage.getItem('fastpilot.browser'))").await, "\"null\"");
    assert!(std::fs::read_to_string(file(&home)).is_ok_and(|s| !s.contains("\"id\"")), "forgotten on the server too");
    client.cancel().await.expect("shutdown");
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}

fn sha256(key: &str) -> String {
    ring::digest::digest(&ring::digest::SHA256, key.as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Security, #178 M1: Forget must win over a holder of the key who keeps /api/unlock busy; L1: a key
/// is never traded or handed out under localhost; L2: a squatted saved port ends the keys.
#[test]
fn forget_wins_over_parallel_unlocks_and_keys_stay_on_127_0_0_1() {
    let home = std::env::temp_dir().join(format!("fp-remember-race-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot/ui")).expect("home");
    let ui = start(&home);
    let p = ui.port;
    let session = http(p, "GET", &format!("/?code={}", ui.code), &[], "").cookie().expect("session");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("time").as_millis() as u64;
    let other = "b".repeat(64);
    for round in 0..5 {
        let key = format!("{:064x}", 0xa000 + round);
        let entry = |id: &str, k: &str| json!({ "id": id, "label": "Chrome on this Mac", "hash": sha256(k), "created_ms": now, "last_ms": now });
        std::fs::write(file(&home), json!({ "browsers": [entry(&format!("a{round}"), &key), entry("b", &other)] }).to_string()).expect("store");
        let storm: Vec<_> = (0..30)
            .map(|_| {
                let k = key.clone();
                std::thread::spawn(move || post(p, "/api/unlock", None, json!({ "key": k })).status)
            })
            .collect();
        let gone = post(p, "/api/browsers", Some(&session), json!({ "action": "forget", "id": format!("a{round}") }));
        for t in storm {
            let _ = t.join();
        }
        let gone: Value = serde_json::from_str(&gone.body).unwrap_or_else(|_| panic!("round {round}: forget failed: {}", gone.body));
        assert_eq!(gone["forgotten"], 1, "round {round}: {gone}");
        assert_eq!(post(p, "/api/unlock", None, json!({ "key": key })).status, 401, "round {round}: the forgotten key must not work");
        assert_eq!(post(p, "/api/unlock", None, json!({ "key": other })).status, 200, "round {round}: the other browser stays remembered");
    }
    let tmp = std::fs::read_dir(home.join(".fastpilot/ui")).expect("dir").flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).count();
    assert_eq!(tmp, 0, "no temp files left");

    // localhost: the page moves to 127.0.0.1; a key is never traded or handed out there.
    let lh = http(p, "GET", "/", &[("Host", format!("localhost:{p}"))], "");
    assert!(lh.status == 303 && lh.head.contains(&format!("http://127.0.0.1:{p}/")), "{} {}", lh.status, lh.head);
    let h = [("Host", format!("localhost:{p}")), ("Origin", format!("http://localhost:{p}")), ("Content-Type", "application/json".to_owned())];
    assert_eq!(http(p, "POST", "/api/unlock", &h, &json!({ "key": other }).to_string()).status, 421, "no unlock under localhost");
    let mut h = h.to_vec();
    h.push(("Cookie", session.clone()));
    assert_ne!(http(p, "POST", "/api/browsers", &h, &json!({ "action": "remember" }).to_string()).status, 200, "no key handed out under localhost");

    // The saved port is taken while fastpilot is stopped: the keys stop working.
    drop(ui);
    let squatter = std::net::TcpListener::bind(("127.0.0.1", p)).expect("squat");
    let ui = start(&home);
    assert_ne!(ui.port, p);
    let mut left = String::new();
    for _ in 0..40 {
        left = std::fs::read_to_string(file(&home)).unwrap_or_default();
        if !left.contains("\"id\"") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(!left.contains("\"id\""), "every remembered browser is forgotten: {left}");
    assert_eq!(post(ui.port, "/api/unlock", None, json!({ "key": other })).status, 401);
    drop(squatter);
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}
