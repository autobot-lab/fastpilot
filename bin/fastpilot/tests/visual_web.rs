//! Visual regression on real Chrome over MCP (`assertScreenshot`): baseline on the first run,
//! a match on the same page, a failure with a diff image after a visible change, `ignore` for a
//! dynamic part, and `update` to accept a change. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}

#[tokio::test]
async fn screenshots_against_a_baseline() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-visual-web-{}", std::process::id()));
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_DIR", &dir);
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let client: Client = client;
    let start = "- session: { start: v, platform: web, target: examples/demo/login.html }\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("{start}- assertScreenshot: login\n") })).await;
    assert!(ok && out.contains("baseline stored"), "{out}");
    assert!(dir.join("baselines/login.png").exists());

    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertScreenshot: login\n" })).await;
    assert!(ok && out.contains("matches baseline"), "{out}");

    // A visible change that keeps the layout: the heading gets a red background.
    let change = "- evalScript: \"document.querySelector('h1').style.background = 'red'; true\"\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("{change}- assertScreenshot: login\n") })).await;
    println!("{out}");
    assert!(!ok && out.contains("differs from baseline login") && out.contains("login.diff.png"), "{out}");
    let diff = std::fs::read(dir.join("baselines/login.diff.png")).expect("diff image");
    assert_eq!(&diff[1..4], b"PNG");

    // The changed part ignored: the rest still matches.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertScreenshot: { name: login, ignore: [\"Masuk ke Akun\"] }\n" })).await;
    assert!(ok && out.contains("matches baseline"), "{out}");
    assert!(!dir.join("baselines/login.diff.png").exists(), "stale diff left behind");

    // Accepting the change.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertScreenshot: { name: login, update: true }\n- assertScreenshot: login\n" })).await;
    assert!(ok && out.contains("baseline stored") && out.contains("matches baseline"), "{out}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(dir);
}
