//! F2 `fastpilot demo`: the demo folder is written from the binary alone (pages, test cases,
//! .mcp.json), a first web test runs from it as Claude Code would start fastpilot there, an existing
//! folder is never overwritten, and Android is skipped without a device. `--android` installs the
//! demo app and the Android login runs on it (needs FASTPILOT_ANDROID_SERIAL or one device).

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

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

/// `fastpilot demo <args>` in `cwd`, with no Android device reachable unless `adb` is true.
fn demo(cwd: &Path, args: &[&str], adb: bool) -> (Option<i32>, String) {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    c.arg("demo").args(args).current_dir(cwd).env("HOME", cwd).env_remove("FASTPILOT_CONFIG");
    if !adb {
        // An adb server nobody listens on: "no device".
        c.env("FASTPILOT_ADB_SERVER", "127.0.0.1:1").env_remove("FASTPILOT_ANDROID_SERIAL").env_remove("ANDROID_SERIAL");
    }
    let out = c.output().expect("demo");
    (out.status.code(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).expect("file")));
            }
        }
    }
    out.sort();
    out
}

/// The MCP server as Claude Code starts it from the demo folder's .mcp.json.
async fn mcp_from(folder: &Path, extra: &[(&str, &str)]) -> Client {
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(folder.join(".mcp.json")).expect(".mcp.json")).expect("json");
    let server = cfg["mcpServers"]["fastpilot"].clone();
    assert_eq!(server["args"].as_array().and_then(|a| a.last()).and_then(Value::as_str), Some("mcp"));
    let env: Vec<(String, String)> = server["env"].as_object().expect("env").iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned())).collect();
    let extra: Vec<(String, String)> = extra.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    let (dir, home) = (folder.to_owned(), folder.join(".home"));
    std::fs::create_dir_all(&home).expect("home");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(&dir).env("HOME", &home).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_LIVE_DIR").env_remove("FASTPILOT_BLOCK_ACTIONS");
        for (k, v) in env.iter().chain(extra.iter()) {
            c.env(k, v);
        }
    });
    ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize")
}

#[tokio::test(flavor = "multi_thread")]
async fn demo_folder_runs_a_first_web_test_and_is_never_overwritten() {
    let tmp = std::env::temp_dir().join(format!("fp-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("tmp");
    let (code, out) = demo(&tmp, &[], false);
    assert_eq!(code, Some(0), "{out}");
    for line in [
        "Membuat folder demo: ./fastpilot-demo",
        "tests/login.md",
        "tests/login-android.md",
        "tests/login-cross-platform.md",
        "fastpilot + DEMO_PASS",
        "Android (dilewati: tidak ada device)",
        "fastpilot demo --android",
        "cd fastpilot-demo && claude",
        "\"Pakai fastpilot, jalankan tests/login.md lalu report\"",
        "Password demo (bukan rahasia): rahasia123",
    ] {
        assert!(out.contains(line), "missing {line:?}:\n{out}");
    }
    let folder = tmp.join("fastpilot-demo");
    for page in ["login", "widgets", "pin", "secret", "maestro"] {
        let repo = std::fs::read(format!("{ROOT}/examples/demo/{page}.html")).expect("repo page");
        assert_eq!(std::fs::read(folder.join(format!("demo/{page}.html"))).expect("page"), repo, "{page}");
    }
    let login = std::fs::read_to_string(folder.join("tests/login.md")).expect("test");
    assert!(login.contains("target: demo/login.html"), "{login}");

    // Already there: nothing changes, exit ≠ 0; --force writes it again.
    let before = snapshot(&folder);
    std::fs::write(folder.join("tests/login.md"), "mine").expect("edit");
    let edited = snapshot(&folder);
    let (code, out) = demo(&tmp, &[], false);
    assert!(code != Some(0) && out.contains("sudah ada"), "{code:?} {out}");
    assert_eq!(snapshot(&folder), edited, "nothing overwritten");
    let (code, _) = demo(&tmp, &["--force"], false);
    assert_eq!(code, Some(0));
    assert_eq!(snapshot(&folder), before, "--force writes the demo again");
    // --android without a device: skipped, exit 0.
    let (code, out) = demo(&tmp, &["--android"], false);
    assert!(code == Some(0) && out.contains("dilewati: tidak ada device"), "{code:?} {out}");
    // Another folder name.
    let (code, out) = demo(&tmp, &["coba lagi"], false);
    assert!(code == Some(0) && out.contains("cd \"coba lagi\" && claude") && tmp.join("coba lagi/.mcp.json").exists(), "{out}");

    // The first web test, from the demo folder as Claude Code starts fastpilot there.
    if !chrome_available() {
        return eprintln!("skipping the web run: no Chrome");
    }
    let client = mcp_from(&folder, &[]).await;
    let flow = "- session: { start: web, platform: web, target: demo/login.html }\n- inputText: { into: Email, text: a@b.com }\n- inputText: { into: Password, text: \"${DEMO_PASS}\" }\n- tapOn: Masuk\n- assertVisible: Budi\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok && out.contains("ok 5/5"), "{out}");
    assert!(!out.contains("rahasia123"), "the demo password stays masked: {out}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `--android` on a real device or emulator (FASTPILOT_ANDROID_SERIAL; skipped without one).
#[tokio::test(flavor = "multi_thread")]
async fn demo_android_installs_the_app_and_its_login_runs() {
    let Some(serial) = std::env::var("FASTPILOT_ANDROID_SERIAL").ok().filter(|s| !s.is_empty()) else {
        return eprintln!("skipping: set FASTPILOT_ANDROID_SERIAL to an emulator to run the Android demo");
    };
    let adb = std::env::var("ANDROID_HOME").map(|h| format!("{h}/platform-tools/adb")).unwrap_or_else(|_| format!("{}/Library/Android/sdk/platform-tools/adb", std::env::var("HOME").unwrap_or_default()));
    // Start without the app, so the install is the demo's.
    let _ = std::process::Command::new(&adb).args(["-s", &serial, "uninstall", "com.fastpilot.demo"]).output();
    let tmp = std::env::temp_dir().join(format!("fp-demo-and-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("tmp");
    let (code, out) = demo(&tmp, &["--android", "--device", &serial], true);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains(&format!("✓ {serial} · com.fastpilot.demo terpasang")) && out.contains("tests/login-android.md"), "{out}");
    let listed = std::process::Command::new(&adb).args(["-s", &serial, "shell", "pm", "list", "packages", "com.fastpilot.demo"]).output().expect("adb");
    assert!(String::from_utf8_lossy(&listed.stdout).contains("com.fastpilot.demo"), "installed");

    let folder = tmp.join("fastpilot-demo");
    let client = mcp_from(&folder, &[("FASTPILOT_ANDROID_SERIAL", serial.as_str())]).await;
    // tests/login-android.md, its login part.
    let flow = "- session: { start: and1, platform: android }\n- launchApp: com.fastpilot.demo/.LoginActivity\n- inputText: { into: Email, text: a@b.com }\n- inputText: { into: Password, text: \"${DEMO_PASS}\" }\n- tapOn: Masuk\n- assertVisible: \"Selamat datang, Budi\"\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(&tmp);
}
