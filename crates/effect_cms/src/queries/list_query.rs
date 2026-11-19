//! List-query admission, request-value reading and filter application.
//!
//! List admission owns the admitted [`ResourceListQuery`], the named-page/language
//! callback types, the pagination and filter defaults, the request query/path
//! readers, filter value normalization and the URL helpers.
//!
//! # Branches preserved
//!
//! - An unknown kind returns the default page (`1`), the configured
//!   page size and empty filter maps.
//! - A page-size option below `1` resolves to `1`, exactly as the
//!   `pageSize` helper does; the page itself is whatever the caller's named-page
//!   parser returns and is not clamped.
//! - Search and sort are read once from the request query parameters named by the
//!   constants; each declared filter then assigns its own value,
//!   defaulting to the declared default when the request carries none.
//! - A multi-value filter normalizes (trim, dedupe, sort, cap at
//!   [`DEFAULT_MAX_FILTER_VALUES`]) and falls back to the declared default only when
//!   the normalized request values are empty; a single-value filter keeps the value
//!   untrimmed beyond the request reader's trim.
//! - Every definition lookup is observed at call time through the shared
//!   collections of the definition model, so an in-place mutation the caller makes
//!   through the registry is what this admission reads. The filter declarations
//!   are borrowed and the request query is decoded once.
//!
//! # Request-query reader
//!
//! The request query reader applies these observable rules locally: an empty segment
//! is skipped; a segment containing `;` is skipped; the key and value split at the
//! first `=`; `+` decodes to a space and `%XX` sequences decode as bytes; a
//! malformed escape skips that segment while decoded non-UTF-8 bytes remain intact.
//! The native partial byte decoder supplies this shared rule; exceeding the source
//! 10,000 encoded-segment limit yields an empty query. Keys and admitted search, sort
//! and filter values use `InputBytes`, including URL re-encoding and callback paths.
//!
//! # Effects
//!
//! The operations are pure request reads over caller-supplied
//! definitions, so the admitted value is computed by a pure function and lifted at
//! the owner boundary by [`ListService::list_query`]. No capability, store, backend
//! or database is held here.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::commands::InputBytes;
use effect::memory::{Bytes, MemoryError};
use effect::{Effect, effect_sync, effect_try_sync};
use effect_server::{Request, ServerError};
use std::num::NonZeroUsize;

use crate::forms::{PreparedMergedForm, PreparedMergedForms};

use crate::definitions::{
    RESOURCE_FILTER_MODE_MULTI_ALL, RESOURCE_FILTER_MODE_SINGLE, RESOURCE_FILTER_SEARCH,
    RESOURCE_FILTER_SORT, Registry, ResourceFilterDefinition, ResourceFilterKey,
    ResourceFilterMode, ResourceFilterValue, ResourceKind,
};

/// The page query parameter name.
pub const PAGE_QUERY_PARAM: &str = "page";

/// The search query parameter name.
pub const SEARCH_QUERY_PARAM: &str = "search";

/// The sort query parameter name.
pub const SORT_QUERY_PARAM: &str = "sort";

/// Caps repeated filter values.
pub const DEFAULT_MAX_FILTER_VALUES: i64 = 12;

/// Repeated query values keyed by parameter name.
pub type QueryValues = BTreeMap<InputBytes, Vec<InputBytes>>;

/// One admitted list query.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceListQuery {
    /// Admitted page number.
    pub page: i64,
    /// Effective page size.
    pub page_size: i64,
    /// Admitted search value.
    pub search: InputBytes,
    /// Admitted sort value.
    pub sort: InputBytes,
    /// First filter value per filter key.
    pub filters: BTreeMap<ResourceFilterKey, InputBytes>,
    /// Every filter value per filter key.
    pub filter_values: BTreeMap<ResourceFilterKey, Vec<InputBytes>>,
}

impl ResourceListQuery {
    /// The first value for a filter key.
    ///
    /// A trimmed single value wins; otherwise the first of the filter's values is
    /// returned, and an absent key yields the empty string.
    #[must_use]
    pub fn filter_value(&self, key: &ResourceFilterKey) -> InputBytes {
        if let Some(value) = self.filters.get(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
        self.values_for_filter(key)
            .first()
            .cloned()
            .unwrap_or_default()
    }

    /// A detached copy of the filter values for a key.
    ///
    /// When no values are stored, a trimmed single value is returned as one value;
    /// otherwise the result is empty.
    #[must_use]
    pub fn values_for_filter(&self, key: &ResourceFilterKey) -> Vec<InputBytes> {
        match self.filter_values.get(key) {
            Some(values) if !values.is_empty() => values.clone(),
            _ => match self.filters.get(key) {
                Some(value) if !value.trim().is_empty() => vec![value.trim().to_owned()],
                _ => Vec::new(),
            },
        }
    }

    /// Whether the query carries a value for the key.
    #[must_use]
    pub fn has_filter(&self, key: &ResourceFilterKey) -> bool {
        !self.values_for_filter(key).is_empty()
    }

    /// Whether the query deviates from the unfiltered first page.
    #[must_use]
    pub fn filtered(&self) -> bool {
        if self.page > 1 || !self.search.trim().is_empty() || !self.sort.trim().is_empty() {
            return true;
        }
        for value in self.filters.values() {
            if !value.trim().is_empty() {
                return true;
            }
        }
        for values in self.filter_values.values() {
            for value in values {
                if !value.trim().is_empty() {
                    return true;
                }
            }
        }
        false
    }
}

/// Configures filter value normalization.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FilterValueOptions {
    /// Sort the normalized values.
    pub sort: bool,
    /// Lowercase each value before deduplicating.
    pub lowercase: bool,
    /// Cap the number of values; a non-positive value means no cap.
    pub max_values: i64,
}

/// Parses a named page parameter for one request.
pub type ParseNamedPageFunc = Arc<dyn Fn(&Request, &str) -> i64 + Send + Sync + 'static>;

/// Resolves the request language.
pub type RequestLanguageFunc = Arc<dyn Fn(&Request) -> InputBytes + Send + Sync + 'static>;

/// Names the canonical detail route parameters.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CanonicalRouteParams {
    /// Identifier path parameter name.
    pub id_param: String,
    /// Slug path parameter name.
    pub slug_param: String,
    /// Parent identifier path parameter name.
    pub parent_param: String,
    /// Parent name path parameter name.
    pub parent_name_param: String,
}

/// Configures list-query admission.
#[derive(Clone, Default)]
pub struct ListOptions {
    /// Configured default page size.
    pub default_page_size: i64,
    /// Optional named-page parser.
    pub parse_named_page: Option<ParseNamedPageFunc>,
    /// Optional request-language resolver.
    ///
    /// The resolver is declared in this option set but the list
    /// admission does not read it; the field is preserved so the option contract
    /// stays complete.
    pub request_language: Option<RequestLanguageFunc>,
}

/// Selected limits for projected list-query data.
#[derive(Clone, Copy)]
pub struct PreparedListLimits {
    /// Largest configured page size admitted to a consumer.
    pub page_size: NonZeroUsize,
    /// Maximum declared filters examined per list request.
    pub filters: NonZeroUsize,
    /// Maximum bytes in the request path and encoded query.
    pub request: Bytes,
    /// Maximum bytes of filter key and default metadata examined per request.
    pub filter_metadata: Bytes,
}

/// Admits list queries against the registered definitions.
#[derive(Clone)]
pub struct ListService {
    resources: Arc<dyn Registry + Send + Sync>,
    options: ListOptions,
}

impl ListService {
    /// Construct the list-query admission service.
    ///
    /// The definition registry is a required capability: there is no `Option`, no
    /// late setter and no runtime lookup for it.
    #[must_use]
    pub fn new(resources: Arc<dyn Registry + Send + Sync>, options: ListOptions) -> Self {
        Self { resources, options }
    }

    /// Admit the list query for a request and kind.
    ///
    /// The operation is lifted as a lazy effect under the caller's error family and
    /// environment. It performs no I/O, so it cannot fail or interrupt; the
    /// environment is carried unchanged for the surrounding workflow.
    pub fn list_query<E, R>(
        &self,
        request: &Request,
        kind: &ResourceKind,
    ) -> Effect<ResourceListQuery, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        let resources = Arc::clone(&self.resources);
        let options = self.options.clone();
        let request = request.clone();
        let kind = kind.clone();
        effect_sync! { body: move || admit_list_query(resources.as_ref(), &options, &request, &kind), }
    }

    /// Decode and admit the query using an existing prepared form workspace.
    /// Limits are checked before named-page parsing. The returned list data is
    /// owned by the caller and bounded by the query and filter capacities.
    pub fn list_query_prepared<R: Send + Sync + 'static>(
        &self,
        request: &Request,
        kind: &ResourceKind,
        storage: Arc<PreparedMergedForms>,
        limits: PreparedListLimits,
    ) -> Effect<ResourceListQuery, ServerError, R> {
        let service = self.clone();
        let request = request.clone();
        let kind = kind.clone();
        effect_try_sync! {
            body: move || {
                check_request_bytes(&request, limits.request)?;
                let fields = storage.read_query(&request)?;
                service.admit_prepared(&request, &kind, &fields, limits)
            },
        }
    }

    pub(super) fn admit_prepared(
        &self,
        request: &Request,
        kind: &ResourceKind,
        fields: &PreparedMergedForm,
        limits: PreparedListLimits,
    ) -> Result<ResourceListQuery, ServerError> {
        check_request_bytes(request, limits.request)?;
        check_count(
            usize::try_from(page_size(&self.options))
                .map_err(|_| MemoryError::ArithmeticOverflow)
                .map_err(ServerError::Memory)?,
            limits.page_size.get(),
        )?;
        let mut query = empty_query(&self.options);
        let mut result = Ok(());
        let _ = self.resources.visit_definition(kind, &mut |definition| {
            result = (|| {
                definition
                    .filters
                    .try_with(|filters| check_filters(filters, limits))
                    .map_err(ServerError::Memory)??;
                if let Some(parse) = &self.options.parse_named_page {
                    query.page = parse(request, PAGE_QUERY_PARAM);
                }
                definition
                    .filters
                    .try_with(|filters| {
                        check_filters(filters, limits)?;
                        apply_list_values(&mut query, QueryValuesRef::Prepared(fields), filters);
                        Ok(())
                    })
                    .map_err(ServerError::Memory)?
            })();
        });
        result.map(|()| query)
    }
}

pub(crate) fn check_request_bytes(request: &Request, maximum: Bytes) -> Result<(), ServerError> {
    check_capacity(
        request
            .uri()
            .path_and_query()
            .map_or(0, |value| value.as_str().len()),
        maximum.get(),
    )
}

pub(super) fn check_capacity(requested: usize, capacity: usize) -> Result<(), ServerError> {
    if requested > capacity {
        return Err(ServerError::Memory(MemoryError::ValueTooLarge {
            requested: Bytes::new(requested),
            capacity: Bytes::new(capacity),
        }));
    }
    Ok(())
}

fn check_count(requested: usize, capacity: usize) -> Result<(), ServerError> {
    if requested > capacity {
        return Err(ServerError::Memory(MemoryError::NoSlot));
    }
    Ok(())
}

fn check_filters(
    filters: &[ResourceFilterDefinition],
    limits: PreparedListLimits,
) -> Result<(), ServerError> {
    check_count(filters.len(), limits.filters.get())?;
    let mut metadata = Bytes::new(0);
    for filter in filters {
        for value in [
            filter.key.as_str(),
            filter.param.as_str(),
            filter.default_value.as_str(),
        ] {
            metadata = metadata
                .checked_add(Bytes::new(value.len()))
                .map_err(ServerError::Memory)?;
        }
    }
    check_capacity(metadata.get(), limits.filter_metadata.get())
}

/// Construct the list-query admission service.
#[must_use]
pub fn new_list_service(
    resources: Arc<dyn Registry + Send + Sync>,
    options: ListOptions,
) -> ListService {
    ListService::new(resources, options)
}

/// The effective page size for the configured options.
fn page_size(options: &ListOptions) -> i64 {
    if options.default_page_size < 1 {
        1
    } else {
        options.default_page_size
    }
}

/// List admission over one request.
fn admit_list_query(
    resources: &(dyn Registry + Send + Sync),
    options: &ListOptions,
    request: &Request,
    kind: &ResourceKind,
) -> ResourceListQuery {
    let mut query = empty_query(options);
    let _ = resources.visit_definition(kind, &mut |definition| {
        if let Some(parse) = &options.parse_named_page {
            query.page = parse(request, PAGE_QUERY_PARAM);
        }
        let values = request_query_map(request);
        definition.filters.with(|filters| {
            apply_list_values(&mut query, QueryValuesRef::Ordinary(&values), filters);
        });
    });
    query
}

fn empty_query(options: &ListOptions) -> ResourceListQuery {
    ResourceListQuery {
        page: 1,
        page_size: page_size(options),
        search: InputBytes::default(),
        sort: InputBytes::default(),
        filters: BTreeMap::new(),
        filter_values: BTreeMap::new(),
    }
}

#[derive(Clone, Copy)]
pub(super) enum QueryValuesRef<'a> {
    Ordinary(&'a QueryValues),
    Prepared(&'a PreparedMergedForm),
}

impl QueryValuesRef<'_> {
    pub(super) fn trimmed(self, key: &str) -> InputBytes {
        let value = match self {
            Self::Ordinary(values) => values
                .get(key.as_bytes())
                .and_then(|values| values.first())
                .map(InputBytes::as_bytes),
            Self::Prepared(values) => values.query_values(key.as_bytes()).next(),
        };
        value
            .map(crate::validation::trim_unicode_whitespace)
            .map(InputBytes::from)
            .unwrap_or_default()
    }
}

fn apply_list_values(
    query: &mut ResourceListQuery,
    values: QueryValuesRef<'_>,
    filters: &[ResourceFilterDefinition],
) {
    query.search = values.trimmed(SEARCH_QUERY_PARAM);
    query.sort = values.trimmed(SORT_QUERY_PARAM);
    for filter in filters {
        apply_list_filter(query, values, filter);
    }
}

/// Apply one declared filter to the admitted query.
fn apply_list_filter(
    query: &mut ResourceListQuery,
    request: QueryValuesRef<'_>,
    filter: &ResourceFilterDefinition,
) {
    let param = borrowed_filter_query_param(filter);
    if filter.key.as_str() == RESOURCE_FILTER_SEARCH {
        query.search = query_value_or_default(request, param, &filter.default_value);
        return;
    }
    if filter.key.as_str() == RESOURCE_FILTER_SORT {
        query.sort = query_value_or_default(request, param, &filter.default_value);
        return;
    }
    if filter.mode.as_str() == RESOURCE_FILTER_MODE_MULTI_ALL {
        let options = FilterValueOptions {
            sort: true,
            lowercase: false,
            max_values: DEFAULT_MAX_FILTER_VALUES,
        };
        let mut values = match request {
            QueryValuesRef::Ordinary(request) => {
                let requested = request.get(param.as_bytes()).map_or(&[][..], Vec::as_slice);
                normalize_filter_values(requested, &options)
            }
            QueryValuesRef::Prepared(request) => prepared_filter_values(request, param),
        };
        if values.is_empty() && !filter.default_value.as_str().trim().is_empty() {
            values = normalize_filter_values(&[filter.default_value.as_str()], &options);
        }
        if let Some(first) = values.first() {
            query.filters.insert(filter.key.clone(), first.clone());
            query.filter_values.insert(filter.key.clone(), values);
        }
        return;
    }
    let value = query_value_or_default(request, param, &filter.default_value);
    if !value.is_empty() {
        query.filters.insert(filter.key.clone(), value.clone());
        query.filter_values.insert(filter.key.clone(), vec![value]);
    }
}

fn prepared_filter_values(request: &PreparedMergedForm, param: &str) -> Vec<InputBytes> {
    const MAXIMUM: usize = DEFAULT_MAX_FILTER_VALUES as usize;
    let mut smallest: [&[u8]; MAXIMUM] = [&[]; MAXIMUM];
    let mut len = 0;
    for value in request
        .query_values(param.as_bytes())
        .map(crate::validation::trim_unicode_whitespace)
    {
        if value.is_empty() {
            continue;
        }
        let (active, _) = smallest.split_at(len);
        let Err(index) = active.binary_search(&value) else {
            continue;
        };
        if index == MAXIMUM {
            continue;
        }
        if len < MAXIMUM {
            len += 1;
        }
        let (active, _) = smallest.split_at_mut(len);
        let (_, tail) = active.split_at_mut(index);
        tail.rotate_right(1);
        if let Some(first) = tail.first_mut() {
            *first = value;
        }
    }
    let (active, _) = smallest.split_at(len);
    active
        .iter()
        .map(|value| InputBytes::from(*value))
        .collect()
}

/// A trimmed request query value, or the declared default when the request has none.
fn query_value_or_default(
    request: QueryValuesRef<'_>,
    param: &str,
    default: &ResourceFilterValue,
) -> InputBytes {
    let value = request.trimmed(param);
    if value.is_empty() {
        default.as_str().into()
    } else {
        value
    }
}

/// Resolve the query parameter for a filter definition.
#[must_use]
pub fn filter_query_param(definition: &ResourceFilterDefinition) -> String {
    borrowed_filter_query_param(definition).to_owned()
}

fn borrowed_filter_query_param(definition: &ResourceFilterDefinition) -> &str {
    let param = definition.param.trim();
    if param.is_empty() {
        definition.key.as_str()
    } else {
        param
    }
}

/// Resolve the filter mode, defaulting to single.
#[must_use]
pub fn resource_filter_mode(definition: &ResourceFilterDefinition) -> ResourceFilterMode {
    if definition.mode.as_str().is_empty() {
        ResourceFilterMode::new(RESOURCE_FILTER_MODE_SINGLE)
    } else {
        definition.mode.clone()
    }
}

/// Read one trimmed query value.
#[must_use]
pub fn request_query_value(request: &Request, key: &str) -> InputBytes {
    QueryValuesRef::Ordinary(&request_query_map(request)).trimmed(key)
}

/// Read the first untrimmed query value, or the empty string.
///
/// Read the first request query value for a key, which the relation
/// admission uses before parsing a page number.
pub(super) fn request_query_value_untrimmed(request: &Request, key: &str) -> InputBytes {
    parse_query(request.uri().query().unwrap_or_default())
        .get(key.as_bytes())
        .and_then(|values| values.first())
        .cloned()
        .unwrap_or_default()
}

/// Read all query values for a key.
#[must_use]
pub fn request_query_values(request: &Request, key: &str) -> Vec<InputBytes> {
    parse_query(request.uri().query().unwrap_or_default())
        .get(key.as_bytes())
        .cloned()
        .unwrap_or_default()
}

/// Read the request's complete query map, preserving repeated values.
#[must_use]
pub fn request_query_map(request: &Request) -> QueryValues {
    parse_query(request.uri().query().unwrap_or_default())
}

/// Read one trimmed path value.
///
/// The request's captured path value; an absent capture reads as the empty string.
#[must_use]
pub fn path_value(request: &Request, key: &str) -> String {
    effect_server::path_capture(request, key)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// The first trimmed non-empty value.
#[must_use]
pub fn first_non_blank(values: &[String]) -> String {
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    String::new()
}

/// Normalize filter values: trim, optionally lowercase, deduplicate, optionally
/// sort and cap.
#[must_use]
pub fn normalize_filter_values<T: AsRef<[u8]>>(
    values: &[T],
    options: &FilterValueOptions,
) -> Vec<InputBytes> {
    if values.is_empty() {
        return Vec::new();
    }
    let mut seen: BTreeSet<InputBytes> = BTreeSet::new();
    let mut normalized: Vec<InputBytes> = Vec::with_capacity(values.len());
    for value in values {
        let mut value =
            InputBytes::from(crate::validation::trim_unicode_whitespace(value.as_ref()));
        if options.lowercase {
            value = crate::byte_text::encoding_text(value.as_bytes())
                .chars()
                .map(|ch| ch.to_lowercase().next().unwrap_or(ch))
                .collect::<String>()
                .into();
        }
        if value.is_empty() || !seen.insert(value.clone()) {
            continue;
        }
        normalized.push(value);
    }
    if options.sort {
        normalized.sort();
    }
    if options.max_values > 0 {
        let cap = usize::try_from(options.max_values).unwrap_or(usize::MAX);
        normalized.truncate(cap);
    }
    normalized
}

/// Append encoded query values to a base URL.
#[must_use]
pub fn resource_url_with_query(base: &str, query: &QueryValues) -> String {
    append_encoded_query(base, query)
}

/// Append encoded query values to a base URL.
#[must_use]
pub fn append_encoded_query(base: &str, query: &QueryValues) -> String {
    let mut url = base.to_owned();
    let mut separator = if base.contains('?') { '&' } else { '?' };
    for (key, entries) in query {
        for value in entries {
            url.push(separator);
            append_component(key.as_bytes(), &mut url);
            url.push('=');
            append_component(value.as_bytes(), &mut url);
            separator = '&';
        }
    }
    url
}

/// Copy a query value map.
#[must_use]
pub fn clone_query(source: &QueryValues) -> QueryValues {
    source.clone()
}

/// Build a URL that carries one filter value.
#[must_use]
pub fn resource_filter_url(base_url: &str, key: &str, value: impl AsRef<[u8]>) -> String {
    let base_url = base_url.trim();
    let key = key.trim();
    let value = crate::validation::trim_unicode_whitespace(value.as_ref());
    if base_url.is_empty() || key.is_empty() || value.is_empty() {
        return base_url.to_owned();
    }
    let mut values: QueryValues = BTreeMap::new();
    values.insert(key.into(), vec![value.into()]);
    let separator = if base_url.contains('?') { '&' } else { '?' };
    format!("{base_url}{separator}{}", encode_values(&values))
}

/// Parse a URL query with the owner's observable query rules.
///
/// The filter owner reuses this reader for the request query it toggles, so one
/// implementation covers both reads.
pub(super) fn parse_request_query(query: &str) -> QueryValues {
    parse_query(query)
}

/// Parse a URL query with the owner's observable query rules.
fn parse_query(query: &str) -> QueryValues {
    // The source ParseQuery rejects the full query at its segment bound; ordinary
    // malformed segments are diagnosed individually and valid pairs survive.
    if query.as_bytes().split(|byte| *byte == b'&').count() > 10_000 {
        return QueryValues::new();
    }
    effect_server::decode_url_encoded_pairs_partial(query.as_bytes())
        .values
        .into_iter()
        .map(|(key, values)| {
            (
                key.into(),
                values.into_iter().map(InputBytes::from).collect(),
            )
        })
        .collect()
}

/// Encode a query map.
fn encode_values(values: &QueryValues) -> String {
    let mut encoded = String::new();
    for (key, entries) in values {
        for value in entries {
            if !encoded.is_empty() {
                encoded.push('&');
            }
            append_component(key.as_bytes(), &mut encoded);
            encoded.push('=');
            append_component(value.as_bytes(), &mut encoded);
        }
    }
    encoded
}

/// Encode one query component.
fn append_component(value: &[u8], encoded: &mut String) {
    for byte in value.iter().copied() {
        let (bytes, length) = encode_byte(byte);
        let (bytes, _) = bytes.split_at(length);
        encoded.extend(bytes.iter().copied().map(char::from));
    }
}

pub(crate) fn append_url_component_into(
    value: &[u8],
    encoded: &mut effect::memory::Buffer<'_>,
) -> Result<(), MemoryError> {
    for byte in value.iter().copied() {
        let (bytes, length) = encode_byte(byte);
        let (bytes, _) = bytes.split_at(length);
        encoded.extend_from_slice(bytes)?;
    }
    Ok(())
}

fn encode_byte(byte: u8) -> ([u8; 3], usize) {
    match byte {
        b' ' => ([b'+', 0, 0], 1),
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => ([byte, 0, 0], 1),
        other => ([b'%', hex_digit(other / 16), hex_digit(other % 16)], 3),
    }
}

/// The uppercase hexadecimal digit for a nibble.
fn hex_digit(nibble: u8) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        _ => b'A' + nibble - 10,
    }
}
