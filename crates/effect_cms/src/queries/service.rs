//! The query admission service.
//!
//! One aggregate admits list queries and detail queries from the same configuration.
//! Construction builds the list service with the pagination/language options and the
//! detail service with the language/canonical-parameter options; that split and
//! order are preserved.
//!
//! The definition registry is a required constructor capability. The service holds
//! no store, database, backend, pool or schema: consumers own all data access and
//! the queries owner performs none.

use std::sync::Arc;

use effect::Effect;
use effect_server::{Request, ServerError};

use crate::definitions::{Registry, ResourceKind};
use crate::forms::{PreparedMergedForm, PreparedMergedForms};

use super::detail_query::{
    DetailOptions, DetailService, PreparedDetailLimits, ResourceDetailQuery,
};
use super::list_query::{
    CanonicalRouteParams, ListOptions, ListService, ParseNamedPageFunc, PreparedListLimits,
    RequestLanguageFunc, ResourceListQuery,
};

/// Configures query admission.
#[derive(Clone, Default)]
pub struct Options {
    /// Configured default page size.
    pub default_page_size: i64,
    /// Optional named-page parser.
    pub parse_named_page: Option<ParseNamedPageFunc>,
    /// Optional request-language resolver.
    pub request_language: Option<RequestLanguageFunc>,
    /// Canonical detail route parameter names.
    pub detail_params: CanonicalRouteParams,
}

/// The query admission service.
#[derive(Clone)]
pub struct QueryService {
    list: ListService,
    detail: DetailService,
}

impl QueryService {
    /// Admit the list query for a request and kind.
    ///
    /// The returned effect is lazy and carries the caller's error family and
    /// environment unchanged; admission performs no I/O.
    pub fn list_query<E, R>(
        &self,
        request: &Request,
        kind: &ResourceKind,
    ) -> Effect<ResourceListQuery, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        self.list.list_query(request, kind)
    }

    /// Admit a list query with prepared decoding and explicit page and projection limits.
    pub fn list_query_prepared<R: Send + Sync + 'static>(
        &self,
        request: &Request,
        kind: &ResourceKind,
        storage: Arc<PreparedMergedForms>,
        limits: PreparedListLimits,
    ) -> Effect<ResourceListQuery, ServerError, R> {
        self.list
            .list_query_prepared(request, kind, storage, limits)
    }

    /// Admit the detail query for a request and kind.
    ///
    /// The returned effect is lazy and carries the caller's error family and
    /// environment unchanged; admission performs no I/O.
    pub fn detail_query<E, R>(
        &self,
        request: &Request,
        kind: &ResourceKind,
    ) -> Effect<ResourceDetailQuery, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        self.detail.detail_query(request, kind)
    }

    /// Admit a detail query with prepared decoding and explicit metadata limits.
    pub fn detail_query_prepared<R: Send + Sync + 'static>(
        &self,
        request: &Request,
        storage: Arc<PreparedMergedForms>,
        limits: PreparedDetailLimits,
    ) -> Effect<ResourceDetailQuery, ServerError, R> {
        self.detail.detail_query_prepared(request, storage, limits)
    }

    pub(crate) fn admit_prepared(
        &self,
        request: &Request,
        kind: &ResourceKind,
        fields: &PreparedMergedForm,
        list: PreparedListLimits,
        detail: PreparedDetailLimits,
    ) -> Result<(ResourceListQuery, ResourceDetailQuery), ServerError> {
        let list = self.list.admit_prepared(request, kind, fields, list)?;
        let detail = self.detail.admit_prepared(request, fields, detail)?;
        Ok((list, detail))
    }
}

/// Construct the query admission service.
///
/// The definition registry is required; there is no absent-registry, setter or
/// runtime-lookup branch.
#[must_use]
pub fn new_query_service(
    resources: Arc<dyn Registry + Send + Sync>,
    options: Options,
) -> QueryService {
    QueryService {
        list: ListService::new(
            resources,
            ListOptions {
                default_page_size: options.default_page_size,
                parse_named_page: options.parse_named_page,
                request_language: options.request_language.clone(),
            },
        ),
        detail: DetailService::new(DetailOptions {
            request_language: options.request_language,
            detail_params: options.detail_params,
        }),
    }
}
