//! The W5 form page service and localized form status.
//!
//! The service resolves the registered definition, asks the command owner for the
//! actions offered on the requested surface, applies the consumer's validation-path
//! catalog to the issues, and projects the page view model. It then calls the
//! consumer's page validator, if supplied, before publishing the page.
//!
//! The surface constructors preserve their result defaults: `new` carries the
//! create action with the valid code, `edit` carries update with the valid code, and
//! every other surface carries the decision's action with the valid code through the
//! same build path. `rehydrate` rebuilds from the supplied validation result.
//!
//! The definition lookup and the command owner's action availability stay lazy in
//! the returned [`Effect`]: no callback runs when the description is constructed.
//! A forged or replayed private stage is unrepresentable here because the page is
//! built in one owner-private sequence and no stage value is published.
//!
//! Consumer callbacks keep their own error family in [`FormPageError::Consumer`];
//! the definitions registry's lookup error keeps its own variant. Defects,
//! interruption and finalization failures keep their own effect channels.
//!
//! The owner supplies no HTML in this module; the localized status and page values
//! are data for the rendering owners.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use effect::{Effect, effect, effect_map_error, effect_try_sync};
use effect_server::Request;

use crate::commands::CommandService;
use crate::definitions::{
    RESOURCE_ACTION_CREATE, RESOURCE_ACTION_UPDATE, RESOURCE_VIEW_MODE_EDIT,
    RESOURCE_VIEW_MODE_NEW, Registry, ResourceDefinition, ResourceFormDefinition, ResourceKind,
    ResourceRouteSet, ResourceViewMode, UnknownResourceKind,
};
use crate::localization::TranslateKeyFunc;
use crate::routing::{
    ManagementBreadcrumbBuilder, ManagementBreadcrumbVM, RouteSetForRequestResolver,
};
use crate::validation::{
    Catalog, LookupOptions, ValidationLocalizationOptions, ValidationResult,
    ValidationTranslation, clone_validation_issue, localize_validation_issues,
    success_validation_result, validation_summary_message,
};

use super::mode::{decide_mode, filter_submit_actions};
use super::sections::{FormStatusVM, ResourceFormPageVM};

/// Resolves the localized title for a definition and surface.
pub type ResourceModeTitleResolver = Arc<
    dyn Fn(&Request, &ResourceDefinition, &ResourceViewMode) -> String + Send + Sync + 'static,
>;

/// Validates and may normalize a page before it is published.
///
/// The validator receives the exact mutable page value that the service returns, so
/// its normalization is observable on that same value. It runs once, and its error is
/// returned unchanged as [`FormPageError::Consumer`].
pub type FormPageValidator<E> =
    Arc<dyn Fn(&mut ResourceFormPageVM) -> Result<(), E> + Send + Sync + 'static>;

/// Configures the form page workflow.
///
/// Every callback and the path catalog are genuinely optional consumer data in the
/// original workflow; an absent title/breadcrumb/routes resolver yields the owner's
/// empty value and an absent translator leaves the label keys in place.
pub struct Options<C, E> {
    /// Resolves the localized page title.
    pub resource_mode_title: Option<ResourceModeTitleResolver>,
    /// Resolves the management breadcrumb.
    pub breadcrumb: Option<ManagementBreadcrumbBuilder>,
    /// Resolves the route set for the request.
    pub route_set_for_request: Option<RouteSetForRequestResolver>,
    /// The consumer's field-label translation port.
    pub translate_key: Option<TranslateKeyFunc<C>>,
    /// Applies the consumer's validation-path bindings.
    pub validation_path_catalog: Catalog,
    /// Validates the completed page.
    pub validate_page: Option<FormPageValidator<E>>,
}

impl<C, E> Clone for Options<C, E> {
    fn clone(&self) -> Self {
        Self {
            resource_mode_title: self.resource_mode_title.clone(),
            breadcrumb: self.breadcrumb.clone(),
            route_set_for_request: self.route_set_for_request.clone(),
            translate_key: self.translate_key.clone(),
            validation_path_catalog: self.validation_path_catalog.clone(),
            validate_page: self.validate_page.clone(),
        }
    }
}

impl<C, E> Default for Options<C, E> {
    fn default() -> Self {
        Self {
            resource_mode_title: None,
            breadcrumb: None,
            route_set_for_request: None,
            translate_key: None,
            validation_path_catalog: Catalog::default(),
            validate_page: None,
        }
    }
}

/// A form page failure.
///
/// The registry's lookup error and the consumer's page-validator error keep their
/// own typed identity; neither is flattened into a message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormPageError<E> {
    /// The requested kind is not registered.
    Definition(UnknownResourceKind),
    /// The consumer's page validator refused the page.
    Consumer(E),
}

impl<E: fmt::Display> fmt::Display for FormPageError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Definition(error) => fmt::Display::fmt(error, formatter),
            Self::Consumer(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl<E: Error + 'static> Error for FormPageError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Definition(error) => Some(error),
            Self::Consumer(error) => Some(error),
        }
    }
}

/// Produces form page view models.
pub struct Service<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    resources: Arc<dyn Registry + Send + Sync>,
    commands: CommandService<E, R>,
    options: Options<C, E>,
}

impl<C, E, R> Clone for Service<C, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            resources: Arc::clone(&self.resources),
            commands: self.commands.clone(),
            options: self.options.clone(),
        }
    }
}

impl<C, E, R> Service<C, E, R>
where
    C: ValidationTranslation + Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Construct the form page service.
    ///
    /// The definition registry and the command service are required typed
    /// capabilities; there is no absent-registry, setter or runtime lookup branch.
    #[must_use]
    pub fn new(
        resources: Arc<dyn Registry + Send + Sync>,
        commands: CommandService<E, R>,
        options: Options<C, E>,
    ) -> Self {
        Self {
            resources,
            commands,
            options,
        }
    }

    /// Build a new-form page.
    #[must_use]
    pub fn new_page(
        &self,
        context: Arc<C>,
        request: &Request,
        kind: &ResourceKind,
    ) -> Effect<ResourceFormPageVM, FormPageError<E>, R> {
        self.build_page(
            context,
            request,
            kind,
            &ResourceViewMode::new(RESOURCE_VIEW_MODE_NEW),
            success_validation_result(crate::definitions::ResourceAction::new(RESOURCE_ACTION_CREATE)),
        )
    }

    /// Build an edit-form page.
    #[must_use]
    pub fn edit_page(
        &self,
        context: Arc<C>,
        request: &Request,
        kind: &ResourceKind,
    ) -> Effect<ResourceFormPageVM, FormPageError<E>, R> {
        self.build_page(
            context,
            request,
            kind,
            &ResourceViewMode::new(RESOURCE_VIEW_MODE_EDIT),
            success_validation_result(crate::definitions::ResourceAction::new(RESOURCE_ACTION_UPDATE)),
        )
    }

    /// Rebuild the page from a validation outcome.
    #[must_use]
    pub fn rehydrate(
        &self,
        context: Arc<C>,
        request: &Request,
        kind: &ResourceKind,
        mode: &ResourceViewMode,
        result: ValidationResult,
    ) -> Effect<ResourceFormPageVM, FormPageError<E>, R> {
        self.build_page(context, request, kind, mode, result)
    }

    /// Build the page for a surface.
    #[must_use]
    pub fn page(
        &self,
        context: Arc<C>,
        request: &Request,
        kind: &ResourceKind,
        mode: &ResourceViewMode,
    ) -> Effect<ResourceFormPageVM, FormPageError<E>, R> {
        match mode.as_str() {
            RESOURCE_VIEW_MODE_NEW => self.new_page(context, request, kind),
            RESOURCE_VIEW_MODE_EDIT => self.edit_page(context, request, kind),
            _ => {
                let decision = decide_mode(mode);
                self.build_page(
                    context,
                    request,
                    kind,
                    mode,
                    success_validation_result(decision.action),
                )
            }
        }
    }

    fn build_page(
        &self,
        context: Arc<C>,
        request: &Request,
        kind: &ResourceKind,
        mode: &ResourceViewMode,
        result: ValidationResult,
    ) -> Effect<ResourceFormPageVM, FormPageError<E>, R> {
        let resources = Arc::clone(&self.resources);
        let commands = self.commands.clone();
        let options = self.options.clone();
        let request = request.clone();
        let kind = kind.clone();
        let mode = mode.clone();
        let context = Arc::clone(&context);
        effect! {
            let request = request.clone();
            let kind = kind.clone();
            let mode = mode.clone();
            let result = result.clone();
            let context = Arc::clone(&context);
            let resources = Arc::clone(&resources);
            let commands = commands.clone();
            let options = options.clone();
            let kind_for_definition = kind.clone();
            let definition = effect_try_sync! {
                once;
                body: move || {
                    resources.definition(&kind_for_definition).map_err(FormPageError::Definition)
                },
            }?;
            let actions_effect = commands.actions(&request, &kind);
            let actions = effect_map_error! {
                program: actions_effect,
                map |error| { FormPageError::Definition(error) },
            }?;
            let page = effect_try_sync! {
                once;
                body: move || {
                    let lookup = LookupOptions {
                        resource: kind.clone(),
                        action: result.action.clone(),
                    };
                    let mut effective = result;
                    effective.issues = options
                        .validation_path_catalog
                        .apply_all(&effective.issues, &lookup);
                    let title = match options.resource_mode_title.as_ref() {
                        Some(resolve) => resolve(&request, &definition, &mode),
                        None => String::new(),
                    };
                    let breadcrumb = match options.breadcrumb.as_ref() {
                        Some(resolve) => resolve(&request, &definition, &mode),
                        None => ManagementBreadcrumbVM::default(),
                    };
                    let routes = match options.route_set_for_request.as_ref() {
                        Some(resolve) => resolve(&request, &definition),
                        None => ResourceRouteSet::default(),
                    };
                    let widgets = definition.widgets.to_vec();
                    let status = localized_form_status(
                        &*context,
                        &effective,
                        &definition.form,
                        options.translate_key.clone(),
                    );
                    let actions = filter_submit_actions(actions, &mode);
                    let mut page = ResourceFormPageVM {
                        kind,
                        mode,
                        title,
                        breadcrumb,
                        routes,
                        form_id: crate::commands::InputBytes::default(),
                        action_href: String::new(),
                        cancel_href: String::new(),
                        actions,
                        widgets,
                        status,
                        sections: Vec::new(),
                    };
                    if let Some(validate) = options.validate_page.as_ref() {
                        validate(&mut page).map_err(FormPageError::Consumer)?;
                    }
                    Ok::<ResourceFormPageVM, FormPageError<E>>(page)
                },
            }?;
            page
        }
    }
}

/// Build the localized form status projection.
///
/// The issue collection is copied deeply, the summary message is the localized
/// summary whenever issues exist, and otherwise the trimmed result message is kept.
/// The issue-localization options carry only the action, the definition and the
/// label port, exactly as the workflow projects them.
#[must_use]
pub fn localized_form_status<C: ValidationTranslation>(
    context: &C,
    result: &ValidationResult,
    definition: &ResourceFormDefinition,
    translate_key: Option<TranslateKeyFunc<C>>,
) -> FormStatusVM {
    let mut issues = Vec::new();
    if !result.issues.is_empty() {
        issues = result
            .issues
            .iter()
            .map(|issue| clone_validation_issue(issue.clone()))
            .collect();
    }
    let options = ValidationLocalizationOptions {
        resource: ResourceKind::default(),
        action: result.action.clone(),
        definition: definition.clone(),
        translate_key,
    };
    let localized = localize_validation_issues(context, &issues, &options);
    let mut message = result.message.trim().to_owned();
    if !issues.is_empty() {
        message = validation_summary_message(context, &issues, &options);
    }
    FormStatusVM {
        code: result.code.clone(),
        message,
        submit_disabled: result.submit_disabled,
        requires_rehydrate: result.requires_rehydrate,
        issues: localized,
    }
}

/// Construct the form page service.
#[must_use]
pub fn new_service<C, E, R>(
    resources: Arc<dyn Registry + Send + Sync>,
    commands: CommandService<E, R>,
    options: Options<C, E>,
) -> Service<C, E, R>
where
    C: ValidationTranslation + Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Service::new(resources, commands, options)
}
