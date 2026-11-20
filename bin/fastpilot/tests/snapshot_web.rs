//! Snapshot regressions on real Chrome over MCP. Skips without Chrome.
//! Issue #1: plain-text keypad keys must stay separate, tappable nodes.

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
async fn pin_pad_keys_are_separate_nodes() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-snapshot-web-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_DIR", dir.join(".fastpilot"));
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let client: Client = client;
    let flow = r#"
- session: { start: w, platform: web, target: examples/demo/pin.html }
- tapOn: "1"
- tapOn: "5"
- assertVisible: "PIN: 15"
- assertVisible: "4.8 (11)"
- assertVisible: "Toko Budi"
"#;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok, "{out}");
    let (ok, snap) = call(&client, "snapshot", json!({ "session": "w", "full": true })).await;
    assert!(ok, "{snap}");
    assert!(!snap.contains("PIN: 15 1 2"), "label merged with the keys: {snap}");
    for k in ["\"1\"", "\"2\"", "\"0\""] {
        assert!(snap.contains(&format!("text {k}")), "key {k} is not its own node: {snap}");
    }
    assert!(!snap.contains("Rating 4.8"), "{snap}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(dir);
}

/// Security finding (wave 2 review of PR #85, pre-existing): `snapshot file: true` built the file
/// name from the session id, which nothing checked, so a `../` id wrote page text outside
/// `.fastpilot/snapshots`. A session id is now a plain name, and the file name is safe anyway.
#[tokio::test]
async fn a_session_id_never_names_a_path() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let dir = std::env::temp_dir().join(format!("fp-snap-id-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("work")).expect("dir");
    let d2 = dir.clone();
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_DIR", d2.join("work/.fastpilot"));
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let page = format!("file://{ROOT}/examples/demo/login.html");
    for bad in ["../../escape", "a/b", "..", "x\ny", ""] {
        let (ok, out) = call(&client, "session", json!({ "action": "start", "id": bad, "platform": "web", "target": page })).await;
        if bad.is_empty() {
            // No id: one is chosen (web1), as before.
            assert!(ok, "{out}");
            let _ = call(&client, "session", json!({ "action": "stop", "id": "web1" })).await;
            continue;
        }
        assert!(!ok && out.contains("session id"), "{bad:?}: {out}");
    }
    let (ok, out) = call(&client, "session", json!({ "action": "start", "id": "shop-1_a", "platform": "web", "target": page })).await;
    assert!(ok, "a plain name works: {out}");
    let (ok, out) = call(&client, "snapshot", json!({ "session": "shop-1_a", "file": true })).await;
    assert!(ok && out.contains("snapshots"), "{out}");
    let written: Vec<_> = std::fs::read_dir(dir.join("work/.fastpilot/snapshots")).expect("snapshots").flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(written.len() == 1 && written[0].starts_with("shop-1_a-"), "{written:?}");
    assert!(!dir.join("escape-").exists() && std::fs::read_dir(&dir).expect("dir").flatten().all(|e| !e.file_name().to_string_lossy().starts_with("escape")), "nothing outside");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(&dir);
}
