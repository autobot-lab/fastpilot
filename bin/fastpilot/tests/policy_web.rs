//! Guard rails on real Chrome over MCP: FASTPILOT_ALLOWED_DOMAINS keeps the session on the
//! listed hosts (openUrl, links, popups), FASTPILOT_CONFIRM_ACTIONS refuses a risky tap until
//! the step confirms it. `localhost` and `127.0.0.1` are different hosts of one local server.
//! Skips without Chrome.

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
                let mut buf = [0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let body = match path.as_str() {
                    "/other" => "<!doctype html><title>Lain</title><h1>Halaman lain</h1>".to_owned(),
                    _ => format!(
                        "<!doctype html><title>Utama</title><h1>Beranda</h1>\
                         <a href=\"http://127.0.0.1:{port}/other\">Ke situs luar</a>\
                         <a href=\"http://localhost:{port}/other\">Ke halaman dalam</a>\
                         <button onclick=\"window.open('http://127.0.0.1:{port}/other')\">Popup luar</button>\
                         <button onclick=\"this.textContent = 'Akun dihapus'\">Hapus akun</button>"
                    ),
                };
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    port
}

#[tokio::test]
async fn allowed_domains_and_confirmed_actions() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let port = serve().await;
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_ALLOWED_DOMAINS", "localhost").env("FASTPILOT_CONFIRM_ACTIONS", "default");
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let client: Client = client;
    let home = format!("http://localhost:{port}/");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- session: {{ start: w, platform: web, target: \"{home}\" }}\n- assertVisible: Beranda\n") })).await;
    assert!(ok, "{out}");

    // openUrl to another host: refused before Chrome goes there.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- openUrl: \"http://127.0.0.1:{port}/other\"\n") })).await;
    assert!(!ok && out.contains("blocked: http://127.0.0.1") && out.contains("FASTPILOT_ALLOWED_DOMAINS"), "{out}");

    // A link to another host: the navigation is stopped in the browser.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- tapOn: \"Ke situs luar\"\n- assertVisible: { text: \"Halaman lain\", timeout: 1500 }\n" })).await;
    println!("{out}");
    assert!(!ok && out.contains("blocked: http://127.0.0.1"), "{out}");

    // A popup to another host is closed.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- openUrl: \"{home}\"\n- tapOn: \"Popup luar\"\n- assertVisible: {{ text: \"Halaman lain\", timeout: 1500 }}\n") })).await;
    println!("{out}");
    assert!(!ok && out.contains("popup to http://127.0.0.1") && out.contains("closed"), "{out}");

    // The allowed host still works.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- openUrl: \"{home}\"\n- tapOn: \"Ke halaman dalam\"\n- assertVisible: \"Halaman lain\"\n") })).await;
    assert!(ok, "{out}");

    // A risky tap needs confirm: true.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- openUrl: \"{home}\"\n- tapOn: \"Hapus akun\"\n") })).await;
    assert!(!ok && out.contains("tapOn \"Hapus akun\": blocked:") && out.contains("confirm: true"), "{out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- tapOn: { text: \"Hapus akun\", confirm: true }\n- assertVisible: \"Akun dihapus\"\n" })).await;
    assert!(ok && out.contains("(confirmed)"), "{out}");
    client.cancel().await.expect("shutdown");
}
