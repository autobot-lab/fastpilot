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
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Script(String),
    #[error("{0}")]
    Session(String),
    #[error("{0}")]
    Flow(String),
    /// Transport/protocol failure inside a driver (CDP, adb, WDA).
    #[error("{0}")]
    Driver(String),
}

impl Error {
    /// Structured reason returned to the AI (PRD §11.2).
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::Ambiguous { .. } => "ambiguous",
            Self::NotActionable(_) => "not_actionable",
            Self::Timeout(_) => "timeout",
            Self::Unsupported(_) => "unsupported",
            Self::Script(_) => "script_error",
            Self::Session(_) => "session",
            Self::Flow(_) => "invalid_flow",
            Self::Driver(_) => "driver_error",
        }
    }

    /// Only lookup/actionability failures are retried; they happen before any input is sent,
    /// so retrying can never double-submit.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::NotFound(_) | Self::NotActionable(_))
    }
}
