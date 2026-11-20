//! FastTest M3 in Chat: "Save test only" on a test card whose file name is already taken said
//! nothing where the person clicked. The refusal now stays in the card. Driven by fastpilot in
//! Chrome against `fastpilot ui` with the stand-in Claude; skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::Path;

type Client = RunningService<RoleClient, ()>;
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude-chat/claude");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| Path::new(p).exists())
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
    assert!(ok, "{out}");
    out.lines().find_map(|l| l.rsplit_once(" = ").map(|(_, v)| v.trim().to_owned())).unwrap_or_default()
}

async fn see(client: &Client, text: &str) {
    let flow = format!("- assertVisible: {{ text: {}, timeout: 5000 }}\n", serde_json::to_string(text).expect("text"));
    let (ok, out) = call(client, "run_flow", json!({ "flow": flow, "session": "viewer" })).await;
    assert!(ok, "{text}: {out}");
}

struct Ui(std::process::Child);
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_save_says_why_in_the_card() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-chat-page-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let project = home.join("shop");
    std::fs::create_dir_all(project.join("tests")).expect("project");
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    std::fs::write(project.join("tests/login-demo.md"), "# Login\n").expect("existing");
    let mcp = json!({ "mcpServers": { "fastpilot": { "command": "fastpilot", "args": ["mcp"], "env": { "FASTPILOT_VARS": "DEMO_PASS", "DEMO_PASS": "x" } } } });
    std::fs::write(project.join(".mcp.json"), mcp.to_string()).expect("mcp");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(&project)
        .env("HOME", &home)
        .env("FASTPILOT_DIR", project.join(".fastpilot"))
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env("FASTPILOT_CLAUDE", FAKE)
        .env("FAKE_CHAT_DELAY_MS", "0")
        .env("FAKE_CHAT_TEST_ID", "login-demo")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("ui");
    let mut link = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut link).expect("link");
    let _ui = Ui(child);
    let viewer_home = home.join("viewer");
    std::fs::create_dir_all(&viewer_home).expect("viewer");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").current_dir(ROOT).env("HOME", &viewer_home).env("FASTPILOT_LIVE_DIR", viewer_home.join("live")).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let flow = format!("- session: {{ start: viewer, platform: web, target: \"{}\" }}\n", link.trim());
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    eval(&client, "location.hash = '#/agents', 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- inputText: { into: { role: textfield }, text: \"write a test for the demo login and save it\" }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    eval(&client, "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Send').click(), 'ok'").await;
    see(&client, "Save test only").await;
    eval(&client, "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Save test only').click(), 'ok'").await;
    see(&client, "Not saved:").await;
    let said = eval(&client, "(() => { const r = document.querySelector('.prop-refused'); return r ? r.textContent + '|' + r.getAttribute('role') + '|' + (document.activeElement === r) : 'none'; })()").await;
    assert!(said.contains("already saved") && said.contains("|alert|true"), "{said}");
    assert_eq!(std::fs::read_to_string(project.join("tests/login-demo.md")).expect("kept"), "# Login\n", "the existing test is kept");
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}
