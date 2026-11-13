//! The form page source, frame resolution and page assembly.
//!
//! The form page adapter owns the binding between the consumer's form-page source
//! and the assembly of a page:
//!
//! - the source keeps the four bound operations (`new`, `edit`, `rehydrate`, `page`);
//!   its zero value is the disabled source used for the original absent-source
//!   contract, where every operation yields an absent page;
//! - frame resolution picks the surface operation, rebuilds through `rehydrate` when
//!   the validation result is not valid, and keeps an empty frame when the source
//!   reports an expected failure or no page;
//! - the frame validation projects an error message into the invalid-request result
//!   only when no validation result was supplied;
//! - page assembly resolves the action/cancel targets, applies the caller's form id,
//!   and builds and marks the sections when a form state is supplied.
//!
//! The source operations are lazy [`Effect`] values: no source callback runs when the
//! page description is constructed. An expected source failure becomes the empty
//! frame, exactly as the workflow projects it; defects, interruption and
//! finalization failures keep their own effect channels.

use std::sync::Arc;

use effect::{Effect, effect, effect_sync};
use effect_server::Request;

use crate::definitions::{
    Registry, RESOURCE_VIEW_MODE_EDIT, RESOURCE_VIEW_MODE_NEW, ResourceKind, ResourceViewMode,
};
use crate::localization::TranslateKeyFunc;
use crate::routing::{
    RoutePathBuilder, resource_form_action_href, resource_form_cancel_href,
};
use crate::validation::{
    CODE_VALID, CODE_INVALID_REQUEST, ValidationResult, invalid_validation_result,
};

use super::sections::{
    ResourceFormPageVM, ResourceFormState, attach_validation_issues_to_form_sections,
    build_resource_form_sections_for_kind,
};

/// Builds a new-form page.
pub type NewPageFunc<C, E, R> = Arc<
    dyn Fn(Arc<C>, &Request, &ResourceKind) -> Effect<Option<ResourceFormPageVM>, E, R>
        + Send
        + Sync
        + 'static,
>;

/// Builds an edit-form page.
pub type EditPageFunc<C, E, R> = NewPageFunc<C, E, R>;

/// Rebuilds a page from a validation result.
pub type RehydratePageFunc<C, E, R> = Arc<
    dyn Fn(
            Arc<C>,
            &Request,
            &ResourceKind,
            &ResourceViewMode,
            &ValidationResult,
        ) -> Effect<Option<ResourceFormPageVM>, E, R>
        + Send
        + Sync
        + 'static,
>;

/// Builds a page for a surface.
pub type PageFunc<C, E, R> = Arc<
    dyn Fn(
            Arc<C>,
            &Request,
            &ResourceKind,
            &ResourceViewMode,
        ) -> Effect<Option<ResourceFormPageVM>, E, R>
        + Send
        + Sync
        + 'static,
>;

/// The complete bound form-page capability.
///
/// The zero value is the disabled source: every operation yields an absent page,
/// which the frame keeps as an empty frame.
pub struct FormPageSource<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    new_page: Option<NewPageFunc<C, E, R>>,
    edit_page: Option<EditPageFunc<C, E, R>>,
    rehydrate: Option<RehydratePageFunc<C, E, R>>,
    page: Option<PageFunc<C, E, R>>,
}

impl<C, E, R> Default for FormPageSource<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            new_page: None,
            edit_page: None,
            rehydrate: None,
            page: None,
        }
    }
}

impl<C, E, R> Clone for FormPageSource<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            new_page: self.new_page.clone(),
            edit_page: self.edit_page.clone(),
            rehydrate: self.rehydrate.clone(),
            page: self.page.clone(),
        }
    }
}

impl<C, E, R> FormPageSource<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Build a new-form page.
    #[must_use]
    pub fn new_page(
        &self,
        context: Arc<C>,
        request: &Request,
        kind: &ResourceKind,
    ) -> Effect<Option<ResourceFormPageVM>, E, R> {
        match self.new_page.as_ref() {
            Some(build) => build(context, request, kind),
            None => disabled_page::<C, E, R>(),
        }
    }

    /// Build an edit-form page.
    #[must_use]
    pub fn edit_page(
        &self,
        context: Arc<C>,
        request: &Request,
        kind: &ResourceKind,
    ) -> Effect<Option<ResourceFormPageVM>, E, R> {
        match self.edit_page.as_ref() {
            Some(build) => build(context, request, kind),
            None => disabled_page::<C, E, R>(),
        }
    }

    /// Rebuild a page from a validation result.
    #[must_use]
    pub fn rehydrate(
        &self,
        context: Arc<C>,
        request: &Request,
        kind: &ResourceKind,
        mode: &ResourceViewMode,
        result: &ValidationResult,
    ) -> Effect<Option<ResourceFormPageVM>, E, R> {
        match self.rehydrate.as_ref() {
            Some(build) => build(context, request, kind, mode, result),
            None => disabled_page::<C, E, R>(),
        }
    }

    /// Build a page for a surface.
    #[must_use]
    pub fn page(
        &self,
        context: Arc<C>,
        request: &Request,
        kind: &ResourceKind,
        mode: &ResourceViewMode,
    ) -> Effect<Option<ResourceFormPageVM>, E, R> {
        match self.page.as_ref() {
            Some(build) => build(context, request, kind, mode),
            None => disabled_page::<C, E, R>(),
        }
    }
}

/// Bind the four form-page operations.
#[must_use]
pub fn bind_form_page_source<C, E, R>(
    new_page: Option<NewPageFunc<C, E, R>>,
    edit_page: Option<EditPageFunc<C, E, R>>,
    rehydrate: Option<RehydratePageFunc<C, E, R>>,
    page: Option<PageFunc<C, E, R>>,
) -> FormPageSource<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    FormPageSource {
        new_page,
        edit_page,
        rehydrate,
        page,
    }
}

/// The owner-bound capabilities and the original page inputs.
pub struct FormPageBinding<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The bound form-page source.
    pub forms: FormPageSource<C, E, R>,
    /// The definition registry used for section building.
    pub resources: Option<Arc<dyn Registry + Send + Sync>>,
    /// The requested kind.
    pub kind: ResourceKind,
    /// The requested surface.
    pub mode: ResourceViewMode,
    /// An error message used to build an invalid result when none was supplied.
    pub error_message: String,
    /// The supplied validation result.
    pub validation: ValidationResult,
    /// Builds route targets.
    pub route_path: Option<RoutePathBuilder>,
    /// The edit target parameters.
    pub edit_params: Vec<(String, String)>,
    /// The fallback edit-post route name.
    pub edit_post_route: String,
    /// The fallback new-post route name.
    pub new_post_route: String,
    /// The caller-supplied form element id.
    pub form_id: String,
    /// The form field state, when sections are built.
    pub state: Option<ResourceFormState>,
    /// The consumer's field-label translation port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
}

impl<C, E, R> Clone for FormPageBinding<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            forms: self.forms.clone(),
            resources: self.resources.clone(),
            kind: self.kind.clone(),
            mode: self.mode.clone(),
            error_message: self.error_message.clone(),
            validation: self.validation.clone(),
            route_path: self.route_path.clone(),
            edit_params: self.edit_params.clone(),
            edit_post_route: self.edit_post_route.clone(),
            new_post_route: self.new_post_route.clone(),
            form_id: self.form_id.clone(),
            state: self.state.clone(),
            translate_key: self.translate_key.clone(),
        }
    }
}

/// The preserved form-page frame projection.
#[derive(Clone, Debug, Default)]
pub struct ResourceFormFrame {
    /// The resolved page, the empty page when absent.
    pub page_vm: ResourceFormPageVM,
}

/// Resolve the form frame from a validation result.
///
/// The new and edit surfaces use their own operation; every other surface uses the
/// general operation. A non-valid, non-empty result code rebuilds through
/// `rehydrate`. An expected failure or an absent page yields the empty frame.
pub fn resolve_resource_form_frame_with_validation<C, E, R>(
    context: Arc<C>,
    request: &Request,
    source: &FormPageSource<C, E, R>,
    kind: &ResourceKind,
    mode: &ResourceViewMode,
    result: &ValidationResult,
) -> Effect<ResourceFormFrame, E, R>
where
    C: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let source = source.clone();
    let request = request.clone();
    let kind = kind.clone();
    let mode = mode.clone();
    let result = result.clone();
    effect! {
        let context = Arc::clone(&context);
        let request = request.clone();
        let kind = kind.clone();
        let mode = mode.clone();
        let result = result.clone();
        let source = source.clone();
        let selected = match mode.as_str() {
            RESOURCE_VIEW_MODE_NEW => {
                source.new_page(Arc::clone(&context), &request, &kind).either()?
            }
            RESOURCE_VIEW_MODE_EDIT => {
                source.edit_page(Arc::clone(&context), &request, &kind).either()?
            }
            _ => source
                .page(Arc::clone(&context), &request, &kind, &mode)
                .either()?,
        };
        let mut page = selected.ok().flatten();
        if !result.code.as_str().is_empty() && result.code.as_str() != CODE_VALID {
            let selection = source
                .rehydrate(context, &request, &kind, &mode, &result)
                .either()?;
            page = selection.ok().flatten();
        }
        ResourceFormFrame {
            page_vm: page.unwrap_or_default(),
        }
    }
}

/// Resolve the form frame from an error message.
pub fn resolve_resource_form_frame<C, E, R>(
    context: Arc<C>,
    request: &Request,
    source: &FormPageSource<C, E, R>,
    kind: &ResourceKind,
    mode: &ResourceViewMode,
    error_message: &str,
) -> Effect<ResourceFormFrame, E, R>
where
    C: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    resolve_resource_form_frame_with_validation(
        context,
        request,
        source,
        kind,
        mode,
        &resource_form_frame_validation(mode, error_message, &ValidationResult::default()),
    )
}

/// Project the frame validation result.
///
/// A supplied result code wins unchanged. Without one, a blank message yields the
/// empty result; otherwise the surface's action and the invalid-request code build
/// the preserved invalid-form projection.
#[must_use]
pub fn resource_form_frame_validation(
    mode: &ResourceViewMode,
    error_message: &str,
    result: &ValidationResult,
) -> ValidationResult {
    if !result.code.as_str().is_empty() {
        return result.clone();
    }
    if error_message.trim().is_empty() {
        return ValidationResult::default();
    }
    let action = if mode.as_str() == RESOURCE_VIEW_MODE_EDIT {
        crate::definitions::ResourceAction::new(crate::definitions::RESOURCE_ACTION_UPDATE)
    } else {
        crate::definitions::ResourceAction::new(crate::definitions::RESOURCE_ACTION_CREATE)
    };
    invalid_validation_result(action, CODE_INVALID_REQUEST.into(), error_message)
}

/// Build the complete form page.
///
/// The page is published even when the frame is empty: the target and cancel
/// addresses and the caller's form id are still applied, and the sections are built
/// and marked only when a form state is supplied.
pub fn build_resource_form_page<C, E, R>(
    context: Arc<C>,
    request: &Request,
    binding: FormPageBinding<C, E, R>,
) -> Effect<ResourceFormPageVM, E, R>
where
    C: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let FormPageBinding {
        forms,
        resources,
        kind,
        mode,
        error_message,
        validation,
        route_path,
        edit_params,
        edit_post_route,
        new_post_route,
        form_id,
        state,
        translate_key,
    } = binding;
    let request = request.clone();
    effect! {
        let context = Arc::clone(&context);
        let request = request.clone();
        let kind = kind.clone();
        let mode = mode.clone();
        let forms = forms.clone();
        let route_path = route_path.clone();
        let edit_params = edit_params.clone();
        let edit_post_route = edit_post_route.clone();
        let new_post_route = new_post_route.clone();
        let form_id = form_id.clone();
        let state = state.clone();
        let resources = resources.clone();
        let translate_key = translate_key.clone();
        let result = resource_form_frame_validation(&mode, &error_message, &validation);
        let frame = resolve_resource_form_frame_with_validation(
            context,
            &request,
            &forms,
            &kind,
            &mode,
            &result,
        )?;
        let mut page = frame.page_vm.clone();
        page.action_href = resource_form_action_href(
            &request,
            &page.routes,
            mode.as_str() == RESOURCE_VIEW_MODE_EDIT,
            route_path.as_ref(),
            &edit_params,
            &edit_post_route,
            &new_post_route,
        );
        page.cancel_href =
            resource_form_cancel_href(&request, &page.routes, route_path.as_ref());
        let form_id = form_id.trim();
        if !form_id.is_empty() {
            page.form_id = form_id.to_owned().into();
        }
        if let Some(state) = state.as_ref() {
            page.sections = build_resource_form_sections_for_kind(
                &request,
                resources.as_deref(),
                &kind,
                state,
                translate_key.as_ref(),
            );
            page.sections = attach_validation_issues_to_form_sections(
                page.sections,
                &page.status.issues,
            );
        }
        page
    }
}

fn disabled_page<C, E, R>() -> Effect<Option<ResourceFormPageVM>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    effect_sync! { body: || { None } }
}
