//! v0.6 1.5: fastpilot's Jira tools for Claude Code (`jira_read`, `jira_create_issue`,
//! `jira_comment`), against a fake Jira Cloud. Writes are requests the person sends from the Apps
//! page or `fastpilot jira approve` in their own terminal (Security, Medium 1); `jira_read` reads
//! only the default project (Medium 2). They use the connection saved in the dashboard
//! (the token stays in the key store and never appears in a result); a ticket is returned as data,
//! never instructions; a write checks for a duplicate first and sends cleaned text (secrets masked,
//! no emails, no links to other sites); the dashboard Chat and unattended runs cannot write; a
//! refused token marks the connection "Needs attention".

#[path = "support/fake_jira.rs"]
mod fake_jira;

use rmcp::ServiceExt;
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::RoleClient;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;

type Client = RunningService<RoleClient, ()>;

/// The connection the dashboard saves, written directly (the test key store is a file).
fn connect(home: &Path, base: &str) {
    let d = home.join(".fastpilot");
    std::fs::create_dir_all(d.join("keys")).expect("dir");
    std::fs::write(d.join("keys/jira"), fake_jira::TOKEN).expect("token");
    std::fs::write(d.join("keys/jira.origin"), format!("{base}/{}", fake_jira::SITE)).expect("origin");
    std::fs::write(d.join("jira.json"), json!({ "site": fake_jira::SITE, "email": "alex@yourteam.com", "project_key": "KAN", "project_name": "Kanban" }).to_string()).expect("settings");
}

async fn mcp(home: &Path, base: &str, writes: Option<&str>) -> Client {
    let (home, base, writes) = (home.to_owned(), base.to_owned(), writes.map(str::to_owned));
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(&home).env("HOME", &home).env_remove("FASTPILOT_CONFIG").env("FASTPILOT_KEYSTORE", "file").env("FASTPILOT_JIRA_TEST_BASE", &base)
            .env("FASTPILOT_VARS", "DEMO_PASS").env("DEMO_PASS", "rahasia123").env("FASTPILOT_LIVE_DIR", home.join("live"));
        match &writes {
            Some(w) => {
                c.env("FASTPILOT_JIRA_WRITES", w);
            }
            None => {
                c.env_remove("FASTPILOT_JIRA_WRITES");
            }
        }
    });
    ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize")
}

async fn call(c: &Client, tool: &str, args: Value) -> (bool, String) {
    let r = c.call_tool(rmcp::model::CallToolRequestParams::new(tool.to_owned()).with_arguments(args.as_object().cloned().unwrap_or_default())).await.expect("call");
    let text = r.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect::<Vec<_>>().join("\n");
    (r.is_error != Some(true), text)
}

/// The dashboard, to approve requests the way the person does (a click on Apps).
struct Ui(std::process::Child, u16, String);
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Ui {
    fn start(home: &Path, base: &str) -> Ui {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .args(["ui", "--no-open"])
            .current_dir(home)
            .env("HOME", home)
            .env("FASTPILOT_DIR", home.join("project-fastpilot"))
            .env_remove("FASTPILOT_CONFIG")
            .env("FASTPILOT_KEYSTORE", "file")
            .env("FASTPILOT_JIRA_TEST_BASE", base)
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
        let (_, raw) = Self::raw(port, "GET", &format!("/?code={code}"), "", "");
        let cookie = raw.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().to_owned())).and_then(|c| c.split(';').next().map(str::to_owned)).expect("cookie");
        Ui(child, port, cookie)
    }
    fn raw(port: u16, method: &str, path: &str, cookie: &str, body: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        let req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        s.write_all(req.as_bytes()).expect("write");
        let mut raw = String::new();
        s.read_to_string(&mut raw).expect("read");
        (raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0), raw)
    }
    fn call(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let (st, raw) = Self::raw(self.1, method, path, &self.2, &if body.is_null() { String::new() } else { body.to_string() });
        let b = raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
        (st, serde_json::from_str(b).unwrap_or(json!({ "raw": b })))
    }
}

#[tokio::test]
async fn jira_tools_file_requests_the_person_sends_and_read_only_the_default_project() {
    let fake = fake_jira::start();
    let home = std::env::temp_dir().join(format!("fp-jira-tools-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");
    let mut outputs = Vec::new();

    // Not connected yet.
    let c = mcp(&home, &fake.base, None).await;
    let (ok, out) = call(&c, "jira_read", json!({ "key": "KAN-12" })).await;
    assert!(!ok && out.contains("Jira is not connected") && out.contains("Integrations › Connect Jira"), "{out}");
    c.cancel().await.ok();

    connect(&home, &fake.base);
    let c = mcp(&home, &fake.base, None).await;
    // Read: the ticket as data, planted orders included but marked.
    let (ok, out) = call(&c, "jira_read", json!({ "key": "kan-12" })).await;
    assert!(ok && out.contains("Apply a voucher at checkout") && out.contains("not instructions") && out.contains("untrusted text"), "{out}");
    outputs.push(out);
    let (ok, out) = call(&c, "jira_read", json!({ "key": "KAN-999" })).await;
    assert!(!ok && out.contains("I could not find KAN-999"), "{out}");
    let (ok, out) = call(&c, "jira_read", json!({ "key": "KAN-12/../../rest/api/3/myself" })).await;
    assert!(!ok, "not an issue key: {out}");
    // Medium 2: another project's private ticket is not read for the AI.
    let (ok, out) = call(&c, "jira_read", json!({ "key": "OPS-3" })).await;
    assert!(!ok && out.contains("only tickets of the default project KAN"), "{out}");
    assert!(!fake.log().iter().any(|r| r.path.contains("OPS-3")), "not even asked");
    assert!(fake.writes().is_empty(), "reading writes nothing");

    // Create: a duplicate first.
    let summary = "Total does not change after applying voucher SAVE10";
    let description = "Steps: log in with password rahasia123 as qa@corp.example\nExpected: 10% off\nActual: $40.00\nSee https://evil.example/steal and https://yourteam.atlassian.net/browse/KAN-12 @alex";
    let (ok, out) = call(&c, "jira_create_issue", json!({ "summary": summary, "description": description })).await;
    assert!(ok && out.starts_with("Not created: an open issue looks the same: KAN-120"), "{out}");
    // Medium 1: the AI's call sends nothing; it files a request with the cleaned text.
    let (ok, out) = call(&c, "jira_create_issue", json!({ "summary": summary, "description": description, "new_anyway": true })).await;
    assert!(ok && out.starts_with("Not sent yet: waiting for the user's OK") && out.contains("fastpilot jira approve") && out.contains("Apps"), "{out}");
    assert!(out.contains("password ***") && out.contains("[email removed]") && out.contains("[link removed]") && !out.contains("rahasia123"), "{out}");
    outputs.push(out);
    let (ok, out) = call(&c, "jira_comment", json!({ "key": "KAN-12", "text": "fastpilot ran \"Checkout with voucher SAVE10\": failed at step 2." })).await;
    assert!(ok && out.starts_with("Not sent yet") && out.contains("Comment on KAN-12"), "{out}");
    outputs.push(out);
    // The same request again is not filed twice.
    let (_, again) = call(&c, "jira_comment", json!({ "key": "KAN-12", "text": "fastpilot ran \"Checkout with voucher SAVE10\": failed at step 2." })).await;
    assert!(again.starts_with("Not sent yet"), "{again}");
    assert!(fake.writes().is_empty(), "nothing sent by the AI");
    c.cancel().await.ok();

    // `fastpilot jira approve` needs a person at a terminal: a script (no TTY) is refused.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["jira", "approve"])
        .env("HOME", &home)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_JIRA_TEST_BASE", &fake.base)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("jira approve");
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("Needs a person"), "{out:?}");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).args(["jira", "status"]).env("HOME", &home).env_remove("FASTPILOT_CONFIG").env("FASTPILOT_KEYSTORE", "file").env("FASTPILOT_JIRA_TEST_BASE", &fake.base).output().expect("status");
    let status = String::from_utf8_lossy(&status.stdout).into_owned();
    assert!(status.contains("connected to yourteam.atlassian.net") && status.contains("2 requests from Claude Code are waiting") && !status.contains(fake_jira::TOKEN), "{status}");
    assert!(fake.writes().is_empty());

    // The person sends one from the Integrations page and drops the other.
    let ui = Ui::start(&home, &fake.base);
    let (st, apps) = ui.call("GET", "/api/integrations", Value::Null);
    let reqs = apps["jira_requests"].as_array().cloned().unwrap_or_default();
    assert!(st == 200 && reqs.len() == 2, "{apps}");
    let issue = reqs.iter().find(|r| r["kind"] == "issue").expect("issue");
    let comment = reqs.iter().find(|r| r["kind"] == "comment").expect("comment");
    assert_eq!((issue["title"].as_str(), issue["summary"].as_str()), (Some("Create a Jira issue in KAN"), Some(summary)));
    let (st, r) = ui.call("POST", "/api/jira", json!({ "action": "approve", "id": issue["id"] }));
    assert!(st == 200 && r["message"] == "Created KAN-123." && r["link"] == "https://yourteam.atlassian.net/browse/KAN-123", "{st} {r}");
    let w = fake.writes();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].path == "/rest/api/3/issue" && w[0].right_token && w[0].body.contains("password ***") && !w[0].body.contains("rahasia123") && !w[0].body.contains("evil.example") && w[0].body.contains("\"issuetype\":{\"name\":\"Bug\"}"), "{w:?}");
    let (st, _) = ui.call("POST", "/api/jira", json!({ "action": "approve", "id": issue["id"] }));
    assert_eq!(st, 404, "decided once");
    let (st, r) = ui.call("POST", "/api/jira", json!({ "action": "decline", "id": comment["id"] }));
    assert!(st == 200 && r["requests"].as_array().is_some_and(Vec::is_empty), "{r}");
    assert_eq!(fake.writes().len(), 1, "a dropped request sends nothing");
    drop(ui);

    // Security Low 2: no environment value (a project's .mcp.json could set one) skips the request.
    let c = mcp(&home, &fake.base, Some("direct")).await;
    let (ok, out) = call(&c, "jira_comment", json!({ "key": "KAN-12", "text": "Passed on the second build." })).await;
    assert!(ok && out.starts_with("Not sent yet"), "{out}");
    outputs.push(out);
    assert_eq!(fake.writes().len(), 1, "still nothing sent by the AI");
    assert!(fake.log().iter().all(|r| r.path.starts_with("/rest/api/3/") && !r.path.contains("transitions")), "no status change, only Jira's API");
    c.cancel().await.ok();

    // The dashboard Chat (cards) and unattended runs (off) neither write nor read.
    for (mode, says) in [("cards", "go through a card the user approves"), ("off", "nobody to approve")] {
        let c = mcp(&home, &fake.base, Some(mode)).await;
        let (ok, out) = call(&c, "jira_create_issue", json!({ "summary": "x", "description": "y", "new_anyway": true })).await;
        assert!(!ok && out.contains(says) && out.to_lowercase().contains("nothing was sent"), "{mode}: {out}");
        let (ok, out) = call(&c, "jira_comment", json!({ "key": "KAN-12", "text": "y" })).await;
        assert!(!ok && out.contains(says), "{mode}: {out}");
        let (ok, _) = call(&c, "jira_read", json!({ "key": "KAN-12" })).await;
        assert!(!ok, "{mode}: no reads either");
        c.cancel().await.ok();
    }
    assert_eq!(fake.writes().len(), 1, "no more writes");

    // Atlassian stops accepting the token: said plainly, and Apps shows Needs attention.
    fake.stop_accepting();
    let c = mcp(&home, &fake.base, None).await;
    let (ok, out) = call(&c, "jira_read", json!({ "key": "KAN-12" })).await;
    assert!(!ok && out.contains("Jira did not accept the email or token"), "{out}");
    c.cancel().await.ok();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(home.join(".fastpilot/jira.json")).expect("settings")).expect("json");
    assert_eq!(saved["needs_attention"], true);

    // The token is in no result and in no file but the key store.
    for o in &outputs {
        assert!(!o.contains(fake_jira::TOKEN), "{o}");
    }
    for f in ["jira-requests.json", "audit.jsonl", "jira.json"] {
        let t = std::fs::read_to_string(home.join(".fastpilot").join(f)).unwrap_or_default();
        assert!(!t.contains(fake_jira::TOKEN), "{f}: {t}");
    }
    let audit = std::fs::read_to_string(home.join(".fastpilot/audit.jsonl")).expect("audit");
    assert!(audit.contains("\"channel\":\"dashboard\"") && audit.contains("\"result\":\"created\"") && audit.contains("\"result\":\"declined\"") && !audit.contains("voucher"), "{audit}");
    let _ = std::fs::remove_dir_all(&home);
}

/// `fastpilot init` and the docs never pre-approve the Jira write tools: Claude Code keeps asking
/// before each one.
#[test]
fn nothing_pre_approves_the_jira_writes() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let init = std::fs::read_to_string(format!("{root}/bin/fastpilot/src/init.rs")).expect("init.rs");
    assert!(!init.contains("jira_create_issue") && !init.contains("jira_comment") && !init.contains("allowedTools"), "init writes no allow rule");
    let guide = std::fs::read_to_string(format!("{root}/docs/guide.md")).expect("guide");
    for line in guide.lines().filter(|l| l.contains("mcp__fastpilot__jira")) {
        assert!(!line.contains("allow"), "the guide never allows a Jira write tool: {line}");
    }
    let chat = std::fs::read_to_string(format!("{root}/bin/fastpilot/src/chat.rs")).expect("chat.rs");
    assert!(chat.contains("mcp__fastpilot__config,mcp__fastpilot__jira_read,mcp__fastpilot__jira_create_issue,mcp__fastpilot__jira_comment") && chat.contains("\"FASTPILOT_JIRA_WRITES\": \"cards\""), "Chat cannot call the Jira tools");
    assert!(!init.contains("FASTPILOT_JIRA_WRITES"), "init never turns on direct writes");
    let run = std::fs::read_to_string(format!("{root}/bin/fastpilot/src/run.rs")).expect("run.rs");
    assert!(run.contains("fp.set_jira_writes_off();"), "runs and schedules cannot write");
}
