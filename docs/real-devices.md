# Real devices: setup

fastpilot drives physical phones with the same tools as emulators and simulators. Nothing is
installed on a phone except fastpilot's own helpers (Android agent; iOS runner + host app).

## Android (Xiaomi/Redmi/POCO, Samsung, Pixel)

1. Developer options → **USB debugging** on; plug in, allow the computer (tick *Always allow*).
2. Xiaomi/Redmi/POCO also: **Install via USB** and **USB debugging (Security settings)** (needs a
   Mi account and a SIM). Samsung: turn off **Auto Blocker**.
3. Battery: *No restrictions* (Xiaomi, plus Autostart) / *Unrestricted* (Samsung) for
   `com.fastpilot.agent`. Screen lock: Swipe or None, or keep the phone unlocked; a PIN is never
   automated.
4. `session { action: devices }` shows it as `android-device:<serial>`; start with
   `device: android-device:<serial>` (needed only when several are attached).

fastpilot saves the phone's animation scales and stay-awake setting before changing them and
restores them when the session stops. Unauthorized/offline phones are listed with what to do.

## iPhone / iPad (iOS 17+; iOS 26 needs Xcode 26)

1. Mac: install the Xcode that matches the phone's iOS, `sudo xcode-select -s /Applications/Xcode.app`,
   open it once. Xcode → Settings → Accounts → add your Apple ID (a free personal team works).
2. iPhone: USB, unlock, **Trust This Computer**; Settings → Privacy & Security → **Developer Mode**
   (restart); Auto-Lock → Never.
3. Build the runner for the phone (free profiles last 7 days: repeat weekly):

```bash
FASTPILOT_IOS_TEAM=<team id> FASTPILOT_IOS_BUNDLE_PREFIX=com.<you>.fastpilot ios-runner/build.sh --device <udid>
```

4. First install: Settings → General → VPN & Device Management → trust your developer certificate.
   iOS 26 may also need Settings → Developer → **Enable UI Automation**.
5. Check everything: `fastpilot ios doctor` (each problem comes with its fix).
6. Start with `device: ios-device:<udid>`. A physical iPhone is never picked unless named.

Free Apple ID budget: 3 apps signed by you per device = host + runner + the app under test.
Install your app yourself: `xcrun devicectl device install app --device <udid> <App.app>`.

Transport: USB via usbmux (default); after 20 s fastpilot also tries the CoreDevice tunnel
(Wi-Fi after pairing). `FASTPILOT_IOS_TRANSPORT=auto|usbmux|tunnel`. The runner listens on all
interfaces on a device, guarded by a random per-start token.

On a device: `openLink` works (iOS 16.4+), `clearState` works for development-signed apps,
`setPermissions` supports `unset`; `clearKeychain`, `setLocation`, `pushNotification`, allow/deny
permissions and screen recording are simulator-only and report `unsupported`.

## WKWebView content (simulator and iPhone)

Web content inside the app is read and driven through the Web Inspector, like Safari's Develop
menu: the DOM is merged into the snapshot under the web view, and `tapOn`, `inputText` and
`evalScript` on web elements go through the inspector protocol. Needed:

1. The app sets `webView.isInspectable = true` (iOS 16.4+; a debug-build flag is enough).
2. iPhone only: Settings → Safari (Apps → Safari on iOS 18+) → Advanced → **Web Inspector** on,
   USB cable, phone unlocked. `fastpilot ios doctor` reports whether the service answers.

The simulator uses its `webinspectord_sim` socket (`FASTPILOT_WEBINSPECTOR_SOCKET` overrides when
several simulators are booted). A device uses lockdownd's `com.apple.webinspector` service over
usbmux with TLS from this Mac's pair record; the RemoteXPC (`.shim.remote`) variant is not
implemented. Without an inspectable page the snapshot keeps the native accessibility tree and
`evalScript` reports `unsupported` with these steps.

Limits: web taps are DOM `click()`s and typing uses `insertText` (no touch events); with several
WKWebViews the first page listed is used; bounds ignore a translucent bar's content inset.
