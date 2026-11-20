//! `fastpilot run --report-github`: the suite's results as one comment on the pull request, while
//! running in GitHub Actions. The comment is found again by a hidden marker and edited, never
//! duplicated: "running" at the start, the results at the end.
//!
//! Everything comes from the Actions environment: `GITHUB_REPOSITORY`, the pull request number
//! (`GITHUB_EVENT_PATH` → `pull_request.number`, or `GITHUB_REF` `refs/pull/<n>/merge`),
//! `GITHUB_API_URL` and the token, only from `GITHUB_TOKEN`. The token is never stored, never put
//! in a URL and never printed. `--report-github=dry` prints the comment instead of posting it.

use std::time::Duration;

use serde_json::{Value, json};

use crate::run::CaseResult;

/// Hidden in the comment: how fastpilot finds its own comment again.
pub const MARKER: &str = "<!-- fastpilot-report -->";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Off,
    /// Create or update the comment through the API.
    Live,
    /// Print the comment and where it would go; no network.
    Dry,
}

impl Mode {
    /// `--report-github`, `--report-github=dry` or `--report-github dry`.
    pub fn from_args(args: &[String]) -> Result<Self, String> {
        let Some(i) = args.iter().position(|a| a == "--report-github" || a.starts_with("--report-github=")) else {
            return Ok(Self::Off);
        };
        let value = match args[i].split_once('=') {
            Some((_, v)) => Some(v),
            None => args.get(i + 1).map(String::as_str).filter(|v| *v == "dry" || *v == "live"),
        };
        match value {
            None | Some("live") => Ok(Self::Live),
            Some("dry") => Ok(Self::Dry),
            Some(other) => Err(format!("--report-github takes `dry` or nothing, not `{other}`")),
        }
    }
}

/// Where the comment goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub api: String,
    pub repo: String,
    pub pr: u64,
    /// Link to this Actions run, when known.
    pub run_url: Option<String>,
}

impl Target {
    /// From the Actions environment (`env` reads one variable).
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let repo = env("GITHUB_REPOSITORY").filter(|r| r.contains('/')).ok_or(
            "not in GitHub Actions (GITHUB_REPOSITORY is not set): --report-github posts to a pull request from a workflow; try --report-github=dry",
        )?;
        let from_event = env("GITHUB_EVENT_PATH").and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| {
            v["pull_request"]["number"].as_u64().or_else(|| v["number"].as_u64().filter(|_| v["pull_request"].is_object()))
        });
        let from_ref = || {
            env("GITHUB_REF").and_then(|r| r.strip_prefix("refs/pull/").and_then(|r| r.split('/').next()).and_then(|n| n.parse().ok()))
        };
        let pr = from_event.or_else(from_ref).ok_or("this workflow run is not for a pull request (no pull_request in GITHUB_EVENT_PATH, GITHUB_REF is not refs/pull/<n>/...): nothing to comment on")?;
        let api = env("GITHUB_API_URL").filter(|u| !u.is_empty()).unwrap_or_else(|| "https://api.github.com".into());
        let run_url = match (env("GITHUB_SERVER_URL"), env("GITHUB_RUN_ID")) {
            (Some(server), Some(id)) => Some(format!("{}/{repo}/actions/runs/{id}", server.trim_end_matches('/'))),
            _ => None,
        };
        Ok(Self { api: api.trim_end_matches('/').to_owned(), repo, pr, run_url })
    }
}

/// A table cell: one line, shown as plain text. Notes come from the model and the page under
/// test: Markdown and HTML in them are escaped, never rendered (no links, images or mentions).
fn cell(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let clipped: String = one.chars().take(200).collect();
    let clipped = if one.chars().count() > 200 { format!("{clipped}…") } else { clipped };
    let mut out = String::with_capacity(clipped.len());
    for c in clipped.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '@' => out.push_str("@\u{200b}"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '(' | ')' | '#' | '!' | '~' | '|' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// The comment while the suite runs.
pub fn running_body(tests: usize, target: &Target) -> String {
    let run = target.run_url.as_deref().map(|u| format!(" · [workflow run]({u})")).unwrap_or_default();
    format!("{MARKER}\n### fastpilot: ⏳ running {tests} test(s)…\n\n<sub>fastpilot {}{run}</sub>\n", env!("CARGO_PKG_VERSION"))
}

/// The comment with the results.
pub fn results_body(results: &[CaseResult], elapsed: Duration, target: &Target) -> String {
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.iter().filter(|r| !r.passed && !r.quarantined).count();
    let quarantined = results.iter().filter(|r| !r.passed && r.quarantined).count();
    let flaky = results.iter().filter(|r| r.passed && r.attempts > 1).count();
    let icon = if failed > 0 { "❌" } else { "✅" };
    let mut b = format!("{MARKER}\n### fastpilot: {icon} {passed} passed · {failed} failed");
    if flaky > 0 {
        b.push_str(&format!(" · {flaky} flaky"));
    }
    if quarantined > 0 {
        b.push_str(&format!(" · {quarantined} quarantined"));
    }
    b.push_str(&format!(" (in {:.1}s)\n\n| | test | result | time |\n|---|---|---|---|\n", elapsed.as_secs_f64()));
    // Failures first, then the rest in suite order.
    let mut rows: Vec<&CaseResult> = results.iter().filter(|r| !r.passed).collect();
    rows.extend(results.iter().filter(|r| r.passed));
    for r in rows {
        let (mark, result) = if r.passed {
            (if r.attempts > 1 { "⚠️" } else { "✅" }, if r.attempts > 1 { format!("passed on attempt {}", r.attempts) } else { "passed".to_owned() })
        } else {
            let class = r.class.map(|c| format!(" ({})", c.as_str())).unwrap_or_default();
            let q = if r.quarantined { " · quarantined" } else { "" };
            ("❌", format!("failed{class}{q}: {}", r.notes))
        };
        b.push_str(&format!(
            "| {mark} | {} | {} | {:.1}s |\n",
            cell(&format!("{} — {}", r.case.id, r.case.title)),
            cell(&fastpilot_core::redact(&result)),
            r.elapsed.as_secs_f64()
        ));
    }
    let run = target.run_url.as_deref().map(|u| format!(" · [workflow run and reports]({u})")).unwrap_or_default();
    b.push_str(&format!("\n<sub>fastpilot {}{run}</sub>\n", env!("CARGO_PKG_VERSION")));
    b
}

/// Creates the comment, or edits the one carrying [`MARKER`]. Errors never contain the token.
pub async fn publish(target: &Target, token: &str, body: &str) -> Result<String, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let send = |req: reqwest::RequestBuilder| {
        req.bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", concat!("fastpilot/", env!("CARGO_PKG_VERSION")))
    };
    let base = format!("{}/repos/{}/issues", target.api, target.repo);
    let fail = |what: &str, e: String| format!("{what}: {}", e.replace(token, "***"));
    // Who comments with this token: a person's token answers /user; the Actions token does not
    // (403) and comments as github-actions[bot].
    let me = match send(client.get(format!("{}/user", target.api))).send().await {
        Ok(r) if r.status().is_success() => r.json::<Value>().await.ok().and_then(|v| v["login"].as_str().map(str::to_owned)),
        _ => None,
    }
    .unwrap_or_else(|| "github-actions[bot]".to_owned());
    let mut existing = None;
    for page in 1..=10 {
        let resp = send(client.get(format!("{base}/{}/comments?per_page=100&page={page}", target.pr))).send().await.map_err(|e| fail("listing PR comments", e.to_string()))?;
        if !resp.status().is_success() {
            return Err(fail("listing PR comments", format!("HTTP {}", resp.status())));
        }
        let items: Vec<Value> = resp.json().await.map_err(|e| fail("listing PR comments", e.to_string()))?;
        // The marker can be copied by anyone who comments: only a marked comment written with this
        // token's account is edited; otherwise fastpilot posts its own.
        existing = items
            .iter()
            .find(|c| c["body"].as_str().is_some_and(|b| b.contains(MARKER)) && c["user"]["login"].as_str() == Some(me.as_str()))
            .and_then(|c| c["id"].as_u64());
        if existing.is_some() || items.len() < 100 {
            break;
        }
    }
    let payload = json!({ "body": body });
    let mut updated = existing.is_some();
    let (mut what, mut resp) = match existing {
        Some(id) => ("updating the PR comment", send(client.patch(format!("{base}/comments/{id}"))).json(&payload).send().await),
        None => ("creating the PR comment", send(client.post(format!("{base}/{}/comments", target.pr))).json(&payload).send().await),
    };
    // Someone else's comment with the marker (403/404 on edit): post our own instead.
    if updated && resp.as_ref().is_ok_and(|r| matches!(r.status().as_u16(), 403 | 404)) {
        updated = false;
        what = "creating the PR comment";
        resp = send(client.post(format!("{base}/{}/comments", target.pr))).json(&payload).send().await;
    }
    let resp = resp.map_err(|e| fail(what, e.to_string()))?;
    if !resp.status().is_success() {
        return Err(fail(what, format!("HTTP {}", resp.status())));
    }
    let url = resp.json::<Value>().await.ok().and_then(|v| v["html_url"].as_str().map(str::to_owned)).unwrap_or_default();
    Ok(format!("{} {}#{} {url}", if updated { "updated" } else { "created" }, target.repo, target.pr).trim_end().to_owned())
}

/// The suite's reporter for one run: posts (live) or prints (dry); never fails the suite.
pub struct Reporter {
    mode: Mode,
    target: Option<Target>,
    /// Why there is no target (not in Actions, not a pull request).
    missing: Option<String>,
}

impl Reporter {
    pub fn new(mode: Mode) -> Self {
        if mode == Mode::Off {
            return Self { mode, target: None, missing: None };
        }
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        match Target::from_env(env) {
            Ok(t) => Self { mode, target: Some(t), missing: None },
            Err(e) => {
                if mode == Mode::Live {
                    eprintln!("report-github: {e}");
                }
                Self { mode, target: None, missing: Some(e) }
            }
        }
    }

    pub async fn send(&self, body: &str) {
        if self.mode == Mode::Dry {
            let to = match (&self.target, &self.missing) {
                (Some(t), _) => format!("{}#{}", t.repo, t.pr),
                (None, Some(why)) => format!("no pull request ({why})"),
                (None, None) => "no pull request".to_owned(),
            };
            println!("--- GitHub PR comment for {to} (dry run, not posted) ---\n{body}--- end ---");
            return;
        }
        let Some(target) = &self.target else { return };
        match self.mode {
            Mode::Off | Mode::Dry => {}
            Mode::Live => match std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()) {
                None => eprintln!("report-github: GITHUB_TOKEN is not set; give the job `permissions: pull-requests: write` and `env: GITHUB_TOKEN: ${{{{ secrets.GITHUB_TOKEN }}}}`"),
                Some(token) => match publish(target, &token, body).await {
                    Ok(done) => println!("report-github: {done}"),
                    Err(e) => eprintln!("report-github: {e}"),
                },
            },
        }
    }

    /// Where the comment goes, or a stand-in for the dry run's text.
    pub fn target_or_local(&self) -> Target {
        self.target.clone().unwrap_or(Target { api: String::new(), repo: String::new(), pr: 0, run_url: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::{Arc, Mutex};

    #[test]
    fn modes_and_targets() {
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(Mode::from_args(&args(&["run", "tests"])), Ok(Mode::Off));
        assert_eq!(Mode::from_args(&args(&["run", "tests", "--report-github"])), Ok(Mode::Live));
        assert_eq!(Mode::from_args(&args(&["run", "--report-github=dry", "tests"])), Ok(Mode::Dry));
        assert_eq!(Mode::from_args(&args(&["run", "--report-github", "dry"])), Ok(Mode::Dry));
        assert!(Mode::from_args(&args(&["run", "--report-github=loud"])).is_err());

        let dir = std::env::temp_dir().join(format!("fastpilot-gh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let event = dir.join("event.json");
        std::fs::write(&event, r#"{"action":"synchronize","number":42,"pull_request":{"number":42}}"#).expect("event");
        let vars = |extra: &[(&str, String)]| {
            let mut m: std::collections::HashMap<String, String> =
                [("GITHUB_REPOSITORY", "autobot-lab/fastpilot".to_owned()), ("GITHUB_SERVER_URL", "https://github.com".into()), ("GITHUB_RUN_ID", "9".into())]
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v))
                    .collect();
            m.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), v.clone())));
            move |k: &str| m.get(k).cloned()
        };
        let t = Target::from_env(vars(&[("GITHUB_EVENT_PATH", event.display().to_string())])).expect("event");
        assert_eq!((t.api.as_str(), t.repo.as_str(), t.pr), ("https://api.github.com", "autobot-lab/fastpilot", 42));
        assert_eq!(t.run_url.as_deref(), Some("https://github.com/autobot-lab/fastpilot/actions/runs/9"));
        let t = Target::from_env(vars(&[("GITHUB_REF", "refs/pull/7/merge".into()), ("GITHUB_API_URL", "http://127.0.0.1:1/".into())])).expect("ref");
        assert_eq!((t.api.as_str(), t.pr), ("http://127.0.0.1:1", 7));
        assert!(Target::from_env(vars(&[("GITHUB_REF", "refs/heads/main".into())])).expect_err("push").contains("not for a pull request"));
        assert!(Target::from_env(|_| None).expect_err("local").contains("not in GitHub Actions"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A stand-in for the GitHub REST API: one PR, its issue comments, every request logged.
    struct FakeGitHub {
        port: u16,
        log: Arc<Mutex<Vec<(String, String, String)>>>,
    }

    impl FakeGitHub {
        fn start() -> Self {
            Self::with(vec![json!({ "id": 1, "body": "LGTM" })])
        }

        /// The Actions token: no /user (403), comments as github-actions[bot].
        fn with(seed: Vec<Value>) -> Self {
            Self::as_user(seed, None)
        }

        /// `me`: a person's token, answered by /user.
        fn as_user(seed: Vec<Value>, me: Option<&'static str>) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().expect("addr").port();
            let log = Arc::new(Mutex::new(Vec::new()));
            let comments = Arc::new(Mutex::new(seed));
            let (log2, comments2) = (log.clone(), comments.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut r = BufReader::new(stream.try_clone().expect("clone"));
                    let mut line = String::new();
                    if r.read_line(&mut line).is_err() {
                        continue;
                    }
                    let (method, path) = {
                        let mut it = line.split_whitespace();
                        (it.next().unwrap_or("").to_owned(), it.next().unwrap_or("").to_owned())
                    };
                    let (mut len, mut auth) = (0usize, String::new());
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                            break;
                        }
                        let lower = h.to_ascii_lowercase();
                        if let Some(v) = lower.strip_prefix("content-length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                        if lower.starts_with("authorization:") {
                            auth = h.trim().to_owned();
                        }
                    }
                    let mut body = vec![0u8; len];
                    let _ = r.read_exact(&mut body);
                    let body = String::from_utf8_lossy(&body).into_owned();
                    log2.lock().expect("log").push((method.clone(), path.clone(), auth.clone()));
                    let (status, reply) = if auth != "authorization: Bearer test-token-123" && auth != "Authorization: Bearer test-token-123" {
                        ("401 Unauthorized", json!({ "message": "Bad credentials" }))
                    } else if method == "GET" && path == "/user" {
                        match me {
                            Some(login) => ("200 OK", json!({ "login": login, "type": "User" })),
                            None => ("403 Forbidden", json!({ "message": "Resource not accessible by integration" })),
                        }
                    } else if method == "GET" && path.starts_with("/repos/o/r/issues/5/comments") {
                        ("200 OK", Value::Array(comments2.lock().expect("c").clone()))
                    } else if method == "POST" && path == "/repos/o/r/issues/5/comments" {
                        let text = serde_json::from_str::<Value>(&body).map(|v| v["body"].clone()).unwrap_or_default();
                        let user = me.map_or_else(|| json!({ "login": "github-actions[bot]", "type": "Bot" }), |l| json!({ "login": l, "type": "User" }));
                        comments2.lock().expect("c").push(json!({ "id": 77, "body": text, "user": user }));
                        ("201 Created", json!({ "id": 77, "html_url": "https://github.com/o/r/pull/5#issuecomment-77" }))
                    } else if method == "PATCH" && path == "/repos/o/r/issues/comments/77" {
                        let text = serde_json::from_str::<Value>(&body).map(|v| v["body"].clone()).unwrap_or_default();
                        if let Some(c) = comments2.lock().expect("c").iter_mut().find(|c| c["id"] == 77) {
                            c["body"] = text;
                        }
                        ("200 OK", json!({ "id": 77, "html_url": "https://github.com/o/r/pull/5#issuecomment-77" }))
                    } else {
                        ("404 Not Found", json!({ "message": "Not Found" }))
                    };
                    let reply = reply.to_string();
                    let mut s = stream;
                    let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
                }
            });
            let _ = comments;
            Self { port, log }
        }

        fn target(&self) -> Target {
            Target { api: format!("http://127.0.0.1:{}", self.port), repo: "o/r".into(), pr: 5, run_url: None }
        }

        fn calls(&self) -> Vec<String> {
            self.log.lock().expect("log").iter().map(|(m, p, _)| format!("{m} {}", p.split('?').next().unwrap_or(p))).collect()
        }
    }

    /// C6: one comment per PR, created once and edited after; the token only in the header.
    #[tokio::test]
    async fn one_comment_created_then_updated() {
        let gh = FakeGitHub::start();
        let t = gh.target();
        let first = publish(&t, "test-token-123", &format!("{MARKER}\nrunning")).await.expect("create");
        assert!(first.starts_with("created o/r#5"), "{first}");
        let second = publish(&t, "test-token-123", &format!("{MARKER}\nresults")).await.expect("update");
        assert!(second.starts_with("updated o/r#5"), "{second}");
        assert_eq!(
            gh.calls(),
            ["GET /user", "GET /repos/o/r/issues/5/comments", "POST /repos/o/r/issues/5/comments", "GET /user", "GET /repos/o/r/issues/5/comments", "PATCH /repos/o/r/issues/comments/77"]
        );
        assert!(gh.log.lock().expect("log").iter().all(|(_, path, auth)| !path.contains("test-token") && auth.ends_with("Bearer test-token-123")));
        // A wrong token: an error that does not repeat the token.
        let e = publish(&t, "wrong-secret-token", "x").await.expect_err("401");
        assert!(e.contains("401") && !e.contains("wrong-secret-token"), "{e}");
    }

    /// Security (Low 1): anyone can copy the marker into a comment of their own, and a token that may
    /// edit others' comments (write access) then wrote the report into it. Only a marked comment of
    /// this token's account is edited: github-actions[bot] for the Actions token, the /user login for
    /// a person's token. Others, bots included, are left alone and fastpilot posts its own.
    #[tokio::test]
    async fn a_copied_marker_does_not_capture_the_report() {
        let fake = |id: u64, login: &str, kind: &str| json!({ "id": id, "body": format!("{MARKER}\nfake results"), "user": { "type": kind, "login": login } });
        let gh = FakeGitHub::with(vec![fake(5, "someone", "User"), fake(6, "evil-app[bot]", "Bot")]);
        let t = gh.target();
        let done = publish(&t, "test-token-123", &format!("{MARKER}\nreal results")).await.expect("publish");
        assert!(done.starts_with("created o/r#5"), "{done}");
        // The next run edits its own comment, not the copies (no new comment each run).
        let again = publish(&t, "test-token-123", &format!("{MARKER}\nresults 2")).await.expect("publish");
        assert!(again.starts_with("updated o/r#5"), "{again}");
        assert_eq!(
            gh.calls(),
            ["GET /user", "GET /repos/o/r/issues/5/comments", "POST /repos/o/r/issues/5/comments", "GET /user", "GET /repos/o/r/issues/5/comments", "PATCH /repos/o/r/issues/comments/77"]
        );
        // A person's token: its own marked comment, found by login.
        let gh = FakeGitHub::as_user(vec![fake(5, "github-actions[bot]", "Bot")], Some("yogi"));
        let t = gh.target();
        assert!(publish(&t, "test-token-123", &format!("{MARKER}\na")).await.expect("publish").starts_with("created"));
        assert!(publish(&t, "test-token-123", &format!("{MARKER}\nb")).await.expect("publish").starts_with("updated"));
    }

    /// Security (Low): notes come from the model and the page: shown as text, never rendered.
    #[test]
    fn notes_are_plain_text_in_the_comment() {
        let c = cell("see [here](https://evil.example) ![x](y) <img src=x> **bold** @org/team `code` | pipe");
        assert!(!c.contains("](https") && !c.contains("<img") && !c.contains("**") && !c.contains("@org/team"), "{c}");
        assert!(c.contains("\\[here\\]\\(https://evil.example\\)") && c.contains("&lt;img src=x&gt;") && c.contains("\\|"), "{c}");
    }
}
