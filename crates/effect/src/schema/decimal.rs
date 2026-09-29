//! Exact decimal syntax and canonical representation.

use std::sync::Arc;

/// An exact, base-ten decimal. The stored text has one canonical spelling:
/// no redundant leading or fractional trailing zeroes, and no negative zero.
/// Precision is bounded by the input and the caller's parse limits, not by a
/// floating-point conversion or an implicit fixed scale.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Decimal(Arc<str>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A typed failure to construct or convert an exact decimal value.
pub enum DecimalError {
    /// The syntax failed validation.
    InvalidSyntax,
}

impl std::fmt::Display for DecimalError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for DecimalError {}

impl Decimal {
    /// Accept ordinary decimal notation exactly; exponent notation and
    /// non-finite values have no representation in this format.
    ///
    /// # Parameters
    /// - `text` (`&str`): Input text parsed or emitted by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, DecimalError>`.
    pub fn parse(text: &str) -> Result<Self, DecimalError> {
        let (negative, magnitude) = match text.strip_prefix('-') {
            Some(magnitude) => (true, magnitude),
            None => (false, text),
        };
        let (integer, fraction) = match magnitude.split_once('.') {
            Some((integer, fraction)) => (integer, Some(fraction)),
            None => (magnitude, None),
        };
        if integer.is_empty()
            || !integer.bytes().all(|digit| digit.is_ascii_digit())
            || fraction.is_some_and(|digits| {
                digits.is_empty() || !digits.bytes().all(|digit| digit.is_ascii_digit())
            })
        {
            return Err(DecimalError::InvalidSyntax);
        }
        let integer = integer.trim_start_matches('0');
        let integer = if integer.is_empty() { "0" } else { integer };
        let fraction = fraction.unwrap_or("").trim_end_matches('0');
        let negative = negative && (integer != "0" || !fraction.is_empty());
        let sign = if negative { "-" } else { "" };
        let separator = if fraction.is_empty() { "" } else { "." };
        Ok(Self(Arc::from(format!(
            "{sign}{integer}{separator}{fraction}"
        ))))
    }

    #[must_use]
    /// Borrow the textual representation without allocating a new string.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
