//! Jira Cloud, connected from the dashboard (v0.6 phase 1, spec SPEC-apps-jira.md).
//!
//! What is kept where:
//! - `~/.fastpilot/jira.json` (owner-only): the site, the email, the default project, the QA
//!   status and the last check. No secret. It is not part of `config.toml`, so neither the MCP
//!   `config` tool nor Chat can read or change it: connecting, changing and disconnecting happen
//!   only by a click on the dashboard's Apps page.
//! - The API token: the OS key store (see [`token`]), never in a file the AI can read, a log, a
//!   report, Chat or the audit log.

use serde::{Deserialize, Serialize};

use std::path::PathBuf;

/// The saved connection (no secret).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// `yourteam.atlassian.net`.
    pub site: String,
    pub email: String,
    #[serde(default)]
    pub project_key: String,
    #[serde(default)]
    pub project_name: String,
    /// Saved for later (automatic testing when a ticket moves there: phase 3).
    #[serde(default)]
    pub qa_status: String,
    /// "Connected as …".
    #[serde(default)]
    pub display_name: String,
    /// When the connection last answered (ms since 1970).
    #[serde(default)]
    pub checked_ms: u64,
    /// Jira refused the saved token on the last call: the Apps page says "Needs attention".
    #[serde(default)]
    pub needs_attention: bool,
}

fn path() -> PathBuf {
    crate::config::path().parent().map(std::path::Path::to_path_buf).unwrap_or_else(std::env::temp_dir).join("jira.json")
}

/// The saved connection, if Jira was connected.
pub fn settings() -> Option<Settings> {
    let s: Settings = serde_json::from_str(&std::fs::read_to_string(path()).ok()?).ok()?;
    (!s.site.is_empty()).then_some(s)
}

/// The Jira Cloud host for what a person typed: `yourteam`, `yourteam.atlassian.net` or a whole
/// address (`https://yourteam.atlassian.net/jira/software/projects/KAN`). Only `*.atlassian.net`.
pub fn parse_site(input: &str) -> Result<String, String> {
    let s = input.trim().to_ascii_lowercase();
    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(&s);
    let host = s.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host).trim_end_matches('.');
    let label = if host.contains('.') { host.strip_suffix(".atlassian.net").unwrap_or("") } else { host };
    let ok = !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-') && label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(format!("{label}.atlassian.net"))
    } else {
        Err("fastpilot works with Jira Cloud addresses that end in .atlassian.net. Jira Server and Data Center are not supported yet.".into())
    }
}

/// Save the connection (owner-only, written atomically).
pub fn save_settings(s: &Settings) -> Result<(), String> {
    let p = path();
    let dir = p.parent().ok_or("no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::remove_file(&tmp);
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, &text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

/// Forget the connection's settings.
pub fn clear_settings() {
    let _ = std::fs::remove_file(path());
}

/// Mark the saved connection after a call: refused (`true`) shows "Needs attention" on Apps.
pub fn mark(refused: bool, display_name: Option<&str>) {
    if let Some(mut s) = settings() {
        s.needs_attention = refused;
        if !refused {
            s.checked_ms = now_ms();
        }
        if let Some(n) = display_name {
            s.display_name = n.to_owned();
        }
        let _ = save_settings(&s);
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Where requests for `site` go: `https://<site>`. Tests (only) point every site at a local fake
/// Jira with `FASTPILOT_JIRA_TEST_BASE=http://127.0.0.1:<port>`; the site becomes the first path
/// segment there. A token is bound to the origin it was saved for and is sent nowhere else.
pub fn origin_for(site: &str) -> String {
    match test_base() {
        Some(b) => format!("{b}/{site}"),
        None => format!("https://{site}"),
    }
}

/// Only in debug builds (the tests): a release build never sends a typed token to a local port,
/// whatever its environment says (Security, Low 8).
#[cfg(debug_assertions)]
fn test_base() -> Option<String> {
    let b = std::env::var("FASTPILOT_JIRA_TEST_BASE").ok()?;
    let b = b.trim_end_matches('/');
    let rest = b.strip_prefix("http://127.0.0.1:").or_else(|| b.strip_prefix("http://localhost:"))?;
    rest.parse::<u16>().ok().map(|_| b.to_owned())
}

#[cfg(not(debug_assertions))]
fn test_base() -> Option<String> {
    None
}

/// The API token in the OS key store (macOS Keychain, Linux secret service, else an owner-only
/// file; `FASTPILOT_KEYSTORE=file` in tests), with the origin it was saved for. Never in argv:
/// `security -i` and `secret-tool store` read it from stdin. No environment variable can supply it.
pub mod token {
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    const SERVICE: &str = "fastpilot-jira";

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Backend {
        Keychain,
        SecretService,
        File,
    }

    pub fn backend() -> Backend {
        match std::env::var("FASTPILOT_KEYSTORE").ok().as_deref() {
            Some("file") => return Backend::File,
            Some("keychain") => return Backend::Keychain,
            Some("secret-service") => return Backend::SecretService,
            _ => {}
        }
        if cfg!(target_os = "macos") {
            Backend::Keychain
        } else if Command::new("secret-tool").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok() {
            Backend::SecretService
        } else {
            Backend::File
        }
    }

    /// The words for where it is kept.
    pub fn kept_in() -> &'static str {
        match backend() {
            Backend::Keychain => "this Mac's Keychain",
            Backend::SecretService => "this computer's secret store",
            Backend::File => "a file only you can read",
        }
    }

    /// A token is one line of printable characters (no spaces or quotes): it is quoted for
    /// `security -i`, and a pasted newline would become part of it.
    pub fn check(token: &str) -> Result<(), String> {
        if token.is_empty() || token.len() > 4096 || !token.chars().all(|c| c.is_ascii_graphic() && c != '"' && c != '\\' && c != '\'') {
            return Err("The token must be one line without spaces or quotes. Copy it again from Atlassian.".into());
        }
        Ok(())
    }

    fn file(account: &str) -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        home.join(".fastpilot").join("keys").join(account)
    }

    fn run(program: &str, args: &[&str], stdin: &str) -> Result<(), String> {
        let mut child = Command::new(program).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_| format!("{program} could not start"))?;
        if let Some(mut i) = child.stdin.take() {
            i.write_all(stdin.as_bytes()).map_err(|_| format!("{program} could not read the token"))?;
        }
        let st = child.wait().map_err(|_| format!("{program} failed"))?;
        if st.success() { Ok(()) } else { Err(format!("{program} refused")) }
    }

    fn store(account: &str, value: &str) -> Result<(), String> {
        match backend() {
            Backend::Keychain => run("security", &["-i"], &format!("add-generic-password -U -s {SERVICE} -a {account} -l \"fastpilot jira\" -w \"{value}\"\n")),
            Backend::SecretService => run("secret-tool", &["store", "--label=fastpilot jira", "service", SERVICE, "account", account], value),
            Backend::File => {
                let p = file(account);
                let dir = p.parent().ok_or("no key folder")?;
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
                    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
                    let tmp = p.with_extension("tmp");
                    let _ = std::fs::remove_file(&tmp);
                    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
                    f.write_all(value.as_bytes()).map_err(|e| e.to_string())?;
                    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
                }
                #[cfg(not(unix))]
                std::fs::write(&p, value).map_err(|e| e.to_string())
            }
        }
    }

    fn load(account: &str) -> Option<String> {
        let v = match backend() {
            Backend::Keychain => {
                let o = Command::new("security").args(["find-generic-password", "-s", SERVICE, "-a", account, "-w"]).stderr(Stdio::null()).output().ok()?;
                o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())?
            }
            Backend::SecretService => {
                let o = Command::new("secret-tool").args(["lookup", "service", SERVICE, "account", account]).stderr(Stdio::null()).output().ok()?;
                o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())?
            }
            Backend::File => std::fs::read_to_string(file(account)).ok()?,
        };
        Some(v.trim().to_owned()).filter(|v| !v.is_empty())
    }

    fn remove(account: &str) -> bool {
        match backend() {
            Backend::Keychain => {
                let _ = Command::new("security").args(["delete-generic-password", "-s", SERVICE, "-a", account]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
            Backend::SecretService => {
                let _ = Command::new("secret-tool").args(["clear", "service", SERVICE, "account", account]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
            Backend::File => {
                let _ = std::fs::remove_file(file(account));
            }
        }
        load(account).is_none()
    }

    /// Keep the token, bound to `origin` (where it may be sent).
    pub fn set(token: &str, origin: &str) -> Result<(), String> {
        check(token)?;
        store("jira", token)?;
        store("jira.origin", origin)
    }

    /// The token and the origin it was saved for.
    pub fn get() -> Option<(String, String)> {
        Some((load("jira")?, load("jira.origin")?))
    }

    pub fn has() -> bool {
        load("jira").is_some()
    }

    /// Forget it. `Err` when it is still there afterwards (the person is told, not reassured).
    pub fn delete() -> Result<(), String> {
        let a = remove("jira");
        let b = remove("jira.origin");
        if a && b { Ok(()) } else { Err("still stored".into()) }
    }
}

// ---- the client ------------------------------------------------------------------------------

/// What can go wrong, each with the words the person reads (design 02, states 3-9, 12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JiraError {
    /// 401: wrong email or token (or the saved token stopped working).
    Refused,
    /// 403: refused access, or API tokens are off for the account.
    NoAccess,
    /// DNS failure, or the site answers 404 on the account check.
    SiteNotFound(String),
    Network,
    /// 429: wait this many seconds.
    Busy(u64),
    /// Not `*.atlassian.net`.
    NotCloud,
    /// No saved connection.
    NotConnected,
    /// A ticket or project that does not exist (or is not visible).
    NotFound(String),
    /// Jira refused the content (400), with its own reason, cleaned.
    Rejected(String),
    Other(u16),
}

impl std::fmt::Display for JiraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused => write!(f, "Jira did not accept the email or token. Check that the email is the one you use for Atlassian. If you are not sure, create a new token and paste it again."),
            Self::NoAccess => write!(f, "Jira refused access. Your account may not be allowed to use tokens, or your Jira admin turned them off. Ask your admin to allow API tokens for you."),
            Self::SiteNotFound(site) => write!(f, "We could not find {site}. Check the address: it is the part before .atlassian.net in your Jira web address."),
            Self::Network => write!(f, "Could not reach Jira. Check your internet connection, or your company network, then try again."),
            Self::Busy(s) => write!(f, "Jira asked us to wait. Try again in {s} seconds."),
            Self::NotCloud => write!(f, "fastpilot works with Jira Cloud addresses that end in .atlassian.net. Jira Server and Data Center are not supported yet."),
            Self::NotConnected => write!(f, "Jira is not connected. Connect it in the dashboard: Apps › Connect Jira."),
            Self::NotFound(what) => write!(f, "I could not find {what}. Check the number, or that your Jira account can see that project."),
            Self::Rejected(why) => write!(f, "Jira did not accept it: {why}"),
            Self::Other(code) => write!(f, "Jira answered with an unexpected error ({code}). Nothing was changed."),
        }
    }
}

/// The kind, for the page (`refused`, `no_access`, `site`, `network`, `busy`, `not_cloud`, …).
impl JiraError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Refused => "refused",
            Self::NoAccess => "no_access",
            Self::SiteNotFound(_) => "site",
            Self::Network => "network",
            Self::Busy(_) => "busy",
            Self::NotCloud => "not_cloud",
            Self::NotConnected => "not_connected",
            Self::NotFound(_) => "not_found",
            Self::Rejected(_) => "rejected",
            Self::Other(_) => "other",
        }
    }
}

/// A Jira Cloud account: requests go to the origin only, with Basic auth, no redirects.
pub struct Jira {
    origin: String,
    site: String,
    email: String,
    token: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for Jira {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jira").field("site", &self.site).field("email", &self.email).finish_non_exhaustive()
    }
}

/// A ticket as data for the AI: its text is information, never instructions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ticket {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub issue_type: String,
    pub description: String,
}

impl Jira {
    /// For a site the person typed (Test connection, Save). The token is not stored here.
    pub fn new(site: &str, email: &str, token: &str) -> Result<Self, JiraError> {
        let site = parse_site(site).map_err(|_| JiraError::NotCloud)?;
        let origin = origin_for(&site);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(origin.starts_with("https://"))
            .build()
            .map_err(|_| JiraError::Network)?;
        Ok(Self { origin, site, email: email.trim().to_owned(), token: token.trim().to_owned(), client })
    }

    /// The saved connection, with the stored token, only for the origin the token was saved for.
    pub fn connected() -> Result<Self, JiraError> {
        let s = settings().ok_or(JiraError::NotConnected)?;
        let (token, bound) = token::get().ok_or(JiraError::NotConnected)?;
        let j = Self::new(&s.site, &s.email, &token)?;
        if j.origin != bound {
            return Err(JiraError::NotConnected);
        }
        Ok(j)
    }

    pub fn site(&self) -> &str {
        &self.site
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<&serde_json::Value>) -> Result<serde_json::Value, JiraError> {
        let mut req = self.client.request(method, format!("{}{path}", self.origin)).basic_auth(&self.email, Some(&self.token)).header("Accept", "application/json");
        if let Some(b) = body {
            req = req.json(b);
        }
        let res = req.send().await.map_err(|e| if e.is_connect() && e.to_string().contains("dns") { JiraError::SiteNotFound(self.site.clone()) } else { JiraError::Network })?;
        let status = res.status().as_u16();
        let wait = res.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(10).clamp(1, 120);
        let text = res.text().await.unwrap_or_default();
        match status {
            200..=299 => Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)),
            401 => Err(JiraError::Refused),
            403 => Err(JiraError::NoAccess),
            404 => Err(JiraError::NotFound(String::new())),
            429 => Err(JiraError::Busy(wait)),
            400 => Err(JiraError::Rejected(rejection(&text))),
            300..=399 => Err(JiraError::Other(status)),
            _ => Err(JiraError::Other(status)),
        }
    }

    /// Like [`call`], waiting once when Jira asks to (at most 30 s), then "busy".
    async fn call_patient(&self, method: reqwest::Method, path: &str, body: Option<&serde_json::Value>) -> Result<serde_json::Value, JiraError> {
        match self.call(method.clone(), path, body).await {
            Err(JiraError::Busy(s)) if s <= 30 => {
                tokio::time::sleep(std::time::Duration::from_secs(s)).await;
                self.call(method, path, body).await
            }
            r => r,
        }
    }

    /// "Connected as <name>".
    pub async fn myself(&self) -> Result<String, JiraError> {
        match self.call(reqwest::Method::GET, "/rest/api/3/myself", None).await {
            Err(JiraError::NotFound(_)) => Err(JiraError::SiteNotFound(self.site.clone())),
            r => Ok(r?["displayName"].as_str().unwrap_or("").to_owned()),
        }
    }

    /// Projects the account can see: (key, name).
    pub async fn projects(&self) -> Result<Vec<(String, String)>, JiraError> {
        let v = self.call(reqwest::Method::GET, "/rest/api/3/project/search?maxResults=100&orderBy=name", None).await?;
        Ok(v["values"].as_array().into_iter().flatten().filter_map(|p| Some((p["key"].as_str()?.to_owned(), p["name"].as_str()?.to_owned()))).collect())
    }

    /// The status names of a project (for the QA status picker), without repeats.
    pub async fn statuses(&self, project: &str) -> Result<Vec<String>, JiraError> {
        check_project(project)?;
        let v = self.call(reqwest::Method::GET, &format!("/rest/api/3/project/{project}/statuses"), None).await?;
        let mut out: Vec<String> = Vec::new();
        for t in v.as_array().into_iter().flatten() {
            for s in t["statuses"].as_array().into_iter().flatten() {
                if let Some(n) = s["name"].as_str()
                    && !out.iter().any(|x| x == n)
                {
                    out.push(n.to_owned());
                }
            }
        }
        Ok(out)
    }

    /// A ticket, as data.
    pub async fn issue(&self, key: &str) -> Result<Ticket, JiraError> {
        check_issue(key)?;
        let v = self
            .call_patient(reqwest::Method::GET, &format!("/rest/api/3/issue/{key}?fields=summary,description,status,issuetype"), None)
            .await
            .map_err(|e| if let JiraError::NotFound(_) = e { JiraError::NotFound(key.to_owned()) } else { e })?;
        let f = &v["fields"];
        Ok(Ticket {
            key: v["key"].as_str().unwrap_or(key).to_owned(),
            summary: plain(f["summary"].as_str().unwrap_or(""), 300),
            status: plain(f["status"]["name"].as_str().unwrap_or(""), 60),
            issue_type: plain(f["issuetype"]["name"].as_str().unwrap_or(""), 60),
            description: plain(&adf_text(&f["description"]), 20_000),
        })
    }

    /// An open issue in `project` whose summary contains `summary` (the duplicate guard): at most one.
    pub async fn find_open(&self, project: &str, summary: &str) -> Result<Option<(String, String)>, JiraError> {
        check_project(project)?;
        let words: String = summary.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect::<String>().split_whitespace().take(12).collect::<Vec<_>>().join(" ");
        if words.is_empty() {
            return Ok(None);
        }
        let jql = format!("project = \"{project}\" AND statusCategory != Done AND summary ~ \"{words}\" ORDER BY created DESC");
        let v = self.call_patient(reqwest::Method::POST, "/rest/api/3/search/jql", Some(&serde_json::json!({ "jql": jql, "maxResults": 1, "fields": ["summary"] }))).await?;
        Ok(v["issues"].as_array().and_then(|a| a.first()).and_then(|i| Some((i["key"].as_str()?.to_owned(), plain(i["fields"]["summary"].as_str().unwrap_or(""), 300)))))
    }

    /// The issue type to use: "Bug" when the project has it, else its first type.
    pub async fn bug_type(&self, project: &str) -> Result<String, JiraError> {
        check_project(project)?;
        let v = self.call_patient(reqwest::Method::GET, &format!("/rest/api/3/issue/createmeta/{project}/issuetypes"), None).await?;
        let names: Vec<String> = v["issueTypes"].as_array().or(v["values"].as_array()).into_iter().flatten().filter(|t| t["subtask"].as_bool() != Some(true)).filter_map(|t| t["name"].as_str().map(str::to_owned)).collect();
        Ok(names.iter().find(|n| n.eq_ignore_ascii_case("bug")).or(names.first()).cloned().unwrap_or_else(|| "Bug".into()))
    }

    /// Create an issue; returns its key.
    pub async fn create_issue(&self, project: &str, issue_type: &str, summary: &str, description: &str) -> Result<String, JiraError> {
        check_project(project)?;
        let body = serde_json::json!({ "fields": {
            "project": { "key": project },
            "issuetype": { "name": issue_type },
            "summary": summary,
            "description": adf(description),
        }});
        let v = self.call_patient(reqwest::Method::POST, "/rest/api/3/issue", Some(&body)).await?;
        v["key"].as_str().map(str::to_owned).ok_or(JiraError::Other(200))
    }

    /// Add a comment to an issue.
    pub async fn comment(&self, key: &str, text: &str) -> Result<(), JiraError> {
        check_issue(key)?;
        self.call_patient(reqwest::Method::POST, &format!("/rest/api/3/issue/{key}/comment"), Some(&serde_json::json!({ "body": adf(text) })))
            .await
            .map_err(|e| if let JiraError::NotFound(_) = e { JiraError::NotFound(key.to_owned()) } else { e })?;
        Ok(())
    }

    /// The web link of an issue (for the person, not for the AI to follow).
    pub fn browse(&self, key: &str) -> String {
        format!("https://{}/browse/{key}", self.site)
    }
}

/// Jira writes Claude Code asked for (Security, Medium 1). In the terminal the AI does not write
/// to Jira itself: `jira_create_issue` and `jira_comment` file a request with the exact cleaned
/// text, and the person approves it on the dashboard's Apps page or with `fastpilot jira approve`
/// in a terminal they type in. The file is read as untrusted input and every request is checked
/// again before it is sent.
pub mod requests {
    use super::*;

    /// A request waits at most this long (30 minutes).
    pub const EXPIRY_MS: u64 = 30 * 60_000;
    const MAX: usize = 10;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Request {
        pub id: String,
        /// `issue` or `comment`.
        pub kind: String,
        pub site: String,
        /// `issue`: the project key.
        #[serde(default)]
        pub project: String,
        /// `comment`: the issue key.
        #[serde(default)]
        pub key: String,
        /// `issue`: one line.
        #[serde(default)]
        pub summary: String,
        /// `issue`: the description; `comment`: the comment.
        pub text: String,
        pub created_ms: u64,
    }

    impl Request {
        /// What it does, in fastpilot's words: "Create an issue in KAN" / "Comment on KAN-12".
        pub fn title(&self) -> String {
            if self.kind == "issue" { format!("Create a Jira issue in {}", self.project) } else { format!("Comment on {}", self.key) }
        }
    }

    fn path() -> PathBuf {
        crate::config::path().parent().map(std::path::Path::to_path_buf).unwrap_or_else(std::env::temp_dir).join("jira-requests.json")
    }

    /// The requests still waiting; broken, too long or expired entries are dropped.
    pub fn list() -> Vec<Request> {
        let p = path();
        if std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 512 * 1024 {
            return vec![];
        }
        let raw: Vec<serde_json::Value> = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let now = now_ms();
        raw.into_iter()
            .filter_map(|v| serde_json::from_value::<Request>(v).ok())
            .filter(|r| {
                r.id.len() <= 32
                    && r.id.chars().all(|c| c.is_ascii_hexdigit())
                    && now.saturating_sub(r.created_ms) < EXPIRY_MS
                    && parse_site(&r.site).is_ok_and(|s| s == r.site)
                    && r.summary.chars().count() <= 250
                    && r.text.chars().count() <= 30_000
                    && match r.kind.as_str() {
                        "issue" => check_project(&r.project).is_ok() && !r.summary.is_empty(),
                        "comment" => check_issue(&r.key).is_ok() && !r.text.is_empty(),
                        _ => false,
                    }
            })
            .collect()
    }

    fn save(list: &[Request]) -> Result<(), String> {
        let text = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
        crate::change::write_private(&path(), &text, false)
    }

    /// Files a request (the text already cleaned). The same write waiting already is not filed twice.
    pub fn add(mut r: Request) -> Result<Request, String> {
        let mut list = list();
        if let Some(same) = list.iter().find(|x| x.kind == r.kind && x.site == r.site && x.project == r.project && x.key == r.key && x.summary == r.summary && x.text == r.text) {
            return Ok(same.clone());
        }
        if list.len() >= MAX {
            return Err(format!("{MAX} Jira requests are already waiting for the user; nothing more is filed until they decide."));
        }
        r.id = crate::change::random_id(4);
        r.created_ms = now_ms();
        list.push(r.clone());
        save(&list)?;
        Ok(r)
    }

    /// Takes request `id` out of the list (approved or declined): it can be decided only once.
    pub fn take(id: &str) -> Option<Request> {
        let list = list();
        let found = list.iter().find(|r| r.id == id).cloned()?;
        let rest: Vec<Request> = list.into_iter().filter(|r| r.id != id).collect();
        save(&rest).ok()?;
        Some(found)
    }

    /// Sends an approved request: still connected to the same site, then one write. `channel` is
    /// for the activity log (`dashboard`, `terminal`): site and key only, never the text.
    pub async fn send(r: &Request, channel: &str) -> Result<(String, String), String> {
        let j = Jira::connected().map_err(|e| e.to_string())?;
        if j.site() != r.site {
            return Err(format!("Jira is now connected to {}, not {}. Nothing was sent.", j.site(), r.site));
        }
        // Security Low 3: an answer that never came may hide a write that arrived.
        let place = if r.kind == "issue" { r.project.clone() } else { r.key.clone() };
        let check = j.browse(&place);
        let fail = |e: JiraError| {
            if e == JiraError::Refused {
                mark(true, None);
            }
            if matches!(e, JiraError::Network | JiraError::Other(500..=599)) {
                crate::change::audit("jira", None, Some(&format!("{} {place}", r.site)), channel, "no-answer");
                return format!("Jira did not answer in time, so I cannot tell whether it got this. Check {place} in Jira before you ask again: {check}");
            }
            format!("{e} Nothing was sent.")
        };
        let key = if r.kind == "issue" {
            let ty = j.bug_type(&r.project).await.map_err(fail)?;
            let key = j.create_issue(&r.project, &ty, &r.summary, &r.text).await.map_err(fail)?;
            crate::change::audit("jira", None, Some(&format!("{} {key}", r.site)), channel, "created");
            key
        } else {
            j.comment(&r.key, &r.text).await.map_err(fail)?;
            crate::change::audit("jira", None, Some(&format!("{} {}", r.site, r.key)), channel, "commented");
            r.key.clone()
        };
        mark(false, None);
        let link = j.browse(&key);
        Ok((if r.kind == "issue" { format!("Created {key}.") } else { format!("Comment added to {key}.") }, link))
    }
}

/// `KAN`: a Jira project key.
pub fn check_project(key: &str) -> Result<(), JiraError> {
    let ok = (2..=10).contains(&key.len()) && key.starts_with(|c: char| c.is_ascii_uppercase()) && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if ok { Ok(()) } else { Err(JiraError::NotFound(format!("project {key}"))) }
}

/// `KAN-12`: a Jira issue key.
pub fn check_issue(key: &str) -> Result<(), JiraError> {
    let ok = key.split_once('-').is_some_and(|(p, n)| check_project(p).is_ok() && !n.is_empty() && n.len() <= 9 && n.chars().all(|c| c.is_ascii_digit()));
    if ok { Ok(()) } else { Err(JiraError::NotFound(key.to_owned())) }
}

/// Jira's own reason for refusing content, short and plain (never the raw response).
fn rejection(text: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let mut parts: Vec<String> = v["errorMessages"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_owned)).collect();
    if let Some(m) = v["errors"].as_object() {
        parts.extend(m.iter().map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or(""))));
    }
    let s = plain(&parts.join("; "), 300);
    if s.is_empty() { "a required field is missing or not allowed".into() } else { s }
}

/// Text fastpilot sends to Jira, cleaned (design 03, "cleaned text"): the project's secrets as
/// `***`, no email addresses, no links to other sites (links to `site` stay), no @mentions or
/// `[~account]` mentions, no control characters; one line for a summary. What is shown on a card
/// is exactly what is sent.
pub fn clean(text: &str, site: &str, one_line: bool, max: usize) -> String {
    let text = no_escapes(&crate::dsl::redact(text));
    let mut out: Vec<String> = Vec::new();
    for word in text.split_inclusive(char::is_whitespace) {
        let (w, tail) = word.split_at(word.trim_end().len());
        let bare = w.trim_matches(|c: char| "()[]<>\"',.;:!?".contains(c));
        let lower = bare.to_ascii_lowercase();
        let cleaned = if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.") {
            let host = lower.trim_start_matches("http://").trim_start_matches("https://").split(['/', '?', '#', ':']).next().unwrap_or("").to_owned();
            if host == site { w.to_owned() } else { w.replace(bare, "[link removed]") }
        } else if bare.contains('@') && bare.split('@').nth(1).is_some_and(|d| d.contains('.')) && !bare.starts_with('@') {
            w.replace(bare, "[email removed]")
        } else if let Some(name) = bare.strip_prefix('@') {
            w.replacen(bare, name, 1)
        } else if bare.starts_with('~') || lower.starts_with("accountid:") {
            String::new()
        } else {
            w.to_owned()
        };
        out.push(format!("{cleaned}{tail}"));
    }
    let joined = out.concat();
    let joined: String = if one_line { joined.split_whitespace().collect::<Vec<_>>().join(" ") } else { joined };
    plain(&joined, max)
}

/// Without terminal escape sequences (`ESC [ … letter`), so nothing is left of them but text.
fn no_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Text without control characters (newlines kept), at most `max` characters.
pub fn plain(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n').take(max).collect::<String>().trim().to_owned()
}

/// Plain text of an Atlassian Document Format value (descriptions, comments).
pub fn adf_text(v: &serde_json::Value) -> String {
    fn walk(v: &serde_json::Value, out: &mut String) {
        match v {
            serde_json::Value::String(s) => out.push_str(s),
            serde_json::Value::Object(o) => {
                if let Some(t) = o.get("text").and_then(|t| t.as_str()) {
                    out.push_str(t);
                }
                if o.get("type").and_then(|t| t.as_str()) == Some("hardBreak") {
                    out.push('\n');
                }
                if let Some(c) = o.get("content") {
                    walk(c, out);
                }
                if matches!(o.get("type").and_then(|t| t.as_str()), Some("paragraph" | "heading" | "listItem" | "codeBlock")) {
                    out.push('\n');
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = String::new();
    walk(v, &mut out);
    out
}

/// Plain text as an Atlassian Document: one paragraph per line, text only (no links or mentions).
pub fn adf(text: &str) -> serde_json::Value {
    let paragraphs: Vec<serde_json::Value> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::json!({ "type": "paragraph", "content": [{ "type": "text", "text": l }] }))
        .collect();
    serde_json::json!({ "type": "doc", "version": 1, "content": paragraphs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_jira_cloud_addresses() {
        for (input, want) in [
            ("yourteam.atlassian.net", "yourteam.atlassian.net"),
            ("yourteam", "yourteam.atlassian.net"),
            ("  https://YourTeam.atlassian.net/jira/software/projects/KAN/boards/1 ", "yourteam.atlassian.net"),
            ("yogisworkspace-42510190.atlassian.net", "yogisworkspace-42510190.atlassian.net"),
        ] {
            assert_eq!(parse_site(input).as_deref(), Ok(want), "{input}");
        }
        assert!(check_issue("KAN-12").is_ok() && check_issue("kan-12").is_err() && check_issue("KAN-12/../x").is_err() && check_issue("KAN-").is_err());
        assert!(check_project("KAN").is_ok() && check_project("K").is_err() && check_project("KAN\" OR").is_err());
        // What fastpilot sends is cleaned: no emails, no links elsewhere, no mentions.
        let c = clean("Total is wrong @alex see https://evil.example/x and https://yourteam.atlassian.net/browse/KAN-1 mail bob@corp.com [~accountid:557058]\u{1b}[2J", "yourteam.atlassian.net", false, 500);
        assert_eq!(c, "Total is wrong alex see [link removed] and https://yourteam.atlassian.net/browse/KAN-1 mail [email removed]");
        assert_eq!(clean("line one\nline two", "x.atlassian.net", true, 9), "line one");
        for bad in ["jira.mycompany.com", "evil.com/x.atlassian.net", "atlassian.net", "a.b.atlassian.net", "https://user@evil.com", "", "-x.atlassian.net", "x.atlassian.net.evil.com"] {
            assert!(parse_site(bad).is_err(), "{bad}");
        }
    }
}
