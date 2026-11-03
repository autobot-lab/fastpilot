# fastpilot iOS runner protocol (version 1)

The runner is a self-built XCUITest bundle (`FastpilotRunner`, host app `FastpilotHost`,
`com.fastpilot.runner`) whose single test method serves a TCP socket on `127.0.0.1:<port>`.
No WebDriverAgent, no third-party code. The simulator shares the Mac's network stack, so the host
connects to `127.0.0.1:<port>` directly (no forwarding).

## Building and starting

```sh
ios-runner/build.sh        # prints XCTESTRUN=<...>.xctestrun and DEMO_APP=<...>/FastpilotDemo.app
xcrun simctl boot <UDID>
xcrun simctl install <UDID> <DEMO_APP>                       # optional: the demo app
TEST_RUNNER_FASTPILOT_PORT=22087 xcodebuild test-without-building \
  -xctestrun <XCTESTRUN> -destination id=<UDID> &            # blocks while the runner serves
```

Readiness: the xcodebuild stdout prints `FASTPILOT_RUNNER_READY port=<port> version=1` once the
socket is listening (about 23 s after starting xcodebuild on an already-booted simulator). After
`quit` it prints `FASTPILOT_RUNNER_EXIT` and xcodebuild ends with `** TEST EXECUTE SUCCEEDED **`.
If the port can't be bound it prints `FASTPILOT_RUNNER_ERROR ...` and the test fails.
`FASTPILOT_PORT` defaults to 22087. xcodebuild passes `TEST_RUNNER_<X>` to the runner as `<X>`.

Device mode: `FASTPILOT_BIND=any` listens on every interface (dual-stack IPv6, `IPV6_V6ONLY=0`) so the
host can reach it over usbmux or the CoreDevice tunnel; otherwise it binds `127.0.0.1`.
`FASTPILOT_TOKEN` (128-bit, random per start) makes `hello {token}` mandatory as the first request;
anything else gets `unauthorized` and the connection is closed. The READY line adds `bind=` and
`token=on|off`. The simulator path sets neither.

Lifetime: the runner outlives its session. It exits after `FASTPILOT_IDLE_EXIT` seconds (default
300) without a client, so the host can reconnect to the same port (fixed per simulator) instead of
starting xcodebuild again. XCTest's wait for app quiescence (animations included) around every action
is replaced with a no-op (as WebDriverAgent does) unless `FASTPILOT_QUIESCENCE=1`.

## Framing

Same as the Android agent. Every message in both directions is a 4-byte big-endian unsigned length
followed by that many bytes of UTF-8 JSON. Frames over 16 MiB drop the connection. One client is
served at a time, and requests are handled sequentially with one response each. Disconnecting
returns the runner to `accept()`.

Request: `{"id": N, "cmd": "<name>", ...args}`
Response: `{"id": N, "ok": true, ...result}` or `{"id": N, "ok": false, "error": "<message>"}`.
A request that isn't JSON gets `{"id": null, "ok": false, "error": "request is not a JSON object"}`.
Errors never stop the server.

All coordinates are screen **points** (portrait origin top-left), not pixels.

## Commands

| cmd | args | result |
|---|---|---|
| `hello` | `token` (device runs) | `version:"1"`, `width`, `height` (screen points), `scale`, `device` (`simulator` \| `physical`), `osVersion` |
| `resetPermissions` | `bundleId`, `resources` (optional: contacts, calendar, reminders, photos, microphone, camera, mediaLibrary, homeKit, bluetooth, location) | `reset`: the resources reset (all when omitted) |
| `openUrl` | `url` | – : `XCUIDevice.system.open` (iOS 16.4+; used on devices, simctl on the simulator) |
| `launch` | `bundleId` | – : (re)launches the app fresh (`XCUIApplication.launch`, which terminates a running instance) and makes it the current app |
| `activate` | `bundleId` | – : brings the app to the foreground without relaunching (launches it if not running) and makes it current |
| `snapshot` | – | `bundleId`, `title`, `nodes` (`[root]`) for the current app (SpringBoard if none) |
| `tap` | `x`, `y` | – |
| `swipe` | `x1`, `y1`, `x2`, `y2`, `ms` (300) | – : press 50 ms, then drag at distance/ms velocity |
| `text` | `text` | replaces the keyboard-focused field's content (deletes the existing characters, then types). Error `no focused text field` |
| `key` | `name`: `return`, `enter`, `delete`, `tab`, `escape`, `space` | typed into the focused field. Error `no focused text field` |
| `back` | – | taps the first button of the first navigationBar. Error `no back button` |
| `screenshot` | `quality` (80) | `jpeg`: base64 (no line wraps) |
| `idle` | `timeoutMs` (1500) | `idle`: true once two snapshots 50 ms apart are identical (false at the timeout). With `FASTPILOT_QUIESCENCE=1` XCTest's own quiescence wait stays on and this returns true at once |
| `longPress` / `doubleTap` | `x`, `y` (`ms` 800) | – |
| `erase` | `count` (default: all) | `erased` |
| `terminate` | `bundleId` | – |
| `hideKeyboard` | – | `hidden` |
| `quit` | – | replies `ok`, then the test method returns |

### Snapshot nodes

One `XCUIApplication.snapshot()` call per request, with no per-element queries.

```json
{"type":"textField","label":"...","id":"email","value":"a@b.com","placeholder":"Email",
 "frame":[x,y,w,h],"enabled":true,"selected":true,"focused":true,"children":[...]}
```

- `type` is the `XCUIElement.ElementType` name in lowerCamel (`application`, `window`, `other`,
  `button`, `staticText`, `textField`, `secureTextField`, `searchField`, `switch`, `link`, `image`,
  `cell`, `navigationBar`, ...).
- False booleans and empty strings are omitted, so a missing `enabled` means disabled.
- `focused` means keyboard focus (`hasKeyboardFocus`).
- An empty field reports its placeholder only as `placeholder`, never as `value`.
  `secureTextField` nodes never carry `value`.
- A node with a zero-size frame, or one entirely off screen, is dropped and its visible children
  take its place.
- `title` is the identifier (or label) of the first navigationBar, or `""`.
- A switch's `value` is `"0"` or `"1"`.
