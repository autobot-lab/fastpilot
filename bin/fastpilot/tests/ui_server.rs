//! C1+C2: `fastpilot ui` only answers its own page. 127.0.0.1, a random port, a token that
//! becomes an HttpOnly SameSite cookie; other Hosts (DNS rebinding), other Origins and non-JSON
//! posts are refused; only the page's settings can be written, atomically and owner-only;
//! secret values never leave; assets are a fixed list (no path lookups).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

struct Ui {
    child: Child,
    port: u16,
    token: String,
    home: std::path::PathBuf,
}

impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn start(name: &str, env: &[(&str, &str)]) -> Ui {
    let home = std::env::temp_dir().join(format!("fp-ui-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("home");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    cmd.args(["ui", "--no-open"]).env("HOME", &home).env_remove("FASTPILOT_CONFIG").stdout(Stdio::piped()).stderr(Stdio::null());
    for k in ["FASTPILOT_ALLOWED_DOMAINS", "FASTPILOT_BLOCK_ACTIONS", "FASTPILOT_VARS", "FASTPILOT_MODEL_REPLAY", "FASTPILOT_ALLOW_EVAL_SCRIPT"] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("fastpilot ui");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("url line");
    let url = line.trim().to_owned();
    let rest = url.strip_prefix("http://127.0.0.1:").unwrap_or_else(|| panic!("not a 127.0.0.1 URL: {url}"));
    let (port, token) = rest.split_once("/?token=").expect("token in URL");
    assert_eq!(token.len(), 64, "256-bit token");
    Ui { child, port: port.parse().expect("port"), token: token.to_owned(), home }
}

struct Reply {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Reply {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_owned()))
    }
}

fn http(port: u16, method: &str, path: &str, headers: &[(&str, String)], body: &str) -> Reply {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        req.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).expect("write");
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).expect("read");
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("head");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).expect("status");
    Reply { status, head, body: raw[split + 4..].to_vec() }
}

#[test]
fn ui_server_only_answers_its_own_page() {
    let ui = start("auth", &[("FP_UI_SECRET", "s3cr3t-value-x9"), ("FASTPILOT_MODEL_REPLAY", "claude-haiku-5-5")]);
    let p = ui.port;
    let cookie = || ("Cookie", format!("fp_ui_{p}={}", ui.token));
    let origin = || ("Origin", format!("http://127.0.0.1:{p}"));
    let json = || ("Content-Type", "application/json".to_owned());

    // No token, a wrong one, another Host (DNS rebinding): refused.
    assert_eq!(http(p, "GET", "/", &[], "").status, 401);
    assert_eq!(http(p, "GET", "/api/settings", &[], "").status, 401);
    assert_eq!(http(p, "GET", &format!("/?token={}", "0".repeat(64)), &[], "").status, 401);
    assert_eq!(http(p, "GET", &format!("/?token={}", ui.token), &[("Host", format!("evil.example:{p}"))], "").status, 421);
    assert_eq!(http(p, "GET", "/api/settings", &[cookie(), ("Host", "evil.example".into())], "").status, 421);

    // The token becomes a cookie and leaves the address bar.
    let r = http(p, "GET", &format!("/?token={}", ui.token), &[], "");
    assert_eq!(r.status, 303);
    assert_eq!(r.header("location").as_deref(), Some("/"));
    let set = r.header("set-cookie").expect("cookie");
    assert!(set.contains("HttpOnly") && set.contains("SameSite=Strict") && set.starts_with(&format!("fp_ui_{p}={}", ui.token)), "{set}");

    // The page, with its own fonts and icons; locked down.
    let r = http(p, "GET", "/", &[cookie()], "");
    assert_eq!(r.status, 200);
    let page = r.text();
    assert!(page.contains("/assets/app.js") && page.contains(r#"id="i-device""#) && !page.contains("http"), "no external links: {page}");
    let csp = r.header("content-security-policy").expect("csp");
    assert!(csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'"), "{csp}");
    assert_eq!(r.header("x-frame-options").as_deref(), Some("DENY"));
    let font = http(p, "GET", "/assets/fonts/ibm-plex-sans-400.woff2", &[cookie()], "");
    assert_eq!((font.status, font.header("content-type").as_deref()), (200, Some("font/woff2")));
    assert_eq!(&font.body[..4], b"wOF2");
    assert!(http(p, "GET", "/assets/app.css", &[cookie()], "").text().contains("Bricolage Grotesque"));
    for bad in ["/assets/../Cargo.toml", "/assets/..%2f..%2fCargo.toml", "/assets/fonts/../../src/ui.rs", "/assets/nope.js"] {
        let r = http(p, "GET", bad, &[cookie()], "");
        assert!(r.status == 404 || r.status == 400, "{bad}: {}", r.status);
        assert!(!r.text().contains("[package]") && !r.text().contains("fn guard"), "{bad} leaked a file");
    }

    // Changes: only JSON from our own Origin.
    let body = r#"{"key":"block_actions","value":["Bayar"]}"#;
    assert_eq!(http(p, "POST", "/api/settings", &[cookie(), json()], body).status, 403, "no Origin");
    assert_eq!(http(p, "POST", "/api/settings", &[cookie(), json(), ("Origin", "http://evil.example".into())], body).status, 403);
    assert_eq!(http(p, "POST", "/api/settings", &[cookie(), origin(), ("Content-Type", "text/plain".into())], body).status, 403);
    assert_eq!(http(p, "POST", "/api/settings", &[origin(), json()], body).status, 401, "no cookie");
    let r = http(p, "POST", "/api/settings", &[cookie(), origin(), json()], body);
    assert_eq!(r.status, 200, "{}", r.text());
    let file = ui.home.join(".fastpilot/config.toml");
    let saved = std::fs::read_to_string(&file).expect("config file");
    assert!(saved.contains(r#"block_actions = ["Bayar"]"#), "{saved}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).expect("meta").permissions().mode() & 0o777, 0o600);
    }
    // The user loosens here (unlike an agent's `config` tool), and switches permissions.
    for b in [r#"{"key":"allowed_domains","value":["toko.com"]}"#, r#"{"key":"allowed_domains","value":null}"#, r#"{"key":"allow_eval_script","value":false}"#] {
        let r = http(p, "POST", "/api/settings", &[cookie(), origin(), json()], b);
        assert_eq!(r.status, 200, "{b}: {}", r.text());
    }
    let saved = std::fs::read_to_string(&file).expect("config file");
    assert!(!saved.contains("allowed_domains") && saved.contains("allow_eval_script = false"), "{saved}");
    // Not the page's settings, bad values, and what the environment sets: refused.
    for (b, code, why) in [
        (r#"{"key":"chrome","value":"/bin/sh"}"#, 400, "cannot be changed here"),
        (r#"{"key":"allow_eval_script","value":"maybe"}"#, 400, "true or false"),
        (r#"{"key":"upload_dirs","value":["/a,/b"]}"#, 400, "comma"),
        (r#"{"key":"model_replay","value":"claude-opus-5-5"}"#, 409, "FASTPILOT_MODEL_REPLAY"),
    ] {
        let r = http(p, "POST", "/api/settings", &[cookie(), origin(), json()], b);
        assert_eq!(r.status, code, "{b}: {}", r.text());
        assert!(r.text().contains(why), "{b}: {}", r.text());
    }
    assert!(!std::fs::read_to_string(&file).expect("file").contains("/bin/sh"));

    // Secrets: names only.
    let r = http(p, "POST", "/api/settings", &[cookie(), origin(), json()], r#"{"key":"vars","value":["FP_UI_SECRET"]}"#);
    assert_eq!(r.status, 200);
    let all = http(p, "GET", "/api/settings", &[cookie()], "").text();
    assert!(all.contains("FP_UI_SECRET") && !all.contains("s3cr3t-value-x9"), "{all}");
    let v: serde_json::Value = serde_json::from_str(&all).expect("json");
    let replay = v["settings"].as_array().and_then(|a| a.iter().find(|s| s["key"] == "model_replay")).cloned().expect("model_replay");
    assert_eq!(replay["source"], "env");
}

#[test]
fn ui_listens_on_loopback_only() {
    let ui = start("loopback", &[]);
    // Another local address of this machine (if any) must not reach it.
    let other = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|_| s))
        .and_then(|s| s.local_addr())
        .map(|a| a.ip())
        .ok()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified());
    if let Some(ip) = other {
        assert!(TcpStream::connect((ip, ui.port)).is_err(), "reachable on {ip}");
    }
    assert!(TcpStream::connect(("127.0.0.1", ui.port)).is_ok());
}
