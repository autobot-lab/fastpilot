# Merilis fastpilot

Satu versi = satu angka di `Cargo.toml` (`[workspace.package] version`) dan `packaging/npm/package.json`.
Artefak disimpan di Package Registry GitLab project ini (paket generik `fastpilot`), dan dipakai oleh
`scripts/install.sh`, paket npm dan formula Homebrew.

| Artefak | Dibuat oleh |
|---|---|
| `fastpilot-<v>-aarch64-apple-darwin.tar.gz` | Mac (Apple Silicon): `scripts/package.sh` |
| `fastpilot-<v>-x86_64-apple-darwin.tar.gz` | Mac: `rustup target add x86_64-apple-darwin && scripts/package.sh --target x86_64-apple-darwin` |
| `fastpilot-<v>-x86_64-unknown-linux-gnu.tar.gz` | pipeline GitLab, job `package-linux` (saat tag) |

Tiap arsip berisi binary, `README.md` dan `LICENSE`, plus `.sha256` di sebelahnya. Agent Android
tertanam di binary (jalankan `agent-android/build.sh` sebelum `scripts/package.sh`); runner iOS
tidak ikut (butuh Xcode, dibangun dari checkout dengan `ios-runner/build.sh`).

## Langkah

1. Naikkan versi di `Cargo.toml` dan `packaging/npm/package.json`, commit lewat MR ke `main`.
2. Di Mac, dari `main`:
   ```bash
   agent-android/build.sh
   scripts/package.sh
   scripts/package.sh --target x86_64-apple-darwin
   GITLAB_TOKEN=<token dengan scope api> scripts/publish.sh
   ```
3. Buat tag `v<versi>` di `main` dan push. Pipeline tag membangun arsip Linux (`package-linux`),
   mengunggahnya, lalu membuat GitLab Release dengan tautan ketiga arsip (`release`).
   Job `package-linux` bisa dicoba manual di pipeline MR sebelum merilis.
4. npm (sekali per versi): `cd packaging/npm && npm publish` (butuh akun npm; nama paket `fastpilot`).
5. Homebrew: `scripts/homebrew-formula.sh <versi> > Formula/fastpilot.rb` di repo tap
   (mis. `autobot-lab/homebrew-fastpilot`), lalu pengguna:
   `brew install autobot-lab/fastpilot/fastpilot`.

## Mengecek tanpa merilis

```bash
scripts/package.sh                                  # dist/…tar.gz + .sha256
mkdir -p /tmp/reg/0.1.0 && cp dist/* /tmp/reg/0.1.0/
(cd /tmp/reg && python3 -m http.server 8765 &)
FASTPILOT_DOWNLOAD_BASE=http://127.0.0.1:8765 FASTPILOT_VERSION=0.1.0 \
  FASTPILOT_INSTALL_DIR=/tmp/fp-bin sh scripts/install.sh
(cd packaging/npm && npm pack) && FASTPILOT_DOWNLOAD_BASE=http://127.0.0.1:8765 \
  npm install -g --prefix /tmp/fp-npm packaging/npm/fastpilot-0.1.0.tgz
```
