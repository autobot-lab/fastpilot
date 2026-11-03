# fastpilot Android agent protocol (version 1)

The agent is an instrumentation APK (`com.fastpilot.agent/.Agent`) that drives the device through
`UiAutomation` and serves a TCP socket on the device's `127.0.0.1:<port>`.

## Starting

```sh
agent-android/build.sh                       # -> agent-android/out/fastpilot-agent.apk
adb install -r agent-android/out/fastpilot-agent.apk
adb shell am instrument -w -e port 7912 com.fastpilot.agent/.Agent &   # blocks while the agent runs
adb forward tcp:7912 tcp:7912
```

Readiness: the `am instrument` stdout prints the line
`FASTPILOT_AGENT_READY port=7912 version=1` once the socket is listening. On exit it prints
`FASTPILOT_AGENT_EXIT` (or `FASTPILOT_AGENT_ERROR <exception>` if startup failed).
Only one `UiAutomation` connection may exist per device, so run one agent per device.

## Framing

Every message in both directions is a 4-byte big-endian unsigned length followed by that many bytes
of UTF-8 JSON. Frames over 16 MiB are rejected (the connection is dropped). One client is served at
a time; requests on a connection are handled sequentially, one response per request. Disconnecting
just returns the agent to `accept()`.

Request: `{"id": N, "cmd": "<name>", ...args}`
Response: `{"id": N, "ok": true, ...result}` or `{"id": N, "ok": false, "error": "<message>"}`.
Any exception in a request becomes `ok:false`; the server keeps running.

## Commands

| cmd | args | result |
|---|---|---|
| `hello` | – | `version:"1"`, `sdk`, `width`, `height` (real display px), `density` |
| `snapshot` | – | `package` (active window), `title` (top app window title), `nodes` (array of window root nodes, top-most first) |
| `tap` | `x`, `y` (screen px) | – |
| `swipe` | `x1`, `y1`, `x2`, `y2`, `ms` (default 300) | – (DOWN, 10 MOVEs, UP) |
| `text` | `text` | sets the input-focused node's text (`ACTION_SET_TEXT`); error `no focused text field` |
| `key` | `code` (Android keycode, e.g. 4 BACK, 66 ENTER) | – |
| `idle` | `quietMs` (300), `timeoutMs` (5000) | `idle: true|false` (timeout is not an error) |
| `screenshot` | `quality` (80) | `jpeg`: base64 (no line wraps) JPEG |
| `shell` | `command` | `out`: full stdout, run as the shell user |
| `quit` | – | replies `ok`, then the instrumentation finishes |

Note `shell` takes `command`, not `cmd`, because `cmd` already names the request.

### Snapshot nodes

```json
{"cls":"android.widget.EditText","text":"a@b.com","desc":"...","hint":"Email","id":"pkg:id/email",
 "bounds":[l,t,r,b],"clickable":true,"editable":true,"checkable":true,"checked":true,"enabled":true,
 "focused":true,"selected":true,"password":true,"scrollable":true,"children":[...]}
```

- False booleans and empty strings are omitted: a missing `enabled` means disabled.
- Only nodes with `isVisibleToUser()` are included (an invisible node drops its subtree).
- `password:true` nodes never carry `text`. An empty field's hint is reported only as `hint`, not `text`.
- Windows: application windows plus non-SystemUI system windows (e.g. ANR / system alerts);
  status bar, navigation bar and IME are skipped. Falls back to the active window root.
- WebView content is best driven over CDP: find `@webview_devtools_remote_<pid>` in
  `shell cat /proc/net/unix`, then `adb forward tcp:N localabstract:webview_devtools_remote_<pid>`.
