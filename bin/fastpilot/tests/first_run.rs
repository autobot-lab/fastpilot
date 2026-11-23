//! E4 (v0.7): the first run, in Chrome, as a new user (temp HOME, no app, no developer folder).
//! The first screen asks what to test; pasting the address adds the app (no Next); one click on
//! "Check the login" goes to Chat with that sentence; the wizard never comes back. "I know my way
//! around" skips it for good. Until a test is saved the menu is simple (Chat, Live, Device,
//! Settings, More), and Settings › "Show all menus" brings the rest. Skips without Chrome.

use rmcp::RoleClient;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::path::Path;

type Client = RunningService<RoleClient, ()>;
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

async fn flow(c: &Client, steps: &str) -> (bool, String) {
    let mut args = json!({ "flow": steps }).as_object().cloned().unwrap_or_default();
    if !steps.contains("start: person") {
        args.insert("session".into(), json!("person"));
    }
    let r = c.call_tool(CallToolRequestParams::new("run_flow").with_arguments(args)).await.expect("call");
    (r.is_error != Some(true), r.content.iter().filter_map(|x| x.as_text().map(|t| t.text.clone())).collect::<Vec<_>>().join("\n"))
}

async fn see(c: &Client, text: &str) {
    let (ok, out) = flow(c, &format!("- assertVisible: {{ text: {}, timeout: 10000 }}\n", serde_json::to_string(text).expect("text"))).await;
    assert!(ok, "{text}: {out}");
}

async fn eval(c: &Client, js: &str) -> String {
    let (ok, out) = flow(c, &format!("- evalScript: {}\n", serde_json::to_string(js).expect("js"))).await;
    assert!(ok, "{out}");
    out.lines().find_map(|l| l.rsplit_once(" = ").map(|(_, v)| v.trim().trim_matches('"').to_owned())).unwrap_or_default()
}

async fn until(c: &Client, js: &str, want: &str) -> String {
    let mut got = String::new();
    for _ in 0..60 {
        got = eval(c, js).await;
        if got == want {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    got
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_user_pastes_an_address_and_one_click_starts_the_first_test() {
    if !["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists()) {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-first-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("person")).expect("home");
    let mut ui = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(&home)
        .env("HOME", &home)
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut link = String::new();
    BufReader::new(ui.stdout.take().expect("stdout")).read_line(&mut link).expect("link");
    let p = home.join("person");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(ROOT).env("HOME", &p).env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").env("FASTPILOT_LIVE_DIR", p.join("live")).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let c: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = flow(&c, &format!("- session: {{ start: person, platform: web, target: \"{}\" }}\n", link.trim())).await;
    assert!(ok, "{out}");

    // The first screen: what to test, and the simple menu.
    see(&c, "What do you want to test?").await;
    see(&c, "Welcome to fastpilot").await;
    let rail = "[...document.querySelectorAll('.rail a, .rail .rail-more')].filter((a) => getComputedStyle(a).display !== 'none').map((a) => a.textContent.trim()).join('|')";
    assert_eq!(eval(&c, rail).await, "Chat|Live|Device|More|Settings", "the simple menu");

    // Paste the address: the app is added, no Next to click.
    let demo = format!("file://{}/examples/demo/login.html", std::fs::canonicalize(ROOT).expect("root").display());
    eval(&c, &format!("(() => {{ const w = document.getElementById('wz-where'); w.value = {}; w.dispatchEvent(new Event('paste')); return 'ok'; }})()", serde_json::to_string(&demo).expect("demo"))).await;
    see(&c, "What should we check first?").await;
    assert_eq!(until(&c, "(async () => (await api('/api/apps')).apps.map((a) => a.name).join('|'))()", "Demo").await, "Demo");

    // One click: Chat, with that sentence sent (caught here: no real Claude in this test).
    eval(&c, "(() => { window.__sent = null; window.chatSend = async (t) => { window.__sent = t; }; [...document.querySelectorAll('.say-row')].find((b) => b.textContent.includes('Check the login')).click(); return 'ok'; })()").await;
    assert_eq!(until(&c, "location.hash + '|' + window.__sent", "#/agents/default/chat|Check the login").await, "#/agents/default/chat|Check the login");
    see(&c, "Send").await;
    // It never comes back.
    eval(&c, "(location.reload(), 'ok')").await;
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    see(&c, "Send").await;
    assert_eq!(eval(&c, "String(!!document.querySelector('.wz'))").await, "false");

    // Settings › Show all menus: the full menu even before a saved test.
    eval(&c, "(location.hash = '#/settings/limits', 'ok')").await;
    see(&c, "Show all menus").await;
    eval(&c, "(document.querySelector('.wz-full input').click(), 'ok')").await;
    assert_eq!(until(&c, rail, "Chat|Live|Schedules|Map|Memory|History|Device|Integrations|Settings").await, "Chat|Live|Schedules|Map|Memory|History|Device|Integrations|Settings");

    // A new user who knows their way around: skipped for good.
    eval(&c, "(async () => { localStorage.clear(); const a = await api('/api/apps'); for (const x of a.apps) await api('/api/apps', { action: 'remove', id: x.id }); location.hash = '#/agents'; location.reload(); return 'ok'; })()").await;
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    see(&c, "What do you want to test?").await;
    let (ok, out) = flow(&c, "- tapOn: I know my way around\n").await;
    assert!(ok, "{out}");
    see(&c, "Send").await;
    eval(&c, "(location.reload(), 'ok')").await;
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    see(&c, "Send").await;
    assert_eq!(eval(&c, "String(!!document.querySelector('.wz'))").await, "false", "skipped for good");

    let _ = c.cancel().await;
    let _ = ui.kill();
    let _ = ui.wait();
    let _ = std::fs::remove_dir_all(&home);
}
