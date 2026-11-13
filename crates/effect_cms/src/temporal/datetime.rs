//! Date/time validation, normalization, display parsing and localized formatting.
//!
//! The behavior is applied in this order:
//!
//! - **Required first.** A trimmed-empty date/time *or* a trimmed-empty time zone is
//!   `required`, before any parsing happens.
//! - **Zone before value.** A nonblank zone is validated first, so a bad zone is
//!   `invalid-time-zone` even when the date/time is also unparseable.
//! - **Instant or wall clock.** An RFC3339/RFC3339Nano instant is stored as the UTC
//!   RFC3339 text of that instant and keeps the declared zone name (or `UTC` when the
//!   zone was blank); anything else is a zoned wall clock interpreted in the declared
//!   zone.
//! - **Message keys are the caller's.** A valid value returns the stored values and a
//!   blank message; a blank configured key stays blank (`""`), no key is invented, and
//!   an absent translator returns the key unchanged.
//! - **Display fallback.** An invalid stored value formats as its own raw text for the
//!   localized formatters; an invalid declared zone is displayed in `UTC` rather than
//!   rejected.
//!
//! # Effects
//!
//! Nothing here performs I/O, so these functions are pure. The current-time status is
//! an `Effect<A, E, R>` in the owner API and gets its clock from the caller's
//! environment; this module only holds its explicit-`now` form.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};

use crate::temporal::layout::{format_layout, parse_layout};
use crate::temporal::translate::{FormatArg, TemporalContext};
use crate::temporal::zone::{TemporalInstant, TemporalZone, ZonedDateTime};

/// The valid date-time code.
pub const DATE_TIME_VALIDATION_VALID: DateTimeValidationCode = DateTimeValidationCode::new("valid");
/// The required date-time code.
pub const DATE_TIME_VALIDATION_REQUIRED: DateTimeValidationCode =
    DateTimeValidationCode::new("required");
/// The invalid date-time code.
pub const DATE_TIME_VALIDATION_INVALID_DATE_TIME: DateTimeValidationCode =
    DateTimeValidationCode::new("invalid-date-time");
/// The invalid time-zone code.
pub const DATE_TIME_VALIDATION_INVALID_TIME_ZONE: DateTimeValidationCode =
    DateTimeValidationCode::new("invalid-time-zone");

/// The today status.
pub const TEMPORAL_STATUS_TODAY: &str = "today";
/// The upcoming status.
pub const TEMPORAL_STATUS_UPCOMING: &str = "upcoming";
/// The past status.
pub const TEMPORAL_STATUS_PAST: &str = "past";

/// The local date-time layout.
pub const LOCAL_DATE_TIME_LAYOUT: &str = "2006-01-02T15:04:05";
/// The local date-time layout with minutes.
pub const LOCAL_DATE_TIME_MINUTE_LAYOUT: &str = "2006-01-02T15:04";
/// The local date layout.
pub const LOCAL_DATE_LAYOUT: &str = "2006-01-02";
/// The display layouts, in order.
pub const DATE_TIME_DISPLAY_LAYOUTS: [&str; 6] = [
    "2006-01-02T15:04:05Z07:00",
    "2006-01-02T15:04:05-07:00",
    "2006-01-02T15:04:05",
    "2006-01-02",
    "January 2, 2006",
    "Monday, January 2, 2006",
];

/// The RFC3339 storage layout.
const RFC3339_LAYOUT: &str = "2006-01-02T15:04:05Z07:00";

/// The local date-time layouts, in order.
const LOCAL_DATE_TIME_LAYOUTS: [&str; 5] = [
    LOCAL_DATE_TIME_LAYOUT,
    LOCAL_DATE_TIME_MINUTE_LAYOUT,
    LOCAL_DATE_LAYOUT,
    "January 2, 2006",
    "Monday, January 2, 2006",
];

/// `time.date_format` translation key.
const DATE_FORMAT_KEY: &str = "time.date_format";
/// `time.date_time_format` translation key.
const DATE_TIME_FORMAT_KEY: &str = "time.date_time_format";
/// `time.date_time_short_format` translation key.
const DATE_TIME_SHORT_FORMAT_KEY: &str = "time.date_time_short_format";
/// `time.time_format` translation key.
const TIME_FORMAT_KEY: &str = "time.time_format";
/// `time.with_time_zone` translation key.
const WITH_TIME_ZONE_KEY: &str = "time.with_time_zone";
/// `component.datetimepicker.use_24_hour` translation key.
const USE_24_HOUR_KEY: &str = "component.datetimepicker.use_24_hour";

/// Month-name translation keys, indexed by one-based month.
const MONTH_KEYS: [&str; 13] = [
    "",
    "component.datetimepicker.month.january",
    "component.datetimepicker.month.february",
    "component.datetimepicker.month.march",
    "component.datetimepicker.month.april",
    "component.datetimepicker.month.may",
    "component.datetimepicker.month.june",
    "component.datetimepicker.month.july",
    "component.datetimepicker.month.august",
    "component.datetimepicker.month.september",
    "component.datetimepicker.month.october",
    "component.datetimepicker.month.november",
    "component.datetimepicker.month.december",
];

/// English month names, used when no translation is available.
const ENGLISH_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// An open date-time validation code.
///
/// This is an open string type with four declared constants, so the
/// native type stores the string rather than a closed enum: callers and consumers may
/// compare or introduce their own codes, and [`DateTimeValidationCode::new`] holds any
/// value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DateTimeValidationCode(&'static str);

impl DateTimeValidationCode {
    /// The `valid` code.
    pub const VALID: Self = DATE_TIME_VALIDATION_VALID;
    /// The `required` code.
    pub const REQUIRED: Self = DATE_TIME_VALIDATION_REQUIRED;
    /// The `invalid-date-time` code.
    pub const INVALID_DATE_TIME: Self = DATE_TIME_VALIDATION_INVALID_DATE_TIME;
    /// The `invalid-time-zone` code.
    pub const INVALID_TIME_ZONE: Self = DATE_TIME_VALIDATION_INVALID_TIME_ZONE;

    /// Build a code from its text; the type stays open.
    #[must_use]
    pub const fn new(code: &'static str) -> Self {
        Self(code)
    }

    /// The code text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// The date-time validation message keys.
///
/// Every field is caller-supplied configuration; a blank field is a blank message, not
/// a request for a default key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DateTimeValidationMessageKeys {
    /// Key reported when a value is required.
    pub required: String,
    /// Key reported when a date/time cannot be interpreted.
    pub invalid_date_time: String,
    /// Key reported when a time zone cannot be resolved.
    pub invalid_time_zone: String,
}

/// The date-time validation translator callback.
pub type DateTimeValidationTranslator = dyn Fn(&str) -> String + Send + Sync;

/// Normalization failure categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StorageError {
    /// The zone name could not be resolved.
    InvalidTimeZone,
    /// The date/time text could not be interpreted.
    InvalidDateTime,
}

/// Interpret civil fields from a parsed layout.
fn civil_date_time(parsed: &crate::temporal::layout::LayoutParse) -> Option<NaiveDateTime> {
    NaiveDate::from_ymd_opt(parsed.year, parsed.month, parsed.day)?.and_hms_nano_opt(
        parsed.hour,
        parsed.minute,
        parsed.second,
        parsed.nanosecond,
    )
}

/// Parse an RFC3339 instant.
fn parse_instant(raw: &str) -> Option<DateTime<Utc>> {
    let parsed = parse_layout(RFC3339_LAYOUT, raw)?;
    let offset_seconds = parsed.offset_seconds?;
    let wall = civil_date_time(&parsed)?;
    let utc = wall.checked_sub_signed(Duration::seconds(i64::from(offset_seconds)))?;
    Some(DateTime::from_naive_utc_and_offset(utc, Utc))
}

/// Render an instant as stored UTC RFC3339 text.
fn format_storage_instant(instant: DateTime<Utc>) -> String {
    format_layout(&TemporalZone::utc().local_at(instant), RFC3339_LAYOUT)
}

/// Normalize a date/time for storage.
fn normalize_date_time_for_storage(
    raw_date_time: &str,
    raw_time_zone: &str,
) -> Result<(String, String), StorageError> {
    let date_time = raw_date_time.trim();
    if date_time.is_empty() {
        return Err(StorageError::InvalidDateTime);
    }
    let zone =
        TemporalZone::normalize(raw_time_zone).map_err(|()| StorageError::InvalidTimeZone)?;

    if let Some(instant) = parse_instant(date_time) {
        let zone_name = zone.map_or_else(|| "UTC".to_owned(), |zone| zone.name().to_owned());
        return Ok((format_storage_instant(instant), zone_name));
    }

    let Some(zone) = zone else {
        return Err(StorageError::InvalidTimeZone);
    };

    for layout in LOCAL_DATE_TIME_LAYOUTS {
        let Some(parsed) = parse_layout(layout, date_time) else {
            continue;
        };
        let Some(wall) = civil_date_time(&parsed) else {
            continue;
        };
        let instant = zone.wall_clock_instant(wall);
        return Ok((format_storage_instant(instant), zone.name().to_owned()));
    }

    Err(StorageError::InvalidDateTime)
}

/// Normalize date/time form values.
#[must_use]
pub fn normalize_date_time_form_values(
    raw_date_time: &str,
    raw_time_zone: &str,
) -> (String, String, DateTimeValidationCode) {
    if raw_date_time.trim().is_empty() || raw_time_zone.trim().is_empty() {
        return (String::new(), String::new(), DATE_TIME_VALIDATION_REQUIRED);
    }
    match normalize_date_time_for_storage(raw_date_time, raw_time_zone) {
        Ok((date_time, time_zone)) => (date_time, time_zone, DATE_TIME_VALIDATION_VALID),
        Err(StorageError::InvalidTimeZone) => (
            String::new(),
            String::new(),
            DATE_TIME_VALIDATION_INVALID_TIME_ZONE,
        ),
        Err(StorageError::InvalidDateTime) => (
            String::new(),
            String::new(),
            DATE_TIME_VALIDATION_INVALID_DATE_TIME,
        ),
    }
}

/// Resolve the date-time validation message key.
#[must_use]
pub fn date_time_validation_message_key(
    code: DateTimeValidationCode,
    keys: &DateTimeValidationMessageKeys,
) -> String {
    match code.as_str() {
        "required" => keys.required.trim().to_owned(),
        "invalid-time-zone" => keys.invalid_time_zone.trim().to_owned(),
        "invalid-date-time" => keys.invalid_date_time.trim().to_owned(),
        _ => String::new(),
    }
}

/// Normalize form values with a message key.
#[must_use]
pub fn normalize_date_time_form_values_with_message_key(
    raw_date_time: &str,
    raw_time_zone: &str,
    keys: &DateTimeValidationMessageKeys,
) -> (String, String, String) {
    let (date_time, time_zone, code) =
        normalize_date_time_form_values(raw_date_time, raw_time_zone);
    if code == DATE_TIME_VALIDATION_VALID {
        return (date_time, time_zone, String::new());
    }
    (
        String::new(),
        String::new(),
        date_time_validation_message_key(code, keys),
    )
}

/// Normalize form values with a translated message.
///
/// `translate` is the caller's optional translator: `None` is an absent callback
/// and returns the message key itself.
#[must_use]
pub fn normalize_date_time_form_values_with_translated_message(
    translate: Option<&DateTimeValidationTranslator>,
    raw_date_time: &str,
    raw_time_zone: &str,
    keys: &DateTimeValidationMessageKeys,
) -> (String, String, String) {
    let (date_time, time_zone, key) =
        normalize_date_time_form_values_with_message_key(raw_date_time, raw_time_zone, keys);
    if key.is_empty() {
        return (date_time, time_zone, String::new());
    }
    let Some(translate) = translate else {
        return (String::new(), String::new(), key);
    };
    (String::new(), String::new(), translate(&key))
}

/// Parse a date/time for display.
///
/// Returns the projected civil value and whether the matched layout carried a time of
/// day. An instant is projected into the declared zone (or `UTC` when the zone is blank
/// or unresolvable); a zoned legacy wall clock is interpreted in the declared zone, not
/// assumed to be UTC.
fn parse_display(date_time: &str, time_zone: &str) -> Option<(ZonedDateTime, bool)> {
    let value = date_time.trim();
    if value.is_empty() {
        return None;
    }
    let zone = TemporalZone::normalize(time_zone).unwrap_or(None);
    if let Some(zone) = zone {
        if let Some(instant) = parse_instant(value) {
            return Some((zone.local_at(instant), true));
        }
        for layout in LOCAL_DATE_TIME_LAYOUTS {
            let Some(parsed) = parse_layout(layout, value) else {
                continue;
            };
            let Some(wall) = civil_date_time(&parsed) else {
                continue;
            };
            let has_time =
                layout == LOCAL_DATE_TIME_LAYOUT || layout == LOCAL_DATE_TIME_MINUTE_LAYOUT;
            return Some((zone.local_at(zone.wall_clock_instant(wall)), has_time));
        }
        return None;
    }

    // The declared zone was invalid: fall back to UTC.
    let utc = TemporalZone::utc();
    if let Some(instant) = parse_instant(value) {
        return Some((utc.local_at(instant), true));
    }
    for layout in LOCAL_DATE_TIME_LAYOUTS {
        let Some(parsed) = parse_layout(layout, value) else {
            continue;
        };
        let Some(wall) = civil_date_time(&parsed) else {
            continue;
        };
        let has_time = layout == LOCAL_DATE_TIME_LAYOUT || layout == LOCAL_DATE_TIME_MINUTE_LAYOUT;
        return Some((utc.local_at(utc.wall_clock_instant(wall)), has_time));
    }
    None
}

/// A display-parsed date/time value and whether it carried a time of day.
#[derive(Clone, Debug)]
pub struct DisplayDateTime {
    /// The value projected into its declared zone.
    pub value: ZonedDateTime,
    /// Whether the matched layout carried a time of day.
    pub has_time: bool,
}

/// Parse a display date/time without a declared zone.
#[must_use]
pub fn parse_display_date_time(date_time: &str) -> Option<DisplayDateTime> {
    parse_display(date_time, "").map(|(value, has_time)| DisplayDateTime { value, has_time })
}

/// Parse a display date.
#[must_use]
pub fn parse_display_date(date_time: &str) -> Option<ZonedDateTime> {
    parse_display_date_time(date_time).map(|parsed| parsed.value)
}

/// The time-zone label.
fn time_zone_label(date_time: &str, time_zone: &str) -> String {
    if let Some((value, _has_time)) = parse_display(date_time, time_zone) {
        let label = value.zone_name().trim();
        if !label.is_empty() {
            return label.to_owned();
        }
    }
    let normalized = TemporalZone::normalize(time_zone).unwrap_or(None);
    match normalized {
        Some(zone) if !zone.name().trim().is_empty() => zone.name().to_owned(),
        _ => "UTC".to_owned(),
    }
}

/// The localized month name for a one-based month, with the key fallback.
fn localized_month_name<C: TemporalContext + ?Sized>(ctx: &C, month: u32) -> String {
    let english = ENGLISH_MONTHS
        .get(usize::try_from(month.saturating_sub(1)).unwrap_or(usize::MAX))
        .copied()
        .unwrap_or_default();
    let Some(key) = MONTH_KEYS
        .get(usize::try_from(month).unwrap_or(usize::MAX))
        .copied()
    else {
        return english.to_owned();
    };
    if key.is_empty() {
        return english.to_owned();
    }
    let name = ctx.translate(key, &[]);
    if name == key {
        english.to_owned()
    } else {
        name
    }
}

/// Format a localized date.
fn format_localized_date<C: TemporalContext + ?Sized>(ctx: &C, value: &ZonedDateTime) -> String {
    let month_name = localized_month_name(ctx, value.month());
    let day = i64::from(value.day());
    let year = i64::from(value.year());
    let formatted = ctx.translate(
        DATE_FORMAT_KEY,
        &[
            FormatArg::Int(day),
            FormatArg::Text(&month_name),
            FormatArg::Int(year),
        ],
    );
    if formatted == DATE_FORMAT_KEY {
        return format!("{month_name} {day}, {year}");
    }
    formatted
}

/// Format a localized time.
///
/// The translated `time.time_format` value is a layout string (the documented default
/// is `15:04`, and `3:04 PM` when `component.datetimepicker.use_24_hour` is `false`), so
/// it is applied as a layout rather than as `strftime`.
fn format_localized_time<C: TemporalContext + ?Sized>(ctx: &C, value: &ZonedDateTime) -> String {
    let translated = ctx.translate(TIME_FORMAT_KEY, &[]);
    let mut layout = translated.trim().to_owned();
    if layout.is_empty() || layout == TIME_FORMAT_KEY {
        layout = "15:04".to_owned();
        if ctx.translate(USE_24_HOUR_KEY, &[]) == "false" {
            layout = "3:04 PM".to_owned();
        }
    }
    format_layout(value, &layout)
}

/// Format a localized date/time.
#[must_use]
pub(crate) fn format_localized_date_time_in_zone<C: TemporalContext + ?Sized>(
    ctx: &C,
    date_time: &str,
    time_zone: &str,
    short: bool,
) -> String {
    let Some((value, has_time)) = parse_display(date_time, time_zone) else {
        return date_time.to_owned();
    };

    let date_text = format_localized_date(ctx, &value);
    if !has_time {
        return date_text;
    }

    let time_text = format_localized_time(ctx, &value);
    let format_key = if short {
        DATE_TIME_SHORT_FORMAT_KEY
    } else {
        DATE_TIME_FORMAT_KEY
    };
    let mut formatted = ctx.translate(
        format_key,
        &[FormatArg::Text(&date_text), FormatArg::Text(&time_text)],
    );
    if formatted == format_key {
        formatted = if short {
            format!("{date_text} · {time_text}")
        } else {
            format!("{date_text} {time_text}")
        };
    }

    let zone_label = time_zone_label(date_time, time_zone);
    if zone_label.trim().is_empty() {
        return formatted;
    }
    let with_zone = ctx.translate(
        WITH_TIME_ZONE_KEY,
        &[FormatArg::Text(&formatted), FormatArg::Text(&zone_label)],
    );
    if with_zone == WITH_TIME_ZONE_KEY {
        format!("{formatted} · {zone_label}")
    } else {
        with_zone
    }
}

/// Localized date/time with a background context and no zone.
#[must_use]
pub fn format_localized_date_time<C: TemporalContext + ?Sized>(ctx: &C, date_time: &str) -> String {
    format_localized_date_time_in_zone(ctx, date_time, "", false)
}

/// Localized short date/time with a background context and no zone.
#[must_use]
pub fn format_localized_date_short<C: TemporalContext + ?Sized>(
    ctx: &C,
    date_time: &str,
) -> String {
    format_localized_date_time_in_zone(ctx, date_time, "", true)
}

/// Format the three source UNIX layouts without chrono's civil-year bound.
fn format_unix(unix_ts: i64, local: &TemporalZone, layout: &str) -> String {
    if unix_ts == 0 {
        return String::new();
    }
    // Go's presentation epoch is March 1, -292277022400. Its unsigned
    // absolute seconds retain the source wrapping behavior at i64 extremes.
    const ABSOLUTE_YEARS: i128 = 292_277_022_400;
    const DAYS_PER_ERA: i128 = 146_097;
    const UNIX_TO_ABSOLUTE: i128 = (ABSOLUTE_YEARS / 400 * DAYS_PER_ERA + 719_468) * 86_400;
    let seconds =
        (i128::from(unix_ts) + i128::from(local.unix_offset_seconds(unix_ts)) + UNIX_TO_ABSOLUTE)
            .rem_euclid(1_i128 << 64);
    let days = seconds / 86_400;
    let era = days / DAYS_PER_ERA;
    let day_of_era = days % DAYS_PER_ERA;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * march_month + 2) / 5 + 1;
    let month = march_month + if march_month < 10 { 3 } else { -9 };
    let year = era * 400 + year_of_era - ABSOLUTE_YEARS + i128::from(month <= 2);
    let year_text = if year < 0 {
        format!("-{:04}", -year)
    } else {
        format!("{year:04}")
    };
    if layout == LOCAL_DATE_LAYOUT {
        return format!("{year_text}-{month:02}-{day:02}");
    }
    let month_name = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(month - 1) as usize];
    if layout == "Jan 2, 2006 15:04" {
        let daytime = seconds % 86_400;
        format!(
            "{month_name} {day}, {year_text} {:02}:{:02}",
            daytime / 3600,
            daytime / 60 % 60
        )
    } else {
        format!("{month_name} {day}, {year_text}")
    }
}

/// Format a UNIX timestamp as a date.
///
/// Formatting uses the selected local zone, supplied
/// explicitly instead of resolved from the process behind the caller's back.
#[must_use]
pub fn format_date(unix_ts: i64, local: &TemporalZone) -> String {
    format_unix(unix_ts, local, "Jan 2, 2006")
}

/// Format a UNIX timestamp as a date/time.
#[must_use]
pub fn format_date_time(unix_ts: i64, local: &TemporalZone) -> String {
    format_unix(unix_ts, local, "Jan 2, 2006 15:04")
}

/// Format a UNIX timestamp as an ISO date.
#[must_use]
pub fn format_date_iso(unix_ts: i64, local: &TemporalZone) -> String {
    format_unix(unix_ts, local, LOCAL_DATE_LAYOUT)
}

/// Resolve the normalized time-zone name.
#[must_use]
pub fn normalized_time_zone_name(time_zone: &str) -> String {
    match TemporalZone::normalize(time_zone).unwrap_or(None) {
        Some(zone) if !zone.name().trim().is_empty() => zone.name().to_owned(),
        _ => "UTC".to_owned(),
    }
}

/// The local date in a time zone.
#[must_use]
pub fn local_date_in_time_zone(date_time: &str, time_zone: &str) -> Option<ZonedDateTime> {
    parse_display(date_time, time_zone).map(|(value, _has_time)| value)
}

/// The temporal status at an explicit `now` value.
///
/// The declared zone (when it resolves) is the zone `now` is projected into; an
/// unparseable value has no status, so the result is `None`.
#[must_use]
pub fn temporal_status_at(
    date_time: &str,
    time_zone: &str,
    now: &TemporalInstant,
) -> Option<&'static str> {
    let (local_time, _has_time) = parse_display(date_time, time_zone)?;
    let mut projected_now = now.local();
    let zone_name = time_zone.trim();
    if !zone_name.is_empty()
        && let Some(zone) = TemporalZone::iana(zone_name)
    {
        projected_now = zone.local_at(now.instant());
    }
    let today = projected_now.start_of_day();
    let local_day = local_time.start_of_day();
    if local_day == today {
        return Some(TEMPORAL_STATUS_TODAY);
    }
    if local_day > today {
        return Some(TEMPORAL_STATUS_UPCOMING);
    }
    Some(TEMPORAL_STATUS_PAST)
}
