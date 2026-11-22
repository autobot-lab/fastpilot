//! S8 (v0.7): "Run on all platforms" on a saved test. The platforms come from the current app's
//! web, Android and iPhone entries (E6 `targets`); the test file is not edited. A platform the app
//! does not have is an "Add Android app" button, not an error. The run is `fastpilot run <test>
//! --all-platforms`, so its report is PF5's side by side, in Live and History like any run.

use fastpilot_core::Platform;
use serde_json::{Value, json};

use crate::run::{AppTargets, platform_name, short_kind};

/// A saved test's id: what Chat writes as `tests/<id>.md`.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 48 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// The saved test `id`: in the current app's tests, else in this project's `tests/`.
fn test_file(id: &str) -> Result<std::path::PathBuf, String> {
    if !valid_id(id) {
        return Err("That is not a saved test.".into());
    }
    let dirs = crate::apps::current().map(|a| a.dir().join("tests")).into_iter().chain([crate::chat::tests_dir()]);
    for d in dirs {
        let f = d.join(format!("{id}.md"));
        if f.is_file() {
            return Ok(f);
        }
    }
    Err("That saved test was not found.".into())
}

fn targets() -> Option<(crate::apps::App, AppTargets)> {
    let app = crate::apps::current()?;
    let t = AppTargets { web: app.targets.web.clone(), android: app.targets.android.clone(), ios: app.targets.ios.clone() };
    Some((app, t))
}

/// What the button would run: the platforms, and an add button for each missing one.
pub fn info(id: &str) -> Result<Value, String> {
    test_file(id)?;
    let Some((app, t)) = targets() else {
        return Ok(json!({ "test": id, "app": null, "platforms": [], "missing": [], "note": "Add your app with the App switcher first." }));
    };
    let missing: Vec<Value> = t
        .missing()
        .into_iter()
        .map(|p: Platform| json!({ "kind": p.to_string(), "button": format!("Add {} app", short_kind(p)), "href": format!("#/apps/add?app={}&kind={p}", app.id) }))
        .collect();
    Ok(json!({ "test": id, "app": app.name, "platforms": t.kinds().iter().map(|p| platform_name(*p)).collect::<Vec<_>>(), "missing": missing }))
}

/// Start the run (a person's click on the dashboard): `fastpilot run <test> --all-platforms`.
pub fn start(id: &str) -> Result<String, String> {
    let file = test_file(id)?;
    let (_, t) = targets().ok_or("Add your app with the App switcher first.")?;
    let kinds = t.kinds();
    if kinds.len() < 2 {
        return Err(format!("This app runs only on {}. Add another kind of app to compare.", kinds.first().map_or("one platform", |p| platform_name(*p))));
    }
    let log = crate::apps::root().join("logs").join(format!("run-all-{id}.log"));
    // Owner-only, like the rest of an app's data (Security, #180).
    crate::filestore::ensure_dir(log.parent().unwrap_or(&crate::apps::root()), true).map_err(|e| format!("Could not prepare the run: {e}"))?;
    let out = std::fs::OpenOptions::new().create(true).append(true).open(&log).map_err(|e| format!("Could not prepare the run: {e}"))?;
    let err = out.try_clone().map_err(|e| format!("Could not prepare the run: {e}"))?;
    let exe = std::env::current_exe().map_err(|e| format!("The fastpilot program file was not found: {e}"))?;
    let mut child = std::process::Command::new(exe)
        .args(["run"])
        .arg(&file)
        .arg("--all-platforms")
        // A person clicked: the first iPhone test may build the runner (V042).
        .env("FASTPILOT_RUN_BY_PERSON", "1")
        .stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|e| format!("The run could not start: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    let names: Vec<&str> = kinds.iter().map(|p| platform_name(*p)).collect();
    Ok(format!("Running on {}: watch it in Live. The side-by-side report shows in History.", crate::side::and_list(&names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())))
}
