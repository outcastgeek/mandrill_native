//! Attachments: the media attachment item, surface and manager contracts and the
//! pure attachment input algebra.
//!
//! Attachments owns:
//!
//! - the attachment input, persisted store record, presented form item, media
//!   surface mounts and attachment manager props records, and the four declared
//!   attachment roles;
//! - the role options (hero name, gallery name and the caller's role normalizer);
//! - ordered collection of attachment inputs from the request's decoded form,
//!   conversion from presented form items, deduplication, the owner's role
//!   defaulting, attaching asset identities under a role, promoting one attachment
//!   to hero, removing one attachment, identity lookup;
//! - the unique selected asset identities, the media shortcode text and the
//!   repeated-form-value legacy fallback.
//!
//! # API
//!
//! The public surface keeps the owner's names and responsibilities. Type aliases
//! exist only where Rust needs a name for a callback type:
//! [`RoleNormalizer`]/[`RoleNormalizerRef`] for a byte-preserving role normalizer.
//!
//! # Decoded form input instead of a request
//!
//! The helpers do not parse the request; they read an already-parsed form map. This
//! owner therefore takes `Option<&effect_server::ByteFormData>` — `None` for an absent
//! request and `Some` for an already-decoded merged form view — and performs no I/O.
//! The transport adapter decides when to decode it: URL-encoded body values
//! precede query values, while multipart fields follow query values;
//! the private `inputs` module documents that responsibility and its limits exactly.
//!
//! # Data, optionality and effects
//!
//! Every record here is caller-supplied or caller-consumed plain data with a zero
//! value; no invariant is invented and no record authorizes a store, table, schema,
//! migration, pool or query. The role options, the role normalizer and every option
//! field are genuinely optional (blank strings and absent callbacks), not optional
//! required capabilities. The owner holds no
//! capability in its pure algebra. `initialize_media_form` is a separate lazy
//! `Effect` adapter over native request input with caller-supplied parse policies.
//! It retains decoded bytes and returns parse diagnostics separately.
//!
//! # HTML
//!
//! This owner produces no HTML: a form item and the manager props are data for the
//! rendering owners, and [`media_shortcode`] is shortcode text, not markup, so Maud
//! is not involved and nothing is assembled, escaped or formatted as HTML here.
//!
//! # Dependencies
//!
//! `effect-server` supplies byte-form snapshots and native request readers. No database, SQL,
//! schema, migration, pool, backend, credential handling or consumer store
//! invocation exists anywhere in this owner. The adapter uses the required effect
//! macros; no identity or OTel capability is introduced.
//!
//! # Private implementation
//!
//! `items` and `inputs` are private implementation modules of this owner: they hold
//! the declarations and the mechanics, while this module is the owner's deliberate
//! public surface.

use effect_server::ByteFormData;
use crate::commands::InputBytes;

mod inputs;
mod items;
mod prepared;
mod request;

pub use self::inputs::{MediaAttachmentRoleOptions, RoleNormalizer, RoleNormalizerRef};
pub use self::prepared::{PreparedAttachmentCapacity, PreparedAttachmentInput, PreparedAttachmentInputs, PreparedAttachmentPool};
pub use self::request::{InitializeForm, MediaFormPolicy, initialize_media_form};
pub use self::items::{
    MEDIA_ATTACHMENT_ROLE_DOWNLOAD, MEDIA_ATTACHMENT_ROLE_GALLERY, MEDIA_ATTACHMENT_ROLE_HERO,
    MEDIA_ATTACHMENT_ROLE_INLINE, MediaAttachmentFormItem, MediaAttachmentInput,
    MediaAttachmentManagerProps, MediaAttachmentStoreRecord, MediaSurfaceMounts,
};

/// Collect ordered attachment inputs from the request's decoded form.
///
/// `None` is an absent request, which reads no values. Six repeated columns are read
/// with primary and legacy fallback names, one row
/// is built per position up to the longest column, blank asset identities are
/// skipped after the row is built, and the result is stable-sorted by the parsed
/// position and then the original column index without deduplication. The private
/// `inputs` module documents the value order, the trim rules and the transport parse
/// responsibility.
#[must_use]
pub fn collect_media_attachment_inputs(
    form: Option<&ByteFormData>,
    normalize_role: Option<RoleNormalizerRef<'_>>,
) -> Vec<MediaAttachmentInput> {
    inputs::collect_media_attachment_inputs(form, normalize_role)
}

/// Derive deduplicated attachment inputs from presented form items.
///
/// A blank trimmed asset identity is skipped before the role normalizer runs, kept
/// rows have their metadata trimmed and their role normalized, and the result passes
/// through the same dedupe as [`dedupe_media_attachment_inputs`].
#[must_use]
pub fn media_attachment_inputs_from_items(
    form_items: &[MediaAttachmentFormItem],
    normalize_role: Option<RoleNormalizerRef<'_>>,
) -> Vec<MediaAttachmentInput> {
    inputs::media_attachment_inputs_from_items(form_items, normalize_role)
}

/// Remove blank and repeated asset identities, keeping the first occurrence.
///
/// The asset identity is trimmed, the first entry for each identity wins, and only
/// the role is normalized; the title override, caption and alternative-text override
/// are not touched here.
#[must_use]
pub fn dedupe_media_attachment_inputs(
    inputs: &[MediaAttachmentInput],
    normalize_role: Option<RoleNormalizerRef<'_>>,
) -> Vec<MediaAttachmentInput> {
    inputs::dedupe_media_attachment_inputs(inputs, normalize_role)
}

/// Preserve the authoring role choices and the gallery default.
///
/// The value is trimmed and lowercased; only the exact `hero`, `inline` and
/// `download` spellings are kept and every other value becomes `gallery`. This
/// defaulting rule is the owner's, and it is not applied by the generic trimming
/// used elsewhere in this owner.
#[must_use]
pub fn normalize_media_attachment_role(role: impl AsRef<[u8]>) -> String {
    inputs::normalize_media_attachment_role(role)
}

/// Add asset identities under one role.
///
/// The current inputs are deduped, the role is normalized, and the configured hero
/// and gallery names come from the role options with the declared names as fallback.
/// The hero branch selects the first trimmed nonblank identity, creates a fresh hero
/// entry without the previous metadata, omits the previous entry for that identity
/// and downgrades older heroes to gallery; every other role appends new identities
/// once. Both branches finish with the same dedupe.
#[must_use]
pub fn attach_asset_ids_to_attachment_inputs(
    current: &[MediaAttachmentInput],
    asset_ids: &[InputBytes],
    role: impl AsRef<[u8]>,
    options: &MediaAttachmentRoleOptions,
) -> Vec<MediaAttachmentInput> {
    inputs::attach_asset_ids_to_attachment_inputs(current, asset_ids, role, options)
}

/// Promote one attachment to the configured hero role.
///
/// A blank identity only dedupes the current inputs; an unknown identity leaves them
/// unchanged. The promoted entry keeps its own metadata, receives the configured
/// hero role, moves to the front, and every other hero entry is downgraded to the
/// configured gallery role.
#[must_use]
pub fn promote_attachment_input(
    current: &[MediaAttachmentInput],
    asset_id: impl AsRef<[u8]>,
    options: &MediaAttachmentRoleOptions,
) -> Vec<MediaAttachmentInput> {
    inputs::promote_attachment_input(current, asset_id, options)
}

/// Remove one attachment by asset identity.
///
/// A blank identity only dedupes the current inputs; otherwise the deduped inputs
/// keep their order with the matching identity removed.
#[must_use]
pub fn remove_attachment_input(
    current: &[MediaAttachmentInput],
    asset_id: impl AsRef<[u8]>,
    normalize_role: Option<RoleNormalizerRef<'_>>,
) -> Vec<MediaAttachmentInput> {
    inputs::remove_attachment_input(current, asset_id, normalize_role)
}

/// Find the position of an asset identity, or absence.
///
/// Stored asset identities are trimmed before comparison, the supplied identity is
/// compared as given, and the first match wins. Absence is [`Option::None`] here.
#[must_use]
pub fn media_attachment_input_index(
    inputs: &[MediaAttachmentInput],
    asset_id: impl AsRef<[u8]>,
) -> Option<usize> {
    inputs::media_attachment_input_index(inputs, asset_id)
}

/// Read the unique selected asset identities from the request's decoded form.
///
/// The primary `selected_asset_ids[]` column wins whenever it carries any entry and
/// only otherwise does the legacy `selected_asset_id` column apply; entries are
/// trimmed, blank entries dropped and the first occurrence of each identity decides
/// its position. `None` is an absent request.
#[must_use]
pub fn media_selected_asset_ids(form: Option<&ByteFormData>) -> Vec<InputBytes> {
    inputs::media_selected_asset_ids(form)
}

/// Render the media shortcode text for one asset and role.
///
/// A blank identity yields no shortcode and a blank role becomes `inline`. The
/// result is the exact `[media id="<asset>" role=<role>]` text — the role is
/// unquoted and nothing is escaped or whitelisted. This is shortcode text, not HTML.
#[must_use]
pub fn media_shortcode(asset_id: impl AsRef<[u8]>, role: impl AsRef<[u8]>) -> InputBytes {
    inputs::media_shortcode(asset_id, role)
}

/// Read one repeated decoded form value with the legacy fallback.
///
/// The primary name wins whenever it carries any entry, including entries that are
/// all blank, and only otherwise does the fallback name apply. The stored values are
/// returned unchanged and in order. `None` is an absent request.
#[must_use]
pub fn form_values_with_fallback<'a>(
    form: Option<&'a ByteFormData>,
    primary: &str,
    fallback: &str,
) -> &'a [Vec<u8>] {
    inputs::form_values_with_fallback(form, primary, fallback)
}
