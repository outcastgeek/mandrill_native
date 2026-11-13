//! Search token normalization.
//!
//! Unicode Letter and Decimal_Number characters are kept, every other character becomes a separating space, the
//! kept characters are lowercased, duplicate tokens are dropped in first-seen order
//! and the result is capped at [`DEFAULT_MAX_SEARCH_TOKENS`] unless the caller
//! supplies a positive cap.
//!
//! One rune is lowercased with a simple single-character mapping (the first character
//! of Rust's full
//! lowercasing) so a token never becomes a different number of characters here.

/// The default token cap.
pub const DEFAULT_MAX_SEARCH_TOKENS: i64 = 12;

/// Configures token normalization.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SearchTokenOptions {
    /// Token cap; a non-positive value uses [`DEFAULT_MAX_SEARCH_TOKENS`].
    pub max_tokens: i64,
}

/// Normalize a search query into unique tokens.
#[must_use]
pub fn search_tokens(query: impl AsRef<[u8]>) -> Vec<String> {
    search_tokens_with_options(query, &SearchTokenOptions::default())
}

/// Normalize a search query into unique tokens under the supplied options.
#[must_use]
pub fn search_tokens_with_options(
    query: impl AsRef<[u8]>,
    options: &SearchTokenOptions,
) -> Vec<String> {
    let query =
        crate::byte_text::encoding_text(crate::validation::trim_unicode_whitespace(query.as_ref()));
    if query.is_empty() {
        return Vec::new();
    }
    let max_tokens = if options.max_tokens <= 0 {
        DEFAULT_MAX_SEARCH_TOKENS
    } else {
        options.max_tokens
    };
    let cap = usize::try_from(max_tokens).unwrap_or(usize::MAX);
    let mut normalized = String::with_capacity(query.len());
    for character in query.chars() {
        if is_search_character(character) {
            normalized.push(simple_lowercase(character));
            continue;
        }
        normalized.push(' ');
    }
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut tokens: Vec<String> = Vec::new();
    for token in normalized.split_whitespace() {
        if !seen.insert(token.to_owned()) {
            continue;
        }
        tokens.push(token.to_owned());
        if tokens.len() >= cap {
            break;
        }
    }
    tokens
}

/// The single-character lowercase form of one character.
fn simple_lowercase(character: char) -> char {
    character.to_lowercase().next().unwrap_or(character)
}

fn is_search_character(character: char) -> bool {
    static LETTER_OR_DIGIT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^[\p{L}\p{Nd}]$").expect("fixed Unicode category expression")
    });
    LETTER_OR_DIGIT.is_match(character.encode_utf8(&mut [0; 4]))
}
