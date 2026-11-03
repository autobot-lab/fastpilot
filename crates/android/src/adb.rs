//! adb server protocol (smart sockets + sync) spoken directly over TCP to 127.0.0.1:5037,
//! so actions never spawn the `adb` CLI (PRD §7.4). The server itself is started once if needed.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::Error;

const SERVER: &str = "127.0.0.1:5037";

#[derive(Clone)]
pub struct Adb;

pub(crate) fn sdk_tool(rel: &str, name: &str) -> String {
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
    pub async fn new() -> Result<Self, Error> {
        if TcpStream::connect(SERVER).await.is_ok() {
            return Ok(Self);
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
        Ok(Self)
    }

    async fn open(&self, service: &str) -> Result<TcpStream, Error> {
        let mut s = TcpStream::connect(SERVER).await.map_err(|e| Error::Adb(format!("adb server: {e}")))?;
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

    /// `adb forward tcp:0 <remote>`; returns the allocated local port.
    pub async fn forward(&self, serial: &str, remote: &str) -> Result<u16, Error> {
        let mut s = self.open(&format!("host-serial:{serial}:forward:tcp:0;{remote}")).await?;
        status(&mut s).await?;
        let port = read_len_string(&mut s).await?;
        port.trim().parse().map_err(|_| Error::Adb(format!("bad forward port {port:?}")))
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
