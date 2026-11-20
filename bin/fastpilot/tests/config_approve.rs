//! D9 in the terminal (Security test 10, and 8 end to end): the AI's `config` tool can only file a
//! request to loosen a limit, never approve it; `fastpilot config approve` works only for a person
//! at a terminal, treats the request file as untrusted, and logs what happened without secrets.
//! The person is played through a pseudo-terminal (`script`, macOS).

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

type Client = RunningService<RoleClient, ()>;
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const CANARY: &str = "canary-gh-value-5512";

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists())
}

fn env(c: &mut std::process::Command, home: &Path) {
    c.env("HOME", home).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS").env_remove("FASTPILOT_VARS").env("FASTPILOT_LIVE_DIR", home.join("live")).env("GITHUB_TOKEN", CANARY);
}

async fn mcp(home: &Path) -> Client {
    let h = home.to_path_buf();
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(ROOT).env("HOME", &h).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS").env_remove("FASTPILOT_VARS").env("FASTPILOT_LIVE_DIR", h.join("live")).env("GITHUB_TOKEN", CANARY);
    });
    ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize")
}

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args.as_object().cloned().unwrap_or_default())).await.expect("call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

/// `fastpilot config approve` with stdin a pipe (a script or the AI).
fn approve_piped(home: &Path, input: &str) -> (i32, String) {
    use std::io::Write;
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    c.args(["config", "approve"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    env(&mut c, home);
    let mut child = c.spawn().expect("approve");
    let _ = child.stdin.take().expect("stdin").write_all(input.as_bytes());
    let out = child.wait_with_output().expect("wait");
    (out.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// `fastpilot config approve` in a pseudo-terminal, typing `input` (a person).
fn approve_typed(home: &Path, input: &str) -> String {
    use std::io::Write;
    let mut c = std::process::Command::new("script");
    c.args(["-q", "/dev/null", env!("CARGO_BIN_EXE_fastpilot"), "config", "approve"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    env(&mut c, home);
    let mut child = c.spawn().expect("script");
    let mut stdin = child.stdin.take().expect("stdin");
    let input = input.to_owned();
    let feeder = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(800));
        let _ = stdin.write_all(input.as_bytes());
        std::thread::sleep(std::time::Duration::from_millis(800));
    });
    let out = child.wait_with_output().expect("wait");
    let _ = feeder.join();
    String::from_utf8_lossy(&out.stdout).replace('\r', "")
}

fn config_text(home: &Path) -> String {
    std::fs::read_to_string(home.join(".fastpilot/config.toml")).unwrap_or_default()
}

fn pending(home: &Path) -> PathBuf {
    home.join(".fastpilot/pending-approvals.json")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ai_files_requests_and_only_a_person_approves_them() {
    if !cfg!(target_os = "macos") {
        return eprintln!("skipping: the pseudo-terminal helper is macOS `script`");
    }
    let home = std::env::temp_dir().join(format!("fp-approve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let client = mcp(&home).await;

    // The AI asks to allow Pay: a request, never "approved", nothing changed.
    let (ok, out) = call(&client, "config", json!({ "action": "set", "key": "block_actions", "value": "default,!pay" })).await;
    assert!(!ok && out.contains("I can't loosen a limit myself") && out.contains("fastpilot config approve") && out.contains("Allow the Pay button"), "{out}");
    assert!(!out.to_lowercase().contains("approved"), "{out}");
    assert!(config_text(&home).is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(pending(&home)).expect("pending").permissions().mode() & 0o777, 0o600);
    }
    // Program paths and endpoints are not even requests.
    let (ok, out) = call(&client, "config", json!({ "action": "set", "key": "provider_base_url", "value": "https://evil.example/v1" })).await;
    assert!(!ok && out.contains("only the user"), "{out}");
    let (ok, _) = call(&client, "config", json!({ "action": "set", "key": "vars", "value": "GITHUB_TOKEN" })).await;
    assert!(!ok);
    let requests: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(pending(&home)).expect("pending")).expect("json");
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(requests.iter().all(|r| r.as_object().is_some_and(|o| o.keys().all(|k| ["id", "key", "before", "after", "created_ms"].contains(&k.as_str())))), "structured only: {requests:?}");

    // 8, before: a flow cannot read the secret yet.
    let chrome = chrome_available();
    if chrome {
        let (ok, out) = call(&client, "run_flow", json!({ "flow": "- session: { start: w, platform: web, target: examples/demo/login.html }\n- inputText: { into: Email, text: \"${GITHUB_TOKEN}\" }\n" })).await;
        assert!(!ok && out.contains("GITHUB_TOKEN") && !out.contains(CANARY), "{out}");
    }
    client.cancel().await.expect("shutdown");

    // A script or the AI (stdin is a pipe): refused, nothing changed.
    let (code, out) = approve_piped(&home, "Pay\nGITHUB_TOKEN\n");
    assert!(code == 1 && out.contains("Needs a person") && out.contains("Nothing has changed"), "{code} {out}");
    assert!(config_text(&home).is_empty());

    // A person: sees fastpilot's own words, types the word for each.
    let out = approve_typed(&home, "Pay\nGITHUB_TOKEN\n");
    assert!(out.contains("2 requests are waiting") && out.contains("Allow the Pay button") && out.contains("LOOSENS A LIMIT"), "{out}");
    assert!(out.contains("by the AI (Claude Code). Did you ask for this?") && out.contains("Type Pay to allow it"), "{out}");
    assert!(out.contains("Let tests use the secret GITHUB_TOKEN") && out.contains("Type GITHUB_TOKEN to allow it") && !out.contains(CANARY), "{out}");
    let cfg = config_text(&home);
    assert!(cfg.contains("block_actions = [\"default\", \"!pay\"]") && cfg.contains("vars = [\"GITHUB_TOKEN\"]"), "{cfg}");
    assert!(!pending(&home).exists() || std::fs::read_to_string(pending(&home)).unwrap_or_default().trim() == "[]");

    // 8, after: a new session reads it (its value stays masked).
    if chrome {
        let client = mcp(&home).await;
        let (ok, out) = call(&client, "run_flow", json!({ "flow": "- session: { start: w, platform: web, target: examples/demo/login.html }\n- inputText: { into: Email, text: \"${GITHUB_TOKEN}\" }\n" })).await;
        assert!(ok && !out.contains(CANARY), "{out}");
        client.cancel().await.expect("shutdown");
    }

    // An untrusted request file: unknown settings, markup, a program path, an expired one, a huge one.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("time").as_millis() as u64;
    let tampered = json!([
        { "id": "ab12", "key": "chrome", "before": null, "after": "/bin/sh", "created_ms": now },
        { "id": "ab13", "key": "no_such", "before": null, "after": "x", "created_ms": now },
        { "id": "<script>", "key": "allow_eval_script", "before": null, "after": "true", "created_ms": now },
        { "id": "ab14", "key": "allow_eval_script", "before": null, "after": "true", "created_ms": now - 11 * 60 * 1000 },
    ]);
    std::fs::write(pending(&home), tampered.to_string()).expect("write");
    let out = approve_typed(&home, "yes\n");
    assert!(out.contains("No requests are waiting"), "{out}");
    std::fs::write(pending(&home), format!("[{}]", "{\"id\":\"ab\",\"key\":\"x\"},".repeat(5000).trim_end_matches(','))).expect("write");
    assert!(approve_typed(&home, "yes\n").contains("No requests are waiting"));
    assert!(!config_text(&home).contains("allow_eval_script"));

    // M14: the log says what happened, never a secret's value or AI text.
    let audit = std::fs::read_to_string(home.join(".fastpilot/audit.jsonl")).expect("audit");
    assert!(audit.contains("\"result\":\"requested\"") && audit.contains("\"result\":\"applied\"") && audit.contains("\"channel\":\"terminal\""), "{audit}");
    assert!(!audit.contains(CANARY));
    let _ = std::fs::remove_dir_all(home);
}
