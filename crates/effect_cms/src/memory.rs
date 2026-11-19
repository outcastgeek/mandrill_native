//! Startup backing carried by the caller's CMS effect environment.

use std::num::NonZeroUsize;
use std::sync::Arc;

use effect::memory::{Buffer, Bytes, MemoryError, PreparedMemory, RegionId, SlotPool};
use effect::{Effect, effect, service};
use effect_server::{HasRequest, RequestEnv, ServerError};

use crate::content::PreparedCmsContent;
use crate::forms::{
    InitializeForm, MergedFormCapacity, MergedFormPolicy, PreparedMergedForm, PreparedMergedForms,
};
use crate::localization::{LocalizedDraftCapacity, PreparedLocalizedDrafts};
use crate::validation::{PreparedValidationResults, ValidationResultCapacity};

/// Startup capacity of the common request, result and publication workspaces.
#[derive(Clone, Copy)]
pub struct CmsMemoryCapacity {
    /// Form and query snapshots and their decoded fields.
    pub forms: MergedFormCapacity,
    /// Published form and schema validation reports.
    pub validation_results: ValidationResultCapacity,
    /// Localized text/content drafts.
    pub localized_drafts: LocalizedDraftCapacity,
    /// Concurrent retained Datastar event frames.
    pub event_frames: NonZeroUsize,
    /// Bytes in each event frame.
    pub event_frame_bytes: Bytes,
}

/// Region-backed Datastar buffers, retained separately by each emitted frame.
pub type CmsEventFrames = SlotPool<'static, Buffer<'static>>;

/// Shared startup workspaces selected by prepared CMS consumers.
///
/// A consumer can size these workspaces with its definition, access, command and
/// provider owners within one application memory plan. HTTP body and HTML reply
/// slots belong to the selected Server endpoint and are claimed through its
/// request.
pub struct PreparedCmsMemory {
    forms: Arc<PreparedMergedForms>,
    validation_results: Arc<PreparedValidationResults>,
    localized_drafts: PreparedLocalizedDrafts,
    event_frames: Arc<CmsEventFrames>,
    content: Option<Arc<PreparedCmsContent>>,
}

impl PreparedCmsMemory {
    /// Compute payload and slot-table demand for the common workspaces.
    /// Account separately for allocator metadata, alignment and other owners.
    pub fn storage_bytes(capacity: CmsMemoryCapacity) -> Result<Bytes, MemoryError> {
        let forms = PreparedMergedForms::storage_bytes(capacity.forms)?;
        let validation = PreparedValidationResults::storage_bytes(capacity.validation_results)?;
        let drafts = PreparedLocalizedDrafts::storage_bytes(capacity.localized_drafts)?;
        let frames = CmsEventFrames::storage_bytes(capacity.event_frames.get())?.checked_add(
            capacity
                .event_frame_bytes
                .checked_mul(capacity.event_frames.get())?,
        )?;
        forms
            .checked_add(validation)?
            .checked_add(drafts)?
            .checked_add(frames)
    }

    /// Reserve the actual owner storage from the caller's selected region at startup.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: CmsMemoryCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(capacity)?;
        Ok(Self {
            forms: Arc::new(PreparedMergedForms::prepare(
                memory,
                region,
                capacity.forms,
            )?),
            validation_results: Arc::new(PreparedValidationResults::prepare(
                memory,
                region,
                capacity.validation_results,
            )?),
            localized_drafts: PreparedLocalizedDrafts::prepare(
                memory,
                region,
                capacity.localized_drafts,
            )?,
            event_frames: Arc::new(memory.buffers(
                region,
                capacity.event_frames.get(),
                capacity.event_frame_bytes,
            )?),
            content: None,
        })
    }

    /// Select explicitly provisioned content/media/authoring backing.
    #[must_use]
    pub fn with_content(mut self, content: Arc<PreparedCmsContent>) -> Self {
        self.content = Some(content);
        self
    }

    /// Borrow the optional content workspace supplied by the application.
    #[must_use]
    pub fn content(&self) -> Option<&Arc<PreparedCmsContent>> {
        self.content.as_ref()
    }

    /// Select the decoded form/query snapshot owner.
    #[must_use]
    pub fn forms(&self) -> &Arc<PreparedMergedForms> {
        &self.forms
    }

    /// Select the finalized validation report owner.
    #[must_use]
    pub fn validation_results(&self) -> &Arc<PreparedValidationResults> {
        &self.validation_results
    }

    /// Select the localized draft snapshot owner.
    #[must_use]
    pub fn localized_drafts(&self) -> &PreparedLocalizedDrafts {
        &self.localized_drafts
    }

    /// Select the transport-held Datastar frame owner.
    #[must_use]
    pub fn event_frames(&self) -> &Arc<CmsEventFrames> {
        &self.event_frames
    }
}

/// An application environment that explicitly selects prepared CMS storage.
pub trait HasCmsMemory {
    /// Borrow the startup capability with its region owners.
    fn cms_memory(&self) -> &Arc<PreparedCmsMemory>;
}

impl<R: HasCmsMemory> HasCmsMemory for RequestEnv<R> {
    fn cms_memory(&self) -> &Arc<PreparedCmsMemory> {
        self.services.cms_memory()
    }
}

/// Read this effect's request into the application's prepared CMS form workspace.
pub fn read_prepared_cms_form<E, R>(
    policy: MergedFormPolicy,
    mode: InitializeForm,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<PreparedMergedForm, E, R>
where
    E: Send + 'static,
    R: HasCmsMemory + HasRequest + Send + Sync + 'static,
{
    let on_error = Arc::new(on_error);
    effect! {
        let on_error = on_error.clone();
        let (request, storage) = service! {
            env: |env: &R| (env.request().clone(), env.cms_memory().forms().clone()),
        }?;
        storage.read(&request, policy, mode, move |error| on_error(error))?
    }
}
