# Merilis fastpilot

Satu versi = satu angka di `Cargo.toml` (`[workspace.package] version`) dan `packaging/npm/package.json`.
Artefak dilampirkan ke **GitHub Release** `v<versi>` di `autobot-lab/fastpilot`, dan dipakai oleh
`scripts/install.sh`, paket npm dan formula Homebrew (`https://github.com/autobot-lab/fastpilot/releases/download/v<versi>/<file>`).

| Artefak | Dibuat oleh |
|---|---|
| `fastpilot-<v>-aarch64-apple-darwin.tar.gz` | GitHub Actions `release` (runner macOS) |
| `fastpilot-<v>-x86_64-apple-darwin.tar.gz` | GitHub Actions `release` (runner macOS, cross-compile) |
| `fastpilot-<v>-x86_64-unknown-linux-gnu.tar.gz` | GitHub Actions `release` (runner Linux) |

Tiap arsip berisi binary, `README.md` dan `LICENSE`, plus `.sha256` di sebelahnya. Agent Android
tertanam di binary (workflow menjalankan `agent-android/build.sh` dulu); runner iOS tidak ikut
(butuh Xcode, dibangun dari checkout dengan `ios-runner/build.sh`).

## Cek sebelum rilis

Satu perintah lokal di Mac (bukan CI) menjalankan semua cek, sama untuk tiap milestone dan rilis:

```bash
scripts/release-check.sh            # semua cek
scripts/release-check.sh --strict   # cek yang terlewati (device tidak ada) juga = NOT READY
scripts/release-check.sh --skip ios # tandai cek sebagai terlewati: code, unit, web, android, ios
```

Urutannya: clippy (`-D warnings`), unit test, suite e2e web (Chrome), Android (hanya di
`emulator-5556`, atau `RELEASE_CHECK_ANDROID_SERIAL`; `emulator-5554` tidak pernah dipakai), lalu
iOS di simulator yang sedang menyala. Hasilnya satu ringkasan: tiap cek lolos / gagal / terlewati
beserta alasannya, lalu `READY TO RELEASE` (exit 0) atau `NOT READY` (exit 1). Device yang tidak
ada tertulis `skipped: no Android emulator`, bukan lolos. Code checks, unit dan web wajib jalan.
Log, `results.tsv`, `release-check-report.html` dan perubahan dibanding run sebelumnya ada di
`target/release-check/<waktu>/`. Test memakai HOME sementara dan `FASTPILOT_KEYSTORE=file`
(Keychain dan `~/.fastpilot` tidak disentuh); `.fastpilot/` di repo disisihkan lalu dikembalikan.

## Langkah

0. `scripts/release-check.sh` di `main` menyatakan `READY TO RELEASE`.
1. Naikkan versi di `Cargo.toml` dan `packaging/npm/package.json`, merge lewat PR ke `main`.
2. Buat tag `v<versi>` di `main` dan push (`git tag v0.1.0 && git push origin v0.1.0`). Workflow
   `release` membangun ketiga arsip dan membuat GitHub Release dengan arsip dan checksum-nya
   (catatan rilis dari `docs/release-notes.md`). Tidak ada token yang perlu disimpan.
   Uji coba tanpa merilis: jalankan workflow `release` secara manual (Actions → release → Run
   workflow, pilih **branch**); arsipnya hanya jadi artifact workflow, dan `release` serta
   `publish-npm` dilewati (keduanya hanya jalan untuk tag yang di-push, juga kalau run manual memilih tag).
3. npm: job `publish-npm` di workflow `release` jalan otomatis setelah GitHub Release jadi. Job ini
   memakai **npm Trusted Publishing** (identitas OIDC sesaat dari GitHub Actions, tanpa token yang
   disimpan, dengan provenance). Job berhenti tanpa publish kalau tag ≠ versi di
   `packaging/npm/package.json`, dan selesai tanpa apa-apa kalau versi itu sudah ada di npm.
   - **Sekali saja, oleh pemilik repo dan paket** (urutannya penting):
     1. GitHub › autobot-lab/fastpilot › Settings › Environments › New environment `npm`, isi
        **Required reviewers** dengan pemilik (dan centang "Prevent self-review" kalau ada lebih dari satu
        pemilik). Job `publish-npm` lalu menunggu klik **Approve** di halaman run; klik itu menggantikan
        passkey, jadi akses push saja tidak cukup untuk publish.
     2. GitHub › Settings › Rules › Rulesets: tag `v*` hanya boleh dibuat oleh admin.
     3. npmjs.com › fastpilot › Settings › Trusted Publisher › GitHub Actions: organization
        `autobot-lab`, repository `fastpilot`, workflow `release.yml`, environment `npm`.
     4. Setelah satu publish lewat workflow berhasil: Settings › Publishing access › "Require two-factor
        authentication and disallow tokens".
   - **Belum disetel (atau job gagal):** pesan error job menyebut apa yang perlu disetel. Publish
     manual seperti dulu: `cd packaging/npm && npm publish`, lalu setujui dengan passkey (2FA tidak
     dilewati).
   - Cek setelahnya: `npm view fastpilot version dist-tags` dan `npx fastpilot@<versi> --version`.
   - **MCP Registry:** job `publish-mcp` jalan setelah `publish-npm` dan mendaftarkan `server.json`
     sebagai `io.github.autobot-lab/fastpilot` (login GitHub OIDC, `mcp-publisher` 1.8.11 yang dikunci
     dengan SHA-256-nya). Versi di `server.json` (dua tempat), `packaging/npm/package.json` dan tag harus
     sama. Cek: `https://registry.modelcontextprotocol.io/v0/servers?search=fastpilot`. Kalau registry
     menolak namespace organisasi, pesan aslinya ada di log job; npm dan GitHub Release tidak terpengaruh.
4. Homebrew: `scripts/homebrew-formula.sh <versi> > Formula/fastpilot.rb` di repo tap
   (mis. `autobot-lab/homebrew-fastpilot`), lalu pengguna:
   `brew install autobot-lab/fastpilot/fastpilot`.

Darurat tanpa CI: di Mac, `agent-android/build.sh && scripts/package.sh && scripts/package.sh
--target x86_64-apple-darwin`, lalu `scripts/publish.sh` (memakai `gh` yang sudah login).

## Mengecek tanpa merilis

```bash
scripts/package.sh                                  # dist/…tar.gz + .sha256
mkdir -p /tmp/reg/v0.1.0 && cp dist/* /tmp/reg/v0.1.0/
(cd /tmp/reg && python3 -m http.server 8765 &)
FASTPILOT_DOWNLOAD_BASE=http://127.0.0.1:8765 FASTPILOT_VERSION=0.1.0 \
  FASTPILOT_INSTALL_DIR=/tmp/fp-bin sh scripts/install.sh
(cd packaging/npm && npm pack) && FASTPILOT_DOWNLOAD_BASE=http://127.0.0.1:8765 \
  npm install -g --prefix /tmp/fp-npm packaging/npm/fastpilot-0.1.0.tgz
```
