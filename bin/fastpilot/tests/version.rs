//! v0.4: one version everywhere it lives. The release workflow refuses a tag when the npm package
//! and Cargo disagree; this catches it before a release.

#[test]
fn the_npm_package_and_cargo_have_the_same_version() {
    let pkg: serde_json::Value = serde_json::from_str(include_str!("../../../packaging/npm/package.json")).expect("package.json");
    assert_eq!(pkg["version"], env!("CARGO_PKG_VERSION"));
    // B1: the Claude Code plugin carries the release version too.
    let plugin: serde_json::Value = serde_json::from_str(include_str!("../../../.claude-plugin/plugin.json")).expect("plugin.json");
    assert_eq!(plugin["version"], env!("CARGO_PKG_VERSION"), ".claude-plugin/plugin.json version");
    // B2: the MCP Registry entry and the npm package that it names.
    let server: serde_json::Value = serde_json::from_str(include_str!("../../../server.json")).expect("server.json");
    assert_eq!(server["version"], env!("CARGO_PKG_VERSION"), "server.json version");
    assert_eq!(server["packages"][0]["version"], env!("CARGO_PKG_VERSION"), "server.json package version");
    assert_eq!(server["packages"][0]["identifier"], pkg["name"]);
    assert_eq!(server["name"], pkg["mcpName"], "the registry checks npm's mcpName");
    assert_eq!(server["name"], "io.github.autobot-lab/fastpilot");
    assert!(server["description"].as_str().is_some_and(|d| d.chars().count() <= 100), "registry limit: 100 characters");
    assert_eq!(server["packages"][0]["packageArguments"], serde_json::json!([{ "type": "positional", "value": "mcp" }]));
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.7.1");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).arg("--version").output().expect("run");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "fastpilot 0.7.1");
}

/// FastTest v0.7.2 (Low): the npm tarball had no `LICENSE`, though `license` says MIT. The package
/// carries the repository's license file, the same text, and `npm pack` puts it in.
#[test]
fn the_npm_package_ships_the_mit_license() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let pkg: serde_json::Value = serde_json::from_str(include_str!("../../../packaging/npm/package.json")).expect("package.json");
    assert_eq!(pkg["license"], "MIT");
    let license = std::fs::read_to_string(format!("{root}/packaging/npm/LICENSE")).expect("packaging/npm/LICENSE");
    assert_eq!(license, std::fs::read_to_string(format!("{root}/LICENSE")).expect("LICENSE"), "the same text as the repository's LICENSE");
    assert!(license.starts_with("MIT License"));
    let Ok(out) = std::process::Command::new("npm").args(["pack", "--dry-run", "--json", "--ignore-scripts"]).current_dir(format!("{root}/packaging/npm")).output() else {
        return eprintln!("skipping npm pack: no npm");
    };
    let listed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("npm pack --json");
    let files: Vec<&str> = listed[0]["files"].as_array().expect("files").iter().filter_map(|f| f["path"].as_str()).collect();
    assert!(files.contains(&"LICENSE"), "npm pack puts LICENSE in: {files:?}");
}
