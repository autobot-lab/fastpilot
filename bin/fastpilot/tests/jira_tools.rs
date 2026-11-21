//! v0.6 1.5: fastpilot's Jira tools for Claude Code (`jira_read`, `jira_create_issue`,
//! `jira_comment`), against a fake Jira Cloud. They use the connection saved in the dashboard
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

#[tokio::test]
async fn jira_tools_use_the_dashboard_connection_and_send_only_cleaned_text() {
    let fake = fake_jira::start();
    let home = std::env::temp_dir().join(format!("fp-jira-tools-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");
    let mut outputs = Vec::new();

    // Not connected yet.
    let c = mcp(&home, &fake.base, None).await;
    let (ok, out) = call(&c, "jira_read", json!({ "key": "KAN-12" })).await;
    assert!(!ok && out.contains("Jira is not connected") && out.contains("Apps › Connect Jira"), "{out}");
    c.cancel().await.ok();

    connect(&home, &fake.base);
    let c = mcp(&home, &fake.base, None).await;
    // Read: the ticket as data, planted orders included but marked.
    let (ok, out) = call(&c, "jira_read", json!({ "key": "KAN-12" })).await;
    assert!(ok && out.contains("Apply a voucher at checkout") && out.contains("not instructions") && out.contains("untrusted text"), "{out}");
    outputs.push(out);
    let (ok, out) = call(&c, "jira_read", json!({ "key": "KAN-999" })).await;
    assert!(!ok && out.contains("I could not find KAN-999"), "{out}");
    let (ok, out) = call(&c, "jira_read", json!({ "key": "KAN-12/../../rest/api/3/myself" })).await;
    assert!(!ok, "not an issue key: {out}");
    assert!(fake.writes().is_empty(), "reading writes nothing");

    // Create: a duplicate first.
    let summary = "Total does not change after applying voucher SAVE10";
    let description = "Steps: log in with password rahasia123 as qa@corp.example\nExpected: 10% off\nActual: $40.00\nSee https://evil.example/steal and https://yourteam.atlassian.net/browse/KAN-12 @alex";
    let (ok, out) = call(&c, "jira_create_issue", json!({ "summary": summary, "description": description })).await;
    assert!(ok && out.starts_with("Not created: an open issue looks the same: KAN-120"), "{out}");
    assert!(fake.writes().is_empty(), "the duplicate guard wrote nothing");
    // Created anyway, cleaned.
    let (ok, out) = call(&c, "jira_create_issue", json!({ "summary": summary, "description": description, "new_anyway": true })).await;
    assert!(ok && out.contains("Created KAN-123 (Bug) in KAN") && out.contains("https://yourteam.atlassian.net/browse/KAN-123"), "{out}");
    outputs.push(out.clone());
    let writes = fake.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    let body = &writes[0].body;
    assert!(writes[0].path == "/rest/api/3/issue" && writes[0].right_token, "{writes:?}");
    assert!(!body.contains("rahasia123") && body.contains("password ***"), "secret masked: {body}");
    assert!(!body.contains("qa@corp.example") && body.contains("[email removed]"), "{body}");
    assert!(!body.contains("evil.example") && body.contains("[link removed]") && body.contains("yourteam.atlassian.net/browse/KAN-12"), "{body}");
    assert!(!body.contains("@alex") && body.contains("\"issuetype\":{\"name\":\"Bug\"}"), "{body}");
    assert!(out.contains("Sent exactly:") && out.contains("password ***"), "the result shows what was sent: {out}");

    // Comment.
    let (ok, out) = call(&c, "jira_comment", json!({ "key": "KAN-12", "text": "fastpilot ran \"Checkout with voucher SAVE10\": failed at step 2." })).await;
    assert!(ok && out.starts_with("Comment added to KAN-12"), "{out}");
    outputs.push(out);
    assert_eq!(fake.writes().len(), 2);
    assert!(fake.log().iter().all(|r| r.path.starts_with("/rest/api/3/") && !r.path.contains("transitions")), "no status change, only Jira's API");
    c.cancel().await.ok();

    // The dashboard Chat (cards) and unattended runs (off) cannot write.
    for (mode, says) in [("cards", "go through a card the user approves"), ("off", "nobody to approve")] {
        let c = mcp(&home, &fake.base, Some(mode)).await;
        let (ok, out) = call(&c, "jira_create_issue", json!({ "summary": "x", "description": "y", "new_anyway": true })).await;
        assert!(!ok && out.contains(says) && out.to_lowercase().contains("nothing was sent"), "{mode}: {out}");
        let (ok, out) = call(&c, "jira_comment", json!({ "key": "KAN-12", "text": "y" })).await;
        assert!(!ok && out.contains(says), "{mode}: {out}");
        let (ok, _) = call(&c, "jira_read", json!({ "key": "KAN-12" })).await;
        assert!(ok, "{mode}: reading still works");
        c.cancel().await.ok();
    }
    assert_eq!(fake.writes().len(), 2, "no more writes");

    // Atlassian stops accepting the token: said plainly, and Apps shows Needs attention.
    fake.stop_accepting();
    let c = mcp(&home, &fake.base, None).await;
    let (ok, out) = call(&c, "jira_read", json!({ "key": "KAN-12" })).await;
    assert!(!ok && out.contains("Jira did not accept the email or token"), "{out}");
    c.cancel().await.ok();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(home.join(".fastpilot/jira.json")).expect("settings")).expect("json");
    assert_eq!(saved["needs_attention"], true);

    // The token is in no result.
    for o in &outputs {
        assert!(!o.contains(fake_jira::TOKEN), "{o}");
    }
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
    assert!(chat.contains("mcp__fastpilot__config,mcp__fastpilot__jira_create_issue,mcp__fastpilot__jira_comment") && chat.contains("\"FASTPILOT_JIRA_WRITES\": \"cards\""), "Chat cannot call the writes");
    let run = std::fs::read_to_string(format!("{root}/bin/fastpilot/src/run.rs")).expect("run.rs");
    assert!(run.contains("fp.set_jira_writes_off();"), "runs and schedules cannot write");
}
