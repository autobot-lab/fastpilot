//! D10d: agents run side by side, and a device runs one test at a time. Every fastpilot process
//! (each agent's Chat, a schedule, Claude Code in a terminal) claims what a session needs before it
//! starts, in files next to the Live folder:
//!
//! - a device (an Android emulator or phone, an iPhone simulator or phone) through an exclusive
//!   lock file, so two sessions never share it. Before this, a second session starting on a device
//!   force-stopped the first one's agent (the F7 "early eof").
//! - one of `max_parallel` slots (default 3) per process that runs a test, web included: an
//!   agent's Chat or a scheduled run is one test, however many sessions it opens, so it never
//!   waits for itself.
//!
//! A session that cannot have them yet waits in the queue (shown in Live with the reason) and
//! starts by itself when they are free; after `FASTPILOT_QUEUE_TIMEOUT` seconds (default 1800) it
//! gives up with the reason. Entries of processes that are gone are dropped. The slot count can be
//! one over the limit when two processes take the last slot at the same moment; the device lock
//! cannot.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{Error, Platform, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub pid: u32,
    pub session: String,
    #[serde(default)]
    pub agent: Option<String>,
    /// The agent's name as people know it (`FASTPILOT_AGENT_NAME`), for the queue's reasons.
    #[serde(default)]
    pub agent_name: Option<String>,
    pub platform: String,
    /// The device it needs (`android:emulator-5556`), `None` for web.
    #[serde(default)]
    pub device: Option<String>,
    pub since_ms: u64,
    /// Waiting in the queue (else running).
    pub waiting: bool,
    #[serde(default)]
    pub reason: String,
}

pub fn dir() -> PathBuf {
    if cfg!(test) {
        // This crate's own tests never touch the claims of the fastpilot on this computer.
        return std::env::temp_dir().join(format!("fastpilot-claims-test-{}", std::process::id()));
    }
    crate::live::dir().with_file_name("claims")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn alive(pid: u32) -> bool {
    // SAFETY: kill with signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).take(80).collect()
}

/// The device a session on `platform` will use: the one named, else the configured default.
pub fn device_key(platform: Platform, device: Option<&str>) -> Option<String> {
    let conf = |k: &str| crate::config::var(k).ok().filter(|v| !v.trim().is_empty());
    match platform {
        Platform::Web => None,
        Platform::Android => Some(format!("android:{}", device.map(str::to_owned).or_else(|| conf("FASTPILOT_ANDROID_SERIAL")).or_else(|| conf("ANDROID_SERIAL")).unwrap_or_else(|| "default".into()))),
        Platform::Ios => Some(format!("ios:{}", device.map(str::to_owned).or_else(|| conf("FASTPILOT_IOS_UDID")).unwrap_or_else(|| "default".into()))),
    }
}

/// The most tests `max_parallel` allows at once: the settings check and the Live page's
/// "Tests at once" list use this one number (L062-12).
pub const MAX_PARALLEL: usize = 20;

/// How many sessions may run at once on this computer (`max_parallel`, 1–20, default 3).
pub fn max_parallel() -> usize {
    crate::config::var("FASTPILOT_MAX_PARALLEL").ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|n| (1..=MAX_PARALLEL).contains(n)).unwrap_or(3)
}

fn entry_path(pid: u32, session: &str) -> PathBuf {
    dir().join(format!("{pid}-{}.json", safe(session)))
}

fn lock_path(device: &str) -> PathBuf {
    dir().join(format!("device-{}.lock", safe(device)))
}

fn write(e: &Entry) {
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(entry_path(e.pid, &e.session), serde_json::to_string(e).unwrap_or_default());
}

/// Every claim and queue entry of a living process (gone ones are removed).
pub fn entries() -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir()) else { return vec![] };
    let mut out = Vec::new();
    for p in rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")) {
        let Some(e) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Entry>(&t).ok()) else { continue };
        if alive(e.pid) {
            out.push(e);
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
    out.sort_by_key(|e| e.since_ms);
    out
}

/// Whether another living process holds the device's lock (without taking it).
fn held_by_other(device: &str) -> bool {
    let held = std::fs::read_to_string(lock_path(device)).unwrap_or_default();
    let pid: u32 = held.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
    pid != 0 && pid != std::process::id() && alive(pid)
}

/// Take the device lock; a lock of a process that is gone is taken over.
fn lock_device(device: &str, session: &str) -> bool {
    let p = lock_path(device);
    let me = format!("{} {session}", std::process::id());
    for _ in 0..2 {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(mut f) => {
                use std::io::Write;
                let _ = f.write_all(me.as_bytes());
                return true;
            }
            Err(_) => {
                let held = std::fs::read_to_string(&p).unwrap_or_default();
                let pid: u32 = held.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
                if held == me || pid == std::process::id() {
                    return true;
                }
                if pid == 0 || !alive(pid) {
                    let _ = std::fs::remove_file(&p);
                    continue;
                }
                return false;
            }
        }
    }
    false
}

fn label(device: &str) -> String {
    match device.split_once(':') {
        Some(("android", "default")) => "the Android device".into(),
        Some(("ios", "default")) => "the iPhone simulator".into(),
        Some(("android", d)) if d.starts_with("emulator-") => "the Android emulator".into(),
        Some(("android", _)) => "the Android phone".into(),
        Some(("ios", _)) => "the iPhone".into(),
        _ => device.into(),
    }
}

/// Wait (in the queue) until `session` may start on `platform`, then hold its claim until
/// [`release`]. Sessions of this process may share a device with each other.
pub async fn acquire(session: &str, platform: Platform, device: Option<&str>) -> Result<()> {
    let me = std::process::id();
    let key = device_key(platform, device);
    let agent = std::env::var("FASTPILOT_AGENT").ok().filter(|a| !a.is_empty());
    let agent_name = std::env::var("FASTPILOT_AGENT_NAME").ok().filter(|a| !a.is_empty()).map(|n| n.chars().take(40).collect());
    let mut e = Entry { pid: me, session: session.to_owned(), agent, agent_name, platform: platform.to_string(), device: key.clone(), since_ms: now_ms(), waiting: true, reason: String::new() };
    let limit = Duration::from_secs(crate::config::var("FASTPILOT_QUEUE_TIMEOUT").ok().and_then(|v| v.parse().ok()).unwrap_or(1800));
    let started = std::time::Instant::now();
    loop {
        let all = entries();
        // A slot is per process: one that already runs a session has its slot.
        let has_slot = all.iter().any(|x| x.pid == me && !x.waiting && x.session != session);
        let mut pids: Vec<u32> = all.iter().filter(|x| !x.waiting && x.pid != me).map(|x| x.pid).collect();
        pids.sort_unstable();
        pids.dedup();
        let running = if has_slot { 0 } else { pids.len() };
        // First come, first served: an earlier waiter (for the same device, if any) goes first.
        let ahead = !has_slot && all.iter().any(|x| x.waiting && x.since_ms < e.since_ms && x.pid != me && (key.is_none() || x.device == key));
        let max = max_parallel();
        let device_busy = |k: &String| {
            let by = all.iter().find(|x| !x.waiting && x.device.as_deref() == Some(k.as_str())).and_then(|x| x.agent_name.clone().or_else(|| x.agent.clone()));
            match by {
                Some(a) => format!("{} is in use by {a}", label(k)),
                None => format!("{} is in use", label(k)),
            }
        };
        // The device first (the more useful reason), then the limit; the lock is taken last, so a
        // session waiting for a slot never holds a device.
        let reason = if let Some(k) = key.as_ref().filter(|k| held_by_other(k)) {
            Some(device_busy(k))
        } else if running >= max {
            Some(format!("parallel limit of {max} reached"))
        } else if ahead {
            Some("waiting for its turn".into())
        } else {
            key.as_ref().filter(|k| !lock_device(k, session)).map(device_busy)
        };
        match reason {
            None => {
                e.waiting = false;
                e.reason.clear();
                write(&e);
                return Ok(());
            }
            Some(r) => {
                if r != e.reason {
                    tracing::info!(session, reason = %r, "waiting in the queue");
                }
                e.reason = r;
                write(&e);
                if started.elapsed() >= limit {
                    let _ = std::fs::remove_file(entry_path(me, session));
                    let waited = if limit.as_secs() < 120 { format!("{} s", limit.as_secs()) } else { format!("{} min", limit.as_secs() / 60) };
                    return Err(Error::Session(format!("waited {waited} in the queue and gave up: {}", e.reason)));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

/// Give back what `session` of this process holds (or waits for).
pub fn release(session: &str) {
    let me = std::process::id();
    let p = entry_path(me, session);
    if let Some(e) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Entry>(&t).ok())
        && let Some(k) = e.device
    {
        // Other sessions of this process on the same device keep it.
        let others = entries().iter().any(|x| x.pid == me && x.session != session && x.device.as_deref() == Some(k.as_str()) && !x.waiting);
        let lock = lock_path(&k);
        if !others && std::fs::read_to_string(&lock).is_ok_and(|h| h.split_whitespace().next() == Some(&me.to_string())) {
            let _ = std::fs::remove_file(lock);
        }
    }
    let _ = std::fs::remove_file(p);
}

/// Today as a day number on this computer's clock (days since 1970 at local midnight), for
/// "used today" counts.
pub fn local_day() -> i64 {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    // SAFETY: an all-zero `tm` is valid; localtime_r writes only into it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = secs as libc::time_t;
    unsafe { libc::localtime_r(&t, &mut tm) };
    (secs + tm.tm_gmtoff) / 86_400
}

/// `21 Nov 2026` on this computer's clock for a time in ms since 1970 (not the UTC day).
pub fn local_date(ms: u64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let t = (ms / 1000) as libc::time_t;
    // SAFETY: an all-zero `tm` is valid; localtime_r writes only into it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{} {} {}", tm.tm_mday, MONTHS[(tm.tm_mon.clamp(0, 11)) as usize], tm.tm_year + 1900)
}

/// `HH:MM` on this computer's clock for a time in ms since 1970 (R1: "watching until 14:30").
pub fn local_hm(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    // SAFETY: an all-zero `tm` is valid; localtime_r writes only into it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = secs as libc::time_t;
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" {
        fn tzset();
    }

    /// FastTest M3: dates are this computer's day, not the UTC day.
    #[test]
    fn dates_are_local() {
        let old = std::env::var_os("TZ");
        // SAFETY: test-only; TZ is read by localtime_r after tzset.
        unsafe {
            std::env::set_var("TZ", "Asia/Jakarta");
            tzset();
        }
        // 2026-11-20 20:00 UTC is 21 Nov, 03:00 in Jakarta (UTC+7).
        let (d, t) = (local_date(1_795_204_800_000), local_hm(1_795_204_800_000));
        unsafe {
            match old {
                Some(v) => std::env::set_var("TZ", v),
                None => std::env::remove_var("TZ"),
            }
            tzset();
        }
        assert_eq!((d.as_str(), t.as_str()), ("21 Nov 2026", "03:00"));
    }

    #[test]
    fn device_keys_and_labels() {
        assert_eq!(device_key(Platform::Web, None), None);
        assert_eq!(device_key(Platform::Android, Some("emulator-5556")).as_deref(), Some("android:emulator-5556"));
        assert_eq!(label("android:emulator-5556"), "the Android emulator");
        assert_eq!(label("ios:default"), "the iPhone simulator");
        assert!(alive(std::process::id()));
        assert_eq!(safe("../x y"), ".._x_y");
    }

    #[tokio::test]
    async fn one_process_never_waits_for_itself() {
        // SAFETY: only this test reads the variable in this crate's tests.
        unsafe { std::env::set_var("FASTPILOT_MAX_PARALLEL", "1") };
        let _ = std::fs::remove_dir_all(dir());
        acquire("t-web", Platform::Web, None).await.unwrap();
        acquire("t-android", Platform::Android, Some("emulator-test")).await.unwrap();
        let mine: Vec<_> = entries().into_iter().filter(|e| e.session.starts_with("t-")).collect();
        assert_eq!(mine.len(), 2);
        assert!(mine.iter().all(|e| !e.waiting));
        assert!(lock_path("android:emulator-test").exists());
        release("t-android");
        release("t-web");
        assert!(!lock_path("android:emulator-test").exists());
        unsafe { std::env::remove_var("FASTPILOT_MAX_PARALLEL") };
    }
}
