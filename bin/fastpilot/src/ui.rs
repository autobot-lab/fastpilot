//! `fastpilot ui`: the local settings and device page (C1+C2), served by this binary.
//!
//! - Listens on 127.0.0.1 only, on a random port (`--port N` to pick one).
//! - The link printed at start carries a one-time code (valid 10 minutes). Opening it trades the
//!   code for a session cookie with a different random value (HttpOnly, SameSite=Strict) and
//!   redirects to the page without it; the code is then spent. The browser is opened through a
//!   local redirect file (owner-only), so the code never appears in another process' arguments.
//!   Every request needs that cookie, a `Host` of 127.0.0.1/localhost on this port (no DNS
//!   rebinding) and, when a browser sends one, our own `Origin`. Changes are POSTs with JSON from
//!   our own Origin only.
//! - Run it yourself, not through an agent: whoever holds the session can loosen the limits.
//! - Everything (HTML, CSS, JS, icons, fonts) is inside the binary: no CDN, works offline.
//! - Changes made here are the user's: they may loosen guard rails, and like every change they
//!   apply from the next run (running fastpilot processes keep the rails they started with).
//!   Secret values are never read or sent: only the names in `vars`.
//! - Live (C7a): running sessions and their steps from `~/.fastpilot/live` (see
//!   `fastpilot_core::live`), and the screen of web sessions on fastpilot's own Chrome as an MJPEG
//!   stream that exists only while the page shows it. The browser's DevTools address stays here.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use fastpilot_core::config::{self, Source};
use serde_json::{Value, json};

use crate::doctor;

type Res<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub struct Ui {
    /// The one-time code of the printed link, until it is used or expires.
    code: std::sync::Mutex<Option<(String, std::time::Instant)>>,
    /// The session cookie handed out for the code (never equal to it).
    session: String,
    port: u16,
}

/// How long the printed link works.
const CODE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

const USAGE: &str = "usage: fastpilot ui [--no-open] [--port N]

Opens the dashboard (Chat, Live, Schedules, Settings, Device, …) in your browser, on this computer only.
  --no-open   only print the link, without opening a browser
  --port N    use port N (1024-65535); default: a free port

The link works once, for 10 minutes. Start fastpilot ui yourself, not through an AI agent:
whoever opens that page can loosen the Limits.";

impl Ui {
    pub fn new(port: u16) -> Res<Self> {
        Ok(Self { code: std::sync::Mutex::new(Some((random_hex(32)?, std::time::Instant::now()))), session: random_hex(32)?, port })
    }

    fn code(&self) -> String {
        self.code.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(c, _)| c.clone()).unwrap_or_default()
    }

    /// Spend the code: true once, for the right code within its time.
    fn redeem(&self, given: &str) -> bool {
        let mut slot = self.code.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_ref() {
            Some((c, at)) if same(c, given) && at.elapsed() < CODE_TTL => {
                *slot = None;
                true
            }
            _ => false,
        }
    }
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
    "allow_takeover",
    "upload_dirs",
    "state_expire_days",
    "max_steps",
    "test_timeout",
    "vars",
];

/// Print a usage problem as plain text (not a Rust error value) and exit 2.
fn fail_args(why: &str) -> ! {
    eprintln!("fastpilot ui: {why}\n\n{USAGE}");
    std::process::exit(2);
}

/// Print why the page cannot start and exit 1.
fn fail(why: &str) -> ! {
    eprintln!("fastpilot ui: {why}");
    std::process::exit(1);
}

pub async fn ui(args: &[String]) -> Res<()> {
    let mut port: u16 = 0;
    let mut open = true;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" | "help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--no-open" => open = false,
            "--port" => {
                let Some(v) = it.next() else { fail_args("--port needs a number (1024-65535)") };
                port = v.parse::<u16>().ok().filter(|p| *p >= 1024).unwrap_or_else(|| fail_args(&format!("--port {v}: use a number from 1024 to 65535")));
            }
            other => fail_args(&format!("unknown option `{other}`")),
        }
    }
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            fail(&format!("port {port} is already in use (another fastpilot ui?): pick another one with --port, or leave --port out to get a free one"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            fail(&format!("port {port} is not allowed for this user: pick 1024 to 65535, or leave --port out"))
        }
        Err(e) => fail(&format!("cannot open 127.0.0.1:{port}: {e}")),
    };
    let port = listener.local_addr()?.port();
    let state = Arc::new(Ui::new(port)?);
    let url = format!("http://127.0.0.1:{port}/?code={}", state.code());
    // Whoever started us may stop reading stdout after the link (a script, a test): a closed pipe
    // must not take the server down, so nothing here panics on a failed write.
    let say = |text: &str| {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{text}").and_then(|()| out.flush());
    };
    if open {
        match open_browser(&url) {
            Ok(()) => say(&format!("fastpilot ui: opened in your browser. If it did not open, use this link (once, within 10 minutes):\n{url}")),
            Err(e) => say(&format!("fastpilot ui: could not open a browser ({e}); open this link (once, within 10 minutes):\n{url}")),
        }
    } else {
        say(&url);
        say("fastpilot ui: open the link above once, within 10 minutes (only this computer can).");
    }
    say("Ctrl-C stops it.");
    tokio::spawn(keep_takeovers());
    axum::serve(listener, router(state)).await?;
    Ok(())
}

pub fn router(state: Arc<Ui>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/{*path}", get(asset))
        .route("/api/settings", get(settings).post(change))
        .route("/api/doctor", get(doctor_api))
        .route("/api/fix", get(crate::fix::api_state).post(crate::fix::api_change))
        .route("/api/apps", get(apps_api))
        .route("/api/map", get(map_api).post(map_change))
        .route("/api/map/thumb", get(map_thumb))
        .route("/api/jira", get(crate::jira_api::state).post(crate::jira_api::change))
        .route("/api/live", get(live_api))
        .route("/api/live/frames", get(frames))
        .route("/api/live/control", post(control_api))
        .route("/api/live/limit", post(live_limit))
        .route("/api/schedules", get(schedules_api).post(schedules_change))
        .route("/api/schedules/report", get(schedule_report))
        .route("/api/chat", get(chat_api).post(chat_send))
        .route("/api/chat/decide", post(chat_decide))
        .route("/api/chat/run", post(chat_run))
        .route("/api/chat/undo", post(chat_undo))
        .route("/api/agents", get(agents_api).post(agents_change))
        .route("/api/skills", get(skills_api))
        .route("/api/provider", get(provider_api).post(provider_change))
        .route("/api/history", get(crate::history_api::list).post(crate::history_api::change))
        .route("/api/history/report", get(crate::history_api::report))
        .route("/api/history/chat", get(crate::history_api::conversation))
        .route("/api/memory", get(crate::memory_api::list).post(crate::memory_api::change))
        .route("/api/notes", get(crate::memory_api::notes_list).post(crate::memory_api::notes_change))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

fn random_hex(bytes: usize) -> Res<String> {
    use std::io::Read;
    let mut buf = vec![0u8; bytes];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Open `url` in the default browser through an owner-only redirect file, so the link (and its
/// code) is not in the opener's arguments, where other processes could read it.
fn open_browser(url: &str) -> Res<()> {
    let dir = std::env::temp_dir().join(format!("fastpilot-ui-{}", random_hex(8)?));
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(&dir)?;
    let file = dir.join("open.html");
    {
        use std::io::Write;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&file)?;
        write!(f, r#"<!doctype html><meta charset="utf-8"><meta name="referrer" content="no-referrer"><meta http-equiv="refresh" content="0;url={url}"><title>fastpilot</title>"#)?;
    }
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let spawned = std::process::Command::new(opener).arg(&file).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    // The browser reads it right away; the code in it works once anyway.
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(30));
        let _ = std::fs::remove_dir_all(dir);
    });
    spawned.map(|_| ()).map_err(|e| format!("{opener}: {e}").into())
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
        return refuse(StatusCode::MISDIRECTED_REQUEST, "wrong host: open the 127.0.0.1 link that `fastpilot ui` printed");
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
    let code = req.uri().query().and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("code="))).map(str::to_owned);
    let cookie_ok = cookie(headers, &cookie_name(ui.port)).is_some_and(|c| same(&c, &ui.session));
    if let Some(code) = code {
        if req.method() != Method::GET || req.uri().path() != "/" {
            return refuse(StatusCode::UNAUTHORIZED, "the access code only opens the page");
        }
        if !ui.redeem(&code) {
            if cookie_ok {
                // Already in: an old link (reload, history) just goes to the page.
                return Response::builder().status(StatusCode::SEE_OTHER).header(header::LOCATION, "/").body(Body::empty()).unwrap_or_default();
            }
            return refuse(
                StatusCode::UNAUTHORIZED,
                "this link was already used or has expired (it works once, for 10 minutes). Run `fastpilot ui` again for a new link.",
            );
        }
        // Trade the code for the session cookie, and drop it from the address bar.
        let mut r = Response::builder().status(StatusCode::SEE_OTHER).header(header::LOCATION, "/").body(Body::empty()).unwrap_or_default();
        if let Ok(v) = HeaderValue::from_str(&format!("{}={}; HttpOnly; SameSite=Strict; Path=/", cookie_name(ui.port), ui.session)) {
            r.headers_mut().insert(header::SET_COOKIE, v);
        }
        return r;
    }
    if !cookie_ok {
        return refuse(StatusCode::UNAUTHORIZED, "open the link that `fastpilot ui` printed (it holds a one-time access code)");
    }
    let mut r = next.run(req).await;
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    // A route with its own, stricter policy (a sandboxed run report) keeps it.
    if !h.contains_key(header::CONTENT_SECURITY_POLICY) {
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; img-src 'self' data: blob:; style-src 'self'; script-src 'self'; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"),
        );
    }
    r
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then(|| v.to_owned())
    })
}

// ---- Model AI: OpenAI-compatible provider (B2) --------------------------------------------

/// The `provider` settings as the file has them now (this process pinned them at start), the
/// presets, and whether a key is stored; never the key.
async fn provider_api() -> Response {
    let file = |k: &str| match config::current(k) {
        Source::File(v) | Source::Env(v) => Some(v),
        Source::Default => None,
    };
    let name = file("provider").unwrap_or_else(|| "claude".into());
    let presets: Vec<Value> = config::PROVIDERS.iter().map(|(p, url)| json!({ "name": p, "base_url": url })).collect();
    let env = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty());
    let key = (name != "claude").then(|| {
        json!({ "has": crate::keystore::has(&name), "where": crate::keystore::backend().describe(), "origin": crate::keystore::origin(&name) })
    });
    axum::Json(json!({
        "provider": name,
        "base_url": file("provider_base_url"),
        "model": file("provider_model"),
        "presets": presets,
        "key": key,
        "env": { "provider": env("FASTPILOT_PROVIDER") || env("FASTPILOT_PROVIDER_BASE_URL") || env("FASTPILOT_PROVIDER_MODEL"), "key": env("FASTPILOT_PROVIDER_API_KEY") },
    }))
    .into_response()
}

/// `{action: save, provider, base_url, model, key?}`, `{action: delete-key}`, `{action: test}`.
/// The key arrives only in this POST body and is handed to the key store, never echoed.
async fn provider_change(axum::Json(body): axum::Json<Value>) -> Response {
    let text = |k: &str| body[k].as_str().map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
    let current = || match config::current("provider") {
        Source::File(v) | Source::Env(v) => v,
        Source::Default => "claude".into(),
    };
    let result: Result<Value, String> = match body["action"].as_str().unwrap_or("") {
        "save" => (|| {
            if let Err(e) = config::check() {
                return Err(format!("the settings file is damaged; fix it first: {e}"));
            }
            for k in ["FASTPILOT_PROVIDER", "FASTPILOT_PROVIDER_BASE_URL", "FASTPILOT_PROVIDER_MODEL"] {
                if std::env::var(k).is_ok_and(|v| !v.is_empty()) {
                    return Err(format!("{k} is set in the environment, which wins over the settings file: change it there"));
                }
            }
            let name = text("provider").unwrap_or_else(|| "claude".into());
            let preset = config::PROVIDERS.iter().find(|(p, _)| *p == name).ok_or("unknown provider")?;
            if name == "claude" {
                for k in ["provider", "provider_base_url", "provider_model"] {
                    config::write(k, None)?;
                }
                return Ok(json!({ "message": "Tests run with Claude Code again." }));
            }
            let (url, model) = (text("base_url"), text("model"));
            // Hosted presets talk to their own server only; a preset's own URL is stored as "unset",
            // so switching provider never leaves the previous server behind.
            let hosted = matches!(name.as_str(), "openai" | "openrouter" | "groq");
            let url = if hosted || url.as_deref().map(config::origin) == Some(preset.1.and_then(config::origin)) { None } else { url };
            if url.is_none() && preset.1.is_none() {
                return Err("enter the Base URL of the custom provider".into());
            }
            let model = model.ok_or("enter the model id, for example gpt-4.1-mini or llama3.1")?;
            // Values are checked by the settings themselves (https, no credentials, model id).
            config::write("provider_base_url", url.as_deref())?;
            config::write("provider_model", Some(&model))?;
            config::write("provider", Some(&name))?;
            let mut message = format!("Saved: {name} · {model}.");
            if let Some(key) = text("key") {
                let place = crate::keystore::set(&name, &key)?;
                message.push_str(&format!(" The API key is saved in {place}."));
            } else if crate::keystore::has(&name) {
                // A key saved for another server is not sent here (the run says so); say it now.
                let now = url.as_deref().or(preset.1).and_then(config::origin);
                if let Some(saved) = crate::keystore::origin(&name).filter(|o| Some(o) != now.as_ref()) {
                    message.push_str(&format!(" The {name} API key was saved for {saved}, not this server: enter the API key again."));
                }
            }
            Ok(json!({ "message": message }))
        })(),
        "delete-key" => {
            let name = current();
            crate::keystore::delete(&name).map(|()| json!({ "message": format!("The {name} API key was deleted.") }))
        }
        // A new process: it reads the settings as saved now (this one pinned them at start).
        "test" => match std::env::current_exe() {
            Ok(exe) => {
                let out = tokio::process::Command::new(exe).args(["config", "test-provider"]).stdin(std::process::Stdio::null()).kill_on_drop(true).output();
                match tokio::time::timeout(std::time::Duration::from_secs(90), out).await {
                    Ok(Ok(o)) => {
                        let said = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_owned();
                        if o.status.success() { Ok(json!({ "message": said })) } else { Err(said) }
                    }
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("the provider did not answer within 90 seconds".into()),
                }
            }
            Err(e) => Err(e.to_string()),
        },
        _ => Err("unknown command".into()),
    };
    match result {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => refuse(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}

// ---- Chat (C4) ----------------------------------------------------------------------------

/// `?agent=ID`: that agent's conversation (default: the first agent).
async fn chat_api(axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Response {
    let agent = q.get("agent").cloned().unwrap_or_else(|| crate::agents::DEFAULT.into());
    axum::Json(crate::chat::state(&agent)).into_response()
}

fn agent_of(body: &Value) -> String {
    body["agent"].as_str().filter(|a| !a.is_empty()).unwrap_or(crate::agents::DEFAULT).to_owned()
}

/// `{agent?, message}` starts the bot's answer; `{agent?, reset: true}` starts a new conversation.
async fn chat_send(axum::Json(body): axum::Json<Value>) -> Response {
    let agent = agent_of(&body);
    let r = if body["reset"] == true { crate::chat::reset(&agent) } else { crate::chat::send(&agent, body["message"].as_str().unwrap_or("")) };
    match r {
        Ok(()) => axum::Json(crate::chat::state(&crate::agents::get_or_default(&agent).id)).into_response(),
        Err(e) => refuse(StatusCode::CONFLICT, &e),
    }
}

/// `{id, apply, mode?, edit?, confirm?, agent?}`: the user's click on a proposal card (`confirm`: the
/// word the user typed on a card that asks for one, D9).
async fn chat_decide(axum::Json(body): axum::Json<Value>) -> Response {
    let Some(id) = body["id"].as_str() else { return refuse(StatusCode::BAD_REQUEST, "id is required") };
    let edit = body.get("edit").filter(|e| e.is_object());
    // v0.6: a Jira card is sent from here, with the token only in this process.
    let r = match crate::chat::decide_jira(id, body["apply"] == true, body["mode"].as_str(), edit).await {
        Some(r) => r,
        None => crate::chat::decide(id, body["apply"] == true, body["mode"].as_str(), edit, body["confirm"].as_str()),
    };
    match r {
        Ok(msg) => axum::Json(json!({ "message": msg, "chat": crate::chat::state(&agent_of(&body)) })).into_response(),
        Err(e) => refuse(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}

/// `{id, agent?}`: Run now on a saved test's card.
async fn chat_run(axum::Json(body): axum::Json<Value>) -> Response {
    let Some(id) = body["id"].as_str() else { return refuse(StatusCode::BAD_REQUEST, "id is required") };
    match crate::chat::run_saved(id) {
        Ok(msg) => axum::Json(json!({ "message": msg, "chat": crate::chat::state(&agent_of(&body)) })).into_response(),
        Err(e) => refuse(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}

/// `{id, agent?}`: Undo / Block again on an applied settings card (D9).
async fn chat_undo(axum::Json(body): axum::Json<Value>) -> Response {
    let Some(id) = body["id"].as_str() else { return refuse(StatusCode::BAD_REQUEST, "id is required") };
    match crate::chat::undo(id) {
        Ok(msg) => axum::Json(json!({ "message": msg, "chat": crate::chat::state(&agent_of(&body)) })).into_response(),
        Err(e) => refuse(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}

// ---- Agents (D10a) ------------------------------------------------------------------------

/// The agents, with what the page needs to show them: devices, suggestions, who is busy, and each
/// one's schedules in short.
async fn agents_api() -> Response {
    use crate::{agents, schedule};
    let busy = crate::chat::busy_agents();
    // D10d: an agent with a session waiting in the queue shows as queued.
    let claims = fastpilot_core::claim::entries();
    let store = schedule::load();
    let list: Vec<Value> = agents::list()
        .into_iter()
        .map(|a| {
            let scheds: Vec<String> = store
                .schedules
                .iter()
                .filter(|s| s.agent.as_deref().unwrap_or(agents::DEFAULT) == a.id)
                .map(|s| schedule::describe(&s.when))
                .collect();
            let (label, tested) = agents::model_label(&a);
            let queued = claims.iter().any(|e| e.waiting && e.agent.as_deref().unwrap_or(agents::DEFAULT) == a.id);
            let running = claims.iter().any(|e| !e.waiting && e.agent.as_deref().unwrap_or(agents::DEFAULT) == a.id);
            json!({ "agent": a, "initials": agents::initials(&a.name), "busy": busy.contains(&a.id) || running, "queued": queued, "schedules": scheds, "model_label": label, "tested": tested })
        })
        .collect();
    let devices: Vec<Value> = agents::DEVICES.iter().map(|(k, n)| json!({ "id": k, "name": n })).collect();
    // D10b: each provider with whether its API key is saved (Settings › AI model); never the key.
    let providers: Vec<Value> = agents::PROVIDERS
        .iter()
        .map(|(k, n)| {
            let key = match *k {
                "claude" => "Your Claude Code login",
                "ollama" => "No key needed (runs on this computer)",
                p if crate::keystore::has(p) => "API key saved",
                _ => "No API key yet: add it in Settings › AI model",
            };
            json!({ "id": k, "name": n, "key": key })
        })
        .collect();
    let claude: Vec<Value> = agents::CLAUDE_MODELS.iter().map(|(id, n, d)| json!({ "id": id, "name": n, "about": d })).collect();
    axum::Json(json!({ "agents": list, "devices": devices, "suggestions": agents::SUGGESTED_INSTRUCTIONS, "providers": providers, "claude_models": claude, "default_model": crate::run::model_explore() })).into_response()
}

/// D10c: the skill library (packs, skills, community); `?id=NAME`: one skill's full text.
async fn skills_api(axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Response {
    match q.get("id") {
        Some(id) if crate::skills_catalog::known(id) => match fastpilot_mcp::skills::get(id) {
            Some(text) => axum::Json(json!({ "id": id, "text": text })).into_response(),
            None => refuse(StatusCode::NOT_FOUND, "No such skill"),
        },
        Some(_) => refuse(StatusCode::NOT_FOUND, "No such skill"),
        None => axum::Json(crate::skills_catalog::library()).into_response(),
    }
}

/// `{action: create|update, agent}`, `{action: delete, id}`, `{action: suggest, agent}`.
async fn agents_change(axum::Json(body): axum::Json<Value>) -> Response {
    use crate::agents;
    let parse = || serde_json::from_value::<agents::Agent>(body["agent"].clone()).map_err(|e| format!("The agent is incomplete ({e})"));
    let r: Result<Value, String> = match body["action"].as_str().unwrap_or("") {
        "create" => parse().and_then(agents::create).map(|a| json!({ "agent": a })),
        "update" => parse().and_then(agents::update).map(|a| json!({ "agent": a })),
        "delete" => agents::delete(body["id"].as_str().unwrap_or("")).map(|()| json!({})),
        // D10c: the skills to tick for a new agent, each with its reason.
        "suggest" => {
            let a = &body["agent"];
            let devices: Vec<String> = a["devices"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
            let s = crate::skills_catalog::suggest(a["job"].as_str().unwrap_or(""), a["app"].as_str().unwrap_or(""), &devices);
            Ok(json!({ "suggested": s.iter().map(|(id, why)| json!({ "id": id, "why": why })).collect::<Vec<_>>() }))
        }
        _ => Err("Unknown action".into()),
    };
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => refuse(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}

// ---- Schedules (C3) --------------------------------------------------------------------------

/// Schedules with their last run; a webhook is shown only as its host.
/// v0.6 Apps page: what is connected. Jira shows the site, project and email, never the token;
/// Slack/webhook is the schedules' own notification (how many schedules use one).
/// PF1: the map being made or the latest one, with which screens have a test.
async fn map_api() -> Response {
    axum::Json(crate::map::state()).into_response()
}

/// PF1: `{action: start, target, platform, screens, minutes}` or `{action: stop}`.
async fn map_change(axum::Json(body): axum::Json<Value>) -> Response {
    match body["action"].as_str().unwrap_or("") {
        "start" => {
            let web = fastpilot_web::LaunchOptions { headless: std::env::var("FASTPILOT_HEADFUL").as_deref() != Ok("1"), executable: None };
            let opts = crate::map::options(body["target"].as_str().unwrap_or(""), body["platform"].as_str().unwrap_or("web"), body["screens"].as_u64(), body["minutes"].as_u64(), web);
            match opts.and_then(crate::map::start) {
                Ok(()) => axum::Json(crate::map::state()).into_response(),
                Err(e) => refuse(StatusCode::UNPROCESSABLE_ENTITY, &e),
            }
        }
        "stop" => {
            crate::map::stop();
            axum::Json(crate::map::state()).into_response()
        }
        _ => refuse(StatusCode::BAD_REQUEST, "Unknown map action."),
    }
}

/// PF1: a screen's thumbnail (`?target=…&id=<8 hex>`), as a JPEG.
async fn map_thumb(axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Response {
    match crate::map::thumb(q.get("target").map(String::as_str).unwrap_or(""), q.get("id").map(String::as_str).unwrap_or("")) {
        Some(bytes) => ([(axum::http::header::CONTENT_TYPE, "image/jpeg"), (axum::http::header::CACHE_CONTROL, "no-store")], bytes).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn apps_api() -> Response {
    let jira = fastpilot_core::jira::settings().filter(|_| fastpilot_core::jira::token::has()).map(|s| {
        json!({ "site": s.site, "email": s.email, "project_key": s.project_key, "project_name": s.project_name,
                "checked_ms": s.checked_ms, "needs_attention": s.needs_attention })
    });
    let store = crate::schedule::load();
    let webhooks = store.schedules.iter().filter(|s| s.notify.webhook && crate::schedule::secret_path(&s.id).exists()).count();
    let where_kept = fastpilot_core::jira::token::kept_in();
    // v0.6 (Security, Medium 1): Jira writes Claude Code asked for, waiting for a click here.
    let requests = if jira.is_some() { crate::jira_api::requests_shown() } else { Vec::new() };
    axum::Json(json!({ "jira": jira, "webhook_schedules": webhooks, "token_kept": where_kept, "jira_requests": requests })).into_response()
}

async fn schedules_api() -> Response {
    use crate::schedule;
    let store = schedule::load();
    let list: Vec<Value> = store
        .schedules
        .iter()
        .map(|s| {
            let hook = std::fs::read_to_string(schedule::secret_path(&s.id)).ok().map(|u| schedule::mask_webhook(u.trim()));
            let slots: Vec<String> = schedule::slots(&s.when).iter().map(|(h, m)| format!("{h:02}:{m:02}")).collect();
            let last = store.last.get(&s.id);
            json!({
                "schedule": s, "when_text": schedule::describe(&s.when), "watch_text": schedule::watch_text(s), "webhook": hook, "last": last,
                "running": schedule::is_running(last), "slots": slots, "days": schedule::run_days(&s.when),
                "tests_text": tests_text(&s.tests),
                "agent_name": crate::agents::get_or_default(s.agent.as_deref().unwrap_or(crate::agents::DEFAULT)).name,
            })
        })
        .collect();
    let cwd = std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned());
    let tests = std::env::current_dir().ok().map(|d| d.join("tests")).filter(|d| d.is_dir()).map(|d| d.to_string_lossy().into_owned());
    let found: Vec<Value> = tests.as_deref().map(|d| test_files(Path::new(d))).unwrap_or_default().into_iter().map(|(path, title)| json!({ "path": path, "title": title })).collect();
    axum::Json(json!({ "schedules": list, "backend": schedule::backend().name(), "cwd": cwd, "tests_hint": tests, "tests": found })).into_response()
}

/// The `.md` test cases in `dir`, with their titles (`# Title`, else the file name), by title.
pub fn test_files(dir: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "md")).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let title = test_title(&p);
            (p.to_string_lossy().into_owned(), title)
        })
        .collect();
    out.sort_by_key(|a| a.1.to_lowercase());
    out
}

/// A test case's title: its first `# ` line, else the file name.
pub fn test_title(p: &Path) -> String {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("# ").map(|x| x.trim().to_owned())).filter(|x| !x.is_empty()))
        .unwrap_or_else(|| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
}

/// What a schedule runs, in the user's words: `Login` or `all 4 tests in tests`.
fn tests_text(tests: &str) -> String {
    let p = Path::new(tests);
    if p.is_file() {
        return test_title(p);
    }
    let n = test_files(p).len();
    let folder = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| tests.to_owned());
    // The project's own `tests` folder needs no name.
    let in_folder = if folder == "tests" { String::new() } else { format!(" in {folder}") };
    match n {
        0 => format!("no tests yet{in_folder}"),
        1 => format!("the 1 test{in_folder}"),
        n => format!("all {n} tests{in_folder}"),
    }
}

/// The HTML report of a schedule's last run, from the schedules' own report folder only. Sandboxed:
/// the report shows tested pages' text, and must not reach this page's API.
async fn schedule_report(axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Response {
    use crate::schedule;
    let id = q.get("id").cloned().unwrap_or_default();
    let store = schedule::load();
    let file = store.last.get(&id).and_then(|l| l.report.clone()).map(PathBuf::from);
    let dir = std::fs::canonicalize(schedule::reports_dir()).ok();
    let real = file.and_then(|f| std::fs::canonicalize(f).ok()).filter(|f| dir.as_ref().is_some_and(|d| f.starts_with(d)) && f.is_file());
    match real.and_then(|f| std::fs::read(f).ok()) {
        Some(body) => Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header("Content-Security-Policy", "sandbox; default-src 'none'; img-src data:; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'")
            .header("X-Content-Type-Options", "nosniff")
            .body(Body::from(body))
            .unwrap_or_default(),
        None => refuse(StatusCode::NOT_FOUND, "This run has no report (it may have been removed)."),
    }
}

/// `{action: save, schedule, webhook?}`, `{action: enable|disable|delete|run|test, id}`.
async fn schedules_change(axum::Json(body): axum::Json<Value>) -> Response {
    use crate::schedule;
    let id = body["id"].as_str().unwrap_or("").to_owned();
    let result: Result<Value, String> = match body["action"].as_str().unwrap_or("") {
        "save" => match serde_json::from_value::<schedule::Schedule>(body["schedule"].clone()) {
            Ok(s) => (|| {
                let fresh = s.id.is_empty();
                let s = schedule::with_watch(s, body["watch_hours"].as_u64())?;
                let watch = s.watch.is_some();
                let s = schedule::save(s, body["webhook"].as_str())?;
                // R1: a new release watch takes its "before" run right away.
                if watch && fresh {
                    schedule::start_now(&s.id)?;
                }
                Ok(json!({ "id": s.id, "watch": watch && fresh }))
            })(),
            Err(e) => Err(format!("The schedule is incomplete ({e})")),
        },
        "enable" => schedule::set_enabled(&id, true).map(|()| json!({})),
        "disable" => schedule::set_enabled(&id, false).map(|()| json!({})),
        "delete" => schedule::delete(&id).map(|()| json!({})),
        "run" => schedule::start_now(&id).map(|()| json!({})),
        "test" => match schedule::load().schedules.into_iter().find(|s| s.id == id) {
            Some(s) => {
                let sent = schedule::notify(&s, &format!("Test notification: {}", s.name), &format!("fastpilot: this is a test notification for the schedule \"{}\".", s.name)).await;
                if sent.is_empty() { Err("This schedule has no notification yet".into()) } else { Ok(json!({ "sent": sent })) }
            }
            None => Err("Schedule not found".into()),
        },
        _ => Err("Unknown command".into()),
    };
    match result {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => refuse(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}

// ---- Ambil alih (C7b) ---------------------------------------------------------------------

/// The control socket of a fastpilot process (see `fastpilot_core::live`).
fn control_socket(pid: u32) -> std::path::PathBuf {
    fastpilot_core::live::dir().join(format!("{pid}.sock"))
}

fn can_control(pid: u32) -> bool {
    cfg!(unix) && control_socket(pid).exists()
}

/// One request to a fastpilot process' control socket, and its answer.
#[cfg(unix)]
async fn control(pid: u32, req: &Value) -> Result<Value, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let gone = "that fastpilot process does not take control (it has stopped, or it is an older version)";
    let mut s = tokio::net::UnixStream::connect(control_socket(pid)).await.map_err(|_| gone.to_owned())?;
    s.write_all(format!("{req}\n").as_bytes()).await.map_err(|_| gone.to_owned())?;
    let mut line = String::new();
    let mut r = tokio::io::BufReader::new(s);
    match tokio::time::timeout(std::time::Duration::from_secs(20), r.read_line(&mut line)).await {
        Ok(Ok(n)) if n > 0 => {}
        Ok(_) => return Err(gone.to_owned()),
        Err(_) => return Err("the device did not answer within 20 seconds".to_owned()),
    }
    let v: Value = serde_json::from_str(&line).map_err(|_| gone.to_owned())?;
    if v["ok"].as_bool() == Some(true) { Ok(v) } else { Err(v["error"].as_str().unwrap_or("failed").to_owned()) }
}

#[cfg(not(unix))]
async fn control(_pid: u32, _req: &Value) -> Result<Value, String> {
    Err("Take over is not available on this system yet".to_owned())
}

/// Android / iOS screens: the device's own screenshot, asked from the fastpilot process that holds
/// the session, at most `fps` per second, only while the page reads this stream.
fn device_frames(pid: u32, session: String, fps: u64) -> Response {
    let period = std::time::Duration::from_millis(1000 / fps.max(1));
    let frames = futures_util::stream::unfold((None::<tokio::time::Instant>, 0u32), move |(last, failures)| {
        let session = session.clone();
        async move {
            if let Some(last) = last {
                tokio::time::sleep_until(last + period).await;
            }
            let started = tokio::time::Instant::now();
            match control(pid, &json!({ "op": "frame", "session": session })).await {
                Ok(v) => {
                    use base64::Engine;
                    let jpeg = base64::engine::general_purpose::STANDARD.decode(v["jpeg"].as_str().unwrap_or("")).ok()?;
                    let mut part = format!("Content-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", jpeg.len()).into_bytes();
                    part.extend_from_slice(&jpeg);
                    part.extend_from_slice(format!("\r\n--{BOUNDARY}\r\n").as_bytes());
                    Some((Ok::<_, std::io::Error>(part), (Some(started), 0)))
                }
                // A busy device may miss a frame; three in a row end the stream.
                Err(_) if failures < 2 => Some((Ok(Vec::new()), (Some(started), failures + 1))),
                Err(_) => None,
            }
        }
    });
    let first = futures_util::stream::once(async { Ok::<_, std::io::Error>(format!("--{BOUNDARY}\r\n").into_bytes()) });
    let stream = futures_util::StreamExt::chain(first, frames);
    let mut r = Response::new(Body::from_stream(stream));
    if let Ok(v) = HeaderValue::from_str(&format!("multipart/x-mixed-replace; boundary={BOUNDARY}")) {
        r.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    r
}

/// `{pid, session, op, ...}`: Ambil alih / Kembalikan, and the user's taps, swipes, text and keys
/// while the session is taken over. Only from this page (cookie, Origin, JSON: see `guard`).
/// Takeovers started from this page: `(pid, session)` -> token. Only in this process' memory: the
/// token is never written to a file or sent to the browser.
static TAKEOVERS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(u32, String), String>>> = std::sync::LazyLock::new(Default::default);
/// When the page last asked for the Live state (it does every second on Live, every few on others).
static PAGE_SEEN: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

fn takeovers() -> std::sync::MutexGuard<'static, std::collections::HashMap<(u32, String), String>> {
    TAKEOVERS.lock().unwrap_or_else(|p| p.into_inner())
}

/// Keep this page's takeovers alive while the page is open; when it closes, the fastpilot process
/// ends them by itself (`live::IDLE`).
async fn keep_takeovers() {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let open = PAGE_SEEN.lock().unwrap_or_else(|p| p.into_inner()).is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(20));
        if !open {
            continue;
        }
        let list: Vec<((u32, String), String)> = takeovers().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for ((pid, session), token) in list {
            if control(pid, &json!({ "op": "keep", "session": session, "token": token })).await.is_err() {
                takeovers().remove(&(pid, session));
            }
        }
    }
}

async fn control_api(axum::Json(req): axum::Json<Value>) -> Response {
    let Some(pid) = req["pid"].as_u64().and_then(|p| u32::try_from(p).ok()) else {
        return refuse(StatusCode::BAD_REQUEST, "pid is required");
    };
    let Some(session) = req["session"].as_str().map(str::to_owned) else {
        return refuse(StatusCode::BAD_REQUEST, "session is required");
    };
    let op = req["op"].as_str().unwrap_or("");
    if !matches!(op, "takeover" | "release" | "tap" | "swipe" | "type" | "key" | "back") {
        return refuse(StatusCode::BAD_REQUEST, "unknown command");
    }
    if !fastpilot_core::live::read_all().iter().any(|p| p.pid == pid) {
        return refuse(StatusCode::NOT_FOUND, "that fastpilot process has stopped");
    }
    let key = (pid, session.clone());
    let mut fwd = json!({ "op": op, "session": session });
    if op != "takeover" {
        let Some(token) = takeovers().get(&key).cloned() else {
            return refuse(StatusCode::CONFLICT, "take over this session first (from this page)");
        };
        fwd["token"] = json!(token);
    }
    for k in ["x", "y", "x2", "y2", "text", "key"] {
        if !req[k].is_null() {
            fwd[k] = req[k].clone();
        }
    }
    match control(pid, &fwd).await {
        Ok(v) => {
            if op == "takeover" {
                if let Some(t) = v["token"].as_str() {
                    takeovers().insert(key, t.to_owned());
                }
            } else if op == "release" {
                takeovers().remove(&key);
            }
            axum::Json(json!({ "ok": true })).into_response()
        }
        Err(e) => {
            // Ended meanwhile (time limit, or the session closed): forget the token.
            if op != "takeover" && (e.contains("take over this session first") || e.contains("is gone") || e.contains("did not take over")) {
                takeovers().remove(&key);
            }
            refuse(StatusCode::CONFLICT, &e)
        }
    }
}

// ---- static files -------------------------------------------------------------------------

const ICONS: &[(&str, &str)] = &[
    ("alert", include_str!("../ui/icons/alert.svg")),
    ("chev", include_str!("../ui/icons/chev.svg")),
    ("clock", include_str!("../ui/icons/clock.svg")),
    ("chat", include_str!("../ui/icons/chat.svg")),
    ("close", include_str!("../ui/icons/close.svg")),
    ("cursor", include_str!("../ui/icons/cursor.svg")),
    ("device", include_str!("../ui/icons/device.svg")),
    ("expand", include_str!("../ui/icons/expand.svg")),
    ("gauge", include_str!("../ui/icons/gauge.svg")),
    ("globe", include_str!("../ui/icons/globe.svg")),
    ("history", include_str!("../ui/icons/history.svg")),
    ("info", include_str!("../ui/icons/info.svg")),
    ("key", include_str!("../ui/icons/key.svg")),
    ("live", include_str!("../ui/icons/live.svg")),
    ("lock", include_str!("../ui/icons/lock.svg")),
    ("mark", include_str!("../ui/icons/mark.svg")),
    ("memory", include_str!("../ui/icons/memory.svg")),
    ("model", include_str!("../ui/icons/model.svg")),
    ("play", include_str!("../ui/icons/play.svg")),
    ("plug", include_str!("../ui/icons/plug.svg")),
    ("map", include_str!("../ui/icons/map.svg")),
    ("plus", include_str!("../ui/icons/plus.svg")),
    ("refresh", include_str!("../ui/icons/refresh.svg")),
    ("risk", include_str!("../ui/icons/risk.svg")),
    ("send", include_str!("../ui/icons/send.svg")),
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
        "history.js" => ("text/javascript; charset=utf-8", include_bytes!("../ui/history.js")),
        "memory.js" => ("text/javascript; charset=utf-8", include_bytes!("../ui/memory.js")),
        "fix.js" => ("text/javascript; charset=utf-8", include_bytes!("../ui/fix.js")),
        "apps.js" => ("text/javascript; charset=utf-8", include_bytes!("../ui/apps.js")),
        "map.js" => ("text/javascript; charset=utf-8", include_bytes!("../ui/map.js")),
        "agents.js" => ("text/javascript; charset=utf-8", include_bytes!("../ui/agents.js")),
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
    // A file that does not parse: say so, and show nothing as if it were the settings in force.
    if let Err(e) = config::check() {
        return axum::Json(json!({ "path": config::path().display().to_string(), "error": e, "settings": [] })).into_response();
    }
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
    let defaults = json!({
        "model_replay": crate::run::DEFAULT_REPLAY,
        "model_explore": crate::run::DEFAULT_EXPLORE,
        "max_steps": 30,
        "state_expire_days": 30,
        // v0.3.2: the lists in force while the key is not set (`default` in block_actions too).
        "block_actions": fastpilot_core::policy::DEFAULT_BLOCKED,
        "confirm_actions": fastpilot_core::policy::DEFAULT_CONFIRM,
    });
    axum::Json(json!({ "path": config::path().display().to_string(), "settings": list, "defaults": defaults })).into_response()
}

/// Why an item of a list setting is not valid there (the page's own checks; `config set` and
/// the file are checked by fastpilot itself).
fn bad_item(key: &str, item: &str) -> Option<String> {
    match key {
        "allowed_domains" => {
            let host = item.strip_prefix("*.").unwrap_or(item);
            let ok = !host.is_empty()
                && host.split('.').all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
                && !item.contains("://");
            (!ok).then(|| format!("\"{item}\" is not a domain name: write the host only, for example shop.com or *.shop.com (no https:// and no path)"))
        }
        "vars" => {
            let ok = item.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && item.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !ok {
                Some(format!("\"{item}\" is not a variable name: letters, digits and _ only, for example DEMO_PASS"))
            } else if item.to_ascii_uppercase().starts_with("FASTPILOT_") {
                Some(format!("\"{item}\" is one of fastpilot's own settings, not a secret a flow may read"))
            } else {
                None
            }
        }
        "upload_dirs" => (!(item.starts_with('/') || item.starts_with("~/"))).then(|| format!("\"{item}\": write the full folder, for example ~/fastpilot/fixtures or /Users/name/fixtures")),
        "confirm_actions" | "block_actions" => (item.chars().count() > 60).then(|| "the word is too long (60 characters at most)".to_owned()),
        _ => None,
    }
}

async fn change(axum::Json(body): axum::Json<Value>) -> Response {
    if let Err(e) = config::check() {
        return refuse(StatusCode::CONFLICT, &format!("the settings file is damaged; fix it before saving: {e}"));
    }
    let Some(key) = body["key"].as_str().filter(|k| KEYS.contains(k)) else {
        return refuse(StatusCode::BAD_REQUEST, "this setting cannot be changed here");
    };
    if std::env::var(format!("FASTPILOT_{}", key.to_uppercase())).is_ok() {
        return refuse(
            StatusCode::CONFLICT,
            &format!("`{key}` is set in the environment (FASTPILOT_{}), which wins over the settings file: change it there", key.to_uppercase()),
        );
    }
    let value = match &body["value"] {
        Value::Null => None,
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if s.trim().is_empty() => None,
        Value::String(s) => Some(s.trim().to_owned()),
        // Emptied on purpose ("Kosongkan"): the built-in lists are off. `null` ("Kembalikan ke
        // bawaan") removes the key, which brings them back.
        Value::Array(items) if items.is_empty() && matches!(key, "block_actions" | "confirm_actions") => Some(String::new()),
        Value::Array(items) if items.is_empty() => None,
        Value::Array(items) => {
            let items: Vec<&str> = items.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).collect();
            if items.iter().any(|s| s.contains(',')) {
                return refuse(StatusCode::BAD_REQUEST, "an item cannot contain a comma");
            }
            if let Some(why) = items.iter().find_map(|i| bad_item(key, i)) {
                return refuse(StatusCode::BAD_REQUEST, &why);
            }
            (!items.is_empty()).then(|| items.join(","))
        }
        Value::Object(_) => return refuse(StatusCode::BAD_REQUEST, "unknown value"),
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
                // The page shows no file paths or device ids (FastTest M3); the terminal does.
                "detail": c.screen.clone().unwrap_or_else(|| c.detail.clone()),
                "hint": c.screen_hint.clone().or_else(|| c.hint.clone()),
                "setup": (!c.fixes.is_empty()).then(|| format!("fastpilot setup {}", c.area)),
            })
        })
        .collect();
    axum::Json(json!({ "checks": checks })).into_response()
}

// ---- live (C7a) ---------------------------------------------------------------------------

/// Running sessions of every fastpilot process, with the steps of their flows. The DevTools
/// address of the browser is not part of it: frames come from `/api/live/frames`.
async fn live_api() -> Response {
    *PAGE_SEEN.lock().unwrap_or_else(|p| p.into_inner()) = Some(std::time::Instant::now());
    let mine: Vec<(u32, String)> = takeovers().keys().cloned().collect();
    let sessions: Vec<Value> = fastpilot_core::live::read_all()
        .into_iter()
        .flat_map(|p| {
            let flows = p.flows.clone();
            let mine = mine.clone();
            p.sessions.into_iter().map(move |s| {
                let flow = s.flow.and_then(|id| flows.iter().find(|f| f.id == id)).map(|f| {
                    json!({ "started_ms": f.started_ms, "finished": f.finished, "steps": f.steps })
                });
                let controllable = can_control(p.pid);
                let by_me = mine.contains(&(p.pid, s.id.clone())) && s.taken_over_ms.is_some();
                let device = matches!(s.platform.as_str(), "android" | "ios");
                json!({
                    "pid": p.pid, "kind": p.kind, "agent": p.agent, "scheduled": p.scheduled, "id": s.id, "platform": s.platform, "started_ms": s.started_ms,
                    "watchable": s.screen.is_some() || (device && controllable),
                    "controllable": controllable && (s.screen.is_some() || device),
                    "taken_over_ms": s.taken_over_ms,
                    "taken_over_here": by_me,
                    "takeover_allowed": p.takeover,
                    "flow": flow,
                })
            })
        })
        .collect();
    // D10d: which agent each session works for, the queue, the limit and today's usage per agent.
    let agents = crate::agents::list();
    let named = |id: Option<&str>| {
        let a = agents.iter().find(|a| Some(a.id.as_str()) == id.or(Some(crate::agents::DEFAULT)))?;
        let (label, tested) = crate::agents::model_label(a);
        Some((a.name.clone(), label, tested))
    };
    let sessions: Vec<Value> = sessions
        .into_iter()
        .map(|mut s| {
            // V042: a session no agent started is not "My agent": say where it came from.
            if s["agent"].is_null() {
                s["agent_name"] = json!(match s["scheduled"].as_str() {
                    Some(name) => format!("Scheduled · {name}"),
                    None => "Started from the terminal".to_owned(),
                });
                return s;
            }
            if let Some((name, model, tested)) = named(s["agent"].as_str()) {
                s["agent_name"] = json!(name);
                s["model_label"] = json!(model);
                s["model_tested"] = json!(tested);
            }
            s
        })
        .collect();
    let claims = fastpilot_core::claim::entries();
    let queue: Vec<Value> = claims
        .iter()
        .filter(|e| e.waiting)
        .map(|e| json!({ "agent": named(e.agent.as_deref()).map(|n| n.0), "session": e.session, "platform": e.platform, "device": e.device.as_deref().and_then(|d| d.split_once(':')).map(|(_, d)| d), "since_ms": e.since_ms, "reason": e.reason }))
        .collect();
    let usage: Vec<Value> = crate::agents::usage_today()
        .into_iter()
        .map(|(id, u)| json!({ "agent": named(Some(&id)).map(|n| n.0).unwrap_or(id), "turns": u.turns, "tokens": u.input + u.output }))
        .collect();
    let running = claims.iter().filter(|e| !e.waiting).count();
    axum::Json(json!({ "sessions": sessions, "queue": queue, "running": running, "limit": fastpilot_core::claim::max_parallel(), "usage": usage })).into_response()
}

/// `{limit}`: how many tests may run at once (D10d, `max_parallel`), set on the Live page.
async fn live_limit(axum::Json(body): axum::Json<Value>) -> Response {
    let n = body["limit"].as_u64().unwrap_or(0);
    match fastpilot_core::config::write("max_parallel", Some(&n.to_string())) {
        Ok(_) => axum::Json(json!({ "limit": n })).into_response(),
        Err(e) => refuse(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}

const BOUNDARY: &str = "fpframe";

/// `?pid=&session=&fps=&w=`: the session's screen as `multipart/x-mixed-replace` JPEG frames,
/// at most `fps` (1–15) per second and `w` (160–1920) pixels wide. Read-only: a separate DevTools
/// connection that only screencasts; it ends when the page stops showing the stream.
async fn frames(Query(q): Query<std::collections::HashMap<String, String>>) -> Response {
    let (Some(pid), Some(session)) = (q.get("pid").and_then(|p| p.parse::<u32>().ok()), q.get("session")) else {
        return refuse(StatusCode::BAD_REQUEST, "pid and session are required");
    };
    let fps = q.get("fps").and_then(|f| f.parse::<u64>().ok()).unwrap_or(2).clamp(1, 15);
    let width = q.get("w").and_then(|w| w.parse::<u32>().ok()).unwrap_or(480).clamp(160, 1920);
    let found = fastpilot_core::live::read_all().into_iter().filter(|p| p.pid == pid).flat_map(|p| p.sessions).find(|s| &s.id == session);
    let Some(found) = found else {
        return refuse(StatusCode::NOT_FOUND, "there is no session with that name");
    };
    let Some(screen) = found.screen else {
        if matches!(found.platform.as_str(), "android" | "ios") && can_control(pid) {
            return device_frames(pid, session.clone(), fps.min(3));
        }
        return refuse(StatusCode::NOT_FOUND, "this session cannot be watched (your own Chrome that fastpilot attached to, or an older fastpilot process)");
    };
    let cast = match fastpilot_web::Screencast::start(&screen.ws, &screen.target, width, if width > 800 { 70 } else { 55 }).await {
        Ok(c) => c,
        Err(e) => return refuse(StatusCode::BAD_GATEWAY, &format!("the screen cannot be shown: {e}")),
    };
    let rx = cast.frames();
    let period = std::time::Duration::from_millis(1000 / fps);
    let frames = futures_util::stream::unfold((cast, rx, None::<tokio::time::Instant>), move |(cast, mut rx, last)| async move {
        // A new frame (waiting as long as the page is static), no more often than `fps`.
        rx.changed().await.ok()?;
        if let Some(last) = last {
            tokio::time::sleep_until(last + period).await;
        }
        let frame = rx.borrow_and_update().clone()?;
        // The boundary right after the frame: a browser shows a part once the next one starts,
        // and a static page sends no next frame.
        let mut part = format!("Content-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", frame.len()).into_bytes();
        part.extend_from_slice(&frame);
        part.extend_from_slice(format!("\r\n--{BOUNDARY}\r\n").as_bytes());
        Some((Ok::<_, std::io::Error>(part), (cast, rx, Some(tokio::time::Instant::now()))))
    });
    let first = futures_util::stream::once(async { Ok::<_, std::io::Error>(format!("--{BOUNDARY}\r\n").into_bytes()) });
    let stream = futures_util::StreamExt::chain(first, frames);
    let mut r = Response::new(Body::from_stream(stream));
    if let Ok(v) = HeaderValue::from_str(&format!("multipart/x-mixed-replace; boundary={BOUNDARY}")) {
        r.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    r
}
