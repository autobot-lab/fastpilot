//! Web Inspector client for WKWebView content: the WIR protocol Safari's Develop menu speaks.
//!
//! - Simulator: the `com.apple.webinspectord_sim.socket` unix socket its launchd_sim holds.
//! - Device: lockdown service `com.apple.webinspector` through usbmux, TLS with this Mac's pair
//!   record (lockdownd requires it on the session and the service; rustls, already in the
//!   dependency tree through reqwest). Not a developer-disk-image service, so iOS 17+ still
//!   offers it over lockdown; the RemoteXPC (`.shim.remote`) path is not implemented.
//!
//! Wire: 4-byte big-endian length + binary plist `{__selector, __argument}`. Inspector protocol
//! JSON travels in `WIRSocketDataKey` / `WIRMessageDataKey`; target-based WebKit (iOS 12.2+)
//! wraps page commands in `Target.sendMessageToTarget`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, WriteHalf};
use tokio::sync::{Notify, oneshot};
use tokio_rustls::rustls;

use crate::usbmux;

/// What the app has to do for its web content to be inspectable.
pub const HINT: &str = "set `webView.isInspectable = true` on the WKWebView (iOS 16.4+, e.g. in debug builds); \
on an iPhone also turn on Settings → Safari (Apps → Safari on iOS 18+) → Advanced → Web Inspector";

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    /// The app is not listed, or lists no web page: nothing to inspect.
    #[error("{0}")]
    NotInspectable(String),
    #[error("web inspector: {0}")]
    Other(String),
}

fn other(e: impl std::fmt::Display) -> Error {
    Error::Other(e.to_string())
}

pub trait Io: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Io for T {}
pub type Stream = Box<dyn Io>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

async fn send_plist<W: AsyncWrite + Unpin + ?Sized>(w: &mut W, d: &plist::Dictionary, binary: bool) -> Result<(), Error> {
    let mut body = Vec::new();
    if binary { plist::to_writer_binary(&mut body, d) } else { plist::to_writer_xml(&mut body, d) }.map_err(other)?;
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    out.extend(body);
    w.write_all(&out).await.map_err(other)
}

async fn read_plist<R: AsyncRead + Unpin + ?Sized>(r: &mut R) -> Result<plist::Dictionary, Error> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await.map_err(other)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > 64 << 20 {
        return Err(other(format!("message of {len} bytes")));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await.map_err(other)?;
    plist::from_bytes(&body).map_err(other)
}

fn str_of<'a>(d: &'a plist::Dictionary, k: &str) -> Option<&'a str> {
    d.get(k).and_then(plist::Value::as_string)
}

fn int_of(d: &plist::Dictionary, k: &str) -> Option<i64> {
    let v = d.get(k)?;
    v.as_signed_integer().or_else(|| v.as_unsigned_integer().and_then(|u| i64::try_from(u).ok()))
}

fn uuid() -> Result<String, Error> {
    let h = crate::token().map_err(other)?;
    Ok(format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]).to_uppercase())
}

#[derive(Default)]
struct State {
    /// WIRApplicationIdentifierKey → (bundle id, host application id when it is a proxy).
    apps: HashMap<String, (String, Option<String>)>,
    /// Application id → its web pages (WIRPageIdentifierKey, URL).
    pages: HashMap<String, Vec<(i64, String)>>,
    /// The page target, on target-based WebKit.
    target: Option<String>,
    /// Command id → (sent wrapped in Target.sendMessageToTarget, answer).
    pending: HashMap<i64, (bool, oneshot::Sender<Result<Value, String>>)>,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Notify,
}

fn app(d: &plist::Dictionary) -> Option<(String, (String, Option<String>))> {
    let id = str_of(d, "WIRApplicationIdentifierKey")?.to_owned();
    let bundle = str_of(d, "WIRApplicationBundleIdentifierKey").unwrap_or("").to_owned();
    Some((id, (bundle, str_of(d, "WIRHostApplicationIdentifierKey").map(str::to_owned))))
}

fn handle(sh: &Shared, msg: &plist::Dictionary) {
    let empty = plist::Dictionary::new();
    let arg = msg.get("__argument").and_then(plist::Value::as_dictionary).unwrap_or(&empty);
    let mut st = lock(&sh.state);
    match str_of(msg, "__selector").unwrap_or("") {
        "_rpc_reportConnectedApplicationList:" => {
            let list = arg.get("WIRApplicationDictionaryKey").and_then(plist::Value::as_dictionary);
            st.apps = list.into_iter().flat_map(|d| d.values()).filter_map(|v| app(v.as_dictionary()?)).collect();
        }
        "_rpc_applicationConnected:" | "_rpc_applicationUpdated:" => {
            if let Some((id, a)) = app(arg) {
                st.apps.insert(id, a);
            }
        }
        "_rpc_applicationDisconnected:" => {
            if let Some(id) = str_of(arg, "WIRApplicationIdentifierKey") {
                st.apps.remove(id);
                st.pages.remove(id);
            }
        }
        "_rpc_applicationSentListing:" => {
            if let Some(id) = str_of(arg, "WIRApplicationIdentifierKey") {
                let listing = arg.get("WIRListingKey").and_then(plist::Value::as_dictionary);
                let pages = listing
                    .into_iter()
                    .flat_map(|d| d.values())
                    .filter_map(|p| {
                        let p = p.as_dictionary()?;
                        // JSContexts, service workers and automation targets are not pages.
                        if !matches!(str_of(p, "WIRTypeKey").unwrap_or("WIRTypeWeb"), "WIRTypeWeb" | "WIRTypeWebPage") {
                            return None;
                        }
                        Some((int_of(p, "WIRPageIdentifierKey")?, str_of(p, "WIRURLKey").unwrap_or("").to_owned()))
                    })
                    .collect();
                st.pages.insert(id.to_owned(), pages);
            }
        }
        "_rpc_applicationSentData:" => {
            let data = match arg.get("WIRMessageDataKey") {
                Some(plist::Value::Data(d)) => d.clone(),
                Some(plist::Value::String(s)) => s.clone().into_bytes(),
                _ => Vec::new(),
            };
            if let Ok(v) = serde_json::from_slice::<Value>(&data) {
                dispatch(&mut st, v);
            }
        }
        _ => {}
    }
    drop(st);
    sh.changed.notify_waiters();
}

fn dispatch(st: &mut State, v: Value) {
    match v["method"].as_str() {
        Some("Target.targetCreated") => {
            let t = &v["params"]["targetInfo"];
            if t["type"] == "page" && t["isProvisional"] != true {
                st.target = t["targetId"].as_str().map(str::to_owned);
            }
        }
        // A navigation that swapped processes: commands go to the new target.
        Some("Target.didCommitProvisionalTarget") => st.target = v["params"]["newTargetId"].as_str().map(str::to_owned),
        Some("Target.dispatchMessageFromTarget") => {
            if let Some(inner) = v["params"]["message"].as_str().and_then(|m| serde_json::from_str::<Value>(m).ok()) {
                answer(st, inner, true);
            }
        }
        Some(_) => {}
        None => answer(st, v, false),
    }
}

fn answer(st: &mut State, v: Value, inner: bool) {
    let Some(id) = v["id"].as_i64() else { return };
    // The outer Target.sendMessageToTarget ack shares the id: only its error counts.
    if !inner && st.pending.get(&id).is_some_and(|p| p.0) && v.get("error").is_none() {
        return;
    }
    if let Some((_, tx)) = st.pending.remove(&id) {
        let _ = tx.send(match v.get("error") {
            Some(e) => Err(e["message"].as_str().unwrap_or("error").to_owned()),
            None => Ok(v["result"].clone()),
        });
    }
}

/// One inspector connection attached to one web page of an app.
pub struct Session {
    shared: Arc<Shared>,
    writer: tokio::sync::Mutex<WriteHalf<Stream>>,
    conn: String,
    sender: String,
    app: String,
    page: i64,
    next: std::sync::atomic::AtomicI64,
    reader: tokio::task::JoinHandle<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl Session {
    /// Attach to the web page of `bundle` (the app, or a WebContent proxy it hosts).
    /// ponytail: the first page listed wins (a non-blank one first); several WKWebViews in
    /// one app would need matching on the on-screen URL.
    pub async fn open(io: Stream, bundle: &str) -> Result<Self, Error> {
        let (mut rd, wr) = tokio::io::split(io);
        let shared = Arc::new(Shared::default());
        let sh = shared.clone();
        let reader = tokio::spawn(async move {
            while let Ok(msg) = read_plist(&mut rd).await {
                handle(&sh, &msg);
            }
            let mut st = lock(&sh.state);
            st.closed = true;
            for (_, (_, tx)) in st.pending.drain() {
                let _ = tx.send(Err("inspector connection closed".into()));
            }
            drop(st);
            sh.changed.notify_waiters();
        });
        let mut s = Session {
            shared,
            writer: tokio::sync::Mutex::new(wr),
            conn: uuid()?,
            sender: uuid()?,
            app: String::new(),
            page: 0,
            next: 1.into(),
            reader,
        };
        s.rpc("_rpc_reportIdentifier:", plist::Dictionary::new()).await?;
        s.rpc("_rpc_getConnectedApplications:", plist::Dictionary::new()).await?;
        let apps = s
            .wait(Duration::from_secs(2), |st| {
                let mine: Vec<String> = st.apps.iter().filter(|(_, (b, _))| b == bundle).map(|(id, _)| id.clone()).collect();
                let proxies = st.apps.iter().filter(|(_, (_, host))| host.as_ref().is_some_and(|h| mine.contains(h))).map(|(id, _)| id.clone());
                let all: Vec<String> = mine.iter().cloned().chain(proxies).collect();
                (!all.is_empty()).then_some(all)
            })
            .await
            .ok_or_else(|| Error::NotInspectable(format!("{bundle} has no inspectable web content: {HINT}")))?;
        for a in &apps {
            let mut arg = plist::Dictionary::new();
            arg.insert("WIRApplicationIdentifierKey".into(), a.as_str().into());
            s.rpc("_rpc_forwardGetListing:", arg).await?;
        }
        let (app, page) = s
            .wait(Duration::from_secs(2), |st| {
                let pages: Vec<_> = apps.iter().flat_map(|a| st.pages.get(a).into_iter().flatten().map(move |p| (a, p))).collect();
                let best = pages.iter().find(|(_, (_, url))| !url.is_empty() && url != "about:blank").or(pages.first());
                best.map(|(a, (id, _))| ((*a).clone(), *id))
            })
            .await
            .ok_or_else(|| Error::NotInspectable(format!("{bundle} lists no inspectable WKWebView page: {HINT}")))?;
        s.app = app;
        s.page = page;
        let mut arg = s.page_arg();
        arg.insert("WIRAutomaticallyPause".into(), false.into());
        s.rpc("_rpc_forwardSocketSetup:", arg).await?;
        // Target-based WebKit announces the page target right away; older versions take
        // commands directly.
        let _ = s.wait(Duration::from_millis(800), |st| st.target.clone()).await;
        Ok(s)
    }

    fn page_arg(&self) -> plist::Dictionary {
        let mut d = plist::Dictionary::new();
        d.insert("WIRApplicationIdentifierKey".into(), self.app.as_str().into());
        d.insert("WIRPageIdentifierKey".into(), plist::Value::Integer(self.page.into()));
        d.insert("WIRSenderKey".into(), self.sender.as_str().into());
        d
    }

    async fn rpc(&self, selector: &str, mut arg: plist::Dictionary) -> Result<(), Error> {
        arg.insert("WIRConnectionIdentifierKey".into(), self.conn.as_str().into());
        let mut d = plist::Dictionary::new();
        d.insert("__selector".into(), selector.into());
        d.insert("__argument".into(), plist::Value::Dictionary(arg));
        send_plist(&mut *self.writer.lock().await, &d, true).await
    }

    async fn wait<T>(&self, max: Duration, f: impl Fn(&State) -> Option<T>) -> Option<T> {
        let waiting = async {
            loop {
                let n = self.shared.changed.notified();
                tokio::pin!(n);
                n.as_mut().enable();
                {
                    let st = lock(&self.shared.state);
                    if let Some(v) = f(&st) {
                        return Some(v);
                    }
                    if st.closed {
                        return None;
                    }
                }
                n.await;
            }
        };
        tokio::time::timeout(max, waiting).await.ok().flatten()
    }

    pub fn alive(&self) -> bool {
        !lock(&self.shared.state).closed
    }

    /// One Web Inspector protocol command on the page; its `result`.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, Error> {
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let inner = json!({ "id": id, "method": method, "params": params }).to_string();
        let (tx, rx) = oneshot::channel();
        let target = {
            let mut st = lock(&self.shared.state);
            if st.closed {
                return Err(other("inspector connection closed"));
            }
            let t = st.target.clone();
            st.pending.insert(id, (t.is_some(), tx));
            t
        };
        let data = match target {
            Some(t) => json!({ "id": id, "method": "Target.sendMessageToTarget", "params": { "targetId": t, "message": inner } }).to_string(),
            None => inner,
        };
        let mut arg = self.page_arg();
        arg.insert("WIRSocketDataKey".into(), plist::Value::Data(data.into_bytes()));
        self.rpc("_rpc_forwardSocketData:", arg).await?;
        match tokio::time::timeout(Duration::from_secs(15), rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(other(format!("{method}: {e}"))),
            Ok(Err(_)) => Err(other("inspector connection closed")),
            Err(_) => {
                lock(&self.shared.state).pending.remove(&id);
                Err(other(format!("{method}: no answer within 15 s")))
            }
        }
    }
}

/// The `webinspectord_sim` socket of simulator `udid`: `lsof` on launchd_sim processes,
/// matched by the simulator's device path in launchd_sim's command line (or the only one).
/// `FASTPILOT_WEBINSPECTOR_SOCKET` overrides.
pub async fn simulator_socket(udid: &str) -> Result<PathBuf, Error> {
    if let Some(p) = std::env::var_os("FASTPILOT_WEBINSPECTOR_SOCKET") {
        return Ok(p.into());
    }
    let out = tokio::process::Command::new("lsof").args(["-aUc", "launchd_sim", "-Fpn"]).output().await.map_err(other)?;
    let found = sockets_in_lsof(&String::from_utf8_lossy(&out.stdout));
    for (pid, path) in &found {
        let cmd = tokio::process::Command::new("ps").args(["-o", "command=", "-p", pid]).output().await.map_err(other)?;
        if String::from_utf8_lossy(&cmd.stdout).contains(udid) {
            return Ok(path.clone());
        }
    }
    match found.as_slice() {
        [(_, only)] => Ok(only.clone()),
        [] => Err(other("no webinspectord_sim socket (is the simulator booted?)")),
        _ => Err(other(format!("several simulators are booted and none matched {udid}: set FASTPILOT_WEBINSPECTOR_SOCKET"))),
    }
}

/// `(pid, socket path)` from `lsof -F pn` output.
fn sockets_in_lsof(out: &str) -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    let mut pid = "";
    for l in out.lines() {
        if let Some(p) = l.strip_prefix('p') {
            pid = p;
        } else if let Some(n) = l.strip_prefix('n')
            && n.ends_with("com.apple.webinspectord_sim.socket")
            && !found.iter().any(|(p, _)| p == pid)
        {
            found.push((pid.to_owned(), PathBuf::from(n)));
        }
    }
    found
}

pub async fn simulator_stream(udid: &str) -> Result<Stream, Error> {
    let path = simulator_socket(udid).await?;
    let s = tokio::net::UnixStream::connect(&path).await.map_err(|e| other(format!("{}: {e}", path.display())))?;
    Ok(Box::new(s))
}

/// lockdownd request (XML plist); its `Error` explained.
async fn lockdown(s: &mut Stream, request: &str, extra: &[(&str, &str)]) -> Result<plist::Dictionary, Error> {
    let mut d = plist::Dictionary::new();
    d.insert("Label".into(), "fastpilot".into());
    d.insert("Request".into(), request.into());
    for (k, v) in extra {
        d.insert((*k).into(), (*v).into());
    }
    send_plist(s, &d, false).await?;
    let r = read_plist(s).await?;
    if let Some(e) = str_of(&r, "Error") {
        return Err(other(match e {
            "InvalidService" => "the iPhone does not offer com.apple.webinspector over lockdown (the RemoteXPC path is not supported yet)".to_owned(),
            "PasswordProtected" => "the iPhone is locked: unlock it".to_owned(),
            "InvalidHostID" => "this Mac's pairing is no longer valid: reconnect the iPhone and tap Trust".to_owned(),
            e => format!("lockdown {request}: {e}"),
        }));
    }
    Ok(r)
}

/// The device's TLS certificate is not checked (it is self-issued by the pairing, carries no
/// name, and the channel is the local root-owned usbmuxd socket), as libimobiledevice does.
#[derive(Debug)]
struct AnyDevice(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyDevice {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Client certificate and key from the pair record (PEM: HostCertificate, HostPrivateKey).
fn tls_config(pair: &plist::Dictionary) -> Result<Arc<rustls::ClientConfig>, Error> {
    use rustls::pki_types::pem::PemObject;
    let data = |k: &str| pair.get(k).and_then(plist::Value::as_data).ok_or_else(|| other(format!("the pair record has no {k}")));
    let cert = rustls::pki_types::CertificateDer::from_pem_slice(data("HostCertificate")?).map_err(other)?;
    let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(data("HostPrivateKey")?).map_err(other)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(other)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyDevice(provider)))
        .with_client_auth_cert(vec![cert], key)
        .map_err(other)?;
    cfg.enable_sni = false;
    Ok(Arc::new(cfg))
}

async fn tls(s: Stream, cfg: &Arc<rustls::ClientConfig>) -> Result<Stream, Error> {
    let name = rustls::pki_types::ServerName::try_from("lockdown").map_err(other)?;
    let t = tokio_rustls::TlsConnector::from(cfg.clone())
        .connect(name, s)
        .await
        .map_err(|e| other(format!("TLS with the device: {e}")))?;
    Ok(Box::new(t))
}

/// `com.apple.webinspector` on the USB-attached device `udid`: lockdownd StartSession (TLS)
/// and StartService, then the service port over usbmux (TLS when lockdown asks for it).
pub async fn device_stream(udid: &str) -> Result<Stream, Error> {
    let sock = usbmux::socket_path();
    let (dev, serial) = usbmux::device(&sock, udid).await.map_err(other)?;
    let pair = usbmux::pair_record(&sock, &serial).await.map_err(other)?;
    let cfg = tls_config(&pair)?;
    let mut ld: Stream = Box::new(usbmux::connect(&sock, dev, 62078).await.map_err(other)?);
    let ids = [("HostID", str_of(&pair, "HostID").unwrap_or("")), ("SystemBUID", str_of(&pair, "SystemBUID").unwrap_or(""))];
    let r = lockdown(&mut ld, "StartSession", &ids).await?;
    if r.get("EnableSessionSSL").and_then(plist::Value::as_boolean) == Some(true) {
        ld = tls(ld, &cfg).await?;
    }
    let r = lockdown(&mut ld, "StartService", &[("Service", "com.apple.webinspector")]).await?;
    let port = int_of(&r, "Port").and_then(|p| u16::try_from(p).ok()).ok_or_else(|| other("StartService returned no port"))?;
    let mut svc: Stream = Box::new(usbmux::connect(&sock, dev, port).await.map_err(other)?);
    if r.get("EnableServiceSSL").and_then(plist::Value::as_boolean) == Some(true) {
        svc = tls(svc, &cfg).await?;
    }
    Ok(svc)
}

/// `fastpilot ios doctor` line: can the Web Inspector service be reached on `udid`?
pub async fn doctor(udid: &str) -> String {
    let probe = async {
        let io = device_stream(udid).await?;
        // No app has this bundle id: reaching "not inspectable" proves the service answers.
        match Session::open(io, "\u{0}").await {
            Err(Error::NotInspectable(_)) | Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    };
    match tokio::time::timeout(Duration::from_secs(10), probe).await {
        Ok(Ok(())) => "  Web Inspector service reachable (WKWebView content needs isInspectable = true)".to_owned(),
        Ok(Err(e)) => format!("  ! {e}\n      fix: USB cable, unlocked iPhone; {HINT}"),
        Err(_) => "  ! Web Inspector: no answer within 10 s".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(pairs: &[(&str, plist::Value)]) -> plist::Dictionary {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
    }

    async fn reply<W: AsyncWrite + Unpin>(w: &mut W, selector: &str, arg: plist::Dictionary) {
        let d = dict(&[("__selector", selector.into()), ("__argument", plist::Value::Dictionary(arg))]);
        send_plist(w, &d, true).await.expect("send");
    }

    /// A fake webinspectord: one app with a WebContent proxy, a target-based page that answers
    /// Runtime.evaluate through Target.dispatchMessageFromTarget.
    async fn fake(io: tokio::io::DuplexStream) {
        let (mut rd, mut wr) = tokio::io::split(io);
        while let Ok(msg) = read_plist(&mut rd).await {
            let arg = msg.get("__argument").and_then(plist::Value::as_dictionary).cloned().unwrap_or_default();
            match str_of(&msg, "__selector").unwrap_or("") {
                "_rpc_getConnectedApplications:" => {
                    let a = |id: &str, bundle: &str, host: Option<&str>| {
                        let mut d = dict(&[("WIRApplicationIdentifierKey", id.into()), ("WIRApplicationBundleIdentifierKey", bundle.into())]);
                        if let Some(h) = host {
                            d.insert("WIRHostApplicationIdentifierKey".into(), h.into());
                        }
                        plist::Value::Dictionary(d)
                    };
                    let apps = dict(&[
                        ("PID:1", a("PID:1", "com.fastpilot.demo", None)),
                        ("PID:2", a("PID:2", "com.apple.WebKit.WebContent", Some("PID:1"))),
                        ("PID:3", a("PID:3", "com.other", None)),
                    ]);
                    reply(&mut wr, "_rpc_reportConnectedApplicationList:", dict(&[("WIRApplicationDictionaryKey", plist::Value::Dictionary(apps))])).await;
                }
                "_rpc_forwardGetListing:" => {
                    let app = str_of(&arg, "WIRApplicationIdentifierKey").unwrap_or("").to_owned();
                    let listing = if app == "PID:2" {
                        let page = dict(&[
                            ("WIRPageIdentifierKey", plist::Value::Integer(4.into())),
                            ("WIRTypeKey", "WIRTypeWebPage".into()),
                            ("WIRURLKey", "https://example.com/login".into()),
                        ]);
                        let ctx = dict(&[("WIRPageIdentifierKey", plist::Value::Integer(5.into())), ("WIRTypeKey", "WIRTypeJavaScript".into())]);
                        dict(&[("4", plist::Value::Dictionary(page)), ("5", plist::Value::Dictionary(ctx))])
                    } else {
                        plist::Dictionary::new()
                    };
                    let arg = dict(&[("WIRApplicationIdentifierKey", app.into()), ("WIRListingKey", plist::Value::Dictionary(listing))]);
                    reply(&mut wr, "_rpc_applicationSentListing:", arg).await;
                }
                "_rpc_forwardSocketSetup:" => {
                    assert_eq!(str_of(&arg, "WIRApplicationIdentifierKey"), Some("PID:2"));
                    assert_eq!(int_of(&arg, "WIRPageIdentifierKey"), Some(4));
                    let ev = json!({ "method": "Target.targetCreated", "params": { "targetInfo": { "targetId": "page-9", "type": "page" } } });
                    let data = plist::Value::Data(ev.to_string().into_bytes());
                    reply(&mut wr, "_rpc_applicationSentData:", dict(&[("WIRMessageDataKey", data)])).await;
                }
                "_rpc_forwardSocketData:" => {
                    let Some(plist::Value::Data(raw)) = arg.get("WIRSocketDataKey") else { panic!("no socket data") };
                    let outer: Value = serde_json::from_slice(raw).expect("json");
                    assert_eq!(outer["method"], "Target.sendMessageToTarget");
                    assert_eq!(outer["params"]["targetId"], "page-9");
                    let inner: Value = serde_json::from_str(outer["params"]["message"].as_str().expect("message")).expect("inner");
                    // The ack comes first and must not be taken for the answer.
                    let ack = json!({ "id": outer["id"], "result": {} });
                    let answer = json!({ "id": inner["id"], "result": { "result": { "type": "string", "value": inner["params"]["expression"] } } });
                    let wrapped = json!({ "method": "Target.dispatchMessageFromTarget", "params": { "targetId": "page-9", "message": answer.to_string() } });
                    for m in [ack, wrapped] {
                        reply(&mut wr, "_rpc_applicationSentData:", dict(&[("WIRMessageDataKey", plist::Value::Data(m.to_string().into_bytes()))])).await;
                    }
                }
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn attaches_to_the_apps_page_and_unwraps_target_messages() {
        let (a, b) = tokio::io::duplex(1 << 16);
        tokio::spawn(fake(b));
        let s = Session::open(Box::new(a), "com.fastpilot.demo").await.expect("open");
        assert_eq!((s.app.as_str(), s.page), ("PID:2", 4));
        let r = s.call("Runtime.evaluate", json!({ "expression": "document.title" })).await.expect("call");
        assert_eq!(r["result"]["value"], "document.title");

        let (a, b) = tokio::io::duplex(1 << 16);
        tokio::spawn(fake(b));
        assert!(matches!(Session::open(Box::new(a), "com.nope").await, Err(Error::NotInspectable(m)) if m.contains("isInspectable")));
    }

    #[test]
    fn finds_simulator_sockets_in_lsof_output() {
        let out = "p501\nn/private/tmp/com.apple.launchd.AbC/com.apple.webinspectord_sim.socket\nn->0x1\np502\nn/private/tmp/com.apple.launchd.XyZ/other.socket\n";
        assert_eq!(
            sockets_in_lsof(out),
            vec![("501".to_owned(), PathBuf::from("/private/tmp/com.apple.launchd.AbC/com.apple.webinspectord_sim.socket"))]
        );
    }
}
