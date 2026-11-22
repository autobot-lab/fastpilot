//! PF1 (v0.6) Security review, High: the app map is read-only at the network layer, not only by
//! button labels. A hostile app on 127.0.0.1 has a "Continue" button that sends a POST, a "Next"
//! button that sends a POST and then moves on, a form with only a checkbox and a "Yes" button, a
//! form whose button says "More", a "Home" link to a leave address, a link to another host, and a
//! screen whose only way on is "Continue with Google". The server's own log proves that nothing
//! but GET reached it, that the leave address and the other host were never asked, and the map says
//! how many requests it blocked instead of "nothing was submitted". Skips without Chrome.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists())
}

/// A tiny HTTP server: logs "METHOD /path" and answers with `page(path)`.
fn serve(page: impl Fn(&str) -> String + Send + Sync + 'static) -> (u16, Arc<Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().expect("addr").port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let (log2, page) = (log.clone(), Arc::new(page));
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (log, page) = (log2.clone(), page.clone());
            std::thread::spawn(move || {
                let mut r = BufReader::new(s.try_clone().expect("clone"));
                let mut line = String::new();
                if r.read_line(&mut line).is_err() || line.is_empty() {
                    return;
                }
                let mut h = String::new();
                while r.read_line(&mut h).is_ok_and(|n| n > 2) {
                    h.clear();
                }
                let mut parts = line.split_whitespace();
                let (method, path) = (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("").to_owned());
                log.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{method} {path}"));
                let body = page(path.split('?').next().unwrap_or(&path));
                let mut s = s;
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
            });
        }
    });
    (port, log)
}

#[test]
fn the_map_sends_nothing_that_could_change_something() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let (ext, ext_log) = serve(|_| "<h1>Docs</h1>".to_owned());
    let (port, log) = serve(move |path| {
        let body = match path {
            "/" => format!(
                r#"<h1>Shop</h1>
<a href="/products">Products</a>
<a href="/leave">Home</a>
<a href="http://127.0.0.1:{ext}/docs">Docs</a>
<a href="/account">Account</a>
<button onclick="fetch('/api/subscribe',{{method:'POST'}}).then(()=>location.href='/thanks')">Continue</button>
<button onclick="fetch('/api/track',{{method:'POST',body:'x'}}).finally(()=>{{location.href='/products'}})">Next</button>
<button onclick="navigator.sendBeacon('/api/beacon','x')">Like</button>"#
            ),
            "/products" => r#"<h1>Products</h1>
<form method="post" action="/profile"><label><input type="checkbox" name="news"> News</label> <button>Yes</button></form>
<form method="post" action="/profile2"><button>More</button></form>
<a href="/">Back to the shop</a>"#
                .to_owned(),
            "/account" => r#"<h1>Account</h1><button onclick="location.href='/oauth'">Continue with Google</button> <a href="/">Back</a>"#.to_owned(),
            other => format!("<h1>{}</h1>", other.trim_start_matches('/')),
        };
        format!("<!doctype html><html><head><title>Hostile shop</title></head><body>{body}</body></html>")
    });
    let home = std::env::temp_dir().join(format!("fp-map-guard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["map", &format!("http://127.0.0.1:{port}/"), "--minutes", "3"])
        .current_dir(&home)
        .env("HOME", &home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env_remove("FASTPILOT_ALLOWED_DOMAINS")
        .output()
        .expect("map");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let seen = log.lock().unwrap_or_else(|p| p.into_inner()).clone();

    // Only GETs reached the app; the leave address and the other host were never asked.
    assert!(seen.iter().all(|r| r.starts_with("GET ")), "only GET reached the server: {seen:?}\n{text}");
    assert!(!seen.iter().any(|r| r.contains("/leave") || r.contains("/oauth") || r.contains("/thanks")), "{seen:?}\n{text}");
    assert!(ext_log.lock().unwrap_or_else(|p| p.into_inner()).is_empty(), "the other host was never asked");
    // It moved on where that is harmless: Next led to Products (its POST was refused on the way).
    assert!(seen.iter().any(|r| r == "GET /products") && text.contains("Products"), "{seen:?}\n{text}");
    // Every skipped button and link is named, and the result counts what was blocked.
    for (label, why) in [
        ("Continue", "may change something"),
        ("Like", "may change something"),
        ("Yes", "may change something"),
        ("More", "would have sent something"),
        ("Home", "could change something"),
        ("Docs", "leads outside the app"),
    ] {
        assert!(text.lines().any(|l| l.contains(&format!("skipped \"{label}\"")) && l.contains(why)), "{label}: {why}\n{text}");
    }
    assert!(text.contains("that would have changed something") && text.contains("blocked") && !text.contains("Nothing was typed in or submitted"), "{text}");
    assert!(!text.contains("is blocked"), "the browser's block page is not a screen: {text}");
    // A screen whose only way on is "Continue with Google" is a sign-in wall: nothing pressed.
    assert!(text.contains("\"Account\" needs a sign-in, so the pages behind it are not on the map yet."), "{text}");
    let _ = std::fs::remove_dir_all(&home);
}
