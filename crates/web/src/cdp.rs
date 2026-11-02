//! Minimal CDP client: request/response by id, events fanned out over a broadcast channel.
//! Uses flattened sessions, so one socket serves the browser and every page.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::Error;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct Event {
    pub session: Option<String>,
    pub method: String,
    pub params: Value,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

pub struct Cdp {
    out: mpsc::UnboundedSender<Message>,
    pending: Pending,
    next: AtomicU64,
    events: broadcast::Sender<Arc<Event>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Cdp {
    pub async fn connect(url: &str) -> Result<Arc<Self>, Error> {
        let (ws, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| Error::WebSocket(e.to_string()))?;
        let (mut sink, mut stream) = ws.split();
        let (out, mut rx) = mpsc::unbounded_channel::<Message>();
        let pending: Pending = Arc::default();
        let (events, _) = broadcast::channel(4096);

        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                if sink.send(m).await.is_err() {
                    break;
                }
            }
        });
        let (p, ev) = (pending.clone(), events.clone());
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                let Message::Text(text) = msg else { continue };
                let Ok(mut v) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(id) = v.get("id").and_then(Value::as_u64) {
                    if let Some(tx) = lock(&p).remove(&id) {
                        let res = match v.get("error") {
                            Some(e) => Err(e["message"].as_str().unwrap_or("unknown error").to_string()),
                            None => Ok(v["result"].take()),
                        };
                        let _ = tx.send(res);
                    }
                } else if let Some(method) = v["method"].as_str() {
                    let _ = ev.send(Arc::new(Event {
                        session: v["sessionId"].as_str().map(str::to_owned),
                        method: method.to_owned(),
                        params: v["params"].take(),
                    }));
                }
            }
            tracing::debug!("CDP socket closed");
            lock(&p).clear(); // wakes every caller with `Closed`
        });
        Ok(Arc::new(Self {
            out,
            pending,
            next: AtomicU64::new(1),
            events,
        }))
    }

    fn message(&self, id: u64, session: Option<&str>, method: &str, params: Value) -> Message {
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = s.into();
        }
        Message::text(msg.to_string())
    }

    pub async fn call(&self, session: Option<&str>, method: &str, params: Value) -> Result<Value, Error> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        if self.out.send(self.message(id, session, method, params)).is_err() {
            lock(&self.pending).remove(&id);
            return Err(Error::Closed);
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(res)) => res.map_err(|message| Error::Protocol {
                method: method.to_owned(),
                message,
            }),
            Ok(Err(_)) => Err(Error::Closed),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(Error::CallTimeout(method.to_owned()))
            }
        }
    }

    /// Fire-and-forget; usable from `Drop`.
    pub fn send(&self, session: Option<&str>, method: &str, params: Value) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let _ = self.out.send(self.message(id, session, method, params));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Event>> {
        self.events.subscribe()
    }
}
