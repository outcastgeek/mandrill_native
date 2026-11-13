//! Shared URL character predicates; parsing and response policy stay with callers.

/// Source URL userinfo admits ASCII letters, digits and these delimiters.
/// Percent-escape validity is checked separately by the component parser.
pub(crate) fn valid_raw_userinfo(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:%@".contains(&byte))
}

