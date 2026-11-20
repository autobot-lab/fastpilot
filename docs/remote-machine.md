# fastpilot on another computer over SSH, and a lighter iOS simulator

There are two ways to use another computer:

| Way | fastpilot runs on | Device/Chrome on | When to use it |
|---|---|---|---|
| A. Only the device on another computer | laptop | other computer | An Android emulator or Chrome on a Linux PC; see Guide › [Devices on another computer](guide.md#devices-on-another-computer) |
| B. fastpilot on another computer | other computer | other computer | iOS (needs a Mac) from a non-Mac laptop, or all devices on one powerful computer |

This page covers way B, then [simslim](#a-lighter-ios-simulator-simslim) to save iOS simulator memory.

> **Testing status.** The parts marked **(not tested yet)** have not been run by the fastpilot team
> over a real SSH connection; the commands follow how fastpilot works and the docs of the related tools. What has been
> tested: `fastpilot mcp` over stdio, `fastpilot ui` with a fixed port (forwarding to the same port is accepted,
> to a different port it is refused with "wrong host"), and adb/Chrome on another `127.0.0.1` endpoint (test `remote_web.rs`).
> Report any step that does not match.

## B. fastpilot on another computer

The fastpilot MCP server talks over stdin/stdout, so SSH can serve directly as the "cable": Claude Code on the
laptop runs `ssh`, and `ssh` runs `fastpilot mcp` on that computer. No port is opened.

### 1. Prepare that computer

```bash
# on the other computer (a Mac for iOS, a Mac/Linux for web and Android)
npm install -g fastpilot            # or install.sh / build from source
fastpilot doctor                     # everything you plan to use must be ✓
command -v fastpilot                 # note its full path
```

- Log in to SSH with a **key**, not a password (`ssh-copy-id user@other-computer`), so Claude Code does not
  get stuck at a password prompt.
- Non-interactive SSH often does not load `PATH` from `.zshrc`/`.bashrc`, so use the **full path** to
  `fastpilot` from the step above. **(not tested yet)**
- Settings (`~/.fastpilot/config.toml`), saved login state, secrets, screenshots, and reports
  are all on that computer, not on the laptop.

### 2. Register it with Claude Code on the laptop **(not tested yet)**

```bash
claude mcp add fastpilot -- ssh -T user@other-computer /opt/homebrew/bin/fastpilot mcp
```

- `-T`: no terminal, so stdin/stdout stay clean for MCP.
- fastpilot reads Secrets (`${DEMO_PASS}` and so on) from the env **on that computer**. Do not put their values in the
  `claude mcp add` command (it is stored in the laptop's configuration and visible in the process list). Write a wrapper
  script on that computer that only its owner can read, then register that script:

  ```bash
  # on the other computer: ~/bin/fastpilot-mcp  (chmod 700)
  #!/bin/sh
  export FASTPILOT_VARS=DEMO_PASS
  export DEMO_PASS='...'
  exec /opt/homebrew/bin/fastpilot mcp
  ```

  ```bash
  # on the laptop
  claude mcp add fastpilot -- ssh -T user@other-computer bin/fastpilot-mcp
  ```
- `fastpilot mcp` stops when the client disconnects, and at that moment it closes its Chrome, agents, and runners.
  If SSH drops, Claude Code starts the server again on the next call; sessions that already existed must be
  started again.

### 3. The `fastpilot ui` page from the laptop

The UI page only listens on `127.0.0.1` on that computer and only accepts its own `Host` `127.0.0.1:<port>` /
`localhost:<port>`. So **the port number on the laptop must be the same** as the port on that
computer; a tunnel to a different local port is refused with "wrong host".

```bash
# on the other computer
fastpilot ui --no-open --port 7777        # prints the link http://127.0.0.1:7777/?code=...
```

```bash
# on the laptop
ssh -N -L 7777:127.0.0.1:7777 user@other-computer
```

Open the printed link (with `?code=`) in the laptop's browser. The code works once and is valid for 10 minutes;
run `fastpilot ui` again if it expires. **(SSH not tested yet; port forwarding and the one-time code are tested)**

### 4. iOS on another Mac **(not tested yet)**

- The simulator and the XCUITest runner are started with `xcrun simctl` and `xcodebuild`. This usually needs the
  Mac's user to be **logged in to the desktop** (an active GUI session), not only over SSH. Keep the user logged in
  (the screen may be locked) and turn off automatic sleep.
- Build the runner once on that Mac: `ios-runner/build.sh` from a checkout, or use a release build.
- The first time, macOS may show permission dialogs (Developer Tools, Keychain) on that Mac's screen. These dialogs
  are not visible over SSH; deal with them once, directly on the Mac.
- Physical iPhone: plug it into that Mac, then follow [real-devices.md](real-devices.md).

## A lighter iOS simulator: simslim

[simslim](https://github.com/MobAI-App/simslim) (MIT, third party, not part of fastpilot) turns off
unused background services inside one simulator. According to its README, one iPhone simulator drops from
about 4.0 GB to about 0.9 GB of memory (258 → 70 processes, M1 Pro 16 GB), so more simulators fit
at the same time. **(not tested with fastpilot yet)**

```bash
brew install mobai-app/tap/simslim
xcrun simctl list devices booted          # or: simslim list --booted
simslim on <udid>                         # slim down, then reboot that simulator
simslim status <udid>
simslim off <udid>                        # back to normal
```

- Only the named simulator changes, not the Mac.
- A permanent slim-down (one that survives reboots) needs runtime **iOS 18.5 or newer**. On iOS 17.x–18.3 only
  `simslim on <udid> --no-reboot` works, and it is lost when the simulator reboots.
- **Erase All Content and Settings**, `simctl erase`, or a new simulator brings it back to normal.
  fastpilot itself does not erase simulators.
- Features that are **off** in a slimmed simulator (from the simslim README), and the categories you need to keep
  with `--except` if the app you test uses them:

  | Needs | Keep |
  |---|---|
  | Push notification | `--except store` or `--keep com.apple.apsd` |
  | StoreKit testing | `--except store` |
  | Universal links | `--except web` |
  | Spotlight search, "Recents" in the file picker | `--except search` |
  | Photos/Contacts/Calendar picker | `--except photos,pim` |
  | MDM / configuration profiles | `--except mdm` |

  Check before running tests: `simslim doctor <udid> --requires push,universal-links`.
- Start the slimmed simulator before the tests: fastpilot uses the simulator that is running.
  To pick one of several, give its UDID in `endpoint` at `session start`; the simulator that is
  booted automatically when none is running is set by `FASTPILOT_IOS_DEVICE` (built-in default `iPhone 16`).
