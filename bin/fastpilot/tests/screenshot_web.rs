//! Screenshot scale on real Chrome over MCP (issue #6): after setViewport with a device scale
//! factor, screenshots (plain and Set-of-Mark) are in CSS pixels, the space taps and snapshot
//! boxes use. Skips without Chrome.

use base64::Engine;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String, CallToolResult) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text, res)
}

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}

/// Width and height from a JPEG's start-of-frame segment.
fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            return None;
        }
        let marker = b[i + 1];
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

async fn shot_size(client: &Client, args: Value) -> (u32, u32) {
    let (ok, text, res) = call(client, "screenshot", args).await;
    assert!(ok, "{text}");
    let image = res.content.iter().find_map(|c| c.as_image()).expect("image");
    let bytes = base64::engine::general_purpose::STANDARD.decode(&image.data).expect("base64");
    jpeg_size(&bytes).expect("jpeg size")
}

#[tokio::test]
async fn screenshots_are_in_css_pixels() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT);
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let client: Client = client;
    let flow = "- session: { start: s, platform: web, target: examples/demo/login.html }\n- setViewport: iPhone 15\n- evalScript: \"innerWidth + 'x' + innerHeight + ' dpr=' + devicePixelRatio\"\n";
    let (ok, out, _) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok && out.contains("dpr=3"), "{out}");
    let css = out.split("= \"").nth(1).and_then(|r| r.split(' ').next()).expect("innerWidth x innerHeight").to_owned();
    let (w, h) = shot_size(&client, json!({ "session": "s" })).await;
    assert_eq!(format!("{w}x{h}"), css, "screenshot is in device pixels");
    let (w2, h2) = shot_size(&client, json!({ "session": "s", "annotate": "som" })).await;
    assert_eq!((w2, h2), (w, h), "Set-of-Mark uses the same scale");
    // Desktop (DPR 1) is unchanged.
    let (ok, out, _) = call(&client, "run_flow", json!({ "flow": "- session: { start: d, platform: web, target: examples/demo/login.html }\n" })).await;
    assert!(ok, "{out}");
    assert_eq!(shot_size(&client, json!({ "session": "d" })).await, (1280, 800));
    client.cancel().await.expect("shutdown");
}
