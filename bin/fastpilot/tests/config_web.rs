//! Settings through the `config` MCP tool on real Chrome: a limit set while a session is open
//! applies to its next step, an agent can tighten guard rails but not loosen them, program paths
//! are for people only, and environment variables win over the file. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

async fn serve() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf).await;
                let body = format!("<!doctype html><title>T</title><h1>Halo</h1><a href=\"http://127.0.0.1:{port}/x\">Luar</a>");
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    port
}

#[tokio::test]
async fn settings_through_the_config_tool() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let port = serve().await;
    let dir = std::env::temp_dir().join(format!("fastpilot-config-web-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("config.toml");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp")
            .current_dir(ROOT)
            .env("FASTPILOT_CONFIG", &file)
            .env("FASTPILOT_FLOW_TIMEOUT", "300")
            .env_remove("FASTPILOT_ALLOWED_DOMAINS")
            .env_remove("FASTPILOT_CONFIRM_ACTIONS");
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let client: Client = client;
    let home = format!("http://localhost:{port}/");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- session: {{ start: w, platform: web, target: \"{home}\" }}\n- tapOn: Luar\n- assertVisible: Halo\n") })).await;
    assert!(ok, "no limit yet: {out}");

    // Tighten while the session is open: the next steps obey.
    let (ok, out) = call(&client, "config", json!({ "action": "set", "key": "allowed_domains", "value": "localhost" })).await;
    assert!(ok && out.contains("allowed_domains = localhost"), "{out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- openUrl: \"http://127.0.0.1:{port}/\"\n") })).await;
    assert!(!ok && out.contains("blocked"), "openUrl to another host: {out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- openUrl: \"{home}\"\n- tapOn: Luar\n- assertVisible: {{ text: Halo, timeout: 1500 }}\n") })).await;
    assert!(!ok && out.contains("blocked: http://127.0.0.1"), "a link in the open session: {out}");

    // Loosening is refused for an agent; tightening further is fine.
    let (ok, out) = call(&client, "config", json!({ "action": "set", "key": "allowed_domains", "value": "localhost, evil.com" })).await;
    assert!(!ok && out.contains("guard rail"), "{out}");
    let (ok, out) = call(&client, "config", json!({ "action": "unset", "key": "allowed_domains" })).await;
    assert!(!ok && out.contains("guard rail"), "{out}");
    let (ok, out) = call(&client, "config", json!({ "action": "set", "key": "confirm_actions", "value": "default" })).await;
    assert!(ok, "{out}");
    // Program paths: people only. Environment wins and is not overwritten.
    let (ok, out) = call(&client, "config", json!({ "action": "set", "key": "chrome", "value": "/bin/sh" })).await;
    assert!(!ok && out.contains("only the user"), "{out}");
    let (ok, out) = call(&client, "config", json!({ "action": "set", "key": "flow_timeout", "value": "60" })).await;
    assert!(!ok && out.contains("set by the environment"), "{out}");
    let (ok, out) = call(&client, "config", json!({ "action": "set", "key": "headful", "value": "false" })).await;
    assert!(ok, "{out}");
    let (ok, out) = call(&client, "config", json!({})).await;
    assert!(ok && out.contains("allowed_domains    localhost (file)") && out.contains("flow_timeout       300 (env"), "{out}");
    let saved = std::fs::read_to_string(&file).expect("file");
    assert!(saved.contains("allowed_domains = [\"localhost\"]") && saved.contains("confirm_actions") && !saved.contains("/bin/sh"), "{saved}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(dir);
}
