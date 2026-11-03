# Eksplorasi motmaina.com dengan fastpilot (2026-11-03)

Situs produksi, eksplorasi read-only: tidak ada form yang dikirim, tidak ada akun dibuat.
Alat: binary `fastpilot` (Phase 1–5, build 2026-11-03 12:16) lewat MCP stdio, Chrome headless.

## Temuan di situs

| # | Prioritas | Temuan | Bukti |
|---|---|---|---|
| 1 | Tinggi | Request Google Analytics (GA4 `G-6071942717`) dikirim ke hostname tidak valid `awq9r1rnl…==`, sehingga gagal `ERR_NAME_NOT_RESOLVED`. Data analytics kemungkinan hilang. | Muncul di setiap halaman, di fastpilot dan di browser kedua; console juga mencatat `postMessage` ke origin yang sama gagal |
| 2 | Sedang | Header `Strict-Transport-Security` dan `Content-Security-Policy` tidak ada di `/ar`. Untuk situs kesehatan jiwa, HSTS sebaiknya wajib. | `curl -I https://motmaina.com/ar` |
| 3 | Sedang | Konsultan tanpa jadwal ("لا يوجد مواعيد") tetap menampilkan tombol "حجز موعد الآن". | Beranda dan halaman konsultan |
| 4 | Rendah | Opsi urutan "الأكثر تقييماً" muncul dua kali di dropdown "التصنيف حسب". | `/ar/consultants` |
| 5 | Rendah | Profil konsultan yang tidak ada (`/ar/consultants/999999`) mengembalikan HTTP 200 dengan judul "المستشار غير متوفر" (soft 404). | `curl` + fastpilot |
| 6 | Rendah | Typo: "تم أنجاز!" seharusnya "تم إنجاز!". Juga "12 سنوات" seharusnya "12 سنة". | `/ar/consultants/46` |
| 7 | Rendah | Halaman 404 berbahasa Arab memakai judul tab Inggris "404 Page", dan pesannya generik ("هنالك خطأ ما قد حدث"), bukan "halaman tidak ditemukan". | `/ar/halaman-tidak-ada-xyz` |
| 8 | Rendah | 238 dari 282 gambar di beranda tanpa atribut `alt`. | Beranda |
| 9 | Info | Tag TikTok Pixel menolak nama event karena formatnya salah; Meta Pixel menahan event yang belum diverifikasi. | Console |

Yang berjalan baik: semua 31 link internal di beranda mengembalikan 200; ganti bahasa ke `/en` benar (`lang=en`, `dir=ltr`); memilih cabang "جدة" memuat 28 konsultan; validasi wajib isi form janji temu aktif; tombol booking menampilkan dialog unduh aplikasi.

Belum dicek: tampilan mobile (fastpilot belum punya langkah emulasi viewport, lihat bagian di bawah), alur login, dan pengiriman form.

## Temuan tentang fastpilot sendiri

| # | Temuan | Dampak |
|---|---|---|
| A | Harakat Arab dirender sebagai escape Rust, mis. `"كن شريك\u{64b}ا"`, bukan karakter aslinya. | AI yang menyalin teks dari snapshot akan membuat selector yang tidak cocok; token terbuang |
| B | Daftar `candidates` saat `not_found` berisi 5 elemen pertama di halaman, bukan yang paling mirip. Mencari "المستشارين" tidak menyarankan "الاستشاريين". | Self-healing lebih lambat; AI harus mengambil snapshot lagi |
| C | Teks rating dipecah per token: `text "("`, `text "11"`, `text ")"`. Tanggal kalender jadi 30 baris tombol. | Snapshot halaman konsultan melewati batas 120 baris |
| D | "recent errors" dipenuhi kegagalan request pihak ketiga (analytics) yang diulang di setiap respons. | Token terbuang; error aplikasi asli tertutup |
| E | Tidak ada langkah untuk emulasi viewport atau perangkat mobile di web. | Tampilan mobile situs tidak bisa diuji |
| F | Membuka sesi di situs nyata butuh 7–9 detik (Chrome + muat halaman), `openUrl` 3–4 detik. Snapshot dan diff tetap 15–40 ms. | Waktu didominasi situs, bukan fastpilot |
