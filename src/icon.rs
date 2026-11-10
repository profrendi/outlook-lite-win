//! Ikon digambar lewat kode (tanpa file aset) + penulis PNG minimal untuk ikon notifikasi.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Badge {
    None,
    /// Ada email belum dibaca.
    Unread,
    /// Perlu login ulang / ada masalah.
    Warning,
}

/// Lingkaran biru dengan cincin putih (amplop sederhana), plus titik lencana opsional.
pub fn rgba(size: u32, badge: Badge) -> Vec<u8> {
    let n = size as f32;
    let c = (n - 1.0) / 2.0;
    let outer = n / 2.0 - 0.5;
    let mut px = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32, y as f32);
            let r = ((fx - c).powi(2) + (fy - c).powi(2)).sqrt();
            let mut p: [u8; 4] = if r > outer {
                [0, 0, 0, 0]
            } else {
                [0, 120, 212, 255]
            };
            // amplop putih di tengah
            let (l, t, rgt, b) = (n * 0.25, n * 0.32, n * 0.75, n * 0.68);
            if fx >= l && fx <= rgt && fy >= t && fy <= b && r <= outer {
                let edge = fx - l < n * 0.06
                    || rgt - fx < n * 0.06
                    || fy - t < n * 0.06
                    || b - fy < n * 0.06;
                // garis V
                let mid = (l + rgt) / 2.0;
                let vy = t + (b - t) * 0.55 * (1.0 - ((fx - mid).abs() / (mid - l)));
                let v = (fy - vy).abs() < n * 0.05 && fy <= t + (b - t) * 0.6;
                if edge || v {
                    p = [255, 255, 255, 255];
                }
            }
            // lencana kanan atas
            if badge != Badge::None {
                let (bx, by, br) = (n * 0.78, n * 0.22, n * 0.22);
                let d = ((fx - bx).powi(2) + (fy - by).powi(2)).sqrt();
                if d <= br {
                    p = match badge {
                        Badge::Unread => [232, 17, 35, 255],
                        Badge::Warning => [255, 185, 0, 255],
                        Badge::None => p,
                    };
                } else if d <= br + n * 0.05 {
                    p = [255, 255, 255, 255];
                }
            }
            px.extend_from_slice(&p);
        }
    }
    px
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Enkode RGBA ke PNG (deflate "stored", tanpa kompresi). Cukup untuk ikon kecil.
pub fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * 4 + 1) as usize * height as usize);
    for row in rgba.chunks((width * 4) as usize) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    if chunks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(c) = chunks.next() {
        z.push(u8::from(chunks.peek().is_none()));
        let len = c.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(c);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn png_shape() {
        let px = rgba(32, Badge::Unread);
        assert_eq!(px.len(), 32 * 32 * 4);
        let p = png(32, 32, &px);
        assert_eq!(&p[1..4], b"PNG");
        assert_eq!(&p[p.len() - 8..p.len() - 4], b"IEND");
    }
}
