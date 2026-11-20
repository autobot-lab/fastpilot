//! The Jadwal page as a user meets it (Design review): a refused save points at its field, the
//! webhook and repo fields have their own labels, Escape closes the new-schedule panel, and after
//! saving the focus is on the new schedule. Driven by fastpilot in Chrome; skips without Chrome.

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
async fn jadwal_form_points_at_fields_and_keeps_focus() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-sched-page-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(ROOT)
        .env("HOME", &home)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env("FASTPILOT_SCHEDULER", "launchd")
        .env("FASTPILOT_SCHEDULER_DIR", home.join("LaunchAgents"))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("ui");
    let mut link = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut link).expect("link");
    let _ui = Ui(child);
    let viewer_home = home.join("viewer");
    std::fs::create_dir_all(&viewer_home).expect("viewer");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(ROOT).env("HOME", &viewer_home).env("FASTPILOT_LIVE_DIR", viewer_home.join("live")).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let flow = format!("- session: {{ start: viewer, platform: web, target: \"{}\" }}\n", link.trim());
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    eval(&client, "location.hash = '#/schedules', 'ok'").await;
    see(&client, "Belum ada jadwal").await;

    // Escape closes the panel and gives focus back to "Jadwal baru".
    eval(&client, "[...document.querySelectorAll('.top button')].find(b => b.textContent.includes('Jadwal baru')).click(), 'ok'").await;
    see(&client, "Simpan jadwal").await;
    assert_eq!(eval(&client, "document.activeElement.closest('.sched-form') ? 'in' : 'out'").await, "\"in\"");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- pressKey: Escape\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    assert_eq!(eval(&client, "!document.querySelector('.sched-form') && document.activeElement.textContent.includes('Jadwal baru')").await, "true");

    // Webhook and repo fields: labelled, shown when their box is ticked.
    eval(&client, "[...document.querySelectorAll('.top button')].find(b => b.textContent.includes('Jadwal baru')).click(), 'ok'").await;
    see(&client, "Simpan jadwal").await;
    let hidden = eval(&client, "(() => { const w = document.querySelector('input[type=password]'); return String(!!w.closest('[hidden]')); })()").await;
    assert_eq!(hidden, "\"true\"", "URL field hidden until Webhook is ticked");
    let label = eval(&client, "(() => { const box = [...document.querySelectorAll('.ck')].find(l => l.textContent.includes('Webhook')).querySelector('input'); box.click(); const w = document.querySelector('input[type=password]'); return (w.closest('[hidden]') ? 'hidden' : 'shown') + ':' + w.closest('label.fld').querySelector('.l').textContent; })()").await;
    assert_eq!(label, "\"shown:URL webhook\"");

    // A refused save: the reason under its field, the field marked and focused.
    let err = eval(&client, "(() => { const f = document.querySelector('.sched-form'); const [name, tests] = f.querySelectorAll('input[type=text]'); name.value = 'Login pagi'; tests.value = 'tests/login.md'; const box = [...document.querySelectorAll('.ck')].find(l => l.textContent.includes('Webhook')).querySelector('input'); box.click(); [...f.querySelectorAll('button')].find(b => b.textContent === 'Simpan jadwal').click(); return 'ok'; })()").await;
    assert_eq!(err, "\"ok\"");
    see(&client, "path lengkap").await;
    let at = eval(&client, "(() => { const e = [...document.querySelectorAll('.fld-err')].find(x => !x.hidden); const i = e.closest('.fld').querySelector('input'); return e.closest('.fld').querySelector('.l').textContent + ':' + i.getAttribute('aria-invalid') + ':' + (document.activeElement === i); })()").await;
    assert_eq!(at, "\"Test:true:true\"");

    // Fixed and saved: focus on the new schedule; Indonesian summary text in the list.
    eval(&client, &format!("(() => {{ const f = document.querySelector('.sched-form'); f.querySelectorAll('input[type=text]')[1].value = '{ROOT}/tests/login.md'; [...f.querySelectorAll('button')].find(b => b.textContent === 'Simpan jadwal').click(); return 'ok'; }})()")).await;
    see(&client, "Setiap hari 08:00").await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(eval(&client, "document.activeElement.className + ':' + document.activeElement.textContent").await, "\"row-name:Login pagi\"");
    assert!(home.join("LaunchAgents/dev.fastpilot.schedule.login-pagi.plist").exists());
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(home);
}
