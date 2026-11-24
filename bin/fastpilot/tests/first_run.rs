//! v0.7.1 FC5: the first run, in Chrome, as a new user (temp HOME, no app, no developer folder).
//! There is no wizard and no "Add your app": the first screen is Chat, "What should I test?" with
//! the hint line; pasting the address in the box adds the app at once (a result line, then the hint
//! how to try it), and the agent's line shows it. Skips without Chrome.

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
async fn a_new_user_pastes_an_address_in_chat_and_it_is_added() {
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

    // The first screen is Chat: the greeting, the hint line and the box. No wizard, no form.
    see(&c, "What should I test?").await;
    see(&c, "Paste your website address, or say “test my Android app”.").await;
    assert_eq!(eval(&c, "[!!document.querySelector('.wz, #appbar, dialog.appdlg'), document.body.textContent.includes('Add your app'), document.querySelector('.hdr2 .sub')?.textContent].join('|')").await, "false|false|No app yet");

    // Paste the address and send: added at once, then how to try it.
    let demo = format!("file://{}/examples/demo/login.html", std::fs::canonicalize(ROOT).expect("root").display());
    let (ok, out) = flow(&c, &format!("- inputText: {{ into: {{ role: textfield }}, text: {} }}\n", serde_json::to_string(&demo).expect("demo"))).await;
    assert!(ok, "{out}");
    eval(&c, "(document.getElementById('chat-send').click(), 'ok')").await;
    see(&c, "Say “check the login” to try it.").await;
    let line = eval(&c, "(() => { const l = document.querySelector('.result-line'); return [l.getAttribute('role'), l.querySelector('.rl-text').textContent.startsWith('Added '), !!l.querySelector('.rl-undo')].join('|'); })()").await;
    assert_eq!(line, "status|true|true");
    assert_eq!(until(&c, "String(document.querySelector('.hdr2 .sub')?.textContent.endsWith('· Website'))", "true").await, "true", "the agent shows its app");
    assert_eq!(until(&c, "(async () => (await api('/api/apps')).apps.length)().then(String)", "1").await, "1");
    // It never asks again: a reload is Chat with that chat.
    eval(&c, "(location.reload(), 'ok')").await;
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    see(&c, "Say “check the login” to try it.").await;
    assert_eq!(eval(&c, "String(!!document.querySelector('.wz'))").await, "false");

    let _ = c.cancel().await;
    let _ = ui.kill();
    let _ = ui.wait();
    let _ = std::fs::remove_dir_all(&home);
}
