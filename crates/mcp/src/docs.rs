//! On-demand reference served by the `docs` tool, so the client system prompt stays small.

pub fn topic(name: &str) -> Option<&'static str> {
    Some(match name {
        "dsl" => DSL,
        "selectors" => SELECTORS,
        "parallel" => PARALLEL,
        "examples" => EXAMPLES,
        "errors" => ERRORS,
        "memory" => MEMORY,
        "devices" => DEVICES,
        _ => return None,
    })
}

const DSL: &str = r#"# Flow DSL (Maestro-compatible subset + fastpilot extensions)

A flow is a YAML list. Each step has exactly one command. Quote numeric text ("+62812", "1.10").

| Step | Form | Platforms |
|---|---|---|
| launchApp | `launchApp: com.example.app` or `pkg/.Activity` (stop + start); `{ appId, arguments: { server: "https://staging", retries: 3, debug: true, tags: [a, b] }, env: { API_TOKEN: "${TOKEN}" } }`. `arguments`: android intent extras (text, whole number, true/false, text list without commas), ios launch arguments `-key value` (read with UserDefaults). `env`: ios only (the app's environment); values always show as `***`, recorded runs keep only `${NAME}` references, and `DYLD_*`, `LD_*`, `FASTPILOT_*`, `MALLOC*`, `OBJC_*`, `XPC_*` names are refused | android, ios |
| openUrl | `openUrl: https://...` or a local path (`examples/demo/login.html`) | web (android/ios: deep link) |
| tapOn | `tapOn: "Login"` / `{ ref: e3 }` / `{ id: btn }` / `{ role: button, name: "Login" }` / `{ point: [x, y] }`; add `confirm: true` for an intended risky action (confirm_actions; built in: hapus, delete, remove) (also longPressOn, doubleTapOn). Payment and account-deletion buttons are blocked by default (block_actions): do not retry or work around a block | all |
| inputText | `inputText: { into: "Email", text: "a@b.com" }` (replaces content); `inputText: "abc"` types into the focused field | all |
| assertVisible / assertNotVisible | `assertVisible: "Dashboard"` or `{ text: "Dashboard", timeout: 10000 }` (ms) | all |
| scroll | `scroll: down` or `{ direction: up, within: "Daftar" }` | all |
| swipe | `swipe: { from: [x1, y1], to: [x2, y2] }` | android, ios |
| back | `back` | all |
| pressKey | `pressKey: Enter` (Tab, Escape, Backspace, Delete, Space, ArrowUp/Down/Left/Right, Home, End, PageUp, PageDown); `{ key: Enter, confirm: true }` when Enter/Space may press a confirm_actions button and the test case means it | all |
| selectOption | `selectOption: { from: "Negara", option: "Indonesia" }` (native `<select>`) | web |
| uploadFile | `uploadFile: { into: "Lampiran", files: ["a.pdf"] }` | web |
| evalScript | `evalScript: "document.title"` (value is returned) | web, android WebView, iOS WKWebView |
| longPressOn / doubleTapOn | `longPressOn: "Item"` (selector) | all |
| scrollUntilVisible | `scrollUntilVisible: "Footer"` or `{ element: <selector>, direction: down, timeout: 20000 }` | all |
| hideKeyboard | `hideKeyboard` (web: blur) | all |
| eraseText | `eraseText: 3` (characters before the cursor) or `eraseText` (all) | all |
| copyTextFrom / pasteText | `copyTextFrom: { id: total }` then `pasteText` (types it into the focused field) | all |
| openLink | `openLink: "myapp://orders/1"` (deep link; same as openUrl) | all |
| stopApp / killApp | `stopApp` (app under test) or `stopApp: com.example.app` | android, ios |
| clearState | `clearState` / `clearState: com.example.app` (android `pm clear`; ios wipes the simulator data container; web: cookies + storage of the origin) | all |
| clearKeychain | `clearKeychain` | ios simulator |
| installApp / uninstallApp | `installApp: build/app.apk` (android) or `Demo.app` (ios simulator); `uninstallApp: com.example.app` | android, ios simulator |
| extendedWaitUntil | `{ visible: <selector>, timeout: 10000 }` or `{ notVisible: ... }` | all |
| waitForAnimationToEnd | `waitForAnimationToEnd` (never fails) | all |
| takeScreenshot | `takeScreenshot: checkout` → `.fastpilot/screenshots/checkout.jpg` | all |
| assertScreenshot | `assertScreenshot: home` or `{ name: home, ignore: ["Jam", { id: banner }], threshold: 0.1, maxDiff: 0.002, update: true }`: visual regression. First run (or `update: true`, or `FASTPILOT_UPDATE_BASELINES=1`) stores `.fastpilot/baselines/home.png`; later runs fail when more than `maxDiff` of the pixels moved by more than `threshold`, with `home.diff.png` (changed pixels red) and `home.actual.png`. `ignore` blanks dynamic parts (clock, ads) | all |
| saveState / loadState | `saveState: admin` after logging in → `.fastpilot/states/admin.json` (cookies + localStorage); `loadState: admin` restores it, so later tests skip the login (owner-only file, expires after 30 days, encrypted when `FASTPILOT_STATE_KEY` is set; also the `state` tool) | web |
| setViewport | `setViewport: iPhone 15` (also iPhone SE, iPhone Pro Max, Pixel 8, Galaxy S23, iPad), `{ width: 390, height: 844, deviceScaleFactor: 3, mobile: true, touch: true, userAgent: "..." }`, `desktop` to reset — the site's mobile layout | web |
| setLocation | `{ latitude: -6.2088, longitude: 106.8456 }` (web: geolocation override; android: emulator console; ios: simctl) | all (android: emulator) |
| setPermissions | `{ appId?, permissions: { camera: allow, location: deny, notifications: unset } }` (also microphone, contacts, photos, calendar, clipboard (web); a platform name passes through) | all (ios: no notifications) |
| setNetwork / setAirplaneMode | `setNetwork: offline \| online \| { latency: 300, download: 1600, upload: 750 }` (ms, kbps); `setAirplaneMode: enabled \| disabled` | web, android (throttling: emulator) |
| setLocale | `{ locale: id-ID, timezone: Asia/Jakarta }` (android: app locale + system timezone; ios: applied by relaunching the app) | all |
| pushNotification | `{ appId?, title, body, payload? }` (ios: simctl push APNs payload; android: a notification posted by the shell) | android, ios |
| startRecording / stopRecording | `startRecording: checkout` … `stopRecording` → `.fastpilot/recordings/checkout.mp4` (android screenrecord, ios simctl) or `.html` (web screencast filmstrip) | all |
| inputRandomText / Number / Email / PersonName | `inputRandomEmail` or `{ length: 10 }`; the value is returned | all |
| runFlow | `runFlow: login.yaml` (relative to the flow file, else the working dir; a Maestro config header `appId: ...` + `---` is skipped), or `{ file \| commands: [...], when: <condition>, env: { USER: a@b.com } }` | all |
| repeat | `{ times: 3, commands: [...] }` and/or `{ while: <condition>, commands: [...] }` (while stops after 1000) | all |
| retry | `{ maxRetries: 2, commands: [...] \| file: x.yaml }` (runs the block again on failure) | all |
| httpRequest | `{ method: POST, url: "${API}/users", headers: {...}, json: {...}, expect: 201, capture: { USER_ID: id, TOKEN: session.token }, secret: [USER_ID] }` — seed/teardown test data; no session needed | — |
| readOtp | `{ from: mailpit \| mailhog \| http \| sms, url, to: "a@b.com", after: "kode", length: 6, into: OTP, timeout: 30000 }` — waits for a one-time code, stores it in `${OTP}` | sms: android |
| session | `session: web1` (switch) or `session: { start: web1, platform: web, target: "...", reset: true }` (`reset: true` replaces a session of that name; a fresh app is `fastpilot run --reset` / `mcp --reset`); mobile: `device: ios-device:<udid>` / `simulator:<udid>` / `emulator:<serial>` / `android-device:<serial>` or a plain id / name (docs topic devices) | — |
| parallel | see docs(topic: parallel) | — |

Conditions (`when`, `while`), checked once on the current screen: `{ visible: <selector> }`,
`{ notVisible: <selector> }`, `{ platform: web | android | ios }`. JavaScript conditions
(`true:`), `runScript`, `assertTrue` and the `*WithAI` commands are not supported.
`env` variables stay set for the rest of the flow. Blocks may not contain `session`/`parallel`.

Test data: `httpRequest` captures JSON fields (dotted paths, `items.0.id`) into `${VARS}` for later
steps; a status other than `expect` (default 2xx) fails with the response. Captured names containing
TOKEN, SECRET, PASS, KEY, OTP, COOKIE, SESSION or AUTH (or listed in `secret`) and every `readOtp`
code are masked as `***` in everything returned, like `FASTPILOT_VARS`. `readOtp` takes the newest
matching message (Mailpit `/api/v1`, MailHog `/api/v2`, any URL's body, or the Android SMS inbox) and
the first run of `length` digits (default 4–8) after `after`; clear the inbox in setup if old codes
could match.

Settings: the `config` tool lists and changes settings (allowed_domains, confirm_actions,
headful, model_replay, ...); environment variables win. An agent may only make guard rails stricter.

Rules
- Auto-wait before/after every step (actionability, DOM/accessibility quiet, navigation and requests
  triggered by the action). There is no sleep.
- Element lookup retries until 5 s (assertVisible: 10 s); actions are never re-sent.
- The first failing step stops the run and returns reason, category, candidates and a snapshot.
- JS dialogs (alert/confirm/beforeunload) are accepted automatically and show up as `alert "confirm: ..."`.
- `${NAME}` is replaced from fastpilot's environment, only for names listed in `FASTPILOT_VARS`;
  values are masked (`***`) in everything returned.
- Android WebView content is merged into the snapshot and driven over CDP (tapOn, inputText, evalScript).
- iOS WKWebView content likewise, over the Web Inspector, when the app sets `webView.isInspectable = true` (iOS 16.4+; on an iPhone also Settings → Safari → Advanced → Web Inspector).
"#;

const SELECTORS: &str = r#"# Selectors

Resolution order: ref (`e12`, from the latest snapshot) → visible text / label → role + name → id → point.
Never XPath.

- Strings shaped like `e12` are refs; other strings are text.
- `id` matches DOM id (web), resource-id without package (android), accessibilityIdentifier (ios).
- Text matches exact, then case-insensitive exact, then case-insensitive "contains".
- tapOn prefers interactive nodes, inputText prefers text fields.
- Several exact matches: the first (topmost) wins. Several fuzzy matches: the step fails `ambiguous`
  and lists candidates; pick one by ref.
- `{ id: x }` matching several elements, or `{ role: button }` without `name` matching several: an
  action (tapOn, inputText, longPressOn, ...) fails `ambiguous` with the candidates instead of using the
  first. Pick one with `index` (0-based, document order): `{ role: button, index: 1 }`,
  `{ id: item, index: 0 }`, or by ref. `index` is only for `id` and for `role` without `name`.
  Existence checks (assertVisible, scrollUntilVisible, `when: visible`) pass with any match.
- Refs follow the element while it exists (session-scoped); a ref never jumps to another element.
- `snapshot { filter: "batal" }` finds nodes with their ancestors; `coords: true` adds boxes.
- `{ point: [x, y] }` uses the page coordinates `coords: true` shows (also after scrolling). The point
  must be on screen (scroll first, or tap by ref); the step reports the element it landed on. With
  block_actions / confirm_actions on, every element covering the point counts (a button inside a
  "Pembayaran" dialog too), and a point no element covers is refused.

Apps without accessibility labels (`low_label_coverage` in the snapshot):
- Jetpack Compose: set `Modifier.semantics { testTagsAsResourceId = true }` on the root and
  `Modifier.testTag("login")`, then use `{ id: login }`.
- Flutter: wrap widgets in `Semantics(label: ...)` / set `identifier`; Flutter builds semantics only
  while an accessibility client is connected (the agent's UiAutomation counts as one).
- Otherwise: `screenshot { annotate: som }` draws a box labelled with its ref on every target (web,
  android, ios) and lists the boxes; tap with `{ ref: eN }` or `{ point: [x, y] }`.
"#;

const PARALLEL: &str = r#"# Multi-session and parallel

```yaml
- session: { start: admin, platform: web, target: "https://admin.example.test" }
- session: { start: buyer, platform: web, target: "https://shop.example.test" }
- parallel:
    - session: admin
      steps:
        - tapOn: "Pesanan"
    - session: buyer
      steps:
        - tapOn: "Keranjang"
- session: admin
- assertVisible: "Pesanan"
```

- Each web session is its own BrowserContext (isolated cookies/storage) in one Chrome process.
- A branch is bound to one session; no `session` steps or nested `parallel` inside.
- Fail-fast: the first failing branch cancels the others; every touched session still reports state.
- Results are prefixed with the session id: `[admin] screen "..."`.
"#;

const EXAMPLES: &str = r#"# Examples

Login in one round-trip:

```yaml
- session: { start: web1, platform: web, target: "examples/demo/login.html" }
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- tapOn: "Masuk"
- assertVisible: "Budi"
```

Response:

```
ok 5/5 steps 1.70s
  ✓ 1 [web1] session start web1 (web) 1063ms
  ✓ 2 [web1] inputText "Email" "a@b.com" 78ms
  ✓ 3 [web1] inputText "Password" "${DEMO_PASS}" 84ms
  ✓ 4 [web1] tapOn "Masuk" 83ms
  ✓ 5 [web1] assertVisible "Budi" 385ms
[web1] screen "Dashboard" (file:///.../login.html)
e7 heading "Dashboard"
e8 text "Selamat datang,"
e9 text "Budi"
e10 button "Keluar"
```
"#;

const MEMORY: &str = r#"# Test runs and path memory

1. `session start` with `test_id` → the response carries a hint: the last passing flow for that test.
2. Send the hint as one `run_flow` with the same `test_id`. If a step fails, explore from the returned
   snapshot and continue with more `run_flow`/`assert` calls (all with `test_id`).
3. `report { test_id, status: pass|fail, notes }` closes the run: `.fastpilot/runs/<utc>-<test_id>/`
   gets `report.md`, a self-contained `report.html` (steps with timings, every run_flow response with
   its diff, assertions, failure and final screenshots, recordings), `steps.jsonl`, `assertions.json`,
   and `fail-N.jpg|txt` evidence.
4. Only a run reported `pass` with no failed step/assert becomes the new hint. Refs are saved as
   role+name selectors. A failed run only adds to the statistics.

`FASTPILOT_MEMORY=readonly` (CI: replay reviewed hints, never rewrite) or `off`. `FASTPILOT_DIR` moves `.fastpilot/`.

Statistics and triage (from the last 20 runs of the test):
- `report` returns the failure class (product | infra | agent | flaky) and `history: pass 18/20 (90%,
  95% CI 70–97%), pass^3 0.72, flake rate 11%` (flake rate = outcome flips between consecutive runs).
- A test whose last 10 runs hold ≥ 2 failures and ≥ 2 flips is auto-quarantined (released after 10
  clean runs). `report { ..., quarantine: true, notes: "why" }` quarantines by hand, `false` releases.
  Quarantined tests still run; the hint and report.md say so; their failures do not fail a suite.
"#;

const ERRORS: &str = r#"# Failure reasons

Every failure also has a category: `product` (screen not as expected: product bug, changed UI,
crash), `infra` (browser/device/transport), `agent` (the flow is wrong or uses an unsupported step).
Over runs a failure can also be classified `flaky` (passed on retry, or the test's history flips);
see docs(topic: memory). A check that fails right after a tap that reported success but changed
nothing on screen is `timing` (the tool acted too early, e.g. during an animation), not `product`:
rerun it or add `waitForAnimationToEnd` before the tap.

| Reason | Meaning | What to do |
|---|---|---|
| not_found | no node matched before the timeout | use a candidate ref, or assertVisible with a longer timeout |
| ambiguous | several fuzzy matches, or an `id` / nameless `role` on several elements in an action | pick one of the listed refs, or add `index: N` to an `id` / `role` selector |
| not_actionable | hidden, disabled, animating or covered | dismiss the overlay / wait for the state with assertVisible |
| timeout | the driver did not answer in time | retry; check the page |
| blocked | a guard rail refused the step: a host outside allowed_domains, a risky tap (confirm_actions), a button the user always blocks (block_actions), a permission the user turned off (allow_eval_script, allow_app_reset, allow_app_install, allow_device_settings, allow_physical_devices), or a file outside upload_dirs (a `point` tap counts as the element under it, and is refused when that element cannot be told; `pressKey Enter`/`Space` that may press a blocked button, and every `evalScript` while block_actions is set) | stay inside the limits; `confirm: true` only lifts confirm_actions, and only when the test case means that tap. Report the rest to the user, never work around it |
| unsupported | step or platform not available on this driver/phase | use a supported step |
| script_error | evalScript threw | fix the expression |
| session | unknown / duplicate session | start or switch sessions |
| transient | page/screen was mid-transition (retried until timeout) | usually resolves; check navigation |
| assertion_failed | assert tool: property did not hold | compare text in the returned snapshot |
| invalid_flow | bad YAML or step | run check_flow |
| driver_error | CDP/browser failure (e.g. Chrome exited, navigation error) | restart the session |
| app_crashed | the app under test died (crash dialog, or its process is gone) | product bug: report it; `launchApp` to continue |
| app_not_responding | Android ANR dialog: the app's main thread is blocked | product bug: report it |

System dialogs (android, ios) are handled before tapOn/inputText:
- A runtime permission prompt (Android permission controller, iOS SpringBoard alert) that the step
  does not target is answered per `FASTPILOT_PERMISSIONS`: `deny` (default), `allow`, `keep`.
  To answer it yourself, target its button (`tapOn: "Allow"`, `{ id: permission_allow_button }`).
- A soft keyboard covering the target is hidden first.
- Crash and ANR dialogs are closed after being reported, so the device recovers.
What was handled shows up under "recent errors" when a step fails.
"#;

const DEVICES: &str = r#"# Devices and attach modes

- Web, own Chrome: `session { action: start }` (one isolated BrowserContext per session).
- Web, your Chrome: `mode: attach` with `endpoint: http://127.0.0.1:9222` (remote debugging port),
  no endpoint (Chrome M144+ autoConnect, chrome://inspect/#remote-debugging), or `endpoint: extension`:
  load `extension-chrome/` (Load unpacked), start the session, then click the extension's button on
  the tab. The relay listens on 127.0.0.1:9230 only (`extension:<port>`; `FASTPILOT_RELAY_TOKEN`).
- Android: attached device or emulator (`endpoint: <serial>`); `session { action: start_device,
  platform: android, endpoint: <AVD> }` boots an emulator. setLocation / network throttling need an emulator.
- iOS simulator: `platform: ios` (booted simulator, else "iPhone 16"); runner from `ios-runner/build.sh`.
  The runner stays up between sessions (exits after 5 idle minutes), so later sessions start fast.
- iOS real device: `platform: ios, device: ios-device:<udid>` (never picked unless named). Needs the
  Xcode matching the phone's iOS, a signing team, and the runner built for it:
  `FASTPILOT_IOS_TEAM=<team> FASTPILOT_IOS_BUNDLE_PREFIX=com.<you>.fastpilot ios-runner/build.sh --device <udid>`.
  `fastpilot ios doctor` lists every blocker with its fix. Reached over USB (usbmux) or the CoreDevice
  tunnel. Simulator-only there: clearKeychain, setLocation, pushNotification, allow/deny permissions,
  recording. Setup guide: docs/real-devices.md.
- Android phone: `device: android-device:<serial>`; unauthorized/offline phones are listed with the fix.
- Another machine (set by the user, never by an agent): `adb_server` / FASTPILOT_ADB_SERVER `host:port`
  (devices behind that adb server; emulator console steps then fail) and `chrome_endpoint` /
  FASTPILOT_CHROME_ENDPOINT (every web session on that Chrome, still one BrowserContext each; file paths
  are that machine's). Both through an SSH tunnel.
"#;
