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
    /// Matikan akselerasi GPU (berlaku setelah restart). Hemat ~50-80 MB.
    pub no_gpu: bool,
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
            low_end_mode: true,
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
            no_gpu: true,
            block_service_worker: true,
            v8_optimize_for_size: true,
            site_isolation: false,
            extra_browser_args: String::new(),
        }
    }
}

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

    pub fn sanitize(&mut self) {
        if self.refresh_mb != 0 {
            self.refresh_mb = self.refresh_mb.clamp(300, 8000);
        }
        self.refresh_hours = self.refresh_hours.min(72);
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

    pub fn work_hours_label(&self) -> String {
        let mut days: Vec<u8> = self.work_days.clone();
        days.sort_unstable();
        days.dedup();
        const NAMES: [&str; 7] = ["Sen", "Sel", "Rab", "Kam", "Jum", "Sab", "Min"];
        let d = if days == [1, 2, 3, 4, 5] {
            "Sen–Jum".to_string()
        } else if days == [1, 2, 3, 4, 5, 6] {
            "Sen–Sab".to_string()
        } else {
            days.iter()
                .map(|d| NAMES[(*d as usize).saturating_sub(1).min(6)])
                .collect::<Vec<_>>()
                .join(",")
        };
        format!("{}–{}, {d}", self.work_start, self.work_end)
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
        assert_eq!(Settings::default().work_hours_label(), "08:00–17:00, Sen–Jum");
    }

    #[test]
    fn isian_halaman_pengaturan_cocok_dengan_kunci() {
        let html = include_str!("ui/settings.html");
        let v = serde_json::to_value(Settings::default()).unwrap();
        let mut n = 0;
        for part in html.split("data-key=\"").skip(1) {
            let key = &part[..part.find('"').unwrap()];
            assert!(v.get(key).is_some(), "kunci tidak dikenal di settings.html: {key}");
            n += 1;
        }
        assert!(n >= 30, "terlalu sedikit isian terbaca: {n}");
    }
}
