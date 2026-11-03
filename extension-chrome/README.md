# fastpilot attach (Chrome extension)

Drives a tab of your everyday Chrome without `--remote-debugging-port` (which Chrome 136+ blocks
on the default profile).

1. `chrome://extensions` → Developer mode → **Load unpacked** → this folder.
2. Ask fastpilot for a session: `session { action: start, mode: attach, endpoint: extension }`
   (`extension:<port>` for another port). fastpilot starts a relay on `127.0.0.1:9230` and waits 60 s.
3. Click the extension's toolbar button on the tab to drive. The badge shows **ON**; click again to
   detach. Chrome shows its "started debugging this browser" bar while attached.

Security: the relay binds to 127.0.0.1 only and refuses WebSocket connections from web pages (any
`Origin` other than `chrome-extension://`). Set `FASTPILOT_RELAY_TOKEN` and the same token in the
extension's options to also require a shared secret.

Limits: one tab per relay; browser-level CDP (`Browser.*`, `Target.createTarget`) is not available
through `chrome.debugger`, so per-session isolation and `setPermissions` do not apply here.
