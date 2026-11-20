# iPhone testing without a fastpilot checkout

`fastpilot setup ios` prepares iPhone testing for installs from npm or a GitHub release:

```
Setting up iPhone testing   (about 5 minutes, once)
✓ Xcode found (Xcode 16.1)
▶ Building the test runner… (the long part)
✓ Test runner built
▶ Starting an iPhone simulator…
✓ iPhone 16 simulator is running
```

## What it does

1. Checks Xcode (`xcodebuild -version`).
2. Writes the test runner's Xcode project, which is built into the fastpilot binary (six small text
   files: `ios-runner/` and the demo app in `examples/demo-ios/`), to `~/.fastpilot/ios-runner/src`.
3. Builds it once with `xcodebuild build-for-testing` into `~/.fastpilot/ios-runner/build`. This is
   where fastpilot looks for the runner when it was not built from a checkout and `ios_runner` is
   not set. The build is repeated only when fastpilot or Xcode changes.
4. Starts an iPhone simulator (`ios_device`, default iPhone 16) if none is running.

`fastpilot setup` without a platform offers the same build as one of its fixes.

## Why it is built on your Mac

We chose to build the runner locally instead of shipping one we prebuilt:

- An XCUITest runner is tied to the Xcode and iOS SDK that built it. A prebuilt one stops working
  when your Xcode is newer or older than ours. Built on your Mac, it always matches.
- A prebuilt runner would also have to be signed and notarized to pass Gatekeeper.
- It needs nothing you do not already need for iPhone simulators: Xcode.
- It costs a few minutes once, and about 200 MB in `~/.fastpilot/ios-runner`.

## When it fails

Each known problem says what to do next:

| Problem | Message |
|---|---|
| No Xcode | Could not set up iPhone testing. Xcode is not installed. Install it from the App Store, then run `fastpilot setup ios` again. |
| Command line tools selected | … run `sudo xcode-select -s /Applications/Xcode.app`, then run `fastpilot setup ios` again. |
| License not accepted | … open Xcode once and accept it (or `sudo xcodebuild -license accept`) … |
| No iPhone simulator runtime | … open Xcode › Settings › Components, add an iOS simulator … |
| Any other build error | The test runner did not build, the last lines from Xcode, and the full log in `~/.fastpilot/ios-runner/build.log`. |

Physical iPhones still need a signed build from a checkout (`ios-runner/build.sh --device`), see
[real-devices.md](real-devices.md).
