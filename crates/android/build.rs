//! Embeds `agent-android/out/fastpilot-agent.apk` (built by `agent-android/build.sh`).
//! Without it the crate still builds; starting an Android session then reports how to build it.

use std::path::Path;

fn main() {
    let apk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agent-android/out/fastpilot-agent.apk");
    println!("cargo:rerun-if-changed={}", apk.display());
    let out = Path::new(&std::env::var("OUT_DIR").unwrap_or_else(|_| ".".into())).join("fastpilot-agent.apk");
    let bytes = std::fs::read(&apk).unwrap_or_default();
    if bytes.is_empty() {
        println!("cargo:warning=agent-android/out/fastpilot-agent.apk missing; run agent-android/build.sh for Android support");
    }
    if let Err(e) = std::fs::write(&out, bytes) {
        println!("cargo:warning=cannot write {}: {e}", out.display());
    }
}
