#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use maud::{Markup, html};
use serde::Serialize;
use std::collections::BTreeMap;

type Attrs = BTreeMap<String, String>;
const ISO_DATE_FORMAT: &str = "%Y-%m-%d";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CalendarCell {
    pub label: String,
    pub value: String,
    #[serde(rename = "ariaLabel")]
    pub aria_label: String,
    pub current: bool,
    pub today: bool,
    pub selected: bool,
    pub disabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CalendarMonthOption {
    pub value: u32,
    pub label: String,
    pub disabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatePickerSelectOption {
    pub value: String,
    pub label: String,
}

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "datepicker",
        name: "DatePicker",
    },
    ComponentInfo {
        family: "unstyled",
        module: "datepicker",
        name: "DatePickerWithLabel",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatePickerProps {
    pub name: String,
    pub id: String,
    pub placeholder: String,
    pub value: String,
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
    pub nav_button_class: String,
    pub day_button_class: String,
    pub footer_button_class: String,
    pub weekday_class: String,
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
}

impl Default for DatePickerProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            placeholder: String::new(),
            value: String::new(),
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
            nav_button_class: String::new(),
            day_button_class: String::new(),
            footer_button_class: String::new(),
            weekday_class: String::new(),
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
        }
    }
}

impl DatePickerProps {
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
    pub fn with_placeholder(mut self, value: impl Into<String>) -> Self {
        self.placeholder = value.into();
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
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    #[must_use]
    pub const fn with_read_only(mut self, value: bool) -> Self {
        self.read_only = value;
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
            sanitizeDOMID(&self.name, "datepicker")
        } else {
            "datepicker".to_string()
        }
    }

    #[must_use]
    pub fn hidden_input_id(&self) -> String {
        format!("{}-value", self.control_id())
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
        sanitizeSignalBase(&self.control_id(), "dp")
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
    pub fn today_signal(&self) -> String {
        format!("{}_today", self.signal_base())
    }
    #[must_use]
    pub fn can_prev_signal(&self) -> String {
        format!("{}_can_prev", self.signal_base())
    }
    #[must_use]
    pub fn can_next_signal(&self) -> String {
        format!("{}_can_next", self.signal_base())
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
            "absolute z-50 mt-2 w-80 rounded-xl border border-border bg-popover p-4 text-popover-foreground shadow-lg",
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

    #[must_use]
    pub fn build_nav_button_classes(&self) -> String {
        shared::build_classes(&[
            "inline-flex h-9 w-9 items-center justify-center rounded-md border border-border hover:bg-muted disabled:opacity-50",
            self.nav_button_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_day_button_classes(&self) -> String {
        shared::build_classes(&[
            "inline-flex h-9 w-9 items-center justify-center rounded-md text-sm hover:bg-muted disabled:opacity-40",
            self.day_button_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_footer_button_classes(&self) -> String {
        shared::build_classes(&[
            "inline-flex h-8 items-center rounded-md px-3 text-xs font-medium text-primary hover:bg-primary/10",
            self.footer_button_class.as_str(),
        ])
    }
}

#[must_use]
pub fn sanitizeDOMID(value: &str, fallback: &str) -> String {
    let mut output = String::new();
    let mut last_dash = false;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            output.push(ch);
            last_dash = false;
        } else if !last_dash {
            output.push('-');
            last_dash = true;
        }
    }
    let trimmed = output.trim_matches(['-', '_']).to_string();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed
    }
}

#[must_use]
pub fn sanitizeSignalBase(value: &str, prefix: &str) -> String {
    let mut id = sanitizeDOMID(value, "datepicker")
        .replace('-', "_")
        .replace('.', "_");
    if id
        .as_bytes()
        .first()
        .is_some_and(|byte| !byte.is_ascii_alphabetic() && *byte != b'_')
    {
        id = format!("{prefix}_{id}");
    }
    format!("{prefix}_{id}")
}

#[must_use]
pub const fn normalizeDate(date: NaiveDate) -> NaiveDate {
    date
}

#[must_use]
pub fn parseISODate(value: &str) -> Option<NaiveDate> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    NaiveDate::parse_from_str(trimmed, ISO_DATE_FORMAT).ok()
}

#[must_use]
pub fn isoDateString(date: NaiveDate) -> String {
    normalizeDate(date).format(ISO_DATE_FORMAT).to_string()
}

#[must_use]
pub const fn resolveDisplayPattern() -> &'static str {
    "{month} {day}, {year}"
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
    let names = monthNames();
    let index = month.saturating_sub(1) as usize;
    names.get(index).copied().unwrap_or("")
}

#[must_use]
pub fn formatDisplayValue(value: &str) -> String {
    parseISODate(value).map_or_else(String::new, formatFullDateValue)
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

fn today_date() -> NaiveDate {
    Utc::now().date_naive()
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
    today: NaiveDate,
) -> NaiveDate {
    if let Some(selected) = parseISODate(value) {
        return selected;
    }

    let mut view_date = normalizeDate(today);
    if let Some(min_date) = parseISODate(min_value) {
        if view_date < min_date {
            view_date = min_date;
        }
    }
    if let Some(max_date) = parseISODate(max_value) {
        if view_date > max_date {
            view_date = max_date;
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
        if target_last < min_date {
            return false;
        }
    }
    if let Some(max_date) = parseISODate(max_value) {
        if target_first > max_date {
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
            let disabled = parseISODate(min_value).is_some_and(|min_date| last < min_date)
                || parseISODate(max_value).is_some_and(|max_date| first > max_date);
            CalendarMonthOption {
                value: u32::try_from(idx).unwrap_or(0),
                label: (*label).to_string(),
                disabled,
            }
        })
        .collect()
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
) -> Vec<CalendarCell> {
    let first = first_of_month(year, month);
    let offset = (first.weekday().num_days_from_sunday() as usize + 7 - week_start.min(6)) % 7;
    let grid_start = first - Duration::days(offset as i64);

    (0..42)
        .map(|idx| {
            let day = grid_start + Duration::days(i64::from(idx));
            let value = isoDateString(day);
            let disabled = (!min_value.is_empty() && value.as_str() < min_value)
                || (!max_value.is_empty() && value.as_str() > max_value);
            CalendarCell {
                label: day.day().to_string(),
                value: value.clone(),
                aria_label: formatFullDateValue(day),
                current: day.month() == month,
                today: value == today_value,
                selected: value == selected_value,
                disabled,
            }
        })
        .collect()
}

#[must_use]
pub fn yearBounds(props: &DatePickerProps, today: NaiveDate) -> (i32, i32) {
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
pub fn yearSelectOptions(props: &DatePickerProps) -> Vec<DatePickerSelectOption> {
    let (start_year, end_year) = yearBounds(props, today_date());
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
pub fn dayCellDate(props: &DatePickerProps, index: usize, today: NaiveDate) -> Option<NaiveDate> {
    if index >= 42 {
        return None;
    }
    let view_date = viewDateForValue(&props.value, &props.min, &props.max, today);
    let first = first_of_month(view_date.year(), view_date.month());
    let offset =
        (first.weekday().num_days_from_sunday() as usize + 7 - props.week_starts_on.min(6)) % 7;
    Some(first - Duration::days(offset as i64) + Duration::days(index as i64))
}

#[must_use]
pub fn dayCellLabel(props: &DatePickerProps, idx: usize) -> String {
    dayCellDate(props, idx, today_date()).map_or_else(String::new, |date| date.day().to_string())
}

#[must_use]
pub fn dayCellAriaLabel(props: &DatePickerProps, idx: usize) -> String {
    dayCellDate(props, idx, today_date()).map_or_else(String::new, formatFullDateValue)
}

#[must_use]
pub fn jsStringLiteral(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

#[must_use]
pub fn datePickerStateJSPrelude() -> &'static str {
    "const pad = (value) => String(value).padStart(2, '0');"
}

#[must_use]
pub fn initialDisplayValue(props: &DatePickerProps) -> String {
    if !props.value.is_empty() {
        formatDisplayValue(&props.value)
    } else {
        String::new()
    }
}

#[must_use]
pub fn resolvePlaceholder(props: &DatePickerProps) -> String {
    if props.placeholder.is_empty() {
        "Select a date".to_string()
    } else {
        props.placeholder.clone()
    }
}

#[must_use]
pub fn buildSignals(props: &DatePickerProps) -> String {
    let base = props.signal_base();
    let today = today_date();
    let today_value = isoDateString(today);
    let selected_value = props.value.trim();
    let view_date = viewDateForValue(selected_value, &props.min, &props.max, today);
    let month_names = monthNames();
    let state = serde_json::json!({
        props.open_signal(): false,
        props.value_signal(): selected_value,
        props.display_signal(): formatDisplayValue(selected_value),
        props.year_signal(): view_date.year(),
        props.month_signal(): view_date.month0(),
        format!("{base}_month_label"): formatMonthYearValue(view_date.year(), view_date.month()),
        format!("{base}_month_options"): buildMonthOptions(view_date.year(), &props.min, &props.max, &month_names),
        props.cells_signal(): buildCalendarCells(
            view_date.year(),
            view_date.month(),
            selected_value,
            &today_value,
            &props.min,
            &props.max,
            props.week_starts_on,
        ),
        props.can_prev_signal(): canShiftMonth(view_date.year(), view_date.month(), -1, &props.min, &props.max),
        props.can_next_signal(): canShiftMonth(view_date.year(), view_date.month(), 1, &props.min, &props.max),
        props.today_signal(): today_value,
        format!("{base}_min"): props.min,
        format!("{base}_max"): props.max,
        format!("{base}_week_start"): props.week_starts_on.min(6),
        format!("{base}_month_names"): month_names,
        format!("{base}_display_pattern"): resolveDisplayPattern(),
        format!("{base}_month_year_pattern"): resolveMonthYearPattern(),
        format!("{base}_full_date_pattern"): resolveFullDatePattern(),
        props.popover_top_signal(): 0,
        props.popover_left_signal(): 0,
    });
    serde_json::to_string(&state).unwrap_or_default()
}

#[must_use]
pub fn dayLabelExpr(props: &DatePickerProps, idx: usize) -> String {
    format!("((${}[{}] || {{}}).label || '')", props.cells_signal(), idx)
}

#[must_use]
pub fn popoverStyleExpr(props: &DatePickerProps) -> String {
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
pub fn DatePicker(props: &DatePickerProps) -> Markup {
    let described_by = if props.aria_described_by.is_empty() && props.error {
        props.error_id()
    } else {
        props.aria_described_by.clone()
    };
    html! {
        div
            class=(props.build_wrapper_classes())
            data-signals=(buildSignals(props))
            data-on:click__outside=(format!("${} = false", props.open_signal()))
            data-on:resize__window=(format!("if (${}) {{ /* reposition */ }}", props.open_signal()))
            data-on:scroll__window=(format!("if (${}) {{ /* reposition */ }}", props.open_signal()))
        {
            input
                type="hidden"
                id=(props.hidden_input_id())
                name=[optional(&props.name)]
                value=[optional(&props.value)]
                disabled[props.disabled]
                data-on:change=(format!("${} = evt.target.value", props.value_signal()));

            div class="relative" {
                input
                    type="text"
                    id=(props.control_id())
                    value=[optional(&initialDisplayValue(props))]
                    placeholder=[optional(&resolvePlaceholder(props))]
                    class=(props.build_display_classes())
                    required[props.required]
                    disabled[props.disabled]
                    readonly[props.read_only]
                    aria-label=[optional(&props.aria_label)]
                    aria-describedby=[optional(&described_by)]
                    aria-invalid=[if props.error || props.aria_invalid { Some("true") } else { None }]
                    data-attr:value=(format!("${}", props.display_signal()))
                    data-attr:aria-expanded=(format!("String(${})", props.open_signal()))
                    data-on:input=(format!("evt.target.value = ${}", props.display_signal()))
                    data-on:paste="evt.preventDefault()"
                    data-on:drop="evt.preventDefault()"
                    data-on:click=(format!("${} = true", props.open_signal()))
                    data-on:keydown=(format!("if (evt.key === 'Enter' || evt.key === 'ArrowDown') {{ evt.preventDefault(); ${} = true }}", props.open_signal()));
                button
                    type="button"
                    aria-label="Open calendar"
                    disabled[props.disabled || props.read_only]
                    class="absolute inset-y-0 right-0 flex items-center pr-3"
                    data-attr:aria-expanded=(format!("String(${})", props.open_signal()))
                    data-on:click=(format!("evt.preventDefault(); ${} = !${}", props.open_signal(), props.open_signal()))
                {
                    svg class=(shared::build_classes(&["h-4 w-4", props.icon_class.as_str()])) xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8" {
                        path stroke-linecap="round" stroke-linejoin="round" d="M8 2v3m8-3v3M3.75 9.75h16.5M5.25 5.25h13.5A1.5 1.5 0 0 1 20.25 6.75v12A1.5 1.5 0 0 1 18.75 20.25H5.25a1.5 1.5 0 0 1-1.5-1.5v-12A1.5 1.5 0 0 1 5.25 5.25Z" {}
                    }
                }
            }

            div
                id=(props.popover_id())
                role="dialog"
                aria-label="Calendar"
                class=(props.build_popover_classes())
                style="display:none"
                data-attr:style=(popoverStyleExpr(props))
                data-on:keydown=(format!("if (evt.key === 'Escape') {{ evt.preventDefault(); ${} = false }}", props.open_signal()))
            {
                div class="flex items-center justify-between gap-2" {
                    button type="button" class=(props.build_nav_button_classes()) aria-label="Previous month" data-on:click=(format!("${} = ${} - 1", props.month_signal(), props.month_signal())) data-attr:disabled=(format!("!${}", props.can_prev_signal())) { "‹" }
                    div class="flex min-w-0 flex-1 items-center gap-2" {
                        select id=(props.month_select_id()) class=(shared::build_classes(&[props.build_select_classes().as_str(), "min-w-0 flex-1"])) aria-label="Month" disabled[props.disabled || props.read_only] data-bind=(props.month_signal()) data-on:change=(format!("${} = Number(evt.target.value)", props.month_signal())) {
                            @for (idx, month) in monthNames().iter().enumerate() {
                                option value=(idx) { (month) }
                            }
                        }
                        select id=(props.year_select_id()) class=(shared::build_classes(&[props.build_select_classes().as_str(), "w-28 shrink-0"])) aria-label="Year" disabled[props.disabled || props.read_only] data-bind=(props.year_signal()) data-on:change=(format!("${} = Number(evt.target.value)", props.year_signal())) {
                            @for year in 2020..=2035 {
                                option value=(year) { (year) }
                            }
                        }
                    }
                    button type="button" class=(props.build_nav_button_classes()) aria-label="Next month" data-on:click=(format!("${} = ${} + 1", props.month_signal(), props.month_signal())) data-attr:disabled=(format!("!${}", props.can_next_signal())) { "›" }
                }
                div class="mt-3 grid grid-cols-7 gap-1" {
                    @for weekday in weekdayNames(props.week_starts_on) {
                        div class=(shared::build_classes(&["text-center text-xs font-medium text-muted-foreground", props.weekday_class.as_str()])) { (weekday) }
                    }
                }
                div class="mt-2 grid grid-cols-7 gap-1" {
                    @for idx in 0..42 {
                        button
                            type="button"
                            class=(props.build_day_button_classes())
                            aria-label=(dayCellAriaLabel(props, idx))
                            data-on:click=(format!("${} = (${}[{}] || {{}}).value; ${} = false", props.value_signal(), props.cells_signal(), idx, props.open_signal()))
                            data-attr:disabled=(format!("(${}[{}] || {{}}).disabled", props.cells_signal(), idx))
                            data-attr:aria-label=(format!("(${}[{}] || {{}}).ariaLabel", props.cells_signal(), idx))
                            data-attr:aria-pressed=(format!("String(Boolean((${}[{}] || {{}}).selected))", props.cells_signal(), idx))
                        {
                            span data-text=(dayLabelExpr(props, idx)) { (dayCellLabel(props, idx)) }
                        }
                    }
                }
                @if !props.hide_today || !props.hide_clear {
                    div class="mt-3 flex items-center justify-between gap-2 border-t border-border pt-3" {
                        div class="flex items-center gap-2" {
                            @if !props.hide_today {
                                button type="button" class=(props.build_footer_button_classes()) data-on:click=(format!("${} = ${}", props.value_signal(), props.today_signal())) { "Today" }
                            }
                        }
                        @if !props.hide_clear {
                            button type="button" class=(props.build_footer_button_classes()) data-on:click=(format!("${} = ''; ${} = ''", props.value_signal(), props.display_signal())) { "Clear" }
                        }
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
pub fn DatePickerWithLabel(props: &DatePickerProps, label: &str, helperText: &str) -> Markup {
    html! {
        div class="space-y-1.5" {
            @if !label.is_empty() {
                label for=(props.control_id()) class=(shared::build_classes(&["text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70", if props.error { "text-destructive" } else { "" }])) {
                    (label)
                    @if props.required {
                        span class="ml-1 text-destructive" aria-label="required" { "*" }
                    }
                }
            }
            (DatePicker(props))
            @if !helperText.is_empty() && !props.error {
                p class="text-xs text-muted-foreground" { (helperText) }
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_contains(html: &str, expected: &str) {
        assert!(html.contains(expected), "missing `{expected}` in `{html}`");
    }

    #[test]
    fn helpers_build_ids_signals_and_calendar_labels() {
        let props = DatePickerProps::default().with_name("starts on");
        assert_eq!(sanitizeDOMID(" starts on! ", "datepicker"), "starts-on");
        assert_eq!(sanitizeDOMID("!!!", "datepicker"), "datepicker");
        assert_eq!(sanitizeSignalBase("123.start", "dp"), "dp_dp_123_start");
        assert_eq!(props.control_id(), "starts-on");
        assert_eq!(props.hidden_input_id(), "starts-on-value");
        assert_eq!(props.popover_id(), "starts-on-calendar");
        assert_eq!(props.signal_base(), "dp_starts_on");
        assert_eq!(props.open_signal(), "dp_starts_on_open");
        assert_eq!(props.value_signal(), "dp_starts_on_value");
        assert!(buildSignals(&props).contains("\"dp_starts_on_open\":false"));
        assert_eq!(
            dayLabelExpr(&props, 3),
            "(($dp_starts_on_cells[3] || {}).label || '')"
        );
        assert_eq!(
            parseISODate("2026-04-30").map(isoDateString),
            Some("2026-04-30".to_string())
        );
        assert_eq!(parseISODate("not-a-date"), None);
        assert_eq!(formatDisplayValue("2026-04-30"), "April 30, 2026");
        assert_eq!(formatDisplayValue(""), "");
        assert_eq!(formatMonthYearValue(2026, 4), "April 2026");
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
        let today = parseISODate("2026-04-30");
        assert_eq!(
            today.map(|date| viewDateForValue("", "2026-05-01", "", date)),
            parseISODate("2026-05-01")
        );
        assert_eq!(
            today.map(|date| viewDateForValue("", "2026-04-01", "", date)),
            parseISODate("2026-04-30")
        );
        assert_eq!(
            today.map(|date| viewDateForValue("", "", "2026-04-01", date)),
            parseISODate("2026-04-01")
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
        let cells = buildCalendarCells(
            2026,
            4,
            "2026-04-30",
            "2026-04-30",
            "2026-04-01",
            "2026-04-30",
            0,
        );
        assert_eq!(cells.len(), 42);
        assert!(
            cells
                .iter()
                .any(|cell| cell.value == "2026-04-30" && cell.selected && cell.today)
        );
        assert!(
            cells
                .iter()
                .any(|cell| cell.value == "2026-03-31" && cell.disabled)
        );
        let bounded = DatePickerProps::default()
            .with_value("")
            .with_attribute("id", "bounded");
        assert_eq!(
            yearBounds(
                &bounded,
                parseISODate("2026-04-30").unwrap_or(NaiveDate::MIN)
            ),
            (2016, 2036)
        );
        assert_eq!(
            yearBounds(
                &DatePickerProps::default().with_value("2026-04-30"),
                parseISODate("2026-04-30").unwrap_or(NaiveDate::MIN)
            ),
            (2016, 2036)
        );
        let min_bounded = DatePickerProps {
            min: "2024-01-01".to_string(),
            ..DatePickerProps::default().with_value("2026-04-30")
        };
        assert_eq!(
            yearBounds(
                &min_bounded,
                parseISODate("2026-04-30").unwrap_or(NaiveDate::MIN)
            ),
            (2024, 2036)
        );
        let max_bounded = DatePickerProps {
            max: "2026-12-31".to_string(),
            ..DatePickerProps::default()
        };
        assert_eq!(
            yearBounds(
                &max_bounded,
                parseISODate("2026-04-30").unwrap_or(NaiveDate::MIN)
            ),
            (1906, 2026)
        );
        let min_after_view = DatePickerProps {
            min: "2026-01-01".to_string(),
            ..DatePickerProps::default().with_value("2000-04-30")
        };
        assert_eq!(
            yearBounds(
                &min_after_view,
                parseISODate("2026-04-30").unwrap_or(NaiveDate::MIN)
            ),
            (2026, 2046)
        );
        let inverted_bounds = DatePickerProps {
            min: "2030-01-01".to_string(),
            max: "2020-12-31".to_string(),
            ..DatePickerProps::default()
        };
        assert_eq!(
            yearBounds(
                &inverted_bounds,
                parseISODate("2026-04-30").unwrap_or(NaiveDate::MIN)
            ),
            (2020, 2030)
        );
        assert!(
            yearSelectOptions(&bounded)
                .iter()
                .any(|option| option.value == "2026")
        );
        assert_eq!(monthSelectOptions().len(), 12);
        assert_eq!(
            dayCellLabel(&DatePickerProps::default().with_value("2026-04-30"), 3),
            "1"
        );
        assert_eq!(dayCellDate(&props, 42, NaiveDate::MIN), None);
        assert_eq!(jsStringLiteral("a'b"), "\"a'b\"");
        assert_eq!(
            datePickerStateJSPrelude(),
            "const pad = (value) => String(value).padStart(2, '0');"
        );
        assert_eq!(
            initialDisplayValue(&DatePickerProps::default().with_value("2026-04-30")),
            "April 30, 2026"
        );
        assert_eq!(
            resolvePlaceholder(&DatePickerProps::default()),
            "Select a date"
        );
        assert_eq!(weekdayNames(1)[0], "Mon");
        assert!(popoverStyleExpr(&props).contains("$dp_starts_on_popover_top"));

        let attr_id = DatePickerProps::default().with_attribute("id", "from-attrs");
        assert_eq!(attr_id.control_id(), "from-attrs");
        assert_eq!(DatePickerProps::default().control_id(), "datepicker");

        let mut success = DatePickerProps::default()
            .with_disabled(true)
            .with_read_only(true)
            .with_class("wrapper-extra")
            .with_attribute("data-testid", "date")
            .with_name("success date");
        success.success = true;
        assert!(success.build_wrapper_classes().contains("wrapper-extra"));
        assert!(success.build_display_classes().contains("border-success"));
        assert_eq!(
            success.attributes.get("data-testid").map(String::as_str),
            Some("date")
        );
        let output = DatePicker(&success).into_string();
        assert_contains(&output, r#"disabled"#);
        assert_contains(&output, r#"readonly"#);
    }

    #[test]
    fn datepicker_renders_datastar_calendar_and_actions() {
        let props = DatePickerProps::default()
            .with_name("starts on")
            .with_value("2026-04-30")
            .with_placeholder("Choose")
            .with_required(true)
            .with_error("Required");
        let output = DatePicker(&props).into_string();
        assert_contains(&output, r#"&quot;dp_starts_on_open&quot;:false"#);
        assert_contains(&output, r#"&quot;dp_starts_on_cells&quot;:["#);
        assert_contains(&output, r#"&quot;value&quot;:&quot;2026-04-30&quot;"#);
        assert_contains(&output, r#"&quot;selected&quot;:true"#);
        assert_contains(
            &output,
            r#"data-on:click__outside="$dp_starts_on_open = false""#,
        );
        assert_contains(&output, r#"data-attr:value="$dp_starts_on_display""#);
        assert_contains(&output, r#"data-bind="dp_starts_on_month""#);
        assert_contains(&output, r#"data-bind="dp_starts_on_year""#);
        assert_contains(
            &output,
            r#"data-attr:style="'top:' + $dp_starts_on_popover_top"#,
        );
        assert_contains(
            &output,
            r#"data-on:click="$dp_starts_on_value = ($dp_starts_on_cells[0] || {}).value; $dp_starts_on_open = false""#,
        );
        assert_contains(
            &output,
            r#"data-text="(($dp_starts_on_cells[0] || {}).label || '')""#,
        );
        assert_contains(&output, r#"id="starts-on-error""#);
        assert_contains(&output, "Required");
    }

    #[test]
    fn datepicker_with_label_renders_required_and_helper_states() {
        let props = DatePickerProps::default()
            .with_id("due")
            .with_required(true);
        let output = DatePickerWithLabel(&props, "Due date", "Pick one").into_string();
        assert_contains(&output, r#"<label for="due""#);
        assert_contains(&output, "Due date");
        assert_contains(&output, r#"aria-label="required""#);
        assert_contains(&output, "Pick one");

        let error_props = props.clone().with_error("Bad date");
        let error_output = DatePickerWithLabel(&error_props, "Due date", "Pick one").into_string();
        assert_contains(&error_output, "text-destructive");
        assert!(!error_output.contains("Pick one"));
    }
}
