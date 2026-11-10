//! Outlook Lite — Outlook web (outlook.office.com) di tray Windows, tanpa hak admin.
//!
//! Satu "instance" Outlook (jendela + WebView2) yang bisa terlihat, tersembunyi, atau
//! tidak ada sama sekali, diatur oleh mode notifikasi:
//! - Mati  : Outlook hanya hidup saat jendela terbuka; X = hancurkan (RAM kembali ~0).
//! - Hemat : seperti Mati, plus cek tersembunyi tiap N menit lalu dihancurkan lagi.
//! - Siaga : Outlook tersembunyi selalu hidup (jam kerja); X = sembunyikan.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#![cfg_attr(not(windows), allow(dead_code))]

mod icon;
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
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    Icon as TrayImage, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use wry::{NewWindowResponse, PageLoadEvent, WebContext, WebView, WebViewBuilder};

use icon::Badge;
use settings::{Mode, Settings, WindowGeom, HEMAT_INTERVALS};
use urlutil::{is_microsoft_url, is_outlook_url, CALENDAR_URL, COMPOSE_URL, MAIL_URL};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTANCE_PORT: u16 = 47193;
const MAGIC: &str = "OUTLOOKLITE1";
/// Argumen bawaan wry yang harus dipertahankan saat kita menambah argumen sendiri.
const WRY_DEFAULT_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
/// Jangan perlambat halaman tersembunyi (penting untuk notifikasi di mode Siaga).
const BG_ARGS: &str = "--disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";
const MONITOR_JS: &str = include_str!("js/monitor.js");
const INSPECT_JS: &str = include_str!("js/inspect.js");

const TICK: Duration = Duration::from_secs(5);
const BG_CHECK_TIMEOUT: Duration = Duration::from_secs(75);
const INSPECT_TIMEOUT: Duration = Duration::from_secs(90);
const CREATE_RETRY: Duration = Duration::from_secs(300);
const HOOK_SUPPRESS: Duration = Duration::from_secs(20);
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 3600);
const UPDATE_FIRST_DELAY: Duration = Duration::from_secs(90);
const DIAG_EVERY: Duration = Duration::from_secs(10);
/// Halaman login yang bertahan lebih lama dari ini (bukan sekadar redirect SSO) = perlu login.
const LOGIN_GRACE: Duration = Duration::from_secs(40);

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
}

enum RemoteCmd {
    Show,
    Open(String),
}

enum PageMsg {
    State { unread: i64, composing: bool },
    Notify { id: u64, title: String, body: String },
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
        }),
        "notify" => Some(PageMsg::Notify {
            id: v.get("id")?.as_u64()?,
            title: s("title", 300),
            body: s("body", 600),
        }),
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
    mailto: Option<String>,
}

fn parse_args() -> Args {
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut a = Args {
        tray: false,
        compose: false,
        restarted: false,
        mailto: None,
    };
    let mut i = 0;
    while i < v.len() {
        match v[i].as_str() {
            "--tray" => a.tray = true,
            "--compose" => a.compose = true,
            "--restarted" => a.restarted = true,
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
    pub const MODE_MATI: &str = "mode_mati";
    pub const MODE_HEMAT: &str = "mode_hemat";
    pub const MODE_SIAGA: &str = "mode_siaga";
    pub const INTERVAL: &str = "interval:";
    pub const SHORTCUT: &str = "shortcut:";
    pub const WORK_HOURS: &str = "work_hours";
    pub const SOUND: &str = "sound";
    pub const DETAILS: &str = "details";
    pub const AUTO_CLOSE: &str = "auto_close";
    pub const LOW_MEM: &str = "low_mem";
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
    pub const RELOAD: &str = "reload";
    pub const RESTART_UPDATE: &str = "restart_update";
    pub const CLOSE_WINDOW: &str = "close_window";
    pub const QUIT: &str = "quit";
}

struct TrayMenu {
    status: MenuItem,
    modes: [(Mode, CheckMenuItem); 3],
    intervals: Vec<(u32, CheckMenuItem)>,
    work_hours: CheckMenuItem,
    sound: CheckMenuItem,
    details: CheckMenuItem,
    toast_compat: CheckMenuItem,
    auto_close: CheckMenuItem,
    low_mem: CheckMenuItem,
    autostart: CheckMenuItem,
    hotkeys: CheckMenuItem,
    mailto: CheckMenuItem,
    auto_update: CheckMenuItem,
    diag: CheckMenuItem,
    restart_update: MenuItem,
}

fn build_menu(s: &Settings) -> Result<(Menu, TrayMenu), tray_icon::menu::Error> {
    let check = |i: &str, t: &str, c: bool| CheckMenuItem::with_id(i, t, true, c, None);
    let item = |i: &str, t: &str| MenuItem::with_id(i, t, true, None);
    let sep = PredefinedMenuItem::separator;

    let status = MenuItem::with_id("status", "Outlook Lite", false, None);
    let modes = [
        (Mode::Mati, check(id::MODE_MATI, "Mode Mati (tanpa notifikasi)", false)),
        (Mode::Hemat, check(id::MODE_HEMAT, "Mode Hemat (cek berkala)", false)),
        (Mode::Siaga, check(id::MODE_SIAGA, "Mode Siaga (hampir real-time)", false)),
    ];
    let intervals: Vec<(u32, CheckMenuItem)> = HEMAT_INTERVALS
        .iter()
        .map(|m| (*m, check(&format!("{}{m}", id::INTERVAL), &format!("{m} menit"), false)))
        .collect();
    let work_hours = check(
        id::WORK_HOURS,
        &format!("Hanya jam kerja ({})", s.work_hours_label()),
        false,
    );
    let sound = check(id::SOUND, "Bunyi notifikasi", false);
    let details = check(id::DETAILS, "Tampilkan pengirim && subjek", false);
    let toast_compat = check(
        id::TOAST_COMPAT,
        "Mode kompatibel (jika notifikasi tidak muncul)",
        false,
    );
    let auto_close = check(
        id::AUTO_CLOSE,
        &format!("Tutup otomatis saat tidak aktif ({} menit)", s.auto_close_min),
        false,
    );
    let low_mem = check(id::LOW_MEM, "Hemat memori WebView2", false);
    let autostart = check(id::AUTOSTART, "Mulai dengan Windows", false);
    let hotkeys = check(
        id::HOTKEYS,
        &format!(
            "Hotkey ({} buka/tutup, {} email baru)",
            s.hotkey_toggle, s.hotkey_compose
        ),
        false,
    );
    let mailto = check(id::MAILTO, "Aplikasi email default (mailto)", false);
    let auto_update = check(id::AUTO_UPDATE, "Update otomatis", false);
    let diag = check(id::DIAG_RAM, "Diagnostik RAM", false);
    let restart_update = MenuItem::with_id(id::RESTART_UPDATE, "Tidak ada update", false, None);

    let interval_refs: Vec<&dyn tray_icon::menu::IsMenuItem> =
        intervals.iter().map(|(_, c)| c as &dyn tray_icon::menu::IsMenuItem).collect();
    let interval_sub = Submenu::with_items("Interval mode Hemat", true, &interval_refs)?;

    let notif = Submenu::with_items(
        "Notifikasi",
        true,
        &[
            &modes[0].1,
            &modes[1].1,
            &modes[2].1,
            &sep(),
            &interval_sub,
            &work_hours,
            &sound,
            &details,
            &sep(),
            &item(id::TEST_TOAST, "Tes notifikasi"),
            &toast_compat,
        ],
    )?;
    let window = Submenu::with_items(
        "Jendela",
        true,
        &[
            &auto_close,
            &low_mem,
            &sep(),
            &item(id::ZOOM_IN, "Perbesar tampilan"),
            &item(id::ZOOM_OUT, "Perkecil tampilan"),
            &item(id::ZOOM_RESET, "Ukuran tampilan 100%"),
        ],
    )?;
    let prefs = Submenu::with_items(
        "Pengaturan",
        true,
        &[
            &autostart,
            &hotkeys,
            &mailto,
            &auto_update,
            &item(id::CHECK_UPDATE, "Cek update sekarang"),
            &diag,
            &sep(),
            &item(id::INSPECT, "Diagnostik halaman Outlook"),
            &item(id::OPEN_FOLDER, "Buka folder pengaturan"),
            &item(id::RELOAD, "Muat ulang pengaturan"),
            &MenuItem::new(format!("Outlook Lite v{VERSION}"), false, None),
        ],
    )?;

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
        &notif,
        &window,
        &prefs,
        &sep(),
        &restart_update,
        &item(id::CLOSE_WINDOW, "Tutup jendela"),
        &item(id::QUIT, "Keluar"),
    ])?;

    Ok((
        menu,
        TrayMenu {
            status,
            modes,
            intervals,
            work_hours,
            sound,
            details,
            toast_compat,
            auto_close,
            low_mem,
            autostart,
            hotkeys,
            mailto,
            auto_update,
            diag,
            restart_update,
        },
    ))
}

// ---------------------------------------------------------------------------
// Aplikasi

struct Instance {
    window: Window,
    webview: WebView,
    visible: bool,
    minimized: bool,
    suspended: bool,
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
    last_hook: Option<Instant>,
    needs_login: bool,
    login_toasted: bool,
    login_seen_at: Option<Instant>,
    last_tip: String,
    last_title: String,

    next_tick: Instant,
    started: Instant,
    last_mode: Mode,
    next_hemat: Option<Instant>,
    bg_deadline: Option<Instant>,
    manual_check: bool,
    inspect_pending: bool,
    inspect_sent: bool,
    create_failed_at: Option<Instant>,

    hk_manager: Option<GlobalHotKeyManager>,
    hk_registered: Vec<HotKey>,
    hk_toggle: Option<u32>,
    hk_compose: Option<u32>,

    ram: Option<(u64, u32)>,
    last_diag: Option<Instant>,

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
        let proxy = self.proxy.clone();
        let cb: win::Callback = Arc::new(move || {
            let _ = proxy.send_event(UserEvent::ToastClick(click));
        });
        win::toast(
            &win::ToastSpec {
                title: title.to_string(),
                body: body.to_string(),
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
        if self.settings.mode != Mode::Mati && active == Mode::Mati {
            parts.push(format!("{} (di luar jam kerja)", self.settings.mode.label()));
        } else {
            parts.push(format!("Mode {}", active.label()));
        }
        if self.needs_login {
            parts.push("perlu login".into());
        } else if let Some(n) = self.unread {
            parts.push(if n > 0 {
                format!("{n} belum dibaca")
            } else {
                "tidak ada yang belum dibaca".into()
            });
        }
        if let Some((bytes, count)) = self.ram {
            parts.push(format!("RAM {} MB ({count} proses)", bytes / (1024 * 1024)));
        }
        parts.join(" • ")
    }

    fn refresh_tray(&mut self) {
        let badge = if self.needs_login {
            Badge::Warning
        } else if self.unread.unwrap_or(0) > 0 {
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
        if let Some(i) = &self.inst {
            let n = self.unread.unwrap_or(0);
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
        let s = &self.settings;
        for (mode, item) in &m.modes {
            item.set_checked(*mode == s.mode);
        }
        for (min, item) in &m.intervals {
            item.set_checked(*min == s.hemat_interval_min);
        }
        m.work_hours.set_checked(s.work_hours_only);
        m.sound.set_checked(s.sound);
        m.details.set_checked(s.show_details);
        m.toast_compat.set_checked(s.toast_compat);
        m.auto_close.set_checked(s.auto_close);
        m.low_mem.set_checked(s.low_memory);
        m.autostart.set_checked(s.autostart);
        m.hotkeys.set_checked(s.hotkeys);
        m.mailto.set_checked(win::mailto_registered());
        m.auto_update.set_checked(s.auto_update);
        m.diag.set_checked(s.diagnostics);
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
        for (text, is_toggle) in [
            (self.settings.hotkey_toggle.clone(), true),
            (self.settings.hotkey_compose.clone(), false),
        ] {
            if text.trim().is_empty() {
                continue;
            }
            match text.parse::<HotKey>() {
                Ok(hk) => match m.register(hk) {
                    Ok(()) => {
                        if is_toggle {
                            self.hk_toggle = Some(hk.id());
                        } else {
                            self.hk_compose = Some(hk.id());
                        }
                        self.hk_registered.push(hk);
                    }
                    Err(e) => errors.push(format!("{text}: {e}")),
                },
                Err(e) => errors.push(format!("{text}: {e}")),
            }
        }
        if !errors.is_empty() {
            let msg = errors.join("; ");
            self.log(&format!("hotkey: {msg}"));
            self.toast(
                "Hotkey tidak bisa dipasang",
                &format!("{msg}. Mungkin sudah dipakai aplikasi lain; ubah di settings.json."),
                None,
                Some(false),
            );
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

        let cfg = serde_json::json!({ "sel": self.settings.unread_selector });
        let init_js = format!("window.__oliteCfg = {cfg};\n{MONITOR_JS}");
        let zoom = self.settings.zoom;
        let low = self.settings.low_memory;
        #[cfg(windows)]
        let args = self.browser_args.clone();
        let proxy_ipc = self.proxy.clone();
        let proxy_load = self.proxy.clone();

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
            builder.with_additional_browser_args(args)
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
        if !visible && low {
            set_memory_low(&webview, true);
        }
        self.create_failed_at = None;
        self.focused = false;
        self.unfocused_since = visible.then(Instant::now);
        self.inst = Some(Instance {
            window,
            webview,
            visible,
            minimized: false,
            suspended: false,
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
    }

    fn show(&mut self, target: &EventLoopWindowTarget<UserEvent>, url: Option<&str>) {
        self.bg_deadline = None;
        if let Some(inst) = self.inst.as_mut() {
            if let Some(u) = url {
                let _ = inst.webview.load_url(u);
            }
            if !inst.visible {
                inst.window.set_visible(true);
                inst.visible = true;
                if self.settings.window.is_some_and(|g| g.maximized) {
                    inst.window.set_maximized(true);
                }
            }
            if inst.window.is_minimized() {
                inst.window.set_minimized(false);
            }
            if inst.suspended {
                let _ = inst.webview.set_visible(true);
                inst.suspended = false;
            }
            inst.minimized = false;
            set_memory_low(&inst.webview, false);
            inst.window.set_focus();
            self.unfocused_since = Some(Instant::now());
            self.refresh_tray();
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
        if self.active_mode() == Mode::Siaga {
            if let Some(inst) = self.inst.as_mut() {
                inst.window.set_visible(false);
                inst.visible = false;
                // Pemantauan hanya jalan di halaman Outlook (bukan pintasan seperti To Do).
                if !inst.webview.url().is_ok_and(|u| is_outlook_url(&u)) {
                    let _ = inst.webview.load_url(MAIL_URL);
                }
                if self.settings.low_memory {
                    set_memory_low(&inst.webview, true);
                }
            }
            self.focused = false;
            self.unfocused_since = None;
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
        let siaga = self.active_mode() == Mode::Siaga;
        let low = self.settings.low_memory;
        let Some(inst) = self.inst.as_mut() else { return };
        if inst.window.is_minimized() {
            if !inst.minimized {
                inst.minimized = true;
                if low {
                    set_memory_low(&inst.webview, true);
                    // Di Siaga halaman harus tetap aktif untuk notifikasi.
                    if !siaga {
                        let _ = inst.webview.set_visible(false);
                        win::try_suspend(&inst.webview);
                        inst.suspended = true;
                    }
                }
            }
        } else {
            if inst.minimized {
                inst.minimized = false;
                if inst.suspended {
                    let _ = inst.webview.set_visible(true); // otomatis melanjutkan halaman
                    inst.suspended = false;
                }
                set_memory_low(&inst.webview, false);
            }
            self.record_geom();
        }
    }

    // ----- cek email -----

    fn start_bg_check(&mut self, target: &EventLoopWindowTarget<UserEvent>, manual: bool) {
        if manual {
            self.manual_check = true;
        }
        if let Some(inst) = &self.inst {
            // Outlook sudah hidup: minta pemantau melapor sekarang juga.
            if manual {
                let _ = inst
                    .webview
                    .evaluate_script("window.__oliteForce && window.__oliteForce()");
                self.bg_deadline
                    .get_or_insert(Instant::now() + Duration::from_secs(20));
            }
            return;
        }
        if self.create_instance(target, MAIL_URL, false) {
            self.bg_deadline = Some(Instant::now() + BG_CHECK_TIMEOUT);
        } else {
            self.manual_check = false;
        }
    }

    fn report_manual(&self, n: i64) {
        let body = if n > 0 {
            format!("{n} email belum dibaca di Inbox.")
        } else {
            "Tidak ada email belum dibaca di Inbox.".into()
        };
        self.toast("Outlook Lite", &body, None, Some(false));
    }

    fn finish_bg_check(&mut self) {
        self.bg_deadline = None;
        let hidden = self.inst.as_ref().is_some_and(|i| !i.visible);
        if hidden && self.active_mode() != Mode::Siaga && !self.inspect_pending {
            self.destroy_instance();
        }
    }

    fn on_state(&mut self, unread: i64, composing: bool) {
        self.composing = composing;
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
        if unread >= 0 {
            let prev = self.unread;
            self.unread = Some(unread);
            if let Some(p) = prev {
                let hook_recent = self.last_hook.is_some_and(|t| t.elapsed() < HOOK_SUPPRESS);
                if unread > p && !hook_recent && !self.user_looking() {
                    let new = unread - p;
                    let title = if new == 1 {
                        "1 email baru".to_string()
                    } else {
                        format!("{new} email baru")
                    };
                    self.toast(
                        &title,
                        &format!("Total {unread} belum dibaca. Klik untuk membuka."),
                        None,
                        None,
                    );
                }
            }
            if self.manual_check {
                self.manual_check = false;
                self.report_manual(unread);
            }
            if self.bg_deadline.is_some() && !self.inspect_pending {
                self.finish_bg_check();
            }
        }
        self.refresh_tray();
    }

    fn on_notify(&mut self, id: u64, title: String, body: String) {
        self.last_hook = Some(Instant::now());
        if self.user_looking() {
            return;
        }
        let (t, b) = if self.settings.show_details {
            (
                if title.is_empty() { "Outlook".to_string() } else { title },
                body,
            )
        } else {
            (
                "Outlook".to_string(),
                "Ada email baru atau pengingat rapat. Klik untuk membuka.".to_string(),
            )
        };
        self.toast(&t, &b, Some(id), None);
    }

    fn on_page_loaded(&mut self, url: String) {
        if is_outlook_url(&url) {
            self.login_seen_at = None; // status login dibersihkan saat pemantau melapor
            return;
        }
        // Di jendela terlihat, halaman lain bisa saja pintasan (To Do, intranet).
        // Di instance tersembunyi kita hanya memuat Outlook, jadi halaman lain = login/redirect.
        let hidden = self.inst.as_ref().is_some_and(|i| !i.visible);
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
        if !self.login_toasted {
            self.login_toasted = true;
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
            Ok(_) => self.quit(),
            Err(e) => self.log(&format!("restart gagal: {e}")),
        }
    }

    fn quit(&mut self) {
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
        let interval = Duration::from_secs(self.settings.hemat_interval_min as u64 * 60);

        if mode != self.last_mode {
            self.last_mode = mode;
            self.next_hemat = None;
        }

        match mode {
            Mode::Siaga => {
                let can_retry = self
                    .create_failed_at
                    .is_none_or(|t| t.elapsed() >= CREATE_RETRY);
                if self.inst.is_none() && can_retry {
                    self.create_instance(target, MAIL_URL, false);
                }
            }
            Mode::Hemat => {
                let due = *self.next_hemat.get_or_insert(now + interval);
                if now >= due {
                    self.next_hemat = Some(now + interval);
                    self.start_bg_check(target, false);
                }
            }
            Mode::Mati => {}
        }

        self.check_login_stuck();

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
        if hidden && mode != Mode::Siaga && self.bg_deadline.is_none() && !self.inspect_pending {
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

        // Diagnostik RAM.
        if self.settings.diagnostics {
            if self.last_diag.is_none_or(|t| t.elapsed() >= DIAG_EVERY) {
                self.last_diag = Some(now);
                self.ram = win::process_tree_memory();
            }
        } else {
            self.ram = None;
        }

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
            id::MODE_MATI => self.settings.mode = Mode::Mati,
            id::MODE_HEMAT => self.settings.mode = Mode::Hemat,
            id::MODE_SIAGA => self.settings.mode = Mode::Siaga,
            id::WORK_HOURS => self.settings.work_hours_only = !self.settings.work_hours_only,
            id::SOUND => self.settings.sound = !self.settings.sound,
            id::DETAILS => self.settings.show_details = !self.settings.show_details,
            id::TOAST_COMPAT => self.settings.toast_compat = !self.settings.toast_compat,
            id::TEST_TOAST => {
                changed = false;
                self.toast(
                    "Tes notifikasi Outlook Lite",
                    "Jika Anda melihat ini, notifikasi berfungsi. Klik untuk membuka Outlook.",
                    None,
                    None,
                );
            }
            id::AUTO_CLOSE => self.settings.auto_close = !self.settings.auto_close,
            id::LOW_MEM => self.settings.low_memory = !self.settings.low_memory,
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
                self.last_diag = None;
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
            id::QUIT => {
                changed = false;
                self.quit()
            }
            other => {
                changed = false;
                if let Some(m) = other.strip_prefix(id::INTERVAL) {
                    if let Ok(m) = m.parse::<u32>() {
                        self.settings.hemat_interval_min = m;
                        self.next_hemat = None;
                        changed = true;
                    }
                } else if let Some(i) = other.strip_prefix(id::SHORTCUT) {
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
                window_id, event, ..
            } if self.is_our_window(window_id) => match event {
                WindowEvent::CloseRequested => self.close_window(),
                WindowEvent::Focused(f) => {
                    self.focused = f;
                    self.unfocused_since = if f { None } else { Some(Instant::now()) };
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
                UserEvent::Page(PageMsg::State { unread, composing }) => {
                    self.on_state(unread, composing)
                }
                UserEvent::Page(PageMsg::Notify { id, title, body }) => {
                    self.on_notify(id, title, body)
                }
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
    update::cleanup_old();

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
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("outlook-lite.exe"));

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

    let mut browser_args = format!("{WRY_DEFAULT_ARGS} {BG_ARGS}");
    if settings.no_gpu {
        browser_args.push_str(" --disable-gpu");
    }
    if !settings.extra_browser_args.trim().is_empty() {
        browser_args.push(' ');
        browser_args.push_str(settings.extra_browser_args.trim());
    }

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
        last_hook: None,
        needs_login: false,
        login_toasted: false,
        login_seen_at: None,
        last_tip: String::new(),
        last_title: String::new(),
        next_tick: Instant::now() + TICK,
        started: Instant::now(),
        last_mode: Mode::Mati,
        next_hemat: None,
        bg_deadline: None,
        manual_check: false,
        inspect_pending: false,
        inspect_sent: false,
        create_failed_at: None,
        hk_manager: None,
        hk_registered: Vec::new(),
        hk_toggle: None,
        hk_compose: None,
        ram: None,
        last_diag: None,
        update_running: false,
        last_update_check: None,
        update_ready: None,
        initial_url,
        exiting: false,
    };

    event_loop.run(move |event, target, control_flow| app.handle(event, target, control_flow));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_messages() {
        assert!(matches!(
            parse_page_msg(r#"{"t":"state","unread":3,"composing":true}"#),
            Some(PageMsg::State { unread: 3, composing: true })
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
            mailto: Some("mailto:x@y.z".into()),
        };
        assert!(urlutil::mailto_to_compose(a.mailto.as_deref().unwrap()).is_some());
    }
}
