//! A1 NPM12 (v0.4.2): npm 12 skips install scripts unless allowed, so `postinstall` never ran
//! and `fastpilot` failed with `spawnSync … vendor/fastpilot ENOENT`. The npm shim
//! (`packaging/npm/bin/fastpilot.js`) now downloads the binary on the first run. Played with the
//! real package files, `node`, and a local server serving a stand-in release (a small script as
//! the "binary"): the first run downloads and checks it, later runs make no request, stdout stays
//! the program's own (MCP), a bad checksum installs nothing, two first runs at once both work, and
//! a read-only folder gets a message with the fix. Skips without `node` or on a platform the
//! package has no binary for.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const PKG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packaging/npm");
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        _ => None,
    }
}

/// A stand-in release archive `<name>.tar.gz` holding a small script as the "binary", and its
/// SHA-256.
fn release(dir: &Path, name: &str) -> (Vec<u8>, String) {
    let stage = dir.join("stage").join(name);
    std::fs::create_dir_all(&stage).expect("stage");
    let fake = format!("#!/bin/sh\nif [ \"$1\" = mcp ]; then echo MCP-OUT; else echo \"fastpilot {VERSION}\"; fi\n");
    std::fs::write(stage.join("fastpilot"), fake).expect("fake binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(stage.join("fastpilot"), std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let archive = dir.join(format!("{name}.tar.gz"));
    assert!(Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(dir.join("stage")).arg(name).status().expect("tar").success());
    let bytes = std::fs::read(&archive).expect("archive");
    let sha = {
        let out = Command::new("shasum").args(["-a", "256"]).arg(&archive).output().or_else(|_| Command::new("sha256sum").arg(&archive).output()).expect("sha");
        String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or_default().to_owned()
    };
    (bytes, sha)
}

/// The release a stand-in server serves: `/good/v<ver>/<name>.tar.gz(.sha256)`; under `/bad/`
/// the checksum is wrong. Returns the base URL and the request counter.
fn serve(dir: &Path, name: &str) -> (String, Arc<AtomicUsize>) {
    let (bytes, sha) = release(dir, name);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let (h, name) = (hits.clone(), name.to_owned());
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let mut s = s;
            let mut line = String::new();
            let mut r = BufReader::new(s.try_clone().expect("clone"));
            if r.read_line(&mut line).is_err() {
                continue;
            }
            loop {
                let mut l = String::new();
                if r.read_line(&mut l).map(|n| n == 0 || l == "\r\n").unwrap_or(true) {
                    break;
                }
            }
            h.fetch_add(1, Ordering::SeqCst);
            let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
            if path.starts_with("/slow/") {
                // A server that never answers: the shim gives up after FASTPILOT_DOWNLOAD_TIMEOUT.
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    drop(s);
                });
                continue;
            }
            let body: Option<Vec<u8>> = [("good", sha.clone()), ("bad", "0".repeat(64))].iter().find_map(|(p, sum)| {
                if path == format!("/{p}/v{VERSION}/{name}.tar.gz") {
                    Some(bytes.clone())
                } else if path == format!("/{p}/v{VERSION}/{name}.tar.gz.sha256") {
                    Some(format!("{sum}  {name}.tar.gz\n").into_bytes())
                } else {
                    None
                }
            });
            let head = match &body {
                Some(b) => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len()),
                None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            };
            let _ = s.write_all(head.as_bytes());
            if let Some(b) = body {
                let _ = s.write_all(&b);
            }
        }
    });
    (format!("http://127.0.0.1:{port}"), hits)
}

/// A copy of the npm package as npm 12 leaves it: the files only, no `vendor/`.
fn package(at: &Path) -> PathBuf {
    let p = at.join("node_modules/fastpilot");
    for f in ["package.json", "install.js", "README.md", "bin/fastpilot.js", "lib/download.js"] {
        let src = Path::new(PKG).join(f);
        let dst = p.join(f);
        std::fs::create_dir_all(dst.parent().expect("dir")).expect("dir");
        std::fs::copy(&src, &dst).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
    }
    p
}

fn shim(pkg: &Path, base: &str, args: &[&str]) -> std::process::Output {
    node(pkg, "bin/fastpilot.js", base, args)
}

/// `node <pkg>/<script>` with a throwaway HOME and no NODE_OPTIONS: no ~/.npmrc or
/// `--dns-result-order` from this computer changes how the download connects.
fn node(pkg: &Path, script: &str, base: &str, args: &[&str]) -> std::process::Output {
    Command::new("node")
        .arg(pkg.join(script))
        .args(args)
        .env("HOME", pkg.parent().expect("parent"))
        .env_remove("NODE_OPTIONS")
        .env("FASTPILOT_DOWNLOAD_BASE", base)
        .env_remove("FASTPILOT_SKIP_DOWNLOAD")
        .output()
        .expect("node")
}

fn vendor(pkg: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(pkg.join("vendor")).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    v.sort();
    v
}

#[test]
fn the_npm_shim_downloads_the_binary_on_the_first_run() {
    let (Some(target), true) = (target(), Command::new("node").arg("--version").output().is_ok()) else {
        return eprintln!("skipping: no node, or no prebuilt binary for this platform");
    };
    let dir = std::env::temp_dir().join(format!("fp-npm12-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let name = format!("fastpilot-{VERSION}-{target}");
    let (url, hits) = serve(&dir, &name);
    let good = format!("{url}/good");
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();

    // First run: downloads (archive + checksum), says so on stderr only, then runs it.
    let pkg = package(&dir.join("a"));
    let out = shim(&pkg, &good, &["--version"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), format!("fastpilot {VERSION}\n"), "stdout is the program's only");
    assert!(text(&out.stderr).contains("downloading fastpilot") && text(&out.stderr).contains("installed"), "{}", text(&out.stderr));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(vendor(&pkg), ["VERSION", "fastpilot"], "no temp files left");
    // Later runs: no request at all.
    let out = shim(&pkg, &good, &["--version"]);
    assert!(out.status.success() && text(&out.stderr).is_empty(), "{}", text(&out.stderr));
    assert_eq!(hits.load(Ordering::SeqCst), 2, "no network on later runs");

    // MCP through the shim: stdout carries only what the program writes.
    let pkg = package(&dir.join("mcp"));
    let out = shim(&pkg, &good, &["mcp"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "MCP-OUT\n");

    // A wrong checksum: refused, nothing installed.
    let pkg = package(&dir.join("bad"));
    let out = shim(&pkg, &format!("{url}/bad"), &["--version"]);
    assert!(!out.status.success() && text(&out.stderr).contains("checksum mismatch") && out.stdout.is_empty(), "{}", text(&out.stderr));
    assert!(vendor(&pkg).iter().all(|f| f != "fastpilot" && !f.ends_with(".part")), "{:?}", vendor(&pkg));

    // Two first runs at once: both work, one binary, no temp files.
    let pkg = package(&dir.join("race"));
    let runs: Vec<_> = (0..2)
        .map(|_| Command::new("node").arg(pkg.join("bin/fastpilot.js")).arg("--version").env("FASTPILOT_DOWNLOAD_BASE", &good).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().expect("node"))
        .collect();
    for r in runs {
        let out = r.wait_with_output().expect("wait");
        assert!(out.status.success(), "{}", text(&out.stderr));
        assert_eq!(text(&out.stdout), format!("fastpilot {VERSION}\n"));
    }
    assert_eq!(vendor(&pkg), ["VERSION", "fastpilot"]);

    // A folder the user may not write to (a root-owned global prefix): the fix, not a stack trace.
    #[cfg(unix)]
    if !Command::new("id").arg("-u").output().is_ok_and(|o| text(&o.stdout).trim() == "0") {
        use std::os::unix::fs::PermissionsExt;
        let pkg = package(&dir.join("ro"));
        std::fs::set_permissions(&pkg, std::fs::Permissions::from_mode(0o555)).expect("chmod");
        let out = shim(&pkg, &good, &["--version"]);
        std::fs::set_permissions(&pkg, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let err = text(&out.stderr);
        assert!(!out.status.success() && out.stdout.is_empty(), "{err}");
        assert!(err.contains("npm config set allow-scripts=fastpilot --location=user") && err.contains(&format!("/v{VERSION}/scripts/install.sh")) && !err.contains("    at "), "{err}");
    }

    // Security review of #121, Low 4: a download that hangs ends with a message, and a mirror
    // must use https unless it is this computer.
    let pkg = package(&dir.join("slow"));
    let started = std::time::Instant::now();
    let out = Command::new("node").arg(pkg.join("bin/fastpilot.js")).arg("--version").env("FASTPILOT_DOWNLOAD_BASE", format!("{url}/slow")).env("FASTPILOT_DOWNLOAD_TIMEOUT", "2").output().expect("node");
    assert!(!out.status.success() && text(&out.stderr).contains("no answer within 2 s") && out.stdout.is_empty(), "{}", text(&out.stderr));
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "it gave up in time");
    assert!(vendor(&pkg).iter().all(|f| f != "fastpilot"));
    let pkg = package(&dir.join("plain-http"));
    let before = hits.load(Ordering::SeqCst);
    let out = shim(&pkg, "http://mirror.example.com/releases", &["--version"]);
    assert!(!out.status.success() && text(&out.stderr).contains("must use https"), "{}", text(&out.stderr));
    assert_eq!(hits.load(Ordering::SeqCst), before, "nothing fetched");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stand-in GitHub on a path that cuts downloads. On 127.0.0.1 the first `cuts` requests end in
/// a TCP reset: the first before any answer, the next ones halfway through the file. Later
/// requests get `/v<ver>/<name>.tar.gz` and its `.sha256` (`sha`). With `v6`, [::1] on the same
/// port resets every request: an IPv6 path that cuts every download while IPv4 works (NAT64 on
/// the user's Mac, v0.6.0). Returns the port and the request counts on IPv4 and IPv6 (`None`
/// when [::1] is not available here).
fn serve_cutting(bytes: Vec<u8>, sha: String, name: String, cuts: usize, v6: bool) -> (u16, Arc<AtomicUsize>, Option<Arc<AtomicUsize>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn path(s: &mut tokio::net::TcpStream) -> String {
        let mut head = Vec::new();
        let mut buf = [0u8; 1024];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => head.extend_from_slice(&buf[..n]),
            }
        }
        String::from_utf8_lossy(&head).split_whitespace().nth(1).unwrap_or("").to_owned()
    }
    fn reset(s: tokio::net::TcpStream) {
        s.set_zero_linger().expect("linger");
        drop(s);
    }
    let v4 = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = v4.local_addr().expect("addr").port();
    let v6 = if v6 { TcpListener::bind(("::1", port)).ok() } else { None };
    let hits4 = Arc::new(AtomicUsize::new(0));
    let hits6 = v6.as_ref().map(|_| Arc::new(AtomicUsize::new(0)));
    let (h4, h6) = (hits4.clone(), hits6.clone());
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        rt.block_on(async move {
            if let (Some(l), Some(h6)) = (v6, h6) {
                l.set_nonblocking(true).expect("nonblocking");
                let l = tokio::net::TcpListener::from_std(l).expect("listener");
                tokio::spawn(async move {
                    while let Ok((mut s, _)) = l.accept().await {
                        path(&mut s).await;
                        h6.fetch_add(1, Ordering::SeqCst);
                        reset(s);
                    }
                });
            }
            v4.set_nonblocking(true).expect("nonblocking");
            let v4 = tokio::net::TcpListener::from_std(v4).expect("listener");
            while let Ok((mut s, _)) = v4.accept().await {
                let p = path(&mut s).await;
                let n = h4.fetch_add(1, Ordering::SeqCst) + 1;
                let body = if p == format!("/v{VERSION}/{name}.tar.gz") {
                    Some(bytes.clone())
                } else if p == format!("/v{VERSION}/{name}.tar.gz.sha256") {
                    Some(format!("{sha}  {name}.tar.gz\n").into_bytes())
                } else {
                    None
                };
                let Some(body) = body else {
                    let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    continue;
                };
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                if n > cuts {
                    let _ = s.write_all(head.as_bytes()).await;
                    let _ = s.write_all(&body).await;
                    continue;
                }
                if n > 1 {
                    // A body cut short: the answer starts, then the connection is reset.
                    let _ = s.write_all(head.as_bytes()).await;
                    let _ = s.write_all(&body[..body.len() / 2]).await;
                    let _ = s.flush().await;
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                reset(s);
            }
        });
    });
    (port, hits4, hits6)
}

/// v0.6.1 hotfix: `npm install -g fastpilot` ended in "fetch failed" on an IPv6 (NAT64) network
/// that reset Node's download from GitHub about half the time, while IPv4 worked. A cut download
/// is now tried again (3 tries), over IPv4 after the first cut; the checksum is still checked; a
/// path that always cuts ends with plain words and leaves nothing in vendor/.
#[test]
fn a_cut_download_is_tried_again_over_ipv4_and_then_explained() {
    let (Some(target), true) = (target(), Command::new("node").arg("--version").output().is_ok()) else {
        return eprintln!("skipping: no node, or no prebuilt binary for this platform");
    };
    let dir = std::env::temp_dir().join(format!("fp-npm-cut-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let name = format!("fastpilot-{VERSION}-{target}");
    let (bytes, sha) = release(&dir, &name);
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();

    // Cut twice (a reset, then a body cut short), then served: postinstall installs it, checked.
    let (port, hits, _) = serve_cutting(bytes.clone(), sha.clone(), name.clone(), 2, false);
    let base = format!("http://127.0.0.1:{port}");
    let pkg = package(&dir.join("cut-twice"));
    let out = node(&pkg, "install.js", &base, &[]);
    let err = text(&out.stderr);
    assert!(out.status.success(), "installed after two cuts: {err}");
    assert!(err.contains("trying again over IPv4 (2 of 3)") && err.contains("trying again over IPv4 (3 of 3)") && err.contains("installed"), "{err}");
    assert!(out.stdout.is_empty(), "stdout stays clean: {}", text(&out.stdout));
    assert_eq!(hits.load(Ordering::SeqCst), 4, "archive 3 tries, checksum once");
    assert_eq!(vendor(&pkg), ["VERSION", "fastpilot"], "no temp files left");
    assert_eq!(std::fs::read(pkg.join("vendor/fastpilot")).expect("binary"), std::fs::read(dir.join("stage").join(&name).join("fastpilot")).expect("staged"), "the checked file is the one installed");
    let out = shim(&pkg, &base, &["--version"]);
    assert_eq!(text(&out.stdout), format!("fastpilot {VERSION}\n"));
    assert_eq!(hits.load(Ordering::SeqCst), 4, "no network on later runs");

    // A retry is checked like a first try: a wrong checksum after a cut installs nothing.
    let (port, _, _) = serve_cutting(bytes.clone(), "0".repeat(64), name.clone(), 1, false);
    let pkg = package(&dir.join("cut-bad-sum"));
    let out = shim(&pkg, &format!("http://127.0.0.1:{port}"), &["--version"]);
    assert!(!out.status.success() && text(&out.stderr).contains("checksum mismatch") && out.stdout.is_empty(), "{}", text(&out.stderr));
    assert!(vendor(&pkg).is_empty(), "{:?}", vendor(&pkg));

    // Always cut (first run of the npm shim): three tries, then plain words; nothing installed.
    let (port, hits, _) = serve_cutting(bytes.clone(), sha.clone(), name.clone(), usize::MAX, false);
    let pkg = package(&dir.join("always-cut"));
    let started = std::time::Instant::now();
    let out = shim(&pkg, &format!("http://127.0.0.1:{port}"), &["--version"]);
    let err = text(&out.stderr);
    assert!(!out.status.success() && out.stdout.is_empty(), "{err}");
    assert!(err.contains("could not download") && err.contains("3 tries") && err.contains("Check your Wi-Fi or VPN first, then try again"), "{err}");
    assert!(!err.contains("    at ") && !err.contains("TypeError"), "no stack trace: {err}");
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "a short pause between tries");
    assert!(vendor(&pkg).is_empty(), "nothing in vendor/: {:?}", vendor(&pkg));

    // IPv6 cuts every download, IPv4 works (the user's Mac): the IPv4 retry gets the file.
    let (port, hits, hits6) = serve_cutting(bytes, sha, name, 0, true);
    if let Some(hits6) = hits6 {
        let pkg = package(&dir.join("v6-cuts"));
        let out = shim(&pkg, &format!("http://localhost:{port}"), &["--version"]);
        let err = text(&out.stderr);
        assert!(out.status.success(), "IPv4 after the IPv6 cut: {err}");
        assert_eq!(text(&out.stdout), format!("fastpilot {VERSION}\n"));
        assert!(hits6.load(Ordering::SeqCst) <= 1, "after the first cut, IPv4 first: {err}");
        assert_eq!(hits.load(Ordering::SeqCst), 2, "{err}");
        eprintln!("IPv6 cuts: {} (localhost tried IPv6 first: {})", hits6.load(Ordering::SeqCst), err.contains("over IPv4"));
    } else {
        eprintln!("skipping the IPv6 part: [::1] is not available here");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stand-in server on 127.0.0.1 that answers every connection with `answer` after `wait`, then
/// resets the connection (`answer` empty: just the reset). Returns the port and the count.
fn serve_raw(answer: &'static [u8], wait: std::time::Duration) -> (u16, Arc<AtomicUsize>) {
    use tokio::io::AsyncWriteExt;
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().expect("addr").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        rt.block_on(async move {
            l.set_nonblocking(true).expect("nonblocking");
            let l = tokio::net::TcpListener::from_std(l).expect("listener");
            while let Ok((mut s, _)) = l.accept().await {
                h.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    tokio::time::sleep(wait).await;
                    let _ = s.write_all(answer).await;
                    let _ = s.flush().await;
                    s.set_zero_linger().expect("linger");
                    drop(s);
                });
            }
        });
    });
    (port, hits)
}

/// v0.7 (Notion L062-6, -7, -8, -14), after the Security review of #161:
/// - a mirror address with a user name or password is refused, and never repeated in a message;
/// - one deadline for the whole download, every try included;
/// - only network cuts are tried again (a TLS failure is not);
/// - the last message says to check Wi-Fi or VPN first.
#[test]
fn the_download_refuses_a_login_in_the_mirror_and_stays_bounded() {
    if target().is_none() {
        return eprintln!("skipping: no prebuilt target for this platform");
    }
    let dir = std::env::temp_dir().join(format!("fp-npm-harden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    // L062-6: a login in the address: refused before any request, and not echoed.
    let (port, hits) = serve_raw(b"", std::time::Duration::ZERO);
    let pkg = package(&dir.join("login"));
    let out = shim(&pkg, &format!("https://ann:s3cret-pw@127.0.0.1:{port}/releases"), &["--version"]);
    let err = text(&out.stderr);
    if err.contains("command not found") || err.is_empty() && out.status.code().is_none() {
        return eprintln!("skipping: no node");
    }
    assert!(!out.status.success() && err.contains("has a user name or password in it, so it was not used"), "{err}");
    assert!(!err.contains("s3cret-pw") && !err.contains("ann:"), "the login is never shown: {err}");
    assert_eq!(hits.load(Ordering::SeqCst), 0, "nothing was requested");
    let out = shim(&pkg, "not a url s3cret-pw", &["--version"]);
    assert!(!text(&out.stderr).contains("s3cret-pw"), "{}", text(&out.stderr));
    // L062-8: https to a server that does not speak TLS: one try, not three.
    let (port, hits) = serve_raw(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n", std::time::Duration::ZERO);
    let pkg = package(&dir.join("tls"));
    let out = shim(&pkg, &format!("https://127.0.0.1:{port}"), &["--version"]);
    let err = text(&out.stderr);
    assert!(!out.status.success() && err.contains("could not download") && !err.contains("3 tries") && !err.contains("trying again"), "{err}");
    assert!(err.contains("Check your Wi-Fi or VPN first, then try again"), "L062-14: {err}");
    assert_eq!(hits.load(Ordering::SeqCst), 1, "a TLS failure is not tried again: {err}");
    // L062-7: every try is cut after 1.5 s and the whole download may take 2 s: it stops at the
    // deadline after the first cut, instead of three tries of their own 2 s each.
    let (port, hits) = serve_raw(b"", std::time::Duration::from_millis(1500));
    let pkg = package(&dir.join("deadline"));
    let started = std::time::Instant::now();
    let out = Command::new("node")
        .arg(pkg.join("bin/fastpilot.js"))
        .arg("--version")
        .env("HOME", pkg.parent().expect("parent"))
        .env_remove("NODE_OPTIONS")
        .env("FASTPILOT_DOWNLOAD_BASE", format!("http://127.0.0.1:{port}"))
        .env("FASTPILOT_DOWNLOAD_TIMEOUT", "2")
        .output()
        .expect("node");
    let err = text(&out.stderr);
    assert!(!out.status.success() && err.contains("no answer within 2 s (all tries together)") && err.contains("Check your Wi-Fi or VPN first"), "{err}");
    assert!(started.elapsed() < std::time::Duration::from_secs(4), "one deadline for all tries: {:?} {err}", started.elapsed());
    assert_eq!(hits.load(Ordering::SeqCst), 1, "{err}");
    assert!(vendor(&pkg).iter().all(|f| f != "fastpilot"));
    let _ = std::fs::remove_dir_all(&dir);
}
