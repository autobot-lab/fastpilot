#!/usr/bin/env bash
# Builds the fastpilot agent APK and the demo APK with plain SDK tools (no Gradle).
# Outputs: agent-android/out/fastpilot-agent.apk, examples/demo-android/out/fastpilot-demo.apk
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$HERE")"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
PLATFORM="${FASTPILOT_PLATFORM:-android-35}"
ANDROID_JAR="$SDK/platforms/$PLATFORM/android.jar"
MIN_SDK=24
TARGET_SDK=35

# Highest installed build-tools that has every tool we need (skips -rc dirs only if incomplete).
BT=""
for d in $(ls -1 "$SDK/build-tools" | sort -V -r); do
  if [[ -x "$SDK/build-tools/$d/aapt2" && -x "$SDK/build-tools/$d/d8" && -x "$SDK/build-tools/$d/zipalign" && -x "$SDK/build-tools/$d/apksigner" ]]; then
    BT="$SDK/build-tools/$d"; break
  fi
done
[[ -n "$BT" ]] || { echo "no usable build-tools in $SDK/build-tools" >&2; exit 1; }
[[ -f "$ANDROID_JAR" ]] || { echo "missing $ANDROID_JAR" >&2; exit 1; }
echo "build-tools: $BT"

KEYSTORE="$HERE/debug.keystore"
if [[ ! -f "$KEYSTORE" ]]; then
  keytool -genkeypair -keystore "$KEYSTORE" -storepass android -keypass android \
    -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 \
    -dname "CN=fastpilot debug" >/dev/null
fi

# build_apk <project dir> <apk name>
build_apk() {
  local dir="$1" name="$2"
  local build="$dir/build" out="$dir/out"
  rm -rf "$build"; mkdir -p "$build/classes" "$build/dex" "$out"
  javac --release 11 -nowarn -encoding UTF-8 -classpath "$ANDROID_JAR" -d "$build/classes" \
    $(find "$dir/src" -name '*.java')
  "$BT/d8" --release --min-api "$MIN_SDK" --lib "$ANDROID_JAR" --output "$build/dex" \
    $(find "$build/classes" -name '*.class')
  "$BT/aapt2" link -o "$build/unsigned.apk" -I "$ANDROID_JAR" --manifest "$dir/AndroidManifest.xml" \
    --min-sdk-version "$MIN_SDK" --target-sdk-version "$TARGET_SDK"
  (cd "$build/dex" && zip -q -j "$build/unsigned.apk" classes.dex)
  "$BT/zipalign" -f -p 4 "$build/unsigned.apk" "$build/aligned.apk"
  "$BT/apksigner" sign --ks "$KEYSTORE" --ks-pass pass:android --key-pass pass:android \
    --out "$out/$name" "$build/aligned.apk"
  echo "built $out/$name"
}

build_apk "$HERE" fastpilot-agent.apk
build_apk "$ROOT/examples/demo-android" fastpilot-demo.apk
