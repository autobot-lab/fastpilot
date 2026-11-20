//! C7b "Ambil alih" on a web session, as a user does it in `fastpilot ui` (driven by fastpilot in
//! Chrome): take the session over, the AI's next step waits, tap and type from the panel, give it
//! back; the step then runs on what the user typed and its result tells the AI what happened.
//! Input is refused while the session is not taken over. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::Duration;

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

/// `evalScript` in `session`; the value as returned (`"..."`).
async fn eval(client: &Client, session: &str, js: &str) -> String {
    let flow = format!("- evalScript: {}\n", serde_json::to_string(js).expect("js"));
    let (ok, out) = call(client, "run_flow", json!({ "flow": flow, "session": session })).await;
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

/// POST from the page itself (its cookie and Origin), status kept in `window.__st`.
fn post(body: &str) -> String {
    format!(
        "(window.__st = 0, fetch('/api/live/control', {{ method: 'POST', headers: {{ 'Content-Type': 'application/json' }}, body: JSON.stringify({body}) }}).then(r => window.__st = r.status), 'sent')"
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn take_over_a_web_session_and_give_it_back() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-ui-take-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
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
    let page = format!("file://{ROOT}/examples/demo/login.html");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- session: {{ start: shop, platform: web, target: \"{page}\" }}\n") })).await;
    assert!(ok, "{out}");
    // Where the Email field is, as fractions of the screen (what a click on the picture sends).
    let at = eval(&client, "shop", "(() => { const r = document.querySelector('#email').getBoundingClientRect(); return [(r.left + r.width / 2) / innerWidth, (r.top + r.height / 2) / innerHeight].join(','); })()").await;
    let (fx, fy) = at.trim_matches('"').split_once(',').expect("fractions");

    let flow = format!("- session: {{ start: viewer, platform: web, target: \"{}\" }}\n- assertVisible: {{ text: \"Ambil alih\", timeout: 10000 }}\n", link.trim());
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "the shop card offers Ambil alih: {out}");

    // Input before taking over is refused.
    eval(&client, "viewer", &post(&format!("{{ pid: Number(document.querySelector('[data-fk^=\"take:\"]').dataset.fk.split(':')[1]), session: 'shop', op: 'tap', x: {fx}, y: {fy} }}"))).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(eval(&client, "viewer", "window.__st").await, "409", "no input without a takeover");

    // Ambil alih: the large screen opens with the steering bar.
    eval(&client, "viewer", "document.querySelector('[data-fk^=\"take:\"]').click(), 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Diambil alih olehmu\", timeout: 5000 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    assert_eq!(eval(&client, "viewer", "!!document.querySelector('dialog.zoom.steering[open] .steer-text')").await, "true");

    {
        // The AI's next step on `shop` waits while the user has it.
        let waiting = call(&client, "run_flow", json!({ "flow": "- evalScript: \"document.querySelector('#email').value\"\n", "session": "shop" }));
        tokio::pin!(waiting);
        let early = tokio::time::timeout(Duration::from_millis(1500), &mut waiting).await;
        assert!(early.is_err(), "the automatic step must wait while the session is taken over: {early:?}");

        // The user taps the Email field and types from the panel.
        eval(&client, "viewer", &post(&format!("{{ pid: Number(document.querySelector('[data-fk^=\"take:\"]').dataset.fk.split(':')[1]), session: 'shop', op: 'tap', x: {fx}, y: {fy} }}"))).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(eval(&client, "viewer", "window.__st").await, "200");
        eval(&client, "viewer", "(() => { const i = document.querySelector('.steer-text'); i.value = 'budi@contoh.id'; [...document.querySelectorAll('.steer-row button')].find(b => b.textContent === 'Ketik').click(); return 'ok'; })()").await;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut waiting).await.is_err(), "still waiting before Kembalikan");

        // Kembalikan: the waiting step runs on what the user typed, and says the user stepped in.
        eval(&client, "viewer", "document.querySelector('[data-fk^=\"take-zoom:\"]').click(), 'ok'").await;
        let (ok, out) = tokio::time::timeout(Duration::from_secs(10), waiting).await.expect("runs after Kembalikan");
        assert!(ok, "{out}");
        assert!(out.contains("= \"budi@contoh.id\""), "the user's text is in the field: {out}");
        assert!(out.contains("the user took over session `shop` in fastpilot ui") && out.contains("1 tap, 1 text input"), "{out}");
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(eval(&client, "viewer", "document.querySelector('[data-fk^=\"take:\"]').textContent").await, "\"Ambil alih\"");

    // Taken over, the banner follows to other pages; Escape on the large screen gives it back.
    eval(&client, "viewer", "document.querySelector('[data-fk^=\"take:\"]').click(), 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Kembalikan & tutup\", timeout: 5000 }\n", "session": "viewer" })).await;
    assert!(ok, "the close button says it gives the session back: {out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- pressKey: Escape\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(eval(&client, "viewer", "document.querySelector('[data-fk^=\"take:\"]').textContent").await, "\"Ambil alih\"", "closing returned it");
    eval(&client, "viewer", "document.querySelector('[data-fk^=\"take:\"]').click(), 'ok'").await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    eval(&client, "viewer", "document.querySelector('dialog.zoom') && document.querySelector('dialog.zoom').close(), location.hash = '#/settings/limits', 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Kamu sedang mengambil alih\", timeout: 8000 }\n", "session": "viewer" })).await;
    assert!(ok, "banner on Batasan: {out}");
    eval(&client, "viewer", "[...document.querySelectorAll('#take-banner button')].find(b => b.textContent.includes('Kembalikan')).click(), 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertNotVisible: { text: \"Kamu sedang mengambil alih\", timeout: 8000 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    // Design L1: focus on the page title, not lost to body.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(eval(&client, "viewer", "document.activeElement.tagName").await, "\"H1\"");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(home);
}
