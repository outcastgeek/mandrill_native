//! Media forms: media asset form inputs, defaults and the media form page/frame.
//!
//! This owner supplies the media-specific form layer over the canonical forms
//! service:
//!
//! - request/default form input parsing (kind, source kind, URL, provider, title,
//!   description, thumbnail, alt, optional positive duration and status);
//! - conversion from a stored asset record and the default new-asset input;
//! - the declared field state and selectable kind/source/status options with the
//!   source normalization and label fallbacks;
//! - the media form frame: the canonical form page, requested/fallback content
//!   identity, owner attachments (supplied override or store lookup) and the
//!   attachment manager props for the surface;
//! - the media form page: action/cancel targets, owner form identity precedence and
//!   canonical section construction from the frame state.
//!
//! # Capabilities and effects
//!
//! The canonical form-page source, the resource registry, the consumer attachment
//! loader and the media runtime are supplied capabilities. Request body decoding is
//! native and lazy under the caller's form policy and error mapping. No media store
//! is constructed, no HTML is authored and no database/SQL/backend exists here.

use crate::commands::InputBytes;
use std::sync::Arc;

use effect::{Effect, effect, effect_sync};
use effect_server::{ByteFormData, Request, ServerError};
use uuid::Uuid;

use crate::attachments::MediaAttachmentFormItem;
use crate::definitions::{
    RESOURCE_VIEW_MODE_EDIT, Registry, ResourceKind, ResourceViewMode,
};
use crate::forms::{
    FormPageSource, ResourceFieldOptionVM, ResourceFormPageVM, ResourceFormState,
    resolve_resource_form_frame_with_validation,
};
use crate::localization::TranslateKeyFunc;
use crate::routing::{RoutePathBuilder, resource_form_action_href, resource_form_cancel_href};
use crate::validation::ValidationResult;

use crate::media::assets::{
    MEDIA_KIND_DOCUMENT, MEDIA_KIND_IMAGE, MEDIA_KIND_VIDEO, MEDIA_SOURCE_KIND_EMBED,
    MEDIA_SOURCE_KIND_REMOTE, MEDIA_STATUS_ARCHIVED, MEDIA_STATUS_DRAFT, MEDIA_STATUS_READY,
    MediaAssetFormInput, MediaAssetRecord, normalize_media_kind, normalize_media_source_kind,
    normalize_media_status,
};
use crate::media::runtime::media_default_surface_mounts;

/// The canonical form-page source used by media pages.
pub type MediaFormSource<C, E, R> = FormPageSource<C, E, R>;

/// Loads one owner's attachment items for the media form.
pub type ResourceMediaAttachmentLoader<E, R> = Arc<
    dyn Fn(&Request, InputBytes, InputBytes) -> Effect<Vec<MediaAttachmentFormItem>, E, R>
        + Send
        + Sync
        + 'static,
>;

/// The media form frame inputs.
pub struct ResourceMediaFormFrameOptions<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The canonical form-page source.
    pub forms: MediaFormSource<C, E, R>,
    /// The form translation context.
    pub context: Arc<C>,
    /// The requested resource kind.
    pub kind: ResourceKind,
    /// The requested surface.
    pub mode: ResourceViewMode,
    /// An error message used when no validation result is supplied.
    pub error_message: String,
    /// The supplied validation result.
    pub validation: ValidationResult,
    /// Fallback content identity.
    pub fallback_content_id: Option<InputBytes>,
    /// The owner type.
    pub owner_type: InputBytes,
    /// The store-backed media runtime.
    pub runtime: crate::media::runtime::MediaAssetStoreRuntime<E, R>,
    /// Supplied attachment items; absent loads from the store.
    pub attachments_override: Option<Vec<MediaAttachmentFormItem>>,
    /// Optional attachment loader replacing the runtime's list.
    pub load_attachments: Option<ResourceMediaAttachmentLoader<E, R>>,
    /// The initialized merged form, when available.
    pub form: Option<Arc<ByteFormData>>,
    /// The caller's form read policy.
    pub form_policy: crate::attachments::MediaFormPolicy,
    /// Maps a form-read failure into the consumer's error family.
    pub on_form_error: Arc<dyn Fn(ServerError) -> E + Send + Sync + 'static>,
}

impl<C, E, R> Clone for ResourceMediaFormFrameOptions<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            forms: self.forms.clone(),
            context: Arc::clone(&self.context),
            kind: self.kind.clone(),
            mode: self.mode.clone(),
            error_message: self.error_message.clone(),
            validation: self.validation.clone(),
            fallback_content_id: self.fallback_content_id.clone(),
            owner_type: self.owner_type.clone(),
            runtime: self.runtime.clone(),
            attachments_override: self.attachments_override.clone(),
            load_attachments: self.load_attachments.clone(),
            form: self.form.clone(),
            form_policy: self.form_policy,
            on_form_error: self.on_form_error.clone(),
        }
    }
}

/// The resolved media form frame.
#[derive(Clone, Debug, Default)]
pub struct ResourceMediaFormFrame {
    /// The canonical form page.
    pub page_vm: ResourceFormPageVM,
    /// The requested or generated content identity.
    pub content_id: InputBytes,
    /// The owner's attachment items.
    pub attachments: Vec<MediaAttachmentFormItem>,
    /// The attachment manager props, when the runtime declares mounts.
    pub manager: Option<crate::attachments::MediaAttachmentManagerProps>,
}

/// The media form page inputs.
pub struct ResourceMediaFormPageOptions<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The frame inputs.
    pub frame: ResourceMediaFormFrameOptions<C, E, R>,
    /// The resource registry used for section building.
    pub resources: Arc<dyn Registry + Send + Sync>,
    /// Builds route targets.
    pub route_path: Option<RoutePathBuilder>,
    /// The edit target parameters.
    pub edit_params: Vec<(String, String)>,
    /// The fallback edit-post route.
    pub edit_post_route: String,
    /// The fallback new-post route.
    pub new_post_route: String,
    /// The owner form identity.
    pub owner_form_id: String,
    /// Builds the canonical form state from the resolved frame.
    pub state: Option<Arc<dyn Fn(&ResourceMediaFormFrame) -> ResourceFormState + Send + Sync>>,
    /// The translation port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
}

impl<C, E, R> Clone for ResourceMediaFormPageOptions<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            frame: self.frame.clone(),
            resources: Arc::clone(&self.resources),
            route_path: self.route_path.clone(),
            edit_params: self.edit_params.clone(),
            edit_post_route: self.edit_post_route.clone(),
            new_post_route: self.new_post_route.clone(),
            owner_form_id: self.owner_form_id.clone(),
            state: self.state.clone(),
            translate_key: self.translate_key.clone(),
        }
    }
}

/// The media form failure remains the consumer's error family.
///
/// A native form-read failure contributes no body values while query values still
/// apply, so the request parser's mapped error never reaches the caller.

/// Parse a stored asset record into form input.
#[must_use]
pub fn media_asset_form_input_from_record(record: &MediaAssetRecord) -> MediaAssetFormInput {
    crate::media::assets::media_asset_form_input_from_record(record)
}

/// The default new-asset form input.
#[must_use]
pub fn default_media_asset_form_input() -> MediaAssetFormInput {
    crate::media::assets::default_media_asset_form_input()
}

/// Parse media asset form input from decoded merged form values.
#[must_use]
pub fn media_asset_form_input_from_form(form: Option<&ByteFormData>) -> MediaAssetFormInput {
    crate::media::assets::media_asset_form_input_from_form(form)
}

/// Parse media asset form input from a request.
///
/// A `GET` request yields the default input. Any other request decodes its merged
/// query and URL-encoded body values under the caller's policy; a body-read failure
/// contributes no body values while query values still apply.
pub fn media_asset_form_input_from_request<E, R>(
    request: &Request,
    form: Option<Arc<ByteFormData>>,
    policy: crate::attachments::MediaFormPolicy,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<MediaAssetFormInput, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    if request.method().as_str().eq_ignore_ascii_case("GET") {
        return effect_sync! { body: || default_media_asset_form_input() };
    }
    crate::media::assets::media_asset_form_input_from_request(request, form, policy, on_error)
}

/// Resolve the requested content identity, generating one when absent.
#[must_use]
pub fn requested_resource_content_id<E: Send + 'static, R: Send + Sync + 'static>(request: Option<&ByteFormData>, fallback: Option<&InputBytes>) -> Effect<InputBytes, E, R> {
    let requested = request
        .and_then(|form| form.values.get(b"content_id".as_slice()))
        .and_then(|values| values.first())
        .map(|value| InputBytes::from(crate::validation::trim_unicode_whitespace(value)))
        .unwrap_or_default();
    if requested.is_empty() {
        ensure_resource_content_id(fallback)
    } else {
        ensure_resource_content_id(Some(&requested))
    }
}

/// Preserve a trimmed content identity, generating one when absent.
#[must_use]
pub fn ensure_resource_content_id<E: Send + 'static, R: Send + Sync + 'static>(raw: Option<&InputBytes>) -> Effect<InputBytes, E, R> {
    let raw = raw.cloned();
    effect_sync! { body: move || {
        if let Some(value) = raw.as_ref() {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
        Uuid::new_v4().to_string().into()
    }}
}

/// Build the media form frame.
pub fn build_resource_media_form_frame<C, E, R>(
    request: &Request,
    options: ResourceMediaFormFrameOptions<C, E, R>,
) -> Effect<ResourceMediaFormFrame, E, R>
where
    C: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request_owner = request.clone();
    effect! {
        let request_owner = request_owner.clone();
        let options = options.clone();
        let result = crate::forms::resource_form_frame_validation(
            &options.mode,
            &options.error_message,
            &options.validation,
        );
        let frame = resolve_resource_form_frame_with_validation(
            Arc::clone(&options.context),
            &request_owner,
            &options.forms,
            &options.kind,
            &options.mode,
            &result,
        )?;
        let form = crate::media::assets::media_request_form::<E, R>(
            &request_owner,
            options.form.clone(),
            options.form_policy,
            options.on_form_error.clone(),
        )?;
        let content_id = requested_resource_content_id(Some(&form), options.fallback_content_id.as_ref())?;
        let mut frame = ResourceMediaFormFrame {
            page_vm: frame.page_vm,
            content_id,
            attachments: Vec::new(),
            manager: None,
        };
        let attachments = load_media_form_attachments(
            request_owner.clone(),
            options.runtime.clone(),
            options.owner_type.clone(),
            frame.content_id.clone(),
            options.attachments_override.clone(),
            options.load_attachments.clone(),
        )?;
        frame.attachments = attachments;
        if options.runtime.mounts_for_scope.is_some() && !options.owner_type.trim().is_empty() {
            let runtime = options.runtime.clone();
            let owner_type = options.owner_type.clone();
            let owner_content_id = frame.content_id.clone();
            frame.manager = runtime.build_attachment_manager(
                &request_owner,
                Some(&form),
                owner_type,
                owner_content_id,
                frame.attachments.clone(),
            );
        }
        frame
    }
}

/// Load the owner's attachments for the media form frame.
fn load_media_form_attachments<E, R>(
    request: Request,
    runtime: crate::media::runtime::MediaAssetStoreRuntime<E, R>,
    owner_type: InputBytes,
    owner_content_id: InputBytes,
    attachments_override: Option<Vec<MediaAttachmentFormItem>>,
    load_attachments: Option<ResourceMediaAttachmentLoader<E, R>>,
) -> Effect<Vec<MediaAttachmentFormItem>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    if let Some(items) = attachments_override {
        return effect_sync! { body { items.clone() } };
    }
    if owner_content_id.is_empty() {
        return effect_sync! { body { Vec::new() } };
    }
    effect! {
        let request = request.clone();
        let runtime = runtime.clone();
        let owner_type = owner_type.clone();
        let owner_content_id = owner_content_id.clone();
        let load_attachments = load_attachments.clone();
        match load_attachments {
            Some(load) => load(&request, owner_type, owner_content_id)?,
            None => runtime.list_attachment_items(&request, owner_type, owner_content_id)?,
        }
    }
}

/// Build the media form page.
pub fn build_resource_media_form_page<C, E, R>(
    request: &Request,
    options: ResourceMediaFormPageOptions<C, E, R>,
) -> Effect<ResourceFormPageVM, E, R>
where
    C: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request_owner = request.clone();
    effect! {
        let request_owner = request_owner.clone();
        let options = options.clone();
        let frame = build_resource_media_form_frame(&request_owner, options.frame.clone())?;
        let mut page = frame.page_vm.clone();
        page.action_href = resource_form_action_href(
            &request_owner,
            &page.routes,
            options.frame.mode.as_str() == RESOURCE_VIEW_MODE_EDIT,
            options.route_path.as_ref(),
            &options.edit_params,
            &options.edit_post_route,
            &options.new_post_route,
        );
        page.cancel_href = resource_form_cancel_href(
            &request_owner,
            &page.routes,
            options.route_path.as_ref(),
        );
        let mut form_id = InputBytes::from(options.owner_form_id.trim());
        if form_id.is_empty()
            && let Some(manager) = frame.manager.as_ref()
        {
            form_id = manager.mounts.owner_form_id.clone();
        }
        if form_id.is_empty() {
            form_id = media_default_surface_mounts(&options.frame.owner_type).owner_form_id;
        }
        if !form_id.is_empty() {
            page.form_id = form_id;
        }
        if let Some(state) = options.state.as_ref() {
            let canonical_state = state(&frame);
            page.sections = crate::forms::build_resource_form_sections_for_kind(
                &request_owner,
                Some(&*options.resources),
                &options.frame.kind,
                &canonical_state,
                options.translate_key.as_ref(),
            );
            page.sections = crate::forms::attach_validation_issues_to_form_sections(
                page.sections,
                &page.status.issues,
            );
        }
        page
    }
}

/// Build the media asset form state.
#[must_use]
pub fn media_asset_resource_form_state(
    request: &Request,
    input: &MediaAssetFormInput,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> ResourceFormState {
    let input = media_asset_form_input_with_defaults(input);
    let mut state = ResourceFormState::new();
    insert_field(
        &mut state,
        "kind",
        &input.kind,
        media_asset_form_options(
            request,
            translate_key,
            &input.kind,
            &[
                (MEDIA_KIND_IMAGE, "media.kind.image"),
                (MEDIA_KIND_VIDEO, "media.kind.video"),
                (MEDIA_KIND_DOCUMENT, "media.kind.document"),
            ],
        ),
    );
    insert_field(
        &mut state,
        "source_kind",
        &input.source_kind,
        media_asset_form_options(
            request,
            translate_key,
            &input.source_kind,
            &[
                (MEDIA_SOURCE_KIND_REMOTE, "media.source.remote"),
                (MEDIA_SOURCE_KIND_EMBED, "media.source.embed"),
            ],
        ),
    );
    insert_field(&mut state, "url", input.url.trim(), Vec::new());
    insert_field(&mut state, "title", input.title.trim(), Vec::new());
    insert_field(&mut state, "description", input.description.trim(), Vec::new());
    insert_field(&mut state, "thumbnail_url", input.thumbnail_url.trim(), Vec::new());
    insert_field(&mut state, "alt", input.alt.trim(), Vec::new());
    insert_field(&mut state, "duration", &media_asset_duration_value(input.duration.as_ref().map(|value|value.load(std::sync::atomic::Ordering::SeqCst))), Vec::new());
    insert_field(
        &mut state,
        "status",
        &input.status,
        media_asset_form_options(
            request,
            translate_key,
            &input.status,
            &[
                (MEDIA_STATUS_READY, "media.status.ready"),
                (MEDIA_STATUS_DRAFT, "media.status.draft"),
                (MEDIA_STATUS_ARCHIVED, "media.status.archived"),
            ],
        ),
    );
    state
}

fn insert_field(
    state: &mut ResourceFormState,
    key: &str,
    value: impl AsRef<[u8]>,
    options: Vec<ResourceFieldOptionVM>,
) {
    state.insert(
        key.to_owned(),
        crate::forms::ResourceFormFieldState {
            value: crate::commands::InputBytes::from(value.as_ref()),
            options,
            ..crate::forms::ResourceFormFieldState::default()
        },
    );
}

fn media_asset_form_input_with_defaults(input: &MediaAssetFormInput) -> MediaAssetFormInput {
    let mut input = input.clone();
    input.kind = normalize_media_kind(&input.kind);
    input.source_kind = normalize_media_source_kind(&input.source_kind);
    input.status = if input.status.trim().is_empty() {
        MEDIA_STATUS_READY.to_owned()
    } else {
        normalize_media_status(&input.status)
    };
    input
}

fn media_asset_form_options(
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
    current: &str,
    options: &[(&str, &str)],
) -> Vec<ResourceFieldOptionVM> {
    let current = current.trim();
    let mut values = Vec::with_capacity(options.len());
    for (value, label_key) in options {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let mut label = label_key.trim().to_owned();
        if let Some(translate) = translate_key {
            label = translate(request, label_key);
        }
        if label.trim().is_empty() {
            label = value.to_owned();
        }
        values.push(ResourceFieldOptionVM {
            value: value.to_owned(),
            label,
            selected: value == current,
            ..ResourceFieldOptionVM::default()
        });
    }
    values
}

fn media_asset_duration_value(duration: Option<i64>) -> String {
    match duration {
        Some(value) if value > 0 => value.to_string(),
        _ => String::new(),
    }
}
