//! FastTest M3 on the Memory page's Project notes: typing into the empty form turns Save note on
//! (it stayed off, so the first note could not be saved), and a note added elsewhere (Chat, another
//! tab) shows when the tab opens again instead of a stale "0 notes". Driven by fastpilot in Chrome
//! with real key presses; skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::Path;

type Client = RunningService<RoleClient, ()>;
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

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
async fn a_typed_note_can_be_saved_and_the_tab_is_never_stale() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-notes-page-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let project = home.join("shop");
    std::fs::create_dir_all(&project).expect("project");
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(&project)
        .env("HOME", &home)
        .env("FASTPILOT_DIR", project.join(".fastpilot"))
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
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
    eval(&client, "location.hash = '#/memory', 'ok'").await;
    let btn = |label: &str| format!("[...document.querySelectorAll('button')].find(b => b.textContent.trim().startsWith({label:?}))");
    eval(&client, &format!("{}.click(), 'ok'", btn("Project notes"))).await;
    // UXA8: notes come from Chat ("Remember this?"); typing one is under More options, closed.
    see(&client, "Open Chat").await;
    let typing = "(() => { const t = document.querySelector('.note-text'); return [document.body.innerText.includes('Remember this?'), !!t && !!t.closest('[hidden]'), document.querySelector('.mem-side .more-btn').textContent].join('|'); })()";
    assert_eq!(eval(&client, typing).await, "\"true|true|Type a note yourself\"", "the typing form waits under More options");
    eval(&client, "document.querySelector('.mem-side .more-btn').click(), 'ok'").await;
    see(&client, "Save note").await;
    assert_eq!(eval(&client, "String([...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Save note').disabled)").await, "\"true\"", "empty form: off");

    // Typed with real key presses: Save note turns on, and saves.
    eval(&client, &format!("{}.click(), 'ok'", btn("Addresses"))).await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- inputText: { into: { role: textfield, name: Note }, text: \"Staging shop: https://staging.shop.example\" }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    assert_eq!(eval(&client, "String([...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Save note').disabled)").await, "\"false\"", "typed: on");
    eval(&client, "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Save note').click(), 'ok'").await;
    see(&client, "1 of 20 notes").await;

    // A note added elsewhere (here: the API, as a Chat card or another tab would) shows when the tab opens.
    eval(&client, &format!("{}.click(), 'ok'", btn("Tests"))).await;
    let added = eval(&client, "fetch('/api/notes', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ action: 'add', group: 'how', text: 'Login uses a one-time code.' }) }).then((r) => String(r.status))").await;
    assert_eq!(added, "\"200\"");
    eval(&client, &format!("{}.click(), 'ok'", btn("Project notes"))).await;
    see(&client, "2 of 20 notes").await;
    assert_eq!(eval(&client, "String(document.body.innerText.includes('Login uses a one-time code'))").await, "\"true\"");
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}
