//! FastTest P0 repros on real Chrome: guard rails are pinned when the MCP server or the shell
//! daemon starts, so a config file loosened before a rail's first use does not apply to the
//! running process. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| Path::new(p).exists())
}

/// A page with a risky button; clicking it shows "terhapus".
fn serve() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let body = "<!doctype html><title>Akun</title><h1>Akun</h1><p id=s>aktif</p>\
                        <button onclick=\"document.getElementById('s').textContent='terhapus'\">Hapus akun</button>";
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = s.write_all(resp.as_bytes());
        }
    });
    port
}

fn temp(name: &str) -> PathBuf {
    // Short: the daemon's Unix socket path lives here.
    let dir = std::env::temp_dir().join(format!("fpgr-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

/// MCP server started with `allowed_domains = localhost`; the file is emptied before any web step.
#[tokio::test]
async fn mcp_keeps_allowed_domains_from_its_start() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let port = serve();
    let dir = temp("mcp");
    let file = dir.join("config.toml");
    std::fs::write(&file, "allowed_domains = [\"localhost\"]\n").expect("write");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp")
            .current_dir(ROOT)
            .env("FASTPILOT_CONFIG", &file)
            .env("FASTPILOT_DIR", dir.join("fp"))
            .env_remove("FASTPILOT_ALLOWED_DOMAINS")
            .env_remove("FASTPILOT_CONFIRM_ACTIONS");
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let client: Client = client;
    // Give the server a moment to have started, then loosen the file behind its back.
    let (ok, out) = call(&client, "docs", json!({ "topic": "errors" })).await;
    assert!(ok, "{out}");
    std::fs::write(&file, "").expect("empty");

    let flow = format!("- session: {{ start: w, platform: web, target: \"http://localhost:{port}/\" }}\n- assertVisible: aktif\n");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "localhost is allowed: {out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- openUrl: \"http://127.0.0.1:{port}/\"\n") })).await;
    assert!(!ok && out.contains("blocked"), "loosened before first use, must stay blocked: {out}");
    client.cancel().await.expect("shutdown");

    // A new server takes the (now empty) file.
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_CONFIG", &file).env("FASTPILOT_DIR", dir.join("fp")).env_remove("FASTPILOT_ALLOWED_DOMAINS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("restart");
    let flow = format!("- session: {{ start: w, platform: web, target: \"http://127.0.0.1:{port}/\" }}\n- assertVisible: aktif\n");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "after a restart the looser file applies: {out}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(dir);
}

/// Shell daemon started with `confirm_actions = default`; unset before the first risky tap.
#[test]
fn daemon_keeps_confirm_actions_from_its_start() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let port = serve();
    let dir = temp("cli");
    let file = dir.join("config.toml");
    // The built-in block list (v0.3.2) would refuse "Hapus akun" outright; this test is about confirm.
    std::fs::write(&file, "confirm_actions = \"default\"\nblock_actions = []\n").expect("write");
    let run = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .args(args)
            .current_dir(ROOT)
            .env("FASTPILOT_SOCKET", dir.join("d.sock"))
            .env("FASTPILOT_CONFIG", &file)
            .env("FASTPILOT_DIR", dir.join("fp"))
            .env_remove("FASTPILOT_SESSION")
            .env_remove("FASTPILOT_CONFIRM_ACTIONS")
            .env_remove("FASTPILOT_ALLOWED_DOMAINS")
            .output()
            .expect("run");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    let (ok, out) = run(&["open", &format!("http://127.0.0.1:{port}/")]);
    assert!(ok && out.contains("Hapus akun"), "{out}");
    let (ok, out) = run(&["config", "unset", "confirm_actions"]);
    assert!(ok, "{out}");
    let (ok, out) = run(&["click", "Hapus akun"]);
    assert!(!ok && out.contains("blocked"), "unset before the first tap, must stay blocked: {out}");
    let (_, out) = run(&["is", "visible", "terhapus"]);
    assert_eq!(out.trim(), "false", "the button must not have been pressed");
    // Confirmed on purpose: allowed.
    let (ok, out) = run(&["click", "Hapus akun", "--confirm"]);
    assert!(ok, "{out}");
    let _ = run(&["daemon", "stop"]);
    let _ = std::fs::remove_dir_all(dir);
}
