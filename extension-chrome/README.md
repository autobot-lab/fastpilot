# fastpilot attach (Chrome extension)

Drives a tab of your everyday Chrome without `--remote-debugging-port` (which Chrome 136+ blocks
on the default profile).

1. `chrome://extensions` → Developer mode → **Load unpacked** → this folder.
2. Ask fastpilot for a session: `session { action: start, mode: attach, endpoint: extension }`
   (`extension:<port>` for another port). fastpilot starts a relay on `127.0.0.1:9230` and waits 60 s.
3. Click the extension's toolbar button on the tab to drive. The badge shows **ON**; click again to
   detach. Chrome shows its "started debugging this browser" bar while attached.

Security: the relay binds to 127.0.0.1 only, refuses WebSocket connections from web pages (any
`Origin` other than `chrome-extension://`), and both its routes (`/extension`, `/cdp`) require a
token that is new for every session. fastpilot hands the token to this extension through Chrome
native messaging, which web pages cannot use:

- When a session waits for the extension, fastpilot writes the token to
  `~/.fastpilot/relay/<port>.token` (owner-only) and registers itself as the native host
  `dev.fastpilot.relay` (`NativeMessagingHosts/dev.fastpilot.relay.json` in the Chrome / Chromium
  profile folder), allowed only for this extension's ID (`iehmgkdbiojkipljgbaljojdmdfpjeeg`, fixed by
  the `key` in `manifest.json`).
- On a click, the extension asks the native host for the token of its port and connects with it.
- Without native messaging (another browser, or a copy of the extension with another ID), set
  `FASTPILOT_RELAY_TOKEN` and the same value in the extension's options.

After updating from 0.1.0, reload the extension (its ID changes once, to the fixed one).

Limits: one tab per relay; browser-level CDP (`Browser.*`, `Target.createTarget`) is not available
through `chrome.debugger`, so per-session isolation and `setPermissions` do not apply here.
