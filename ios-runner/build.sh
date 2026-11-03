#!/usr/bin/env bash
# Builds the fastpilot iOS runner (XCUITest bundle + host app) and the demo app.
#   ios-runner/build.sh            simulator (default): prints the .xctestrun and the demo .app
#   ios-runner/build.sh --device   real devices: signed with FASTPILOT_TEAM (Apple team id);
#                                  fastpilot installs and runs it with go-ios
set -euo pipefail
cd "$(dirname "$0")"

if [[ "${1:-}" == "--device" ]]; then
  : "${FASTPILOT_TEAM:?set FASTPILOT_TEAM to your Apple development team id}"
  xcodebuild build-for-testing \
    -project FastpilotRunner.xcodeproj \
    -scheme FastpilotRunner \
    -sdk iphoneos \
    -destination 'generic/platform=iOS' \
    -derivedDataPath build \
    -allowProvisioningUpdates \
    DEVELOPMENT_TEAM="$FASTPILOT_TEAM" CODE_SIGN_STYLE=Automatic \
    -quiet
  products="$PWD/build/Build/Products/Debug-iphoneos"
  echo "RUNNER_APP=$products/FastpilotRunner-Runner.app"
  echo "HOST_APP=$products/FastpilotHost.app"
  echo "DEMO_APP=$products/FastpilotDemo.app"
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
xctestrun=$(ls -t "$products"/FastpilotRunner_*.xctestrun | head -1)
echo "XCTESTRUN=$xctestrun"
echo "DEMO_APP=$products/Debug-iphonesimulator/FastpilotDemo.app"
