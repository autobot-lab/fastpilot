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

/// `fastpilot ui` in a temp HOME, opened in a Chrome session called "viewer". `seed` writes the
/// project's files first (`<home>/tests`, and fastpilot's folder `<home>/project-fastpilot`). Keys go
/// to a file in the temp HOME, schedules to a stand-in LaunchAgents folder: nothing reaches this Mac.
async fn dashboard(name: &str, seed: fn(&Path)) -> Dash {
    let home = std::env::temp_dir().join(format!("fp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("viewer")).expect("home");
    std::fs::create_dir_all(home.join("project-fastpilot/memory")).expect("fastpilot dir");
    std::fs::create_dir_all(home.join("tests")).expect("tests");
    seed(&home);
    let mut ui = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(&home)
        .env("HOME", &home)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_DIR", home.join("project-fastpilot"))
        .env_remove("FASTPILOT_MEMORY")
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_SCHEDULER", "launchd")
        .env("FASTPILOT_SCHEDULER_DIR", home.join("LaunchAgents"))
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
    let d = dashboard("ui-components", |_| {}).await;
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
    // L07-1: no saved test yet: one empty state with one way to Chat, no form with an empty list.
    open(c, "#/schedules", "No saved test yet").await;
    let first = eval(c, "[document.querySelectorAll('#body .empty button.p').length, document.querySelector('#body .empty button.p').textContent, !!document.querySelector('.sched-form, .more, .presets'), [...document.querySelectorAll('#body button')].some((b) => b.textContent.includes('New schedule'))].join('|')").await;
    assert_eq!(first, "\"1|Make a test first in Chat|false|false\"");
    // S5/A1 (seamless-v13 s4a): title and subtitle; "New schedule" lives under More options.
    let top = eval(c, "[document.querySelector('#top .ph-title h1').textContent, document.querySelector('#top .ph-title .s').textContent, !!document.querySelector('#top .right'), /null/.test(document.getElementById('top').textContent)].join('|')").await;
    assert_eq!(top, "\"Schedules|Run your tests by themselves|false|false\"");
    assert_eq!(eval(c, "document.querySelectorAll('#body .empty button.p').length").await, "1", "one main button in the empty state");
    eval(c, "document.querySelector('#body .empty button.p').click(), 'ok'").await;
    assert!(eval(c, "location.hash").await.starts_with("\"#/agents"), "it opens Chat");
    open(c, "#/settings/limits", "Limits & permissions").await;
    assert_eq!(eval(c, "document.querySelector('#top .ph-title h1').textContent").await, "\"Settings\"");
    open(c, "#/map", "Which parts of your app have a test").await;
    assert_eq!(eval(c, "document.querySelector('#top .ph-title h1').textContent").await, "\"Map\"");
}

/// UXA1 (v0.7): the four browser confirm() boxes (delete an agent, delete the API key, delete a
/// schedule, set a test aside) are the in-page ConfirmDialog: no browser box, the safe button first
/// and focused, Esc changes nothing, the named verb does it.
#[tokio::test(flavor = "multi_thread")]
async fn every_confirmation_is_in_the_page_and_esc_changes_nothing() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let d = dashboard("ui-confirm", |home| {
        std::fs::write(home.join("tests/export.md"), "# Admin export\nid: export\nplatform: web\ntarget: https://shop.example\n- Open the shop\n").expect("test");
        // A test that failed its last 4 runs at the same step: Memory offers "Set aside".
        let fp = home.join("project-fastpilot");
        let history = [true, true, false, false, false, false];
        let m = json!({ "test_id": "export", "flow": "- tapOn: Download\n", "start": "https://shop.example", "screens": [], "duration_ms": 1200, "pass_count": 2, "fail_count": 4,
            "last_pass": "2026-11-20 07:02:00", "failed_at": [], "history": history, "consecutive_failures": 4, "stale": true, "classes": [] });
        std::fs::write(fp.join("memory/export.json"), m.to_string()).expect("memory");
        for (i, passed) in history.iter().enumerate() {
            let dir = fp.join(format!("runs/20261120-{:06}-export", 100 + i));
            std::fs::create_dir_all(&dir).expect("run");
            let step = (!passed).then_some("tapOn \"Download\"");
            let o = json!({ "test_id": "export", "passed": passed, "passed_on_retry": false, "could_not_run": false, "started_ms": 1_795_000_000_000u64 + i as u64 * 3_600_000, "finished_ms": 1_795_000_060_000u64 + i as u64 * 3_600_000,
                "platforms": ["web"], "failed_step": step, "reason": step.map(|_| "not_found"), "category": step.map(|_| "product"), "error": step.map(|_| "not visible yet") });
            std::fs::write(dir.join("outcome.json"), o.to_string()).expect("outcome");
        }
    })
    .await;
    let c = &d.client;
    let tests = d.home.join("tests").display().to_string();

    // What there is to delete, made through the page's own API calls.
    let made = eval(c, &format!(r#"(async () => {{
        const a = await api('/api/agents', {{ action: 'create', agent: {{ id: '', name: 'QA Checkout', job: 'Keep the checkout working', devices: ['web'], app: 'https://shop.example', instructions: [], blocked: [] }} }});
        const s = await api('/api/schedules', {{ action: 'save', schedule: {{ id: '', name: 'Night run', tests: {tests:?}, when: {{ every: 'day', time: '02:00' }}, notify: {{ os: true }}, enabled: true }} }});
        await api('/api/provider', {{ action: 'save', provider: 'openai', model: 'gpt-4.1-mini', key: 'sk-test-not-real' }});
        await loadSettings();
        window.confirm = () => {{ window.__native = (window.__native || 0) + 1; return false; }};
        return a.agent.id + '|' + s.id;
    }})()"#)).await;
    let made = made.trim_matches('"').to_owned();
    let (agent, schedule) = made.split_once('|').expect("ids");
    let state = |what: &str| -> String {
        match what {
            "agent" => format!("(async () => String((await api('/api/agents')).agents.some((x) => (x.agent || x).id === '{agent}')))()"),
            "key" => "(async () => String(!!(await api('/api/provider')).key.has))()".to_owned(),
            "schedule" => format!("(async () => String((await api('/api/schedules')).schedules.some((x) => x.id === '{schedule}')))()"),
            _ => "(async () => (await api('/api/memory')).tests.find((t) => t.id === 'export').status)()".to_owned(),
        }
    };
    for (what, hash, sees, opener, verb, after) in [
        ("agent", format!("#/agents/{agent}/settings"), "Delete agent", "Delete agent", "Delete agent", "\"false\""),
        ("key", "#/settings/model".to_owned(), "Delete key", "Delete key", "Delete key", "\"false\""),
        ("schedule", "#/schedules".to_owned(), "Night run", "Delete", "Delete schedule", "\"false\""),
        ("aside", "#/memory".to_owned(), "Why failing?", "Set aside", "Set aside", "\"set_aside\""),
    ] {
        open(c, &hash, sees).await;
        let before = eval(c, &state(what)).await;
        // Memory: the "Why failing?" panel's Set aside is the one that asks (the row's acts at once
        // and is undone with Bring back).
        let scope = if what == "aside" { "#body .mem-doctor button" } else { "#body button" };
        let click = format!("[...document.querySelectorAll('{scope}')].find((b) => b.textContent.trim() === {opener:?}).click()");
        let shown = eval(c, &format!("(() => {{ window.__native = 0; {click}; const d = document.querySelector('dialog.cdlg'); return [window.__native, d ? d.getAttribute('role') : 'none', d ? [...d.querySelectorAll('button')].map((b) => b.textContent).join('/') : '', document.activeElement.textContent].join('|'); }})()")).await;
        assert_eq!(shown, format!("\"0|alertdialog|Keep it/{verb}|Keep it\""), "{what}: an in-page dialog, not the browser's");
        key(c, "Escape").await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(eval(c, "String(!!document.querySelector('dialog.cdlg'))").await, "\"false\"", "{what}: Esc closes it");
        assert_eq!(eval(c, &state(what)).await, before, "{what}: Esc changes nothing");
        eval(c, &format!("(() => {{ {click}; [...document.querySelectorAll('dialog.cdlg button')].find((b) => b.textContent === {verb:?}).click(); return 'ok'; }})()")).await;
        let mut now = String::new();
        for _ in 0..40 {
            now = eval(c, &state(what)).await;
            if now == after {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert_eq!(now, after, "{what}: the verb does it");
    }
    // No browser confirm() is left in the dashboard's code.
    for f in ["app.js", "agents.js", "memory.js", "history.js", "apps.js", "map.js", "fix.js"] {
        let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("ui").join(f)).expect("js");
        assert!(!js.contains("confirm("), "{f} still calls confirm()");
    }
}

/// UXA2 (v0.7): the Settings menu and page say "Secrets" (plural), like everywhere else; a link
/// made before v0.7 (#/settings/secret) still opens it; no "Secret" label is left in the dashboard.
#[tokio::test(flavor = "multi_thread")]
async fn settings_menu_says_secrets() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let d = dashboard("ui-secrets", |_| {}).await;
    let c = &d.client;
    open(c, "#/settings/limits", "Limits & permissions").await;
    assert_eq!(eval(c, "[...document.querySelectorAll('nav.snav a')].map((a) => a.textContent + '>' + a.getAttribute('href')).join('|')").await,
        "\"AI model>#/settings/model|Limits & permissions>#/settings/limits|Secrets>#/settings/secrets|Security>#/settings/security|Your apps>#/settings/apps\"");
    for hash in ["#/settings/secrets", "#/settings/secret"] {
        open(c, hash, "Nothing saved yet").await;
        let page = eval(c, "[document.querySelector('#body h2').firstChild.textContent, document.querySelector('nav.snav a[aria-current=page]').textContent].join('|')").await;
        assert_eq!(page, "\"Secrets|Secrets\"", "{hash}");
    }
    for f in ["app.js", "memory.js", "agents.js", "history.js", "apps.js", "map.js", "fix.js"] {
        let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("ui").join(f)).expect("js");
        for old in ["\"Secret\"", "→ Secret\"", "settings/secret\""] {
            assert!(!js.contains(old), "{f} still has {old}");
        }
    }
}

/// E2 (v0.7): a password is asked in the page, in a dialog, kept for the current app (here the
/// owner-only key file of `FASTPILOT_KEYSTORE=file`), and never shown again: the page says "Set",
/// and the value is in neither the page nor its list.
#[tokio::test(flavor = "multi_thread")]
async fn a_password_is_asked_in_the_page_and_kept_for_the_app() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let d = dashboard("ui-pass", |_| {}).await;
    let c = &d.client;
    let seeded = eval(c, "(async () => { await api('/api/apps', { action: 'add', name: 'Shop', targets: { web: 'https://shop.example/' } }); await api('/api/settings', { key: 'vars', value: ['DEMO_PASS'] }); return 'ok'; })()").await;
    assert_eq!(seeded, "\"ok\"");
    open(c, "#/settings/secrets", "a test of Shop needs it").await;
    see(c, "Not set yet").await;
    let flow = "- tapOn: Set value\n- assertVisible: Set DEMO_PASS\n- inputText: { into: Set DEMO_PASS, text: kunci-rahasia-9 }\n- tapOn: Save password\n- assertVisible: Saved on this\n";
    let (ok, out) = call(c, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    let kept = std::fs::read_to_string(d.home.join(".fastpilot/keys/secrets/shop/DEMO_PASS")).expect("kept for the app");
    assert!(!kept.contains("kunci-rahasia-9"), "kept encoded, not as typed");
    let page = eval(c, "(async () => { const t = document.body.innerHTML + JSON.stringify(await api('/api/secrets')); const i = t.indexOf('kunci-rahasia-9'); return i < 0 ? 'clean DEMO_PASS' : t.slice(Math.max(0, i - 300), i + 50); })()").await;
    assert!(page.contains("DEMO_PASS") && !page.contains("kunci-rahasia-9"), "never shown again: {page}");
}

/// U1 (v0.7): one look everywhere (Design Critic's Lows, FastUI/UX's HANDOFF-lows-v07). No terminal
/// command on the main view of Live, Device or Secrets; one Chat page for every way in; with one
/// agent no agent list or tabs; a local website is called "Your website", its address under it.
#[tokio::test(flavor = "multi_thread")]
async fn one_chat_no_commands_and_no_tabs_for_one_agent() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let d = dashboard("ui-u1", |_| {}).await;
    let c = &d.client;
    // Visible text of the main view: hidden "More options" panels do not count.
    let shown = "(() => { const out = []; const w = document.createTreeWalker(document.getElementById('body'), NodeFilter.SHOW_TEXT); while (w.nextNode()) { const n = w.currentNode; if (n.parentElement && !n.parentElement.closest('[hidden]')) out.push(n.textContent); } return out.join(' '); })()";
    let command = |t: &str| ["fastpilot open", "fastpilot run", "fastpilot doctor", "claude mcp add", "brew install", "sudo ", "export FASTPILOT"].iter().find(|w| t.contains(*w)).map(|w| (*w).to_owned());

    // Live, nothing running: one button to Chat, no command.
    open(c, "#/live", "No test is running yet").await;
    let live = eval(c, shown).await;
    assert_eq!(command(&live), None, "{live}");
    see(c, "Make a test in Chat").await;
    eval(c, "([...document.querySelectorAll('#body button')].find((b) => b.textContent.includes('Make a test in Chat')).click(), 'ok')").await;
    see(c, "Send").await;
    assert_eq!(eval(c, "location.hash + '|' + (document.querySelector('.rail a[aria-current=page]')?.textContent || '')").await, "\"#/agents/default/chat|Chat\"");

    // One agent: Chat is just Chat, no agent list, no tabs; the rest under More.
    let solo = eval(c, "[!!document.querySelector('.ag-tabs'), !!document.querySelector('.ag-col'), document.querySelector('.top h1').textContent, !!document.querySelector('#body .more-btn')].join('|')").await;
    assert_eq!(solo, "\"false|false|Chat|true\"");

    // An old #/chat link lands on the same page.
    eval(c, "(location.hash = '#/chat', 'ok')").await;
    let mut at = String::new();
    for _ in 0..40 {
        at = eval(c, "location.hash").await;
        if at.contains("agents") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(at, "\"#/agents/default/chat\"");

    // Device and Secrets: no command on the main view.
    open(c, "#/device", "Device").await;
    for _ in 0..100 {
        if eval(c, "String(!document.querySelector('#body .spin'))").await == "\"true\"" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    let device = eval(c, shown).await;
    assert_eq!(command(&device), None, "{device}");
    open(c, "#/settings/secrets", "Secrets").await;
    let secrets = eval(c, shown).await;
    assert_eq!(command(&secrets), None, "{secrets}");
    assert_eq!(eval(c, "String([...document.querySelectorAll('#body .panel input')].filter((i) => !i.closest('[hidden]')).length)").await, "\"0\"", "no text box on the main view");

    // A local website gets a readable name, its address under it.
    let added = eval(c, "(async () => { const r = await api('/api/apps', { action: 'add', targets: { web: 'http://127.0.0.1:3000/' } }); return r.app.name; })()").await;
    assert_eq!(added, "\"Your website\"");
    // The app bar reads the list when the page loads.
    eval(c, "(location.hash = '#/live', location.reload(), 'ok')").await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let mut bar = String::new();
    for _ in 0..40 {
        bar = eval(c, "document.querySelector('#appbar .appbtn')?.textContent || ''").await;
        if bar.contains("Your website") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    assert!(bar.contains("Your website") && bar.contains("127.0.0.1:3000"), "{bar}");
}

/// UXA3 (v0.7): one set of button words. New thing: Create + the thing; an edit: Save changes; into a
/// list: Add; out of a list: Remove; data gone: Delete; the safe choice: Keep it. Never "Apply",
/// "Save the …", "Keep as it is", or a bare "Save".
#[tokio::test(flavor = "multi_thread")]
async fn buttons_use_one_set_of_words() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    // In the code: no label outside the list is left in any page.
    for f in ["app.js", "agents.js", "memory.js", "history.js", "apps.js", "map.js", "fix.js", "components.js"] {
        let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("ui").join(f)).expect("js");
        for old in ["\"Apply\"", "\"Apply the limits\"", "\"Save\"", "\"Save the schedule\"", "\"Save schedule\"", "\"Keep as it is\"", "\"Keep them\"", "\"Delete the key\""] {
            assert!(!js.contains(old), "{f} still has the button {old}");
        }
    }
    let d = dashboard("ui-verbs", |home| std::fs::write(home.join("tests/login.md"), "# Login\n- Open the shop\n").expect("test")).await;
    let c = &d.client;
    let tests = d.home.join("tests").display().to_string();
    let main_button = "[...document.querySelectorAll('#body button.p, #top button.p')].map((b) => b.textContent.trim()).join('|')";

    // Schedules: a new one is "Create schedule"; editing it is "Save changes".
    eval(c, &format!("(async () => {{ await api('/api/schedules', {{ action: 'save', schedule: {{ id: '', name: 'Night run', tests: {tests:?}, when: {{ every: 'day', time: '02:00' }}, notify: {{ os: true }}, enabled: true }} }}); return 'ok'; }})()")).await;
    open(c, "#/schedules", "Night run").await;
    eval(c, "([...document.querySelectorAll('#body .more button')].find((b) => b.textContent.includes('New schedule')).click(), 'ok')").await;
    assert!(eval(c, main_button).await.contains("Create schedule"), "a new schedule");
    eval(c, "(() => { schedForm = null; pageSchedules(); [...document.querySelectorAll('#body button')].find((b) => b.textContent.trim() === 'Edit').click(); return 'ok'; })()").await;
    let edit = eval(c, main_button).await;
    assert!(edit.contains("Save changes") && !edit.contains("Create schedule"), "editing a schedule: {edit}");

    // Agent settings and the AI model: edits say "Save changes"; the key is "Delete key".
    open(c, "#/agents/default/settings", "AI model").await;
    assert!(eval(c, main_button).await.contains("Save changes"), "agent settings");
    eval(c, "(async () => { await api('/api/provider', { action: 'save', provider: 'openai', model: 'gpt-4.1-mini', key: 'sk-test-not-real' }); await loadSettings(); return 'ok'; })()").await;
    open(c, "#/settings/model", "Delete key").await;
    assert!(eval(c, main_button).await.contains("Save changes"), "AI model");

    // A confirmation's safe button is "Keep it" (History's "Keep them" is gone: see the scan above).
    let keep = eval(c, "(async () => { const p = UI.confirmDialog({ title: 't', text: 'x', verb: 'Delete' }); const k = document.querySelector('dialog.cdlg button').textContent; document.querySelector('dialog.cdlg button').click(); await p; return k; })()").await;
    assert_eq!(keep, "\"Keep it\"");
}

/// E6b (v0.7): the menu in its locked order (Chat first, Integrations for Jira and the rest), the App
/// switcher above every page, the Add app dialog (name taken from the address), adding a platform to
/// an app from #/apps/add (S1 and S8 link there), switching, and Settings › Your apps, where
/// removing an app keeps its folder. Old #/apps links land on Integrations.
#[tokio::test(flavor = "multi_thread")]
async fn the_app_switcher_adds_switches_and_removes_apps() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let d = dashboard("ui-apps", |_| {}).await;
    let c = &d.client;
    let menu = eval(c, "[...document.querySelectorAll('.rail a small')].map((t) => t.textContent).join('|')").await;
    assert_eq!(menu, "\"Chat|Live|Schedules|Map|Memory|History|Device|Integrations|Settings\"", "the locked menu (s6a)");

    // The first screen is Chat, the first item of the menu (s6a; design M2).
    assert_eq!(eval(c, "document.querySelector('.rail a[aria-current=page]')?.textContent || 'none'").await, "\"Chat\"", "opens on Chat");
    // Typed right away, the message survives the page being drawn again (apps and settings load).
    see(c, "Send").await;
    let kept = eval(c, "(async () => { const t = document.getElementById('chat-input'); t.focus(); t.value = 'check the login'; t.dispatchEvent(new Event('input')); render(); for (let i = 0; i < 100; i++) { const c = document.getElementById('chat-input'); if (c && c !== t) break; await new Promise((r) => setTimeout(r, 100)); } await new Promise((r) => setTimeout(r, 50)); const n = document.getElementById('chat-input'); return [n !== t, n.value, document.activeElement === n].join('|'); })()").await;
    assert_eq!(kept, "\"true|check the login|true\"", "a redraw keeps the draft and the cursor");

    // No app yet: one button. The dialog asks for the address only (design M1). Opened from Live,
    // a new app still goes to Chat.
    eval(c, "(location.hash = '#/live', 'ok')").await;
    see(c, "Add your app").await;
    eval(c, "(document.querySelector('#appbar button').click(), 'ok')").await;
    see(c, "What do you want to test?").await;
    let kinds = eval(c, "[...document.querySelectorAll('dialog.appdlg [role=radio]')].map((b) => b.dataset.v + ':' + b.getAttribute('aria-checked')).join('|')").await;
    assert_eq!(kinds, "\"web:true|android:false|ios:false\"");
    let typed = "(v) => { const w = document.getElementById('app-where'); w.value = v; w.dispatchEvent(new Event('input')); const n = document.getElementById('app-name'); return n ? n.value : ''; }";
    assert_eq!(eval(c, "String(document.querySelectorAll('dialog.appdlg input').length)").await, "\"1\"", "one field: the address, no Name");
    // A wrong address is said under the field, and the dialog stays.
    eval(c, &format!("(({typed})('staging shop'), document.querySelector('dialog.appdlg button[type=submit]').click(), 'ok')")).await;
    see(c, "is not a website address").await;
    assert_eq!(eval(c, "String(!!document.querySelector('dialog.appdlg'))").await, "\"true\"");
    eval(c, &format!("(({typed})('https://staging.shop.example'), document.querySelector('dialog.appdlg button[type=submit]').click(), 'ok')")).await;
    see(c, "Shop staging was added").await;
    assert_eq!(eval(c, "document.querySelector('.rail a[aria-current=page]')?.textContent || 'none'").await, "\"Chat\"", "a new app goes to Chat");
    let bar = eval(c, "document.querySelector('#appbar .appbtn').textContent").await;
    assert!(bar.contains("Shop staging") && bar.contains("Website"), "{bar}");

    // Add an Android app from the switcher; the first app stays current.
    eval(c, "(document.querySelector('#appbar .appbtn').click(), [...document.querySelectorAll('.appmenu button')].find((b) => b.textContent.includes('Add app')).click(), 'ok')").await;
    see(c, "What do you want to test?").await;
    eval(c, "(document.querySelector('dialog.appdlg [data-v=android]').click(), 'ok')").await;
    eval(c, &format!("({typed})('com.fastpilot.demo')")).await;
    eval(c, "(document.querySelector('dialog.appdlg button[type=submit]').click(), 'ok')").await;
    see(c, "Demo was added").await;
    let cur = eval(c, "(async () => (await api('/api/apps')).current)()").await;
    assert_eq!(cur, "\"shop-staging\"");

    // Switch.
    eval(c, "(document.querySelector('#appbar .appbtn').click(), [...document.querySelectorAll('.appmenu [role=menuitemradio]')].find((b) => b.textContent.includes('Demo')).click(), 'ok')").await;
    let mut now = String::new();
    for _ in 0..40 {
        now = eval(c, "document.querySelector('#appbar .appbtn').textContent").await;
        if now.contains("Demo") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    assert!(now.contains("Demo") && now.contains("Android app"), "{now}");

    // From a "Add iPhone app" button (S1, S8): a platform for an existing app.
    open(c, "#/apps/add?app=shop-staging&kind=ios", "Add to Shop staging").await;
    let offered = eval(c, "[...document.querySelectorAll('dialog.appdlg [role=radio]')].map((b) => b.dataset.v).join('|')").await;
    assert_eq!(offered, "\"android|ios\"", "the website it has is not offered again");
    eval(c, &format!("(({typed})('com.shop.app'), document.querySelector('dialog.appdlg button[type=submit]').click(), 'ok')")).await;
    see(c, "iPhone app added to Shop staging").await;
    let shop = eval(c, "(async () => JSON.stringify((await api('/api/apps')).apps.find((a) => a.id === 'shop-staging').kinds))()").await;
    assert_eq!(shop, r#""[\"web\",\"ios\"]""#);

    // Settings › Your apps: Remove keeps the folder.
    open(c, "#/settings/apps", "Where its tests are saved").await;
    assert_eq!(eval(c, "document.querySelectorAll('tbody tr').length").await, "2");
    eval(c, "([...document.querySelectorAll('tr[data-id=demo] button')].find((b) => b.textContent.startsWith('Remove')).click(), 'ok')").await;
    see(c, "Remove Demo?").await;
    eval(c, "([...document.querySelectorAll('dialog.cdlg button')].find((b) => b.textContent === 'Remove from list').click(), 'ok')").await;
    see(c, "Demo was removed from the list").await;
    assert_eq!(eval(c, "(async () => String((await api('/api/apps')).apps.length))()").await, "\"1\"");
    assert!(d.home.join("fastpilot/apps/demo").is_dir(), "its folder stays");

    // Old links to the integrations page.
    for (old, new) in [("#/apps", "#/integrations"), ("#/apps/jira", "#/integrations/jira")] {
        eval(c, &format!("(location.hash = '{old}', 'ok')")).await;
        let mut at = String::new();
        for _ in 0..40 {
            at = eval(c, "location.hash").await;
            if at.trim_matches('"') == new {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(at.trim_matches('"'), new, "{old}");
    }
    see(c, "Connect Jira").await;
}

/// S6 (v0.7): Settings › AI model says in one line what runs the tests (the signed-in Claude Code,
/// found by itself); the models and other providers are under More options, closed. A new install
/// explores with Sonnet, not Opus (DC7). Another provider shows on that line, and More options
/// counts it as changed.
#[tokio::test(flavor = "multi_thread")]
async fn the_ai_model_page_says_what_runs_the_tests_in_one_line() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let d = dashboard("ui-model", |_| {}).await;
    let c = &d.client;
    open(c, "#/settings/model", "AI model").await;
    // The line: Claude Code found (this Mac) or not (a CI runner without it); never a form first.
    // The machine check (doctor) can take a while; the line waits for it, never guessing.
    let mut line = String::new();
    for _ in 0..120 {
        line = eval(c, "(() => { const u = document.querySelector('.using.ok') || document.querySelector('#body .notice'); return u ? u.textContent : document.querySelector('.using') ? document.querySelector('.using').textContent : 'none'; })()").await;
        if !line.contains("Looking for Claude Code") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(line.starts_with("\"Using Claude Code ✓") || line.contains("Claude Code is not on this computer yet"), "{line}");
    let shut = eval(c, "(() => { const b = document.querySelector('#body .more-btn'); const sel = document.querySelector('select[aria-label=\"Exploring model\"]'); return [b.textContent, b.getAttribute('aria-expanded'), !!sel.closest('[hidden]')].join('|'); })()").await;
    assert_eq!(shut, "\"More options|false|true\"", "the models are under More options, closed");
    // DC7: the built-in exploring model is Sonnet.
    let built_in = eval(c, "document.querySelector('select[aria-label=\"Exploring model\"]').options[0].textContent").await;
    assert_eq!(built_in, "\"Built in · Claude Code · Sonnet 5.5\"");
    // Another provider: said on the line, and counted under More options.
    eval(c, "(async () => { await api('/api/provider', { action: 'save', provider: 'openai', model: 'gpt-4.1-mini', key: 'sk-test-not-real' }); provData = null; await loadProvider(); return 'ok'; })()").await;
    open(c, "#/settings/model", "Using OpenAI").await;
    let other = eval(c, "document.querySelector('.using').textContent + '|' + document.querySelector('#body .more-btn').textContent").await;
    assert!(other.contains("Using OpenAI · gpt-4.1-mini") && other.contains("More options · 1 changed"), "{other}");
    assert_eq!(eval(c, "document.querySelector('#body .more-btn').getAttribute('aria-expanded')").await, "\"true\"", "open while another provider is in use");
}

/// A2 (v0.7): Settings › Limits starts with one protection statement worked out from the settings
/// in force (never "only your app's pages" while every domain is allowed), three example sentences
/// that only tighten or show something and go to Chat, and every limit under More options, closed.
/// With the block list emptied it says so instead of "You are protected".
#[tokio::test(flavor = "multi_thread")]
async fn the_limits_page_says_what_protects_you_and_changes_go_through_chat() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let d = dashboard("ui-limits", |_| {}).await;
    let c = &d.client;
    open(c, "#/settings/limits", "You are protected").await;
    let said = eval(c, "document.querySelector('.protect').textContent").await;
    // The page text is long; checked in parts (the tool shortens long values).
    let part = |js: &str| js.to_owned();
    for (k, want) in [
        ("0", "You are protected"),
        ("1", "Buttons like \\\"Pay\\\" and \\\"Delete account\\\" are never pressed."),
        ("2", "Buttons like \\\"Delete\\\" are only pressed when the test step says it means it."),
        ("3", "Every test fastpilot runs (Run, schedules) stops after 30 minutes and 30 AI steps."),
    ] {
        let got = eval(c, &part(&format!("(() => {{ const p = document.querySelector('.protect'); const t = p.querySelector('p').textContent.split(/(?<=\\.) /); return {k} === 0 ? p.querySelector('b').textContent : t[{k} - 1]; }})()"))).await;
        assert_eq!(got, format!("\"{want}\""), "statement part {k}");
    }
    assert!(!said.contains("only open") && !said.contains("your app's pages"), "no domain claim while every domain is allowed");
    let examples = eval(c, "[...document.querySelectorAll('.say-row')].map((b) => b.textContent).join('|')").await;
    assert_eq!(examples, "\"Also block the word \\\"Refund\\\"→|Never press \\\"Log out\\\"→|Show me what is blocked→\"");
    let more = eval(c, "(() => { const b = document.querySelector('#body .more-btn'); const chips = document.querySelector('[aria-label=\"Add a blocked word\"]') || document.querySelector('.panel .grp .opt'); return b.getAttribute('aria-expanded') + '|' + !!chips.closest('[hidden]'); })()").await;
    assert_eq!(more, "\"false|true\"", "every limit is under More options, closed");
    // A sentence goes to Chat (sending is caught here: no real Claude in this test).
    eval(c, "(() => { window.__sent = null; window.chatSend = async (t) => { window.__sent = t; }; document.querySelectorAll('.say-row')[1].click(); return 'ok'; })()").await;
    assert_eq!(eval(c, "location.hash + '|' + window.__sent").await, "\"#/agents/default/chat|Never press \\\"Log out\\\"\"", "the one Chat page (U1)");
    // Security #187 L1: page scripts allowed means a script can press any button: not "protected".
    eval(c, "(async () => { await api('/api/settings', { key: 'allow_eval_script', value: true }); await loadSettings(); location.hash = '#/settings/limits'; render(); return 'ok'; })()").await;
    open(c, "#/settings/limits", "Less protected").await;
    let scripts = eval(c, "document.querySelector('.protect b').textContent + '|' + document.querySelector('.protect p').textContent.split('. ')[0]").await;
    assert_eq!(scripts, "\"Less protected: page scripts may run|Test steps never press buttons like \\\"Pay\\\" and \\\"Delete account\\\", but page scripts may run, and a script can press any button\"");
    eval(c, "(async () => { await api('/api/settings', { key: 'allow_eval_script', value: null }); return 'ok'; })()").await;
    // Domains set: then it says so. Block list emptied: not "You are protected".
    eval(c, "(async () => { await api('/api/settings', { key: 'allowed_domains', value: ['shop.example'] }); await api('/api/settings', { key: 'block_actions', value: [] }); await loadSettings(); location.hash = '#/settings/limits'; render(); return 'ok'; })()").await;
    open(c, "#/settings/limits", "Less protected").await;
    let less = eval(c, "document.querySelector('.protect').textContent").await;
    assert!(less.contains("Less protected: no button is blocked") && less.contains("Tests only open shop.example.") && !less.contains("never pressed"), "{less}");
}

/// C1 (v0.7): every page says "or just say it" with fixed sentences that go to the one Chat; an
/// empty Chat offers three; a card that only opens a page goes to one of fastpilot's own pages; the
/// Chat footer is one line.
#[tokio::test(flavor = "multi_thread")]
async fn every_page_can_be_said_in_chat_and_cards_open_only_our_pages() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let d = dashboard("ui-c1", |_| {}).await;
    let c = &d.client;
    for (hash, wait, want) in [
        ("#/device", "Device", "Start the Android emulator"),
        ("#/integrations", "Integrations", "Connect Jira"),
    ] {
        open(c, hash, wait).await;
        see(c, "or just say it in chat").await;
        let rows = eval(c, "[...document.querySelectorAll('.limit-say .say-row')].map((b) => b.firstChild.textContent).join('|')").await;
        assert!(rows.contains(want), "{hash}: {rows}");
    }
    // A sentence goes to the one Chat page (sending is caught: no real Claude here).
    eval(c, "(() => { window.__sent = null; window.chatSend = async (t) => { window.__sent = t; }; [...document.querySelectorAll('.limit-say .say-row')][0].click(); return 'ok'; })()").await;
    let mut got = String::new();
    for _ in 0..40 {
        got = eval(c, "location.hash + '|' + window.__sent").await;
        if got.contains("agents") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(got, "\"#/agents/default/chat|Connect Jira\"");
    // The footer is one line; an "open" card links only to our page, whatever the bot wrote.
    see(c, "Changes wait for your click on their card.").await;
    let link = eval(c, "(() => { const s = proposalCard({ id: 'x', kind: 'open', open: 'secrets', title: 'Set the password', rows: [], status: 'pending' }); const bad = proposalCard({ id: 'y', kind: 'open', open: 'https://evil.example', title: 'x', rows: [], status: 'pending' }); return s.querySelector('a.btn').getAttribute('href') + '|' + s.querySelector('a.btn').textContent + '|' + !!bad.querySelector('a[href^=\"http\"]'); })()").await;
    assert_eq!(link, "\"#/settings/secrets|Open Secrets|false\"");
}
