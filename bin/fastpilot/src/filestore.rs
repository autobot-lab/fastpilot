//! Small JSON files that several fastpilot processes of one user change (remembered browsers, the
//! apps list). One change at a time (Security, #178 M1): a thread lock in this process plus a file
//! lock for other processes, held around every load-change-save; each write goes to a temp file of
//! its own (pid and random, never an existing file) renamed into place, owner-only (0600).

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// Held while it lives.
pub struct Held(#[allow(dead_code)] MutexGuard<'static, ()>, #[allow(dead_code)] Option<std::fs::File>);

/// Take `lock` and the file lock `<dir>/<name>` (best effort: without it the thread lock still holds).
pub fn hold(lock: &'static Mutex<()>, dir: &Path, name: &str, private: bool) -> Held {
    let g = lock.lock().unwrap_or_else(|e| e.into_inner());
    let f = ensure_dir(dir, private).ok().and_then(|()| {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(false);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let f = o.open(dir.join(name)).ok()?;
        f.lock().ok()?;
        Some(f)
    });
    Held(g, f)
}

/// The folder, made if missing; `private` makes it owner-only (0700).
pub fn ensure_dir(dir: &Path, private: bool) -> Result<(), String> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    if private {
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    }
    b.create(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Write `<dir>/<file>` whole and owner-only. Call with the lock held.
pub fn write_atomic(dir: &Path, file: &str, bytes: &[u8], private: bool) -> Result<(), String> {
    ensure_dir(dir, private)?;
    let tmp = dir.join(format!(".{file}.{}.{:016x}.tmp", std::process::id(), random_u64()));
    {
        use std::io::Write;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&tmp).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, dir.join(file)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

fn random_u64() -> u64 {
    use std::io::Read;
    let mut b = [0u8; 8];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    u64::from_le_bytes(b) ^ t
}
