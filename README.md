# fastpilot

**Tes aplikasi web, Android, dan iOS cukup dengan bahasa biasa.**
Kamu tulis apa yang mau dicek, lalu AI (Claude Code) yang menjalankannya di komputermu.
Gratis, tanpa cloud, dan tombol pembayaran diblokir sejak awal.

## Yang kamu butuhkan

- Mac (atau Linux)
- Google Chrome
- [Claude Code](https://claude.com/claude-code) yang sudah login

## Mulai dalam 3 langkah

**1. Pasang**

```bash
npm install -g fastpilot
```

**2. Coba demo**

```bash
fastpilot demo
cd fastpilot-demo
claude
```

Di Claude, ketik: *Pakai fastpilot, jalankan tests/login.md lalu report*

Kalau muncul ✓ `login-ok` lolos, fastpilot sudah jalan. 🎉

**3. Pakai di proyekmu**

```bash
cd ~/proyek-kamu
fastpilot init
```

Buka `tests/contoh.md`, ganti `target` dengan alamat aplikasimu, lalu minta Claude menjalankannya.

## Pakai lewat dashboard (paling mudah)

```bash
fastpilot ui
```

Dashboard terbuka di browser. Biarkan Terminal tetap terbuka selama dashboard dipakai.

| Menu | Untuk apa |
|---|---|
| **Chat** | Minta test dengan kalimat biasa, mis. *"Tes https://example.com, pastikan tulisan 'Example Domain' tampil"* |
| **Live** | Tonton test berjalan, dan ambil alih kalau perlu |
| **Jadwal** | Jalankan test otomatis tiap hari atau tiap minggu. Kalau gagal, kamu dikabari lewat Slack atau notifikasi Mac |
| **Device** | Cek apakah Chrome, Android, dan iOS sudah siap |
| **Pengaturan** | Pilih model AI, atur batasan, simpan secret |

Link dashboard hanya berlaku sekali selama 10 menit. Kalau kedaluwarsa, jalankan `fastpilot ui` lagi.

## Menulis test

Satu test = satu file `.md` di folder `tests/`:

```markdown
# Cari produk
id: cari-produk
platform: web
target: https://tokomu.com
- Ketik "sepatu" di kolom pencarian, lalu tekan Enter
- Pastikan daftar hasil pencarian tampil
```

Tidak perlu selector atau kode. Tulis seperti menjelaskan ke teman.
`platform` bisa `web`, `android`, atau `ios`.

## Aman sejak awal

- Tombol **bayar, beli, checkout, transfer, hapus akun** selalu diblokir.
- Tombol **hapus/delete/remove** biasa harus dikonfirmasi dulu.
- Password ditulis sebagai `${NAMA}` dan selalu tampil `***` di hasil ([cara menyimpannya](docs/panduan.md#menulis-test-sendiri)).
- AI hanya bisa memperketat aturan, tidak bisa melonggarkannya.

Mau mengetes checkout? Buka `fastpilot ui` → **Pengaturan › Batasan & izin**, lalu hapus katanya dari daftar.

## Kalau macet

Jalankan dulu:

```bash
fastpilot doctor
```

`doctor` menunjukkan apa yang belum siap dan cara memperbaikinya.

| Masalah | Coba |
|---|---|
| Chrome tidak ditemukan | `fastpilot setup web` |
| fastpilot tidak muncul di Claude | `fastpilot init` lagi |
| Android belum siap | `fastpilot setup android`, lalu `fastpilot demo --android` |
| Test berhenti dengan "Diblokir Batasan" | Itu disengaja. Ubah di dashboard › Pengaturan › Batasan & izin |

## Lebih lanjut

Semua detail ada di **[Panduan lengkap](docs/panduan.md)**:

- [Android](docs/panduan.md#android) dan [iOS](docs/panduan.md#ios-menyusul-di-v033-sekarang-dari-checkout)
- [Claude Desktop](docs/panduan.md#claude-desktop)
- [Semua pengaturan](docs/panduan.md#pengaturan)
- [Menjalankan test tanpa membuka Claude (`fastpilot run`)](docs/panduan.md#tanpa-membuka-claude-code-fastpilot-run)
- [Perintah satu per satu di terminal (shell CLI)](docs/panduan.md#shell-cli-satu-perintah-per-langkah)
- [Device di mesin lain](docs/panduan.md#device-di-mesin-lain) dan [HP fisik](docs/real-devices.md)
- [Build dari source](docs/panduan.md#build-dari-source)
- [Batasan yang diketahui](docs/panduan.md#batasan-yang-diketahui)

Lisensi: [Apache-2.0](LICENSE)
