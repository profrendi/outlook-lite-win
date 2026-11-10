# Outlook Lite

Outlook web (outlook.office.com) di system tray Windows, ditulis dengan Rust.
Portable, **tanpa hak admin**, memakai WebView2 bawaan Windows dan login web biasa
(tidak butuh izin Microsoft Graph dari admin kantor).

## Unduh dan pasang

1. Buka halaman **Releases** repo ini, unduh `outlook-lite.exe`.
2. Jalankan dari mana saja (mis. folder Downloads). Pada start pertama aplikasi otomatis:
   - menyalin dirinya ke `%LOCALAPPDATA%\Programs\OutlookLite\`,
   - membuat shortcut **Outlook Lite** di Start Menu,
   - mendaftar di **Settings > Apps > Installed apps** (bisa di-uninstall dari sana),
   - lalu berjalan dari folder itu. Semua tanpa hak admin.
3. Login sekali; login tersimpan. File di Downloads boleh dihapus.

Butuh WebView2 Runtime (sudah ada di Windows 11 dan di PC yang memakai Edge).

**Mode portable** (tanpa instalasi): letakkan file kosong bernama `portable` di sebelah exe,
atau jalankan dengan argumen `--portable`.

**Hapus instalasi**: tray > Pengaturan > *Hapus instalasi…*, atau dari Installed apps.
Pengaturan dan data login Outlook Lite ikut dihapus.

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

Notifikasi berasal dari beberapa sinyal yang dipancarkan Outlook web sendiri: notifikasi
bawaan Outlook web, **suara email masuk** Outlook (sama seperti PWA), **angka badge** (sumber
tanda merah di taskbar), dan **jumlah belum dibaca di Inbox**. Pastikan di Outlook web:
*Pengaturan > Umum > Pemberitahuan* → pemberitahuan desktop aktif.

Semua sinyal dalam beberapa detik digabung menjadi **satu toast**:

```
3 email baru
Belum dibaca di Inbox: 15
```

- Baris atas: jumlah email yang baru masuk (email beruntun digabung).
- Baris bawah: total belum dibaca di Inbox, sudah termasuk email baru itu.
- Bila jumlah belum dibaca tidak terbaca, toast hanya berisi *"Email baru"*.
- Notifikasi Outlook tanpa kenaikan jumlah (biasanya pengingat rapat) tampil sebagai toast
  *"Outlook"* terpisah.
- Suara Outlook ikut berbunyi; toast tidak menambah suara kedua.

Tanda merah juga muncul di ikon taskbar selama jendela terbuka atau diminimalkan.

Gunakan **Tes notifikasi** setelah instal. Jika toast tidak muncul, centang
*Mode kompatibel*: toast dikirim memakai identitas PowerShell (tampil sebagai
"Windows PowerShell") yang selalu terdaftar di Windows.

Demi privasi mailbox, toast tidak berisi pengirim atau subjek. Aktifkan *Tampilkan pengirim
& subjek* bila ingin detail: untuk satu email baru baris atas menjadi *"Dari X"* diikuti subjek.

## Menu tray

Klik kiri ikon = buka jendela. Klik kanan:

- **Status** (mode, jumlah belum dibaca, RAM bila diagnostik aktif)
- Buka Email · Buka Kalender · Email baru · Pintasan · **Cek email sekarang**
- **Notifikasi**: mode Mati/Hemat/Siaga, interval Hemat (5/10/15/30 menit), hanya jam kerja,
  bunyi, tampilkan pengirim & subjek, **Tes notifikasi**, mode kompatibel
- **Jendela**: tutup otomatis saat tidak aktif, hemat memori WebView2, zoom
- **Pengaturan**: mulai dengan Windows, hotkey, aplikasi email default (mailto),
  update otomatis, cek update, diagnostik RAM, diagnostik halaman Outlook,
  buka folder pengaturan, buka folder aplikasi, muat ulang pengaturan, hapus instalasi
- Restart untuk memakai versi baru · Tutup jendela · Keluar

Ikon tray: **titik merah** = ada email belum dibaca, **titik kuning** = perlu login ulang.

## Fitur lain

- **Tutup otomatis saat tidak aktif** (bawaan 10 menit). Tidak berlaku saat sedang menulis email.
- **Hemat memori WebView2**: target memori rendah saat tersembunyi/diminimalkan; di mode
  Mati/Hemat halaman juga ditangguhkan saat diminimalkan.
- **Ingat ukuran, posisi, zoom** jendela.
- **Hotkey**: `Ctrl+Alt+O` buka/tutup jendela, `Ctrl+Alt+N` email baru. Bila sudah dipakai
  aplikasi lain, otomatis diganti kombinasi cadangan (mis. `Ctrl+Alt+M`) dan diberitahukan sekali.
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

Aplikasi: `%LOCALAPPDATA%\Programs\OutlookLite\outlook-lite.exe`. Data: `%LOCALAPPDATA%\OutlookLite\`: `settings.json`, `outlook-lite.log`,
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
