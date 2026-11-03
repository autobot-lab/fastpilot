pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    NotFound(String),
    #[error("{selector} matches {} nodes", candidates.len())]
    Ambiguous {
        selector: String,
        candidates: Vec<String>,
    },
    #[error("{0}")]
    NotActionable(String),
    #[error("{0}")]
    Timeout(String),
    /// The page/screen was mid-transition (navigation, context teardown); safe to retry.
    #[error("{0}")]
    Transient(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Script(String),
    #[error("{0}")]
    Assertion(String),
    #[error("{0}")]
    Session(String),
    #[error("{0}")]
    Flow(String),
    /// Transport/protocol failure inside a driver (CDP, adb, WDA).
    #[error("{0}")]
    Driver(String),
    /// The app under test died (crash dialog, process gone).
    #[error("{0}")]
    AppCrashed(String),
    /// The app's main thread stopped responding (Android ANR dialog).
    #[error("{0}")]
    AppNotResponding(String),
}

impl Error {
    /// Structured reason returned to the AI (PRD §11.2).
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::Ambiguous { .. } => "ambiguous",
            Self::NotActionable(_) => "not_actionable",
            Self::Timeout(_) => "timeout",
            Self::Transient(_) => "transient",
            Self::Unsupported(_) => "unsupported",
            Self::Script(_) => "script_error",
            Self::Assertion(_) => "assertion_failed",
            Self::Session(_) => "session",
            Self::Flow(_) => "invalid_flow",
            Self::Driver(_) => "driver_error",
            Self::AppCrashed(_) => "app_crashed",
            Self::AppNotResponding(_) => "app_not_responding",
        }
    }

    /// Who most likely has to act (Phase 3 taxonomy): `app` (the screen is not as the test
    /// expects: product bug or changed UI), `infra` (browser/device/transport), `flow` (the
    /// flow itself is wrong or uses something unsupported).
    pub fn category(&self) -> &'static str {
        match self {
            Self::NotFound(_)
            | Self::Ambiguous { .. }
            | Self::NotActionable(_)
            | Self::Timeout(_)
            | Self::Script(_)
            | Self::Assertion(_)
            | Self::AppCrashed(_)
            | Self::AppNotResponding(_) => "app",
            Self::Transient(_) | Self::Driver(_) | Self::Session(_) => "infra",
            Self::Flow(_) | Self::Unsupported(_) => "flow",
        }
    }

    /// Only lookup/actionability failures are retried; they happen before any input is sent,
    /// so retrying can never double-submit.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::NotFound(_) | Self::NotActionable(_) | Self::Transient(_))
    }
}
