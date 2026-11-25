//! E1: a one-time code that a person types while a run waits for it.
//!
//! Only a run started under the dashboard (`fastpilot ui` sets [`ENV`] for everything it starts)
//! can ask. The run writes `<id>.ask` in that owner-only folder: the code's name and the test,
//! never a value. The dashboard shows one hidden field for it and writes `<id>.code` once. The run
//! reads it, deletes both files, and types it once: it is never saved, logged or shown to the AI
//! (its value is masked in everything a tool returns). With nobody to type it, or after the time
//! limit, the step fails with a plain reason and nothing is typed.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The dashboard's folder for asked codes (absolute), set for every process it starts.
pub const ENV: &str = "FASTPILOT_CODE_DIR";

/// How long a run waits for the person to type the code.
pub const WAIT: Duration = Duration::from_secs(180);

/// The test the asking run belongs to, shown next to the field.
static TEST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
/// Names the run's test text marks as codes by their step ("Isi kode OTP ${SMS}").
static NAMES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The run's test: its title for the field, and the `${NAME}`s its steps mark as one-time codes.
pub fn set_test(title: &str, codes: &[String]) {
    *TEST.lock().unwrap_or_else(|p| p.into_inner()) = title.to_owned();
    // Several tests can run in one process: a name stays a code once a test marked it.
    let mut names = NAMES.lock().unwrap_or_else(|p| p.into_inner());
    for c in codes {
        if !names.contains(c) {
            names.push(c.clone());
        }
    }
}

/// Is `name` a one-time code in this run (by its name, or by its test's step)?
pub fn is_run_code(name: &str) -> bool {
    crate::dsl::is_code(name, "") || NAMES.lock().unwrap_or_else(|p| p.into_inner()).iter().any(|n| n == name)
}

/// The dashboard's code folder, when this process was started by one.
pub fn dir() -> Option<PathBuf> {
    std::env::var_os(ENV).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// Wait for the person to type the code `name` in the dashboard (at most [`WAIT`]).
pub fn ask(name: &str) -> Result<String, String> {
    let Some(dir) = dir() else { return Err(crate::dsl::code_needed(name)) };
    // Security #233 M1: a code is only asked for the app in use, and only typed on its own site
    // (the S-1 check runs before this; the field says whose site it is).
    let Some(scope) = crate::secrets::scope() else {
        return Err(format!("one-time code needed: no app is in use, so fastpilot does not know whose code {name} is and does not ask for it; nothing was typed"));
    };
    let site = [scope.web.as_deref().and_then(|w| url::Url::parse(w).ok()).and_then(|u| u.host_str().map(str::to_owned)), scope.android.clone(), scope.ios.clone()].into_iter().flatten().next().unwrap_or_default();
    // Tests shorten the wait; nothing lengthens it.
    let wait = std::env::var("FASTPILOT_CODE_WAIT_MS").ok().and_then(|v| v.parse().ok()).map(Duration::from_millis).map_or(WAIT, |w| w.min(WAIT));
    let test = TEST.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let run = || ask_in(&dir, name, &test, &site, wait);
    // Waiting blocks this thread: tell a multi-threaded runtime, so its other work goes on.
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(run),
        _ => run(),
    }
}

fn ask_in(dir: &Path, name: &str, test: &str, site: &str, wait: Duration) -> Result<String, String> {
    let failed = |why: String| format!("one-time code needed: {why}; nothing was typed");
    ensure_dir(dir).map_err(|e| failed(format!("the dashboard cannot ask for the code {name} ({e})")))?;
    let id = crate::change::random_id(16);
    let (ask, code) = (dir.join(format!("{id}.ask")), dir.join(format!("{id}.code")));
    let until = crate::change::now_ms() + wait.as_millis() as u64;
    let what = serde_json::json!({ "name": name, "test": test, "site": site, "until_ms": until });
    create_private(&ask, &what.to_string()).map_err(|e| failed(format!("the dashboard cannot ask for the code {name} ({e})")))?;
    let deadline = Instant::now() + wait;
    let got = loop {
        if let Some(v) = read_code(&code) {
            break Some(v);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let _ = std::fs::remove_file(&ask);
    let _ = std::fs::remove_file(&code);
    match got {
        Some(v) => {
            // Masked everywhere, and kept to the app's own site like its passwords (S-1), also
            // when a step copies it (Security #233 M1).
            crate::dsl::keep_on_site(&v);
            Ok(v)
        }
        None => {
            let m = wait.as_secs().div_ceil(60).max(1);
            Err(failed(format!(
                "nobody typed the one-time code {name} within {m} minute{}, so this step failed. Run the test again and type the code when the dashboard asks",
                if m == 1 { "" } else { "s" }
            )))
        }
    }
}

/// A code as a person types it: 4 to 12 letters or digits.
pub fn valid(code: &str) -> bool {
    (4..=12).contains(&code.len()) && code.chars().all(|c| c.is_ascii_alphanumeric())
}

fn read_code(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > 64 {
        return None;
    }
    let v = std::fs::read_to_string(path).ok()?.trim().to_owned();
    valid(&v).then_some(v)
}

fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(std::io::Error::other("the folder is a symlink"));
    }
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)?;
    // A folder that was already there is made owner-only too (it fails when it is not ours).
    #[cfg(unix)]
    std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    Ok(())
}

/// A new owner-only file (never over an existing one or through a link).
fn create_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)?.write_all(text.as_bytes())
}

/// A code a run is waiting for: its id, the code's name and test, and when the wait ends.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Asked {
    pub id: String,
    pub name: String,
    pub test: String,
    /// The app's website host (or Android / iPhone app id) the code is typed on.
    pub site: String,
    pub until_ms: u64,
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// The codes runs wait for now in `dir`, oldest first. An ask whose wait ended is removed with
/// its code, and so is a code whose run is gone (Security #233 L1: a run stopped while waiting).
pub fn pending(dir: &Path) -> Vec<Asked> {
    let now = crate::change::now_ms();
    let mut out: Vec<Asked> = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = e.path();
        let file = path.file_name().and_then(|f| f.to_str()).unwrap_or("").to_owned();
        if let Some(id) = file.strip_suffix(".code") {
            if !dir.join(format!("{id}.ask")).exists() {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        let Some(id) = file.strip_suffix(".ask").filter(|id| valid_id(id)) else { continue };
        if !e.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let until = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).map(|v| (v["until_ms"].as_u64().unwrap_or(0), v));
        let Some((until, v)) = until.filter(|(u, _)| *u > now) else {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(dir.join(format!("{id}.code")));
            continue;
        };
        if dir.join(format!("{id}.code")).exists() {
            continue;
        }
        let name = v["name"].as_str().unwrap_or("").to_owned();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') || name.len() > 64 {
            continue;
        }
        let clean = |k: &str| v[k].as_str().unwrap_or("").chars().filter(|c| !c.is_control()).take(120).collect::<String>();
        out.push(Asked { id: id.to_owned(), name, test: clean("test"), site: clean("site"), until_ms: until });
    }
    out.sort_by_key(|a| a.until_ms);
    out
}

/// The person typed `code` for the ask `id`: hand it to the waiting run, once.
pub fn answer(dir: &Path, id: &str, code: &str) -> Result<(), String> {
    let code = code.trim();
    if !valid(code) {
        return Err("Type the code as it was sent: 4 to 12 letters or digits.".into());
    }
    if !valid_id(id) || !pending(dir).iter().any(|a| a.id == id) {
        return Err("The test is not waiting for this code any more. Run it again to get a new code.".into());
    }
    create_private(&dir.join(format!("{id}.code")), code).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists { "This code was already sent.".to_owned() } else { format!("The code could not be handed to the test: {e}") }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E1: the run waits for the code the person types in the dashboard, uses it once and leaves
    /// nothing behind; without an answer it fails in plain words after the time limit.
    #[test]
    fn a_run_waits_for_the_code_the_person_types() {
        let dir = std::env::temp_dir().join(format!("fp-codes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let d = dir.clone();
        let person = std::thread::spawn(move || {
            for _ in 0..100 {
                if let Some(a) = pending(&d).first() {
                    assert_eq!((a.name.as_str(), a.test.as_str(), a.site.as_str()), ("MOTMAINA_OTP", "Login OTP", "shop.example"));
                    assert!(answer(&d, &a.id, "12ab").is_ok());
                    assert!(answer(&d, &a.id, "99999").is_err(), "once");
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            panic!("never asked");
        });
        assert_eq!(ask_in(&dir, "MOTMAINA_OTP", "Login OTP", "shop.example", Duration::from_secs(10)), Ok("12ab".into()));
        person.join().expect("person");
        assert_eq!(std::fs::read_dir(&dir).expect("dir").count(), 0, "nothing left on disk");
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&dir).expect("dir").permissions()) & 0o777, 0o700);

        let e = ask_in(&dir, "MOTMAINA_OTP", "Login OTP", "shop.example", Duration::from_millis(300)).expect_err("nobody typed it");
        assert!(e.starts_with("one-time code needed: nobody typed the one-time code MOTMAINA_OTP within 1 minute,") && e.ends_with("nothing was typed"), "{e}");
        assert!(pending(&dir).is_empty() && std::fs::read_dir(&dir).expect("dir").count() == 0);
        assert!(answer(&dir, "00000000000000000000000000000000", "1234").is_err(), "no such ask");
        // Security #233 L1: a run stopped while waiting leaves its ask and the code typed late;
        // both go (the ask once its wait ended), and so does a code with no ask.
        let id = "0123456789abcdef0123456789abcdef";
        std::fs::write(dir.join(format!("{id}.ask")), serde_json::json!({ "name": "X_OTP", "until_ms": 1 }).to_string()).expect("ask");
        std::fs::write(dir.join(format!("{id}.code")), "4821").expect("code");
        std::fs::write(dir.join("fedcba9876543210fedcba9876543210.code"), "4821").expect("orphan");
        assert!(pending(&dir).is_empty() && std::fs::read_dir(&dir).expect("dir").count() == 0, "nothing left on disk");
        assert!(!valid("12") && !valid("12 34") && !valid("1234567890123") && valid("483921"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
