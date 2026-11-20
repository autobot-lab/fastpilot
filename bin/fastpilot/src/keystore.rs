//! API keys of OpenAI-compatible providers (`fastpilot config set-key <provider>`): the macOS
//! Keychain, the Linux secret service (`secret-tool`), or else an owner-only file under
//! `~/.fastpilot/keys/`. `FASTPILOT_PROVIDER_API_KEY` in the environment wins over all of them.
//!
//! A key never goes on a command line (other processes can read argv): `security -i` and
//! `secret-tool store` read it from stdin. It is never logged or printed; callers show only
//! where it is kept.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Keychain service / secret-service attribute for every provider key.
const SERVICE: &str = "fastpilot-provider";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Keychain,
    SecretService,
    File,
}

impl Backend {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Keychain => "macOS Keychain",
            Self::SecretService => "secret service (secret-tool)",
            Self::File => "~/.fastpilot/keys (owner-only file)",
        }
    }
}

/// `FASTPILOT_KEYSTORE=keychain|secret-service|file` picks one (tests use `file`); else the
/// platform's own store, or the file when there is none.
pub fn backend() -> Backend {
    match std::env::var("FASTPILOT_KEYSTORE").ok().as_deref() {
        Some("file") => return Backend::File,
        Some("keychain") => return Backend::Keychain,
        Some("secret-service") => return Backend::SecretService,
        _ => {}
    }
    if cfg!(target_os = "macos") {
        Backend::Keychain
    } else if Command::new("secret-tool").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok() {
        Backend::SecretService
    } else {
        Backend::File
    }
}

/// Provider names are the `provider` setting's values: safe as a file name and an account.
fn check_provider(provider: &str) -> Result<(), String> {
    if fastpilot_core::config::PROVIDERS.iter().any(|(p, _)| *p == provider && *p != "claude") {
        Ok(())
    } else {
        let names: Vec<&str> = fastpilot_core::config::PROVIDERS.iter().map(|(p, _)| *p).filter(|p| *p != "claude").collect();
        Err(format!("unknown provider `{provider}` (one of {})", names.join(", ")))
    }
}

/// A key is one printable token: no spaces, quotes or control characters (it is quoted for
/// `security -i`, and a pasted newline would be part of it).
fn check_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > 4096 || !key.chars().all(|c| c.is_ascii_graphic() && c != '"' && c != '\\' && c != '\'') {
        return Err("the key must be one line of printable characters, without spaces or quotes".into());
    }
    Ok(())
}

fn key_file(provider: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join(".fastpilot").join("keys").join(provider)
}

fn run_with_stdin(program: &str, args: &[&str], stdin: &str) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    if let Some(mut i) = child.stdin.take() {
        i.write_all(stdin.as_bytes()).map_err(|e| format!("{program}: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        // Their messages name the item, never the secret.
        Err(format!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Keep `key` for `provider`, tied to the server it may be sent to (the provider's preset, or the
/// configured base URL for custom): see [`origin`]. Returns where it went.
pub fn set(provider: &str, key: &str) -> Result<&'static str, String> {
    check_provider(provider)?;
    let key = key.trim();
    check_key(key)?;
    let tied_to = crate::provider::key_origin(provider)?;
    store(provider, key)?;
    store(&format!("{provider}.origin"), &tied_to)?;
    Ok(backend().describe())
}

/// The origin (`https://host[:port]`) a stored key was saved for; it is sent nowhere else.
pub fn origin(provider: &str) -> Option<String> {
    check_provider(provider).ok()?;
    load(&format!("{provider}.origin"))
}

/// Where a key comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyFrom {
    /// FASTPILOT_PROVIDER_API_KEY: the person running fastpilot chose it for this run.
    Env,
    /// The key store, with the origin it was saved for; `None`: saved by an older fastpilot,
    /// before keys were bound to a server.
    Store(Option<String>),
}

/// The key and where it comes from.
pub fn get_bound(provider: &str) -> Option<(String, KeyFrom)> {
    if let Ok(k) = std::env::var("FASTPILOT_PROVIDER_API_KEY")
        && !k.trim().is_empty()
    {
        return Some((k.trim().to_owned(), KeyFrom::Env));
    }
    let key = get(provider)?;
    Some((key, KeyFrom::Store(origin(provider))))
}

/// One secret under `account` (a provider, or `<provider>.origin`) in the store.
fn store(account: &str, key: &str) -> Result<(), String> {
    let provider = account;
    let b = backend();
    match b {
        Backend::Keychain => {
            // `-i`: the command (and so the key) comes on stdin, not argv.
            run_with_stdin("security", &["-i"], &format!("add-generic-password -U -s {SERVICE} -a {provider} -l \"fastpilot {provider} API key\" -w \"{key}\"\n"))?;
        }
        Backend::SecretService => {
            run_with_stdin("secret-tool", &["store", &format!("--label=fastpilot {provider} API key"), "service", SERVICE, "account", provider], key)?;
        }
        Backend::File => {
            let path = key_file(provider);
            let dir = path.parent().ok_or("no key folder")?;
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
                let tmp = path.with_extension("tmp");
                let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
                f.write_all(key.as_bytes()).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
            }
            #[cfg(not(unix))]
            std::fs::write(&path, key).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The key for `provider`: the environment first, then the store.
pub fn get(provider: &str) -> Option<String> {
    if let Ok(k) = std::env::var("FASTPILOT_PROVIDER_API_KEY")
        && !k.trim().is_empty()
    {
        return Some(k.trim().to_owned());
    }
    check_provider(provider).ok()?;
    load(provider)
}

fn load(account: &str) -> Option<String> {
    let provider = account;
    let key = match backend() {
        Backend::Keychain => {
            let out = Command::new("security").args(["find-generic-password", "-s", SERVICE, "-a", provider, "-w"]).stderr(Stdio::null()).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())?
        }
        Backend::SecretService => {
            let out = Command::new("secret-tool").args(["lookup", "service", SERVICE, "account", provider]).stderr(Stdio::null()).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())?
        }
        Backend::File => std::fs::read_to_string(key_file(provider)).ok()?,
    };
    Some(key.trim().to_owned()).filter(|k| !k.is_empty())
}

/// Whether a key is available, without handing it out (for status displays).
pub fn has(provider: &str) -> bool {
    get(provider).is_some()
}

/// Forget the stored key (the environment variable is the user's own).
pub fn delete(provider: &str) -> Result<(), String> {
    check_provider(provider)?;
    remove(provider);
    remove(&format!("{provider}.origin"));
    Ok(())
}

fn remove(account: &str) {
    let provider = account;
    match backend() {
        Backend::Keychain => {
            let _ = Command::new("security").args(["delete-generic-password", "-s", SERVICE, "-a", provider]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
        Backend::SecretService => {
            let _ = Command::new("secret-tool").args(["clear", "service", SERVICE, "account", provider]).status();
        }
        Backend::File => {
            let _ = std::fs::remove_file(key_file(provider));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B2: keys go to the store and back, owner-only, never for an unknown provider or a key
    /// that would not survive the trip. File backend: the user's Keychain is left alone.
    #[test]
    fn file_store_round_trip() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let home = std::env::temp_dir().join(format!("fastpilot-keys-{}", std::process::id()));
        let old_home = std::env::var_os("HOME");
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("FASTPILOT_KEYSTORE", "file");
            std::env::remove_var("FASTPILOT_PROVIDER_API_KEY");
        }
        assert_eq!(backend(), Backend::File);
        assert!(!has("openai"));
        unsafe {
            std::env::remove_var("FASTPILOT_PROVIDER");
            std::env::remove_var("FASTPILOT_PROVIDER_BASE_URL");
            std::env::set_var("FASTPILOT_CONFIG", home.join("none.toml"));
        }
        assert_eq!(set("openai", "  sk-test-abc123\n"), Ok("~/.fastpilot/keys (owner-only file)"));
        assert_eq!(get("openai").as_deref(), Some("sk-test-abc123"));
        // Tied to the server it may go to: the preset's, or the custom base URL.
        assert_eq!(origin("openai").as_deref(), Some("https://api.openai.com"));
        assert_eq!(get_bound("openai"), Some(("sk-test-abc123".into(), KeyFrom::Store(Some("https://api.openai.com".into())))));
        assert!(set("custom", "sk-c").expect_err("custom without its server").contains("provider_base_url"));
        unsafe {
            std::env::set_var("FASTPILOT_PROVIDER", "custom");
            std::env::set_var("FASTPILOT_PROVIDER_BASE_URL", "https://llm.example.com:8443/v1");
        }
        set("custom", "sk-c").expect("custom");
        assert_eq!(origin("custom").as_deref(), Some("https://llm.example.com:8443"));
        unsafe {
            std::env::remove_var("FASTPILOT_PROVIDER");
            std::env::remove_var("FASTPILOT_PROVIDER_BASE_URL");
            std::env::remove_var("FASTPILOT_CONFIG");
        }
        delete("custom").expect("delete");
        assert!(origin("custom").is_none(), "the origin goes with the key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &std::path::Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode(&key_file("openai")), 0o600);
            assert_eq!(mode(key_file("openai").parent().expect("dir")), 0o700);
        }
        assert!(set("claude", "x").is_err() && set("../../etc", "x").is_err(), "provider names only");
        assert!(set("groq", "two words").is_err() && set("groq", "q\"uote").is_err() && set("groq", "").is_err());
        // The environment wins.
        unsafe { std::env::set_var("FASTPILOT_PROVIDER_API_KEY", "sk-from-env") };
        assert_eq!(get("openai").as_deref(), Some("sk-from-env"));
        unsafe { std::env::remove_var("FASTPILOT_PROVIDER_API_KEY") };
        delete("openai").expect("delete");
        assert!(!has("openai"));
        unsafe {
            match old_home {
                Some(h) => std::env::set_var("HOME", h),
                None => std::env::remove_var("HOME"),
            }
            std::env::remove_var("FASTPILOT_KEYSTORE");
        }
        let _ = std::fs::remove_dir_all(home);
    }
}
