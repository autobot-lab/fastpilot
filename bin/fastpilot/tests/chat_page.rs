//! FastTest M3 in Chat: "Save test only" on a test card whose file name is already taken said
//! nothing where the person clicked. The refusal now stays in the card. Then (L062-13) the card's
//! Edit path: after that refusal, Edit opens the form with the bot's draft, the person changes the
//! title, a step and the file name, and "Save test only" saves exactly that. Driven by fastpilot in
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
    eval(&client, "document.getElementById('chat-send').click(), 'ok'").await;
    see(&client, "Save test only").await;
    eval(&client, "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Save test only').click(), 'ok'").await;
    see(&client, "Not saved:").await;
    let said = eval(&client, "(() => { const r = document.querySelector('.prop-refused'); return r ? r.textContent + '|' + r.getAttribute('role') + '|' + (document.activeElement === r) : 'none'; })()").await;
    assert!(said.contains("already saved") && said.contains("|alert|true"), "{said}");
    assert_eq!(std::fs::read_to_string(project.join("tests/login-demo.md")).expect("kept"), "# Login\n", "the existing test is kept");

    // L062-13: Edit after the refused click. The form holds the bot's draft.
    eval(&client, "[...document.querySelectorAll('.prop.test button')].find((b) => b.textContent.trim() === 'Edit').click(), 'ok'").await;
    see(&client, "Cancel edit").await;
    let form = eval(&client, "(() => { const q = (l) => document.querySelector(`.t-edit [aria-label=\"${l}\"]`); return [q('Test title').value, q('File name').value, q('Steps, one per line').value.split('\\n').length > 0].join('|'); })()").await.trim_matches('"').to_owned();
    assert!(form.ends_with("|login-demo|true") && !form.starts_with('|'), "the draft is in the form: {form}");
    // Cancel edit goes back to the card, untouched; Edit again, then change it and save.
    eval(&client, "[...document.querySelectorAll('.prop.test button')].find((b) => b.textContent.trim() === 'Cancel edit').click(), 'ok'").await;
    see(&client, "Save test only").await;
    assert_eq!(eval(&client, "String(!!document.querySelector('.t-edit'))").await.trim_matches('"'), "false", "Cancel edit closes the form");
    eval(&client, "[...document.querySelectorAll('.prop.test button')].find((b) => b.textContent.trim() === 'Edit').click(), 'ok'").await;
    see(&client, "Cancel edit").await;
    let set = "(() => { const put = (l, v) => { const e = document.querySelector(`.t-edit [aria-label=\"${l}\"]`); e.value = v; e.dispatchEvent(new Event('input', { bubbles: true })); }; \
        put('Test title', 'Login with the second account'); put('File name', 'login-second'); \
        put('Steps, one per line', 'Open the login page\\nSign in with b@b.com / ' + '$' + '{DEMO_PASS}\\nCheck that the Dashboard shows'); return 'ok'; })()";
    eval(&client, set).await;
    eval(&client, "[...document.querySelectorAll('.t-edit button')].find((b) => b.textContent.trim() === 'Save test only').click(), 'ok'").await;
    see(&client, "Test saved.").await;
    let saved = std::fs::read_to_string(project.join("tests/login-second.md")).expect("saved under the edited file name");
    assert!(saved.contains("Login with the second account") && saved.contains("Sign in with b@b.com / ${DEMO_PASS}") && saved.contains("Check that the Dashboard shows"), "{saved}");
    assert_eq!(std::fs::read_to_string(project.join("tests/login-demo.md")).expect("kept"), "# Login\n", "the other test is still untouched");
    let card = eval(&client, "(() => { const c = document.querySelector('.prop.test'); return c.className + '|' + c.textContent; })()").await;
    assert!(card.contains("applied") && card.contains("Login with the second account") && card.contains("Saved as tests/login-second.md") && !card.contains("Cancel edit"), "the card shows what was saved: {card}");
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}

/// `fastpilot ui` in `project` with the stand-in Claude, and a fastpilot Chrome session on it.
async fn open_ui(home: &Path, project: &Path, env: &[(&str, &str)]) -> (Ui, Client) {
    std::fs::create_dir_all(project.join("tests")).expect("project");
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(project)
        .env("HOME", home)
        .env("FASTPILOT_DIR", project.join(".fastpilot"))
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env("FASTPILOT_CLAUDE", FAKE)
        .env("FAKE_CHAT_DELAY_MS", "0")
        .env("FAKE_CHAT_NO_RUN", "1")
        .envs(env.iter().copied())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("ui");
    let mut link = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut link).expect("link");
    let ui = Ui(child);
    let viewer_home = home.join("viewer");
    std::fs::create_dir_all(&viewer_home).expect("viewer");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").current_dir(ROOT).env("HOME", &viewer_home).env("FASTPILOT_LIVE_DIR", viewer_home.join("live")).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let flow = format!("- session: {{ start: viewer, platform: web, target: \"{}\" }}\n", link.trim());
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    (ui, client)
}

async fn say(client: &Client, text: &str) {
    let flow = format!("- inputText: {{ into: {{ role: textfield }}, text: {} }}\n", serde_json::to_string(text).expect("text"));
    let (ok, out) = call(client, "run_flow", json!({ "flow": flow, "session": "viewer" })).await;
    assert!(ok, "{out}");
    eval(client, "document.getElementById('chat-send').click(), 'ok'").await;
}

/// v0.7.1 FC4: Chat in the OpenBot style. The agents sidebar lists the agent's chats (each titled
/// from its first message, the open one marked), "+ New chat" starts another, the bot's Markdown
/// is drawn as headings, lists, tables, code and links, and everything that is not Markdown stays
/// text: a `<script>` in a reply is shown, never run; no remote image is loaded; a `javascript:` link
/// is not a link. Under 900 px the sidebar is a drawer behind Menu.
#[tokio::test(flavor = "multi_thread")]
async fn chat_has_chats_in_the_sidebar_and_draws_markdown_safely() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-chat-fc4-page-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let (_ui, client) = open_ui(&home, &home.join("shop"), &[]).await;
    let txt = |s: String| s.trim_matches('"').to_owned();
    eval(&client, "location.hash = '#/agents/default/chat', 'ok'").await;
    see(&client, "What should I test?").await;
    assert_eq!(txt(eval(&client, "String(!!document.querySelector('.rail, .chat-suggest, .say-row'))").await), "false", "no rail, no suggestions");

    say(&client, "show the release table").await;
    see(&client, "Ship with fixes").await;
    let md = txt(eval(&client, "(() => { const m = [...document.querySelectorAll('.msg.bot .md')].pop(); return [m.querySelector('h3')?.textContent, m.querySelector('b')?.textContent, m.querySelectorAll('li').length, m.querySelectorAll('.tablewrap table tr').length, m.querySelector('code')?.textContent, m.querySelectorAll('script, img, iframe').length, String(window.__fpInjected), [...m.querySelectorAll('a')].map((a) => a.getAttribute('href') + ' ' + a.target + ' ' + a.rel).join(';'), m.textContent.includes('<script>window.__fpInjected = 1</script>'), m.textContent.includes('<b>this</b>'), m.textContent.includes('javascript:alert(1)')].join('|'); })()").await);
    assert_eq!(md, "Ship with fixes|passed|2|3|Place order|0|undefined|https://example.com/report _blank noopener noreferrer|true|true|true", "Markdown drawn, HTML and script shown as text, no image, only the https link");

    // The chat is in the sidebar, titled from its first message, and marked as the open one.
    let side = txt(eval(&client, "(() => { const a = document.querySelector('.side .crow[aria-current=\"page\"]'); return a ? a.textContent + '|' + /#\\/agents\\/default\\/chat\\/\\d/.test(location.hash) : 'none'; })()").await);
    assert_eq!(side, "show the release table|true");
    // "+ New chat": the empty chat; its first message makes a second chat, listed first.
    eval(&client, "document.querySelector('.side .crow.newc').click(), 'ok'").await;
    see(&client, "What should I test?").await;
    say(&client, "hello there").await;
    see(&client, "Halo! Mau tes apa?").await;
    let rows = txt(eval(&client, "[...document.querySelectorAll('.side .crow-li .crow')].map((a) => a.textContent + (a.getAttribute('aria-current') ? '*' : '')).join(';')").await);
    assert_eq!(rows, "hello there*;show the release table");
    // Each chat has a labelled remove button.
    assert_eq!(txt(eval(&client, "[...document.querySelectorAll('.side .crow-li .del')].map((b) => b.getAttribute('aria-label')).join(';')").await), "Remove chat “hello there”;Remove chat “show the release table”");

    // Narrow window: the sidebar is a drawer behind Menu; Esc closes it and focus goes back.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- setViewport: { width: 820, height: 900 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    let closed = txt(eval(&client, "getComputedStyle(document.getElementById('side')).visibility + '|' + getComputedStyle(document.getElementById('menubtn')).display").await);
    assert_eq!(closed, "hidden|grid");
    eval(&client, "document.getElementById('menubtn').click(), 'ok'").await;
    let open = txt(eval(&client, "(() => { const s = document.getElementById('side'); return [getComputedStyle(s).visibility, s.getAttribute('role'), s.contains(document.activeElement), document.getElementById('menubtn').getAttribute('aria-expanded')].join('|'); })()").await);
    assert_eq!(open, "visible|dialog|true|true");
    eval(&client, "document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })), 'ok'").await;
    let back = txt(eval(&client, "[document.body.classList.contains('drawer'), document.activeElement && document.activeElement.id].join('|')").await);
    assert_eq!(back, "false|menubtn");
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}
