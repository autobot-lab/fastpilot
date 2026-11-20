//! B2 in Pengaturan › Model AI: pick an OpenAI-compatible provider, base URL and model, store the
//! API key (key store; the file backend here, so the user's Keychain is untouched), test the
//! connection against a fake chat-completions API, and go back to Claude Code. The key is never
//! echoed by the API nor written to the settings file; bad values are refused with a reason.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

fn fake_openai() -> (u16, Arc<Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().expect("addr").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut r = BufReader::new(s.try_clone().expect("clone"));
            let mut line = String::new();
            let _ = r.read_line(&mut line);
            let (mut len, mut auth) = (0usize, String::new());
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
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
            log.lock().expect("log").push(format!("{} {auth}", line.trim()));
            let (status, reply) = if auth.ends_with("Bearer sk-ui-test-123") {
                ("200 OK", json!({ "choices": [{ "message": { "role": "assistant", "content": "OK" } }] }))
            } else {
                ("401 Unauthorized", json!({ "error": { "message": "Incorrect API key provided" } }))
            };
            let reply = reply.to_string();
            let mut s = s;
            let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
        }
    });
    (port, seen)
}

struct Ui {
    child: std::process::Child,
    port: u16,
    cookie: String,
}
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn request(port: u16, method: &str, path: &str, cookie: &str, body: &str, origin: bool) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let o = if origin { format!("Origin: http://127.0.0.1:{port}\r\n") } else { String::new() };
    let req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\n{o}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let status = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, raw.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default())
}

fn start_ui(home: &Path) -> Ui {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    cmd.args(["ui", "--no-open"]).env("HOME", home).env("FASTPILOT_KEYSTORE", "file").env_remove("FASTPILOT_CONFIG");
    for k in ["FASTPILOT_PROVIDER", "FASTPILOT_PROVIDER_BASE_URL", "FASTPILOT_PROVIDER_MODEL", "FASTPILOT_PROVIDER_API_KEY"] {
        cmd.env_remove(k);
    }
    let mut child = cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().expect("ui");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("link");
    let rest = line.trim().strip_prefix("http://127.0.0.1:").expect("link").to_owned();
    let (port, code) = rest.split_once("/?code=").expect("code");
    let port: u16 = port.parse().expect("port");
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.write_all(format!("GET /?code={code} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let cookie = raw.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().to_owned())).and_then(|c| c.split(';').next().map(str::to_owned)).expect("cookie");
    Ui { child, port, cookie }
}

#[test]
fn provider_settings_key_and_connection_test_from_the_page() {
    let home = std::env::temp_dir().join(format!("fp-prov-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let (port, seen) = fake_openai();
    let ui = start_ui(&home);
    let get = || {
        let (st, body) = request(ui.port, "GET", "/api/provider", &ui.cookie, "", false);
        assert_eq!(st, 200, "{body}");
        serde_json::from_str::<Value>(&body).expect("json")
    };
    let post = |body: Value| request(ui.port, "POST", "/api/provider", &ui.cookie, &body.to_string(), true);
    let base = format!("http://127.0.0.1:{port}/v1");

    let v = get();
    assert_eq!(v["provider"], "claude");
    assert!(v["key"].is_null());
    let names: Vec<&str> = v["presets"].as_array().expect("presets").iter().filter_map(|p| p["name"].as_str()).collect();
    assert_eq!(names, ["claude", "openai", "openrouter", "groq", "ollama", "custom"]);

    // Refused, with the reason.
    let (st, body) = post(json!({ "action": "save", "provider": "custom", "base_url": "http://example.com/v1", "model": "m" }));
    assert!(st == 422 && body.contains("unencrypted"), "{st} {body}");
    let (st, body) = post(json!({ "action": "save", "provider": "custom", "model": "m" }));
    assert!(st == 422 && body.contains("Base URL"), "{st} {body}");
    let (st, body) = post(json!({ "action": "save", "provider": "openai", "model": "" }));
    assert!(st == 422 && body.contains("model"), "{st} {body}");
    let (st, body) = post(json!({ "action": "save", "provider": "custom", "base_url": base, "model": "fake-model", "key": "sk with space" }));
    assert!(st == 422 && body.contains("without spaces"), "{st} {body}");
    let (st, _) = request(ui.port, "POST", "/api/provider", &ui.cookie, &json!({ "action": "test" }).to_string(), false);
    assert!(st == 403 || st == 400, "no Origin: {st}");

    // A wrong key: saved (never echoed), the connection test says 401 without the key.
    let (st, body) = post(json!({ "action": "save", "provider": "custom", "base_url": base, "model": "fake-model", "key": "sk-wrong-1" }));
    assert!(st == 200 && !body.contains("sk-wrong-1"), "{st} {body}");
    let v = get();
    assert_eq!((v["provider"].as_str(), v["model"].as_str(), v["key"]["has"].as_bool()), (Some("custom"), Some("fake-model"), Some(true)), "{v}");
    assert!(v["key"]["where"].as_str().is_some_and(|w| w.contains("file khusus pemilik")) && !v.to_string().contains("sk-wrong-1"), "{v}");
    let cfg = std::fs::read_to_string(home.join(".fastpilot/config.toml")).expect("config");
    assert!(cfg.contains("provider = \"custom\"") && !cfg.contains("sk-"), "{cfg}");
    let (st, body) = post(json!({ "action": "test" }));
    assert!(st == 422 && body.contains("401") && !body.contains("sk-wrong-1"), "{st} {body}");

    // The right key: OK.
    let (st, body) = post(json!({ "action": "save", "provider": "custom", "base_url": base, "model": "fake-model", "key": "sk-ui-test-123" }));
    assert_eq!(st, 200, "{body}");
    let (st, body) = post(json!({ "action": "test" }));
    assert!(st == 200 && body.contains("ok:") && body.contains("fake-model") && !body.contains("sk-ui-test-123"), "{st} {body}");
    assert!(seen.lock().expect("seen").iter().any(|l| l.starts_with("POST /v1/chat/completions") && l.ends_with("Bearer sk-ui-test-123")));

    assert_eq!(get()["key"]["origin"], format!("http://127.0.0.1:{port}"), "the key is shown with the server it is for");
    // Another server for custom without a new key: told that the key belongs to the old one.
    let (st, body) = post(json!({ "action": "save", "provider": "custom", "base_url": "http://127.0.0.1:9/v1", "model": "fake-model" }));
    assert!(st == 200 && body.contains(&format!("tersimpan untuk http://127.0.0.1:{port}")), "{st} {body}");
    // A hosted preset with the previous server still in the field: the preset's server is stored.
    let (st, body) = post(json!({ "action": "save", "provider": "openrouter", "base_url": "http://127.0.0.1:9/v1", "model": "x/y" }));
    assert_eq!(st, 200, "{body}");
    let v = get();
    assert!(v["base_url"].is_null(), "no leftover server for a hosted preset: {v}");
    assert!(!std::fs::read_to_string(home.join(".fastpilot/config.toml")).expect("config").contains("provider_base_url"));
    let (st, _) = post(json!({ "action": "save", "provider": "custom", "base_url": base, "model": "fake-model" }));
    assert_eq!(st, 200);

    // Saving without a key keeps the stored one; Hapus kunci removes it; back to Claude Code.
    let (st, _) = post(json!({ "action": "save", "provider": "custom", "base_url": base, "model": "fake-model-2" }));
    assert_eq!(st, 200);
    assert_eq!(get()["key"]["has"], true);
    let (st, _) = post(json!({ "action": "delete-key" }));
    assert_eq!(st, 200);
    assert_eq!(get()["key"]["has"], false);
    let (st, _) = post(json!({ "action": "save", "provider": "claude" }));
    assert_eq!(st, 200);
    let cfg = std::fs::read_to_string(home.join(".fastpilot/config.toml")).expect("config");
    assert!(!cfg.contains("provider"), "{cfg}");
    drop(ui);
    let _ = std::fs::remove_dir_all(home);
}
