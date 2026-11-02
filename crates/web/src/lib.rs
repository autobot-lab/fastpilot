//! Web driver: Chrome DevTools Protocol over one persistent WebSocket (no Node, no chromedriver).
//! Launch mode runs one Chrome process; every session is its own `BrowserContext` + page.

mod browser;
mod cdp;
mod page;

pub use browser::{Browser, LaunchOptions};
pub use page::WebPage;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Chrome not found; install Google Chrome/Chromium or set FASTPILOT_CHROME to its executable")]
    ChromeNotFound,
    #[error("failed to launch Chrome: {0}")]
    Launch(String),
    #[error("CDP websocket: {0}")]
    WebSocket(String),
    #[error("CDP connection closed (did Chrome exit?)")]
    Closed,
    #[error("CDP {method}: {message}")]
    Protocol { method: String, message: String },
    #[error("CDP {0} timed out")]
    CallTimeout(String),
    #[error("unexpected CDP response for {0}")]
    Unexpected(&'static str),
}

impl From<Error> for fastpilot_core::Error {
    fn from(e: Error) -> Self {
        Self::Driver(e.to_string())
    }
}
