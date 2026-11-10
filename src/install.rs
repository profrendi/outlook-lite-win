//! Instalasi otomatis per-pengguna (tanpa admin), seperti ZFPlus.
//!
//! Saat exe dijalankan dari lokasi lain (mis. Downloads), ia menyalin dirinya ke
//! `%LOCALAPPDATA%\Programs\OutlookLite\outlook-lite.exe`, membuat shortcut Start Menu,
//! mendaftarkan entri uninstall di "Apps & features", lalu menjalankan salinan itu.
//! Mode portable: jalankan dengan `--portable`, atau letakkan file bernama `portable`
//! (atau `portable.txt`) di sebelah exe.

use std::path::{Path, PathBuf};

pub const EXE_NAME: &str = "outlook-lite.exe";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\OutlookLite";

pub fn install_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Programs")
        .join("OutlookLite")
}

pub fn installed_exe() -> PathBuf {
    install_dir().join(EXE_NAME)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// Apakah exe ini berjalan dari folder instalasi.
pub fn is_installed_copy(exe: &Path) -> bool {
    exe.parent().is_some_and(|d| same_dir(d, &install_dir()))
}

pub fn is_portable(exe: &Path, flag: bool) -> bool {
    flag || exe
        .parent()
        .is_some_and(|d| d.join("portable").exists() || d.join("portable.txt").exists())
}

pub fn should_install(exe: &Path, portable_flag: bool) -> bool {
    cfg!(windows) && !is_portable(exe, portable_flag) && !is_installed_copy(exe)
}

/// Versi yang terpasang menurut entri uninstall (None bila belum terpasang).
pub fn installed_version() -> Option<String> {
    imp::installed_version()
}

pub fn start_menu_shortcut() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| {
        PathBuf::from(a)
            .join(r"Microsoft\Windows\Start Menu\Programs")
            .join("Outlook Lite.lnk")
    })
}

/// Shortcut Start Menu + entri uninstall untuk salinan terpasang.
pub fn ensure_integration(exe: &Path) {
    if let Some(lnk) = start_menu_shortcut() {
        if !lnk.exists() {
            if let Err(e) = imp::create_shortcut(&lnk, exe) {
                crate::log(&crate::data_dir(), &format!("shortcut gagal: {e}"));
            }
        }
    }
    imp::write_uninstall_entry(exe);
}

pub fn remove_integration() {
    if let Some(lnk) = start_menu_shortcut() {
        let _ = std::fs::remove_file(lnk);
    }
    imp::remove_registry();
}

/// Salin exe ini ke folder instalasi (bila perlu). Mengembalikan exe yang harus dijalankan.
/// `stop_running` dipanggil sebelum mengganti exe terpasang yang mungkin sedang berjalan.
pub fn install_self(exe: &Path, stop_running: impl FnOnce()) -> Result<PathBuf, String> {
    let target = installed_exe();
    std::fs::create_dir_all(install_dir()).map_err(|e| format!("buat folder: {e}"))?;

    let mine = env!("CARGO_PKG_VERSION");
    let keep_existing = target.exists()
        && installed_version()
            .is_some_and(|v| !crate::update::is_newer(mine, &v));
    if !keep_existing {
        stop_running();
        if target.exists() {
            let old = target.with_extension("exe.old");
            let _ = std::fs::remove_file(&old);
            // Windows mengizinkan mengganti nama exe yang sedang berjalan.
            let _ = std::fs::rename(&target, &old);
        }
        std::fs::copy(exe, &target).map_err(|e| format!("salin exe: {e}"))?;
    }
    ensure_integration(&target);
    Ok(target)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    pub fn installed_version() -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(UNINSTALL_KEY)
            .ok()?
            .get_value("DisplayVersion")
            .ok()
    }

    pub fn write_uninstall_entry(exe: &Path) {
        let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY) else {
            return;
        };
        let exe_s = exe.display().to_string();
        let dir = exe.parent().map(|d| d.display().to_string()).unwrap_or_default();
        let size_kb = std::fs::metadata(exe).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
        let _ = k.set_value("DisplayName", &"Outlook Lite".to_string());
        let _ = k.set_value("DisplayVersion", &env!("CARGO_PKG_VERSION").to_string());
        let _ = k.set_value("Publisher", &"profrendi".to_string());
        let _ = k.set_value("DisplayIcon", &format!("\"{exe_s}\",0"));
        let _ = k.set_value("InstallLocation", &dir);
        let _ = k.set_value("UninstallString", &format!("\"{exe_s}\" --uninstall"));
        let _ = k.set_value(
            "URLInfoAbout",
            &format!("https://github.com/{}", crate::update::REPO),
        );
        let _ = k.set_value("NoModify", &1u32);
        let _ = k.set_value("NoRepair", &1u32);
        let _ = k.set_value("EstimatedSize", &size_kb);
    }

    pub fn remove_registry() {
        let h = RegKey::predef(HKEY_CURRENT_USER);
        let _ = h.delete_subkey_all(UNINSTALL_KEY);
        let _ = h.delete_subkey_all(format!(r"Software\Classes\AppUserModelId\{}", crate::win::AUMID));
    }

    /// Shortcut .lnk dengan AppUserModelID, supaya notifikasi Windows dikaitkan dengan aplikasi ini.
    pub fn create_shortcut(lnk: &Path, exe: &Path) -> Result<(), String> {
        use windows::core::{Interface, HSTRING};
        use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, IPersistFile, StructuredStorage::PROPVARIANT,
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink};

        if let Some(dir) = lnk.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
            link.SetPath(&HSTRING::from(exe.as_os_str())).map_err(|e| e.to_string())?;
            if let Some(d) = exe.parent() {
                let _ = link.SetWorkingDirectory(&HSTRING::from(d.as_os_str()));
            }
            let _ = link.SetDescription(&HSTRING::from("Outlook web ringan di tray"));
            let _ = link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0);

            let store: IPropertyStore = link.cast().map_err(|e| e.to_string())?;
            let pv = PROPVARIANT::from(crate::win::AUMID);
            store.SetValue(&PKEY_AppUserModel_ID, &pv).map_err(|e| e.to_string())?;
            store.Commit().map_err(|e| e.to_string())?;

            let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
            file.Save(&HSTRING::from(lnk.as_os_str()), true)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn installed_version() -> Option<String> {
        None
    }
    pub fn write_uninstall_entry(_exe: &Path) {}
    pub fn remove_registry() {}
    pub fn create_shortcut(_lnk: &Path, _exe: &Path) -> Result<(), String> {
        let _ = UNINSTALL_KEY;
        Ok(())
    }
}
