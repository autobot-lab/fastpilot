//! C1+C2: `fastpilot ui` only answers its own page. 127.0.0.1, a random port, a one-time code
//! (wave 3: spent on first use, traded for a different session cookie, HttpOnly SameSite); other Hosts (DNS rebinding), other Origins and non-JSON
//! posts are refused; only the page's settings can be written, atomically and owner-only;
//! secret values never leave; assets are a fixed list (no path lookups).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

struct Ui {
    child: Child,
    port: u16,
    /// The one-time code of the printed link.
    code: String,
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
    let (port, code) = rest.split_once("/?code=").expect("code in URL");
    assert_eq!(code.len(), 64, "256-bit code");
    Ui { child, port: port.parse().expect("port"), code: code.to_owned(), home }
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
    let origin = || ("Origin", format!("http://127.0.0.1:{p}"));
    let json = || ("Content-Type", "application/json".to_owned());

    // No code, a wrong one, another Host (DNS rebinding), the code on an API route: refused, and
    // none of these spends the code.
    assert_eq!(http(p, "GET", "/", &[], "").status, 401);
    assert_eq!(http(p, "GET", "/api/settings", &[], "").status, 401);
    assert_eq!(http(p, "GET", &format!("/?code={}", "0".repeat(64)), &[], "").status, 401);
    assert_eq!(http(p, "GET", &format!("/?code={}", ui.code), &[("Host", format!("evil.example:{p}"))], "").status, 421);
    assert_eq!(http(p, "GET", &format!("/api/settings?code={}", ui.code), &[], "").status, 401);
    // The old `?token=` link form gives nothing either.
    assert_eq!(http(p, "GET", &format!("/?token={}", ui.code), &[], "").status, 401);

    // The code becomes a session cookie with another value, and leaves the address bar.
    let r = http(p, "GET", &format!("/?code={}", ui.code), &[], "");
    assert_eq!(r.status, 303);
    assert_eq!(r.header("location").as_deref(), Some("/"));
    let set = r.header("set-cookie").expect("cookie");
    assert!(set.contains("HttpOnly") && set.contains("SameSite=Strict") && set.starts_with(&format!("fp_ui_{p}=")), "{set}");
    let session = set.split(';').next().and_then(|kv| kv.split_once('=')).map(|(_, v)| v.to_owned()).expect("session");
    assert!(session.len() == 64 && session != ui.code, "the cookie is not the code");
    let cookie = || ("Cookie", format!("fp_ui_{p}={session}"));
    assert_eq!(http(p, "GET", "/api/settings", &[cookie(), ("Host", "evil.example".into())], "").status, 421);
    // Spent: the same link again opens nothing (security review, Medium 3).
    let again = http(p, "GET", &format!("/?code={}", ui.code), &[], "");
    assert!(again.status == 401 && again.text().contains("sudah dipakai"), "{} {}", again.status, again.text());
    // The code is not a cookie either.
    assert_eq!(http(p, "GET", "/api/settings", &[("Cookie", format!("fp_ui_{p}={}", ui.code))], "").status, 401);
    // With the session, an old link just goes to the page.
    assert_eq!(http(p, "GET", &format!("/?code={}", ui.code), &[cookie()], "").status, 303);

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
        (r#"{"key":"chrome","value":"/bin/sh"}"#, 400, "tidak bisa diubah di sini"),
        (r#"{"key":"allow_eval_script","value":"maybe"}"#, 400, "true or false"),
        (r#"{"key":"upload_dirs","value":["/a,/b"]}"#, 400, "koma"),
        // QA #6: what the page lets through is checked too.
        (r#"{"key":"allowed_domains","value":["http://bad domain/x!"]}"#, 400, "bukan nama domain"),
        (r#"{"key":"allowed_domains","value":["https://example.net/path"]}"#, 400, "tanpa https://"),
        (r#"{"key":"vars","value":["bad name!"]}"#, 400, "bukan nama variabel"),
        (r#"{"key":"vars","value":["FASTPILOT_STATE_KEY"]}"#, 400, "pengaturan fastpilot sendiri"),
        (r#"{"key":"upload_dirs","value":["relative/dir"]}"#, 400, "folder lengkap"),
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

/// QA #3: `--help` and unknown options print help and start nothing; a taken or forbidden port
/// says which port and what to do.
#[test]
fn ui_arguments_and_port_errors_are_explained() {
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .args(args)
            .env("HOME", std::env::temp_dir())
            .stdin(Stdio::null())
            .output()
            .expect("run");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    // QA N2 (P2): plain lines with a real line break, not `Error: "...\n..."`; exit code 2.
    let out = Command::new(env!("CARGO_BIN_EXE_fastpilot")).args(["ui", "--bogus"]).env("HOME", std::env::temp_dir()).stdin(Stdio::null()).output().expect("run");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.starts_with("fastpilot ui: unknown option `--bogus`\n") && !err.contains("\\n") && !err.contains("Error: \""), "{err}");
    // These must return at once (a server would hang the test).
    let (ok, out) = run(&["ui", "--help"]);
    assert!(ok && out.contains("usage: fastpilot ui") && !out.contains("http://"), "{out}");
    let (ok, out) = run(&["ui", "--bogus"]);
    assert!(!ok && out.contains("unknown option `--bogus`") && out.contains("usage:"), "{out}");
    let (ok, out) = run(&["ui", "--port", "99999"]);
    assert!(!ok && out.contains("1024 to 65535"), "{out}");
    let (ok, out) = run(&["ui", "--port", "80"]);
    assert!(!ok && out.contains("1024 to 65535"), "{out}");
    let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = taken.local_addr().expect("addr").port().to_string();
    let (ok, out) = run(&["ui", "--no-open", "--port", &port]);
    assert!(!ok && out.contains(&format!("port {port} is already in use")) && !out.contains("Os {"), "{out}");
}

/// QA #4 / Design #4: a settings file that does not parse is shown as an error, not as
/// "everything allowed", and nothing can be saved until it is fixed.
#[test]
fn ui_shows_a_broken_settings_file() {
    let home = std::env::temp_dir().join(format!("fp-ui-broken-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".fastpilot")).expect("dir");
    std::fs::write(home.join(".fastpilot/config.toml"), "block_actions = [\"Bayar\"]\nallowed_domains = [\n").expect("broken");
    let mut child = Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .env("HOME", &home)
        .env_remove("FASTPILOT_CONFIG")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("ui starts with a broken file");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("url");
    let ui = Ui {
        port: line.trim().strip_prefix("http://127.0.0.1:").and_then(|r| r.split('/').next()).and_then(|p| p.parse().ok()).expect("port"),
        code: line.trim().rsplit("code=").next().unwrap_or("").to_owned(),
        child,
        home,
    };
    let p = ui.port;
    let r = http(p, "GET", &format!("/?code={}", ui.code), &[], "");
    let session = r.header("set-cookie").and_then(|c| c.split(';').next().map(str::to_owned)).expect("cookie");
    let v: serde_json::Value = serde_json::from_str(&http(p, "GET", "/api/settings", &[("Cookie", session.clone())], "").text()).expect("json");
    assert!(v["error"].as_str().is_some_and(|e| e.contains("not valid TOML") && e.contains("config.toml")), "{v}");
    assert_eq!(v["settings"], serde_json::json!([]), "no made-up values while the file is broken");
    let r = http(
        p,
        "POST",
        "/api/settings",
        &[("Cookie", session), ("Origin", format!("http://127.0.0.1:{p}")), ("Content-Type", "application/json".into())],
        r#"{"key":"allow_eval_script","value":false}"#,
    );
    assert!(r.status == 409 && r.text().contains("rusak"), "{} {}", r.status, r.text());
    let text = std::fs::read_to_string(ui.home.join(".fastpilot/config.toml")).expect("file");
    assert!(text.contains("allowed_domains = [\n") && !text.contains("allow_eval_script"), "the broken file is left as it was");
}

/// Security review, Medium 3: the browser is opened with a local redirect file, so the code is
/// not in the opener's arguments (readable by other processes); the file is owner-only.
#[cfg(unix)]
#[test]
fn ui_opens_the_browser_without_the_code_in_argv() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("fp-ui-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    // Stand-ins for `open` / `xdg-open`: record the arguments and the file they were given.
    let log = dir.join("argv.txt");
    let script = format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{0}'\nls -l \"$1\" >> '{0}'\ncat \"$1\" >> '{0}'\n", log.display());
    for name in ["open", "xdg-open"] {
        let p = dir.join(name);
        std::fs::write(&p, &script).expect("script");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default());
    let mut child = Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui"])
        .env("PATH", path)
        .env("HOME", &dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("ui");
    let mut out = String::new();
    let mut lines = BufReader::new(child.stdout.take().expect("stdout"));
    lines.read_line(&mut out).expect("line");
    lines.read_line(&mut out).expect("line");
    let code = out.rsplit("code=").next().map(|c| c.trim().to_owned()).expect("code");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let recorded = loop {
        if let Ok(t) = std::fs::read_to_string(&log)
            && t.contains("<meta")
        {
            break t;
        }
        assert!(std::time::Instant::now() < deadline, "the opener was not called");
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let _ = child.kill();
    let _ = child.wait();
    let argv = recorded.lines().next().unwrap_or("");
    assert!(argv.ends_with("open.html") && !argv.contains(&code), "argv: {argv}");
    assert!(recorded.contains("-rw-------"), "owner-only redirect file: {recorded}");
    assert!(recorded.contains(&format!("code={code}")), "the file carries the link: {recorded}");
    let _ = std::fs::remove_dir_all(dir);
}

/// P2 item 6: the checks on Perangkat are explained in Indonesian (the CLI `doctor` stays English).
#[test]
fn ui_doctor_speaks_indonesian() {
    let ui = start("doctor-id", &[]);
    let p = ui.port;
    let r = http(p, "GET", &format!("/?code={}", ui.code), &[], "");
    let session = r.header("set-cookie").and_then(|c| c.split(';').next().map(str::to_owned)).expect("cookie");
    let r = http(p, "GET", "/api/doctor", &[("Cookie", session)], "");
    assert_eq!(r.status, 200, "{}", r.text());
    let v: serde_json::Value = serde_json::from_str(&r.text()).expect("json");
    let checks = v["checks"].as_array().expect("checks");
    assert!(!checks.is_empty(), "{v}");
    // English words that only appear in the untranslated detail/hint texts.
    let english = ["not found", "not installed", "Install ", "install it", " found ", "missing", "running", "Run `", "no device"];
    for c in checks {
        for field in ["detail", "hint"] {
            let Some(t) = c[field].as_str() else { continue };
            for w in english {
                assert!(!t.contains(w), "{field} still English ({w:?}): {t}");
            }
        }
    }
}
