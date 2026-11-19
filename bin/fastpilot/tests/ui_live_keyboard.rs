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
    out.lines().find_map(|l| l.split_once(" = ").map(|(_, v)| v.trim().to_owned())).unwrap_or_default()
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

    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("HOME", &home).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_LIVE_DIR").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    // A test that a guard rail stops.
    let page = format!("file://{ROOT}/examples/demo/login.html");
    let flow = format!(
        "- session: {{ start: shop, platform: web, target: \"{page}\" }}\n- inputText: {{ into: Email, text: a@b.com }}\n- inputText: {{ into: Password, text: x }}\n- tapOn: Masuk\n- tapOn: Keluar\n"
    );
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(!ok && out.contains("block_actions"), "{out}");

    // The user opens the printed link: no `#/live` in it.
    let flow = format!("- session: {{ start: viewer, platform: web, target: \"{}\" }}\n- assertVisible: {{ text: \"Langkah 5/5\", timeout: 10000 }}\n", link.trim());
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "the first page must show the sessions, not hang on 'Membaca sesi…': {out}");
    // Header pill counts the blocked session; the step says why.
    assert!(eval(&client, "document.querySelector('.top .pill').textContent").await.contains("1 diblokir"));
    let why = eval(&client, "[...document.querySelectorAll('.steps .blk')].map(l => l.textContent).join('|')").await;
    assert!(why.contains("diblokir: teks berisi \\\"keluar\\\""), "{why}");

    // Focus survives the refresh (polls every second).
    eval(&client, "document.querySelector('[data-fk^=\"zoom:\"]').focus(), 'ok'").await;
    tokio::time::sleep(std::time::Duration::from_millis(2600)).await;
    assert_eq!(eval(&client, "document.activeElement.dataset.fk && document.activeElement.dataset.fk.slice(0, 5)").await, "\"zoom:\"");
    eval(&client, "document.querySelector('[data-fk^=\"limits:\"]').focus(), 'ok'").await;
    tokio::time::sleep(std::time::Duration::from_millis(2600)).await;
    assert_eq!(eval(&client, "document.activeElement.textContent").await, "\"Lihat Batasan\"");

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
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(home);
}
