//! Saved login state (`saveState` / `loadState`, the `state` tool, `fastpilot state ...`):
//! what a driver returns from `Driver::save_state` (web: cookies + localStorage), stored under
//! `.fastpilot/states/<name>.json` (or `FASTPILOT_DIR`) so a test can skip the login.
//!
//! The file holds session tokens. It is written owner-only (0600), refused once older than
//! `FASTPILOT_STATE_EXPIRE_DAYS` (default 30), and encrypted with AES-256-GCM when
//! `FASTPILOT_STATE_KEY` is set (64 hex characters; `openssl rand -hex 32`).

use std::path::PathBuf;

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use serde_json::{Value, json};

/// Encrypted files start with this, then the 12-byte nonce, then the ciphertext.
const MAGIC: &[u8] = b"FPSTATE1";

pub fn dir() -> PathBuf {
    std::env::var_os("FASTPILOT_DIR").map(PathBuf::from).unwrap_or_else(|| ".fastpilot".into()).join("states")
}

fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) || name.contains("..") {
        return Err(format!("state name `{name}`: use letters, digits, '-', '_' and '.'"));
    }
    Ok(())
}

fn path(name: &str) -> Result<PathBuf, String> {
    check_name(name)?;
    Ok(dir().join(format!("{}.json", name.trim_end_matches(".json"))))
}

fn key() -> Result<Option<Key<Aes256Gcm>>, String> {
    let Some(hex) = std::env::var("FASTPILOT_STATE_KEY").ok().filter(|k| !k.is_empty()) else { return Ok(None) };
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()))
        .collect::<Option<_>>()
        .filter(|b: &Vec<u8>| b.len() == 32 && hex.len() == 64)
        .ok_or("FASTPILOT_STATE_KEY must be 64 hex characters (32 bytes; `openssl rand -hex 32`)")?;
    Ok(Some(*Key::<Aes256Gcm>::from_slice(&bytes)))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn max_age_days() -> u64 {
    std::env::var("FASTPILOT_STATE_EXPIRE_DAYS").ok().and_then(|d| d.parse().ok()).unwrap_or(30)
}

/// Write `state` as `name`; returns the path and whether it was encrypted.
pub fn write(name: &str, platform: &str, state: &Value) -> Result<(PathBuf, bool), String> {
    let path = path(name)?;
    let doc = json!({ "version": 1, "saved_at": now(), "platform": platform, "state": state });
    let plain = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
    let key = key()?;
    let bytes = match &key {
        Some(k) => {
            let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
            let ct = Aes256Gcm::new(k).encrypt(&nonce, plain.as_slice()).map_err(|_| "encryption failed")?;
            [MAGIC, nonce.as_slice(), &ct].concat()
        }
        None => plain,
    };
    std::fs::create_dir_all(dir()).map_err(|e| format!("cannot create {}: {e}", dir().display()))?;
    write_private(&path, &bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok((path, key.is_some()))
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(bytes)
}

/// The saved document (`version`, `saved_at`, `platform`, `state`), decrypted and checked for age.
pub fn read(name: &str) -> Result<Value, String> {
    let path = path(name)?;
    let bytes = std::fs::read(&path).map_err(|_| format!("no saved state `{name}` ({}); save one with saveState", path.display()))?;
    let plain = if let Some(rest) = bytes.strip_prefix(MAGIC) {
        let key = key()?.ok_or(format!("state `{name}` is encrypted: set FASTPILOT_STATE_KEY to the key it was saved with"))?;
        if rest.len() < 12 {
            return Err(format!("state `{name}` is damaged"));
        }
        let (nonce, ct) = rest.split_at(12);
        Aes256Gcm::new(&key)
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| format!("state `{name}` cannot be decrypted with this FASTPILOT_STATE_KEY"))?
    } else {
        bytes
    };
    let doc: Value = serde_json::from_slice(&plain).map_err(|e| format!("state `{name}` is not valid: {e}"))?;
    let age_days = now().saturating_sub(doc["saved_at"].as_u64().unwrap_or(0)) / 86_400;
    if age_days >= max_age_days() {
        return Err(format!(
            "state `{name}` is {age_days} days old (limit FASTPILOT_STATE_EXPIRE_DAYS={}); log in again and save it anew",
            max_age_days()
        ));
    }
    Ok(doc)
}

/// Saved states: (name, saved_at, encrypted).
pub fn list() -> Vec<(String, u64, bool)> {
    let mut out: Vec<(String, u64, bool)> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?.strip_suffix(".json")?.to_owned();
            let bytes = std::fs::read(&p).ok()?;
            let encrypted = bytes.starts_with(MAGIC);
            let saved = if encrypted {
                e.metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs()
            } else {
                serde_json::from_slice::<Value>(&bytes).ok()?["saved_at"].as_u64()?
            };
            Some((name, saved, encrypted))
        })
        .collect();
    out.sort();
    out
}

pub fn delete(name: &str) -> Result<PathBuf, String> {
    let path = path(name)?;
    std::fs::remove_file(&path).map_err(|_| format!("no saved state `{name}`"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test: it changes process-wide environment variables.
    #[test]
    fn plain_encrypted_expired_and_wrong_key() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let d = std::env::temp_dir().join(format!("fastpilot-state-{}", std::process::id()));
        let state = json!({ "cookies": [{ "name": "sid", "value": "tok-123" }] });
        // SAFETY: test-only env mutation; no other test in this crate reads these variables.
        unsafe {
            std::env::set_var("FASTPILOT_DIR", &d);
            std::env::remove_var("FASTPILOT_STATE_KEY");
        }
        let (p, enc) = write("login", "web", &state).expect("write");
        assert!(!enc && std::fs::read_to_string(&p).expect("file").contains("tok-123"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).expect("meta").permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(read("login").expect("read")["state"], state);

        unsafe { std::env::set_var("FASTPILOT_STATE_KEY", "11".repeat(32)) };
        let (p, enc) = write("secure", "web", &state).expect("write");
        let raw = std::fs::read(&p).expect("file");
        assert!(enc && raw.starts_with(MAGIC) && !String::from_utf8_lossy(&raw).contains("tok-123"));
        assert_eq!(read("secure").expect("read")["state"], state);
        unsafe { std::env::set_var("FASTPILOT_STATE_KEY", "22".repeat(32)) };
        assert!(read("secure").expect_err("wrong key").contains("cannot be decrypted"));
        unsafe { std::env::remove_var("FASTPILOT_STATE_KEY") };
        assert!(read("secure").expect_err("no key").contains("is encrypted"));
        unsafe { std::env::set_var("FASTPILOT_STATE_KEY", "abc") };
        assert!(write("x", "web", &state).expect_err("bad key").contains("64 hex"));
        unsafe { std::env::remove_var("FASTPILOT_STATE_KEY") };

        // Expired.
        let old = json!({ "version": 1, "saved_at": now() - 31 * 86_400, "platform": "web", "state": state });
        std::fs::write(dir().join("old.json"), old.to_string()).expect("old");
        assert!(read("old").expect_err("expired").contains("31 days old"));

        assert_eq!(list().iter().map(|(n, _, e)| (n.as_str(), *e)).collect::<Vec<_>>(), [("login", false), ("old", false), ("secure", true)]);
        assert!(check_name("../etc").is_err() && read("a/b").is_err());
        delete("login").expect("delete");
        assert!(read("login").is_err());
        unsafe { std::env::remove_var("FASTPILOT_DIR") };
        let _ = std::fs::remove_dir_all(d);
    }
}
