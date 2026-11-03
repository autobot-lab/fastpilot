#!/usr/bin/env bash
# Builds the fastpilot iOS runner (XCUITest bundle + host app).
#   ios-runner/build.sh                          simulator (default), plus the demo app
#   ios-runner/build.sh --device <udid> [--team T]
#        a physical iPhone: scheme FastpilotRunnerDevice (host + runner only, no demo, so it uses
#        2 of the 3 app slots of a free Apple ID), signed automatically for team T
#        (FASTPILOT_IOS_TEAM), bundle ids under FASTPILOT_IOS_BUNDLE_PREFIX (required, unique per
#        team, e.g. com.yourname.fastpilot), output in ios-runner/build-device.
set -euo pipefail
cd "$(dirname "$0")"

if [[ "${1:-}" == "--device" ]]; then
  udid="${2:?usage: build.sh --device <udid> [--team T]}"
  team="${FASTPILOT_IOS_TEAM:-}"
  if [[ "${3:-}" == "--team" ]]; then team="${4:?--team needs a team id}"; fi
  if [[ -z "$team" ]]; then
    # Only when Xcode knows exactly one team.
    teams=$(defaults read com.apple.dt.Xcode IDEProvisioningTeamByIdentifier 2>/dev/null | grep -o 'teamID = [A-Z0-9]*' | sort -u | awk '{print $3}' || true)
    if [[ $(printf '%s\n' "$teams" | grep -c .) -eq 1 ]]; then team="$teams"; fi
  fi
  [[ -n "$team" ]] || { echo "set FASTPILOT_IOS_TEAM (Xcode > Settings > Accounts shows it) or pass --team" >&2; exit 2; }
  prefix="${FASTPILOT_IOS_BUNDLE_PREFIX:?set FASTPILOT_IOS_BUNDLE_PREFIX, e.g. com.yourname.fastpilot}"
  echo "note: a free Apple ID can register about 10 App IDs per 7 days; keep FASTPILOT_IOS_BUNDLE_PREFIX stable." >&2
  xcodebuild build-for-testing \
    -project FastpilotRunner.xcodeproj \
    -scheme FastpilotRunnerDevice \
    -destination "id=$udid" \
    -derivedDataPath build-device \
    -allowProvisioningUpdates -allowProvisioningDeviceRegistration \
    CODE_SIGN_STYLE=Automatic DEVELOPMENT_TEAM="$team" CODE_SIGN_IDENTITY="Apple Development" \
    FASTPILOT_BUNDLE_PREFIX="$prefix" \
    -quiet
  products="$PWD/build-device/Build/Products"
  xctestrun=$(ls -t "$products"/*iphoneos*.xctestrun | head -1)
  echo "XCTESTRUN=$xctestrun"
  for app in "$products"/Debug-iphoneos/*.app; do
    expiry=$(security cms -D -i "$app/embedded.mobileprovision" 2>/dev/null | plutil -extract ExpirationDate raw - 2>/dev/null || echo unknown)
    echo "PROFILE $(basename "$app") expires $expiry"
  done
  exit 0
fi

xcodebuild build-for-testing \
  -project FastpilotRunner.xcodeproj \
  -scheme FastpilotRunner \
  -sdk iphonesimulator \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath build \
  -quiet

products="$PWD/build/Build/Products"
xctestrun=$(ls -t "$products"/*iphonesimulator*.xctestrun | head -1)
echo "XCTESTRUN=$xctestrun"
echo "DEMO_APP=$products/Debug-iphonesimulator/FastpilotDemo.app"
