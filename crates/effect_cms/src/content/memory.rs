//! Prepared workspaces shared by the CMS content and media consumers.

use super::{
    HtmlSanitizeLimits, PreparedHtmlSanitizer, PreparedShortcodeRenderer, ShortcodeRenderLimits,
};
use crate::attachments::{
    MediaAttachmentFormItem, MediaAttachmentStoreRecord, PreparedAttachmentCapacity,
    PreparedAttachmentPool,
};
use crate::authoring::{GeminiTranslator, GeminiWorkingLimits, PreparedGeminiWorkspace};
use crate::media::attachmentflow::PreparedAttachmentRowPool;
use crate::media::contentmedia::{MediaDecodeLimits, PreparedMediaDecoder};
use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Explicit content, media and optional Gemini workspace capacities.
#[derive(Clone, Copy)]
#[must_use]
pub struct PreparedCmsContentCapacity {
    /// Concurrent sanitizer work and retained outputs.
    pub sanitizers: NonZeroUsize,
    /// Input and output limits for HTML sanitization.
    pub sanitizer: HtmlSanitizeLimits,
    /// Concurrent shortcode work and retained outputs.
    pub shortcodes: NonZeroUsize,
    /// Limits passed to the existing shortcode resolver boundary.
    pub shortcode: ShortcodeRenderLimits,
    /// Concurrent stored-media decoding and retained results.
    pub media_decoders: NonZeroUsize,
    /// Stored-media input and item limits.
    pub media: MediaDecodeLimits,
    /// Attachment input snapshots and per-batch row count.
    pub attachments: PreparedAttachmentCapacity,
    /// Maximum native metadata bytes projected into each selected attachment batch.
    pub attachment_metadata: Bytes,
    /// Optional concurrent provider calls and request/decode envelopes.
    pub gemini: Option<(NonZeroUsize, GeminiWorkingLimits)>,
}

/// Content and media consumers retain the same physical startup workspaces.
pub struct PreparedCmsContent {
    sanitizer: PreparedHtmlSanitizer,
    shortcodes: Arc<PreparedShortcodeRenderer>,
    media: PreparedMediaDecoder,
    pub(crate) attachments: Arc<PreparedAttachmentPool>,
    pub(crate) attachment_items: Arc<PreparedAttachmentRowPool<MediaAttachmentFormItem>>,
    pub(crate) attachment_records: Arc<PreparedAttachmentRowPool<MediaAttachmentStoreRecord>>,
    pub(crate) attachment_replacements: Arc<PreparedAttachmentRowPool<MediaAttachmentStoreRecord>>,
    pub(crate) attachment_metadata: Bytes,
    gemini: Option<Arc<PreparedGeminiWorkspace>>,
}

impl PreparedCmsContent {
    /// Calculate the regional payload before preparing the shared backing.
    pub fn storage_bytes(capacity: PreparedCmsContentCapacity) -> Result<Bytes, MemoryError> {
        let calls = capacity.attachments.snapshots;
        let rows = capacity.attachments.rows;
        let main = PreparedHtmlSanitizer::storage_bytes(capacity.sanitizers, capacity.sanitizer)?
            .checked_add(PreparedShortcodeRenderer::storage_bytes(
                capacity.shortcodes,
                capacity.shortcode,
            )?)?
            .checked_add(PreparedMediaDecoder::storage_bytes(
                capacity.media_decoders,
                capacity.media,
            )?)?
            .checked_add(PreparedAttachmentPool::storage_bytes(capacity.attachments)?)?
            .checked_add(
                PreparedAttachmentRowPool::<MediaAttachmentFormItem>::storage_bytes(calls, rows)?,
            )?
            .checked_add(
                PreparedAttachmentRowPool::<MediaAttachmentStoreRecord>::storage_bytes(
                    calls, rows,
                )?,
            )?
            .checked_add(
                PreparedAttachmentRowPool::<MediaAttachmentStoreRecord>::storage_bytes(
                    calls, rows,
                )?,
            )?;
        if let Some((calls, limits)) = capacity.gemini {
            main.checked_add(PreparedGeminiWorkspace::storage_bytes(calls, limits)?)
        } else {
            Ok(main)
        }
    }

    /// Provision all selected owners in one caller-supplied region.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedCmsContentCapacity,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(capacity)?;
        let calls = capacity.attachments.snapshots;
        let rows = capacity.attachments.rows;
        Ok(Self {
            sanitizer: PreparedHtmlSanitizer::prepare(
                memory,
                region,
                capacity.sanitizers,
                capacity.sanitizer,
            )?,
            shortcodes: Arc::new(PreparedShortcodeRenderer::prepare(
                memory,
                region,
                capacity.shortcodes,
                capacity.shortcode,
            )?),
            media: PreparedMediaDecoder::prepare(
                memory,
                region,
                capacity.media_decoders,
                capacity.media,
            )?,
            attachments: Arc::new(PreparedAttachmentPool::prepare(
                memory,
                region,
                capacity.attachments,
            )?),
            attachment_items: Arc::new(PreparedAttachmentRowPool::prepare(
                memory, region, calls, rows,
            )?),
            attachment_records: Arc::new(PreparedAttachmentRowPool::prepare(
                memory, region, calls, rows,
            )?),
            attachment_replacements: Arc::new(PreparedAttachmentRowPool::prepare(
                memory, region, calls, rows,
            )?),
            attachment_metadata: capacity.attachment_metadata,
            gemini: capacity
                .gemini
                .map(|(calls, limits)| {
                    PreparedGeminiWorkspace::prepare(memory, region, calls, limits).map(Arc::new)
                })
                .transpose()?,
        })
    }

    /// Use the existing sanitizer with the selected region owners.
    #[must_use]
    pub fn sanitizer(&self) -> &PreparedHtmlSanitizer {
        &self.sanitizer
    }

    /// Invoke an existing shortcode resolver using the selected workspaces.
    #[must_use]
    pub fn shortcodes(&self) -> &Arc<PreparedShortcodeRenderer> {
        &self.shortcodes
    }

    /// Decode stored media using the selected workspace.
    #[must_use]
    pub fn media(&self) -> &PreparedMediaDecoder {
        &self.media
    }

    /// Select the prepared workspace for the caller's existing Gemini translator.
    pub fn gemini_translator(
        &self,
        translator: GeminiTranslator,
    ) -> Result<GeminiTranslator, MemoryError> {
        let workspace = self
            .gemini
            .as_ref()
            .ok_or(MemoryError::MissingRuntimeStorage)?;
        Ok(translator.with_prepared_workspace(workspace.clone()))
    }
}
