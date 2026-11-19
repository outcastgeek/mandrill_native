//! Media command and validation bindings.
//!
//! Create and update are generic resource commands over the media runtime: the
//! request form is decoded under the caller's form policy, the canonical assets rules
//! build/prepare the record and the consumer store performs the write. Delete uses
//! the source's direct consumer-store delete path and preserves its error. The
//! validation binding enforces the declared required-URL observation over the
//! admitted command input before any command runs.

use std::sync::Arc;

use effect::{Effect, effect, effect_try_sync};
use effect_server::{ByteFieldBounds, ByteFormPolicy, MultipartPolicy, Request, ServerError};
use std::num::NonZeroUsize;

use crate::commands::{
    CommandInput, CommandInputHandle, MutationResult, ResourceCommandBindings,
    ResourceCommandExecutor,
};
use crate::definitions::{
    RESOURCE_ACTION_CREATE, RESOURCE_ACTION_DELETE, RESOURCE_ACTION_UPDATE, ResourceAction,
    ResourceKind,
};
use crate::media::assets::{MediaAssetFormInput, media_asset_form_input_from_prepared_request, media_asset_form_input_from_request};
use crate::forms::PreparedMergedForms;
use crate::validation::{
    ActionValidator, Bindings, VALIDATION_ISSUE_REQUIRED,
    ValidationError, ValidationIssue, ValidationIssueCode,
};

use crate::media::runtime::MediaAssetStoreRuntime;

/// Builds the configured invalid-request error value with the source's context message.
pub type MediaInvalidRequest<E> = Arc<dyn Fn(&str) -> E + Send + Sync + 'static>;

/// Returns the configured validation error without a command-context suffix.
pub type MediaValidationInvalidRequest<E> = Arc<dyn Fn() -> E + Send + Sync + 'static>;

/// Builds the required-URL validation message.
pub type MediaURLRequiredMessage = Arc<dyn Fn(&Request) -> String + Send + Sync + 'static>;

/// Configures media command bindings.
pub struct MediaAssetCommandBindingOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The store-backed runtime.
    pub runtime: MediaAssetStoreRuntime<E, R>,
    /// The media resource kind.
    pub kind: ResourceKind,
    /// The result redirect route.
    pub redirect_route: String,
    /// The invalid-request error factory.
    pub invalid_request_error: MediaInvalidRequest<E>,
    /// Initialized merged form, when provided by the caller.
    pub form: Option<Arc<effect_server::ByteFormData>>,
    /// The caller's form read policy.
    pub form_policy: crate::attachments::MediaFormPolicy,
}

impl<E, R> Clone for MediaAssetCommandBindingOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            kind: self.kind.clone(),
            redirect_route: self.redirect_route.clone(),
            invalid_request_error: self.invalid_request_error.clone(),
            form: self.form.clone(),
            form_policy: self.form_policy,
        }
    }
}

/// Configures media validation bindings.
pub struct MediaAssetValidationBindingOptions<E> {
    /// The media resource kind.
    pub kind: ResourceKind,
    /// The supplied required-URL message resolver.
    pub url_required_message: Option<MediaURLRequiredMessage>,
    /// Returns the configured base missing-identity error unchanged.
    pub invalid_request_error: MediaValidationInvalidRequest<E>,
    /// Initialized merged form, when supplied by the caller.
    pub form: Option<Arc<effect_server::ByteFormData>>,
    /// Injects a validation issue into the caller's error family.
    pub validation_error: Arc<dyn Fn(ValidationError) -> E + Send + Sync>,
    /// Maps native form read errors into the caller's error family.
    pub on_form_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
    /// Shared URL-encoded byte policy.
    pub form_policy: ByteFormPolicy,
    /// Multipart body policy.
    pub multipart_policy: MultipartPolicy,
    /// Multipart field count bound.
    pub multipart_fields: NonZeroUsize,
    /// Multipart field bounds.
    pub multipart_values: ByteFieldBounds,
    /// Multipart file-memory allowance.
    pub file_memory: usize,
}

impl<E> Clone for MediaAssetValidationBindingOptions<E> {
    fn clone(&self) -> Self {
        Self { kind: self.kind.clone(), url_required_message: self.url_required_message.clone(), invalid_request_error: self.invalid_request_error.clone(), validation_error: self.validation_error.clone(), form: self.form.clone(), on_form_error: self.on_form_error.clone(), form_policy: self.form_policy, multipart_policy: self.multipart_policy, multipart_fields: self.multipart_fields, multipart_values: self.multipart_values, file_memory: self.file_memory }
    }
}

impl<E> MediaAssetValidationBindingOptions<E> {
    /// The required-URL message for one request.
    #[must_use]
    pub fn message(&self, request: &Request) -> String {
        match self.url_required_message.as_ref() {
            Some(resolve) => resolve(request),
            None => "media.form.validation.url_required".to_owned(),
        }
    }
}

/// Register the create/update/delete executors.
pub fn bind_media_asset_resource_commands<E, R>(
    bindings: &ResourceCommandBindings<E, R>,
    options: MediaAssetCommandBindingOptions<E, R>,
) where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    bind_media_commands(bindings, options, None);
}

/// Bind the existing media executors to the shared prepared form owner.
pub fn bind_prepared_media_asset_resource_commands<E, R>(
    bindings: &ResourceCommandBindings<E, R>,
    options: MediaAssetCommandBindingOptions<E, R>,
    memory: &crate::PreparedCmsMemory,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    bind_media_commands(bindings, options, Some(PreparedMediaInput {
        forms: memory.forms().clone(), on_error: Arc::new(on_error),
    }));
}

struct PreparedMediaInput<E> {
    forms: Arc<PreparedMergedForms>,
    on_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
}
impl<E> Clone for PreparedMediaInput<E> {
    fn clone(&self) -> Self {
        Self { forms: self.forms.clone(), on_error: self.on_error.clone() }
    }
}

fn bind_media_commands<E, R>(
    bindings: &ResourceCommandBindings<E, R>,
    options: MediaAssetCommandBindingOptions<E, R>,
    selected: Option<PreparedMediaInput<E>>,
) where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    bindings.set(
        options.kind.clone(),
        ResourceAction::new(RESOURCE_ACTION_CREATE),
        Some(create_executor(options.clone(), selected.clone())),
    );
    bindings.set(
        options.kind.clone(),
        ResourceAction::new(RESOURCE_ACTION_UPDATE),
        Some(update_executor(options.clone(), selected)),
    );
    bindings.set(
        options.kind.clone(),
        ResourceAction::new(RESOURCE_ACTION_DELETE),
        Some(media_asset_delete_executor(options)),
    );
}

/// Register the create/update validators.
pub fn bind_media_asset_resource_validation<E: Send + 'static, R: Send + Sync + 'static>(
    bindings: &Bindings<CommandInput, E, R>,
    options: MediaAssetValidationBindingOptions<E>,
) {
    bind_media_validation(bindings, options, None);
}

/// Bind the existing media validators to the selected form storage.
pub fn bind_prepared_media_asset_resource_validation<E: Send + 'static, R: Send + Sync + 'static>(
    bindings: &Bindings<CommandInput, E, R>,
    options: MediaAssetValidationBindingOptions<E>,
    memory: &crate::PreparedCmsMemory,
) {
    let prepared = PreparedMediaInput { forms: memory.forms().clone(), on_error: options.on_form_error.clone() };
    bind_media_validation(bindings, options, Some(prepared));
}

fn bind_media_validation<E: Send + 'static, R: Send + Sync + 'static>(
    bindings: &Bindings<CommandInput, E, R>,
    options: MediaAssetValidationBindingOptions<E>,
    selected: Option<PreparedMediaInput<E>>,
) {
    bindings.set(
        options.kind.clone(),
        ResourceAction::new(RESOURCE_ACTION_CREATE),
        Some(create_validator(options.clone(), selected.clone())),
    );
    bindings.set(
        options.kind.clone(),
        ResourceAction::new(RESOURCE_ACTION_UPDATE),
        Some(update_validator(options, selected)),
    );
}

/// The create executor.
#[must_use]
pub fn media_asset_create_executor<E, R>(
    options: MediaAssetCommandBindingOptions<E, R>,
) -> ResourceCommandExecutor<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    create_executor(options, None)
}

fn create_executor<E, R>(
    options: MediaAssetCommandBindingOptions<E, R>,
    selected: Option<PreparedMediaInput<E>>,
) -> ResourceCommandExecutor<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(move |request, _input| {
        let options = options.clone();
        let request = request.clone();
        let selected = selected.clone();
        effect! {
            let options = options.clone();
            let request = request.clone();
            let selected = selected.clone();
            if options.runtime.assets.is_none() {
                let invalid = options.invalid_request_error.clone();
                effect_try_sync! {
                    body: move || -> Result<(), E> { Err(invalid("media store unavailable")) },
                }?;
            }
            let invalid = options.invalid_request_error.clone();
            let form_input = read_media_form::<E, R>(
                &request,
                options.form.clone(),
                options.form_policy,
                selected,
                move |_error| invalid("media form read"),
            )?;
            let _id = options
                .runtime
                .create_asset_from_form(&request, form_input)?;
            mutation_result(
                &options.kind,
                &ResourceAction::new(RESOURCE_ACTION_CREATE),
                &options.redirect_route,
            )
        }
    })
}

/// The update executor.
#[must_use]
pub fn media_asset_update_executor<E, R>(
    options: MediaAssetCommandBindingOptions<E, R>,
) -> ResourceCommandExecutor<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    update_executor(options, None)
}

fn update_executor<E, R>(
    options: MediaAssetCommandBindingOptions<E, R>,
    selected: Option<PreparedMediaInput<E>>,
) -> ResourceCommandExecutor<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(move |request, input| {
        let options = options.clone();
        let request = request.clone();
        let selected = selected.clone();
        effect! {
            let options = options.clone();
            let request = request.clone();
            let input = input.clone();
            let selected = selected.clone();
            if options.runtime.assets.is_none() {
                let invalid = options.invalid_request_error.clone();
                effect_try_sync! {
                    body: move || -> Result<(), E> { Err(invalid("media store unavailable")) },
                }?;
            }
            let id = command_input_media_id(&input)?;
            if id.is_empty() {
                let invalid = options.invalid_request_error.clone();
                effect_try_sync! {
                    body: move || -> Result<MutationResult, E> {
                        Err(invalid("missing media id"))
                    },
                }?
            } else {
                let invalid = options.invalid_request_error.clone();
                let form_input = read_media_form::<E, R>(
                    &request,
                    options.form.clone(),
                    options.form_policy,
                    selected,
                    move |_error| invalid("media form read"),
                )?;
                options.runtime.update_asset_from_form(id, form_input)?;
                mutation_result(
                    &options.kind,
                    &ResourceAction::new(RESOURCE_ACTION_UPDATE),
                    &options.redirect_route,
                )
            }
        }
    })
}

/// The delete executor.
#[must_use]
pub fn media_asset_delete_executor<E, R>(
    options: MediaAssetCommandBindingOptions<E, R>,
) -> ResourceCommandExecutor<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(move |_request, input| {
        let options = options.clone();
        effect! {
            let options = options.clone();
            let input = input.clone();
            let id = command_input_media_id(&input)?;
            if options.runtime.assets.is_none() || id.is_empty() {
                let invalid = options.invalid_request_error.clone();
                let missing_store = options.runtime.assets.is_none();
                effect_try_sync! {
                    body: move || -> Result<MutationResult, E> {
                        Err(invalid(if missing_store {
                            "media store unavailable"
                        } else {
                            "missing media id"
                        }))
                    },
                }?
            } else {
                options.runtime.delete_asset(id)?;
                mutation_result(
                    &options.kind,
                    &ResourceAction::new(RESOURCE_ACTION_DELETE),
                    &options.redirect_route,
                )
            }
        }
    })
}

/// The create validator.
#[must_use]
pub fn media_asset_create_validator<E: Send + 'static, R: Send + Sync + 'static>(
    options: MediaAssetValidationBindingOptions<E>,
) -> ActionValidator<CommandInput, E, R> {
    create_validator(options, None)
}

fn create_validator<E: Send + 'static, R: Send + Sync + 'static>(
    options: MediaAssetValidationBindingOptions<E>,
    selected: Option<PreparedMediaInput<E>>,
) -> ActionValidator<CommandInput, E, R> {
    Arc::new(move |request, _input| validate_media_asset_mutation(request, options.clone(), selected.clone()))
}

/// The update validator.
#[must_use]
pub fn media_asset_update_validator<E: Send + 'static, R: Send + Sync + 'static>(
    options: MediaAssetValidationBindingOptions<E>,
) -> ActionValidator<CommandInput, E, R> {
    update_validator(options, None)
}

fn update_validator<E: Send + 'static, R: Send + Sync + 'static>(
    options: MediaAssetValidationBindingOptions<E>,
    selected: Option<PreparedMediaInput<E>>,
) -> ActionValidator<CommandInput, E, R> {
    Arc::new(move |request, input| {
        let request = request.clone();
        let options = options.clone();
        let selected = selected.clone();
        effect! {
            let request = request.clone();
            let options = options.clone();
            let input = input.clone();
            let selected = selected.clone();
            let id = command_input_media_id(&input)?;
            if id.is_empty() {
                let invalid = options.invalid_request_error.clone();
                effect_try_sync! { body: move || -> Result<(), E> { Err(invalid()) } }?
            } else {
                validate_media_asset_mutation(&request, options, selected)?
            }
        }
    })
}

fn validate_media_asset_mutation<E: Send + 'static, R: Send + Sync + 'static>(
    request: &Request,
    options: MediaAssetValidationBindingOptions<E>,
    selected: Option<PreparedMediaInput<E>>,
) -> Effect<(), E, R> {
    let request = request.clone();
    effect! {
        let request = request.clone();
        let options = options.clone();
        let selected = selected.clone();
        let on_error = options.on_form_error.clone();
        let input = read_media_form::<E, R>(
            &request,
            options.form.clone(),
            crate::attachments::MediaFormPolicy {
                urlencoded: options.form_policy,
                multipart: options.multipart_policy,
                multipart_fields: options.multipart_fields,
                multipart_values: options.multipart_values,
                file_memory: options.file_memory,
            },
            selected,
            move |error| on_error(error),
        )?;
        let empty = input.url.trim().is_empty();
        let message = options.message(&request);
        if empty && !message.trim().is_empty() {
            let map = options.validation_error.clone();
            effect_try_sync! { body: move || -> Result<(), E> { Err(map(validation_issue_error(VALIDATION_ISSUE_REQUIRED, "url", &message))) } }?
        } else { () }
    }
}

fn read_media_form<E: Send + 'static, R: Send + Sync + 'static>(
    request: &Request,
    form: Option<Arc<effect_server::ByteFormData>>,
    policy: crate::attachments::MediaFormPolicy,
    selected: Option<PreparedMediaInput<E>>,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<MediaAssetFormInput, E, R> {
    match (selected, form) {
        (Some(selected), None) => {
            let on_error = selected.on_error;
            media_asset_form_input_from_prepared_request(
                request, policy, &selected.forms, move |error| on_error(error),
            )
        }
        (Some(_), form @ Some(_)) | (None, form) => {
            media_asset_form_input_from_request(request, form, policy, on_error)
        }
    }
}

fn validation_issue_error(code: &str, path: &str, message: &str) -> ValidationError {
    let issue = ValidationIssue {
        code: ValidationIssueCode::new(code),
        path: path.to_owned(),
        field_key: path.to_owned(),
        message: message.to_owned(),
        ..ValidationIssue::default()
    };
    match ValidationError::new(vec![issue]) {
        Some(error) => error,
        None => ValidationError::new(vec![ValidationIssue::default()])
            .unwrap_or_else(|| unreachable_validation_error()),
    }
}

fn unreachable_validation_error() -> ValidationError {
    // ValidationError::new rejects only an empty normalized issue set. A default
    // issue normalizes to the required vocabulary, so this path has no observable
    // success value; return a normalized required error as the invariant fallback.
    ValidationError::new(vec![ValidationIssue {
        code: ValidationIssueCode::new(VALIDATION_ISSUE_REQUIRED),
        ..ValidationIssue::default()
    }])
    .unwrap_or_else(|| unreachable_validation_error())
}

fn command_input_media_id<E, R>(input: &CommandInputHandle) -> Effect<crate::commands::InputBytes, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    input.with(|input| input.detail_query.id.trim().to_owned())
}

fn mutation_result(
    kind: &ResourceKind,
    action: &ResourceAction,
    redirect_route: &str,
) -> MutationResult {
    MutationResult {
        kind: kind.clone(),
        action: action.clone(),
        redirect_route: redirect_route.trim().to_owned(),
        ..MutationResult::default()
    }
}
