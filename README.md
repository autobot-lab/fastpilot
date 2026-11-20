# fastpilot

QA otomatis yang dijalankan AI untuk **web, Android, dan iOS**, dalam **satu binary** di komputermu.
Kamu menulis test dalam bahasa biasa; Claude Code menjalankannya lewat fastpilot (server MCP), juga lintas
platform dalam satu skenario. Tanpa Appium, tanpa cloud, dan tombol pembayaran diblokir sejak awal.

## 10 menit pertama

Butuh: macOS atau Linux, Chrome, dan [Claude Code](https://claude.com/claude-code) yang sudah login.

**1. Pasang** (± 1 menit)

```bash
npm install -g fastpilot
```

**2. Coba demo** (± 3 menit)

```bash
fastpilot demo && cd fastpilot-demo && claude
```

Ketik di Claude: *"Pakai fastpilot, jalankan tests/login.md lalu report"* (setujui server `fastpilot` saat
Claude menanyakannya). Hasil: ✓ `login-ok` lolos, laporan di `.fastpilot/runs/`. `fastpilot demo` membuat
halaman demo, contoh test, `.mcp.json` (fastpilot + password demo publik), dan Skill fastpilot di folder itu.

**3. Pakai di proyekmu** (± 5 menit)

```bash
cd ~/proyek-kamu && fastpilot init
```

`init` mengecek mesin, mendaftarkan fastpilot ke Claude Code, memasang Skill, membuat `tests/contoh.md`,
dan menampilkan Batasan; tiap perubahan ditanya dulu (`--yes` untuk menyetujui semua). Ubah `target` di
`tests/contoh.md` ke URL aplikasimu, lalu minta Claude menjalankannya.

### Kalau macet

Jalankan `fastpilot doctor` dulu; biasanya penyebab dan perbaikannya langsung tampil.

| Lihat pesan | Lakukan |
|---|---|
| `Chrome atau Chromium tidak ditemukan` | `fastpilot setup web` |
| fastpilot tidak muncul di Claude | `claude mcp list`, lalu `fastpilot init` lagi |
| `blocked: … (block_actions, built-in list)` / "Diblokir Batasan" di Live | sengaja: tombol pembayaran dan hapus akun diblokir bawaan. Ubah di `fastpilot ui` → Pengaturan › Batasan & izin |
| `evalScript is off while block_actions is on` | toggle "Jalankan JavaScript bebas" di Batasan & izin, atau `fastpilot config set allow_eval_script true` |
| `looks like a risky action … confirm_actions` | tombol hapus/delete/remove: tulis langkahnya dengan `confirm: true` kalau memang dimaksud |
| `build fastpilot ini tidak membawa agent Android` | pakai binary rilis (npm/curl), atau `agent-android/build.sh` lalu build ulang |
| `runner XCUITest belum di-build` | `ios-runner/build.sh` dari checkout (iOS, lihat di bawah) |
| `variable X is not allowed` | tambahkan `X` ke `FASTPILOT_VARS` di konfigurasi MCP |
| Log detail | tambahkan `-e FASTPILOT_LOG=debug` (log ke stderr server MCP) |

### Android

```bash
fastpilot setup android
```

```bash
fastpilot demo --android
```

`setup android` menyiapkan SDK dan emulator (ditanya dulu tiap langkah); `demo --android` memasang app demo
di device atau emulator yang tersambung (`--device SERIAL` kalau ada beberapa). Lalu di folder demo:
*"Pakai fastpilot, jalankan tests/login-android.md"*. Langkah terakhirnya mencoba menekan "Bayar" dan
mengharapkan fastpilot menolaknya.

### iOS (menyusul di v0.3.3; sekarang dari checkout)

iOS butuh Xcode dan runner XCUITest yang dibangun dari checkout (`ios-runner/build.sh`, lihat
[Build dari source](#build-dari-source)).

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

### Claude Desktop

Tambahkan blok berikut ke `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) atau
`%APPDATA%\Claude\claude_desktop_config.json` (Windows), lalu restart Claude Desktop:

```json
{
  "mcpServers": {
    "fastpilot": {
      "command": "/Users/NAMA/.local/bin/fastpilot",
      "args": ["mcp"],
      "env": { "FASTPILOT_VARS": "DEMO_PASS", "DEMO_PASS": "rahasia123", "FASTPILOT_DIR": "/Users/NAMA/fastpilot-runs" }
    }
  }
}
```

Claude Desktop tidak menjalankan server dari folder proyek. Karena itu, pakai path command absolut (lihat
`which fastpilot`; `~/.local/bin` untuk skrip instal, `~/.cargo/bin` untuk `cargo install`), URL atau path
absolut di `target`, dan set `FASTPILOT_DIR` ke folder hasil yang bisa ditulisi.

### Batasan aman bawaan

Sejak 0.3.2, selama kamu belum mengisi daftarnya:

- **Selalu diblokir** (`block_actions`): bayar, pay, beli, buy, checkout, purchase, transfer, buat pesanan,
  place order, hapus akun, delete account. Dicocokkan per kata utuh ("Pay now" dan "Delete my account" kena,
  "Paypal" tidak), juga dengan `confirm: true`.
- **Butuh konfirmasi** (`confirm_actions`): hapus, delete, remove. AI harus menulis `confirm: true`.
- **evalScript mati** selama daftar blokir aktif, karena script bisa menekan tombol yang diblokir. Nyalakan
  sendiri lewat toggle "Jalankan JavaScript bebas" atau `fastpilot config set allow_eval_script true`;
  AI tidak bisa menyalakannya.
- **Mengosongkan:** tombol **Kosongkan** di `fastpilot ui` › Pengaturan › Batasan & izin, atau
  `fastpilot config set block_actions ""`, lalu restart server MCP. Kembali ke bawaan:
  `fastpilot config unset block_actions`. AI (lewat chat atau tool `config`) hanya bisa memperketat.

Semua pengaturan: [Pengaturan](#pengaturan).

## Detail pemasangan

### Cara pasang lain

```bash
curl -fsSL https://raw.githubusercontent.com/autobot-lab/fastpilot/main/scripts/install.sh | sh
```

- npm dan skrip mengunduh arsip rilis dari [GitHub Releases](https://github.com/autobot-lab/fastpilot/releases)
  dan mengecek SHA-256-nya sebelum memasang. Skrip memasang ke `~/.local/bin`; lokasi lain:
  `FASTPILOT_INSTALL_DIR=/usr/local/bin`, versi tertentu: `FASTPILOT_VERSION=0.3.1`.
- Tanpa instal: `npx -y fastpilot mcp`.
- Homebrew belum tersedia (repo tap belum dibuat).
- Agent Android sudah tertanam di binary rilis. iOS tetap butuh Xcode dan runner dari checkout
  (`ios-runner/build.sh`, lihat [Build dari source](#build-dari-source)).

### Cek dan siapkan mesin

```bash
fastpilot doctor
```

`doctor` mengecek tiap platform dan menampilkan ✓/!/✗ dengan petunjuk perbaikan:

- **web:** Chrome ditemukan dan bisa dinyalakan headless.
- **android:** `adb`, agent tertanam, device/emulator menyala, AVD tersedia.
- **ios:** Xcode, simulator, runner XCUITest.
- **run:** Claude Code (`claude`) untuk `fastpilot run`.

`fastpilot doctor android` (atau `web`, `ios`) hanya mengecek satu platform dan keluar dengan kode ≠ 0
kalau platform itu belum siap. `fastpilot doctor` tanpa argumen menampilkan semua platform, tapi kode
keluarnya hanya ditentukan oleh web (≠ 0 bila ada ✗ di web), aturan yang sama dengan `setup`. Di CI,
sebut platform yang dipakai, mis. `fastpilot doctor web android`. Untuk memperbaiki yang bisa diperbaiki otomatis (pasang Chrome lewat
Homebrew, unduh image Android, buat AVD `fastpilot_api35`, nyalakan emulator):

```bash
fastpilot setup android
```

`setup` menanyakan tiap perbaikan (y/N) sebelum menjalankannya; `--yes` untuk menyetujui semuanya. Di luar
terminal (CI, agent) `setup` hanya mencetak rencananya. Seperti `doctor`, `setup` keluar dengan kode ≠ 0
kalau sesudahnya masih ada ✗ (yang tidak bisa diperbaiki otomatis, dilewati, gagal, atau baru direncanakan)
di platform yang disebut, atau di web bila tidak ada platform yang disebut.

### Daftarkan ke Claude Code secara manual

`fastpilot init` melakukan ini untukmu. Manualnya, dari folder proyek:

```bash
claude mcp add fastpilot -e FASTPILOT_VARS=DEMO_PASS -e DEMO_PASS=rahasia123 -- fastpilot mcp
```

```bash
claude mcp list
```

`fastpilot` harus tampil dengan status connected. Variabel di `FASTPILOT_VARS` (Secret) adalah satu-satunya
variabel yang boleh dipakai flow lewat `${NAMA}`, dan nilainya selalu tampil sebagai `***` di output. Untuk
melihat browser saat test berjalan, tambahkan `-e FASTPILOT_HEADFUL=1` (atau `fastpilot config set headful true`).

Diuji dengan Claude Code dan Claude Desktop. Klien MCP lain (Cursor, Codex, Gemini CLI) memakai cara yang
sama (`fastpilot mcp` lewat stdio), tapi belum diuji.

**Agent Skill** (`fastpilot init` dan `fastpilot demo` sudah memasangnya). [`skills/fastpilot/SKILL.md`](skills/fastpilot/SKILL.md) (format
[Agent Skills](https://agentskills.io)) mengajari agent cara memakai fastpilot: alur `session` →
`run_flow` → `report`, hint path memory, secret sebagai `${NAMA}`, dan batasan yang tidak boleh diakali.
Claude Code memuatnya otomatis saat tugasnya cocok:

```bash
mkdir -p ~/.claude/skills && cp -R skills/fastpilot ~/.claude/skills/        # semua proyek
```

```bash
mkdir -p .claude/skills && cp -R /path/ke/fastpilot/skills/fastpilot .claude/skills/   # satu proyek
```

Tanpa checkout: unduh foldernya dari GitHub (`skills/fastpilot/`). Klien lain yang mendukung format Agent
Skills memakai folder yang sama di lokasi skill masing-masing (belum diuji).

### Menulis test sendiri

Tulis test case Markdown seperti `tests/login.md`: judul, `id`, `platform`, `target`, lalu langkah dan
harapan dalam bahasa biasa. Tidak perlu menulis selector atau script; Claude yang menyusun flow-nya.
Simpan secret di konfigurasi MCP (`-e NAMA=nilai`, lalu tambahkan `NAMA` ke `FASTPILOT_VARS`), bukan di
file config. Menjalankan test yang sama dua kali: kali kedua `session start` mengembalikan *hint* dari run
sebelumnya, sehingga test bisa selesai dalam satu `run_flow`.

## Fitur

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

## Pengaturan

Pengaturan dibaca berurutan: **env** (mis. `-e` di MCP config) → **file** `~/.fastpilot/config.toml` (atau
path di `FASTPILOT_CONFIG`) → default. File dibaca ulang saat berubah, jadi tidak perlu restart server.
Kecuali batasan: nilainya dikunci saat fastpilot mulai (server MCP, daemon shell, `fastpilot run`), dan
selama jalan hanya nilai yang lebih ketat yang diikuti. Nilai yang lebih longgar baru berlaku setelah
restart (server MCP atau `fastpilot daemon stop`), juga kalau batasan itu belum pernah dipakai. File yang tidak valid
membuat fastpilot berhenti dengan pesan error, tidak diam-diam diabaikan.

```bash
fastpilot config list                              # semua kunci, nilai, dan asalnya
fastpilot config set allowed_domains "staging.contoh.com,*.contoh.com"
fastpilot config get flow_timeout
fastpilot config unset allowed_domains
fastpilot config path
```

```toml
# ~/.fastpilot/config.toml
allowed_domains = ["staging.contoh.com", "*.contoh.com"]
headful = true
flow_timeout = 600
```

**Lewat browser:** `fastpilot ui` membuka halaman Live, Pengaturan, dan Device di browser default (hanya
`127.0.0.1`, port acak; `--no-open` hanya mencetak link-nya, `--port N` memilih port, `--help` bantuan).
Link-nya berisi kode **sekali pakai** yang berlaku 10 menit; begitu dibuka, kode ditukar ke cookie sesi
dan tidak berlaku lagi (jalankan `fastpilot ui` lagi untuk link baru). Jalankan `fastpilot ui` sendiri,
bukan lewat agent AI: siapa pun yang membuka halamannya bisa melonggarkan batasan. Kalau
`config.toml` rusak, halaman Pengaturan menampilkan error-nya dan tidak menyimpan apa pun sampai file
diperbaiki.
Isinya Model AI untuk `fastpilot run`, Batasan & izin, nama Secret (nilainya tidak pernah tampil), dan
hasil `doctor` dengan tombol Cek ulang. Perubahan dari halaman ini adalah perubahan user: boleh
melonggarkan, ditulis atomik ke file (mode 600), dan berlaku mulai run berikutnya. Semua aset dan font
ada di dalam binary, jadi halaman ini jalan tanpa internet.

**Panel Live** (`fastpilot ui`, menu Live): sesi yang sedang berjalan dari server MCP (Claude Code),
CLI, dan `fastpilot run`, masing-masing dengan langkah flow-nya (✓ selesai, ▶ berjalan, 🔒 diblokir
atau akan diblokir Batasan) dan layarnya secara live, hanya selama panel terbuka:

- **Web** (Chrome milik fastpilot): screencast lewat koneksi DevTools terpisah, sekitar 2 fps untuk
  thumbnail dan 8 fps saat diperbesar.
- **Android dan iOS**: screenshot dari device (agent/runner yang sama dengan langkah `takeScreenshot`),
  1 fps untuk thumbnail dan 3 fps saat diperbesar. Ini juga berlaku untuk HP fisik dan adb di mesin lain.
- Chrome milikmu (mode attach) tidak ditampilkan; langkahnya tetap terlihat.

**Ambil alih.** Tombol *Ambil alih* di kartu sesi menjeda langkah otomatis sesi itu dan membuka layar besar:
klik untuk tap, seret untuk swipe, kolom *Ketik* untuk teks di kolom yang aktif, tombol Enter/Hapus (dan
*Kembali* di Android/iOS). Langkah AI berikutnya menunggu sampai kamu menekan *Kembalikan*, lalu hasil
`run_flow` berikutnya untuk sesi itu memberi tahu AI bahwa user sempat mengambil alih (berapa lama, berapa
tap/teks); dengan `test_id` catatan ini juga masuk `steps.jsonl` dan laporan run. Batasan tetap berlaku
untuk semua langkah otomatis, sedangkan input yang kamu ketik sendiri tidak diperiksa Batasan.

- Input hanya diterima dari halaman `fastpilot ui` (cookie sesi, Origin, JSON). Halaman meneruskannya ke
  proses fastpilot pemilik sesi lewat socket `~/.fastpilot/live/<pid>.sock` (folder 0700, socket 0600).
- Socket itu hanya melayani proses `fastpilot ui` dari **file program yang sama** dengan proses fastpilot
  pemilik sesi: pid dan path program di ujung lain dicek tiap koneksi, jadi skrip, agent, atau program lain
  yang sekadar bernama `fastpilot` ditolak (juga untuk frame). Akibatnya `fastpilot ui` dan server MCP harus
  dari instalasi yang sama (mis. keduanya dari npm), dan binary yang di-rename tidak bisa memakai Ambil alih.
  Jalankan `fastpilot ui` dengan cara yang sama seperti server MCP-nya: kalau MCP didaftarkan sebagai
  `npx -y fastpilot mcp`, buka halamannya dengan `npx -y fastpilot ui`. Tiap ambil alih memberi token acak yang
  hanya ada di memori proses fastpilot dan `fastpilot ui` yang memulainya (tidak di file, tidak dikirim ke
  browser). Tap, ketik, dan Kembalikan wajib membawa token itu, jadi halaman lain tidak bisa ikut
  mengendalikan atau menutup ambil alih orang lain.
- Ambil alih berakhir sendiri bila halaman berhenti mengirim tanda hidup selama ±45 detik (tab ditutup,
  laptop tidur) atau setelah 30 menit. Menutup layar besar (Tutup / Escape) juga mengembalikan sesi. Selama
  diambil alih, penanda kuning tampil di semua halaman `fastpilot ui` dengan tombol *Kembalikan ke AI*.
- **Sisa risiko, jujur saja:** pemeriksaan di atas membedakan `fastpilot ui` dari proses lain milik user
  yang sama, tetapi bukan batas keamanan terhadap kode yang sudah berjalan sebagai kamu. Agent yang punya
  akses shell (Bash) bisa menjalankan `fastpilot ui` sendiri, membaca link sekali pakainya, lalu mengambil
  alih tanpa klik manusia; input ambil alih memang tidak diperiksa Batasan. Agent seperti itu juga bisa
  melewati Batasan dengan cara lain (menjalankan Chrome atau `adb` langsung). Batasan fastpilot melindungi
  dari AI yang hanya memegang tool fastpilot; untuk agent dengan shell, batasi akses shell-nya di aplikasi
  agent itu, dan matikan Ambil alih: `fastpilot config set allow_takeover false` (atau Pengaturan › Batasan ›
  *Boleh ambil alih sesi*). Seperti batasan lain, nilai ini dikunci saat proses fastpilot mulai, jadi
  menyalakannya lagi baru berlaku setelah restart.
- Waktu selama diambil alih tetap dihitung ke batas waktu flow (`FASTPILOT_FLOW_TIMEOUT`).
- macOS dan Linux saja (Unix socket).

Data sesi ada di `~/.fastpilot/live/` (hanya bisa dibaca pemiliknya); alamat DevTools tidak pernah
dikirim ke browser.

**Jadwal** (`fastpilot ui`, menu Jadwal): jalankan test (`fastpilot run` pada folder `tests/` atau satu
file `.md`) setiap hari, hari kerja, atau mingguan pada jam tertentu (waktu komputer ini).

- Jadwal disimpan di `~/.fastpilot/schedules.json` dan didaftarkan ke penjadwal sistem **hanya saat kamu
  menyimpannya**: LaunchAgent launchd di macOS (`~/Library/LaunchAgents/dev.fastpilot.schedule.<id>.plist`),
  systemd user timer di Linux, atau crontab bila systemd tidak ada. Mematikan atau menghapus jadwal
  menghapus entri itu. Entri hanya berisi `fastpilot schedule run <id>`; path test dan notifikasi ada di
  file JSON.
- **Kalau gagal**: webhook https yang kompatibel Slack (`{"text": ...}`), notifikasi di komputer ini,
  dan/atau issue GitHub di `pemilik/repo` (token dari `GITHUB_TOKEN`/`GH_TOKEN`, atau Keychain item
  `fastpilot-github`: `security add-generic-password -s fastpilot-github -a fastpilot -w`). Tombol
  *Tes notifikasi* mengirim pesan contoh.
- URL webhook diperlakukan sebagai rahasia: disimpan di `~/.fastpilot/secrets/` (hanya pemilik), tidak
  di file jadwal, dan halaman hanya menampilkan host-nya.
- Komputer harus menyala dan kamu login saat jadwal jalan (launchd menjalankan yang terlewat saat bangun
  dari sleep; systemd memakai `Persistent=true`). Laporan HTML tiap run ada di
  `~/.fastpilot/schedules/reports/`, log di `~/.fastpilot/schedules/<id>.log`.
- Dari terminal: `fastpilot schedule list`, `fastpilot schedule run <id>`, `fastpilot schedule remove <id>`.

**Chat** (`fastpilot ui`, menu Chat): minta fastpilot bot menjalankan test, menjelaskan kegagalan, membuat
jadwal, atau memperketat batasan dengan bahasa biasa. Tiap pesan dijawab oleh Claude Code-mu sendiri
(`claude -p`, login-mu, tanpa API key) dengan tool fastpilot saja, sama seperti `fastpilot run`;
percakapan berlanjut dengan `--resume`. Sesi yang dibuka bot muncul di Live dan bisa diambil alih.

- Bot **tidak bisa** mengubah pengaturan atau jadwal sendiri: tool `config` tidak tersedia untuknya. Ia
  hanya mengusulkan, dan usulan tampil sebagai kartu (Jadwal baru / Batasan baru). Tidak ada yang berubah
  sampai kamu klik *Simpan jadwal* atau *Terapkan batasan*.
- Lewat chat batasan hanya bisa **diperketat**. Usulan yang melonggarkan (mis. `max_steps` lebih besar,
  menghapus kata dari `block_actions`) ditolak di kartunya dan juga saat diklik; melonggarkan hanya lewat
  Pengaturan › Batasan.
- URL webhook dan token tidak pernah lewat chat: jadwal dari chat yang meminta webhook disimpan tanpa
  webhook, lalu URL-nya diisi di Jadwal › Ubah.
- Model: `model_explore` (lihat Pengaturan › Model AI).

Batasan dan izin yang bisa diatur (semua juga kunci `fastpilot config`):

| Kunci | Arti |
|---|---|
| `allowed_domains` | sesi web hanya membuka host ini |
| `confirm_actions` | tap pada elemen bernama begini butuh `confirm: true` (cocok per kata). **Bawaan (sejak 0.3.2, selama belum diisi):** `hapus`, `delete`, `remove`. `[]` = tanpa konfirmasi |
| `block_actions` | tap pada elemen yang teksnya **berisi** kata ini selalu ditolak, juga dengan `confirm: true` ("Bayar" juga memblokir "Lanjut ke Pembayaran"). **Bawaan (sejak 0.3.2, selama belum diisi):** `bayar`, `pay`, `beli`, `buy`, `checkout`, `purchase`, `transfer`, `buat pesanan`, `place order`, `hapus akun`, `delete account`, dicocokkan per kata utuh ("pay" kena "Pay now", tidak kena "Paypal"); `default` di daftar = kata bawaan itu. Mengosongkan: `fastpilot config set block_actions ""` atau tombol **Kosongkan** di `fastpilot ui` (berlaku setelah server MCP di-restart; variabel env kosong tidak mengosongkannya); kembali ke bawaan: `fastpilot config unset block_actions`. Teks dinormalkan dulu (karakter tak terlihat, huruf fullwidth, spasi aneh), dan frasa bawaan boleh disela satu kata ("Delete my account"). Tap koordinat (`point`) dicek terhadap elemen di titik itu; bila elemennya tidak bisa dipastikan, tap ditolak selama `block_actions`/`confirm_actions` aktif. Juga `pressKey Enter`/`Space` yang bisa menekan tombol itu (dari kolom isian: selama ada tombol terblokir di layar), dan `evalScript` dimatikan selama `block_actions` aktif |
| `allow_app_install` | langkah `installApp` / `uninstallApp` (bawaan: boleh) |
| `allow_app_reset` | `clearState` / `clearKeychain` (bawaan: boleh) |
| `allow_physical_devices` | sesi boleh memakai HP fisik; `false` = hanya emulator/simulator (bawaan: boleh) |
| `allow_device_settings` | `setNetwork`, `setLocation`, `setPermissions`, `setLocale` di Android/iOS (bawaan: boleh) |
| `allow_eval_script` | langkah `evalScript`. Bawaan (sejak 0.3.2): **mati selama `block_actions` aktif** (termasuk daftar bawaan), karena script bisa menekan tombol yang diblokir; boleh kalau daftar blokir dikosongkan. `true` = selalu boleh (hanya user: `fastpilot config set allow_eval_script true` atau toggle "Jalankan JavaScript bebas"; AI tidak bisa menyalakannya) |
| `allow_takeover` | user boleh mengambil alih sesi di `fastpilot ui` (bawaan: boleh; matikan untuk agent yang punya shell) |
| `upload_dirs` | `uploadFile` hanya mengambil file di folder ini (kosong = mana saja) |
| `state_expire_days` | umur maksimal login tersimpan (bawaan 30 hari) |
| `max_steps` / `test_timeout` | `fastpilot run`: maksimal panggilan tool AI per test (bawaan 30) / menit per test (bawaan 30) |

Lewat chat, agent memakai tool `config` dengan aturan:

- **Batasan** (tabel di atas, plus `vars`): agent hanya boleh memperketat (menambah kata/domain,
  mematikan izin, menurunkan batas). Melonggarkan atau menghapusnya hanya oleh user lewat
  `fastpilot config` atau `fastpilot ui`.
- **Program, lokasi, dan mesin lain** (`chrome`, `claude`, `dir`, `ios_runner`, `adb_server`, `chrome_endpoint`):
  hanya user. fastpilot yang sedang jalan memakai nilai saat ia mulai; perubahan file berlaku setelah restart.
- **Lainnya** (`headful`, `model*`, `flow_timeout`, `action_timeout`, `retry_budget`, `breaker`, device default,
  `memory`, `daemon_idle`): bebas. `action_timeout` (detik, bawaan 5) adalah batas satu aksi (tap, ketik,
  pilih, scroll); `inputText` di Android/iOS mendapat 3× nilai itu karena keyboard bisa lambat muncul. Bisa juga lewat env
  `FASTPILOT_ACTION_TIMEOUT` (env menang atas file).
- Secret (`FASTPILOT_STATE_KEY`, token relay) tidak pernah dibaca dari file, hanya dari env. Token relay
  extension dibuat acak untuk tiap sesi; `FASTPILOT_RELAY_TOKEN` hanya untuk browser tanpa native messaging.
- Saat `fastpilot run` (tanpa orang di depan layar), tool `config` tidak tersedia.
- Batasan ini melindungi dari agent yang hanya memakai tool fastpilot. Proses fastpilot yang sedang
  jalan (server MCP, daemon, `run`) mengunci batasan dan kunci "hanya user" sejak ia mulai, jadi file
  yang diubah di tengah jalan tidak berpengaruh. Tapi agent yang juga punya akses shell atau file
  (mis. Bash di Claude Code) bisa mengubah file lalu me-restart fastpilot (`fastpilot daemon stop`).
  Itu batas desain: set batasan lewat **env** di konfigurasi MCP, dan tolak akses itu di izin agent,
  mis. di Claude Code `"deny": ["Bash(fastpilot config:*)", "Bash(fastpilot daemon:*)", "Edit(~/.fastpilot/**)"]`.

Nama kunci = nama env tanpa `FASTPILOT_` dan huruf kecil (`FASTPILOT_FLOW_TIMEOUT` → `flow_timeout`).

## Build dari source

Butuh Rust 1.88+ (edition 2024) dan Google Chrome/Chromium. Untuk Android dan iOS, artefak on-device
dibangun dari source (tanpa download):

```bash
git clone https://github.com/autobot-lab/fastpilot.git && cd fastpilot
```

```bash
agent-android/build.sh
```

```bash
ios-runner/build.sh
```

```bash
cargo install --path bin/fastpilot --locked
```

- `agent-android/build.sh` butuh Android SDK (build-tools + platform 35) dan JDK 17. Hasilnya adalah agent
  APK (di-embed ke binary saat build, jadi jalankan sebelum `cargo`) dan demo APK.
- `ios-runner/build.sh` butuh Xcode 16+. Hasilnya adalah runner XCUITest dan demo app.
- `cargo install` memasang ke `~/.cargo/bin`; `cargo build --release` saja menaruhnya di
  `target/release/fastpilot`.
- Cara merilis: [docs/releasing.md](docs/releasing.md).

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

Semua env di bawah (kecuali `FASTPILOT_STATE_KEY` dan `FASTPILOT_LOG`) juga bisa ditaruh di
`~/.fastpilot/config.toml`; lihat [Pengaturan](#pengaturan). Env selalu menang.

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
| `FASTPILOT_ALLOWED_DOMAINS` | daftar host (dipisah koma, `*.contoh.com` untuk subdomain) yang boleh dibuka sesi web: `openUrl`, link, redirect, dan popup ke host lain diblokir (`blocked`). Resource lain (CDN, API) tetap jalan |
| `FASTPILOT_CONFIRM_ACTIONS` | `default` (hapus, delete, bayar, pay, beli, checkout, transfer, ...) atau daftar kata sendiri: tap pada elemen bernama begitu ditolak (`blocked`) kecuali langkahnya `confirm: true` (CLI: `--confirm`). Tidak diisi: hapus, delete, remove; kosong (`""`): tanpa konfirmasi |
| `FASTPILOT_BLOCK_ACTIONS` | daftar kata yang selalu diblokir. Tidak diisi: daftar bawaan (pembayaran dan hapus akun); kosong (`""`): tidak ada yang diblokir |
| `FASTPILOT_STATE_KEY` / `FASTPILOT_STATE_EXPIRE_DAYS` | kunci AES-256 (64 hex) untuk mengenkripsi login tersimpan / umur maksimalnya (default 30 hari) |
| `FASTPILOT_FLOW_TIMEOUT` | batas waktu satu `run_flow` dalam detik (default 300) |
| `FASTPILOT_MODEL` / `FASTPILOT_MODEL_REPLAY` / `FASTPILOT_MODEL_EXPLORE` | model untuk `fastpilot run` (lihat di bawah) |
| `FASTPILOT_CONFIG` | lokasi file pengaturan (default `~/.fastpilot/config.toml`) |
| `FASTPILOT_ADB_SERVER` | adb server di mesin lain, `host:port` (lihat [Device di mesin lain](#device-di-mesin-lain)) |
| `FASTPILOT_CHROME_ENDPOINT` | Chrome di mesin lain untuk semua sesi web: `http://host:port` atau `ws://...` |
| `FASTPILOT_LOG` | filter tracing, mis. `debug` (log ke stderr; stdout dipakai MCP) |

## Tools (10)

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
| `config` | Lihat dan ubah pengaturan (`~/.fastpilot/config.toml`) lewat chat: `list` / `get` / `set` / `unset`. Agent hanya boleh **memperketat** batasan (`allowed_domains`, `confirm_actions`, `block_actions`, `vars`, `state_expire_days`, izin `allow_*`, `upload_dirs`, `max_steps`, `test_timeout`); pengetatan langsung berlaku, pelonggaran baru setelah restart. Path program, lokasi, dan mesin lain (`chrome`, `claude`, `dir`, `ios_runner`, `adb_server`, `chrome_endpoint`) hanya bisa diubah user lewat `fastpilot config set` atau `fastpilot ui` |
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

Membuka app ke server staging atau mode debug tanpa build ulang (`launchApp` dengan `arguments` dan `env`):

```yaml
- launchApp:
    appId: com.contoh.app
    arguments: { server: "https://staging.contoh.id", debug: true, retries: 3, fitur: [baru, beta] }
    env: { API_TOKEN: "${STAGING_TOKEN}" }   # hanya iOS
```

- `arguments`: di Android menjadi intent extras (teks, angka bulat, `true`/`false`, daftar teks tanpa
  koma; dibaca dengan `getIntent().getStringExtra(...)` dan sejenisnya), di iOS menjadi launch argument
  `-server https://...` (dibaca dengan `UserDefaults.standard.string(forKey: "server")`).
- `env` (iOS): environment app (`ProcessInfo.processInfo.environment`). Nilainya selalu tampil `***`
  di output dan laporan; run yang direkam hanya menyimpan referensi `${NAMA}`, jadi taruh secret di
  `FASTPILOT_VARS`. Nama yang mengubah cara app dimuat (`DYLD_*`, `LD_*`, `MALLOC*`, `OBJC_*`,
  `XPC_*`) dan `FASTPILOT_*` ditolak.

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
  `clear-state`, `hide-keyboard`, `flow <file|->`, `check-flow`, `state save|load|list|delete`,
  `diff screenshot <name> [--update] [--ignore sel]`, `errors`,
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
dan sudah login sekali (`claude`); lokasi lain: `fastpilot config set claude /path/ke/claude`.

```bash
DEMO_PASS=rahasia123 FASTPILOT_VARS=DEMO_PASS \
  fastpilot run tests/ --junit junit.xml --html report.html
```

- Claude Code hanya mendapat tool fastpilot: tool bawaan (shell, file, web) dimatikan dan izin tool
  fastpilot disetujui otomatis, karena tidak ada yang menunggu di depan layar. Tool `report` dan `config`
  tidak diberikan.
- Test dengan hint path memory diputar ulang oleh model cepat (`claude-haiku-5-5`); bila replay gagal
  atau belum ada hint, model besar (`claude-opus-5-5`) melanjutkan dari layar terakhir (keduanya bisa
  diganti: `model_replay`, `model_explore`). `--model` (atau `model`) memaksa satu model, `--max-steps` membatasi jumlah panggilan tool per test (default 30).
- Kegagalan diulang selama `FASTPILOT_RETRY_BUDGET` (default 3) masih ada; retry yang lolos = flaky.
  Setelah `FASTPILOT_BREAKER` (default 3) kegagalan infra beruntun, sisa test dilewati sebagai infra.
- `--workers N` menjalankan N test sekaligus. Tiap test punya Chrome sendiri; device Android dan simulator
  iOS yang sudah boot dipinjamkan ke satu test pada satu waktu (dicek sehat saat dipinjam, device yang
  gagal cek dikeluarkan), retry memakai device lain bila ada, dan antrean diurutkan dari durasi tercatat
  terlama (test yang belum pernah jalan lebih dulu).
- `--record` merekam layar tiap sesi (web: screencast CDP jadi filmstrip HTML; Android: `screenrecord`;
  iOS: `simctl io recordVideo`). `--html` menghasilkan satu file HTML mandiri yang memuat laporan tiap
  run (langkah, durasi, diff, screenshot, rekaman); `--junit` untuk dashboard CI.
  Di laporan tiap run, rekaman punya penanda tiap langkah (merah di langkah yang gagal; klik untuk lompat)
  dan satu tombol per flow yang memutar bagian flow itu saja. Flow yang lolos menyertakan screenshot
  layar akhirnya.
- `--reset` membuat tiap test mulai dari keadaan bersih: sesi Android/iOS me-reset app-nya sebelum dibuka,
  sekali per app per run (per test untuk `run`, per proses untuk `mcp`): lewat `target` sesi, atau pada
  `launchApp` pertama untuk app itu
  (Android: `pm clear` untuk data dan izin runtime, layar tegak; simulator iOS: data dan semua izin privasi,
  orientasi tidak bisa diatur lewat simctl; iPhone fisik tidak didukung). Sesi web memang selalu mulai dengan
  context browser baru (tanpa cookie dan storage). Butuh `allow_app_reset` (bawaan: boleh); bila dimatikan,
  sesi mobile ditolak dengan `blocked`. `fastpilot mcp --reset` melakukan hal yang sama untuk sesi dari chat.
  Orientasi dan setelan device dikembalikan saat sesi selesai.
- `--report-github` (di GitHub Actions, pada pull request) menulis hasil suite sebagai **satu** komentar di PR:
  "running" saat mulai, lalu diganti dengan tabel hasil (lolos, gagal + kelas + catatan, flaky, karantina,
  durasi, link ke workflow run). Komentar yang sama diperbarui di run berikutnya (ditandai
  `<!-- fastpilot-report -->`), tidak menumpuk. Token hanya dibaca dari env `GITHUB_TOKEN`, tidak pernah
  disimpan, dimasukkan ke URL, atau dicetak. Gagal menulis komentar tidak menggagalkan suite.
  `--report-github=dry` mencetak isi komentar tanpa mengirim apa pun. Contoh job:

  ```yaml
  permissions:
    pull-requests: write
  steps:
    - run: fastpilot run tests --report-github
      env:
        GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
  ```
- **Selain Claude Code (opsional):** `fastpilot run` bisa dijalankan oleh API yang kompatibel dengan OpenAI
  (chat completions + tool calling). Claude Code tetap bawaan. Tool, verdict `finish`, `max_steps`, dan
  `test_timeout` sama persis.

  ```bash
  fastpilot config set provider openrouter        # openai, openrouter, groq, ollama (lokal), custom
  fastpilot config set provider_model <model id>  # wajib
  fastpilot config set-key openrouter             # key dibaca dari stdin (tersembunyi), disimpan di Keychain
  fastpilot config test-provider                  # satu request kecil untuk cek
  ```

  Key disimpan di macOS Keychain (Linux: secret service lewat `secret-tool`, kalau tidak ada: file 600 di
  `~/.fastpilot/keys/`), atau diberikan lewat env `FASTPILOT_PROVIDER_API_KEY`. Key tidak pernah ditulis ke
  config, log, laporan, atau argumen proses. `provider` dan `provider_base_url` hanya bisa diubah user (agent
  tidak bisa mengalihkan data test ke server lain). `http://` hanya untuk mesin ini (mis. Ollama); server lain
  wajib `https://`. Kembali ke Claude Code: `fastpilot config unset provider`.

  Key terikat ke server tempat ia disimpan: preset (`openai`, `openrouter`, `groq`) hanya ke servernya
  sendiri, `custom` ke `provider_base_url` saat `set-key` dijalankan. Bila server berbeda, key tidak
  dikirim dan fastpilot memberi tahu caranya (set ulang key, atau kembalikan URL). Key yang disimpan
  fastpilot versi lama (sebelum key terikat) hanya dipakai untuk server preset hosted; untuk `custom` atau
  `ollama` jalankan `fastpilot config set-key <provider>` sekali lagi. Redirect dari server provider
  tidak diikuti (isi permintaan tidak ikut ke server lain). Mengganti `provider`
  lewat `fastpilot config set provider …` menghapus `provider_base_url` lama, sehingga provider baru memakai
  presetnya. `FASTPILOT_KEYSTORE=file` memaksa penyimpanan file 600 (berguna di CI, atau dengan `HOME`
  terisolasi supaya macOS tidak meminta Keychain).

  **Privasi:** dengan provider pihak ketiga, isi tool yang dikirim ke model (snapshot layar, hasil langkah,
  URL halaman, termasuk path lokal seperti `file:///Users/<nama>/…`) ikut sampai ke provider itu. Nilai
  `${NAMA}` tetap disamarkan `***`. Pakai Ollama lokal bila data test tidak boleh keluar dari mesin ini.

  Sama lewat halaman: `fastpilot ui` › Atur › Model AI › *penggerak test* (penyedia, Base URL, model,
  API key, **Tes koneksi**). Key dikirim sekali ke penyimpan kunci dan tidak pernah ditampilkan lagi.
- Exit code ≠ 0 bila ada test di luar karantina yang gagal. Nilai `FASTPILOT_VARS` yang tertulis di
  test case dikirim ke model sebagai `${NAMA}`, tidak pernah sebagai nilainya.

## Chrome harian dan iPhone fisik

- **Chrome sehari-hari tanpa restart**: muat `extension-chrome/` (chrome://extensions → Developer mode →
  Load unpacked), minta `session { action: start, mode: attach, endpoint: extension }`, lalu klik tombol
  extension di tab yang mau dikendalikan. Relay hanya menerima koneksi yang membawa token acak per sesi;
  extension mengambilnya lewat native messaging (tidak bisa dijangkau halaman web). Detail di
  [extension-chrome/README.md](extension-chrome/README.md).
- **iPhone/iPad dan ponsel Android fisik**: lihat [docs/real-devices.md](docs/real-devices.md). Ringkasnya:
  cek kesiapan dengan `fastpilot ios doctor`, build runner untuk iPhone, lalu mulai sesi dengan
  `device: ios-device:<udid>` atau `device: android-device:<serial>`.

```bash
FASTPILOT_IOS_TEAM=<team id> FASTPILOT_IOS_BUNDLE_PREFIX=com.<nama>.fastpilot ios-runner/build.sh --device <udid>
```

## Device di mesin lain

Emulator Android dan Chrome bisa berjalan di mesin lain (mis. PC Linux dengan KVM), sementara fastpilot
dan agent AI tetap di laptop. Semua lalu lintas lewat **SSH tunnel**; port debug tidak pernah dibuka ke
jaringan.

> Port adb (5037) dan port remote debugging Chrome memberi kendali penuh atas device dan browser,
> tanpa password. Jangan pernah membukanya ke internet atau LAN (`adb -a`, `--remote-debugging-address=0.0.0.0`).
> Biarkan keduanya mendengarkan di `127.0.0.1` mesin itu dan sambungkan lewat SSH.

**Android.** Di mesin device: nyalakan emulator/HP seperti biasa (`adb devices` menampilkannya). Di laptop:

```bash
ssh -N -L 15037:127.0.0.1:5037 user@mesin-device
```

```bash
fastpilot config set adb_server 127.0.0.1:15037      # atau env FASTPILOT_ADB_SERVER
fastpilot doctor android                             # adb server 127.0.0.1:15037 ✓, device ✓
```

- Pakai port lokal selain 5037, supaya tidak tertukar dengan adb server di laptop sendiri.
- fastpilot meneruskan koneksi ke agent dan WebView lewat adb server itu sendiri, jadi cukup satu tunnel.
- Yang hanya bisa dari mesin emulator: `setLocation`, `setNetwork` (throttling) dan perintah konsol
  emulator lain (ditolak dengan pesan jelas), serta `session start_device` (menyalakan AVD).

**Chrome.** Di mesin itu, jalankan Chrome dengan profil terpisah dan port debug hanya di localhost:

```bash
google-chrome --headless=new --remote-debugging-port=9222 --user-data-dir=/tmp/fastpilot-chrome
```

Di laptop:

```bash
ssh -N -L 19222:127.0.0.1:9222 user@mesin-device
```

```bash
fastpilot config set chrome_endpoint http://127.0.0.1:19222   # atau env FASTPILOT_CHROME_ENDPOINT
fastpilot doctor web                                          # remote Chrome at http://127.0.0.1:19222 ✓
```

- Tiap sesi tetap mendapat `BrowserContext` sendiri (cookie dan storage terpisah); fastpilot tidak
  menyalakan Chrome lokal.
- Path file dibaca oleh mesin Chrome: `target` lokal (`examples/demo/login.html`), `uploadFile`, dan
  unduhan memakai sistem file mesin itu. Pakai URL `http(s)://` untuk aplikasi yang dites.
- `adb_server` dan `chrome_endpoint` hanya bisa diubah user (`fastpilot config set`), tidak oleh agent.
- Bila host-nya bukan mesin ini (bukan `127.0.0.1`/`localhost`), `doctor` dan log memberi peringatan: DevTools
  dan adb tidak punya password maupun enkripsi. `user:password@` di alamat disamarkan (`***@`) di `doctor` dan pesan error.

Menjalankan **fastpilot sendiri** di mesin lain (mis. Mac untuk iOS) lewat SSH, membuka `fastpilot ui`
dari laptop, dan menghemat memori simulator iOS dengan simslim: lihat [docs/remote-machine.md](docs/remote-machine.md).

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
- `bin/fastpilot/tests/cli.rs`, `cli_shell.rs`, `config_web.rs`, `state_web.rs`, `policy_web.rs` dan
  lainnya menguji CLI, daemon shell, pengaturan, login tersimpan, dan guard rail di Chrome sungguhan.
- Test yang butuh Chrome, device, atau simulator otomatis di-skip kalau tidak tersedia.
- CI GitHub hanya build (`cargo build --workspace --all-targets --locked`); test dijalankan lokal.

## Layout

```
crates/core      config (env → config.toml), policy, Driver trait, Selector, Snapshot (prune, ref stabil, diff), DSL + check_flow, engine, assert, SessionRegistry
crates/web       CDP client, launch/attach Chrome, WebPage (Driver) per BrowserContext
crates/android   adb server protocol, agent client, AndroidDriver (+ WebView via CDP)
crates/ios       simctl/devicectl + XCUITest runner, usbmux, Web Inspector, IosDriver
crates/memory    run evidence (.fastpilot/runs) + path memory (.fastpilot/memory)
crates/mcp       rmcp stdio server, 10 tools, docs
bin/fastpilot    entrypoint: `mcp`, shell CLI + daemon, `run`, `doctor`/`setup`, `config`, `bench`, `stability`
agent-android/   instrumentation APK (Java, UiAutomation), dibangun tanpa Gradle
ios-runner/      XCUITest runner + demo app iOS
examples/        demo web (login, widgets), demo Android
tests/           test case bahasa natural
skills/fastpilot Agent Skill (SKILL.md) untuk agent AI
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
