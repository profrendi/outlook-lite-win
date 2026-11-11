//! Pemulihan otomatis bila Outlook gagal dimuat karena jaringan (mis. aplikasi dibuka
//! sebelum PC tersambung ke internet / Wi-Fi kantor, atau proxy kantor belum siap).
//!
//! Langkah: tunggu internet tersedia menurut Windows, lalu muat ulang Outlook. Bila masih
//! gagal, WebView2 dibuat ulang dari awal (proses browser baru), sama seperti Keluar lalu
//! membuka aplikasi lagi, tapi otomatis. Windows bisa salah melaporkan "tidak ada internet"
//! di jaringan kantor (proxy), jadi tetap dicoba berkala walau dilaporkan offline.

use std::time::{Duration, Instant};

/// Status kegagalan navigasi WebView2 yang disebabkan jaringan.
/// 6 server unreachable, 7 timeout, 9 aborted, 10 reset, 11 disconnected,
/// 12 cannot connect, 13 host not resolved, 16 unexpected error.
pub fn is_network_error(status: i32) -> bool {
    matches!(status, 6 | 7 | 9 | 10 | 11 | 12 | 13 | 16)
}

/// Jeda sebelum percobaan pertama setelah gagal.
const FIRST_DELAY: Duration = Duration::from_secs(3);
/// Jeda cek ulang selama Windows melaporkan offline.
const OFFLINE_POLL: Duration = Duration::from_secs(5);
/// Walau Windows melaporkan offline, tetap dicoba tiap ini (proxy kantor bisa menipu Windows).
const FORCE_EVERY: Duration = Duration::from_secs(60);
/// Jeda antar percobaan saat online.
const RETRY_EVERY: Duration = Duration::from_secs(20);
/// Setelah beberapa kali gagal, percobaan diperjarang.
const SLOW_AFTER: u32 = 6;
const SLOW_EVERY: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// Muat ulang halaman Outlook.
    Reload,
    /// Tutup WebView2 lalu buat lagi dari awal (proses browser baru).
    Recreate,
}

#[derive(Debug, Default)]
pub struct NetRecovery {
    since: Option<Instant>,
    attempts: u32,
    next: Option<Instant>,
    last_attempt: Option<Instant>,
}

impl NetRecovery {
    pub fn failing(&self) -> bool {
        self.since.is_some()
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    pub fn next_at(&self) -> Option<Instant> {
        self.next
    }

    /// Navigasi Outlook gagal karena jaringan.
    pub fn on_failure(&mut self, now: Instant) {
        if self.since.is_none() {
            self.since = Some(now);
            self.attempts = 0;
            self.last_attempt = None;
        }
        // Gagal sebelum/sesudah muat ulang pertama: lanjut cepat (3 dtk). Setelah WebView2
        // dibuat ulang, jadwal dari tick() (20 dtk / 2 menit) dipertahankan.
        let soon = now + FIRST_DELAY;
        if self.attempts <= 1 && self.next.is_none_or(|t| t > soon) {
            self.next = Some(soon);
        }
    }

    /// Outlook berhasil dimuat. Mengembalikan (lama gagal, jumlah percobaan) bila sebelumnya gagal.
    pub fn on_success(&mut self, now: Instant) -> Option<(Duration, u32)> {
        let since = self.since.take()?;
        let n = self.attempts;
        *self = Self::default();
        Some((now.duration_since(since), n))
    }

    /// Pengguna membuka jendela: coba segera.
    pub fn poke(&mut self, now: Instant) {
        if self.since.is_some() {
            self.next = Some(now);
        }
    }

    pub fn tick(&mut self, now: Instant, online: bool) -> Action {
        if self.since.is_none() || self.next.is_none_or(|t| now < t) {
            return Action::None;
        }
        let forced = self.last_attempt.is_none_or(|t| now.duration_since(t) >= FORCE_EVERY);
        if !online && !forced {
            self.next = Some(now + OFFLINE_POLL);
            return Action::None;
        }
        self.attempts += 1;
        self.last_attempt = Some(now);
        let every = if self.attempts >= SLOW_AFTER { SLOW_EVERY } else { RETRY_EVERY };
        self.next = Some(now + every);
        if self.attempts == 1 {
            Action::Reload
        } else {
            Action::Recreate
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: Instant, secs: u64) -> Instant {
        t + Duration::from_secs(secs)
    }

    #[test]
    fn jenis_error() {
        assert!(is_network_error(11)); // disconnected
        assert!(is_network_error(13)); // DNS
        assert!(!is_network_error(14)); // dibatalkan (redirect biasa)
        assert!(!is_network_error(3)); // sertifikat
    }

    #[test]
    fn offline_lalu_online_muat_ulang_lalu_buat_ulang() {
        let t = Instant::now();
        let mut r = NetRecovery::default();
        r.on_failure(t);
        assert_eq!(r.tick(s(t, 1), true), Action::None); // tunggu 3 dtk
        // Belum ada internet (login Windows): menunggu, tidak mencoba.
        assert_eq!(r.tick(s(t, 3), false), Action::Reload); // percobaan pertama tetap sekali
        r.on_failure(s(t, 4)); // gagal lagi, masih offline
        assert_eq!(r.tick(s(t, 10), false), Action::None);
        assert_eq!(r.tick(s(t, 30), false), Action::None);
        // Internet tersambung: dicoba pada pemeriksaan berikutnya (paling lama 5 detik).
        assert_eq!(r.tick(s(t, 31), true), Action::None);
        assert_eq!(r.tick(s(t, 35), true), Action::Recreate);
        assert!(r.on_success(s(t, 40)).is_some());
        assert!(!r.failing());
        assert_eq!(r.tick(s(t, 100), true), Action::None);
    }

    #[test]
    fn windows_salah_lapor_offline_tetap_dicoba() {
        let t = Instant::now();
        let mut r = NetRecovery::default();
        r.on_failure(t);
        assert_eq!(r.tick(s(t, 3), false), Action::Reload);
        r.on_failure(s(t, 5));
        assert_eq!(r.tick(s(t, 60), false), Action::None);
        assert_eq!(r.tick(s(t, 65), false), Action::Recreate);
        // Setelah dibuat ulang dan gagal lagi: tidak langsung diulang terus-menerus.
        r.on_failure(s(t, 70));
        assert_eq!(r.tick(s(t, 75), true), Action::None);
        assert_eq!(r.tick(s(t, 85), true), Action::Recreate);
    }

    #[test]
    fn buka_jendela_mencoba_segera() {
        let t = Instant::now();
        let mut r = NetRecovery::default();
        r.on_failure(t);
        assert_eq!(r.tick(s(t, 3), true), Action::Reload);
        r.on_failure(s(t, 4));
        r.poke(s(t, 5));
        assert_eq!(r.tick(s(t, 5), true), Action::Recreate);
    }

    #[test]
    fn diperjarang_setelah_banyak_gagal() {
        let t = Instant::now();
        let mut r = NetRecovery::default();
        r.on_failure(t);
        let mut now = s(t, 3);
        for _ in 0..SLOW_AFTER {
            assert_ne!(r.tick(now, true), Action::None);
            r.on_failure(now);
            now = r.next_at().unwrap();
        }
        assert!(r.next_at().unwrap() >= now);
        assert_eq!(r.attempts(), SLOW_AFTER);
    }
}
