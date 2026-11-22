//! v0.6 1.2 Connect Jira, end to end against a fake Jira Cloud: every Test connection outcome
//! (success, 401, 403, site not found, 429 wait, not a Jira Cloud address), the project and QA
//! status pickers, Save checked again on the server, the saved token reused for Manage, a token
//! refused later ("Needs attention"), Disconnect, and the token never written anywhere but the key
//! store: not in settings, the audit log, or the server's own log.

#[path = "support/fake_jira.rs"]
mod fake_jira;

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

struct Ui {
    child: std::process::Child,
    port: u16,
    cookie: String,
    home: PathBuf,
}
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(home: &Path, base: &str) -> Ui {
    std::fs::create_dir_all(home).expect("home");
    let log = std::fs::File::create(home.join("ui.log")).expect("log");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(home)
        .env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_JIRA_TEST_BASE", base)
        .env("RUST_LOG", "trace")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .stdout(std::process::Stdio::piped())
        .stderr(log)
        .spawn()
        .expect("ui");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("link");
    let rest = line.trim().strip_prefix("http://127.0.0.1:").expect("link").to_owned();
    let (port, code) = rest.split_once("/?code=").expect("code");
    let port: u16 = port.parse().expect("port");
    let (_, head, _) = raw(port, "GET", &format!("/?code={code}"), "", "");
    let cookie = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().to_owned())).and_then(|c| c.split(';').next().map(str::to_owned)).expect("cookie");
    Ui { child, port, cookie, home: home.to_owned() }
}

fn raw(port: u16, method: &str, path: &str, body: &str, cookie: &str) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).expect("write");
    let mut out = String::new();
    s.read_to_string(&mut out).expect("read");
    let status = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let (head, body) = out.split_once("\r\n\r\n").map(|(a, b)| (a.to_owned(), b.to_owned())).unwrap_or_default();
    (status, head, body)
}

impl Ui {
    fn get(&self, path: &str) -> Value {
        serde_json::from_str(&raw(self.port, "GET", path, "", &self.cookie).2).expect("json")
    }
    fn post(&self, body: Value) -> (u16, Value) {
        let (st, _, text) = raw(self.port, "POST", "/api/jira", &body.to_string(), &self.cookie);
        (st, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
}

/// Every file under `dir` that contains `needle`.
fn files_with(dir: &Path, needle: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_with(&p, needle));
        } else if std::fs::read(&p).is_ok_and(|b| String::from_utf8_lossy(&b).contains(needle)) {
            out.push(p.display().to_string());
        }
    }
    out
}

#[test]
fn connect_jira_only_with_a_working_token_kept_only_in_the_key_store() {
    let fake = fake_jira::start();
    let home = std::env::temp_dir().join(format!("fp-jira-connect-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let ui = start(&home, &fake.base);
    let token = fake_jira::TOKEN;
    let good = |extra: Value| {
        let mut b = json!({ "action": "test", "site": "yourteam", "email": "alex@yourteam.com", "token": token });
        for (k, v) in extra.as_object().expect("obj") {
            b[k] = v.clone();
        }
        b
    };

    assert_eq!(ui.get("/api/jira")["connected"], false);

    // Not Jira Cloud: refused before any request.
    let (st, r) = ui.post(good(json!({ "site": "jira.mycompany.com" })));
    assert_eq!((st, r["kind"].as_str()), (409, Some("not_cloud")), "{r}");
    assert!(r["error"].as_str().is_some_and(|e| e.contains("Jira Server and Data Center are not supported yet")));
    assert!(fake.log().is_empty(), "no request for another host");

    // 401, 403, site not found, 429 then fine.
    let (_, r) = ui.post(good(json!({ "token": "ATATT-wrong-token" })));
    assert_eq!(r["kind"], "refused", "{r}");
    assert!(r["error"].as_str().is_some_and(|e| e.starts_with("Jira did not accept the email or token.") && !e.contains("ATATT")));
    let (_, r) = ui.post(good(json!({ "email": "noaccess@yourteam.com" })));
    assert_eq!(r["kind"], "no_access", "{r}");
    let (_, r) = ui.post(good(json!({ "site": "https://nosuchteam.atlassian.net/jira/software/projects/KAN" })));
    assert_eq!(r["kind"], "site", "{r}");
    assert!(r["error"].as_str().is_some_and(|e| e.contains("We could not find nosuchteam.atlassian.net")), "{r}");
    let (_, r) = ui.post(good(json!({ "email": "slow@yourteam.com" })));
    assert_eq!((r["kind"].as_str(), r["wait"].as_u64()), (Some("busy"), Some(1)), "Jira asked to wait: {r}");
    let (_, r) = ui.post(good(json!({ "email": "slow@yourteam.com" })));
    assert_eq!(r["ok"], true, "after the wait: {r}");

    // Success: name and projects; the QA status picker.
    let (st, r) = ui.post(good(json!({ "site": "https://yourteam.atlassian.net/jira/software/projects/KAN" })));
    assert_eq!(st, 200, "{r}");
    assert_eq!((r["display_name"].as_str(), r["site"].as_str(), r["projects"].as_array().map(Vec::len)), (Some("Alex Rivera"), Some("yourteam.atlassian.net"), Some(2)));
    let (_, r) = ui.post(good(json!({ "action": "statuses", "project": "KAN" })));
    assert_eq!(r["statuses"], json!(["To Do", "QA", "Done"]), "no repeats: {r}");

    // Save: checked again on the server, a project that exists.
    let (st, r) = ui.post(good(json!({ "action": "save", "project": "NOPE" })));
    assert_eq!((st, r["kind"].as_str()), (400, Some("project")), "{r}");
    let (st, r) = ui.post(good(json!({ "action": "save", "token": "ATATT-wrong-token", "project": "KAN" })));
    assert_eq!((st, r["kind"].as_str()), (409, Some("refused")), "Save cannot skip the test: {r}");
    assert_eq!(ui.get("/api/jira")["connected"], false);
    let (st, r) = ui.post(good(json!({ "action": "save", "project": "KAN", "qa_status": "QA" })));
    assert_eq!(st, 200, "{r}");
    assert_eq!(r["ok"], "Saved. Jira is connected to yourteam.atlassian.net · Kanban (KAN).");
    let state = ui.get("/api/jira");
    assert_eq!((state["connected"].as_bool(), state["settings"]["project_key"].as_str(), state["settings"]["qa_status"].as_str()), (Some(true), Some("KAN"), Some("QA")), "{state}");
    let apps = ui.get("/api/integrations");
    assert_eq!((apps["jira"]["site"].as_str(), apps["jira"]["needs_attention"].as_bool()), (Some("yourteam.atlassian.net"), Some(false)), "{apps}");

    // Manage: Test connection with the saved token; another address needs the token again.
    let (_, r) = ui.post(json!({ "action": "test", "site": "yourteam.atlassian.net", "email": "alex@yourteam.com" }));
    assert_eq!(r["ok"], true, "{r}");
    let (st, r) = ui.post(json!({ "action": "test", "site": "otherteam.atlassian.net", "email": "alex@yourteam.com" }));
    assert_eq!((st, r["kind"].as_str()), (400, Some("empty")), "the saved token stays with its own site: {r}");

    // The token never left the key store.
    let found = files_with(&home, token);
    assert_eq!(found, [home.join(".fastpilot/keys/jira").display().to_string()], "only the key store holds the token");
    let audit = std::fs::read_to_string(home.join(".fastpilot/audit.jsonl")).unwrap_or_default();
    assert!(audit.contains("\"setting\":\"jira\"") && audit.contains("connected") && audit.contains("yourteam.atlassian.net alex@yourteam.com"), "{audit}");
    assert!(fake.log().iter().all(|r| r.path.starts_with("/rest/api/3/")), "only Jira's REST API");

    // Atlassian stops accepting the token: Manage says so, Apps shows Needs attention.
    fake.stop_accepting();
    let (_, r) = ui.post(json!({ "action": "test", "site": "yourteam.atlassian.net", "email": "alex@yourteam.com" }));
    assert_eq!(r["kind"], "refused", "{r}");
    assert_eq!(ui.get("/api/integrations")["jira"]["needs_attention"], true);

    // Disconnect: token and settings gone, audit says so.
    let (st, r) = ui.post(json!({ "action": "disconnect" }));
    assert_eq!(st, 200, "{r}");
    assert!(r["ok"].as_str().is_some_and(|m| m.starts_with("Jira disconnected.")));
    assert_eq!(ui.get("/api/jira")["connected"], false);
    assert_eq!(ui.get("/api/integrations")["jira"], Value::Null);
    assert!(!home.join(".fastpilot/keys/jira").exists() && !home.join(".fastpilot/jira.json").exists());
    let audit = std::fs::read_to_string(home.join(".fastpilot/audit.jsonl")).unwrap_or_default();
    assert!(audit.contains("disconnected"), "{audit}");
    drop(ui);
    assert!(files_with(&home, token).is_empty(), "nowhere after disconnect (server log included)");
    assert!(files_with(&home, "ATATT-wrong-token").is_empty(), "a refused token is not written either");
    let _ = std::fs::remove_dir_all(&home);
}

/// Only the dashboard's own page connects Jira: no cookie, another origin, or a non-JSON post is
/// refused, so a tested page or another site cannot connect or disconnect it.
#[test]
fn only_the_dashboard_page_can_change_jira() {
    let fake = fake_jira::start();
    let home = std::env::temp_dir().join(format!("fp-jira-guard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let ui = start(&home, &fake.base);
    let body = json!({ "action": "save", "site": "yourteam", "email": "alex@yourteam.com", "token": fake_jira::TOKEN, "project": "KAN" }).to_string();
    let (st, _, _) = raw(ui.port, "POST", "/api/jira", &body, "");
    assert_eq!(st, 401, "no cookie");
    let mut s = TcpStream::connect(("127.0.0.1", ui.port)).expect("connect");
    let req = format!("POST /api/jira HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nCookie: {}\r\nOrigin: http://evil.example\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", ui.port, ui.cookie, body.len());
    s.write_all(req.as_bytes()).expect("write");
    let mut out = String::new();
    s.read_to_string(&mut out).expect("read");
    assert!(out.starts_with("HTTP/1.1 403"), "another site: {out}");
    assert!(fake.log().is_empty() && !ui.home.join(".fastpilot/jira.json").exists());
    let _ = std::fs::remove_dir_all(&home);
}
