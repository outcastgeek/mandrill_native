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
use effect::{Effect, effect_sync};
use effect_server::Request;

use crate::definitions::{RESOURCE_ID_PATH_PARAM, RESOURCE_SLUG_PATH_PARAM, ResourceKind};

use super::list_query::{
    CanonicalRouteParams, RequestLanguageFunc, first_non_blank, path_value, request_query_value,
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
        effect_sync! { body: move || admit_detail_query(&options, &request), }
    }
}

/// Construct the detail-query admission service.
#[must_use]
pub fn new_detail_service(options: DetailOptions) -> DetailService {
    DetailService::new(options)
}

/// Detail admission over one request.
fn admit_detail_query(options: &DetailOptions, request: &Request) -> ResourceDetailQuery {
    let params = &options.detail_params;
    let id_param = first_non_blank(&[params.id_param.clone(), RESOURCE_ID_PATH_PARAM.to_owned()]);
    let slug_param = first_non_blank(&[
        params.slug_param.clone(),
        RESOURCE_SLUG_PATH_PARAM.to_owned(),
    ]);
    let id = first_non_blank_bytes(&[
        path_value(request, &id_param).into(),
        request_query_value(request, &id_param),
    ]);
    let slug = first_non_blank_bytes(&[
        path_value(request, &slug_param).into(),
        request_query_value(request, &slug_param),
    ]);
    let parent = detail_param_value(
        request,
        &[
            params.parent_param.clone(),
            params.parent_name_param.clone(),
        ],
    );
    let locale = match &options.request_language {
        Some(resolve) => resolve(request).trim().to_owned(),
        None => InputBytes::default(),
    };
    ResourceDetailQuery {
        id,
        slug,
        parent,
        locale,
    }
}

/// The first path or query value across the supplied parameter names.
fn detail_param_value(request: &Request, params: &[String]) -> InputBytes {
    let mut values: Vec<InputBytes> = Vec::with_capacity(params.len() * 2);
    for param in params {
        let param = param.trim();
        if param.is_empty() {
            continue;
        }
        values.push(path_value(request, param).into());
        values.push(request_query_value(request, param));
    }
    first_non_blank_bytes(&values)
}

fn first_non_blank_bytes(values: &[InputBytes]) -> InputBytes {
    values
        .iter()
        .map(InputBytes::trim)
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}
