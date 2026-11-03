#!/usr/bin/env bash
# Builds the fastpilot iOS runner (XCUITest bundle + host app) and the demo app for the simulator.
# Prints the .xctestrun to pass to `xcodebuild test-without-building` and the demo .app path.
set -euo pipefail
cd "$(dirname "$0")"

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
