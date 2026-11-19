#![allow(non_snake_case)]

use super::datepicker::{CalendarMonthOption, DatePickerSelectOption};
use crate::components::{ComponentInfo, shared};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike, Utc};
use maud::{Markup, html};
use serde::Serialize;
use std::collections::BTreeMap;

type Attrs = BTreeMap<String, String>;
const ISO_DATE_TIME_FORMAT: &str = "%Y-%m-%dT%H:%M";
const MINUTE_STEP: u32 = 5;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DateTimeCalendarCell {
    pub label: String,
    pub value: String,
    #[serde(rename = "ariaLabel")]
    pub aria_label: String,
    pub current: bool,
    pub today: bool,
    pub selected: bool,
    pub disabled: bool,
}

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "datetimepicker",
        name: "DateTimePicker",
    },
    ComponentInfo {
        family: "unstyled",
        module: "datetimepicker",
        name: "DateTimePickerWithLabel",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DateTimePickerProps {
    pub name: String,
    pub id: String,
    pub placeholder: String,
    pub value: String,
    pub time_zone_name: String,
    pub time_zone_value: String,
    pub time_zone_placeholder: String,
    pub size: &'static str,
    pub disabled: bool,
    pub read_only: bool,
    pub required: bool,
    pub error: bool,
    pub success: bool,
    pub error_message: String,
    pub show_error: bool,
    pub full_width: bool,
    pub class: String,
    pub display_class: String,
    pub popover_class: String,
    pub select_class: String,
    pub time_zone_class: String,
    pub icon_class: String,
    pub attributes: Attrs,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_invalid: bool,
    pub min: String,
    pub max: String,
    pub week_starts_on: usize,
    pub hide_today: bool,
    pub hide_clear: bool,
    pub use_24_hour: bool,
    pub locale_aware_use_24_hour: bool,
    pub locale_aware_week_starts_on: bool,
}

impl Default for DateTimePickerProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            placeholder: String::new(),
            value: String::new(),
            time_zone_name: String::new(),
            time_zone_value: String::new(),
            time_zone_placeholder: String::new(),
            size: shared::SIZE_MD,
            disabled: false,
            read_only: false,
            required: false,
            error: false,
            success: false,
            error_message: String::new(),
            show_error: true,
            full_width: false,
            class: String::new(),
            display_class: String::new(),
            popover_class: String::new(),
            select_class: String::new(),
            time_zone_class: String::new(),
            icon_class: String::new(),
            attributes: Attrs::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_invalid: false,
            min: String::new(),
            max: String::new(),
            week_starts_on: 0,
            hide_today: false,
            hide_clear: false,
            use_24_hour: true,
            locale_aware_use_24_hour: false,
            locale_aware_week_starts_on: false,
        }
    }
}

impl DateTimePickerProps {
    /// Resolve request-locale preferences at the component boundary.
    pub fn with_locale_preferences(mut self,use_24_hour:bool,week_starts_on:usize)->Self{
        if self.locale_aware_use_24_hour{self.use_24_hour=use_24_hour;}
        if self.locale_aware_week_starts_on{self.week_starts_on=week_starts_on;}
        self.locale_aware_use_24_hour=false;self.locale_aware_week_starts_on=false;self
    }
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
    pub fn with_time_zone(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.time_zone_name = name.into();
        self.time_zone_value = value.into();
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
    pub const fn with_use_24_hour(mut self, value: bool) -> Self {
        self.use_24_hour = value;
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
            "datetimepicker".to_string()
        }
    }

    #[must_use]
    pub fn hidden_input_id(&self) -> String {
        format!("{}-value", self.control_id())
    }
    #[must_use]
    pub fn time_zone_input_id(&self) -> String {
        format!("{}-timezone", self.control_id())
    }
    #[must_use]
    pub fn time_zone_hidden_input_id(&self) -> String {
        format!("{}-timezone-value", self.control_id())
    }
    #[must_use]
    pub fn time_zone_listbox_id(&self) -> String {
        format!("{}-timezone-listbox", self.control_id())
    }
    #[must_use]
    pub fn popover_id(&self) -> String {
        format!("{}-calendar", self.control_id())
    }
    #[must_use]
    pub fn month_select_id(&self) -> String {
        format!("{}-month", self.control_id())
    }
    #[must_use]
    pub fn year_select_id(&self) -> String {
        format!("{}-year", self.control_id())
    }
    #[must_use]
    pub fn error_id(&self) -> String {
        format!("{}-error", self.control_id())
    }
    #[must_use]
    pub fn signal_base(&self) -> String {
        sanitizeSignalBase(&self.control_id())
    }
    #[must_use]
    pub fn open_signal(&self) -> String {
        format!("{}_open", self.signal_base())
    }
    #[must_use]
    pub fn value_signal(&self) -> String {
        format!("{}_value", self.signal_base())
    }
    #[must_use]
    pub fn display_signal(&self) -> String {
        format!("{}_display", self.signal_base())
    }
    #[must_use]
    pub fn time_zone_signal(&self) -> String {
        format!("{}_timezone", self.signal_base())
    }
    #[must_use]
    pub fn time_zone_query_signal(&self) -> String {
        format!("{}_timezone_query", self.signal_base())
    }
    #[must_use]
    pub fn time_zone_open_signal(&self) -> String {
        format!("{}_timezone_open", self.signal_base())
    }
    #[must_use]
    pub fn time_zone_results_signal(&self) -> String {
        format!("{}_timezone_results", self.signal_base())
    }
    #[must_use]
    pub fn time_zone_active_index_signal(&self) -> String {
        format!("{}_timezone_active_index", self.signal_base())
    }
    #[must_use]
    pub fn time_zone_offset_signal(&self) -> String {
        format!("{}_timezone_offset", self.signal_base())
    }
    #[must_use]
    pub fn year_signal(&self) -> String {
        format!("{}_year", self.signal_base())
    }
    #[must_use]
    pub fn month_signal(&self) -> String {
        format!("{}_month", self.signal_base())
    }
    #[must_use]
    pub fn cells_signal(&self) -> String {
        format!("{}_cells", self.signal_base())
    }
    #[must_use]
    pub fn hour_signal(&self) -> String {
        format!("{}_hour", self.signal_base())
    }
    #[must_use]
    pub fn minute_signal(&self) -> String {
        format!("{}_minute", self.signal_base())
    }
    #[must_use]
    pub fn ampm_signal(&self) -> String {
        format!("{}_ampm", self.signal_base())
    }
    #[must_use]
    pub fn today_signal(&self) -> String {
        format!("{}_today", self.signal_base())
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
            "relative",
            if self.full_width { "w-full" } else { "" },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_display_classes(&self) -> String {
        shared::build_classes(&[
            "flex w-full rounded-md border border-input bg-background pr-10 text-sm shadow-sm transition-[border-color,box-shadow] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
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
            "absolute z-50 mt-2 w-[28rem] rounded-xl border border-border bg-popover p-4 text-popover-foreground shadow-lg",
            self.popover_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_select_classes(&self) -> String {
        shared::build_classes(&[
            "h-9 rounded-md border border-input bg-background px-2 text-sm",
            self.select_class.as_str(),
        ])
    }
}

#[must_use]
pub fn sanitizeDOMID(value: &str) -> String {
    super::datepicker::sanitizeDOMID(value, "datetimepicker")
}

#[must_use]
pub fn sanitizeSignalBase(value: &str) -> String {
    let mut id = sanitizeDOMID(value).replace(['-', '.'], "_");
    if id
        .as_bytes()
        .first()
        .is_some_and(|byte| !byte.is_ascii_alphabetic() && *byte != b'_')
    {
        id = format!("dp_{id}");
    }
    format!("dp_{id}")
}

#[must_use]
pub const fn normalizeDate(date: NaiveDate) -> NaiveDate {
    date
}

#[must_use]
pub fn parseISODate(value: &str) -> Option<NaiveDateTime> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(parsed.naive_utc());
    }
    for layout in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        ISO_DATE_TIME_FORMAT,
    ] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(trimmed, layout) {
            return Some(parsed);
        }
    }
    NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .ok()
        .and_then(|date| normalizeDate(date).and_hms_opt(12, 0, 0))
}

#[must_use]
pub fn loadPickerLocation(timeZone: &str) -> Option<String> {
    let trimmed = timeZone.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[must_use]
pub fn pickerNow(_timeZone: &str) -> NaiveDateTime {
    Utc::now().naive_utc()
}

#[must_use]
pub fn parseValueForDisplay(value: &str, _timeZone: &str) -> Option<NaiveDateTime> {
    parseISODate(value)
}

#[must_use]
pub fn displayTimeZone(timeZone: &str) -> String {
    timeZone.trim().replace('_', " ")
}

#[must_use]
pub fn timeZoneOffsetValue(timeZone: &str) -> String {
    let trimmed = timeZone.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        "UTC".to_string()
    }
}

#[must_use]
pub const fn timeZoneQuickPickZones() -> [&'static str; 5] {
    [
        "UTC",
        "Africa/Lagos",
        "Europe/London",
        "Europe/Paris",
        "America/Toronto",
    ]
}

#[must_use]
pub fn timeZoneChipLabel(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "UTC" {
        "UTC".to_string()
    } else {
        trimmed
            .rsplit('/')
            .next()
            .unwrap_or(trimmed)
            .replace('_', " ")
    }
}

#[must_use]
pub fn isoDateString(date: NaiveDateTime) -> String {
    date.format(ISO_DATE_TIME_FORMAT).to_string()
}

#[must_use]
pub const fn monthNames() -> [&'static str; 12] {
    [
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
    ]
}

#[must_use]
pub fn weekdayNames(week_starts_on: usize) -> Vec<&'static str> {
    let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    (0..7)
        .map(|idx| days[(idx + week_starts_on.min(6)) % 7])
        .collect()
}

#[must_use]
pub const fn resolveWeekStartsOn(props: &DateTimePickerProps) -> usize {
    if props.week_starts_on > 6 {
        0
    } else {
        props.week_starts_on
    }
}

#[must_use]
pub const fn resolveDisplayPattern(use24Hour: bool) -> &'static str {
    if use24Hour {
        "{month} {day}, {year} {hour}:{minute}"
    } else {
        "{month} {day}, {year} {hour12}:{minute} {ampm}"
    }
}

#[must_use]
pub const fn resolveDisplayPatternWithZone(use24Hour: bool) -> &'static str {
    if use24Hour {
        "{month} {day}, {year} {hour}:{minute} {timezone}"
    } else {
        "{month} {day}, {year} {hour12}:{minute} {ampm} {timezone}"
    }
}

#[must_use]
pub const fn resolveMonthYearPattern() -> &'static str {
    "{month} {year}"
}

#[must_use]
pub const fn resolveFullDatePattern() -> &'static str {
    "{month} {day}, {year}"
}

#[must_use]
pub fn applyPattern(pattern: &str, tokens: &[(&str, String)]) -> String {
    let mut result = pattern.to_string();
    for (key, value) in tokens {
        result = result.replace(&format!("{{{key}}}"), value);
    }
    result
}

fn month_name(month: u32) -> &'static str {
    monthNames()
        .get(month.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("")
}

#[must_use]
pub const fn hour24To12(hour24: u32) -> u32 {
    if hour24 == 0 {
        12
    } else if hour24 <= 12 {
        hour24
    } else {
        hour24 - 12
    }
}

#[must_use]
pub const fn floorMinuteToStep(minute: i32) -> u32 {
    if minute < 0 {
        return 0;
    }
    let floored = minute as u32 - (minute as u32 % MINUTE_STEP);
    if floored > 55 { 55 } else { floored }
}

#[must_use]
pub fn hasExplicitTime(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.contains('T') && trimmed.len() >= "2006-01-02T15:04".len()
}

#[must_use]
pub fn timePartsForValue(value: &str, fallback: NaiveDateTime, timeZone: &str) -> (u32, u32) {
    if hasExplicitTime(value) {
        if let Some(parsed) = parseValueForDisplay(value, timeZone) {
            return (parsed.hour(), floorMinuteToStep(parsed.minute() as i32));
        }
    }
    (fallback.hour(), floorMinuteToStep(fallback.minute() as i32))
}

#[must_use]
pub fn formatDisplayValue(value: &str, use24Hour: bool, timeZone: &str) -> String {
    let Some(date_value) = parseValueForDisplay(value, timeZone) else {
        return String::new();
    };
    let pattern = if timeZone.trim().is_empty() {
        resolveDisplayPattern(use24Hour)
    } else {
        resolveDisplayPatternWithZone(use24Hour)
    };
    let hour = date_value.hour();
    let mut tokens = vec![
        ("month", month_name(date_value.month()).to_string()),
        ("day", date_value.day().to_string()),
        ("year", date_value.year().to_string()),
        ("minute", format!("{:02}", date_value.minute())),
        ("timezone", displayTimeZone(timeZone)),
    ];
    if use24Hour {
        tokens.push(("hour", format!("{hour:02}")));
    } else {
        tokens.push(("hour12", hour24To12(hour).to_string()));
        tokens.push(("ampm", if hour < 12 { "AM" } else { "PM" }.to_string()));
    }
    applyPattern(pattern, &tokens)
}

#[must_use]
pub fn formatMonthYearValue(year: i32, month: u32) -> String {
    applyPattern(
        resolveMonthYearPattern(),
        &[
            ("month", month_name(month).to_string()),
            ("year", year.to_string()),
        ],
    )
}

#[must_use]
pub fn formatFullDateValue(date: NaiveDate) -> String {
    applyPattern(
        resolveFullDatePattern(),
        &[
            ("month", month_name(date.month()).to_string()),
            ("day", date.day().to_string()),
            ("year", date.year().to_string()),
        ],
    )
}

fn first_of_month(year: i32, month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month.clamp(1, 12), 1).unwrap_or(NaiveDate::MIN)
}

fn last_of_month(year: i32, month: u32) -> NaiveDate {
    let (next_year, next_month) = if month >= 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    first_of_month(next_year, next_month) - Duration::days(1)
}

#[must_use]
pub fn viewDateForValue(
    value: &str,
    min_value: &str,
    max_value: &str,
    today: NaiveDateTime,
) -> NaiveDate {
    if let Some(selected) = parseISODate(value) {
        return selected.date();
    }

    let mut view_date = normalizeDate(today.date());
    if let Some(min_date) = parseISODate(min_value) {
        if view_date < min_date.date() {
            view_date = min_date.date();
        }
    }
    if let Some(max_date) = parseISODate(max_value) {
        if view_date > max_date.date() {
            view_date = max_date.date();
        }
    }
    view_date
}

#[must_use]
pub fn canShiftMonth(year: i32, month: u32, delta: i32, min_value: &str, max_value: &str) -> bool {
    let month_zero = month.saturating_sub(1) as i32;
    let shifted = month_zero + delta;
    let target_year = year + shifted.div_euclid(12);
    let target_month = (shifted.rem_euclid(12) + 1) as u32;
    let target_first = first_of_month(target_year, target_month);
    let target_last = last_of_month(target_year, target_month);

    if let Some(min_date) = parseISODate(min_value) {
        if target_last < min_date.date() {
            return false;
        }
    }
    if let Some(max_date) = parseISODate(max_value) {
        if target_first > max_date.date() {
            return false;
        }
    }
    true
}

#[must_use]
pub fn buildMonthOptions(
    year: i32,
    min_value: &str,
    max_value: &str,
    names: &[&str],
) -> Vec<CalendarMonthOption> {
    names
        .iter()
        .enumerate()
        .map(|(idx, label)| {
            let month = (idx + 1) as u32;
            let first = first_of_month(year, month);
            let last = last_of_month(year, month);
            let disabled = parseISODate(min_value).is_some_and(|min_date| last < min_date.date())
                || parseISODate(max_value).is_some_and(|max_date| first > max_date.date());
            CalendarMonthOption {
                value: u32::try_from(idx).unwrap_or(0),
                label: (*label).to_string(),
                disabled,
            }
        })
        .collect()
}

#[must_use]
pub fn datePartOnly(value: &str) -> String {
    value
        .split_once('T')
        .map_or_else(|| value.to_string(), |(date, _)| date.to_string())
}

#[must_use]
pub fn buildCalendarCells(
    year: i32,
    month: u32,
    selected_value: &str,
    today_value: &str,
    min_value: &str,
    max_value: &str,
    week_start: usize,
) -> Vec<DateTimeCalendarCell> {
    let first = first_of_month(year, month);
    let offset = (first.weekday().num_days_from_sunday() as usize + 7 - week_start.min(6)) % 7;
    let grid_start = first - Duration::days(offset as i64);
    let selected_date = datePartOnly(selected_value);
    let today_date = datePartOnly(today_value);
    let min_date = datePartOnly(min_value);
    let max_date = datePartOnly(max_value);

    (0..42)
        .map(|idx| {
            let day = grid_start + Duration::days(i64::from(idx));
            let date_part = day.format("%Y-%m-%d").to_string();
            let value = format!("{date_part}T12:00");
            let disabled = (!min_date.is_empty() && date_part.as_str() < min_date.as_str())
                || (!max_date.is_empty() && date_part.as_str() > max_date.as_str());
            DateTimeCalendarCell {
                label: day.day().to_string(),
                value,
                aria_label: formatFullDateValue(day),
                current: day.month() == month,
                today: date_part == today_date,
                selected: date_part == selected_date,
                disabled,
            }
        })
        .collect()
}

#[must_use]
pub fn yearBounds(props: &DateTimePickerProps, today: NaiveDateTime) -> (i32, i32) {
    let view_date = viewDateForValue(&props.value, &props.min, &props.max, today);
    let mut start_year = view_date.year() - 10;
    let mut end_year = view_date.year() + 10;

    if let Some(min_date) = parseISODate(&props.min) {
        start_year = min_date.year();
    }
    if let Some(max_date) = parseISODate(&props.max) {
        end_year = max_date.year();
        if props.min.is_empty() {
            start_year = max_date.year() - 120;
        }
    }
    if !props.min.is_empty() && props.max.is_empty() && start_year > end_year {
        end_year = start_year + 20;
    }
    if start_year > end_year {
        (end_year, start_year)
    } else {
        (start_year, end_year)
    }
}

#[must_use]
pub fn yearSelectOptions(props: &DateTimePickerProps) -> Vec<DatePickerSelectOption> {
    let (start_year, end_year) = yearBounds(props, pickerNow(&props.time_zone_value));
    (start_year..=end_year)
        .map(|year| DatePickerSelectOption {
            value: year.to_string(),
            label: year.to_string(),
        })
        .collect()
}

#[must_use]
pub fn monthSelectOptions() -> Vec<DatePickerSelectOption> {
    monthNames()
        .iter()
        .enumerate()
        .map(|(idx, label)| DatePickerSelectOption {
            value: idx.to_string(),
            label: (*label).to_string(),
        })
        .collect()
}

#[must_use]
pub fn dayCellDate(
    props: &DateTimePickerProps,
    index: usize,
    today: NaiveDateTime,
) -> Option<NaiveDate> {
    if index >= 42 {
        return None;
    }
    let view_date = viewDateForValue(&props.value, &props.min, &props.max, today);
    let first = first_of_month(view_date.year(), view_date.month());
    let offset = (first.weekday().num_days_from_sunday() as usize + 7
        - resolveWeekStartsOn(props).min(6))
        % 7;
    Some(first - Duration::days(offset as i64) + Duration::days(index as i64))
}

#[must_use]
pub fn dayCellLabel(props: &DateTimePickerProps, idx: usize) -> String {
    dayCellDate(props, idx, pickerNow(&props.time_zone_value))
        .map_or_else(String::new, |date| date.day().to_string())
}

#[must_use]
pub fn dayCellAriaLabel(props: &DateTimePickerProps, idx: usize) -> String {
    dayCellDate(props, idx, pickerNow(&props.time_zone_value))
        .map_or_else(String::new, formatFullDateValue)
}

#[must_use]
pub fn jsStringLiteral(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

#[must_use]
pub fn dateTimePickerStateJSPrelude() -> &'static str {
    "const pad = (value) => String(value).padStart(2, '0');"
}

#[must_use]
pub fn timeZoneResultExpr(props: &DateTimePickerProps, idx: usize) -> String {
    format!("(${}[{}] || {{}})", props.time_zone_results_signal(), idx)
}

#[must_use]
pub fn timeZoneResultSelectedExpr(props: &DateTimePickerProps, idx: usize) -> String {
    format!("Boolean(({}).selected)", timeZoneResultExpr(props, idx))
}

#[must_use]
pub fn timeZoneResultLocalExpr(props: &DateTimePickerProps, idx: usize) -> String {
    format!("Boolean(({}).local)", timeZoneResultExpr(props, idx))
}

#[must_use]
pub fn initialViewDate(props: &DateTimePickerProps) -> NaiveDate {
    viewDateForValue(
        &props.value,
        &props.min,
        &props.max,
        pickerNow(&props.time_zone_value),
    )
}

#[must_use]
pub fn initialMonthSelection(props: &DateTimePickerProps) -> String {
    initialViewDate(props).month0().to_string()
}

#[must_use]
pub fn initialMonthTriggerText(props: &DateTimePickerProps) -> String {
    let view = initialViewDate(props);
    formatMonthYearValue(view.year(), view.month())
}

#[must_use]
pub fn initialYearSelection(props: &DateTimePickerProps) -> String {
    initialViewDate(props).year().to_string()
}

#[must_use]
pub fn initialTimeSelection(props: &DateTimePickerProps) -> (u32, u32) {
    timePartsForValue(
        &props.value,
        pickerNow(&props.time_zone_value),
        &props.time_zone_value,
    )
}

#[must_use]
pub fn initialHour24Selection(props: &DateTimePickerProps) -> String {
    let (hour, _) = initialTimeSelection(props);
    format!("{hour:02}")
}

#[must_use]
pub fn initialMinuteSelection(props: &DateTimePickerProps) -> String {
    let (_, minute) = initialTimeSelection(props);
    format!("{minute:02}")
}

#[must_use]
pub fn initialHour12Selection(props: &DateTimePickerProps) -> String {
    let (hour, _) = initialTimeSelection(props);
    format!("{:02}", hour24To12(hour))
}

#[must_use]
pub fn initialAmpmSelection(props: &DateTimePickerProps) -> String {
    let (hour, _) = initialTimeSelection(props);
    if hour < 12 { "0" } else { "1" }.to_string()
}

#[must_use]
pub fn initialDisplayValue(props: &DateTimePickerProps) -> String {
    formatDisplayValue(
        &props.value,
        effectiveUse24Hour(props),
        &props.time_zone_value,
    )
}

#[must_use]
pub fn resolvePlaceholder(props: &DateTimePickerProps) -> String {
    if props.placeholder.is_empty() {
        "Select date and time".to_string()
    } else {
        props.placeholder.clone()
    }
}

#[must_use]
pub fn resolveTimeZonePlaceholder(props: &DateTimePickerProps) -> String {
    if props.time_zone_placeholder.is_empty() {
        "Search time zone".to_string()
    } else {
        props.time_zone_placeholder.clone()
    }
}

#[must_use]
pub const fn effectiveUse24Hour(props: &DateTimePickerProps) -> bool {
    props.use_24_hour
}

#[must_use]
pub fn buildSignals(props: &DateTimePickerProps) -> String {
    let base = props.signal_base();
    let now = pickerNow(&props.time_zone_value);
    let today_value = isoDateString(now);
    let selected_value = props.value.trim();
    let view_date = viewDateForValue(selected_value, &props.min, &props.max, now);
    let (hour, minute) = timePartsForValue(selected_value, now, &props.time_zone_value);
    let names = monthNames();
    let use24 = effectiveUse24Hour(props);
    let mut state = serde_json::Map::new();
    state.insert(props.open_signal(), serde_json::json!(false));
    state.insert(props.value_signal(), serde_json::json!(selected_value));
    state.insert(
        props.display_signal(),
        serde_json::json!(formatDisplayValue(
            selected_value,
            use24,
            &props.time_zone_value
        )),
    );
    state.insert(
        props.time_zone_signal(),
        serde_json::json!(props.time_zone_value.trim()),
    );
    state.insert(
        props.time_zone_query_signal(),
        serde_json::json!(props.time_zone_value.trim()),
    );
    state.insert(props.time_zone_open_signal(), serde_json::json!(false));
    state.insert(props.time_zone_results_signal(), serde_json::json!([]));
    state.insert(props.time_zone_active_index_signal(), serde_json::json!(0));
    state.insert(
        props.time_zone_offset_signal(),
        serde_json::json!(timeZoneOffsetValue(&props.time_zone_value)),
    );
    state.insert(props.year_signal(), serde_json::json!(view_date.year()));
    state.insert(props.month_signal(), serde_json::json!(view_date.month0()));
    state.insert(
        format!("{base}_month_label"),
        serde_json::json!(formatMonthYearValue(view_date.year(), view_date.month())),
    );
    state.insert(
        format!("{base}_month_options"),
        serde_json::json!(buildMonthOptions(
            view_date.year(),
            &props.min,
            &props.max,
            &names
        )),
    );
    state.insert(
        props.cells_signal(),
        serde_json::json!(buildCalendarCells(
            view_date.year(),
            view_date.month(),
            selected_value,
            &today_value,
            &props.min,
            &props.max,
            resolveWeekStartsOn(props),
        )),
    );
    state.insert(
        format!("{base}_can_prev"),
        serde_json::json!(canShiftMonth(
            view_date.year(),
            view_date.month(),
            -1,
            &props.min,
            &props.max
        )),
    );
    state.insert(
        format!("{base}_can_next"),
        serde_json::json!(canShiftMonth(
            view_date.year(),
            view_date.month(),
            1,
            &props.min,
            &props.max
        )),
    );
    state.insert(props.hour_signal(), serde_json::json!(format!("{hour:02}")));
    state.insert(
        props.minute_signal(),
        serde_json::json!(format!("{minute:02}")),
    );
    state.insert(
        format!("{base}_hour12"),
        serde_json::json!(format!("{:02}", hour24To12(hour))),
    );
    state.insert(
        props.ampm_signal(),
        serde_json::json!(if hour < 12 { "0" } else { "1" }),
    );
    state.insert(props.today_signal(), serde_json::json!(today_value));
    state.insert(format!("{base}_min"), serde_json::json!(props.min));
    state.insert(format!("{base}_max"), serde_json::json!(props.max));
    state.insert(
        format!("{base}_week_start"),
        serde_json::json!(resolveWeekStartsOn(props)),
    );
    state.insert(format!("{base}_month_names"), serde_json::json!(names));
    state.insert(
        format!("{base}_display_pattern"),
        serde_json::json!(resolveDisplayPattern(use24)),
    );
    state.insert(
        format!("{base}_display_pattern_with_zone"),
        serde_json::json!(resolveDisplayPatternWithZone(use24)),
    );
    state.insert(
        format!("{base}_month_year_pattern"),
        serde_json::json!(resolveMonthYearPattern()),
    );
    state.insert(
        format!("{base}_full_date_pattern"),
        serde_json::json!(resolveFullDatePattern()),
    );
    state.insert(props.popover_top_signal(), serde_json::json!(0));
    state.insert(props.popover_left_signal(), serde_json::json!(0));
    state.insert(format!("{base}_am_label"), serde_json::json!("AM"));
    state.insert(format!("{base}_pm_label"), serde_json::json!("PM"));
    serde_json::to_string(&state).unwrap_or_default()
}

#[must_use]
pub fn dayLabelExpr(props: &DateTimePickerProps, idx: usize) -> String {
    format!("((${}[{}] || {{}}).label || '')", props.cells_signal(), idx)
}

#[must_use]
pub fn monthTriggerTextExpr(props: &DateTimePickerProps) -> String {
    format!(
        "(((${}_month_options || [])[${}] || {{}}).label || 'Month')",
        props.signal_base(),
        props.month_signal()
    )
}

#[must_use]
pub fn yearTriggerTextExpr(props: &DateTimePickerProps) -> String {
    format!("String(${})", props.year_signal())
}

#[must_use]
pub fn popoverStyleExpr(props: &DateTimePickerProps) -> String {
    format!(
        "'top:' + ${} + 'px;left:' + ${} + 'px;display:' + (${} ? 'block' : 'none')",
        props.popover_top_signal(),
        props.popover_left_signal(),
        props.open_signal()
    )
}

#[must_use]
pub fn useLocalTimeZoneExpr(props: &DateTimePickerProps) -> String {
    format!(
        "const zone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'; ${0} = zone; ${1} = zone; ${2} = false",
        props.time_zone_signal(),
        props.time_zone_query_signal(),
        props.time_zone_open_signal()
    )
}

#[must_use]
pub fn timeZoneResultLabelExpr(props: &DateTimePickerProps, idx: usize) -> String {
    format!(
        "((${}[{}] || {{}}).label || '')",
        props.time_zone_results_signal(),
        idx
    )
}

#[must_use]
pub fn timeZoneResultSubtitleExpr(props: &DateTimePickerProps, idx: usize) -> String {
    format!(
        "((${}[{}] || {{}}).subtitle || '')",
        props.time_zone_results_signal(),
        idx
    )
}

#[must_use]
pub fn timeZoneResultOffsetExpr(props: &DateTimePickerProps, idx: usize) -> String {
    format!(
        "((${}[{}] || {{}}).offset || '')",
        props.time_zone_results_signal(),
        idx
    )
}

#[must_use]
pub fn closeExpr(props: &DateTimePickerProps, commit: bool) -> String {
    if commit {
        format!(
            "${} = false; ${} = false",
            props.open_signal(),
            props.time_zone_open_signal()
        )
    } else {
        format!("${} = false", props.open_signal())
    }
}

fn optional(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

fn time_zone_quick_picks() -> [&'static str; 5] {
    timeZoneQuickPickZones()
}

#[must_use]
pub fn BuildControlAttributes(props:&DateTimePickerProps)->Attrs{
    let reserved=["id","name","value","class","type","min","max","required","disabled","readonly","placeholder","autocomplete"];
    let mut attrs:Attrs=props.attributes.iter().filter(|(key,_)|!reserved.contains(&key.as_str())).map(|(k,v)|(k.clone(),v.clone())).collect();
    let described=if props.aria_described_by.is_empty()&&props.error{props.error_id()}else{props.aria_described_by.clone()};
    if !props.aria_label.is_empty(){attrs.insert("aria-label".into(),props.aria_label.clone());}
    if !described.is_empty(){attrs.insert("aria-describedby".into(),described);}
    if props.error||props.aria_invalid{attrs.insert("aria-invalid".into(),"true".into());}
    for (key,value) in [("autocomplete","off".into()),("inputmode","none".into()),("spellcheck","false".into()),("aria-haspopup","dialog".into()),("aria-controls",props.popover_id()),("aria-expanded","false".into()),("aria-readonly","true".into()),("placeholder",resolvePlaceholder(props)),("data-bind",props.display_signal()),("data-attr:aria-expanded",format!("String(${})",props.open_signal())),("data-on:paste","evt.preventDefault()".into()),("data-on:drop","evt.preventDefault()".into()),("data-on:click",format!("${} = true",props.open_signal())),("data-on:keydown",format!("if (evt.key === 'Enter' || evt.key === 'ArrowDown') {{ evt.preventDefault(); ${} = true }}",props.open_signal()))]{attrs.insert(key.into(),value);}
    attrs
}

pub fn DateTimePicker(props: &DateTimePickerProps) -> Markup {
    let localized=props.clone().with_locale_preferences(true,0);
    let props=&localized;
    let control_attrs=BuildControlAttributes(props);
    crate::html! {
        div
            class=(props.build_wrapper_classes())
            data-signals=(buildSignals(props))
            data-on:click__outside=(closeExpr(props, true))
            data-on:resize__window=(format!("if (${}) {{ /* reposition */ }}", props.open_signal()))
            data-on:scroll__window=(format!("if (${}) {{ /* reposition */ }}", props.open_signal()))
        {
            input type="hidden" id=(props.hidden_input_id()) name=[optional(&props.name)] value=[optional(&props.value)] disabled[props.disabled] data-on:change=(format!("${} = evt.target.value", props.value_signal()));
            div class="relative" {
                input
                    type="text"
                    readonly
                    id=(props.control_id())
                    value=[optional(&initialDisplayValue(props))]
                    class=(props.build_display_classes())
                    required[props.required]
                    disabled[props.disabled]
                    ..(&control_attrs);
                button type="button" aria-label="Open date and time picker" disabled[props.disabled || props.read_only] class="absolute inset-y-0 right-0 flex items-center pr-3" data-on:click=(format!("evt.preventDefault(); ${} = !${}", props.open_signal(), props.open_signal())) {
                    svg class=(shared::build_classes(&["h-4 w-4", props.icon_class.as_str()])) xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8" {
                        path stroke-linecap="round" stroke-linejoin="round" d="M8 2v3m8-3v3M3.75 9.75h16.5M5.25 5.25h13.5A1.5 1.5 0 0 1 20.25 6.75v12A1.5 1.5 0 0 1 18.75 20.25H5.25a1.5 1.5 0 0 1-1.5-1.5v-12A1.5 1.5 0 0 1 5.25 5.25Z" {}
                    }
                }
            }
            div id=(props.popover_id()) role="dialog" aria-label="Date and time calendar" class=(props.build_popover_classes()) style="display:none" data-attr:style=(popoverStyleExpr(props)) data-on:keydown=(format!("if (evt.key === 'Escape') {{ evt.preventDefault(); {} }}", closeExpr(props, false))) {
                div class="flex items-center justify-between gap-2" {
                    button type="button" class="inline-flex h-9 w-9 items-center justify-center rounded-md border border-border hover:bg-muted" aria-label="Previous month" data-on:click=(format!("${} = ${} - 1", props.month_signal(), props.month_signal())) { "‹" }
                    div class="flex min-w-0 flex-1 items-center gap-2" {
                        div class="relative min-w-0 flex-1" {
                            select id=(props.month_select_id()) class="peer absolute inset-0 z-10 h-full w-full cursor-pointer opacity-0 disabled:cursor-not-allowed" aria-label="Month" data-bind=(props.month_signal()) data-on:change=(format!("${} = Number(evt.target.value)", props.month_signal())) {
                                @for option in monthSelectOptions() {
                                    option value=(option.value) { (option.label) }
                                }
                            }
                            div class="pointer-events-none flex h-9 items-center rounded-md border border-input bg-background px-3 pr-9 text-left text-sm font-medium text-foreground shadow-sm" {
                                span class="truncate" data-text=(monthTriggerTextExpr(props)) { "Month" }
                            }
                        }
                        div class="relative w-28 shrink-0" {
                            select id=(props.year_select_id()) class="peer absolute inset-0 z-10 h-full w-full cursor-pointer opacity-0 disabled:cursor-not-allowed" aria-label="Year" data-bind=(props.year_signal()) data-on:change=(format!("${} = Number(evt.target.value)", props.year_signal())) {
                                @for option in yearSelectOptions(props) {
                                    option value=(option.value) { (option.label) }
                                }
                            }
                            div class="pointer-events-none flex h-9 items-center rounded-md border border-input bg-background px-3 pr-9 text-left text-sm font-medium text-foreground shadow-sm" {
                                span data-text=(yearTriggerTextExpr(props)) { "2026" }
                            }
                        }
                    }
                    button type="button" class="inline-flex h-9 w-9 items-center justify-center rounded-md border border-border hover:bg-muted" aria-label="Next month" data-on:click=(format!("${} = ${} + 1", props.month_signal(), props.month_signal())) { "›" }
                }
                div class="mt-3 grid grid-cols-7 gap-1" {
                    @for weekday in weekdayNames(props.week_starts_on) {
                        div class="text-center text-xs font-medium text-muted-foreground" { (weekday) }
                    }
                }
                div class="mt-2 grid grid-cols-7 gap-1" {
                    @for idx in 0..42 {
                        button
                            type="button"
                            class="inline-flex h-9 w-9 items-center justify-center rounded-md text-sm hover:bg-muted disabled:opacity-40"
                            aria-label=(dayCellAriaLabel(props, idx))
                            data-on:click=(format!("${} = (${}[{}] || {{}}).value", props.value_signal(), props.cells_signal(), idx))
                            data-attr:disabled=(format!("(${}[{}] || {{}}).disabled", props.cells_signal(), idx))
                            data-attr:aria-label=(format!("(${}[{}] || {{}}).ariaLabel", props.cells_signal(), idx))
                            data-attr:aria-pressed=(format!("String(Boolean((${}[{}] || {{}}).selected))", props.cells_signal(), idx))
                        {
                            span data-text=(dayLabelExpr(props, idx)) { (dayCellLabel(props, idx)) }
                        }
                    }
                }
                div class="mt-4 border-t border-border pt-4" {
                    div class="rounded-lg border border-border bg-muted/20 p-2.5" {
                        div class=[if props.use_24_hour { Some("grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-1.5") } else { Some("grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)_minmax(0,1.1fr)] items-center gap-1.5") }] {
                            @if props.use_24_hour {
                                select class=(shared::build_classes(&[props.build_select_classes().as_str(), "min-w-0 px-2.5 text-center"])) aria-label="Hour" data-bind=(props.hour_signal()) {
                                    @for hour in 0..24 { option value=(format!("{hour:02}")) { (format!("{hour:02}")) } }
                                }
                            } @else {
                                select class=(shared::build_classes(&[props.build_select_classes().as_str(), "min-w-0 px-2.5 text-center"])) aria-label="Hour" data-bind=(props.hour_signal()) {
                                    @for hour in 1..=12 { option value=(format!("{hour:02}")) { (hour) } }
                                }
                            }
                            span class="shrink-0 text-center text-sm font-semibold text-muted-foreground" { ":" }
                            select class=(shared::build_classes(&[props.build_select_classes().as_str(), "min-w-0 px-2.5 text-center"])) aria-label="Minute" data-bind=(props.minute_signal()) {
                                @for minute in (0..60).step_by(5) { option value=(format!("{minute:02}")) { (format!("{minute:02}")) } }
                            }
                            @if !props.use_24_hour {
                                select class=(shared::build_classes(&[props.build_select_classes().as_str(), "min-w-0 px-2.5 text-center"])) aria-label="AM or PM" data-bind=(props.ampm_signal()) {
                                    option value="0" { "AM" }
                                    option value="1" { "PM" }
                                }
                            }
                        }
                    }
                    @if !props.time_zone_name.is_empty() {
                        div class=(shared::build_classes(&["mt-4 space-y-3", props.time_zone_class.as_str()])) {
                            div class="flex items-center justify-between gap-2" {
                                label for=(props.time_zone_input_id()) class="text-xs font-medium uppercase tracking-[0.08em] text-muted-foreground" { "Time zone" }
                                button type="button" class="inline-flex min-h-7 items-center rounded-full border border-border px-2 text-[11px]" data-on:click=(useLocalTimeZoneExpr(props)) { "Use local" }
                            }
                            input type="hidden" id=(props.time_zone_hidden_input_id()) name=(&props.time_zone_name) value=[optional(&props.time_zone_value)] disabled[props.disabled || props.read_only] data-bind=(props.time_zone_signal());
                            div class="flex flex-wrap items-center gap-2" {
                                @for zone in time_zone_quick_picks() {
                                    button type="button" class="inline-flex min-h-8 items-center rounded-full border border-border px-3 text-xs" aria-label=(zone) data-on:click=(format!("${0} = '{1}'; ${2} = '{1}'; ${3} = false", props.time_zone_signal(), zone, props.time_zone_query_signal(), props.time_zone_open_signal())) { (zone) }
                                }
                            }
                            div class="relative" {
                                input
                                    type="text"
                                    id=(props.time_zone_input_id())
                                    value=[optional(&props.time_zone_value)]
                                    placeholder=[optional(&resolveTimeZonePlaceholder(props))]
                                    class="h-10 w-full rounded-md border border-input bg-background px-3 pr-20 text-sm"
                                    role="combobox"
                                    aria-controls=(props.time_zone_listbox_id())
                                    aria-expanded=(format!("${}", props.time_zone_open_signal()))
                                    data-bind=(props.time_zone_query_signal())
                                    data-on:input=(format!("${} = true", props.time_zone_open_signal()))
                                    data-on:keydown=(format!("if (evt.key === 'ArrowDown') {{ ${0} = Math.min(${1}.length - 1, ${0} + 1) }} if (evt.key === 'Enter') {{ evt.preventDefault(); const row = ${1}[${0}] || {{}}; if (row.value) {{ ${2} = row.value; ${3} = row.label || row.value; ${4} = false }} }} if (evt.key === 'Escape') {{ ${4} = false }}", props.time_zone_active_index_signal(), props.time_zone_results_signal(), props.time_zone_signal(), props.time_zone_query_signal(), props.time_zone_open_signal()));
                                span class="pointer-events-none absolute inset-y-0 right-3 flex items-center rounded-full bg-muted px-2 text-xs text-muted-foreground" data-text=(format!("${}", props.time_zone_offset_signal())) { "UTC" }
                            }
                            div id=(props.time_zone_listbox_id()) role="listbox" aria-label="Time zone" class="hidden rounded-md border border-border bg-popover p-1" data-show=(format!("${}", props.time_zone_open_signal())) {
                                @for idx in 0..5 {
                                    button type="button" role="option" class="flex w-full items-center justify-between rounded px-2 py-1 text-left text-sm hover:bg-muted" data-on:click=(format!("const row = ${}[{}] || {{}}; if (row.value) {{ ${} = row.value; ${} = row.label || row.value; ${} = false }}", props.time_zone_results_signal(), idx, props.time_zone_signal(), props.time_zone_query_signal(), props.time_zone_open_signal())) data-attr:style=(format!("${}.length <= {} ? 'display:none' : ''", props.time_zone_results_signal(), idx)) {
                                        span data-text=(timeZoneResultLabelExpr(props, idx)) { "Zone" }
                                        span class="text-xs text-muted-foreground" data-text=(timeZoneResultOffsetExpr(props, idx)) { "+00:00" }
                                    }
                                }
                            }
                        }
                    }
                }
                @if !props.hide_today || !props.hide_clear {
                    div class="mt-3 flex items-center justify-between gap-2 border-t border-border pt-3" {
                        @if !props.hide_today { button type="button" class="inline-flex h-8 items-center rounded-md px-3 text-xs font-medium text-primary hover:bg-primary/10" data-on:click=(format!("${} = ${}", props.value_signal(), props.today_signal())) { "Today" } }
                        @if !props.hide_clear { button type="button" class="inline-flex h-8 items-center rounded-md px-3 text-xs font-medium text-primary hover:bg-primary/10" data-on:click=(format!("${} = ''; ${} = ''", props.value_signal(), props.display_signal())) { "Clear" } }
                        button type="button" class="inline-flex h-8 items-center rounded-md bg-primary px-3 text-xs font-medium text-primary-foreground" data-on:click=(closeExpr(props, true)) { "Apply" }
                    }
                }
            }
            @if props.error && props.show_error && !props.error_message.is_empty() {
                p id=(props.error_id()) class="mt-1.5 text-xs text-destructive" role="alert" { (&props.error_message) }
            }
        }
    }
}

#[must_use]
pub fn DateTimePickerWithLabel(
    props: &DateTimePickerProps,
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
            (DateTimePicker(props))
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
    fn helpers_build_datetime_and_timezone_expressions() {
        let props = DateTimePickerProps::default()
            .with_name("starts at")
            .with_time_zone("timezone", "UTC");
        assert_eq!(props.control_id(), "starts-at");
        assert_eq!(props.signal_base(), "dp_starts_at");
        assert_eq!(sanitizeDOMID(" starts at! "), "starts-at");
        assert_eq!(sanitizeDOMID("!!!"), "datetimepicker");
        assert_eq!(sanitizeSignalBase("123.start"), "dp_dp_123_start");
        assert_eq!(props.time_zone_signal(), "dp_starts_at_timezone");
        assert!(buildSignals(&props).contains("\"dp_starts_at_timezone\":\"UTC\""));
        assert!(buildSignals(&props).contains("\"dp_starts_at_cells\":["));
        assert!(
            buildSignals(&props.clone().with_value("2026-04-30T09:30"))
                .contains("\"dp_starts_at_ampm\":\"0\"")
        );
        assert!(
            buildSignals(&props.clone().with_value("2026-04-30T22:30"))
                .contains("\"dp_starts_at_ampm\":\"1\"")
        );
        assert_eq!(
            dayLabelExpr(&props, 4),
            "(($dp_starts_at_cells[4] || {}).label || '')"
        );
        assert_eq!(parseISODate(""), None);
        assert_eq!(parseISODate("not-a-date"), None);
        assert_eq!(
            parseISODate("2026-04-30T10:30:00Z").map(isoDateString),
            Some("2026-04-30T10:30".to_string())
        );
        assert_eq!(
            parseISODate("2026-04-30T10:30:05.123").map(isoDateString),
            Some("2026-04-30T10:30".to_string())
        );
        assert_eq!(
            parseISODate("2026-04-30T10:30:05").map(isoDateString),
            Some("2026-04-30T10:30".to_string())
        );
        assert_eq!(
            parseISODate("2026-04-30T10:30").map(isoDateString),
            Some("2026-04-30T10:30".to_string())
        );
        assert_eq!(
            parseISODate("2026-04-30").map(isoDateString),
            Some("2026-04-30T12:00".to_string())
        );
        assert_eq!(loadPickerLocation(""), None);
        assert_eq!(loadPickerLocation(" UTC "), Some("UTC".to_string()));
        assert!(pickerNow("UTC").year() >= 2026);
        assert_eq!(
            parseValueForDisplay("2026-04-30T10:30", "UTC").map(isoDateString),
            Some("2026-04-30T10:30".to_string())
        );
        assert_eq!(displayTimeZone("America/New_York"), "America/New York");
        assert_eq!(timeZoneOffsetValue(""), "");
        assert_eq!(timeZoneOffsetValue("America/Toronto"), "UTC");
        assert_eq!(timeZoneQuickPickZones()[1], "Africa/Lagos");
        assert_eq!(timeZoneChipLabel(""), "UTC");
        assert_eq!(timeZoneChipLabel("UTC"), "UTC");
        assert_eq!(timeZoneChipLabel("America/New_York"), "New York");
        assert_eq!(weekdayNames(1)[0], "Mon");
        assert_eq!(
            resolveWeekStartsOn(&DateTimePickerProps {
                week_starts_on: 9,
                ..DateTimePickerProps::default()
            }),
            0
        );
        assert_eq!(
            resolveDisplayPattern(true),
            "{month} {day}, {year} {hour}:{minute}"
        );
        assert_eq!(
            resolveDisplayPattern(false),
            "{month} {day}, {year} {hour12}:{minute} {ampm}"
        );
        assert_eq!(
            resolveDisplayPatternWithZone(true),
            "{month} {day}, {year} {hour}:{minute} {timezone}"
        );
        assert_eq!(
            resolveDisplayPatternWithZone(false),
            "{month} {day}, {year} {hour12}:{minute} {ampm} {timezone}"
        );
        assert_eq!(resolveMonthYearPattern(), "{month} {year}");
        assert_eq!(resolveFullDatePattern(), "{month} {day}, {year}");
        assert_eq!(
            applyPattern(
                "{month}/{day}/{year}",
                &[
                    ("month", "04".to_string()),
                    ("day", "30".to_string()),
                    ("year", "2026".to_string()),
                ]
            ),
            "04/30/2026"
        );
        assert_eq!(hour24To12(0), 12);
        assert_eq!(hour24To12(12), 12);
        assert_eq!(hour24To12(23), 11);
        assert_eq!(floorMinuteToStep(-1), 0);
        assert_eq!(floorMinuteToStep(34), 30);
        assert_eq!(floorMinuteToStep(59), 55);
        assert_eq!(floorMinuteToStep(61), 55);
        assert!(hasExplicitTime("2026-04-30T10:30"));
        assert!(!hasExplicitTime("2026-04-30"));
        let fallback = parseISODate("2026-04-30T09:37").expect("valid fallback datetime");
        assert_eq!(
            timePartsForValue("2026-04-30T10:34", fallback, "UTC"),
            (10, 30)
        );
        assert_eq!(
            timePartsForValue("2026-04-30Tbad", fallback, "UTC"),
            (9, 35)
        );
        assert_eq!(
            timePartsForValue("2026-04-30Txx:yy", fallback, "UTC"),
            (9, 35)
        );
        assert_eq!(timePartsForValue("badTtime", fallback, "UTC"), (9, 35));
        assert_eq!(timePartsForValue("2026-04-30", fallback, "UTC"), (9, 35));
        assert_eq!(
            formatDisplayValue("2026-04-30T10:30", true, ""),
            "April 30, 2026 10:30"
        );
        assert_eq!(
            formatDisplayValue("2026-04-30T22:30", false, "UTC"),
            "April 30, 2026 10:30 PM UTC"
        );
        assert_eq!(
            formatDisplayValue("2026-04-30T09:15", false, "UTC"),
            "April 30, 2026 9:15 AM UTC"
        );
        assert_eq!(formatDisplayValue("bad", true, ""), "");
        assert_eq!(formatMonthYearValue(2026, 4), "April 2026");
        assert_eq!(
            parseISODate("2026-04-30T10:30").map(|date| formatFullDateValue(date.date())),
            Some("April 30, 2026".to_string())
        );
        let today = parseISODate("2026-04-30T10:30").expect("valid today datetime");
        assert_eq!(
            viewDateForValue("", "2026-05-01", "", today)
                .and_hms_opt(12, 0, 0)
                .map(isoDateString),
            Some("2026-05-01T12:00".to_string())
        );
        assert_eq!(
            viewDateForValue("", "2026-04-01", "", today)
                .and_hms_opt(12, 0, 0)
                .map(isoDateString),
            Some("2026-04-30T12:00".to_string())
        );
        assert_eq!(
            viewDateForValue("", "", "2026-04-01", today)
                .and_hms_opt(12, 0, 0)
                .map(isoDateString),
            Some("2026-04-01T12:00".to_string())
        );
        assert_eq!(
            viewDateForValue("2026-04-15T10:30", "", "", today)
                .and_hms_opt(12, 0, 0)
                .map(isoDateString),
            Some("2026-04-15T12:00".to_string())
        );
        assert!(!canShiftMonth(2026, 4, -1, "2026-04-01", ""));
        assert!(!canShiftMonth(2026, 4, 1, "", "2026-04-30"));
        assert!(canShiftMonth(2026, 4, 0, "2026-04-01", "2026-04-30"));
        let month_options = buildMonthOptions(2026, "2026-04-01", "2026-04-30", &monthNames());
        assert!(
            month_options
                .iter()
                .any(|option| option.label == "March" && option.disabled)
        );
        assert!(
            month_options
                .iter()
                .any(|option| option.label == "April" && !option.disabled)
        );
        assert_eq!(datePartOnly("2026-04-30T10:30"), "2026-04-30");
        assert_eq!(datePartOnly("2026-04-30"), "2026-04-30");
        let cells = buildCalendarCells(
            2026,
            4,
            "2026-04-30T10:30",
            "2026-04-30T09:00",
            "2026-04-01",
            "2026-04-30",
            0,
        );
        assert_eq!(cells.len(), 42);
        assert!(
            cells
                .iter()
                .any(|cell| cell.value == "2026-04-30T12:00" && cell.selected && cell.today)
        );
        assert!(
            cells
                .iter()
                .any(|cell| cell.value == "2026-03-31T12:00" && cell.disabled)
        );
        let bounded = DateTimePickerProps::default().with_attribute("id", "bounded");
        assert_eq!(yearBounds(&bounded, today), (2016, 2036));
        let min_bounded = DateTimePickerProps {
            min: "2024-01-01".to_string(),
            ..DateTimePickerProps::default().with_value("2026-04-30T10:30")
        };
        assert_eq!(yearBounds(&min_bounded, today), (2024, 2036));
        let max_bounded = DateTimePickerProps {
            max: "2026-12-31".to_string(),
            ..DateTimePickerProps::default()
        };
        assert_eq!(yearBounds(&max_bounded, today), (1906, 2026));
        let min_after_view = DateTimePickerProps {
            min: "2026-01-01".to_string(),
            ..DateTimePickerProps::default().with_value("2000-04-30")
        };
        assert_eq!(yearBounds(&min_after_view, today), (2026, 2046));
        let inverted_bounds = DateTimePickerProps {
            min: "2030-01-01".to_string(),
            max: "2020-12-31".to_string(),
            ..DateTimePickerProps::default()
        };
        assert_eq!(yearBounds(&inverted_bounds, today), (2020, 2030));
        assert!(
            yearSelectOptions(&bounded)
                .iter()
                .any(|option| option.value == "2026")
        );
        assert_eq!(monthSelectOptions().len(), 12);
        assert_eq!(dayCellDate(&props, 42, today), None);
        assert_eq!(
            dayCellLabel(
                &DateTimePickerProps::default().with_value("2026-04-30T10:30"),
                3
            ),
            "1"
        );
        assert_eq!(
            dayCellAriaLabel(
                &DateTimePickerProps::default().with_value("2026-04-30T10:30"),
                3
            ),
            "April 1, 2026"
        );
        assert_eq!(jsStringLiteral("a'b"), "\"a'b\"");
        assert_eq!(
            dateTimePickerStateJSPrelude(),
            "const pad = (value) => String(value).padStart(2, '0');"
        );
        assert_eq!(
            timeZoneResultExpr(&props, 2),
            "($dp_starts_at_timezone_results[2] || {})"
        );
        assert_eq!(
            timeZoneResultSelectedExpr(&props, 2),
            "Boolean((($dp_starts_at_timezone_results[2] || {})).selected)"
        );
        assert_eq!(
            timeZoneResultLocalExpr(&props, 2),
            "Boolean((($dp_starts_at_timezone_results[2] || {})).local)"
        );
        assert_eq!(
            initialMonthSelection(&DateTimePickerProps::default().with_value("2026-04-30T10:30")),
            "3"
        );
        assert_eq!(
            initialMonthTriggerText(&DateTimePickerProps::default().with_value("2026-04-30T10:30")),
            "April 2026"
        );
        assert_eq!(
            initialYearSelection(&DateTimePickerProps::default().with_value("2026-04-30T10:30")),
            "2026"
        );
        assert_eq!(
            initialHour24Selection(&DateTimePickerProps::default().with_value("2026-04-30T22:30")),
            "22"
        );
        assert_eq!(
            initialMinuteSelection(&DateTimePickerProps::default().with_value("2026-04-30T22:34")),
            "30"
        );
        assert_eq!(
            initialHour12Selection(&DateTimePickerProps::default().with_value("2026-04-30T22:30")),
            "10"
        );
        assert_eq!(
            initialAmpmSelection(&DateTimePickerProps::default().with_value("2026-04-30T22:30")),
            "1"
        );
        assert_eq!(
            initialAmpmSelection(&DateTimePickerProps::default().with_value("2026-04-30T09:30")),
            "0"
        );
        assert_eq!(
            initialDisplayValue(&DateTimePickerProps::default().with_value("2026-04-30T10:30")),
            "April 30, 2026 10:30"
        );
        assert_eq!(
            resolvePlaceholder(&DateTimePickerProps::default()),
            "Select date and time"
        );
        assert_eq!(
            resolvePlaceholder(&DateTimePickerProps {
                placeholder: "Choose".to_string(),
                ..DateTimePickerProps::default()
            }),
            "Choose"
        );
        assert_eq!(
            resolveTimeZonePlaceholder(&DateTimePickerProps::default()),
            "Search time zone"
        );
        assert_eq!(
            resolveTimeZonePlaceholder(&DateTimePickerProps {
                time_zone_placeholder: "Zone".to_string(),
                ..DateTimePickerProps::default()
            }),
            "Zone"
        );
        assert!(!effectiveUse24Hour(
            &DateTimePickerProps::default().with_use_24_hour(false)
        ));
        assert!(monthTriggerTextExpr(&props).contains("dp_starts_at_month_options"));
        assert_eq!(yearTriggerTextExpr(&props), "String($dp_starts_at_year)");
        assert!(popoverStyleExpr(&props).contains("$dp_starts_at_popover_top"));
        assert!(useLocalTimeZoneExpr(&props).contains("Intl.DateTimeFormat"));
        assert_eq!(
            timeZoneResultLabelExpr(&props, 2),
            "(($dp_starts_at_timezone_results[2] || {}).label || '')"
        );
        assert_eq!(
            timeZoneResultSubtitleExpr(&props, 2),
            "(($dp_starts_at_timezone_results[2] || {}).subtitle || '')"
        );
        assert_eq!(
            timeZoneResultOffsetExpr(&props, 2),
            "(($dp_starts_at_timezone_results[2] || {}).offset || '')"
        );
        assert_eq!(closeExpr(&props, false), "$dp_starts_at_open = false");

        let attr_id = DateTimePickerProps::default().with_attribute("id", "from-attrs");
        assert_eq!(attr_id.control_id(), "from-attrs");
        assert_eq!(
            DateTimePickerProps::default().control_id(),
            "datetimepicker"
        );

        let mut success = DateTimePickerProps::default()
            .with_name("success at")
            .with_class("wrapper-extra")
            .with_attribute("data-testid", "datetime");
        success.success = true;
        assert!(success.build_wrapper_classes().contains("wrapper-extra"));
        assert!(success.build_display_classes().contains("border-success"));
        assert_eq!(
            success.attributes.get("data-testid").map(String::as_str),
            Some("datetime")
        );
    }

    #[test]
    fn datetimepicker_renders_calendar_time_and_timezone_datastar() {
        let props = DateTimePickerProps::default()
            .with_name("starts at")
            .with_value("2026-04-30T10:30:00Z")
            .with_time_zone("time_zone", "UTC")
            .with_required(true);
        let output = DateTimePicker(&props).into_string();
        assert_contains(&output, r#"&quot;dp_starts_at_open&quot;:false"#);
        assert_contains(&output, r#"&quot;dp_starts_at_cells&quot;:["#);
        assert_contains(&output, r#"&quot;value&quot;:&quot;2026-04-30T12:00&quot;"#);
        assert_contains(&output, r#"&quot;selected&quot;:true"#);
        assert_contains(&output, r#"data-bind="dp_starts_at_display""#);
        assert_contains(&output, r#"data-bind="dp_starts_at_month""#);
        assert_contains(&output, r#"data-bind="dp_starts_at_year""#);
        assert_contains(&output, r#"data-bind="dp_starts_at_hour""#);
        assert_contains(&output, r#"data-bind="dp_starts_at_minute""#);
        assert_contains(
            &output,
            r#"data-attr:style="'top:' + $dp_starts_at_popover_top"#,
        );
        assert_contains(
            &output,
            r#"data-text="(($dp_starts_at_cells[0] || {}).label || '')""#,
        );
        assert_contains(&output, r#"data-bind="dp_starts_at_timezone""#);
        assert_contains(&output, r#"data-bind="dp_starts_at_timezone_query""#);
        assert_contains(&output, r#"role="combobox""#);
        assert_contains(&output, "ArrowDown");
        assert_contains(&output, "Enter");
        assert_contains(&output, r#"data-show="$dp_starts_at_timezone_open""#);
        assert_contains(
            &output,
            r#"data-on:click="const row = $dp_starts_at_timezone_results[0] || {};"#,
        );
        assert_contains(&output, r#"data-text="$dp_starts_at_timezone_offset""#);
    }

    #[test]
    fn datetimepicker_with_label_and_12_hour_variation_render() {
        let props = DateTimePickerProps::default()
            .with_id("meeting")
            .with_required(true)
            .with_use_24_hour(false);
        let output = DateTimePickerWithLabel(&props, "Meeting", "Pick a time").into_string();
        assert_contains(&output, r#"<label for="meeting""#);
        assert_contains(&output, "Meeting");
        assert_contains(&output, "Pick a time");
        assert_contains(&output, r#"aria-label="AM or PM""#);
        assert_contains(&output, r#"data-bind="dp_meeting_ampm""#);

        let error = DateTimePickerWithLabel(&props.with_error("Bad"), "Meeting", "Pick a time")
            .into_string();
        assert_contains(&error, "text-destructive");
        assert!(!error.contains("Pick a time"));
    }
}
