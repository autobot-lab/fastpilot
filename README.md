# fastpilot

MCP server berupa **satu binary Rust** untuk QA automation full-AI di **Web, Android, dan iOS**. AI agent
(Claude Code, Claude Desktop, Cursor, dll.) membaca test case bahasa natural, lalu menjalankan banyak langkah
dalam **satu** `run_flow`, juga lintas platform dalam satu skenario. Hasilnya berupa snapshot teks ringkas
dan diff, bukan view hierarchy mentah.

- **Web:** CDP langsung lewat satu WebSocket (tanpa Node/chromedriver). Mode *launch* (Chrome milik
  fastpilot, satu `BrowserContext` per sesi) atau *attach* (Chrome milik user).
- **Android:** protokol adb server ditulis di Rust, plus agent instrumentation (UiAutomation) yang
  di-embed di binary. Konten WebView digabung ke snapshot dan dikendalikan lewat CDP.
- **iOS (simulator dan iPhone):** runner XCUITest milik fastpilot sendiri (`ios-runner/`), dibangun dari
  source, tanpa Appium/WebDriverAgent. Konten WKWebView lewat Web Inspector.
- DSL YAML kompatibel Maestro, ditambah `openUrl`, `evalScript`, `session`, `parallel`, `pressKey`,
  `selectOption`, `uploadFile`.
- **Andal:**
  - Auto-wait tanpa `sleep` (actionability, DOM/a11y quiet, navigasi, dan request yang dipicu aksi).
  - Ref stabil: ref mengikuti elemen dan tidak pernah pindah ke elemen lain.
  - Stop-early dengan alasan, kategori, dan kandidat.
  - Dialog JS ditangani otomatis.
- **Path memory:** flow yang lolos disimpan sebagai *hint* untuk replay dalam 1 round-trip berikutnya.
  Evidence tiap run tersimpan di `.fastpilot/runs/`.

Spesifikasi: [docs/PRD.md](docs/PRD.md). Riset improvement: [docs/research-improvements.md](docs/research-improvements.md).

## Mulai cepat dengan Claude Code

### 1. Prasyarat

- [Rust](https://rustup.rs) 1.88+ dan Google Chrome (atau Chromium; set `FASTPILOT_CHROME` kalau lokasinya tidak standar).
- [Claude Code](https://docs.claude.com/en/docs/claude-code) (`claude`).
- Opsional, untuk Android: Android SDK (platform-tools, emulator, build-tools, platform 35), JDK 17, dan satu AVD.
- Opsional, untuk iOS: Xcode 16+ dengan simulator iOS.

### 2. Build dan pasang

```bash
git clone https://gitlab.com/autobot-lab/fastpilot.git
```

```bash
cd fastpilot
```

Kalau mau Android, bangun agent APK dulu, karena APK itu di-embed ke binary saat build:

```bash
agent-android/build.sh
```

Kalau mau iOS, bangun runner XCUITest dan demo app:

```bash
ios-runner/build.sh
```

Pasang binary ke `~/.cargo/bin`, yang biasanya sudah ada di `PATH`:

```bash
cargo install --path bin/fastpilot --locked
```

```bash
fastpilot --version
```

### 3. Daftarkan ke Claude Code

Jalankan dari folder proyek yang mau dites. Untuk mencoba demo bawaan, jalankan dari folder `fastpilot`:

```bash
claude mcp add fastpilot -e FASTPILOT_VARS=DEMO_PASS -e DEMO_PASS=rahasia123 -- fastpilot mcp
```

```bash
claude mcp list
```

`fastpilot` harus tampil dengan status connected. Variabel di `FASTPILOT_VARS` adalah satu-satunya variabel
yang boleh dipakai flow lewat `${NAMA}`, dan nilainya selalu tampil sebagai `***` di output. Untuk melihat
browser saat test berjalan, tambahkan `-e FASTPILOT_HEADFUL=1`.

### 4. Jalankan test lewat Claude

Buka `claude` di folder yang sama, lalu beri perintah dalam bahasa biasa, misalnya:

> Pakai fastpilot, jalankan test case di `tests/login.md`. Password ada di `${DEMO_PASS}`.
> Pakai `test_id` dari file, lalu tutup dengan `report`.

Claude akan memanggil `docs` (sekali), `session start`, satu atau beberapa `run_flow`, lalu `report`.
Hasilnya ada di `.fastpilot/runs/<waktu>-login-ok/report.md`. Jalankan perintah yang sama sekali lagi:
kali ini `session start` mengembalikan *hint* dari run sebelumnya, sehingga test bisa selesai dalam satu `run_flow`.

**Android:**

1. Nyalakan emulator, lewat Android Studio atau minta Claude: *"pakai fastpilot, start_device AVD
   Small_Phone_API_35"*. Daftar AVD bisa dilihat dengan `session devices`.
2. Pasang demo app:

   ```bash
   adb install -r examples/demo-android/out/fastpilot-demo.apk
   ```

3. Minta Claude: *"Pakai fastpilot, jalankan `tests/login-android.md`."*

**iOS (simulator):**

```bash
xcrun simctl boot "iPhone 16"
```

```bash
xcrun simctl install booted ios-runner/build/Build/Products/Debug-iphonesimulator/FastpilotDemo.app
```

Lalu minta Claude: *"Pakai fastpilot di platform ios, buka app com.fastpilot.demo, login pakai a@b.com /
`${DEMO_PASS}`, pastikan 'Selamat datang, Budi' tampil."* Sesi iOS pertama butuh sekitar 20–30 detik
untuk menyalakan runner.

**Web + Android sekaligus:** *"Pakai fastpilot, jalankan `tests/login-cross-platform.md`."*

### 5. Tes aplikasi sendiri

Tulis test case Markdown seperti `tests/login.md`: judul, `id`, `platform`, `target`, lalu langkah dan
harapan dalam bahasa biasa. Tidak perlu menulis selector atau script; Claude yang menyusun flow-nya.
Simpan secret di MCP config (`-e NAMA=nilai`, lalu tambahkan `NAMA` ke `FASTPILOT_VARS`).

### Claude Desktop

Tambahkan blok berikut ke `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) atau
`%APPDATA%\Claude\claude_desktop_config.json` (Windows), lalu restart Claude Desktop:

```json
{
  "mcpServers": {
    "fastpilot": {
      "command": "/Users/NAMA/.cargo/bin/fastpilot",
      "args": ["mcp"],
      "env": { "FASTPILOT_VARS": "DEMO_PASS", "DEMO_PASS": "rahasia123", "FASTPILOT_DIR": "/Users/NAMA/fastpilot-runs" }
    }
  }
}
```

Claude Desktop tidak menjalankan server dari folder proyek. Karena itu, pakai path command absolut, URL atau
path absolut di `target`, dan set `FASTPILOT_DIR` ke folder hasil yang bisa ditulisi.

### Kalau ada masalah

| Pesan | Solusi |
|---|---|
| `Chrome not found` | Pasang Chrome, atau set `-e FASTPILOT_CHROME=/path/ke/chrome` |
| `variable X is not allowed` | Tambahkan `X` ke `FASTPILOT_VARS` di MCP config |
| `agent APK not embedded` | Jalankan `agent-android/build.sh`, lalu `cargo install --path bin/fastpilot --locked` lagi |
| `runner not built` | Jalankan `ios-runner/build.sh` |
| `no Android device` | Nyalakan emulator atau sambungkan device (`adb devices`) |
| Log detail | Tambahkan `-e FASTPILOT_LOG=debug` (log ke stderr server MCP) |

## Build

Butuh Rust 1.88+ (edition 2024) dan Google Chrome/Chromium (atau set `FASTPILOT_CHROME`). Untuk Android
dan iOS, artefak on-device dibangun dari source (tanpa download):

```bash
agent-android/build.sh
```

```bash
ios-runner/build.sh
```

```bash
cargo build --release
```

- `agent-android/build.sh` butuh Android SDK (build-tools + platform 35) dan JDK 17. Hasilnya adalah agent
  APK (di-embed ke binary saat `cargo build`) dan demo APK.
- `ios-runner/build.sh` butuh Xcode 16+. Hasilnya adalah runner XCUITest dan demo app.

Binary ada di `target/release/fastpilot`.

## Konfigurasi MCP

**Claude Code** (dari root proyek yang mau dites):

```bash
claude mcp add fastpilot -e FASTPILOT_VARS=DEMO_PASS -e DEMO_PASS=rahasia123 -- /path/to/fastpilot mcp
```

atau `.mcp.json`:

```json
{
  "mcpServers": {
    "fastpilot": {
      "command": "/path/to/fastpilot",
      "args": ["mcp"],
      "env": { "FASTPILOT_VARS": "DEMO_PASS", "DEMO_PASS": "rahasia123" }
    }
  }
}
```

**Claude Desktop** (`~/Library/Application Support/Claude/claude_desktop_config.json` di macOS,
`%APPDATA%\Claude\claude_desktop_config.json` di Windows): blok `mcpServers` yang sama. Claude Desktop tidak
menjalankan server dari folder proyek, jadi pakai path absolut di `target`/`openUrl`.

| Env | Fungsi |
|---|---|
| `FASTPILOT_VARS` | daftar variabel (dipisah koma) yang boleh dipakai flow lewat `${NAMA}`; nilainya selalu disamarkan `***` di output |
| `FASTPILOT_CHROME` | path executable Chrome/Chromium |
| `FASTPILOT_HEADFUL=1` | tampilkan jendela browser (sama dengan `fastpilot mcp --headful`) |
| `FASTPILOT_ANDROID_SERIAL` / `FASTPILOT_IOS_UDID` | device default untuk `session: { start, platform }` di flow |
| `FASTPILOT_IOS_DEVICE` | simulator yang di-boot jika belum ada yang menyala (default `iPhone 16`) |
| `FASTPILOT_IOS_RUNNER` | folder build runner iOS (default `ios-runner/build`) |
| `FASTPILOT_DIR` | lokasi `.fastpilot/` (runs + memory) |
| `FASTPILOT_MEMORY` | `rw` (default), `readonly` (CI), atau `off` |
| `FASTPILOT_FLOW_TIMEOUT` | batas waktu satu `run_flow` dalam detik (default 300) |
| `FASTPILOT_LOG` | filter tracing, mis. `debug` (log ke stderr; stdout dipakai MCP) |

## Tools (9)

| Tool | Fungsi |
|---|---|
| `session` | `start` / `stop` / `list` / `devices` / `start_device`. `start` mengembalikan snapshot pertama, dan hint path memory jika `test_id` diberikan. Web: `mode: launch\|attach` (+`endpoint`). Android/iOS: `endpoint` = serial/UDID (untuk `start_device`: nama AVD/simulator) |
| `run_flow` | Jalankan flow YAML (multi-langkah, multi-sesi, lintas platform, `parallel:`). Mengembalikan timing per langkah dan diff per sesi. Dengan `test_id`, flow dan evidence direkam |
| `check_flow` | Validasi tanpa eksekusi: syntax, step belum didukung, sesi tak dikenal, aturan parallel, `${VAR}` tidak diizinkan |
| `snapshot` | Snapshot ringkas. `filter` = find (hasil + ancestor), `coords`, `full`, `file` (tulis ke `.fastpilot/snapshots/`). Jarang dibutuhkan |
| `screenshot` | JPEG. `annotate: som` = kotak bernomor per target (digambar di web) + daftar ref → koordinat |
| `assert` | `visible` / `not_visible` / `text_equals` / `contains` / `enabled`, deterministik dengan auto-retry |
| `report` | Tutup run: `report.md`, `steps.jsonl`, `assertions.json`, screenshot kegagalan. Status `pass` memperbarui path memory |
| `state` | Simpan/muat login (web: cookies + localStorage) supaya test berikutnya tidak perlu login: `save` / `load` / `list` / `delete`. File hanya bisa dibaca pemiliknya, kedaluwarsa setelah 30 hari (`FASTPILOT_STATE_EXPIRE_DAYS`), dan dienkripsi AES-256-GCM bila `FASTPILOT_STATE_KEY` (64 hex, `openssl rand -hex 32`) di-set. Sama dengan langkah `saveState` / `loadState` |
| `docs` | Referensi on-demand: `dsl`, `selectors`, `parallel`, `examples`, `errors`, `memory` |

## Contoh

Login web dalam satu round-trip ([tests/login.md](tests/login.md),
[examples/demo/login.html](examples/demo/login.html)):

```yaml
- session: { start: web1, platform: web, target: "examples/demo/login.html" }
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- tapOn: "Masuk"
- assertVisible: "Budi"
```

```
ok 5/5 steps 1.70s
  ✓ 1 [web1] session start web1 (web) 1063ms
  ✓ 2 [web1] inputText "Email" "a@b.com" 78ms
  ✓ 3 [web1] inputText "Password" "${DEMO_PASS}" 84ms
  ✓ 4 [web1] tapOn "Masuk" 83ms
  ✓ 5 [web1] assertVisible "Budi" 385ms
[web1] screen "Dashboard" (file:///.../examples/demo/login.html)
e7 heading "Dashboard"
e8 text "Selamat datang,"
e9 text "Budi"
e10 button "Keluar"
```

Android + WebView ([tests/login-android.md](tests/login-android.md)). Langkah di halaman WebView
dijalankan lewat CDP:

```yaml
- session: { start: and1, platform: android }
- launchApp: com.fastpilot.demo/.LoginActivity
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- tapOn: "Masuk"
- assertVisible: "Selamat datang, Budi"
- tapOn: "Bayar"
- tapOn: "Bayar sekarang"
- assertVisible: "Pembayaran berhasil"
```

Lintas platform dalam satu `run_flow` ([tests/login-cross-platform.md](tests/login-cross-platform.md)):

```yaml
- session: { start: web1, target: "examples/demo/login.html" }
- session: { start: and1, platform: android }
- session: and1
- launchApp: com.fastpilot.demo/.LoginActivity
- parallel:
    - session: web1
      steps:
        - inputText: { into: "Email", text: "a@b.com" }
        - inputText: { into: "Password", text: "${DEMO_PASS}" }
        - tapOn: "Masuk"
        - assertVisible: "Budi"
    - session: and1
      steps:
        - inputText: { into: "Email", text: "a@b.com" }
        - inputText: { into: "Password", text: "${DEMO_PASS}" }
        - tapOn: "Masuk"
        - assertVisible: "Selamat datang, Budi"
```

Langkah gagal berhenti dini dan mengembalikan kandidat:

```
fail step 1/1 [web1] tapOn "Hapus Akun": not_found: no element matches "Hapus Akun" (timeout 5000ms)
candidates: e10 button "Keluar"
```

Path memory: `session start` dengan `test_id` mengembalikan hint YAML dari run lolos terakhir. Kirim hint
itu sebagai satu `run_flow` dengan `test_id` yang sama, lalu tutup dengan `report`. Lihat `docs` topic `memory`.

## Shell CLI: satu perintah per langkah

Untuk agent yang hanya bisa menjalankan perintah shell, atau untuk dicoba manual. Sesi tetap terbuka di
daemon latar belakang (`~/.fastpilot/cli.sock`, dinyalakan otomatis oleh perintah pertama, mati sendiri
setelah 30 menit tanpa perintah atau lewat `fastpilot daemon stop`), jadi tiap perintah cepat.

```bash
export FASTPILOT_VARS=DEMO_PASS DEMO_PASS=rahasia123   # dibaca daemon saat menyala
fastpilot open examples/demo/login.html
fastpilot snapshot -i                  # hanya elemen yang bisa diaksi, dengan ref
fastpilot fill @e2 a@b.com
fastpilot fill "Password" '${DEMO_PASS}'
fastpilot click @e5
fastpilot wait Budi && fastpilot get title
fastpilot screenshot hasil.jpg --annotate
fastpilot close
```

- Selector: `@e7` (ref dari snapshot), `"Masuk"` (teks), `#email` (id), `role=button:Masuk`, `120,340` (titik).
- Perintah: `open`, `attach`, `close [--all]`, `sessions`, `devices`, `snapshot [-i] [--json] [--full]
  [--coords] [--filter T]`, `click`/`tap`, `dblclick`, `longpress`, `fill`, `type`, `clear`, `press`,
  `select`, `check`/`uncheck`, `upload`, `scroll`, `scrollto`, `swipe`, `back`, `forward`, `reload`,
  `wait <ms|sel> [--gone]`, `wait --url`, `assert`, `is visible|enabled`, `get text|url|title`, `eval`,
  `screenshot`, `record start|stop`, `set viewport|locale|geo|network|permission`, `launch`, `stop-app`,
  `clear-state`, `hide-keyboard`, `flow <file|->`, `check-flow`, `state save|load|list|delete`, `errors`,
  `docs`, `daemon status|stop`.
  `fastpilot help` menampilkan semuanya.
- Web, Android dan iOS sama saja: `fastpilot open com.app/.MainActivity --platform android`.
- Beberapa sesi sekaligus: `--session NAMA` (atau `FASTPILOT_SESSION`); `--json` untuk output mentah.
- Exit code ≠ 0 bila langkah gagal; outputnya alasan, kandidat, dan layar saat itu, sama seperti MCP.
- macOS dan Linux (Unix socket). Secret di `FASTPILOT_VARS` tetap tersamar `***`.

## Tanpa membuka Claude Code: `fastpilot run`

Menjalankan test case bahasa natural (`tests/*.md`) tanpa sesi chat: fastpilot memanggil **Claude Code
milik user** (`claude -p`, login dan langganan user sendiri) dan memberinya tool fastpilot lewat MCP.
fastpilot tidak memanggil API model sendiri, jadi **tidak perlu API key**. Syaratnya `claude` ter-install
dan sudah login sekali (`claude`); lokasi lain bisa diatur lewat `FASTPILOT_CLAUDE`.

```bash
DEMO_PASS=rahasia123 FASTPILOT_VARS=DEMO_PASS \
  ./target/release/fastpilot run tests/ --junit junit.xml --html report.html
```

- Claude Code hanya mendapat tool fastpilot: tool bawaan (shell, file, web) dimatikan dan izin tool
  fastpilot disetujui otomatis, karena tidak ada yang menunggu di depan layar.
- Test dengan hint path memory diputar ulang oleh model cepat (`claude-haiku-5-5`); bila replay gagal
  atau belum ada hint, model besar (`claude-opus-5-5`) melanjutkan dari layar terakhir. `--model` memaksa
  satu model, `--max-steps` membatasi jumlah panggilan tool per test (default 30).
- Kegagalan diulang selama `FASTPILOT_RETRY_BUDGET` (default 3) masih ada; retry yang lolos = flaky.
  Setelah `FASTPILOT_BREAKER` (default 3) kegagalan infra beruntun, sisa test dilewati sebagai infra.
- `--workers N` menjalankan N test sekaligus. Tiap test punya Chrome sendiri; device Android dan simulator
  iOS yang sudah boot dipinjamkan ke satu test pada satu waktu (dicek sehat saat dipinjam, device yang
  gagal cek dikeluarkan), retry memakai device lain bila ada, dan antrean diurutkan dari durasi tercatat
  terlama (test yang belum pernah jalan lebih dulu).
- `--record` merekam layar tiap sesi (web: screencast CDP jadi filmstrip HTML; Android: `screenrecord`;
  iOS: `simctl io recordVideo`). `--html` menghasilkan satu file HTML mandiri yang memuat laporan tiap
  run (langkah, durasi, diff, screenshot, rekaman); `--junit` untuk dashboard CI.
- Exit code ≠ 0 bila ada test di luar karantina yang gagal. Nilai `FASTPILOT_VARS` yang tertulis di
  test case dikirim ke model sebagai `${NAMA}`, tidak pernah sebagai nilainya.

## Chrome harian dan iPhone fisik

- **Chrome sehari-hari tanpa restart**: muat `extension-chrome/` (chrome://extensions → Developer mode →
  Load unpacked), minta `session { action: start, mode: attach, endpoint: extension }`, lalu klik tombol
  extension di tab yang mau dikendalikan. Detail di [extension-chrome/README.md](extension-chrome/README.md).
- **iPhone/iPad dan ponsel Android fisik**: lihat [docs/real-devices.md](docs/real-devices.md). Ringkasnya:
  cek kesiapan dengan `fastpilot ios doctor`, build runner untuk iPhone, lalu mulai sesi dengan
  `device: ios-device:<udid>` atau `device: android-device:<serial>`.

```bash
FASTPILOT_IOS_TEAM=<team id> FASTPILOT_IOS_BUNDLE_PREFIX=com.<nama>.fastpilot ios-runner/build.sh --device <udid>
```

## Bench

```bash
./target/release/fastpilot bench --iterations 30
```

```bash
./target/release/fastpilot bench android --iterations 20
```

Hasil di M1 Pro (Chrome headless; emulator Android API 35; simulator iPhone 16 iOS 18.1):

| Metrik | Web | Android | iOS | Target PRD |
|---|---|---|---|---|
| Session start | 149 ms (warm) | 629 ms | ±23 s (xcodebuild menyalakan runner) | <1 s / <2 s |
| Snapshot p50 | 1.1 ms | 4.3 ms | 16–55 ms | <50 / <100 ms |
| Tap → diff p50 | 119 ms | 154 ms (dengan verifikasi tap) | ±0.4–1.4 s (XCUITest menunggu animasi) | <200 ms |

iOS diukur manual lewat klien protokol; belum ada `bench ios`.

Biaya token yang dibaca AI (aproksimasi offline; aturannya dicetak di output):

```bash
./target/release/fastpilot bench tokens web      # atau android / ios
```

| Yang dibaca AI (aproksimasi) | Web | Android |
|---|---|---|
| Snapshot layar login | 111 | 86 |
| `run_flow` 1 langkah (tap → diff) | 92 | 60 |
| `run_flow` login 4 langkah | 236 | 379 |
| Screenshot sebagai pembanding (w·h/750) | 1366 | 1229 |

Stabilitas (PRD G6) diukur dengan pass^k dan batas atas failure rate, bukan dari satu angka "lolos 20 kali":

```bash
FASTPILOT_VARS=DEMO_PASS DEMO_PASS=rahasia123 ./target/release/fastpilot stability examples/flows/login-web.yaml --runs 30
```

```
30/30 passed (100.0%), p50 1.11s
pass^1: 1.000
pass^3: 1.000
pass^8: 1.000
failure rate <= 11.4% (95% Wilson upper bound); a <=1% claim needs >= 381 runs without failure
```

## Test

```bash
cargo test
```

- Unit test: `crates/core` (DSL, `check_flow`, prune/ref/diff, resolusi selector, engine), `crates/memory`,
  dan konversi node di `crates/android` serta `crates/ios`.
- `bin/fastpilot/tests/mcp_login.rs` menjalankan `fastpilot mcp` lewat stdio dengan Chrome sungguhan:
  - login, stop-early, dua sesi paralel;
  - widget (select, klik div, confirm dialog, pressKey, upload, iframe, shadow DOM);
  - assert, report, dan path memory;
  - attach ke Chrome yang sudah berjalan.
- `bin/fastpilot/tests/web_advanced.rs` menguji iframe lintas situs (OOPIF), popup, download, elemen
  `opacity:0`, error console/HTTP di laporan gagal, Set-of-Mark, snapshot find/file, dan tool di sesi lain
  yang tidak ikut menunggu flow yang sedang berjalan.
- `bin/fastpilot/tests/android.rs` menguji login Android, WebView, dan skenario web+Android paralel di
  device/emulator yang menyala.
- `bin/fastpilot/tests/ios.rs` menguji login di iOS simulator.
- Test yang butuh Chrome, device, atau simulator otomatis di-skip kalau tidak tersedia.

## Layout

```
crates/core      Driver trait, Selector, Snapshot (prune, ref stabil, diff), DSL + check_flow, engine, assert, SessionRegistry
crates/web       CDP client, launch/attach Chrome, WebPage (Driver) per BrowserContext
crates/android   adb server protocol, agent client, AndroidDriver (+ WebView via CDP)
crates/ios       simctl/devicectl + XCUITest runner, usbmux, Web Inspector, IosDriver
crates/memory    run evidence (.fastpilot/runs) + path memory (.fastpilot/memory)
crates/mcp       rmcp stdio server, 8 tools, docs
bin/fastpilot    entrypoint: `mcp`, `bench [web|android]`, `stability <flow> --runs N`
agent-android/   instrumentation APK (Java, UiAutomation), dibangun tanpa Gradle
ios-runner/      XCUITest runner + demo app iOS
examples/        demo web (login, widgets), demo Android
tests/           test case bahasa natural
```

## Batasan yang diketahui

- iOS device fisik: kode sudah ada (usbmux/tunnel, `fastpilot ios doctor`) tapi belum diuji di hardware.
  Butuh Xcode yang cocok dengan iOS-nya (iOS 26 → Xcode 26) dan Apple ID di Xcode; profil gratis berlaku
  7 hari dan maksimal 3 app per device. `setLocation`, `pushNotification`, rekaman layar, `clearKeychain`
  dan izin allow/deny hanya ada di simulator. Lihat [docs/real-devices.md](docs/real-devices.md).
- iOS WKWebView: lewat Web Inspector, hanya jika app memasang `webView.isInspectable = true` (iOS 16.4+);
  di iPhone juga Settings → Safari → Advanced → Web Inspector. Tap web berupa `click()` DOM (bukan
  sentuhan), dengan beberapa WKWebView dipakai halaman pertama, dan jalur RemoteXPC belum ada.
- iOS `setNetwork`: tidak didukung (simulator memakai jaringan Mac).
- Web attach lewat extension relay ([extension-chrome/](extension-chrome/)) perlu klik tombol extension
  di tab yang mau dikendalikan; tanpa extension: Chrome M144+ `autoConnect` atau `--remote-debugging-port`.
