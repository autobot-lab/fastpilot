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
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.4.2");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).arg("--version").output().expect("run");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "fastpilot 0.4.2");
}
