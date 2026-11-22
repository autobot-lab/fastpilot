//! v0.6 QA bugs 2 and 3, driven like a person in Chrome (real clicks and typing, by fastpilot
//! itself) against a fake Jira Cloud: the first "Test connection" press after typing the token
//! works (leaving a field used to redraw the whole form, so the next click or focus was lost), and
//! after Replace token the default project stays chosen. Skips without Chrome.

#[path = "support/fake_jira.rs"]
mod fake_jira;

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::Path;

type Client = RunningService<RoleClient, ()>;

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists())
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

struct Ui(std::process::Child);
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn myself_calls(fake: &fake_jira::Fake) -> usize {
    fake.log().iter().filter(|r| r.path == "/rest/api/3/myself").count()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_test_press_works_and_replace_token_keeps_the_project() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let fake = fake_jira::start();
    let home = std::env::temp_dir().join(format!("fp-jira-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .env("HOME", &home)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_JIRA_TEST_BASE", &fake.base)
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut link = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut link).expect("link");
    let _ui = Ui(child);
    let person_home = home.join("person");
    std::fs::create_dir_all(&person_home).expect("person");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp")
            .env("HOME", &person_home)
            .env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true")
            .env("FASTPILOT_LIVE_DIR", person_home.join("live"))
            .env("FASTPILOT_VARS", "JIRA_TOKEN")
            .env("JIRA_TOKEN", fake_jira::TOKEN)
            .env_remove("FASTPILOT_CONFIG")
            .env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = flow(&client, &format!("- session: {{ start: person, platform: web, target: \"{}\" }}\n- assertVisible: {{ text: \"Integrations\", timeout: 10000 }}\n", link.trim())).await;
    assert!(ok, "{out}");
    eval(&client, "location.hash = '#/apps/jira', 'ok'").await;

    // Bug 2: type the three fields one after another, then one press of Test connection.
    let (ok, out) = flow(
        &client,
        "- assertVisible: { text: \"Connect Jira\", timeout: 8000 }\n- inputText: { into: Jira address, text: yourteam }\n- inputText: { into: Email, text: alex@yourteam.com }\n- inputText: { into: { role: textfield, name: API token }, text: \"${JIRA_TOKEN}\" }\n- tapOn: Test connection\n- assertVisible: { text: \"Connected as\", timeout: 8000 }\n",
    )
    .await;
    assert!(ok, "the first press must test the connection: {out}");
    assert_eq!(myself_calls(&fake), 1, "one press, one test");
    // Choose the project and save.
    eval(&client, "(() => { const s = document.querySelector('select[aria-label=\"Default project\"]'); s.value = 'OPS'; s.dispatchEvent(new Event('change')); return 'ok'; })()").await;
    let (ok, out) = flow(&client, "- tapOn: Connect Jira\n- assertVisible: { text: \"Connected\", timeout: 8000 }\n").await;
    assert!(ok, "{out}");
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(home.join(".fastpilot/jira.json")).expect("saved")).expect("json");
    assert_eq!(saved["project_key"], "OPS");

    // Bug 3: Manage, Replace token, one Test press: the default project stays OPS.
    eval(&client, "location.hash = '#/apps/jira', 'ok'").await;
    let before = myself_calls(&fake);
    let (ok, out) = flow(
        &client,
        "- assertVisible: { text: \"Replace token\", timeout: 8000 }\n- tapOn: Replace token\n- inputText: { into: { role: textfield, name: API token }, text: \"${JIRA_TOKEN}\" }\n- tapOn: Test connection\n- assertVisible: { text: \"Connected as\", timeout: 8000 }\n",
    )
    .await;
    assert!(ok, "{out}");
    assert_eq!(myself_calls(&fake), before + 1, "one press after Replace token, one test");
    assert_eq!(eval(&client, "document.querySelector('select[aria-label=\"Default project\"]').value").await, "OPS", "the project is kept after Replace token");
    assert_eq!(eval(&client, "String([...document.querySelectorAll('.top button')].find((b) => b.textContent.includes('Save')).disabled)").await, "false", "Save changes is ready without choosing again");
    // QA re-test Low 8: a request dropped elsewhere (as `fastpilot jira approve` does: it takes it
    // out of the list) leaves "Waiting for your OK" without a reload.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let request = serde_json::json!([{ "id": "a1b2c3d4", "kind": "comment", "site": fake_jira::SITE, "project": "", "key": "KAN-12", "summary": "", "text": "Passed on build 7", "created_ms": now }]);
    std::fs::write(home.join(".fastpilot/jira-requests.json"), request.to_string()).expect("request");
    eval(&client, "location.hash = '#/apps', 'ok'").await;
    let (ok, out) = flow(&client, "- assertVisible: { text: \"Waiting for your OK\", timeout: 8000 }\n").await;
    assert!(ok, "{out}");
    std::fs::write(home.join(".fastpilot/jira-requests.json"), "[]").expect("dropped");
    let (ok, out) = flow(&client, "- assertNotVisible: { text: \"Waiting for your OK\", timeout: 10000 }\n").await;
    assert!(ok, "the dropped request leaves the page without a reload: {out}");
    let _ = client.cancel().await;
    let _ = std::fs::remove_dir_all(&home);
}
