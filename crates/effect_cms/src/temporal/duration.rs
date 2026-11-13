//! Duration normalization and localized duration formatting.
//!
//! Duration values use this input language and formatting contract. The picker
//! component, its markup and its JavaScript are a different owner's subject and are
//! not reproduced here.
//!
//! # Precedence
//!
//! Normalization applies this precedence: trimmed typed text first (and an
//! invalid typed text fails normalization even when a canonical value is present), then
//! any nonblank structured part, then the hidden canonical value, and finally
//! all-blank input, which is valid and empty.
//!
//! # Input language
//!
//! A duration is signed integral canonical minutes, ISO week/day/hour/minute form,
//! `H:MM` with minutes `< 60`, compact `2h30`, or a multilingual tokenized form
//! (`2 semaines 3 jours 4 heures`) whose units are weeks/`wk`/`week`/`sem`/`semaine`/
//! `semana`, days/`jour`/`día`, hours/`hr`/`heure`/`hora` and minutes/`min`/`minuto`,
//! with fractional-unit rounding and the allowed separators
//! (`,`, `+`, `;`, `/`, `and`, `et`, `y`, `e`). Structured parts are nonnegative
//! integers, minutes are `0..=59`, and the value must be a multiple of the step; the
//! step default (`1`) and the maximum default (`0`, meaning unlimited) apply, and no
//! additional limit is invented.

use crate::temporal::translate::TemporalContext;

/// The default step in minutes.
pub const DEFAULT_STEP_MINUTES: i64 = 1;

/// The default maximum in minutes; `0` means unlimited.
pub const DEFAULT_MAX_MINUTES: i64 = 0;

const MINUTES_PER_HOUR: i64 = 60;
const MINUTES_PER_DAY: i64 = 24 * MINUTES_PER_HOUR;
const MINUTES_PER_WEEK: i64 = 7 * MINUTES_PER_DAY;

/// A parsed duration value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParsedDuration {
    /// Total minutes the value denotes.
    pub minutes: i64,
    /// Whether the value was a duration at all.
    pub valid: bool,
}

/// A duration breakdown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DurationParts {
    /// Whole weeks.
    pub weeks: i64,
    /// Whole days after weeks.
    pub days: i64,
    /// Whole hours after days.
    pub hours: i64,
    /// Minutes after hours.
    pub minutes: i64,
}

/// Take a leading digit run followed by one suffix character.
fn digits_then(value: &str, suffix: char) -> Option<(i64, &str)> {
    let digits = value.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let number = value.get(..digits)?.parse::<i64>().unwrap_or(0);
    let rest = value.get(digits..)?.strip_prefix(suffix)?;
    Some((number, rest))
}

/// Parse the ISO duration form (`p2w3dt4h5m`); `total > 0` is required.
fn parse_iso_duration(value: &str) -> Option<i64> {
    let mut rest = value.strip_prefix('p')?;
    let mut weeks = 0_i64;
    let mut days = 0_i64;
    let mut hours = 0_i64;
    let mut minutes = 0_i64;
    if let Some((number, tail)) = digits_then(rest, 'w') {
        weeks = number;
        rest = tail;
    }
    if let Some((number, tail)) = digits_then(rest, 'd') {
        days = number;
        rest = tail;
    }
    if let Some(tail) = rest.strip_prefix('t') {
        rest = tail;
        if let Some((number, tail)) = digits_then(rest, 'h') {
            hours = number;
            rest = tail;
        }
        if let Some((number, tail)) = digits_then(rest, 'm') {
            minutes = number;
            rest = tail;
        }
    }
    if !rest.is_empty() {
        return None;
    }
    Some(weeks * MINUTES_PER_WEEK + days * MINUTES_PER_DAY + hours * MINUTES_PER_HOUR + minutes)
}

/// Parse the `H:MM` form; minutes must be below 60.
fn parse_colon_duration(value: &str) -> Option<i64> {
    let (hours_text, minutes_text) = value.split_once(':')?;
    if hours_text.is_empty() || hours_text.len() > 4 || minutes_text.len() != 2 {
        return None;
    }
    if !hours_text.bytes().all(|byte| byte.is_ascii_digit())
        || !minutes_text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let hours = hours_text.parse::<i64>().ok()?;
    let minutes = minutes_text.parse::<i64>().ok()?;
    if minutes >= MINUTES_PER_HOUR {
        return None;
    }
    Some(hours * MINUTES_PER_HOUR + minutes)
}

/// Parse the compact `2h30` form; minutes must be below 60.
fn parse_compact_duration(value: &str) -> Option<i64> {
    let (hours_text, tail) = value.split_once('h')?;
    let hours_text = hours_text.trim();
    let minutes_text = tail.trim();
    if hours_text.is_empty()
        || minutes_text.is_empty()
        || minutes_text.len() > 2
        || !hours_text.bytes().all(|byte| byte.is_ascii_digit())
        || !minutes_text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let hours = hours_text.parse::<i64>().ok()?;
    let minutes = minutes_text.parse::<i64>().ok()?;
    if minutes >= MINUTES_PER_HOUR {
        return None;
    }
    Some(hours * MINUTES_PER_HOUR + minutes)
}

/// Parse the tokenized multilingual form.
fn parse_tokenized_duration(value: &str) -> Option<i64> {
    let mut total = 0_i64;
    let mut last_end = 0_usize;
    let mut found = false;
    let mut index = 0_usize;

    while index < value.len() {
        let Some(character) = value.get(index..).and_then(|rest| rest.chars().next()) else {
            break;
        };
        if !character.is_ascii_digit() {
            index += character.len_utf8();
            continue;
        }

        let number_start = index;
        index += character.len_utf8();
        while value.as_bytes().get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if value.as_bytes().get(index) == Some(&b'.')
            && value
                .as_bytes()
                .get(index + 1)
                .is_some_and(u8::is_ascii_digit)
        {
            index += 1;
            while value.as_bytes().get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
        }
        let number_end = index;

        while let Some(space) = value.get(index..).and_then(|rest| rest.chars().next())
            && space.is_whitespace()
        {
            index += space.len_utf8();
        }
        let unit_start = index;
        while let Some(letter) = value.get(index..).and_then(|rest| rest.chars().next())
            && letter.is_alphabetic()
        {
            index += letter.len_utf8();
        }
        let unit_end = index;
        if unit_end == unit_start {
            index = number_end;
            continue;
        }

        if !valid_duration_separator(value.get(last_end..number_start)?) {
            return None;
        }
        let numeric = value.get(number_start..number_end)?.parse::<f64>().ok()?;
        if numeric < 0.0 {
            return None;
        }
        let unit = value.get(unit_start..unit_end)?;
        if !known_duration_unit(unit) {
            return None;
        }
        let unit_minutes = duration_unit_minutes(unit);
        total = total.checked_add((numeric * unit_minutes + 0.5) as i64)?;
        last_end = unit_end;
        found = true;
    }

    if !found || !valid_duration_separator(value.get(last_end..)?) {
        return None;
    }
    Some(total)
}

/// Whether a segment is an allowed duration separator.
fn valid_duration_separator(segment: &str) -> bool {
    if segment.trim().is_empty() {
        return true;
    }
    let lowered = segment.to_lowercase();
    let mut leftover = String::new();
    let mut index = 0_usize;
    while let Some(rest) = lowered.get(index..) {
        if let Some(rest) = rest.strip_prefix("and") {
            index += "and".len();
            let _ = rest;
            continue;
        }
        if rest.starts_with("et") {
            index += "et".len();
            continue;
        }
        if rest.starts_with('y') || rest.starts_with('e') {
            index += 1;
            continue;
        }
        let Some(character) = rest.chars().next() else {
            break;
        };
        if matches!(character, ',' | '+' | ';' | '/') {
            index += character.len_utf8();
            continue;
        }
        leftover.push(character);
        index += character.len_utf8();
    }
    leftover.trim().is_empty()
}

/// The minutes represented by one unit.
fn duration_unit_minutes(unit: &str) -> f64 {
    match unit.trim().trim_end_matches('.').to_lowercase().as_str() {
        "w" | "wk" | "wks" | "week" | "weeks" | "sem" | "sems" | "semaine" | "semaines"
        | "semana" | "semanas" => MINUTES_PER_WEEK as f64,
        "d" | "day" | "days" | "jour" | "jours" | "dia" | "dias" | "día" | "días" => {
            MINUTES_PER_DAY as f64
        }
        "h" | "hr" | "hrs" | "hour" | "hours" | "heure" | "heures" | "hora" | "horas" => {
            MINUTES_PER_HOUR as f64
        }
        _ => 1.0,
    }
}

/// Whether a unit token is recognized.
fn known_duration_unit(unit: &str) -> bool {
    let normalized = unit.trim().trim_end_matches('.').to_lowercase();
    matches!(
        normalized.as_str(),
        "w" | "wk"
            | "wks"
            | "week"
            | "weeks"
            | "sem"
            | "sems"
            | "semaine"
            | "semaines"
            | "semana"
            | "semanas"
            | "d"
            | "day"
            | "days"
            | "jour"
            | "jours"
            | "dia"
            | "dias"
            | "día"
            | "días"
            | "h"
            | "hr"
            | "hrs"
            | "hour"
            | "hours"
            | "heure"
            | "heures"
            | "hora"
            | "horas"
            | "m"
            | "min"
            | "mins"
            | "minute"
            | "minutes"
            | "minuto"
            | "minutos"
    )
}

/// Parse a duration in the input-language precedence order.
#[must_use]
pub fn parse_duration(value: &str) -> ParsedDuration {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return ParsedDuration::default();
    }
    let lowered = trimmed.to_lowercase();
    if let Ok(numeric) = lowered.parse::<i64>() {
        return ParsedDuration {
            minutes: numeric,
            valid: true,
        };
    }
    if let Some(total) = parse_iso_duration(&lowered) {
        return ParsedDuration {
            minutes: total,
            valid: total > 0,
        };
    }
    if let Some(total) = parse_colon_duration(&lowered) {
        return ParsedDuration {
            minutes: total,
            valid: true,
        };
    }
    if let Some(total) = parse_compact_duration(&lowered) {
        return ParsedDuration {
            minutes: total,
            valid: true,
        };
    }
    match parse_tokenized_duration(&lowered) {
        Some(total) => ParsedDuration {
            minutes: total,
            valid: total > 0,
        },
        None => ParsedDuration::default(),
    }
}

/// Parse a duration into canonical minutes.
#[must_use]
pub fn parse_duration_minutes(value: &str) -> Option<i64> {
    let parsed = parse_duration(value);
    parsed.valid.then_some(parsed.minutes)
}

/// Normalize a minute count against the maximum.
fn normalize_minutes(minutes: i64, max_minutes: i64) -> (String, i64, bool) {
    if minutes <= 0 {
        return (String::new(), 0, true);
    }
    if max_minutes > 0 && minutes > max_minutes {
        return (String::new(), 0, false);
    }
    (minutes.to_string(), minutes, true)
}

/// Normalize a duration value.
#[must_use]
pub fn normalize_duration_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Some(String::new());
    }
    let minutes = parse_duration_minutes(trimmed)?;
    let (canonical, _minutes, ok) = normalize_minutes(minutes, DEFAULT_MAX_MINUTES);
    ok.then_some(canonical)
}

/// Canonical minutes from structured weeks/days/hours/minutes.
#[must_use]
pub fn canonical_minutes_from_structured_parts(
    weeks: &str,
    days: &str,
    hours: &str,
    minutes: &str,
    step_minutes: i64,
    max_minutes: i64,
) -> (String, i64, bool) {
    let weeks_text = weeks.trim();
    let days_text = days.trim();
    let hours_text = hours.trim();
    let minutes_text = minutes.trim();

    if weeks_text.is_empty()
        && days_text.is_empty()
        && hours_text.is_empty()
        && minutes_text.is_empty()
    {
        return (String::new(), 0, true);
    }

    let parse_part = |text: &str| -> Option<i64> {
        if text.is_empty() {
            return Some(0);
        }
        text.parse::<i64>().ok().filter(|value| *value >= 0)
    };
    let (Some(weeks), Some(days), Some(hours), Some(minutes)) = (
        parse_part(weeks_text),
        parse_part(days_text),
        parse_part(hours_text),
        parse_part(minutes_text),
    ) else {
        return (String::new(), 0, false);
    };
    if minutes >= MINUTES_PER_HOUR {
        return (String::new(), 0, false);
    }

    let step = if step_minutes <= 0 {
        DEFAULT_STEP_MINUTES
    } else {
        step_minutes
    };
    if minutes % step != 0 {
        return (String::new(), 0, false);
    }

    let total =
        weeks * MINUTES_PER_WEEK + days * MINUTES_PER_DAY + hours * MINUTES_PER_HOUR + minutes;
    normalize_minutes(total, max_minutes)
}

/// Canonical minutes from hours and minutes.
#[must_use]
pub fn canonical_minutes_from_parts(
    hours: &str,
    minutes: &str,
    step_minutes: i64,
    max_minutes: i64,
) -> (String, i64, bool) {
    canonical_minutes_from_structured_parts("", "", hours, minutes, step_minutes, max_minutes)
}

/// Break minutes into weeks/days/hours/minutes.
#[must_use]
pub fn breakdown_duration(total_minutes: i64) -> DurationParts {
    if total_minutes <= 0 {
        return DurationParts::default();
    }
    let weeks = total_minutes / MINUTES_PER_WEEK;
    let remainder = total_minutes % MINUTES_PER_WEEK;
    let days = remainder / MINUTES_PER_DAY;
    let remainder = remainder % MINUTES_PER_DAY;
    let hours = remainder / MINUTES_PER_HOUR;
    let minutes = remainder % MINUTES_PER_HOUR;
    DurationParts {
        weeks,
        days,
        hours,
        minutes,
    }
}

/// Localized duration parts.
fn localized_duration_parts<C: TemporalContext + ?Sized>(ctx: &C, minutes: i64) -> Vec<String> {
    if minutes <= 0 {
        return Vec::new();
    }
    let parts = breakdown_duration(minutes);
    let mut localized = Vec::with_capacity(4);
    let mut append = |value: i64, singular: &str, plural: &str| {
        if value <= 0 {
            return;
        }
        let mut unit = ctx.translate(plural, &[]);
        if value == 1 {
            unit = ctx.translate(singular, &[]);
        }
        localized.push(format!("{value} {unit}"));
    };
    append(parts.weeks, WEEK_SINGULAR_KEY, WEEK_PLURAL_KEY);
    append(parts.days, DAY_SINGULAR_KEY, DAY_PLURAL_KEY);
    append(parts.hours, HOUR_SINGULAR_KEY, HOUR_PLURAL_KEY);
    append(parts.minutes, MINUTE_SINGULAR_KEY, MINUTE_PLURAL_KEY);
    localized
}

/// `component.durationpicker.week_singular`.
const WEEK_SINGULAR_KEY: &str = "component.durationpicker.week_singular";
/// `component.durationpicker.week_plural`.
const WEEK_PLURAL_KEY: &str = "component.durationpicker.week_plural";
/// `component.durationpicker.day_singular`.
const DAY_SINGULAR_KEY: &str = "component.durationpicker.day_singular";
/// `component.durationpicker.day_plural`.
const DAY_PLURAL_KEY: &str = "component.durationpicker.day_plural";
/// `component.durationpicker.hour_singular`.
const HOUR_SINGULAR_KEY: &str = "component.durationpicker.hour_singular";
/// `component.durationpicker.hour_plural`.
const HOUR_PLURAL_KEY: &str = "component.durationpicker.hour_plural";
/// `component.durationpicker.minute_singular`.
const MINUTE_SINGULAR_KEY: &str = "component.durationpicker.minute_singular";
/// `component.durationpicker.minute_plural`.
const MINUTE_PLURAL_KEY: &str = "component.durationpicker.minute_plural";
/// `component.durationpicker.part_joiner`.
const PART_JOINER_KEY: &str = "component.durationpicker.part_joiner";

/// The localized part joiner.
fn localized_duration_joiner<C: TemporalContext + ?Sized>(ctx: &C) -> String {
    let joiner = ctx.translate(PART_JOINER_KEY, &[]);
    if joiner == PART_JOINER_KEY || joiner.trim().is_empty() || joiner == "space" {
        return " ".to_owned();
    }
    joiner
}

/// Normalize decoded duration form values.
///
/// Precedence: trimmed typed text first (an invalid typed text
/// fails even when the canonical value is valid), then any nonblank structured part,
/// then the hidden canonical value, and only then a valid empty result.
#[must_use]
pub fn normalize_duration_form_values(
    raw_canonical: &str,
    raw_text: &str,
    weeks: &str,
    days: &str,
    hours: &str,
    minutes: &str,
) -> (String, bool) {
    let raw_canonical = raw_canonical.trim();
    let raw_text = raw_text.trim();
    let weeks = weeks.trim();
    let days = days.trim();
    let hours = hours.trim();
    let minutes = minutes.trim();

    if !raw_text.is_empty() {
        return match normalize_duration_value(raw_text) {
            Some(canonical) => (canonical, true),
            None => (String::new(), false),
        };
    }

    if !weeks.is_empty() || !days.is_empty() || !hours.is_empty() || !minutes.is_empty() {
        let (canonical, _total, ok) = canonical_minutes_from_structured_parts(
            weeks,
            days,
            hours,
            minutes,
            DEFAULT_STEP_MINUTES,
            DEFAULT_MAX_MINUTES,
        );
        return if ok {
            (canonical, true)
        } else {
            (String::new(), false)
        };
    }

    if !raw_canonical.is_empty() {
        return match normalize_duration_value(raw_canonical) {
            Some(canonical) => (canonical, true),
            None => (String::new(), false),
        };
    }

    (String::new(), true)
}

/// Format a canonical or legacy value as localized human text.
/// An unparseable value is returned trimmed, a zero value formats as empty, and the
/// rendered parts keep the `"<value> <unit>"` shape joined by the translated part
/// joiner (defaulting to a single space).
#[must_use]
pub fn format_duration<C: TemporalContext + ?Sized>(ctx: &C, value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let Some(minutes) = parse_duration_minutes(trimmed) else {
        return trimmed.to_owned();
    };
    if minutes <= 0 {
        return String::new();
    }
    let parts = localized_duration_parts(ctx, minutes);
    if parts.is_empty() {
        return String::new();
    }
    parts.join(&localized_duration_joiner(ctx))
}
