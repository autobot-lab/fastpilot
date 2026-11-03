# PRD — fastpilot

**Full-AI Mobile + Web Automation MCP Server (Rust)**

| | |
|---|---|
| Status | Draft v0.1 |
| Tanggal | 2026-11-03 |
| Nama kerja | fastpilot (nama final belum ditentukan, lihat §19) |
| Owner | Yogi Is Ariyanto |
| Repo | https://gitlab.com/autobot-lab/fastpilot |

---

## 1. Ringkasan produk

fastpilot adalah MCP server berupa **satu binary Rust** yang memungkinkan AI agent (Claude Code, Cursor, Codex, dll.) mengendalikan **Android, iOS, dan Web** untuk QA testing. Tidak ada script yang dirawat manusia: test case ditulis dalam bahasa natural (Markdown), dan AI agent yang membaca lalu menjalankannya setiap run, memperbaiki diri sendiri saat UI berubah. Pembeda utama dari tool sejenis: (a) **kecepatan**, dengan meminimalkan jumlah dan ukuran round-trip LLM, bukan sekadar driver cepat; (b) **multi-sesi lintas platform dalam satu skenario**, sehingga satu test bisa menjalankan web dan mobile beriringan (mis. admin approve di web, notifikasi muncul di app Android) dan memverifikasi keduanya dalam satu round-trip; (c) **stabilitas**, dengan auto-wait, selector semantik, snapshot diff, dan path memory agar hasil run tidak flaky.

## 2. Masalah dan latar belakang

**Script automation mahal dirawat.** Setiap perubahan UI memaksa engineer memperbaiki selector, XPath, atau koordinat. Studi vendor (World Quality Report yang dikutip berbagai tool self-healing) menyebut 40–65% effort QA habis untuk maintenance; angka pastinya diperdebatkan, tetapi masalahnya nyata.

**Tool AI mobile yang ada lambat.** Survei tool yang sudah ada (§6) menunjukkan pola yang sama:

- Arsitektur 3-hop (app desktop → HTTP API → wrapper MCP Node) seperti MobAI menambah latensi per aksi dan start bridge iOS sampai ~60 detik.
- JVM cold start dan RAM besar (Maestro).
- Satu aksi per tool call, sehingga skenario 5 langkah butuh 5+ round-trip LLM.
- Process spawn per aksi (`adb shell input tap`, `uiautomator dump`) yang memakan 300–1000 ms per tap dan 1–5 detik per dump hierarchy, dibanding <10 ms jika dilakukan in-process di device.
- View hierarchy mentah yang verbose (CSV/XML) memboroskan token.

**Tidak ada tool yang menyatukan web dan mobile dalam satu sesi AI.** Tool web (Playwright MCP, chrome-devtools-mcp, agent-browser) hanya web; tool mobile (mobile-mcp, DroidRun, Maestro MCP) hanya mobile. Skenario lintas platform (dashboard web ↔ app pelanggan) harus dirangkai manual oleh manusia.

**Flaky adalah pembunuh kepercayaan.** Test yang gagal 1 dari 20 run membuat tim mengabaikan hasil. Sumber flaky pada AI-driven testing lebih banyak daripada script biasa: selain timing dan state, ada non-determinisme LLM dan perbedaan grounding elemen antar run.

## 3. Tujuan dan non-tujuan

### 3.1 Tujuan (v1)

| # | Tujuan | Ukuran |
|---|---|---|
| G1 | Full-AI execution tanpa script yang dirawat manusia | 100% test case berupa Markdown bahasa natural; tidak ada script tersimpan sebagai sumber kebenaran |
| G2 | Round-trip LLM minimal | Skenario login 5 langkah selesai dalam 1 round-trip dengan path memory hint, ≤3 tanpa hint |
| G3 | Token per round-trip kecil | Snapshot login page ≤40 baris; diff setelah aksi ≤15 baris |
| G4 | Driver cepat | Snapshot web <50 ms, Android <100 ms; tap→diff <200 ms; session start Android <2 s |
| G5 | Multi-sesi lintas platform dalam satu skenario | Satu `run_flow` dapat memegang ≥2 sesi (web+web, web+android) dan menjalankan blok `parallel:` |
| G6 | Stabil | Pass rate ≥99% pada 20 run berulang untuk skenario referensi, tanpa perubahan UI |
| G7 | Satu binary, tanpa runtime eksternal | Tidak ada Node, JVM, Appium, Python saat runtime |

### 3.2 Non-tujuan (v1)

- **Suite-level parallel / sharding** (N test case ke pool device, retry quota ala Marathon). Dicatat sebagai roadmap Phase 6 (§16, §19).
- **Multi-agent worker** (beberapa LLM worker paralel dengan budget token). Roadmap.
- **Cloud device farm** (BrowserStack, Firebase Test Lab). Lokal saja.
- **iOS real device** di v1; v1 iOS = simulator. Real device via go-ios + prebuilt WDA di Phase 5.
- **Firefox / WebKit** web driver. Chrome/Chromium saja (CDP).
- **Dashboard web/UI** untuk melihat report. Report berupa file di `.fastpilot/runs/`.
- **Visual testing / pixel diff** sebagai assertion.

## 4. Persona dan use case

### 4.1 Persona

| Persona | Kebutuhan | Interaksi dengan fastpilot |
|---|---|---|
| **AI agent** (Claude Code, Cursor, Codex) | Tool yang sedikit, output ringkas, aksi mengembalikan state | Pengguna utama; memanggil MCP tools |
| **QA engineer** | Menulis test case bahasa natural, membaca report, menandai flaky | Menulis `tests/*.md`, membaca `.fastpilot/runs/` |
| **Developer** | Menjalankan smoke test lokal sebelum push, debug kegagalan | Menjalankan agent dengan fastpilot di laptop, melihat screenshot evidence |
| **Maintainer tool** | Benchmark, menambah driver, menjaga DSL kompatibel | `cargo bench`, `fastpilot bench` |

### 4.2 Use case

1. **Login regression harian.** AI membaca `tests/login.md`, menjalankan hint dari path memory, assert dashboard tampil. Target: 1 round-trip.
2. **Eksplorasi test case baru.** Belum ada hint; AI snapshot → batch 3–5 langkah → baca diff → lanjut. Path memory disimpan setelah pass.
3. **Skenario lintas platform (fitur utama).** Admin login di web (sesi A), approve order; app pelanggan Android (sesi B) menerima notifikasi dan status order berubah. Satu `run_flow` dengan blok `parallel:` memverifikasi keduanya.
4. **Self-healing setelah UI berubah.** Tombol "Masuk" diganti "Login". Hint gagal di langkah 3, `run_flow` berhenti dini dengan snapshot; AI memilih `e3 button "Login"`, lanjut, path memory diperbarui.
5. **Test WebView di app Android.** Halaman pembayaran berbasis WebView; fastpilot menghubungkan CDP via `webview_devtools_remote_<pid>` sehingga DSL yang sama (`tapOn`, `inputText`, `evalScript`) tetap berlaku.

## 5. Prinsip desain

1. **Optimasi berurutan:** (1) lebih sedikit round-trip LLM: batching, aksi mengembalikan state, auto-wait, path memory; (2) lebih sedikit token per round-trip: snapshot ringkas + diff; (3) driver cepat: koneksi persisten, agent on-device, tanpa process spawn per aksi.
2. **Satu binary Rust.** Koneksi driver dibuka sekali per sesi dan dipakai ulang.
3. **Selector semantik, bukan struktural.** Ref dari snapshot → teks/label a11y → role+name → id → koordinat (fallback). **Tanpa XPath.**
4. **Setiap aksi mengembalikan state.** `run_flow` selalu mengembalikan diff; AI jarang perlu `snapshot`.
5. **Gagal cepat, gagal informatif.** Stop early pada step gagal, kembalikan alasan + snapshot sehingga AI memutuskan langkah berikut dalam round-trip yang sama.
6. **Tidak ada `sleep` yang diekspos.** Auto-wait berbasis kondisi (actionability, idle), bukan waktu.
7. **DSL yang sudah dikenal LLM.** Kompatibel Maestro di mana mungkin, sehingga tidak perlu system prompt panjang.
8. **Tool minimal.** 8 tool; dokumentasi DSL diambil on-demand via `docs`.

## 6. Referensi dan pelajaran dari tool yang ada

| Sumber | Diadopsi | Dihindari | Mengapa |
|---|---|---|---|
| **Playwriter** (Chrome extension + `chrome.debugger` + WS relay localhost:19988, satu tool `execute`) | Attach ke Chrome user tanpa relaunch, tanpa banner automation, sesi login tetap; batch banyak aksi dalam satu call | Runtime Node; free-form JS sebagai interface utama | Attach mode menghilangkan setup login berulang; JS bebas sulit divalidasi dan boros token |
| **MobAI** (app desktop, HTTP API port 8686, bridge on-device, wrapper MCP Node) | Bridge on-device persisten; dukungan Safari/Chrome/WebView; DSL batch executor | 3-hop; satu aksi per tool call; iOS bridge start ~60 s | Setiap hop menambah latensi dan titik gagal |
| **Maestro MCP** (`maestro mcp`: `list_devices`, `start_device`, `launch_app`, `run_flow`, `run_flow_files`, `inspect_view_hierarchy`, `query_docs`) | DSL YAML; tool cek syntax; tool docs; device mgmt; auto-wait toleran | JVM cold start/RAM; view hierarchy CSV verbose; `--shard-split` tidak expose shard id | LLM sudah "tahu" Maestro, system prompt kecil; JVM melanggar single-binary |
| **Vercel agent-browser** (CLI Rust; v0.20 full native Rust: install 710 MB→7 MB, daemon RAM 143→8 MB, cold start 1002→617 ms) | Snapshot a11y ringkas dengan ref stabil (`@e1`); klaim ~90% token lebih hemat dari Playwright MCP | Hanya web | Bukti bahwa Rust + CDP langsung tanpa Node layak dan jauh lebih ringan |
| **Playwright MCP / chrome-devtools-mcp** | Snapshot aria "ai mode" dengan ref; incremental snapshot | Snapshot penuh setiap aksi | Diff lebih hemat daripada snapshot penuh |
| **mobile-mcp** (mobile-next) | a11y-tree first, screenshot+koordinat hanya fallback; go-ios + WDA prebuilt untuk iOS | Process spawn per aksi | Tree tanpa vision = 0 image token |
| **DroidRun Portal** | APK on-device berbasis AccessibilityService dengan endpoint `a11y_tree`/`state` | Transport via content-provider ADB | Content-provider lambat; TCP via `adb forward` lebih cepat |
| **uiautomator2 / NeuralBridge** | Satu koneksi AccessibilityService per device; tap/tree in-process <10 ms vs 300–1000 ms via `adb shell` | Hop atx-agent HTTP | Dasar target latency Android |
| **Midscene.js / Stagehand** | Cache langkah sukses; AI dipanggil ulang hanya saat cache gagal (self-healing default on; klaim hemat biaya LLM hingga 90%) | Grounding screenshot-only (Midscene) | Menjadi dasar "path memory" |
| **Marathon runner** | Preventive retry berbasis probabilitas, retry di unit berbeda, device health check, long-test-first | — | Roadmap suite-level parallel |
| **Playwright actionability** | Auto-wait: attached, visible, stable, enabled, receives events; assertion auto-retry; larangan `waitForTimeout` | `networkidle` sebagai sinyal utama | Sumber flaky terbesar adalah timing |
| **Chronicle (arXiv 2609.20625), determinism ladder** | Temperature 0 + pinned model + record/replay; assertion berbasis properti | — | LLM tidak bitwise-reproducible bahkan di temp 0 |
| **AndroidWorld, Aria-UI, ScienceBoard, Screen2AX** | a11y tree Android "noisy" (Compose/custom view sering tanpa label) → fallback visual Set-of-Mark | Vision-only sebagai default | Tree lebih murah; vision hanya saat tree miskin label |
| **CDP `Target.createBrowserContext`** | Satu proses Chrome, banyak context terisolasi (cookie/storage/cache terpisah) | Spawn Chrome per sesi | Multi-sesi web murah dan terisolasi |
| **Appium preinstalled-WDA guide** | WDA prebuilt dilaunch via `devicectl` (iOS 17+) tanpa `xcodebuild`; install via go-ios | Appium server | Hilangkan startup WDA yang lama |

## 7. Arsitektur

```
AI client ──stdio/MCP──► fastpilot (Rust single binary)
                           ├─ core:    Driver trait, Selector, Snapshot, ref registry, diff,
                           │           flow engine, auto-wait, SessionRegistry
                           ├─ web:     CDP over WebSocket (persisten)
                           │           ├─ launch mode: Chrome sendiri, 1 BrowserContext per sesi
                           │           └─ attach mode: Chrome extension (chrome.debugger) + relay
                           ├─ android: adb protocol native (adb_client) + adb forward
                           │           ──TCP──► agent-android (instrumentation APK, UiAutomation)
                           │           └─ WebView: CDP via webview_devtools_remote_<pid>
                           ├─ ios:     WebDriverAgent prebuilt over HTTP (persisten), tanpa Appium
                           └─ memory:  path memory + run reports
```

### 7.1 Driver trait (dibagi semua platform)

```rust
#[async_trait]
pub trait Driver: Send + Sync {
    async fn open(&self, target: &str) -> Result<()>;        // URL | package/activity | bundleId
    async fn snapshot(&self) -> Result<Snapshot>;            // a11y tree ternormalisasi + refs
    async fn tap(&self, sel: &Selector) -> Result<()>;
    async fn type_text(&self, sel: &Selector, text: &str) -> Result<()>;
    async fn scroll(&self, dir: Direction, sel: Option<&Selector>) -> Result<()>;
    async fn swipe(&self, from: Point, to: Point) -> Result<()>;
    async fn back(&self) -> Result<()>;
    async fn screenshot(&self) -> Result<Vec<u8>>;           // JPEG
    async fn wait_idle(&self, timeout: Duration) -> Result<()>;
    async fn eval_script(&self, js: &str) -> Result<Value>;  // web/WebView saja; lainnya Unsupported
}
```

### 7.2 Urutan resolusi selector

1. `ref` dari snapshot terakhir (`e12`)
2. Teks terlihat / accessibility label (`"Login"`)
3. `role` + `name` (`{role: button, name: "Login"}`)
4. `id` (resource-id / DOM id / accessibilityIdentifier)
5. Koordinat (fallback: canvas, game, custom view)

Tidak pernah XPath. Jika lebih dari satu kandidat cocok, pilih yang terlihat dan paling atas; jika masih ambigu, step gagal dengan daftar kandidat (AI memilih ref).

### 7.3 Web (crate `web`)

- CDP langsung lewat WebSocket (`chromiumoxide` atau `tokio-tungstenite` mentah), koneksi persisten per proses Chrome.
- **Launch mode:** fastpilot menjalankan Chrome (headful/headless) sekali; setiap sesi = `Target.createBrowserContext` + satu page. Cookie, storage, cache terisolasi antar sesi.
- **Attach mode (Phase 4):** Chrome extension ala Playwriter; user klik ikon pada tab yang diizinkan; extension attach via `chrome.debugger` dan membuka WS ke relay lokal yang dijalankan fastpilot. Sesi login user tetap, tanpa banner automation.
- Snapshot dari `Accessibility.getFullAXTree` atau `DOMSnapshot.captureSnapshot` dalam satu CDP call.
- Auto-wait web: actionability (attached, visible, stable, enabled, receives events) + "DOM quiet" (tidak ada mutation selama 2 frame rAF) + in-flight request = 0 untuk request yang dimulai oleh aksi (bukan `networkidle` global yang rapuh untuk halaman dengan polling).

### 7.4 Android (crate `android` + `agent-android/`)

- adb protocol diimplementasikan di Rust (`adb_client`), tanpa spawn `adb` CLI.
- **agent-android:** instrumentation APK Kotlin memakai `UiAutomation`: snapshot dari `AccessibilityNodeInfo` (pruned), input via `injectInputEvent`, screenshot via `takeScreenshot` → JPEG, `waitForIdle`. Menjalankan TCP server di device; host connect via `adb forward`. Wire protocol: length-prefixed binary (postcard/msgpack), bukan JSON.
- APK di-embed di binary (`include_bytes!`), auto-install saat session start jika hilang/outdated (cek versi via handshake).
- Satu agent per device (AccessibilityService hanya mengizinkan satu koneksi). Dua sesi Android = dua device/emulator.
- **WebView:** deteksi `webview_devtools_remote_<pid>` via unix socket, forward, lalu drive dengan driver CDP yang sama seperti web. Snapshot menggabungkan node native dan node WebView dalam satu tree.

### 7.5 iOS (crate `ios`, Phase 5)

- WebDriverAgent prebuilt (di-cache di `~/.fastpilot/wda/`), dilaunch via `xcrun devicectl` (iOS 17+) atau `simctl` untuk simulator; tanpa `xcodebuild` per sesi; tanpa Appium.
- Sesi HTTP persisten ke WDA; snapshot dari `/source?format=json` lalu dinormalisasi; input via endpoint WDA.
- Real device: install WDA via go-ios; signing menjadi tanggung jawab user (didokumentasikan).

## 8. Multi-sesi dalam satu skenario (fitur utama)

### 8.1 Model

- `session.start` mengembalikan `session_id` (mis. `web1`, `and1`, nama bebas dari AI). Satu proses fastpilot memegang N sesi hidup dalam `SessionRegistry` (`HashMap<SessionId, Box<dyn Driver>>`), lintas platform.
- Setiap step DSL boleh membawa `session:`; default = sesi aktif terakhir atau satu-satunya sesi.
- Blok `parallel:` berisi beberapa sub-flow yang dijalankan serentak (`tokio::join!`); setiap sub-flow terikat satu sesi. Blok selesai saat semua sub-flow selesai atau salah satu gagal (fail-fast, sub-flow lain dibatalkan dengan aman, state tetap dikembalikan).
- Hasil `run_flow` memuat diff per sesi, diberi prefix id sesi, sehingga AI melihat state kedua platform dalam satu respons.

### 8.2 Contoh skenario lintas platform

```yaml
- session: { start: web1, platform: web, target: "https://admin.example.test" }
- session: { start: and1, platform: android, target: "com.example.app/.MainActivity" }

- parallel:
    - session: web1
      steps:
        - tapOn: "Login"
        - inputText: { into: "Email", text: "admin@example.test" }
        - inputText: { into: "Password", text: "${ADMIN_PASS}" }
        - tapOn: "Masuk"
        - assertVisible: "Pesanan"
    - session: and1
      steps:
        - launchApp: com.example.app
        - tapOn: "Pesanan Saya"
        - assertVisible: "Menunggu Persetujuan"

- session: web1
- tapOn: "ORD-1042"
- tapOn: "Setujui"
- assertVisible: "Disetujui"

- session: and1
- assertVisible: { text: "Pesanan ORD-1042 disetujui", timeout: 15000 }   # notifikasi/push
```

Respons `run_flow` (ringkas):

```
ok 11/11 steps 6.8s
[web1] screen "Detail Pesanan" (https://admin.example.test/orders/1042)
  ~ e9 badge "Menunggu" -> "Disetujui"
[and1] screen "Pesanan Saya" (android, com.example.app/.OrdersActivity)
  + e4 text "Pesanan ORD-1042 disetujui"
  ~ e2 chip "Menunggu Persetujuan" -> "Disetujui"
```

### 8.3 Isolasi dan batasan

| Platform | Isolasi antar sesi | Batas v1 |
|---|---|---|
| Web (launch) | 1 proses Chrome, 1 `BrowserContext` per sesi | Tidak terbatas praktis; diuji sampai 4 |
| Web (attach) | Tab yang dipilih user | 1 tab per sesi |
| Android | 1 agent per device | 1 sesi per device; multi-sesi = multi-device/emulator |
| iOS | 1 WDA per device/simulator | 1 sesi per device |

Variabel `${ENV}` diresolusi dari environment fastpilot, tidak pernah dikirim balik ke AI (§15).

## 9. Format snapshot dan diff

### 9.1 Snapshot

Teks ringkas, satu node per baris, indentasi = hierarki:

```
screen "Login" (android, com.app/.LoginActivity)
e1 textfield "Email" value=""
e2 textfield "Password" secure
e3 button "Masuk"
e4 link "Lupa password?"
```

Aturan:

- **Prune:** node tidak terlihat, ukuran nol, dekoratif, container kosong; wrapper satu anak dilipat.
- **Ref stabil:** `eN` dialokasikan dari hash `(role, name, path-ke-root)`, bukan index urutan, sehingga ref yang sama tetap valid setelah aksi selama node masih ada. Ref hanya berlaku dalam sesi.
- **Atribut yang dibawa:** role, name/label, value (dipotong 40 char), `secure`, `disabled`, `checked`, `selected`, `expanded`. Koordinat tidak dibawa kecuali diminta (`snapshot(coords: true)`).
- **Hard cap:** 120 baris default; jika melebihi, dipotong dengan baris ringkasan `... +84 nodes (scroll or snapshot(filter:) to see more)`.
- **Fallback visual:** jika tree memiliki <3 node berlabel padahal layar tidak kosong (umum pada Compose/custom view tanpa semantics), snapshot menyertakan screenshot Set-of-Mark (kotak bernomor) dan ref berbasis koordinat `c1..cN`.

### 9.2 Diff

Setelah aksi, `run_flow` mengembalikan hanya:

- baris `screen` jika layar/route berubah,
- `+` node baru, `-` node hilang, `~` node berubah (value/state),
- maksimum 40 baris; jika melebihi, kembalikan snapshot penuh baru dengan catatan `screen changed`.

## 10. MCP tools

| Tool | Input utama | Output | Catatan |
|---|---|---|---|
| `session` | `action: start\|stop\|list\|devices\|start_device`, `platform`, `target`, `id`, `mode: launch\|attach` | `session_id`, platform, hint path memory (jika `test_id` diberikan) | Satu tool untuk lifecycle sesi dan device |
| `run_flow` | `flow` (YAML inline), `session` default, `test_id`, `stop_on_fail` (default true) | Per-step result, timing, diff per sesi, step gagal + alasan + snapshot | Tool yang paling sering dipanggil |
| `check_flow` | `flow` | Daftar error syntax/semantik dengan nomor baris | Tanpa eksekusi; memvalidasi `session`/`parallel` |
| `snapshot` | `session`, `filter` (teks/role), `coords`, `full` | Snapshot ringkas | Jarang dibutuhkan |
| `screenshot` | `session`, `annotate: som` | JPEG (base64) | Untuk fallback visual dan evidence |
| `assert` | `session`, `kind: visible\|not_visible\|text_equals\|contains\|enabled`, `selector`, `timeout` | `pass\|fail`, alasan, snapshot ringkas saat gagal | Auto-retry sampai timeout |
| `report` | `run_id`, `status`, `notes` | Path `.fastpilot/runs/<ts>/report.md`; memperbarui path memory jika pass | Menutup run |
| `docs` | `topic: dsl\|selectors\|parallel\|examples\|errors` | Markdown referensi | Menjaga system prompt kecil |

Semua tool menerima `session`; `run_flow` juga menerima `sessions` implisit lewat step `session:` dan blok `parallel:`.

## 11. Flow DSL

### 11.1 Step v1

| Step | Bentuk | Platform |
|---|---|---|
| `launchApp` | `launchApp: com.example.app` | android, ios |
| `openUrl` | `openUrl: https://...` | web, webview |
| `tapOn` | `tapOn: "Login"` atau `tapOn: { id: btn_login }` / `{ ref: e3 }` / `{ role: button, name: "Login" }` / `{ point: [x, y] }` | semua |
| `inputText` | `inputText: { into: "Email", text: "a@b.com" }` | semua |
| `assertVisible` / `assertNotVisible` | `assertVisible: "Dashboard"` atau `{ text: ..., timeout: 10000 }` | semua |
| `scroll` | `scroll: down` atau `{ direction: down, within: "Daftar" }` | semua |
| `swipe` | `swipe: { from: [x1,y1], to: [x2,y2] }` | android, ios |
| `back` | `back` | semua |
| `evalScript` | `evalScript: "document.title"` | web, webview |
| `session` | `session: web1` (ganti sesi aktif) atau `session: { start: ..., platform: ..., target: ... }` | — |
| `parallel` | `parallel: [ { session: ..., steps: [...] }, ... ]` | — |

Kompatibel Maestro untuk `launchApp`, `tapOn`, `inputText`, `assertVisible`, `assertNotVisible`, `scroll`, `swipe`, `back`. `openUrl`, `evalScript`, `session`, `parallel` adalah ekstensi fastpilot. `check_flow` menolak step Maestro yang belum didukung dengan pesan jelas.

### 11.2 Aturan eksekusi

1. **Auto-wait sebelum dan sesudah setiap step.** Web: actionability + DOM quiet + request yang dipicu aksi selesai. Android/iOS: `waitForIdle` dengan batas 1 s, lalu tunggu target selector stabil.
2. **Retry lookup elemen** sampai timeout (default 5 s; `assertVisible` 10 s) dengan backoff 50→400 ms. Tidak ada retry untuk aksi yang sudah terkirim (tap/type) agar tidak double-submit.
3. **Stop early.** Step gagal → hentikan, kembalikan step index, alasan terstruktur (`not_found`, `ambiguous`, `not_actionable`, `timeout`, `unsupported`), kandidat terdekat, dan snapshot sesi terkait.
4. **Selalu kembalikan diff** untuk setiap sesi yang disentuh.
5. **Tidak ada `sleep`/`wait` berbasis waktu** di DSL publik. Jika AI butuh menunggu kondisi, gunakan `assertVisible` dengan `timeout`.
6. **Dialog sistem** (permission, crash/ANR, update prompt) dideteksi dari snapshot; default `dismiss` untuk permission yang tidak terkait, `fail` untuk crash dengan alasan `app_crashed`.

## 12. Model run full-AI dan path memory

### 12.1 Test case

```markdown
# Login berhasil
id: login-ok
platform: android
- Buka app, login pakai a@b.com / rahasia123
- Pastikan masuk ke Dashboard dan nama user tampil
```

Test case lintas platform menyebut sesi yang dibutuhkan:

```markdown
# Approve order tampil di app pelanggan
id: approve-order-x-platform
sessions: web (admin), android (customer)
- Admin login di web, buka pesanan ORD-1042, setujui
- App pelanggan menampilkan notifikasi dan status "Disetujui"
```

### 12.2 Alur run

1. AI membaca test case, memanggil `session.start` dengan `test_id`; respons menyertakan **hint** dari path memory jika ada.
2. Dengan hint: AI mengirim hint sebagai satu `run_flow` (1 round-trip). Pass → `report`.
3. Tanpa hint atau hint gagal: AI membaca snapshot/diff yang dikembalikan, menyusun batch berikutnya, ulangi sampai assert terpenuhi.
4. `report` menyimpan evidence dan, jika pass, menulis path memory.

### 12.3 Path memory

- Lokasi: `.fastpilot/memory/<test-id>.json`: urutan step yang lolos, selector yang dipakai (teks/role, bukan ref), `screen` yang dilewati, timing, hash versi app/URL.
- **Hint, bukan script.** Tidak dieksekusi otomatis oleh fastpilot; AI memutuskan memakainya. Jika gagal, AI eksplorasi; memory diperbarui setelah pass berikutnya. Memory lama dipertahankan dengan `failed_at` untuk statistik.
- Mirip cache Stagehand/Midscene, tetapi sumber kebenaran tetap Markdown.

### 12.4 Evidence

`.fastpilot/runs/<timestamp>-<test-id>/`: `steps.jsonl` (step, sesi, durasi, hasil), `assertions.json`, screenshot JPEG pada step kunci dan setiap kegagalan, `snapshot-before/after` pada kegagalan, `report.md`.

### 12.5 Routing model (sisi client, dokumentasi saja)

- Eksekusi rutin dengan hint → model cepat/murah.
- Eksplorasi / diagnosis kegagalan → model besar.
- Rekomendasi: temperature 0, model version dipin, agar replay hint deterministik.

## 13. Strategi anti-flaky dan akurasi

| Sumber flaky | Mitigasi di fastpilot |
|---|---|
| Elemen belum render / animasi | Auto-wait actionability (visible, stable 2 frame, enabled, receives events); Android `waitForIdle` |
| Request async belum selesai | Tunggu request yang dipicu aksi (bukan `networkidle` global); `assertVisible` auto-retry dengan timeout |
| `sleep` berbasis waktu | Tidak ada di DSL |
| Selector rapuh (XPath, index) | Tanpa XPath; ref stabil berbasis hash role+name+path; urutan resolusi semantik |
| Elemen ambigu | Step gagal `ambiguous` dengan kandidat; tidak menebak |
| Tap ganda / double-submit | Retry hanya pada lookup, tidak pada aksi yang sudah terkirim |
| Overlay/dialog sistem | Deteksi dari snapshot; dismiss permission yang tidak relevan; `app_crashed` eksplisit |
| State sesi bocor antar test | 1 `BrowserContext` per sesi web; `session.stop` membersihkan; Android `pm clear` opsional saat start |
| Non-determinisme LLM | Path memory hint (1 round-trip); temperature 0 + model pinned (rekomendasi client); assertion berbasis properti (`contains`, `visible`), bukan string exact |
| Tree a11y miskin label (Compose/custom view) | Fallback Set-of-Mark screenshot + ref koordinat; laporan `low_label_coverage` agar developer menambah semantics |
| Device/koneksi putus | Handshake versi agent; reconnect sekali dengan `adb forward` ulang; jika gagal, step `device_lost` |
| Flaky terselubung | Benchmark stabilitas: 20 run berulang skenario referensi di CI; pass rate <99% = regresi |

Akurasi grounding diukur dengan suite referensi (login, form, list+scroll, dialog, WebView) pada app demo milik proyek; target: resolusi selector teks/role benar ≥98% tanpa fallback koordinat.

## 14. Target performa

| Metrik | Target | Cara ukur |
|---|---|---|
| Snapshot web | <50 ms | `fastpilot bench web` |
| Snapshot Android | <100 ms | `fastpilot bench android` |
| Tap → diff | <200 ms | bench |
| Session start web (launch) | <1 s (Chrome sudah jalan) | bench |
| Session start Android (agent terpasang) | <2 s | bench |
| Login 5 langkah dengan hint | 1 round-trip LLM | contoh e2e |
| Dua sesi paralel (web+web, web+android) | Latensi `run_flow` ≤ +20% dibanding sesi tunggal terlama | bench `parallel` |
| Ukuran binary | <20 MB (termasuk APK embedded) | CI |
| RAM proses | <50 MB idle, <150 MB dengan 2 sesi | CI |

`cargo bench` untuk core (parser, diff, prune) dan subcommand `fastpilot bench` yang mencetak angka per platform; dijalankan sejak Phase 1.

## 15. Keamanan dan privasi

- **Attach mode** hanya menyentuh tab yang user pilih (activeTab + debugger per tab); relay hanya di localhost.
- **Secret:** field `secure` tidak pernah membawa value di snapshot/diff/log; variabel `${ENV}` diresolusi di fastpilot dan tidak dikembalikan ke AI; evidence screenshot pada field secure di-mask (overlay hitam dari bounding box).
- **Evidence lokal saja**; fastpilot tidak mengirim data ke layanan eksternal. Pengiriman ke LLM sepenuhnya di sisi client.
- `evalScript` dibatasi ke sesi web/WebView milik run; tidak ada akses filesystem dari DSL.
- APK agent hanya aktif saat sesi, dihentikan saat `session.stop`.

## 16. Fase dan milestone

| Fase | Lingkup | Definition of done |
|---|---|---|
| **1. core + mcp + web (launch)** | Workspace; Driver trait; Selector; Snapshot renderer; DSL parser (`openUrl`, `tapOn`, `inputText`, `assertVisible`, `scroll`, `back`, `evalScript`, `session`, `parallel`) + `check_flow`; CDP driver launch; tools `session`, `run_flow`, `check_flow`, `snapshot`, `screenshot`, `docs`; **multi-sesi web+web** via BrowserContext; contoh login demo HTML; README config MCP; bench | Login demo lolos dalam 1 `run_flow`; 2 sesi web paralel lolos; bench snapshot <50 ms |
| **2. android** | agent-android APK; `adb_client`; embedded install; DSL sama tanpa perubahan; **multi-sesi web+android** | Login demo Android lolos; skenario lintas platform §8.2 lolos; session start <2 s |
| **3. diff + stop-early + assert + report + path memory** | Diff mode; alasan gagal terstruktur; tool `assert`, `report`; `.fastpilot/memory` dan `.fastpilot/runs` | Login 5 langkah dengan hint = 1 round-trip; 20 run berulang ≥99% pass |
| **4. web attach + WebView** | Chrome extension + relay; WebView CDP bridge Android | Attach ke Chrome user tanpa login ulang; halaman WebView dapat di-`tapOn`/`evalScript` |
| **5. iOS** | WDA prebuilt cache; driver WDA; simulator dulu, real device via go-ios | Login demo iOS simulator lolos dengan DSL yang sama |
| **6. (roadmap)** | Suite-level parallel: device pool, sharding, retry quota ala Marathon; multi-agent worker | — |

## 17. Metrik sukses

| Metrik | Baseline (tool lain) | Target fastpilot |
|---|---|---|
| Round-trip LLM untuk login 5 langkah | 5–6 (satu aksi per call) | 1 dengan hint, ≤3 tanpa |
| Token snapshot login page | 1–5 k (hierarchy mentah) | ≤600 |
| Latensi tap (Android) | 300–1000 ms via adb shell | <200 ms termasuk diff |
| Session start Android | 10–60 s (bridge/JVM) | <2 s |
| Pass rate 20 run berulang | tidak diukur | ≥99% |
| Skenario lintas platform dalam satu run | tidak didukung | didukung sejak Phase 2 |
| Runtime eksternal | Node/JVM/Appium | tidak ada |

## 18. Risiko dan mitigasi

| Risiko | Dampak | Mitigasi |
|---|---|---|
| a11y tree Android miskin label (Compose, Flutter, custom view) | Selector teks gagal | Fallback Set-of-Mark + ref koordinat; laporan `low_label_coverage`; dokumentasi best practice semantics untuk developer app |
| Bug `chrome.debugger` (semua page `about:blank`) di attach mode | Sesi attach gagal | Deteksi dan beri pesan "restart Chrome"; launch mode sebagai fallback |
| Signing WDA untuk real device iOS | Setup rumit | v1 simulator; dokumentasi langkah go-ios; cache WDA tersigned per tim |
| LLM non-deterministik | Hasil beda antar run | Path memory; rekomendasi temp 0 + pinned model; assertion properti |
| Rate limit LLM saat banyak sesi | Run lambat | Batching mengurangi call; roadmap worker budget |
| Device/emulator putus | Run gagal | Reconnect sekali; alasan `device_lost`; evidence tersimpan |
| Diff menyesatkan saat layar berubah total | AI salah paham state | Jika diff >40 baris atau `screen` berubah, kirim snapshot penuh |
| Perubahan CDP/Android API | Driver rusak | Pin versi Chrome for Testing untuk bench; CI matrix Android API 30–35 |
| Nama "fastpilot" bentrok | Rebranding | Nama kerja; keputusan di §19 |

## 19. Pertanyaan terbuka

1. Nama final produk dan lisensi (usulan: Apache-2.0, seperti agent-browser dan mobile-mcp).
2. Apakah `parallel:` perlu `wait_for: any|all` untuk skenario race (mis. salah satu sesi yang pertama melihat notifikasi)?
3. Format path memory: JSON terstruktur saja, atau juga YAML flow siap pakai agar bisa langsung dikirim sebagai `run_flow`?
4. Batas jumlah sesi per proses dan kebijakan jika device pool habis.
5. Kapan suite-level parallel (Phase 6) dimulai, dan apakah memakai konsep Marathon (preventive retry, device health) atau cukup sharding sederhana.
6. Apakah iOS real device masuk v1.x atau v2.

## 20. Lampiran

### A. Contoh `tests/login.md`

```markdown
# Login berhasil
id: login-ok
platform: web
target: http://localhost:8080/demo/login.html
- Buka halaman login, isi a@b.com / rahasia123, klik Masuk
- Pastikan halaman Dashboard tampil dan nama "Budi" terlihat
```

### B. Contoh respons `run_flow` saat gagal (stop early)

```
fail step 3/5 tapOn "Masuk": not_found (timeout 5000ms)
candidates: e3 button "Login", e5 button "Daftar"
[web1] screen "Login" (http://localhost:8080/demo/login.html)
e1 textfield "Email" value="a@b.com"
e2 textfield "Password" secure
e3 button "Login"
e4 link "Lupa password?"
e5 button "Daftar"
```

### C. Konfigurasi MCP

Claude Desktop / Claude Code (`claude_desktop_config.json` atau `.mcp.json`):

```json
{
  "mcpServers": {
    "fastpilot": {
      "command": "/usr/local/bin/fastpilot",
      "args": ["mcp"],
      "env": { "ADMIN_PASS": "..." }
    }
  }
}
```

### D. Layout workspace

```
fastpilot/
├─ Cargo.toml                 # workspace
├─ crates/
│  ├─ core/                   # Driver trait, Selector, Snapshot, diff, DSL parser/engine, auto-wait, SessionRegistry
│  ├─ mcp/                    # rmcp server, tool definitions
│  ├─ web/                    # CDP driver (launch + attach)
│  ├─ android/                # adb client, agent protocol, WebView CDP bridge
│  ├─ ios/                    # WDA client (phase 5)
│  └─ memory/                 # path memory + run reports
├─ agent-android/             # Kotlin instrumentation APK
├─ extension-chrome/          # attach mode (chrome.debugger relay)
├─ tests/                     # test case bahasa natural
├─ docs/                      # PRD, DSL reference
└─ bin/fastpilot/             # single binary entrypoint
```

### E. Sumber

- Playwriter: https://github.com/remorses/playwriter
- MobAI: https://mobai.run , https://lobehub.com/mcp/mobai-app-mobai-mcp
- Maestro MCP: https://docs.maestro.dev/get-started/maestro-mcp ; sharding: https://docs.maestro.dev/maestro-flows/flow-control-and-logic/specify-and-start-devices , https://maestro.dev/insights/best-practices-parallel-test-execution-cli
- Vercel agent-browser: https://agent-browser.dev , https://agent-browser.dev/native-mode.md
- Playwright MCP vs chrome-devtools-mcp: https://www.vibebrowser.app/blog/chrome-devtools-mcp-vs-playwright-mcp
- mobile-mcp: https://github.com/mobile-next/mobile-mcp
- DroidRun Portal: https://docs.droidrun.ai/v3/concepts/portal-app
- uiautomator2 internals: https://deepwiki.com/openatx/uiautomator2/7-architecture-and-internals ; NeuralBridge latency claim: https://hackernoon.com/i-built-a-100x-faster-android-automation-tool-because-ai-agents-deserve-better
- Midscene.js: https://midscenejs.com/introduction ; Stagehand: https://docs.stagehand.dev/v3/references/stagehand
- Marathon: https://docs.marathonlabs.io/runner/intro/overview/ , https://proandroiddev.com/marathon-chapter-1-97f295054cc4
- Playwright actionability: https://playwright.dev/dotnet/docs/actionability ; flaky guide: https://endform.dev/blog/playwright-flaky-tests
- Chronicle (cut-point replay): https://arxiv.org/pdf/2609.20625 ; determinism ladder: https://www.preprints.org/manuscript/202610.0961
- AndroidWorld: https://arxiv.org/pdf/2405.14573 ; Aria-UI: https://arxiv.org/html/2412.16256v1 ; ScienceBoard: https://arxiv.org/pdf/2505.19897 ; Screen2AX: https://research.macpaw.com/publications/screen2axvisionbasedapproachautomatic
- CDP browser contexts: https://pydoll.tech/docs/guides/browser-contexts/
- Preinstalled WDA: https://appium.github.io/appium-xcuitest-driver/latest/guides/run-preinstalled-wda
- Tool AI testing 2026 (self-healing, QA Wolf, Momentic, Autify): https://qa.tech/blog/the-13-best-ai-testing-tools-in-2026.md

## 21. Catatan implementasi (2026-11-03)

Status: Phase 1–5 terimplementasi. Phase 6 tetap roadmap. Penyimpangan dari draf ini dan alasannya
(detail di [research-improvements.md](research-improvements.md)):

| Bagian | Draf | Implementasi | Alasan |
|---|---|---|---|
| §7.4 agent | Kotlin, wire postcard/msgpack | Java (tanpa Gradle, dibangun dengan SDK build-tools), frame 4-byte + JSON (`org.json` bawaan) | build offline yang reproducible tanpa dependency; JSON cukup cepat (snapshot Android p50 9 ms) |
| §7.4 adb | crate `adb_client` | klien protokol adb server sendiri (smart socket + sync) | dukungan forward/install di `adb_client` tidak terkonfirmasi; ±200 baris |
| §7.5 iOS | WDA prebuilt via Appium | runner XCUITest milik fastpilot (`ios-runner/`), protokol sama dengan Android | tanpa download/Appium; pendekatan yang sama dengan Maestro |
| §7.3 attach | extension + relay | endpoint `--remote-debugging-port` atau Chrome M144+ `autoConnect` (`DevToolsActivePort`) | sejak Chrome 136 port debugging di profil default diblokir; extension relay ditunda |
| §8.3 / §15 `${ENV}` | semua env | hanya nama di `FASTPILOT_VARS`; nilai disamarkan `***` di semua output | mencegah eksfiltrasi secret lewat flow/prompt injection |
| §9.2 diff | di Phase 3 | sejak Phase 1; full snapshot hanya jika URL/lokasi berubah | `run_flow` perlu state per sesi |
| §11.1 DSL | — | tambahan `assertNotVisible`, `swipe`, `pressKey`, `selectOption`, `uploadFile`, `launchApp`, `session.reset` | kebutuhan QA umum |
| G6 | ≥99% dari 20 run | metrik pass^k + flake rate | 20/20 hanya membuktikan failure rate <15% (95% CI); klaim 99% butuh ±300 run |

### 21.1 Iterasi lanjutan (2026-11-03): 12 gap dari PRD dan riset

| # | Fitur | Implementasi | Verifikasi |
|---|---|---|---|
| 1 | Dialog sistem mobile (§11.2 aturan 6) | Android: agent melaporkan paket pemilik tiap window dan kotak IME; host mengenali prompt izin (`permissioncontroller`, id `permission_*_button`), dialog crash (`android:id/aerr_close`) dan ANR (`aerr_wait`) dari resource id, jadi tidak bergantung bahasa. Crash tanpa dialog dikenali dari `logcat -b crash` setelah tiap input. iOS: runner menyertakan alert SpringBoard (`system: true`) dan `state` aplikasi; `notRunning` = crash. Prompt izin yang tidak ditarget step dijawab sesuai `FASTPILOT_PERMISSIONS` (`deny` default / `allow` / `keep`); keyboard yang menutupi target disembunyikan; error terstruktur `app_crashed` / `app_not_responding`. Injeksi input sinkron di agent dibatasi 3 s supaya ANR bisa terlihat. iOS tidak punya ANR (watchdog membunuh app → terlapor sebagai crash). | e2e emulator API 35 (keyboard, izin, crash, ANR) dan simulator iOS 18.1 (alert lokasi, crash) |
| 2 | Langkah Maestro sisa | `longPressOn`, `doubleTapOn`, `scrollUntilVisible` (web: dicek terhadap viewport karena snapshot web memuat node di luar layar), `hideKeyboard`, `eraseText`, `openLink`, `stopApp`/`killApp`, `clearState`, `clearKeychain`, `copyTextFrom`/`pasteText`, `extendedWaitUntil`, `waitForAnimationToEnd`, `takeScreenshot`, `inputRandom*`, `runFlow` (file atau `commands`, `when`, `env`; header config Maestro dilewati; batas kedalaman 8), `repeat` (`times`/`while`), `retry` (`maxRetries`). Kondisi: `visible`, `notVisible`, `platform`; `true:` (JavaScript) ditolak `check_flow` dengan pesan jelas. Variabel lokal flow (`env`) lolos `check_flow` tanpa `FASTPILOT_VARS`. Blok dihitung satu langkah di laporan; kegagalan di dalam blok menyebut iterasi/percobaan dan langkahnya. | e2e Chrome (27 langkah), emulator (20 langkah), simulator (18 langkah); unit test parser dan engine |
| 3 | Pengukuran token | `fastpilot bench tokens [web\|android\|ios]`: token per snapshot, per `run_flow` 1 langkah (diff) dan per `run_flow` login penuh, dibandingkan screenshot (w·h/750). Pakai `POST /v1/messages/count_tokens` (crate baru `fastpilot-llm`, HTTP langsung karena Rust tidak punya SDK resmi) bila `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN` di-set; selain itu aproksimasi yang dinyatakan di output (run huruf 1 per 5, run digit 1 per 3, karakter lain 1, ±20%). Hasil aproksimasi: web 111 / 92 / 236, Android 86 / 60 / 379 token vs screenshot 1366 / 1229. | test CLI di Chrome; Android dijalankan manual di emulator. **Belum diverifikasi dengan `count_tokens` karena tidak ada API key di mesin ini.** |
| 4 | Taksonomi kegagalan dan statistik | Kategori error kini `product` / `infra` / `agent`; kelas `flaky` diputuskan lintas run (lolos saat retry, atau riwayat yang berganti hasil ≥ 2 kali dalam ≥ 5 run dengan pass rate > 50%). Dari riwayat 20 run per test: pass rate dengan 95% Wilson CI, pass^3, dan flake rate (= flip antar run berturut-turut). Daftar karantina `.fastpilot/quarantine.json`: otomatis bila 10 run terakhir punya ≥ 2 gagal dan ≥ 2 flip (lepas sendiri setelah 10 run bersih), manual lewat `report { quarantine: true/false }`; tampil di hint, `report.md`, dan respons `report`. `RetryBudget` (`FASTPILOT_RETRY_BUDGET`, default 3) dan `Breaker` infra (`FASTPILOT_BREAKER`, default 3 kegagalan infra beruntun) dipakai suite runner (#6, #8). Bug ikutan diperbaiki: dua run dalam detik yang sama memakai direktori yang sama. | unit test triage + memory; e2e MCP (kelas, history, karantina) |
