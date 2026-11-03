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

pub use dsl::{
    Branch, Condition, Flow, FlowError, FlowStep, Random, Step, check_flow, check_flow_file, flow_to_yaml, parse_flow,
    parse_flow_file, redact,
};
pub use engine::{AssertKind, FlowReport, Launcher, RunOptions, SessionRegistry, StepRecord, assert, run_flow, sessions_used};
pub use error::{Error, Result};
pub use selector::Selector;
pub use snapshot::{Entry, Intent, Node, Rect, RefRegistry, Snapshot, diff};

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

    // Beyond §7.1: optional capabilities; drivers that lack them report `unsupported`.
    /// Key name: Enter, Tab, Escape, Backspace, Delete, ArrowUp/Down/Left/Right, Home, End, PageUp, PageDown, Space.
    async fn press_key(&self, key: &str) -> Result<()> {
        Err(Error::Unsupported(format!("pressKey {key} is not supported by this driver")))
    }
    async fn select_option(&self, _sel: &Selector, _option: &str) -> Result<()> {
        Err(Error::Unsupported("selectOption is not supported by this driver".into()))
    }
    async fn upload(&self, _sel: &Selector, _files: &[String]) -> Result<()> {
        Err(Error::Unsupported("uploadFile is not supported by this driver".into()))
    }
    /// Screenshot with numbered boxes (Set-of-Mark) drawn at `marks` (ref, box); drivers that
    /// cannot draw return a plain screenshot.
    async fn screenshot_annotated(&self, _marks: &[(String, Rect)]) -> Result<Vec<u8>> {
        self.screenshot().await
    }
    /// Console errors, failed requests and similar, newest last (attached to failures).
    async fn diagnostics(&self) -> Vec<String> {
        Vec::new()
    }
    /// Maestro `launchApp`: package / package/activity / bundleId.
    async fn launch_app(&self, app: &str) -> Result<()> {
        self.open(app).await
    }
    /// Which platform this driver drives (for `when: { platform: ... }`).
    fn platform(&self) -> Platform {
        Platform::Web
    }
    async fn long_press(&self, _sel: &Selector) -> Result<()> {
        Err(Error::Unsupported("longPressOn is not supported by this driver".into()))
    }
    async fn double_tap(&self, _sel: &Selector) -> Result<()> {
        Err(Error::Unsupported("doubleTapOn is not supported by this driver".into()))
    }
    async fn hide_keyboard(&self) -> Result<()> {
        Err(Error::Unsupported("hideKeyboard is not supported by this driver".into()))
    }
    /// Delete `count` characters before the cursor of the focused field (`None`: all of them).
    async fn erase_text(&self, _count: Option<usize>) -> Result<()> {
        Err(Error::Unsupported("eraseText is not supported by this driver".into()))
    }
    /// Stop the app (`None`: the app under test). Mobile only.
    async fn stop_app(&self, _app: Option<&str>) -> Result<()> {
        Err(Error::Unsupported("stopApp is mobile-only".into()))
    }
    /// Wipe the app's data (web: cookies and storage of the session's origin).
    async fn clear_state(&self, _app: Option<&str>) -> Result<()> {
        Err(Error::Unsupported("clearState is not supported by this driver".into()))
    }
    /// Visible area in snapshot coordinates, when the snapshot includes off-screen nodes (web).
    async fn viewport(&self) -> Option<Rect> {
        None
    }
    async fn clear_keychain(&self) -> Result<()> {
        Err(Error::Unsupported("clearKeychain is iOS-only".into()))
    }
}
