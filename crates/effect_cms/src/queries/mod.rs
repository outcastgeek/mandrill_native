//! Queries: list/detail admission, filter/search values and relation lookup.
//!
//! Queries owns:
//!
//! - list-query admission against the registered definitions: page/page size,
//!   search/sort, repeated multi-value filters, single-value filters, declared
//!   defaults and the admitted query record;
//! - detail-query admission with path-before-query parameter precedence and the
//!   canonical identifier/slug/parent parameter names;
//! - filter query parameters, filter modes, filter value normalization, URL
//!   building/toggling and tag filter chips (data only; presentation owners render
//!   them);
//! - search token normalization;
//! - the scoped relation lookup: typed target selections, constructor-bound scope
//!   admission and lookup capabilities, observation validation and the closed
//!   publication outcomes (source workflow W19).
//!
//! # API
//!
//! - [`new_query_service`] builds the [`QueryService`] over a required definition
//!   registry, and [`new_list_service`]/[`new_detail_service`] build the two
//!   admission services composed from the same options.
//! - [`new_relation_lookup_binding`]/[`new_relation_lookup_service`] construct the
//!   scoped relation lookup from the consumer's required capabilities;
//!   [`observe_relation_lookup`] and the two stop constructors are the only way a
//!   consumer returns an observation.
//! - The pure helpers keep the owner's names and behavior; the service methods
//!   lift them as lazy [`Effect`] values under the caller's error family and
//!   environment.
//!
//! # Capabilities and data access
//!
//! Admission itself performs no I/O: it reads the request's query/path values and
//! the caller-supplied definitions. All related data and scope access belongs to the
//! consuming application through [`RelationScopeAdmissionFunc`] and
//! [`RelationLookupFunc`]; both are required constructor arguments of the binding,
//! are stored in its private value and are invoked by this owner. The CMS owns no
//! database backend, SQL adapter, schema, migration, pool or connection.
//!
//! # Effects
//!
//! Effectful operations use the workspace `Effect<A, E, R>` model. Admission is a
//! pure calculation lifted with `Effect::sync`; the relation workflow sequences the
//! consumer capabilities with `effect!` and maps an expected capability failure into
//! the rejected/failed outcomes while defects, interruption and cleanup
//! failures keep their own effect channels. No macro-driven side effect is executed
//! when a description is constructed.
//!
//! # Private implementation
//!
//! `list_query`, `detail_query`, `filters`, `search_tokens`, `service` and
//! `relation` are private implementation modules of this owner: they hold the
//! mechanics, while this module is the owner's deliberate public surface.

use effect::Effect;

use crate::definitions::ResourceKind;
use effect_server::Request;

mod detail_query;
mod filters;
mod list_query;
mod relation;
mod search_tokens;
mod service;

pub use self::detail_query::{
    DetailOptions, DetailService, ResourceDetailQuery, new_detail_service,
};
pub use self::filters::{
    FilterToggleURLOptions, ResourceFilterChipLabelFunc, ResourceFilterChipVM,
    ResourceTagChipsOptions, build_resource_tag_chips, resource_filter_toggle_url,
};
pub use self::list_query::{
    CanonicalRouteParams, DEFAULT_MAX_FILTER_VALUES, FilterValueOptions, ListOptions, ListService,
    PAGE_QUERY_PARAM, ParseNamedPageFunc, QueryValues, RequestLanguageFunc, ResourceListQuery,
    SEARCH_QUERY_PARAM, SORT_QUERY_PARAM, append_encoded_query, clone_query, filter_query_param,
    first_non_blank, new_list_service, normalize_filter_values, path_value, request_query_value,
    request_query_map, request_query_values, resource_filter_mode, resource_filter_url,
    resource_url_with_query,
};
pub use self::relation::{
    RELATION_LOOKUP_CHOICES, RELATION_LOOKUP_INVERSE, RelationContractError, RelationKeyValue,
    RelationLookupBinding, RelationLookupCause, RelationLookupEmpty, RelationLookupFailed,
    RelationLookupDecodeError, RelationLookupFound, RelationLookupFunc, RelationLookupInput, RelationLookupMode,
    RelationLookupObservation, RelationLookupOptions, RelationLookupOutcome, RelationLookupRecord,
    RelationLookupRejected, RelationLookupResult, RelationLookupService, RelationLookupUnavailable,
    RelationQuery, RelationScopeAdmissionFunc, RelationSelection, new_relation_bool_key,
    new_relation_int_key, new_relation_lookup_binding, new_relation_lookup_record,
    new_relation_lookup_service, new_relation_selection, new_relation_string_key,
    observe_relation_lookup, observe_relation_lookup_failed, observe_relation_lookup_unavailable,
    parse_relation_selection,
};
pub use self::search_tokens::{
    DEFAULT_MAX_SEARCH_TOKENS, SearchTokenOptions, search_tokens, search_tokens_with_options,
};
pub use self::service::{Options, QueryService, new_query_service};

/// Compile-time witness that the admission service keeps the owner's effect shape.
///
/// The methods are usable for any caller error family and environment, and their
/// results stay lazy [`Effect`] values.
const _: fn() = || {
    fn witness<E, R>()
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        #[allow(clippy::type_complexity)]
        let _: fn(
            &QueryService,
            &Request,
            &ResourceKind,
        ) -> Effect<ResourceListQuery, E, R> = QueryService::list_query::<E, R>;
        #[allow(clippy::type_complexity)]
        let _: fn(
            &QueryService,
            &Request,
            &ResourceKind,
        ) -> Effect<ResourceDetailQuery, E, R> = QueryService::detail_query::<E, R>;
        #[allow(clippy::type_complexity)]
        let _: fn(
            &ListService,
            &Request,
            &ResourceKind,
        ) -> Effect<ResourceListQuery, E, R> = ListService::list_query::<E, R>;
        #[allow(clippy::type_complexity)]
        let _: fn(
            &DetailService,
            &Request,
            &ResourceKind,
        ) -> Effect<ResourceDetailQuery, E, R> = DetailService::detail_query::<E, R>;
    }
    witness::<std::fmt::Error, ()>();
};

/// Compile-time witness that the relation lookup keeps its consumer capability
/// contract and its lazy outcome.
const _: fn() = || {
    fn witness<E, R>()
    where
        E: std::fmt::Display + Send + 'static,
        R: Send + Sync + 'static,
    {
        #[allow(clippy::type_complexity)]
        let _: fn(
            &RelationLookupService<E, R>,
            RelationLookupInput<E>,
        ) -> Effect<Option<RelationLookupOutcome<E>>, E, R> = RelationLookupService::lookup;
    }
    witness::<std::fmt::Error, ()>();
};
