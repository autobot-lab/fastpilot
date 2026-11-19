//! C2: the new limits from the settings page are enforced by the engine (real Chrome over MCP),
//! not only shown in the UI: always-blocked buttons (even with `confirm: true`), evalScript off,
//! upload folders, app reset and install off. A looser value written while the server runs
//! applies from the next start. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::Path;

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| Path::new(p).exists())
}

/// A checkout page: a pay button, a confirm-worthy delete button, and a file field.
fn serve() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let body = "<!doctype html><title>Checkout</title><h1>Checkout</h1><p id=s>belum</p>\
                        <button onclick=\"document.getElementById('s').textContent='dibayar'\">Lanjut ke Pembayaran</button>\
                        <button onclick=\"document.getElementById('s').textContent='kurir dipilih'\">Pilih kurir</button>\
                        <label>Lampiran <input type=file aria-label=Lampiran></label>";
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = s.write_all(resp.as_bytes());
        }
    });
    port
}

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

async fn server(file: &Path, dir: &Path) -> Client {
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_CONFIG", file).env("FASTPILOT_DIR", dir.join("fp"));
        for k in ["FASTPILOT_BLOCK_ACTIONS", "FASTPILOT_ALLOW_EVAL_SCRIPT", "FASTPILOT_UPLOAD_DIRS", "FASTPILOT_ALLOW_APP_RESET", "FASTPILOT_ALLOW_APP_INSTALL", "FASTPILOT_CONFIRM_ACTIONS", "FASTPILOT_ALLOWED_DOMAINS"] {
            c.env_remove(k);
        }
    });
    ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize")
}

#[tokio::test(flavor = "multi_thread")]
async fn new_limits_are_enforced_by_the_engine() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let port = serve();
    let dir = std::env::temp_dir().join(format!("fp-limits-{}", std::process::id()));
    let fixtures = dir.join("fixtures");
    std::fs::create_dir_all(&fixtures).expect("dir");
    std::fs::write(fixtures.join("ok.txt"), "ok").expect("fixture");
    std::fs::write(dir.join("private.txt"), "private").expect("private");
    let file = dir.join("config.toml");
    std::fs::write(
        &file,
        format!(
            "block_actions = [\"Bayar\"]\nconfirm_actions = [\"Pilih\"]\nallow_eval_script = false\nallow_app_reset = false\nallow_app_install = false\nupload_dirs = [\"{}\"]\n",
            fixtures.display()
        ),
    )
    .expect("config");
    let client = server(&file, &dir).await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- session: {{ start: w, platform: web, target: \"http://127.0.0.1:{port}/\" }}\n") })).await;
    assert!(ok, "{out}");
    let step = |flow: &str| json!({ "flow": flow, "session": "w" });

    // "Bayar" inside "Lanjut ke Pembayaran": blocked, confirmed or not; the button was not pressed.
    for flow in ["- tapOn: \"Lanjut ke Pembayaran\"\n", "- tapOn: { text: \"Lanjut ke Pembayaran\", confirm: true }\n", "- longPressOn: \"Lanjut ke Pembayaran\"\n"] {
        let (ok, out) = call(&client, "run_flow", step(flow)).await;
        assert!(!ok && out.contains("blocked") && out.contains("block_actions"), "{flow}: {out}");
    }
    let (ok, out) = call(&client, "run_flow", step("- assertVisible: belum\n")).await;
    assert!(ok, "the pay button must not have been pressed: {out}");
    // confirm_actions still works as before next to it.
    let (ok, out) = call(&client, "run_flow", step("- tapOn: \"Pilih kurir\"\n")).await;
    assert!(!ok && out.contains("confirm: true"), "{out}");
    let (ok, out) = call(&client, "run_flow", step("- tapOn: { text: \"Pilih kurir\", confirm: true }\n- assertVisible: \"kurir dipilih\"\n")).await;
    assert!(ok, "{out}");

    // evalScript, clearState and installApp are off.
    for (flow, key) in [("- evalScript: \"1 + 1\"\n", "allow_eval_script"), ("- clearState\n", "allow_app_reset"), ("- installApp: x.apk\n", "allow_app_install")] {
        let (ok, out) = call(&client, "run_flow", step(flow)).await;
        assert!(!ok && out.contains("blocked") && out.contains(key), "{flow}: {out}");
    }

    // Uploads only from the allowed folder (`..` does not leave it).
    let inside = fixtures.join("ok.txt").display().to_string();
    let escape = fixtures.join("../private.txt").display().to_string();
    let (ok, out) = call(&client, "run_flow", step(&format!("- uploadFile: {{ into: Lampiran, files: [\"{escape}\"] }}\n"))).await;
    assert!(!ok && out.contains("upload_dirs"), "{out}");
    let (ok, out) = call(&client, "run_flow", step(&format!("- uploadFile: {{ into: Lampiran, files: [\"{inside}\"] }}\n"))).await;
    assert!(ok, "{out}");

    // The user loosens in the settings (as `fastpilot ui` does): not under this running server...
    std::fs::write(&file, "allow_eval_script = true\n").expect("loosen");
    let (ok, out) = call(&client, "run_flow", step("- evalScript: \"1 + 1\"\n- tapOn: \"Lanjut ke Pembayaran\"\n")).await;
    assert!(!ok && out.contains("allow_eval_script"), "still the rails it started with: {out}");
    client.cancel().await.expect("shutdown");

    // ... but from the next start.
    let client = server(&file, &dir).await;
    let flow = format!("- session: {{ start: w, platform: web, target: \"http://127.0.0.1:{port}/\" }}\n- evalScript: \"1 + 1\"\n- tapOn: \"Lanjut ke Pembayaran\"\n- assertVisible: dibayar\n");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "next run takes the looser settings: {out}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(dir);
}
