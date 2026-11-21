//! v0.6 1.2 Connect Jira: the dashboard's Apps › Jira page (design apps-jira-v10, sheet 02). Only
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
    axum::Json(json!({
        "connected": s.is_some() && jira::token::has(),
        "settings": s,
        "kept_in": jira::token::kept_in(),
    }))
    .into_response()
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

/// `POST /api/jira`: `{action: test|statuses|save|disconnect, …}`.
pub async fn change(axum::Json(body): axum::Json<Value>) -> Response {
    match body["action"].as_str().unwrap_or("") {
        "test" => test(&body).await,
        "statuses" => statuses(&body).await,
        "save" => save(&body).await,
        "disconnect" => disconnect(),
        _ => refuse(StatusCode::BAD_REQUEST, "other", "Unknown Jira action."),
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
    };
    if let Err(e) = jira::save_settings(&s) {
        return refuse(StatusCode::CONFLICT, "other", &format!("The settings could not be saved: {e}"));
    }
    // Site and email only, never the token.
    let what = format!("{site} {email}");
    fastpilot_core::change::audit("jira", was.as_ref().map(|w| format!("{} {}", w.site, w.email)).as_deref(), Some(&what), "dashboard", if was.is_some() { "changed" } else { "connected" });
    axum::Json(json!({ "ok": format!("Saved. Jira is connected to {site} · {} ({key}).", s.project_name), "settings": s })).into_response()
}

fn disconnect() -> Response {
    let was = jira::settings();
    jira::clear_settings();
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
