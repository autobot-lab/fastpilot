//! v0.4: one version everywhere it lives. The release workflow refuses a tag when the npm package
//! and Cargo disagree; this catches it before a release.

#[test]
fn the_npm_package_and_cargo_have_the_same_version() {
    let pkg: serde_json::Value = serde_json::from_str(include_str!("../../../packaging/npm/package.json")).expect("package.json");
    assert_eq!(pkg["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.4.0");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).arg("--version").output().expect("run");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "fastpilot 0.4.0");
}
