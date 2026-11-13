//! URL components needed by media classification and provider inference.

pub(super) struct MediaUrl {
    pub host: Vec<u8>,
    pub path: Vec<u8>,
}

pub(super) fn parse(raw: &[u8]) -> Option<MediaUrl> {
    let (raw, fragment) = split(raw, b'#');
    // Fragments are decoded separately; queries retain arbitrary escape text.
    if let Some(fragment) = fragment { unescape(fragment, Mode::Path)?; }
    if raw.iter().any(|byte| *byte < 0x20 || *byte == 0x7f) { return None; }
    let mut rest = raw;
    let mut scheme = &b""[..];
    for (index, byte) in raw.iter().copied().enumerate() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' => {},
            b'0'..=b'9' | b'+' | b'-' | b'.' if index > 0 => {},
            b':' => {
                if index == 0 { return None; }
                scheme = &raw[..index];
                rest = &raw[index + 1..];
                break;
            }
            _ => break,
        }
    }
    rest = split(rest, b'?').0;
    if !rest.starts_with(b"/") {
        if !scheme.is_empty() { return Some(MediaUrl { host: Vec::new(), path: Vec::new() }); }
        if split(rest, b'/').0.contains(&b':') { return None; }
    }
    let mut host = Vec::new();
    if rest.starts_with(b"//") && (!scheme.is_empty() || !rest.starts_with(b"///")) {
        let (authority, path) = split(&rest[2..], b'/');
        rest = path.map(|path| &rest[rest.len() - path.len() - 1..]).unwrap_or_default();
        let authority = if let Some(index) = authority.iter().rposition(|byte| *byte == b'@') {
            let userinfo = &authority[..index];
            if !crate::url_text::valid_raw_userinfo(std::str::from_utf8(userinfo).ok()?) {
                return None;
            }
            unescape(userinfo, Mode::Path)?;
            &authority[index + 1..]
        } else { authority };
        host = parse_host(authority, scheme)?;
    }
    Some(MediaUrl { host, path: unescape(rest, Mode::Path)? })
}

fn split(raw: &[u8], delimiter: u8) -> (&[u8], Option<&[u8]>) {
    match raw.iter().position(|byte| *byte == delimiter) {
        Some(index) => (&raw[..index], Some(&raw[index + 1..])),
        None => (raw, None),
    }
}

fn parse_host(raw: &[u8], scheme: &[u8]) -> Option<Vec<u8>> {
    if raw.iter().rposition(|byte| *byte == b'[').is_some_and(|index| index > 0) {
        return None;
    }
    if raw.starts_with(b"[") {
        let close = raw.iter().rposition(|byte| *byte == b']')?;
        if !valid_port(&raw[close + 1..]) { return None; }
        let raw = &raw[1..close];
        let host = if let Some(zone) = raw.windows(3).position(|bytes| bytes == b"%25") {
            let mut host = unescape(&raw[..zone], Mode::Host)?;
            host.extend(unescape(&raw[zone..], Mode::Zone)?);
            host
        } else { unescape(raw, Mode::Host)? };
        let (address, zone) = split(&host, b'%');
        if zone.is_some_and(|zone| zone.is_empty()) { return None; }
        std::str::from_utf8(address).ok()?.parse::<std::net::Ipv6Addr>().ok()?;
        return Some(host);
    }
    let host = if let Some(first_colon) = raw.iter().position(|byte| *byte == b':') {
        let colon = if scheme.eq_ignore_ascii_case(b"http") || scheme.eq_ignore_ascii_case(b"https") {
            first_colon
        } else { raw.iter().rposition(|byte| *byte == b':')? };
        if !valid_port(&raw[colon..]) { return None; }
        &raw[..colon]
    } else { raw };
    unescape(host, Mode::Host)
}

fn valid_port(port: &[u8]) -> bool {
    port.is_empty() || (port[0] == b':' && port[1..].iter().all(u8::is_ascii_digit))
}

#[derive(Clone, Copy)]
enum Mode { Path, Host, Zone }

fn host_byte(byte: u8) -> bool {
    byte >= 0x80 || byte.is_ascii_alphanumeric() || b"!$&'()*+,;=:-._~[]\"<>".contains(&byte)
}

fn unescape(raw: &[u8], mode: Mode) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        let byte = raw[index];
        if byte == b'%' {
            let high = (*raw.get(index + 1)? as char).to_digit(16)? as u8;
            let low = (*raw.get(index + 2)? as char).to_digit(16)? as u8;
            let value = high * 16 + low;
            if matches!(mode, Mode::Host) && value < 0x80 && value != b'%' { return None; }
            if matches!(mode, Mode::Zone) && value != b'%' && value != b' ' && !host_byte(value) { return None; }
            decoded.push(value);
            index += 3;
        } else {
            if !matches!(mode, Mode::Path) && !host_byte(byte) { return None; }
            decoded.push(byte);
            index += 1;
        }
    }
    Some(decoded)
}
