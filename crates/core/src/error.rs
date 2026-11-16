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
    /// A guard rail refused the step (FASTPILOT_ALLOWED_DOMAINS, FASTPILOT_CONFIRM_ACTIONS).
    #[error("{0}")]
    Blocked(String),
}

impl Error {
    /// Same failure, message prefixed (e.g. which step of a `repeat` block failed).
    pub fn context(self, prefix: &str) -> Self {
        let p = |m: String| format!("{prefix}: {m}");
        match self {
            Self::NotFound(m) => Self::NotFound(p(m)),
            Self::Ambiguous { selector, candidates } => Self::Ambiguous { selector: p(selector), candidates },
            Self::NotActionable(m) => Self::NotActionable(p(m)),
            Self::Timeout(m) => Self::Timeout(p(m)),
            Self::Transient(m) => Self::Transient(p(m)),
            Self::Unsupported(m) => Self::Unsupported(p(m)),
            Self::Script(m) => Self::Script(p(m)),
            Self::Assertion(m) => Self::Assertion(p(m)),
            Self::Session(m) => Self::Session(p(m)),
            Self::Flow(m) => Self::Flow(p(m)),
            Self::Driver(m) => Self::Driver(p(m)),
            Self::AppCrashed(m) => Self::AppCrashed(p(m)),
            Self::AppNotResponding(m) => Self::AppNotResponding(p(m)),
            Self::Blocked(m) => Self::Blocked(p(m)),
        }
    }

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
            Self::Blocked(_) => "blocked",
            Self::Driver(_) => "driver_error",
            Self::AppCrashed(_) => "app_crashed",
            Self::AppNotResponding(_) => "app_not_responding",
        }
    }

    /// Who most likely has to act (failure taxonomy): `product` (the app is not as the test
    /// expects: a bug, a changed UI, a crash), `infra` (browser/device/transport), `agent` (the
    /// flow the AI wrote is wrong or uses something unsupported). Flakiness is judged over
    /// runs, not from one error (see fastpilot-memory).
    pub fn category(&self) -> &'static str {
        match self {
            Self::NotFound(_)
            | Self::Ambiguous { .. }
            | Self::NotActionable(_)
            | Self::Timeout(_)
            | Self::Script(_)
            | Self::Assertion(_)
            | Self::AppCrashed(_)
            | Self::AppNotResponding(_) => "product",
            Self::Transient(_) | Self::Driver(_) | Self::Session(_) => "infra",
            Self::Flow(_) | Self::Unsupported(_) | Self::Blocked(_) => "agent",
        }
    }

    /// Only lookup/actionability failures are retried; they happen before any input is sent,
    /// so retrying can never double-submit.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::NotFound(_) | Self::NotActionable(_) | Self::Transient(_))
    }
}
