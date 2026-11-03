//! Maestro-compatible steps on real Chrome over MCP: longPressOn, doubleTapOn, eraseText,
//! copyTextFrom/pasteText, hideKeyboard, runFlow (file + when), repeat, retry,
//! scrollUntilVisible, clearState, takeScreenshot, inputRandom*. Skips without Chrome.

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

#[tokio::test]
async fn maestro_steps_on_web() {
    let dir = std::env::temp_dir().join(format!("fastpilot-maestro-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let sub = dir.join("visit.yaml");
    std::fs::write(&sub, "appId: web\n---\n- openUrl: examples/demo/maestro.html\n").expect("sub-flow");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_DIR", dir.join(".fastpilot"));
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let client: Client = client;
    let flow = format!(
        r#"
- session: {{ start: m, target: examples/demo/maestro.html }}
- startRecording: maestro-rec
- runFlow: {{ when: {{ platform: android }}, commands: [ tapOn: "Tidak ada" ] }}
- longPressOn: "Tahan"
- assertVisible: "Ditahan"
- doubleTapOn: "Ketuk dua kali"
- assertVisible: "Dua kali"
- inputText: {{ into: "Kode", text: "ABCDEF" }}
- eraseText: 2
- copyTextFrom: {{ id: kode }}
- tapOn: "Salinan"
- pasteText
- evalScript: "document.getElementById('salin').value"
- eraseText
- evalScript: "document.getElementById('salin').value.length"
- hideKeyboard
- evalScript: "document.activeElement.tagName"
- runFlow: {{ when: {{ visible: "Kode promo" }}, commands: [ copyTextFrom: "Kode promo" ] }}
- scrollUntilVisible: {{ element: "Akhir halaman", direction: down }}
- evalScript: "window.scrollY > 1000"
- repeat: {{ times: 2, commands: [ runFlow: "{sub}" ] }}
- assertVisible: "Kunjungan 3"
- clearState
- openUrl: examples/demo/maestro.html
- retry: {{ maxRetries: 2, commands: [ assertVisible: {{ text: "Kunjungan 1", timeout: 500 }} ] }}
- tapOn: "Salinan"
- inputRandomEmail
- takeScreenshot: maestro-web
- stopRecording
"#,
        sub = sub.display()
    );
    let (ok, out) = call(&client, "check_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok, "{out}");
    for want in [
        "= skipped (platform android is false)",
        "copyTextFrom {id: kode}",
        "= \"ABCD\"",
        "= 0",
        "= \"BODY\"",
        "= true",
        "= 2 iterations",
        "maestro-rec.html",
        "= passed on attempt 1",
        "@example.com",
        "maestro-web.jpg",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
    assert!(!out.contains("visible \"Kode promo\" is false"), "{out}");
    assert!(dir.join(".fastpilot/screenshots/maestro-web.jpg").exists());
    // Web recording = CDP screencast frames in a self-contained filmstrip player.
    let rec = std::fs::read_to_string(dir.join(".fastpilot/recordings/maestro-rec.html")).expect("recording");
    assert!(rec.matches("data:image/jpeg;base64,").count() >= 2, "too few frames");
    let (ok, out) = call(&client, "check_flow", json!({ "flow": "- runFlow: { when: { true: \"${A}\" }, commands: [ back ] }" })).await;
    assert!(!ok && out.contains("not supported"), "{out}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(dir);
}
