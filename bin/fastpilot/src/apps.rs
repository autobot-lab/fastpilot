//! TEMPORARY on the S8 branch: the read side of FastDev's E6 app contract (channel, 23 Nov 2026),
//! so "Run on all platforms" can be built before E6a merges. E6a's own `apps.rs` replaces this file
//! at the rebase; nothing here may grow beyond the contract.
//!
//! `~/fastpilot/apps.json`: `{ "current": "<id>", "apps": [ { "id", "name", "targets": { "web",
//! "android", "ios" }, "repo", "created_ms" } ] }`. One app is one entry; its web, Android and
//! iPhone versions are the keys of `targets`. An app's data is in `~/fastpilot/apps/<id>/`.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Targets {
    pub web: Option<String>,
    pub android: Option<String>,
    pub ios: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub targets: Targets,
    #[serde(default)]
    pub repo: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct Store {
    current: Option<String>,
    #[serde(default)]
    apps: Vec<App>,
}

/// `~/fastpilot`: the user's one workspace.
pub fn root() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("fastpilot")
}

fn store() -> Store {
    std::fs::read_to_string(root().join("apps.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

impl App {
    /// This app's folder: tests, memory, map.
    pub fn dir(&self) -> PathBuf {
        root().join("apps").join(&self.id)
    }
}

pub fn current() -> Option<App> {
    let s = store();
    let id = s.current?;
    s.apps.into_iter().find(|a| a.id == id)
}
