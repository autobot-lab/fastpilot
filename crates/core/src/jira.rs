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
        for bad in ["jira.mycompany.com", "evil.com/x.atlassian.net", "atlassian.net", "a.b.atlassian.net", "https://user@evil.com", "", "-x.atlassian.net", "x.atlassian.net.evil.com"] {
            assert!(parse_site(bad).is_err(), "{bad}");
        }
    }
}
