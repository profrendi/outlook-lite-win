//! Tanam ikon dan informasi versi ke exe Windows, supaya shortcut Start Menu,
//! Explorer, dan Task Manager menampilkan ikon serta nama "Outlook Lite".

#[allow(dead_code)]
#[path = "src/icon.rs"]
mod icon;

/// File .ico berisi beberapa ukuran PNG (didukung Windows Vista ke atas).
#[allow(dead_code)]
fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = sizes
        .iter()
        .map(|&s| (s, icon::png(s, s, &icon::rgba(s, icon::Badge::None))))
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (s, data) in &images {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    out
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon.rs");

    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app.ico");
        std::fs::write(&out, ico(&[16, 24, 32, 48, 64, 128])).expect("tulis ikon");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(out.to_str().unwrap())
            .set("ProductName", "Outlook Lite")
            .set("FileDescription", "Outlook Lite")
            .set("CompanyName", "profrendi")
            .set("LegalCopyright", "MIT");
        res.compile().expect("kompilasi resource Windows");
    }
}
