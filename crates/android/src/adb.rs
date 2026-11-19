//! adb server protocol (smart sockets + sync) spoken directly over TCP to the adb server
//! (127.0.0.1:5037, or `FASTPILOT_ADB_SERVER` for devices on another machine), so actions never
//! spawn the `adb` CLI (PRD §7.4). A local server is started once if needed.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::Error;

const LOCAL: &str = "127.0.0.1:5037";

/// The adb server to talk to: `FASTPILOT_ADB_SERVER` (or config `adb_server`), else the local one.
#[derive(Clone)]
pub struct Adb {
    server: String,
    /// Set when the server was configured: it may run on another machine, where its own
    /// `adb forward` ports would be, so ports are forwarded by fastpilot instead.
    configured: bool,
}

/// `host:port` of a configured adb server, or why it is not one.
pub fn configured_server() -> Option<Result<String, String>> {
    let v = fastpilot_core::config::var("FASTPILOT_ADB_SERVER").ok()?;
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    let ok = v.rsplit_once(':').is_some_and(|(h, p)| !h.is_empty() && p.parse::<u16>().is_ok());
    Some(if ok { Ok(v.to_owned()) } else { Err(format!("FASTPILOT_ADB_SERVER `{v}` is not host:port (e.g. 127.0.0.1:15037)")) })
}

/// Local ports fastpilot forwards itself, per (server, serial, device socket): reused, so a
/// session that reconnects does not open another listener.
/// (server, serial, device socket) → local port.
type ForwardKey = (String, String, String);
static FORWARDS: Mutex<Option<HashMap<ForwardKey, u16>>> = Mutex::new(None);

/// An Android SDK tool (`platform-tools/adb`, `emulator/emulator`, ...): its path under
/// ANDROID_HOME / ANDROID_SDK_ROOT / ~/Library/Android/sdk, else the bare `name` (PATH).
pub fn sdk_tool(rel: &str, name: &str) -> String {
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(home) = std::env::var_os(var) {
            let p = std::path::Path::new(&home).join(rel);
            if p.exists() {
                return p.display().to_string();
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = std::path::Path::new(&home).join("Library/Android/sdk").join(rel);
        if p.exists() {
            return p.display().to_string();
        }
    }
    name.to_owned()
}

impl Adb {
    /// Connect to the adb server, starting it once (`adb start-server`) if it is not running.
    /// A configured server is never started here: it belongs to the machine it runs on.
    pub async fn new() -> Result<Self, Error> {
        if let Some(server) = configured_server() {
            let server = server.map_err(Error::Adb)?;
            if let Some(host) = fastpilot_core::policy::remote_host(&server) {
                tracing::warn!(host, "adb_server is not this machine: adb has no password or encryption, use an SSH tunnel to 127.0.0.1");
            }
            return match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&server)).await {
                Ok(Ok(_)) => Ok(Self { server, configured: true }),
                _ => Err(Error::Adb(format!(
                    "cannot reach the adb server at {server} (FASTPILOT_ADB_SERVER); is the SSH tunnel open and `adb start-server` running on that machine?"
                ))),
            };
        }
        if TcpStream::connect(LOCAL).await.is_ok() {
            return Ok(Self { server: LOCAL.into(), configured: false });
        }
        let adb = sdk_tool("platform-tools/adb", "adb");
        let status = tokio::process::Command::new(&adb)
            .arg("start-server")
            .status()
            .await
            .map_err(|e| Error::Adb(format!("cannot start adb server ({adb}): {e}")))?;
        if !status.success() {
            return Err(Error::Adb(format!("`{adb} start-server` failed")));
        }
        Ok(Self { server: LOCAL.into(), configured: false })
    }

    /// The server this client talks to (`host:port`).
    pub fn server(&self) -> &str {
        &self.server
    }

    async fn open(&self, service: &str) -> Result<TcpStream, Error> {
        let mut s = TcpStream::connect(&self.server).await.map_err(|e| Error::Adb(format!("adb server {}: {e}", self.server)))?;
        s.set_nodelay(true).ok();
        s.write_all(format!("{:04x}{service}", service.len()).as_bytes()).await.map_err(Error::io)?;
        status(&mut s).await?;
        Ok(s)
    }

    async fn transport(&self, serial: &str) -> Result<TcpStream, Error> {
        self.open(&format!("host:transport:{serial}")).await
    }

    /// `(serial, state)` of attached devices/emulators.
    pub async fn devices(&self) -> Result<Vec<(String, String)>, Error> {
        let mut s = self.open("host:devices").await?;
        let text = read_len_string(&mut s).await?;
        Ok(text
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
            .collect())
    }

    /// `host:devices-l`: serial, state, product/model/device/transport_id.
    pub async fn devices_long(&self) -> Result<Vec<crate::device::Entry>, Error> {
        let mut s = self.open("host:devices-l").await?;
        Ok(crate::device::parse_devices_l(&read_len_string(&mut s).await?))
    }

    pub async fn shell(&self, serial: &str, cmd: &str) -> Result<String, Error> {
        let mut s = self.shell_stream(serial, cmd).await?;
        let mut out = Vec::new();
        tokio::time::timeout(Duration::from_secs(120), s.read_to_end(&mut out))
            .await
            .map_err(|_| Error::Adb(format!("shell `{cmd}` timed out")))?
            .map_err(Error::io)?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Long-running shell; the stream stays open while the command runs.
    pub async fn shell_stream(&self, serial: &str, cmd: &str) -> Result<TcpStream, Error> {
        let mut s = self.transport(serial).await?;
        let svc = format!("shell:{cmd}");
        s.write_all(format!("{:04x}{svc}", svc.len()).as_bytes()).await.map_err(Error::io)?;
        status(&mut s).await?;
        Ok(s)
    }

    /// A stream to a device socket (`tcp:7912`, `localabstract:...`) through the server itself.
    pub async fn connect_device(&self, serial: &str, remote: &str) -> Result<TcpStream, Error> {
        let mut s = self.transport(serial).await?;
        s.write_all(format!("{:04x}{remote}", remote.len()).as_bytes()).await.map_err(Error::io)?;
        status(&mut s).await?;
        Ok(s)
    }

    /// `adb forward tcp:0 <remote>`; returns the local port. With a configured (possibly remote)
    /// server, fastpilot listens on 127.0.0.1 here and carries each connection through the
    /// server ([`Adb::connect_device`]); a server's own forward would open the port on its machine.
    pub async fn forward(&self, serial: &str, remote: &str) -> Result<u16, Error> {
        if self.configured {
            return self.forward_here(serial, remote).await;
        }
        let mut s = self.open(&format!("host-serial:{serial}:forward:tcp:0;{remote}")).await?;
        status(&mut s).await?;
        let port = read_len_string(&mut s).await?;
        port.trim().parse().map_err(|_| Error::Adb(format!("bad forward port {port:?}")))
    }

    async fn forward_here(&self, serial: &str, remote: &str) -> Result<u16, Error> {
        let key = (self.server.clone(), serial.to_owned(), remote.to_owned());
        if let Some(port) = FORWARDS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get(&key) {
            return Ok(*port);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(Error::io)?;
        let port = listener.local_addr().map_err(Error::io)?.port();
        let (adb, serial_owned, remote_owned) = (self.clone(), serial.to_owned(), remote.to_owned());
        tokio::spawn(async move {
            while let Ok((mut local, _)) = listener.accept().await {
                let (adb, serial, remote) = (adb.clone(), serial_owned.clone(), remote_owned.clone());
                tokio::spawn(async move {
                    match adb.connect_device(&serial, &remote).await {
                        Ok(mut device) => {
                            local.set_nodelay(true).ok();
                            let _ = tokio::io::copy_bidirectional(&mut local, &mut device).await;
                        }
                        Err(e) => tracing::debug!(serial, remote, error = %e, "forwarded connection refused"),
                    }
                });
            }
        });
        FORWARDS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key, port);
        Ok(port)
    }

    /// Push bytes with the sync protocol (`SEND`/`DATA`/`DONE`).
    pub async fn push(&self, serial: &str, data: &[u8], remote: &str) -> Result<(), Error> {
        let mut s = self.transport(serial).await?;
        s.write_all(b"0005sync:").await.map_err(Error::io)?;
        status(&mut s).await?;
        let spec = format!("{remote},420"); // 0644
        let mut buf = Vec::with_capacity(data.len() + 64);
        buf.extend_from_slice(b"SEND");
        buf.extend_from_slice(&(spec.len() as u32).to_le_bytes());
        buf.extend_from_slice(spec.as_bytes());
        for chunk in data.chunks(64 * 1024) {
            buf.extend_from_slice(b"DATA");
            buf.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            buf.extend_from_slice(chunk);
        }
        buf.extend_from_slice(b"DONE");
        buf.extend_from_slice(&0u32.to_le_bytes());
        s.write_all(&buf).await.map_err(Error::io)?;
        let mut head = [0u8; 8];
        s.read_exact(&mut head).await.map_err(Error::io)?;
        if &head[..4] != b"OKAY" {
            let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
            let mut msg = vec![0u8; len.min(4096)];
            let _ = s.read_exact(&mut msg).await;
            return Err(Error::Adb(format!("push {remote}: {}", String::from_utf8_lossy(&msg))));
        }
        let _ = s.write_all(b"QUIT\0\0\0\0").await;
        Ok(())
    }

    /// Pull a file with the sync protocol (`RECV`, then `DATA` chunks until `DONE`).
    pub async fn pull(&self, serial: &str, remote: &str) -> Result<Vec<u8>, Error> {
        let mut s = self.transport(serial).await?;
        s.write_all(b"0005sync:").await.map_err(Error::io)?;
        status(&mut s).await?;
        let mut req = b"RECV".to_vec();
        req.extend_from_slice(&(remote.len() as u32).to_le_bytes());
        req.extend_from_slice(remote.as_bytes());
        s.write_all(&req).await.map_err(Error::io)?;
        let mut out = Vec::new();
        loop {
            let mut head = [0u8; 8];
            s.read_exact(&mut head).await.map_err(Error::io)?;
            let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
            match &head[..4] {
                b"DATA" => {
                    let start = out.len();
                    out.resize(start + len, 0);
                    s.read_exact(&mut out[start..]).await.map_err(Error::io)?;
                }
                b"DONE" => break,
                _ => {
                    let mut msg = vec![0u8; len.min(4096)];
                    let _ = s.read_exact(&mut msg).await;
                    return Err(Error::Adb(format!("pull {remote}: {}", String::from_utf8_lossy(&msg))));
                }
            }
        }
        let _ = s.write_all(b"QUIT\0\0\0\0").await;
        Ok(out)
    }

    pub async fn install(&self, serial: &str, apk: &[u8]) -> Result<(), Error> {
        let tmp = "/data/local/tmp/fastpilot-install.apk";
        self.push(serial, apk, tmp).await?;
        let out = self.shell(serial, &format!("pm install -r -t {tmp}; rm -f {tmp}")).await?;
        if out.contains("Success") {
            Ok(())
        } else {
            Err(Error::Adb(format!("pm install failed: {}", out.trim())))
        }
    }
}

async fn status(s: &mut TcpStream) -> Result<(), Error> {
    let mut st = [0u8; 4];
    s.read_exact(&mut st).await.map_err(Error::io)?;
    match &st {
        b"OKAY" => Ok(()),
        b"FAIL" => Err(Error::Adb(read_len_string(s).await.unwrap_or_else(|e| e.to_string()))),
        other => Err(Error::Adb(format!("unexpected adb reply {:?}", String::from_utf8_lossy(other)))),
    }
}

async fn read_len_string(s: &mut TcpStream) -> Result<String, Error> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await.map_err(Error::io)?;
    let len = usize::from_str_radix(&String::from_utf8_lossy(&len), 16).map_err(|_| Error::Adb("bad length".into()))?;
    let mut buf = vec![0u8; len];
    s.read_exact(&mut buf).await.map_err(Error::io)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Read lines from a shell stream until one contains `needle` (or EOF / timeout).
pub(crate) async fn wait_line(s: TcpStream, needle: &str, timeout: Duration) -> Result<(TcpStream, String), Error> {
    let mut r = BufReader::new(s);
    let mut seen = String::new();
    let found = tokio::time::timeout(timeout, async {
        let mut line = Vec::new();
        loop {
            line.clear();
            let mut byte = [0u8; 1];
            loop {
                match r.read(&mut byte).await {
                    Ok(0) | Err(_) => return None,
                    Ok(_) if byte[0] == b'\n' => break,
                    Ok(_) => line.push(byte[0]),
                }
            }
            let l = String::from_utf8_lossy(&line).trim().to_owned();
            seen.push_str(&l);
            seen.push('\n');
            if l.contains(needle) {
                return Some(l);
            }
        }
    })
    .await
    .ok()
    .flatten();
    match found {
        Some(l) => Ok((r.into_inner(), l)),
        None => Err(Error::Adb(format!("agent did not report `{needle}`: {}", seen.trim()))),
    }
}
