//! Fungsi khusus Windows. Semuanya bekerja di level pengguna (HKCU), tanpa hak admin.
//! Di platform lain fungsi-fungsi ini berupa stub supaya proyek tetap bisa dicek.

use std::path::Path;
use std::sync::Arc;

pub const AUMID: &str = "OutlookLite.App";
pub const APP_REG_NAME: &str = "OutlookLite";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub type Callback = Arc<dyn Fn() + Send + Sync>;

/// Memori proses aplikasi + semua proses WebView2 turunannya.
#[derive(Clone, Copy, Debug, Default)]
pub struct TreeMem {
    /// Private working set (kolom Memory di Task Manager), byte.
    pub working: u64,
    /// Private bytes / commit (memori yang benar-benar dialokasikan, termasuk yang di page file), byte.
    pub commit: u64,
    pub count: u32,
}

pub struct ToastSpec {
    pub title: String,
    pub body: String,
    /// Baris kedua (kosong = tidak ada).
    pub body2: String,
    pub sound: bool,
    /// Pakai identitas PowerShell yang selalu terdaftar di Windows.
    pub compat: bool,
    pub on_click: Option<Callback>,
}

/// Buka URL http/https di browser default. Skema lain ditolak.
pub fn open_url(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    spawn_hidden("rundll32", &["url.dll,FileProtocolHandler", url]);
}

pub fn open_folder(path: &Path) {
    let _ = std::process::Command::new(if cfg!(windows) { "explorer" } else { "xdg-open" })
        .arg(path)
        .spawn();
}

/// Buka Explorer dengan file ini terpilih.
pub fn reveal_file(path: &Path) {
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.raw_arg(format!("/select,\"{}\"", path.display()));
    }
    #[cfg(not(windows))]
    cmd.arg(path.parent().unwrap_or(path));
    let _ = cmd.spawn();
}

/// Hapus folder-folder ini beberapa detik setelah proses ini keluar.
pub fn delete_dirs_later(dirs: &[&Path]) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut script = String::from("/c ping 127.0.0.1 -n 4 >nul");
        for d in dirs {
            script.push_str(&format!(" & rmdir /s /q \"{}\"", d.display()));
        }
        let _ = std::process::Command::new("cmd")
            .raw_arg(script)
            .creation_flags(0x0800_0000)
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = dirs;
}

pub fn open_in_notepad(path: &Path) {
    let _ = std::process::Command::new(if cfg!(windows) { "notepad" } else { "xdg-open" })
        .arg(path)
        .spawn();
}

/// Halaman "Aplikasi default" Windows, langsung ke entri Outlook Lite bila didukung.
pub fn open_default_apps_settings() {
    let _ = std::process::Command::new("explorer")
        .arg(format!("ms-settings:defaultapps?registeredAppUser={APP_REG_NAME}"))
        .spawn();
}

fn spawn_hidden(program: &str, args: &[&str]) {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let _ = cmd.spawn();
}

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    fn message_box(text: &str, flags: windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE) -> i32 {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_SETFOREGROUND, MB_TOPMOST};
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(text),
                &HSTRING::from("Outlook Lite"),
                flags | MB_TOPMOST | MB_SETFOREGROUND,
            )
            .0
        }
    }

    pub fn confirm(text: &str) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONQUESTION, MB_YESNO};
        message_box(text, MB_YESNO | MB_ICONQUESTION) == IDYES.0
    }

    pub fn info(text: &str) {
        use windows::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK};
        message_box(text, MB_OK | MB_ICONINFORMATION);
    }

    fn hkcu() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    // ---------- identitas aplikasi untuk notifikasi ----------

    pub fn init_identity(icon_png: Option<&Path>) {
        use windows::core::HSTRING;
        use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
        if let Ok((k, _)) = hkcu().create_subkey(format!(r"Software\Classes\AppUserModelId\{AUMID}")) {
            let _ = k.set_value("DisplayName", &"Outlook Lite".to_string());
            if let Some(p) = icon_png {
                let _ = k.set_value("IconUri", &p.display().to_string());
            }
        }
        let _ = unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID)) };
    }

    pub fn toast(spec: &ToastSpec, icon: Option<&Path>) {
        use tauri_winrt_notification::{IconCrop, Sound, Toast};
        let build = |app_id: &str| {
            let mut t = Toast::new(app_id).title(&spec.title).text1(&spec.body);
            if !spec.body2.is_empty() {
                t = t.text2(&spec.body2);
            }
            t = t.sound(if spec.sound { Some(Sound::Mail) } else { None });
            if let Some(p) = icon {
                t = t.icon(p, IconCrop::Square, "");
            }
            if let Some(cb) = spec.on_click.clone() {
                t = t.on_activated(move |_| {
                    cb();
                    Ok(())
                });
            }
            t
        };
        if spec.compat {
            let _ = build(Toast::POWERSHELL_APP_ID).show();
        } else if build(AUMID).show().is_err() {
            // Cadangan: pakai identitas PowerShell yang selalu terdaftar.
            let _ = build(Toast::POWERSHELL_APP_ID).show();
        }
    }

    // ---------- mulai otomatis ----------

    pub fn set_autostart(enabled: bool, exe: &Path) -> std::io::Result<()> {
        let (run, _) = hkcu().create_subkey(RUN_KEY)?;
        if enabled {
            run.set_value(APP_REG_NAME, &format!("\"{}\" --tray", exe.display()))
        } else {
            match run.delete_value(APP_REG_NAME) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                r => r,
            }
        }
    }

    pub fn autostart_value() -> Option<String> {
        hkcu().open_subkey(RUN_KEY).ok()?.get_value(APP_REG_NAME).ok()
    }

    // ---------- mailto ----------

    pub fn register_mailto(exe: &Path) -> std::io::Result<()> {
        let h = hkcu();
        let exe_s = exe.display().to_string();
        let (prog, _) = h.create_subkey(r"Software\Classes\OutlookLite.mailto")?;
        prog.set_value("", &"Outlook Lite mailto".to_string())?;
        prog.set_value("FriendlyTypeName", &"Outlook Lite".to_string())?;
        let (icon, _) = prog.create_subkey("DefaultIcon")?;
        icon.set_value("", &format!("\"{exe_s}\",0"))?;
        let (cmd, _) = prog.create_subkey(r"shell\open\command")?;
        cmd.set_value("", &format!("\"{exe_s}\" --mailto \"%1\""))?;

        let (caps, _) = h.create_subkey(r"Software\OutlookLite\Capabilities")?;
        caps.set_value("ApplicationName", &"Outlook Lite".to_string())?;
        caps.set_value(
            "ApplicationDescription",
            &"Outlook web ringan di tray".to_string(),
        )?;
        let (urls, _) = caps.create_subkey("URLAssociations")?;
        urls.set_value("mailto", &"OutlookLite.mailto".to_string())?;

        let (reg, _) = h.create_subkey(r"Software\RegisteredApplications")?;
        reg.set_value(APP_REG_NAME, &r"Software\OutlookLite\Capabilities".to_string())
    }

    pub fn unregister_mailto() {
        let h = hkcu();
        if let Ok(reg) = h.open_subkey_with_flags(
            r"Software\RegisteredApplications",
            winreg::enums::KEY_SET_VALUE,
        ) {
            let _ = reg.delete_value(APP_REG_NAME);
        }
        let _ = h.delete_subkey_all(r"Software\OutlookLite");
        let _ = h.delete_subkey_all(r"Software\Classes\OutlookLite.mailto");
    }

    pub fn mailto_registered() -> bool {
        hkcu()
            .open_subkey(r"Software\RegisteredApplications")
            .and_then(|k| k.get_value::<String, _>(APP_REG_NAME))
            .is_ok()
    }

    /// Perbarui jalur exe di registri bila exe dipindah.
    pub fn refresh_registrations(exe: &Path) {
        let want = format!("\"{}\" --tray", exe.display());
        if let Some(v) = autostart_value() {
            if v != want {
                let _ = set_autostart(true, exe);
            }
        }
        if mailto_registered() {
            let _ = register_mailto(exe);
        }
    }

    // ---------- diagnostik RAM ----------

    /// PID proses ini + semua proses WebView2 turunannya. Program lain yang dibuka aplikasi
    /// (Notepad, Explorer, browser untuk tautan) tidak ikut dihitung.
    fn tree_pids() -> Vec<u32> {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };
        use windows::Win32::System::Threading::GetCurrentProcessId;

        let mut pairs: Vec<(u32, u32)> = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return Vec::new();
            };
            let mut e = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snap, &mut e).is_ok() {
                loop {
                    let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                    let name = String::from_utf16_lossy(&e.szExeFile[..len]);
                    if name.eq_ignore_ascii_case("msedgewebview2.exe") {
                        pairs.push((e.th32ProcessID, e.th32ParentProcessID));
                    }
                    if Process32NextW(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
        }

        let me = unsafe { GetCurrentProcessId() };
        let mut tree = vec![me];
        let mut i = 0;
        while i < tree.len() {
            let parent = tree[i];
            for &(pid, ppid) in &pairs {
                if ppid == parent && pid != parent && !tree.contains(&pid) {
                    tree.push(pid);
                }
            }
            i += 1;
        }
        tree
    }

    /// Snapshot RAM dan waktu CPU proses ini + semua turunannya (proses WebView2).
    /// `kinds` = jenis proses menurut WebView2 (PID yang tidak ada di sana = Lain).
    pub fn process_snapshot(
        kinds: &std::collections::HashMap<u32, crate::diag::Kind>,
    ) -> Vec<crate::diag::Proc> {
        use windows::Win32::Foundation::{CloseHandle, FILETIME};
        use windows::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
        };
        use windows::Win32::System::Threading::{
            GetCurrentProcessId, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let me = unsafe { GetCurrentProcessId() };
        let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
        let mut out = Vec::new();
        for pid in tree_pids() {
            unsafe {
                let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                    continue;
                };
                let mut p = crate::diag::Proc {
                    pid,
                    kind: if pid == me {
                        Some(crate::diag::Kind::Aplikasi)
                    } else {
                        kinds.get(&pid).copied()
                    },
                    ..Default::default()
                };
                let mut pmc = PROCESS_MEMORY_COUNTERS_EX2 {
                    cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
                    ..Default::default()
                };
                if GetProcessMemoryInfo(
                    h,
                    &mut pmc as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
                    pmc.cb,
                )
                .is_ok()
                {
                    p.working = if pmc.PrivateWorkingSetSize > 0 {
                        pmc.PrivateWorkingSetSize
                    } else {
                        pmc.WorkingSetSize
                    } as u64;
                    p.commit = pmc.PrivateUsage as u64;
                }
                let (mut c, mut e, mut k, mut u) = (
                    FILETIME::default(),
                    FILETIME::default(),
                    FILETIME::default(),
                    FILETIME::default(),
                );
                if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u).is_ok() {
                    p.cpu_time = ft(k) + ft(u);
                }
                let _ = CloseHandle(h);
                out.push(p);
            }
        }
        out
    }

    /// Memori proses ini + semua turunannya. `working` sama dengan kolom Memory di Task Manager.
    pub fn process_tree_memory() -> Option<TreeMem> {
        let procs = process_snapshot(&Default::default());
        if procs.is_empty() {
            return None;
        }
        Some(TreeMem {
            working: procs.iter().map(|p| p.working).sum(),
            commit: procs.iter().map(|p| p.commit).sum(),
            count: procs.len() as u32,
        })
    }

    /// Jenis tiap proses WebView2 (browser, renderer, GPU, utility) menurut WebView2 sendiri.
    pub fn webview_process_kinds(
        webview: &wry::WebView,
    ) -> std::collections::HashMap<u32, crate::diag::Kind> {
        use crate::diag::Kind;
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2Environment8, ICoreWebView2_2, COREWEBVIEW2_PROCESS_KIND,
            COREWEBVIEW2_PROCESS_KIND_BROWSER, COREWEBVIEW2_PROCESS_KIND_GPU,
            COREWEBVIEW2_PROCESS_KIND_RENDERER, COREWEBVIEW2_PROCESS_KIND_UTILITY,
        };
        use windows::core::Interface;
        use wry::WebViewExtWindows;
        let mut map = std::collections::HashMap::new();
        let res: windows::core::Result<()> = (|| unsafe {
            let wv2: ICoreWebView2_2 = webview.webview().cast()?;
            let env: ICoreWebView2Environment8 = wv2.Environment()?.cast()?;
            let list = env.GetProcessInfos()?;
            let mut n = 0u32;
            list.Count(&mut n)?;
            for i in 0..n {
                let info = list.GetValueAtIndex(i)?;
                let mut pid = 0i32;
                let mut kind = COREWEBVIEW2_PROCESS_KIND::default();
                info.ProcessId(&mut pid)?;
                info.Kind(&mut kind)?;
                let k = match kind {
                    COREWEBVIEW2_PROCESS_KIND_BROWSER => Kind::Browser,
                    COREWEBVIEW2_PROCESS_KIND_RENDERER => Kind::Renderer,
                    COREWEBVIEW2_PROCESS_KIND_GPU => Kind::Gpu,
                    COREWEBVIEW2_PROCESS_KIND_UTILITY => Kind::Utility,
                    _ => Kind::Lain,
                };
                map.insert(pid as u32, k);
            }
            Ok(())
        })();
        let _ = res;
        map
    }

    /// Mode efisiensi untuk aplikasi + proses WebView2 saat Outlook tersembunyi/diminimalkan:
    /// EcoQoS (daun hijau "Efficiency mode" di Task Manager), prioritas CPU di bawah normal,
    /// dan prioritas memori rendah (Windows memindahkan halaman ini lebih dulu bila RAM perlu).
    /// `false` mengembalikan ke normal. Mengembalikan jumlah proses yang berhasil diatur.
    pub fn set_background_mode(background: bool) -> u32 {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, ProcessMemoryPriority, ProcessPowerThrottling, SetPriorityClass,
            SetProcessInformation, BELOW_NORMAL_PRIORITY_CLASS, MEMORY_PRIORITY_INFORMATION,
            MEMORY_PRIORITY_LOW, MEMORY_PRIORITY_NORMAL, NORMAL_PRIORITY_CLASS,
            PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            PROCESS_POWER_THROTTLING_STATE, PROCESS_SET_INFORMATION,
        };
        let throttle = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            // Saat normal: ControlMask 0 = serahkan ke Windows (bawaan).
            ControlMask: if background { PROCESS_POWER_THROTTLING_EXECUTION_SPEED } else { 0 },
            StateMask: if background { PROCESS_POWER_THROTTLING_EXECUTION_SPEED } else { 0 },
        };
        let mem = MEMORY_PRIORITY_INFORMATION {
            MemoryPriority: if background { MEMORY_PRIORITY_LOW } else { MEMORY_PRIORITY_NORMAL },
        };
        let class = if background { BELOW_NORMAL_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS };
        let mut n = 0;
        for pid in tree_pids() {
            unsafe {
                let Ok(h) = OpenProcess(PROCESS_SET_INFORMATION, false, pid) else {
                    continue;
                };
                let _ = SetPriorityClass(h, class);
                let _ = SetProcessInformation(
                    h,
                    ProcessPowerThrottling,
                    &throttle as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
                );
                if SetProcessInformation(
                    h,
                    ProcessMemoryPriority,
                    &mem as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
                )
                .is_ok()
                {
                    n += 1;
                }
                let _ = CloseHandle(h);
            }
        }
        n
    }

    /// Pengaturan WebView2 yang tidak dibutuhkan Outlook dimatikan (autofill formulir,
    /// simpan kata sandi), dan permintaan telemetri ke `blocked_hosts` dijawab langsung
    /// "204 No Content" tanpa keluar ke internet.
    pub fn tune_webview(webview: &wry::WebView, blocked_hosts: &'static [&'static str]) {
        use webview2_com::{
            take_pwstr,
            Microsoft::Web::WebView2::Win32::{
                ICoreWebView2Settings4, ICoreWebView2_2, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            },
            WebResourceRequestedEventHandler,
        };
        use windows::core::{w, Interface, HSTRING, PWSTR};
        use wry::WebViewExtWindows;
        let wv = webview.webview();
        unsafe {
            if let Ok(st) = wv.Settings().and_then(|s| s.cast::<ICoreWebView2Settings4>()) {
                let _ = st.SetIsGeneralAutofillEnabled(false);
                let _ = st.SetIsPasswordAutosaveEnabled(false);
            }
            if blocked_hosts.is_empty() {
                return;
            }
            for host in blocked_hosts {
                let filter = HSTRING::from(format!("https://{host}/*"));
                let _ = wv.AddWebResourceRequestedFilter(&filter, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);
            }
            let handler = WebResourceRequestedEventHandler::create(Box::new(move |sender, args| {
                let (Some(sender), Some(args)) = (sender, args) else {
                    return Ok(());
                };
                let mut uri = PWSTR::null();
                args.Request()?.Uri(&mut uri)?;
                let uri = take_pwstr(uri);
                let host = crate::urlutil::host_of(&uri).unwrap_or_default();
                if !crate::urlutil::host_matches_pattern(&host, blocked_hosts) {
                    return Ok(());
                }
                let env = sender.cast::<ICoreWebView2_2>()?.Environment()?;
                let resp = env.CreateWebResourceResponse(
                    None::<&windows::Win32::System::Com::IStream>,
                    204,
                    w!("No Content"),
                    w!(""),
                )?;
                args.SetResponse(&resp)?;
                Ok(())
            }));
            let mut token = 0i64;
            let _ = wv.add_WebResourceRequested(&handler, &mut token);
        }
    }

    /// Panggil `cb(berhasil, status_error)` setiap navigasi halaman utama selesai.
    pub fn on_navigation_completed(webview: &wry::WebView, cb: impl Fn(bool, i32) + 'static) {
        use webview2_com::{
            Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_ERROR_STATUS,
            NavigationCompletedEventHandler,
        };
        use wry::WebViewExtWindows;
        let handler = NavigationCompletedEventHandler::create(Box::new(move |_wv, args| {
            if let Some(a) = args {
                let mut ok = windows::core::BOOL(0);
                let mut status = COREWEBVIEW2_WEB_ERROR_STATUS(0);
                unsafe {
                    let _ = a.IsSuccess(&mut ok);
                    let _ = a.WebErrorStatus(&mut status);
                }
                cb(ok.as_bool(), status.0);
            }
            Ok(())
        }));
        let mut token = 0i64;
        unsafe {
            let _ = webview.webview().add_NavigationCompleted(&handler, &mut token);
        }
    }

    /// Apakah Windows melaporkan ada koneksi internet. Bila tidak bisa ditanyakan, dianggap ada.
    pub fn is_online() -> bool {
        use windows::Win32::Networking::NetworkListManager::{INetworkListManager, NetworkListManager};
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
        unsafe {
            CoCreateInstance::<_, INetworkListManager>(&NetworkListManager, None, CLSCTX_ALL)
                .and_then(|m| m.IsConnectedToInternet())
                .map(|v| v.0 != 0)
                .unwrap_or(true)
        }
    }

    /// Panggil `cb` dengan jenis kegagalan bila proses WebView2 mati/macet
    /// (0 = browser, 1 = renderer halaman keluar, 2 = renderer tidak merespons, 6 = GPU, ...).
    pub fn on_process_failed(webview: &wry::WebView, cb: impl Fn(i32) + 'static) {
        use webview2_com::{
            Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_KIND,
            ProcessFailedEventHandler,
        };
        use wry::WebViewExtWindows;
        let handler = ProcessFailedEventHandler::create(Box::new(move |_wv, args| {
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND(-1);
            if let Some(a) = args {
                unsafe {
                    let _ = a.ProcessFailedKind(&mut kind);
                }
            }
            cb(kind.0);
            Ok(())
        }));
        let mut token = 0i64;
        unsafe {
            let _ = webview.webview().add_ProcessFailed(&handler, &mut token);
        }
    }

    /// Ringkasan sistem untuk laporan diagnostik (tanpa data pribadi).
    pub fn system_info() -> String {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        use winreg::enums::HKEY_LOCAL_MACHINE;
        let mut out = String::new();
        if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        {
            let get = |n: &str| k.get_value::<String, _>(n).unwrap_or_default();
            let ubr: u32 = k.get_value("UBR").unwrap_or(0);
            out.push_str(&format!(
                "Windows: {} {} (build {}.{ubr})\n",
                get("ProductName"),
                get("DisplayVersion"),
                get("CurrentBuild")
            ));
        }
        let mut m = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() {
            out.push_str(&format!(
                "RAM: {:.1} GB total, {:.1} GB tersedia saat ini ({}% terpakai)\n",
                m.ullTotalPhys as f64 / 1073741824.0,
                m.ullAvailPhys as f64 / 1073741824.0,
                m.dwMemoryLoad
            ));
        }
        out
    }

    /// Kosongkan working set proses WebView2 (dan proses ini). Halaman yang jarang disentuh
    /// dilepas dari RAM fisik; Windows memuatnya kembali saat dibutuhkan. Hanya dipakai
    /// saat Outlook tersembunyi/diminimalkan. Mengembalikan jumlah proses yang dirapikan.
    pub fn trim_process_tree() -> u32 {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
        };
        let mut n = 0;
        for pid in tree_pids() {
            unsafe {
                let Ok(h) =
                    OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA, false, pid)
                else {
                    continue;
                };
                if EmptyWorkingSet(h).is_ok() {
                    n += 1;
                }
                let _ = CloseHandle(h);
            }
        }
        n
    }

    // ---------- WebView2 ----------

    /// Minta WebView2 menangguhkan halaman (hanya berhasil bila webview tidak terlihat).
    pub fn try_suspend(webview: &wry::WebView) {
        use webview2_com::{Microsoft::Web::WebView2::Win32::ICoreWebView2_3, TrySuspendCompletedHandler};
        use windows::core::Interface;
        use wry::WebViewExtWindows;
        if let Ok(wv3) = webview.webview().cast::<ICoreWebView2_3>() {
            let handler = TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(())));
            let _ = unsafe { wv3.TrySuspend(&handler) };
        }
    }

    // ---------- unduhan (memakai pengaturan proxy & sertifikat Windows) ----------

    pub fn download(url: &str, dest: &Path) -> Result<(), String> {
        use windows::core::HSTRING;
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        use windows::Win32::System::Com::Urlmon::URLDownloadToFileW;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            URLDownloadToFileW(
                None,
                &HSTRING::from(url),
                &HSTRING::from(dest.as_os_str()),
                0,
                None,
            )
            .map_err(|e| format!("unduh gagal: {e}"))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn confirm(text: &str) -> bool {
        eprintln!("[konfirmasi] {text}");
        false
    }
    pub fn info(text: &str) {
        eprintln!("[info] {text}");
    }
    pub fn init_identity(_icon_png: Option<&Path>) {}
    pub fn toast(spec: &ToastSpec, _icon: Option<&Path>) {
        eprintln!("[toast] {} — {} {}", spec.title, spec.body, spec.body2);
    }
    pub fn set_autostart(_enabled: bool, _exe: &Path) -> std::io::Result<()> {
        Ok(())
    }
    pub fn autostart_value() -> Option<String> {
        None
    }
    pub fn register_mailto(_exe: &Path) -> std::io::Result<()> {
        Ok(())
    }
    pub fn unregister_mailto() {}
    pub fn mailto_registered() -> bool {
        false
    }
    pub fn refresh_registrations(_exe: &Path) {}
    pub fn process_tree_memory() -> Option<TreeMem> {
        None
    }
    pub fn process_snapshot(
        _kinds: &std::collections::HashMap<u32, crate::diag::Kind>,
    ) -> Vec<crate::diag::Proc> {
        Vec::new()
    }
    pub fn webview_process_kinds(
        _webview: &wry::WebView,
    ) -> std::collections::HashMap<u32, crate::diag::Kind> {
        Default::default()
    }
    pub fn system_info() -> String {
        String::new()
    }
    pub fn on_process_failed(_webview: &wry::WebView, _cb: impl Fn(i32) + 'static) {}
    pub fn on_navigation_completed(_webview: &wry::WebView, _cb: impl Fn(bool, i32) + 'static) {}
    pub fn is_online() -> bool {
        true
    }
    pub fn set_background_mode(_background: bool) -> u32 {
        0
    }
    pub fn tune_webview(_webview: &wry::WebView, _blocked_hosts: &'static [&'static str]) {}
    pub fn trim_process_tree() -> u32 {
        0
    }
    pub fn try_suspend(_webview: &wry::WebView) {}
    pub fn download(_url: &str, _dest: &Path) -> Result<(), String> {
        Err("hanya di Windows".into())
    }
}

pub use imp::*;
