//! The SEO service over the registered resource definitions.
//!
//! The service owns page, sitemap-document and sitemap-index production. The
//! definition registry is a required constructor capability; the translation port,
//! the canonical-route path builder and the three validators are caller-supplied
//! optional behavior stored in [`Options`].
//!
//! # Operation rules
//!
//! - [`Service::page`] resolves the definition, derives indexability for the surface,
//!   translates the list title and description keys, builds the canonical route and
//!   includes the page in the sitemap only when the definition's sitemap is enabled
//!   and the page is indexable.
//! - [`Service::sitemap`] resolves the definition and builds its document under the
//!   requested kind.
//! - [`Service::sitemap_index`] builds the index across every registered definition in
//!   registration order.
//! - An unknown kind is the registry's own lookup error in [`SEOError::UnknownKind`].
//! - A validator receives the exact value that is returned and runs once; its error is
//!   returned unchanged in [`SEOError::Validation`].
//!
//! # Effects and capabilities
//!
//! Every operation returns a lazy [`Effect`](effect::Effect) under the caller's error
//! family and environment. Nothing runs when the description is constructed. The
//! service owns no store, database backend, SQL statement, schema, migration, pool or
//! connection.

use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;

use effect::{Effect, effect_try_sync};
use effect::memory::{Buffer, BufferView, SlotPool};
use effect_server::Request;

use crate::definitions::{
    RESOURCE_VIEW_MODE_DETAIL, RESOURCE_VIEW_MODE_LIST, RESOURCE_VIEW_MODE_PUBLIC, Registry,
    ResourceDefinition, ResourceKind, ResourceViewMode, UnknownResourceKind,
};
use crate::localization::TranslateKeyFunc;
use crate::routing::CanonicalRoutePathBuilder;

use super::documents::{
    SitemapDocumentVM, SitemapIndexVM, SitemapWriteError, definition_sitemap_index,
    sitemap_document_for_kind, write_sitemap_index,
};
use super::props::SEOVM;
use super::publication::{Prepared, publish};

/// Validates one published page value.
pub type SEOValidator<E> =
    Arc<dyn Fn(&mut SEOVM) -> Result<(), E> + Send + Sync + 'static>;

/// Validates one published sitemap-document value.
pub type SitemapDocumentValidator<E> =
    Arc<dyn Fn(&mut SitemapDocumentVM) -> Result<(), E> + Send + Sync + 'static>;

/// Validates one published sitemap-index value.
pub type SitemapIndexValidator<E> =
    Arc<dyn Fn(&mut SitemapIndexVM) -> Result<(), E> + Send + Sync + 'static>;

/// Configures the SEO service.
///
/// Every field is optional behavior: an absent translation port keeps the trimmed key
/// as the text, an absent canonical-route builder keeps the canonical route empty, and
/// an absent validator publishes the prepared value unchanged.
pub struct Options<E> {
    /// Resolves one translated key over the request context.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
    /// Builds the canonical route path for a definition and surface.
    pub canonical_route_path: Option<CanonicalRoutePathBuilder>,
    /// Validates the published page value.
    pub validate_seo: Option<SEOValidator<E>>,
    /// Validates the published sitemap-document value.
    pub validate_sitemap_document: Option<SitemapDocumentValidator<E>>,
    /// Validates the published sitemap-index value.
    pub validate_sitemap_index: Option<SitemapIndexValidator<E>>,
}

impl<E> Default for Options<E> {
    fn default() -> Self {
        Self {
            translate_key: None,
            canonical_route_path: None,
            validate_seo: None,
            validate_sitemap_document: None,
            validate_sitemap_index: None,
        }
    }
}

impl<E> Clone for Options<E> {
    fn clone(&self) -> Self {
        Self {
            translate_key: self.translate_key.clone(),
            canonical_route_path: self.canonical_route_path.clone(),
            validate_seo: self.validate_seo.clone(),
            validate_sitemap_document: self.validate_sitemap_document.clone(),
            validate_sitemap_index: self.validate_sitemap_index.clone(),
        }
    }
}

/// A CMS-owned SEO service failure.
///
/// The two variants keep the registry's lookup error and the caller's validator error
/// distinct and unchanged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SEOError<E> {
    /// The requested kind is not registered.
    UnknownKind(UnknownResourceKind),
    /// A supplied validator rejected the prepared value.
    Validation(E),
}

impl<E> fmt::Display for SEOError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKind(cause) => fmt::Display::fmt(cause, formatter),
            Self::Validation(cause) => fmt::Display::fmt(cause, formatter),
        }
    }
}

impl<E> std::error::Error for SEOError<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnknownKind(cause) => Some(cause),
            Self::Validation(cause) => Some(cause),
        }
    }
}

/// The SEO service.
///
/// The definition registry is a required constructor input; there is no absent-
/// registry branch, late setter or runtime lookup.
pub struct Service<E>
where
    E: Send + 'static,
{
    resources: Arc<dyn Registry + Send + Sync>,
    options: Options<E>,
}

impl<E> Clone for Service<E>
where
    E: Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            resources: Arc::clone(&self.resources),
            options: self.options.clone(),
        }
    }
}

impl<E> Service<E>
where
    E: Send + 'static,
{
    /// Construct the SEO service.
    #[must_use]
    pub fn new(resources: Arc<dyn Registry + Send + Sync>, options: Options<E>) -> Self {
        Self { resources, options }
    }

    /// Produce the page SEO value for a request, kind and surface.
    ///
    /// The returned effect is lazy; the registry lookup, translation and canonical
    /// route callbacks run when it runs. The page validator receives the returned
    /// value and runs at most once.
    #[must_use]
    pub fn page<R>(
        &self,
        request: &Request,
        kind: &ResourceKind,
        mode: &ResourceViewMode,
    ) -> Effect<SEOVM, SEOError<E>, R>
    where
        R: Send + Sync + 'static,
    {
        let resources = Arc::clone(&self.resources);
        let options = self.options.clone();
        let request = request.clone();
        let kind = kind.clone();
        let mode = mode.clone();
        effect_try_sync! {
            body {
                let definition = resources
                    .definition(&kind)
                    .map_err(SEOError::UnknownKind)?;
                let value = page_view_model(&definition, &kind, &mode, &request, &options);
                publish(Prepared {
                    value,
                    validator: options.validate_seo.clone(),
                })
                .map_err(SEOError::Validation)
            },
        }
    }

    /// Produce the sitemap document for one kind.
    ///
    /// The returned effect is lazy; the registry lookup runs when it runs, and the
    /// document validator receives the returned value and runs at most once.
    #[must_use]
    pub fn sitemap<R>(&self, kind: &ResourceKind) -> Effect<SitemapDocumentVM, SEOError<E>, R>
    where
        R: Send + Sync + 'static,
    {
        let resources = Arc::clone(&self.resources);
        let options = self.options.clone();
        let kind = kind.clone();
        effect_try_sync! {
            body {
                let definition = resources
                    .definition(&kind)
                    .map_err(SEOError::UnknownKind)?;
                let value = sitemap_document_for_kind(&definition, &kind);
                publish(Prepared {
                    value,
                    validator: options.validate_sitemap_document.clone(),
                })
                .map_err(SEOError::Validation)
            },
        }
    }

    /// Produce the sitemap index across the registered definitions.
    ///
    /// The returned effect is lazy; the definitions are read and the index validator
    /// runs when it runs. The index validator receives the returned value and runs at
    /// most once.
    #[must_use]
    pub fn sitemap_index<R>(&self) -> Effect<SitemapIndexVM, SEOError<E>, R>
    where
        R: Send + Sync + 'static,
    {
        let resources = Arc::clone(&self.resources);
        let options = self.options.clone();
        effect_try_sync! {
            body {
                let definitions_list = resources.definitions();
                let value = definition_sitemap_index(&definitions_list);
                publish(Prepared {
                    value,
                    validator: options.validate_sitemap_index.clone(),
                })
                .map_err(SEOError::Validation)
            },
        }
    }

    /// Serialize the sitemap index into a checked-out prepared output buffer.
    ///
    /// The mutable owned-index validator remains available through `sitemap_index`.
    /// This writer rejects that configuration before admission or registry work.
    pub fn sitemap_index_prepared<R: Send + Sync + 'static>(
        &self,
        outputs: Arc<SlotPool<'static, Buffer<'static>>>,
        maximum_entries: NonZeroUsize,
    ) -> Effect<BufferView<'static>, SitemapWriteError, R> {
        let resources = self.resources.clone();
        let has_owned_validator = self.options.validate_sitemap_index.is_some();
        effect_try_sync! {
            body {
                if has_owned_validator {
                    return Err(SitemapWriteError::OwnedIndexValidator);
                }
                let mut output = outputs.try_checkout().map_err(SitemapWriteError::Memory)?;
                output.clear();
                write_sitemap_index(resources.as_ref(), maximum_entries, &mut output)?;
                Ok(BufferView::new(output.share()))
            },
        }
    }
}

/// Construct the SEO service.
#[must_use]
pub fn new_service<E>(resources: Arc<dyn Registry + Send + Sync>, options: Options<E>) -> Service<E>
where
    E: Send + 'static,
{
    Service::new(resources, options)
}

/// Whether a definition makes a surface indexable.
///
/// The list and public surfaces consult the definition's list indexability; the
/// detail surface consults its detail indexability; every other surface is not
/// indexable.
#[must_use]
pub fn seo_indexable(definition: &ResourceDefinition, mode: &ResourceViewMode) -> bool {
    match mode.as_str() {
        RESOURCE_VIEW_MODE_LIST | RESOURCE_VIEW_MODE_PUBLIC => definition.seo.indexable_list,
        RESOURCE_VIEW_MODE_DETAIL => definition.seo.indexable_detail,
        _ => false,
    }
}

/// Build the page SEO value for one definition and surface.
fn page_view_model<E>(
    definition: &ResourceDefinition,
    kind: &ResourceKind,
    mode: &ResourceViewMode,
    request: &Request,
    options: &Options<E>,
) -> SEOVM {
    let indexable = seo_indexable(definition, mode);
    SEOVM {
        kind: kind.clone(),
        mode: mode.clone(),
        page_type: definition.seo.page_type.clone(),
        title: translate_key(
            request,
            options.translate_key.as_ref(),
            &definition.seo.list_title_key,
        ),
        description: translate_key(
            request,
            options.translate_key.as_ref(),
            &definition.seo.list_description_key,
        ),
        canonical_route: canonical_route_path(
            request,
            options.canonical_route_path.as_ref(),
            definition,
            mode,
        ),
        indexable,
        included_in_sitemap: definition.sitemap.enabled && indexable,
    }
}

/// Resolve one translated key, falling back to the trimmed key.
fn translate_key(
    request: &Request,
    translate: Option<&TranslateKeyFunc<Request>>,
    key: &str,
) -> String {
    match translate {
        Some(translate) => translate(request, key),
        None => key.trim().to_owned(),
    }
}

/// Resolve the canonical route path, falling back to the empty path.
fn canonical_route_path(
    request: &Request,
    build: Option<&CanonicalRoutePathBuilder>,
    definition: &ResourceDefinition,
    mode: &ResourceViewMode,
) -> String {
    match build {
        Some(build) => build(request, definition, mode),
        None => String::new(),
    }
}
