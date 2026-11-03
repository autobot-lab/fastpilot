//! Claude over raw HTTP (Rust has no official Anthropic SDK): Messages and token counting,
//! plus a stated offline token approximation for when no credentials are available.

use std::time::Duration;

use serde_json::{Value, json};

pub const API_VERSION: &str = "2026-10-21";
/// Exploration / judgment (PRD §12.5: the larger model).
pub const MODEL_EXPLORE: &str = "claude-opus-5-5";
/// Replaying a path-memory hint (PRD §12.5: the fast model).
pub const MODEL_REPLAY: &str = "claude-haiku-5-5";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(String),
    #[error("api {status}: {message}")]
    Api { status: u16, message: String },
    #[error("unexpected response: {0}")]
    Response(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
enum Auth {
    ApiKey(String),
    /// OAuth access token (`Authorization: Bearer` + the oauth beta header).
    Bearer(String),
}

#[derive(Clone)]
pub struct Claude {
    http: reqwest::Client,
    auth: Auth,
    base: String,
}

impl Claude {
    /// `ANTHROPIC_API_KEY`, else `ANTHROPIC_AUTH_TOKEN`; `ANTHROPIC_BASE_URL` overrides the host.
    pub fn from_env() -> Option<Self> {
        let auth = match (std::env::var("ANTHROPIC_API_KEY"), std::env::var("ANTHROPIC_AUTH_TOKEN")) {
            (Ok(k), _) if !k.is_empty() => Auth::ApiKey(k),
            (_, Ok(t)) if !t.is_empty() => Auth::Bearer(t),
            _ => return None,
        };
        let base = std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".into());
        let http = reqwest::Client::builder().timeout(Duration::from_secs(600)).build().ok()?;
        Some(Self { http, auth, base: base.trim_end_matches('/').to_owned() })
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        // Retry what the SDKs retry: 408/409/429/5xx and connection errors.
        let mut delay = Duration::from_millis(500);
        for attempt in 0..3 {
            let mut req = self
                .http
                .post(format!("{}{path}", self.base))
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json")
                .json(body);
            req = match &self.auth {
                Auth::ApiKey(k) => req.header("x-api-key", k),
                Auth::Bearer(t) => req.bearer_auth(t).header("anthropic-beta", "oauth-2025-04-20"),
            };
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) if attempt < 2 => {
                    tracing::debug!(error = %e, "claude request failed, retrying");
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                    continue;
                }
                Err(e) => return Err(Error::Http(e.to_string())),
            };
            let status = resp.status().as_u16();
            let v: Value = resp.json().await.map_err(|e| Error::Http(e.to_string()))?;
            if status < 300 {
                return Ok(v);
            }
            if attempt < 2 && (status == 408 || status == 409 || status == 429 || status >= 500) {
                tokio::time::sleep(delay).await;
                delay *= 2;
                continue;
            }
            let message = v["error"]["message"].as_str().unwrap_or("request failed").to_owned();
            return Err(Error::Api { status, message });
        }
        Err(Error::Http("retries exhausted".into()))
    }

    /// `POST /v1/messages` (non-streaming).
    pub async fn messages(&self, body: &Value) -> Result<Value> {
        self.post("/v1/messages", body).await
    }

    /// Exact input tokens of `text` as one user message, minus the per-message overhead.
    pub async fn count_tokens(&self, model: &str, text: &str) -> Result<u64> {
        let count = |t: &str| {
            let body = json!({ "model": model, "messages": [{ "role": "user", "content": t }] });
            async move {
                let v = self.post("/v1/messages/count_tokens", &body).await?;
                v["input_tokens"].as_u64().ok_or_else(|| Error::Response(v.to_string()))
            }
        };
        let overhead = count(".").await?.saturating_sub(1);
        Ok(count(text).await?.saturating_sub(overhead))
    }
}

/// Offline approximation of Claude tokens, used when no credentials are set. Rule: a run of
/// letters is 1 token per started 5 letters after the first; a run of digits 1 per 3; every
/// other non-space character 1; spaces are free and each newline is 1. Close (about ±20%) for
/// English-like UI text; `count_tokens` is the exact count.
pub fn approx_tokens(text: &str) -> usize {
    let mut n = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphabetic() {
            let mut len: usize = 1;
            while chars.peek().is_some_and(|c| c.is_alphabetic()) {
                chars.next();
                len += 1;
            }
            n += 1 + (len - 1) / 5;
        } else if c.is_ascii_digit() {
            let mut len: usize = 1;
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
                len += 1;
            }
            n += len.div_ceil(3);
        } else if c == '\n' || !c.is_whitespace() {
            n += 1;
        }
    }
    n
}

/// Image input tokens for a `w`×`h` screenshot (the documented `w*h/750` estimate).
pub fn image_tokens(w: u32, h: u32) -> u64 {
    (u64::from(w) * u64::from(h)).div_ceil(750)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approximates() {
        assert_eq!(approx_tokens(""), 0);
        assert_eq!(approx_tokens("e12 button \"Masuk\""), 7); // e,12,button(2),",Masuk,"
        assert_eq!(approx_tokens("Dashboard\n"), 3);
        assert_eq!(image_tokens(1280, 720), 1229);
    }
}
