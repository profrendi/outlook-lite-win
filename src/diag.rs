//! Diagnostik otomatis: RAM dan CPU per jenis proses, dicatat berkala ke CSV,
//! lalu diringkas menjadi satu file laporan yang bisa dikirim ke pengembang.
//!
//! File: `%LOCALAPPDATA%\OutlookLite\diagnostik.csv` (dirotasi ke diagnostik.old.csv)
//! dan `laporan-diagnostik.txt` (dibuat dari menu).

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Jenis proses (menurut WebView2). Proses yang tidak dikenali WebView2 (mis. Crashpad) = Lain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Aplikasi,
    Browser,
    Renderer,
    Gpu,
    Utility,
    Lain,
}

#[derive(Clone, Debug, Default)]
pub struct Proc {
    pub pid: u32,
    pub kind: Option<Kind>,
    /// Private working set (kolom Memory di Task Manager), byte.
    pub working: u64,
    /// Private bytes / commit, byte.
    pub commit: u64,
    /// Total waktu CPU (kernel + user) sejak proses mulai, satuan 100 ns.
    pub cpu_time: u64,
    /// Total page fault sejak proses mulai (halaman memori yang harus diambil lagi).
    pub page_faults: u64,
}

/// Keadaan aplikasi saat sampel diambil.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum State {
    /// Jendela terlihat dan difokus.
    Dipakai,
    /// Jendela terlihat tapi tidak difokus.
    TakFokus,
    Minimize,
    Tray,
    /// Tidak ada Outlook yang hidup (di luar jam kerja).
    Jeda,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Dipakai => "dipakai",
            State::TakFokus => "tak-fokus",
            State::Minimize => "minimize",
            State::Tray => "tray",
            State::Jeda => "jeda",
        }
    }
}

pub const CSV_HEADER: &str = "waktu,keadaan,proses,ram_mb,commit_mb,cpu_pct,\
app_mb,browser_mb,renderer_utama_mb,renderer_lain_mb,gpu_mb,utility_mb,lain_mb,\
app_cpu,browser_cpu,renderer_cpu,gpu_cpu,utility_cpu,lain_cpu,belum_dibaca,page_fault_per_mnt,catatan";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub time: String,
    pub state: String,
    pub procs: u32,
    pub ram_mb: f64,
    pub commit_mb: f64,
    pub cpu: f64,
    pub app_mb: f64,
    pub browser_mb: f64,
    pub renderer_main_mb: f64,
    pub renderer_other_mb: f64,
    pub gpu_mb: f64,
    pub utility_mb: f64,
    pub other_mb: f64,
    pub app_cpu: f64,
    pub browser_cpu: f64,
    pub renderer_cpu: f64,
    pub gpu_cpu: f64,
    pub utility_cpu: f64,
    pub other_cpu: f64,
    pub unread: Option<i64>,
    /// Page fault per menit (semua proses). Tinggi = memori sering diambil kembali setelah dirapikan.
    pub pf_per_min: f64,
    pub note: String,
}

fn mb(b: u64) -> f64 {
    (b as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0
}

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Menghitung CPU% dari selisih waktu CPU antar sampel.
pub struct Sampler {
    prev: HashMap<u32, u64>,
    prev_pf: HashMap<u32, u64>,
    prev_at: Option<Instant>,
    cpus: f64,
}

impl Sampler {
    pub fn new(cpus: usize) -> Self {
        Self {
            prev: HashMap::new(),
            prev_pf: HashMap::new(),
            prev_at: None,
            cpus: cpus.max(1) as f64,
        }
    }

    /// Ubah satu snapshot proses menjadi baris CSV. CPU% = persen dari seluruh CPU
    /// (sama seperti Task Manager); sampel pertama CPU-nya 0.
    pub fn row(
        &mut self,
        now: Instant,
        time: String,
        state: State,
        procs: &[Proc],
        unread: Option<i64>,
        note: &str,
    ) -> Row {
        let elapsed_100ns = self
            .prev_at
            .map(|t| now.duration_since(t).as_nanos() as f64 / 100.0)
            .unwrap_or(0.0);
        let pct = |p: &Proc, prev: &HashMap<u32, u64>| -> f64 {
            match prev.get(&p.pid) {
                Some(&before) if elapsed_100ns > 0.0 && p.cpu_time >= before => {
                    (p.cpu_time - before) as f64 / elapsed_100ns / self.cpus * 100.0
                }
                _ => 0.0,
            }
        };

        let mut row = Row {
            time,
            state: state.label().to_string(),
            procs: procs.len() as u32,
            unread,
            note: note.replace([',', '\n', '\r'], ";"),
            ..Default::default()
        };
        // Renderer terbesar = halaman Outlook; sisanya service worker / iframe.
        let main_renderer = procs
            .iter()
            .filter(|p| p.kind == Some(Kind::Renderer))
            .max_by_key(|p| p.working)
            .map(|p| p.pid);

        let (mut ws, mut commit) = (0u64, 0u64);
        for p in procs {
            let c = pct(p, &self.prev);
            ws += p.working;
            commit += p.commit;
            row.cpu += c;
            let m = mb(p.working);
            match p.kind.unwrap_or(Kind::Lain) {
                Kind::Aplikasi => {
                    row.app_mb += m;
                    row.app_cpu += c;
                }
                Kind::Browser => {
                    row.browser_mb += m;
                    row.browser_cpu += c;
                }
                Kind::Renderer => {
                    if Some(p.pid) == main_renderer {
                        row.renderer_main_mb += m;
                    } else {
                        row.renderer_other_mb += m;
                    }
                    row.renderer_cpu += c;
                }
                Kind::Gpu => {
                    row.gpu_mb += m;
                    row.gpu_cpu += c;
                }
                Kind::Utility => {
                    row.utility_mb += m;
                    row.utility_cpu += c;
                }
                Kind::Lain => {
                    row.other_mb += m;
                    row.other_cpu += c;
                }
            }
        }
        // Page fault per menit: hanya proses yang juga ada di sampel sebelumnya.
        if elapsed_100ns > 0.0 {
            let delta: u64 = procs
                .iter()
                .filter_map(|p| {
                    self.prev_pf
                        .get(&p.pid)
                        .map(|&b| p.page_faults.saturating_sub(b))
                })
                .sum();
            let minutes = elapsed_100ns / 600_000_000.0;
            row.pf_per_min = (delta as f64 / minutes).round();
        }
        row.ram_mb = mb(ws);
        row.commit_mb = mb(commit);
        for v in [
            &mut row.cpu,
            &mut row.app_mb,
            &mut row.browser_mb,
            &mut row.renderer_main_mb,
            &mut row.renderer_other_mb,
            &mut row.gpu_mb,
            &mut row.utility_mb,
            &mut row.other_mb,
            &mut row.app_cpu,
            &mut row.browser_cpu,
            &mut row.renderer_cpu,
            &mut row.gpu_cpu,
            &mut row.utility_cpu,
            &mut row.other_cpu,
        ] {
            *v = r1(*v);
        }

        self.prev = procs.iter().map(|p| (p.pid, p.cpu_time)).collect();
        self.prev_pf = procs.iter().map(|p| (p.pid, p.page_faults)).collect();
        self.prev_at = Some(now);
        row
    }
}

impl Row {
    pub fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            self.time,
            self.state,
            self.procs,
            self.ram_mb,
            self.commit_mb,
            self.cpu,
            self.app_mb,
            self.browser_mb,
            self.renderer_main_mb,
            self.renderer_other_mb,
            self.gpu_mb,
            self.utility_mb,
            self.other_mb,
            self.app_cpu,
            self.browser_cpu,
            self.renderer_cpu,
            self.gpu_cpu,
            self.utility_cpu,
            self.other_cpu,
            self.unread.map(|n| n.to_string()).unwrap_or_default(),
            self.pf_per_min,
            self.note
        )
    }

    pub fn from_csv(line: &str) -> Option<Row> {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 21 || f[0] == "waktu" {
            return None;
        }
        let n = |i: usize| f[i].trim().parse::<f64>().unwrap_or(0.0);
        Some(Row {
            time: f[0].to_string(),
            state: f[1].to_string(),
            procs: f[2].parse().unwrap_or(0),
            ram_mb: n(3),
            commit_mb: n(4),
            cpu: n(5),
            app_mb: n(6),
            browser_mb: n(7),
            renderer_main_mb: n(8),
            renderer_other_mb: n(9),
            gpu_mb: n(10),
            utility_mb: n(11),
            other_mb: n(12),
            app_cpu: n(13),
            browser_cpu: n(14),
            renderer_cpu: n(15),
            gpu_cpu: n(16),
            utility_cpu: n(17),
            other_cpu: n(18),
            unread: f[19].trim().parse().ok(),
            // Format lama (v0.1.6-v0.1.10) tanpa kolom page fault: 21 kolom.
            pf_per_min: if f.len() >= 22 { n(20) } else { 0.0 },
            note: if f.len() >= 22 { f[21..].join(",") } else { f[20..].join(",") },
        })
    }
}

// ----- domain yang dihubungi WebView2 -----

static DOMAINS_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static DOMAINS: std::sync::Mutex<BTreeMap<String, (u64, u64)>> =
    std::sync::Mutex::new(BTreeMap::new());

pub fn set_domains_enabled(on: bool) {
    DOMAINS_ON.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn domains_enabled() -> bool {
    DOMAINS_ON.load(std::sync::atomic::Ordering::Relaxed)
}

/// Catat satu permintaan ke `host` (diblokir atau tidak). Hanya nama domain, tanpa alamat lengkap.
pub fn record_domain(host: &str, blocked: bool) {
    if let Ok(mut m) = DOMAINS.lock() {
        let e = m.entry(host.to_ascii_lowercase()).or_insert((0, 0));
        if blocked {
            e.1 += 1;
        } else {
            e.0 += 1;
        }
    }
}

fn domains_path(dir: &Path) -> PathBuf {
    dir.join("diagnostik-domain.txt")
}

/// Gabungkan hitungan di memori ke file (kumulatif antar-sesi), lalu kosongkan memori.
pub fn flush_domains(dir: &Path) {
    let fresh = match DOMAINS.lock() {
        Ok(mut m) => std::mem::take(&mut *m),
        Err(_) => return,
    };
    if fresh.is_empty() {
        return;
    }
    let mut all = load_domains(dir);
    for (h, (n, b)) in fresh {
        let e = all.entry(h).or_insert((0, 0));
        e.0 += n;
        e.1 += b;
    }
    let mut out = String::new();
    for (h, (n, b)) in &all {
        let _ = writeln!(out, "{h}\t{n}\t{b}");
    }
    let _ = std::fs::write(domains_path(dir), out);
}

pub fn load_domains(dir: &Path) -> BTreeMap<String, (u64, u64)> {
    let mut m = BTreeMap::new();
    if let Ok(s) = std::fs::read_to_string(domains_path(dir)) {
        for line in s.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() == 3 {
                m.insert(
                    f[0].to_string(),
                    (f[1].parse().unwrap_or(0), f[2].parse().unwrap_or(0)),
                );
            }
        }
    }
    m
}

/// Tabel domain untuk laporan, diurutkan dari yang paling sering.
pub fn domains_report(dir: &Path) -> String {
    let all = load_domains(dir);
    if all.is_empty() {
        return "Belum ada data domain (aktifkan Catat diagnostik).\n".into();
    }
    let mut v: Vec<(String, u64, u64)> = all.into_iter().map(|(h, (n, b))| (h, n, b)).collect();
    v.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)).then(a.0.cmp(&b.0)));
    let mut out = String::new();
    let _ = writeln!(out, "{:<48} {:>10} {:>10}", "domain", "dimuat", "diblokir");
    for (h, n, b) in v {
        let _ = writeln!(out, "{h:<48} {n:>10} {b:>10}");
    }
    out
}

pub fn csv_path(dir: &Path) -> PathBuf {
    dir.join("diagnostik.csv")
}

const CSV_MAX: u64 = 2 * 1024 * 1024;

pub fn append_row(dir: &Path, row: &Row) {
    let p = csv_path(dir);
    if std::fs::metadata(&p).map(|m| m.len() > CSV_MAX).unwrap_or(false) {
        let _ = std::fs::rename(&p, dir.join("diagnostik.old.csv"));
    }
    let new = !p.exists();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        if new {
            let _ = writeln!(f, "{CSV_HEADER}");
        }
        let _ = writeln!(f, "{}", row.to_csv());
    }
}

/// Baca semua baris (file lama dulu, lalu yang sekarang).
pub fn read_rows(dir: &Path) -> Vec<Row> {
    let mut rows = Vec::new();
    for name in ["diagnostik.old.csv", "diagnostik.csv"] {
        if let Ok(s) = std::fs::read_to_string(dir.join(name)) {
            rows.extend(s.lines().filter_map(Row::from_csv));
        }
    }
    rows
}

#[derive(Default)]
struct Stat {
    n: usize,
    sum: f64,
    min: f64,
    max: f64,
}

impl Stat {
    fn add(&mut self, v: f64) {
        if self.n == 0 || v < self.min {
            self.min = v;
        }
        if self.n == 0 || v > self.max {
            self.max = v;
        }
        self.n += 1;
        self.sum += v;
    }
    fn avg(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.sum / self.n as f64
        }
    }
}

/// Ringkasan per keadaan: RAM, commit, CPU, dan rincian proses terbesar.
pub fn summarize(rows: &[Row]) -> String {
    const ORDER: [&str; 5] = ["dipakai", "tak-fokus", "minimize", "tray", "jeda"];
    let mut groups: BTreeMap<usize, Vec<&Row>> = BTreeMap::new();
    for r in rows {
        let i = ORDER.iter().position(|s| *s == r.state).unwrap_or(ORDER.len());
        groups.entry(i).or_default().push(r);
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<10} {:>6} | {:>17} | {:>8} | {:>8} | {:>17} | {:>8} | {:>6} | {:>8} | {:>9}",
        "keadaan", "sampel", "RAM MB rata (min-maks)", "commit", "CPU % rt", "renderer utama", "rend.lain", "GPU", "CPU% maks", "PF/mnt rt"
    );
    for (i, rs) in groups {
        let name = ORDER.get(i).copied().unwrap_or("lain");
        let (mut ram, mut commit, mut cpu, mut rend, mut other, mut gpu, mut pf) = (
            Stat::default(),
            Stat::default(),
            Stat::default(),
            Stat::default(),
            Stat::default(),
            Stat::default(),
            Stat::default(),
        );
        for r in &rs {
            ram.add(r.ram_mb);
            commit.add(r.commit_mb);
            cpu.add(r.cpu);
            rend.add(r.renderer_main_mb);
            other.add(r.renderer_other_mb);
            gpu.add(r.gpu_mb);
            pf.add(r.pf_per_min);
        }
        let _ = writeln!(
            out,
            "{:<10} {:>6} | {:>6.0} ({:>4.0}-{:>4.0}) | {:>8.0} | {:>8.1} | {:>6.0} ({:>4.0}-{:>4.0}) | {:>8.0} | {:>6.0} | {:>8.1} | {:>9.0}",
            name,
            ram.n,
            ram.avg(),
            ram.min,
            ram.max,
            commit.avg(),
            cpu.avg(),
            rend.avg(),
            rend.min,
            rend.max,
            other.avg(),
            gpu.avg(),
            cpu.max,
            pf.avg()
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn procs(cpu: u64) -> Vec<Proc> {
        vec![
            Proc { pid: 1, kind: Some(Kind::Aplikasi), working: 5 << 20, commit: 6 << 20, cpu_time: cpu, page_faults: 0 },
            Proc { pid: 2, kind: Some(Kind::Browser), working: 40 << 20, commit: 50 << 20, cpu_time: 0, page_faults: 0 },
            Proc { pid: 3, kind: Some(Kind::Renderer), working: 300 << 20, commit: 400 << 20, cpu_time: cpu * 2, page_faults: 0 },
            Proc { pid: 4, kind: Some(Kind::Renderer), working: 30 << 20, commit: 35 << 20, cpu_time: 0, page_faults: 0 },
            Proc { pid: 5, kind: Some(Kind::Gpu), working: 13 << 20, commit: 20 << 20, cpu_time: 0, page_faults: 0 },
            Proc { pid: 6, kind: None, working: 2 << 20, commit: 2 << 20, cpu_time: 0, page_faults: 0 },
        ]
    }

    #[test]
    fn baris_dan_cpu() {
        let mut s = Sampler::new(4);
        let t = Instant::now();
        let r0 = s.row(t, "a".into(), State::Tray, &procs(0), Some(3), "");
        assert_eq!(r0.cpu, 0.0);
        assert_eq!(r0.ram_mb, 390.0);
        assert_eq!(r0.renderer_main_mb, 300.0);
        assert_eq!(r0.renderer_other_mb, 30.0);
        assert_eq!(r0.other_mb, 2.0);
        // 1 detik kemudian: aplikasi pakai 0,1 dtk CPU, renderer 0,2 dtk, dari 4 CPU.
        let r1 = s.row(
            t + Duration::from_secs(1),
            "b".into(),
            State::Dipakai,
            &procs(1_000_000),
            None,
            "rapikan, uji",
        );
        assert_eq!(r1.app_cpu, 2.5);
        assert_eq!(r1.renderer_cpu, 5.0);
        assert_eq!(r1.cpu, 7.5);
        let line = r1.to_csv();
        assert_eq!(line.split(',').count(), CSV_HEADER.split(',').count());
        assert_eq!(Row::from_csv(&line).unwrap(), r1);
    }

    #[test]
    fn page_fault_dan_format_lama() {
        let mut s = Sampler::new(1);
        let t = Instant::now();
        let mut p = procs(0);
        s.row(t, "a".into(), State::Tray, &p, None, "");
        for x in &mut p {
            x.page_faults += 100; // 6 proses x 100 = 600 dalam 30 detik = 1200/menit
        }
        let r = s.row(t + Duration::from_secs(30), "b".into(), State::Tray, &p, Some(4), "x");
        assert_eq!(r.pf_per_min, 1200.0);
        assert_eq!(Row::from_csv(&r.to_csv()).unwrap(), r);
        // Baris versi lama (21 kolom) tetap terbaca.
        let old = "2026-11-10 18:54:04,tray,8,26.3,488.9,0.2,1.8,6.7,10.7,0,1.6,5.5,0,0.2,0,0,0,0,0,5,laporan dibuat";
        let o = Row::from_csv(old).unwrap();
        assert_eq!(o.unread, Some(5));
        assert_eq!(o.pf_per_min, 0.0);
        assert_eq!(o.note, "laporan dibuat");
    }

    #[test]
    fn catatan_domain() {
        let dir = std::env::temp_dir().join(format!("olite-dom-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        record_domain("Outlook.Cloud.Microsoft", false);
        record_domain("outlook.cloud.microsoft", false);
        record_domain("browser.events.data.microsoft.com", true);
        flush_domains(&dir);
        record_domain("outlook.cloud.microsoft", false);
        flush_domains(&dir);
        let m = load_domains(&dir);
        assert_eq!(m["outlook.cloud.microsoft"], (3, 0));
        assert_eq!(m["browser.events.data.microsoft.com"], (0, 1));
        let rep = domains_report(&dir);
        assert!(rep.lines().nth(1).unwrap().starts_with("outlook.cloud.microsoft"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ringkasan() {
        let mut s = Sampler::new(2);
        let t = Instant::now();
        let rows: Vec<Row> = (0..4)
            .map(|i| {
                let st = if i < 2 { State::Dipakai } else { State::Tray };
                s.row(t + Duration::from_secs(i), format!("t{i}"), st, &procs(i * 100), None, "")
            })
            .collect();
        let sum = summarize(&rows);
        assert!(sum.contains("dipakai"));
        assert!(sum.contains("tray"));
        assert!(sum.lines().count() == 3);
    }
}
