//! fastpilot core: the platform-agnostic driver contract, selectors, snapshots,
//! the flow DSL and the flow engine. Drivers (web, android, ios) live in their own crates.

mod dsl;
mod engine;
mod error;
mod selector;
mod snapshot;

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use dsl::{Branch, Flow, FlowError, FlowStep, Step, check_flow, parse_flow};
pub use engine::{FlowReport, Launcher, RunOptions, SessionRegistry, StepRecord, run_flow};
pub use error::{Error, Result};
pub use selector::Selector;
pub use snapshot::{Entry, Intent, Node, RefRegistry, Snapshot, diff};

pub type Value = serde_json::Value;
pub type SessionId = String;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Web,
    Android,
    Ios,
}

impl Platform {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "web" => Some(Self::Web),
            "android" => Some(Self::Android),
            "ios" => Some(Self::Ios),
            _ => None,
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Web => "web",
            Self::Android => "android",
            Self::Ios => "ios",
        })
    }
}

/// Shared by every platform (PRD §7.1).
#[async_trait]
pub trait Driver: Send + Sync {
    async fn open(&self, target: &str) -> Result<()>; // URL | package/activity | bundleId
    async fn snapshot(&self) -> Result<Snapshot>; // normalized a11y tree + refs
    async fn tap(&self, sel: &Selector) -> Result<()>;
    async fn type_text(&self, sel: &Selector, text: &str) -> Result<()>;
    async fn scroll(&self, dir: Direction, sel: Option<&Selector>) -> Result<()>;
    async fn swipe(&self, from: Point, to: Point) -> Result<()>;
    async fn back(&self) -> Result<()>;
    async fn screenshot(&self) -> Result<Vec<u8>>; // JPEG
    async fn wait_idle(&self, timeout: Duration) -> Result<()>;
    async fn eval_script(&self, js: &str) -> Result<Value>; // web/WebView only; others Unsupported
}
