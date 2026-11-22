//! UXA4 (v0.7): the dashboard's shared building blocks, in the running dashboard, driven by
//! fastpilot itself in Chrome. PageHeader never draws a null child (the Map header said "null",
//! L062-9); MoreOptions opens and closes with aria-expanded; ConfirmDialog is an in-page
//! alertdialog whose safe button has the focus, Esc keeps, and focus goes back; Empty and Notice say
//! what they mean in words. Map, Schedules and Settings use them. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists())
}

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

/// `evalScript` in the viewer session; the value as returned (JSON text).
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

async fn key(client: &Client, k: &str) {
    let (ok, out) = call(client, "run_flow", json!({ "flow": format!("- pressKey: {k}\n"), "session": "viewer" })).await;
    assert!(ok, "{out}");
}

struct Dash {
    ui: std::process::Child,
    home: PathBuf,
    client: Client,
}

impl Drop for Dash {
    fn drop(&mut self) {
        let _ = self.ui.kill();
        let _ = self.ui.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// `fastpilot ui` in a temp HOME, opened in a Chrome session called "viewer".
async fn dashboard(name: &str) -> Dash {
    let home = std::env::temp_dir().join(format!("fp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("viewer")).expect("home");
    let mut ui = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(&home)
        .env("HOME", &home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut link = String::new();
    BufReader::new(ui.stdout.take().expect("stdout")).read_line(&mut link).expect("link");
    let viewer = home.join("viewer");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").current_dir(ROOT).env("HOME", &viewer).env("FASTPILOT_LIVE_DIR", viewer.join("live")).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let flow = format!("- session: {{ start: viewer, platform: web, target: \"{}\" }}\n", link.trim());
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    see(&client, "Live").await;
    Dash { ui, home, client }
}

async fn open(c: &Client, hash: &str, text: &str) {
    eval(c, &format!("location.hash = '{hash}', 'ok'")).await;
    see(c, text).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_components_draw_no_null_and_behave_the_same_everywhere() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let d = dashboard("ui-components").await;
    let c = &d.client;

    // L062-9: the Map header with no map yet said "null".
    open(c, "#/map", "Which parts of your app have a test").await;
    let top = eval(c, "document.getElementById('top').textContent").await;
    assert!(!top.contains("null") && top.contains("Map"), "Map header: {top}");

    // The components exist once, for every page.
    assert_eq!(eval(c, "typeof window.UI").await, "\"object\"", "components.js is loaded");
    assert_eq!(eval(c, "UI.kids(null, undefined, false, '', 'a', [null, ['b']]).join('|')").await, "\"a|b\"");

    // PageHeader: null subtitle, crumb and actions draw nothing; actions sit on the right.
    let top = eval(c, "(() => { UI.pageHeader({ icon: 'map', title: 'T', sub: null, crumb: null, actions: [null, h('button', {}, 'Go')] }); const t = document.getElementById('top'); return [t.textContent, t.querySelectorAll('.right button').length, t.querySelector('h1').textContent].join('|'); })()").await;
    assert_eq!(top, "\"TGo|1|T\"", "no 'null' text, one action");

    // MoreOptions: closed by default, aria-expanded follows, the changed count is said in words.
    let more = eval(c, "(() => { const m = UI.moreOptions({ changed: 2 }, h('p', {}, 'inside')); document.getElementById('body').replaceChildren(m); const b = m.querySelector('button'); const p = m.querySelector('.more-body'); const a = [b.getAttribute('aria-expanded'), p.hidden, b.textContent, b.getAttribute('aria-controls') === p.id]; b.click(); a.push(b.getAttribute('aria-expanded'), p.hidden); b.click(); a.push(b.getAttribute('aria-expanded'), p.hidden); return a.join('|'); })()").await;
    assert_eq!(more, "\"false|true|More options · 2 changed|true|true|false|false|true\"");

    // ConfirmDialog: an alertdialog in the page; the safe button first and focused; Esc keeps and
    // gives focus back to the button that opened it.
    let opened = eval(c, "(() => { const o = h('button', { id: 'opener' }, 'Open'); document.getElementById('body').replaceChildren(o); o.focus(); window.__r = 'pending'; UI.confirmDialog({ title: 'Delete the schedule \"Night\"?', text: 'Its runs stay in History.', verb: 'Delete schedule' }).then((v) => { window.__r = String(v); }); const d = document.querySelector('dialog.cdlg'); const bs = [...d.querySelectorAll('button')].map((b) => b.textContent); return [d.getAttribute('role'), d.open, bs.join('/'), document.activeElement.textContent].join('|'); })()").await;
    assert_eq!(opened, "\"alertdialog|true|Keep it/Delete schedule|Keep it\"");
    key(c, "Escape").await;
    let after = eval(c, "new Promise((r) => setTimeout(() => r([window.__r, !!document.querySelector('dialog.cdlg'), document.activeElement.id].join('|')), 100))").await;
    assert_eq!(after, "\"false|false|opener\"", "Esc keeps; focus goes back");
    let yes = eval(c, "(async () => { const p = UI.confirmDialog({ title: 't', text: 'x', verb: 'Delete' }); [...document.querySelectorAll('dialog.cdlg button')].find((b) => b.textContent === 'Delete').click(); return String(await p); })()").await;
    assert_eq!(yes, "\"true\"", "the verb does it");
    // A typed word: the action waits for it.
    let typed = eval(c, "(async () => { const p = UI.confirmDialog({ title: 't', text: 'x', verb: 'Allow', typed: 'PAY' }); const d = document.querySelector('dialog.cdlg'); const go = [...d.querySelectorAll('button')].find((b) => b.textContent === 'Allow'); const i = d.querySelector('input'); const a = [go.disabled]; i.value = 'pay'; i.dispatchEvent(new Event('input')); a.push(go.disabled); i.value = 'PAY'; i.dispatchEvent(new Event('input')); a.push(go.disabled); go.click(); a.push(await p); return a.join('|'); })()").await;
    assert_eq!(typed, "\"true|true|false|true\"");

    // Empty and Notice: words, not colour alone; errors are alerts.
    let empty = eval(c, "(() => { const e = UI.empty({ icon: 'clock', title: 'Nothing yet', text: null, action: { label: 'Create schedule', onclick: () => {} } }); return [e.textContent, e.querySelectorAll('button.p').length].join('|'); })()").await;
    assert_eq!(empty, "\"Nothing yetCreate schedule|1\"");
    let notes = eval(c, "['ok', 'info', 'warn', 'error'].map((k) => { const n = UI.notice(k, 'it', null); return n.getAttribute('role') + ':' + n.textContent; }).join('|')").await;
    assert_eq!(notes, "\"status:Done: it|status:Note: it|alert:Warning: it|alert:Error: it\"");

    // Schedules and Settings draw their headers with PageHeader too.
    open(c, "#/schedules", "Nothing is scheduled").await;
    let top = eval(c, "[document.querySelector('#top .ph-title h1').textContent, document.querySelector('#top .right button').textContent, /null/.test(document.getElementById('top').textContent)].join('|')").await;
    assert_eq!(top, "\"Schedules|New schedule|false\"");
    assert_eq!(eval(c, "document.querySelectorAll('#body .empty button.p').length").await, "1", "one main button in the empty state");
    open(c, "#/settings/limits", "Limits & permissions").await;
    assert_eq!(eval(c, "document.querySelector('#top .ph-title h1').textContent").await, "\"Settings\"");
    open(c, "#/map", "Which parts of your app have a test").await;
    assert_eq!(eval(c, "document.querySelector('#top .ph-title h1').textContent").await, "\"Map\"");
}
