//! v0.6 1.2 Connect Jira: the dashboard's Integrations › Jira page (design apps-jira-v10, sheet 02). Only
//! this page connects, changes or disconnects Jira, by a person's click (the dashboard guard: our
//! own page, cookie, JSON). Chat, the AI and the MCP `config` tool have no way here.
//!
//! The token arrives once, in a masked field, goes to the OS key store bound to the site's origin,
//! and is never sent back to the page, written to a file the AI reads, logged, or put in the audit
//! log (which gets "Jira connected/disconnected", the site and the email).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use fastpilot_core::jira::{self, Jira, JiraError};
use serde_json::{Value, json};

fn refuse(status: StatusCode, kind: &str, message: &str) -> Response {
    (status, axum::Json(json!({ "error": message, "kind": kind }))).into_response()
}

fn jira_refusal(e: &JiraError) -> Response {
    let wait = if let JiraError::Busy(s) = e { Some(*s) } else { None };
    (StatusCode::CONFLICT, axum::Json(json!({ "error": e.to_string(), "kind": e.kind(), "wait": wait }))).into_response()
}

/// `GET /api/jira`: the connection, never the token.
pub async fn state() -> Response {
    let s = jira::settings();
    let connected = s.as_ref().is_some_and(|s| if s.signed_in() { jira::oauth::connected() } else { jira::token::has() });
    let choose: Vec<Value> = if connected { Vec::new() } else { choices().into_iter().map(|(h, n)| json!({ "site": h, "name": n })).collect() };
    axum::Json(json!({
        "connected": connected,
        "settings": s,
        "kept_in": jira::token::kept_in(),
        "choose_site": choose,
    }))
    .into_response()
}

// ---- E5: Sign in with Atlassian ----------------------------------------------------------------

/// The sites to choose from after a sign-in that can use more than one (until one is chosen).
static CHOICES: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

fn choices() -> Vec<(String, String)> {
    if !jira::oauth::connected() {
        return Vec::new();
    }
    CHOICES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// This dashboard's own return address: always 127.0.0.1 (the page's origin) and its port.
fn redirect_uri(host: &str) -> Option<String> {
    let port: u16 = host.rsplit_once(':')?.1.parse().ok()?;
    Some(format!("http://127.0.0.1:{port}/oauth/atlassian/callback"))
}

/// `{action: connect}`: where the browser goes to sign in (Atlassian's page). Nothing is kept
/// until the person comes back with a valid `state`.
async fn connect(host: &str) -> Response {
    let Some(r) = redirect_uri(host) else { return refuse(StatusCode::BAD_REQUEST, "other", "Open the dashboard at its 127.0.0.1 address, then click Connect again.") };
    match jira::oauth::begin(&r).await {
        Ok(url) => axum::Json(json!({ "url": url })).into_response(),
        Err(e) => refuse(StatusCode::CONFLICT, e.kind(), &format!("Could not reach Atlassian's sign-in. Check your internet connection, then try again. ({e})")),
    }
}

/// Save the connection for `site` (one of the account's sites), with its most recent project.
async fn use_site(site: &str) -> Result<jira::Settings, Response> {
    let j = jira::Jira::signed_in(site).map_err(|e| jira_refusal(&e))?;
    let name = j.myself().await.map_err(|e| jira_refusal(&e))?;
    let projects = j.projects().await.unwrap_or_default();
    let (key, pname) = projects.first().cloned().unwrap_or_default();
    let was = jira::settings();
    let s = jira::Settings { site: j.site().to_owned(), display_name: name, project_key: key, project_name: pname, checked_ms: jira::now_ms(), auth: "atlassian".into(), ..Default::default() };
    jira::save_settings(&s).map_err(|e| refuse(StatusCode::CONFLICT, "other", &format!("The settings could not be saved: {e}")))?;
    CHOICES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    fastpilot_core::change::audit("jira", was.as_ref().map(|w| w.site.clone()).as_deref(), Some(&format!("{} (Sign in with Atlassian)", s.site)), "dashboard", if was.is_some() { "changed" } else { "connected" });
    Ok(s)
}

/// `GET /oauth/atlassian/callback?code=…&state=…`: the browser back from Atlassian. Open without
/// the dashboard cookie (a cross-site return does not carry a SameSite=Strict cookie); the
/// single-use `state` of a sign-in this dashboard started is what lets it in.
pub async fn callback(axum::extract::RawQuery(q): axum::extract::RawQuery) -> Response {
    let q = q.unwrap_or_default();
    let param = |k: &str| reqwest::Url::parse(&format!("http://x/?{q}")).ok().and_then(|u| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned()));
    let outcome = match (param("state"), param("code"), param("error")) {
        (_, _, Some(_)) => Err("Atlassian did not connect: the sign-in was cancelled or not allowed. Nothing was saved.".to_owned()),
        (Some(state), Some(code), None) => match jira::oauth::finish(&state, &code).await {
            Ok(()) => match jira::oauth::sites().await {
                Ok(sites) if sites.len() == 1 => use_site(&sites[0].0).await.map(|_| ()).map_err(|_| "Signed in, but Jira did not answer. Open Integrations › Jira and try again.".to_owned()),
                Ok(sites) if sites.is_empty() => Err("Signed in, but this Atlassian account has no Jira Cloud site.".to_owned()),
                Ok(sites) => {
                    *CHOICES.lock().unwrap_or_else(|e| e.into_inner()) = sites;
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            },
            Err(e) => Err(e.to_string()),
        },
        _ => Err("This page is only for returning from Atlassian's sign-in. Nothing was saved.".to_owned()),
    };
    let (title, line) = match &outcome {
        Ok(()) => ("Jira is connected", "Back to fastpilot…".to_owned()),
        Err(why) => ("Jira is not connected", why.clone()),
    };
    let to = if outcome.is_ok() { "/#/integrations/jira" } else { "/#/integrations/jira/failed" };
    let esc = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let page = format!(
        "<!doctype html><meta charset=utf-8><meta name=referrer content=no-referrer><meta http-equiv=refresh content=\"{}; url={to}\"><title>{title}</title><p><b>{title}.</b> {}</p><p><a href=\"{to}\">Open fastpilot</a></p>",
        if outcome.is_ok() { 0 } else { 5 },
        esc(&line)
    );
    let status = if outcome.is_ok() { StatusCode::OK } else { StatusCode::BAD_REQUEST };
    (status, [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"), (axum::http::header::CACHE_CONTROL, "no-store")], page).into_response()
}

fn text(body: &Value, k: &str) -> String {
    body[k].as_str().unwrap_or("").trim().to_owned()
}

/// The account to try: the typed token, or (Manage, same site and email) the saved one.
fn account(body: &Value) -> Result<Jira, Box<Response>> {
    let site = jira::parse_site(&text(body, "site")).map_err(|_| Box::new(jira_refusal(&JiraError::NotCloud)))?;
    let email = text(body, "email");
    if email.is_empty() || !email.contains('@') || email.len() > 200 {
        return Err(Box::new(refuse(StatusCode::BAD_REQUEST, "empty", "Enter the email you use to sign in to Atlassian.")));
    }
    let typed = text(body, "token");
    let token = if typed.is_empty() {
        let saved = jira::settings().filter(|s| s.site == site && s.email.eq_ignore_ascii_case(&email));
        match (saved, jira::token::get()) {
            (Some(_), Some((t, bound))) if bound == jira::origin_for(&site) => t,
            _ => return Err(Box::new(refuse(StatusCode::BAD_REQUEST, "empty", "Paste your API token. A new address or email needs the token again."))),
        }
    } else {
        jira::token::check(&typed).map_err(|e| Box::new(refuse(StatusCode::BAD_REQUEST, "token", &e)))?;
        typed
    };
    Jira::new(&site, &email, &token).map_err(|e| Box::new(jira_refusal(&e)))
}

/// `POST /api/jira`: `{action: connect|site|project|test|statuses|save|disconnect, …}`.
pub async fn change(headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    match body["action"].as_str().unwrap_or("") {
        "connect" => connect(headers.get(axum::http::header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("")).await,
        "site" => {
            let site = text(&body, "site");
            if !choices().iter().any(|(h, _)| *h == site) {
                return refuse(StatusCode::BAD_REQUEST, "site", "Choose one of the sites your Atlassian account can use.");
            }
            match use_site(&site).await {
                Ok(s) => axum::Json(json!({ "ok": format!("Connected to {}.", s.site), "settings": s })).into_response(),
                Err(r) => r,
            }
        }
        "project" => project(&body).await,
        "projects" => match jira::Jira::connected() {
            Ok(j) => match j.projects().await {
                Ok(p) => axum::Json(json!({ "projects": p.iter().map(|(k, n)| json!({ "key": k, "name": n })).collect::<Vec<_>>() })).into_response(),
                Err(e) => jira_refusal(&e),
            },
            Err(e) => jira_refusal(&e),
        },
        "test" => test(&body).await,
        "statuses" => statuses(&body).await,
        "save" => save(&body).await,
        "disconnect" => disconnect().await,
        "approve" => decide(&body, true).await,
        "decline" => decide(&body, false).await,
        _ => refuse(StatusCode::BAD_REQUEST, "other", "Unknown Jira action."),
    }
}

/// The Jira writes Claude Code asked for, as the Integrations page shows them (fastpilot's words and the
/// exact text that is sent).
pub fn requests_shown() -> Vec<Value> {
    jira::requests::list()
        .into_iter()
        .map(|r| json!({ "id": r.id, "kind": r.kind, "title": r.title(), "site": r.site, "project": r.project, "key": r.key, "summary": r.summary, "text": r.text, "created_ms": r.created_ms, "expires_ms": r.created_ms + jira::requests::EXPIRY_MS }))
        .collect()
}

/// `{action: approve|decline, id}`: the person's click on a request from Claude Code (Security,
/// Medium 1). The request is taken out first, so it is decided once.
async fn decide(body: &Value, approve: bool) -> Response {
    let id = text(body, "id");
    let Some(r) = jira::requests::take(&id) else {
        return refuse(StatusCode::NOT_FOUND, "gone", "This request is not waiting any more (it was decided, or it expired after 30 minutes). Nothing was sent.");
    };
    if !approve {
        fastpilot_core::change::audit("jira", None, Some(&r.site), "dashboard", "declined");
        return axum::Json(json!({ "message": "Dropped. Nothing was sent to Jira.", "requests": requests_shown() })).into_response();
    }
    match jira::requests::send(&r, "dashboard").await {
        Ok((msg, link)) => axum::Json(json!({ "message": msg, "link": link, "requests": requests_shown() })).into_response(),
        Err(e) => refuse(StatusCode::CONFLICT, "send", &e),
    }
}

fn saved_token_used(body: &Value) -> bool {
    text(body, "token").is_empty()
}

async fn test(body: &Value) -> Response {
    let j = match account(body) {
        Ok(j) => j,
        Err(r) => return *r,
    };
    let name = match j.myself().await {
        Ok(n) => n,
        Err(e) => {
            if e == JiraError::Refused && saved_token_used(body) {
                jira::mark(true, None);
            }
            return jira_refusal(&e);
        }
    };
    let projects = match j.projects().await {
        Ok(p) => p,
        Err(e) => return jira_refusal(&e),
    };
    if saved_token_used(body) {
        jira::mark(false, Some(&name));
    }
    let list: Vec<Value> = projects.iter().map(|(k, n)| json!({ "key": k, "name": n })).collect();
    axum::Json(json!({ "ok": true, "site": j.site(), "display_name": name, "projects": list })).into_response()
}

async fn statuses(body: &Value) -> Response {
    let j = match account(body) {
        Ok(j) => j,
        Err(r) => return *r,
    };
    match j.statuses(&text(body, "project")).await {
        Ok(s) => axum::Json(json!({ "statuses": s })).into_response(),
        Err(e) => jira_refusal(&e),
    }
}

async fn save(body: &Value) -> Response {
    let j = match account(body) {
        Ok(j) => j,
        Err(r) => return *r,
    };
    // Checked again on the server: the page cannot skip the test.
    let name = match j.myself().await {
        Ok(n) => n,
        Err(e) => return jira_refusal(&e),
    };
    let projects = match j.projects().await {
        Ok(p) => p,
        Err(e) => return jira_refusal(&e),
    };
    let key = text(body, "project");
    let Some((_, project_name)) = projects.iter().find(|(k, _)| *k == key) else {
        return refuse(StatusCode::BAD_REQUEST, "project", "Choose a project: Save stays off until one is chosen.");
    };
    let qa = jira::plain(&text(body, "qa_status"), 60);
    let typed = text(body, "token");
    let site = j.site().to_owned();
    let email = text(body, "email");
    let was = jira::settings();
    if !typed.is_empty() && jira::token::set(&typed, j.origin()).is_err() {
        return refuse(
            StatusCode::CONFLICT,
            "keychain",
            if jira::token::kept_in() == "this Mac's Keychain" {
                "macOS did not let fastpilot save the token. Nothing was saved. Press Save again and choose Allow when macOS asks."
            } else {
                "fastpilot could not save the token in this computer's secret store. Nothing was saved."
            },
        );
    }
    let s = jira::Settings {
        site: site.clone(),
        email: email.clone(),
        project_key: key.clone(),
        project_name: jira::plain(project_name, 120),
        qa_status: qa,
        display_name: jira::plain(&name, 120),
        checked_ms: jira::now_ms(),
        needs_attention: false,
        auth: String::new(),
    };
    if let Err(e) = jira::save_settings(&s) {
        return refuse(StatusCode::CONFLICT, "other", &format!("The settings could not be saved: {e}"));
    }
    // Site and email only, never the token.
    let what = format!("{site} {email}");
    fastpilot_core::change::audit("jira", was.as_ref().map(|w| format!("{} {}", w.site, w.email)).as_deref(), Some(&what), "dashboard", if was.is_some() { "changed" } else { "connected" });
    axum::Json(json!({ "ok": format!("Saved. Jira is connected to {site} · {} ({key}).", s.project_name), "settings": s })).into_response()
}

/// `{action: project, project}`: the default project of a signed-in connection.
async fn project(body: &Value) -> Response {
    let Some(mut s) = jira::settings().filter(|s| s.signed_in()) else { return jira_refusal(&JiraError::NotConnected) };
    let key = text(body, "project");
    let projects = match jira::Jira::connected() {
        Ok(j) => match j.projects().await {
            Ok(p) => p,
            Err(e) => return jira_refusal(&e),
        },
        Err(e) => return jira_refusal(&e),
    };
    let Some((_, name)) = projects.iter().find(|(k, _)| *k == key) else { return refuse(StatusCode::BAD_REQUEST, "project", "Choose one of the projects in the list.") };
    s.project_key = key;
    s.project_name = jira::plain(name, 120);
    match jira::save_settings(&s) {
        Ok(()) => axum::Json(json!({ "ok": format!("Default project: {} ({}).", s.project_name, s.project_key), "settings": s })).into_response(),
        Err(e) => refuse(StatusCode::CONFLICT, "other", &format!("The settings could not be saved: {e}")),
    }
}

async fn disconnect() -> Response {
    let was = jira::settings();
    jira::clear_settings();
    CHOICES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    // Signed in with Atlassian: revoked there, then forgotten here. An API token: removed.
    let revoked = jira::oauth::revoke().await;
    if was.as_ref().is_some_and(jira::Settings::signed_in) {
        fastpilot_core::change::audit("jira", was.as_ref().map(|w| w.site.clone()).as_deref(), None, "dashboard", "disconnected");
        return match revoked {
            Ok(()) => axum::Json(json!({ "ok": "Jira disconnected. fastpilot's access was revoked at Atlassian and removed from this computer. Tickets already created are still in Jira." })).into_response(),
            Err(_) => refuse(StatusCode::CONFLICT, "keychain", "Could not remove the sign-in from the key store. On a Mac, open Keychain Access and delete \"fastpilot jira\"."),
        };
    }
    let removed = jira::token::delete();
    let what = was.as_ref().map(|w| format!("{} {}", w.site, w.email));
    fastpilot_core::change::audit("jira", what.as_deref(), None, "dashboard", "disconnected");
    match removed {
        Ok(()) => axum::Json(json!({ "ok": "Jira disconnected. Your token was removed from this computer. Tickets already created are still in Jira." })).into_response(),
        Err(_) => refuse(
            StatusCode::CONFLICT,
            "keychain",
            "Could not remove the token from the key store. The settings were cleared, but the token may still be stored. On a Mac, open Keychain Access and delete \"fastpilot jira\".",
        ),
    }
}
