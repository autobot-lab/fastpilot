//! Client for on-device agents (agent-android/PROTOCOL.md, ios-runner/PROTOCOL.md): 4-byte BE
//! length + JSON frames. Shared by the Android and iOS drivers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::Error;

pub struct Agent {
    stream: Mutex<TcpStream>,
    next: AtomicU64,
}

impl Agent {
    pub async fn connect(port: u16) -> Result<Self, Error> {
        let mut last = None;
        for _ in 0..20 {
            match TcpStream::connect(("127.0.0.1", port)).await {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    let agent = Self { stream: Mutex::new(s), next: AtomicU64::new(1) };
                    // adb accepts forwarded connections before the device side answers.
                    match agent.call("hello", json!({})).await {
                        Ok(_) => return Ok(agent),
                        Err(e) => last = Some(e),
                    }
                }
                Err(e) => last = Some(Error::io(e)),
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        Err(last.unwrap_or(Error::Agent("agent not reachable".into())))
    }

    pub async fn call(&self, cmd: &str, args: Value) -> Result<Value, Error> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut msg = args;
        msg["id"] = id.into();
        msg["cmd"] = cmd.into();
        let body = msg.to_string();
        let mut s = self.stream.lock().await;
        let exchange = async {
            let mut frame = (body.len() as u32).to_be_bytes().to_vec();
            frame.extend_from_slice(body.as_bytes());
            s.write_all(&frame).await?;
            // Skip replies of earlier calls whose futures were dropped mid-flight.
            loop {
                let mut len = [0u8; 4];
                s.read_exact(&mut len).await?;
                let mut buf = vec![0u8; u32::from_be_bytes(len) as usize];
                s.read_exact(&mut buf).await?;
                let v: Value = serde_json::from_slice(&buf).unwrap_or_default();
                if v["id"].as_u64() == Some(id) {
                    return Ok::<_, std::io::Error>(v);
                }
            }
        };
        let v = tokio::time::timeout(Duration::from_secs(30), exchange)
            .await
            .map_err(|_| Error::Agent(format!("{cmd} timed out")))?
            .map_err(Error::io)?;
        if v["ok"].as_bool() == Some(true) {
            Ok(v)
        } else {
            Err(Error::Agent(v["error"].as_str().unwrap_or("unknown agent error").to_owned()))
        }
    }
}
