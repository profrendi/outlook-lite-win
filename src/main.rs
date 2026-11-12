//! Outlook Lite — Outlook web (outlook.office.com) di tray Windows, tanpa hak admin.
//!
//! Satu "instance" Outlook (jendela + WebView2) yang bisa terlihat, tersembunyi, atau
//! tidak ada sama sekali:
//! - Siaga (jam kerja): Outlook tersembunyi selalu hidup; X = sembunyikan.
//! - Jeda (di luar jam kerja): Outlook hanya hidup saat jendela terbuka; X = hancurkan.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#![cfg_attr(not(windows), allow(dead_code))]

mod diag;
mod icon;
mod install;
mod mailalert;
mod netrecover;
mod settings;
mod update;
mod urlutil;
mod win;

use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{Datelike, Local, Timelike};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tao::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Icon as WindowIcon, Window, WindowBuilder, WindowId},
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    Icon as TrayImage, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use wry::{NewWindowResponse, PageLoadEvent, WebContext, WebView, WebViewBuilder};

use icon::Badge;
use settings::{Mode, Settings, WindowGeom};
use urlutil::{
    is_microsoft_url, is_outlook_mail_url, is_outlook_url, CALENDAR_URL, COMPOSE_URL, MAIL_URL,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const SETTINGS_HTML: &str = include_str!("ui/settings.html");
/// Ada selama aplikasi berjalan; tersisa saat start berikutnya = sesi lalu berhenti tidak normal.
const SESSION_MARKER: &str = "sesi-berjalan.txt";
const INSTANCE_PORT: u16 = 47193;
const MAGIC: &str = "OUTLOOKLITE1";
/// Fitur Chromium yang dimatikan. Tiga pertama bawaan wry (harus dipertahankan).
/// IntensiveWakeUpThrottling: halaman tersembunyi tidak diperlambat (notifikasi).
/// SpareRendererForSitePerProcess: tanpa proses renderer cadangan yang menganggur.
/// BackForwardCache: halaman lama tidak disimpan di memori.
const DISABLED_FEATURES: &str = "msWebOOUI,msPdfOOUI,msSmartScreenProtection,IntensiveWakeUpThrottling,SpareRendererForSitePerProcess,BackForwardCache,Translate,MediaRouter,OptimizationHints,AutofillServerCommunication";
/// Telemetri Outlook web/Office (pengukuran pemakaian ke Microsoft). Tidak dibutuhkan untuk
/// email; diblokir di dalam WebView2 supaya tidak ada kerja jaringan & CPU tambahan.
const TELEMETRY_HOSTS: &[&str] = &[
    "*.events.data.microsoft.com",
    "browser.pipe.aria.microsoft.com",
];
/// Tanpa isolasi situs, iframe Microsoft lintas-domain berbagi proses dengan Outlook
/// (lebih sedikit proses). Aman di sini karena WebView hanya memuat halaman Microsoft;
/// tautan lain dibuka di browser default.
const NO_SITE_ISOLATION: &str = "IsolateOrigins,site-per-process";
/// Izinkan Outlook membunyikan suara email masuk tanpa klik pengguna (seperti PWA).
const AUTOPLAY_ARGS: &str = "--autoplay-policy=no-user-gesture-required";
/// Jangan perlambat halaman tersembunyi (penting untuk notifikasi di mode Siaga).
const BG_ARGS: &str = "--disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";
const MONITOR_JS: &str = include_str!("js/monitor.js");
const INSPECT_JS: &str = include_str!("js/inspect.js");

const TICK: Duration = Duration::from_secs(5);
/// Rapikan memori sesaat setelah jendela disembunyikan/diminimalkan, lalu berkala.
const TRIM_DELAY: Duration = Duration::from_secs(20);
const TRIM_EVERY: Duration = Duration::from_secs(5 * 60);
/// Saat tersembunyi/diminimalkan, RAM diperiksa tiap ini; dirapikan bila melewati `trim_ws_mb`.
const TRIM_CHECK: Duration = Duration::from_secs(30);
/// Jeda minimum antar perapian otomatis (merapikan terlalu sering hanya memindah halaman bolak-balik).
const TRIM_MIN_GAP: Duration = Duration::from_secs(60);
/// Jendela terbuka tapi tidak difokus selama ini: WebView2 diminta memakai memori rendah.
const UNFOCUSED_LOW: Duration = Duration::from_secs(60);
/// Cek apakah Outlook tersembunyi perlu disegarkan (memori membengkak).
const REFRESH_CHECK_EVERY: Duration = Duration::from_secs(5 * 60);
/// Jangan segarkan bila jendela baru saja ditutup (pengguna mungkin membukanya lagi).
const REFRESH_MIN_HIDDEN: Duration = Duration::from_secs(10 * 60);
/// Setelah halaman disegarkan, angka belum dibaca sempat tidak stabil: jangan dianggap email baru.
const REFRESH_SETTLE: Duration = Duration::from_secs(90);
const BG_CHECK_TIMEOUT: Duration = Duration::from_secs(75);
const INSPECT_TIMEOUT: Duration = Duration::from_secs(90);
const CREATE_RETRY: Duration = Duration::from_secs(300);
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 3600);
const UPDATE_FIRST_DELAY: Duration = Duration::from_secs(90);
/// Sampel diagnostik (RAM & CPU per proses) tiap ini, bila diagnostik aktif.
/// Baca ulang angka Inbox sekali lagi setelah email masuk.
const FORCE_AGAIN: Duration = Duration::from_millis(2000);
const DIAG_EVERY: Duration = Duration::from_secs(30);
/// Sampel tambahan sesaat setelah keadaan jendela berubah.
const DIAG_AFTER_CHANGE: Duration = Duration::from_secs(15);
/// Halaman login yang bertahan lebih lama dari ini (bukan sekadar redirect SSO) = perlu login.
const LOGIN_GRACE: Duration = Duration::from_secs(120);
/// Peringatan "perlu login" paling sering sekali per jangka waktu ini.
const LOGIN_TOAST_COOLDOWN: Duration = Duration::from_secs(3 * 3600);

// ---------------------------------------------------------------------------
// Event

enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    HotKey(GlobalHotKeyEvent),
    Remote(RemoteCmd),
    Page(PageMsg),
    PageLoaded(String),
    ToastClick(Option<u64>),
    Update(update::Outcome, bool),
    /// Proses WebView2 mati/macet (jenis kegagalan WebView2).
    ProcessFailed(i32),
    /// Navigasi halaman utama selesai (berhasil, status error WebView2).
    NavDone(bool, i32),
    /// Pesan dari jendela Pengaturan.
    SettingsUi(String),
}

enum RemoteCmd {
    Show,
    Open(String),
    /// Diminta keluar oleh installer/uninstaller.
    Exit,
}

enum PageMsg {
    State {
        unread: i64,
        composing: bool,
        /// Cara angka dibaca (label, angka, tanpa-angka, tak-terbaca, ...), untuk diagnostik.
        how: String,
        /// Pembacaan yang diminta aplikasi (email masuk / Cek email sekarang).
        force: bool,
    },
    Notify { id: u64, title: String, body: String },
    Badge(i64),
    Sound,
    Inspect(String),
}

fn clean(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

fn parse_page_msg(body: &str) -> Option<PageMsg> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let s = |k: &str, max: usize| clean(v.get(k).and_then(|x| x.as_str()).unwrap_or(""), max);
    match v.get("t")?.as_str()? {
        "state" => Some(PageMsg::State {
            unread: v.get("unread")?.as_i64()?,
            composing: v.get("composing").and_then(|c| c.as_bool()).unwrap_or(false),
            how: s("how", 24),
            force: v.get("force").and_then(|c| c.as_bool()).unwrap_or(false),
        }),
        "notify" => Some(PageMsg::Notify {
            id: v.get("id")?.as_u64()?,
            title: s("title", 300),
            body: s("body", 600),
        }),
        "badge" => Some(PageMsg::Badge(v.get("count")?.as_i64()?.clamp(0, 1_000_000))),
        "sound" => Some(PageMsg::Sound),
        "inspect" => Some(PageMsg::Inspect(
            v.get("data")?.as_str()?.chars().take(300_000).collect(),
        )),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Argumen & single instance

struct Args {
    tray: bool,
    compose: bool,
    restarted: bool,
    portable: bool,
    uninstall: bool,
    mailto: Option<String>,
}

fn parse_args() -> Args {
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut a = Args {
        tray: false,
        compose: false,
        restarted: false,
        portable: false,
        uninstall: false,
        mailto: None,
    };
    let mut i = 0;
    while i < v.len() {
        match v[i].as_str() {
            "--tray" => a.tray = true,
            "--compose" => a.compose = true,
            "--restarted" => a.restarted = true,
            "--portable" => a.portable = true,
            "--uninstall" => a.uninstall = true,
            "--mailto" => {
                a.mailto = v.get(i + 1).cloned();
                i += 1;
            }
            other if other.to_ascii_lowercase().starts_with("mailto:") => {
                a.mailto = Some(other.to_string())
            }
            _ => {}
        }
        i += 1;
    }
    a
}

enum Lock {
    Primary(TcpListener),
    Secondary,
    None,
}

fn acquire_lock(cmd: Option<&str>, restarted: bool) -> Lock {
    let addr = SocketAddr::from(([127, 0, 0, 1], INSTANCE_PORT));
    let attempts = if restarted { 40 } else { 2 };
    for _ in 0..attempts {
        if let Ok(l) = TcpListener::bind(addr) {
            return Lock::Primary(l);
        }
        // Port terpakai: apakah itu Outlook Lite yang lain?
        let mut is_ours = false;
        if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(800)) {
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new(&s).read_line(&mut line).is_ok() && line.trim() == MAGIC {
                is_ours = true;
                if !restarted {
                    if let Some(c) = cmd {
                        let _ = s.write_all(format!("{c}\n").as_bytes());
                    }
                    return Lock::Secondary;
                }
            }
        }
        if !is_ours && !restarted {
            // Port dipakai aplikasi lain: jalan tanpa penjaga single-instance.
            return Lock::None;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Lock::None
}

/// Minta instance yang sedang berjalan untuk keluar, lalu tunggu sampai benar-benar keluar.
/// Mengembalikan true bila memang ada instance yang berjalan.
fn stop_running_instance() -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], INSTANCE_PORT));
    let mut asked = false;
    if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(800)) {
        let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
        let mut line = String::new();
        if BufReader::new(&s).read_line(&mut line).is_ok() && line.trim() == MAGIC {
            let _ = s.write_all(b"exit\n");
            asked = true;
        }
    }
    if asked {
        for _ in 0..50 {
            if TcpListener::bind(addr).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        // beri waktu proses WebView2 ikut berhenti
        std::thread::sleep(Duration::from_millis(800));
    }
    asked
}

fn run_uninstall(dir: &Path) {
    let exe = std::env::current_exe().unwrap_or_default();
    let installed = install::is_installed_copy(&exe);
    if !win::confirm(
        "Hapus Outlook Lite dari komputer ini?\n\nPengaturan dan data login Outlook Lite juga akan dihapus. Akun Outlook Anda sendiri tidak terpengaruh.",
    ) {
        return;
    }
    stop_running_instance();
    let _ = win::set_autostart(false, &exe);
    win::unregister_mailto();
    install::remove_integration();
    let install_dir = install::install_dir();
    if installed {
        win::delete_dirs_later(&[dir, &install_dir]);
    } else {
        // Salinan portable: jangan hapus folder tempat exe berada.
        win::delete_dirs_later(&[dir]);
    }
    win::info(if installed {
        "Outlook Lite sudah dihapus."
    } else {
        "Pengaturan dan data Outlook Lite sudah dihapus. File exe portable bisa Anda hapus sendiri."
    });
}

fn serve_lock(listener: TcpListener, proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut s) = conn else { continue };
            let _ = s.write_all(format!("{MAGIC}\n").as_bytes());
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new(&s).read_line(&mut line).is_err() {
                continue;
            }
            let line = line.trim();
            let cmd = if line == "show" {
                Some(RemoteCmd::Show)
            } else if line == "exit" {
                Some(RemoteCmd::Exit)
            } else if let Some(url) = line.strip_prefix("open ") {
                // Hanya URL compose Outlook yang diterima dari proses lain.
                url.starts_with(COMPOSE_URL)
                    .then(|| RemoteCmd::Open(url.to_string()))
            } else {
                None
            };
            if let Some(c) = cmd {
                let _ = proxy.send_event(UserEvent::Remote(c));
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Util

/// User agent Edge Android untuk eksperimen tampilan mobile di tray.
fn mobile_user_agent() -> String {
    let major = wry::webview_version()
        .ok()
        .and_then(|v| v.split('.').next().map(str::to_string))
        .unwrap_or_else(|| "140".into());
    format!(
        "Mozilla/5.0 (Linux; Android 14; K) AppleWebKit/537.36 (KHTML, like Gecko) \
         Chrome/{major}.0.0.0 Mobile Safari/537.36 EdgA/{major}.0.0.0"
    )
}

fn browser_args_for(s: &Settings, started_in_tray: bool) -> String {
    let mut features = DISABLED_FEATURES.to_string();
    let mut extra = String::new();
    let mut enable: Vec<&str> = Vec::new();
    if s.simulate_memory_pressure {
        // Flag WebView2: WebView yang tidak aktif (tersembunyi) diperlakukan seperti sedang
        // kekurangan memori, sehingga mesinnya sendiri membuang cache & menjalankan GC.
        enable.push("msWebView2SimulateMemoryPressureWhenInactive");
    }
    if !enable.is_empty() {
        extra.push_str(&format!(" --enable-features={}", enable.join(",")));
    }
    if s.system_proxy_resolver {
        // Proxy kantor dibaca lewat Windows (WinHTTP), sama seperti aplikasi Windows lain.
        extra.push_str(" --use-system-proxy-resolver");
    }
    if started_in_tray {
        // Dimulai langsung di tray (mulai dengan Windows): WebView2 diluncurkan sebagai latar.
        extra.push_str(" --edge-webview-is-background");
    }
    if !s.site_isolation {
        features.push(',');
        features.push_str(NO_SITE_ISOLATION);
        extra.push_str(" --disable-site-isolation-trials");
    }
    // Tanpa layanan jaringan latar Chromium (pembaruan komponen, dsb.).
    extra.push_str(" --disable-background-networking");
    if s.low_end_mode {
        // Mode perangkat RAM kecil Chromium: cache gambar/grafis lebih kecil.
        extra.push_str(" --enable-low-end-device-mode");
    }
    if s.v8_optimize_for_size {
        // V8 mengutamakan ukuran memori dibanding kecepatan (Outlook tetap lancar).
        extra.push_str(" --js-flags=--optimize-for-size");
    }
    format!("--disable-features={features} {AUTOPLAY_ARGS} {BG_ARGS}{extra}")
}

/// Argumen WebView2 lengkap (termasuk GPU & argumen tambahan) untuk pengaturan `s`.
fn full_browser_args(s: &Settings, started_in_tray: bool) -> String {
    let mut a = browser_args_for(s, started_in_tray);
    if s.no_gpu {
        a.push_str(" --disable-gpu");
    }
    if !s.extra_browser_args.trim().is_empty() {
        a.push(' ');
        a.push_str(s.extra_browser_args.trim());
    }
    a
}

fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("OutlookLite")
}

fn log(dir: &Path, msg: &str) {
    let p = dir.join("outlook-lite.log");
    if std::fs::metadata(&p).map(|m| m.len() > 256 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(&p, dir.join("outlook-lite.old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{} {msg}", Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}

fn now_parts() -> (u8, u32) {
    let n = Local::now();
    (
        n.weekday().number_from_monday() as u8,
        n.hour() * 60 + n.minute(),
    )
}

fn set_memory_low(webview: &WebView, low: bool) {
    #[cfg(windows)]
    {
        use wry::{MemoryUsageLevel, WebViewExtWindows};
        let _ = webview.set_memory_usage_level(if low {
            MemoryUsageLevel::Low
        } else {
            MemoryUsageLevel::Normal
        });
    }
    #[cfg(not(windows))]
    let _ = (webview, low);
}

fn tray_image(badge: Badge) -> Option<TrayImage> {
    TrayImage::from_rgba(icon::rgba(32, badge), 32, 32).ok()
}

fn window_image() -> Option<WindowIcon> {
    WindowIcon::from_rgba(icon::rgba(32, Badge::None), 32, 32).ok()
}

// ---------------------------------------------------------------------------
// Menu tray

mod id {
    pub const OPEN_MAIL: &str = "open_mail";
    pub const OPEN_CAL: &str = "open_cal";
    pub const COMPOSE: &str = "compose";
    pub const CHECK_NOW: &str = "check_now";
    pub const TEST_TOAST: &str = "test_toast";
    pub const TOAST_COMPAT: &str = "toast_compat";
    pub const SHORTCUT: &str = "shortcut:";
    pub const WORK_HOURS: &str = "work_hours";
    pub const SOUND: &str = "sound";
    pub const DETAILS: &str = "details";
    pub const AUTO_CLOSE: &str = "auto_close";
    pub const LOW_MEM: &str = "low_mem";
    pub const NO_GPU: &str = "no_gpu";
    pub const NO_SW: &str = "no_sw";
    pub const DIAG_REPORT: &str = "diag_report";
    pub const ZOOM_IN: &str = "zoom_in";
    pub const ZOOM_OUT: &str = "zoom_out";
    pub const ZOOM_RESET: &str = "zoom_reset";
    pub const AUTOSTART: &str = "autostart";
    pub const HOTKEYS: &str = "hotkeys";
    pub const MAILTO: &str = "mailto";
    pub const AUTO_UPDATE: &str = "auto_update";
    pub const CHECK_UPDATE: &str = "check_update";
    pub const DIAG_RAM: &str = "diag_ram";
    pub const INSPECT: &str = "inspect";
    pub const OPEN_FOLDER: &str = "open_folder";
    pub const OPEN_APP_FOLDER: &str = "open_app_folder";
    pub const UNINSTALL: &str = "uninstall";
    pub const RELOAD: &str = "reload";
    pub const RESTART_UPDATE: &str = "restart_update";
    pub const CLOSE_WINDOW: &str = "close_window";
    pub const QUIT: &str = "quit";
    pub const SETTINGS: &str = "settings";
}

struct TrayMenu {
    status: MenuItem,
    restart_update: MenuItem,
}

/// Menu tray ringkas; semua pengaturan ada di jendela Pengaturan.
fn build_menu(s: &Settings) -> Result<(Menu, TrayMenu), tray_icon::menu::Error> {
    let item = |i: &str, t: &str| MenuItem::with_id(i, t, true, None);
    let sep = PredefinedMenuItem::separator;

    let status = MenuItem::with_id("status", "Outlook Lite", false, None);
    let restart_update = MenuItem::with_id(id::RESTART_UPDATE, "Tidak ada update", false, None);

    let shortcut_items: Vec<MenuItem> = s
        .shortcuts
        .iter()
        .enumerate()
        .map(|(i, sc)| item(&format!("{}{i}", id::SHORTCUT), &sc.name))
        .collect();
    let shortcut_refs: Vec<&dyn tray_icon::menu::IsMenuItem> = shortcut_items
        .iter()
        .map(|c| c as &dyn tray_icon::menu::IsMenuItem)
        .collect();
    let shortcuts = Submenu::with_items("Pintasan", !shortcut_items.is_empty(), &shortcut_refs)?;

    let menu = Menu::with_items(&[
        &status,
        &sep(),
        &item(id::OPEN_MAIL, "Buka Email"),
        &item(id::OPEN_CAL, "Buka Kalender"),
        &item(id::COMPOSE, "Email baru"),
        &shortcuts,
        &item(id::CHECK_NOW, "Cek email sekarang"),
        &sep(),
        &item(id::SETTINGS, "Pengaturan…"),
        &sep(),
        &restart_update,
        &item(id::CLOSE_WINDOW, "Tutup jendela"),
        &item(id::QUIT, "Keluar"),
    ])?;

    Ok((menu, TrayMenu { status, restart_update }))
}

/// Kombinasi cadangan bila hotkey bawaan sudah dipakai aplikasi lain.
const TOGGLE_ALTS: [&str; 3] = ["Ctrl+Alt+O", "Ctrl+Shift+Alt+O", "Ctrl+Alt+K"];
const COMPOSE_ALTS: [&str; 3] = ["Ctrl+Alt+N", "Ctrl+Alt+M", "Ctrl+Shift+Alt+N"];

// ---------------------------------------------------------------------------
// Aplikasi

/// Jendela Pengaturan (WebView2 kecil berisi settings.html). Dihancurkan saat ditutup.
struct SettingsWin {
    window: Window,
    webview: WebView,
}

/// Opsi environment WebView2 saat aplikasi mulai. Semua WebView dalam satu sesi memakai opsi
/// yang sama (WebView2 menolak opsi berbeda selama prosesnya berjalan); perubahan berlaku
/// setelah restart.
#[derive(Clone, Copy, PartialEq)]
struct EnvOpts {
    custom: bool,
    tracking_prevention: bool,
    local_crash_reports: bool,
}

impl EnvOpts {
    fn of(s: &Settings) -> Self {
        Self {
            custom: s.custom_environment,
            tracking_prevention: s.tracking_prevention,
            local_crash_reports: s.local_crash_reports,
        }
    }
}

struct Instance {
    window: Window,
    webview: WebView,
    visible: bool,
    minimized: bool,
    suspended: bool,
    /// Outlook tersembunyi dimuat dengan tampilan mobile (eksperimen `tray_mobile_ui`).
    mobile: bool,
}

struct App {
    settings: Settings,
    dirty: bool,
    dir: PathBuf,
    exe: PathBuf,
    icon_png: Option<PathBuf>,
    proxy: EventLoopProxy<UserEvent>,
    web_context: WebContext,
    browser_args: String,

    tray: Option<TrayIcon>,
    menu: Option<TrayMenu>,
    badge: Badge,

    inst: Option<Instance>,
    focused: bool,
    unfocused_since: Option<Instant>,
    composing: bool,

    unread: Option<i64>,
    badge_unread: Option<i64>,
    last_sound: Option<Instant>,
    mail: mailalert::Tracker,
    last_login_toast: Option<Instant>,
    overlay_on: bool,
    needs_login: bool,
    login_toasted: bool,
    login_seen_at: Option<Instant>,
    last_tip: String,
    last_title: String,

    next_tick: Instant,
    started: Instant,
    last_mode: Mode,
    bg_deadline: Option<Instant>,
    manual_check: bool,
    inspect_pending: bool,
    inspect_sent: bool,
    create_failed_at: Option<Instant>,

    hk_manager: Option<GlobalHotKeyManager>,
    hk_registered: Vec<HotKey>,
    hk_toggle: Option<u32>,
    hk_compose: Option<u32>,

    last_row: Option<diag::Row>,
    unread_how: String,
    force_again: Option<Instant>,
    last_unread_note: String,
    sampler: diag::Sampler,
    next_sample: Option<Instant>,
    diag_notes: Vec<String>,
    diag_state: Option<diag::State>,
    trim_at: Option<Instant>,
    hidden_since: Option<Instant>,
    page_fresh_at: Option<Instant>,
    last_refresh_check: Option<Instant>,
    settle_until: Option<Instant>,
    low_unfocused: bool,
    next_trim_check: Option<Instant>,
    last_trim: Option<Instant>,
    background_mode: bool,
    net: netrecover::NetRecovery,
    env_logged: bool,
    env_opts: EnvOpts,
    started_tray: bool,
    settings_win: Option<SettingsWin>,
    /// Jendela dibuka dari tray/minimize: kapan, dan dari mana (untuk mengukur waktu buka).
    restore_started: Option<(Instant, &'static str)>,
    /// WebView2 dibuat ulang pada waktu ini (jendela terlihat?), setelah proses lama selesai.
    recreate_at: Option<(Instant, bool)>,

    update_running: bool,
    last_update_check: Option<Instant>,
    update_ready: Option<String>,

    initial_url: Option<String>,
    exiting: bool,
}

impl App {
    fn active_mode(&self) -> Mode {
        let (d, m) = now_parts();
        self.settings.active_mode(d, m)
    }

    fn user_looking(&self) -> bool {
        self.inst
            .as_ref()
            .is_some_and(|i| i.visible && !i.minimized && self.focused)
    }

    fn is_our_window(&self, wid: WindowId) -> bool {
        self.inst.as_ref().is_some_and(|i| i.window.id() == wid)
    }

    fn log(&self, msg: &str) {
        log(&self.dir, msg);
    }

    // ----- notifikasi -----

    fn toast(&self, title: &str, body: &str, click: Option<u64>, force_sound: Option<bool>) {
        self.toast3(title, body, "", click, force_sound);
    }

    fn toast3(&self, title: &str, body: &str, body2: &str, click: Option<u64>, force_sound: Option<bool>) {
        let proxy = self.proxy.clone();
        let cb: win::Callback = Arc::new(move || {
            let _ = proxy.send_event(UserEvent::ToastClick(click));
        });
        win::toast(
            &win::ToastSpec {
                title: title.to_string(),
                body: body.to_string(),
                body2: body2.to_string(),
                sound: force_sound.unwrap_or(self.settings.sound),
                compat: self.settings.toast_compat,
                on_click: Some(cb),
            },
            self.icon_png.as_deref(),
        );
    }

    // ----- tray -----

    fn status_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let active = self.active_mode();
        parts.push(match active {
            Mode::Siaga => "Siaga".into(),
            Mode::Jeda => "Jeda (di luar jam kerja)".into(),
        });
        if self.net.failing() {
            parts.push("tidak terhubung, mencoba lagi otomatis".into());
        } else if self.needs_login {
            parts.push("perlu login".into());
        } else if let Some(n) = self.shown_unread() {
            parts.push(if n > 0 {
                format!("{n} belum dibaca")
            } else {
                "tidak ada yang belum dibaca".into()
            });
        }
        if let Some(r) = &self.last_row {
            parts.push(format!("RAM {:.0} MB, CPU {:.1}% ({} proses)", r.ram_mb, r.cpu, r.procs));
        }
        parts.join(" • ")
    }

    fn refresh_tray(&mut self) {
        let badge = if self.needs_login {
            Badge::Warning
        } else if self.shown_unread().unwrap_or(0) > 0 {
            Badge::Unread
        } else {
            Badge::None
        };
        let status = self.status_text();
        let tip: String = format!("Outlook Lite — {status}").chars().take(120).collect();
        if let Some(t) = &self.tray {
            if badge != self.badge {
                let _ = t.set_icon(tray_image(badge));
            }
            if tip != self.last_tip {
                let _ = t.set_tooltip(Some(&tip));
                if let Some(m) = &self.menu {
                    m.status.set_text(&status);
                }
            }
        }
        self.badge = badge;
        self.last_tip = tip;
        self.push_settings_state();
        let has_unread = self.shown_unread().unwrap_or(0) > 0;
        if let Some(i) = &self.inst {
            // Tanda merah di ikon taskbar, seperti PWA Outlook.
            if has_unread != self.overlay_on {
                #[cfg(windows)]
                {
                    use tao::platform::windows::WindowExtWindows;
                    let img = WindowIcon::from_rgba(icon::dot(32), 32, 32).ok();
                    i.window.set_overlay_icon(if has_unread { img.as_ref() } else { None });
                }
                self.overlay_on = has_unread;
            }
            let n = self.shown_unread().unwrap_or(0);
            let title = if n > 0 {
                format!("Outlook Lite ({n})")
            } else {
                "Outlook Lite".into()
            };
            if title != self.last_title {
                i.window.set_title(&title);
                self.last_title = title;
            }
        } else {
            self.last_title.clear();
        }
    }

    fn sync_menu(&self) {
        let Some(m) = &self.menu else { return };
        match &self.update_ready {
            Some(v) => {
                m.restart_update
                    .set_text(format!("Restart untuk memakai versi baru (v{v})"));
                m.restart_update.set_enabled(true);
            }
            None => {
                m.restart_update.set_text("Tidak ada update tertunda");
                m.restart_update.set_enabled(false);
            }
        }
    }

    fn build_tray(&mut self) {
        match build_menu(&self.settings) {
            Ok((menu, handles)) => {
                if let Some(t) = &self.tray {
                    t.set_menu(Some(Box::new(menu)));
                } else {
                    match TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_menu_on_left_click(false)
                        .with_tooltip("Outlook Lite")
                        .with_icon(tray_image(Badge::None).expect("ikon tray"))
                        .build()
                    {
                        Ok(t) => self.tray = Some(t),
                        Err(e) => self.log(&format!("tray gagal: {e}")),
                    }
                }
                self.menu = Some(handles);
                self.badge = Badge::None;
                self.last_tip.clear();
            }
            Err(e) => self.log(&format!("menu gagal: {e}")),
        }
        self.sync_menu();
        self.refresh_tray();
    }

    // ----- hotkey -----

    fn apply_hotkeys(&mut self) {
        if let Some(m) = &self.hk_manager {
            for hk in self.hk_registered.drain(..) {
                let _ = m.unregister(hk);
            }
        }
        self.hk_toggle = None;
        self.hk_compose = None;
        if !self.settings.hotkeys {
            return;
        }
        if self.hk_manager.is_none() {
            match GlobalHotKeyManager::new() {
                Ok(m) => self.hk_manager = Some(m),
                Err(e) => {
                    self.log(&format!("hotkey manager gagal: {e}"));
                    return;
                }
            }
        }
        let Some(m) = &self.hk_manager else { return };
        let mut errors = Vec::new();
        let mut replaced = Vec::new();
        for is_toggle in [true, false] {
            let configured = if is_toggle {
                self.settings.hotkey_toggle.clone()
            } else {
                self.settings.hotkey_compose.clone()
            };
            if configured.trim().is_empty() {
                continue; // dimatikan oleh pengguna
            }
            // Coba kombinasi yang diatur dulu, lalu cadangan bila bentrok dengan aplikasi lain.
            let alts: &[&str] = if is_toggle { &TOGGLE_ALTS } else { &COMPOSE_ALTS };
            let mut candidates = vec![configured.clone()];
            candidates.extend(
                alts.iter()
                    .map(|a| a.to_string())
                    .filter(|a| !a.eq_ignore_ascii_case(&configured)),
            );
            let mut first_err = None;
            let mut got = None;
            for text in candidates {
                match text.parse::<HotKey>() {
                    Ok(hk) => match m.register(hk) {
                        Ok(()) => {
                            got = Some((text, hk));
                            break;
                        }
                        Err(e) => {
                            first_err.get_or_insert(format!("{text}: {e}"));
                        }
                    },
                    Err(e) => {
                        first_err.get_or_insert(format!("{text}: {e}"));
                    }
                }
            }
            match got {
                Some((text, hk)) => {
                    if is_toggle {
                        self.hk_toggle = Some(hk.id());
                    } else {
                        self.hk_compose = Some(hk.id());
                    }
                    self.hk_registered.push(hk);
                    if text != configured {
                        replaced.push(format!("{configured} → {text}"));
                        if is_toggle {
                            self.settings.hotkey_toggle = text;
                        } else {
                            self.settings.hotkey_compose = text;
                        }
                    }
                }
                None => errors.push(first_err.unwrap_or_else(|| configured.clone())),
            }
        }

        if !replaced.is_empty() {
            let msg = replaced.join(", ");
            self.log(&format!("hotkey bentrok, diganti: {msg}"));
            self.settings.hotkey_warned.clear();
            // Simpan segera supaya pemberitahuan ini tidak muncul lagi di start berikutnya.
            self.save();
            self.toast(
                "Hotkey diganti otomatis",
                &format!("Kombinasi lama sudah dipakai aplikasi lain. Sekarang: {msg}."),
                None,
                Some(false),
            );
        }
        if errors.is_empty() {
            if !self.settings.hotkey_warned.is_empty() {
                self.settings.hotkey_warned.clear();
                self.dirty = true;
            }
        } else {
            let msg = errors.join("; ");
            self.log(&format!("hotkey gagal: {msg}"));
            // Beri tahu sekali saja untuk kombinasi yang sama, bukan di setiap start.
            if self.settings.hotkey_warned != msg {
                self.settings.hotkey_warned = msg.clone();
                self.save();
                self.toast(
                    "Hotkey tidak bisa dipasang",
                    &format!(
                        "{msg}. Fitur lain tetap jalan. Ganti lewat Pengaturan > Buka folder pengaturan > settings.json, lalu Muat ulang pengaturan."
                    ),
                    None,
                    Some(false),
                );
            }
        }
    }

    // ----- instance Outlook -----

    fn valid_geom(&self, target: &EventLoopWindowTarget<UserEvent>) -> Option<WindowGeom> {
        let g = self.settings.window?;
        if g.width < 400 || g.height < 300 {
            return None;
        }
        let on_screen = target.available_monitors().any(|m| {
            let p = m.position();
            let s = m.size();
            let (l, t) = (p.x, p.y);
            let (r, b) = (p.x + s.width as i32, p.y + s.height as i32);
            // minimal sebagian bilah judul terlihat
            g.x + 100 < r && g.x + g.width as i32 - 100 > l && g.y >= t - 10 && g.y + 40 < b
        });
        on_screen.then_some(g)
    }

    fn create_instance(
        &mut self,
        target: &EventLoopWindowTarget<UserEvent>,
        url: &str,
        visible: bool,
    ) -> bool {
        let mut wb = WindowBuilder::new()
            .with_title("Outlook Lite")
            .with_window_icon(window_image())
            .with_visible(visible);
        wb = match self.valid_geom(target) {
            Some(g) => wb
                .with_position(PhysicalPosition::new(g.x, g.y))
                .with_inner_size(PhysicalSize::new(g.width, g.height))
                // Maksimalkan hanya jendela yang terlihat; di Windows maximize ikut menampilkan.
                .with_maximized(g.maximized && visible),
            None => wb.with_inner_size(LogicalSize::new(1280.0, 800.0)),
        };
        let window = match wb.build(target) {
            Ok(w) => w,
            Err(e) => {
                self.log(&format!("jendela gagal: {e}"));
                self.create_failed_at = Some(Instant::now());
                return false;
            }
        };

        let cfg = serde_json::json!({
            "sel": self.settings.unread_selector,
            "nosw": self.settings.block_service_worker,
        });
        let init_js = format!("window.__oliteCfg = {cfg};\n{MONITOR_JS}");
        let zoom = self.settings.zoom;
        let low = self.settings.low_memory;
        #[cfg(windows)]
        let args = self.browser_args.clone();
        // Environment WebView2 sendiri (dibuat sebelum builder meminjam web_context).
        #[cfg(windows)]
        let custom_env = self.webview_env();
        let proxy_ipc = self.proxy.clone();
        let proxy_load = self.proxy.clone();
        let mobile = !visible && self.settings.tray_mobile_ui;

        let builder = WebViewBuilder::new_with_web_context(&mut self.web_context)
            .with_url(url)
            .with_initialization_script(init_js)
            .with_ipc_handler(move |req| {
                // Hanya terima pesan dari halaman Outlook web.
                if !is_outlook_url(&req.uri().to_string()) {
                    return;
                }
                if let Some(m) = parse_page_msg(req.body()) {
                    let _ = proxy_ipc.send_event(UserEvent::Page(m));
                }
            })
            .with_on_page_load_handler(move |ev, url| {
                if matches!(ev, PageLoadEvent::Finished) {
                    let _ = proxy_load.send_event(UserEvent::PageLoaded(url));
                }
            })
            .with_new_window_req_handler(|url, _features| {
                if url.starts_with("about:") || is_microsoft_url(&url) {
                    // Popup login, compose pop-out, dsb.
                    NewWindowResponse::Allow
                } else {
                    // Tautan di email dibuka di browser default.
                    win::open_url(&url);
                    NewWindowResponse::Deny
                }
            });
        #[cfg(windows)]
        let builder = {
            use wry::WebViewBuilderExtWindows;
            match custom_env {
                Some(env) => builder.with_environment(env),
                None => builder.with_additional_browser_args(args),
            }
        };
        let builder = if mobile {
            builder.with_user_agent(mobile_user_agent())
        } else {
            builder
        };

        let webview = match builder.build(&window) {
            Ok(v) => v,
            Err(e) => {
                self.log(&format!(
                    "WebView2 gagal (WebView2 Runtime terpasang?): {e}"
                ));
                self.create_failed_at = Some(Instant::now());
                if visible {
                    self.toast(
                        "Outlook Lite tidak bisa membuka jendela",
                        "WebView2 Runtime mungkin belum terpasang. Lihat outlook-lite.log.",
                        None,
                        Some(false),
                    );
                }
                return false;
            }
        };
        let _ = webview.zoom(zoom);
        win::tune_webview(
            &webview,
            if self.settings.block_telemetry { TELEMETRY_HOSTS } else { &[] },
        );
        {
            let p = self.proxy.clone();
            win::on_process_failed(&webview, move |k| {
                let _ = p.send_event(UserEvent::ProcessFailed(k));
            });
            let p = self.proxy.clone();
            win::on_navigation_completed(&webview, move |ok, status| {
                let _ = p.send_event(UserEvent::NavDone(ok, status));
            });
        }
        if !visible {
            // Beri tahu WebView2 bahwa halaman tidak terlihat supaya ia berhemat
            // (halaman tetap berjalan, seperti tab di latar belakang).
            let _ = webview.set_visible(false);
            if low {
                set_memory_low(&webview, true);
            }
        }
        self.create_failed_at = None;
        self.overlay_on = false;
        self.focused = false;
        self.unfocused_since = visible.then(Instant::now);
        self.low_unfocused = false;
        self.page_fresh_at = Some(Instant::now());
        self.hidden_since = (!visible).then(Instant::now);
        // Rapikan setelah Outlook selesai dimuat.
        self.trim_at = (!visible).then(|| Instant::now() + TRIM_DELAY * 4);
        self.inst = Some(Instance {
            window,
            webview,
            visible,
            minimized: false,
            suspended: false,
            mobile,
        });
        self.refresh_tray();
        true
    }

    fn destroy_instance(&mut self) {
        if self.inst.is_some() {
            self.record_geom();
        }
        if let Some(inst) = self.inst.take() {
            // Webview dulu, baru jendela: menutup controller menghentikan proses WebView2.
            drop(inst.webview);
            drop(inst.window);
        }
        self.composing = false;
        self.focused = false;
        self.unfocused_since = None;
        self.trim_at = None;
        self.hidden_since = None;
        self.low_unfocused = false;
        self.background_mode = false;
        self.next_trim_check = None;
    }

    fn show(&mut self, target: &EventLoopWindowTarget<UserEvent>, url: Option<&str>) {
        self.bg_deadline = None;
        self.net.poke(Instant::now());
        // Outlook tersembunyi bertampilan mobile tidak dipakai untuk jendela: buat versi desktop.
        if self.inst.as_ref().is_some_and(|i| i.mobile) {
            self.destroy_instance();
        }
        if let Some(inst) = self.inst.as_mut() {
            if let Some(u) = url {
                let _ = inst.webview.load_url(u);
            }
            if !inst.visible || inst.minimized || inst.window.is_minimized() {
                let from = if inst.visible { "minimize" } else { "tray" };
                self.restore_started = Some((Instant::now(), from));
            }
            if !inst.visible {
                inst.window.set_visible(true);
                inst.visible = true;
                self.overlay_on = false; // tombol taskbar baru: pasang ulang tanda merah
                if self.settings.window.is_some_and(|g| g.maximized) {
                    inst.window.set_maximized(true);
                }
            }
            if inst.window.is_minimized() {
                inst.window.set_minimized(false);
            }
            let _ = inst.webview.set_visible(true); // juga melanjutkan halaman yang ditangguhkan
            inst.suspended = false;
            inst.minimized = false;
            set_memory_low(&inst.webview, false);
            inst.window.set_focus();
            self.unfocused_since = Some(Instant::now());
            self.low_unfocused = false;
            self.leave_background_mode();
            self.trim_at = None;
            self.hidden_since = None;
            self.refresh_tray();
            if self.restore_started.is_some() {
                self.force_read();
            }
        } else {
            self.create_instance(target, url.unwrap_or(MAIL_URL), true);
        }
    }

    /// X, tutup otomatis, atau menu "Tutup jendela".
    fn close_window(&mut self) {
        let Some(visible) = self.inst.as_ref().map(|i| i.visible) else {
            return;
        };
        if !visible {
            return;
        }
        self.record_geom();
        if self.active_mode() == Mode::Siaga && self.settings.tray_mobile_ui {
            // Eksperimen: ganti dengan Outlook tersembunyi bertampilan mobile (dibuat di tick).
            self.destroy_instance();
            self.next_tick = Instant::now();
        } else if self.active_mode() == Mode::Siaga {
            if let Some(inst) = self.inst.as_mut() {
                inst.window.set_visible(false);
                inst.visible = false;
                // Sama seperti minimize: WebView2 berhemat, halaman tetap memantau.
                let _ = inst.webview.set_visible(false);
                // Jumlah belum dibaca hanya terbaca di halaman Email
                // (bukan Kalender/Kontak atau pintasan seperti To Do).
                if !inst.webview.url().is_ok_and(|u| is_outlook_mail_url(&u)) {
                    let _ = inst.webview.load_url(MAIL_URL);
                }
                if self.settings.low_memory {
                    set_memory_low(&inst.webview, true);
                }
            }
            self.focused = false;
            self.unfocused_since = None;
            self.hidden_since = Some(Instant::now());
            self.trim_at = Some(Instant::now() + TRIM_DELAY);
        } else {
            self.destroy_instance();
        }
        self.refresh_tray();
    }

    fn record_geom(&mut self) {
        let Some(inst) = &self.inst else { return };
        let w = &inst.window;
        if !inst.visible || w.is_minimized() {
            return;
        }
        if w.is_maximized() {
            let g = self.settings.window.get_or_insert(WindowGeom {
                x: 100,
                y: 100,
                width: 1280,
                height: 800,
                maximized: true,
            });
            if !g.maximized {
                g.maximized = true;
                self.dirty = true;
            }
            return;
        }
        if let Ok(p) = w.outer_position() {
            let s = w.inner_size();
            let g = WindowGeom {
                x: p.x,
                y: p.y,
                width: s.width,
                height: s.height,
                maximized: false,
            };
            if self.settings.window != Some(g) {
                self.settings.window = Some(g);
                self.dirty = true;
            }
        }
    }

    fn on_resized(&mut self) {
        let jeda = self.active_mode() == Mode::Jeda;
        let low = self.settings.low_memory;
        let Some(inst) = self.inst.as_mut() else { return };
        if inst.window.is_minimized() {
            if !inst.minimized {
                inst.minimized = true;
                if low {
                    // Seperti saat ditutup ke tray: WebView2 tahu halaman tidak terlihat
                    // dan melepas memori tampilan, tapi halaman tetap memantau.
                    let _ = inst.webview.set_visible(false);
                    set_memory_low(&inst.webview, true);
                    self.trim_at = Some(Instant::now() + TRIM_DELAY);
                    // Di luar jam kerja halaman sekalian ditidurkan.
                    if jeda {
                        win::try_suspend(&inst.webview);
                        inst.suspended = true;
                    }
                }
            }
        } else {
            if inst.minimized {
                inst.minimized = false;
                let _ = inst.webview.set_visible(true); // juga melanjutkan halaman yang ditidurkan
                inst.suspended = false;
                set_memory_low(&inst.webview, false);
                self.low_unfocused = false;
                self.trim_at = None;
                self.leave_background_mode();
                if self.restore_started.is_none() {
                    self.restore_started = Some((Instant::now(), "minimize"));
                    self.force_read();
                }
            }
            self.record_geom();
        }
    }

    // ----- cek email -----

    fn start_bg_check(&mut self, target: &EventLoopWindowTarget<UserEvent>, manual: bool) {
        if manual {
            self.manual_check = true;
        }
        if self.inst.is_some() {
            // Outlook sudah hidup: bangunkan bila tidur, lalu minta pemantau melapor.
            self.wake_page();
            if manual {
                let (visible, on_mail) = self
                    .inst
                    .as_ref()
                    .map(|i| {
                        let mail = i.webview.url().is_ok_and(|u| is_outlook_mail_url(&u));
                        (i.visible, mail)
                    })
                    .unwrap_or((false, true));
                if !on_mail && visible {
                    // Pengguna sedang di Kalender/Kontak: jangan pindahkan halamannya.
                    self.manual_check = false;
                    match self.shown_unread().filter(|n| *n >= 0) {
                        Some(n) => self.report_manual(n),
                        None => self.toast(
                            "Outlook Lite",
                            "Buka halaman Email dulu untuk mengecek jumlah belum dibaca.",
                            None,
                            Some(false),
                        ),
                    }
                    return;
                }
                if let Some(inst) = &self.inst {
                    if on_mail {
                        let _ = inst
                            .webview
                            .evaluate_script("window.__oliteForce && window.__oliteForce()");
                    } else {
                        // Tersembunyi di Kalender/Kontak: kembali ke Email lalu hitung.
                        let _ = inst.webview.load_url(MAIL_URL);
                    }
                }
                self.bg_deadline.get_or_insert(
                    Instant::now()
                        + if on_mail {
                            Duration::from_secs(20)
                        } else {
                            BG_CHECK_TIMEOUT
                        },
                );
            }
            return;
        }
        if self.create_instance(target, MAIL_URL, false) {
            self.bg_deadline = Some(Instant::now() + BG_CHECK_TIMEOUT);
        } else {
            self.manual_check = false;
        }
    }

    fn on_process_failed(&mut self, kind: i32) {
        let what = match kind {
            0 => "proses inti WebView2 berhenti",
            1 => "proses halaman Outlook berhenti",
            2 => "halaman Outlook tidak merespons",
            3 => "proses bingkai halaman berhenti",
            4 => "proses utility WebView2 berhenti",
            6 => "proses GPU WebView2 berhenti",
            _ => "proses WebView2 lain berhenti",
        };
        self.log(&format!("WebView2: {what} (jenis {kind})"));
        self.diag_note(&format!("WebView2 gagal: {what}"));
        match kind {
            // Proses inti mati: WebView tidak bisa dipakai lagi, buat ulang di tick berikutnya
            // (Siaga membuat Outlook tersembunyi baru otomatis).
            0 => {
                self.destroy_instance();
                self.refresh_tray();
            }
            // Halaman mati/macet: muat ulang Outlook.
            1 | 2 => {
                if let Some(inst) = &self.inst {
                    let _ = inst.webview.load_url(MAIL_URL);
                }
                self.settle_until = Some(Instant::now() + REFRESH_SETTLE);
            }
            _ => {}
        }
    }

    fn on_nav_done(&mut self, ok: bool, status: i32) {
        let now = Instant::now();
        if ok {
            if let Some((dur, n)) = self.net.on_success(now) {
                let msg = format!(
                    "koneksi pulih setelah {} detik ({n} percobaan otomatis)",
                    dur.as_secs()
                );
                self.log(&msg);
                self.diag_note(&msg);
                self.refresh_tray();
            }
            return;
        }
        if !netrecover::is_network_error(status) {
            return;
        }
        if !self.net.failing() {
            let msg = format!(
                "Outlook gagal dimuat: tidak ada koneksi (status {status}, Windows: {})",
                if win::is_online() { "online" } else { "offline" }
            );
            self.log(&msg);
            self.diag_note(&msg);
        }
        self.net.on_failure(now);
        self.refresh_tray();
        if let Some(t) = self.net.next_at() {
            self.next_tick = self.next_tick.min(t);
        }
    }

    /// Pemulihan koneksi otomatis (dari tick). Lihat netrecover.rs.
    fn net_tick(&mut self, target: &EventLoopWindowTarget<UserEvent>, now: Instant) {
        if let Some((t, visible)) = self.recreate_at {
            if now >= t {
                self.recreate_at = None;
                if visible {
                    self.show(target, None);
                } else if self.active_mode() == Mode::Siaga {
                    self.create_instance(target, MAIL_URL, false);
                }
            } else {
                self.next_tick = self.next_tick.min(t);
            }
        }
        if !self.net.failing() {
            return;
        }
        if self.net.next_at().is_some_and(|t| now >= t) {
            let online = win::is_online();
            match self.net.tick(now, online) {
                netrecover::Action::None => {}
                netrecover::Action::Reload => {
                    self.log("koneksi: memuat ulang Outlook");
                    self.diag_note("koneksi: muat ulang");
                    if let Some(inst) = &self.inst {
                        let _ = inst.webview.load_url(MAIL_URL);
                    }
                    self.settle_until = Some(now + REFRESH_SETTLE);
                }
                netrecover::Action::Recreate => {
                    // Sama seperti Keluar lalu membuka lagi: proses WebView2 baru, sehingga
                    // jaringan/proxy dibaca ulang dari awal.
                    let visible = self
                        .inst
                        .as_ref()
                        .is_some_and(|i| i.visible && !i.minimized);
                    self.log(&format!(
                        "koneksi: membuat ulang WebView2 (percobaan {})",
                        self.net.attempts()
                    ));
                    self.diag_note("koneksi: buat ulang WebView2");
                    self.destroy_instance();
                    // Beri waktu proses WebView2 lama berhenti agar yang baru benar-benar baru.
                    self.recreate_at = Some((now + Duration::from_secs(4), visible));
                    self.settle_until = Some(now + REFRESH_SETTLE);
                }
            }
        }
        if let Some(t) = self.net.next_at() {
            self.next_tick = self.next_tick.min(t);
        }
    }

    /// Environment WebView2 dengan opsi resmi (lihat EnvOpts), atau None = environment bawaan wry.
    #[cfg(windows)]
    fn webview_env(
        &mut self,
    ) -> Option<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment> {
        let o = self.env_opts;
        let env = if o.custom {
            match win::create_environment(
                &self.dir.join("WebView2"),
                &self.browser_args,
                o.tracking_prevention,
                o.local_crash_reports,
            ) {
                Ok(env) => Some(env),
                Err(e) => {
                    // Cadangan: cara lama (environment dibuat wry). Outlook tetap jalan.
                    self.log(&format!("environment WebView2 sendiri gagal, pakai cara lama: {e}"));
                    None
                }
            }
        } else {
            None
        };
        if !self.env_logged {
            self.env_logged = true;
            self.log(if env.is_some() {
                "WebView2: environment sendiri (tracking prevention mati, crash disimpan lokal)"
            } else {
                "WebView2: environment bawaan"
            });
        }
        env
    }

    // ----- jendela Pengaturan -----

    fn open_settings(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        if let Some(w) = &self.settings_win {
            w.window.set_visible(true);
            w.window.set_minimized(false);
            w.window.set_focus();
            return;
        }
        let size = LogicalSize::new(400.0, 680.0);
        let mut wb = WindowBuilder::new()
            .with_title("Pengaturan Outlook Lite")
            .with_window_icon(window_image())
            .with_inner_size(size)
            .with_min_inner_size(LogicalSize::new(360.0, 420.0));
        // Di pojok kanan bawah, dekat ikon tray.
        if let Some(m) = target.primary_monitor() {
            let sf = m.scale_factor();
            let ms = m.size();
            let mp = m.position();
            let w = (size.width * sf) as i32;
            let h = (size.height * sf) as i32;
            let x = mp.x + ms.width as i32 - w - (16.0 * sf) as i32;
            let y = mp.y + ms.height as i32 - h - (64.0 * sf) as i32;
            wb = wb.with_position(PhysicalPosition::new(x.max(mp.x), y.max(mp.y)));
        }
        let window = match wb.build(target) {
            Ok(w) => w,
            Err(e) => {
                self.log(&format!("jendela pengaturan gagal: {e}"));
                return;
            }
        };
        #[cfg(windows)]
        let env = self.webview_env();
        #[cfg(windows)]
        let args = self.browser_args.clone();
        let proxy = self.proxy.clone();
        let builder = WebViewBuilder::new_with_web_context(&mut self.web_context)
            .with_html(SETTINGS_HTML)
            .with_ipc_handler(move |req: wry::http::Request<String>| {
                let _ = proxy.send_event(UserEvent::SettingsUi(req.body().clone()));
            })
            // Halaman lokal saja: tidak ada navigasi atau jendela baru ke internet.
            .with_navigation_handler(|url: String| {
                !(url.starts_with("http:") || url.starts_with("https:"))
            })
            .with_new_window_req_handler(|_url: String, _features| wry::NewWindowResponse::Deny);
        #[cfg(windows)]
        let builder = {
            use wry::WebViewBuilderExtWindows;
            match env {
                Some(env) => builder.with_environment(env),
                None => builder.with_additional_browser_args(args),
            }
        };
        match builder.build(&window) {
            Ok(webview) => {
                self.settings_win = Some(SettingsWin { window, webview });
            }
            Err(e) => self.log(&format!("halaman pengaturan gagal: {e}")),
        }
    }

    fn close_settings(&mut self) {
        self.settings_win = None;
    }

    /// Ada perubahan yang baru berlaku setelah restart?
    fn restart_needed(&self) -> bool {
        full_browser_args(&self.settings, self.started_tray) != self.browser_args
            || EnvOpts::of(&self.settings) != self.env_opts
    }

    /// Kirim keadaan terbaru ke jendela Pengaturan.
    fn push_settings_state(&self) {
        let Some(w) = &self.settings_win else { return };
        let st = serde_json::json!({
            "version": VERSION,
            "status": self.status_text(),
            "s": self.settings,
            "mailto": win::mailto_registered(),
            "update": self.update_ready,
            "restart": self.restart_needed(),
        });
        let _ = w
            .webview
            .evaluate_script(&format!("window.__olite && window.__olite.render({st})"));
    }

    fn on_settings_msg(&mut self, target: &EventLoopWindowTarget<UserEvent>, body: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
            return;
        };
        match v.get("t").and_then(|t| t.as_str()).unwrap_or("") {
            "ready" => {}
            "menu" => {
                if let Some(mid) = v.get("id").and_then(|x| x.as_str()) {
                    let mid = mid.to_string();
                    if mid == id::SETTINGS {
                        return;
                    }
                    self.on_menu(target, &mid);
                }
            }
            "set" => {
                if let (Some(key), Some(value)) =
                    (v.get("key").and_then(|k| k.as_str()), v.get("value"))
                {
                    self.apply_setting(target, key, value.clone());
                }
            }
            "shortcuts" => {
                if let Some(list) = v.get("list") {
                    self.apply_setting(target, "shortcuts", list.clone());
                }
            }
            "restart" => {
                self.save();
                self.restart();
                return;
            }
            "close" => {
                self.close_settings();
                return;
            }
            _ => return,
        }
        self.push_settings_state();
    }

    /// Ubah satu pengaturan dari jendela Pengaturan, simpan, lalu terapkan efeknya.
    fn apply_setting(
        &mut self,
        target: &EventLoopWindowTarget<UserEvent>,
        key: &str,
        value: serde_json::Value,
    ) {
        let Ok(mut all) = serde_json::to_value(&self.settings) else {
            return;
        };
        let Some(slot) = all.get_mut(key) else {
            return;
        };
        *slot = value;
        let Ok(mut new) = serde_json::from_value::<Settings>(all) else {
            return; // nilai tidak valid: abaikan, tampilan dikembalikan ke nilai lama
        };
        new.sanitize();
        self.settings = new;
        self.save();
        match key {
            "hotkey_toggle" | "hotkey_compose" => self.apply_hotkeys(),
            "efficiency_mode" | "low_memory"
                if !(self.settings.efficiency_mode && self.settings.low_memory) =>
            {
                self.leave_background_mode()
            }
            // Dipasang saat Outlook dibuat: muat ulang Outlook.
            "block_telemetry" | "unread_selector" => self.recreate_instance(target),
            "shortcuts" => self.build_tray(),
            _ => {}
        }
        self.tick(target);
        self.sync_menu();
        self.refresh_tray();
    }

    /// Buat ulang WebView (mis. setelah pengaturan skrip halaman berubah).
    fn recreate_instance(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        let Some(visible) = self.inst.as_ref().map(|i| i.visible && !i.minimized) else {
            return;
        };
        self.destroy_instance();
        if visible {
            self.show(target, None);
        } else if self.active_mode() == Mode::Siaga {
            self.create_instance(target, MAIL_URL, false);
        }
    }

    // ----- diagnostik -----

    fn diag_state_now(&self) -> diag::State {
        match &self.inst {
            None => diag::State::Jeda,
            Some(i) if !i.visible => diag::State::Tray,
            Some(i) if i.minimized => diag::State::Minimize,
            Some(_) if self.focused => diag::State::Dipakai,
            Some(_) => diag::State::TakFokus,
        }
    }

    /// Catat kejadian; ikut di kolom "catatan" sampel berikutnya.
    fn diag_note(&mut self, note: &str) {
        if self.settings.diagnostics && self.diag_notes.len() < 20 {
            self.diag_notes.push(note.to_string());
        }
    }

    fn take_sample(&mut self) {
        let kinds = self
            .inst
            .as_ref()
            .map(|i| win::webview_process_kinds(&i.webview))
            .unwrap_or_default();
        let procs = win::process_snapshot(&kinds);
        // Catat sumber angka belum dibaca bila berubah (Inbox vs badge Outlook).
        let src = format!(
            "inbox={} ({}) badge={}",
            self.unread.map_or("?".into(), |n| n.to_string()),
            if self.unread_how.is_empty() { "-" } else { &self.unread_how },
            self.badge_unread.map_or("?".into(), |n| n.to_string())
        );
        if src != self.last_unread_note {
            self.diag_notes.push(src.clone());
            self.last_unread_note = src;
        }
        let note = std::mem::take(&mut self.diag_notes).join(" | ");
        let row = self.sampler.row(
            Instant::now(),
            Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            self.diag_state_now(),
            &procs,
            self.shown_unread(),
            &note,
        );
        diag::append_row(&self.dir, &row);
        diag::flush_domains(&self.dir);
        self.last_row = Some(row);
    }

    fn diag_tick(&mut self, now: Instant) {
        if !self.settings.diagnostics {
            return;
        }
        let state = self.diag_state_now();
        if self.diag_state != Some(state) {
            if let Some(prev) = self.diag_state {
                self.diag_note(&format!("{} -> {}", prev.label(), state.label()));
                let soon = now + DIAG_AFTER_CHANGE;
                self.next_sample = Some(self.next_sample.map_or(soon, |t| t.min(soon)));
            }
            self.diag_state = Some(state);
        }
        let due = *self.next_sample.get_or_insert(now);
        if now >= due {
            self.take_sample();
            self.next_sample = Some(now + DIAG_EVERY);
        }
        if let Some(t) = self.next_sample {
            self.next_tick = self.next_tick.min(t);
        }
    }

    /// Satu file berisi semua yang dibutuhkan untuk perbaikan: versi, sistem, pengaturan,
    /// ringkasan RAM/CPU per keadaan, log, dan data mentah. Tanpa isi email.
    fn write_report(&mut self) {
        use std::fmt::Write as _;
        if self.settings.diagnostics {
            self.diag_note("laporan dibuat");
            self.take_sample();
        }
        let rows = diag::read_rows(&self.dir);
        let mut r = String::new();
        let _ = writeln!(r, "LAPORAN DIAGNOSTIK OUTLOOK LITE");
        let _ = writeln!(r, "Dibuat: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));
        let _ = writeln!(r, "Versi aplikasi: {VERSION}");
        let _ = writeln!(
            r,
            "WebView2 Runtime: {}",
            wry::webview_version().unwrap_or_else(|_| "?".into())
        );
        let _ = writeln!(
            r,
            "CPU logis: {}",
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0)
        );
        r.push_str(&win::system_info());
        let _ = writeln!(r, "Keadaan sekarang: {}", self.diag_state_now().label());
        let _ = writeln!(r, "Status: {}", self.status_text());
        let _ = writeln!(r, "Argumen WebView2: {}", self.browser_args);
        let _ = writeln!(r);

        let _ = writeln!(r, "== Ringkasan RAM (MB) dan CPU (%) per keadaan ==");
        if rows.is_empty() {
            let _ = writeln!(
                r,
                "Belum ada data. Aktifkan Pengaturan > Catat diagnostik, pakai aplikasi seperti biasa\n\
                 (buka, minimize, tutup ke tray) minimal 30 menit, lalu buat laporan lagi."
            );
        } else {
            let _ = writeln!(
                r,
                "{} sampel, {} s.d. {}",
                rows.len(),
                rows.first().map(|x| x.time.as_str()).unwrap_or(""),
                rows.last().map(|x| x.time.as_str()).unwrap_or("")
            );
            r.push_str(&diag::summarize(&rows));
        }
        let _ = writeln!(r);

        let _ = writeln!(r, "== Domain yang dihubungi WebView2 (jumlah permintaan) ==");
        diag::flush_domains(&self.dir);
        r.push_str(&diag::domains_report(&self.dir));
        let _ = writeln!(r);

        let _ = writeln!(r, "== Pengaturan ==");
        let mut st = self.settings.clone();
        st.window = None;
        let _ = writeln!(r, "{}", serde_json::to_string_pretty(&st).unwrap_or_default());
        let _ = writeln!(r);

        let _ = writeln!(r, "== Log (300 baris terakhir) ==");
        let mut log_lines: Vec<String> = Vec::new();
        for name in ["outlook-lite.old.log", "outlook-lite.log"] {
            if let Ok(s) = std::fs::read_to_string(self.dir.join(name)) {
                log_lines.extend(s.lines().map(str::to_string));
            }
        }
        let skip = log_lines.len().saturating_sub(300);
        for l in &log_lines[skip..] {
            let _ = writeln!(r, "{l}");
        }
        let _ = writeln!(r);

        let _ = writeln!(r, "== Data mentah (1000 sampel terakhir) ==");
        let _ = writeln!(r, "{}", diag::CSV_HEADER);
        let skip = rows.len().saturating_sub(1000);
        for row in &rows[skip..] {
            let _ = writeln!(r, "{}", row.to_csv());
        }

        let path = self.dir.join("laporan-diagnostik.txt");
        let _ = std::fs::create_dir_all(&self.dir);
        match std::fs::write(&path, r) {
            Ok(()) => {
                win::reveal_file(&path);
                win::open_in_notepad(&path);
                if !self.settings.diagnostics {
                    self.toast(
                        "Laporan dibuat",
                        "Catat diagnostik belum aktif: aktifkan dulu agar laporan berisi data RAM & CPU.",
                        None,
                        Some(false),
                    );
                }
            }
            Err(e) => self.log(&format!("laporan gagal: {e}")),
        }
    }

    /// Kembalikan prioritas normal (jendela dibuka): Outlook langsung responsif.
    fn leave_background_mode(&mut self) {
        if self.background_mode {
            win::set_background_mode(false);
            self.background_mode = false;
            self.diag_note("mode efisiensi nonaktif");
        }
        self.next_trim_check = None;
    }

    /// Hemat RAM berkala (dipanggil dari tick).
    fn manage_memory(&mut self, now: Instant) {
        if !self.settings.low_memory {
            return;
        }
        let Some(inst) = &self.inst else { return };
        let (visible, minimized) = (inst.visible, inst.minimized);

        // Jendela terbuka tapi lama tidak dipakai: minta WebView2 berhemat.
        if visible
            && !minimized
            && !self.focused
            && !self.low_unfocused
            && self.unfocused_since.is_some_and(|t| t.elapsed() >= UNFOCUSED_LOW)
        {
            set_memory_low(&inst.webview, true);
            self.low_unfocused = true;
        }

        let idle = !visible || minimized;
        if !idle {
            return;
        }

        // Mode efisiensi (EcoQoS, prioritas rendah) selama tersembunyi/diminimalkan. Diterapkan
        // ulang tiap pemeriksaan supaya proses WebView2 baru (setelah segarkan) ikut.
        let check_due = self.next_trim_check.is_none_or(|t| now >= t);
        if self.settings.efficiency_mode && (!self.background_mode || check_due) {
            let n = win::set_background_mode(true);
            if !self.background_mode {
                self.diag_note(&format!("mode efisiensi aktif ({n} proses)"));
            }
            self.background_mode = true;
        }

        // Rapikan RAM: sesaat setelah disembunyikan, lalu otomatis setiap kali RAM nyata
        // melewati batas (dicek tiap 30 detik), dan paling lambat tiap 5 menit.
        let first_due = self.trim_at.is_some_and(|t| now >= t);
        if first_due || check_due {
            self.next_trim_check = Some(now + TRIM_CHECK);
            let ws_mb = win::process_tree_memory()
                .map(|m| m.working / (1024 * 1024))
                .unwrap_or(0);
            let over = self.settings.trim_ws_mb > 0
                && ws_mb > self.settings.trim_ws_mb as u64
                && self.last_trim.is_none_or(|t| t.elapsed() >= TRIM_MIN_GAP);
            if first_due || over {
                if let Some(inst) = &self.inst {
                    // Pindah ke Low lagi memicu WebView2 membersihkan memori (GC JavaScript,
                    // cache), bukan hanya memindahkan halaman ke page file.
                    set_memory_low(&inst.webview, false);
                    set_memory_low(&inst.webview, true);
                }
                let n = win::trim_process_tree();
                self.diag_note(&format!("rapikan: {ws_mb} MB ({n} proses)"));
                self.last_trim = Some(now);
                self.trim_at = Some(now + TRIM_EVERY);
            }
        }
        if let Some(t) = self.trim_at {
            self.next_tick = self.next_tick.min(t);
        }
        if let Some(t) = self.next_trim_check {
            self.next_tick = self.next_tick.min(t);
        }

        // Outlook web membengkak bila berjalan berjam-jam: segarkan saat tersembunyi.
        let hidden_long = !visible
            && self.hidden_since.is_some_and(|t| t.elapsed() >= REFRESH_MIN_HIDDEN);
        let check_due = self
            .last_refresh_check
            .is_none_or(|t| t.elapsed() >= REFRESH_CHECK_EVERY);
        if !hidden_long
            || !check_due
            || self.composing
            || self.bg_deadline.is_some()
            || self.inspect_pending
            || self.needs_login
        {
            return;
        }
        self.last_refresh_check = Some(now);
        let s = &self.settings;
        let mem = win::process_tree_memory();
        let commit_mb = mem.map(|m| m.commit / (1024 * 1024)).unwrap_or(0);
        // RAM nyata (working set) yang naik lagi setelah dirapikan = Outlook aktif membengkak.
        let ws_mb = mem.map(|m| m.working / (1024 * 1024)).unwrap_or(0);
        let ws_big = s.refresh_ws_mb > 0 && ws_mb > s.refresh_ws_mb as u64;
        let too_big = ws_big || (s.refresh_mb > 0 && commit_mb > s.refresh_mb as u64);
        let too_old = s.refresh_hours > 0
            && self
                .page_fresh_at
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(s.refresh_hours as u64 * 3600));
        if too_big || too_old {
            let why = format!(
                "segarkan Outlook ({}; RAM {ws_mb} MB, commit {commit_mb} MB)",
                if ws_big {
                    "RAM naik lagi setelah dirapikan"
                } else if too_big {
                    "commit besar"
                } else {
                    "berjalan lama"
                }
            );
            self.log(&why);
            self.diag_note(&why);
            if let Some(inst) = &self.inst {
                let _ = inst.webview.load_url(MAIL_URL);
            }
            self.page_fresh_at = Some(now);
            self.settle_until = Some(now + REFRESH_SETTLE);
            self.trim_at = Some(now + TRIM_DELAY * 4);
        }
    }

    /// Lanjutkan halaman yang ditangguhkan (minimize di luar jam kerja).
    fn wake_page(&mut self) {
        if let Some(inst) = self.inst.as_mut() {
            if inst.suspended {
                let _ = inst.webview.set_visible(true);
                inst.suspended = false;
            }
        }
    }

    /// Total belum dibaca di Inbox: dibaca dari panel folder Outlook. Badge Outlook hanya
    /// cadangan bila panel folder tidak terbaca, karena badge = email baru sejak terakhir
    /// dilihat (bukan total), dan Outlook mengosongkannya saat jendela dibuka.
    fn shown_unread(&self) -> Option<i64> {
        self.unread.or(self.badge_unread)
    }

    /// Minta halaman membaca ulang angka Inbox sekarang, lalu sekali lagi sebentar kemudian
    /// (angka Outlook kadang baru berubah 1-2 detik setelah suara/notifikasi).
    fn force_read(&mut self) {
        if let Some(inst) = &self.inst {
            let _ = inst
                .webview
                .evaluate_script("window.__oliteForce && window.__oliteForce()");
        }
        let again = Instant::now() + FORCE_AGAIN;
        if self.force_again.is_none() {
            self.force_again = Some(again);
        }
        self.next_tick = self.next_tick.min(again);
    }

    fn sound_recent(&self) -> bool {
        self.last_sound.is_some_and(|t| t.elapsed() < Duration::from_secs(10))
    }

    /// Jadwalkan tick saat batch email masuk perlu ditutup.
    fn schedule_mail_flush(&mut self) {
        if let Some(b) = self.mail.batch() {
            self.next_tick = self.next_tick.min(b.due_at());
        }
    }

    /// Jumlah belum dibaca berubah (badge atau halaman).
    fn note_count(&mut self, prev: Option<i64>, total: i64) {
        // Sesaat setelah Outlook disegarkan, angka bisa naik-turun saat halaman dimuat.
        let settling = self.settle_until.is_some_and(|t| Instant::now() < t);
        self.mail.on_count(if settling { None } else { prev }, total, Instant::now());
        self.schedule_mail_flush();
    }

    fn on_badge(&mut self, count: i64) {
        let prev = self.badge_unread;
        self.badge_unread = Some(count);
        self.login_seen_at = None;
        if self.unread.is_none() {
            // Panel folder belum/tidak terbaca: badge satu-satunya angka.
            self.note_count(prev, count);
        } else if prev.is_some_and(|p| count > p) {
            self.mail.on_badge_rise(self.shown_unread(), Instant::now());
            self.schedule_mail_flush();
            self.force_read();
        }
        self.refresh_tray();
    }

    fn on_sound(&mut self) {
        self.last_sound = Some(Instant::now());
        self.mail.on_sound(self.shown_unread(), Instant::now());
        self.schedule_mail_flush();
        self.force_read();
    }

    /// Tutup batch email masuk bila waktunya, lalu tampilkan satu toast:
    /// baris atas jumlah email baru, baris bawah total belum dibaca.
    fn flush_mail_alert(&mut self) {
        let outlook_sound = self.mail.batch().is_some_and(|b| b.outlook_played_sound());
        let Some(alert) = self.mail.take_due(Instant::now()) else {
            self.schedule_mail_flush();
            return;
        };
        if self.user_looking() {
            return;
        }
        let click = match &alert {
            mailalert::Alert::Mail { notice, .. } => notice.as_ref().map(|n| n.id),
            mailalert::Alert::Other { notice, .. } => Some(notice.id),
        };
        let (title, l1, l2) = mailalert::texts(&alert, self.settings.show_details);
        // Outlook sudah membunyikan suaranya sendiri: jangan dobel.
        let sound = self.settings.sound && !outlook_sound && !self.sound_recent();
        let what = match &alert {
            mailalert::Alert::Mail { new, total, .. } => format!(
                "toast email baru: {} (belum dibaca {})",
                new.map_or("?".into(), |n| n.to_string()),
                total.map_or("?".into(), |n| n.to_string())
            ),
            mailalert::Alert::Other { .. } => "toast pengingat/pemberitahuan Outlook".into(),
        };
        self.log(&what);
        self.diag_note(&what);
        self.toast3(&title, &l1, &l2, click, Some(sound));
    }

    fn report_manual(&self, n: i64) {
        let body = if n > 0 {
            format!("Belum dibaca di Inbox: {n}")
        } else {
            "Tidak ada email belum dibaca di Inbox.".into()
        };
        self.toast("Outlook Lite", &body, None, Some(false));
    }

    fn finish_bg_check(&mut self) {
        self.bg_deadline = None;
        let hidden = self.inst.as_ref().is_some_and(|i| !i.visible);
        if hidden && self.active_mode() == Mode::Jeda && !self.inspect_pending {
            self.destroy_instance();
        }
    }

    fn on_state(&mut self, unread: i64, composing: bool, how: String, force: bool) {
        self.composing = composing;
        if let Some((t, from)) = self.restore_started.take() {
            // Waktu dari jendela dibuka sampai halaman Outlook menjawab lagi.
            let ms = t.elapsed().as_millis();
            let msg = format!("buka jendela dari {from}: halaman merespons {ms} ms");
            self.log(&msg);
            self.diag_note(&msg);
        }
        if how != self.unread_how && !how.is_empty() {
            self.diag_note(&format!("baca Inbox: {how}"));
            self.unread_how = how;
        }
        self.login_seen_at = None;
        if self.needs_login {
            self.needs_login = false;
            self.login_toasted = false;
        }
        if self.inspect_pending && !self.inspect_sent {
            if let Some(inst) = &self.inst {
                let _ = inst.webview.evaluate_script(INSPECT_JS);
                self.inspect_sent = true;
            }
        }
        if unread < 0 && force && self.manual_check && self.unread.is_none() {
            // Halaman termuat tapi panel folder tidak tampil.
            self.manual_check = false;
            self.toast(
                "Jumlah belum dibaca tidak terbaca",
                "Panel folder Outlook tidak tampil. Lebarkan jendela Outlook Lite atau tampilkan panel folder.",
                None,
                Some(false),
            );
        }
        if unread >= 0 {
            let prev = self.unread;
            self.unread = Some(unread);
            self.note_count(prev, unread);
            if self.manual_check {
                self.manual_check = false;
                self.report_manual(self.shown_unread().unwrap_or(unread));
            }
            if self.bg_deadline.is_some() && !self.inspect_pending {
                self.finish_bg_check();
            }
        }
        self.refresh_tray();
    }

    fn on_notify(&mut self, id: u64, title: String, body: String) {
        let notice = mailalert::Notice { id, title, body };
        self.mail.on_notice(notice, self.shown_unread(), Instant::now());
        self.schedule_mail_flush();
        self.force_read();
    }

    fn on_page_loaded(&mut self, url: String) {
        let hidden = self.inst.as_ref().is_some_and(|i| !i.visible);
        let host = urlutil::host_of(&url).unwrap_or_else(|| "?".into());
        self.log(&format!(
            "halaman dimuat: {host}{}",
            if hidden { " (tersembunyi)" } else { "" }
        ));
        if is_outlook_url(&url) {
            self.login_seen_at = None; // status login dibersihkan saat pemantau melapor
            return;
        }
        // Di jendela terlihat, halaman lain bisa saja pintasan (To Do, intranet).
        // Di instance tersembunyi kita hanya memuat Outlook, jadi halaman lain = login/redirect.
        if hidden && self.login_seen_at.is_none() {
            self.login_seen_at = Some(Instant::now());
        }
    }

    /// Dipanggil dari tick: halaman login bertahan > LOGIN_GRACE berarti sesi habis.
    fn check_login_stuck(&mut self) {
        let Some(t) = self.login_seen_at else { return };
        if t.elapsed() < LOGIN_GRACE || self.needs_login {
            return;
        }
        self.needs_login = true;
        self.diag_note("perlu login");
        self.log("halaman login bertahan; notifikasi berhenti sampai login");
        let cooled = self
            .last_login_toast
            .is_none_or(|t| t.elapsed() >= LOGIN_TOAST_COOLDOWN);
        if !self.login_toasted && cooled {
            self.login_toasted = true;
            self.last_login_toast = Some(Instant::now());
            self.toast(
                "Outlook Lite perlu login ulang",
                "Notifikasi berhenti sampai Anda login. Klik untuk membuka.",
                None,
                Some(false),
            );
        }
        if self.bg_deadline.is_some() && !self.inspect_pending {
            self.manual_check = false;
            self.finish_bg_check();
        }
        self.refresh_tray();
    }

    fn on_inspect(&mut self, data: String) {
        if !self.inspect_pending {
            return; // hasil ganda
        }
        self.inspect_pending = false;
        self.inspect_sent = false;
        let path = self.dir.join("diagnostik-halaman.json");
        match std::fs::write(&path, data) {
            Ok(()) => {
                win::open_in_notepad(&path);
                self.toast(
                    "Diagnostik halaman selesai",
                    "Hasil disimpan di diagnostik-halaman.json (tanpa isi email).",
                    None,
                    Some(false),
                );
            }
            Err(e) => self.log(&format!("simpan diagnostik gagal: {e}")),
        }
        if self.bg_deadline.is_some() {
            self.finish_bg_check();
        }
    }

    fn start_inspect(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        if self.inspect_pending {
            return;
        }
        self.inspect_pending = true;
        self.inspect_sent = false;
        self.wake_page();
        if let Some(inst) = &self.inst {
            let _ = inst.webview.evaluate_script(INSPECT_JS);
            self.inspect_sent = true;
            self.bg_deadline.get_or_insert(Instant::now() + INSPECT_TIMEOUT);
        } else if self.create_instance(target, MAIL_URL, false) {
            self.bg_deadline = Some(Instant::now() + INSPECT_TIMEOUT);
        } else {
            self.inspect_pending = false;
        }
    }

    // ----- update -----

    fn start_update_check(&mut self, manual: bool) {
        if self.update_running {
            return;
        }
        self.update_running = true;
        self.last_update_check = Some(Instant::now());
        let proxy = self.proxy.clone();
        let dir = self.dir.clone();
        std::thread::spawn(move || {
            let out = update::check_and_stage(&dir);
            let _ = proxy.send_event(UserEvent::Update(out, manual));
        });
    }

    fn on_update(&mut self, out: update::Outcome, manual: bool) {
        self.update_running = false;
        match out {
            update::Outcome::Ready(v) => {
                self.log(&format!("update v{v} siap"));
                self.toast(
                    &format!("Outlook Lite v{v} siap dipakai"),
                    "Pilih \"Restart untuk memakai versi baru\" di menu tray, atau biarkan aktif saat Windows dinyalakan ulang.",
                    None,
                    Some(false),
                );
                self.update_ready = Some(v);
            }
            update::Outcome::UpToDate => {
                if manual {
                    self.toast(
                        "Outlook Lite",
                        &format!("Sudah versi terbaru (v{VERSION})."),
                        None,
                        Some(false),
                    );
                }
            }
            update::Outcome::Failed(e) => {
                self.log(&format!("cek update gagal: {e}"));
                if manual {
                    self.toast("Cek update gagal", &e, None, Some(false));
                }
            }
        }
        self.sync_menu();
    }

    fn restart(&mut self) {
        let visible = self.inst.as_ref().is_some_and(|i| i.visible);
        let mut cmd = std::process::Command::new(&self.exe);
        cmd.arg("--restarted");
        if !visible {
            cmd.arg("--tray");
        }
        match cmd.spawn() {
            Ok(_) => self.quit("restart"),
            Err(e) => self.log(&format!("restart gagal: {e}")),
        }
    }

    fn quit(&mut self, reason: &str) {
        self.log(&format!("keluar: {reason}"));
        let _ = std::fs::remove_file(self.dir.join(SESSION_MARKER));
        self.destroy_instance();
        self.save();
        self.tray = None;
        self.exiting = true;
    }

    fn save(&mut self) {
        if let Err(e) = self.settings.save(&self.dir) {
            self.log(&format!("simpan pengaturan gagal: {e}"));
        }
        self.dirty = false;
    }

    // ----- loop -----

    fn init(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        #[cfg(windows)]
        {
            self.settings.autostart = win::autostart_value().is_some();
        }
        self.build_tray();
        self.apply_hotkeys();
        if let Some(url) = self.initial_url.take() {
            self.show(target, Some(&url));
        }
        self.last_mode = self.active_mode();
        self.tick(target);
    }

    fn tick(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        let now = Instant::now();
        self.next_tick = now + TICK;
        let mode = self.active_mode();
        if mode != self.last_mode {
            self.last_mode = mode;
        }

        if mode == Mode::Siaga {
            let can_retry = self
                .create_failed_at
                .is_none_or(|t| t.elapsed() >= CREATE_RETRY);
            if self.inst.is_none() && can_retry && self.recreate_at.is_none() {
                self.create_instance(target, MAIL_URL, false);
            }
        }

        self.check_login_stuck();
        if self.force_again.is_some_and(|t| now >= t) {
            self.force_again = None;
            if let Some(inst) = &self.inst {
                let _ = inst
                    .webview
                    .evaluate_script("window.__oliteForce && window.__oliteForce()");
            }
        }
        self.flush_mail_alert();
        self.net_tick(target, now);
        self.manage_memory(now);

        // Batas waktu cek/diagnostik tersembunyi.
        if self.bg_deadline.is_some_and(|d| now >= d) {
            if self.manual_check {
                self.manual_check = false;
                self.toast(
                    "Cek email gagal",
                    "Outlook belum termuat. Buka jendela untuk memeriksa login.",
                    None,
                    Some(false),
                );
            }
            if self.inspect_pending {
                self.inspect_pending = false;
                self.inspect_sent = false;
                self.toast(
                    "Diagnostik halaman gagal",
                    "Buka Outlook, pastikan sudah login, lalu coba lagi.",
                    None,
                    Some(false),
                );
            }
            self.finish_bg_check();
        }

        // Jendela tersembunyi yang tidak lagi dibutuhkan (mis. jam kerja selesai).
        let hidden = self.inst.as_ref().is_some_and(|i| !i.visible);
        if hidden && mode == Mode::Jeda && self.bg_deadline.is_none() && !self.inspect_pending {
            self.destroy_instance();
            self.refresh_tray();
        }

        // Tutup otomatis saat tidak aktif.
        if self.settings.auto_close && !self.composing {
            let limit = Duration::from_secs(self.settings.auto_close_min as u64 * 60);
            let visible = self.inst.as_ref().is_some_and(|i| i.visible);
            if visible && !self.focused && self.unfocused_since.is_some_and(|t| t.elapsed() >= limit)
            {
                self.close_window();
            }
        }

        self.diag_tick(now);

        // Update otomatis.
        if self.settings.auto_update && self.update_ready.is_none() {
            let due = match self.last_update_check {
                None => self.started.elapsed() >= UPDATE_FIRST_DELAY,
                Some(t) => t.elapsed() >= UPDATE_EVERY,
            };
            if due {
                self.start_update_check(false);
            }
        }

        if self.dirty {
            self.save();
        }
        self.refresh_tray();
    }

    fn on_menu(&mut self, target: &EventLoopWindowTarget<UserEvent>, mid: &str) {
        let mut changed = true;
        match mid {
            id::OPEN_MAIL => {
                changed = false;
                self.show(target, Some(MAIL_URL))
            }
            id::OPEN_CAL => {
                changed = false;
                self.show(target, Some(CALENDAR_URL))
            }
            id::COMPOSE => {
                changed = false;
                self.show(target, Some(COMPOSE_URL))
            }
            id::CHECK_NOW => {
                changed = false;
                self.start_bg_check(target, true)
            }
            id::WORK_HOURS => self.settings.work_hours_only = !self.settings.work_hours_only,
            id::SOUND => self.settings.sound = !self.settings.sound,
            id::DETAILS => self.settings.show_details = !self.settings.show_details,
            id::TOAST_COMPAT => self.settings.toast_compat = !self.settings.toast_compat,
            id::TEST_TOAST => {
                changed = false;
                // Contoh bentuk toast email masuk, dengan angka belum dibaca saat ini.
                let total = self.shown_unread().map(|n| n.max(0) + 1);
                let (title, l1, _) = mailalert::texts(
                    &mailalert::Alert::Mail { new: Some(1), total, notice: None },
                    false,
                );
                self.toast3(
                    &format!("[Tes] {title}"),
                    &l1,
                    "Notifikasi berfungsi. Klik untuk membuka Outlook.",
                    None,
                    None,
                );
            }
            id::AUTO_CLOSE => self.settings.auto_close = !self.settings.auto_close,
            id::LOW_MEM => self.settings.low_memory = !self.settings.low_memory,
            id::NO_GPU => {
                self.settings.no_gpu = !self.settings.no_gpu;
                self.save();
                changed = false;
                if win::confirm("Pengaturan GPU berlaku setelah restart. Restart Outlook Lite sekarang?") {
                    self.restart();
                }
            }
            id::ZOOM_IN | id::ZOOM_OUT | id::ZOOM_RESET => {
                let z = match mid {
                    id::ZOOM_IN => self.settings.zoom + 0.1,
                    id::ZOOM_OUT => self.settings.zoom - 0.1,
                    _ => 1.0,
                };
                self.settings.zoom = (z.clamp(0.5, 2.0) * 10.0).round() / 10.0;
                if let Some(i) = &self.inst {
                    let _ = i.webview.zoom(self.settings.zoom);
                }
            }
            id::AUTOSTART => {
                let on = !self.settings.autostart;
                match win::set_autostart(on, &self.exe) {
                    Ok(()) => self.settings.autostart = on,
                    Err(e) => self.log(&format!("autostart gagal: {e}")),
                }
            }
            id::HOTKEYS => {
                self.settings.hotkeys = !self.settings.hotkeys;
                self.apply_hotkeys();
            }
            id::MAILTO => {
                if win::mailto_registered() {
                    win::unregister_mailto();
                } else {
                    match win::register_mailto(&self.exe) {
                        Ok(()) => {
                            win::open_default_apps_settings();
                            self.toast(
                                "Satu langkah lagi",
                                "Di jendela Aplikasi default, pilih Outlook Lite untuk MAILTO.",
                                None,
                                Some(false),
                            );
                        }
                        Err(e) => self.log(&format!("daftar mailto gagal: {e}")),
                    }
                }
            }
            id::AUTO_UPDATE => self.settings.auto_update = !self.settings.auto_update,
            id::CHECK_UPDATE => {
                changed = false;
                self.start_update_check(true)
            }
            id::DIAG_RAM => {
                self.settings.diagnostics = !self.settings.diagnostics;
                diag::set_domains_enabled(self.settings.diagnostics);
                self.next_sample = None;
                self.diag_state = None;
                self.last_row = None;
                if self.settings.diagnostics {
                    self.diag_note("diagnostik dimulai");
                }
            }
            id::DIAG_REPORT => {
                changed = false;
                self.write_report();
            }
            id::NO_SW => {
                self.settings.block_service_worker = !self.settings.block_service_worker;
                self.diag_note(if self.settings.block_service_worker {
                    "service worker dimatikan"
                } else {
                    "service worker diizinkan"
                });
                // Skrip halaman dipasang saat WebView dibuat: buat ulang.
                self.recreate_instance(target);
            }
            id::INSPECT => {
                changed = false;
                self.start_inspect(target)
            }
            id::OPEN_FOLDER => {
                changed = false;
                if self.dirty {
                    self.save();
                } else if !Settings::path(&self.dir).exists() {
                    self.save();
                }
                win::open_folder(&self.dir)
            }
            id::OPEN_APP_FOLDER => {
                changed = false;
                win::reveal_file(&self.exe);
            }
            id::UNINSTALL => {
                changed = false;
                // Proses terpisah menampilkan konfirmasi, lalu meminta instance ini keluar.
                if let Err(e) = std::process::Command::new(&self.exe).arg("--uninstall").spawn() {
                    self.log(&format!("uninstall gagal: {e}"));
                }
            }
            id::RELOAD => {
                self.settings = Settings::load(&self.dir);
                #[cfg(windows)]
                {
                    self.settings.autostart = win::autostart_value().is_some();
                }
                self.build_tray();
                self.apply_hotkeys();
                self.dirty = false;
                changed = false;
            }
            id::RESTART_UPDATE => {
                changed = false;
                if self.update_ready.is_some() {
                    self.restart()
                }
            }
            id::CLOSE_WINDOW => {
                changed = false;
                self.close_window()
            }
            id::SETTINGS => {
                changed = false;
                self.open_settings(target)
            }
            id::QUIT => {
                changed = false;
                self.quit("menu Keluar")
            }
            other => {
                changed = false;
                if let Some(i) = other.strip_prefix(id::SHORTCUT) {
                    if let Some(sc) = i
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| self.settings.shortcuts.get(i))
                    {
                        let url = sc.url.clone();
                        self.show(target, Some(&url));
                    }
                }
            }
        }
        if changed {
            self.dirty = true;
            if !self.exiting {
                self.tick(target);
            }
        }
        self.sync_menu();
        self.refresh_tray();
    }

    fn handle(
        &mut self,
        event: Event<'_, UserEvent>,
        target: &EventLoopWindowTarget<UserEvent>,
        control_flow: &mut ControlFlow,
    ) {
        match event {
            Event::NewEvents(StartCause::Init) => self.init(target),
            Event::NewEvents(_) => {
                if Instant::now() >= self.next_tick {
                    self.tick(target);
                }
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } if self.settings_win.as_ref().is_some_and(|w| w.window.id() == window_id) => {
                self.close_settings()
            }
            Event::WindowEvent {
                window_id, event, ..
            } if self.is_our_window(window_id) => match event {
                WindowEvent::CloseRequested => self.close_window(),
                WindowEvent::Focused(f) => {
                    self.focused = f;
                    self.unfocused_since = if f { None } else { Some(Instant::now()) };
                    if f && self.low_unfocused {
                        self.low_unfocused = false;
                        if let Some(i) = &self.inst {
                            set_memory_low(&i.webview, false);
                        }
                    }
                }
                WindowEvent::Resized(_) => self.on_resized(),
                WindowEvent::Moved(_) => self.record_geom(),
                _ => {}
            },
            Event::UserEvent(ev) => match ev {
                UserEvent::Menu(m) => {
                    let mid = m.id.0.clone();
                    self.on_menu(target, &mid);
                }
                UserEvent::Tray(TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }) => {
                    if self.user_looking() {
                        self.close_window();
                    } else {
                        self.show(target, None);
                    }
                }
                UserEvent::Tray(_) => {}
                UserEvent::HotKey(e) => {
                    if e.state() == HotKeyState::Pressed {
                        if Some(e.id()) == self.hk_toggle {
                            if self.user_looking() {
                                self.close_window();
                            } else {
                                self.show(target, None);
                            }
                        } else if Some(e.id()) == self.hk_compose {
                            self.show(target, Some(COMPOSE_URL));
                        }
                    }
                }
                UserEvent::Remote(RemoteCmd::Show) => self.show(target, None),
                UserEvent::Remote(RemoteCmd::Open(url)) => self.show(target, Some(&url)),
                UserEvent::Remote(RemoteCmd::Exit) => {
                    self.quit("diminta instance lain (instal/update/hapus instalasi)")
                }
                UserEvent::ProcessFailed(kind) => self.on_process_failed(kind),
                UserEvent::NavDone(ok, status) => self.on_nav_done(ok, status),
                UserEvent::SettingsUi(body) => self.on_settings_msg(target, &body),
                UserEvent::Page(PageMsg::State {
                    unread,
                    composing,
                    how,
                    force,
                }) => self.on_state(unread, composing, how, force),
                UserEvent::Page(PageMsg::Notify { id, title, body }) => {
                    self.on_notify(id, title, body)
                }
                UserEvent::Page(PageMsg::Badge(n)) => self.on_badge(n),
                UserEvent::Page(PageMsg::Sound) => self.on_sound(),
                UserEvent::Page(PageMsg::Inspect(data)) => self.on_inspect(data),
                UserEvent::PageLoaded(url) => self.on_page_loaded(url),
                UserEvent::ToastClick(id) => {
                    self.show(target, None);
                    if let (Some(id), Some(inst)) = (id, &self.inst) {
                        let _ = inst
                            .webview
                            .evaluate_script(&format!("window.__oliteClick && window.__oliteClick({id})"));
                    }
                }
                UserEvent::Update(out, manual) => self.on_update(out, manual),
            },
            Event::LoopDestroyed => {
                if self.dirty {
                    self.save();
                }
            }
            _ => {}
        }

        *control_flow = if self.exiting {
            ControlFlow::Exit
        } else {
            ControlFlow::WaitUntil(self.next_tick)
        };
    }
}

fn main() {
    let args = parse_args();
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    {
        // Catat lokasi error sebelum aplikasi berhenti (release memakai panic = abort).
        let d = dir.clone();
        std::panic::set_hook(Box::new(move |info| {
            log(&d, &format!("ERROR (aplikasi berhenti): {info}"));
        }));
    }

    if args.uninstall {
        run_uninstall(&dir);
        return;
    }
    update::cleanup_old();

    // Instal otomatis ke %LOCALAPPDATA%\Programs\OutlookLite (tanpa admin), lalu jalankan dari sana.
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(install::EXE_NAME));
    if install::should_install(&exe, args.portable) {
        let mut stopped = false;
        match install::install_self(&exe, || stopped = stop_running_instance()) {
            Ok(target) => {
                let mut cmd = std::process::Command::new(&target);
                cmd.args(std::env::args_os().skip(1).filter(|a| a != "--restarted"));
                if stopped || args.restarted {
                    cmd.arg("--restarted");
                }
                match cmd.spawn() {
                    Ok(_) => {
                        log(&dir, &format!("dijalankan dari {}", target.display()));
                        return;
                    }
                    Err(e) => log(&dir, &format!("jalankan salinan terpasang gagal: {e}")),
                }
            }
            Err(e) => log(&dir, &format!("instal otomatis gagal, jalan portable: {e}")),
        }
    } else if install::is_installed_copy(&exe) {
        install::ensure_integration(&exe);
    }

    let initial_url: Option<String> = if let Some(m) = &args.mailto {
        urlutil::mailto_to_compose(m)
    } else if args.compose {
        Some(COMPOSE_URL.to_string())
    } else if args.tray {
        None
    } else {
        Some(MAIL_URL.to_string())
    };

    let remote_cmd = initial_url.as_deref().map(|u| {
        if u.starts_with(COMPOSE_URL) {
            format!("open {u}")
        } else {
            "show".to_string()
        }
    });
    let listener = match acquire_lock(remote_cmd.as_deref(), args.restarted) {
        Lock::Primary(l) => Some(l),
        Lock::Secondary => return,
        Lock::None => {
            log(&dir, "port single-instance terpakai aplikasi lain; jalan tanpa penjaga");
            None
        }
    };

    let settings = Settings::load(&dir);

    // Catat start dan deteksi sesi sebelumnya yang berhenti tanpa "keluar" (crash, dimatikan
    // paksa lewat Task Manager, atau Windows dimatikan/logoff saat aplikasi berjalan).
    let marker = dir.join(SESSION_MARKER);
    let abnormal = std::fs::read_to_string(&marker).ok();
    if let Some(prev) = &abnormal {
        log(
            &dir,
            &format!("sesi sebelumnya berhenti tidak normal ({})", prev.trim()),
        );
    }
    log(
        &dir,
        &format!(
            "mulai v{VERSION} (pid {}, argumen: {})",
            std::process::id(),
            std::env::args().skip(1).collect::<Vec<_>>().join(" ")
        ),
    );
    let _ = std::fs::write(
        &marker,
        format!(
            "v{VERSION}, pid {}, mulai {}",
            std::process::id(),
            Local::now().format("%Y-%m-%d %H:%M:%S")
        ),
    );

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    if let Some(l) = listener {
        serve_lock(l, proxy.clone());
    }
    {
        let p = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(UserEvent::Menu(e));
        }));
    }
    {
        let p = proxy.clone();
        TrayIconEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(UserEvent::Tray(e));
        }));
    }
    {
        let p = proxy.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(UserEvent::HotKey(e));
        }));
    }

    let icon_png = {
        let p = dir.join("icon.png");
        std::fs::write(&p, icon::png(64, 64, &icon::rgba(64, Badge::None)))
            .ok()
            .map(|_| p)
    };
    win::init_identity(icon_png.as_deref());
    win::refresh_registrations(&exe);

    let browser_args = full_browser_args(&settings, args.tray);
    let settings_snapshot = settings.clone();

    let mut app = App {
        web_context: WebContext::new(Some(dir.join("WebView2"))),
        settings,
        dirty: false,
        dir,
        exe,
        icon_png,
        proxy,
        browser_args,
        tray: None,
        menu: None,
        badge: Badge::None,
        inst: None,
        focused: false,
        unfocused_since: None,
        composing: false,
        unread: None,
        badge_unread: None,
        last_sound: None,
        mail: mailalert::Tracker::default(),
        last_login_toast: None,
        overlay_on: false,
        needs_login: false,
        login_toasted: false,
        login_seen_at: None,
        last_tip: String::new(),
        last_title: String::new(),
        next_tick: Instant::now() + TICK,
        started: Instant::now(),
        last_mode: Mode::Jeda,
        bg_deadline: None,
        manual_check: false,
        inspect_pending: false,
        inspect_sent: false,
        create_failed_at: None,
        hk_manager: None,
        hk_registered: Vec::new(),
        hk_toggle: None,
        hk_compose: None,
        last_row: None,
        unread_how: String::new(),
        force_again: None,
        last_unread_note: String::new(),
        sampler: diag::Sampler::new(
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        ),
        next_sample: None,
        diag_notes: Vec::new(),
        diag_state: None,
        trim_at: None,
        hidden_since: None,
        page_fresh_at: None,
        last_refresh_check: None,
        settle_until: None,
        low_unfocused: false,
        next_trim_check: None,
        last_trim: None,
        background_mode: false,
        net: netrecover::NetRecovery::default(),
        env_logged: false,
        env_opts: EnvOpts::of(&settings_snapshot),
        started_tray: args.tray,
        settings_win: None,
        restore_started: None,
        recreate_at: None,
        update_running: false,
        last_update_check: None,
        update_ready: None,
        initial_url,
        exiting: false,
    };
    diag::set_domains_enabled(app.settings.diagnostics);
    if abnormal.is_some() {
        app.diag_note("sesi sebelumnya berhenti tidak normal");
    }

    event_loop.run(move |event, target, control_flow| app.handle(event, target, control_flow));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_messages() {
        assert!(matches!(
            parse_page_msg(r#"{"t":"state","unread":3,"composing":true,"how":"angka"}"#),
            Some(PageMsg::State { unread: 3, composing: true, force: false, .. })
        ));
        match parse_page_msg(r#"{"t":"notify","id":7,"title":"A\u0007B","body":"x"}"#) {
            Some(PageMsg::Notify { id, title, .. }) => {
                assert_eq!(id, 7);
                assert_eq!(title, "AB");
            }
            _ => panic!(),
        }
        assert!(parse_page_msg("not json").is_none());
        assert!(parse_page_msg(r#"{"t":"evil"}"#).is_none());
    }

    #[test]
    fn args() {
        // mailto langsung sebagai argumen tunggal juga dikenali
        let a = Args {
            tray: false,
            compose: false,
            restarted: false,
            portable: false,
            uninstall: false,
            mailto: Some("mailto:x@y.z".into()),
        };
        assert!(urlutil::mailto_to_compose(a.mailto.as_deref().unwrap()).is_some());
    }
}
