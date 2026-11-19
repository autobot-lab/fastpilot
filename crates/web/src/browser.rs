use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::cdp::Cdp;
use crate::{Error, WebPage};

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub headless: bool,
    /// Defaults to `FASTPILOT_CHROME`, then well-known install locations.
    pub executable: Option<PathBuf>,
}

/// One Chrome process (launched by us, or one we attached to) and one CDP socket shared by
/// every web session on it.
pub struct Browser {
    pub(crate) cdp: Arc<Cdp>,
    child: Option<Child>,
    profile: Option<PathBuf>,
}

impl Browser {
    /// fastpilot's own Chrome, or the remote one in `FASTPILOT_CHROME_ENDPOINT` (config
    /// `chrome_endpoint`): either way every session gets its own `BrowserContext`.
    pub async fn launch(opts: LaunchOptions) -> Result<Arc<Self>, Error> {
        if opts.executable.is_none()
            && let Some(endpoint) = chrome_endpoint()
        {
            return Self::connect(Some(&endpoint)).await.map_err(|e| Error::Launch(format!("remote Chrome at {endpoint} (FASTPILOT_CHROME_ENDPOINT): {e}")));
        }
        Self::launch_local(opts).await
    }

    /// Always a Chrome process of our own (`doctor` checks this one separately).
    pub async fn launch_local(opts: LaunchOptions) -> Result<Arc<Self>, Error> {
        let exe = opts.executable.or_else(find_chrome).ok_or(Error::ChromeNotFound)?;
        reap_orphans();
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let profile = std::env::temp_dir().join(format!("fastpilot-{}-{nanos}", std::process::id()));
        let mut cmd = Command::new(&exe);
        cmd.args([
            "--remote-debugging-port=0",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-background-timer-throttling",
            "--disable-backgrounding-occluded-windows",
            "--disable-renderer-backgrounding",
            "--disable-hang-monitor",
            "--disable-ipc-flooding-protection",
            "--disable-prompt-on-repost",
            "--disable-sync",
            // No bfcache: every back/forward fires a real load we can wait on.
            "--disable-features=Translate,MediaRouter,OptimizationHints,BackForwardCache",
            "--password-store=basic",
            "--use-mock-keychain",
            "--window-size=1280,800",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
        if opts.headless {
            cmd.arg("--headless=new");
        }
        cmd.arg("about:blank");
        tracing::debug!(exe = %exe.display(), headless = opts.headless, "launching chrome");
        let mut child = cmd.spawn().map_err(|e| Error::Launch(format!("{}: {e}", exe.display())))?;
        let stderr = child.stderr.take().ok_or(Error::Launch("stderr not captured".into()))?;
        let mut lines = BufReader::new(stderr).lines();
        let mut tail = Vec::new();
        let ws = tokio::time::timeout(Duration::from_secs(20), async {
            while let Ok(Some(l)) = lines.next_line().await {
                if let Some(url) = l.strip_prefix("DevTools listening on ") {
                    return Some(url.trim().to_owned());
                }
                tail.push(l);
            }
            None
        })
        .await
        .ok()
        .flatten()
        .ok_or_else(|| Error::Launch(format!("Chrome did not print its DevTools URL: {}", tail.join(" | "))))?;
        // Keep draining stderr (lossy: Chrome may print non-UTF-8), or Chrome blocks once the pipe fills.
        let mut rest = lines.into_inner();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            while matches!(rest.read(&mut buf).await, Ok(n) if n > 0) {}
        });
        let cdp = Cdp::connect(&ws).await?;
        // Popups (window.open) of our pages are attached as they appear.
        let _ = cdp.call(None, "Target.setDiscoverTargets", json!({ "discover": true })).await;
        Ok(Arc::new(Self { cdp, child: Some(child), profile: Some(profile) }))
    }

    /// Attach to a running Chrome (PRD §7.3 attach mode). `endpoint` is a `ws://` browser URL or
    /// `http://host:port`; `None` reads `DevToolsActivePort` of the user's default profile, which
    /// Chrome M144+ writes after remote debugging is enabled at chrome://inspect/#remote-debugging
    /// (Chrome asks the user to allow each connection).
    pub async fn connect(endpoint: Option<&str>) -> Result<Arc<Self>, Error> {
        let ws = match endpoint {
            Some(e) if e.starts_with("ws://") || e.starts_with("wss://") => e.to_owned(),
            Some(e) => {
                let host = e.trim_start_matches("http://").trim_end_matches('/');
                let v: Value = serde_json::from_str(&http_get(host, "/json/version").await?)
                    .map_err(|_| Error::Unexpected("/json/version"))?;
                let ws = v["webSocketDebuggerUrl"].as_str().ok_or(Error::Unexpected("webSocketDebuggerUrl"))?;
                // Chrome names itself as it sees itself (its own port behind a tunnel): use the
                // address we reached it at.
                with_host(ws, host)
            }
            None => active_port_url().ok_or(Error::Launch(
                "no DevToolsActivePort found; enable chrome://inspect/#remote-debugging in Chrome (M144+) or pass a cdp endpoint".into(),
            ))?,
        };
        let cdp = Cdp::connect(&ws).await?;
        let _ = cdp.call(None, "Target.setDiscoverTargets", json!({ "discover": true })).await;
        Ok(Arc::new(Self { cdp, child: None, profile: None }))
    }

    /// A page-level endpoint (e.g. an Android WebView's `/json/list` entry): no Target domain.
    pub async fn connect_page(ws_url: &str, target_id: &str) -> Result<WebPage, Error> {
        let browser = Arc::new(Self { cdp: Cdp::connect(ws_url).await?, child: None, profile: None });
        WebPage::init(browser, None, target_id.to_owned(), None, None).await
    }

    pub fn is_closed(&self) -> bool {
        self.cdp.is_closed()
    }

    /// New isolated session: fresh `BrowserContext` (cookies, storage, cache) + one page.
    pub async fn new_page(self: &Arc<Self>) -> Result<WebPage, Error> {
        WebPage::create(self.clone()).await
    }

    /// Session on an existing tab of an attached browser (the first normal page), or a new tab
    /// in its default profile when none is open.
    pub async fn attach_page(self: &Arc<Self>) -> Result<WebPage, Error> {
        let targets = self.cdp.call(None, "Target.getTargets", json!({})).await?;
        let existing = targets["targetInfos"].as_array().into_iter().flatten().find(|t| {
            t["type"] == "page" && !t["url"].as_str().unwrap_or("").starts_with("chrome")
        });
        let target = match existing.and_then(|t| t["targetId"].as_str()) {
            Some(id) => id.to_owned(),
            None => {
                let t = self.cdp.call(None, "Target.createTarget", json!({ "url": "about:blank" })).await?;
                t["targetId"].as_str().ok_or(Error::Unexpected("targetId"))?.to_owned()
            }
        };
        let a = self.cdp.call(None, "Target.attachToTarget", json!({ "targetId": target, "flatten": true })).await?;
        let session = a["sessionId"].as_str().ok_or(Error::Unexpected("sessionId"))?.to_owned();
        WebPage::init(self.clone(), Some(session), target, None, None).await
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
        if let Some(p) = &self.profile {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

/// Chrome processes left by a fastpilot that was killed (SIGKILL skips `kill_on_drop`):
/// profiles are `fastpilot-<pid>-<nanos>`; when `<pid>` is gone, kill its Chrome and delete it.
#[cfg(unix)]
fn reap_orphans() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix("fastpilot-") else { continue };
        let parts: Vec<&str> = rest.split('-').collect();
        let [pid, nanos] = parts.as_slice() else { continue };
        if !pid.bytes().all(|b| b.is_ascii_digit()) || !nanos.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let alive = std::process::Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !alive {
            let dir = e.path();
            tracing::debug!(dir = %dir.display(), "reaping orphaned chrome profile");
            // `--` ends the options: otherwise pkill parses the pattern itself as `--user-data-dir…`.
            let _ = std::process::Command::new("pkill").args(["-f", "--"]).arg(format!("--user-data-dir={}", dir.display())).status();
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(not(unix))]
fn reap_orphans() {}

/// Plain HTTP/1.1 GET against a local DevTools endpoint (`/json/version`, `/json/list`).
/// Reads by Content-Length: Chrome keeps the connection open despite `Connection: close`.
pub async fn http_get(host: &str, path: &str) -> Result<String, Error> {
    let fetch = async {
        let mut s = tokio::net::TcpStream::connect(host).await?;
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes()).await?;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = s.read(&mut chunk).await?;
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf);
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let len = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_owned()))
                    .and_then(|v| v.parse::<usize>().ok());
                if n == 0 || len.is_some_and(|l| body.len() >= l) {
                    return Ok::<_, std::io::Error>(body.to_owned());
                }
            } else if n == 0 {
                return Ok(String::new());
            }
        }
    };
    match tokio::time::timeout(Duration::from_secs(5), fetch).await {
        Ok(Ok(body)) => Ok(body),
        Ok(Err(e)) => Err(Error::WebSocket(format!("{host}{path}: {e}"))),
        Err(_) => Err(Error::CallTimeout(format!("GET {host}{path}"))),
    }
}

/// `FASTPILOT_CHROME_ENDPOINT` / config `chrome_endpoint`: a remote Chrome for every web
/// session (`ws://...` or `http://host:port`).
pub fn chrome_endpoint() -> Option<String> {
    fastpilot_core::config::var("FASTPILOT_CHROME_ENDPOINT").ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// `ws://a:1/devtools/browser/x` with `host` (`b:2`) in place of `a:1`.
fn with_host(ws: &str, host: &str) -> String {
    match ws.split_once("://").and_then(|(scheme, rest)| rest.split_once('/').map(|(_, path)| (scheme, path))) {
        Some((scheme, path)) => format!("{scheme}://{host}/{path}"),
        None => ws.to_owned(),
    }
}

fn active_port_url() -> Option<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let candidates = [
        home.as_ref().map(|h| h.join("Library/Application Support/Google/Chrome/DevToolsActivePort")),
        home.as_ref().map(|h| h.join(".config/google-chrome/DevToolsActivePort")),
        std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Google/Chrome/User Data/DevToolsActivePort")),
    ];
    let text = candidates.into_iter().flatten().find_map(|p| std::fs::read_to_string(p).ok())?;
    let mut lines = text.lines();
    let (port, path) = (lines.next()?.trim(), lines.next()?.trim());
    Some(format!("ws://127.0.0.1:{port}{path}"))
}

/// The Chrome/Chromium fastpilot would launch (`chrome` setting, else well-known places, else PATH).
pub fn find_chrome() -> Option<PathBuf> {
    if let Some(p) = fastpilot_core::config::var_os("FASTPILOT_CHROME") {
        return Some(PathBuf::from(p));
    }
    let fixed = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ];
    if let Some(p) = fixed.iter().map(Path::new).find(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    let names = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "chrome"];
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_takes_the_address_we_reached() {
        assert_eq!(with_host("ws://127.0.0.1:9222/devtools/browser/abc", "127.0.0.1:15222"), "ws://127.0.0.1:15222/devtools/browser/abc");
        assert_eq!(with_host("wss://h/devtools/browser/x", "a:1"), "wss://a:1/devtools/browser/x");
    }

    /// #9: a Chrome left by a killed fastpilot (its `--user-data-dir` points at a profile whose
    /// pid is gone) must be killed, not only have its profile deleted.
    #[test]
    fn reap_orphans_kills_chrome_of_dead_owner() {
        // A pid that is certainly gone: a child we already waited for.
        let mut gone = std::process::Command::new("true").spawn().unwrap();
        let dead_pid = gone.id();
        gone.wait().unwrap();
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("fastpilot-{dead_pid}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        // Stand-in for the orphaned Chrome: same `--user-data-dir=` flag in its command line.
        let mut chrome = std::process::Command::new("sh")
            .args(["-c", "while :; do sleep 1; done", "sh"])
            .arg(format!("--user-data-dir={}", dir.display()))
            .spawn()
            .unwrap();

        reap_orphans();

        let mut exited = None;
        for _ in 0..50 {
            exited = chrome.try_wait().unwrap();
            if exited.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if exited.is_none() {
            let _ = chrome.kill();
            let _ = chrome.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(exited.is_some(), "orphaned chrome still running after reap_orphans");
        assert!(!dir.exists(), "orphaned profile not deleted");
    }
}
