//! Detail-query admission.
//!
//! The admitted [`ResourceDetailQuery`] carries the identifier, slug, parent and
//! locale read from the request.
//!
//! # Branches preserved
//!
//! - Path values are read before query values for the identifier and slug; the
//!   first trimmed non-empty value wins.
//! - The parent is the first trimmed non-empty value across the configured parent
//!   identifier and parent-name parameters, each checked as a path value and then
//!   as a query value.
//! - An empty configured parameter name falls back to the owner's declared default
//!   path parameters; a blank configured name never contributes a value.
//! - The locale is the trimmed request-language result and is empty when no
//!   resolver was supplied. The kind argument is part of the service contract and is
//!   not read by admission either.
//!
//! The private admitted/rejected outcome pair and its one-shot
//! bookkeeping enforce a one-shot stage; the admission here is a pure function over
//! the request, so ownership already provides the same non-reuse guarantee and no
//! outcome factory is introduced.

use crate::commands::InputBytes;
use crate::forms::{PreparedMergedForm, PreparedMergedForms};
use effect::memory::Bytes;
use effect::{Effect, effect_sync, effect_try_sync};
use effect_server::{Request, ServerError};
use std::sync::Arc;

use crate::definitions::{RESOURCE_ID_PATH_PARAM, RESOURCE_SLUG_PATH_PARAM, ResourceKind};

use super::list_query::{
    CanonicalRouteParams, QueryValuesRef, RequestLanguageFunc, check_capacity, check_request_bytes,
    path_value, request_query_map,
};

/// One admitted detail query.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceDetailQuery {
    /// Resource identifier.
    pub id: InputBytes,
    /// Resource slug.
    pub slug: InputBytes,
    /// Parent identifier.
    pub parent: InputBytes,
    /// Request locale.
    pub locale: InputBytes,
}

/// Configures detail-query admission.
#[derive(Clone, Default)]
pub struct DetailOptions {
    /// Optional request-language resolver.
    pub request_language: Option<RequestLanguageFunc>,
    /// Canonical detail route parameter names.
    pub detail_params: CanonicalRouteParams,
}

/// Selected metadata limits for detail-query admission.
#[derive(Clone, Copy)]
pub struct PreparedDetailLimits {
    /// Maximum bytes in the request path and encoded query.
    pub request: Bytes,
    /// Maximum bytes retained from the language resolver.
    pub locale: Bytes,
}

/// Admits detail queries.
#[derive(Clone, Default)]
pub struct DetailService {
    options: DetailOptions,
}

impl DetailService {
    /// Construct the detail-query admission service.
    #[must_use]
    pub fn new(options: DetailOptions) -> Self {
        Self { options }
    }

    /// Admit the detail query for a request.
    ///
    /// The operation is lifted as a lazy effect under the caller's error family and
    /// environment. It performs no I/O, so it cannot fail or interrupt.
    pub fn detail_query<E, R>(
        &self,
        request: &Request,
        _kind: &ResourceKind,
    ) -> Effect<ResourceDetailQuery, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        let options = self.options.clone();
        let request = request.clone();
        effect_sync! {
            body: move || {
                let fields = request_query_map(&request);
                let (mut admitted, locale) = admit_detail_query(&options, &request, QueryValuesRef::Ordinary(&fields));
                admitted.locale = locale.trim();
                admitted
            },
        }
    }

    /// Admit the detail query without ordinary query-decoding tables.
    /// The path and query limit is checked before resolving the consumer locale.
    pub fn detail_query_prepared<R: Send + Sync + 'static>(
        &self,
        request: &Request,
        storage: Arc<PreparedMergedForms>,
        limits: PreparedDetailLimits,
    ) -> Effect<ResourceDetailQuery, ServerError, R> {
        let service = self.clone();
        let request = request.clone();
        effect_try_sync! {
            body: move || {
                check_request_bytes(&request, limits.request)?;
                let fields = storage.read_query(&request)?;
                service.admit_prepared(&request, &fields, limits)
            },
        }
    }

    pub(super) fn admit_prepared(
        &self,
        request: &Request,
        fields: &PreparedMergedForm,
        limits: PreparedDetailLimits,
    ) -> Result<ResourceDetailQuery, ServerError> {
        check_request_bytes(request, limits.request)?;
        let (mut admitted, locale) =
            admit_detail_query(&self.options, request, QueryValuesRef::Prepared(fields));
        let locale = crate::validation::trim_unicode_whitespace(locale.as_bytes());
        check_capacity(locale.len(), limits.locale.get())?;
        admitted.locale = InputBytes::from(locale);
        Ok(admitted)
    }
}

/// Construct the detail-query admission service.
#[must_use]
pub fn new_detail_service(options: DetailOptions) -> DetailService {
    DetailService::new(options)
}

/// Detail admission over one request.
fn admit_detail_query(
    options: &DetailOptions,
    request: &Request,
    fields: QueryValuesRef<'_>,
) -> (ResourceDetailQuery, InputBytes) {
    let params = &options.detail_params;
    let id = detail_param_value(
        request,
        fields,
        parameter_or(&params.id_param, RESOURCE_ID_PATH_PARAM),
    );
    let slug = detail_param_value(
        request,
        fields,
        parameter_or(&params.slug_param, RESOURCE_SLUG_PATH_PARAM),
    );
    let parent = [params.parent_param.trim(), params.parent_name_param.trim()]
        .into_iter()
        .filter(|param| !param.is_empty())
        .map(|param| detail_param_value(request, fields, param))
        .find(|value| !value.is_empty())
        .unwrap_or_default();
    let locale = options
        .request_language
        .as_ref()
        .map_or_else(InputBytes::default, |resolve| resolve(request));
    let query = ResourceDetailQuery {
        id,
        slug,
        parent,
        locale: InputBytes::default(),
    };
    (query, locale)
}

fn parameter_or<'a>(parameter: &'a str, default: &'a str) -> &'a str {
    let parameter = parameter.trim();
    if parameter.is_empty() {
        default
    } else {
        parameter
    }
}

fn detail_param_value(request: &Request, fields: QueryValuesRef<'_>, param: &str) -> InputBytes {
    let path = path_value(request, param);
    if path.is_empty() {
        fields.trimmed(param)
    } else {
        path.into()
    }
}
