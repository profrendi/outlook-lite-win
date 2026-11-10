//! Utilitas URL kecil (tanpa crate tambahan) + konversi mailto: ke compose Outlook web.

pub const MAIL_URL: &str = "https://outlook.office.com/mail/";
pub const CALENDAR_URL: &str = "https://outlook.office.com/calendar/";
pub const COMPOSE_URL: &str = "https://outlook.office.com/mail/deeplink/compose";

/// Host dari URL http(s), huruf kecil. `None` jika bukan http(s) atau ada userinfo.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let end = rest
        .find(|c| matches!(c, '/' | '?' | '#' | ':'))
        .unwrap_or(rest.len());
    let host = &rest[..end];
    if host.is_empty() || host.contains('@') {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

fn host_matches(host: &str, domains: &[&str]) -> bool {
    domains
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// Domain Microsoft yang boleh dibuka sebagai popup di dalam aplikasi (login, compose pop-out).
pub fn is_microsoft_url(url: &str) -> bool {
    if !url.starts_with("https://") {
        return false;
    }
    host_of(url).is_some_and(|h| {
        host_matches(
            &h,
            &[
                "office.com",
                "office365.com",
                "cloud.microsoft",
                "microsoft.com",
                "microsoftonline.com",
                "live.com",
                "outlook.com",
                "sharepoint.com",
                "msauth.net",
                "msftauth.net",
            ],
        )
    })
}

/// Halaman Outlook web itu sendiri (tempat skrip pemantau aktif).
/// Sejak akhir 2025 akun kantor dialihkan ke outlook.cloud.microsoft.
pub fn is_outlook_url(url: &str) -> bool {
    if !url.starts_with("https://") {
        return false;
    }
    host_of(url).is_some_and(|h| {
        matches!(
            h.as_str(),
            "outlook.cloud.microsoft"
                | "outlook.office.com"
                | "outlook.office365.com"
                | "outlook.live.com"
        )
    })
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &c in s.as_bytes() {
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'~') {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

/// `mailto:a@b.c,d@e.f?subject=..&body=..&cc=..&bcc=..` -> URL compose Outlook web.
/// Mengembalikan `None` bila bukan URL mailto.
pub fn mailto_to_compose(mailto: &str) -> Option<String> {
    let rest = mailto.trim();
    if rest.len() < 7 || !rest[..7].eq_ignore_ascii_case("mailto:") {
        return None;
    }
    let rest = &rest[7..];
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, q),
        None => (rest, ""),
    };

    let mut to: Vec<String> = Vec::new();
    let mut cc: Vec<String> = Vec::new();
    let mut bcc: Vec<String> = Vec::new();
    let mut subject = String::new();
    let mut body = String::new();

    let push_addrs = |list: &mut Vec<String>, raw: &str| {
        for a in percent_decode(raw).split([',', ';']) {
            let a = a.trim();
            if !a.is_empty() {
                list.push(a.to_string());
            }
        }
    };
    push_addrs(&mut to, path);

    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k.to_ascii_lowercase().as_str() {
            "to" => push_addrs(&mut to, v),
            "cc" => push_addrs(&mut cc, v),
            "bcc" => push_addrs(&mut bcc, v),
            "subject" => subject = percent_decode(v),
            "body" => body = percent_decode(v),
            _ => {}
        }
    }

    let mut params: Vec<String> = Vec::new();
    for (key, list) in [("to", &to), ("cc", &cc), ("bcc", &bcc)] {
        if !list.is_empty() {
            params.push(format!("{key}={}", percent_encode(&list.join(";"))));
        }
    }
    if !subject.is_empty() {
        params.push(format!("subject={}", percent_encode(&subject)));
    }
    if !body.is_empty() {
        params.push(format!("body={}", percent_encode(&body)));
    }

    Some(if params.is_empty() {
        COMPOSE_URL.to_string()
    } else {
        format!("{COMPOSE_URL}?{}", params.join("&"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert!(is_microsoft_url("https://outlook.office.com/mail/"));
        assert!(is_microsoft_url("https://login.microsoftonline.com/x"));
        assert!(!is_microsoft_url("https://evil-office.com/"));
        assert!(!is_microsoft_url("https://office.com.evil.com/"));
        assert!(!is_microsoft_url("https://outlook.office.com@evil.com/"));
        assert!(!is_microsoft_url("http://outlook.office.com/"));
        assert!(is_outlook_url("https://outlook.office.com/mail/inbox"));
        assert!(is_outlook_url("https://outlook.cloud.microsoft/mail/"));
        assert!(!is_outlook_url("https://outlook.cloud.microsoft.evil.com/"));
        assert!(is_microsoft_url("https://m365.cloud.microsoft/"));
        assert!(!is_outlook_url("https://login.microsoftonline.com/"));
    }

    #[test]
    fn decode_encode() {
        assert_eq!(percent_decode("a%20b%2"), "a b%2");
        assert_eq!(percent_decode("%E2%82%AC"), "€");
        assert_eq!(percent_encode("a b&c"), "a%20b%26c");
    }

    #[test]
    fn mailto() {
        assert_eq!(mailto_to_compose("https://x"), None);
        assert_eq!(mailto_to_compose("mailto:").unwrap(), COMPOSE_URL);
        assert_eq!(
            mailto_to_compose("MAILTO:a@b.co,c@d.co?subject=Gaji%20Mei&cc=e@f.co&body=Halo+x").unwrap(),
            format!("{COMPOSE_URL}?to=a%40b.co%3Bc%40d.co&cc=e%40f.co&subject=Gaji%20Mei&body=Halo%2Bx")
        );
    }
}
