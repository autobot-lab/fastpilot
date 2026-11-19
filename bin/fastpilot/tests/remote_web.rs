//! D2: web sessions on a remote Chrome (`FASTPILOT_CHROME_ENDPOINT`), here a separately started
//! headless Chrome behind a TCP relay that stands in for an SSH tunnel. Sessions stay isolated
//! (own BrowserContext) and fastpilot launches no Chrome of its own. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::path::Path;

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn chrome() -> Option<&'static str> {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .into_iter()
        .find(|p| Path::new(p).exists())
}

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

/// Relay 127.0.0.1:<new port> → 127.0.0.1:<target>.
async fn tunnel(target: u16) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut a, _)) = listener.accept().await {
            tokio::spawn(async move {
                if let Ok(mut b) = tokio::net::TcpStream::connect(("127.0.0.1", target)).await {
                    let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
                }
            });
        }
    });
    port
}

struct Remote {
    child: std::process::Child,
    profile: std::path::PathBuf,
}

impl Drop for Remote {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn web_sessions_on_a_remote_chrome() {
    let Some(exe) = chrome() else {
        return eprintln!("skipping: no Chrome");
    };
    let profile = std::env::temp_dir().join(format!("fp-remote-chrome-{}", std::process::id()));
    std::fs::create_dir_all(&profile).expect("profile");
    let child = std::process::Command::new(exe)
        .args(["--headless=new", "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check", "--user-agent=fp-remote-chrome/1"])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("chrome");
    let remote = Remote { child, profile };
    let active = remote.profile.join("DevToolsActivePort");
    let mut port = None;
    for _ in 0..100 {
        if let Some(p) = std::fs::read_to_string(&active).ok().and_then(|t| t.lines().next().and_then(|l| l.trim().parse::<u16>().ok())) {
            port = Some(p);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let port = port.expect("remote Chrome did not start");
    let relay = tunnel(port).await;
    let endpoint = format!("http://127.0.0.1:{relay}");

    // `doctor web` checks the remote one.
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["doctor", "web"])
        .env("FASTPILOT_CHROME_ENDPOINT", &endpoint)
        .output()
        .await
        .expect("doctor");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && text.contains(&format!("remote Chrome at {endpoint}")), "{text}");

    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_CHROME_ENDPOINT", &endpoint).env("FASTPILOT_CHROME", "/nonexistent/chrome");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let page = format!("file://{ROOT}/examples/demo/login.html");
    let flow = format!(
        "- session: {{ start: r1, platform: web, target: \"{page}\" }}\n- evalScript: \"navigator.userAgent\"\n\
         - evalScript: \"document.cookie = 'k=r1'; localStorage.setItem('who', 'r1'); localStorage.getItem('who')\"\n\
         - session: {{ start: r2, platform: web, target: \"{page}\" }}\n- evalScript: \"String(localStorage.getItem('who'))\"\n"
    );
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    // FASTPILOT_CHROME points nowhere: only the remote Chrome can have run this.
    assert!(ok && out.contains("fp-remote-chrome/1"), "{out}");
    assert!(out.contains("= \"r1\"") && out.contains("= \"null\""), "sessions share no storage: {out}");
    client.cancel().await.expect("shutdown");

    // A remote Chrome that is not there: a clear error naming the setting.
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_CHROME_ENDPOINT", "http://127.0.0.1:1");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- session: { start: x, platform: web }\n" })).await;
    assert!(!ok && out.contains("FASTPILOT_CHROME_ENDPOINT"), "{out}");
    client.cancel().await.expect("shutdown");
    drop(remote);
}
