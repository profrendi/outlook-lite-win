//! Fungsi khusus Windows. Semuanya bekerja di level pengguna (HKCU), tanpa hak admin.
//! Di platform lain fungsi-fungsi ini berupa stub supaya proyek tetap bisa dicek.

use std::path::Path;
use std::sync::Arc;

pub const AUMID: &str = "OutlookLite.App";
pub const APP_REG_NAME: &str = "OutlookLite";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub type Callback = Arc<dyn Fn() + Send + Sync>;

pub struct ToastSpec {
    pub title: String,
    pub body: String,
    pub sound: bool,
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
        if build(AUMID).show().is_err() {
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

    /// Total "private working set" proses ini + semua turunannya (proses WebView2).
    /// Angka ini sama dengan kolom Memory di Task Manager.
    pub fn process_tree_memory() -> Option<(u64, u32)> {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };
        use windows::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
        };
        use windows::Win32::System::Threading::{
            GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let mut pairs: Vec<(u32, u32)> = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
            let mut e = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snap, &mut e).is_ok() {
                loop {
                    pairs.push((e.th32ProcessID, e.th32ParentProcessID));
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

        let mut total: u64 = 0;
        for &pid in &tree {
            unsafe {
                let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                    continue;
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
                    total += if pmc.PrivateWorkingSetSize > 0 {
                        pmc.PrivateWorkingSetSize
                    } else {
                        pmc.WorkingSetSize
                    } as u64;
                }
                let _ = CloseHandle(h);
            }
        }
        Some((total, tree.len() as u32))
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
    pub fn init_identity(_icon_png: Option<&Path>) {}
    pub fn toast(spec: &ToastSpec, _icon: Option<&Path>) {
        eprintln!("[toast] {} — {}", spec.title, spec.body);
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
    pub fn process_tree_memory() -> Option<(u64, u32)> {
        None
    }
    pub fn try_suspend(_webview: &wry::WebView) {}
    pub fn download(_url: &str, _dest: &Path) -> Result<(), String> {
        Err("hanya di Windows".into())
    }
}

pub use imp::*;
