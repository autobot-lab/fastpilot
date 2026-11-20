# fastpilot di mesin lain lewat SSH, dan simulator iOS yang lebih ringan

Ada dua cara memakai mesin lain:

| Cara | fastpilot jalan di | Device/Chrome di | Kapan dipakai |
|---|---|---|---|
| A. Hanya device di mesin lain | laptop | mesin lain | Emulator Android atau Chrome di PC Linux; lihat README › [Device di mesin lain](../README.md#device-di-mesin-lain) |
| B. fastpilot di mesin lain | mesin lain | mesin lain | iOS (butuh Mac) dari laptop non-Mac, atau semua device di satu mesin kuat |

Halaman ini membahas cara B, lalu [simslim](#simulator-ios-lebih-ringan-simslim) untuk menghemat memori simulator iOS.

> **Status pengujian.** Bagian yang ditandai **(belum diuji)** belum pernah dijalankan oleh tim fastpilot
> lewat SSH sungguhan; perintahnya mengikuti cara kerja fastpilot dan dokumentasi alat terkait. Yang sudah
> diuji: `fastpilot mcp` lewat stdio, `fastpilot ui` dengan port tetap (penerusan ke port yang sama diterima,
> ke port lain ditolak "Host salah"), dan adb/Chrome di endpoint `127.0.0.1` lain (test `remote_web.rs`).
> Laporkan bila ada langkah yang tidak cocok.

## B. fastpilot di mesin lain

Server MCP fastpilot bicara lewat stdin/stdout, jadi SSH bisa langsung menjadi "kabelnya": Claude Code di
laptop menjalankan `ssh`, dan `ssh` menjalankan `fastpilot mcp` di mesin itu. Tidak ada port yang dibuka.

### 1. Siapkan mesin itu

```bash
# di mesin lain (Mac untuk iOS, Mac/Linux untuk web dan Android)
npm install -g fastpilot            # atau install.sh / build dari source
fastpilot doctor                     # semua yang mau dipakai harus ✓
command -v fastpilot                 # catat path lengkapnya
```

- Login SSH memakai **kunci**, bukan password (`ssh-copy-id user@mesin`), supaya Claude Code tidak
  tertahan di prompt password.
- SSH non-interaktif sering tidak memuat `PATH` dari `.zshrc`/`.bashrc`, jadi pakai **path lengkap**
  `fastpilot` dari langkah di atas. **(belum diuji)**
- Pengaturan (`~/.fastpilot/config.toml`), state login tersimpan, secret, screenshot, dan laporan
  semuanya ada di mesin itu, bukan di laptop.

### 2. Daftarkan ke Claude Code di laptop **(belum diuji)**

```bash
claude mcp add fastpilot -- ssh -T user@mesin /opt/homebrew/bin/fastpilot mcp
```

- `-T`: tanpa terminal, supaya stdin/stdout bersih untuk MCP.
- Secret (`${DEMO_PASS}` dsb.) dibaca fastpilot dari env **di mesin itu**. Jangan taruh nilainya di
  perintah `claude mcp add` (tersimpan di konfigurasi laptop dan terlihat di daftar proses). Buat skrip
  pembungkus yang hanya bisa dibaca pemiliknya di mesin itu, lalu daftarkan skrip itu:

  ```bash
  # di mesin lain: ~/bin/fastpilot-mcp  (chmod 700)
  #!/bin/sh
  export FASTPILOT_VARS=DEMO_PASS
  export DEMO_PASS='...'
  exec /opt/homebrew/bin/fastpilot mcp
  ```

  ```bash
  # di laptop
  claude mcp add fastpilot -- ssh -T user@mesin bin/fastpilot-mcp
  ```
- `fastpilot mcp` berhenti ketika klien terputus, dan saat itu menutup Chrome, agent, dan runner miliknya.
  Bila SSH putus, Claude Code menyalakan server lagi pada panggilan berikutnya; sesi yang sudah ada perlu
  dimulai ulang.

### 3. Halaman `fastpilot ui` dari laptop

Halaman UI hanya mendengarkan `127.0.0.1` di mesin itu dan hanya menerima `Host` `127.0.0.1:<port>` /
`localhost:<port>` miliknya sendiri. Karena itu **nomor port di laptop harus sama** dengan port di mesin
itu; tunnel ke port lokal yang berbeda ditolak dengan "Host salah".

```bash
# di mesin lain
fastpilot ui --no-open --port 7777        # cetak link http://127.0.0.1:7777/?code=...
```

```bash
# di laptop
ssh -N -L 7777:127.0.0.1:7777 user@mesin
```

Buka link yang dicetak (dengan `?code=`) di browser laptop. Kode itu sekali pakai dan berlaku 10 menit;
jalankan ulang `fastpilot ui` bila kedaluwarsa. **(SSH belum diuji; penerusan port dan kode sekali pakai sudah)**

### 4. iOS di Mac lain **(belum diuji)**

- Simulator dan runner XCUITest dinyalakan lewat `xcrun simctl` dan `xcodebuild`. Biasanya ini butuh user
  Mac itu **sudah login di desktop** (sesi GUI aktif), bukan hanya lewat SSH. Biarkan user tetap login
  (layar boleh terkunci) dan matikan sleep otomatis.
- Build runner sekali di Mac itu: `ios-runner/build.sh` dari checkout, atau pakai build rilis.
- Saat pertama, macOS bisa menampilkan dialog izin (Developer Tools, Keychain) di layar Mac itu. Dialog
  tersebut tidak terlihat dari SSH; selesaikan sekali langsung di Mac.
- iPhone fisik: colok ke Mac itu, lalu ikuti [real-devices.md](real-devices.md).

## Simulator iOS lebih ringan: simslim

[simslim](https://github.com/MobAI-App/simslim) (MIT, pihak ketiga, bukan bagian dari fastpilot) mematikan
service latar yang tidak terpakai di dalam satu simulator. Menurut README-nya, satu simulator iPhone turun dari
±4,0 GB menjadi ±0,9 GB memori (258 → 70 proses, M1 Pro 16 GB), sehingga lebih banyak simulator muat
sekaligus. **(belum diuji dengan fastpilot)**

```bash
brew install mobai-app/tap/simslim
xcrun simctl list devices booted          # atau: simslim list --booted
simslim on <udid>                         # rampingkan lalu reboot simulator itu
simslim status <udid>
simslim off <udid>                        # kembali normal
```

- Yang berubah hanya simulator yang disebut, bukan Mac-nya.
- Ramping permanen (tahan reboot) butuh runtime **iOS 18.5 atau lebih baru**. Di iOS 17.x–18.3 hanya
  `simslim on <udid> --no-reboot` yang jalan, dan hilang saat simulator di-reboot.
- **Erase All Content and Settings**, `simctl erase`, atau simulator baru mengembalikan ke kondisi normal.
  fastpilot sendiri tidak meng-erase simulator.
- Fitur yang **mati** di simulator ramping (dari README simslim), dan kategori yang perlu dipertahankan
  dengan `--except` bila aplikasi yang dites memakainya:

  | Butuh | Pertahankan |
  |---|---|
  | Push notification | `--except store` atau `--keep com.apple.apsd` |
  | StoreKit testing | `--except store` |
  | Universal links | `--except web` |
  | Pencarian Spotlight, "Recents" di pemilih file | `--except search` |
  | Pemilih Foto/Kontak/Kalender | `--except photos,pim` |
  | MDM / profil konfigurasi | `--except mdm` |

  Cek sebelum menjalankan test: `simslim doctor <udid> --requires push,universal-links`.
- Nyalakan simulator yang sudah dirampingkan sebelum test: fastpilot memakai simulator yang sedang menyala.
  Untuk memilih satu dari beberapa, beri UDID-nya di `endpoint` saat `session start`; simulator yang
  di-boot otomatis bila belum ada yang menyala diatur `FASTPILOT_IOS_DEVICE` (bawaan `iPhone 16`).
