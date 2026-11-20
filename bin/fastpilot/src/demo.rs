//! `fastpilot demo [folder]` (F2): the demo pages, example test cases and an MCP config, written to
//! a new folder, so a first test runs without a checkout of the repository. `--android` installs
//! the demo app (built into the binary) on the attached device or emulator.
//!
//! Everything is embedded at build time: the pages and test cases with `include_str!` (the very
//! files of the repository; test cases point at `demo/` instead of `examples/demo/`), the APK
//! through `build.rs`.

use std::path::{Path, PathBuf};

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The demo pages, as in `examples/demo/`.
pub const PAGES: &[(&str, &str)] = &[
    ("login.html", include_str!("../../../examples/demo/login.html")),
    ("widgets.html", include_str!("../../../examples/demo/widgets.html")),
    ("pin.html", include_str!("../../../examples/demo/pin.html")),
    ("secret.html", include_str!("../../../examples/demo/secret.html")),
    ("maestro.html", include_str!("../../../examples/demo/maestro.html")),
];

/// The example test cases of `tests/`, with the platform shown in the output.
pub const TESTS: &[(&str, &str, &str)] = &[
    ("login.md", include_str!("../../../tests/login.md"), "web"),
    ("login-android.md", include_str!("../../../tests/login-android.md"), "android"),
    ("login-cross-platform.md", include_str!("../../../tests/login-cross-platform.md"), "web + android"),
];

/// The Android demo app (empty when it was not built before this binary).
const APK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fastpilot-demo.apk"));
const APP: &str = "com.fastpilot.demo";

/// The public demo password of the demo pages and app (not a secret).
pub const DEMO_PASS: &str = "rahasia123";

/// A test case as written to the demo folder: pages are under `demo/`.
pub fn test_case(text: &str) -> String {
    text.replace("examples/demo/", "demo/")
}

/// Started through `npx` (the npm launcher sets FASTPILOT_VIA_NPX; npm itself sets npm_command).
pub fn via_npx() -> bool {
    std::env::var("FASTPILOT_VIA_NPX").is_ok_and(|v| v == "1") || std::env::var("npm_command").is_ok_and(|v| v == "exec")
}

/// How an MCP client starts this fastpilot: `fastpilot mcp`, or `npx -y fastpilot mcp`.
pub fn mcp_command() -> (String, Vec<String>) {
    if via_npx() {
        ("npx".into(), vec!["-y".into(), "fastpilot".into(), "mcp".into()])
    } else {
        ("fastpilot".into(), vec!["mcp".into()])
    }
}

/// `.mcp.json` of the demo folder: fastpilot with the demo password.
pub fn mcp_json() -> String {
    let (command, args) = mcp_command();
    let v = serde_json::json!({
        "mcpServers": {
            "fastpilot": {
                "type": "stdio",
                "command": command,
                "args": args,
                "env": { "FASTPILOT_VARS": "DEMO_PASS", "DEMO_PASS": DEMO_PASS },
            }
        }
    });
    format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default())
}

/// Every file of the demo folder, relative path and contents.
pub fn files() -> Vec<(PathBuf, Vec<u8>)> {
    let mut out: Vec<(PathBuf, Vec<u8>)> = PAGES.iter().map(|(n, t)| (Path::new("demo").join(n), t.as_bytes().to_vec())).collect();
    out.extend(TESTS.iter().map(|(n, t, _)| (Path::new("tests").join(n), test_case(t).into_bytes())));
    out.push((PathBuf::from(".mcp.json"), mcp_json().into_bytes()));
    out
}

const USAGE: &str = "usage: fastpilot demo [folder] [--force] [--android [--device SERIAL]]
  folder     where to write the demo (default ./fastpilot-demo)
  --force    write into a folder that already exists (files with the same name are replaced)
  --android  also install the demo app on the attached Android device or emulator";

pub async fn demo(args: &[String]) -> Res<()> {
    let mut folder: Option<String> = None;
    let (mut force, mut android, mut device) = (false, false, None::<String>);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--force" => force = true,
            "--android" => android = true,
            "--device" => device = Some(it.next().cloned().ok_or("--device butuh serial device, mis. emulator-5556")?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            s if s.starts_with('-') => {
                eprintln!("fastpilot demo: opsi `{s}` tidak dikenal\n\n{USAGE}");
                std::process::exit(2);
            }
            s => folder = Some(s.to_owned()),
        }
    }
    let shown = folder.clone().unwrap_or_else(|| "fastpilot-demo".into());
    let dir = PathBuf::from(&shown);
    // `--android` after the folder was made: only the app.
    let write = !(android && dir.exists() && !force);
    if write {
        if dir.exists() && !force {
            eprintln!("Folder {} sudah ada; tidak ada yang diubah.\nPakai folder lain (fastpilot demo <folder>) atau --force untuk menulis ulang isinya.", dir.display());
            std::process::exit(1);
        }
        write_folder(&dir, &shown)?;
    }
    let installed = install_android(android, device.as_deref()).await;
    if write {
        let cd = if shown.contains(char::is_whitespace) { format!("cd \"{shown}\"") } else { format!("cd {shown}") };
        println!("\nCoba sekarang:\n  {cd} && claude\n  lalu ketik:\n  \"Pakai fastpilot, jalankan tests/login.md lalu report\"");
        println!("\nPassword demo (bukan rahasia): {DEMO_PASS}, sudah di .mcp.json");
    }
    if installed == Some(false) {
        std::process::exit(1);
    }
    Ok(())
}

fn write_folder(dir: &Path, shown: &str) -> Res<()> {
    let prefix = if Path::new(shown).is_absolute() || shown.starts_with('.') { shown.to_owned() } else { format!("./{shown}") };
    println!("Membuat folder demo: {prefix}");
    for (rel, bytes) in files() {
        let path = dir.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("tidak bisa membuat {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, bytes).map_err(|e| format!("tidak bisa menulis {}: {e}", path.display()))?;
    }
    let names: Vec<String> = PAGES.iter().take(2).map(|(n, _)| format!("demo/{n}")).collect();
    println!("  ✓ {:<42} {} halaman", format!("{}, …", names.join(", ")), PAGES.len());
    for (n, _, platform) in TESTS {
        println!("  ✓ {:<42} {platform}", format!("tests/{n}"));
    }
    println!("  ✓ {:<42} fastpilot + DEMO_PASS", ".mcp.json");
    Ok(())
}

/// The Android part. `None`: nothing to do (skipped); `Some(true)`: installed; `Some(false)`: failed.
async fn install_android(asked: bool, device: Option<&str>) -> Option<bool> {
    let serials: Vec<String> = match fastpilot_android::devices().await {
        Ok((attached, _)) => attached.into_iter().filter(|(_, st)| st == "device").map(|(s, _)| s).collect(),
        Err(_) => Vec::new(),
    };
    let wanted = device
        .map(str::to_owned)
        .or_else(|| ["FASTPILOT_ANDROID_SERIAL", "ANDROID_SERIAL"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())));
    if !asked {
        if serials.is_empty() {
            println!("\nAndroid (dilewati: tidak ada device)\n  nyalakan emulator lalu: fastpilot demo --android\n  → memasang fastpilot-demo.apk dari binary");
        } else {
            println!("\nAndroid: device tersambung ({})\n  pasang app demo: fastpilot demo --android", serials.join(", "));
        }
        return None;
    }
    let serial = match (wanted, serials.as_slice()) {
        (Some(w), list) if list.contains(&w) => w,
        (Some(w), _) => {
            println!("\nAndroid (dilewati: device {w} tidak tersambung)");
            return None;
        }
        (None, []) => {
            println!("\nAndroid (dilewati: tidak ada device)\n  nyalakan emulator lalu jalankan lagi: fastpilot demo --android");
            return None;
        }
        (None, [one]) => one.clone(),
        (None, many) => {
            println!("\nAndroid: ada {} device ({}); pilih satu: fastpilot demo --android --device <serial>", many.len(), many.join(", "));
            return Some(false);
        }
    };
    if APK.is_empty() {
        println!("\nAndroid: app demo tidak ikut di build fastpilot ini.\n  pakai build rilis (npm / install.sh), atau dari checkout: agent-android/build.sh lalu build ulang");
        return Some(false);
    }
    if !fastpilot_core::config::flag("allow_app_install") {
        println!("\nAndroid: memasang app dimatikan di Batasan (allow_app_install = false).\n  ubah di: fastpilot ui → Pengaturan › Batasan & izin");
        return Some(false);
    }
    let done = match fastpilot_android::Adb::new().await {
        Ok(adb) => adb.install(&serial, APK).await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match done {
        Ok(()) => {
            println!("  ✓ {serial} · {APP} terpasang\n  ketik di Claude: \"Pakai fastpilot, jalankan tests/login-android.md\"");
            Some(true)
        }
        Err(e) => {
            println!("  ✗ {serial}: app demo tidak bisa dipasang: {e}");
            Some(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    #[test]
    fn written_files_are_the_repository_files() {
        for (name, text) in PAGES {
            assert_eq!(*text, std::fs::read_to_string(format!("{ROOT}/examples/demo/{name}")).expect("page"), "{name}");
        }
        assert_eq!(PAGES.len(), std::fs::read_dir(format!("{ROOT}/examples/demo")).expect("dir").filter(|e| e.as_ref().is_ok_and(|e| e.path().extension().is_some_and(|x| x == "html"))).count(), "every demo page is embedded");
        for (name, text, _) in TESTS {
            let repo = std::fs::read_to_string(format!("{ROOT}/tests/{name}")).expect("test");
            assert_eq!(*text, repo, "{name}");
            assert_eq!(test_case(text), repo.replace("examples/demo/", "demo/"));
            assert!(!test_case(text).contains("examples/"), "{name} points into the demo folder");
        }
        let apk = std::fs::read(format!("{ROOT}/examples/demo-android/out/fastpilot-demo.apk")).unwrap_or_default();
        assert_eq!(APK, apk.as_slice(), "the APK built into the binary is the demo app");
    }

    #[test]
    fn mcp_json_runs_fastpilot_with_the_demo_password() {
        let v: serde_json::Value = serde_json::from_str(&mcp_json()).expect("json");
        let s = &v["mcpServers"]["fastpilot"];
        assert_eq!(s["env"]["FASTPILOT_VARS"], "DEMO_PASS");
        assert_eq!(s["env"]["DEMO_PASS"], DEMO_PASS);
        assert!(s["args"].as_array().is_some_and(|a| a.last().is_some_and(|x| x == "mcp")));
    }
}
