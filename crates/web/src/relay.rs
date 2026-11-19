//! Attach through the fastpilot Chrome extension (`extension-chrome/`): the extension drives
//! the user's tab with `chrome.debugger` and connects to this localhost-only WebSocket relay
//! (`/extension`); fastpilot's own CDP client connects to `/cdp`, and frames are passed
//! through unchanged, so the tab behaves like any page-level CDP endpoint.
//!
//! Needed since Chrome 136 blocks `--remote-debugging-port` on the default profile: the
//! extension works in the user's normal Chrome without restarting it.

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

type Socket = WebSocketStream<TcpStream>;

struct Relay {
    port: u16,
    /// The extension's socket, waiting for fastpilot's CDP client.
    extension: Mutex<Option<Socket>>,
    arrived: Notify,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn relays() -> &'static Mutex<Vec<Arc<Relay>>> {
    static R: OnceLock<Mutex<Vec<Arc<Relay>>>> = OnceLock::new();
    R.get_or_init(Mutex::default)
}

/// Shared secret the extension must send (`?token=`), from `FASTPILOT_RELAY_TOKEN`.
fn token() -> Option<String> {
    fastpilot_core::config::var("FASTPILOT_RELAY_TOKEN").ok().filter(|t| !t.is_empty())
}

async fn start(port: u16) -> Result<Arc<Relay>, Error> {
    if let Some(r) = lock(relays()).iter().find(|r| r.port == port) {
        return Ok(r.clone());
    }
    // Loopback only: nothing off this machine can reach the browser.
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| Error::Launch(format!("extension relay on 127.0.0.1:{port}: {e}")))?;
    let relay = Arc::new(Relay { port, extension: Mutex::new(None), arrived: Notify::new() });
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
    let check = move |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        // Web pages can open WebSockets to localhost too: only the extension
        // (chrome-extension:// origin) or a non-browser client (no Origin) may connect.
        let origin = req.headers().get("origin").and_then(|o| o.to_str().ok()).unwrap_or("");
        if !origin.is_empty() && !origin.starts_with("chrome-extension://") {
            let mut deny = ErrorResponse::new(Some("origin not allowed".into()));
            *deny.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;
            return Err(deny);
        }
        *lock(&seen) = req.uri().to_string();
        Ok(resp)
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, check).await else { return };
    let uri = lock(&path).clone();
    let (route, query) = uri.split_once('?').unwrap_or((uri.as_str(), ""));
    match route {
        "/extension" => {
            let given = query.split('&').find_map(|kv| kv.strip_prefix("token="));
            if token().is_some_and(|t| given != Some(t.as_str())) {
                tracing::warn!("extension relay: wrong token, connection refused");
                return;
            }
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
    if tokio::time::timeout(Duration::from_secs(60), waiting).await.is_err() {
        return Err(Error::Launch(format!(
            "the fastpilot extension did not connect to 127.0.0.1:{port} within 60s: load extension-chrome/ (chrome://extensions, \
             Developer mode, Load unpacked), then click its toolbar button on the tab to drive"
        )));
    }
    Browser::connect_page(&format!("ws://127.0.0.1:{port}/cdp"), "extension").await
}
