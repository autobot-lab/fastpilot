//! E2 (v0.7): test passwords kept per app, and where they may be used (Security S-1).
//!
//! - A value is kept in the OS key store (macOS Keychain, Linux secret service, else an owner-only
//!   file; `FASTPILOT_KEYSTORE=file` in tests): service `fastpilot-secret`, account
//!   `<app-id>/<NAME>`. It is stored hex-encoded, so no quote or space of a password ever reaches
//!   `security`'s command parser, and it is never in a program's arguments (`security -i` reads it
//!   from stdin).
//! - It is set only by the person on the dashboard (never by Chat, cards, MCP tools or files).
//! - S-1: a password is used only where its app is. A step that would type it, open an address
//!   with it, or send it with `httpRequest` runs only when that page or address is on the current
//!   app's own website (same scheme, host and port; a `file:` app: under its folder), or, on a
//!   phone, when the app has that platform. Without an app (CI, a plain project) the old rules
//!   stay: `allowed_domains` when set, else anywhere.
//!   A value from the environment fastpilot starts in (CI, `FASTPILOT_VARS`) is not an app's: it
//!   keeps the old rules too.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::RwLock;

use crate::Platform;

const SERVICE: &str = "fastpilot-secret";

/// The app this process works for: its id (for the key store) and where its secrets may go.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scope {
    pub app: String,
    /// The app's website, as stored (already parsed and normalized).
    pub web: Option<String>,
    pub android: bool,
    pub ios: bool,
}

static SCOPE: RwLock<Option<Scope>> = RwLock::new(None);

/// Set by the fastpilot program for the current app (dashboard, MCP server, `fastpilot run`).
pub fn set_scope(scope: Option<Scope>) {
    *SCOPE.write().unwrap_or_else(|e| e.into_inner()) = scope;
}

pub fn scope() -> Option<Scope> {
    SCOPE.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A name a secret may have: `DEMO_PASS`, upper-case letters, digits and `_`.
pub fn check_name(name: &str) -> Result<(), String> {
    let mut c = name.chars();
    let ok = name.len() <= 64 && c.next().is_some_and(|f| f.is_ascii_uppercase() || f == '_') && c.all(|x| x.is_ascii_uppercase() || x.is_ascii_digit() || x == '_');
    if ok { Ok(()) } else { Err("a secret's name is like DEMO_PASS: capital letters, digits and _".into()) }
}

/// One line, 4 to 4096 characters (a pasted line break is not part of a password; a shorter
/// value could not be told apart from ordinary text, so it could not be masked or kept on its site).
pub fn check_value(value: &str) -> Result<(), String> {
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return Err("The value must be one line, without line breaks.".into());
    }
    if value.chars().count() < 4 {
        return Err("The value must be at least 4 characters.".into());
    }
    Ok(())
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<String> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

fn file(app: &str, name: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join(".fastpilot").join("keys").join("secrets").join(app).join(name)
}

fn run(program: &str, args: &[&str], stdin: &str) -> Result<(), String> {
    let mut child = Command::new(program).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_| format!("{program} could not start"))?;
    if let Some(mut i) = child.stdin.take() {
        i.write_all(stdin.as_bytes()).map_err(|_| format!("{program} could not read the value"))?;
    }
    let st = child.wait().map_err(|_| format!("{program} failed"))?;
    if st.success() { Ok(()) } else { Err(format!("{program} refused")) }
}

fn ids_ok(app: &str, name: &str) -> Result<(), String> {
    check_name(name)?;
    if app.is_empty() || !app.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Err("no app is chosen".into());
    }
    Ok(())
}

/// Keep `value` for `name` of `app` (replacing an older one).
pub fn set(app: &str, name: &str, value: &str) -> Result<(), String> {
    ids_ok(app, name)?;
    check_value(value)?;
    let account = format!("{app}/{name}");
    let v = hex(value);
    match crate::jira::token::backend() {
        crate::jira::token::Backend::Keychain => run("security", &["-i"], &format!("add-generic-password -U -s {SERVICE} -a {account} -l \"fastpilot test password\" -w \"{v}\"\n")),
        crate::jira::token::Backend::SecretService => run("secret-tool", &["store", "--label=fastpilot test password", "service", SERVICE, "account", &account], &v),
        crate::jira::token::Backend::File => {
            let p = file(app, name);
            let dir = p.parent().ok_or("no key folder")?;
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
                for d in [dir, dir.parent().unwrap_or(dir)] {
                    std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
                }
                let tmp = p.with_extension("tmp");
                let _ = std::fs::remove_file(&tmp);
                let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
                f.write_all(v.as_bytes()).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
            }
            #[cfg(not(unix))]
            std::fs::write(&p, v).map_err(|e| e.to_string())
        }
    }
}

/// The value kept for `name` of `app`.
pub fn get(app: &str, name: &str) -> Option<String> {
    ids_ok(app, name).ok()?;
    let account = format!("{app}/{name}");
    let raw = match crate::jira::token::backend() {
        crate::jira::token::Backend::Keychain => {
            let o = Command::new("security").args(["find-generic-password", "-s", SERVICE, "-a", &account, "-w"]).stderr(Stdio::null()).output().ok()?;
            o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())?
        }
        crate::jira::token::Backend::SecretService => {
            let o = Command::new("secret-tool").args(["lookup", "service", SERVICE, "account", &account]).stderr(Stdio::null()).output().ok()?;
            o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())?
        }
        crate::jira::token::Backend::File => std::fs::read_to_string(file(app, name)).ok()?,
    };
    unhex(&raw).filter(|v| !v.is_empty())
}

pub fn has(app: &str, name: &str) -> bool {
    get(app, name).is_some()
}

/// Forget it. `Err` when it is still there afterwards.
pub fn remove(app: &str, name: &str) -> Result<(), String> {
    ids_ok(app, name)?;
    let account = format!("{app}/{name}");
    match crate::jira::token::backend() {
        crate::jira::token::Backend::Keychain => {
            let _ = Command::new("security").args(["delete-generic-password", "-s", SERVICE, "-a", &account]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
        crate::jira::token::Backend::SecretService => {
            let _ = Command::new("secret-tool").args(["clear", "service", SERVICE, "account", &account]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
        crate::jira::token::Backend::File => {
            let _ = std::fs::remove_file(file(app, name));
        }
    }
    if has(app, name) { Err("still stored".into()) } else { Ok(()) }
}

/// The value for `name` in the current app's key store, if there is an app and a value.
pub fn lookup(name: &str) -> Option<String> {
    scope().and_then(|s| get(&s.app, name))
}

/// S-1: may a secret be used on `url` (the open page, or an address it goes to) of a session on
/// `platform`? See the module text.
pub fn allowed_at(url: Option<&str>, platform: Platform) -> Result<(), String> {
    let Some(s) = scope() else {
        let domains = crate::policy::allowed_domains();
        return match url {
            Some(u) if !crate::policy::url_allowed(u, &domains) => Err(format!("a password is not sent to {}: it is not in your allowed domains", host_of(u))),
            _ => Ok(()),
        };
    };
    match platform {
        Platform::Android | Platform::Ios if url.is_none_or(|u| !u.contains("://")) => {
            let has = if platform == Platform::Android { s.android } else { s.ios };
            if has { Ok(()) } else { Err(format!("this app has no {} app, so its passwords are not typed here", if platform == Platform::Android { "Android" } else { "iPhone" })) }
        }
        _ => {
            let Some(u) = url else { return Err("the page's address is not known, so no password is typed".into()) };
            let (Some(web), Ok(at)) = (s.web.as_deref().and_then(|w| url::Url::parse(w).ok()), url::Url::parse(u)) else {
                return Err(format!("a password of this app is only used on its own website, not on {}", host_of(u)));
            };
            let same = if web.scheme() == "file" {
                let dir = web.path().rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
                at.scheme() == "file" && at.host_str().is_none_or(|h| h.is_empty() || h == "localhost") && at.path().starts_with(&dir)
            } else {
                at.scheme() == web.scheme() && at.host_str() == web.host_str() && at.port_or_known_default() == web.port_or_known_default()
            };
            if same { Ok(()) } else { Err(format!("a password of this app is only used on its own website ({}), not on {}", host_of(web.as_str()), host_of(u))) }
        }
    }
}

fn host_of(u: &str) -> String {
    url::Url::parse(u).ok().and_then(|x| x.host_str().map(str::to_owned)).unwrap_or_else(|| "this address".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_through_hex_whatever_they_hold() {
        for v in ["rahasia123", "pa ss\"w'o\\rd", "pässwörd 🔑", "-w \"x\"\nadd"] {
            assert_eq!(unhex(&hex(v)).as_deref(), Some(v));
        }
        assert!(check_value("a\nb").is_err() && check_value("").is_err() && check_value("abc").is_err() && check_value("ok pass").is_ok());
        assert!(check_name("DEMO_PASS").is_ok() && check_name("demo").is_err() && check_name("A;B").is_err());
    }

    #[test]
    fn a_password_goes_only_to_its_apps_site() {
        let shop = Scope { app: "shop".into(), web: Some("https://shop.example/".into()), android: true, ios: false };
        set_scope(Some(shop));
        assert!(allowed_at(Some("https://shop.example/login"), Platform::Web).is_ok());
        assert!(allowed_at(Some("https://evil.example/login"), Platform::Web).is_err());
        assert!(allowed_at(Some("https://shop.example.evil.example/"), Platform::Web).is_err());
        assert!(allowed_at(Some("http://shop.example/"), Platform::Web).is_err(), "another scheme");
        assert!(allowed_at(Some("https://shop.example:8443/"), Platform::Web).is_err(), "another port");
        assert!(allowed_at(None, Platform::Web).is_err());
        assert!(allowed_at(None, Platform::Android).is_ok() && allowed_at(None, Platform::Ios).is_err());
        set_scope(Some(Scope { app: "demo".into(), web: Some("file:///Users/me/demo/login.html".into()), android: false, ios: false }));
        assert!(allowed_at(Some("file:///Users/me/demo/dashboard.html"), Platform::Web).is_ok());
        assert!(allowed_at(Some("file:///Users/me/other/x.html"), Platform::Web).is_err());
        assert!(allowed_at(Some("https://evil.example/"), Platform::Web).is_err());
        set_scope(None);
        assert!(allowed_at(Some("https://anywhere.example/"), Platform::Web).is_ok(), "no app: the old rules");
    }
}
