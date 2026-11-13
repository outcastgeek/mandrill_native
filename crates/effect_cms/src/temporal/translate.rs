//! Explicit translation context and typed format arguments.
//!
//! The translation call receives either the
//! translation or the key itself when the context carries no translator. Two of its
//! translation calls also pass typed arguments (the day, the localized month
//! name and the year for `time.date_format`, the date and time text for
//! `time.date_time_format`, the formatted value and zone label for
//! `time.with_time_zone`), so the native port keeps the arguments typed instead of
//! collapsing them into formatted text.
//!
//! There is no translation backend and no catalog here: [`TemporalContext`] is the
//! caller's capability, [`BackgroundContext`] installs no translator (so every lookup
//! returns the key), and a request-scoped consumer supplies its own implementation
//! over its own culture selection. No `effect_server::Request` is required, which is
//! what lets the pure date and duration helpers keep working for background, non-HTTP
//! callers.

/// One argument passed to a translation lookup.
///
/// The translation calls in this owner pass only integers
/// (day, year) and strings (month name, date text, time text, zone label), so those
/// two argument kinds are the whole typed surface and `{}`/`%s`/`%d` placement stays
/// the caller's translation data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatArg<'a> {
    /// An integer argument, rendered for a `%d` verb.
    Int(i64),
    /// A text argument, rendered for a `%s` verb.
    Text(&'a str),
}

/// The caller's translation capability.
///
/// A `translate` implementation returns the key itself when it has no message for that
/// key; this owner detects that recovery by comparing the result with the key, so an
/// implementation must not invent a message.
pub trait TemporalContext {
    /// Translate one key with typed format arguments, returning the key when absent.
    fn translate(&self, key: &str, args: &[FormatArg<'_>]) -> String;
}

/// A background context: no translator is installed,
/// so every lookup returns the key and the owner's own documented fallbacks apply.
#[derive(Clone, Copy, Debug, Default)]
pub struct BackgroundContext;

impl TemporalContext for BackgroundContext {
    fn translate(&self, key: &str, _args: &[FormatArg<'_>]) -> String {
        key.to_owned()
    }
}
