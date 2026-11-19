//! Attachment replacement: store-backed preview, listing, current-input selection,
//! ordered owner replacement and derived mutation media state.
//!
//! This owner coordinates supplied capabilities and owns no persistence. The asset
//! loader, attachment row lister, row replacer and current-item loader are
//! consumer-supplied and return lazy effects; the CMS calls them and owns only the
//! workflow order and the rules described by each public operation.
//!
//! # Ordering and identity
//!
//! The request's merged form selection is collected in position order and is never
//! deduplicated by preview or replacement. A row builder runs once per collected
//! input and its accepted rows replace the owner's rows in one call, so an empty
//! selection still clears the owner's rows through that call. A replacement error
//! is returned unchanged and final items are loaded only after replacement
//! succeeds. Asset rows absent from an asset lookup are skipped.
//!
//! # Stages and reuse
//!
//! A replacement runs through a private by-value stage that only this owner can
//! construct. The stage is not cloneable and transfers its rows to the replacement
//! capability once per run; the public workflow itself stays reusable because each
//! run builds a fresh stage.
//!
//! # Request input
//!
//! Request variants read the caller's initialized merged [`effect_server::ByteFormData`]
//! together with request presence. They do not parse or claim a request body. This
//! preserves the collection helper's decoded-form boundary: an absent request or
//! uninitialized form has no selection. URL-encoded, partial-body and multipart
//! initialization belong to the existing request adapter, whose byte-preserving
//! snapshot passes through this owner without another parse or text conversion.
//!
//! # HTML and persistence
//!
//! No HTML is produced here. The presented form items are data for rendering
//! owners, and no table, schema, migration, query, pool, connection or backend is
//! declared or opened.

mod prepared;
mod state;
mod workflow;

pub use self::state::{
    ContentMediaCollectionMutationState, ContentVideoMutationMediaState,
    ContentVideoMutationMediaStateOptions, build_content_media_collection_mutation_state,
    build_content_video_mutation_media_state,
};
pub use self::prepared::{
    PreparedAttachmentReplaceOptions, PreparedAttachmentRowBuilder, PreparedAttachmentRowPool,
    PreparedAttachmentRowReplacer, PreparedAttachmentRows,
    list_prepared_media_attachment_form_items,
    preview_prepared_media_attachment_form_items_from_request,
    replace_prepared_media_attachments_from_request,
};
pub(crate) use self::prepared::{list_prepared_admitted, reserve_attachment_list};
pub use self::workflow::{
    AttachmentPreviewer, AttachmentReplacer, CurrentMediaAttachmentInputOptions,
    MediaAttachmentAssetLoader, MediaAttachmentItemBuilder, MediaAttachmentItemLoader,
    MediaAttachmentListOptions, MediaAttachmentPreviewOptions, MediaAttachmentReplaceOptions,
    MediaAttachmentRowBuilder, MediaAttachmentRowLister, MediaAttachmentRowRecord,
    MediaAttachmentRowReplacer, current_media_attachment_inputs, list_media_attachment_form_items,
    preview_media_attachment_form_items_from_inputs,
    preview_media_attachment_form_items_from_request, replace_media_attachments_from_request,
    replace_resource_media_attachments_from_request,
};
