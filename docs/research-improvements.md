# fastpilot — Riset Improvement (pasca Phase 1)

*Dibuat: 2026-11-03 | Metode: ECC deep-research (WebSearch, karena Firecrawl/Exa belum terpasang) + review kode `ecc:rust-reviewer` | ±55 sumber | Confidence: Medium*

Catatan: angka latensi/benchmark vendor bersifat **self-reported** (SR). Klaim yang hanya punya satu sumber ditandai **[1-src]**. Nama method CDP untuk dialog/upload/download perlu dicek ulang ke spec resmi sebelum implementasi.

> **Status implementasi (2026-11-03):** semua rekomendasi Tier 0–3 sudah dikerjakan, kecuali beberapa
> hal yang sengaja ditunda: extension relay untuk attach, OOPIF bertingkat, gambar Set-of-Mark di
> Android/iOS, dan hook privat XCUITest untuk mempercepat iOS. Untuk iOS dipakai runner XCUITest sendiri,
> bukan WDA prebuilt.

## Ringkasan eksekutif

1. Arsitektur Phase 1 sudah searah dengan state of the art. agent-browser, browser-use, Stagehand v3, dan Chrome DevTools MCP semuanya pindah ke **CDP langsung**, snapshot teks dengan ref, dan diff. Snapshot 1.5 ms dan tap→diff 83 ms juga sudah di bawah target.
2. Risiko terbesar ada di **stabilitas** di halaman nyata, bukan di arsitektur. Review menemukan lima bug yang akan menurunkan pass rate di bawah 99%:
   - error CDP sementara saat navigasi tidak di-retry;
   - navigasi yang dipicu klik tidak ditunggu;
   - JS dialog membuat sesi hang;
   - `open`/`back` bisa selesai sebelum halaman load;
   - nama elemen dipotong sebelum dicocokkan.
3. Ada celah keamanan: `${ENV}` bisa membaca env var apa pun milik proses, dan nilainya bisa muncul kembali di diff atau URL.
4. **Rekomendasi: kerjakan "Phase 1.1 hardening" (±1–2 hari, semua effort S) sebelum mulai Android.** Bug yang sama (retry, settle, dialog, cancellation) akan terbawa ke driver Android kalau tidak diperbaiki di core sekarang.
5. Target PRD G6 (≥99% dari 20 run) tidak bisa dibuktikan secara statistik. 20/20 lolos hanya membuktikan failure rate <15% (95% CI). Klaim ≥99% butuh ±300 run bersih. Ubah metriknya jadi pass^k + flake rate per test.

## Tier 0 — Hardening sekarang (Phase 1.1, effort S)

| # | Masalah (lokasi) | Perbaikan |
|---|---|---|
| 1 | Error CDP saat navigasi ("Cannot find context", "Could not find node") jadi `driver_error` fatal (`web/src/lib.rs`, `core/src/error.rs`) | Tambah varian `Transient` yang retryable |
| 2 | `wait_idle` tidak melacak navigasi dari klik `<a>`/submit, dan jendela mutasi hanya 2 frame (`page.rs`) | Lacak `Page.frameStartedNavigating`/lifecycle sebagai busy; syarat diam 100–150 ms + `document.getAnimations()` kosong |
| 3 | `alert`/`confirm`/`beforeunload` tidak ditangani → hang 30 s per step; `dom_quiet` tidak dibatasi deadline | Handle `Page.javascriptDialogOpening` dengan policy (accept/dismiss), laporkan dialog di diff, bungkus `dom_quiet` dengan `timeout` |
| 4 | `open`/`back` memakai satu counter nav; SPA `replaceState` atau bfcache membuat return terlalu cepat atau timeout 30 s | Tunggu `lifecycleEvent load` dengan `loaderId` dari `Page.navigate`; tambah `--disable-back-forward-cache` |
| 5 | Nama dipotong 80 char sebelum matching → assert pesan error panjang tidak pernah cocok (`snapshot.rs` `walk`) | Simpan nama penuh, potong hanya saat render |
| 6 | Urutan tier di `pick`: fuzzy interaktif mengalahkan exact non-interaktif | Tier menjadi loop luar, preferensi role menjadi loop dalam |
| 7 | `assertVisible: {id}` selalu gagal karena `Node.id` tidak diisi | Resolve `Id` lewat DOM di driver untuk assert juga |
| 8 | `${...}` merusak template literal JS di `evalScript`; substitusi berulang | Regex `\$\{[A-Z_][A-Z0-9_]*\}`, escape `$${`, single-pass |
| 9 | **Keamanan:** `${ENV}` membaca env var apa saja; nilai bocor lewat diff `value=`, URL, dan pesan error | Allowlist (prefix `FASTPILOT_VAR_` atau daftar di config); node yang diisi dari `${ENV}` ditandai `secure`; redaksi nilai di semua output |
| 10 | Deteksi `secure` hanya `type=password` dan tidak tembus shadow DOM | Satu pass JS: `type=password`, autocomplete `*-password`/`one-time-code`/`cc-*`, `-webkit-text-security` |
| 11 | Timeout retry bisa membatalkan aksi di antara `mousePressed` dan `mouseReleased` | Timeout hanya untuk lookup/actionability; dispatch input jadi unit yang tidak bisa dibatalkan |
| 12 | Chrome mati → call pertama tetap menunggu 30 s; entri `pending` bocor saat future di-drop (`cdp.rs`) | Flag `closed` + drop guard |
| 13 | `to_url`: `localhost:3000` dianggap scheme, `localhost` dipaksa https | Allowlist scheme; `host:port`/`localhost` → `http://` |
| 14 | Viewport tidak deterministik (headless ≠ headful) | `Emulation.setDeviceMetricsOverride` 1280×800 default |
| 15 | Chrome yang crash di-cache selamanya (`OnceCell`) | Relaunch saat koneksi tertutup |

Juga: `evalScript` dengan `replMode: true` dan timeout sendiri; filter request `EventSource`/`WebSocket` dari in-flight tracking; shadow-DOM-aware hit test (`getRootNode().elementFromPoint`).

## Tier 1 — Paritas web dengan tool terdepan (effort S–M)

1. **Ref berbasis frame + iframe/OOPIF.** Playwright memakai `f1e5`, agent-browser meng-inline iframe satu level. Pakai `Target.setAutoAttach{flatten}` dan AX tree per frame, lalu jahit di bawah node iframe. Popup/tab baru otomatis jadi bagian sesi. ([playwright a6b4b222e](https://github.com/microsoft/playwright/commit/a6b4b222e), [agent-browser snapshots](https://agent-browser.dev/snapshots))
2. **Ref harus tahan duplikat.** Kunci ref sekarang memakai counter urutan, jadi kalau baris 1 dihapus, ref "Hapus #1" diam-diam pindah ke baris 2. Simpan peta `backendNodeId → ref`, dan gagal `not_found` daripada pindah target.
3. **Elemen cursor-interactive tanpa role** (`cursor:pointer`, `onclick`, `tabindex`) di-tag `clickable` dalam satu pass JS. agent-browser kini menyalakannya default [1-src].
4. **Modal state gate** untuk dialog dan file chooser, mengikuti Playwright MCP. Step lain gagal dengan pesan jelas selama modal terbuka.
5. **Step baru: `pressKey`, pilihan `<select>`, upload (`DOM.setFileInputFiles`), download events.** Semua kecil dan sering dibutuhkan QA.
6. **Tool `find`** (grep snapshot dengan konteks ancestor) dan opsi snapshot ke file. Ini dipakai Playwright MCP untuk menekan token ([README](https://github.com/microsoft/playwright-mcp)).
7. **Diff tidak perlu full snapshot hanya karena judul berubah** (mis. `(3) Inbox`). Bandingkan lokasi yang dinormalisasi.
8. **State a11y tambahan:** `pressed`, `invalid`, `required`, `readonly`, heading `level`.
9. **Error yang bisa ditindaklanjuti.** Alasan (covered/disabled/offscreen) + kandidat + petunjuk perbaikan, sesuai prinsip desain [Chrome DevTools MCP](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/main/docs/design-principles.md).

## Tier 2 — Desain Phase 3 (path memory, assert, report)

Polanya konvergen di Firebase App Testing agent, Momentic, Midscene, Stagehand, dan Skyvern:

- **Kunci cache:** `test_id` + hash intent step + versi app/URL ternormalisasi + kelas device + signature layar. Hindari data volatil di kunci ([Stagehand caching](https://docs.stagehand.dev/v3/best-practices/caching)).
- **Tulis hanya setelah run lolos penuh.** Healing dari run gagal tidak pernah disimpan, dan assertion tidak pernah dilemahkan ([Momentic auto-heal](https://momentic.ai/docs/reliability/auto-heal.md)).
- **Saat replay:** cek postcondition setiap step. Pada mismatch pertama, buang entri dan kembalikan kendali ke AI dari step itu ([Firebase App Testing agent](https://firebase.google.com/docs/app-distribution/android/app-testing-agent)).
- **Mode cache:** read-write / read-only (CI) / off ([Midscene caching](https://midscenejs.com/caching)).
- **Simpan locator tahan lama** (role+name), bukan ref. Ini mirip `generate-locator` di [playwright-cli](https://github.com/microsoft/playwright-cli).
- **`assert` deterministik:** predikat terstruktur atas tree. Assertion berbasis LLM/vision diberi label "soft" ([arXiv 2509.19136](https://arxiv.org/html/2509.19136v2)).
- **Taksonomi kegagalan:** product / infra / agent / flaky. Tambah retry budget, circuit breaker untuk outage, dan karantina yang tetap terlihat.
- **Metrik:** pass^k + interval kepercayaan, flake rate per test ([τ-bench](https://arxiv.org/abs/2406.12045), [Marathon](https://docs.marathonlabs.io/runner/intro/overview/)).
- **Evidence saat gagal:** screenshot + console error terakhir + request gagal.

## Tier 3 — Desain Phase 2 Android (dan iOS)

1. **Server persisten di instrumentation APK** yang memegang UiAutomation. Pakai `ServerSocket` biasa, **jangan Ktor** (cold start JIT 30–120 s, [NeuralBridge](https://hackernoon.com/i-built-a-100x-faster-android-automation-tool-because-ai-agents-deserve-better), SR). Desainnya sama dengan [mobilecli/mobile-mcp](https://github.com/mobile-next/mobilecli).
2. **Hanya satu UiAutomation yang boleh aktif.** Deteksi pemegang lain (uiautomator2 `u2.jar`, Appium) saat start, lalu laporkan sebagai error terstruktur.
3. **Settle terbatas, bukan `waitForIdle`.**
   - Jendela diam a11y-event 100–150 ms + hash tree stabil, batas ±1.5 s.
   - Matikan 3 animation scale saat start.
   - Verifikasi tap lewat perubahan tree atau screenshot, retry 1×. Maestro memakai pola serupa, menurut [DeviceLab](https://devicelab.dev/blog/maestro-flakiness-code-deep-dive), satu-satunya sumber dan ditulis kompetitor.
4. **Hindari AccessibilityService sebagai jalur utama.** Di Android 15+ harus diaktifkan manual, dan consent MediaProjection reset di Android 14+.
5. **Spike awal `adb_client` 3.2:** README-nya tidak menyebut `forward`/`install`. Kalau tidak ada, implementasikan protokol `host:forward` + sync sendiri ([adb_client](https://github.com/cocool97/adb_client)).
6. **Tree a11y miskin label:** dokumentasikan `testTagsAsResourceId` (Compose) dan semantics Flutter. Fallback ke screenshot Set-of-Mark dan biarkan model client yang melakukan grounding. **Jangan embed model grounding**: OmniParser 0.6 s/frame di A100, Midscene locate 1–3 s ([Midscene](https://quanru.github.io/2026/05/26/Why-Midscene-Splits-Locate-and-Action)).
7. **iOS:**
   - WDA prebuilt untuk simulator, dengan `snapshotMaxDepth`, attribute exclusion, dan deadline per snapshot ([Appium WDA slowness](https://appium.github.io/appium-xcuitest-driver/latest/troubleshooting/wda-slowness/)).
   - Hindari idb, karena [Maestro meninggalkannya](https://maestro.dev/blog/maestro-re-building-the-ios-driver).
   - Real device butuh `sudo ios tunnel start` (go-ios).

## Yang dihindari

- Mengembalikan full tree setiap aksi. Pertahankan diff sebagai default; Chrome DevTools MCP men-default-kan `includeSnapshot=false`.
- Settle berbasis sleep tetap. 500 ms Playwright MCP hanyalah batas bawah, sedangkan pendekatan MutationObserver + in-flight kita lebih kuat.
- Menebak saat ref ambigu.
- Menggantungkan diri pada API privat Playwright (`_snapshotForAI`).
- **Attach mode lewat `--remote-debugging-port` ke profil default.** Mulai Chrome 136 ini diblokir ([Chrome blog](https://developer.chrome.com/blog/remote-debugging-port)). Urutan pilihan: profil sendiri (sekarang) → `autoConnect` Chrome M144+ → extension relay. **PRD §7.3/§16 Phase 4 perlu disesuaikan.**
- Terlalu banyak tool. Kalau tool bertambah, gate di balik flag kategori seperti Playwright MCP `--caps`.

## Koreksi untuk PRD

| Bagian | Koreksi |
|---|---|
| G6 / §13 | "≥99% dari 20 run" → pass^k + flake rate. Klaim 99% butuh ±300 run |
| §6 | Playwright MCP "incremental snapshot" sudah tidak ada di README terbaru (`--snapshot-mode full\|none`) |
| §7.3 Attach | Tambahkan jalur Chrome M144 `autoConnect`; port debugging ke profil default diblokir sejak Chrome 136 |
| §11.1 | Tambah `pressKey`, select, upload; dokumentasikan bahwa `inputText {into}` mengganti isi (Maestro: append) |
| §15 | Allowlist `${ENV}`, taint node, batasi `openUrl` ke `file://`/`chrome://` |

## Celah bukti

- Tidak ada perbandingan token antar-tool yang terkontrol. Perlu ukur sendiri tokens-per-step di aplikasi nyata.
- Tidak ada benchmark independen di device yang sama untuk `UiAutomation.takeScreenshot` vs `screencap`.
- Tidak ada data flake rate AI-driven vs scripted untuk mobile.
- Dukungan `forward`/`install` di `adb_client` belum terkonfirmasi.

## Sumber utama

- agent-browser: https://github.com/vercel-labs/agent-browser , https://agent-browser.dev/snapshots , https://agent-browser.dev/diffing
- Playwright MCP: https://github.com/microsoft/playwright-mcp , releases; playwright-cli: https://github.com/microsoft/playwright-cli
- Chrome DevTools MCP: https://github.com/ChromeDevTools/chrome-devtools-mcp (tool-reference, design-principles); autoConnect: https://developer.chrome.com/blog/chrome-devtools-mcp-debug-your-browser-session
- Chrome 136 remote debugging: https://developer.chrome.com/blog/remote-debugging-port
- browser-use → CDP: https://browser-use.com/posts/playwright-to-cdp
- Stagehand v3: https://browserbase.com/blog/stagehand-v3 , https://docs.stagehand.dev/v3/best-practices/caching
- Skyvern code caching: https://skyvern.com/docs/developers/features/code-caching
- Midscene: https://midscenejs.com/caching
- Code mode / MCP: https://www.anthropic.com/engineering/code-execution-with-mcp
- Checkly MCP vs CLI: https://www.checklyhq.com/blog/mcp-vs-cli-token-efficiency/ [1-src, n=3]
- mobile-mcp / mobilecli: https://github.com/mobile-next/mobile-mcp , https://github.com/mobile-next/mobilecli
- NeuralBridge: https://hackernoon.com/i-built-a-100x-faster-android-automation-tool-because-ai-agents-deserve-better (SR)
- Maestro: https://maestro.dev/blog/maestro-cli-v2-6-0 , https://maestro.dev/blog/maestro-re-building-the-ios-driver , https://devicelab.dev/blog/maestro-flakiness-code-deep-dive [kompetitor]
- Firebase App Testing agent: https://firebase.google.com/docs/app-distribution/android/app-testing-agent
- Momentic: https://momentic.ai/docs/reliability/auto-heal.md
- Android stability: https://developer.android.com/training/testing/instrumented-tests/stability
- Appium WDA: https://appium.github.io/appium-xcuitest-driver/latest/guides/run-prebuilt-wda
- agent-device PR #700: https://github.com/callstack/agent-device/pull/700
- adb_client: https://github.com/cocool97/adb_client
- Flakiness: https://arxiv.org/html/2509.19136v2 , https://arxiv.org/abs/2406.12045 , https://docs.marathonlabs.io/runner/intro/overview/
