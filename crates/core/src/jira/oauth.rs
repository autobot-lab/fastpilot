//! E5 (v0.7): Jira through "Sign in with Atlassian", with no API token.
//!
//! fastpilot is an OAuth 2.1 public client of Atlassian's remote MCP server, like Claude and
//! OpenBot:
//! - **Registration:** dynamic (RFC 7591), once per redirect address
//!   `http://127.0.0.1:<port>/oauth/atlassian/callback`. The client id is not a secret; it is kept
//!   in `~/.fastpilot/jira-oauth.json` with the server's endpoints.
//! - **Sign-in:** authorization code with PKCE S256. `state` is 32 random bytes, used once, and
//!   valid for 10 minutes; the redirect address must match exactly.
//! - **Tokens:** access and refresh tokens go to the OS key store (as the API token does), are
//!   refreshed silently, and are never sent to the page, the AI, Chat, logs or reports.
//!   Disconnect revokes them at Atlassian and removes them.
//! - **Calls:** only the Jira tools fastpilot needs (see [`ALLOWED`]) are called on the MCP server,
//!   and only on the site the person picked.
//!
//! Tests (debug builds only) point every Atlassian address at a fake server on 127.0.0.1 with
//! `FASTPILOT_ATLASSIAN_TEST_BASE`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::JiraError;

const BASE: &str = "https://mcp.atlassian.com";
const MCP_PATH: &str = "/v1/mcp";
/// How long a sign-in may take between the click and the return.
const STATE_TTL: Duration = Duration::from_secs(10 * 60);

/// The MCP tools fastpilot calls. Anything else is refused before it is sent.
pub const ALLOWED: &[&str] = &[
    "atlassianUserInfo",
    "getAccessibleAtlassianResources",
    "getJiraIssue",
    "searchJiraIssuesUsingJql",
    "createJiraIssue",
    "addOrEditJiraIssueComment",
];

/// Where Atlassian is: the real server, or (debug builds, tests only) a fake one on loopback.
pub fn base() -> String {
    test_base().unwrap_or_else(|| BASE.to_owned())
}

#[cfg(debug_assertions)]
fn test_base() -> Option<String> {
    let b = std::env::var("FASTPILOT_ATLASSIAN_TEST_BASE").ok()?;
    let b = b.trim_end_matches('/');
    let rest = b.strip_prefix("http://127.0.0.1:")?;
    rest.parse::<u16>().ok().map(|_| b.to_owned())
}

#[cfg(not(debug_assertions))]
fn test_base() -> Option<String> {
    None
}

fn http() -> Result<reqwest::Client, JiraError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .https_only(test_base().is_none())
        .build()
        .map_err(|_| JiraError::Network)
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random(n: usize) -> Result<Vec<u8>, JiraError> {
    use ring::rand::SecureRandom;
    let mut buf = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut buf).map_err(|_| JiraError::Other(0))?;
    Ok(buf)
}

/// PKCE S256: the challenge sent with the sign-in for a verifier kept here.
pub fn challenge(verifier: &str) -> String {
    b64(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref())
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The server's endpoints and our registration for one redirect address. No secret.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Client {
    pub redirect_uri: String,
    pub client_id: String,
    pub authorize: String,
    pub token: String,
    pub revoke: String,
}

fn client_path() -> std::path::PathBuf {
    crate::config::path().parent().map(std::path::Path::to_path_buf).unwrap_or_else(std::env::temp_dir).join("jira-oauth.json")
}

fn saved_client(redirect_uri: &str) -> Option<Client> {
    let c: Client = serde_json::from_str(&std::fs::read_to_string(client_path()).ok()?).ok()?;
    (c.redirect_uri == redirect_uri && c.authorize.starts_with(&format!("{}/", base()))).then_some(c)
}

fn save_client(c: &Client) -> Result<(), JiraError> {
    let p = client_path();
    let dir = p.parent().ok_or(JiraError::Other(0))?;
    std::fs::create_dir_all(dir).map_err(|_| JiraError::Other(0))?;
    let text = serde_json::to_string_pretty(c).map_err(|_| JiraError::Other(0))?;
    let tmp = p.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).map_err(|_| JiraError::Other(0))?;
        f.write_all(text.as_bytes()).map_err(|_| JiraError::Other(0))?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, &text).map_err(|_| JiraError::Other(0))?;
    std::fs::rename(&tmp, &p).map_err(|_| JiraError::Other(0))
}

/// An endpoint from the server's metadata, only on the server itself.
fn endpoint(meta: &Value, key: &str) -> Result<String, JiraError> {
    let u = meta[key].as_str().unwrap_or("");
    if u.starts_with(&format!("{}/", base())) { Ok(u.to_owned()) } else { Err(JiraError::Other(0)) }
}

/// The registration for `redirect_uri`, made once (dynamic client registration).
async fn client(redirect_uri: &str) -> Result<Client, JiraError> {
    if let Some(c) = saved_client(redirect_uri) {
        return Ok(c);
    }
    let h = http()?;
    let meta: Value = h.get(format!("{}/.well-known/oauth-authorization-server", base())).send().await.map_err(|_| JiraError::Network)?.json().await.map_err(|_| JiraError::Network)?;
    if !meta["code_challenge_methods_supported"].as_array().is_some_and(|m| m.iter().any(|x| x == "S256")) {
        return Err(JiraError::Other(0));
    }
    let register = endpoint(&meta, "registration_endpoint")?;
    let reg: Value = h
        .post(register)
        .json(&json!({
            "client_name": "fastpilot",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        }))
        .send()
        .await
        .map_err(|_| JiraError::Network)?
        .json()
        .await
        .map_err(|_| JiraError::Network)?;
    let client_id = reg["client_id"].as_str().filter(|s| !s.is_empty() && s.len() < 200).ok_or(JiraError::Other(0))?.to_owned();
    let token = endpoint(&meta, "token_endpoint")?;
    let revoke = endpoint(&meta, "revocation_endpoint").unwrap_or_else(|_| token.clone());
    let c = Client { redirect_uri: redirect_uri.to_owned(), client_id, authorize: endpoint(&meta, "authorization_endpoint")?, token, revoke };
    save_client(&c)?;
    Ok(c)
}

struct Pending {
    state: String,
    verifier: String,
    redirect_uri: String,
    at: Instant,
}

static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());

fn pending() -> std::sync::MutexGuard<'static, Vec<Pending>> {
    PENDING.lock().unwrap_or_else(|p| p.into_inner())
}

/// The person clicked Connect: the address of Atlassian's sign-in page for this dashboard.
pub async fn begin(redirect_uri: &str) -> Result<String, JiraError> {
    let c = client(redirect_uri).await?;
    let state = b64(&random(32)?);
    let verifier = b64(&random(32)?);
    let mut url = reqwest::Url::parse(&c.authorize).map_err(|_| JiraError::Other(0))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &c.client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge(&verifier))
        .append_pair("code_challenge_method", "S256");
    let mut p = pending();
    p.retain(|x| x.at.elapsed() < STATE_TTL);
    // A few open sign-ins at most (several tabs); the oldest go first.
    while p.len() >= 5 {
        p.remove(0);
    }
    p.push(Pending { state, verifier, redirect_uri: redirect_uri.to_owned(), at: Instant::now() });
    Ok(url.into())
}

/// Take the open sign-in `state` belongs to: once, and only within 10 minutes.
fn take(state: &str) -> Option<Pending> {
    let mut p = pending();
    let i = p.iter().position(|x| same(&x.state, state))?;
    let x = p.remove(i);
    (x.at.elapsed() < STATE_TTL).then_some(x)
}

/// The tokens, in the key store as one hex-encoded JSON value.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tokens {
    access: String,
    refresh: String,
    /// When the access token stops working (ms since 1970).
    expires_ms: u64,
}

fn keep(t: &Tokens) -> Result<(), JiraError> {
    let text = serde_json::to_string(t).map_err(|_| JiraError::Other(0))?;
    super::token::set_oauth(&text).map_err(|_| JiraError::Other(0))
}

fn kept() -> Option<Tokens> {
    serde_json::from_str(&super::token::get_oauth()?).ok()
}

pub fn connected() -> bool {
    kept().is_some()
}

fn tokens_from(v: &Value, old_refresh: Option<&str>) -> Result<Tokens, JiraError> {
    let access = v["access_token"].as_str().filter(|s| !s.is_empty()).ok_or(JiraError::Refused)?.to_owned();
    let refresh = v["refresh_token"].as_str().map(str::to_owned).or_else(|| old_refresh.map(str::to_owned)).unwrap_or_default();
    let secs = v["expires_in"].as_u64().unwrap_or(3600).min(30 * 24 * 3600);
    Ok(Tokens { access, refresh, expires_ms: super::now_ms() + secs * 1000 })
}

/// The person came back from Atlassian with `code` for `state`: trade it for tokens.
pub async fn finish(state: &str, code: &str) -> Result<(), JiraError> {
    let p = take(state).ok_or(JiraError::Rejected("This sign-in link is not valid any more (it works once, for 10 minutes). Click Connect again.".into()))?;
    let c = saved_client(&p.redirect_uri).ok_or(JiraError::Other(0))?;
    let res = http()?
        .post(&c.token)
        .form(&[("grant_type", "authorization_code"), ("code", code), ("redirect_uri", &p.redirect_uri), ("client_id", &c.client_id), ("code_verifier", &p.verifier)])
        .send()
        .await
        .map_err(|_| JiraError::Network)?;
    if !res.status().is_success() {
        return Err(JiraError::Refused);
    }
    let v: Value = res.json().await.map_err(|_| JiraError::Network)?;
    keep(&tokens_from(&v, None)?)
}

fn any_client() -> Option<Client> {
    let c: Client = serde_json::from_str(&std::fs::read_to_string(client_path()).ok()?).ok()?;
    c.token.starts_with(&format!("{}/", base())).then_some(c)
}

/// A working access token: the kept one, or a fresh one when it expires within a minute.
async fn access(force: bool) -> Result<String, JiraError> {
    let t = kept().ok_or(JiraError::NotConnected)?;
    if !force && t.expires_ms > super::now_ms() + 60_000 {
        return Ok(t.access);
    }
    let c = any_client().ok_or(JiraError::NotConnected)?;
    if t.refresh.is_empty() {
        return Err(JiraError::Refused);
    }
    let res = http()?.post(&c.token).form(&[("grant_type", "refresh_token"), ("refresh_token", &t.refresh), ("client_id", &c.client_id)]).send().await.map_err(|_| JiraError::Network)?;
    if !res.status().is_success() {
        return Err(JiraError::Refused);
    }
    let v: Value = res.json().await.map_err(|_| JiraError::Network)?;
    let fresh = tokens_from(&v, Some(&t.refresh))?;
    keep(&fresh)?;
    Ok(fresh.access)
}

/// Disconnect: revoke at Atlassian (refresh token, then access token), then forget them here.
/// `Err` only when they are still kept here afterwards.
pub async fn revoke() -> Result<(), String> {
    if let (Some(t), Some(c), Ok(h)) = (kept(), any_client(), http()) {
        for (tok, hint) in [(&t.refresh, "refresh_token"), (&t.access, "access_token")] {
            if !tok.is_empty() {
                let _ = h.post(&c.revoke).form(&[("token", tok.as_str()), ("token_type_hint", hint), ("client_id", &c.client_id)]).send().await;
            }
        }
    }
    // The registration (no secret) goes too: a new sign-in registers again.
    let _ = std::fs::remove_file(client_path());
    super::token::delete_oauth()
}

/// One call to an allowed tool on the MCP server: its result as JSON (or text).
pub async fn call(tool: &str, args: Value) -> Result<Value, JiraError> {
    if !ALLOWED.contains(&tool) {
        return Err(JiraError::Rejected(format!("fastpilot does not call the Atlassian tool {tool}")));
    }
    match call_with(tool, &args, &access(false).await?).await {
        Err(JiraError::Refused) => call_with(tool, &args, &access(true).await?).await,
        r => r,
    }
}

async fn rpc(h: &reqwest::Client, token: &str, session: Option<&str>, body: &Value) -> Result<(Option<String>, Value), JiraError> {
    let mut req = h.post(format!("{}{MCP_PATH}", base())).bearer_auth(token).header("Accept", "application/json, text/event-stream").json(body);
    if let Some(s) = session {
        req = req.header("Mcp-Session-Id", s);
    }
    let res = req.send().await.map_err(|_| JiraError::Network)?;
    match res.status().as_u16() {
        200..=299 => {}
        401 => return Err(JiraError::Refused),
        403 => return Err(JiraError::NoAccess),
        429 => return Err(JiraError::Busy(10)),
        s => return Err(JiraError::Other(s)),
    }
    let session = res.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let text = res.text().await.map_err(|_| JiraError::Network)?;
    // JSON, or a server-sent event stream whose `data:` lines hold the answer.
    let v = serde_json::from_str::<Value>(&text).ok().or_else(|| text.lines().filter_map(|l| l.strip_prefix("data:")).filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok()).find(|v| v.get("id").is_some()));
    Ok((session, v.unwrap_or(Value::Null)))
}

async fn call_with(tool: &str, args: &Value, token: &str) -> Result<Value, JiraError> {
    let h = http()?;
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "fastpilot", "version": env!("CARGO_PKG_VERSION") } } });
    let (session, _) = rpc(&h, token, None, &init).await?;
    let _ = rpc(&h, token, session.as_deref(), &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    let (_, v) = rpc(&h, token, session.as_deref(), &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": tool, "arguments": args } })).await?;
    let r = &v["result"];
    let text: String = r["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");
    if r["isError"].as_bool() == Some(true) || v.get("error").is_some() {
        let why = if text.is_empty() { v["error"]["message"].as_str().unwrap_or("the Atlassian tool refused").to_owned() } else { text };
        return Err(JiraError::Rejected(super::plain(&why, 300)));
    }
    if let Some(s) = r.get("structuredContent").filter(|s| !s.is_null()) {
        return Ok(s.clone());
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// The Jira sites the account may use: `(host, name)`, `*.atlassian.net` only.
pub async fn sites() -> Result<Vec<(String, String)>, JiraError> {
    let v = call("getAccessibleAtlassianResources", json!({})).await?;
    let list = v.as_array().cloned().or_else(|| v["resources"].as_array().cloned()).unwrap_or_default();
    let mut out: Vec<(String, String)> = Vec::new();
    for r in list {
        let host = r["url"].as_str().and_then(|u| super::parse_site(u).ok());
        if let Some(h) = host
            && !out.iter().any(|(x, _)| *x == h)
        {
            out.push((h, super::plain(r["name"].as_str().unwrap_or(""), 120)));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_is_s256_of_the_verifier() {
        // Expected value from Python: base64url(sha256(verifier)) without padding.
        assert_eq!(challenge("fastpilot-pkce-check-0123456789-abcdefghijklmnop"), "8cKRXSyVkXfsHVqNpschvMNzyky82BS8KYbbOV7uW8M");
    }

    #[test]
    fn a_state_works_once_and_only_for_ten_minutes() {
        pending().push(Pending { state: "s-ok".into(), verifier: "v".into(), redirect_uri: "r".into(), at: Instant::now() });
        assert!(take("s-other").is_none(), "another state");
        assert!(take("s-ok").is_some());
        assert!(take("s-ok").is_none(), "used once");
        let old = Instant::now().checked_sub(STATE_TTL + Duration::from_secs(1));
        if let Some(at) = old {
            pending().push(Pending { state: "s-old".into(), verifier: "v".into(), redirect_uri: "r".into(), at });
            assert!(take("s-old").is_none(), "expired");
        }
    }

    #[tokio::test]
    async fn only_the_jira_tools_fastpilot_needs_are_called() {
        let e = call("deleteJiraIssue", json!({})).await.expect_err("refused");
        assert!(e.to_string().contains("does not call the Atlassian tool deleteJiraIssue"), "{e}");
    }
}
