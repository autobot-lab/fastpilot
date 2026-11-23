//! Wave 3 (QA #2, Design #1–#3, #5, #6): the Live page as a user sees it, driven by fastpilot
//! itself in Chrome. The first page (link without `#/live`) shows the sessions instead of hanging;
//! keyboard focus survives the 1 s refresh; Perbesar is a modal dialog that keeps Tab inside,
//! closes on Escape and gives focus back; a blocked step says why; the header counts it.
//! Skips without Chrome.

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

/// `evalScript` in the viewer session; the value as returned (`= "..."`).
async fn eval(client: &Client, js: &str) -> String {
    let flow = format!("- evalScript: {}\n", serde_json::to_string(js).expect("js"));
    let (ok, out) = call(client, "run_flow", json!({ "flow": flow, "session": "viewer" })).await;
    assert!(ok, "{out}");
    out.lines().find_map(|l| l.rsplit_once(" = ").map(|(_, v)| v.trim().to_owned())).unwrap_or_default()
}

struct Ui(std::process::Child);
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn live_page_works_with_the_keyboard() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-ui-kbd-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    std::fs::write(home.join(".fastpilot/config.toml"), "block_actions = [\"Keluar\"]\n").expect("config");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .env("HOME", &home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_LIVE_DIR")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("ui");
    let mut link = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut link).expect("link");
    let _ui = Ui(child);

    // The tested app runs in a fastpilot with block_actions; the page is watched from another one
    // without it (evalScript is off while block_actions is set). Both publish to the same Live folder.
    let live = home.join(".fastpilot/live");
    let viewer_home = home.join("viewer");
    std::fs::create_dir_all(&viewer_home).expect("viewer home");
    let mcp = |h: &Path| {
        let (h, live) = (h.to_owned(), live.clone());
        let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
            c.arg("mcp").env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").current_dir(ROOT).env("HOME", &h).env("FASTPILOT_LIVE_DIR", &live).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
        });
        TokioChildProcess::new(cmd).expect("spawn")
    };
    let shop: Client = ().serve(mcp(&home)).await.expect("initialize");
    let client: Client = ().serve(mcp(&viewer_home)).await.expect("initialize");
    // A test that a guard rail stops.
    let page = format!("file://{ROOT}/examples/demo/login.html");
    let flow = format!(
        "- session: {{ start: shop, platform: web, target: \"{page}\" }}\n- inputText: {{ into: Email, text: a@b.com }}\n- inputText: {{ into: Password, text: x }}\n- tapOn: Masuk\n- tapOn: Keluar\n"
    );
    let (ok, out) = call(&shop, "run_flow", json!({ "flow": flow })).await;
    assert!(!ok && out.contains("(your blocked words)"), "{out}");

    // The user opens the printed link (no hash: Chat first, s6a), then Live.
    let flow = format!("- session: {{ start: viewer, platform: web, target: \"{}\" }}\n", link.trim());
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    // Someone who already knows the dashboard: no first-run wizard (E4).
    eval(&client, "(localStorage.setItem('fastpilot.welcome', 'done'), location.hash = '#/live', 'ok')").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Step 5/5\", timeout: 10000 }\n" })).await;
    assert!(ok, "Live must show the sessions, not hang on 'Membaca sesi…': {out}");
    // Header pill counts the blocked session; the step says why.
    assert!(eval(&client, "document.querySelector('.top .pill').textContent").await.contains("1 blocked"));
    // FastTest final report: no "null" in the header (a null child became text).
    let top = eval(&client, "document.querySelector('.top').textContent").await;
    // UXA5: "Tests at once" is no longer on the Live header (it is in Settings › Limits).
    assert!(!top.contains("null") && !top.contains("Tests at once"), "{top}");

    // FastTest final report: "click Web, fill in, Create" said "Pick where it tests" (Web was
    // picked already and the click took it off). The last place picked stays on, with a reason.
    // S4 + U1: with one agent, Chat has no agent list; "New agent" is under More below it.
    eval(&client, "location.hash = '#/agents/default/chat', 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Send\", timeout: 8000 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    let hidden_new = "(() => { const a = [...document.querySelectorAll('#body .more a')].find((x) => x.textContent.includes('New agent')); return a ? String(!!a.closest('[hidden]')) : 'none'; })()";
    assert_eq!(eval(&client, hidden_new).await, "\"true\"", "New agent waits under More");
    eval(&client, "[...document.querySelectorAll('#body .more-btn')].find((b) => b.textContent.includes('More')).click(), 'ok'").await;
    assert_eq!(eval(&client, hidden_new).await, "\"false\"", "More shows it");

    // S4: New agent needs no typing: three focuses, the form under "Describe it yourself".
    eval(&client, "location.hash = '#/agents/new', 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Checkout and payments\", timeout: 8000 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    let quick = eval(&client, "[[...document.querySelectorAll('.ag-new button[data-focus]')].map((b) => b.textContent.replace('→', '').trim()).join('|'), [...document.querySelectorAll('.ag-new input, .ag-new textarea')].filter((e) => !e.closest('[hidden]')).length, document.activeElement.dataset.focus].join(' / ')").await;
    assert_eq!(quick, "\"Regression of everything|Checkout and payments|Login and accounts / 0 / regression\"", "no field to type in on the main path, the first focus has the keyboard");
    eval(&client, "document.querySelector('.ag-new .more-btn').click(), 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Create agent\", timeout: 5000 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    let chip = |id: &str| format!("document.querySelector('.ag-new button[data-device=\"{id}\"]')");
    let state = |id: &str| format!("(() => {{ const b = {}; return b.getAttribute('aria-pressed'); }})()", chip(id));
    assert_eq!(eval(&client, &state("web")).await, "\"true\"", "Web is picked from the start");
    eval(&client, &format!("{}.click(), 'ok'", chip("web"))).await;
    assert_eq!(eval(&client, &state("web")).await, "\"true\"", "clicking the only pick keeps it");
    assert!(eval(&client, "document.querySelector('.ag-new .dev-hint').textContent").await.contains("stays picked"));
    eval(&client, &format!("{}.click(), {}.click(), 'ok'", chip("android_emulator"), chip("web"))).await;
    assert_eq!((eval(&client, &state("web")).await, eval(&client, &state("android_emulator")).await), ("\"false\"".to_owned(), "\"true\"".to_owned()), "switched to the emulator");
    eval(&client, &format!("{}.click(), 'ok'", chip("web"))).await;
    let fill = "(() => { for (const [l, v] of [['Name', 'Device pick'], ['What it does', 'Check the shop on the web and Android']]) { const e = document.querySelector(`.ag-new [aria-label=\"${l}\"]`); e.value = v; e.dispatchEvent(new Event('input')); } [...document.querySelectorAll('.ag-new button')].find((b) => b.textContent.includes('Create agent')).click(); return 'ok'; })()";
    eval(&client, fill).await;
    let mut saved = String::new();
    for _ in 0..40 {
        saved = std::fs::read_to_string(home.join(".fastpilot/agents.json")).unwrap_or_default();
        if saved.contains("device-pick") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let agents: Value = serde_json::from_str(&saved).unwrap_or_default();
    let made = agents["agents"].as_array().and_then(|a| a.iter().find(|x| x["id"] == "device-pick")).cloned().unwrap_or_default();
    assert_eq!(made["devices"], json!(["web", "android_emulator"]), "{saved}");
    let mut at = String::new();
    for _ in 0..40 {
        at = eval(&client, "location.hash").await;
        if at.contains("device-pick/chat") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert!(at.contains("#/agents/device-pick/chat"), "Create opens the new agent: {at}");
    // v0.4.1 (the user): none of the old suggestion chips ("Test the login page in Chrome", …).
    // C1 / DC1 (accepted 22 Nov 2026): three one-click suggestions that name what they do.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Ask or tell the fastpilot bot\", timeout: 8000 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    let empty = eval(&client, "(() => { const e = document.querySelector('.empty'); return [[...e.querySelectorAll('.say-row')].map((b) => b.firstChild.textContent).join(','), /Test the login page|every 2 hours|Block the Pay button/.test(document.body.textContent)].join('|'); })()").await;
    assert_eq!(empty, "\"Check the login,Map the app,Can we release?|false\"", "three suggestions, none of the old chips");
    // S4: one click on a focus makes the agent and opens its Chat; a second click opens the same one.
    for _ in 0..2 {
        eval(&client, "location.hash = '#/agents/new', 'ok'").await;
        // Clicked from the page: the viewer's own limits rightly block a tap on "checkout".
        let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Login and accounts\", timeout: 8000 }\n", "session": "viewer" })).await;
        assert!(ok, "{out}");
        eval(&client, "document.querySelector('.ag-new button[data-focus=\"checkout\"]').click(), 'ok'").await;
        let mut at = String::new();
        for _ in 0..40 {
            at = eval(&client, "location.hash").await;
            if at.contains("checkout-tester/chat") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert!(at.contains("#/agents/checkout-tester/chat"), "one click opens the new agent: {at}");
    }
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(home.join(".fastpilot/agents.json")).unwrap_or_default()).unwrap_or_default();
    let made: Vec<&Value> = saved["agents"].as_array().map(|a| a.iter().filter(|x| x["name"] == "Checkout tester").collect()).unwrap_or_default();
    assert_eq!(made.len(), 1, "{saved}");
    assert!(made[0]["skills"].as_array().is_some_and(|s| s.iter().any(|x| x == "safe-checkout")), "skills picked by itself: {saved}");
    eval(&client, "location.hash = '#/live', 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Step 5/5\", timeout: 10000 }\n", "session": "viewer" })).await;
    assert!(ok, "back on Live: {out}");
    // The blocked session's steps: pick its card (the viewer's own session may be listed first).
    eval(&client, "(() => { const s = lastLive.sessions.find((x) => x.id === 'shop'); selectSession(`${s.pid}:${s.id}`); return 'ok'; })()").await;
    let why = eval(&client, "[...document.querySelectorAll('.steps .blk')].map(l => l.textContent).join('|')").await;
    // Design N2 (P2): the word as the user wrote it in Batasan, not lower-cased.
    // v0.3.2 design (s3): the button, the word as written in Batasan, and where to change it.
    assert!(why.contains("Blocked by Limits. The button text \\\"Keluar\\\" contains Keluar. The page was not pressed."), "{why}");

    // Focus survives the refresh (polls every second).
    eval(&client, "document.querySelector('[data-fk^=\"zoom:\"]').focus(), 'ok'").await;
    tokio::time::sleep(std::time::Duration::from_millis(2600)).await;
    assert_eq!(eval(&client, "document.activeElement.dataset.fk && document.activeElement.dataset.fk.slice(0, 5)").await, "\"zoom:\"");
    eval(&client, "document.querySelector('[data-fk^=\"limits:\"]').focus(), 'ok'").await;
    tokio::time::sleep(std::time::Duration::from_millis(2600)).await;
    assert_eq!(eval(&client, "document.activeElement.textContent").await, "\"Change it in Limits →\"");

    // Perbesar: modal, Tab stays inside, Escape closes, focus returns.
    eval(&client, "document.querySelector('[data-fk^=\"zoom:\"]').focus(), 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- pressKey: Enter\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    assert_eq!(eval(&client, "!!document.querySelector('dialog.zoom[open]') && !!document.activeElement.closest('dialog')").await, "true");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- pressKey: Tab\n- pressKey: Tab\n- pressKey: Tab\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    assert_eq!(eval(&client, "!!document.activeElement.closest('dialog')").await, "true", "Tab must stay in the dialog");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- pressKey: Escape\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    assert_eq!(eval(&client, "!document.querySelector('dialog.zoom') && document.activeElement.dataset.fk.startsWith('zoom:')").await, "true");

    // QA N3 (P2): a message about the last attempt goes away once the user edits the text.
    eval(&client, "location.hash = '#/settings/limits', 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Limits & permissions\", timeout: 5000 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    // UXA5: "Tests at once" is in Settings › Limits, with its default marked. A2: every limit
    // is under More options, below the one-line summary.
    eval(&client, "document.querySelector('#body .more-btn').click(), 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Tests at once\", timeout: 5000 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    let mut picked = String::new();
    for _ in 0..20 {
        picked = eval(&client, "(() => { const s = document.querySelector('select[aria-label=\"Tests at once\"]'); return s ? s.selectedOptions[0].textContent + '|' + s.options.length : 'none'; })()").await;
        if picked != "\"none\"" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(picked, "\"3 (default)|20\"", "the default is marked");
    let enter = "(() => { const i = document.querySelector('input[data-key=\"block_actions\"]'); i.value = VALUE; i.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); return document.querySelector('.toast')?.textContent || ''; })()";
    let msg = eval(&client, &enter.replace("VALUE", "'a, b'")).await;
    assert!(msg.contains("without commas"), "{msg}");
    let left = eval(&client, "(() => { const i = document.querySelector('input[data-key=\"block_actions\"]'); i.value = 'a'; i.dispatchEvent(new Event('input', { bubbles: true })); return document.querySelectorAll('.toast').length; })()").await;
    assert_eq!(left, "0", "the old message must go when the user types");
    let msg = eval(&client, &enter.replace("VALUE", "'KELUAR'")).await;
    assert!(msg.contains("\\\"KELUAR\\\" is already in the list"), "a duplicate in other letters is caught: {msg}");

    // Design N3 (P2): nothing wider than a 390 px phone screen.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- setViewport: { width: 390, height: 844 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    for page in ["#/live", "#/schedules", "#/settings/limits", "#/settings/model", "#/settings/secrets", "#/device", "#/apps"] {
        eval(&client, &format!("location.hash = '{page}', 'ok'")).await;
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        let w = eval(&client, "document.documentElement.scrollWidth + '/' + innerWidth + ' ' + [...document.querySelectorAll('body *')].filter(e => e.getBoundingClientRect().right > innerWidth + 0.5).slice(-4).map(e => e.tagName + '.' + e.className + ':' + Math.round(e.getBoundingClientRect().right)).join(' ')").await;
        let (scroll, inner) = w.trim_matches('"').split(' ').next().unwrap_or_default().split_once('/').expect("widths");
        assert!(scroll.parse::<u32>().expect("n") <= inner.parse::<u32>().expect("n"), "{page} overflows at 390 px: {w}");
    }
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- setViewport: desktop\n", "session": "viewer" })).await;
    assert!(ok, "{out}");

    // Design N1 (P2): after "Muat ulang" on a broken file, keyboard focus lands on the page title.
    let config = home.join(".fastpilot/config.toml");
    std::fs::write(&config, "block_actions = [\"Keluar\"\n").expect("break");
    eval(&client, "location.hash = '#/settings/limits', location.reload(), 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"The settings file is damaged\", timeout: 8000 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    std::fs::write(&config, "block_actions = [\"Keluar\"]\n").expect("fix");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- tapOn: \"Reload after fixing it\"\n- assertNotVisible: { text: \"The settings file is damaged\", timeout: 5000 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    assert_eq!(eval(&client, "document.activeElement.tagName + ':' + document.activeElement.textContent").await, "\"H1:Settings\"");
    client.cancel().await.expect("shutdown");
    shop.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(home);
}
