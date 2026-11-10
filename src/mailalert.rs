//! Menggabungkan sinyal email masuk menjadi satu toast:
//!
//! ```text
//! 3 email baru                 <- jumlah email yang baru masuk
//! Belum dibaca di Inbox: 15    <- total belum dibaca, sudah termasuk email baru
//! ```
//!
//! Sinyal datang dari angka badge Outlook, jumlah belum dibaca di halaman, suara email
//! masuk, dan notifikasi bawaan Outlook. Urutan dan jedanya tidak pasti, jadi semua sinyal
//! dalam beberapa detik dikumpulkan dulu, lalu dihitung sekali saat batch ditutup.

use std::time::{Duration, Instant};

/// Batch ditutup setelah sepi selama ini sejak sinyal terakhir, bila angka sudah naik.
pub const QUIET_COUNTED: Duration = Duration::from_millis(2500);
/// Bila baru ada suara/notifikasi, tunggu lebih lama: angka halaman sering menyusul.
pub const QUIET_SIGNAL: Duration = Duration::from_secs(6);
/// Batas lama satu batch.
pub const MAX_BATCH: Duration = Duration::from_secs(12);
/// Total yang sudah diumumkan dianggap berlaku selama ini (menahan toast ganda saat angka menyusul).
pub const ANNOUNCED_TTL: Duration = Duration::from_secs(90);
/// Suara/notifikasi yang datang sesaat setelah toast email adalah gema email yang sama.
pub const ECHO: Duration = Duration::from_secs(20);

const MAX_LINE: usize = 120;

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub id: u64,
    pub title: String,
    pub body: String,
}

#[derive(Debug)]
pub struct Batch {
    started: Instant,
    last_event: Instant,
    /// Total belum dibaca sebelum email baru (None bila tidak terbaca).
    baseline: Option<i64>,
    /// Total belum dibaca terbaru selama batch.
    latest: Option<i64>,
    counted: bool,
    sound: bool,
    notice: Option<Notice>,
}

impl Batch {
    fn new(now: Instant, baseline: Option<i64>, latest: Option<i64>) -> Self {
        Self {
            started: now,
            last_event: now,
            baseline,
            latest,
            counted: false,
            sound: false,
            notice: None,
        }
    }

    pub fn due_at(&self) -> Instant {
        let quiet = if self.counted { QUIET_COUNTED } else { QUIET_SIGNAL };
        (self.last_event + quiet).min(self.started + MAX_BATCH)
    }

    pub fn outlook_played_sound(&self) -> bool {
        self.sound
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Alert {
    /// Email baru. `new` None = jumlahnya tidak bisa dihitung.
    Mail {
        new: Option<i64>,
        total: Option<i64>,
        notice: Option<Notice>,
    },
    /// Notifikasi Outlook tanpa kenaikan jumlah belum dibaca (biasanya pengingat rapat).
    Other { notice: Notice, total: Option<i64> },
}

#[derive(Debug, Default)]
pub struct Tracker {
    batch: Option<Batch>,
    announced: Option<(i64, Instant)>,
    last_mail: Option<Instant>,
}

impl Tracker {
    pub fn batch(&self) -> Option<&Batch> {
        self.batch.as_ref()
    }

    fn announced_total(&self, now: Instant) -> Option<i64> {
        self.announced
            .filter(|(_, t)| now.duration_since(*t) < ANNOUNCED_TTL)
            .map(|(n, _)| n)
    }

    /// Jumlah belum dibaca berubah dari `prev` ke `total`.
    pub fn on_count(&mut self, prev: Option<i64>, total: i64, now: Instant) {
        let announced = self.announced_total(now);
        if announced.is_none_or(|a| total >= a) {
            self.announced = None;
        }
        if let Some(b) = self.batch.as_mut() {
            b.latest = Some(total);
            if b.baseline.is_some_and(|base| total > base) {
                b.counted = true;
                b.last_event = now;
            }
            return;
        }
        let Some(p) = prev else { return };
        // Angka yang menyusul toast sebelumnya (sudah diumumkan) bukan email baru lagi.
        let base = announced.map_or(p, |a| a.max(p));
        if total > base {
            let mut b = Batch::new(now, Some(base), Some(total));
            b.counted = true;
            self.batch = Some(b);
        }
    }

    fn echo(&self, now: Instant) -> bool {
        self.batch.is_none() && self.last_mail.is_some_and(|t| now.duration_since(t) < ECHO)
    }

    /// Outlook membunyikan suara email masuk.
    pub fn on_sound(&mut self, current: Option<i64>, now: Instant) {
        if self.echo(now) {
            return;
        }
        let b = self
            .batch
            .get_or_insert_with(|| Batch::new(now, current, current));
        b.sound = true;
        b.last_event = now;
    }

    /// Notifikasi bawaan Outlook (email baru atau pengingat).
    pub fn on_notice(&mut self, notice: Notice, current: Option<i64>, now: Instant) {
        if self.echo(now) {
            return;
        }
        let b = self
            .batch
            .get_or_insert_with(|| Batch::new(now, current, current));
        if b.notice.is_none() {
            b.notice = Some(notice);
        }
        b.last_event = now;
    }

    /// Tutup batch bila waktunya. Mengembalikan peringatan yang harus ditampilkan.
    pub fn take_due(&mut self, now: Instant) -> Option<Alert> {
        if now < self.batch.as_ref()?.due_at() {
            return None;
        }
        let b = self.batch.take()?;
        let alert = evaluate(&b);
        if let Some(Alert::Mail { total, .. }) = &alert {
            self.last_mail = Some(now);
            if let Some(t) = total {
                self.announced = Some((*t, now));
            }
        }
        alert
    }

    /// Buang batch tanpa menampilkan (mis. pengguna sedang melihat jendela).
    pub fn discard(&mut self) {
        self.batch = None;
    }
}

fn evaluate(b: &Batch) -> Option<Alert> {
    let by_count = match (b.baseline, b.latest) {
        (Some(base), Some(now)) if now > base => Some(now - base),
        _ => None,
    };
    if let Some(n) = by_count {
        return Some(Alert::Mail {
            new: Some(n),
            total: b.latest,
            notice: b.notice.clone(),
        });
    }
    if let Some(notice) = &b.notice {
        // Notifikasi tanpa kenaikan angka: kemungkinan pengingat rapat.
        // Kecuali jumlah tidak terbaca sama sekali: anggap email.
        if b.latest.is_some() {
            return Some(Alert::Other {
                notice: notice.clone(),
                total: b.latest,
            });
        }
        return Some(Alert::Mail {
            new: None,
            total: None,
            notice: Some(notice.clone()),
        });
    }
    if b.sound {
        // Suara email masuk tapi angka belum naik: minimal satu email baru.
        return Some(match (b.baseline, b.latest) {
            (Some(base), latest) => Alert::Mail {
                new: Some(1),
                total: Some(latest.unwrap_or(base).max(base) + 1),
                notice: None,
            },
            (None, latest) => Alert::Mail {
                new: None,
                total: latest,
                notice: None,
            },
        });
    }
    None
}

/// Teks toast: (judul, baris 1, baris 2).
pub fn texts(alert: &Alert, show_details: bool) -> (String, String, String) {
    let unread_line = |t: Option<i64>| match t {
        Some(n) => format!("Belum dibaca di Inbox: {n}"),
        None => String::new(),
    };
    match alert {
        Alert::Mail { new, total, notice } => {
            let detail = notice
                .as_ref()
                .filter(|_| show_details && new.is_none_or(|n| n == 1))
                .filter(|n| !n.title.trim().is_empty());
            if let Some(n) = detail {
                return (
                    clip(&format!("Dari {}", n.title.trim())),
                    clip(first_line(&n.body)),
                    unread_line(*total),
                );
            }
            let title = match new {
                Some(n) => format!("{n} email baru"),
                None => "Email baru".to_string(),
            };
            match total {
                Some(_) => (title, unread_line(*total), String::new()),
                None => (title, "Klik untuk membuka Outlook.".into(), String::new()),
            }
        }
        Alert::Other { notice, total } => {
            if show_details && !notice.title.trim().is_empty() {
                (
                    clip(notice.title.trim()),
                    clip(first_line(&notice.body)),
                    unread_line(*total),
                )
            } else {
                (
                    "Outlook".into(),
                    "Pengingat rapat atau pemberitahuan Outlook.".into(),
                    unread_line(*total),
                )
            }
        }
    }
}

fn first_line(s: &str) -> &str {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
}

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_LINE {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(MAX_LINE - 1).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    fn after(t: Instant, ms: u64) -> Instant {
        t + Duration::from_millis(ms)
    }

    fn notice() -> Notice {
        Notice {
            id: 7,
            title: "Budi".into(),
            body: "Slip gaji Oktober\nisi pratinjau".into(),
        }
    }

    #[test]
    fn beberapa_email_beruntun_jadi_satu_toast() {
        let t = t0();
        let mut tr = Tracker::default();
        tr.on_count(Some(12), 13, t);
        tr.on_sound(Some(13), after(t, 300));
        tr.on_count(Some(13), 14, after(t, 1500));
        tr.on_count(Some(14), 15, after(t, 3000));
        assert_eq!(tr.take_due(after(t, 4000)), None);
        let a = tr.take_due(after(t, 5600)).unwrap();
        assert_eq!(
            a,
            Alert::Mail { new: Some(3), total: Some(15), notice: None }
        );
        assert_eq!(
            texts(&a, false),
            ("3 email baru".into(), "Belum dibaca di Inbox: 15".into(), String::new())
        );
    }

    #[test]
    fn suara_dulu_angka_menyusul() {
        let t = t0();
        let mut tr = Tracker::default();
        tr.on_sound(Some(12), t);
        tr.on_count(Some(12), 13, after(t, 2000));
        let a = tr.take_due(after(t, 4600)).unwrap();
        assert_eq!(a, Alert::Mail { new: Some(1), total: Some(13), notice: None });
    }

    #[test]
    fn suara_tanpa_kenaikan_dianggap_satu_email() {
        let t = t0();
        let mut tr = Tracker::default();
        tr.on_sound(Some(12), t);
        let a = tr.take_due(after(t, 6100)).unwrap();
        assert_eq!(a, Alert::Mail { new: Some(1), total: Some(13), notice: None });
        // Angka asli menyusul: bukan email baru lagi.
        tr.on_count(Some(12), 13, after(t, 9000));
        assert!(tr.batch().is_none());
        // Email berikutnya tetap terdeteksi.
        tr.on_count(Some(13), 14, after(t, 20_000));
        let a = tr.take_due(after(t, 23_000)).unwrap();
        assert_eq!(a, Alert::Mail { new: Some(1), total: Some(14), notice: None });
    }

    #[test]
    fn jumlah_tidak_terbaca() {
        let t = t0();
        let mut tr = Tracker::default();
        tr.on_sound(None, t);
        let a = tr.take_due(after(t, 6100)).unwrap();
        assert_eq!(a, Alert::Mail { new: None, total: None, notice: None });
        let (title, l1, l2) = texts(&a, false);
        assert_eq!(title, "Email baru");
        assert!(!l1.contains("Belum dibaca"));
        assert!(l2.is_empty());
    }

    #[test]
    fn angka_turun_bukan_email_baru() {
        let t = t0();
        let mut tr = Tracker::default();
        tr.on_count(Some(12), 10, t);
        assert!(tr.batch().is_none());
        assert_eq!(tr.take_due(after(t, 20_000)), None);
    }

    #[test]
    fn notifikasi_tanpa_kenaikan_adalah_pengingat() {
        let t = t0();
        let mut tr = Tracker::default();
        tr.on_notice(notice(), Some(5), t);
        let a = tr.take_due(after(t, 6100)).unwrap();
        assert!(matches!(a, Alert::Other { total: Some(5), .. }));
        assert_eq!(texts(&a, false).0, "Outlook");
    }

    #[test]
    fn detail_pengirim_bila_diaktifkan() {
        let t = t0();
        let mut tr = Tracker::default();
        tr.on_notice(notice(), Some(4), t);
        tr.on_count(Some(4), 5, after(t, 800));
        let a = tr.take_due(after(t, 3400)).unwrap();
        assert_eq!(
            texts(&a, true),
            (
                "Dari Budi".into(),
                "Slip gaji Oktober".into(),
                "Belum dibaca di Inbox: 5".into()
            )
        );
        assert_eq!(texts(&a, false).0, "1 email baru");
    }

    #[test]
    fn gema_suara_setelah_toast_diabaikan() {
        let t = t0();
        let mut tr = Tracker::default();
        tr.on_count(Some(2), 3, t);
        assert!(tr.take_due(after(t, 2600)).is_some());
        tr.on_sound(Some(3), after(t, 3000));
        tr.on_notice(notice(), Some(3), after(t, 3500));
        assert!(tr.batch().is_none());
    }

    #[test]
    fn batas_lama_batch() {
        let t = t0();
        let mut tr = Tracker::default();
        let mut n = 0;
        for i in 0..10 {
            tr.on_count(Some(n), n + 1, after(t, i * 2000));
            n += 1;
        }
        assert!(tr.take_due(after(t, 12_000)).is_some());
    }
}
