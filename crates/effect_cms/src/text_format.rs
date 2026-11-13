//! Percent-style formatting for explicitly supplied string operands.

use std::fmt::Write as _;

fn quote(value: &str, ascii: bool, raw: bool) -> String {
    if raw && value.chars().all(|c| c != '`' && c != '\u{feff}' && (c == '\t' || (c >= ' ' && c != '\u{7f}'))) {
        return format!("`{value}`");
    }
    static PRINTABLE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(||
        regex::Regex::new(r"^[\p{L}\p{M}\p{N}\p{P}\p{S} ]$").expect("printable Unicode categories"));
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""), '\\' => out.push_str("\\\\"),
            '\u{7}' => out.push_str("\\a"), '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"), '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"), '\t' => out.push_str("\\t"), '\u{b}' => out.push_str("\\v"),
            _ => {
                let mut encoded = [0; 4];
                if (!ascii || c.is_ascii()) && PRINTABLE.is_match(c.encode_utf8(&mut encoded)) { out.push(c); }
                else if (c as u32) < 0x20 || c == '\u{7f}' { let _ = write!(out, "\\x{:02x}", c as u32); }
                else if (c as u32) <= 0xffff { let _ = write!(out, "\\u{:04x}", c as u32); }
                else { let _ = write!(out, "\\U{:08x}", c as u32); }
            }
        }
    }
    out.push('"');
    out
}

fn padded(value: String, width: Option<usize>, left: bool, zero: bool) -> String {
    let count = width.unwrap_or_default().saturating_sub(value.chars().count());
    let pad = if zero && !left { "0" } else { " " }.repeat(count);
    if left { value + &pad } else { pad + &value }
}

fn number(chars: &[char], at: &mut usize) -> Option<usize> {
    let start = *at;
    let mut value = 0usize;
    while let Some(c) = chars.get(*at).filter(|c| c.is_ascii_digit()) {
        if value > 1_000_000 { *at = chars.len(); return None; }
        value = value.saturating_mul(10).saturating_add(*c as usize - '0' as usize);
        *at += 1;
    }
    (start != *at).then_some(value)
}

fn index(chars: &[char], at: &mut usize, next: &mut usize, reordered: &mut bool, valid: &mut bool, count: usize) -> bool {
    if chars.get(*at) != Some(&'[') { return false; }
    *reordered = true;
    let start = *at;
    *at += 1;
    if chars.len() - start < 3 { *valid = false; return false; }
    if let Some(close) = chars[*at..].iter().position(|c| *c == ']') {
        let close = *at + close;
        let value = number(&chars[..close], at);
        let parsed = value.is_some() && *at == close;
        if let Some(value)=value.filter(|value|parsed && *value>0 && *value<=count) { *next=value-1; } else { *valid=false; }
        *at = close + 1;
        parsed
    } else { *at = start + 1; *valid = false; false }
}

pub(crate) fn format_one_string(label: &str, language: &str) -> String {
    format_strings(label,&[language])
}

pub(crate) fn format_strings(label: &str, arguments: &[&str]) -> String {
    let chars: Vec<char> = label.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    let mut next = 0;
    let mut reordered = false;
    while at < chars.len() {
        if chars[at] != '%' { out.push(chars[at]); at += 1; continue; }
        at += 1;
        let (mut left, mut zero, mut sharp, mut plus, mut space) = (false, false, false, false, false);
        while let Some(c) = chars.get(at) {
            match c { '-' => left = true, '0' => zero = true, '#' => sharp = true, '+' => plus = true, ' ' => space = true, _ => break }
            at += 1;
        }
        let mut valid = true;
        let mut after_index = index(&chars, &mut at, &mut next, &mut reordered, &mut valid, arguments.len());
        let width = if chars.get(at) == Some(&'*') {
            at += 1; next += 1; after_index = false; out.push_str("%!(BADWIDTH)"); None
        } else {
            let width = number(&chars, &mut at);
            if after_index && width.is_some() { valid = false; }
            width
        };
        let precision = if at + 1 < chars.len() && chars.get(at) == Some(&'.') {
            at += 1;
            if after_index { valid = false; }
            after_index = index(&chars, &mut at, &mut next, &mut reordered, &mut valid, arguments.len());
            if chars.get(at) == Some(&'*') {
                at += 1; next += 1; after_index = false; out.push_str("%!(BADPREC)"); None
            } else { Some(number(&chars, &mut at).unwrap_or_default()) }
        } else { None };
        if !after_index { index(&chars, &mut at, &mut next, &mut reordered, &mut valid, arguments.len()); }
        let Some(&verb) = chars.get(at) else { out.push_str("%!(NOVERB)"); break; };
        at += 1;
        if verb == '%' { out.push('%'); continue; }
        if !valid { let _ = write!(out, "%!{verb}(BADINDEX)"); continue; }
        if next >= arguments.len() { let _ = write!(out, "%!{verb}(MISSING)"); continue; }
        let language=arguments[next];
        next += 1;
        let text: String = language.chars().take(precision.unwrap_or(usize::MAX)).collect();
        let rendered = match verb {
            's' => text,
            'v' if !sharp => text,
            'v' => quote(&text, false, false),
            'q' => quote(&text, plus, sharp),
            'T' => "string".chars().take(precision.unwrap_or(usize::MAX)).collect(),
            'x' | 'X' => {
                let bytes = &language.as_bytes()[..language.len().min(precision.unwrap_or(usize::MAX))];
                let prefix = if verb == 'x' { "0x" } else { "0X" };
                let mut hex = String::new();
                for (i, byte) in bytes.iter().enumerate() {
                    if space && i > 0 { hex.push(' '); }
                    if sharp && (space || i == 0) { hex.push_str(prefix); }
                    if verb == 'x' { let _ = write!(hex, "{byte:02x}"); } else { let _ = write!(hex, "{byte:02X}"); }
                }
                hex
            }
            _ => {
                let value = if sharp && verb == 'w' { quote(&text, false, false) } else { text };
                let value = padded(value, width, left, zero);
                let _ = write!(out, "%!{verb}(string={value})");
                continue;
            }
        };
        out.push_str(&padded(rendered, width, left, zero));
    }
    if !reordered && next < arguments.len() {
        out.push_str("%!(EXTRA ");
        for (index,value) in arguments[next..].iter().enumerate(){
            if index>0{out.push_str(", ");}out.push_str("string=");out.push_str(value);
        }
        out.push(')');
    }
    out
}
