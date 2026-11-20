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

/// `fastpilot demo` in a pseudo-terminal (`script`, macOS), answering the dashboard question with
/// `answer`; stops once `until` shows up (or after 60 s) and returns what was printed.
fn demo_in_terminal(cwd: &Path, answer: &str, until: &str) -> String {
    use std::io::{Read, Write};
    let mut c = std::process::Command::new("script");
    c.args(["-q", "/dev/null", env!("CARGO_BIN_EXE_fastpilot"), "demo"])
        .current_dir(cwd)
        .env("HOME", cwd)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_ADB_SERVER", "127.0.0.1:1")
        .env("FASTPILOT_NO_BROWSER", "1")
        .env("FASTPILOT_LIVE_DIR", cwd.join("live"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    let mut child = c.spawn().expect("script");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(String::from_utf8_lossy(&buf[..n]).into_owned()).is_err() {
                break;
            }
        }
    });
    let (mut out, mut answered) = (String::new(), false);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < end && !out.contains(until) {
        if let Ok(chunk) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
            out.push_str(&chunk.replace('\r', ""));
        }
        if !answered && out.contains("[Y/n]") {
            let _ = stdin.write_all(answer.as_bytes());
            answered = true;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    // The dashboard started under `script` may outlive it.
    let _ = std::process::Command::new("pkill").args(["-f", &format!("{} ui --no-open", env!("CARGO_BIN_EXE_fastpilot"))]).status();
    out
}

/// D3 (v9 G7): the demo ends by offering the dashboard for the demo folder. Outside a terminal, a
/// line says how; in a terminal, "n" leaves it for later and Enter starts it there.
#[test]
fn demo_offers_the_dashboard() {
    let tmp = std::env::temp_dir().join(format!("fp-demo-d3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("tmp");
    let (code, out) = demo(&tmp, &["first"], false);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("Dashboard: run `cd first && fastpilot ui` and keep this window open.") && !out.contains("[Y/n]"), "{out}");
    if cfg!(target_os = "macos") {
        let out = demo_in_terminal(&tmp, "n\n", "Later:");
        assert!(out.contains("Open the dashboard now to chat, schedule and see results?") && out.contains("Later: run `cd fastpilot-demo && fastpilot ui`"), "{out}");
        let _ = std::fs::remove_dir_all(tmp.join("fastpilot-demo"));
        let out = demo_in_terminal(&tmp, "\n", "?code=");
        assert!(out.contains("Starting the dashboard") && out.contains("http://127.0.0.1:") && out.contains("?code="), "{out}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
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
        "Creating the demo folder: ./fastpilot-demo",
        "tests/login.md",
        "tests/login-android.md",
        "tests/login-cross-platform.md",
        "fastpilot + DEMO_PASS",
        "Android (skipped: no device)",
        "fastpilot demo --android",
        "cd fastpilot-demo && claude",
        "\"Use fastpilot to run tests/login.md, then report\"",
        "Demo password (not a secret): rahasia123",
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
    // QA Low 4: the Skill is in the demo folder, so Claude knows the steps.
    let skill = std::fs::read(format!("{ROOT}/skills/fastpilot/SKILL.md")).expect("repo skill");
    assert_eq!(std::fs::read(folder.join(".claude/skills/fastpilot/SKILL.md")).expect("skill"), skill);
    assert!(out.contains("21 skills"), "{out}");
    // S5 (v0.4): the whole skill library is in the demo folder too.
    let regression = std::fs::read(format!("{ROOT}/skills/regression/SKILL.md")).expect("repo regression");
    assert_eq!(std::fs::read(folder.join(".claude/skills/regression/SKILL.md")).expect("regression"), regression);

    // Already there: nothing changes, exit ≠ 0; --force writes it again.
    let before = snapshot(&folder);
    std::fs::write(folder.join("tests/login.md"), "mine").expect("edit");
    let edited = snapshot(&folder);
    let (code, out) = demo(&tmp, &[], false);
    assert!(code != Some(0) && out.contains("already exists"), "{code:?} {out}");
    assert_eq!(snapshot(&folder), edited, "nothing overwritten");
    let (code, _) = demo(&tmp, &["--force"], false);
    assert_eq!(code, Some(0));
    assert_eq!(snapshot(&folder), before, "--force writes the demo again");
    // --android without a device: skipped, exit 0.
    let (code, out) = demo(&tmp, &["--android"], false);
    assert!(code == Some(0) && out.contains("skipped: no device"), "{code:?} {out}");
    // QA Low 5: run from inside the demo folder: warned that the new one is nested.
    let (code, out) = demo(&folder, &[], false);
    assert!(code == Some(0) && out.contains("inside a demo folder") && out.contains("cd .."), "{code:?} {out}");
    let _ = std::fs::remove_dir_all(folder.join("fastpilot-demo"));
    let (_, out) = demo(&tmp, &["elsewhere"], false);
    assert!(!out.contains("di dalam folder demo"), "no warning outside a demo folder: {out}");

    // Security Low 2: --force keeps the user's own .mcp.json (other servers stay, a backup is made).
    let mine = "{\n  \"mcpServers\": {\n    \"other\": { \"command\": \"other-mcp\" }\n  }\n}\n";
    std::fs::write(folder.join(".mcp.json"), mine).expect("mine");
    let (code, out) = demo(&tmp, &["--force"], false);
    assert!(code == Some(0) && out.contains("was saved as .mcp.json.bak"), "{code:?} {out}");
    assert_eq!(std::fs::read_to_string(folder.join(".mcp.json.bak")).expect("backup"), mine);
    let merged: Value = serde_json::from_str(&std::fs::read_to_string(folder.join(".mcp.json")).expect("merged")).expect("json");
    assert_eq!(merged["mcpServers"]["other"]["command"], "other-mcp", "the other server stays");
    assert_eq!(merged["mcpServers"]["fastpilot"]["env"]["FASTPILOT_VARS"], "DEMO_PASS");
    let (_, out) = demo(&tmp, &["--force"], false);
    assert!(!out.contains(".mcp.json.bak") && !folder.join(".mcp.json.bak-2").exists(), "nothing to save the second time: {out}");
    std::fs::remove_file(folder.join(".mcp.json.bak")).expect("rm");

    // Security Low 1: never written through a symlink (a folder inside, or the demo folder itself).
    #[cfg(unix)]
    {
        let outside = tmp.join("outside");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::rename(folder.join("tests"), tmp.join("tests-real")).expect("move");
        std::os::unix::fs::symlink(&outside, folder.join("tests")).expect("link");
        let (code, out) = demo(&tmp, &["--force"], false);
        assert!(code != Some(0) && out.contains("symlink"), "{code:?} {out}");
        assert_eq!(std::fs::read_dir(&outside).expect("outside").count(), 0, "nothing written outside");
        std::fs::remove_file(folder.join("tests")).expect("unlink");
        std::fs::rename(tmp.join("tests-real"), folder.join("tests")).expect("back");
        std::os::unix::fs::symlink(&outside, tmp.join("linked-demo")).expect("link");
        let (code, out) = demo(&tmp, &["linked-demo", "--force"], false);
        assert!(code != Some(0) && out.contains("symlink"), "{code:?} {out}");
        assert_eq!(std::fs::read_dir(&outside).expect("outside").count(), 0);
        // T1-T3 (Security Low 3): a symlinked .mcp.json is refused before it is read, so its
        // target (here a "private" file) is never copied into .mcp.json.bak.
        let private = tmp.join("private.txt");
        std::fs::write(&private, "PRIVATE-CONTENT").expect("private");
        std::fs::remove_file(folder.join(".mcp.json")).expect("rm mcp");
        std::os::unix::fs::symlink(&private, folder.join(".mcp.json")).expect("link mcp");
        let (code, out) = demo(&tmp, &["--force"], false);
        assert!(code != Some(0) && out.contains(".mcp.json is a symlink"), "{code:?} {out}");
        let copied = std::fs::read_dir(&folder).expect("folder").flatten().any(|e| std::fs::read_to_string(e.path()).is_ok_and(|t| t.contains("PRIVATE-CONTENT")) && e.file_name() != ".mcp.json");
        assert!(!copied, "the link's target was copied into the demo folder");
        assert_eq!(std::fs::read_to_string(&private).expect("private"), "PRIVATE-CONTENT", "the target is untouched");
        std::fs::remove_file(folder.join(".mcp.json")).expect("unlink mcp");
        let (code, out) = demo(&tmp, &["--force"], false);
        assert_eq!(code, Some(0), "the demo folder is whole again: {out}");
    }

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

#[path = "support/android_device.rs"]
mod android_device;

/// `--android` on the emulator named in FASTPILOT_ANDROID_SERIAL. No device fails, saying why
/// (FASTPILOT_SKIP_ANDROID=1 skips on purpose).
#[tokio::test(flavor = "multi_thread")]
async fn demo_android_installs_the_app_and_its_login_runs() {
    let Some(serial) = android_device::device() else { return };
    android_device::need_file(&format!("{ROOT}/examples/demo-android/out/fastpilot-demo.apk"), "the demo app");
    let adb = android_device::adb();
    // Start without the app, so the install is the demo's.
    let _ = std::process::Command::new(&adb).args(["-s", &serial, "uninstall", "com.fastpilot.demo"]).output();
    let tmp = std::env::temp_dir().join(format!("fp-demo-and-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("tmp");

    // QA Low 3: a demo app signed with another key is installed: the message says how to fix it.
    let sdk = std::env::var("ANDROID_HOME").unwrap_or_else(|_| format!("{}/Library/Android/sdk", std::env::var("HOME").unwrap_or_default()));
    let signer = std::fs::read_dir(format!("{sdk}/build-tools")).ok().and_then(|d| d.flatten().map(|e| e.path().join("apksigner")).filter(|p| p.exists()).max());
    if let Some(signer) = signer {
        let key = tmp.join("other.jks");
        let apk = tmp.join("other.apk");
        std::fs::copy(format!("{ROOT}/examples/demo-android/out/fastpilot-demo.apk"), &apk).expect("copy");
        let made = std::process::Command::new("keytool")
            .args(["-genkeypair", "-keystore"]).arg(&key)
            .args(["-storepass", "android", "-keypass", "android", "-alias", "other", "-keyalg", "RSA", "-keysize", "2048", "-validity", "1", "-dname", "CN=other"])
            .output()
            .is_ok_and(|o| o.status.success());
        let signed = made
            && std::process::Command::new(&signer).args(["sign", "--ks"]).arg(&key).args(["--ks-pass", "pass:android"]).arg(&apk).output().is_ok_and(|o| o.status.success());
        if signed && std::process::Command::new(&adb).args(["-s", &serial, "install", "-r"]).arg(&apk).output().is_ok_and(|o| o.status.success()) {
            let (code, out) = demo(&tmp, &["other-demo", "--android", "--device", &serial], true);
            assert_eq!(code, Some(1), "{out}");
            assert!(out.contains(&format!("adb -s {serial} uninstall com.fastpilot.demo")) && !out.contains("INSTALL_FAILED"), "{out}");
        } else {
            eprintln!("note: could not install a differently signed demo app; the hint is covered by the unit test");
        }
        let _ = std::process::Command::new(&adb).args(["-s", &serial, "uninstall", "com.fastpilot.demo"]).output();
    }
    let (code, out) = demo(&tmp, &["--android", "--device", &serial], true);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains(&format!("✓ {serial} · com.fastpilot.demo installed")) && out.contains("tests/login-android.md"), "{out}");
    let listed = std::process::Command::new(&adb).args(["-s", &serial, "shell", "pm", "list", "packages", "com.fastpilot.demo"]).output().expect("adb");
    assert!(String::from_utf8_lossy(&listed.stdout).contains("com.fastpilot.demo"), "installed");

    let folder = tmp.join("fastpilot-demo");
    let client = mcp_from(&folder, &[("FASTPILOT_ANDROID_SERIAL", serial.as_str())]).await;
    // tests/login-android.md, its login part.
    let flow = "- session: { start: and1, platform: android }\n- launchApp: com.fastpilot.demo/.LoginActivity\n- inputText: { into: Email, text: a@b.com }\n- inputText: { into: Password, text: \"${DEMO_PASS}\" }\n- tapOn: Masuk\n- assertVisible: \"Selamat datang, Budi\"\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    // Its last step (Design M2, F5): "Bayar" is refused by the built-in limits; the app stays put.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- tapOn: Bayar\n", "session": "and1" })).await;
    assert!(!ok && out.contains("contains \"bayar\" (block_actions, built-in list)"), "{out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: \"Selamat datang, Budi\"\n- assertNotVisible: \"Total Rp 150.000\"\n", "session": "and1" })).await;
    assert!(ok, "still on the dashboard: {out}");
    let md = std::fs::read_to_string(folder.join("tests/login-android.md")).expect("login-android.md");
    assert!(md.contains("must block that step") && !md.contains("Bayar sekarang"), "{md}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(&tmp);
}
