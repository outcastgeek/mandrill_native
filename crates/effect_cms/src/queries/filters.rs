//! Filter chips and toggle URLs.
//!
//! The filter owner owns the data a filter chip carries and the URL that toggles one
//! filter value; the consuming presentation owner renders it. No HTML is authored
//! here.
//!
//! The toggle URL is built from the request's own query when the caller supplies no
//! base path, toggles a single-value parameter between absent/present,
//! and toggles a multi-value parameter by normalizing the repeated values, removing
//! or appending the toggled value and normalizing again. `ResetPage` drops the page
//! parameter in both modes. Those branches and their order are preserved.

use crate::commands::InputBytes;
use std::sync::Arc;

use crate::definitions::{
    RESOURCE_FILTER_MODE_MULTI_ALL, ResourceFilterDefinition, ResourceFilterKey,
};
use effect_server::Request;

use super::list_query::{
    DEFAULT_MAX_FILTER_VALUES, FilterValueOptions, PAGE_QUERY_PARAM, QueryValues,
    append_encoded_query, filter_query_param, normalize_filter_values, parse_request_query,
    resource_filter_mode,
};

/// One rendered filter chip.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceFilterChipVM {
    /// Filter key the chip belongs to.
    pub key: ResourceFilterKey,
    /// Filter value the chip carries.
    pub value: InputBytes,
    /// Chip label.
    pub label: String,
    /// Chip aria label.
    pub aria_label: String,
    /// Toggle URL.
    pub href: String,
    /// Whether the chip's value is currently selected.
    pub active: bool,
    /// Remove URL; empty unless the chip is active.
    pub remove_href: String,
}

/// Configures a filter toggle URL.
#[derive(Clone, Default)]
pub struct FilterToggleURLOptions {
    /// Request whose query is the toggle base.
    pub request: Option<Request>,
    /// Base path; empty means the request path.
    pub base_path: String,
    /// Filter declaration.
    pub definition: ResourceFilterDefinition,
    /// Value to toggle.
    pub value: InputBytes,
    /// Whether to drop the page parameter.
    pub reset_page: bool,
    /// Whether to sort the normalized values.
    pub sort_values: bool,
}

/// Resolves a chip label and aria label from its value and active state.
pub type ResourceFilterChipLabelFunc = Arc<dyn Fn(&[u8], bool) -> (String, String) + Send + Sync>;

/// Configures tag chip rendering.
#[derive(Clone, Default)]
pub struct ResourceTagChipsOptions {
    /// Request whose query is the toggle base.
    pub request: Option<Request>,
    /// Base path; empty means the request path.
    pub base_path: String,
    /// Known tags.
    pub tags: Vec<InputBytes>,
    /// Selected tags.
    pub selected_tags: Vec<InputBytes>,
    /// Tag filter declaration.
    pub filter: ResourceFilterDefinition,
    /// Optional chip label resolver.
    pub label: Option<ResourceFilterChipLabelFunc>,
}

/// The first value stored for a query parameter.
fn query_first(query: &QueryValues, key: &str) -> InputBytes {
    query
        .get(key.as_bytes())
        .and_then(|values| values.first())
        .cloned()
        .unwrap_or_default()
}

/// Build the URL that toggles one filter value.
#[must_use]
pub fn resource_filter_toggle_url(options: &FilterToggleURLOptions) -> String {
    let mut base = options.base_path.trim().to_owned();
    if base.is_empty()
        && let Some(request) = &options.request
    {
        base = request.uri().path().to_owned();
    }
    let mut query: QueryValues = match &options.request {
        Some(request) => match request.uri().query() {
            Some(raw) => parse_request_query(raw),
            None => QueryValues::new(),
        },
        None => QueryValues::new(),
    };
    let param = filter_query_param(&options.definition);
    let value = options.value.trim().to_owned();
    if value.is_empty() {
        return append_encoded_query(&base, &query);
    }
    if resource_filter_mode(&options.definition).as_str() != RESOURCE_FILTER_MODE_MULTI_ALL {
        if query_first(&query, &param) == value {
            query.remove(param.as_bytes());
        } else {
            query.insert(param.into(), vec![value]);
        }
    } else {
        let normalize_options = FilterValueOptions {
            sort: options.sort_values,
            lowercase: false,
            max_values: DEFAULT_MAX_FILTER_VALUES,
        };
        let existing = request_query_values_from(&query, &param);
        let values = normalize_filter_values(&existing, &normalize_options);
        let mut next_values: Vec<InputBytes> = Vec::with_capacity(values.len() + 1);
        let mut removed = false;
        for existing in &values {
            if *existing == value {
                removed = true;
                continue;
            }
            next_values.push(existing.clone());
        }
        if !removed {
            next_values.push(value);
        }
        let next_values = normalize_filter_values(&next_values, &normalize_options);
        if next_values.is_empty() {
            query.remove(param.as_bytes());
        } else {
            query.insert(param.into(), next_values);
        }
    }
    if options.reset_page {
        query.remove(PAGE_QUERY_PARAM.as_bytes());
    }
    append_encoded_query(&base, &query)
}

/// Build tag filter chips.
#[must_use]
pub fn build_resource_tag_chips(options: &ResourceTagChipsOptions) -> Vec<ResourceFilterChipVM> {
    let tags = normalize_filter_values(
        &options.tags,
        &FilterValueOptions {
            sort: true,
            lowercase: false,
            max_values: 0,
        },
    );
    let selected = normalize_filter_values(
        &options.selected_tags,
        &FilterValueOptions {
            sort: true,
            lowercase: false,
            max_values: DEFAULT_MAX_FILTER_VALUES,
        },
    );
    let selected_set: std::collections::BTreeSet<InputBytes> = selected.iter().cloned().collect();
    let mut chips: Vec<ResourceFilterChipVM> = Vec::with_capacity(tags.len());
    for tag in tags {
        let active = selected_set.contains(&tag);
        let (label, aria_label) = match &options.label {
            Some(resolve) => resolve(tag.as_bytes(), active),
            None => (
                format!("#{}", crate::byte_text::encoding_text(tag.as_bytes())),
                String::new(),
            ),
        };
        let href = resource_filter_toggle_url(&FilterToggleURLOptions {
            request: options.request.clone(),
            base_path: options.base_path.clone(),
            definition: options.filter.clone(),
            value: tag.clone(),
            reset_page: true,
            sort_values: true,
        });
        let remove_href = if active { href.clone() } else { String::new() };
        chips.push(ResourceFilterChipVM {
            key: options.filter.key.clone(),
            value: tag,
            label,
            aria_label,
            href,
            active,
            remove_href,
        });
    }
    chips
}

/// Values stored for one parameter in an already parsed query map.
fn request_query_values_from(query: &QueryValues, key: &str) -> Vec<InputBytes> {
    query.get(key.as_bytes()).cloned().unwrap_or_default()
}
