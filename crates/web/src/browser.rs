use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::cdp::Cdp;
use crate::{Error, WebPage};

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub headless: bool,
    /// Defaults to `FASTPILOT_CHROME`, then well-known install locations.
    pub executable: Option<PathBuf>,
}

/// One Chrome process and one CDP socket shared by every web session.
pub struct Browser {
    pub(crate) cdp: Arc<Cdp>,
    child: Child,
    profile: PathBuf,
}

impl Browser {
    pub async fn launch(opts: LaunchOptions) -> Result<Arc<Self>, Error> {
        let exe = opts.executable.or_else(find_chrome).ok_or(Error::ChromeNotFound)?;
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
            "--disable-sync",
            "--disable-features=Translate,MediaRouter,OptimizationHints",
            "--password-store=basic",
            "--use-mock-keychain",
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
        let ws = tokio::time::timeout(Duration::from_secs(20), async {
            while let Ok(Some(l)) = lines.next_line().await {
                if let Some(url) = l.strip_prefix("DevTools listening on ") {
                    return Some(url.trim().to_owned());
                }
            }
            None
        })
        .await
        .ok()
        .flatten()
        .ok_or(Error::Launch("Chrome did not print its DevTools URL".into()))?;
        // Keep draining stderr, or Chrome blocks once the pipe fills up.
        tokio::spawn(async move {
            while let Ok(Some(l)) = lines.next_line().await {
                tracing::trace!(target: "chrome", "{l}");
            }
        });
        let cdp = Cdp::connect(&ws).await?;
        Ok(Arc::new(Self { cdp, child, profile }))
    }

    /// New isolated session: fresh `BrowserContext` (cookies, storage, cache) + one page.
    pub async fn new_page(self: &Arc<Self>) -> Result<WebPage, Error> {
        WebPage::create(self.clone()).await
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

fn find_chrome() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FASTPILOT_CHROME") {
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
