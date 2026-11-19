## What's new in 0.2.0

**Upgrading from the Chrome extension 0.1.0:** the extension now has a fixed ID and gets its relay
token through native messaging. Reload it once (chrome://extensions → fastpilot → reload).

### New
- **`fastpilot ui`**: a local page (127.0.0.1 only) with **Live**, **Settings** and **Device**.
  - The printed link works once and for 10 minutes; it is traded for a session cookie, and it is
    never passed in another process' arguments. Fonts and assets are built into the binary (no CDN).
  - Settings: AI models for `fastpilot run`, limits and permissions, secret names (values never shown).
    A broken `config.toml` is shown with its line and column, and nothing is saved until it is fixed.
  - Device: `doctor` results with a re-check button.
  - Live: the running sessions of `fastpilot mcp`, the shell CLI and `fastpilot run`, each with its
    steps (done, running, failed, blocked, and what a limit *will* block), and the screen of web
    sessions on fastpilot's own Chrome, read-only and only while the panel is open.
- **New limits**, enforced by the engine (an agent can only make them stricter):
  `block_actions` (taps on text containing a word are always refused, even with `confirm: true`),
  `allow_app_install`, `allow_app_reset`, `allow_physical_devices`, `allow_device_settings`,
  `allow_eval_script`, `upload_dirs`, `max_steps` and `test_timeout` for `fastpilot run`.
  New steps `installApp` / `uninstallApp`.
- **`launchApp` with `arguments` and `env`** (E1): Android intent extras, iOS launch arguments and
  environment. Env values always show as `***`; names such as `DYLD_*` and `FASTPILOT_*` are refused.
- **Ambiguous selectors** (E2): an `id` or a nameless `role` that matches several elements fails with
  the candidates instead of picking the first; `index: N` picks one.
- **Devices and Chrome on another machine** (D2): `adb_server` / `FASTPILOT_ADB_SERVER` and
  `chrome_endpoint` / `FASTPILOT_CHROME_ENDPOINT`, through an SSH tunnel (see the README).
- **Agent Skill** (B4): `skills/fastpilot/SKILL.md` teaches an AI agent how to test with fastpilot.
- **Recovery**: when fastpilot's Chrome crashes or is closed, its sessions end with a clear message
  and the next start launches a new Chrome.
- `fastpilot doctor` exits non-zero when web is not ready; `fastpilot ui --help`; clearer errors for
  ports and settings.

### Security fixes
- Android: values passed to the device shell (`appId`, `launchApp`, `uninstallApp`, `setLocale`,
  `setPermissions`, `pushNotification`) are quoted or refused; shell injection is no longer possible.
- `tapOn: { point: [x, y] }` is checked against `block_actions` and `confirm_actions` like a named tap.
- Chrome extension relay: a random 256-bit token per session is required on both routes; the token
  file only exists while waiting for the extension.
- Guard rails, program paths and remote endpoints are pinned when fastpilot starts; a looser value
  written to the settings file applies only after a restart. Remote endpoints that are not loopback
  get a warning.
- Settings values fastpilot would misread stop it with the key and the reason.

### Known limitations
- After a page is scrolled, a `tapOn: { point }` taken from snapshot coordinates can miss but still
  report success (since 0.1.2). Use refs or text selectors.
- `block_actions` filters taps: `pressKey` (e.g. Enter on a focused button) and `evalScript` are not
  covered. Set `allow_eval_script = false` when you rely on `block_actions`.
- A `point` tap on an element without a name is refused while `block_actions` or `confirm_actions`
  is set (it cannot be told what it is).
- The Chrome extension still connects when the relay gives no token (Low).
- On a slow emulator the soft keyboard can take longer than the 5 s action limit; run emulators
  headless (`emulator -avd <name> -no-window`).

Install (macOS arm64/x86_64, Linux x86_64):

```bash
curl -fsSL https://raw.githubusercontent.com/autobot-lab/fastpilot/main/scripts/install.sh | sh
npm install -g fastpilot
```

Every archive has a `.sha256` next to it; the install script and the npm package check it.
