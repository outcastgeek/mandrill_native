//! Media asset form pages.
//!
//! The form page is assembled through the canonical forms owner. A `GET` edit page
//! loads the asset from the consumer store by the request's `id` path capture and
//! projects it into form input; a submission keeps the submitted input. New requests
//! use the declared media defaults. The form mode prefers the update action, then a
//! non-blank path identity, and otherwise the new surface.

use std::sync::Arc;

use effect::{Effect, effect, effect_sync};
use effect_server::Request;
use maud::Markup;

use crate::definitions::{
    RESOURCE_VIEW_MODE_EDIT, RESOURCE_VIEW_MODE_NEW, Registry, ResourceKind, ResourceViewMode,
};
use crate::forms::{
    FormPageBinding, FormPageSource, ResourceFormPageVM, build_resource_form_page,
};
use crate::localization::TranslateKeyFunc;
use crate::media::assets::MediaAssetFormInput;
use crate::media::assets::{
    default_media_asset_form_input, media_asset_form_input_from_record,
    media_asset_form_input_from_request as parse_media_asset_form_input,
};
use super::media_form::media_asset_resource_form_state;
use crate::queries::path_value;
use crate::routing::RoutePathBuilder;
use crate::validation::ValidationResult;

use crate::media::runtime::MediaAssetStoreRuntime;

use super::bindings::MediaInvalidRequest;

/// Renders a completed form page to markup.
pub type MediaFormRenderPage =
    Arc<dyn Fn(&Request, &ResourceFormPageVM) -> Markup + Send + Sync + 'static>;

/// The media asset form page inputs.
pub struct MediaAssetFormPageOptions<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The request.
    pub request: Request,
    /// The canonical form-page source.
    pub forms: FormPageSource<C, E, R>,
    /// The form translation context.
    pub context: Arc<C>,
    /// The resource registry.
    pub resources: Arc<dyn Registry + Send + Sync>,
    /// The media resource kind.
    pub kind: ResourceKind,
    /// The submitted/default input.
    pub input: MediaAssetFormInput,
    /// The requested surface.
    pub mode: ResourceViewMode,
    /// The edit target parameters.
    pub edit_params: Vec<(String, String)>,
    /// The supplied validation result.
    pub validation: ValidationResult,
    /// Builds route targets.
    pub route_path: Option<RoutePathBuilder>,
    /// The translation port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
}

impl<C, E, R> Clone for MediaAssetFormPageOptions<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            forms: self.forms.clone(),
            context: Arc::clone(&self.context),
            resources: Arc::clone(&self.resources),
            kind: self.kind.clone(),
            input: self.input.clone(),
            mode: self.mode.clone(),
            edit_params: self.edit_params.clone(),
            validation: self.validation.clone(),
            route_path: self.route_path.clone(),
            translate_key: self.translate_key.clone(),
        }
    }
}

/// Configures the store-backed form page builder.
pub struct MediaAssetFormPageBuilderOptions<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The canonical form-page source.
    pub forms: FormPageSource<C, E, R>,
    /// The form translation context.
    pub context: Arc<C>,
    /// The resource registry.
    pub resources: Arc<dyn Registry + Send + Sync>,
    /// The media resource kind.
    pub kind: ResourceKind,
    /// The store-backed runtime.
    pub runtime: MediaAssetStoreRuntime<E, R>,
    /// Builds route targets.
    pub route_path: Option<RoutePathBuilder>,
    /// The translation port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
    /// Optional markup renderer.
    pub render_page: Option<MediaFormRenderPage>,
    /// Builds the configured invalid-request error.
    pub invalid_request_error: MediaInvalidRequest<E>,
    /// The caller's form read policy.
    pub form_policy: crate::attachments::MediaFormPolicy,
}

impl<C, E, R> Clone for MediaAssetFormPageBuilderOptions<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            forms: self.forms.clone(),
            context: Arc::clone(&self.context),
            resources: Arc::clone(&self.resources),
            kind: self.kind.clone(),
            runtime: self.runtime.clone(),
            route_path: self.route_path.clone(),
            translate_key: self.translate_key.clone(),
            render_page: self.render_page.clone(),
            invalid_request_error: self.invalid_request_error.clone(),
            form_policy: self.form_policy,
        }
    }
}

/// Builds one media asset form page for a request and validation result.
pub type MediaFormPageBuilder<E, R> = Arc<
    dyn Fn(&Request, ValidationResult) -> Effect<Option<Markup>, E, R> + Send + Sync + 'static,
>;

/// Construct the store-backed form page builder.
#[must_use]
pub fn new_media_asset_form_page_builder<C, E, R>(
    options: MediaAssetFormPageBuilderOptions<C, E, R>,
) -> MediaFormPageBuilder<E, R>
where
    C: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(move |request, result| {
        let request = request.clone();
        let options = options.clone();
        effect! {
            let request = request.clone();
            let options = options.clone();
            let result = result.clone();
            let (input, mode, edit_params) = load_media_asset_form_input(
                &options.context,
                &request,
                &options.runtime,
                &options.kind,
                &result,
                options.form_policy,
                options.invalid_request_error.clone(),
            )?;
            let page = build_media_asset_form_page(MediaAssetFormPageOptions {
                request: request.clone(),
                forms: options.forms.clone(),
                context: Arc::clone(&options.context),
                resources: Arc::clone(&options.resources),
                kind: options.kind.clone(),
                input,
                mode,
                edit_params,
                validation: result.clone(),
                route_path: options.route_path.clone(),
                translate_key: options.translate_key.clone(),
            })?;
            match options.render_page.as_ref() {
                Some(render) => Some(render(&request, &page)),
                None => None,
            }
        }
    })
}

/// Build the media asset form page.
#[must_use]
pub fn build_media_asset_form_page<C, E, R>(
    options: MediaAssetFormPageOptions<C, E, R>,
) -> Effect<ResourceFormPageVM, E, R>
where
    C: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request = options.request.clone();
    let kind = if options.kind.as_str().is_empty() {
        ResourceKind::new(crate::definitions::RESOURCE_KIND_MEDIA)
    } else {
        options.kind.clone()
    };
    let mode = if options.mode.as_str().is_empty() {
        ResourceViewMode::new(RESOURCE_VIEW_MODE_NEW)
    } else {
        options.mode.clone()
    };
    effect! {
        let request = request.clone();
        let options = options.clone();
        let kind = kind.clone();
        let mode = mode.clone();
        let state_request = request.clone();
        let state_input = options.input.clone();
        let state_translate = options.translate_key.clone();
        let state = effect_sync! {
            body {
                media_asset_resource_form_state(
                    &state_request,
                    &state_input,
                    state_translate.as_ref(),
                )
            }
        }?;
        let binding = FormPageBinding {
            forms: options.forms.clone(),
            resources: Some(Arc::clone(&options.resources)),
            kind,
            mode,
            error_message: String::new(),
            validation: options.validation.clone(),
            route_path: options.route_path.clone(),
            edit_params: options.edit_params.clone(),
            edit_post_route: String::new(),
            new_post_route: String::new(),
            form_id: String::new(),
            state: Some(state),
            translate_key: options.translate_key.clone(),
        };
        let context = Arc::clone(&options.context);
        build_resource_form_page(context, &request, binding)?
    }
}

/// Load the form input, mode and edit parameters for the page.
fn load_media_asset_form_input<C, E, R>(
    context: &Arc<C>,
    request: &Request,
    runtime: &MediaAssetStoreRuntime<E, R>,
    _kind: &ResourceKind,
    result: &ValidationResult,
    form_policy: crate::attachments::MediaFormPolicy,
    invalid_request_error: MediaInvalidRequest<E>,
) -> Effect<(MediaAssetFormInput, ResourceViewMode, Vec<(String, String)>), E, R>
where
    C: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let _ = context;
    let request = request.clone();
    let runtime = runtime.clone();
    let result = result.clone();
    effect! {
        let request = request.clone();
        let runtime = runtime.clone();
        let result = result.clone();
        let invalid_request_error = invalid_request_error.clone();
        let mode = media_asset_form_mode(&request, &result);
        if !request.method().as_str().eq_ignore_ascii_case("GET") {
            let input = parse_media_asset_form_input::<E, R>(
                &request,
                None,
                form_policy,
                move |_error| invalid_request_error("media form read"),
            )?;
            (
                input,
                mode.clone(),
                media_asset_form_edit_params(&request, &mode),
            )
        } else if mode.as_str() != RESOURCE_VIEW_MODE_EDIT {
            (default_media_asset_form_input(), mode, Vec::new())
        } else {
            let id = path_value(&request, "id");
            let store = runtime.assets.clone();
            let loaded = match (id.is_empty(), store) {
                (true, _) | (false, None) => None,
                (false, Some(store)) => store.get_by_id(id.into())?,
            };
            match loaded {
                Some(record) => (
                    media_asset_form_input_from_record(&record),
                    mode.clone(),
                    vec![("id".to_owned(), path_value(&request, "id"))],
                ),
                None => (
                    default_media_asset_form_input(),
                    mode.clone(),
                    media_asset_form_edit_params(&request, &mode),
                ),
            }
        }
    }
}

/// Decide the media form surface.
#[must_use]
pub fn media_asset_form_mode(request: &Request, result: &ValidationResult) -> ResourceViewMode {
    if result.action.as_str() == crate::definitions::RESOURCE_ACTION_UPDATE
        || !path_value(request, "id").trim().is_empty()
    {
        ResourceViewMode::new(RESOURCE_VIEW_MODE_EDIT)
    } else {
        ResourceViewMode::new(RESOURCE_VIEW_MODE_NEW)
    }
}

fn media_asset_form_edit_params(request: &Request, mode: &ResourceViewMode) -> Vec<(String, String)> {
    if mode.as_str() != RESOURCE_VIEW_MODE_EDIT {
        return Vec::new();
    }
    let id = path_value(request, "id");
    if id.is_empty() {
        return Vec::new();
    }
    vec![("id".to_owned(), id)]
}
