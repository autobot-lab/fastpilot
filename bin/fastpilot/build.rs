//! Embeds the Android demo app `examples/demo-android/out/fastpilot-demo.apk` (built by
//! `agent-android/build.sh`) for `fastpilot demo --android`. Without it the binary still builds;
//! `--android` then says how to get the app.

use std::path::Path;

fn main() {
    let apk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-android/out/fastpilot-demo.apk");
    println!("cargo:rerun-if-changed={}", apk.display());
    let out = Path::new(&std::env::var("OUT_DIR").unwrap_or_else(|_| ".".into())).join("fastpilot-demo.apk");
    let bytes = std::fs::read(&apk).unwrap_or_default();
    if let Err(e) = std::fs::write(&out, bytes) {
        println!("cargo:warning=cannot write {}: {e}", out.display());
    }
}
