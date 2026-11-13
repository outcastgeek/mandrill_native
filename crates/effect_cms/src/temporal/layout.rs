//! Layout-string formatting and parsing for date/time values.
//!
//! The CMS uses layout strings rather than `strftime`: the timezone label is literally
//! `MST`, the local time layout is the caller-translated `time.time_format`, whose
//! documented default is `15:04`, and the RFC3339 storage layout is
//! `2006-01-02T15:04:05Z07:00`. Formatting and parsing recognize one chunk of the
//! layout at a time; unrecognized text is copied verbatim, and recognized chunks are
//! rendered from or matched against the civil fields.
//!
//! # Formatting coverage
//!
//! Formatting covers every chunk the layout language defines — year, month, day,
//! weekday, day-of-year, hour, minute, second, AM/PM, fractional seconds and every
//! numeric/`Z` zone form — because the caller supplies the `time.time_format` and
//! `date_format` translations and this owner must format whatever layout the
//! translation table contains.
//!
//! # Parsing coverage
//!
//! Parsing covers the five local layouts (`2006-01-02T15:04:05`, `2006-01-02T15:04`,
//! `2006-01-02`, `January 2, 2006`, `Monday, January 2, 2006`) and RFC3339/RFC3339Nano.
//! Those use month/weekday names, numeric date and clock fields and the numeric/`Z`
//! zone forms; a chunk outside that set (the `MST` abbreviation and day-of-year forms)
//! reports "not this layout" rather than guessing a value.

use crate::temporal::zone::ZonedDateTime;

const MONTHS_LONG: [&str; 12] = [
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

const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const WEEKDAYS_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

const WEEKDAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// One recognized layout chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Std {
    /// `January`
    LongMonth,
    /// `Jan`
    Month,
    /// `1`
    NumMonth,
    /// `01`
    ZeroMonth,
    /// `Monday`
    LongWeekday,
    /// `Mon`
    Weekday,
    /// `2`
    Day,
    /// `_2`
    UnderDay,
    /// `02`
    ZeroDay,
    /// `__2`
    UnderYearDay,
    /// `002`
    ZeroYearDay,
    /// `2006`
    LongYear,
    /// `06`
    Year,
    /// `15`
    Hour,
    /// `3`
    Hour12,
    /// `03`
    ZeroHour12,
    /// `4`
    Minute,
    /// `04`
    ZeroMinute,
    /// `5`
    Second,
    /// `05`
    ZeroSecond,
    /// `PM`
    Pm,
    /// `pm`
    PmLower,
    /// `MST`
    Tz,
    /// `-0700`
    NumTz,
    /// `-07`
    NumShortTz,
    /// `-07:00`
    NumColonTz,
    /// `-070000`
    NumSecondsTz,
    /// `-07:00:00`
    NumColonSecondsTz,
    /// `Z0700`
    IsoTz,
    /// `Z07`
    IsoShortTz,
    /// `Z07:00`
    IsoColonTz,
    /// `Z070000`
    IsoSecondsTz,
    /// `Z07:00:00`
    IsoColonSecondsTz,
    /// `,000`/`.000`/`,999`/`.999` fractional second.
    Frac {
        /// Number of requested digits.
        digits: usize,
        /// Whether the separator is `,` instead of `.`.
        comma: bool,
        /// Whether trailing zeros are trimmed (the `9` form).
        trim: bool,
    },
}

/// Byte at an index, without panicking on a short or non-boundary slice.
fn byte_at(text: &str, index: usize) -> Option<u8> {
    text.as_bytes().get(index).copied()
}

/// Whether the byte at an index is an ASCII digit.
fn is_digit_at(text: &str, index: usize) -> bool {
    byte_at(text, index).is_some_and(|byte| byte.is_ascii_digit())
}

/// Whether `text` continues with `prefix` at `index`.
fn starts_with_at(text: &str, index: usize, prefix: &str) -> bool {
    text.get(index..index.saturating_add(prefix.len())) == Some(prefix)
}

/// Whether the character after an index is an ASCII lower-case letter.
/// `startsWithLowerCase` guard that keeps `January`/`Jan` and `Monday`/`Mon` apart.
fn starts_with_lower_case(text: &str, index: usize) -> bool {
    text.get(index..)
        .and_then(|rest| rest.as_bytes().first())
        .is_some_and(|byte| byte.is_ascii_lowercase())
}

/// Split the layout at its next recognized chunk: `(prefix length, chunk, consumed)`.
fn next_std_chunk(layout: &str) -> (usize, Option<Std>, usize) {
    for index in 0..layout.len() {
        let Some(byte) = byte_at(layout, index) else {
            break;
        };
        match byte {
            b'J' => {
                if starts_with_at(layout, index, "January") {
                    return (index, Some(Std::LongMonth), index + 7);
                }
                if starts_with_at(layout, index, "Jan")
                    && !starts_with_lower_case(layout, index + 3)
                {
                    return (index, Some(Std::Month), index + 3);
                }
            }
            b'M' => {
                if starts_with_at(layout, index, "Monday") {
                    return (index, Some(Std::LongWeekday), index + 6);
                }
                if starts_with_at(layout, index, "Mon")
                    && !starts_with_lower_case(layout, index + 3)
                {
                    return (index, Some(Std::Weekday), index + 3);
                }
                if starts_with_at(layout, index, "MST") {
                    return (index, Some(Std::Tz), index + 3);
                }
            }
            b'0' => {
                if let Some(digit) = byte_at(layout, index + 1)
                    && (b'1'..=b'6').contains(&digit)
                {
                    let chunk = match digit {
                        b'1' => Std::ZeroMonth,
                        b'2' => Std::ZeroDay,
                        b'3' => Std::ZeroHour12,
                        b'4' => Std::ZeroMinute,
                        b'5' => Std::ZeroSecond,
                        _ => Std::Year,
                    };
                    return (index, Some(chunk), index + 2);
                }
                if starts_with_at(layout, index, "002") {
                    return (index, Some(Std::ZeroYearDay), index + 3);
                }
            }
            b'1' => {
                if byte_at(layout, index + 1) == Some(b'5') {
                    return (index, Some(Std::Hour), index + 2);
                }
                return (index, Some(Std::NumMonth), index + 1);
            }
            b'2' => {
                if starts_with_at(layout, index, "2006") {
                    return (index, Some(Std::LongYear), index + 4);
                }
                return (index, Some(Std::Day), index + 1);
            }
            b'_' => {
                if byte_at(layout, index + 1) == Some(b'2') {
                    if starts_with_at(layout, index + 1, "2006") {
                        return (index + 1, Some(Std::LongYear), index + 5);
                    }
                    return (index, Some(Std::UnderDay), index + 2);
                }
                if starts_with_at(layout, index, "__2") {
                    return (index, Some(Std::UnderYearDay), index + 3);
                }
            }
            b'3' => return (index, Some(Std::Hour12), index + 1),
            b'4' => return (index, Some(Std::Minute), index + 1),
            b'5' => return (index, Some(Std::Second), index + 1),
            b'P' => {
                if byte_at(layout, index + 1) == Some(b'M') {
                    return (index, Some(Std::Pm), index + 2);
                }
            }
            b'p' => {
                if byte_at(layout, index + 1) == Some(b'm') {
                    return (index, Some(Std::PmLower), index + 2);
                }
            }
            b'-' => {
                let chunks = [
                    ("-070000", Std::NumSecondsTz, 7_usize),
                    ("-07:00:00", Std::NumColonSecondsTz, 9),
                    ("-0700", Std::NumTz, 5),
                    ("-07:00", Std::NumColonTz, 6),
                    ("-07", Std::NumShortTz, 3),
                ];
                for (text, chunk, consumed) in chunks {
                    if starts_with_at(layout, index, text) {
                        return (index, Some(chunk), index + consumed);
                    }
                }
            }
            b'Z' => {
                let chunks = [
                    ("Z070000", Std::IsoSecondsTz, 7_usize),
                    ("Z07:00:00", Std::IsoColonSecondsTz, 9),
                    ("Z0700", Std::IsoTz, 5),
                    ("Z07:00", Std::IsoColonTz, 6),
                    ("Z07", Std::IsoShortTz, 3),
                ];
                for (text, chunk, consumed) in chunks {
                    if starts_with_at(layout, index, text) {
                        return (index, Some(chunk), index + consumed);
                    }
                }
            }
            b'.' | b',' => {
                if let Some(digit) = byte_at(layout, index + 1)
                    && (digit == b'0' || digit == b'9')
                {
                    let mut end = index + 1;
                    while byte_at(layout, end) == Some(digit) {
                        end += 1;
                    }
                    if !is_digit_at(layout, end) {
                        return (
                            index,
                            Some(Std::Frac {
                                digits: end - (index + 1),
                                comma: byte == b',',
                                trim: digit == b'9',
                            }),
                            end,
                        );
                    }
                }
            }
            _ => {}
        }
    }
    (layout.len(), None, layout.len())
}

/// Append a zero-padded decimal value with a width.
fn append_int(out: &mut String, value: i64, width: usize) {
    let text = value.to_string();
    for _ in text.len()..width {
        out.push('0');
    }
    out.push_str(&text);
}

/// Append the fractional second.
fn append_nano(out: &mut String, nanosecond: u32, digits: usize, comma: bool, trim: bool) {
    let fraction = format!("{nanosecond:09}");
    let requested = fraction.get(..digits).unwrap_or(fraction.as_str());
    if trim {
        let trimmed = requested.trim_end_matches('0');
        if !trimmed.is_empty() {
            out.push(if comma { ',' } else { '.' });
            out.push_str(trimmed);
        }
        return;
    }
    out.push(if comma { ',' } else { '.' });
    out.push_str(requested);
}

/// Render a numeric zone form.
fn append_zone(out: &mut String, offset_seconds: i32, chunk: Std) {
    let iso = matches!(
        chunk,
        Std::IsoTz | Std::IsoShortTz | Std::IsoColonTz | Std::IsoSecondsTz | Std::IsoColonSecondsTz
    );
    if iso && offset_seconds == 0 {
        out.push('Z');
        return;
    }
    let mut remaining = offset_seconds;
    if remaining < 0 {
        out.push('-');
        remaining = -remaining;
    } else {
        out.push('+');
    }
    let zone_minutes = remaining / 60;
    append_int(out, i64::from(zone_minutes / 60), 2);
    if matches!(
        chunk,
        Std::IsoColonTz | Std::NumColonTz | Std::IsoColonSecondsTz | Std::NumColonSecondsTz
    ) {
        out.push(':');
    }
    if !matches!(chunk, Std::NumShortTz | Std::IsoShortTz) {
        append_int(out, i64::from(zone_minutes % 60), 2);
    }
    if matches!(
        chunk,
        Std::IsoSecondsTz | Std::NumSecondsTz | Std::NumColonSecondsTz | Std::IsoColonSecondsTz
    ) {
        if matches!(chunk, Std::NumColonSecondsTz | Std::IsoColonSecondsTz) {
            out.push(':');
        }
        append_int(out, i64::from(remaining % 60), 2);
    }
}

/// Format an instant with a layout string.
pub(crate) fn format_layout(value: &ZonedDateTime, layout: &str) -> String {
    let mut out = String::with_capacity(layout.len() + layout.len() / 2);
    let mut remaining = layout;
    loop {
        let (prefix_len, chunk, consumed) = next_std_chunk(remaining);
        if let Some(prefix) = remaining.get(..prefix_len) {
            out.push_str(prefix);
        }
        let Some(chunk) = chunk else {
            break;
        };
        let Some(suffix) = remaining.get(consumed..) else {
            break;
        };
        remaining = suffix;
        match chunk {
            Std::Year => {
                let year = i64::from(value.year());
                append_int(&mut out, year.abs() % 100, 2);
            }
            Std::LongYear => append_int(&mut out, i64::from(value.year()), 4),
            Std::Month => out.push_str(
                MONTHS_SHORT
                    .get(usize::try_from(value.month().saturating_sub(1)).unwrap_or_default())
                    .copied()
                    .unwrap_or_default(),
            ),
            Std::LongMonth => out.push_str(
                MONTHS_LONG
                    .get(usize::try_from(value.month().saturating_sub(1)).unwrap_or_default())
                    .copied()
                    .unwrap_or_default(),
            ),
            Std::NumMonth => append_int(&mut out, i64::from(value.month()), 0),
            Std::ZeroMonth => append_int(&mut out, i64::from(value.month()), 2),
            Std::Weekday => out.push_str(
                WEEKDAYS_SHORT
                    .get(usize::try_from(value.weekday_number()).unwrap_or_default())
                    .copied()
                    .unwrap_or_default(),
            ),
            Std::LongWeekday => out.push_str(
                WEEKDAYS_LONG
                    .get(usize::try_from(value.weekday_number()).unwrap_or_default())
                    .copied()
                    .unwrap_or_default(),
            ),
            Std::Day => append_int(&mut out, i64::from(value.day()), 0),
            Std::UnderDay => {
                if value.day() < 10 {
                    out.push(' ');
                }
                append_int(&mut out, i64::from(value.day()), 0);
            }
            Std::ZeroDay => append_int(&mut out, i64::from(value.day()), 2),
            Std::UnderYearDay => {
                let year_day = value.year_day();
                if year_day < 100 {
                    out.push(' ');
                    if year_day < 10 {
                        out.push(' ');
                    }
                }
                append_int(&mut out, i64::from(year_day), 0);
            }
            Std::ZeroYearDay => append_int(&mut out, i64::from(value.year_day()), 3),
            Std::Hour => append_int(&mut out, i64::from(value.hour()), 2),
            Std::Hour12 => {
                let mut hour = value.hour() % 12;
                if hour == 0 {
                    hour = 12;
                }
                append_int(&mut out, i64::from(hour), 0);
            }
            Std::ZeroHour12 => {
                let mut hour = value.hour() % 12;
                if hour == 0 {
                    hour = 12;
                }
                append_int(&mut out, i64::from(hour), 2);
            }
            Std::Minute => append_int(&mut out, i64::from(value.minute()), 0),
            Std::ZeroMinute => append_int(&mut out, i64::from(value.minute()), 2),
            Std::Second => append_int(&mut out, i64::from(value.second()), 0),
            Std::ZeroSecond => append_int(&mut out, i64::from(value.second()), 2),
            Std::Pm => out.push_str(if value.hour() >= 12 { "PM" } else { "AM" }),
            Std::PmLower => out.push_str(if value.hour() >= 12 { "pm" } else { "am" }),
            Std::Tz => {
                let label = value.zone_name();
                if label.is_empty() {
                    append_zone(&mut out, value.offset_seconds(), Std::NumTz);
                } else {
                    out.push_str(label);
                }
            }
            Std::Frac {
                digits,
                comma,
                trim,
            } => append_nano(&mut out, value.nanosecond(), digits, comma, trim),
            zone_chunk @ (Std::NumTz
            | Std::NumShortTz
            | Std::NumColonTz
            | Std::NumSecondsTz
            | Std::NumColonSecondsTz
            | Std::IsoTz
            | Std::IsoShortTz
            | Std::IsoColonTz
            | Std::IsoSecondsTz
            | Std::IsoColonSecondsTz) => append_zone(&mut out, value.offset_seconds(), zone_chunk),
        }
    }
    out
}

/// Parse a value against a layout string; `None` when it does not match.
///
/// The result keeps the parsed civil fields and the parsed zone offset (when the
/// layout carries one). A layout with no offset leaves the fields as a wall clock,
/// which the caller interprets in its selected zone.
pub(crate) fn parse_layout(layout: &str, value: &str) -> Option<LayoutParse> {
    let mut remaining_layout = layout;
    let mut remaining_value = value;
    let mut year: i32 = 0;
    let mut month: Option<u32> = None;
    let mut day: Option<u32> = None;
    let mut hour: u32 = 0;
    let mut minute: u32 = 0;
    let mut second: u32 = 0;
    let mut nanosecond: u32 = 0;
    let mut offset_seconds: Option<i32> = None;
    let mut post_meridiem = false;
    let mut ante_meridiem = false;

    loop {
        let (prefix_len, chunk, consumed) = next_std_chunk(remaining_layout);
        let prefix = remaining_layout.get(..prefix_len)?;
        remaining_value = skip(remaining_value, prefix)?;
        let Some(chunk) = chunk else {
            if !remaining_value.is_empty() {
                return None;
            }
            break;
        };
        remaining_layout = remaining_layout.get(consumed..)?;
        match chunk {
            Std::Month => {
                let (index, used) = lookup(&MONTHS_SHORT, remaining_value)?;
                remaining_value = remaining_value.get(used..)?;
                month = Some(month_number(index)?);
            }
            Std::LongMonth => {
                let (index, used) = lookup(&MONTHS_LONG, remaining_value)?;
                remaining_value = remaining_value.get(used..)?;
                month = Some(month_number(index)?);
            }
            Std::Weekday => {
                let (_index, used) = lookup(&WEEKDAYS_SHORT, remaining_value)?;
                remaining_value = remaining_value.get(used..)?;
            }
            Std::LongWeekday => {
                let (_index, used) = lookup(&WEEKDAYS_LONG, remaining_value)?;
                remaining_value = remaining_value.get(used..)?;
            }
            Std::NumMonth | Std::ZeroMonth => {
                let (number, used) = getnum(remaining_value, chunk == Std::ZeroMonth)?;
                remaining_value = remaining_value.get(used..)?;
                if !(1..=12).contains(&number) {
                    return None;
                }
                month = Some(number);
            }
            Std::Day | Std::UnderDay | Std::ZeroDay => {
                if chunk == Std::UnderDay && remaining_value.starts_with(' ') {
                    remaining_value = remaining_value.get(1..)?;
                }
                let (number, used) = getnum(remaining_value, chunk == Std::ZeroDay)?;
                remaining_value = remaining_value.get(used..)?;
                day = Some(number);
            }
            Std::Hour => {
                let (number, used) = getnum(remaining_value, false)?;
                remaining_value = remaining_value.get(used..)?;
                if number > 23 {
                    return None;
                }
                hour = number;
            }
            Std::Hour12 | Std::ZeroHour12 => {
                let (number, used) = getnum(remaining_value, chunk == Std::ZeroHour12)?;
                remaining_value = remaining_value.get(used..)?;
                if !(1..=12).contains(&number) {
                    return None;
                }
                hour = number;
            }
            Std::Minute | Std::ZeroMinute => {
                let (number, used) = getnum(remaining_value, chunk == Std::ZeroMinute)?;
                remaining_value = remaining_value.get(used..)?;
                if number > 59 {
                    return None;
                }
                minute = number;
            }
            Std::Second | Std::ZeroSecond => {
                let (number, used) = getnum(remaining_value, chunk == Std::ZeroSecond)?;
                remaining_value = remaining_value.get(used..)?;
                if number > 59 {
                    return None;
                }
                second = number;
                // A fractional second is accepted even when the layout omits one; the
                // RFC3339 parse relies on it for stored nanosecond values.
                let next_chunk = next_std_chunk(remaining_layout).1;
                let layout_has_fraction = matches!(next_chunk, Some(Std::Frac { .. }));
                if !layout_has_fraction
                    && (remaining_value.starts_with('.') || remaining_value.starts_with(','))
                    && is_digit_at(remaining_value, 1)
                {
                    let mut digits = 2;
                    while is_digit_at(remaining_value, digits) {
                        digits += 1;
                    }
                    let (parsed, used) = parse_nanoseconds(remaining_value, digits)?;
                    nanosecond = parsed;
                    remaining_value = remaining_value.get(used..)?;
                }
            }
            Std::Pm | Std::PmLower => {
                let (ante, post) = if chunk == Std::Pm {
                    ("AM", "PM")
                } else {
                    ("am", "pm")
                };
                let marker = remaining_value.get(..2)?;
                remaining_value = remaining_value.get(2..)?;
                if marker == ante {
                    ante_meridiem = true;
                } else if marker == post {
                    post_meridiem = true;
                } else {
                    return None;
                }
            }
            Std::LongYear => {
                let (number, used) = getnum4(remaining_value)?;
                remaining_value = remaining_value.get(used..)?;
                year = i32::try_from(number).ok()?;
            }
            Std::Year => {
                let (number, used) = getnum(remaining_value, true)?;
                remaining_value = remaining_value.get(used..)?;
                year = i32::try_from(number).ok()?;
            }
            Std::Frac {
                digits,
                comma,
                trim,
            } => {
                if trim {
                    if remaining_value.len() < 2
                        || !(remaining_value.starts_with('.') || remaining_value.starts_with(','))
                        || !is_digit_at(remaining_value, 1)
                    {
                        continue;
                    }
                    let mut consumed_digits = 0;
                    while is_digit_at(remaining_value, consumed_digits + 1) {
                        consumed_digits += 1;
                    }
                    let (parsed, used) = parse_nanoseconds(remaining_value, consumed_digits + 1)?;
                    nanosecond = parsed;
                    remaining_value = remaining_value.get(used..)?;
                } else {
                    let separator = if comma { ',' } else { '.' };
                    if !remaining_value.starts_with(separator) {
                        return None;
                    }
                    let needed = digits.saturating_add(1);
                    let (parsed, used) = parse_nanoseconds(remaining_value, needed)?;
                    nanosecond = parsed;
                    remaining_value = remaining_value.get(used..)?;
                }
            }
            Std::Tz | Std::UnderYearDay | Std::ZeroYearDay => return None,
            zone_chunk @ (Std::NumTz
            | Std::NumShortTz
            | Std::NumColonTz
            | Std::NumSecondsTz
            | Std::NumColonSecondsTz
            | Std::IsoTz
            | Std::IsoShortTz
            | Std::IsoColonTz
            | Std::IsoSecondsTz
            | Std::IsoColonSecondsTz) => {
                let (parsed_offset, rest) = consume_zone(remaining_value, zone_chunk)?;
                offset_seconds = Some(parsed_offset);
                remaining_value = rest;
            }
        }
    }

    if post_meridiem && hour < 12 {
        hour = hour.saturating_add(12);
    } else if ante_meridiem && hour == 12 {
        hour = 0;
    }

    let month = month.unwrap_or(1);
    let day = day.unwrap_or(1);
    if day == 0 || day > days_in_month(month, year) {
        return None;
    }

    Some(LayoutParse {
        year,
        month,
        day,
        hour,
        minute,
        second,
        nanosecond,
        offset_seconds,
    })
}

/// Parsed civil fields plus the layout's zone offset when it had one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LayoutParse {
    /// Civil year.
    pub(crate) year: i32,
    /// Civil month, `1`–`12`.
    pub(crate) month: u32,
    /// Civil day of month.
    pub(crate) day: u32,
    /// Civil hour, `0`–`23`.
    pub(crate) hour: u32,
    /// Civil minute.
    pub(crate) minute: u32,
    /// Civil second.
    pub(crate) second: u32,
    /// Nanosecond within the second.
    pub(crate) nanosecond: u32,
    /// Zone offset carried by the layout, in seconds east of UTC.
    pub(crate) offset_seconds: Option<i32>,
}

/// Read one or two digits; a fixed field means exactly two.
fn getnum(value: &str, fixed: bool) -> Option<(u32, usize)> {
    let first = byte_at(value, 0)?;
    if !first.is_ascii_digit() {
        return None;
    }
    let Some(second) = byte_at(value, 1) else {
        if fixed {
            return None;
        }
        return Some((u32::from(first - b'0'), 1));
    };
    if !second.is_ascii_digit() {
        if fixed {
            return None;
        }
        return Some((u32::from(first - b'0'), 1));
    }
    Some((u32::from(first - b'0') * 10 + u32::from(second - b'0'), 2))
}

/// Read exactly four digits.
fn getnum4(value: &str) -> Option<(u32, usize)> {
    let mut number: u32 = 0;
    for index in 0..4 {
        let digit = byte_at(value, index)?;
        if !digit.is_ascii_digit() {
            return None;
        }
        number = number * 10 + u32::from(digit - b'0');
    }
    Some((number, 4))
}

/// Literal layout text must appear in the value, with space runs equal.
fn skip<'a>(mut value: &'a str, mut prefix: &str) -> Option<&'a str> {
    while let Some(character) = prefix.chars().next() {
        if character == ' ' {
            if !value.starts_with(' ') {
                return None;
            }
            prefix = prefix.trim_start_matches(' ');
            value = value.trim_start_matches(' ');
            continue;
        }
        let rest = value.strip_prefix(character)?;
        value = rest;
        prefix = prefix.get(character.len_utf8()..)?;
    }
    Some(value)
}

/// Fold-matching name lookup, returning the index and consumed byte count.
fn lookup(table: &[&str], value: &str) -> Option<(usize, usize)> {
    for (index, name) in table.iter().enumerate() {
        if value.len() >= name.len()
            && value
                .get(..name.len())
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        {
            return Some((index, name.len()));
        }
    }
    None
}

/// One-based month number for a month table index.
fn month_number(index: usize) -> Option<u32> {
    u32::try_from(index.checked_add(1)?).ok()
}

/// Parse fractional seconds; `nbytes` counts the separator plus digits.
fn parse_nanoseconds(value: &str, nbytes: usize) -> Option<(u32, usize)> {
    if !(value.starts_with('.') || value.starts_with(',')) {
        return None;
    }
    let nbytes = nbytes.min(10);
    let digits = value.get(1..nbytes)?;
    let mut nanosecond: u32 = digits.parse().ok()?;
    for _ in 0..(10_usize.saturating_sub(nbytes)) {
        nanosecond = nanosecond.checked_mul(10)?;
    }
    Some((nanosecond, nbytes))
}

/// Parse a zone chunk and return its offset plus the remaining value.
///
/// The `Z` forms accept a literal `Z` for UTC (one byte) and otherwise parse the same
/// numeric offset forms as `-07:00`; the numeric forms require an explicit sign.
fn consume_zone(value: &str, chunk: Std) -> Option<(i32, &str)> {
    let iso = matches!(
        chunk,
        Std::IsoTz | Std::IsoShortTz | Std::IsoColonTz | Std::IsoSecondsTz | Std::IsoColonSecondsTz
    );
    if iso && value.starts_with('Z') {
        return Some((0, value.get(1..)?));
    }
    let consumed = match chunk {
        Std::IsoColonTz | Std::NumColonTz => 6,
        Std::NumShortTz | Std::IsoShortTz => 3,
        Std::IsoColonSecondsTz | Std::NumColonSecondsTz => 9,
        Std::IsoSecondsTz | Std::NumSecondsTz => 7,
        Std::NumTz | Std::IsoTz => 5,
        Std::LongMonth
        | Std::Month
        | Std::NumMonth
        | Std::ZeroMonth
        | Std::LongWeekday
        | Std::Weekday
        | Std::Day
        | Std::UnderDay
        | Std::ZeroDay
        | Std::UnderYearDay
        | Std::ZeroYearDay
        | Std::LongYear
        | Std::Year
        | Std::Hour
        | Std::Hour12
        | Std::ZeroHour12
        | Std::Minute
        | Std::ZeroMinute
        | Std::Second
        | Std::ZeroSecond
        | Std::Pm
        | Std::PmLower
        | Std::Tz
        | Std::Frac { .. } => return None,
    };
    let text = value.get(..consumed)?;
    let hours: i32 = text.get(1..3)?.parse().ok()?;
    let (minutes_text, seconds_text, sign) = match chunk {
        Std::IsoColonTz | Std::NumColonTz => {
            if text.get(3..4)? != ":" {
                return None;
            }
            (text.get(4..6)?, "00", text.get(..1)?)
        }
        Std::NumShortTz | Std::IsoShortTz => (text.get(1..3)?, "00", text.get(..1)?),
        Std::IsoColonSecondsTz | Std::NumColonSecondsTz => {
            if text.get(3..4)? != ":" || text.get(6..7)? != ":" {
                return None;
            }
            (text.get(4..6)?, text.get(7..9)?, text.get(..1)?)
        }
        Std::IsoSecondsTz | Std::NumSecondsTz => (text.get(3..5)?, text.get(5..7)?, text.get(..1)?),
        Std::NumTz | Std::IsoTz => (text.get(3..5)?, "00", text.get(..1)?),
        Std::LongMonth
        | Std::Month
        | Std::NumMonth
        | Std::ZeroMonth
        | Std::LongWeekday
        | Std::Weekday
        | Std::Day
        | Std::UnderDay
        | Std::ZeroDay
        | Std::UnderYearDay
        | Std::ZeroYearDay
        | Std::LongYear
        | Std::Year
        | Std::Hour
        | Std::Hour12
        | Std::ZeroHour12
        | Std::Minute
        | Std::ZeroMinute
        | Std::Second
        | Std::ZeroSecond
        | Std::Pm
        | Std::PmLower
        | Std::Tz
        | Std::Frac { .. } => return None,
    };
    let minutes: i32 = minutes_text.parse().ok()?;
    let seconds: i32 = seconds_text.parse().ok()?;
    if hours > 24 || minutes > 60 || seconds > 60 {
        return None;
    }
    let offset = (hours * 60 + minutes) * 60 + seconds;
    let offset = match sign {
        "+" => offset,
        "-" => -offset,
        _ => return None,
    };
    Some((offset, value.get(consumed..)?))
}

/// Days in a month.
fn days_in_month(month: u32, year: i32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            if leap { 29 } else { 28 }
        }
        _ => 0,
    }
}
