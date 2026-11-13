//! Byte text primitives and explicit serialization conversion.

/// Replace each malformed byte independently while retaining valid text.
pub(crate) fn encoding_text(mut bytes: &[u8]) -> String {
    let mut output = String::new();
    loop {
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                output.push_str(text);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if let Ok(prefix) = std::str::from_utf8(&bytes[..valid]) {
                    output.push_str(prefix);
                }
                output.push('\u{fffd}');
                bytes = &bytes[valid + 1..];
            }
        }
    }
    output
}

/// Lowercase one Unicode scalar at a time, matching the source string operation.
/// Invalid input bytes become replacement scalars at this explicit text operation.
pub(crate) fn lowercase(bytes: &[u8]) -> String {
    encoding_text(bytes).chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}
