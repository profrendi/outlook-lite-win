//! Pengaturan yang disimpan di %LOCALAPPDATA%\OutlookLite\settings.json.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Keadaan pemantauan saat ini. Aplikasi selalu memakai Siaga; di luar jam kerja
/// (bila "Hanya jam kerja" aktif) pemantauan dijeda.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Di luar jam kerja: Outlook hanya hidup saat jendela terbuka.
    Jeda,
    /// Outlook tersembunyi selalu hidup; notifikasi hampir real-time.
    Siaga,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Shortcut {
    pub name: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct WindowGeom {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    // --- notifikasi ---
    pub work_hours_only: bool,
    /// "HH:MM"
    pub work_start: String,
    pub work_end: String,
    /// 1 = Senin ... 7 = Minggu
    pub work_days: Vec<u8>,
    pub sound: bool,
    /// Tampilkan pengirim/subjek di notifikasi (mati bawaan demi privasi).
    pub show_details: bool,
    /// Kirim toast memakai identitas PowerShell (jika toast dengan identitas sendiri tidak muncul).
    pub toast_compat: bool,
    /// Selector CSS manual untuk angka belum dibaca (hasil Diagnostik halaman).
    pub unread_selector: Option<String>,

    // --- jendela ---
    pub auto_close: bool,
    pub auto_close_min: u32,
    pub low_memory: bool,
    /// Segarkan Outlook tersembunyi bila memori WebView2 (commit) melebihi ini, dalam MB. 0 = tidak.
    pub refresh_mb: u32,
    /// Saat tersembunyi/diminimalkan, rapikan RAM otomatis bila melewati ini (MB). 0 = hanya berkala.
    pub trim_ws_mb: u32,
    /// Mode efisiensi Windows (EcoQoS) + prioritas rendah untuk WebView2 saat tersembunyi.
    pub efficiency_mode: bool,
    /// Blokir telemetri Outlook/Office di dalam WebView2.
    pub block_telemetry: bool,
    /// Mode perangkat RAM kecil Chromium (berlaku setelah restart).
    pub low_end_mode: bool,
    /// Tema jendela Pengaturan & Outlook Lite: "system", "light", atau "dark".
    pub theme: String,
    /// Buat environment WebView2 sendiri (opsi resmi di bawah). Bila gagal, otomatis cara lama.
    pub custom_environment: bool,
    /// Tracking prevention WebView2 (daftar pelacak). Mati = lebih ringan; aman karena hanya Outlook.
    pub tracking_prevention: bool,
    /// Laporan crash WebView2 disimpan lokal, tidak dikirim ke Microsoft.
    pub local_crash_reports: bool,
    /// Flag WebView2: WebView tersembunyi diperlakukan seperti kekurangan memori (bersih-bersih sendiri).
    pub simulate_memory_pressure: bool,
    /// Proxy dibaca lewat Windows (WinHTTP). Membantu jaringan kantor saat PC baru menyala.
    pub system_proxy_resolver: bool,
    /// Segarkan Outlook tersembunyi bila RAM nyata (working set) naik lagi melebihi ini, MB. 0 = tidak.
    pub refresh_ws_mb: u32,
    /// Eksperimen: Outlook tersembunyi di tray memakai tampilan mobile (mungkin lebih ringan).
    /// Jumlah belum dibaca lalu memakai badge Outlook. Membuka jendela jadi beberapa detik lebih lama.
    pub tray_mobile_ui: bool,
    /// Segarkan Outlook tersembunyi paling lambat tiap N jam (Outlook web cenderung membengkak). 0 = tidak.
    pub refresh_hours: u32,
    pub zoom: f64,
    pub window: Option<WindowGeom>,
    pub shortcuts: Vec<Shortcut>,

    // --- sistem ---
    pub autostart: bool,
    pub hotkeys: bool,
    pub hotkey_toggle: String,
    pub hotkey_compose: String,
    /// Kombinasi hotkey gagal yang sudah diberitahukan (agar toast tidak berulang).
    pub hotkey_warned: String,
    pub auto_update: bool,
    pub diagnostics: bool,
    /// Versi lama (sampai v0.1.15): GPU mati. Hanya dibaca untuk migrasi ke `gpu_mode`.
    #[serde(default, skip_serializing)]
    pub no_gpu: bool,
    /// GPU WebView2: "full" (penuh), "lite" (komposisi GPU, gambar halaman oleh CPU), "off".
    /// Berlaku setelah restart.
    pub gpu_mode: String,
    /// Pratinjau Excel/Word/PowerPoint Online berjalan di proses sendiri (berlaku setelah restart).
    pub isolate_office: bool,
    /// Blokir iklan/konten MSN dan telemetri Excel/Azure (arc.msn.com dll.).
    pub block_extra: bool,
    /// Matikan eksperimen jarak jauh Microsoft (field trial) dan API iklan Chromium (restart).
    pub disable_field_trials: bool,
    /// Uji coba: Outlook selalu dimuat dari outlook.cloud.microsoft (satu alamat, satu cache).
    pub unify_domain: bool,
    /// Peringatan bila RAM Outlook Lite melewati ini (MB) saat jendela dipakai. 0 = mati.
    pub memory_warn_mb: u32,
    // --- lampiran ---
    /// Lampiran Office yang diunduh langsung dibuka di aplikasi desktop (Excel/Word).
    pub attach_desktop: bool,
    /// Hanya file berukuran minimal ini (MB). 0 = semua ukuran.
    pub attach_min_mb: f64,
    pub attach_excel: bool,
    pub attach_word: bool,
    pub attach_ppt: bool,
    pub attach_pdf: bool,
    /// Eksperimen: klik lampiran Office = unduh lalu buka di aplikasi desktop (bukan pratinjau web).
    pub attach_intercept_preview: bool,
    /// Versi format pengaturan (untuk migrasi otomatis). File lama tanpa kunci ini = 0.
    #[serde(default)]
    pub settings_version: u32,
    /// Matikan service worker Outlook (proses renderer tambahan ~30-45 MB).
    pub block_service_worker: bool,
    /// V8 mengutamakan ukuran memori (berlaku setelah restart).
    pub v8_optimize_for_size: bool,
    /// Pisahkan iframe lintas-situs ke proses sendiri (lebih banyak proses & RAM). Berlaku setelah restart.
    pub site_isolation: bool,
    /// Argumen tambahan WebView2 (lanjutan, berlaku setelah restart).
    pub extra_browser_args: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            work_hours_only: true,
            work_start: "08:00".into(),
            work_end: "17:00".into(),
            work_days: vec![1, 2, 3, 4, 5],
            sound: true,
            show_details: false,
            toast_compat: false,
            unread_selector: None,
            auto_close: true,
            auto_close_min: 10,
            low_memory: true,
            refresh_mb: 700,
            refresh_ws_mb: 350,
            trim_ws_mb: 40,
            efficiency_mode: true,
            block_telemetry: true,
            low_end_mode: false,
            theme: "system".into(),
            custom_environment: true,
            tracking_prevention: false,
            local_crash_reports: true,
            simulate_memory_pressure: true,
            system_proxy_resolver: true,
            tray_mobile_ui: false,
            refresh_hours: 8,
            zoom: 1.0,
            window: None,
            shortcuts: vec![
                Shortcut {
                    name: "Kontak".into(),
                    url: "https://outlook.office.com/people/".into(),
                },
                Shortcut {
                    name: "To Do".into(),
                    url: "https://to-do.office.com/tasks/".into(),
                },
            ],
            autostart: false,
            hotkeys: true,
            hotkey_toggle: "Ctrl+Alt+O".into(),
            hotkey_compose: "Ctrl+Alt+N".into(),
            hotkey_warned: String::new(),
            auto_update: true,
            diagnostics: false,
            no_gpu: false,
            gpu_mode: "lite".into(),
            isolate_office: true,
            block_extra: true,
            disable_field_trials: true,
            unify_domain: false,
            memory_warn_mb: 1000,
            attach_desktop: true,
            attach_min_mb: 0.0,
            attach_excel: true,
            attach_word: true,
            attach_ppt: false,
            attach_pdf: false,
            attach_intercept_preview: false,
            settings_version: SETTINGS_VERSION,
            block_service_worker: true,
            v8_optimize_for_size: true,
            site_isolation: false,
            extra_browser_args: String::new(),
        }
    }
}

/// Versi format pengaturan saat ini. Naikkan bila perlu migrasi baru (lihat `migrate`).
pub const SETTINGS_VERSION: u32 = 16;

fn parse_hm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

impl Settings {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("settings.json")
    }

    /// Baca pengaturan; file rusak disimpan sebagai cadangan lalu kembali ke bawaan.
    pub fn load(dir: &Path) -> Self {
        let p = Self::path(dir);
        match std::fs::read_to_string(&p) {
            Ok(s) => match serde_json::from_str::<Settings>(&s) {
                Ok(mut st) => {
                    if st.migrate() {
                        let _ = std::fs::copy(&p, dir.join("settings.sebelum-migrasi.json"));
                        st.sanitize();
                        let _ = st.save(dir);
                    }
                    st.sanitize();
                    st
                }
                Err(_) => {
                    let _ = std::fs::copy(&p, dir.join("settings.rusak.json"));
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join("settings.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default())?;
        std::fs::rename(tmp, Self::path(dir))
    }

    /// Perbarui pengaturan dari versi lama. Mengembalikan true bila ada yang diubah.
    pub fn migrate(&mut self) -> bool {
        if self.settings_version >= SETTINGS_VERSION {
            return false;
        }
        if self.settings_version < 16 {
            // v0.1.16: Mode RAM kecil Chromium membuat Outlook kehabisan memori saat membuka
            // lampiran besar (Chromium mengira PC hanya ber-RAM ~512 MB) → dimatikan.
            self.low_end_mode = false;
            // "Tanpa GPU" diganti pilihan GPU; GPU mati menjadi "Ringan" (gulir lebih halus).
            self.gpu_mode = "lite".into();
            self.no_gpu = false;
        }
        self.settings_version = SETTINGS_VERSION;
        true
    }

    pub fn sanitize(&mut self) {
        if !matches!(self.gpu_mode.as_str(), "full" | "lite" | "off") {
            self.gpu_mode = "lite".into();
        }
        if self.memory_warn_mb != 0 {
            self.memory_warn_mb = self.memory_warn_mb.clamp(300, 16000);
        }
        if !self.attach_min_mb.is_finite() || self.attach_min_mb < 0.0 {
            self.attach_min_mb = 0.0;
        }
        self.attach_min_mb = self.attach_min_mb.min(2000.0);
        if self.refresh_mb != 0 {
            self.refresh_mb = self.refresh_mb.clamp(300, 8000);
        }
        self.refresh_hours = self.refresh_hours.min(72);
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            self.theme = "system".into();
        }
        if self.trim_ws_mb != 0 {
            self.trim_ws_mb = self.trim_ws_mb.clamp(15, 2000);
        }
        if self.refresh_ws_mb != 0 {
            self.refresh_ws_mb = self.refresh_ws_mb.clamp(150, 8000);
        }
        self.auto_close_min = self.auto_close_min.clamp(1, 240);
        if !(0.25..=4.0).contains(&self.zoom) || self.zoom.is_nan() {
            self.zoom = 1.0;
        }
        self.work_days.retain(|d| (1..=7).contains(d));
        self.shortcuts
            .retain(|s| s.url.starts_with("https://") && !s.name.trim().is_empty());
        self.shortcuts.truncate(15);
    }

    /// Apakah waktu `weekday` (1=Senin..7=Minggu) dan `minute` (0..1439) termasuk jam kerja.
    pub fn in_work_hours(&self, weekday: u8, minute: u32) -> bool {
        if !self.work_hours_only {
            return true;
        }
        let (Some(start), Some(end)) = (parse_hm(&self.work_start), parse_hm(&self.work_end))
        else {
            return true; // format salah: anggap selalu jam kerja
        };
        if !self.work_days.contains(&weekday) {
            return false;
        }
        if start <= end {
            minute >= start && minute < end
        } else {
            // shift malam, mis. 22:00-06:00
            minute >= start || minute < end
        }
    }

    /// Keadaan yang berlaku saat ini (di luar jam kerja = Jeda).
    pub fn active_mode(&self, weekday: u8, minute: u32) -> Mode {
        if self.in_work_hours(weekday, minute) {
            Mode::Siaga
        } else {
            Mode::Jeda
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_hours() {
        let s = Settings::default();
        assert!(s.in_work_hours(1, 8 * 60));
        assert!(s.in_work_hours(5, 16 * 60 + 59));
        assert!(!s.in_work_hours(5, 17 * 60));
        assert!(!s.in_work_hours(6, 10 * 60));
        assert_eq!(s.active_mode(6, 600), Mode::Jeda);
        assert_eq!(s.active_mode(2, 600), Mode::Siaga);

        let mut n = s.clone();
        n.work_start = "22:00".into();
        n.work_end = "06:00".into();
        assert!(n.in_work_hours(1, 23 * 60));
        assert!(n.in_work_hours(1, 60));
        assert!(!n.in_work_hours(1, 12 * 60));

        let mut off = s.clone();
        off.work_hours_only = false;
        assert!(off.in_work_hours(7, 3 * 60));
    }

    #[test]
    fn roundtrip_and_partial() {
        // Pengaturan versi lama (mode Hemat) tetap terbaca; kunci lama diabaikan.
        let s: Settings =
            serde_json::from_str(r#"{"mode":"hemat","hemat_interval_min":15,"zoom":9}"#).unwrap();
        assert!(s.hotkeys);
        assert_eq!(s.refresh_mb, 700);
        let mut s2 = s.clone();
        s2.sanitize();
        assert_eq!(s2.zoom, 1.0);
        let j = serde_json::to_string(&s2).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&j).unwrap(), s2);
    }

    #[test]
    fn isian_halaman_pengaturan_cocok_dengan_kunci() {
        let html = include_str!("ui/settings.html");
        let v = serde_json::to_value(Settings::default()).unwrap();
        let mut n = 0;
        for part in html.split("data-key=\"").skip(1) {
            let key = &part[..part.find('"').unwrap()];
            assert!(
                v.get(key).is_some(),
                "kunci tidak dikenal di settings.html: {key}"
            );
            n += 1;
        }
        assert!(n >= 30, "terlalu sedikit isian terbaca: {n}");
    }

    #[test]
    fn migrasi_dari_v0_1_15() {
        let lama = r#"{"low_end_mode":true,"no_gpu":true,"zoom":1.0,"sound":false}"#;
        let mut s: Settings = serde_json::from_str(lama).unwrap();
        assert_eq!(s.settings_version, 0);
        assert!(s.migrate());
        assert!(!s.low_end_mode);
        assert_eq!(s.gpu_mode, "lite");
        assert!(!s.sound, "pengaturan lain tetap");
        assert_eq!(s.settings_version, SETTINGS_VERSION);
        assert!(!s.migrate(), "migrasi hanya sekali");
        // Setelah disimpan, no_gpu tidak ditulis lagi dan migrasi tidak berulang.
        let baru = serde_json::to_string(&s).unwrap();
        assert!(!baru.contains("no_gpu"));
        let mut s2: Settings = serde_json::from_str(&baru).unwrap();
        assert!(!s2.migrate());
        // Pengguna lalu menyalakan mode RAM kecil lagi: tidak dimatikan ulang.
        s2.low_end_mode = true;
        let mut s3: Settings = serde_json::from_str(&serde_json::to_string(&s2).unwrap()).unwrap();
        assert!(!s3.migrate());
        assert!(s3.low_end_mode);
    }

    #[test]
    fn instal_baru_tidak_perlu_migrasi() {
        let mut s = Settings::default();
        assert!(!s.migrate());
        assert_eq!(s.gpu_mode, "lite");
        assert!(!s.low_end_mode);
    }
}
