//! `fastpilot ui`: the local settings and device page (C1+C2), served by this binary.
//!
//! - Listens on 127.0.0.1 only, on a random port (`--port N` to pick one).
//! - The URL printed at start carries a random token; opening it sets an HttpOnly, SameSite=Strict
//!   cookie and redirects to the page without the token. Every request needs that cookie (or the
//!   token), a `Host` of 127.0.0.1/localhost on this port (no DNS rebinding) and, when a browser
//!   sends one, our own `Origin`. Changes are POSTs with JSON from our own Origin only.
//! - Everything (HTML, CSS, JS, icons, fonts) is inside the binary: no CDN, works offline.
//! - Changes made here are the user's: they may loosen guard rails, and like every change they
//!   apply from the next run (running fastpilot processes keep the rails they started with).
//!   Secret values are never read or sent: only the names in `vars`.

use std::error::Error;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use fastpilot_core::config::{self, Source};
use serde_json::{Value, json};

use crate::doctor;

type Res<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub struct Ui {
    token: String,
    port: u16,
}

/// Settings the page shows and changes; nothing else can be written through it.
const KEYS: &[&str] = &[
    "model_replay",
    "model_explore",
    "allowed_domains",
    "confirm_actions",
    "block_actions",
    "allow_app_install",
    "allow_app_reset",
    "allow_physical_devices",
    "allow_device_settings",
    "allow_eval_script",
    "upload_dirs",
    "state_expire_days",
    "max_steps",
    "test_timeout",
    "vars",
];

pub async fn ui(args: &[String]) -> Res<()> {
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let port: u16 = value("--port").map(|p| p.parse()).transpose().map_err(|_| "--port takes a number")?.unwrap_or(0);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let port = listener.local_addr()?.port();
    let state = Arc::new(Ui { token: random_hex(32)?, port });
    let url = format!("http://127.0.0.1:{port}/?token={}", state.token);
    println!("{url}");
    println!("fastpilot ui: only this computer can open it; keep the link to yourself. Ctrl-C stops it.");
    if !args.iter().any(|a| a == "--no-open") {
        open_browser(&url);
    }
    axum::serve(listener, router(state)).await?;
    Ok(())
}

pub fn router(state: Arc<Ui>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/{*path}", get(asset))
        .route("/api/settings", get(settings).post(change))
        .route("/api/doctor", get(doctor_api))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

fn random_hex(bytes: usize) -> Res<String> {
    use std::io::Read;
    let mut buf = vec![0u8; bytes];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    if let Err(e) = std::process::Command::new(opener).arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
        eprintln!("could not open a browser ({opener}: {e}); open the link above yourself");
    }
}

/// Same length and bytes, without an early exit on the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn cookie_name(port: u16) -> String {
    format!("fp_ui_{port}")
}

fn refuse(status: StatusCode, why: &str) -> Response {
    let mut r = (status, format!("fastpilot ui: {why}\n")).into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    r
}

/// Host, Origin, token / cookie and, for changes, JSON from our own page.
async fn guard(State(ui): State<Arc<Ui>>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let ours = [format!("127.0.0.1:{}", ui.port), format!("localhost:{}", ui.port)];
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    if !ours.iter().any(|o| o == host) {
        return refuse(StatusCode::MISDIRECTED_REQUEST, "wrong Host: open the 127.0.0.1 link printed by `fastpilot ui`");
    }
    let origin = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok());
    if let Some(o) = origin
        && !ours.iter().any(|h| o == format!("http://{h}"))
    {
        return refuse(StatusCode::FORBIDDEN, "requests from other sites are refused");
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let json = headers.get(header::CONTENT_TYPE).and_then(|h| h.to_str().ok()).is_some_and(|c| c.starts_with("application/json"));
        if origin.is_none() || !json {
            return refuse(StatusCode::FORBIDDEN, "changes are only accepted from the fastpilot ui page");
        }
    }
    let query_token = req.uri().query().and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token="))).map(str::to_owned);
    let cookie_ok = cookie(headers, &cookie_name(ui.port)).is_some_and(|c| same(&c, &ui.token));
    match query_token {
        Some(t) if same(&t, &ui.token) => {
            if req.method() == Method::GET && req.uri().path() == "/" {
                // Trade the token in the address bar for a cookie, and drop it from the URL.
                let mut r = Response::builder().status(StatusCode::SEE_OTHER).header(header::LOCATION, "/").body(Body::empty()).unwrap_or_default();
                if let Ok(v) = HeaderValue::from_str(&format!("{}={}; HttpOnly; SameSite=Strict; Path=/", cookie_name(ui.port), ui.token)) {
                    r.headers_mut().insert(header::SET_COOKIE, v);
                }
                return r;
            }
        }
        Some(_) => return refuse(StatusCode::UNAUTHORIZED, "wrong token: use the link printed by `fastpilot ui`"),
        None if cookie_ok => {}
        None => return refuse(StatusCode::UNAUTHORIZED, "open the link printed by `fastpilot ui` (it carries the access token)"),
    }
    let mut r = next.run(req).await;
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; img-src 'self' data: blob:; style-src 'self'; script-src 'self'; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"),
    );
    r
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then(|| v.to_owned())
    })
}

// ---- static files -------------------------------------------------------------------------

const ICONS: &[(&str, &str)] = &[
    ("alert", include_str!("../ui/icons/alert.svg")),
    ("chev", include_str!("../ui/icons/chev.svg")),
    ("clock", include_str!("../ui/icons/clock.svg")),
    ("close", include_str!("../ui/icons/close.svg")),
    ("device", include_str!("../ui/icons/device.svg")),
    ("gauge", include_str!("../ui/icons/gauge.svg")),
    ("globe", include_str!("../ui/icons/globe.svg")),
    ("info", include_str!("../ui/icons/info.svg")),
    ("key", include_str!("../ui/icons/key.svg")),
    ("live", include_str!("../ui/icons/live.svg")),
    ("lock", include_str!("../ui/icons/lock.svg")),
    ("mark", include_str!("../ui/icons/mark.svg")),
    ("model", include_str!("../ui/icons/model.svg")),
    ("play", include_str!("../ui/icons/play.svg")),
    ("plus", include_str!("../ui/icons/plus.svg")),
    ("refresh", include_str!("../ui/icons/refresh.svg")),
    ("risk", include_str!("../ui/icons/risk.svg")),
    ("settings", include_str!("../ui/icons/settings.svg")),
    ("shield", include_str!("../ui/icons/shield.svg")),
    ("stop", include_str!("../ui/icons/stop.svg")),
    ("tests", include_str!("../ui/icons/tests.svg")),
];

/// The icons as one inline SVG sprite (`<use href="#i-live"/>`), so they take `currentColor`.
fn sprite() -> String {
    let mut out = String::from(r#"<svg class="sprite" width="0" height="0" aria-hidden="true">"#);
    for (name, svg) in ICONS {
        let Some(start) = svg.find("<svg") else { continue };
        let open_end = svg[start..].find('>').map(|i| start + i).unwrap_or(start);
        let attrs: String = svg[start + 4..open_end]
            .split_whitespace()
            .filter(|a| !a.starts_with("xmlns") && !a.starts_with("width") && !a.starts_with("height"))
            .collect::<Vec<_>>()
            .join(" ");
        let inner = svg[open_end + 1..].trim_end().trim_end_matches("</svg>");
        out.push_str(&format!(r#"<symbol id="i-{name}" {attrs}>{inner}</symbol>"#));
    }
    out.push_str("</svg>");
    out
}

async fn index() -> Response {
    let html = include_str!("../ui/index.html").replace("<!--SPRITE-->", &sprite());
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
}

fn static_file(path: &str) -> Option<(&'static str, &'static [u8])> {
    Some(match path {
        "app.css" => ("text/css; charset=utf-8", include_bytes!("../ui/app.css")),
        "app.js" => ("text/javascript; charset=utf-8", include_bytes!("../ui/app.js")),
        "logo.svg" => ("image/svg+xml", include_bytes!("../ui/logo.svg")),
        "fonts/bricolage-grotesque-600.woff2" => ("font/woff2", include_bytes!("../ui/fonts/bricolage-grotesque-600.woff2")),
        "fonts/bricolage-grotesque-700.woff2" => ("font/woff2", include_bytes!("../ui/fonts/bricolage-grotesque-700.woff2")),
        "fonts/ibm-plex-sans-400.woff2" => ("font/woff2", include_bytes!("../ui/fonts/ibm-plex-sans-400.woff2")),
        "fonts/ibm-plex-sans-500.woff2" => ("font/woff2", include_bytes!("../ui/fonts/ibm-plex-sans-500.woff2")),
        "fonts/ibm-plex-sans-600.woff2" => ("font/woff2", include_bytes!("../ui/fonts/ibm-plex-sans-600.woff2")),
        "fonts/jetbrains-mono-400.woff2" => ("font/woff2", include_bytes!("../ui/fonts/jetbrains-mono-400.woff2")),
        "fonts/jetbrains-mono-500.woff2" => ("font/woff2", include_bytes!("../ui/fonts/jetbrains-mono-500.woff2")),
        "fonts/LICENSE-bricolage-grotesque.txt" => ("text/plain; charset=utf-8", include_bytes!("../ui/fonts/bricolage-grotesque-OFL.txt")),
        "fonts/LICENSE-ibm-plex-sans.txt" => ("text/plain; charset=utf-8", include_bytes!("../ui/fonts/ibm-plex-sans-OFL.txt")),
        "fonts/LICENSE-jetbrains-mono.txt" => ("text/plain; charset=utf-8", include_bytes!("../ui/fonts/jetbrains-mono-OFL.txt")),
        _ => return None,
    })
}

/// Only the files listed above: a path is a key, never a file system lookup.
async fn asset(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    match static_file(&path) {
        Some((ty, body)) => ([(header::CONTENT_TYPE, ty)], body).into_response(),
        None => refuse(StatusCode::NOT_FOUND, "no such file"),
    }
}

// ---- settings -----------------------------------------------------------------------------

fn kind(key: &str) -> &'static str {
    if config::LISTS.contains(&key) {
        "list"
    } else if config::FLAGS.contains(&key) {
        "flag"
    } else if matches!(key, "state_expire_days" | "max_steps" | "test_timeout") {
        "number"
    } else {
        "text"
    }
}

fn shown(key: &str, v: &str) -> Value {
    match kind(key) {
        "list" => json!(v.split(',').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>()),
        "flag" => json!(!matches!(v.trim().to_lowercase().as_str(), "0" | "false" | "no" | "off")),
        "number" => v.trim().parse::<u64>().map(|n| json!(n)).unwrap_or(json!(v)),
        _ => json!(v),
    }
}

async fn settings() -> Response {
    let list: Vec<Value> = KEYS
        .iter()
        .filter_map(|k| config::setting(k))
        .map(|s| {
            let (value, source) = match config::current(s.key) {
                Source::Env(v) => (shown(s.key, &v), "env"),
                Source::File(v) => (shown(s.key, &v), "file"),
                Source::Default => (Value::Null, "default"),
            };
            json!({ "key": s.key, "help": s.help, "kind": kind(s.key), "value": value, "source": source, "env": format!("FASTPILOT_{}", s.key.to_uppercase()) })
        })
        .collect();
    let defaults = json!({ "model_replay": crate::run::DEFAULT_REPLAY, "model_explore": crate::run::DEFAULT_EXPLORE, "max_steps": 30, "state_expire_days": 30 });
    axum::Json(json!({ "path": config::path().display().to_string(), "settings": list, "defaults": defaults })).into_response()
}

async fn change(axum::Json(body): axum::Json<Value>) -> Response {
    let Some(key) = body["key"].as_str().filter(|k| KEYS.contains(k)) else {
        return refuse(StatusCode::BAD_REQUEST, "this setting cannot be changed here");
    };
    if std::env::var(format!("FASTPILOT_{}", key.to_uppercase())).is_ok() {
        return refuse(
            StatusCode::CONFLICT,
            &format!("`{key}` is set by the environment (FASTPILOT_{}), which wins over the settings file: change it there", key.to_uppercase()),
        );
    }
    let value = match &body["value"] {
        Value::Null => None,
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if s.trim().is_empty() => None,
        Value::String(s) => Some(s.trim().to_owned()),
        Value::Array(items) if items.is_empty() => None,
        Value::Array(items) => {
            let items: Vec<&str> = items.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).collect();
            if items.iter().any(|s| s.contains(',')) {
                return refuse(StatusCode::BAD_REQUEST, "an item may not contain a comma");
            }
            (!items.is_empty()).then(|| items.join(","))
        }
        Value::Object(_) => return refuse(StatusCode::BAD_REQUEST, "unexpected value"),
    };
    match config::write(key, value.as_deref()) {
        Ok(_) => settings().await,
        Err(e) => refuse(StatusCode::BAD_REQUEST, &e),
    }
}

// ---- device (doctor) ----------------------------------------------------------------------

async fn doctor_api(Query(q): Query<std::collections::HashMap<String, String>>) -> Response {
    let areas: Vec<&str> = match q.get("area").map(String::as_str) {
        Some("web") => vec!["web"],
        Some("android") => vec!["android"],
        Some("ios") => vec!["ios"],
        Some("run") => vec!["run"],
        _ => vec!["web", "android", "ios", "run"],
    };
    let checks: Vec<Value> = doctor::checks(&areas)
        .await
        .into_iter()
        .map(|c| {
            json!({
                "area": c.area,
                "name": c.name,
                "status": match c.status { doctor::Status::Ok => "ok", doctor::Status::Warn => "warn", doctor::Status::Fail => "fail" },
                "detail": c.detail,
                "hint": c.hint,
                "setup": (!c.fixes.is_empty()).then(|| format!("fastpilot setup {}", c.area)),
            })
        })
        .collect();
    axum::Json(json!({ "checks": checks })).into_response()
}
