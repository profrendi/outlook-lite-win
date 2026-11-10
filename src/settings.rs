//! Pengaturan yang disimpan di %LOCALAPPDATA%\OutlookLite\settings.json.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Tanpa pemantauan: Outlook hanya hidup saat jendela terbuka.
    Mati,
    /// Buka Outlook tersembunyi tiap N menit, baca, lalu hancurkan.
    Hemat,
    /// Outlook tersembunyi selalu hidup; notifikasi hampir real-time.
    #[default]
    Siaga,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Mati => "Mati",
            Mode::Hemat => "Hemat",
            Mode::Siaga => "Siaga",
        }
    }
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
    pub mode: Mode,
    pub hemat_interval_min: u32,
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
    /// Matikan akselerasi GPU (berlaku setelah restart).
    pub no_gpu: bool,
    /// Argumen tambahan WebView2 (lanjutan, berlaku setelah restart).
    pub extra_browser_args: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Siaga,
            hemat_interval_min: 15,
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
            extra_browser_args: String::new(),
        }
    }
}

pub const HEMAT_INTERVALS: [u32; 4] = [5, 10, 15, 30];

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
        self.hemat_interval_min = self.hemat_interval_min.clamp(1, 240);
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

    /// Mode yang benar-benar berlaku saat ini (di luar jam kerja = Mati).
    pub fn active_mode(&self, weekday: u8, minute: u32) -> Mode {
        if self.mode == Mode::Mati || !self.in_work_hours(weekday, minute) {
            Mode::Mati
        } else {
            self.mode
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
        assert_eq!(s.active_mode(6, 600), Mode::Mati);
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
        let s: Settings = serde_json::from_str(r#"{"mode":"hemat","zoom":9}"#).unwrap();
        assert_eq!(s.mode, Mode::Hemat);
        assert!(s.hotkeys);
        let mut s2 = s.clone();
        s2.sanitize();
        assert_eq!(s2.zoom, 1.0);
        let j = serde_json::to_string(&s2).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&j).unwrap(), s2);
        assert_eq!(Settings::default().work_hours_label(), "08:00–17:00, Sen–Jum");
    }
}
