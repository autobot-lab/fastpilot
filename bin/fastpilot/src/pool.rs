//! Device pool for parallel suites (PRD Phase 6): Android devices and booted iOS simulators
//! are leased to one test at a time (one UiAutomation agent / XCUITest runner per device),
//! health-checked when leased, and a retry prefers a device other than the one that failed.
//! Web needs no lease: every test launches its own Chrome.
//!
//! Android: when `FASTPILOT_ANDROID_SERIAL` (or `ANDROID_SERIAL`) names a device, the pool has
//! that device only, and nothing is sent to any other one (v0.4.1: the release check sent a
//! health check to the user's own emulator). Listing and health checks go through the same adb
//! server (`FASTPILOT_ADB_SERVER` when set), and the session plan says which devices were taken.

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
    /// Which Android devices were taken and why, for the session plan.
    note: String,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The Android device the user chose: `FASTPILOT_ANDROID_SERIAL`, else `ANDROID_SERIAL`.
fn chosen_serial() -> Option<(String, &'static str)> {
    ["FASTPILOT_ANDROID_SERIAL", "ANDROID_SERIAL"]
        .into_iter()
        .find_map(|k| fastpilot_core::config::var(k).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()).map(|v| (v, k)))
}

/// Ready Android devices for the pool, and a line for the session plan. With a chosen serial,
/// only that one (no other device is ever contacted); without, every connected one, as before.
async fn android_devices() -> (Vec<Device>, String) {
    let attached: Vec<String> = match fastpilot_android::Adb::new().await {
        Ok(adb) => adb.devices().await.unwrap_or_default().into_iter().filter(|(_, st)| st == "device").map(|(id, _)| id).collect(),
        Err(_) => Vec::new(),
    };
    let dev = |id: String| Device { platform: Platform::Android, id };
    match chosen_serial() {
        Some((serial, var)) if attached.contains(&serial) => (vec![dev(serial.clone())], format!("android: {serial} only ({var})")),
        Some((serial, var)) => (Vec::new(), format!("android: {serial} ({var}) is not connected")),
        None if attached.is_empty() => (Vec::new(), String::new()),
        None => {
            let note = format!("android: every connected device ({}); set FASTPILOT_ANDROID_SERIAL to use one", attached.join(", "));
            (attached.into_iter().map(dev).collect(), note)
        }
    }
}

/// Boot finished (Android, through the same adb server as the listing) / the simulator is
/// still booted.
async fn healthy(d: Device) -> bool {
    match d.platform {
        Platform::Android => {
            let probe = async { fastpilot_android::Adb::new().await.ok()?.shell(&d.id, "getprop sys.boot_completed").await.ok() };
            matches!(tokio::time::timeout(std::time::Duration::from_secs(10), probe).await, Ok(Some(out)) if out.trim() == "1")
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
        Self { free: Mutex::new(devices.clone()), all: Mutex::new(devices), notify: Notify::new(), health, note: String::new() }
    }

    /// Attached Android devices and booted iPhone/iPad simulators.
    pub async fn discover() -> Self {
        let (mut devices, note) = android_devices().await;
        if let Ok(sims) = fastpilot_ios::devices().await {
            devices.extend(
                sims.into_iter()
                    .filter(|(_, n, st)| st == "Booted" && (n.starts_with("iPhone") || n.starts_with("iPad")))
                    .map(|(id, _, _)| Device { platform: Platform::Ios, id }),
            );
        }
        // Physical iPhones are never leased automatically (free-signing app slots); name one
        // in the test case's session plan instead.
        Self { note, ..Self::new(devices, Box::new(|d| Box::pin(healthy(d)))) }
    }

    pub fn summary(&self) -> String {
        let all = lock(&self.all);
        let n = |p: Platform| all.iter().filter(|d| d.platform == p).count();
        let note = if self.note.is_empty() { String::new() } else { format!(" · {}", self.note) };
        format!("devices: {} android, {} ios (web: one Chrome per test){note}", n(Platform::Android), n(Platform::Ios))
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

    /// A fake adb server with two emulators: it answers `host:devices`, `host:transport:<serial>`
    /// and `shell:getprop sys.boot_completed` (booted), and logs every request it gets.
    async fn fake_adb() -> (String, Arc<Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let seen = seen.clone();
                tokio::spawn(async move {
                    loop {
                        let mut len = [0u8; 4];
                        if sock.read_exact(&mut len).await.is_err() {
                            return;
                        }
                        let n = usize::from_str_radix(std::str::from_utf8(&len).unwrap_or("0"), 16).unwrap_or(0);
                        let mut req = vec![0u8; n];
                        if sock.read_exact(&mut req).await.is_err() {
                            return;
                        }
                        let req = String::from_utf8_lossy(&req).into_owned();
                        lock(&seen).push(req.clone());
                        if req == "host:devices" {
                            let list = "emulator-5554\tdevice\nemulator-5556\tdevice\n";
                            let _ = sock.write_all(format!("OKAY{:04x}{list}", list.len()).as_bytes()).await;
                            return;
                        } else if req.starts_with("host:transport:") {
                            let _ = sock.write_all(b"OKAY").await;
                        } else if req.starts_with("shell:") {
                            let _ = sock.write_all(b"OKAY1\n").await;
                            return;
                        } else {
                            let _ = sock.write_all(b"FAIL0004nope").await;
                            return;
                        }
                    }
                });
            }
        });
        (addr, log)
    }

    /// v0.4.1 (FastTest's release check): with FASTPILOT_ANDROID_SERIAL set, the pool takes that
    /// device only, and nothing at all goes to the other one, health check included. Without a
    /// serial, every connected device, as before, and the session plan says so.
    #[tokio::test]
    // The environment must stay as set for the whole test, so the env lock is held across awaits.
    #[allow(clippy::await_holding_lock)]
    async fn a_chosen_serial_is_the_only_device_contacted() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (server, log) = fake_adb().await;
        let cfg = std::env::temp_dir().join(format!("fastpilot-pool-{}.toml", std::process::id()));
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("FASTPILOT_CONFIG", &cfg);
            std::env::set_var("FASTPILOT_ADB_SERVER", &server);
            std::env::set_var("FASTPILOT_ANDROID_SERIAL", "emulator-5556");
            std::env::remove_var("ANDROID_SERIAL");
        }
        let (devices, note) = android_devices().await;
        assert_eq!(devices, vec![dev(Platform::Android, "emulator-5556")]);
        assert_eq!(note, "android: emulator-5556 only (FASTPILOT_ANDROID_SERIAL)");
        let pool = Pool { note, ..Pool::new(devices, Box::new(|d| Box::pin(healthy(d)))) };
        let leased = pool.lease(&[Platform::Android], &[]).await.expect("lease");
        assert_eq!(leased, vec![dev(Platform::Android, "emulator-5556")]);
        assert!(pool.summary().contains("1 android") && pool.summary().contains("emulator-5556 only"), "{}", pool.summary());
        let asked = lock(&log).clone();
        assert!(asked.contains(&"host:transport:emulator-5556".to_owned()) && asked.iter().any(|r| r == "shell:getprop sys.boot_completed"), "the health check ran: {asked:?}");
        assert!(asked.iter().all(|r| !r.contains("5554")), "nothing goes to the other emulator: {asked:?}");
        // ANDROID_SERIAL counts the same; a serial that is not connected leaves the pool empty.
        unsafe {
            std::env::remove_var("FASTPILOT_ANDROID_SERIAL");
            std::env::set_var("ANDROID_SERIAL", "emulator-5560");
        }
        let (devices, note) = android_devices().await;
        assert!(devices.is_empty() && note == "android: emulator-5560 (ANDROID_SERIAL) is not connected", "{devices:?} {note}");
        // No serial: every connected device, named in the plan (listing only, no health check here).
        unsafe { std::env::remove_var("ANDROID_SERIAL") };
        let (devices, note) = android_devices().await;
        assert_eq!(devices.len(), 2);
        assert!(note.contains("every connected device (emulator-5554, emulator-5556)"), "{note}");
        unsafe {
            std::env::remove_var("FASTPILOT_ADB_SERVER");
            std::env::remove_var("FASTPILOT_CONFIG");
        }
    }
}
