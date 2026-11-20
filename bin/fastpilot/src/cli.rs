//! Shell CLI: `fastpilot open <url>`, `fastpilot snapshot -i`, `fastpilot click @e2`, ...
//!
//! Every command is one short process. Sessions live in a background daemon (`fastpilot daemon`,
//! started on first use, one per user, on a Unix socket) that serves the same tools as the MCP
//! server, so an agent that can only run shell commands drives web, Android and iOS the same way.
//! Commands map onto the existing tools and DSL steps; nothing here talks to a browser directly.

use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fastpilot_mcp::FastPilot;
use fastpilot_web::LaunchOptions;
use serde_json::{Value, json};

type Res<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Subcommands handled here (the rest of `main` keeps `mcp`, `run`, `bench`, ...).
pub const COMMANDS: &[&str] = &[
    "open", "attach", "close", "sessions", "devices", "snapshot", "click", "tap", "dblclick", "longpress", "fill", "type", "clear",
    "press", "select", "check", "uncheck", "upload", "scroll", "scrollto", "swipe", "back", "forward", "reload", "wait", "assert",
    "is", "get", "eval", "screenshot", "record", "set", "launch", "stop-app", "clear-state", "hide-keyboard", "flow", "check-flow",
    "errors", "docs", "daemon", "help", "state", "diff", "ship-check",
];

const HELP: &str = "fastpilot shell commands (sessions stay open in a background daemon between commands)

  first time? fastpilot demo (a demo folder to try) · fastpilot init (set up this folder)

  open <url|path|app> [--platform web|android|ios] [--device ID]   start a session or navigate it
  attach [endpoint]            drive your running Chrome (http://127.0.0.1:9222, extension, or autoConnect)
  close [--all]                close the session (--all: every session and the daemon)
  sessions | devices           list sessions / attached devices and simulators
  snapshot [-i] [--json] [--full] [--coords] [--filter TEXT]   accessibility tree with refs (@e1 ...)
  click|tap|dblclick|longpress <sel> [--confirm]   --confirm: a risky action (confirm_actions)
  fill <sel> <text>            replace the content of a field
  type <text>                  type into the focused field
  clear [N]                    erase N characters (all without N)
  press <key>                  Enter, Tab, Escape, ArrowDown, ...
  select <sel> <option>        pick an option of a <select>
  check|uncheck <sel>          set a checkbox / switch
  upload <sel> <file>...       attach files to a file input
  scroll <up|down|left|right> [sel] | scrollto <sel> | swipe <x1> <y1> <x2> <y2>
  back | forward | reload
  wait <ms|sel> [--gone] [--timeout MS] | wait --url <part>
  assert <visible|hidden|text|contains|enabled> <sel> [text]
  is <visible|enabled> <sel>   prints true or false
  get <text|url|title> [sel]
  eval <js>                    run JavaScript (web, WebViews)
  screenshot [path] [--annotate]
  record <start [name]|stop>
  set viewport <preset|WxH> | set locale <locale> [timezone] | set geo <lat> <lon>
  set network <offline|online> | set permission <name> <allow|deny|unset>
  launch <app> | stop-app [app] | clear-state [app] | hide-keyboard
  flow <file.yaml|-> [--keep-going] | check-flow <file.yaml|->   run / validate a DSL flow
  state <save|load|delete> <name> | state list   saved login state (web: cookies + localStorage;
                               encrypted with FASTPILOT_STATE_KEY, expires after 30 days)
  diff screenshot <name> [--update] [--threshold 0.1] [--max-diff 0.002] [--ignore sel]...
                               compare with the stored baseline (stored on first use)
  errors                       recent console errors, failed requests, crashes
  ship-check [url]... [--links N] [--cookies]   one verdict for the pages: console errors, failed
                               requests, broken links (HEAD, no cookies unless --cookies; servers treat
                               HEAD like GET), layout at 3 sizes, accessibility basics (exit 0: SHIP or
                               SHIP WITH FIXES, 1: DO NOT SHIP, 2: INCONCLUSIVE)
  docs [dsl|selectors|errors|...]
  daemon <status|stop>

selectors: @e7 (ref from snapshot), \"Sign in\" (text), #email (id), role=button:Sign in, 120,340 (point)
options: --session NAME (default: $FASTPILOT_SESSION or default), --json (raw tool output),
         --headed (show the browser; applies when the daemon starts)
env: FASTPILOT_VARS / ${NAME} values are read by the daemon when it starts; restart it after changing them.
other commands: mcp, run, bench, stability, ios doctor (`fastpilot` without arguments lists them)";

/// The page's URL from a snapshot (no evalScript: it works under the block lists).
fn page_url(ctx: &Ctx, sid: &str) -> Res<Option<String>> {
    let v = ctx.tool("snapshot", json!({ "session": sid, "format": "json" }))?;
    if v["ok"] == false {
        return Ok(None);
    }
    let snap: Value = serde_json::from_str(&text_of(&v)).map_err(|e| format!("snapshot is not JSON: {e}"))?;
    Ok(snap["location"].as_str().map(str::to_owned))
}

/// The shell command in `args`, skipping global options written before it
/// (`fastpilot --session two open ...`).
pub fn command_of(args: &[String]) -> Option<&str> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--session" => {
                it.next();
            }
            "--json" | "--headed" | "--headful" => {}
            c => return COMMANDS.contains(&c).then_some(c),
        }
    }
    None
}

/// `~/.fastpilot/cli.sock`, or `FASTPILOT_SOCKET`.
fn socket_path() -> PathBuf {
    if let Some(p) = fastpilot_core::config::var_os("FASTPILOT_SOCKET") {
        return p.into();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join(".fastpilot").join("cli.sock")
}

// ---------------------------------------------------------------------------------------------
// Daemon

/// `fastpilot daemon`: serve the tools on the socket until `daemon stop` or idle for
/// `FASTPILOT_DAEMON_IDLE` seconds (default 1800); sessions are closed on the way out.
#[cfg(unix)]
pub async fn daemon(web: LaunchOptions) -> Res<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let path = socket_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    if tokio::net::UnixStream::connect(&path).await.is_ok() {
        return Err(format!("a fastpilot daemon already serves {}", path.display()).into());
    }
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path)?;
    let fp = FastPilot::new(web);
    let idle = Duration::from_secs(fastpilot_core::config::var("FASTPILOT_DAEMON_IDLE").ok().and_then(|v| v.parse().ok()).unwrap_or(1800));
    let last = std::sync::Arc::new(std::sync::Mutex::new(Instant::now()));
    // Requests being handled: a long one (launching Chrome, a flow) is never cut by the idle exit.
    let busy = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    eprintln!("fastpilot daemon on {} (idle exit after {}s)", path.display(), idle.as_secs());
    loop {
        let since = last.lock().map(|l| l.elapsed()).unwrap_or_default();
        let wait = idle.saturating_sub(since).max(Duration::from_secs(1));
        tokio::select! {
            _ = stop_rx.changed() => break,
            () = tokio::time::sleep(wait) => {
                let quiet = busy.load(std::sync::atomic::Ordering::SeqCst) == 0;
                if quiet && last.lock().map(|l| l.elapsed() >= idle).unwrap_or(true) {
                    break;
                }
            }
            conn = listener.accept() => {
                let Ok((sock, _)) = conn else { continue };
                let (fp, last, stop_tx, busy) = (fp.clone(), last.clone(), stop_tx.clone(), busy.clone());
                busy.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::spawn(async move {
                    struct Done(std::sync::Arc<std::sync::atomic::AtomicUsize>);
                    impl Drop for Done {
                        fn drop(&mut self) {
                            self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    let _done = Done(busy);
                    let (r, mut w) = sock.into_split();
                    let mut lines = tokio::io::BufReader::new(r).lines();
                    let Ok(Some(line)) = lines.next_line().await else { return };
                    if let Ok(mut l) = last.lock() { *l = Instant::now(); }
                    let req: Value = serde_json::from_str(&line).unwrap_or_default();
                    let reply = handle(&fp, &req).await;
                    if let Ok(mut l) = last.lock() { *l = Instant::now(); }
                    let _ = w.write_all(format!("{reply}\n").as_bytes()).await;
                    let _ = w.shutdown().await;
                    if req["op"] == "stop" {
                        let _ = stop_tx.send(true);
                    }
                });
            }
        }
    }
    fp.close_all().await;
    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[cfg(not(unix))]
pub async fn daemon(_web: LaunchOptions) -> Res<()> {
    Err("the fastpilot shell CLI needs a Unix socket (macOS or Linux); use `fastpilot mcp` instead".into())
}

async fn handle(fp: &FastPilot, req: &Value) -> Value {
    match req["op"].as_str().unwrap_or("") {
        "ping" => json!({ "ok": true, "pid": std::process::id() }),
        "stop" => json!({ "ok": true }),
        "sessions" => {
            let (ids, active) = fp.session_ids().await;
            json!({ "ok": true, "sessions": ids, "active": active })
        }
        "errors" => match fp.diagnostics(req["session"].as_str()).await {
            Ok(lines) => json!({ "ok": true, "lines": lines }),
            Err(e) => json!({ "ok": false, "content": [{ "type": "text", "text": e }] }),
        },
        "tool" => {
            let (ok, content) = fp.call_json(req["tool"].as_str().unwrap_or(""), req["args"].clone()).await;
            json!({ "ok": ok, "content": content })
        }
        other => json!({ "ok": false, "content": [{ "type": "text", "text": format!("unknown op `{other}`") }] }),
    }
}

// ---------------------------------------------------------------------------------------------
// Client

#[cfg(unix)]
fn send_once(req: &Value) -> std::io::Result<Value> {
    let mut sock = std::os::unix::net::UnixStream::connect(socket_path())?;
    sock.write_all(format!("{req}\n").as_bytes())?;
    let mut line = String::new();
    BufReader::new(sock).read_line(&mut line)?;
    serde_json::from_str(&line).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("no reply from the daemon: {e}")))
}

#[cfg(not(unix))]
fn send_once(_req: &Value) -> std::io::Result<Value> {
    Err(std::io::Error::other("the fastpilot shell CLI needs macOS or Linux"))
}

/// Send a request, starting the daemon first when none answers.
fn send(req: &Value, headed: bool) -> Res<Value> {
    match send_once(req) {
        Ok(v) => return Ok(v),
        // Connected, but no reply: a daemon that was exiting (idle) just then. Requests only
        // reach a tool once they were read, so this one never ran: send it again.
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => std::thread::sleep(Duration::from_millis(300)),
        Err(_) => {}
    }
    if send_once(&json!({ "op": "ping" })).is_err() {
        start_daemon(headed)?;
    }
    Ok(send_once(req)?)
}

fn start_daemon(headed: bool) -> Res<()> {
    let path = socket_path();
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir)?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("daemon.log"))?;
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg("daemon").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(log);
    if headed {
        cmd.env("FASTPILOT_HEADFUL", "1");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: closing the terminal does not take the sessions down.
        cmd.process_group(0);
    }
    cmd.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if send_once(&json!({ "op": "ping" })).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!("the fastpilot daemon did not start; see {}", dir.join("daemon.log").display()).into())
}

/// Command-line selector → DSL selector YAML.
pub fn selector(s: &str) -> String {
    let q = |t: &str| serde_json::to_string(t).unwrap_or_default();
    if let Some(r) = s.strip_prefix('@') {
        return format!("{{ ref: {} }}", q(r));
    }
    if let Some(id) = s.strip_prefix('#').filter(|i| !i.is_empty()) {
        return format!("{{ id: {} }}", q(id));
    }
    if let Some(t) = s.strip_prefix("text=") {
        return q(t);
    }
    if let Some(rest) = s.strip_prefix("role=") {
        return match rest.split_once(':') {
            Some((role, name)) => format!("{{ role: {}, name: {} }}", q(role), q(name)),
            None => format!("{{ role: {} }}", q(rest)),
        };
    }
    if let Some((x, y)) = s.split_once(',')
        && let (Ok(x), Ok(y)) = (x.trim().parse::<f64>(), y.trim().parse::<f64>())
    {
        return format!("{{ point: [{x}, {y}] }}");
    }
    q(s)
}

/// `--confirm`: the selector with `confirm: true` (FASTPILOT_CONFIRM_ACTIONS).
fn confirmed(sel: &str) -> String {
    match sel.strip_suffix(" }") {
        Some(map) => format!("{map}, confirm: true }}"),
        None => format!("{{ text: {sel}, confirm: true }}"),
    }
}

fn q(t: &str) -> String {
    serde_json::to_string(t).unwrap_or_default()
}

/// A local file path made absolute (the daemon's working directory may differ).
fn abs_if_file(s: &str) -> String {
    let p = Path::new(s);
    if !s.contains("://") && p.exists() {
        return std::fs::canonicalize(p).map(|p| p.display().to_string()).unwrap_or_else(|_| s.to_owned());
    }
    s.to_owned()
}

struct Ctx {
    session: String,
    json: bool,
    headed: bool,
}

impl Ctx {
    fn tool(&self, tool: &str, args: Value) -> Res<Value> {
        send(&json!({ "op": "tool", "tool": tool, "args": args }), self.headed)
    }

    fn flow(&self, yaml: &str, keep_going: bool) -> Res<Value> {
        self.tool("run_flow", json!({ "flow": yaml, "session": self.session, "stop_on_fail": !keep_going }))
    }

    fn has_session(&self) -> Res<bool> {
        let v = send(&json!({ "op": "sessions" }), self.headed)?;
        Ok(v["sessions"].as_array().is_some_and(|s| s.iter().any(|x| x == self.session.as_str())))
    }
}

fn text_of(v: &Value) -> String {
    v["content"].as_array().map(|c| c.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
}

/// The value a step returned (`  ✓ 1 [s] evalScript "..." 3ms = "x"` → `x`).
fn step_value(out: &str) -> Option<String> {
    let line = out.lines().find(|l| l.trim_start().starts_with('✓') && l.contains(" = "))?;
    let raw = line.split_once(" = ")?.1.trim();
    Some(match serde_json::from_str::<Value>(raw) {
        Ok(Value::String(s)) => s,
        _ => raw.to_owned(),
    })
}

/// Print a reply and turn a failure into the exit status.
fn finish(ctx: &Ctx, v: &Value) -> Res<()> {
    if ctx.json {
        println!("{v}");
    } else {
        let t = text_of(v);
        if !t.is_empty() {
            println!("{t}");
        }
    }
    if v["ok"] == false {
        std::process::exit(1);
    }
    Ok(())
}

fn need<'a>(args: &'a [String], i: usize, what: &str) -> Res<&'a str> {
    args.get(i).map(String::as_str).ok_or_else(|| format!("missing {what}; see `fastpilot help`").into())
}

/// Read `flow` input: a file, `-` for stdin, or inline YAML.
fn flow_source(arg: &str) -> Res<String> {
    if arg == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        return Ok(s);
    }
    if Path::new(arg).is_file() {
        return Ok(std::fs::read_to_string(arg)?);
    }
    Ok(arg.to_owned())
}

/// R2: `fastpilot ship-check [url]...`: the Ship check of each page (the open one without urls),
/// then one verdict for all of them, in the S8 terminal layout. The verdict covers these pages;
/// the regression and the main journeys are named as not checked here.
fn ship_check(ctx: &Ctx, urls: &[String], links: Option<&str>, cookies: bool) -> Res<()> {
    let links = match links {
        Some(n) => n.parse::<usize>().map_err(|_| "--links needs a number (0: do not ask links)")?,
        None => 20,
    };
    let step = if links == 20 && !cookies { "- shipCheck\n".to_owned() } else { format!("- shipCheck: {{ links: {links}, cookies: {cookies} }}\n") };
    let urls: Vec<String> = urls.iter().map(|u| abs_if_file(u)).collect();
    if urls.is_empty() && !ctx.has_session()? {
        return Err("give the pages to check: fastpilot ship-check https://staging.example.com [more pages], or `fastpilot open <url>` first".into());
    }
    let mut reports = Vec::new();
    let pages: Vec<Option<&String>> = if urls.is_empty() { vec![None] } else { urls.iter().map(Some).collect() };
    for url in pages {
        if let Some(u) = url
            && !ctx.has_session()?
        {
            let v = ctx.tool("session", json!({ "action": "start", "id": ctx.session, "platform": "web", "target": u }))?;
            if v["ok"] == false {
                return finish(ctx, &v);
            }
        }
        let flow = match url {
            Some(u) => format!("- openUrl: {}\n- waitForAnimationToEnd\n{step}", q(u)),
            None => step.clone(),
        };
        let v = ctx.flow(&flow, false)?;
        if ctx.json {
            println!("{v}");
        }
        let text = text_of(&v);
        let Some(report) = ship_report(&text) else {
            if !ctx.json {
                println!("{text}");
            }
            reports.push((url.cloned().unwrap_or_default(), "INCONCLUSIVE".to_owned()));
            continue;
        };
        if !ctx.json {
            println!("{report}\n");
        }
        let verdict = report.lines().find_map(|l| l.strip_prefix("Verdict: ")).unwrap_or("INCONCLUSIVE").to_owned();
        reports.push((url.cloned().unwrap_or_default(), verdict));
    }
    const ORDER: [&str; 4] = ["DO NOT SHIP", "INCONCLUSIVE", "SHIP WITH FIXES", "SHIP"];
    let worst = ORDER.iter().find(|w| reports.iter().any(|(_, v)| v == *w)).copied().unwrap_or("INCONCLUSIVE");
    if !ctx.json {
        if reports.len() > 1 {
            println!("Verdict for the {} pages: {worst}", reports.len());
        }
        println!("Not checked here: the regression and the main journeys (run your tests, or ask the Chat bot for a release check).");
        println!("This is a recommendation. You decide.");
    }
    std::process::exit(match worst {
        "SHIP" | "SHIP WITH FIXES" => 0,
        "DO NOT SHIP" => 1,
        _ => 2,
    });
}

/// The Ship check report inside a `run_flow` answer: from "Ship check ·" to fastpilot's own
/// closing "Not checked: colour contrast, …" line (page text in it is one line, never at the start).
fn ship_report(text: &str) -> Option<String> {
    let at = text.find("Ship check · ")?;
    let lines: Vec<&str> = text[at..].lines().collect();
    let end = lines.iter().rposition(|l| l.starts_with("Not checked: colour contrast"))?;
    Some(lines[..=end].join("\n"))
}

/// One shell command. `argv` starts at the command name.
pub fn main(argv: &[String]) -> Res<()> {
    // Global options, anywhere on the line.
    let mut session = fastpilot_core::config::var("FASTPILOT_SESSION").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "default".into());
    let (mut json, mut headed) = (false, fastpilot_core::config::flag("headful"));
    let mut opts: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    let mut flags: Vec<&str> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    let mut ignores: Vec<String> = Vec::new();
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--session" => session = it.next().cloned().ok_or("--session needs a name")?,
            "--json" => json = true,
            "--headed" | "--headful" => headed = true,
            "--ignore" => ignores.push(it.next().cloned().ok_or("--ignore needs a selector")?),
            "--platform" | "--device" | "--timeout" | "--filter" | "--url" | "--threshold" | "--max-diff" | "--links" => {
                let key = a.as_str();
                opts.insert(key, it.next().cloned().ok_or_else(|| format!("{key} needs a value"))?);
            }
            "-i" | "--interactive" | "--full" | "--coords" | "--all" | "--gone" | "--annotate" | "--keep-going" | "-c" | "--compact" | "--confirm" | "--update" | "--cookies" => {
                flags.push(a.as_str());
            }
            // Help, not a step: `open --help` used to open https://--help.
            "--help" | "-h" => {
                println!("{HELP}");
                return Ok(());
            }
            _ => args.push(a.clone()),
        }
    }
    let flag = |f: &str| flags.contains(&f);
    let ctx = Ctx { session: session.clone(), json, headed };
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let arg = |i: usize, what: &str| need(&args, i, what);
    let rest = |i: usize| args.get(i..).map(|r| r.join(" ")).unwrap_or_default();
    let sid = session.as_str();

    let step = |yaml: String| -> Res<()> { finish(&ctx, &ctx.flow(&yaml, false)?) };
    match cmd {
        "help" => println!("{HELP}"),
        "open" => {
            let given = arg(1, "a URL, path or app")?;
            if given.starts_with("--") {
                return Err(format!("unknown option `{given}` for open (see `fastpilot help`)").into());
            }
            let target = abs_if_file(given);
            if ctx.has_session()? {
                let looks_url = target.contains("://") || target.contains('/') || target.ends_with(".html");
                step(if looks_url { format!("- openUrl: {}\n", q(&target)) } else { format!("- launchApp: {}\n", q(&target)) })?;
            } else {
                let mut a = json!({ "action": "start", "id": sid, "target": target, "platform": opts.get("--platform").cloned().unwrap_or_else(|| "web".into()) });
                if let Some(d) = opts.get("--device") {
                    a["device"] = json!(d);
                }
                finish(&ctx, &ctx.tool("session", a)?)?;
            }
        }
        "attach" => {
            let mut a = json!({ "action": "start", "id": sid, "platform": "web", "mode": "attach" });
            if let Some(e) = args.get(1) {
                a["endpoint"] = json!(e);
            }
            finish(&ctx, &ctx.tool("session", a)?)?;
        }
        "close" if flag("--all") => {
            send_once(&json!({ "op": "stop" })).ok();
            println!("closed every session; daemon stopped");
        }
        "close" => finish(&ctx, &ctx.tool("session", json!({ "action": "stop", "id": sid }))?)?,
        "sessions" => {
            let v = send(&json!({ "op": "sessions" }), headed)?;
            if json {
                println!("{v}");
            } else {
                let active = v["active"].as_str();
                for s in v["sessions"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    println!("{s}{}", if Some(s) == active { " (active)" } else { "" });
                }
            }
        }
        "devices" => finish(&ctx, &ctx.tool("session", json!({ "action": "devices" }))?)?,
        "snapshot" => {
            let mut a = json!({ "session": sid });
            if flag("-i") || flag("--interactive") {
                a["interactive"] = json!(true);
            }
            if flag("--full") {
                a["full"] = json!(true);
            }
            if flag("--coords") {
                a["coords"] = json!(true);
            }
            if let Some(f) = opts.get("--filter").or(args.get(1)) {
                a["filter"] = json!(f);
            }
            if json {
                a["format"] = json!("json");
                let v = ctx.tool("snapshot", a)?;
                println!("{}", text_of(&v));
                if v["ok"] == false {
                    std::process::exit(1);
                }
            } else {
                finish(&ctx, &ctx.tool("snapshot", a)?)?;
            }
        }
        "click" | "tap" | "dblclick" | "longpress" => {
            let verb = match cmd {
                "dblclick" => "doubleTapOn",
                "longpress" => "longPressOn",
                _ => "tapOn",
            };
            let mut sel = selector(arg(1, "a selector")?);
            if flag("--confirm") {
                sel = confirmed(&sel);
            }
            step(format!("- {verb}: {sel}\n"))?;
        }
        "fill" => step(format!("- inputText: {{ into: {}, text: {} }}\n", selector(arg(1, "a selector")?), q(&rest(2))))?,
        "type" => step(format!("- inputText: {}\n", q(&rest(1))))?,
        "clear" => step(match args.get(1) {
            Some(n) => format!("- eraseText: {}\n", n.parse::<usize>().map_err(|_| "clear takes a number of characters")?),
            None => "- eraseText\n".into(),
        })?,
        "press" => step(format!("- pressKey: {}\n", q(arg(1, "a key")?)))?,
        "select" => step(format!("- selectOption: {{ from: {}, option: {} }}\n", selector(arg(1, "a selector")?), q(&rest(2))))?,
        "check" | "uncheck" => {
            let want = cmd == "check";
            let sel = arg(1, "a selector")?;
            // Read the current state first, so check/uncheck are idempotent.
            let snap = ctx.tool("snapshot", json!({ "session": sid, "format": "json", "full": true }))?;
            let entries: Value = serde_json::from_str(&text_of(&snap)).unwrap_or_default();
            let found = entries["entries"].as_array().into_iter().flatten().find(|e| match sel.strip_prefix('@') {
                Some(r) => e["ref"] == r,
                None => e["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(sel)) && e["role"] != "text",
            });
            match found {
                Some(e) if (e["checked"] == true) == want => println!("already {}", if want { "checked" } else { "unchecked" }),
                _ => step(format!("- tapOn: {}\n", selector(sel)))?,
            }
        }
        "upload" => {
            let files: Vec<String> = args.get(2..).unwrap_or_default().iter().map(|f| q(&abs_if_file(f))).collect();
            if files.is_empty() {
                return Err("upload needs at least one file".into());
            }
            step(format!("- uploadFile: {{ into: {}, files: [{}] }}\n", selector(arg(1, "a selector")?), files.join(", ")))?;
        }
        "scroll" => {
            let dir = arg(1, "a direction (up, down, left, right)")?;
            step(match args.get(2) {
                Some(s) => format!("- scroll: {{ direction: {dir}, within: {} }}\n", selector(s)),
                None => format!("- scroll: {dir}\n"),
            })?;
        }
        "scrollto" => step(format!("- scrollUntilVisible: {}\n", selector(arg(1, "a selector")?)))?,
        "swipe" => {
            let n: Vec<f64> = (1..=4).map(|i| arg(i, "x1 y1 x2 y2").and_then(|v| v.parse::<f64>().map_err(|_| "swipe takes four numbers".into()))).collect::<Res<_>>()?;
            step(format!("- swipe: {{ from: [{}, {}], to: [{}, {}] }}\n", n[0], n[1], n[2], n[3]))?;
        }
        "back" => step("- back\n".into())?,
        "forward" => step("- evalScript: \"history.forward()\"\n- waitForAnimationToEnd\n".into())?,
        "reload" => {
            // The URL from the snapshot, not a script: evalScript is off under the built-in
            // block list (v0.3.2).
            let url = page_url(&ctx, sid)?.filter(|u| !u.is_empty()).ok_or("no page to reload")?;
            step(format!("- openUrl: {}\n", q(&url)))?;
        }
        "wait" => {
            let timeout: u64 = opts.get("--timeout").map(|t| t.parse()).transpose().map_err(|_| "--timeout takes milliseconds")?.unwrap_or(10_000);
            if let Some(part) = opts.get("--url") {
                let deadline = Instant::now() + Duration::from_millis(timeout);
                loop {
                    let url = page_url(&ctx, sid)?.unwrap_or_default();
                    if url.contains(part.as_str()) {
                        println!("{url}");
                        break;
                    }
                    if Instant::now() >= deadline {
                        eprintln!("url is {url}, still without {part:?} after {timeout}ms");
                        std::process::exit(1);
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            } else {
                let what = arg(1, "milliseconds or a selector")?;
                if let Ok(ms) = what.parse::<u64>() {
                    std::thread::sleep(Duration::from_millis(ms));
                } else {
                    let kind = if flag("--gone") { "assertNotVisible" } else { "assertVisible" };
                    let sel = selector(what);
                    let body = if sel.starts_with('{') {
                        format!("{{ {}, timeout: {timeout} }}", sel.trim_start_matches('{').trim_end_matches('}').trim())
                    } else {
                        format!("{{ text: {sel}, timeout: {timeout} }}")
                    };
                    step(format!("- {kind}: {body}\n"))?;
                }
            }
        }
        "assert" | "is" => {
            let kind = match arg(1, "visible, hidden, text, contains or enabled")? {
                "visible" => "visible",
                "hidden" | "not-visible" | "not_visible" => "not_visible",
                "text" | "text_equals" => "text_equals",
                "contains" => "contains",
                "enabled" => "enabled",
                other => return Err(format!("unknown check `{other}` (visible, hidden, text, contains, enabled)").into()),
            };
            let mut a = json!({ "session": sid, "kind": kind, "selector": selector(arg(2, "a selector")?) });
            if matches!(kind, "text_equals" | "contains") {
                a["text"] = json!(rest(3));
            }
            if cmd == "is" {
                a["timeout"] = json!(opts.get("--timeout").and_then(|t| t.parse::<u64>().ok()).unwrap_or(500));
                let v = ctx.tool("assert", a)?;
                println!("{}", v["ok"] == true);
            } else {
                if let Some(t) = opts.get("--timeout") {
                    a["timeout"] = json!(t.parse::<u64>().map_err(|_| "--timeout takes milliseconds")?);
                }
                finish(&ctx, &ctx.tool("assert", a)?)?;
            }
        }
        "get" => {
            // Title and URL come from the snapshot: no evalScript, so they work with
            // allow_eval_script = false or block_actions set.
            let what = arg(1, "text, url or title")?;
            if matches!(what, "url" | "title") {
                let v = ctx.tool("snapshot", json!({ "session": sid, "format": "json" }))?;
                if v["ok"] == false {
                    return finish(&ctx, &v);
                }
                let snap: Value = serde_json::from_str(&text_of(&v)).map_err(|e| format!("snapshot is not JSON: {e}"))?;
                let value = snap[if what == "url" { "location" } else { "title" }].as_str().unwrap_or_default().to_owned();
                if json {
                    println!("{}", json!({ "ok": true, what: value }));
                } else {
                    println!("{value}");
                }
                return Ok(());
            }
            let yaml = match what {
                "text" | "value" => format!("- copyTextFrom: {}\n", selector(arg(2, "a selector")?)),
                other => return Err(format!("unknown `get {other}` (text, url, title)").into()),
            };
            let v = ctx.flow(&yaml, false)?;
            match (json, step_value(&text_of(&v))) {
                (false, Some(value)) if v["ok"] == true => println!("{value}"),
                _ => finish(&ctx, &v)?,
            }
        }
        "eval" => {
            let v = ctx.flow(&format!("- evalScript: {}\n", q(&rest(1))), false)?;
            match (json, step_value(&text_of(&v))) {
                (false, Some(value)) if v["ok"] == true => println!("{value}"),
                _ => finish(&ctx, &v)?,
            }
        }
        "screenshot" => {
            let mut a = json!({ "session": sid });
            if flag("--annotate") {
                a["annotate"] = json!("som");
            }
            let v = ctx.tool("screenshot", a)?;
            if v["ok"] == false {
                return finish(&ctx, &v);
            }
            let data = v["content"].as_array().into_iter().flatten().find_map(|b| b["data"].as_str()).ok_or("no image in the reply")?;
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
            let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let path = args.get(1).cloned().unwrap_or_else(|| format!("screenshot-{ms}.jpg"));
            std::fs::write(&path, bytes)?;
            println!("{path}");
            let marks = text_of(&v);
            if !marks.is_empty() {
                println!("{marks}");
            }
        }
        "record" => match arg(1, "start or stop")? {
            "start" => step(format!("- startRecording: {}\n", q(args.get(2).map_or("recording", String::as_str))))?,
            "stop" => step("- stopRecording\n".into())?,
            other => return Err(format!("unknown `record {other}` (start, stop)").into()),
        },
        "set" => {
            let yaml = match arg(1, "viewport, locale, geo, network or permission")? {
                "viewport" => {
                    let v = rest(2);
                    match v.split_once('x').map(|(w, h)| (w.trim().parse::<u32>(), h.trim().parse::<u32>())) {
                        Some((Ok(w), Ok(h))) => format!("- setViewport: {{ width: {w}, height: {h} }}\n"),
                        _ if v.is_empty() => return Err("set viewport needs a preset (iPhone 15, Pixel 8, desktop) or WxH".into()),
                        _ => format!("- setViewport: {}\n", q(&v)),
                    }
                }
                "locale" => match args.get(3) {
                    Some(tz) => format!("- setLocale: {{ locale: {}, timezone: {} }}\n", q(arg(2, "a locale")?), q(tz)),
                    None => format!("- setLocale: {{ locale: {} }}\n", q(arg(2, "a locale")?)),
                },
                "geo" | "location" => format!("- setLocation: {{ latitude: {}, longitude: {} }}\n", arg(2, "latitude")?, arg(3, "longitude")?),
                "network" => format!("- setNetwork: {}\n", arg(2, "offline or online")?),
                "permission" => format!("- setPermissions: {{ permissions: {{ {}: {} }} }}\n", arg(2, "a permission")?, arg(3, "allow, deny or unset")?),
                other => return Err(format!("unknown `set {other}` (viewport, locale, geo, network, permission)").into()),
            };
            step(yaml)?;
        }
        "launch" => step(format!("- launchApp: {}\n", q(arg(1, "an app id")?)))?,
        "stop-app" => step(args.get(1).map_or_else(|| "- stopApp\n".to_owned(), |a| format!("- stopApp: {}\n", q(a))))?,
        "clear-state" => step(args.get(1).map_or_else(|| "- clearState\n".to_owned(), |a| format!("- clearState: {}\n", q(a))))?,
        "hide-keyboard" => step("- hideKeyboard\n".into())?,
        "flow" => {
            let src = flow_source(arg(1, "a flow file, - or YAML")?)?;
            // A flow that does not start its own session runs in the shell's: say so up front.
            let open = ctx.has_session()?;
            if !src.contains("session:") && !open {
                return Err(format!(
                    "no session `{sid}` is open: run `fastpilot open <url>` first, or start one at the top of the flow with `- session: {{ start: web1, platform: web, target: <url> }}`"
                )
                .into());
            }
            // #9: the shell's session is only named when it is open. A flow that starts its own
            // (`- session: { start: … }`) was refused with "unknown session `default`" before its
            // first step ran.
            let session = if open { json!(ctx.session) } else { Value::Null };
            let v = ctx.tool("run_flow", json!({ "flow": src, "session": session, "stop_on_fail": !flag("--keep-going") }))?;
            finish(&ctx, &v)?;
        }
        "ship-check" => return ship_check(&ctx, &args[1..], opts.get("--links").map(String::as_str), flag("--cookies")),
        "check-flow" => {
            let src = flow_source(arg(1, "a flow file, - or YAML")?)?;
            finish(&ctx, &ctx.tool("check_flow", json!({ "flow": src }))?)?;
        }
        "errors" => {
            let v = send(&json!({ "op": "errors", "session": sid }), headed)?;
            if v["ok"] == false {
                return finish(&ctx, &v);
            }
            let lines: Vec<&str> = v["lines"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            if json {
                println!("{v}");
            } else if lines.is_empty() {
                println!("no errors");
            } else {
                println!("{}", lines.join("\n"));
            }
        }
        "diff" => {
            if arg(1, "screenshot")? != "screenshot" {
                return Err("only `diff screenshot <name>` exists".into());
            }
            let mut parts = vec![format!("name: {}", q(arg(2, "a baseline name")?))];
            for (opt, key) in [("--threshold", "threshold"), ("--max-diff", "maxDiff")] {
                if let Some(v) = opts.get(opt) {
                    parts.push(format!("{key}: {}", v.parse::<f64>().map_err(|_| format!("{opt} takes a number from 0 to 1"))?));
                }
            }
            if !ignores.is_empty() {
                parts.push(format!("ignore: [{}]", ignores.iter().map(|i| selector(i)).collect::<Vec<_>>().join(", ")));
            }
            if flag("--update") {
                parts.push("update: true".into());
            }
            step(format!("- assertScreenshot: {{ {} }}\n", parts.join(", ")))?;
        }
        "state" => {
            let action = arg(1, "save, load, list or delete")?;
            let mut a = json!({ "action": action, "session": sid });
            if action != "list" {
                a["name"] = json!(arg(2, "a state name")?);
            }
            finish(&ctx, &ctx.tool("state", a)?)?;
        }
        "docs" => finish(&ctx, &ctx.tool("docs", json!({ "topic": args.get(1).cloned().unwrap_or_else(|| "dsl".into()) }))?)?,
        "daemon" => match args.get(1).map(String::as_str) {
            Some("stop") => match send_once(&json!({ "op": "stop" })) {
                Ok(_) => println!("daemon stopped (sessions closed)"),
                Err(_) => println!("no daemon running"),
            },
            Some("status") | None => match send_once(&json!({ "op": "ping" })) {
                Ok(v) => {
                    let s = send_once(&json!({ "op": "sessions" })).unwrap_or_default();
                    println!("running (pid {}) on {}; sessions: {}", v["pid"], socket_path().display(), s["sessions"]);
                }
                Err(_) => {
                    println!("not running ({})", socket_path().display());
                    std::process::exit(1);
                }
            },
            Some(other) => return Err(format!("unknown `daemon {other}` (status, stop)").into()),
        },
        other => return Err(format!("unknown command `{other}`; see `fastpilot help`").into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_from_the_command_line() {
        assert_eq!(selector("@e7"), "{ ref: \"e7\" }");
        assert_eq!(selector("#email"), "{ id: \"email\" }");
        assert_eq!(selector("role=button:Masuk"), "{ role: \"button\", name: \"Masuk\" }");
        assert_eq!(selector("role=link"), "{ role: \"link\" }");
        assert_eq!(selector("120,340"), "{ point: [120, 340] }");
        assert_eq!(selector("Lupa \"password\"?"), "\"Lupa \\\"password\\\"?\"");
        assert_eq!(selector("text=#1 pilihan"), "\"#1 pilihan\"");
        for s in ["@e7", "#email", "role=button:Masuk", "120,340", "Masuk", "text=e12"] {
            assert!(fastpilot_core::Selector::parse_yaml(&selector(s)).is_ok(), "{s} → {}", selector(s));
        }
    }

    #[test]
    fn confirmed_selectors_parse() {
        for s in ["@e7", "Hapus akun", "role=button:Bayar"] {
            let yaml = format!("- tapOn: {}\n", confirmed(&selector(s)));
            let flow = fastpilot_core::parse_flow(&yaml).unwrap_or_else(|e| panic!("{yaml}: {e:?}"));
            assert!(matches!(flow.steps[0].step, fastpilot_core::Step::Confirmed(_)), "{yaml}");
        }
    }

    #[test]
    fn finds_the_command_after_global_options() {
        let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(command_of(&v(&["--session", "two", "open", "x"])), Some("open"));
        assert_eq!(command_of(&v(&["--json", "snapshot"])), Some("snapshot"));
        assert_eq!(command_of(&v(&["mcp"])), None);
        assert_eq!(command_of(&v(&["--session", "open"])), None);
    }

    #[test]
    fn step_values() {
        assert_eq!(step_value("ok 1/1\n  ✓ 1 [s] evalScript \"document.title\" 2ms = \"Login\"\n"), Some("Login".into()));
        assert_eq!(step_value("  ✓ 1 [s] copyTextFrom \"Email\" 3ms = a@b.com"), Some("a@b.com".into()));
        assert_eq!(step_value("  ✗ 1 [s] evalScript \"x\" 2ms script_error"), None);
    }
}
