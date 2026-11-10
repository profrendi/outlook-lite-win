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

## Cara kerja dan notifikasi

Outlook Lite selalu memakai satu cara kerja (Siaga): Outlook web tetap hidup tersembunyi
saat jendela ditutup ke tray atau diminimalkan, jadi email baru dan pengingat rapat
diberitahukan dalam hitungan detik. Tombol X menyembunyikan jendela ke tray.

**Hanya jam kerja** (bawaan 08:00–17:00, Senin–Jumat): di luar jam itu pemantauan dijeda.
Outlook tersembunyi ditutup (RAM ~0) dan hanya hidup saat jendela dibuka. Matikan opsi ini
bila ingin notifikasi 24 jam.

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
- Baris bawah: total belum dibaca di Inbox, sudah termasuk email baru itu. Angka ini dibaca
  dari folder **Kotak Masuk** di panel folder Outlook (sama dengan yang Anda lihat), dan dibaca
  ulang saat email masuk serta saat *Cek email sekarang*. Badge Outlook (tanda merah PWA)
  hanya menghitung email baru sejak terakhir dilihat, jadi hanya dipakai sebagai tanda email
  masuk, atau sebagai cadangan bila panel folder tidak tampil (jendela terlalu sempit).
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
- **Notifikasi**: hanya jam kerja, bunyi, tampilkan pengirim & subjek, **Tes notifikasi**, mode kompatibel
- **Jendela**: tutup otomatis saat tidak aktif, hemat RAM, tanpa GPU, tanpa service worker, zoom
- **Pengaturan**: mulai dengan Windows, hotkey, aplikasi email default (mailto),
  update otomatis, cek update, catat diagnostik, buat laporan diagnostik, diagnostik halaman Outlook,
  buka folder pengaturan, buka folder aplikasi, muat ulang pengaturan, hapus instalasi
- Restart untuk memakai versi baru · Tutup jendela · Keluar

Ikon tray: **titik merah** = ada email belum dibaca, **titik kuning** = perlu login ulang.

## Fitur lain

- **Tutup otomatis saat tidak aktif** (bawaan 10 menit). Tidak berlaku saat sedang menulis email.
- **Hemat RAM** (bawaan aktif), lihat bagian di bawah.
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

## Hemat RAM

| Keadaan | Yang dilakukan |
|---|---|
| Selalu | Tanpa GPU, tanpa service worker Outlook, V8 hemat memori, mode perangkat RAM kecil Chromium, tanpa proses renderer cadangan, tanpa cache halaman sebelumnya, tanpa layanan jaringan latar, iframe Microsoft berbagi proses dengan Outlook, telemetri Outlook/Office diblokir, autofill & simpan kata sandi WebView2 mati |
| Dipakai, tapi tidak difokus > 1 menit | WebView2 diminta memakai memori rendah; normal lagi saat jendela difokus |
| Diminimalkan / di tray | Tampilan WebView2 dimatikan; **mode efisiensi** Windows (daun hijau di Task Manager), prioritas CPU dan memori rendah; RAM dicek tiap 30 detik dan dirapikan otomatis bila melewati 40 MB (paling sering tiap 60 detik, paling lambat tiap 5 menit), sekaligus memicu WebView2 membersihkan memori JavaScript; panel folder dibaca tiap 9 detik (email masuk tetap terdeteksi seketika) |
| Dibuka lagi | Prioritas dan memori langsung kembali normal |
| Di tray berjam-jam | Outlook disegarkan otomatis bila RAM nyata naik lagi di atas 350 MB setelah dirapikan, commit melebihi 700 MB, atau sudah berjalan 8 jam (hanya bila tersembunyi ≥ 10 menit dan tidak sedang menulis email) |
| Di luar jam kerja | Outlook tersembunyi ditutup |

Merapikan memori melepas halaman yang jarang disentuh dari RAM fisik; Windows memuatnya
kembali saat dibutuhkan, jadi angka di Task Manager bisa naik perlahan lagi. Saat jendela
dibuka setelah lama di tray, tampilan pertama bisa terasa sedikit lebih lambat.

Di menu **Jendela** (bawaan aktif):
- **Tanpa GPU** (perlu restart): proses GPU turun dari ~70–90 MB ke ~13 MB. Gulir sedikit
  kurang halus.
- **Tanpa service worker Outlook**: menghapus satu proses (~30–45 MB) yang hanya dipakai
  Outlook untuk cache/offline. Outlook mungkin sedikit lebih lama terbuka pertama kali.
  Notifikasi tidak terpengaruh.

**Eksperimen tampilan mobile di tray** (`tray_mobile_ui`, bawaan mati, lewat settings.json):
Outlook tersembunyi dimuat dengan tampilan HP yang mungkin lebih ringan. Membuka jendela jadi
beberapa detik lebih lama (dimuat ulang versi desktop), dan jumlah belum dibaca di tray memakai
badge Outlook. Bandingkan hasilnya lewat laporan diagnostik.

## Laporan diagnostik (RAM & CPU)

1. Tray > Pengaturan > centang **Catat diagnostik (RAM & CPU tiap 30 detik)**.
2. Pakai seperti biasa (buka, minimize, tutup ke tray) setidaknya 30–60 menit.
3. Tray > Pengaturan > **Buat laporan diagnostik**. File `laporan-diagnostik.txt` terbuka di
   Notepad dan Explorer. Kirim file itu.

Isi laporan: versi aplikasi, Windows, WebView2, RAM PC, pengaturan, ringkasan RAM/CPU per
keadaan (dipakai, tak-fokus, minimize, tray, jeda), 300 baris log terakhir, dan data mentah
per 30 detik: RAM & CPU per jenis proses (aplikasi, WebView2 Manager, renderer Outlook,
renderer lain, GPU, utility), jumlah belum dibaca, dan kejadian (pindah keadaan, rapikan
memori sebelum/sesudah, segarkan, toast, perlu login, sumber angka belum dibaca, proses
WebView2 yang mati/macet). Log juga mencatat setiap aplikasi mulai dan keluar beserta
alasannya, sesi yang berhenti tidak normal, dan lokasi error bila aplikasi crash. Tidak berisi
pengirim, subjek, atau isi email. Data mentah juga ada di `diagnostik.csv`.

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
  "refresh_mb": 700,
  "refresh_ws_mb": 350,
  "trim_ws_mb": 40,
  "efficiency_mode": true,
  "block_telemetry": true,
  "low_end_mode": true,
  "refresh_hours": 8,
  "tray_mobile_ui": false,
  "site_isolation": false,
  "no_gpu": true,
  "block_service_worker": true,
  "v8_optimize_for_size": true
}
```

`refresh_mb` / `refresh_ws_mb` / `refresh_hours` = 0 mematikan penyegaran otomatis. `trim_ws_mb` = 0
hanya merapikan berkala. `low_end_mode` bisa membuat gulir terlihat sedikit kurang halus; set `false`
bila mengganggu. `site_isolation`, `no_gpu`, `low_end_mode`, `v8_optimize_for_size`, dan
`extra_browser_args` berlaku setelah aplikasi di-restart.

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

- Di luar jam kerja (bila *Hanya jam kerja* aktif) tidak ada notifikasi.
- Saat Outlook tersembunyi disegarkan, notifikasi berhenti beberapa detik selama halaman dimuat.
- Bila sesi login kedaluwarsa, notifikasi berhenti sampai Anda login lagi (ikon kuning + toast).
- Toast yang sudah masuk Pusat Notifikasi mungkin tidak membuka jendela saat diklik.

## Build dan rilis

GitHub Actions membangun setiap push. Bila versi di `Cargo.toml` belum punya rilis, push ke
`main` otomatis menerbitkan rilis `vX.Y.Z` berisi `outlook-lite.exe` dan `latest.json`, dan
aplikasi yang terpasang menemukannya lewat update otomatis. Tidak perlu membuat rilis manual.
Lokal di Windows: `cargo build --release`.
