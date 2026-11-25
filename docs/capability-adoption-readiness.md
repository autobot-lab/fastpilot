# Capability: Kesiapan Adopsi fastpilot

*Dibuat: 2026-11-03 | Sumber: product-lens review (kebutuhan non-fitur) atas `docs/PRD.md` §21 dan `docs/research-improvements.md` | Status: draft, menunggu jawaban Open Questions*

## Capability

- **Nama:** Kesiapan adopsi (CI, rilis, bukti di aplikasi nyata, biaya per run)
- **Sumber:** product-lens review 2026-11-03, setelah Phase 1–5 dan 12 fitur tambahan dijadwalkan
- **Aktor utama:** QA engineer dan developer di tim lain yang belum pernah membangun fastpilot dari source
- **Hasil setelah rilis:** tim baru bisa memasang fastpilot dalam hitungan menit, setiap push ke `main` diverifikasi otomatis, klaim cepat dan anti-flaky terbukti di aplikasi nyata, dan biaya LLM per run terlihat serta bisa dibatasi
- **Sinyal sukses:**
  - Waktu dari "belum punya fastpilot" sampai test login demo lolos: ≤10 menit tanpa Rust, JDK, Android SDK, atau Xcode
  - Pipeline `main` hijau, dan merah dalam ≤15 menit setelah commit yang merusak build atau test web
  - Minimal satu test case per aplikasi nyata (Qualia front-end, Motmaina) dengan pass^k dan flake rate yang dilaporkan
  - Setiap report run mencantumkan token masuk/keluar dan biaya; run berhenti saat budget habis

## Product Intent

fastpilot menjanjikan automation full-AI yang cepat dan stabil untuk web dan mobile dalam satu skenario. Janji itu baru bisa dipercaya tim lain kalau (1) main tidak bisa rusak diam-diam, (2) pemasangan tidak butuh toolchain lengkap, (3) ada bukti di aplikasi sungguhan, bukan hanya demo, dan (4) biaya LLM per run bisa diprediksi.

## Constraints

**Kebijakan tetap (dari PRD, tidak boleh dilanggar):**
- Binary fastpilot tetap satu file tanpa runtime Node, JVM, Appium, atau Python. Pipeline CI boleh memakai tool apa pun untuk *membangun*, bukan untuk *menjalankan*.
- APK agent tetap di-embed ke binary (`include_bytes!`). Artefak rilis tidak boleh meminta user membangun APK sendiri.
- Secret hanya lewat `FASTPILOT_VARS`, disamarkan `***` di semua output. Aturan ini berlaku juga untuk log CI, artefak pipeline, report HTML, dan JUnit.
- Path memory dan evidence tetap lokal di `.fastpilot/`; CI hanya mengunggahnya sebagai artefak pipeline, tidak ke layanan pihak ketiga.
- Lisensi MIT sejak 0.7.2 (sampai 0.7.1: Apache-2.0), sesuai `Cargo.toml`.

**Invarian:**
- `main` hanya bergerak lewat fast-forward atau merge commit; tidak ada force-push (aturan yang sudah dipakai sesi implementasi).
- Kunci API LLM tidak pernah muncul di artefak, report, atau log, termasuk saat request gagal.
- Biaya yang dilaporkan dihitung dari jumlah token yang dikembalikan API, bukan estimasi, kecuali ditandai `estimated`.
- Run yang dihentikan karena budget tidak dihitung sebagai `pass` dan tidak menulis path memory.

**Batas lingkup:**
- Uji di aplikasi nyata hanya ke lingkungan staging atau lokal, tidak ke produksi. Skill Motmaina yang ada sudah menandai titik henti keamanan produksi; aturan itu ikut berlaku.
- Test case untuk aplikasi lain disimpan di repo fastpilot, tidak menulis ke repo aplikasi tersebut tanpa persetujuan pemiliknya (lihat Open Questions).

**Kebijakan rollout:**
- CI dipasang sebelum rilis pertama; rilis pertama hanya dibuat dari `main` yang hijau.
- Pengujian penuh yang ditunda saat ini (keputusan user 2026-11-03) harus sudah lolos sebelum tag rilis pertama.

## Actors and Surfaces

| Aktor | Surface |
|---|---|
| Developer fastpilot | Push ke GitHub → workflow CI; tag → workflow rilis |
| Tim QA pengguna baru | Skrip install / GitHub Releases → `fastpilot --version` → README quick start |
| AI agent (Claude Code, dll.) | MCP stdio, tidak berubah |
| CI pengguna | `fastpilot run --junit --html --budget` |
| Pemilik budget | Field biaya di `report.md`, HTML, JUnit properties |

## States and Transitions

**Pipeline per commit:** `queued → build → lint → unit → e2e-web → passed | failed`. Job Android dan iOS berjalan terjadwal (nightly), tidak memblokir merge: `scheduled → running → passed | failed | skipped(no-device)`.

**Rilis:** `tag vX.Y.Z → build-matrix → embed APK + runner iOS → checksum → publish GitHub Release`. Rilis gagal di langkah mana pun tidak mempublikasikan artefak parsial.

**Run dengan budget:** `running → passed | failed | budget_exceeded`. `budget_exceeded` diklasifikasikan `infra`, bukan `product`, dan tidak dihitung ke flake rate.

## Interface Contract

**CI (`.github/workflows/`)**
- Input: commit di branch mana pun; jadwal nightly untuk mobile.
- Output: status pipeline, artefak `target/release/fastpilot`, JUnit test Rust, evidence `.fastpilot/runs/` saat e2e gagal.
- Kegagalan: job gagal jika build, `clippy -D warnings`, `cargo test` unit, atau e2e web headless gagal.
- Retry: hanya untuk job yang gagal karena runner (`runner_system_failure`, `stuck_or_timeout_failure`), maksimal 1 kali. Kegagalan test tidak di-retry otomatis.

**Rilis**
- Output: binary macOS arm64, macOS x86_64, Linux x86_64 (APK ter-embed di semua; runner iOS hanya untuk macOS), file checksum SHA-256, skrip install satu baris yang memverifikasi checksum.
- Idempotensi: tag yang sama dijalankan ulang menimpa artefak rilis yang sama, bukan membuat rilis baru.

**Biaya per run**
- Input: `fastpilot run --budget <jumlah> [--currency usd]`, tabel harga per model dari file konfigurasi.
- Output: per step dan per run `input_tokens`, `output_tokens`, `cache_read_tokens`, `cost`; total di `report.md`, HTML, dan properti JUnit.
- Kegagalan: saat total biaya melewati budget, step berjalan selesai, lalu run berhenti dengan `budget_exceeded`.

**Benchmark pembanding**
- Input: satu skenario login dan satu skenario lintas platform yang sama untuk fastpilot, Maestro MCP, mobile-mcp, dan Playwright MCP.
- Output: tabel round-trip LLM, token, waktu total, dan pass^k per tool, plus versi setiap tool dan model.

**Bantuan menulis test case**
- `fastpilot new <id>` membuat template Markdown; `fastpilot lint tests/` menandai langkah ambigu (tanpa assertion, tanpa data uji, rujukan elemen yang tidak jelas).

## Data Implications

- **Sumber kebenaran test case:** tetap Markdown di `tests/` (PRD §12).
- **Field baru di report run:** `usage` (token per kategori), `cost`, `budget`, `model`, `stop_reason`.
- **Tabel harga:** file konfigurasi yang diisi pengguna, bukan nilai yang di-hardcode. Harga model berubah; fastpilot tidak boleh mengarang angka.
- **Artefak CI:** evidence run disimpan sebagai artefak pipeline dengan masa simpan terbatas (lihat Open Questions).

## Security and Policy

- **Batas kepercayaan:** kunci API LLM dan kredensial aplikasi hanya lewat CI/CD variable bertipe *masked* dan *protected*, diteruskan ke `FASTPILOT_VARS`.
- **Jalur penyalahgunaan:** test case atau halaman web yang berisi instruksi untuk membocorkan variabel. Redaksi `FASTPILOT_VARS` yang sudah ada harus diuji di CI dengan test khusus.
- **Rantai pasok rilis:** artefak hanya dibangun dari job CI, disertai checksum; skrip install menolak binary yang checksum-nya tidak cocok.
- **Aplikasi nyata:** akun uji khusus staging; tidak memakai akun pribadi atau data pelanggan.

## Non-Goals

- Device farm cloud (BrowserStack, Firebase Test Lab).
- Dashboard web untuk report di luar file HTML statis.
- Paket Homebrew resmi dan crates.io pada rilis pertama.
- Integrasi dua arah dengan Qualia (menarik test case, mengirim hasil). Build prompt menyatakan fastpilot terpisah dari Qualia Pilot.
- Penandatanganan dan notarisasi binary macOS, kecuali dijawab lain di Open Questions.

## Open Questions

1. **Runner CI untuk mobile.** Repo publik di GitHub mendapat runner GitHub Actions gratis: Linux (emulator Android butuh KVM diaktifkan di job) dan macOS (simulator iOS, menit lebih lambat dan antrean lebih panjang). Pilihan: job mobile di runner GitHub, runner self-hosted di Mac ini, atau job mobile hanya dijalankan manual. Saat ini CI hanya build; test dijalankan lokal.
2. **Aplikasi nyata yang boleh diuji.** Qualia front-end dan Motmaina: URL staging mana, akun uji mana, dan apakah ada skenario lintas platform yang nyata (mis. admin web Qualia ↔ app Motmaina)?
3. **Lokasi test case aplikasi nyata.** Di repo fastpilot (`tests/real/`) atau di repo aplikasi masing-masing?
4. **Penandatanganan binary macOS.** Tanpa notarisasi, Gatekeeper menandai binary hasil download. Cukup instruksi `xattr -d com.apple.quarantine`, atau perlu Apple Developer ID?
5. **Masa simpan artefak CI** untuk evidence dan screenshot, mengingat isinya bisa memuat data staging.
6. **Mata uang dan pemilik budget** untuk batas biaya per run dan per pipeline.
7. **Siapa yang menjalankan benchmark pembanding** dan model LLM apa yang dipakai, agar perbandingannya adil (versi model dipin).

## Handoff

- **Siap diimplementasikan langsung:** file LICENSE; pipeline CI untuk build, clippy, unit test, dan e2e web headless di Linux; biaya per run dan budget di `fastpilot run`; `fastpilot new` dan `fastpilot lint`.
- **Perlu keputusan produk dulu:** runner mobile (Q1), aplikasi nyata dan lokasi test case (Q2, Q3), penandatanganan rilis (Q4).
- **Perlu review arsitektur:** tidak ada; semua item memakai struktur crate yang sudah ada.
- **Lane ECC berikutnya:** `verification-loop` untuk CI dan pengujian penuh yang ditunda; `tdd-workflow` untuk biaya per run dan budget; `benchmark-methodology` untuk benchmark pembanding.
- **Urutan yang disarankan:** LICENSE → CI web → biaya per run → rilis binary → uji aplikasi nyata → benchmark pembanding → bantuan menulis test case.
