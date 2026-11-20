#!/usr/bin/env sh
# Build a release archive: dist/fastpilot-<version>-<target>.tar.gz (+ .sha256).
#   scripts/package.sh                  host target
#   scripts/package.sh --target x86_64-apple-darwin
# The archive holds the binary, README.md, LICENSE, THIRD_PARTY_NOTICES and the licenses of the fonts built into
# `fastpilot ui` (SIL OFL 1.1, under licenses/). Android support is embedded when
# agent-android/out/fastpilot-agent.apk exists (run agent-android/build.sh first); the iOS
# runner is not shipped (it needs Xcode: ios-runner/build.sh from a checkout).
set -eu
cd "$(dirname "$0")/.."

target=""
if [ "${1:-}" = "--target" ]; then target="${2:?--target needs a triple}"; fi
[ -n "$target" ] || target="$(rustc -vV | sed -n 's/^host: //p')"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

if [ ! -s agent-android/out/fastpilot-agent.apk ]; then
  echo "warning: agent-android/out/fastpilot-agent.apk missing: this build has no Android support" >&2
fi
cargo build --release --locked -p fastpilot --target "$target"

name="fastpilot-$version-$target"
stage="dist/$name"
rm -rf "$stage" && mkdir -p "$stage"
cp "${CARGO_TARGET_DIR:-target}/$target/release/fastpilot" "$stage/"
cp README.md LICENSE THIRD_PARTY_NOTICES "$stage/"
mkdir -p "$stage/licenses"
cp bin/fastpilot/ui/fonts/*-OFL.txt "$stage/licenses/"
tar -C dist -czf "dist/$name.tar.gz" "$name"
rm -rf "$stage"
( cd dist && { sha256sum "$name.tar.gz" 2>/dev/null || shasum -a 256 "$name.tar.gz"; } > "$name.tar.gz.sha256" )
echo "dist/$name.tar.gz"
