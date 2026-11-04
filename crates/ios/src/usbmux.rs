//! usbmuxd client (`/var/run/usbmuxd`): the same USB channel Xcode uses. A local TCP port is
//! forwarded to a port on the iPhone, like `adb forward`, so the runner on the device is
//! reached as `127.0.0.1:<local>` (no Local Network prompt, no tunnel needed; USB cable required).
//!
//! Wire format: 16-byte little-endian header (`length` including the header, `version` 1,
//! `message` 8 = plist, `tag`), then an XML plist. After a successful `Connect` the socket
//! carries the device port's raw bytes.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixStream};
use tokio::task::JoinHandle;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("usbmuxd: {0}")]
    Io(String),
    #[error("usbmuxd: {0}")]
    Protocol(String),
    #[error("device {0} is not connected over USB")]
    NotAttached(String),
    #[error("usbmuxd refused the connection to port {0} (is the runner listening?)")]
    Refused(u16),
}

fn io(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}

/// `USBMUXD_SOCKET_ADDRESS` (as libimobiledevice) or the system socket.
pub fn socket_path() -> PathBuf {
    std::env::var_os("USBMUXD_SOCKET_ADDRESS")
        .map(|v| PathBuf::from(v.to_string_lossy().trim_start_matches("UNIX:").to_owned()))
        .unwrap_or_else(|| PathBuf::from("/var/run/usbmuxd"))
}

/// Header + XML plist body.
pub fn frame(tag: u32, body: &plist::Dictionary) -> Result<Vec<u8>, Error> {
    let mut xml = Vec::new();
    plist::to_writer_xml(&mut xml, body).map_err(|e| Error::Protocol(e.to_string()))?;
    let mut out = Vec::with_capacity(16 + xml.len());
    out.extend_from_slice(&((16 + xml.len()) as u32).to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // version
    out.extend_from_slice(&8u32.to_le_bytes()); // plist message
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&xml);
    Ok(out)
}

async fn read_frame(s: &mut UnixStream) -> Result<plist::Dictionary, Error> {
    let mut head = [0u8; 16];
    s.read_exact(&mut head).await.map_err(io)?;
    let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if !(16..=4 << 20).contains(&len) {
        return Err(Error::Protocol(format!("bad frame length {len}")));
    }
    let mut body = vec![0u8; len - 16];
    s.read_exact(&mut body).await.map_err(io)?;
    plist::from_bytes::<plist::Dictionary>(&body).map_err(|e| Error::Protocol(e.to_string()))
}

fn request(kind: &str) -> plist::Dictionary {
    let mut d = plist::Dictionary::new();
    d.insert("MessageType".into(), kind.into());
    d.insert("ClientVersionString".into(), "fastpilot".into());
    d.insert("ProgName".into(), "fastpilot".into());
    d
}

/// UDIDs compare without the dash: usbmux reports `00008110001C...` for `00008110-001C...`.
fn same_udid(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('-', "").to_ascii_uppercase();
    norm(a) == norm(b)
}

/// `(DeviceID, udid)` of USB-attached devices.
pub async fn list(socket: &Path) -> Result<Vec<(u64, String)>, Error> {
    let mut s = UnixStream::connect(socket).await.map_err(io)?;
    s.write_all(&frame(1, &request("ListDevices"))?).await.map_err(io)?;
    let reply = read_frame(&mut s).await?;
    let list = reply.get("DeviceList").and_then(plist::Value::as_array).cloned().unwrap_or_default();
    Ok(list
        .iter()
        .filter_map(|d| {
            let d = d.as_dictionary()?;
            let props = d.get("Properties")?.as_dictionary()?;
            if props.get("ConnectionType").and_then(plist::Value::as_string).is_some_and(|t| t != "USB") {
                return None;
            }
            let id = d.get("DeviceID").or_else(|| props.get("DeviceID"))?.as_unsigned_integer()?;
            Some((id, props.get("SerialNumber")?.as_string()?.to_owned()))
        })
        .collect())
}

/// A raw stream to `port` on the device (the port goes over the wire byte-swapped, htons).
pub async fn connect(socket: &Path, device_id: u64, port: u16) -> Result<UnixStream, Error> {
    let mut s = UnixStream::connect(socket).await.map_err(io)?;
    let mut req = request("Connect");
    req.insert("DeviceID".into(), plist::Value::Integer(device_id.into()));
    req.insert("PortNumber".into(), plist::Value::Integer(u64::from(port.to_be()).into()));
    s.write_all(&frame(2, &req)?).await.map_err(io)?;
    let reply = read_frame(&mut s).await?;
    match reply.get("Number").and_then(plist::Value::as_unsigned_integer) {
        Some(0) => Ok(s),
        Some(2) => Err(Error::NotAttached(device_id.to_string())),
        Some(3) => Err(Error::Refused(port)),
        other => Err(Error::Protocol(format!("Connect result {other:?}"))),
    }
}

/// Forward `127.0.0.1:<local>` to `remote_port` on the device `udid`; every accepted
/// connection opens its own usbmux channel. Dropping (aborting) the handle stops it.
pub async fn forward(udid: &str, remote_port: u16) -> Result<(u16, JoinHandle<()>), Error> {
    forward_via(&socket_path(), udid, remote_port).await
}

/// `(DeviceID, usbmux serial)` of the USB-attached device `udid`.
pub async fn device(socket: &Path, udid: &str) -> Result<(u64, String), Error> {
    list(socket).await?.into_iter().find(|(_, u)| same_udid(u, udid)).ok_or_else(|| Error::NotAttached(udid.to_owned()))
}

/// usbmuxd's pair record for `serial` (as `list` reports it): HostCertificate, HostPrivateKey,
/// HostID, SystemBUID — what lockdownd sessions need.
pub async fn pair_record(socket: &Path, serial: &str) -> Result<plist::Dictionary, Error> {
    let mut s = UnixStream::connect(socket).await.map_err(io)?;
    let mut req = request("ReadPairRecord");
    req.insert("PairRecordID".into(), serial.into());
    s.write_all(&frame(3, &req)?).await.map_err(io)?;
    let reply = read_frame(&mut s).await?;
    let data = reply
        .get("PairRecordData")
        .and_then(plist::Value::as_data)
        .ok_or_else(|| Error::Protocol(format!("no pair record for {serial}: unlock the device and tap Trust")))?;
    plist::from_bytes(data).map_err(|e| Error::Protocol(e.to_string()))
}

pub async fn forward_via(socket: &Path, udid: &str, remote_port: u16) -> Result<(u16, JoinHandle<()>), Error> {
    let (device, _) = device(socket, udid).await?;
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(io)?;
    let local = listener.local_addr().map_err(io)?.port();
    let socket = socket.to_owned();
    let task = tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let socket = socket.clone();
            tokio::spawn(async move {
                match connect(&socket, device, remote_port).await {
                    Ok(mut usb) => {
                        let _ = tokio::io::copy_bidirectional(&mut tcp, &mut usb).await;
                    }
                    Err(e) => tracing::debug!(error = %e, "usbmux connect failed"),
                }
            });
        }
    });
    Ok((local, task))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    /// One fake usbmuxd exchange: answers ListDevices, or Connect with `result` and then
    /// echoes bytes.
    async fn fake(listener: UnixListener, result: u64) {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(req) = read_frame(&mut s).await else { return };
                let kind = req.get("MessageType").and_then(plist::Value::as_string).unwrap_or("").to_owned();
                let mut reply = plist::Dictionary::new();
                if kind == "ListDevices" {
                    let mut props = plist::Dictionary::new();
                    props.insert("SerialNumber".into(), "00008110001C5C9E0AC0A01E".into());
                    props.insert("ConnectionType".into(), "USB".into());
                    let mut dev = plist::Dictionary::new();
                    dev.insert("DeviceID".into(), plist::Value::Integer(7u64.into()));
                    dev.insert("Properties".into(), plist::Value::Dictionary(props));
                    reply.insert("DeviceList".into(), plist::Value::Array(vec![plist::Value::Dictionary(dev)]));
                } else {
                    let port = req.get("PortNumber").and_then(plist::Value::as_unsigned_integer).unwrap_or(0);
                    assert_eq!(port, u64::from(22087u16.to_be()), "port is sent byte-swapped");
                    reply.insert("MessageType".into(), "Result".into());
                    reply.insert("Number".into(), plist::Value::Integer(result.into()));
                }
                let Ok(f) = frame(1, &reply) else { return };
                if s.write_all(&f).await.is_err() || kind == "ListDevices" || result != 0 {
                    return;
                }
                let mut buf = [0u8; 64];
                while let Ok(n) = s.read(&mut buf).await {
                    if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    }

    #[tokio::test]
    async fn lists_connects_and_forwards() {
        let header = frame(9, &request("ListDevices")).expect("frame");
        assert_eq!((&header[4..8], &header[8..12], &header[12..16]), (&1u32.to_le_bytes()[..], &8u32.to_le_bytes()[..], &9u32.to_le_bytes()[..]));
        assert_eq!(u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize, header.len());

        let path = std::env::temp_dir().join(format!("fastpilot-usbmux-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        tokio::spawn(fake(UnixListener::bind(&path).expect("bind"), 0));
        assert_eq!(list(&path).await.expect("list"), vec![(7, "00008110001C5C9E0AC0A01E".to_owned())]);
        // The dashed UDID from devicectl matches the dashless one from usbmux.
        let (local, task) = forward_via(&path, "00008110-001C5C9E0AC0A01E", 22087).await.expect("forward");
        let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1", local)).await.expect("connect");
        tcp.write_all(b"hello").await.expect("write");
        let mut got = [0u8; 5];
        tcp.read_exact(&mut got).await.expect("echo");
        assert_eq!(&got, b"hello");
        task.abort();
        assert!(matches!(forward_via(&path, "NOPE", 1).await, Err(Error::NotAttached(_))));
        let _ = std::fs::remove_file(&path);

        let refused = std::env::temp_dir().join(format!("fastpilot-usbmux-r-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&refused);
        tokio::spawn(fake(UnixListener::bind(&refused).expect("bind"), 3));
        assert!(matches!(connect(&refused, 7, 22087).await, Err(Error::Refused(22087))));
        let _ = std::fs::remove_file(&refused);
    }
}
