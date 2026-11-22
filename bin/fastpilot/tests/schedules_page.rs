//! The Schedules page as a user meets it: the three presets on top (S5: a click says "Created ·
//! Undo", a preset that is on says "On · Turn off"), the empty state, the full form under More
//! options (A1: tests from a list, no path and no name field, a Mac notification by default), a
//! refused save points at its field, Escape closes the panel, "every 2 hours, 08:00–20:00" is created
//! (D5) with its Next time and OS entry, and after saving the focus is on the new schedule. Driven
//! by fastpilot in Chrome; skips without Chrome.

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
async fn schedules_page_creates_every_n_hours_and_points_at_fields() {
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
        c.arg("mcp").env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").current_dir(ROOT).env("HOME", &viewer_home).env("FASTPILOT_LIVE_DIR", viewer_home.join("live")).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let flow = format!("- session: {{ start: viewer, platform: web, target: \"{}\" }}\n", link.trim());
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    eval(&client, "location.hash = '#/schedules', 'ok'").await;
    // Empty state (v7 #s8).
    see(&client, "Nothing is scheduled").await;
    let btn = |label: &str| format!("[...document.querySelectorAll('button')].find(b => b.textContent.trim().endsWith({label:?}) && !b.closest('[hidden]'))");

    // S5: three presets for all of this app's tests; one click, "Created · Undo", then "On · Turn off".
    see(&client, "of this app by themselves").await;
    let cards = eval(&client, "[...document.querySelectorAll('.preset')].map(c => c.querySelector('b').textContent + ':' + c.querySelector('button').textContent).join('|')").await;
    assert_eq!(cards, "\"Every 2 hours (8–20):Turn on|Every night:Turn on|Before each release:Turn on\"");
    assert!(eval(&client, "document.querySelector('[data-preset=release] .s').textContent").await.contains("Can we release?"), "the release trigger is explained");
    // S7: one button before a release, one toggle after it.
    let rel = eval(&client, "(() => { const s = document.querySelector('.sched-release'); return [s.querySelector('h2').textContent, [...s.querySelectorAll('.rel-row b')].map(b => b.textContent).join('+'), s.querySelector('button.p').textContent, s.querySelector('[role=switch]').getAttribute('aria-label') + ':' + s.querySelector('[role=switch]').getAttribute('aria-checked')].join('|'); })()").await;
    assert_eq!(rel, "\"Before and after a release|Check before release+Watch for 2 hours after a release|Check before release|Watch for 2 hours after a release:false\"");
    eval(&client, "document.querySelector('[data-preset=night] button').click(), 'ok'").await;
    see(&client, "Created: every night at 2:00.").await;
    assert_eq!(eval(&client, "document.activeElement.textContent").await, "\"Undo\"", "focus on Undo");
    let night = eval(&client, "(() => { const c = document.querySelector('[data-preset=night]'); return c.querySelector('.pill').textContent + ':' + c.querySelector('button').textContent; })()").await;
    assert_eq!(night, "\"On:Turn off\"");
    see(&client, "All tests · every night").await;
    eval(&client, "document.activeElement.click(), 'ok'").await;
    see(&client, "Nothing is scheduled").await;
    assert_eq!(eval(&client, "document.querySelector('[data-preset=night] button').textContent").await, "\"Turn on\"", "Undo removed it");

    // Escape closes the panel and gives focus back to "New schedule".
    eval(&client, &format!("{}.click(), 'ok'", btn("New schedule"))).await;
    see(&client, "Create schedule").await;
    assert_eq!(eval(&client, "document.activeElement.closest('.sched-form') ? 'in' : 'out'").await, "\"in\"");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- pressKey: Escape\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    assert_eq!(eval(&client, "!document.querySelector('.sched-form') && document.activeElement.textContent.includes('New schedule')").await, "true");

    // The form (A1): Tests lists this app's saved tests by title, nothing to type; no name field;
    // a Mac notification by default; "How often" is a radio group.
    eval(&client, &format!("{}.click(), 'ok'", btn("New schedule"))).await;
    see(&client, "Create schedule").await;
    let opts = eval(&client, "[...document.querySelector('.sched-form select').options].map(o => o.textContent).join('|')").await;
    assert!(opts.contains("tests of this app") && opts.contains("Login works") && !opts.contains("Another folder"), "{opts}");
    let typed = eval(&client, "document.querySelectorAll('.sched-form input[type=text]:not([hidden])').length + ':' + [...document.querySelectorAll('.sched-form .l')].map(l => l.textContent).filter(t => t === 'Name' || t === 'Folder or file').length + ':' + [...document.querySelectorAll('.ck')].find(l => l.textContent.includes('Mac')).querySelector('input').checked").await;
    assert_eq!(typed, "\"1:0:true\"", "only the GitHub repo is a text box (hidden until ticked); no path, no name; Mac notification on");
    let kinds = eval(&client, "[...document.querySelectorAll('.seg [role=radio]')].map(b => b.textContent + ':' + b.getAttribute('aria-checked')).join('|')").await;
    assert_eq!(kinds, "\"Fixed time:true|Every N hours:false|Weekdays:false|Weekly:false|Before each release:false|Release watch:false\"");
    // R1: Release watch shows its minutes and hours and says what it notifies about.
    let watch = eval(&client, "(() => { [...document.querySelectorAll('.seg [role=radio]')].find(b => b.textContent === 'Release watch').click(); const m = document.querySelector('select[aria-label=\"Minutes between runs\"]'); const h = document.querySelector('input[aria-label=\"Hours to watch\"]'); const out = [m && !m.closest('[hidden]'), h && h.value, document.querySelector('.sched-form legend').textContent, document.querySelector('.sched-form input[type=time]').closest('[hidden]') ? 'time hidden' : 'time shown'].join('|'); [...document.querySelectorAll('.seg [role=radio]')].find(b => b.textContent === 'Fixed time').click(); return out; })()").await;
    assert_eq!(watch, "\"true|2|Notify me about a new problem (any you like)|time hidden\"");
    // Webhook field: labelled, shown when its box is ticked.
    let label = eval(&client, "(() => { const box = [...document.querySelectorAll('.ck')].find(l => l.textContent.includes('Slack')).querySelector('input'); const w = document.querySelector('input[type=password]'); const before = w.closest('[hidden]') ? 'hidden' : 'shown'; box.click(); return before + '>' + (w.closest('[hidden]') ? 'hidden' : 'shown') + ':' + w.closest('label.fld').querySelector('.l').textContent; })()").await;
    assert_eq!(label, "\"hidden>shown:Webhook URL\"");
    eval(&client, "([...document.querySelectorAll('.ck')].find(l => l.textContent.includes('Slack')).querySelector('input').click(), 'ok')").await;

    // Every N hours with an inverted window: the reason under the window, the field marked and focused.
    let set = format!("(() => {{ const f = document.querySelector('.sched-form'); f.querySelector('[data-v=hours]').click(); const sel = f.querySelector('select'); sel.value = [...sel.options].find(o => o.textContent === 'Login works').value; sel.dispatchEvent(new Event('change')); f.querySelector('input[type=number]').value = '2'; const [a, b] = f.querySelectorAll('input[aria-label^=Active]'); a.value = '20:00'; b.value = '08:00'; {}.click(); return 'ok'; }})()", btn("Create schedule"));
    eval(&client, &set).await;
    see(&client, "ends before it starts").await;
    let at = eval(&client, "(() => { const i = document.activeElement; return i.getAttribute('aria-label') + ':' + i.getAttribute('aria-invalid') + ':' + document.getElementById(i.getAttribute('aria-describedby')).textContent.slice(0, 10); })()").await;
    assert_eq!(at, "\"Active from:true:The active\"");

    // Fixed and created: the row says when, and Next is one of the 2-hour times.
    eval(&client, &format!("(() => {{ const f = document.querySelector('.sched-form'); const [a, b] = f.querySelectorAll('input[aria-label^=Active]'); a.value = '08:00'; b.value = '20:00'; {}.click(); return 'ok'; }})()", btn("Create schedule"))).await;
    see(&client, "every 2 hours, 08:00–20:00").await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    // A1: the name is made from the test and the time.
    assert_eq!(eval(&client, "document.activeElement.className + ':' + document.activeElement.textContent").await, "\"row-name:Login works · every 2 hours\"");
    let id = eval(&client, "document.activeElement.closest('tr').dataset.id").await.trim_matches('"').to_owned();
    let next = eval(&client, &format!("document.querySelector('tr[data-id={id}] td.next').textContent")).await;
    let next = next.trim_matches('"').to_owned();
    let time = next.rsplit(' ').next().unwrap_or("");
    assert!(["08:00", "10:00", "12:00", "14:00", "16:00", "18:00", "20:00"].contains(&time), "Next: {next}");
    assert_eq!(eval(&client, &format!("document.querySelector('tr[data-id={id}] td.last').textContent")).await, "\"Not run yet\"");
    assert!(eval(&client, &format!("document.querySelector('tr[data-id={id}]').textContent")).await.contains("Mac notification"), "a Mac notification by default");
    let plist = std::fs::read_to_string(home.join(format!("LaunchAgents/dev.fastpilot.schedule.{id}.plist"))).expect("OS entry");
    assert_eq!(plist.matches("<key>Hour</key>").count(), 7, "{plist}");
    let heads = eval(&client, "[...document.querySelectorAll('.tbl th')].map(t => t.textContent).join('|')").await;
    // D10d (design v7 #s3): one table for all agents, the agent first.
    assert_eq!(heads, "\"Agent|Name|When|Next|Last|Notify|On|\"");

    // Design L4: the page says in its own words that fastpilot ui stopped (not "Failed to fetch").
    drop(_ui);
    eval(&client, "location.hash = '#/live', 'ok'").await;
    see(&client, "the fastpilot ui server is not answering").await;
    assert_eq!(eval(&client, "String(document.body.textContent.includes('Failed to fetch'))").await, "\"false\"");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(home);
}
