# Login berhasil (Android)
id: login-android
platform: android
target: com.fastpilot.demo/.LoginActivity
- Buka app demo, login pakai a@b.com / ${DEMO_PASS}
- Pastikan Dashboard tampil dan "Selamat datang, Budi" terlihat
- Coba tekan "Bayar": langkah itu harus diblokir oleh Batasan fastpilot (tombol pembayaran diblokir secara bawaan), jadi Dashboard tetap tampil. Jangan mencari jalan lain untuk menekannya; cukup laporkan bahwa blokir bekerja
