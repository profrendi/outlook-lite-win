# Outlook Lite

Outlook web (outlook.office.com) di system tray Windows, ditulis dengan Rust.
Portable, **tanpa hak admin**, memakai WebView2 bawaan Windows dan login web biasa
(tidak butuh izin Microsoft Graph dari admin kantor).

## Unduh dan jalankan

1. Buka halaman **Releases** repo ini, unduh `outlook-lite.exe`.
2. Taruh di folder pengguna, misalnya `%LOCALAPPDATA%\OutlookLite\app\`.
3. Jalankan. Login sekali; login tersimpan.

Butuh WebView2 Runtime (sudah ada di Windows 11 dan di PC yang memakai Edge).

## Mode notifikasi

| | Mati | Hemat | Siaga (bawaan) |
|---|---|---|---|
| Cara kerja | Outlook hanya hidup saat jendela terbuka | Outlook tersembunyi dibuka tiap N menit, dibaca, lalu dihancurkan | Outlook tersembunyi selalu hidup |
| Notifikasi email | Tidak ada | Tertunda sampai N menit | Beberapa detik |
| Pengingat rapat | Tidak ada | Tidak | Ya |
| RAM saat jendela tertutup | ~0 (hanya tray) | ~0, lonjakan singkat tiap cek | ~300–400 MB |
| Tombol X | Hancurkan jendela | Hancurkan jendela | Sembunyikan jendela |

**Hanya jam kerja** (bawaan 08:00–17:00, Senin–Jumat): di luar jam itu aplikasi otomatis
berperilaku seperti mode Mati.

Notifikasi berasal dari dua sumber:
- **Notifikasi bawaan Outlook web** (email baru, pengingat rapat) diteruskan menjadi toast
  Windows. Pastikan di Outlook web: *Pengaturan > Umum > Pemberitahuan* → pemberitahuan
  desktop aktif.
- **Jumlah belum dibaca di Inbox**, sebagai cadangan dan untuk lencana merah di ikon tray.

Gunakan **Tes notifikasi** setelah instal. Jika toast tidak muncul, centang
*Mode kompatibel*: toast dikirim memakai identitas PowerShell (tampil sebagai
"Windows PowerShell") yang selalu terdaftar di Windows.

Demi privasi mailbox, toast bawaan hanya berisi *"Ada email baru atau pengingat rapat"*.
Aktifkan *Tampilkan pengirim & subjek* bila ingin detail.

## Menu tray

Klik kiri ikon = buka jendela. Klik kanan:

- **Status** (mode, jumlah belum dibaca, RAM bila diagnostik aktif)
- Buka Email · Buka Kalender · Email baru · Pintasan · **Cek email sekarang**
- **Notifikasi**: mode Mati/Hemat/Siaga, interval Hemat (5/10/15/30 menit), hanya jam kerja,
  bunyi, tampilkan pengirim & subjek, **Tes notifikasi**, mode kompatibel
- **Jendela**: tutup otomatis saat tidak aktif, hemat memori WebView2, zoom
- **Pengaturan**: mulai dengan Windows, hotkey, aplikasi email default (mailto),
  update otomatis, cek update, diagnostik RAM, diagnostik halaman Outlook,
  buka folder pengaturan, muat ulang pengaturan
- Restart untuk memakai versi baru · Tutup jendela · Keluar

Ikon tray: **titik merah** = ada email belum dibaca, **titik kuning** = perlu login ulang.

## Fitur lain

- **Tutup otomatis saat tidak aktif** (bawaan 10 menit). Tidak berlaku saat sedang menulis email.
- **Hemat memori WebView2**: target memori rendah saat tersembunyi/diminimalkan; di mode
  Mati/Hemat halaman juga ditangguhkan saat diminimalkan.
- **Ingat ukuran, posisi, zoom** jendela.
- **Hotkey**: `Ctrl+Alt+O` buka/tutup jendela, `Ctrl+Alt+N` email baru.
- **Mailto**: centang *Aplikasi email default (mailto)*, lalu pilih Outlook Lite untuk
  MAILTO di jendela *Aplikasi default* yang terbuka. Tautan email akan membuka layar tulis
  Outlook web dengan penerima/subjek terisi.
- **Mulai dengan Windows** (registri pengguna, tanpa admin), mulai langsung di tray.
- **Update otomatis** dari GitHub Releases (cek saat start lalu tiap 24 jam), diverifikasi
  SHA-256. Versi baru aktif setelah restart.
- **Single instance**: menjalankan exe lagi hanya menampilkan jendela yang ada.
- Tautan di email dibuka di browser default Anda.

## settings.json

Di `%LOCALAPPDATA%\OutlookLite\settings.json`. Setelah diubah manual, pilih
*Muat ulang pengaturan* di tray. Contoh kunci yang hanya bisa diubah lewat file:

```json
{
  "work_start": "07:30",
  "work_end": "16:30",
  "work_days": [1, 2, 3, 4, 5, 6],
  "auto_close_min": 15,
  "hotkey_toggle": "Ctrl+Alt+O",
  "hotkey_compose": "Ctrl+Alt+N",
  "shortcuts": [
    { "name": "Kontak", "url": "https://outlook.office.com/people/" },
    { "name": "Intranet", "url": "https://intranet.contoh.co.id/" }
  ],
  "unread_selector": null,
  "no_gpu": false
}
```

`no_gpu` dan `extra_browser_args` berlaku setelah aplikasi di-restart.

## Diagnostik halaman (jika jumlah belum dibaca tidak terbaca)

Pembacaan jumlah belum dibaca bergantung pada struktur halaman Outlook. Bila status di tray
tidak pernah menampilkan jumlah:

1. Tray > Pengaturan > **Diagnostik halaman Outlook**.
2. File `diagnostik-halaman.json` terbuka di Notepad. Isinya hanya struktur halaman;
   pengirim, subjek, dan isi email tidak diambil (label baris email disamarkan).
3. Kirim isinya ke pengembang. Perbaikannya bisa berupa nilai `unread_selector` di settings.json
   tanpa perlu versi baru.

Skrip yang sama ada di `tools/inspect-outlook.js` untuk ditempel di Console DevTools browser.

## File dan log

Semua di `%LOCALAPPDATA%\OutlookLite\`: `settings.json`, `outlook-lite.log`,
`WebView2\` (data login), `diagnostik-halaman.json`.

## Batasan

- Mode Hemat mendeteksi email baru dari **kenaikan** jumlah belum dibaca. Bila Anda membaca
  email di perangkat lain di antara dua cek, email baru bisa tidak terdeteksi.
- Pengingat rapat hanya di mode Siaga.
- Bila sesi login kedaluwarsa, notifikasi berhenti sampai Anda login lagi (ikon kuning + toast).
- Toast yang sudah masuk Pusat Notifikasi mungkin tidak membuka jendela saat diklik.

## Build sendiri

GitHub Actions membangun setiap push. Rilis dibuat dengan tag yang sama dengan versi di
`Cargo.toml`: di GitHub buka *Releases > Draft a new release*, buat tag `v0.1.0`, lalu *Publish*. Actions akan melampirkan `outlook-lite.exe` dan `latest.json` (dibutuhkan update otomatis). Lokal di Windows: `cargo build --release`.
