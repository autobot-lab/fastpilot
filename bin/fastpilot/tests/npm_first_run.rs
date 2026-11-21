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

/// The release a stand-in server serves: `/good/v<ver>/<name>.tar.gz(.sha256)`; under `/bad/`
/// the checksum is wrong. Returns the base URL and the request counter.
fn serve(dir: &Path, name: &str) -> (String, Arc<AtomicUsize>) {
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
    Command::new("node").arg(pkg.join("bin/fastpilot.js")).args(args).env("FASTPILOT_DOWNLOAD_BASE", base).env_remove("FASTPILOT_SKIP_DOWNLOAD").output().expect("node")
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
        assert!(err.contains("npm config set allow-scripts=fastpilot --location=user") && err.contains("scripts/install.sh") && !err.contains("    at "), "{err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
