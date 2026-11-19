//! Web driver: Chrome DevTools Protocol over one persistent WebSocket (no Node, no chromedriver).
//! Launch mode runs one Chrome process; every session is its own `BrowserContext` + page.

mod browser;
mod cdp;
mod page;
mod relay;

pub use browser::{Browser, LaunchOptions, chrome_endpoint, find_chrome, http_get};
pub use page::WebPage;
pub use relay::{DEFAULT_PORT as RELAY_PORT, extension_page, native_host as relay_native_host};

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

/// Protocol errors Chrome returns while a page is mid-navigation; the step can safely retry.
const TRANSIENT: &[&str] = &[
    "Cannot find context",
    "Execution context was destroyed",
    "Inspected target navigated or closed",
    "Could not find node",
    "No node with given id",
    "Node with given id does not belong to the document",
    "Cannot find default execution context",
    "Frame with the given id was not found",
];

impl From<Error> for fastpilot_core::Error {
    fn from(e: Error) -> Self {
        match &e {
            Error::Protocol { message, .. } if TRANSIENT.iter().any(|t| message.contains(t)) => {
                Self::Transient(e.to_string())
            }
            _ => Self::Driver(e.to_string()),
        }
    }
}
