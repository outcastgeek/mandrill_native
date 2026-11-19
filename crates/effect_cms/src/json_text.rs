//! Shared JSON string-token compatibility at decoding boundaries.

use std::convert::Infallible;

fn unicode_unit(raw: &[u8], start: usize) -> Option<u16> {
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

fn integer_zero_at(raw: &[u8], index: usize) -> bool {
    let boundary_before = index == 0
        || raw.get(index - 1).is_some_and(|byte| {
            matches!(
                byte,
                b' ' | b'\t' | b'\r' | b'\n' | b'[' | b'{' | b',' | b':'
            )
        });
    let boundary_after = raw
        .get(index + 2)
        .is_none_or(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b']' | b'}' | b','));
    boundary_before && boundary_after && raw.get(index..index + 2) == Some(b"-0")
}

/// Visit repaired JSON bytes without materializing a separate text or document.
/// Returns false for malformed string syntax, so prepared callers can roll back.
pub(crate) fn write_normalized_json<E>(
    raw: &[u8],
    integer_zero: bool,
    mut emit: impl FnMut(&[u8]) -> Result<(), E>,
) -> Result<bool, E> {
    let mut index = 0;
    let mut in_string = false;
    while let Some(&byte) = raw.get(index) {
        if !in_string {
            if integer_zero && byte == b'-' && integer_zero_at(raw, index) {
                emit(b"0")?;
                index += 2;
            } else {
                emit(&[byte])?;
                index += 1;
                in_string = byte == b'"';
            }
            continue;
        }
        match byte {
            b'"' => {
                emit(b"\"")?;
                index += 1;
                in_string = false;
            }
            b'\\' => {
                let Some(escaped) = raw.get(index + 1) else {
                    return Ok(false);
                };
                let width = match escaped {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => 2,
                    b'u' => {
                        let Some(unit) = unicode_unit(raw, index + 2) else {
                            return Ok(false);
                        };
                        if (0xd800..=0xdbff).contains(&unit)
                            && raw.get(index + 6..index + 8) == Some(b"\\u")
                            && unicode_unit(raw, index + 8)
                                .is_some_and(|next| (0xdc00..=0xdfff).contains(&next))
                        {
                            12
                        } else if (0xd800..=0xdfff).contains(&unit) {
                            emit(b"\\ufffd")?;
                            index += 6;
                            continue;
                        } else {
                            6
                        }
                    }
                    _ => return Ok(false),
                };
                let Some(escape) = raw.get(index..index + width) else {
                    return Ok(false);
                };
                emit(escape)?;
                index += width;
            }
            0..=0x1f => return Ok(false),
            _ => {
                let start = index;
                while raw
                    .get(index)
                    .is_some_and(|byte| !matches!(byte, b'"' | b'\\' | 0..=0x1f))
                {
                    index += 1;
                }
                let Some(span) = raw.get(start..index) else {
                    return Ok(false);
                };
                crate::byte_text::visit_encoding(span, |text| emit(text.as_bytes()))?;
            }
        }
    }
    Ok(!in_string)
}

pub(crate) fn normalize_json_string_tokens(raw: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(raw.len());
    let Ok(valid) = write_normalized_json(raw, false, |bytes| {
        output.extend_from_slice(bytes);
        Ok::<_, Infallible>(())
    });
    valid.then_some(output)
}

/// serde_json classifies the integer spelling -0 as a float. The source accepts
/// that integer spelling, but rejects decimal/exponent spellings for integer fields.
pub(crate) fn normalize_integer_negative_zero(document: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(document.len());
    let mut in_string = false;
    let mut index = 0;
    while let Some(&byte) = document.get(index) {
        if in_string {
            normalized.push(byte);
            index += 1;
            if byte == b'\\' {
                if let Some(escaped) = document.get(index) {
                    normalized.push(*escaped);
                    index += 1;
                }
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'-' && integer_zero_at(document, index) {
            normalized.push(b'0');
            index += 2;
        } else {
            normalized.push(byte);
            index += 1;
            in_string = byte == b'"';
        }
    }
    normalized
}
