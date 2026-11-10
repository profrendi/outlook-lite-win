//! Update otomatis dari GitHub Releases.
//!
//! CI menerbitkan `outlook-lite.exe` dan `latest.json` ({version, url, sha256}) di setiap rilis.
//! Aplikasi mengunduh exe baru ke sebelah exe lama, memeriksa SHA-256, lalu menukarnya.
//! Exe lama diganti nama menjadi `.old` (Windows mengizinkan ini walau sedang berjalan)
//! dan dihapus pada start berikutnya. Versi baru aktif setelah aplikasi di-restart.

use std::path::Path;

use serde::Deserialize;

pub const REPO: &str = "profrendi/outlook-lite-win";

pub enum Outcome {
    UpToDate,
    Ready(String),
    Failed(String),
}

#[derive(Deserialize)]
struct Latest {
    version: String,
    url: String,
    sha256: String,
}

pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches('v');
    let mut it = s.split(['.', '-', '+']);
    Some((
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next().unwrap_or("0").parse().ok()?,
    ))
}

pub fn is_newer(remote: &str, local: &str) -> bool {
    matches!((parse_version(remote), parse_version(local)), (Some(r), Some(l)) if r > l)
}

/// Hapus exe lama sisa update sebelumnya.
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
        let _ = std::fs::remove_file(exe.with_extension("exe.new"));
    }
}

fn cache_buster() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(windows)]
fn sha256_hex(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(windows)]
pub fn check_and_stage(data_dir: &Path) -> Outcome {
    match stage(data_dir) {
        Ok(Some(v)) => Outcome::Ready(v),
        Ok(None) => Outcome::UpToDate,
        Err(e) => Outcome::Failed(e),
    }
}

#[cfg(windows)]
fn stage(data_dir: &Path) -> Result<Option<String>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let meta = data_dir.join("latest.json");
    crate::win::download(
        &format!(
            "https://github.com/{REPO}/releases/latest/download/latest.json?t={}",
            cache_buster()
        ),
        &meta,
    )?;
    let text = std::fs::read_to_string(&meta).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&meta);
    let latest: Latest =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("latest.json: {e}"))?;

    if !is_newer(&latest.version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let prefix = format!("https://github.com/{REPO}/releases/download/");
    if !latest.url.starts_with(&prefix) {
        return Err("URL update tidak dikenal, dibatalkan".into());
    }

    let new = exe.with_extension("exe.new");
    let old = exe.with_extension("exe.old");
    crate::win::download(&latest.url, &new)?;
    let got = sha256_hex(&new)?;
    if !got.eq_ignore_ascii_case(latest.sha256.trim()) {
        let _ = std::fs::remove_file(&new);
        return Err("checksum update tidak cocok, dibatalkan".into());
    }
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("ganti nama exe lama: {e}"))?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("pasang exe baru: {e}"));
    }
    Ok(Some(latest.version))
}

#[cfg(not(windows))]
pub fn check_and_stage(_data_dir: &Path) -> Outcome {
    let _ = cache_buster;
    let _: Option<Latest> = None;
    Outcome::Failed("update hanya di Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.2"), Some((0, 2, 0)));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("abc", "0.1.0"));
    }
}
