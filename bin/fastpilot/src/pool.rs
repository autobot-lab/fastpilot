//! Device pool for parallel suites (PRD Phase 6): Android devices and booted iOS simulators
//! are leased to one test at a time (one UiAutomation agent / XCUITest runner per device),
//! health-checked when leased, and a retry prefers a device other than the one that failed.
//! Web needs no lease: every test launches its own Chrome.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, MutexGuard};

use fastpilot_core::Platform;
use tokio::sync::Notify;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub platform: Platform,
    pub id: String,
}

type Health = Box<dyn Fn(Device) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

pub struct Pool {
    /// Healthy devices known to the pool; an unhealthy one is dropped for the rest of the run.
    all: Mutex<Vec<Device>>,
    free: Mutex<Vec<Device>>,
    notify: Notify,
    health: Health,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// `adb get-state` is `device` and boot finished / the simulator is still booted.
async fn healthy(d: Device) -> bool {
    match d.platform {
        Platform::Android => {
            let adb = std::env::var("ANDROID_HOME")
                .map(|h| format!("{h}/platform-tools/adb"))
                .unwrap_or_else(|_| format!("{}/Library/Android/sdk/platform-tools/adb", std::env::var("HOME").unwrap_or_default()));
            let adb = if std::path::Path::new(&adb).exists() { adb } else { "adb".into() };
            let out = tokio::process::Command::new(adb).args(["-s", &d.id, "shell", "getprop", "sys.boot_completed"]).output();
            match tokio::time::timeout(std::time::Duration::from_secs(10), out).await {
                Ok(Ok(o)) => String::from_utf8_lossy(&o.stdout).trim() == "1",
                _ => false,
            }
        }
        Platform::Ios => {
            fastpilot_ios::devices().await.is_ok_and(|v| v.iter().any(|(u, _, st)| *u == d.id && st == "Booted"))
                || fastpilot_ios::physical_devices().await.contains(&d.id)
        }
        Platform::Web => true,
    }
}

impl Pool {
    pub fn new(devices: Vec<Device>, health: Health) -> Self {
        Self { free: Mutex::new(devices.clone()), all: Mutex::new(devices), notify: Notify::new(), health }
    }

    /// Attached Android devices and booted iPhone/iPad simulators.
    pub async fn discover() -> Self {
        let mut devices = Vec::new();
        if let Ok((attached, _)) = fastpilot_android::devices().await {
            devices.extend(attached.into_iter().filter(|(_, st)| st == "device").map(|(id, _)| Device { platform: Platform::Android, id }));
        }
        if let Ok(sims) = fastpilot_ios::devices().await {
            devices.extend(
                sims.into_iter()
                    .filter(|(_, n, st)| st == "Booted" && (n.starts_with("iPhone") || n.starts_with("iPad")))
                    .map(|(id, _, _)| Device { platform: Platform::Ios, id }),
            );
        }
        // Physical iPhones are never leased automatically (free-signing app slots); name one
        // in the test case's session plan instead.
        Self::new(devices, Box::new(|d| Box::pin(healthy(d))))
    }

    pub fn summary(&self) -> String {
        let all = lock(&self.all);
        let n = |p: Platform| all.iter().filter(|d| d.platform == p).count();
        format!("devices: {} android, {} ios (web: one Chrome per test)", n(Platform::Android), n(Platform::Ios))
    }

    /// One device per mobile platform in `need`, preferring ones not in `avoid` (a retry
    /// moves to another device when there is one). Waits while they are all leased; fails
    /// when no healthy device of a needed platform is left.
    pub async fn lease(&self, need: &[Platform], avoid: &[String]) -> Result<Vec<Device>, String> {
        let mut need: Vec<Platform> = need.iter().copied().filter(|p| *p != Platform::Web).collect();
        need.dedup();
        if need.is_empty() {
            return Ok(Vec::new());
        }
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let picked = {
                let all = lock(&self.all);
                let mut free = lock(&self.free);
                let mut picked = Vec::new();
                for p in &need {
                    let known: Vec<&Device> = all.iter().filter(|d| d.platform == *p).collect();
                    if known.is_empty() {
                        return Err(format!("no healthy {p} device in the pool"));
                    }
                    let elsewhere = known.iter().any(|d| !avoid.contains(&d.id));
                    let pos = free.iter().position(|d| d.platform == *p && (!elsewhere || !avoid.contains(&d.id)));
                    match pos {
                        Some(i) => picked.push(free.remove(i)),
                        None => {
                            free.extend(picked.drain(..));
                            break;
                        }
                    }
                }
                (picked.len() == need.len()).then_some(picked)
            };
            let Some(devices) = picked else {
                notified.await;
                continue;
            };
            let mut sick = Vec::new();
            for d in &devices {
                if !(self.health)(d.clone()).await {
                    sick.push(d.clone());
                }
            }
            if sick.is_empty() {
                return Ok(devices);
            }
            for d in &sick {
                eprintln!("pool: {} {} failed its health check; removed for this run", d.platform, d.id);
            }
            lock(&self.all).retain(|d| !sick.contains(d));
            self.release(devices.into_iter().filter(|d| !sick.contains(d)).collect());
        }
    }

    pub fn release(&self, devices: Vec<Device>) {
        {
            let all = lock(&self.all);
            lock(&self.free).extend(devices.into_iter().filter(|d| all.contains(d)));
        }
        self.notify.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    fn dev(p: Platform, id: &str) -> Device {
        Device { platform: p, id: id.into() }
    }

    #[tokio::test]
    async fn leases_exclusively_avoids_failed_devices_and_drops_unhealthy_ones() {
        let pool = Arc::new(Pool::new(
            vec![dev(Platform::Android, "a1"), dev(Platform::Android, "a2"), dev(Platform::Android, "sick"), dev(Platform::Ios, "i1")],
            Box::new(|d| Box::pin(async move { d.id != "sick" })),
        ));
        assert!(pool.lease(&[Platform::Web], &[]).await.expect("web").is_empty());
        // A retry after a failure on a1 gets another device.
        let a = pool.lease(&[Platform::Android], &["a1".into()]).await.expect("lease");
        assert_eq!(a, vec![dev(Platform::Android, "a2")]);
        let b = pool.lease(&[Platform::Android, Platform::Ios], &[]).await.expect("lease");
        assert_eq!(b, vec![dev(Platform::Android, "a1"), dev(Platform::Ios, "i1")]);
        // Only "sick" is free: it fails the health check and is dropped; the lease waits.
        let p2 = pool.clone();
        let waiting = tokio::spawn(async move { p2.lease(&[Platform::Android], &[]).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished());
        assert!(!pool.summary().contains("3 android"), "{}", pool.summary());
        pool.release(a);
        assert_eq!(waiting.await.expect("join").expect("lease"), vec![dev(Platform::Android, "a2")]);
        // With a1 as the only other device, avoiding it still yields it when nothing else exists.
        pool.release(b);
        let only = Pool::new(vec![dev(Platform::Android, "a1")], Box::new(|_| Box::pin(async { true })));
        assert_eq!(only.lease(&[Platform::Android], &["a1".into()]).await.expect("lease"), vec![dev(Platform::Android, "a1")]);
        assert!(only.lease(&[Platform::Ios], &[]).await.is_err());
    }
}
