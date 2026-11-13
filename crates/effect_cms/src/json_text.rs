//! Shared JSON string-token compatibility at decoding boundaries.

fn append_string_bytes(output: &mut Vec<u8>, mut remaining: &[u8]) {
    loop {
        match std::str::from_utf8(remaining) {
            Ok(_) => {
                output.extend_from_slice(remaining);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                output.extend_from_slice(&remaining[..valid]);
                output.extend_from_slice(b"\xef\xbf\xbd");
                remaining = &remaining[valid + 1..];
            }
        }
    }
}

pub(crate) fn normalize_json_string_tokens(raw: &[u8]) -> Option<Vec<u8>> {
    fn unit(raw: &[u8], start: usize) -> Option<u16> {
        let mut value = 0u16;
        for &digit in raw.get(start..start.checked_add(4)?)? {
            value = (value << 4)
                | u16::from(match digit {
                    b'0'..=b'9' => digit - b'0',
                    b'a'..=b'f' => digit - b'a' + 10,
                    b'A'..=b'F' => digit - b'A' + 10,
                    _ => return None,
                });
        }
        Some(value)
    }
    let mut output = Vec::with_capacity(raw.len());
    let mut index = 0;
    let mut in_string = false;
    while index < raw.len() {
        let byte = raw[index];
        if !in_string {
            output.push(byte);
            index += 1;
            if byte == b'"' {
                in_string = true;
            }
            continue;
        }
        match byte {
            b'"' => {
                output.push(byte);
                index += 1;
                in_string = false;
            }
            b'\\' => match *raw.get(index + 1)? {
                b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {
                    output.extend_from_slice(&raw[index..index + 2]);
                    index += 2;
                }
                b'u' => {
                    let value = unit(raw, index + 2)?;
                    if (0xd800..=0xdbff).contains(&value)
                        && raw.get(index + 6..index + 8) == Some(b"\\u".as_slice())
                        && unit(raw, index + 8)
                            .is_some_and(|next| (0xdc00..=0xdfff).contains(&next))
                    {
                        output.extend_from_slice(&raw[index..index + 12]);
                        index += 12;
                    } else {
                        if (0xd800..=0xdfff).contains(&value) {
                            output.extend_from_slice(b"\\ufffd");
                        } else {
                            output.extend_from_slice(&raw[index..index + 6]);
                        }
                        index += 6;
                    }
                }
                _ => return None,
            },
            0..=0x1f => return None,
            _ => {
                let start = index;
                while index < raw.len() && !matches!(raw[index], b'"' | b'\\' | 0..=0x1f) {
                    index += 1;
                }
                append_string_bytes(&mut output, &raw[start..index]);
            }
        }
    }
    if in_string { None } else { Some(output) }
}

/// serde_json classifies the integer spelling -0 as a float. The source accepts
/// that integer spelling, but rejects decimal/exponent spellings for integer fields.
pub(crate) fn normalize_integer_negative_zero(document: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(document.len());
    let mut in_string = false;
    let mut index = 0;
    while index < document.len() {
        let byte = document[index];
        if in_string {
            normalized.push(byte);
            index += 1;
            if byte == b'\\' && index < document.len() {
                normalized.push(document[index]);
                index += 1;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        if byte == b'"' {
            in_string = true;
        }
        let boundary_before = index == 0
            || matches!(
                document[index - 1],
                b' ' | b'\t' | b'\r' | b'\n' | b'[' | b'{' | b',' | b':'
            );
        let boundary_after = document
            .get(index + 2)
            .is_none_or(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b']' | b'}' | b','));
        if byte == b'-'
            && document.get(index + 1) == Some(&b'0')
            && boundary_before
            && boundary_after
        {
            normalized.push(b'0');
            index += 2;
        } else {
            normalized.push(byte);
            index += 1;
        }
    }
    normalized
}

