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
    assert!(again.status == 401 && again.text().contains("already used"), "{} {}", again.status, again.text());
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

    // F5 (v0.3.2): nothing set = the built-in lists, sent with the settings for the page.
    let st: serde_json::Value = serde_json::from_str(&http(p, "GET", "/api/settings", &[cookie()], "").text()).expect("settings json");
    let block = st["settings"].as_array().and_then(|l| l.iter().find(|s| s["key"] == "block_actions")).expect("block_actions").clone();
    assert_eq!((block["source"].as_str(), block["value"].is_null()), (Some("default"), true), "{block}");
    assert_eq!(st["defaults"]["block_actions"][0], "bayar", "{st}");
    assert_eq!(st["defaults"]["confirm_actions"], serde_json::json!(["hapus", "delete", "remove"]), "{st}");

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
    // F5: "Empty it" (an empty list) is kept as emptied on purpose; "Back to built-in" (null)
    // removes the key, so the built-in list is back; adding to the built-in list keeps `default`.
    for (b, want) in [
        (r#"{"key":"block_actions","value":[]}"#, "block_actions = []"),
        (r#"{"key":"confirm_actions","value":[]}"#, "confirm_actions = []"),
        (r#"{"key":"block_actions","value":["default","Keluar"]}"#, r#"block_actions = ["default", "Keluar"]"#),
    ] {
        let r = http(p, "POST", "/api/settings", &[cookie(), origin(), json()], b);
        assert_eq!(r.status, 200, "{b}: {}", r.text());
        let saved = std::fs::read_to_string(&file).expect("config file");
        assert!(saved.contains(want), "{b}: {saved}");
    }
    for k in ["block_actions", "confirm_actions"] {
        let r = http(p, "POST", "/api/settings", &[cookie(), origin(), json()], &format!(r#"{{"key":"{k}","value":null}}"#));
        assert_eq!(r.status, 200, "{}", r.text());
    }
    let saved = std::fs::read_to_string(&file).expect("config file");
    assert!(!saved.contains("block_actions") && !saved.contains("confirm_actions"), "back to the built-in lists: {saved}");
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
        // QA #6: what the page lets through is checked too.
        (r#"{"key":"allowed_domains","value":["http://bad domain/x!"]}"#, 400, "is not a domain name"),
        (r#"{"key":"allowed_domains","value":["https://example.net/path"]}"#, 400, "no https://"),
        (r#"{"key":"vars","value":["bad name!"]}"#, 400, "is not a variable name"),
        (r#"{"key":"vars","value":["FASTPILOT_STATE_KEY"]}"#, 400, "fastpilot's own settings"),
        (r#"{"key":"upload_dirs","value":["relative/dir"]}"#, 400, "full folder"),
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
    assert!(!ok && out.contains("from 1024 to 65535"), "{out}");
    let (ok, out) = run(&["ui", "--port", "80"]);
    assert!(!ok && out.contains("from 1024 to 65535"), "{out}");
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
    assert!(r.status == 409 && r.text().contains("is damaged"), "{} {}", r.status, r.text());
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

/// The Device page's checks (v0.4, FastTest M3): in English (E1 replaced the old Indonesian-only
/// rule, so this no longer fails whenever an iPhone simulator is "running"), and with no file paths
/// or device serials on screen; the terminal's `fastpilot doctor` keeps those.
#[test]
fn ui_doctor_shows_no_paths_or_device_ids() {
    let ui = start("doctor-screen", &[]);
    let p = ui.port;
    let r = http(p, "GET", &format!("/?code={}", ui.code), &[], "");
    let session = r.header("set-cookie").and_then(|c| c.split(';').next().map(str::to_owned)).expect("cookie");
    let r = http(p, "GET", "/api/doctor", &[("Cookie", session)], "");
    assert_eq!(r.status, 200, "{}", r.text());
    let v: serde_json::Value = serde_json::from_str(&r.text()).expect("json");
    let checks = v["checks"].as_array().expect("checks");
    assert!(!checks.is_empty(), "{v}");
    for c in checks {
        for field in ["detail", "hint"] {
            let Some(t) = c[field].as_str() else { continue };
            for bad in ["/Applications/", "/usr/", "/opt/", "/Users/", ".xctestrun", ".app/Contents", "emulator-55"] {
                assert!(!t.contains(bad), "{field} shows a path or a device id ({bad:?}): {t}");
            }
        }
    }
}

/// Whoever started `fastpilot ui` may stop reading its output (a script that only wanted the link):
/// the server keeps running instead of dying on a broken pipe (flaky "connection reset" in tests).
#[test]
fn ui_survives_a_closed_stdout() {
    let home = std::env::temp_dir().join(format!("fp-ui-pipe-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("home");
    // The free port is picked here and handed over, so another test running in parallel can take
    // it in between (v0.4.0 gate: "up true, alive false"). That shows as "already in use" on the
    // server's stderr: then try another port. Any other exit is the failure this test is about.
    let (mut up, mut alive, mut status, mut last_err) = (false, false, 0, String::new());
    for _attempt in 0..3 {
        let free = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = free.local_addr().expect("addr").port();
        drop(free);
        let mut child = Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .args(["ui", "--no-open", "--port", &port.to_string()])
            .env("HOME", &home)
            .env_remove("FASTPILOT_CONFIG")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("ui");
        // Closed before the server writes its first line: every write it makes hits a closed pipe.
        drop(child.stdout.take());
        up = false;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                up = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        alive = child.try_wait().expect("wait").is_none();
        status = if up && alive { http(port, "GET", "/api/live", &[], "").status } else { 0 };
        let _ = child.kill();
        let _ = child.wait();
        last_err.clear();
        if let Some(mut err) = child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut err, &mut last_err);
        }
        if alive || !last_err.contains("already in use") {
            break;
        }
    }
    let _ = std::fs::remove_dir_all(home);
    assert!(up && alive, "still serving with its stdout closed (up {up}, alive {alive}): {last_err}");
    assert_eq!(status, 401, "and answers");
}

/// V042 (QA Low): a session no agent started is not labelled "My agent" in Live: a terminal session
/// says "Started from the terminal", a scheduled run "Scheduled · <name>", Chat keeps its agent.
#[test]
fn live_says_where_a_session_came_from() {
    let ui = start("live-source", &[]);
    let p = ui.port;
    let live = ui.home.join(".fastpilot/live");
    std::fs::create_dir_all(&live).expect("live");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("time").as_millis() as u64;
    let proc = |pid: u32, kind: &str, extra: serde_json::Value, id: &str| {
        let mut v = serde_json::json!({ "pid": pid, "kind": kind, "updated_ms": now, "sessions": [{ "id": id, "platform": "web", "started_ms": now }], "flows": [] });
        for (k, x) in extra.as_object().expect("obj") {
            v[k] = x.clone();
        }
        std::fs::write(live.join(format!("{pid}.json")), v.to_string()).expect("write");
    };
    proc(4_000_001, "mcp", serde_json::json!({}), "term1");
    proc(4_000_002, "run", serde_json::json!({ "scheduled": "Checkout every 2 hours" }), "sched1");
    proc(4_000_003, "mcp", serde_json::json!({ "agent": "default" }), "chat1");
    let r = http(p, "GET", &format!("/?code={}", ui.code), &[], "");
    let session = r.header("set-cookie").and_then(|c| c.split(';').next().map(str::to_owned)).expect("cookie");
    let r = http(p, "GET", "/api/live", &[("Cookie", session)], "");
    assert_eq!(r.status, 200, "{}", r.text());
    let v: serde_json::Value = serde_json::from_str(&r.text()).expect("json");
    let label = |id: &str| v["sessions"].as_array().and_then(|a| a.iter().find(|s| s["id"] == id)).map(|s| (s["agent_name"].as_str().unwrap_or("").to_owned(), s["model_label"].is_null()));
    assert_eq!(label("term1"), Some(("Started from the terminal".to_owned(), true)), "{v}");
    assert_eq!(label("sched1"), Some(("Scheduled · Checkout every 2 hours".to_owned(), true)), "{v}");
    assert_eq!(label("chat1").map(|l| l.0), Some("My tester".to_owned()), "Chat keeps its agent: {v}");
}

/// V051 (QA Lows 1, 3, 4): the words people read agree with what fastpilot does.
#[test]
fn the_dashboard_and_readme_words_are_current() {
    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let app = std::fs::read_to_string(format!("{ROOT}/bin/fastpilot/ui/app.js")).expect("app.js");
    assert!(!app.contains("New in 0.3.2"), "Settings › Limits has no stale 0.3.2 notice");
    // The Skills page counts the skills an agent can switch; the fastpilot skill is always on, so the
    // page and the README's 24 add up.
    let agents = std::fs::read_to_string(format!("{ROOT}/bin/fastpilot/ui/agents.js")).expect("agents.js");
    // v0.7.1 FC8: General QA's 8 are always on; the specialists are counted as "you added".
    assert!(agents.contains("8 always on, plus ${n} you added"), "Skills page count");
    let skills = std::fs::read_dir(format!("{ROOT}/skills")).expect("skills").flatten().filter(|e| e.path().join("SKILL.md").is_file()).count();
    let readme = std::fs::read_to_string(format!("{ROOT}/README.md")).expect("README");
    assert!(readme.contains(&format!("its {skills} skills")), "README counts all {skills} skills");
    // The README's words, whatever the line breaks.
    let flat = readme.split_whitespace().filter(|w| *w != ">").collect::<Vec<_>>().join(" ");
    // v0.7: no project folder; `init` is the optional developer mode.
    assert!(flat.contains("No project folder is needed") && flat.contains("Developer mode: tests next to your code (optional)"), "no project folder");
    // The plugin or fastpilot's own registration, not both.
    assert!(flat.contains("Use **either** the plugin **or** the registration fastpilot adds itself, not both: both together start two fastpilot servers"), "plugin or registration");
    // E2: a password is asked once in the dashboard, kept for the app; the environment is for CI.
    assert!(flat.contains("asks for the test account right there, once, in a masked field") && flat.contains("`export DEMO_PASS=...`"), "password once, environment for CI");
}

/// v0.6 1.1: the Integrations page. A new menu item between Device and Settings; `/api/integrations` says what is
/// connected (Jira: site, project, email, never a token; the schedules' webhooks); the bug card
/// no longer has a disabled "Send to Jira" button but says to ask the bot or Claude Code.
#[test]
fn the_apps_page_shows_what_is_connected() {
    let ui = start("apps", &[("FASTPILOT_KEYSTORE", "file")]);
    let p = ui.port;
    let r = http(p, "GET", &format!("/?code={}", ui.code), &[], "");
    let session = r.header("set-cookie").and_then(|c| c.split(';').next().map(str::to_owned)).expect("cookie");
    let get = |path: &str| http(p, "GET", path, &[("Cookie", session.clone())], "");

    let page = get("/").text();
    assert!(page.contains("/assets/apps.js") && page.contains("/assets/shell.js"));
    // FC4: Live, Device and Settings at the foot of the sidebar; Integrations is in Settings.
    let shell = get("/assets/shell.js").text();
    let (device, settings) = (shell.find("\"#/device\"").expect("device"), shell.find("\"#/settings/limits\"").expect("settings"));
    assert!(shell.find("\"#/live\"").expect("live") < device && device < settings, "Live, Device, Settings");
    assert!(get("/assets/app.js").text().contains("h(\"a\", { href: \"#/integrations\", class: cur === \"integrations\""), "Integrations in the Settings menu");
    let js = get("/assets/apps.js");
    assert!(js.status == 200 && js.text().contains("PAGES.integrations") && js.text().contains("Connect Jira"));
    let app = get("/assets/app.js").text();
    assert!(app.contains("Ask the bot or Claude Code to file it in Jira") && !app.contains("\"Send to Jira\""), "the dashboard only connects Jira");

    let v: serde_json::Value = serde_json::from_str(&get("/api/integrations").text()).expect("json");
    assert_eq!(v["jira"], serde_json::Value::Null, "not connected yet: {v}");
    assert_eq!(v["webhook_schedules"], 0);
    assert_eq!(v["token_kept"], "a file only you can read");

    // A saved connection: shown without any secret.
    let dir = ui.home.join(".fastpilot");
    std::fs::create_dir_all(&dir).expect("dir");
    // Connected = settings and a stored token (the file key store of tests).
    std::fs::create_dir_all(dir.join("keys")).expect("keys");
    std::fs::write(dir.join("keys/jira"), "test-token").expect("token");
    std::fs::write(dir.join("keys/jira.origin"), "https://yourteam.atlassian.net").expect("origin");
    std::fs::write(dir.join("jira.json"), r#"{"site":"yourteam.atlassian.net","email":"alex@yourteam.com","project_key":"KAN","project_name":"Kanban","checked_ms":1,"needs_attention":true}"#).expect("jira.json");
    let v: serde_json::Value = serde_json::from_str(&get("/api/integrations").text()).expect("json");
    assert_eq!((v["jira"]["site"].as_str(), v["jira"]["project_key"].as_str(), v["jira"]["needs_attention"].as_bool()), (Some("yourteam.atlassian.net"), Some("KAN"), Some(true)), "{v}");
    assert!(v["jira"].get("token").is_none());
}
