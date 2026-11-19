//! A read-only view of a page for the Live panel: a separate DevTools connection to the browser,
//! attached to the page target, with `Page.startScreencast`. Frames only flow while a
//! [`Screencast`] exists; dropping it stops the screencast and closes the connection. Nothing is
//! sent to the page except the screencast commands and frame acknowledgements.

use std::sync::Arc;

use base64::Engine;
use serde_json::json;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::Error;
use crate::cdp::Cdp;

pub struct Screencast {
    cdp: Arc<Cdp>,
    session: String,
    frames: watch::Receiver<Option<Arc<Vec<u8>>>>,
    task: JoinHandle<()>,
}

impl Screencast {
    /// Watch `target` on the browser at `ws`; frames at most `max_width` wide, JPEG `quality`.
    pub async fn start(ws: &str, target: &str, max_width: u32, quality: u8) -> Result<Self, Error> {
        let cdp = Cdp::connect(ws).await?;
        let attached = cdp.call(None, "Target.attachToTarget", json!({ "targetId": target, "flatten": true })).await?;
        let session = attached["sessionId"].as_str().ok_or(Error::Unexpected("sessionId"))?.to_owned();
        let (tx, frames) = watch::channel(None);
        let mut events = cdp.subscribe();
        let (c, s) = (cdp.clone(), session.clone());
        let task = tokio::spawn(async move {
            while let Ok(ev) = events.recv().await {
                if ev.session.as_deref() != Some(s.as_str()) || ev.method != "Page.screencastFrame" {
                    continue;
                }
                c.send(Some(&s), "Page.screencastFrameAck", json!({ "sessionId": ev.params["sessionId"] }));
                if let Some(data) = ev.params["data"].as_str()
                    && let Ok(jpeg) = base64::engine::general_purpose::STANDARD.decode(data)
                    && tx.send(Some(Arc::new(jpeg))).is_err()
                {
                    break;
                }
            }
        });
        cdp.call(Some(&session), "Page.enable", json!({})).await?;
        cdp.call(
            Some(&session),
            "Page.startScreencast",
            json!({ "format": "jpeg", "quality": quality.clamp(20, 90), "maxWidth": max_width.clamp(160, 1920), "maxHeight": 1600, "everyNthFrame": 1 }),
        )
        .await?;
        Ok(Self { cdp, session, frames, task })
    }

    /// The latest frame as it changes (`None` until the first one).
    pub fn frames(&self) -> watch::Receiver<Option<Arc<Vec<u8>>>> {
        self.frames.clone()
    }

    pub fn is_closed(&self) -> bool {
        self.cdp.is_closed()
    }
}

impl Drop for Screencast {
    fn drop(&mut self) {
        self.cdp.send(Some(&self.session), "Page.stopScreencast", json!({}));
        self.cdp.send(None, "Target.detachFromTarget", json!({ "sessionId": self.session }));
        self.task.abort();
    }
}
