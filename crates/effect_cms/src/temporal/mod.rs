//! Temporal: date-time validation/normalization and duration values.
//!
//! Temporal owns:
//!
//! - the date-time validation codes, message-key record and translator contract;
//! - date-time normalization to stored UTC RFC3339 instants plus the declared zone,
//!   and the message-key / translated-message wrappers;
//! - display parsing, the UNIX-timestamp formatters, the localized date/time
//!   formatters and the localized short form, the timezone label and the normalized
//!   zone name, the local date in a zone and the today/upcoming/past status;
//! - duration parsing, normalization precedence, structured-part conversion and the
//!   localized duration format.
//!
//! # Zones, clocks and translation are explicit
//!
//! IANA zones come from a real time-zone database ([`TemporalZone::iana`]) and keep the
//! database's DST rules and abbreviations. Beyond its transition table, the native
//! final offset remains in effect; no POSIX future recurrence evaluator is added. The
//! UNIX-timestamp formatters take the selected local zone as an argument instead of
//! resolving the process zone behind the caller's back. The current-time read in
//! `TemporalStatus` is a capability: [`temporal_status`] is an `Effect<A, E, R>` that
//! reads the clock through `R: HasWallClock` and its macros, and no domain path calls a
//! system clock directly. Translation is the caller's [`TemporalContext`];
//! [`BackgroundContext`] serves pure and non-HTTP callers, so they never construct a
//! `Request`.
//!
//! # Text only
//!
//! Every output here is text (a stored instant, a status word, formatted date/time
//! text, a duration summary). No HTML is produced, so Maud is not involved: the owners
//! that render these values build the markup.
//!
//! # No store
//!
//! This owner reads no database and owns no table, schema, migration, query, pool or
//! backend; it validates, converts and formats values its callers supply.
//!
//! # Private implementation
//!
//! `zone`, `layout`, `translate`, `datetime`, `duration` and `request` are this
//! owner's implementation modules.

use effect::{
    Effect, HasWallClock, WallClock, effect, effect_map_error, effect_provide_some, service,
};

mod datetime;
mod duration;
mod layout;
mod request;
mod translate;
mod zone;

pub use self::datetime::{
    DATE_TIME_DISPLAY_LAYOUTS, DATE_TIME_VALIDATION_INVALID_DATE_TIME,
    DATE_TIME_VALIDATION_INVALID_TIME_ZONE, DATE_TIME_VALIDATION_REQUIRED,
    DATE_TIME_VALIDATION_VALID, DateTimeValidationCode, DateTimeValidationMessageKeys,
    DateTimeValidationTranslator, DisplayDateTime, LOCAL_DATE_LAYOUT, LOCAL_DATE_TIME_LAYOUT,
    LOCAL_DATE_TIME_MINUTE_LAYOUT, TEMPORAL_STATUS_PAST, TEMPORAL_STATUS_TODAY,
    TEMPORAL_STATUS_UPCOMING, date_time_validation_message_key, format_date, format_date_iso,
    format_date_time, format_localized_date_short, format_localized_date_time,
    local_date_in_time_zone, normalize_date_time_form_values,
    normalize_date_time_form_values_with_message_key,
    normalize_date_time_form_values_with_translated_message, normalized_time_zone_name,
    parse_display_date, parse_display_date_time, temporal_status_at,
};
pub use self::duration::{
    DEFAULT_MAX_MINUTES, DEFAULT_STEP_MINUTES, DurationParts, ParsedDuration, breakdown_duration,
    canonical_minutes_from_parts, canonical_minutes_from_structured_parts, format_duration,
    normalize_duration_form_values, normalize_duration_value, parse_duration,
    parse_duration_minutes,
};
pub use self::request::{
    DURATION_DAYS_FIELD, DURATION_FIELD, DURATION_HOURS_FIELD, DURATION_MINUTES_FIELD,
    DURATION_TEXT_FIELD, DURATION_WEEKS_FIELD, DurationFormPolicy, DurationFormValues,
    invalid_duration_text_from_decoded, invalid_duration_text_from_request,
    normalize_duration_form_request, normalize_duration_form_request_from_decoded,
    requested_duration_form_value, requested_duration_form_value_from_decoded,
};
pub use self::translate::{BackgroundContext, FormatArg, TemporalContext};
pub use self::zone::{TemporalInstant, TemporalZone, ZonedDateTime};

/// Localized date/time in a declared zone.
#[must_use]
pub fn format_localized_date_time_ctx<C: TemporalContext + ?Sized>(
    ctx: &C,
    date_time: &str,
    time_zone: &str,
) -> String {
    datetime::format_localized_date_time_in_zone(ctx, date_time, time_zone, false)
}

/// Localized short date/time in a zone.
#[must_use]
pub fn format_localized_date_short_ctx<C: TemporalContext + ?Sized>(
    ctx: &C,
    date_time: &str,
    time_zone: &str,
) -> String {
    datetime::format_localized_date_time_in_zone(ctx, date_time, time_zone, true)
}

/// The today/upcoming/past status at the current time.
///
/// The clock is a real dependency and is read from
/// the caller's environment (`R: HasWallClock`) through the effect macros; the effect is
/// lazy and holds no resource. `local` is the selected process-local zone the clock
/// value is projected into; supplying it is the caller's choice, not a hidden default.
/// The result is the status
/// text, or the empty string when the value cannot be interpreted (the
/// `""`), matching [`temporal_status_at`].
pub fn temporal_status<E, R>(
    date_time: String,
    time_zone: String,
    local: TemporalZone,
    on_error: impl Fn(<R::Clock as WallClock>::Error) -> E + Send + Sync + 'static,
) -> Effect<String, E, R>
where
    E: Send + 'static,
    R: HasWallClock + Send + Sync + 'static,
{
    let on_error: std::sync::Arc<dyn Fn(<R::Clock as WallClock>::Error) -> E + Send + Sync> =
        std::sync::Arc::new(on_error);
    effect! {
        let on_error = on_error.clone();
        let date_time = date_time.clone();
        let time_zone = time_zone.clone();
        let local = local.clone();
        let clock = service! {
            env |env: &R| {
                let map = on_error.clone();
                effect_provide_some! {
                    program: effect_map_error! {
                        program: env.wall_clock().now(),
                        map |error| { map(error) },
                    },
                    project |_: &R| { () },
                }
            },
        }?;
        let millis = clock?;
        let status = TemporalInstant::from_utc_millis(millis.value(), local)
            .and_then(|now| temporal_status_at(&date_time, &time_zone, &now))
            .unwrap_or_default()
            .to_owned();
        status
    }
}
