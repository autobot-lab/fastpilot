//! E8 + E6 (v0.7.2) end to end: providers, model lists and a model per agent, with fake
//! OpenAI-compatible servers on this computer. A temp HOME, the file key store, no Claude Code (a
//! stand-in that only logs), no Chrome and no device: each fake model calls `finish` at once.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

const FAKE_CLAUDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude/claude");

type Log = Arc<Mutex<Vec<(String, String, Value)>>>;

/// A fake OpenAI-compatible server: `GET /v1/models` lists `models` (404 when `None`), and
/// `POST /v1/chat/completions` answers with a `finish` call. Logs (request line, Authorization, body).
fn fake_server(models: Option<Vec<&'static str>>) -> (u16, Log) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut r = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            let _ = r.read_line(&mut line);
            let (mut len, mut auth) = (0usize, String::new());
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                    break;
                }
                let lower = h.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if lower.starts_with("authorization:") {
                    auth = h.trim().to_owned();
                }
            }
            let mut body = vec![0u8; len];
            let _ = r.read_exact(&mut body);
            let req: Value = serde_json::from_slice(&body).unwrap_or_default();
            log2.lock().expect("log").push((line.trim().to_owned(), auth, req.clone()));
            let (status, reply) = if line.starts_with("GET /v1/models") {
                match &models {
                    Some(m) => ("200 OK", json!({ "object": "list", "data": m.iter().map(|id| json!({ "id": id, "object": "model" })).collect::<Vec<_>>() })),
                    None => ("404 Not Found", json!({ "error": { "message": "not found" } })),
                }
            } else {
                let model = req["model"].as_str().unwrap_or("").to_owned();
                let args = json!({ "status": "pass", "notes": format!("checked by {model}") }).to_string();
                ("200 OK", json!({ "choices": [{ "message": { "role": "assistant", "content": null, "tool_calls": [{ "id": "call_finish", "type": "function", "function": { "name": "finish", "arguments": args } }] } }], "usage": { "prompt_tokens": 5, "completion_tokens": 1 } }))
            };
            let reply = reply.to_string();
            let mut s = stream;
            let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
        }
    });
    (port, log)
}

/// A port nobody listens on.
fn closed_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port()
}

struct Home {
    root: PathBuf,
    ollama: String,
}

impl Home {
    fn new(name: &str, ollama_port: u16) -> Self {
        let root = std::env::temp_dir().join(format!("fdd-device-e8-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home/.fastpilot")).expect("home");
        std::fs::create_dir_all(root.join("project/tests")).expect("project");
        std::fs::write(root.join("project/tests/quick.md"), "# Quick check\nid: quick\nplatform: web\n- Check the page says hello\n").expect("test");
        Home { root, ollama: format!("http://127.0.0.1:{ollama_port}/v1") }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn fp_env(&self, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> (bool, String) {
        use std::io::Write;
        let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
        c.args(args)
            .current_dir(self.root.join("project"))
            .env("HOME", self.home())
            .env("FASTPILOT_KEYSTORE", "file")
            .env("FASTPILOT_OLLAMA_URL", &self.ollama)
            .env("FASTPILOT_CLAUDE", FAKE_CLAUDE)
            .env("FAKE_CLAUDE_LOG", self.root.join("claude-log"))
            .env("FASTPILOT_DIR", self.root.join("project/.fastpilot"))
            .env("FASTPILOT_RETRY_BUDGET", "0")
            .env("FASTPILOT_ADB_SERVER", "127.0.0.1:1")
            .env_remove("FASTPILOT_CONFIG")
            .env_remove("FASTPILOT_PROVIDER")
            .env_remove("FASTPILOT_PROVIDER_BASE_URL")
            .env_remove("FASTPILOT_PROVIDER_MODEL")
            .env_remove("FASTPILOT_PROVIDER_API_KEY")
            .env_remove("FASTPILOT_MODEL")
            .env_remove("FASTPILOT_MODEL_REPLAY")
            .env_remove("FASTPILOT_MODEL_EXPLORE")
            .env_remove("FASTPILOT_AGENT")
            .env_remove("ANDROID_SERIAL")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for (k, v) in env {
            c.env(k, v);
        }
        let mut child = c.spawn().expect("spawn");
        child.stdin.take().expect("stdin").write_all(stdin.as_bytes()).expect("write");
        let out = child.wait_with_output().expect("run");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }

    fn fp(&self, args: &[&str]) -> (bool, String) {
        self.fp_env(args, "", &[])
    }

    /// Two agents (and the default one), written as the dashboard keeps them.
    fn agents(&self) {
        let a = |id: &str, name: &str| json!({ "id": id, "name": name, "job": "Checks the shop.", "devices": ["web"], "skills": [], "created_ms": 1 });
        let store = json!({ "agents": [a("default", "My tester"), a("android-regression", "Android regression"), a("checkout-qa", "Checkout QA")] });
        std::fs::write(self.home().join(".fastpilot/agents.json"), serde_json::to_string_pretty(&store).expect("json")).expect("agents");
    }

    fn claude_calls(&self) -> usize {
        std::fs::read_dir(self.root.join("claude-log")).map(|d| d.count()).unwrap_or(0)
    }

    fn report_of(&self, out: &str) -> String {
        // The newest run's report.md under the project's memory.
        let mut found: Vec<PathBuf> = Vec::new();
        fn walk(p: &Path, found: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, found);
                } else if p.file_name().is_some_and(|n| n == "report.md") {
                    found.push(p);
                }
            }
        }
        walk(&self.root, &mut found);
        found.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        found.last().map(|p| std::fs::read_to_string(p).unwrap_or_default()).unwrap_or_else(|| panic!("no report.md:\n{out}"))
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn chat_models(log: &Log) -> Vec<String> {
    log.lock().expect("log").iter().filter(|(l, ..)| l.starts_with("POST /v1/chat/completions")).map(|(_, _, b)| b["model"].as_str().unwrap_or("").to_owned()).collect()
}

/// E6: a provider's models come from `GET {base}/models`; a server without the list still
/// connects, says so in plain words, and a typed model id still works.
#[test]
fn providers_list_their_models_or_say_why_not() {
    let (pa, la) = fake_server(Some(vec!["model-a", "vendor/model-a2"]));
    let (pn, _) = fake_server(None);
    let h = Home::new("lists", closed_port());
    // Ollama is not running: not listed, and adding it says so.
    let (ok, out) = h.fp(&["provider", "list"]);
    assert!(ok && out.contains("Claude Code") && out.contains("Connected") && !out.contains("Ollama") && out.contains("Add another provider: fastpilot provider add <provider>"), "{out}");
    let (ok, out) = h.fp(&["provider", "add", "ollama"]);
    assert!(!ok && out.contains("✗ Ollama is not running on this Mac. Start Ollama, then run the command again. Nothing was saved."), "{out}");
    // Other, on this computer: no key asked, the list loaded from the server.
    let base = format!("http://127.0.0.1:{pa}/v1");
    let (ok, out) = h.fp(&["provider", "add", "other", &base]);
    assert!(ok && out.contains(&format!("✓ Added 127.0.0.1:{pa} · 2 models found")) && out.contains(&format!("Tests that use 127.0.0.1:{pa} send page text to 127.0.0.1:{pa}.")) && out.contains("Undo: fastpilot undo "), "{out}");
    assert!(la.lock().expect("log").iter().any(|(l, ..)| l.starts_with("GET /v1/models")), "the list comes from the server");
    let (ok, out) = h.fp(&["provider", "models", "other"]);
    assert!(ok && out.contains("model-a, vendor/model-a2"), "{out}");
    let (ok, out) = h.fp(&["provider", "add", "other", &base]);
    assert!(ok && out.contains(&format!("✓ 127.0.0.1:{pa} is already connected. Nothing changed.")), "{out}");
    // A key never goes on the command line.
    let (ok, out) = h.fp(&["provider", "add", "openrouter", "sk-or-in-the-command"]);
    assert!(!ok && out.contains("✗ Do not put the key in the command. Run: fastpilot provider add openrouter and type it when asked. Nothing was saved.") && !out.contains("sk-or-in-the-command"), "{out}");
    let (ok, out) = h.fp(&["provider", "add", "opnai"]);
    assert!(!ok && out.contains("✗ No provider called “opnai”. Providers: openai, openrouter, groq, ollama, other. Claude Code is always connected."), "{out}");
    let (ok, out) = h.fp(&["provider", "add", "other", "llm.example.com"]);
    assert!(!ok && out.contains("✗ “llm.example.com” is not a web address. Use a full address like https://llm.example.com/v1. Nothing was saved."), "{out}");
    let (ok, out) = h.fp(&["provider", "add", "other", "http://llm.example.com/v1"]);
    assert!(!ok && out.contains("✗ https is required, except to this computer. Use https://… Nothing was saved."), "{out}");

    // A server without /models: connected, the reason in plain words, and a typed id works.
    let h2 = Home::new("nolist", closed_port());
    h2.agents();
    let base = format!("http://127.0.0.1:{pn}/v1");
    let (ok, out) = h2.fp(&["provider", "add", "other", &base]);
    assert!(ok && out.contains(&format!("✓ Added 127.0.0.1:{pn}")) && out.contains(&format!("Could not load the list of models from 127.0.0.1:{pn}. A model id you type still works.")), "{out}");
    let (ok, out) = h2.fp(&["provider", "refresh", "other"]);
    assert!(!ok && out.contains(&format!("✗ Could not load the list of models from 127.0.0.1:{pn}. Check the address and key, then run: fastpilot provider refresh other. A model id you type still works.")), "{out}");
    let (ok, out) = h2.fp(&["agent", "model", "Checkout QA", "--explore", "other:my-own-model"]);
    assert!(ok && out.contains("✓ Changed Explore and repair model in Checkout QA · Sonnet 5.5 → my-own-model") && out.contains(&format!("The model list of 127.0.0.1:{pn} could not be loaded, so “my-own-model” was not checked.")), "{out}");
}

/// E8: two agents on different providers each run with their own model, and each run's report
/// says which model really ran. An agent with no choice follows the default; a default change
/// reaches it but not the agents with their own.
#[test]
fn two_agents_run_on_their_own_models_and_the_report_says_which() {
    let (pa, la) = fake_server(Some(vec!["model-a"]));
    let (pb, lb) = fake_server(Some(vec!["model-b"]));
    let h = Home::new("agents", pb);
    h.agents();
    let (ok, out) = h.fp(&["provider", "add", "other", &format!("http://127.0.0.1:{pa}/v1")]);
    assert!(ok, "{out}");
    // Ollama on "this Mac" (the second fake) is found by itself.
    let (ok, out) = h.fp(&["provider", "list"]);
    assert!(ok && out.contains("Ollama") && out.contains("this Mac · found automatically · 1 model"), "{out}");
    let (ok, out) = h.fp(&["agent", "model", "Android regression", "--replay", "other:model-a", "--explore", "model-a"]);
    assert!(ok && out.contains("✓ Changed models in Android regression · Replay: Haiku 5.5 → model-a · Explore and repair: Sonnet 5.5 → model-a"), "{out}");
    assert!(out.contains(&format!("Page text from this agent’s tests goes to 127.0.0.1:{pa}.")) && out.contains("This model is not tried with fastpilot yet.") && out.contains("It applies from the next run."), "{out}");
    let (ok, out) = h.fp(&["agent", "model", "Checkout QA", "--explore", "ollama:model-b"]);
    assert!(ok && out.contains("✓ Changed Explore and repair model in Checkout QA · Sonnet 5.5 → model-b") && out.contains("Page text from this agent’s tests stays on this Mac."), "{out}");
    let (ok, out) = h.fp(&["agent", "model", "Checkout QA", "--explore", "ollama:model-b"]);
    assert!(ok && out.contains("✓ Explore and repair in Checkout QA already uses model-b. Nothing changed."), "{out}");
    let (ok, out) = h.fp(&["agent", "model", "Checkout QA"]);
    assert!(ok && out.contains("Replay              Haiku 5.5 · Claude Code · default") && out.contains("Explore and repair  model-b · Ollama"), "{out}");

    let run = |agent: &str| h.fp_env(&["run", "tests/quick.md"], "", &[("FASTPILOT_AGENT", agent)]);
    let (ok, out) = run("android-regression");
    assert!(ok && out.contains("✓ quick") && out.contains(&format!("model-a (127.0.0.1:{pa})")), "{out}");
    assert!(h.report_of(&out).contains(&format!("Model: model-a (127.0.0.1:{pa})")), "report: {}", h.report_of(&out));
    let (ok, out) = run("checkout-qa");
    assert!(ok && out.contains("✓ quick") && out.contains("model-b (Ollama)"), "{out}");
    assert!(h.report_of(&out).contains("Model: model-b (Ollama)"), "report");
    assert_eq!(chat_models(&la), ["model-a"], "agent A's run went to A only");
    assert_eq!(chat_models(&lb), ["model-b"], "agent B's run went to B only");
    assert_eq!(h.claude_calls(), 0, "Claude Code was not used");

    // The default agent follows the default; a default change reaches it, and the others keep theirs.
    let (ok, out) = h.fp(&["agent", "model", "--default", "--explore", "other:model-a"]);
    assert!(ok && out.contains("✓ Changed default Explore and repair model · Sonnet 5.5 → model-a · 2 agents keep their own"), "{out}");
    let (ok, out) = run("default");
    assert!(ok && out.contains(&format!("model-a (127.0.0.1:{pa})")), "{out}");
    assert_eq!(chat_models(&la), ["model-a", "model-a"]);
    // Undo puts the default back.
    let (ok, out) = h.fp(&["undo"]);
    assert!(ok && out.contains("✓ Undone · Changed default Explore and repair model"), "{out}");
    let (ok, out) = h.fp(&["agent", "model"]);
    assert!(ok && out.contains("Explore and repair  Sonnet 5.5 · Claude Code"), "{out}");

    // Removing a provider: its agents follow the default, and the command says which.
    let (ok, out) = h.fp(&["provider", "remove", "other"]);
    assert!(ok && out.contains(&format!("✓ Removed provider 127.0.0.1:{pa} · this cannot be undone")) && out.contains("Android regression used it and now follows the default models.") && out.contains("To connect it again: fastpilot provider add other"), "{out}");
    let (ok, out) = h.fp(&["agent", "model", "Android regression"]);
    assert!(ok && out.contains("Replay              Haiku 5.5 · Claude Code · default") && out.contains("Explore and repair  Sonnet 5.5 · Claude Code · default"), "{out}");
    let (ok, out) = h.fp(&["agent", "model", "Android regression", "--explore", "other:model-a"]);
    assert!(!ok && out.contains("✗ Other is not connected. Add it first: fastpilot provider add other. Nothing changed."), "{out}");
}

/// No silent switch: a provider that does not answer stops the test with a plain line, and no other
/// provider (not Claude Code either) is used in its place. It is "could not run", not a product failure.
#[test]
fn a_provider_that_does_not_answer_stops_the_run_and_nothing_is_switched() {
    let (pb, _) = fake_server(Some(vec!["model-b"]));
    let (pa, la) = fake_server(Some(vec!["model-a"]));
    let h = Home::new("down", pb);
    h.agents();
    assert!(h.fp(&["provider", "add", "other", &format!("http://127.0.0.1:{pa}/v1")]).0);
    let (ok, out) = h.fp(&["agent", "model", "Checkout QA", "--replay", "ollama:model-b", "--explore", "ollama:model-b"]);
    assert!(ok, "{out}");
    // Ollama stops (here: its address has nobody listening).
    let down = format!("http://127.0.0.1:{}/v1", closed_port());
    let (ok, out) = h.fp_env(&["run", "tests/quick.md"], "", &[("FASTPILOT_AGENT", "checkout-qa"), ("FASTPILOT_OLLAMA_URL", down.as_str())]);
    assert!(!ok && out.contains("Stopped: Ollama is not running on this Mac. Start Ollama, then run it again."), "{out}");
    assert_eq!(h.claude_calls(), 0, "Claude Code was not used instead");
    assert!(chat_models(&la).is_empty(), "the other provider was not used instead");
    assert!(out.contains("[infra]") || out.contains("could not run"), "not a product failure: {out}");
    let (ok, out) = h.fp_env(&["provider", "list"], "", &[("FASTPILOT_OLLAMA_URL", down.as_str())]);
    assert!(ok && !out.contains("Ollama"), "an Ollama that is not running is not listed when it was never added: {out}");
}
