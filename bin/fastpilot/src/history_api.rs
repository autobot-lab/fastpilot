//! The History page of `fastpilot ui` (D7, v0.4): past runs and Chat conversations of this project,
//! deleting them one by one, several, or all, and auto-delete after N days. The store is
//! `fastpilot_memory::history`; requests pass the same guard as the rest of the page (own cookie,
//! JSON from our own page for changes).

use std::collections::HashMap;

use axum::Json;
use axum::body::Body;
use axum::extract::Query;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use fastpilot_memory::history::{Deleted, History, now_ms};
use serde_json::{Value, json};

fn refuse(status: StatusCode, why: &str) -> Response {
    let mut r = (status, format!("fastpilot ui: {why}\n")).into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    r
}

/// `GET /api/history`: runs, conversations and the auto-delete setting (old items go first).
pub async fn list() -> Response {
    let h = History::from_env();
    h.prune(now_ms());
    Json(json!({ "runs": h.runs(), "chats": h.chats(), "keep_days": h.keep_days(), "keep_default": h.keep_days_is_default() })).into_response()
}

/// Conversations to delete, minus the one Chat is still answering (with why).
fn releasable(ids: Vec<String>) -> (Vec<String>, Option<String>) {
    let mut note = None;
    let ok = ids
        .into_iter()
        .filter(|id| match crate::chat::release(id) {
            Ok(()) => true,
            Err(e) => {
                note = Some(e);
                false
            }
        })
        .collect();
    (ok, note)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

/// `POST /api/history`: `{action: delete, runs: [...], chats: [...]}`, `{action: delete_all}`, or
/// `{action: keep_days, days: N | null}`. Only the page's own buttons send these (never Chat).
pub async fn change(Json(body): Json<Value>) -> Response {
    let h = History::from_env();
    let (deleted, note): (Deleted, Option<String>) = match body["action"].as_str().unwrap_or("") {
        "delete" => {
            let (chats, note) = releasable(strings(&body["chats"]));
            (h.delete(&strings(&body["runs"]), &chats), note)
        }
        "delete_all" => {
            let (chats, note) = releasable(h.chats().into_iter().map(|c| c.id).collect());
            let runs: Vec<String> = h.runs().into_iter().map(|r| r.id).collect();
            (h.delete(&runs, &chats), note)
        }
        "keep_days" => {
            let days = match &body["days"] {
                Value::Null => None,
                v => match v.as_u64().and_then(|d| u32::try_from(d).ok()) {
                    Some(d) => Some(d),
                    None => return refuse(StatusCode::BAD_REQUEST, "Auto-delete takes a number of days, or off."),
                },
            };
            match h.set_keep_days(days) {
                Ok(d) => (d, None),
                Err(e) => return refuse(StatusCode::BAD_REQUEST, &e.to_string()),
            }
        }
        _ => return refuse(StatusCode::BAD_REQUEST, "Unknown History action."),
    };
    Json(json!({ "deleted": deleted, "note": note, "keep_days": h.keep_days(), "keep_default": h.keep_days_is_default() })).into_response()
}

/// `GET /api/history/report?id=suite:<id>|run:<folder>`: a run's HTML report, sandboxed like the
/// schedule reports.
pub async fn report(Query(q): Query<HashMap<String, String>>) -> Response {
    let id = q.get("id").map(String::as_str).unwrap_or("");
    match History::from_env().report(id).and_then(|p| std::fs::read(p).ok()) {
        Some(body) => Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header("Content-Security-Policy", "sandbox; default-src 'none'; img-src data:; media-src data:; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'")
            .header("X-Content-Type-Options", "nosniff")
            .body(Body::from(body))
            .unwrap_or_default(),
        None => refuse(StatusCode::NOT_FOUND, "This run has no report (it may have been deleted)."),
    }
}

/// `GET /api/history/chat?id=<id>`: one saved conversation, to read.
pub async fn conversation(Query(q): Query<HashMap<String, String>>) -> Response {
    match q.get("id").and_then(|id| History::from_env().chat(id)) {
        Some(c) => Json(c).into_response(),
        None => refuse(StatusCode::NOT_FOUND, "This conversation is not in History (it may have been deleted)."),
    }
}
