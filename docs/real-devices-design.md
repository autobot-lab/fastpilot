# Desain: dukungan real device (iPhone iOS 17+/26, ponsel Android)

*Dibuat: 2026-11-03 | Alur: ECC feature-dev (eksplorasi `code-explorer`, desain `code-architect`) | Status: siap diimplementasikan; uji hardware ditunda (keputusan user)*

Menggantikan rencana lama di PRD §7.5 dan §16 Phase 5 ("real device via go-ios + prebuilt WDA"): fastpilot kini memakai runner XCUITest sendiri, dan go-ios tidak boleh dipakai saat runtime.

## Keputusan user

- Signing iOS memakai **Apple ID gratis** (personal team): profile berlaku 7 hari, maksimal 3 app per perangkat, bundle id harus unik per team.
- User akan memasang **Xcode 26**; iPhone 14 Plus memakai iOS 26.6.1, Xcode terpasang 16.1 (SDK iphoneos 18.1).
- Ponsel Android yang didukung: **Xiaomi/Redmi/POCO (MIUI/HyperOS), Samsung (One UI), Pixel/stock**.
- Implementasi dikerjakan sesi implementasi; **pengujian hardware nanti**, kode disiapkan dulu.

## Fakta dari kode dan perangkat

- `devicectl list devices -j` sudah jalan di Xcode 16.1 (jsonVersion 2). iPhone: `osVersionNumber 26.6.1`, `pairingState paired`, `tunnelState unavailable` (belum tersambung), `hardwareProperties.udid 00008110-001C5C9E0AC0A01E`. Pakai `hardwareProperties.udid` sebagai id iOS kanonik; `xcodebuild -destination id=` dan `devicectl --device` menerimanya.
- `devicectl` punya `info lockState`, `info details`, `info apps --bundle-id`, `process launch --payload-url`, `copy to --domain-type appDataContainer --remove-existing-content`.
- Runner iOS bind `127.0.0.1` (`RunnerTests.swift:45`) dan host connect ke `127.0.0.1` (`crates/android/src/agent.rs:23-26`); hanya jalan di simulator karena berbagi network stack dengan Mac.
- Method khusus simulator: `devices()`, `boot()`, `open(url)` (`simctl openurl`), `clear_state` (`get_app_container`), `clear_keychain` (`simctl keychain reset`). `xctestrun()` memilih file `.xctestrun` pertama (urutan tidak pasti).
- Project Xcode: `CODE_SIGN_IDENTITY="-"`, `CODE_SIGN_STYLE=Manual`, `DEVELOPMENT_TEAM=""`; scheme bersama ikut membangun `FastpilotDemo` (memakan slot app gratis).
- `xcodebuild test-without-building` di device memasang 2 app: `FastpilotHost` dan xctrunner. Sisa satu slot untuk app yang diuji (AUT); demo tidak boleh ikut dipasang.
- Android: jalur `adb forward` sudah jalan di ponsel fisik. Celah: animation scale di-set 0 dan tidak dikembalikan; tidak ada wake/unlock; device `unauthorized`/`offline` dibuang diam-diam; keystore debug per mesin bisa memicu `INSTALL_FAILED_UPDATE_INCOMPATIBLE`; deteksi dialog izin/crash hanya ID AOSP.

## Keputusan desain

- **Id bertag, id polos tetap jalan.** Tanpa device yang disebut, perilaku lama tidak berubah (iOS = simulator yang sedang boot, Android = satu-satunya device). iPhone fisik hanya dipakai kalau disebut eksplisit, agar slot app gratis tidak terpakai tanpa sengaja.
- **Transport iOS: usbmux utama, tunnel CoreDevice sebagai fallback.** usbmux lewat proxy TCP lokal, sama seperti `adb forward`, sehingga `Agent::connect(port)` tidak berubah, tidak ada prompt "Local Network" macOS, dan tidak bergantung pada key JSON yang tidak terdokumentasi. Butuh kabel USB. Fallback tunnel (IPv6 dari `devicectl device info details`) juga menutup koneksi nirkabel. `FASTPILOT_IOS_TRANSPORT=auto|usbmux|tunnel`; `auto` mencoba usbmux, lalu tunnel jika `hello` terautentikasi belum sukses dalam 20 s.
- **Keamanan runner.** Host mengirim token acak 128-bit lewat `TEST_RUNNER_FASTPILOT_TOKEN`; request pertama wajib `hello{token}`, selain itu koneksi ditutup. Jalur simulator tetap identik (tanpa env token).
- **Kesiapan di device = polling `hello`**, bukan stdout: tiap 500 ms, berlomba dengan exit xcodebuild, batas 240 s. Marker `FASTPILOT_RUNNER_READY` hanya petunjuk. Ekor stderr selalu ditangkap.
- **Direktori build per platform:** simulator `ios-runner/build`, device `ios-runner/build-device` (override `FASTPILOT_IOS_RUNNER_DEVICE`). Pemilih `xctestrun` disaring per nama file `iphonesimulator`/`iphoneos`, mtime terbaru menang.
- **Kedaluwarsa profile dicek di host:** baca `embedded.mobileprovision` dari `Products/Debug-iphoneos/*.app` (lewati `*Demo*`). Error jika kedaluwarsa, warning jika sisa < 24 jam, error jika `ProvisionedDevices` tidak memuat UDID.
- **Satu dependency baru:** crate `plist` (pure Rust) di `fastpilot-ios` saja, untuk usbmux dan profile.
- **`Unsupported` yang jujur:** `clear_keychain` di iPhone fisik mengembalikan `Unsupported` dengan pesan. Tidak ada step DSL untuk install app (path dari AI adalah risiko); install AUT tugas user.

## 1. Model device dan pemilihan

File baru `crates/core/src/device.rs`:

```rust
pub enum DeviceKind { Simulator, IosDevice, Emulator, AndroidDevice } // simulator|ios-device|emulator|android-device
pub struct DeviceSpec { pub kind: Option<DeviceKind>, pub id: String } // "ios-device:0000..." atau id/nama polos
pub struct DeviceInfo { pub id: String, pub kind: DeviceKind, pub name: String, pub state: String, pub detail: Vec<String> }
pub fn select<'a>(all: &'a [DeviceInfo], spec: Option<&DeviceSpec>, platform: Platform) -> Result<&'a DeviceInfo, SelectError>
```

- Urutan cocok: id persis → nama persis → prefix unik tanpa beda huruf besar/kecil. Ambigu atau tidak ada → `SelectError` bertipe yang mendaftar kandidat dan petunjuk perbaikan.
- Prefix kind menyiratkan platform; konflik = error.
- Sumber (urutan prioritas): DSL `session: { start: ios1, platform: ios, device: ios-device:<udid> }` → argumen MCP `session start` baru `device` → `endpoint` lama → env `FASTPILOT_IOS_UDID` / `FASTPILOT_ANDROID_SERIAL` (kini menerima id bertag dan nama) → untuk Android, `ANDROID_SERIAL`.
- Plumbing: `Launcher::launch(&self, platform, device: Option<&str>)` (dua impl: `PlatformLauncher`, `FakeLauncher`); `Step::StartSession` menambah `device: Option<String>`; `check_flow` memvalidasi sintaks spec dan konsistensi platform.
- Output `devices`: prefix baris `android:`, `android avd:`, `ios:` dipertahankan, ditambah id kanonik dan petunjuk, mis. `ios: ios-device:00008110-... "Yogi's iPhone" iOS 26.6.1 (paired, tunnel unavailable; runner profile 5d left)`. iPhone fisik dari `devicectl` ikut terdaftar; filter `iPhone*/iPad*` hanya untuk simulator. Android `unauthorized`/`offline` ikut terdaftar dengan petunjuk.
- `start_device` untuk device fisik → `Unsupported("physical devices cannot be started; connect and unlock one")`.

## 2. Transport host ↔ runner iOS

- **Runner (`RunnerTests.swift`):** jika `FASTPILOT_BIND == "any"`, bind dual-stack `AF_INET6` + `IPV6_V6ONLY=0` + `in6addr_any`; selain itu tetap `127.0.0.1`. Cek token. Baris READY menambah `bind=` dan `token=on|off`. `hello` mengembalikan `device: simulator|physical` dan `osVersion`. Sampai terverifikasi di hardware apakah koneksi usbmux tiba sebagai loopback, default bind device = `any` (dijaga token).
- **`Agent` (`crates/android/src/agent.rs`):**
  ```rust
  pub struct Endpoint { pub host: IpAddr, pub port: u16, pub token: Option<String> }
  impl Agent { pub async fn connect(port: u16) -> Result<Self>; pub async fn connect_to(ep: Endpoint) -> Result<Self> }
  ```
  `connect` = `connect_to(127.0.0.1, port, None)`; timeout 3 s per `TcpStream::connect`; batas retry jadi parameter (Android tetap 20 × 150 ms).
- **`crates/ios/src/usbmux.rs`:** socket `/var/run/usbmuxd`; header 16 byte little-endian (`len` termasuk header, `version=1`, `request=8`, `tag`) + plist XML. `ListDevices` → `DeviceList` `{DeviceID, Properties{SerialNumber=UDID, ConnectionType}}`. `Connect{DeviceID, PortNumber}` dengan port di-byte-swap (htons); balasan `Result{Number}`: 0 OK (socket jadi stream mentah), 2 device hilang, 3 ditolak. `pub async fn forward(udid, remote_port) -> Result<(u16, JoinHandle<()>)>`: bind `127.0.0.1:0`, tiap accept membuka Connect baru dan `copy_bidirectional`; drop membatalkan task.
- **Fallback tunnel:** polling `devicectl device info details -d <udid> -j f`, baca `result.connectionProperties.tunnelIPAddress`, `Agent::connect_to(Endpoint{addr, port, token})`.

## 3. Build dan signing

- `project.pbxproj`: variabel `FASTPILOT_BUNDLE_PREFIX = com.fastpilot`; Host `$(FASTPILOT_BUNDLE_PREFIX).runner`, UI tests `.runner.uitests`, Demo `.demo`. Default simulator tidak berubah.
- Scheme baru `FastpilotRunnerDevice.xcscheme`: Host + bundle test, tanpa demo.
- `ios-runner/build.sh --device <udid> [--team T]`: `-scheme FastpilotRunnerDevice -destination id=<udid> -derivedDataPath build-device CODE_SIGN_STYLE=Automatic DEVELOPMENT_TEAM=$FASTPILOT_IOS_TEAM CODE_SIGN_IDENTITY="Apple Development" FASTPILOT_BUNDLE_PREFIX=$FASTPILOT_IOS_BUNDLE_PREFIX -allowProvisioningUpdates -allowProvisioningDeviceRegistration`. Team dari env/`--team`; autodetect hanya jika `defaults read com.apple.dt.Xcode IDEProvisioningTeamByIdentifier` berisi tepat satu team. `FASTPILOT_IOS_BUNDLE_PREFIX` wajib di mode device; cetak catatan bahwa mengganti prefix memakan kuota App ID gratis (±10 per 7 hari). Cetak `XCTESTRUN=` dan masa berlaku profile.
- Eksperimen opsional: bundle UI test tanpa target host (hapus `FastpilotHost`) untuk membebaskan satu slot lagi; pertahankan hanya jika Xcode menerimanya.
- `crates/ios/src/runner.rs`: lookup xctestrun, cek profile, spawn xcodebuild (dipindah dari lib.rs, perilaku simulator tetap).
- Klasifikasi kegagalan dari ekor output xcodebuild/devicectl → `Error` bertipe dengan petunjuk:

| Output memuat | Petunjuk |
|---|---|
| `not been explicitly trusted` | Settings > General > VPN & Device Management > percayai developer |
| `maximum number of installed apps` | batas 3 app gratis; hapus satu app di ponsel |
| `profile` + `expired` / `No profile` | jalankan `ios-runner/build.sh --device <udid>` (kasus 7 hari) |
| `Developer Mode` | Settings > Privacy & Security > Developer Mode, lalu reboot |
| `locked` | buka kunci ponsel |
| `not supported` / `Xcode` | Xcode terlalu lama |

## 4. Pengganti real device dan preflight

- **Preflight** (`crates/ios/src/device.rs`), fungsi murni `preflight(&DeviceFacts, &HostFacts) -> Vec<Problem>`: (1) `devicectl` dan `xcodebuild` ada; (2) major SDK iphoneos ≥ major iOS device, jika tidak "pasang Xcode 26"; (3) device terdaftar dan terjangkau (`tunnelState != unavailable`); (4) `pairingState == paired`; (5) `developerModeStatus == enabled`; (6) `ddiServicesAvailable == true`; (7) tidak terkunci (`lockState`); (8) runner dibangun untuk iphoneos dan profile valid untuk UDID ini; (9) saat membuka bundle, cek sekali bahwa app terpasang (`info apps --bundle-id`). Semua field di-parse sebagai `Option`/`serde(default)`. Petunjuk run pertama menyebut kemungkinan "Enable UI Automation" di iOS 26.
- **`open(url)`:** perintah runner baru `openUrl {url}` dengan `XCUIDevice.shared.system.open(_:)` (`#available(iOS 16.4, *)`); fallback `devicectl device process launch --payload-url <url> <bundle>` (`com.apple.mobilesafari` untuk http/https). Simulator tetap `simctl openurl`.
- **Reset izin:** perintah runner baru `resetPermissions {bundleId, resources?}` dengan `XCUIApplication(bundleIdentifier:).resetAuthorizationStatus(for:)`; jalan di simulator dan device; di device menjadi bagian `clear_state`.
- **`clear_state` di device:** `terminate` → `devicectl device copy to --domain-type appDataContainer --domain-identifier <bundle> --source <tmp kosong> --destination {Documents,Library,tmp} --remove-existing-content true` → `resetPermissions`. Hanya untuk AUT yang ditandatangani development (get-task-allow); selain itu `Unsupported` dengan pesan.
- **`clear_keychain` di device:** `Unsupported` ("iOS tidak punya API; buat app menghapus keychain saat launch atau logout lewat UI").
- **Install app:** bukan fitur driver; dokumentasikan `devicectl device install app --device <udid> <App.app>` dan anggaran slot: host (1) + xctrunner (2) + AUT (3).

## 5. Pengerasan ponsel Android

- **Daftar device:** `host:devices-l` (model, device, transport_id). `AndroidDevice { serial, state, kind: Emulator|Physical, model, oem }`, OEM dari `ro.product.manufacturer`, `ro.miui.ui.version.name`, `ro.mi.os.version.name`. Ganti filter diam `state == "device"`.
- **Error bertipe:** `Unauthorized`, `Offline`, `Locked`, `InstallBlocked`, dengan petunjuk (izinkan USB debugging dan centang Always; cabut otorisasi lalu colok ulang; Samsung: Auto Blocker; offline: colok ulang, toggle USB debugging, `adb kill-server`).
- **Animation scale:** simpan nilai asli ke `/data/local/tmp/fastpilot-anim.bak` (jika file belum ada) sebelum di-nol-kan; pulihkan dan hapus file saat close. Tambah `Driver::close(&self)` (default no-op) di core; `AndroidDriver::close` memulihkan lalu `quit`; dipanggil di `session stop` dan close-all saat shutdown; `Drop` sebagai cadangan. Sama untuk `svc power stayon usb`.
- **Wake/unlock sebelum agent jalan:** `dumpsys power` (`mWakefulness`), `input keyevent 224`, `wm dismiss-keyguard`, cek keyguard dari `dumpsys window`. Jika tetap terkunci → `Locked` ("buka kunci; PIN tidak bisa diotomasi; pakai Stay awake dan kunci Swipe/None"). PIN tidak pernah lewat env.
- **Install/keystore:** pada `INSTALL_FAILED_UPDATE_INCOMPATIBLE`/`VERSION_DOWNGRADE`, `pm uninstall com.fastpilot.agent` lalu coba sekali lagi, hanya untuk paket agent. `INSTALL_FAILED_USER_RESTRICTED` → petunjuk Xiaomi ("Install via USB" + "USB debugging (Security settings)"). Jika `mCurrentFocus` berisi `AdbInstallActivity`/`packageinstaller` → "tap Install di ponsel". Play Protect: petunjuk saja. Error input pertama berisi `INJECT_EVENTS` → petunjuk Xiaomi yang sama.
- **Dialog OEM (`system.rs`):** longgarkan ID crash/ANR ke sufiks `:id/aerr_close|aerr_wait`; log `tracing::debug!` untuk jendela non-app tak dikenal (paket + id) agar fixture MIUI/Samsung bisa direkam saat uji hardware. Fixture unit test hanya dari dump nyata. Dokumentasikan setelan baterai (Xiaomi Autostart + No restrictions, Samsung Unrestricted).

## 6. Test

- **Unit (tanpa hardware):** `DeviceSpec::parse` dan `select()`; parsing JSON `devicectl` (fixture nyata dari iPhone di atas + varian "connected" bertanda unverified); tabel `preflight()`; perbandingan SDK vs iOS; header/plist usbmux, byte-swap port, fake usbmuxd di `UnixListener` (OK, ditolak, proxy end-to-end); parser kedaluwarsa profile; klasifikasi kegagalan xcodebuild; filter xctestrun; `Agent::connect_to` dengan token; parsing Android (`devices-l`, OEM, wakefulness/keyguard, error install, skrip simpan/pulihkan animasi); DSL `device:` dan `FakeLauncher`.
- **E2E simulator tambahan:** `FASTPILOT_BIND=any` + token (tanpa token ditolak), `openUrl`, `resetPermissions`.
- **E2E hardware opt-in** (dilewati tanpa env): `tests/real_ios.rs` (`FASTPILOT_IOS_REAL_UDID`; app bawaan Settings dan Safari, tanpa install) dan `tests/real_android.rs` (`FASTPILOT_ANDROID_REAL_SERIAL`; Settings, demo APK jika ada, animasi pulih setelah stop).
- **Perbaiki `android.rs`:** helper `serial()` dan `adb -s` di semua perintah; swipe dihitung dari `wm size`.

## 7. Urutan implementasi

Setiap langkah bisa di-merge sendiri. Aturan saat ini: merge ke main setelah `cargo build --workspace` lolos; test dijalankan nanti bersama uji penuh.

| # | File | Perubahan |
|---|---|---|
| S1 | `crates/core/src/{device,lib,dsl,engine}.rs`, `crates/mcp/src/{lib,docs}.rs` | Model device, `select()`, `Launcher::launch(.., device)`, DSL `device:`, argumen MCP `device`, `Driver::close`, output `devices` |
| S2 | `crates/android/src/{adb,lib,device}.rs` | `devices-l`, state dengan petunjuk, error bertipe, `ANDROID_SERIAL`, error multi-device |
| S3 | `crates/android/src/{lib,device}.rs`, stop/shutdown di mcp | Simpan/pulihkan animasi dan stay-awake, wake/unlock, retry keystore, petunjuk install |
| S4 | `crates/android/src/system.rs` | ID aerr dilonggarkan, log jendela tak dikenal |
| S5 | `bin/fastpilot/tests/{android,real_android}.rs` | `-s`, swipe dari `wm size`, test ponsel fisik |
| S6 | `crates/ios/Cargo.toml` (`plist`), `crates/ios/src/device.rs`, `bin/fastpilot/src/main.rs` (`fastpilot ios doctor [device]`), `crates/ios/src/lib.rs` | Wrapper `devicectl`, parsing, preflight, `doctor`, `devices()` mendaftar iPhone fisik |
| S7 | `RunnerTests.swift`, `ios-runner/PROTOCOL.md`, `crates/android/src/agent.rs`, `crates/ios/src/lib.rs` | Mode bind, token, `openUrl`, `resetPermissions`, field `hello`, `Agent::connect_to` |
| S8 | `project.pbxproj`, `FastpilotRunnerDevice.xcscheme`, `build.sh`, `crates/ios/src/runner.rs` | Prefix bundle, scheme device, build `--device`, xctestrun per platform, cek profile, klasifikasi kegagalan |
| S9 | `crates/ios/src/usbmux.rs` | Klien usbmux dan proxy forward |
| S10 | `crates/ios/src/{lib,runner,device}.rs` | Cabang device di `IosDriver::start`: preflight, build lookup, spawn xcodebuild dengan env, race kesiapan, pemilihan transport, pengganti open/clear_state/clear_keychain |
| S11 | `bin/fastpilot/tests/real_ios.rs`, `docs/real-devices.md`, PRD §7.4/§7.5/§21, README, `crates/mcp/src/docs.rs` | Test hardware, panduan setup user, dokumentasi |

## Yang hanya bisa dipastikan dengan hardware

- Forward port app lewat usbmux di iOS 26, dan apakah koneksinya tiba sebagai loopback.
- Fallback tunnel, termasuk prompt Local Network macOS.
- Apakah stdout runner sampai ke xcodebuild di device.
- Perilaku `XCUIDevice.system.open` dan `resetAuthorizationStatus`.
- Semantik `copy to --remove-existing-content` dan cache cfprefsd.
- JSON `devicectl` persis saat device tersambung.
- Batas 3 app gratis dengan bundle UI test tanpa host.
- Waktu start xcodebuild di device (±30–90 s; batas 240 s).
- Dialog izin, crash, dan ANR di MIUI/HyperOS dan Samsung.

## Yang harus disiapkan user sebelum uji hardware

- **Mac:** pasang Xcode 26, jalankan `xcode-select`, selesaikan komponen first-launch. Tambahkan Apple ID di Xcode > Settings > Accounts. Siapkan `FASTPILOT_IOS_TEAM` dan `FASTPILOT_IOS_BUNDLE_PREFIX` (mis. `com.<nama>.fastpilot`).
- **iPhone:** sambung USB, Trust This Computer, aktifkan Developer Mode (reboot), Auto-Lock = Never, buka kunci. Setelah install pertama, percayai developer di Settings > General > VPN & Device Management. Ulangi `build.sh --device <udid>` setiap 7 hari.
- **Xiaomi/Redmi/POCO:** Developer options > USB debugging, Install via USB, USB debugging (Security settings); baterai "No restrictions" untuk `com.fastpilot.agent`.
- **Samsung:** izinkan USB debugging, matikan Auto Blocker, baterai "Unrestricted" untuk agent.
- **Pixel:** cukup USB debugging.
