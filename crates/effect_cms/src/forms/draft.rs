//! The consumer-facing draft render state and slug target resolution.
//!
//! The draft render adapter turns one request into the data a resource form renders:
//! the edit flag, the row, the localized drafts, the attachments and the caller's
//! extra value. The query owner admits the command input; the owner's command-input
//! parser supplies the exact merged form bytes and the admitted list/detail queries;
//! the consumer's build callback produces the mutation; and the consumer's row,
//! drafts, media and extra accessors project it.
//!
//! The adapter performs no data access of its own: listing and selecting the target
//! metadata are supplied callbacks in [`resolve_slug_update_target`] and
//! [`resolve_parent_slug_update_target`]. A missing input, list callback, selector or
//! target invokes the caller's invalid-request callback; a listing failure keeps its
//! own error value.
//!
//! Reading the request form, admitting the queries and invoking the build callback
//! happen only when the returned [`Effect`] runs.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use effect::{Effect, effect, effect_map_error, effect_try_sync};
use effect_server::{ByteFormPolicy, Request};

use crate::attachments::MediaAttachmentFormItem;
use crate::commands::CommandInput;
use crate::definitions::{RESOURCE_ACTION_CREATE, RESOURCE_ACTION_UPDATE, ResourceAction, ResourceKind};
use crate::queries::QueryService;
use crate::validation::{ActionValidationError, build_command_input};

/// The empty extra value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoResourceDraftRenderExtra;

/// The data a resource form renders for one request.
#[derive(Clone, Debug, Default)]
pub struct ResourceDraftRenderState<Row, Draft, Extra> {
    /// Whether the request is an edit.
    pub is_edit: bool,
    /// The resolved row.
    pub row: Row,
    /// The localized drafts.
    pub drafts: Vec<Draft>,
    /// The presented attachment rows.
    pub attachments: Vec<MediaAttachmentFormItem>,
    /// The caller's extra value.
    pub extra: Extra,
}

/// Builds the mutation from the mutable parsed command input.
///
/// The callback receives the same admitted input value the flow parsed, so consumer
/// normalization of that input is visible on the input and on the mutation it returns.
pub type DraftBuildFunc<Row, Mutation, E> =
    Arc<dyn Fn(&Request, &mut CommandInput, Row) -> Result<Mutation, E> + Send + Sync + 'static>;

/// Projects the rendered row from the mutation and fallback row.
pub type DraftRowFunc<Row, Mutation> =
    Arc<dyn Fn(&Mutation, Row) -> Row + Send + Sync + 'static>;

/// Projects the localized drafts from the mutation.
pub type DraftsFunc<Draft, Mutation> =
    Arc<dyn Fn(&Mutation) -> Vec<Draft> + Send + Sync + 'static>;

/// Projects the presented attachments from the mutation.
pub type DraftMediaFunc<Mutation> =
    Arc<dyn Fn(&Mutation) -> Vec<MediaAttachmentFormItem> + Send + Sync + 'static>;

/// Projects the caller's extra value from the mutation.
pub type DraftExtraFunc<Extra, Mutation> =
    Arc<dyn Fn(&Mutation) -> Extra + Send + Sync + 'static>;

/// Configures one draft render projection.
pub struct ResourceDraftRenderStateOptions<Row, Draft, Mutation, Extra, E> {
    /// The admitted query service.
    pub queries: QueryService,
    /// The requested kind.
    pub kind: ResourceKind,
    /// Whether the request is an edit.
    pub is_edit: bool,
    /// The fallback row.
    pub fallback: Row,
    /// The owner's byte-form transport policy.
    pub form_policy: ByteFormPolicy,
    /// Builds the mutation; an absent callback is the invalid request.
    pub build: Option<DraftBuildFunc<Row, Mutation, E>>,
    /// Projects the rendered row.
    pub row: Option<DraftRowFunc<Row, Mutation>>,
    /// Projects the localized drafts.
    pub drafts: Option<DraftsFunc<Draft, Mutation>>,
    /// Projects the presented attachments.
    pub media: Option<DraftMediaFunc<Mutation>>,
    /// Projects the extra value.
    pub extra: Option<DraftExtraFunc<Extra, Mutation>>,
    /// The zero extra value returned through the preserved empty-extra branch.
    pub zero_extra: Extra,
}

impl<Row, Draft, Mutation, Extra, E> Clone for ResourceDraftRenderStateOptions<Row, Draft, Mutation, Extra, E>
where
    Row: Clone,
    Extra: Clone,
{
    fn clone(&self) -> Self {
        Self {
            queries: self.queries.clone(),
            kind: self.kind.clone(),
            is_edit: self.is_edit,
            fallback: self.fallback.clone(),
            form_policy: self.form_policy,
            build: self.build.clone(),
            row: self.row.clone(),
            drafts: self.drafts.clone(),
            media: self.media.clone(),
            extra: self.extra.clone(),
            zero_extra: self.zero_extra.clone(),
        }
    }
}

/// A draft flow failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DraftFlowError<E> {
    /// The request input was refused by the command-input parser.
    Request(ActionValidationError),
    /// The consumer's invalid-request or build callback failed.
    Consumer(E),
}

impl<E: fmt::Display> fmt::Display for DraftFlowError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(error) => fmt::Display::fmt(error, formatter),
            Self::Consumer(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl<E: Error + 'static> Error for DraftFlowError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Request(error) => Some(error),
            Self::Consumer(error) => Some(error),
        }
    }
}

/// Build the draft render state for one request.
///
/// The command input is parsed first; a parse failure wins over the absent build
/// callback, which in turn invokes the caller's invalid-request callback. The build
/// callback receives the request, the mutable parsed input and a copy of the fallback
/// row. The row/drafts/media/extra accessors run only when supplied, and the row falls
/// back to its zero value.
pub fn build_resource_draft_render_state<Row, Draft, Mutation, Extra, E, R>(
    request: &Request,
    options: ResourceDraftRenderStateOptions<Row, Draft, Mutation, Extra, E>,
    invalid_request: impl Fn() -> E + Send + Sync + 'static,
) -> Effect<ResourceDraftRenderState<Row, Draft, Extra>, DraftFlowError<E>, R>
where
    Row: Clone + Default + Send + Sync + 'static,
    Draft: Send + 'static,
    Mutation: Send + 'static,
    Extra: Clone + Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request = request.clone();
    let invalid_request = Arc::new(invalid_request);
    effect! {
        let options = options.clone();
        let invalid_request = Arc::clone(&invalid_request);
        let request = request.clone();
        let action = if options.is_edit {
            ResourceAction::new(RESOURCE_ACTION_UPDATE)
        } else {
            ResourceAction::new(RESOURCE_ACTION_CREATE)
        };
        let input_effect = build_command_input(
            Some(options.queries.clone()),
            Some(&request),
            &options.kind,
            &action,
            options.form_policy,
        );
        let mut input = effect_map_error! {
            program: input_effect,
            map |error| { DraftFlowError::Request(error) },
        }?;
        let request = request.clone();
        let state = effect_try_sync! {
            once;
            body: move || {
                let Some(build) = options.build.as_ref() else {
                    return Err(DraftFlowError::Consumer(invalid_request()));
                };
                let mutation = build(&request, &mut input, options.fallback.clone())
                    .map_err(DraftFlowError::Consumer)?;
                let mut state = ResourceDraftRenderState {
                    is_edit: options.is_edit,
                    row: Row::default(),
                    drafts: Vec::new(),
                    attachments: Vec::new(),
                    extra: options.zero_extra.clone(),
                };
                if let Some(row) = options.row.as_ref() {
                    state.row = row(&mutation, options.fallback.clone());
                }
                if let Some(drafts) = options.drafts.as_ref() {
                    state.drafts = drafts(&mutation);
                }
                if let Some(media) = options.media.as_ref() {
                    state.attachments = media(&mutation);
                }
                if let Some(extra) = options.extra.as_ref() {
                    state.extra = extra(&mutation);
                }
                Ok(state)
            },
        }?;
        state
    }
}

/// Resolve the slug update target.
///
/// The slug is trimmed from the command input's detail query; an absent input, list
/// callback or selector, an empty slug, and a missing metadata row all invoke the
/// caller's invalid-request callback. A listing failure keeps its own error value.
///
/// # Errors
///
/// Returns the caller's listing error or the caller's invalid-request error.
pub fn resolve_slug_update_target<Row, E>(
    request: &Request,
    input: Option<&CommandInput>,
    list: Option<&(dyn Fn(&Request, &[u8]) -> Result<Vec<Row>, E> + Send + Sync)>,
    select_metadata: Option<&(dyn Fn(&[Row]) -> Option<Row> + Send + Sync)>,
    invalid_request: &(dyn Fn() -> E + Send + Sync),
) -> Result<(Vec<Row>, Row), E> {
    let (Some(input), Some(list), Some(select_metadata)) = (input, list, select_metadata) else {
        return Err(invalid_request());
    };
    let slug = input.detail_query.slug.trim();
    if slug.is_empty() {
        return Err(invalid_request());
    }
    let rows = list(request, slug.as_bytes())?;
    let Some(metadata) = select_metadata(&rows) else {
        return Err(invalid_request());
    };
    Ok((rows, metadata))
}

/// Resolve the parent/slug update target.
///
/// The parent and slug are trimmed from the command input's detail query; an absent
/// input, list callback or selector, an empty parent or slug, and a missing metadata
/// row all invoke the caller's invalid-request callback.
///
/// # Errors
///
/// Returns the caller's listing error or the caller's invalid-request error.
pub fn resolve_parent_slug_update_target<Row, E>(
    request: &Request,
    input: Option<&CommandInput>,
    list: Option<&(dyn Fn(&Request, &[u8], &[u8]) -> Result<Vec<Row>, E> + Send + Sync)>,
    select_metadata: Option<&(dyn Fn(&[Row]) -> Option<Row> + Send + Sync)>,
    invalid_request: &(dyn Fn() -> E + Send + Sync),
) -> Result<(Vec<Row>, Row), E> {
    let (Some(input), Some(list), Some(select_metadata)) = (input, list, select_metadata) else {
        return Err(invalid_request());
    };
    let parent = input.detail_query.parent.trim();
    let slug = input.detail_query.slug.trim();
    if parent.is_empty() || slug.is_empty() {
        return Err(invalid_request());
    }
    let rows = list(request, parent.as_bytes(), slug.as_bytes())?;
    let Some(metadata) = select_metadata(&rows) else {
        return Err(invalid_request());
    };
    Ok((rows, metadata))
}
