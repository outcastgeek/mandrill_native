#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

type Attrs = BTreeMap<String, String>;
const DEFAULT_STEP_MINUTES: i32 = 1;
const DEFAULT_MAX_MINUTES: i32 = 0;
const MINUTES_PER_HOUR: i32 = 60;
const MINUTES_PER_DAY: i32 = 24 * MINUTES_PER_HOUR;
const MINUTES_PER_WEEK: i32 = 7 * MINUTES_PER_DAY;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedDuration {
    pub minutes: i32,
    pub valid: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurationParts {
    pub weeks: i32,
    pub days: i32,
    pub hours: i32,
    pub minutes: i32,
}

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "durationpicker",
        name: "DurationPicker",
    },
    ComponentInfo {
        family: "unstyled",
        module: "durationpicker",
        name: "DurationPickerWithLabel",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurationPickerProps {
    pub name: String,
    pub id: String,
    pub placeholder: String,
    pub helper_text: String,
    pub value: String,
    pub display_value: String,
    pub size: &'static str,
    pub disabled: bool,
    pub read_only: bool,
    pub required: bool,
    pub error: bool,
    pub success: bool,
    pub error_message: String,
    pub show_error: bool,
    pub full_width: bool,
    pub step_minutes: i32,
    pub max_minutes: i32,
    pub presets: Vec<i32>,
    pub class: String,
    pub display_class: String,
    pub popover_class: String,
    pub select_class: String,
    pub preset_class: String,
    pub clear_button_class: String,
    pub trigger_button_class: String,
    pub section_label_class: String,
    pub attributes: Attrs,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_invalid: bool,
}

impl Default for DurationPickerProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            placeholder: String::new(),
            helper_text: String::new(),
            value: String::new(),
            display_value: String::new(),
            size: shared::SIZE_MD,
            disabled: false,
            read_only: false,
            required: false,
            error: false,
            success: false,
            error_message: String::new(),
            show_error: true,
            full_width: false,
            step_minutes: 1,
            max_minutes: 0,
            presets: vec![15, 30, 60, 120, 240, 1440, 4320, 10080, 20160],
            class: String::new(),
            display_class: String::new(),
            popover_class: String::new(),
            select_class: String::new(),
            preset_class: String::new(),
            clear_button_class: String::new(),
            trigger_button_class: String::new(),
            section_label_class: String::new(),
            attributes: Attrs::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_invalid: false,
        }
    }
}

impl DurationPickerProps {
    #[must_use]
    pub fn with_name(mut self, value: impl Into<String>) -> Self {
        self.name = value.into();
        self
    }
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    #[must_use]
    pub fn with_display_value(mut self, value: impl Into<String>) -> Self {
        self.display_value = value.into();
        self
    }
    #[must_use]
    pub fn with_helper_text(mut self, value: impl Into<String>) -> Self {
        self.helper_text = value.into();
        self
    }
    #[must_use]
    pub fn with_error(mut self, message: impl Into<String>) -> Self {
        self.error = true;
        self.error_message = message.into();
        self
    }
    #[must_use]
    pub const fn with_required(mut self, value: bool) -> Self {
        self.required = value;
        self
    }
    #[must_use]
    pub const fn with_max_minutes(mut self, value: i32) -> Self {
        self.max_minutes = value;
        self
    }
    #[must_use]
    pub fn with_presets(mut self, values: Vec<i32>) -> Self {
        self.presets = values;
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn attr_string(&self, key: &str) -> String {
        self.attributes
            .get(key)
            .map_or_else(String::new, |value| value.trim().to_string())
    }

    #[must_use]
    pub fn control_id(&self) -> String {
        if !self.id.is_empty() {
            self.id.clone()
        } else if !self.attr_string("id").is_empty() {
            self.attr_string("id")
        } else if !self.name.is_empty() {
            sanitizeDOMID(&self.name)
        } else {
            "durationpicker".to_string()
        }
    }

    #[must_use]
    pub fn hidden_input_id(&self) -> String {
        format!("{}-value", self.control_id())
    }
    #[must_use]
    pub fn editor_input_id(&self) -> String {
        format!("{}-editor", self.control_id())
    }
    #[must_use]
    pub fn hours_select_id(&self) -> String {
        format!("{}-hours", self.control_id())
    }
    #[must_use]
    pub fn weeks_input_id(&self) -> String {
        format!("{}-weeks", self.control_id())
    }
    #[must_use]
    pub fn days_input_id(&self) -> String {
        format!("{}-days", self.control_id())
    }
    #[must_use]
    pub fn minutes_select_id(&self) -> String {
        format!("{}-minutes", self.control_id())
    }
    #[must_use]
    pub fn helper_id(&self) -> String {
        format!("{}-help", self.control_id())
    }
    #[must_use]
    pub fn popover_id(&self) -> String {
        format!("{}-popover", self.control_id())
    }
    #[must_use]
    pub fn summary_id(&self) -> String {
        format!("{}-summary", self.control_id())
    }
    #[must_use]
    pub fn error_id(&self) -> String {
        format!("{}-error", self.control_id())
    }
    #[must_use]
    pub fn text_field_name(&self) -> String {
        if self.name.is_empty() {
            String::new()
        } else {
            format!("{}_text", self.name)
        }
    }
    #[must_use]
    pub fn weeks_field_name(&self) -> String {
        if self.name.is_empty() {
            String::new()
        } else {
            format!("{}_weeks", self.name)
        }
    }
    #[must_use]
    pub fn days_field_name(&self) -> String {
        if self.name.is_empty() {
            String::new()
        } else {
            format!("{}_days", self.name)
        }
    }
    #[must_use]
    pub fn hours_field_name(&self) -> String {
        if self.name.is_empty() {
            String::new()
        } else {
            format!("{}_hours", self.name)
        }
    }
    #[must_use]
    pub fn minutes_field_name(&self) -> String {
        if self.name.is_empty() {
            String::new()
        } else {
            format!("{}_minutes", self.name)
        }
    }
    #[must_use]
    pub fn signal_base(&self) -> String {
        sanitizeSignalBase(&self.control_id())
    }
    #[must_use]
    pub fn total_signal(&self) -> String {
        format!("{}_total", self.signal_base())
    }
    #[must_use]
    pub fn display_signal(&self) -> String {
        format!("{}_display", self.signal_base())
    }
    #[must_use]
    pub fn summary_signal(&self) -> String {
        format!("{}_summary", self.signal_base())
    }
    #[must_use]
    pub fn error_signal(&self) -> String {
        format!("{}_error", self.signal_base())
    }
    #[must_use]
    pub fn open_signal(&self) -> String {
        format!("{}_open", self.signal_base())
    }
    #[must_use]
    pub fn parts_open_signal(&self) -> String {
        format!("{}_parts_open", self.signal_base())
    }
    #[must_use]
    pub fn weeks_signal(&self) -> String {
        format!("{}_weeks", self.signal_base())
    }
    #[must_use]
    pub fn days_signal(&self) -> String {
        format!("{}_days", self.signal_base())
    }
    #[must_use]
    pub fn hours_signal(&self) -> String {
        format!("{}_hours", self.signal_base())
    }
    #[must_use]
    pub fn minutes_signal(&self) -> String {
        format!("{}_minutes", self.signal_base())
    }
    #[must_use]
    pub fn no_duration_signal(&self) -> String {
        format!("{}_no_duration", self.signal_base())
    }
    #[must_use]
    pub fn done_editing_signal(&self) -> String {
        format!("{}_done_editing", self.signal_base())
    }
    #[must_use]
    pub fn edit_parts_signal(&self) -> String {
        format!("{}_edit_parts", self.signal_base())
    }
    #[must_use]
    pub fn popover_top_signal(&self) -> String {
        format!("{}_popover_top", self.signal_base())
    }
    #[must_use]
    pub fn popover_left_signal(&self) -> String {
        format!("{}_popover_left", self.signal_base())
    }

    #[must_use]
    pub fn build_wrapper_classes(&self) -> String {
        shared::build_classes(&[
            "relative space-y-1.5",
            if self.full_width { "w-full" } else { "" },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_display_classes(&self) -> String {
        shared::build_classes(&[
            "flex w-full rounded-md border border-input bg-background pr-10 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
            shared::input_size_classes(self.size),
            if self.error {
                "border-destructive focus-visible:ring-destructive"
            } else {
                ""
            },
            if self.success {
                "border-success focus-visible:ring-success"
            } else {
                ""
            },
            self.display_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_popover_classes(&self) -> String {
        shared::build_classes(&[
            "absolute z-50 mt-2 w-[32rem] rounded-2xl border border-border bg-popover p-4 text-popover-foreground shadow-lg",
            self.popover_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_select_classes(&self) -> String {
        shared::build_classes(&[
            "h-10 rounded-md border border-input bg-background px-3 text-sm",
            self.select_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_section_label_classes(&self) -> String {
        shared::build_classes(&[
            "text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground",
            self.section_label_class.as_str(),
        ])
    }

    #[must_use]
    pub fn open_expr(&self) -> String {
        format!("${} = true", self.open_signal())
    }
    #[must_use]
    pub fn close_expr(&self, commit: bool) -> String {
        if commit {
            format!(
                "${} = false; ${} = false",
                self.open_signal(),
                self.parts_open_signal()
            )
        } else {
            format!("${} = false", self.open_signal())
        }
    }
    #[must_use]
    pub fn toggle_open_expr(&self) -> String {
        format!("${} = !${}", self.open_signal(), self.open_signal())
    }
    #[must_use]
    pub fn apply_expr(&self) -> String {
        self.close_expr(true)
    }
    #[must_use]
    pub fn clear_expr(&self) -> String {
        format!(
            "${0} = ''; ${1} = ''; ${2} = 0",
            self.display_signal(),
            self.summary_signal(),
            self.total_signal()
        )
    }
    #[must_use]
    pub fn set_total_expr(&self, minutes: i32) -> String {
        format!(
            "const rawTotal = Number({minutes}); ${0} = rawTotal; ${1} = String(rawTotal);",
            self.total_signal(),
            self.display_signal()
        )
    }
    #[must_use]
    pub fn part_input_expr(&self) -> String {
        format!(
            "const weeks = Number(${0} || 0); const days = Number(${1} || 0); const hours = Number(${2} || 0); const minutes = Number(${3} || 0); ${4} = weeks * 10080 + days * 1440 + hours * 60 + minutes;",
            self.weeks_signal(),
            self.days_signal(),
            self.hours_signal(),
            self.minutes_signal(),
            self.total_signal()
        )
    }
}

#[must_use]
pub fn sanitizeDOMID(value: &str) -> String {
    super::datepicker::sanitizeDOMID(value, "durationpicker")
}

#[must_use]
pub fn sanitizeSignalBase(value: &str) -> String {
    let mut id = sanitizeDOMID(value).replace(['-', '.'], "_");
    if id
        .as_bytes()
        .first()
        .is_some_and(|byte| !byte.is_ascii_alphabetic() && *byte != b'_')
    {
        id = format!("du_{id}");
    }
    format!("du_{id}")
}

#[must_use]
pub fn parseDuration(value: &str) -> ParsedDuration {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return ParsedDuration {
            minutes: 0,
            valid: false,
        };
    }
    let lower = trimmed.to_lowercase();
    if let Ok(numeric) = lower.parse::<i32>() {
        return ParsedDuration {
            minutes: numeric,
            valid: true,
        };
    }
    if let Some(total) = parse_iso_duration(&lower) {
        return ParsedDuration {
            minutes: total,
            valid: total > 0,
        };
    }
    if let Some(total) = parse_colon_duration(&lower) {
        return ParsedDuration {
            minutes: total,
            valid: true,
        };
    }
    if let Some(total) = parse_compact_hour_minute(&lower) {
        return ParsedDuration {
            minutes: total,
            valid: true,
        };
    }
    parseTokenizedDuration(&lower)
}

fn parse_iso_duration(value: &str) -> Option<i32> {
    let rest = value.strip_prefix('p')?;
    if rest.is_empty() {
        return None;
    }
    let mut total = 0;
    let mut number = String::new();
    let mut in_time = false;
    let mut saw_unit = false;
    for ch in rest.chars() {
        if ch == 't' {
            if in_time || !number.is_empty() {
                return None;
            }
            in_time = true;
        } else if ch.is_ascii_digit() {
            number.push(ch);
        } else {
            let value = parseInt(&number);
            if number.is_empty() {
                return None;
            }
            match ch {
                'w' if !in_time => total += value * MINUTES_PER_WEEK,
                'd' if !in_time => total += value * MINUTES_PER_DAY,
                'h' if in_time => total += value * MINUTES_PER_HOUR,
                'm' if in_time => total += value,
                _ => return None,
            }
            number.clear();
            saw_unit = true;
        }
    }
    if !number.is_empty() || !saw_unit {
        None
    } else {
        Some(total)
    }
}

fn parse_colon_duration(value: &str) -> Option<i32> {
    let (hours, minutes) = value.split_once(':')?;
    if hours.is_empty()
        || hours.len() > 4
        || minutes.len() != 2
        || !hours.chars().all(|ch| ch.is_ascii_digit())
        || !minutes.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    let hours = parseInt(hours);
    let minutes = parseInt(minutes);
    (minutes < MINUTES_PER_HOUR).then_some(hours * MINUTES_PER_HOUR + minutes)
}

fn parse_compact_hour_minute(value: &str) -> Option<i32> {
    let (hours, minutes) = value.split_once('h')?;
    let hours = hours.trim();
    let minutes = minutes.trim();
    if hours.is_empty()
        || minutes.is_empty()
        || minutes.len() > 2
        || !hours.chars().all(|ch| ch.is_ascii_digit())
        || !minutes.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    let minutes = parseInt(minutes);
    (minutes < MINUTES_PER_HOUR).then_some(parseInt(hours) * MINUTES_PER_HOUR + minutes)
}

#[must_use]
pub fn parseTokenizedDuration(value: &str) -> ParsedDuration {
    let mut total = 0;
    let mut last_end = 0;
    let mut found = false;
    let char_indices: Vec<(usize, char)> = value.char_indices().collect();
    let mut pos = 0;

    while pos < char_indices.len() {
        let (start, ch) = char_indices[pos];
        if !ch.is_ascii_digit() {
            pos += 1;
            continue;
        }
        if !validDurationSeparator(&value[last_end..start]) {
            return ParsedDuration {
                minutes: 0,
                valid: false,
            };
        }

        let number_start = start;
        let mut number_end = start + ch.len_utf8();
        pos += 1;
        let mut seen_dot = false;
        while pos < char_indices.len() {
            let (idx, ch) = char_indices[pos];
            if ch.is_ascii_digit() || (ch == '.' && !seen_dot) {
                seen_dot |= ch == '.';
                number_end = idx + ch.len_utf8();
                pos += 1;
            } else {
                break;
            }
        }
        while pos < char_indices.len() && char_indices[pos].1.is_whitespace() {
            pos += 1;
        }
        if pos >= char_indices.len() || !char_indices[pos].1.is_alphabetic() {
            return ParsedDuration {
                minutes: 0,
                valid: false,
            };
        }
        let unit_start = char_indices[pos].0;
        let mut unit_end = unit_start + char_indices[pos].1.len_utf8();
        pos += 1;
        while pos < char_indices.len() && char_indices[pos].1.is_alphabetic() {
            unit_end = char_indices[pos].0 + char_indices[pos].1.len_utf8();
            pos += 1;
        }
        let number_value = &value[number_start..number_end];
        let unit_value = &value[unit_start..unit_end];
        let Some(unit_minutes) = durationUnitMinutes(unit_value) else {
            return ParsedDuration {
                minutes: 0,
                valid: false,
            };
        };
        let numeric = number_value
            .parse::<f64>()
            .expect("tokenized duration number is numeric");
        total += (numeric * unit_minutes).round() as i32;
        last_end = unit_end;
        found = true;
    }

    if !found || !validDurationSeparator(&value[last_end..]) {
        return ParsedDuration {
            minutes: 0,
            valid: false,
        };
    }
    ParsedDuration {
        minutes: total,
        valid: total > 0,
    }
}

#[must_use]
pub fn validDurationSeparator(segment: &str) -> bool {
    if segment.trim().is_empty() {
        return true;
    }
    let mut cleaned = segment.to_lowercase();
    for token in [",", "+", ";", "/", "and", "et", "y", "e"] {
        cleaned = cleaned.replace(token, "");
    }
    cleaned.trim().is_empty()
}

#[must_use]
pub fn durationUnitMinutes(unit: &str) -> Option<f64> {
    match unit.trim().trim_end_matches('.').to_lowercase().as_str() {
        "w" | "wk" | "wks" | "week" | "weeks" | "sem" | "sems" | "semaine" | "semaines"
        | "semana" | "semanas" => Some(f64::from(MINUTES_PER_WEEK)),
        "d" | "day" | "days" | "jour" | "jours" | "dia" | "dias" | "día" | "días" => {
            Some(f64::from(MINUTES_PER_DAY))
        }
        "h" | "hr" | "hrs" | "hour" | "hours" | "heure" | "heures" | "hora" | "horas" => {
            Some(f64::from(MINUTES_PER_HOUR))
        }
        "m" | "min" | "mins" | "minute" | "minutes" | "minuto" | "minutos" => Some(1.0),
        _ => None,
    }
}

#[must_use]
pub fn parseInt(value: &str) -> i32 {
    value.trim().parse::<i32>().unwrap_or(0)
}

#[must_use]
pub const fn breakdownDuration(totalMinutes: i32) -> DurationParts {
    if totalMinutes <= 0 {
        return DurationParts {
            weeks: 0,
            days: 0,
            hours: 0,
            minutes: 0,
        };
    }
    let weeks = totalMinutes / MINUTES_PER_WEEK;
    let remainder = totalMinutes % MINUTES_PER_WEEK;
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

#[must_use]
pub fn normalizeMinutes(minutes: i32, maxMinutes: i32) -> (String, i32, bool) {
    if minutes <= 0 {
        return (String::new(), 0, true);
    }
    if maxMinutes > 0 && minutes > maxMinutes {
        return (String::new(), 0, false);
    }
    (minutes.to_string(), minutes, true)
}

#[must_use]
pub fn ParseDurationMinutes(value: &str) -> Option<i32> {
    let parsed = parseDuration(value);
    parsed.valid.then_some(parsed.minutes)
}

#[must_use]
pub fn NormalizeDurationValue(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Some(String::new());
    }
    let minutes = ParseDurationMinutes(trimmed)?;
    let (canonical, _, ok) = normalizeMinutes(minutes, DEFAULT_MAX_MINUTES);
    ok.then_some(canonical)
}

#[must_use]
pub fn CanonicalMinutesFromParts(
    hoursValue: &str,
    minutesValue: &str,
    stepMinutes: i32,
    maxMinutes: i32,
) -> Option<(String, i32)> {
    CanonicalMinutesFromStructuredParts("", "", hoursValue, minutesValue, stepMinutes, maxMinutes)
}

#[must_use]
pub fn CanonicalMinutesFromStructuredParts(
    weeksValue: &str,
    daysValue: &str,
    hoursValue: &str,
    minutesValue: &str,
    stepMinutes: i32,
    maxMinutes: i32,
) -> Option<(String, i32)> {
    let weeks_trimmed = weeksValue.trim();
    let days_trimmed = daysValue.trim();
    let hours_trimmed = hoursValue.trim();
    let minutes_trimmed = minutesValue.trim();
    if weeks_trimmed.is_empty()
        && days_trimmed.is_empty()
        && hours_trimmed.is_empty()
        && minutes_trimmed.is_empty()
    {
        return Some((String::new(), 0));
    }
    let weeks = defaultIfBlank(weeks_trimmed, "0").parse::<i32>().ok()?;
    let days = defaultIfBlank(days_trimmed, "0").parse::<i32>().ok()?;
    let hours = defaultIfBlank(hours_trimmed, "0").parse::<i32>().ok()?;
    let minutes = defaultIfBlank(minutes_trimmed, "0").parse::<i32>().ok()?;
    if weeks < 0 || days < 0 || hours < 0 || !(0..MINUTES_PER_HOUR).contains(&minutes) {
        return None;
    }
    let step = if stepMinutes <= 0 {
        DEFAULT_STEP_MINUTES
    } else {
        stepMinutes
    };
    if minutes % step != 0 {
        return None;
    }
    let total =
        weeks * MINUTES_PER_WEEK + days * MINUTES_PER_DAY + hours * MINUTES_PER_HOUR + minutes;
    let (canonical, normalized, ok) = normalizeMinutes(total, maxMinutes);
    ok.then_some((canonical, normalized))
}

#[must_use]
pub fn defaultIfBlank(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

#[must_use]
pub fn localizedDurationParts(minutes: i32) -> Vec<String> {
    if minutes <= 0 {
        return Vec::new();
    }
    let parts = breakdownDuration(minutes);
    let mut output = Vec::new();
    let mut append = |value: i32, singular: &str, plural: &str| {
        if value > 0 {
            output.push(format!(
                "{value} {}",
                if value == 1 { singular } else { plural }
            ));
        }
    };
    append(parts.weeks, "week", "weeks");
    append(parts.days, "day", "days");
    append(parts.hours, "hour", "hours");
    append(parts.minutes, "minute", "minutes");
    output
}

#[must_use]
pub const fn localizedDurationJoiner() -> &'static str {
    " "
}

#[must_use]
pub fn FormatDuration(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let Some(minutes) = ParseDurationMinutes(trimmed) else {
        return trimmed.to_string();
    };
    if minutes <= 0 {
        return String::new();
    }
    localizedDurationParts(minutes).join(localizedDurationJoiner())
}

#[must_use]
pub fn FormatCompactDuration(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let Some(minutes) = ParseDurationMinutes(trimmed) else {
        return trimmed.to_string();
    };
    if minutes <= 0 {
        return String::new();
    }
    let parts = breakdownDuration(minutes);
    let mut output = Vec::new();
    if parts.weeks > 0 {
        output.push(format!("{}w", parts.weeks));
    }
    if parts.days > 0 {
        output.push(format!("{}d", parts.days));
    }
    if parts.hours > 0 {
        output.push(format!("{}h", parts.hours));
    }
    if parts.minutes > 0 {
        output.push(format!("{}m", parts.minutes));
    }
    output.join(" ")
}

#[must_use]
pub fn resolvePlaceholder(props: &DurationPickerProps) -> String {
    if props.placeholder.trim().is_empty() {
        "Enter duration".to_string()
    } else {
        props.placeholder.clone()
    }
}

#[must_use]
pub fn resolveHelperText(props: &DurationPickerProps) -> String {
    if props.helper_text.trim().is_empty() {
        "Examples: 90m, 1h 30m, 2 days".to_string()
    } else {
        props.helper_text.clone()
    }
}

#[must_use]
pub const fn resolveInvalidText() -> &'static str {
    "Enter a valid duration"
}

#[must_use]
pub fn initialDurationMinutes(props: &DurationPickerProps) -> i32 {
    let Some(minutes) = ParseDurationMinutes(&props.value) else {
        return 0;
    };
    let (_, normalized, ok) = normalizeMinutes(minutes, props.max_minutes);
    if ok { normalized } else { 0 }
}

#[must_use]
pub fn initialHiddenValue(props: &DurationPickerProps) -> String {
    NormalizeDurationValue(&props.value).unwrap_or_else(|| props.value.trim().to_string())
}

#[must_use]
pub fn initialDisplayValue(props: &DurationPickerProps) -> String {
    if !props.display_value.trim().is_empty() {
        props.display_value.trim().to_string()
    } else {
        let formatted = FormatDuration(&props.value);
        if formatted.trim().is_empty() {
            props.value.trim().to_string()
        } else {
            formatted
        }
    }
}

#[must_use]
pub fn jsStringLiteral(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

#[must_use]
pub fn jsDurationParserExpr(valueExpr: &str) -> String {
    format!(
        "(() => {{ const rawValue = String({valueExpr} ?? '').trim().toLowerCase(); if (rawValue === '') {{ return {{ ok: true, total: 0 }}; }} /* duration parser */ }})()"
    )
}

#[must_use]
pub fn durationSummaryExpr(props: &DurationPickerProps, totalExpr: &str) -> String {
    format!(
        "(() => {{ const resolvedTotal = Number({totalExpr}) || 0; return resolvedTotal <= 0 ? '' : String(resolvedTotal) + ' minutes'; /* {} */ }})()",
        props.summary_signal()
    )
}

#[must_use]
pub const fn max(a: i32, b: i32) -> i32 {
    if a > b { a } else { b }
}

#[must_use]
pub fn buildSignals(props: &DurationPickerProps) -> String {
    let base = props.signal_base();
    let initial_minutes = initialDurationMinutes(props);
    let initial_breakdown = breakdownDuration(initial_minutes);
    let initial_display = initialDisplayValue(props);
    let mut initial_summary = FormatDuration(&props.value);
    if initial_summary.trim().is_empty() {
        initial_summary = props.value.trim().to_string();
    }
    let mut state = serde_json::Map::new();
    state.insert(props.total_signal(), serde_json::json!(initial_minutes));
    state.insert(props.open_signal(), serde_json::json!(false));
    state.insert(props.parts_open_signal(), serde_json::json!(false));
    state.insert(
        props.weeks_signal(),
        serde_json::json!(initial_breakdown.weeks),
    );
    state.insert(
        props.days_signal(),
        serde_json::json!(initial_breakdown.days),
    );
    state.insert(
        props.hours_signal(),
        serde_json::json!(initial_breakdown.hours),
    );
    state.insert(
        props.minutes_signal(),
        serde_json::json!(initial_breakdown.minutes),
    );
    state.insert(props.display_signal(), serde_json::json!(initial_display));
    state.insert(props.summary_signal(), serde_json::json!(initial_summary));
    state.insert(
        props.error_signal(),
        serde_json::json!(props.error_message.trim()),
    );
    state.insert(
        format!("{base}_placeholder"),
        serde_json::json!(resolvePlaceholder(props)),
    );
    state.insert(format!("{base}_week_one"), serde_json::json!("week"));
    state.insert(format!("{base}_week_other"), serde_json::json!("weeks"));
    state.insert(format!("{base}_day_one"), serde_json::json!("day"));
    state.insert(format!("{base}_day_other"), serde_json::json!("days"));
    state.insert(format!("{base}_hour_one"), serde_json::json!("hour"));
    state.insert(format!("{base}_hour_other"), serde_json::json!("hours"));
    state.insert(format!("{base}_minute_one"), serde_json::json!("minute"));
    state.insert(format!("{base}_minute_other"), serde_json::json!("minutes"));
    state.insert(
        format!("{base}_joiner"),
        serde_json::json!(localizedDurationJoiner()),
    );
    state.insert(
        format!("{base}_step"),
        serde_json::json!(props.step_minutes),
    );
    state.insert(format!("{base}_max"), serde_json::json!(props.max_minutes));
    state.insert(props.no_duration_signal(), serde_json::json!("No duration"));
    state.insert(props.edit_parts_signal(), serde_json::json!("Edit parts"));
    state.insert(props.done_editing_signal(), serde_json::json!("Done"));
    state.insert(props.popover_top_signal(), serde_json::json!(0));
    state.insert(props.popover_left_signal(), serde_json::json!(0));
    serde_json::to_string(&state).unwrap_or_default()
}

#[must_use]
pub fn popoverStyleExpr(props: &DurationPickerProps) -> String {
    format!(
        "'top:' + ${} + 'px;left:' + ${} + 'px;display:' + (${} ? 'block' : 'none')",
        props.popover_top_signal(),
        props.popover_left_signal(),
        props.open_signal()
    )
}

fn optional(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

#[must_use]
pub fn DurationPicker(props: &DurationPickerProps) -> Markup {
    let helper_id = props.helper_id();
    let described_by = if !props.aria_described_by.is_empty() {
        props.aria_described_by.clone()
    } else if !props.helper_text.is_empty() {
        helper_id
    } else if props.error {
        props.error_id()
    } else {
        String::new()
    };
    html! {
        div
            class=(props.build_wrapper_classes())
            data-signals=(buildSignals(props))
            data-on:click__outside=(props.close_expr(false))
            data-on:resize__window=(format!("if (${}) {{ /* reposition */ }}", props.open_signal()))
            data-on:scroll__window=(format!("if (${}) {{ /* reposition */ }}", props.open_signal()))
        {
            input type="hidden" id=(props.hidden_input_id()) name=[optional(&props.name)] value=[optional(&initialHiddenValue(props))] disabled[props.disabled] data-on:change=(format!("${} = evt.target.value", props.total_signal()));
            div class="relative" {
                input
                    type="text"
                    readonly
                    id=(props.control_id())
                    value=[optional(&initialDisplayValue(props))]
                    placeholder=[optional(&props.placeholder)]
                    class=(props.build_display_classes())
                    required[props.required]
                    disabled[props.disabled || props.read_only]
                    aria-label=[optional(&props.aria_label)]
                    aria-describedby=[optional(&described_by)]
                    aria-invalid=[if props.error || props.aria_invalid { Some("true") } else { None }]
                    data-bind=(props.display_signal())
                    data-attr:aria-expanded=(format!("String(${})", props.open_signal()))
                    data-attr:aria-invalid=(format!("${} !== '' ? 'true' : null", props.error_signal()))
                    data-on:focus=(props.open_expr())
                    data-on:click=(props.open_expr())
                    data-on:keydown=(format!("if (evt.key === 'Enter' || evt.key === ' ' || evt.key === 'ArrowDown') {{ evt.preventDefault(); {} }}", props.open_expr()));
                button type="button" class=(shared::build_classes(&["absolute inset-y-0 right-0 flex items-center pr-3", props.trigger_button_class.as_str()])) aria-label="Open duration picker" disabled[props.disabled || props.read_only] data-attr:aria-expanded=(format!("String(${})", props.open_signal())) data-on:mousedown="evt.preventDefault();" data-on:click=(format!("evt.preventDefault(); evt.stopPropagation(); {}", props.toggle_open_expr())) {
                    svg class="h-4 w-4" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8" {
                        circle cx="12" cy="12" r="9" {}
                        path stroke-linecap="round" stroke-linejoin="round" d="M12 7v5l3 2" {}
                    }
                }
            }
            @if !props.helper_text.is_empty() {
                p id=(props.helper_id()) class="text-xs leading-relaxed text-muted-foreground" { (&props.helper_text) }
            }
            div id=(props.popover_id()) role="dialog" aria-label="Duration picker" class=(props.build_popover_classes()) style="display:none" data-attr:style=(popoverStyleExpr(props)) data-on:keydown=(format!("if (evt.key === 'Escape') {{ evt.preventDefault(); {} }}", props.close_expr(false))) {
                div class="flex flex-col gap-4" {
                    div class="space-y-2" {
                        div class=(props.build_section_label_classes()) { "Duration" }
                        input type="text" id=(props.editor_input_id()) name=[optional(&props.text_field_name())] value=[optional(&initialDisplayValue(props))] class=(props.build_display_classes()) disabled[props.disabled || props.read_only] data-bind=(props.display_signal()) data-attr:aria-invalid=(format!("${} !== '' ? 'true' : null", props.error_signal())) data-on:input=(format!("const rawInput = String(evt.target.value ?? ''); ${} = rawInput;", props.display_signal())) data-on:blur=(format!("if (${} === '') {{ ${} = '' }}", props.display_signal(), props.summary_signal()));
                        p class="text-xs text-destructive" role="alert" data-text=(format!("${}", props.error_signal())) data-class:hidden=(format!("${} === '' || !${}", props.error_signal(), props.open_signal())) {}
                    }
                    div class="flex flex-wrap items-start justify-between gap-3" {
                        div class="space-y-3" {
                            div class="inline-flex items-center rounded-full border border-primary/20 bg-primary/5 px-3 py-1 text-xs font-semibold uppercase tracking-[0.12em] text-primary" { "Stored as minutes" }
                            div class="space-y-2" {
                                div class=(props.build_section_label_classes()) { "Summary" }
                                p id=(props.summary_id()) class="text-lg font-semibold leading-tight text-foreground sm:text-xl" aria-live="polite" data-text=(format!("${} === '' ? ${} : ${}", props.summary_signal(), props.no_duration_signal(), props.summary_signal())) {}
                            }
                        }
                        div class="min-w-[9rem] rounded-xl border border-border/50 bg-card/80 p-3" {
                            div class=(props.build_section_label_classes()) { "Total minutes" }
                            p class="mt-1 text-2xl font-semibold leading-none text-foreground" data-text=(format!("${}", props.total_signal())) {}
                        }
                    }
                    div class="space-y-3 rounded-2xl border border-border/50 bg-background/70 p-4" {
                        div class=(props.build_section_label_classes()) { "Parts" }
                        div class="flex flex-wrap gap-2" {
                            @for (signal, label) in [(props.weeks_signal(), "weeks"), (props.days_signal(), "days"), (props.hours_signal(), "hours"), (props.minutes_signal(), "minutes")] {
                                div class="inline-flex items-center gap-2 rounded-full border border-border/60 bg-card/70 px-3 py-1.5 text-sm text-foreground" {
                                    span class="font-semibold" data-text=(format!("${signal}")) {}
                                    span { (label) }
                                }
                            }
                        }
                        div class="flex flex-wrap items-start justify-between gap-3" {
                            p class="max-w-xl text-xs leading-relaxed text-muted-foreground" { "Edit the individual parts or type a compact duration." }
                            button type="button" class="inline-flex min-h-9 items-center justify-center rounded-md border border-border bg-background px-3 text-xs font-medium text-foreground" disabled[props.disabled || props.read_only] data-on:click=(format!("${0} = !${0}", props.parts_open_signal())) data-attr:aria-expanded=(format!("${}", props.parts_open_signal())) data-text=(format!("${} ? ${} : ${}", props.parts_open_signal(), props.done_editing_signal(), props.edit_parts_signal())) {}
                        }
                        div class="grid grid-cols-2 gap-3 border-t border-border/50 pt-4 xl:grid-cols-4" style="display:none" data-show=(format!("${}", props.parts_open_signal())) {
                            div class="space-y-1.5" { label for=(props.weeks_input_id()) class="text-sm font-medium leading-none text-foreground" { "Weeks" } input type="number" id=(props.weeks_input_id()) name=[optional(&props.weeks_field_name())] class=(props.build_select_classes()) disabled[props.disabled || props.read_only] data-bind=(props.weeks_signal()) data-on:input=(props.part_input_expr()); }
                            div class="space-y-1.5" { label for=(props.days_input_id()) class="text-sm font-medium leading-none text-foreground" { "Days" } input type="number" id=(props.days_input_id()) name=[optional(&props.days_field_name())] class=(props.build_select_classes()) disabled[props.disabled || props.read_only] data-bind=(props.days_signal()) data-on:input=(props.part_input_expr()); }
                            div class="space-y-1.5" { label for=(props.hours_select_id()) class="text-sm font-medium leading-none text-foreground" { "Hours" } input type="number" id=(props.hours_select_id()) name=[optional(&props.hours_field_name())] class=(props.build_select_classes()) disabled[props.disabled || props.read_only] data-bind=(props.hours_signal()) data-on:input=(props.part_input_expr()); }
                            div class="space-y-1.5" { label for=(props.minutes_select_id()) class="text-sm font-medium leading-none text-foreground" { "Minutes" } input type="number" id=(props.minutes_select_id()) name=[optional(&props.minutes_field_name())] class=(props.build_select_classes()) disabled[props.disabled || props.read_only] data-bind=(props.minutes_signal()) data-on:input=(props.part_input_expr()); }
                        }
                    }
                    div class="border-t border-border/50 pt-4" {
                        div class=(props.build_section_label_classes()) { "Presets" }
                        div class="mt-3 flex gap-2 overflow-x-auto pb-1 sm:flex-wrap" {
                            @for preset in &props.presets {
                                button type="button" class=(shared::build_classes(&["inline-flex min-h-9 items-center justify-center rounded-full border border-border bg-background px-3 text-sm", props.preset_class.as_str()])) disabled[props.disabled || props.read_only] data-on:click=(props.set_total_expr(*preset)) {
                                    (FormatCompactDuration(&preset.to_string()))
                                }
                            }
                        }
                    }
                    div class="flex items-center justify-between gap-3 border-t border-border/50 pt-4" {
                        @if !props.required {
                            button type="button" class=(shared::build_classes(&["inline-flex min-h-9 items-center rounded-md border border-border px-4 text-sm", props.clear_button_class.as_str()])) disabled[props.disabled || props.read_only] data-on:click=(props.clear_expr()) { "Clear" }
                        } @else { div {} }
                        button type="button" class="inline-flex min-h-9 items-center justify-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground" disabled[props.disabled || props.read_only] data-on:click=(props.apply_expr()) { "Apply" }
                    }
                }
            }
            @if props.error && props.show_error && !props.error_message.is_empty() {
                p id=(props.error_id()) class="mt-1.5 text-xs text-destructive" role="alert" data-text=(format!("${}", props.error_signal())) { (&props.error_message) }
            }
        }
    }
}

#[must_use]
pub fn DurationPickerWithLabel(
    props: &DurationPickerProps,
    label: &str,
    helperText: &str,
) -> Markup {
    html! {
        div class="space-y-1.5" {
            @if !label.is_empty() {
                label for=(props.control_id()) class=(shared::build_classes(&["text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70", if props.error { "text-destructive" } else { "" }])) {
                    (label)
                    @if props.required { span class="ml-1 text-destructive" aria-label="required" { "*" } }
                }
            }
            (DurationPicker(props))
            @if !helperText.is_empty() && !props.error {
                p class="text-xs text-muted-foreground" { (helperText) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_contains(html: &str, expected: &str) {
        assert!(html.contains(expected), "missing `{expected}` in `{html}`");
    }

    #[test]
    fn helpers_build_duration_ids_signals_and_expressions() {
        let props = DurationPickerProps::default()
            .with_name("event duration")
            .with_value("90");
        assert_eq!(props.control_id(), "event-duration");
        assert_eq!(props.hidden_input_id(), "event-duration-value");
        assert_eq!(props.editor_input_id(), "event-duration-editor");
        assert_eq!(props.text_field_name(), "event duration_text");
        assert_eq!(props.signal_base(), "du_event_duration");
        assert_eq!(props.total_signal(), "du_event_duration_total");
        assert_eq!(initialHiddenValue(&props), "90");
        assert_eq!(initialDisplayValue(&props), "1 hour 30 minutes");
        assert_eq!(
            initialDisplayValue(&props.clone().with_display_value("1h 30m")),
            "1h 30m"
        );
        assert_eq!(sanitizeDOMID(" event duration! "), "event-duration");
        assert_eq!(sanitizeDOMID("!!!"), "durationpicker");
        assert_eq!(sanitizeSignalBase("123.duration"), "du_du_123_duration");
        assert_eq!(parseDuration("").valid, false);
        assert_eq!(
            parseDuration("90"),
            ParsedDuration {
                minutes: 90,
                valid: true
            }
        );
        assert_eq!(
            parseDuration("P1W2DT3H4M"),
            ParsedDuration {
                minutes: 13_144,
                valid: true
            }
        );
        assert_eq!(parseDuration("P").valid, false);
        assert_eq!(parseDuration("PT1T").valid, false);
        assert_eq!(parseDuration("PTM").valid, false);
        assert_eq!(parseDuration("P1H").valid, false);
        assert_eq!(parseDuration("PT1").valid, false);
        assert_eq!(parseDuration("PT0M").valid, false);
        assert_eq!(parseDuration("1:30").minutes, 90);
        assert_eq!(parseDuration(":30").valid, false);
        assert_eq!(parseDuration("1:70").valid, false);
        assert_eq!(parseDuration("1h30").minutes, 90);
        assert_eq!(parseDuration("1h70").valid, false);
        assert_eq!(parseDuration("1.5 hours and 30 minutes").minutes, 120);
        assert_eq!(parseTokenizedDuration("1h nope 2m").valid, false);
        assert_eq!(parseTokenizedDuration("1 mystery").valid, false);
        assert!(validDurationSeparator(" and / + "));
        assert!(!validDurationSeparator(" nope "));
        assert_eq!(durationUnitMinutes("weeks"), Some(10080.0));
        assert_eq!(durationUnitMinutes("días"), Some(1440.0));
        assert_eq!(durationUnitMinutes("hrs"), Some(60.0));
        assert_eq!(durationUnitMinutes("mins"), Some(1.0));
        assert_eq!(durationUnitMinutes("bogus"), None);
        assert_eq!(parseInt(" 42 "), 42);
        assert_eq!(parseInt("bad"), 0);
        assert_eq!(
            breakdownDuration(10_150),
            DurationParts {
                weeks: 1,
                days: 0,
                hours: 1,
                minutes: 10
            }
        );
        assert_eq!(
            breakdownDuration(0),
            DurationParts {
                weeks: 0,
                days: 0,
                hours: 0,
                minutes: 0
            }
        );
        assert_eq!(normalizeMinutes(0, 0), (String::new(), 0, true));
        assert_eq!(normalizeMinutes(90, 120), ("90".to_string(), 90, true));
        assert_eq!(normalizeMinutes(130, 120), (String::new(), 0, false));
        assert_eq!(ParseDurationMinutes("2h"), Some(120));
        assert_eq!(ParseDurationMinutes("bad"), None);
        assert_eq!(NormalizeDurationValue(""), Some(String::new()));
        assert_eq!(NormalizeDurationValue("1h 30m"), Some("90".to_string()));
        assert_eq!(NormalizeDurationValue("bad"), None);
        assert_eq!(
            CanonicalMinutesFromParts("1", "30", 5, 0),
            Some(("90".to_string(), 90))
        );
        assert_eq!(
            CanonicalMinutesFromStructuredParts("", "", "", "", 1, 0),
            Some((String::new(), 0))
        );
        assert_eq!(
            CanonicalMinutesFromStructuredParts("1", "2", "3", "30", 5, 0),
            Some(("13170".to_string(), 13170))
        );
        assert_eq!(
            CanonicalMinutesFromStructuredParts("bad", "", "", "", 1, 0),
            None
        );
        assert_eq!(
            CanonicalMinutesFromStructuredParts("", "bad", "", "", 1, 0),
            None
        );
        assert_eq!(
            CanonicalMinutesFromStructuredParts("", "", "bad", "", 1, 0),
            None
        );
        assert_eq!(
            CanonicalMinutesFromStructuredParts("", "", "", "bad", 1, 0),
            None
        );
        assert_eq!(
            CanonicalMinutesFromStructuredParts("", "-1", "", "", 1, 0),
            None
        );
        assert_eq!(
            CanonicalMinutesFromStructuredParts("", "", "-1", "", 1, 0),
            None
        );
        assert_eq!(
            CanonicalMinutesFromStructuredParts("", "", "", "60", 1, 0),
            None
        );
        assert_eq!(
            CanonicalMinutesFromStructuredParts("", "", "", "7", 5, 0),
            None
        );
        assert_eq!(
            CanonicalMinutesFromStructuredParts("", "", "3", "30", 0, 120),
            None
        );
        assert_eq!(defaultIfBlank("", "0"), "0");
        assert_eq!(defaultIfBlank("7", "0"), "7");
        assert_eq!(
            localizedDurationParts(90),
            vec!["1 hour".to_string(), "30 minutes".to_string()]
        );
        assert!(localizedDurationParts(0).is_empty());
        assert_eq!(localizedDurationJoiner(), " ");
        assert_eq!(FormatDuration(""), "");
        assert_eq!(FormatDuration("bad"), "bad");
        assert_eq!(FormatDuration("0"), "");
        assert_eq!(FormatDuration("90"), "1 hour 30 minutes");
        assert_eq!(FormatCompactDuration(""), "");
        assert_eq!(FormatCompactDuration("bad"), "bad");
        assert_eq!(FormatCompactDuration("0"), "");
        assert_eq!(FormatCompactDuration("10080"), "1w");
        assert_eq!(FormatCompactDuration("1440"), "1d");
        assert_eq!(FormatCompactDuration("60"), "1h");
        assert_eq!(FormatCompactDuration("45"), "45m");
        assert_eq!(
            resolvePlaceholder(&DurationPickerProps::default()),
            "Enter duration"
        );
        assert_eq!(
            resolvePlaceholder(&DurationPickerProps {
                placeholder: "How long?".to_string(),
                ..DurationPickerProps::default()
            }),
            "How long?"
        );
        assert_eq!(
            resolveHelperText(&DurationPickerProps::default()),
            "Examples: 90m, 1h 30m, 2 days"
        );
        assert_eq!(
            resolveHelperText(&DurationPickerProps::default().with_helper_text("Help")),
            "Help"
        );
        assert_eq!(resolveInvalidText(), "Enter a valid duration");
        assert_eq!(initialDurationMinutes(&props), 90);
        assert_eq!(
            initialDurationMinutes(
                &DurationPickerProps::default()
                    .with_value("130")
                    .with_max_minutes(120)
            ),
            0
        );
        assert_eq!(
            initialHiddenValue(&DurationPickerProps::default().with_value("bad")),
            "bad"
        );
        assert_eq!(
            initialDisplayValue(&DurationPickerProps::default().with_value("bad")),
            "bad"
        );
        assert_eq!(jsStringLiteral("a'b"), "\"a'b\"");
        assert!(jsDurationParserExpr("rawInput").contains("rawValue"));
        assert!(durationSummaryExpr(&props, "total").contains("du_event_duration_summary"));
        assert_eq!(max(3, 5), 5);
        assert_eq!(max(7, 5), 7);
        assert!(buildSignals(&props).contains("\"du_event_duration_total\":90"));
        assert!(buildSignals(&props).contains("\"du_event_duration_hours\":1"));
        assert!(buildSignals(&props).contains("\"du_event_duration_minutes\":30"));
        assert!(popoverStyleExpr(&props).contains("$du_event_duration_popover_top"));
        assert!(props.part_input_expr().contains("weeks * 10080"));
        assert!(
            props
                .set_total_expr(15)
                .contains("const rawTotal = Number(15);")
        );
        assert!(
            props
                .clear_expr()
                .contains("$du_event_duration_display = ''")
        );

        let attr_id = DurationPickerProps::default().with_attribute("id", "from-attrs");
        assert_eq!(attr_id.control_id(), "from-attrs");
        assert_eq!(
            DurationPickerProps::default().control_id(),
            "durationpicker"
        );

        let mut success = DurationPickerProps::default()
            .with_name("success duration")
            .with_max_minutes(120)
            .with_presets(vec![5, 10])
            .with_class("wrapper-extra")
            .with_attribute("data-testid", "duration");
        success.success = true;
        assert_eq!(success.max_minutes, 120);
        assert_eq!(success.presets, vec![5, 10]);
        assert!(success.build_wrapper_classes().contains("wrapper-extra"));
        assert!(success.build_display_classes().contains("border-success"));
        assert_eq!(
            success.attributes.get("data-testid").map(String::as_str),
            Some("duration")
        );
    }

    #[test]
    fn durationpicker_renders_datastar_bindings_parts_and_presets() {
        let props = DurationPickerProps::default()
            .with_name("duration")
            .with_value("60")
            .with_helper_text("Stored as minutes")
            .with_presets(vec![15, 60]);
        let output = DurationPicker(&props).into_string();
        assert_contains(&output, r#"&quot;du_duration_open&quot;:false"#);
        assert_contains(&output, r#"&quot;du_duration_total&quot;:60"#);
        assert_contains(
            &output,
            r#"&quot;du_duration_summary&quot;:&quot;1 hour&quot;"#,
        );
        assert_contains(
            &output,
            r#"data-on:click__outside="$du_duration_open = false""#,
        );
        assert_contains(&output, r#"data-bind="du_duration_display""#);
        assert_contains(
            &output,
            r#"data-attr:aria-invalid="$du_duration_error !== '' ? 'true' : null""#,
        );
        assert_contains(&output, r#"data-text="$du_duration_error""#);
        assert_contains(&output, r#"data-text="$du_duration_total""#);
        assert_contains(&output, r#"data-bind="du_duration_weeks""#);
        assert_contains(&output, r#"data-bind="du_duration_days""#);
        assert_contains(&output, r#"data-bind="du_duration_hours""#);
        assert_contains(&output, r#"data-bind="du_duration_minutes""#);
        assert_contains(&output, r#"data-show="$du_duration_parts_open""#);
        assert_contains(&output, r#"data-on:click="const rawTotal = Number(15);"#);
        assert_contains(&output, "Stored as minutes");
    }

    #[test]
    fn durationpicker_with_label_required_and_error_states_render() {
        let props = DurationPickerProps::default()
            .with_id("length")
            .with_required(true);
        let output = DurationPickerWithLabel(&props, "Length", "Use minutes").into_string();
        assert_contains(&output, r#"<label for="length""#);
        assert_contains(&output, r#"aria-label="required""#);
        assert_contains(&output, "Use minutes");
        assert!(!output.contains(">Clear</button>"));

        let error = DurationPickerWithLabel(&props.with_error("Too long"), "Length", "Use minutes")
            .into_string();
        assert_contains(&error, "text-destructive");
        assert_contains(&error, "Too long");
        assert!(!error.contains("Use minutes"));

        let error_only =
            DurationPicker(&DurationPickerProps::default().with_error("Bad")).into_string();
        assert_contains(&error_only, r#"aria-describedby="durationpicker-error""#);
        let mut described = DurationPickerProps::default();
        described.aria_described_by = "external-help".to_string();
        let described_html = DurationPicker(&described).into_string();
        assert_contains(&described_html, r#"aria-describedby="external-help""#);
    }
}
