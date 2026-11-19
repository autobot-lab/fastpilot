//! Attach through the fastpilot Chrome extension (`extension-chrome/`): the extension drives
//! the user's tab with `chrome.debugger` and connects to this localhost-only WebSocket relay
//! (`/extension`); fastpilot's own CDP client connects to `/cdp`, and frames are passed
//! through unchanged, so the tab behaves like any page-level CDP endpoint.
//!
//! Needed since Chrome 136 blocks `--remote-debugging-port` on the default profile: the
//! extension works in the user's normal Chrome without restarting it.
//!
//! Both routes need a token (`?token=`), new for every session start: any local process could
//! otherwise take over the user's signed-in tab. fastpilot passes it to its own `/cdp` client in
//! memory. The extension gets it through Chrome native messaging ([`native_host`]), which web pages
//! cannot reach: fastpilot writes it to `~/.fastpilot/relay/<port>.token` (owner-only) and registers
//! itself as the native host `dev.fastpilot.relay`, allowed for the fastpilot extension only.
//! `FASTPILOT_RELAY_TOKEN` replaces the random token (the same value goes into the extension's
//! options) where native messaging is not available.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

use crate::{Browser, Error, WebPage};

pub const DEFAULT_PORT: u16 = 9230;

/// Native messaging host name, and the extension it answers (fixed by the `key` in
/// `extension-chrome/manifest.json`).
pub const NATIVE_HOST: &str = "dev.fastpilot.relay";
pub const EXTENSION_ORIGIN: &str = "chrome-extension://iehmgkdbiojkipljgbaljojdmdfpjeeg/";

type Socket = WebSocketStream<TcpStream>;

struct Relay {
    port: u16,
    /// The extension's socket, waiting for fastpilot's CDP client.
    extension: Mutex<Option<Socket>>,
    arrived: Notify,
    /// Required on `/extension` and `/cdp`; replaced at every session start.
    token: Mutex<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn relays() -> &'static Mutex<Vec<Arc<Relay>>> {
    static R: OnceLock<Mutex<Vec<Arc<Relay>>>> = OnceLock::new();
    R.get_or_init(Mutex::default)
}

/// A new token: `FASTPILOT_RELAY_TOKEN` when set, else 256 random bits.
fn new_token() -> Result<String, Error> {
    if let Some(t) = fastpilot_core::config::var("FASTPILOT_RELAY_TOKEN").ok().filter(|t| !t.is_empty()) {
        return Ok(t);
    }
    let mut b = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b))
        .map_err(|e| Error::Launch(format!("extension relay token: {e}")))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// Where the current token of the relay on `port` is kept for the native host.
pub fn token_file(port: u16) -> PathBuf {
    home().join(".fastpilot").join("relay").join(format!("{port}.token"))
}

/// Owner-only (dir 0700, file 0600), written whole (temp file + rename).
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent"))?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let done = o.open(&tmp).and_then(|mut f| f.write_all(text.as_bytes())).and_then(|()| std::fs::rename(&tmp, path));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done
}

/// Register this binary as the native messaging host of the browsers installed for this user
/// (best effort: without it, the extension falls back to the token in its options).
fn register_native_host() {
    let Ok(exe) = std::env::current_exe() else { return };
    let manifest = serde_json::json!({
        "name": NATIVE_HOST,
        "description": "fastpilot: hands the extension relay token to the fastpilot extension",
        "path": exe,
        "type": "stdio",
        "allowed_origins": [EXTENSION_ORIGIN],
    })
    .to_string();
    let h = home();
    let browsers = if cfg!(target_os = "macos") {
        ["Library/Application Support/Google/Chrome", "Library/Application Support/Chromium"]
    } else {
        [".config/google-chrome", ".config/chromium"]
    };
    for b in browsers {
        let dir = h.join(b);
        if dir.is_dir() {
            let file = dir.join("NativeMessagingHosts").join(format!("{NATIVE_HOST}.json"));
            if std::fs::read_to_string(&file).ok().as_deref() != Some(manifest.as_str())
                && let Err(e) = std::fs::create_dir_all(file.parent().unwrap_or(&dir)).and_then(|()| std::fs::write(&file, &manifest))
            {
                tracing::warn!(file = %file.display(), error = %e, "extension relay: native host not registered");
            }
        }
    }
}

/// `fastpilot chrome-extension://<id>/`: Chrome starts fastpilot as the native messaging host for
/// the extension; one request `{ "port": 9230 }` on stdin, the relay token (or an error) back.
pub fn native_host(origin: &str) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let mut stdin = std::io::stdin().lock();
    let mut len = [0u8; 4];
    stdin.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > 4096 {
        return Err(std::io::Error::other("request too large"));
    }
    let mut body = vec![0u8; len];
    stdin.read_exact(&mut body)?;
    let reply = if origin != EXTENSION_ORIGIN {
        serde_json::json!({ "error": "not the fastpilot extension" })
    } else {
        let port = serde_json::from_slice::<serde_json::Value>(&body).ok().and_then(|v| v["port"].as_u64()).and_then(|p| u16::try_from(p).ok());
        match port.map(|p| (p, std::fs::read_to_string(token_file(p)))) {
            Some((_, Ok(t))) => serde_json::json!({ "token": t.trim() }),
            Some((p, Err(_))) => serde_json::json!({ "error": format!("no fastpilot relay is waiting on port {p}: start a session with endpoint: extension first") }),
            None => serde_json::json!({ "error": "request must be { \"port\": <number> }" }),
        }
    };
    let out = reply.to_string();
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&(out.len() as u32).to_le_bytes())?;
    stdout.write_all(out.as_bytes())?;
    stdout.flush()
}

/// A file deleted when this goes out of scope.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Same length and bytes, without an early exit on the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn start(port: u16) -> Result<Arc<Relay>, Error> {
    if let Some(r) = lock(relays()).iter().find(|r| r.port == port) {
        return Ok(r.clone());
    }
    // Loopback only: nothing off this machine can reach the browser.
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| Error::Launch(format!("extension relay on 127.0.0.1:{port}: {e}")))?;
    serve(port, listener)
}

/// The relay on a listener that is already bound (tests bind port 0 and keep it).
fn serve(port: u16, listener: TcpListener) -> Result<Arc<Relay>, Error> {
    let relay = Arc::new(Relay { port, extension: Mutex::new(None), arrived: Notify::new(), token: Mutex::new(new_token()?) });
    lock(relays()).push(relay.clone());
    let r = relay.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let r = r.clone();
            tokio::spawn(async move { accept(r, stream).await });
        }
    });
    Ok(relay)
}

// The handshake callback's signature (and its large error type) is tungstenite's.
#[allow(clippy::result_large_err)]
async fn accept(relay: Arc<Relay>, stream: TcpStream) {
    let path = Arc::new(Mutex::new(String::new()));
    let seen = path.clone();
    let expected = lock(&relay.token).clone();
    let check = move |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        let deny = |why: &str| {
            let mut deny = ErrorResponse::new(Some(why.into()));
            *deny.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;
            Err(deny)
        };
        // Web pages can open WebSockets to localhost too: only the extension
        // (chrome-extension:// origin) or a non-browser client (no Origin) may connect.
        let origin = req.headers().get("origin").and_then(|o| o.to_str().ok()).unwrap_or("");
        if !origin.is_empty() && !origin.starts_with("chrome-extension://") {
            return deny("origin not allowed");
        }
        // And only with this session's token: on both routes, before the upgrade.
        let given = req.uri().query().unwrap_or("").split('&').find_map(|kv| kv.strip_prefix("token=")).unwrap_or("");
        if !same(given, &expected) {
            tracing::warn!(path = req.uri().path(), "extension relay: missing or wrong token, connection refused");
            return deny("token required");
        }
        *lock(&seen) = req.uri().path().to_owned();
        Ok(resp)
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, check).await else { return };
    let route = lock(&path).clone();
    match route.as_str() {
        "/extension" => {
            tracing::info!("fastpilot extension connected");
            *lock(&relay.extension) = Some(ws);
            relay.arrived.notify_waiters();
        }
        "/cdp" => {
            let ext = lock(&relay.extension).take();
            if let Some(ext) = ext {
                pipe(ws, ext).await;
            }
        }
        _ => {}
    }
}

/// Frames both ways until either side closes.
async fn pipe(a: Socket, b: Socket) {
    let (mut a_tx, mut a_rx) = a.split();
    let (mut b_tx, mut b_rx) = b.split();
    let up = async {
        while let Some(Ok(m)) = a_rx.next().await {
            if matches!(m, Message::Text(_) | Message::Binary(_)) && b_tx.send(m).await.is_err() {
                break;
            }
        }
    };
    let down = async {
        while let Some(Ok(m)) = b_rx.next().await {
            if matches!(m, Message::Text(_) | Message::Binary(_)) && a_tx.send(m).await.is_err() {
                break;
            }
        }
    };
    tokio::select! {
        () = up => {}
        () = down => {}
    }
}

/// Session on the tab the user attached with the fastpilot extension: starts the relay on
/// `port` (default 9230) and waits up to 60 s for the extension to connect.
pub async fn extension_page(port: Option<u16>) -> Result<WebPage, Error> {
    let port = port.unwrap_or(DEFAULT_PORT);
    let relay = start(port).await?;
    // A new token for this session, for the extension's next connect (through the native host).
    let token = new_token()?;
    *lock(&relay.token) = token.clone();
    write_private(&token_file(port), &token).map_err(|e| Error::Launch(format!("extension relay token file: {e}")))?;
    // The native host only needs it until the extension connects: removed then, and on every other
    // way out of this wait (timeout, error, the request being dropped).
    let token_on_disk = RemoveOnDrop(token_file(port));
    register_native_host();
    let waiting = async {
        loop {
            let notified = relay.arrived.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if lock(&relay.extension).is_some() {
                return;
            }
            notified.await;
        }
    };
    let waited = tokio::time::timeout(Duration::from_secs(60), waiting).await;
    drop(token_on_disk);
    if waited.is_err() {
        return Err(Error::Launch(format!(
            "the fastpilot extension did not connect to 127.0.0.1:{port} within 60s: load extension-chrome/ (chrome://extensions, \
             Developer mode, Load unpacked), then click its toolbar button on the tab to drive"
        )));
    }
    Browser::connect_page(&format!("ws://127.0.0.1:{port}/cdp?token={token}"), "extension").await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn connect(port: u16, route: &str, token: Option<&str>) -> Option<Socket> {
        let q = token.map(|t| format!("?token={t}")).unwrap_or_default();
        let (ws, _) = tokio_tungstenite::client_async(format!("ws://127.0.0.1:{port}{route}{q}"), TcpStream::connect(("127.0.0.1", port)).await.ok()?)
            .await
            .ok()?;
        Some(ws)
    }

    /// Security review (Medium): any local process could open `/cdp` without a token and drive the
    /// tab the extension attached, and `/extension` only checked a token when one was configured.
    #[tokio::test]
    async fn both_routes_need_the_session_token() {
        // Bound once and kept: probing a free port and binding it again raced other tests.
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let relay = serve(port, listener).expect("relay");
        let token = lock(&relay.token).clone();
        assert_eq!(token.len(), 64, "256 random bits, hex");

        // Without the token, or with a wrong one: refused at the handshake, on both routes.
        for route in ["/extension", "/cdp"] {
            assert!(connect(port, route, None).await.is_none(), "{route} without a token");
            assert!(connect(port, route, Some("0000")).await.is_none(), "{route} with a wrong token");
        }
        // The extension with the token waits; a token-less /cdp cannot take it over.
        let mut ext = connect(port, "/extension", Some(&token)).await.expect("extension with the token");
        assert!(connect(port, "/cdp", None).await.is_none(), "/cdp without a token");
        let quiet = tokio::time::timeout(Duration::from_millis(300), ext.next()).await;
        assert!(quiet.is_err(), "nothing may reach the extension from a refused client: {quiet:?}");
        // fastpilot's own client with the token is piped to the extension.
        let mut cdp = connect(port, "/cdp", Some(&token)).await.expect("/cdp with the token");
        cdp.send(Message::Text("{\"id\":1}".into())).await.expect("send");
        let got = tokio::time::timeout(Duration::from_secs(2), ext.next()).await.expect("frame").expect("open").expect("ok");
        assert_eq!(got.into_text().expect("text").as_str(), "{\"id\":1}");
    }

    /// Security review (Low): the token file stayed in ~/.fastpilot/relay after the session, with a
    /// token anyone reading it could replay while the relay lives. It exists only while fastpilot
    /// waits for the extension.
    #[tokio::test]
    async fn token_file_only_lives_while_waiting_for_the_extension() {
        let _home = HomeGuard::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        serve(port, listener).expect("relay");
        let file = token_file(port);
        let session = tokio::spawn(extension_page(Some(port)));
        let mut token = None;
        for _ in 0..50 {
            token = std::fs::read_to_string(&file).ok();
            if token.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let token = token.expect("token file while waiting");
        let _ext = connect(port, "/extension", Some(token.trim())).await.expect("extension with the token");
        let mut gone = false;
        for _ in 0..50 {
            if !file.exists() {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        session.abort();
        assert!(gone, "token file still there after the extension connected: {}", file.display());
    }

    /// HOME pointed at a temp folder for the token file (restored on drop).
    struct HomeGuard(Option<std::ffi::OsString>, PathBuf);

    impl HomeGuard {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("fastpilot-relay-home-{}", std::process::id()));
            let old = std::env::var_os("HOME");
            // SAFETY: the only test of this crate that reads HOME.
            unsafe { std::env::set_var("HOME", &dir) };
            Self(old, dir)
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            // SAFETY: as above.
            unsafe {
                match &self.0 {
                    Some(h) => std::env::set_var("HOME", h),
                    None => std::env::remove_var("HOME"),
                }
            }
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    #[test]
    fn token_compare_is_exact() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd") && !same("abc", "ab") && !same("", "a"));
    }
}
