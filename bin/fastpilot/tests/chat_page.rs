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
    // E1: one message only: the red refusal replaces the note that said the same.
    let twice = eval(&client, "String(document.querySelector('.prop.test').textContent.split('is already saved').length - 1)").await;
    assert_eq!(twice.trim_matches('"'), "1", "the same message is shown once");
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
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    cmd.args(["ui", "--no-open"])
        .current_dir(project)
        .env("HOME", home)
        .env("FASTPILOT_DIR", project.join(".fastpilot"))
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env("FASTPILOT_CLAUDE", FAKE)
        .env("FAKE_CHAT_DELAY_MS", "0")
        .env("FAKE_CHAT_NO_RUN", "1")
        .envs(env.iter().copied())
        .stdout(std::process::Stdio::piped());
    // `("FASTPILOT_DIR", "")`: as installed, with apps (Chat's fastpilot works for the app in use).
    if env.contains(&("FASTPILOT_DIR", "")) {
        cmd.env_remove("FASTPILOT_DIR");
    }
    let mut child = cmd.spawn().expect("ui");
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

/// v0.7.1 FC8: the Skills tab. General QA is locked: its 8 skills are plain text marked "Always
/// on", with no switch. The 15 specialist skills are real switches (`role=switch`, `aria-checked`,
/// the skill's name as their name) that act at once with Undo; "Back to General QA" turns them all
/// off and is greyed when none is on; Jira skills are not listed until Jira is connected. An agent
/// saved before v0.7.1 with no skill list becomes General QA when the dashboard starts, with an Undo
/// that brings the old list back; a list that was chosen is kept as it is.
#[tokio::test(flavor = "multi_thread")]
async fn the_skills_tab_has_general_qa_locked_and_specialist_switches() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-skills-tab-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let agents = json!({ "agents": [
        { "id": "default", "name": "My tester", "job": "Tests this project.", "devices": ["web"], "skills": [] },
        { "id": "checkout-qa", "name": "Checkout QA", "job": "Only the checkout", "devices": ["web"], "skills": ["choose-technique", "safe-checkout"] },
    ] });
    std::fs::write(home.join(".fastpilot/agents.json"), agents.to_string()).expect("agents");
    let (_ui, client) = open_ui(&home, &home.join("shop"), &[]).await;
    let txt = |s: String| s.trim_matches('"').to_owned();
    let saved = || -> Value { serde_json::from_str(&std::fs::read_to_string(home.join(".fastpilot/agents.json")).expect("agents")).expect("json") };
    let skills_of = |id: &str| -> Vec<String> { saved()["agents"].as_array().and_then(|a| a.iter().find(|x| x["id"] == id)).map(|x| x["skills"].as_array().cloned().unwrap_or_default().iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default() };
    // The migration: no list → General QA (8); a chosen list stays.
    assert_eq!(skills_of("default"), ["choose-technique", "use-case-scenarios", "edge-cases", "exploratory-check", "regression", "bug-report", "test-report", "release-check"]);
    assert_eq!(skills_of("checkout-qa"), ["choose-technique", "safe-checkout"]);

    eval(&client, "location.hash = '#/agents/default/skills', 'ok'").await;
    see(&client, "General QA · Default · Always on").await;
    let page = txt(eval(&client, "(() => { const locked = [...document.querySelectorAll('.sk-card.locked')]; const sw = [...document.querySelectorAll('.skills [role=switch]')]; const back = [...document.querySelectorAll('.skills button')].find((b) => b.textContent === 'Back to General QA'); return [locked.length, locked.every((c) => c.textContent.includes('Always on') && !c.querySelector('button, [role=switch]')), sw.length, sw.every((b) => b.tagName === 'BUTTON' && b.getAttribute('aria-checked') === 'false' && b.getAttribute('aria-label')), sw.some((b) => b.getAttribute('aria-label').startsWith('Jira')), back.disabled, document.body.textContent.includes('Jira skills appear after you connect Jira.'), [...document.querySelectorAll('.sk-group h3')].map((h) => h.textContent).join(',')].join('|'); })()").await);
    assert_eq!(page, "8|true|12|true|false|true|true|Black-box,Experience-based,Collaboration", "locked General QA, 12 switches without Jira, Back greyed");

    // A switch acts at once: the line says what changed, with Undo; the header counts it.
    eval(&client, "document.querySelector('[role=switch][aria-label=\"Checklist · Safe checkout\"]').click(), 'ok'").await;
    see(&client, "General QA + 1 specialist skill").await;
    let after = txt(eval(&client, "[document.querySelector('[role=switch][aria-label=\"Checklist · Safe checkout\"]').getAttribute('aria-checked'), document.querySelector('.sk-done .rt').textContent, !!document.querySelector('.sk-done .rl-undo'), document.querySelector('.sk-done').getAttribute('role')].join('|')").await);
    assert_eq!(after, "true|My tester: + Safe checkout|true|status");
    assert!(skills_of("default").contains(&"safe-checkout".to_owned()));
    eval(&client, "document.querySelector('[role=switch][aria-label=\"Decision table\"]').click(), 'ok'").await;
    see(&client, "General QA + 2 specialist skills").await;
    // Back to General QA: every specialist off, with Undo; then greyed again.
    eval(&client, "[...document.querySelectorAll('.skills button')].find((b) => b.textContent === 'Back to General QA').click(), 'ok'").await;
    see(&client, "General QA · Default · Always on").await;
    assert_eq!(txt(eval(&client, "document.querySelector('.sk-done .rt').textContent").await), "My tester: back to General QA (− Safe checkout, − Decision table)");
    assert_eq!(skills_of("default").len(), 8, "General QA only");
    eval(&client, "document.querySelector('.sk-done .rl-undo').click(), 'ok'").await;
    see(&client, "General QA + 2 specialist skills").await;
    assert!(skills_of("default").contains(&"decision-table".to_owned()), "Undo puts them back");
    // Skills never touch the limits.
    assert!(!std::fs::read_to_string(home.join(".fastpilot/config.toml")).unwrap_or_default().contains("block_actions"));
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}

/// v0.7.1 FC-STOP: "About this agent" opens from the agent's name as a dialog that reads first:
/// focus on Close, its rows as text (nothing drawn as "[object …]"), Skills as one line to the
/// Skills tab, no Stop row while nothing runs, Esc closes it and focus goes back to the name.
#[tokio::test(flavor = "multi_thread")]
async fn the_about_panel_reads_first_and_closes_with_escape() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-about-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let (_ui, client) = open_ui(&home, &home.join("shop"), &[]).await;
    let txt = |s: String| s.trim_matches('"').to_owned();
    eval(&client, "location.hash = '#/agents/default/chat', 'ok'").await;
    see(&client, "What should I test?").await;
    eval(&client, "document.querySelector('.hdr2 .agname').click(), 'ok'").await;
    see(&client, "About My tester").await;
    let p = txt(eval(&client, "(() => { const d = document.getElementById('about-panel'); return [d.getAttribute('role'), d.getAttribute('aria-modal'), document.activeElement.getAttribute('aria-label'), d.textContent.includes('[object'), d.querySelector('.ab-link').getAttribute('href'), d.querySelector('.ab-link').textContent, !!d.querySelector('.ab-run'), !!d.querySelector('[data-fk=remove]'), d.textContent.includes('Tests this project.')].join('|'); })()").await);
    assert_eq!(p, "dialog|true|Close|false|#/agents/default/skills|General QA|false|false|true");
    eval(&client, "document.getElementById('about-panel').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })), 'ok'").await;
    assert_eq!(txt(eval(&client, "[!!document.getElementById('about-panel'), document.activeElement.classList.contains('agname')].join('|')").await), "false|true");
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}

/// FastTest 7345 N2: "Save and run" runs. "ask me for the password and then check the login with
/// a@b.com": the bot runs nothing in that answer and shows the password card. The person clicks
/// Save and run and types the password in the masked dialog; the check then starts in that chat
/// by itself (fastpilot's sentence, then a new answer that opens the browser session).
#[tokio::test(flavor = "multi_thread")]
async fn save_and_run_runs_the_check_after_an_answer_that_ran_nothing() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-save-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let (_ui, client) = open_ui(&home, &home.join("shop"), &[("FAKE_CHAT_NO_RUN", ""), ("FASTPILOT_KEYSTORE", "file")]).await;
    eval(&client, "(async () => { await api('/api/apps', { action: 'add', name: 'Shop', targets: { web: 'https://shop.example' } }); location.hash = '#/agents/default/chat'; return 'ok'; })()").await;
    see(&client, "What should I test?").await;
    say(&client, "ask me for the password and then check the login with a@b.com").await;
    see(&client, "Save and run").await;
    eval(&client, "[...document.querySelectorAll('.prop .btn.p')].find((b) => b.textContent === 'Save and run').click(), 'ok'").await;
    see(&client, "Set DEMO_PASS").await;
    eval(&client, "(() => { const i = document.querySelector('dialog.cdlg input'); i.value = 'fp-save-run-1'; i.form.requestSubmit(); return 'ok'; })()").await;
    see(&client, "The test account is saved now. Run that check again with it.").await;
    // The stand-in answers "Correct login: …" only after it opened the browser session for the check.
    see(&client, "Correct login:").await;
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}

/// E1: a step types a one-time code sent to the person's phone. The run waits; Chat shows one
/// hidden field ("Type the code sent to your phone"); the person types it once and the step goes on
/// with it. The code is never in the chat, in what the bot got, or left on disk.
#[tokio::test(flavor = "multi_thread")]
async fn the_dashboard_asks_for_a_one_time_code_while_the_step_waits() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-otp-ask-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("site")).expect("site");
    let page = home.join("site/code.html");
    std::fs::write(&page, "<!doctype html><label>Code <input id=c aria-label=Code></label><p id=o>waiting</p><script>c.addEventListener('input', () => { o.textContent = c.value === '48213' ? 'code accepted' : 'wrong code'; });</script>").expect("page");
    let target = std::fs::canonicalize(&page).expect("page").to_string_lossy().into_owned();
    let (_ui, client) = open_ui(&home, &home.join("shop"), &[("FAKE_CHAT_NO_RUN", ""), ("FAKE_CHAT_TARGET", &target), ("FASTPILOT_DIR", ""), ("FASTPILOT_KEYSTORE", "file")]).await;
    // The code is typed only on the app's own site (Security #233 M1): the app is this page.
    let add = format!("(async () => {{ await api('/api/apps', {{ action: 'add', name: 'Shop', targets: {{ web: {} }} }}); location.hash = '#/agents/default/chat'; return 'ok'; }})()", serde_json::to_string(&format!("file://{target}")).expect("url"));
    eval(&client, &add).await;
    see(&client, "What should I test?").await;
    say(&client, "tes kode dari HP saya").await;
    see(&client, "Type the code sent to your phone").await;
    let field = eval(&client, "(() => { const i = document.querySelector('#code-ask input'); return [i.type, i.autocomplete, document.querySelectorAll('#code-ask input').length, document.getElementById('code-ask').textContent.includes('MOTMAINA_OTP')].join('|'); })()").await;
    assert_eq!(field.trim_matches('"'), "password|one-time-code|1|true", "one hidden field, named");
    eval(&client, "(() => { document.querySelector('#code-ask input').value = '48213'; [...document.querySelectorAll('#code-ask button')].find((b) => b.textContent === 'Use code').click(); return 'ok'; })()").await;
    see(&client, "Code step: passed.").await;
    assert_eq!(eval(&client, "String(!!document.getElementById('code-ask'))").await.trim_matches('"'), "false", "the field is gone");
    // Never kept: not in the chat, not in what the bot was given, nothing left in the code folder.
    let chat = eval(&client, "(async () => JSON.stringify(await api('/api/chat?agent=default')))()").await;
    assert!(!chat.contains("48213"), "{chat}");
    for e in std::fs::read_dir(home.join("claude-log")).into_iter().flatten().flatten() {
        assert!(!std::fs::read_to_string(e.path()).unwrap_or_default().contains("48213"), "the bot never gets the code");
    }
    assert_eq!(std::fs::read_dir(home.join(".fastpilot/ui/codes")).map(|d| d.count()).unwrap_or(0), 0, "nothing left on disk");
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}

/// S1 (FastTest backlog): a chat that waits its turn says "Waiting · 1 ahead" on its sidebar row,
/// and every tab, button and row shows the 2 px blue focus ring when reached with the keyboard.
#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_chat_says_how_many_go_first_and_keyboard_focus_shows() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-s1-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    std::fs::write(home.join(".fastpilot/config.toml"), "max_parallel = 1\n").expect("config");
    let (_ui, client) = open_ui(&home, &home.join("shop"), &[("FAKE_CHAT_DELAY_MS", "6000")]).await;
    eval(&client, "(async () => { await api('/api/chat', { agent: 'default', new: true, message: 'hello from the first chat' }); const b = await api('/api/chat', { agent: 'default', new: true, message: 'hello from the second chat' }); location.hash = '#/agents/default/chat/' + b.chat; return 'ok'; })()").await;
    see(&client, "Waiting · 1 ahead").await;
    let ring = eval(&client, "(() => { const out = []; for (const sel of ['.tabs2 a', '.crow', 'button.btn, button']) { const el = document.querySelector(sel); if (!el) { out.push(sel + ':none'); continue; } el.focus({ focusVisible: true }); const s = getComputedStyle(el); out.push(s.outlineStyle + ' ' + s.outlineWidth + ' ' + s.outlineColor + ' ' + s.outlineOffset); } return out.join('|'); })()").await;
    for part in ring.trim_matches('"').split('|') {
        assert!(part.starts_with("solid 2px rgb(36, 89, 214) 2px") || part.ends_with(":none"), "focus ring: {ring}");
    }
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}

/// F3 (FastTest v0.7.2): "rename this app to …" and "use Haiku for this agent" in Chat show their
/// result line with an Undo button (design e3a, g19f), and the button puts the old value back.
#[tokio::test(flavor = "multi_thread")]
async fn rename_and_model_lines_in_chat_have_undo() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-f3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let (_ui, client) = open_ui(&home, &home.join("shop"), &[("FASTPILOT_DIR", "")]).await;
    eval(&client, "(async () => { await api('/api/apps', { action: 'add', name: 'Shop', targets: { web: 'https://shop.example' } }); location.hash = '#/agents/default/chat'; return 'ok'; })()").await;
    see(&client, "What should I test?").await;
    let app_name = "(async () => (await (await fetch('/api/apps')).json()).apps[0].name)()";
    say(&client, "rename this app to Shop (test)").await;
    see(&client, "Renamed the app to “Shop (test)”").await;
    assert_eq!(eval(&client, app_name).await.trim_matches('"'), "Shop (test)");
    let undo = eval(&client, "[...document.querySelectorAll('.result-line')].filter((l) => l.querySelector('.rl-undo')).length").await;
    assert_eq!(undo, "1", "the rename line has Undo");
    eval(&client, "document.querySelector('.result-line .rl-undo').click(), 'ok'").await;
    see(&client, "Undone").await;
    assert_eq!(eval(&client, app_name).await.trim_matches('"'), "Shop", "Undo gives the old name back");

    say(&client, "use Haiku for this agent").await;
    eval(&client, "new Promise((ok) => { const t = setInterval(() => { if (document.querySelector('.result-line .rl-undo')) { clearInterval(t); ok('ok'); } }, 100); setTimeout(() => ok('none'), 10000); })").await;
    let model = "(async () => { const j = await (await fetch('/api/agent-model?agent=default')).json(); return JSON.stringify(j.explore).toLowerCase().includes('haiku-5') ? 'haiku' : 'other'; })()";
    let before = eval(&client, model).await;
    let lines = eval(&client, "[...document.querySelectorAll('.result-line')].map((l) => (l.querySelector('.rl-text').textContent) + (l.querySelector('.rl-undo') ? ' [Undo]' : '')).join('|')").await;
    assert!(lines.contains("[Undo]"), "the model line has Undo: {lines}");
    eval(&client, "document.querySelector('.result-line .rl-undo').click(), 'ok'").await;
    eval(&client, "new Promise((ok) => setTimeout(() => ok('ok'), 800))").await;
    let after = eval(&client, model).await;
    assert_eq!((before.trim_matches('"'), after.trim_matches('"')), ("haiku", "other"), "Haiku, then Undo puts the old models back");
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}

/// E7: every agent has a Live and an About tab from the first start. Live lists only this agent's
/// tests ("Nothing is running" and Go to Chat when none run); About shows what the agent is, what it
/// is told and how to remove it; arrow keys move between the tabs.
#[tokio::test(flavor = "multi_thread")]
async fn every_agent_has_live_and_about_tabs() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-e7-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let (_ui, client) = open_ui(&home, &home.join("shop"), &[]).await;
    eval(&client, "location.hash = '#/agents/default/chat', 'ok'").await;
    see(&client, "What should I test?").await;
    let tabs = eval(&client, "[...document.querySelectorAll('.tabs2 a')].map((a) => a.textContent).join('|')").await;
    assert_eq!(tabs.trim_matches('"'), NINE_TABS, "before a saved test: all nine tabs (E10)");
    eval(&client, "[...document.querySelectorAll('.tabs2 a')].find((a) => a.textContent === 'Live').click(), 'ok'").await;
    see(&client, "Nothing is running").await;
    see(&client, "Tests that run for My tester show up here, so you can watch them and take over.").await;
    let go = eval(&client, "[...document.querySelectorAll('a.btn')].find((a) => a.textContent.includes('Go to Chat'))?.getAttribute('href') || 'none'").await;
    assert_eq!(go.trim_matches('"'), "#/agents/default/chat");
    // Arrow keys move between the tabs.
    let moved = eval(&client, "(() => { const links = [...document.querySelectorAll('.tabs2 a')]; links[1].focus(); links[1].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })); const a = document.activeElement.textContent; links[8].focus(); links[8].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })); return a + '|' + document.activeElement.textContent; })()").await;
    assert_eq!(moved.trim_matches('"'), "Skills|Chat", "Live → Skills; About wraps to Chat");
    eval(&client, "location.hash = '#/agents/default/about', 'ok'").await;
    see(&client, "About My tester").await;
    see(&client, "say “rename this agent to …” in Chat").await;
    see(&client, "What it is told").await;
    let on = eval(&client, "document.querySelector('.tabs2 a.on')?.textContent || 'none'").await;
    assert_eq!(on.trim_matches('"'), "About");

    // Live lists only this agent's tests: one of "My tester" (no agent set) and one of another agent.
    let tester = |agent: Option<&'static str>| {
        let (h, live) = (home.join(format!("t-{}", agent.unwrap_or("mine"))), home.join("live"));
        std::fs::create_dir_all(&h).expect("home");
        tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
            c.arg("mcp").current_dir(ROOT).env("HOME", &h).env("FASTPILOT_LIVE_DIR", &live).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_AGENT");
            if let Some(a) = agent {
                c.env("FASTPILOT_AGENT", a).env("FASTPILOT_AGENT_NAME", a);
            }
        })
    };
    let mine: Client = ().serve(TokioChildProcess::new(tester(None)).expect("spawn")).await.expect("mine");
    let theirs: Client = ().serve(TokioChildProcess::new(tester(Some("checkout-qa"))).expect("spawn")).await.expect("theirs");
    let (ok, out) = call(&mine, "session", json!({ "action": "start", "id": "mine1", "platform": "web", "target": "examples/demo/login.html" })).await;
    assert!(ok, "{out}");
    let (ok, out) = call(&theirs, "session", json!({ "action": "start", "id": "theirs1", "platform": "web", "target": "examples/demo/login.html" })).await;
    assert!(ok, "{out}");
    eval(&client, "location.hash = '#/agents/default/live', 'ok'").await;
    see(&client, "Tests running now for My tester. Take over to click for yourself.").await;
    see(&client, "mine1").await;
    let listed = eval(&client, "String(document.body.innerText.includes('theirs1')) + '|' + (document.querySelector('.tabs2 a[href$=\"/live\"]')?.getAttribute('aria-label') || 'none')").await;
    assert_eq!(listed.trim_matches('"'), "false|Live, 1 running", "only this agent's test, and the count on the tab");
    let _ = call(&mine, "session", json!({ "action": "stop", "id": "mine1" })).await;
    let _ = call(&theirs, "session", json!({ "action": "stop", "id": "theirs1" })).await;
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}

async fn open_tab(client: &Client, name: &str) {
    eval(client, &format!("[...document.querySelectorAll('.tabs2 a')].find((a) => a.textContent === '{name}').click(), 'ok'")).await;
}

const NINE_TABS: &str = "Chat|Live|Skills|Schedules|History|Memory|Map|Model|About";

/// E10 (e10a–e10d): a new agent, before any test is saved, shows all nine tabs on its Chat page and
/// on every tab page; its Model tab picks a model and saves it; Schedules, History and Memory say what
/// is missing in one sentence with one button to this agent's Chat; no tab is blank or an error.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_agent_shows_all_nine_tabs_and_sets_its_model_before_any_test() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-e10-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let (_ui, client) = open_ui(&home, &home.join("shop"), &[]).await;
    let made = eval(&client, "(async () => { const r = await api('/api/agents', { action: 'create', agent: { id: '', name: 'Checkout QA', job: 'Tests checkout.', devices: ['web'], app: 'https://shop.example', instructions: [], blocked: [], provider: null, model: null, skills: [] } }); const a = await (await fetch('/api/agents')).json(); return r.agent.id + '|' + a.saved_tests; })()").await;
    let (id, saved) = made.trim_matches('"').split_once('|').expect("agent id");
    assert_eq!(saved, "0", "no saved test");
    eval(&client, &format!("location.hash = '#/agents/{id}/chat', location.reload(), 'ok'")).await;
    see(&client, "What should I test?").await;
    let tabs = || eval(&client, "[...document.querySelectorAll('.tabs2 a')].map((a) => a.textContent).join('|')");
    assert_eq!(tabs().await.trim_matches('"'), NINE_TABS, "a new agent, no saved test: all nine tabs");

    // Model works before any test: pick a Replay model; it is saved for this agent.
    open_tab(&client, "Model").await;
    see(&client, "Which AI runs this agent’s tests. Each agent can use its own.").await;
    let picked = eval(&client, "(() => { document.querySelector('.mpick').click(); const o = [...document.querySelectorAll('#mlist [role=option]')].find((o) => o.dataset.v.startsWith('claude:') && o.getAttribute('aria-selected') !== 'true'); o.click(); return o.dataset.v.slice('claude:'.length); })()").await;
    see(&client, "Changed Replay model").await;
    let saved = eval(&client, &format!("(async () => {{ const d = await (await fetch('/api/agent-model?agent={id}')).json(); return d.replay.choice ? d.replay.choice.model : 'default'; }})()")).await;
    assert_eq!(saved, picked, "the Model tab saved the picked Replay model");
    assert_eq!(tabs().await.trim_matches('"'), NINE_TABS);

    // Schedules, History and Memory: a title, one sentence, one button to this agent's Chat.
    for (tab, title, sentence) in [
        ("Schedules", "No saved test yet", "A schedule runs a saved test by itself. Make your first test in Chat, then come back."),
        ("History", "No runs yet", "Your test runs show up here after the first one."),
        ("Memory", "Nothing remembered yet", "After a test passes once, fastpilot remembers how, and runs get faster."),
    ] {
        open_tab(&client, tab).await;
        see(&client, title).await;
        see(&client, sentence).await;
        assert_eq!(tabs().await.trim_matches('"'), NINE_TABS, "{tab}");
        let page = eval(&client, "[document.querySelectorAll('#body button.p').length, document.querySelector('#body .empty button.p')?.textContent, document.querySelectorAll('#body [role=alert]').length].join('|')").await;
        assert_eq!(page.trim_matches('"'), "1|Make a test in Chat|0", "{tab}: one button, no error");
        eval(&client, "document.querySelector('#body .empty button.p').click(), 'ok'").await;
        see(&client, "What should I test?").await;
        assert_eq!(eval(&client, "location.hash").await.trim_matches('"'), format!("#/agents/{id}/chat"), "{tab}: the button opens this agent's Chat");
    }
    // Skills, Map, Live and About are never blank and never an error.
    for (tab, text) in [("Skills", "General QA"), ("Map", "Add your app first"), ("Live", "Nothing is running"), ("About", "About Checkout QA")] {
        open_tab(&client, tab).await;
        see(&client, text).await;
        assert_eq!(tabs().await.trim_matches('"'), NINE_TABS, "{tab}");
        assert_eq!(eval(&client, "String(document.querySelectorAll('#body [role=alert]').length)").await, "\"0\"", "{tab}: no error");
    }
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}

/// v0.7.1 FC4b: one look for the whole dashboard. On every page: no "More options" (nothing hidden
/// behind one), no hard shadow on cards or buttons, control borders in the --ctl grey; Settings has
/// one tab bar with Integrations in it; Integrations shows the webhook as it is and no "Coming
/// soon"; History says Runs · Chats · Recently removed.
#[tokio::test(flavor = "multi_thread")]
async fn every_page_has_one_look_and_no_more_options() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-one-look-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let (_ui, client) = open_ui(&home, &home.join("shop"), &[]).await;
    let txt = |s: String| s.trim_matches('"').to_owned();
    let check = "(() => { const shadow = [...document.querySelectorAll('.grp, .btn')].filter((e) => { const s = getComputedStyle(e).boxShadow; return s && s !== 'none' && /\\d+px \\d+px 0px/.test(s) && !/^rgba\\(0, 0, 0, 0\\)/.test(s); }).length; const more = document.querySelectorAll('.more-btn').length + (/More options|Slack|send results to/.test(document.body.innerText) ? 1 : 0); const ctl = [...document.querySelectorAll('#body .btn')].every((b) => b.disabled || b.classList.contains('p') || b.classList.contains('d') || b.classList.contains('danger') || getComputedStyle(b).borderTopColor === 'rgb(115, 124, 138)'); return [more, shadow, ctl].join('|'); })()";
    for (hash, wait) in [
        ("#/agents/default/chat", "What should I test?"),
        ("#/agents/default/skills", "Specialist skills"),
        ("#/schedules", "Schedules"),
        ("#/history", "History"),
        ("#/memory", "Memory"),
        ("#/map", "Map"),
        ("#/live", "Live"),
        ("#/device", "Device"),
        ("#/settings/model", "AI model"),
        ("#/settings/limits", "Every limit"),
        ("#/settings/secrets", "Secrets"),
        ("#/settings/security", "Security"),
        ("#/integrations", "Integrations"),
    ] {
        eval(&client, &format!("location.hash = '{hash}', 'ok'")).await;
        see(&client, wait).await;
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert_eq!(txt(eval(&client, check).await), "0|0|true", "{hash}: More options, Slack or \"send results to\" | hard shadows | control borders");
    }
    // Settings: one tab bar, Integrations in it (and marked there).
    let tabs = txt(eval(&client, "[...document.querySelectorAll('.snav.tabs2 a')].map((a) => a.textContent + (a.getAttribute('aria-current') ? '*' : '')).join('|')").await);
    assert_eq!(tabs, "AI model|Limits & permissions|Secrets|Security|Integrations*");
    let integ = txt(eval(&client, "[document.body.innerText.includes('Coming soon'), !!document.querySelector('[data-app=slack]'), document.querySelector('[data-app=webhook]')?.textContent.includes('Not set. Add a webhook URL to a schedule on the Schedules page.')].join('|')").await);
    assert_eq!(integ, "false|false|true");
    eval(&client, "location.hash = '#/history', 'ok'").await;
    see(&client, "Recently removed").await;
    assert_eq!(txt(eval(&client, "[...document.querySelectorAll('.hist-tab')].map((t) => t.firstChild.textContent).join(' · ')").await), "Runs · Chats · Recently removed");
    let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
    let _ = std::fs::remove_dir_all(&home);
}
