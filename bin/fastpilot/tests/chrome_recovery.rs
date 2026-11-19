//! Wave 3, QA #5: when fastpilot's own Chrome dies (crash, killed, closed), its sessions are
//! dropped with a message that says how to go on, and the next start launches a new Chrome,
//! for the shell CLI and for MCP alike. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| Path::new(p).exists())
}

/// Kill the Chrome that process `owner` launched (its profile is `fastpilot-<owner>-<nanos>`).
fn kill_chrome_of(owner: u32) {
    let out = Command::new("pgrep").args(["-f", &format!("user-data-dir=.*fastpilot-{owner}-")]).output().expect("pgrep");
    let pids: Vec<String> = String::from_utf8_lossy(&out.stdout).split_whitespace().map(str::to_owned).collect();
    assert!(!pids.is_empty(), "no Chrome of {owner} found");
    for p in pids {
        let _ = Command::new("kill").args(["-9", &p]).status();
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
}

struct Shell {
    dir: PathBuf,
}

impl Shell {
    fn run(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .args(args)
            .current_dir(ROOT)
            .env("FASTPILOT_SOCKET", self.dir.join("d.sock"))
            .env("FASTPILOT_DIR", self.dir.join("fp"))
            .env("HOME", &self.dir)
            .env_remove("FASTPILOT_SESSION")
            .output()
            .expect("run");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.run(&["daemon", "stop"]);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn shell_recovers_after_chrome_dies() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let sh = Shell { dir: std::env::temp_dir().join(format!("fprc-{}", std::process::id())) };
    std::fs::create_dir_all(&sh.dir).expect("dir");
    let page = "examples/demo/login.html";
    let (ok, out) = sh.run(&["open", page]);
    assert!(ok, "{out}");
    let (_, status) = sh.run(&["daemon", "status"]);
    let pid: u32 = status.split("pid ").nth(1).and_then(|r| r.split(')').next()).and_then(|p| p.parse().ok()).expect("daemon pid");
    kill_chrome_of(pid);

    // The next command says what happened and what to do.
    let (ok, out) = sh.run(&["snapshot"]);
    assert!(!ok && out.contains("ended") && out.contains("fastpilot open"), "{out}");
    let (ok, out) = sh.run(&["sessions"]);
    assert!(ok && !out.contains("default"), "the dead session is gone: {out}");
    // And `open` starts over with a new Chrome.
    let (ok, out) = sh.run(&["open", page]);
    assert!(ok && out.contains("session default started"), "{out}");
    let (ok, out) = sh.run(&["get", "title"]);
    assert!(ok && out.contains("Login"), "{out}");
}

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

#[tokio::test]
async fn mcp_recovers_after_chrome_dies() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let dir = std::env::temp_dir().join(format!("fprc-mcp-{}", std::process::id()));
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_DIR", &dir).env("HOME", std::env::temp_dir());
    });
    let child = TokioChildProcess::new(cmd).expect("spawn");
    let pid = child.id().expect("pid");
    let client: Client = ().serve(child).await.expect("initialize");
    let flow = "- session: { start: w, platform: web, target: \"examples/demo/login.html\" }\n- assertVisible: \"Masuk ke Akun\"\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    kill_chrome_of(pid);

    // A step on the dead session: the note says it ended and how to start again.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: \"Masuk ke Akun\"\n", "session": "w" })).await;
    assert!(!ok && out.contains("session `w` ended") && out.contains("session start"), "{out}");
    // The same name can be started again, on a new Chrome.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(dir);
}
