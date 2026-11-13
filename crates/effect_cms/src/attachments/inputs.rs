//! The attachment input algebra: collection, item conversion, dedupe, role
//! normalization, attach/promote/remove, index lookup, selected identities, the
//! media shortcode and the repeated-form-value fallback.
//!
//! # Decoded-form input contract
//!
//! The collection helpers do **not** parse the request. They read a form map
//! directly, which means:
//!
//! - the values come from a parse the *caller* already performed, usually in server
//!   middleware, so these helpers perform no I/O and have no error channel;
//! - URL-encoded body values precede query values; multipart fields follow
//!   query values, with repeated values preserved in each source-defined order;
//! - an absent request reads nothing, and a request whose form has not been parsed
//!   also reads nothing.
//!
//! The split is kept exactly. The helpers here take
//! `form: Option<&effect_server::ByteFormData>`: `None` is an absent request, and `Some`
//! is an already-decoded merged form view — `ByteFormData` is
//! precisely a field-name-to-repeated-values snapshot, so it represents the parsed
//! data and not a request that still has to be read. This owner therefore performs
//! no parsing, opens no body, creates no runtime and holds no capability; the
//! transport adapter that owns the request decides when and how to decode it.
//!
//! That transport responsibility is real and is not silently narrowed here: the
//! native `effect_server::Request::read_form` outcome is the URL-encoded **body**
//! only, so it is *not* by itself the merged body-then-query form map. A consumer
//! that needs query values must append them in order before calling these helpers,
//! and a consumer that accepts multipart submissions must decode them with its own
//! explicit multipart policy first. Nothing in this owner treats a body-only
//! snapshot as equivalent to the merged form map, and no hidden byte, field or
//! repetition bound is introduced.
//!
//! # Order of observations
//!
//! The caller's role normalizer is invoked at specific points, and an observable
//! callback makes that order part of the contract. Collection builds the whole item
//! (so the normalizer runs) before a blank asset identity is skipped; item
//! conversion trims the asset identity and skips before the normalizer runs; dedupe
//! normalizes the role of every kept entry, so item conversion and the
//! attach/promote paths normalize an entry twice. No shortcut pass or
//! single-normalization rewrite is applied.

use std::collections::BTreeSet;
use std::sync::Arc;

use effect_server::ByteFormData;
use crate::commands::InputBytes;
use crate::validation::trim_unicode_whitespace;

use super::items::{
    MEDIA_ATTACHMENT_ROLE_DOWNLOAD, MEDIA_ATTACHMENT_ROLE_GALLERY, MEDIA_ATTACHMENT_ROLE_HERO,
    MEDIA_ATTACHMENT_ROLE_INLINE, MediaAttachmentFormItem, MediaAttachmentInput,
};

/// A caller-supplied attachment role normalizer.
///
/// A stored, shareable byte-preserving callback (`Arc`). The callback receives the
/// role exactly as supplied — it is not pre-trimmed — and this owner trims its
/// result afterwards.
pub type RoleNormalizer = Arc<dyn Fn(&[u8]) -> InputBytes + Send + Sync + 'static>;

/// The borrowed form of a [`RoleNormalizer`], and how an absent normalizer is
/// spelled.
///
/// `None` means the role is trimmed and otherwise left unchanged, so a
/// consumer-defined role such as `"cover"` stays `"cover"`.
pub type RoleNormalizerRef<'a> = &'a (dyn Fn(&[u8]) -> InputBytes + Send + Sync);

/// The hero and gallery role names plus the caller's role normalizer.
///
/// The zero value has both role names empty (so the declared `hero`/`gallery` names
/// are used) and no normalizer (so roles are only trimmed).
#[derive(Clone, Default)]
pub struct MediaAttachmentRoleOptions {
    /// The role name treated as the single hero role; blank means `hero`.
    pub hero_role: String,
    /// The role name non-hero entries are downgraded to; blank means `gallery`.
    pub gallery_role: String,
    /// The caller's role normalizer; absent means trim only.
    pub normalize_role: Option<RoleNormalizer>,
}

/// Read one repeated form value with the legacy single-value fallback.
///
/// The primary name wins whenever it carries **any** entry, including entries that
/// are all blank, and only then does the fallback name apply. `None` is an absent
/// request and yields no values.
///
/// The values are returned as stored: no entry is trimmed, dropped or reordered
/// here, because the merged form map is not normalized.
#[must_use]
pub fn form_values_with_fallback<'a>(
    form: Option<&'a ByteFormData>,
    primary: &str,
    fallback: &str,
) -> &'a [Vec<u8>] {
    let Some(form) = form else {
        return &[];
    };
    let primary_values = form.values.get(primary.as_bytes()).map_or(&[][..], Vec::as_slice);
    if primary_values.is_empty() {
        form.values.get(fallback.as_bytes()).map_or(&[][..], Vec::as_slice)
    } else {
        primary_values
    }
}

/// Collect ordered attachment inputs from decoded form values.
///
/// Six repeated columns are read — asset identities, positions, roles, title
/// overrides, captions and alternative-text overrides — each with a primary name
/// and a legacy fallback name. One row is built per column position up
/// to the longest column, so a row whose asset identity is blank is still built
/// first (the role normalizer runs for it) and only then skipped. The result is
/// stable-sorted by the parsed position and then by the original column index; no
/// deduplication happens here.
///
/// The position parse trims the value and accepts a trimmed signed integer; a
/// malformed or negative value falls back to the row's original column index.
#[must_use]
pub fn collect_media_attachment_inputs(
    form: Option<&ByteFormData>,
    normalize_role: Option<RoleNormalizerRef<'_>>,
) -> Vec<MediaAttachmentInput> {
    let asset_ids =
        form_values_with_fallback(form, "attachment_ids[]", "media_attachment_asset_id");
    let positions =
        form_values_with_fallback(form, "attachment_positions[]", "media_attachment_position");
    let roles = form_values_with_fallback(form, "attachment_roles[]", "media_attachment_role");
    let title_overrides = form_values_with_fallback(
        form,
        "attachment_titles[]",
        "media_attachment_title_override",
    );
    let captions =
        form_values_with_fallback(form, "attachment_captions[]", "media_attachment_caption");
    let alt_overrides =
        form_values_with_fallback(form, "attachment_alts[]", "media_attachment_alt_override");

    let count = [
        asset_ids.len(),
        positions.len(),
        roles.len(),
        title_overrides.len(),
        captions.len(),
        alt_overrides.len(),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);

    let mut collected: Vec<(i64, usize, MediaAttachmentInput)> = Vec::with_capacity(count);
    for index in 0..count {
        // The item is constructed completely — which invokes the caller's role
        // normalizer — before a blank asset identity is skipped.
        let item = MediaAttachmentInput {
            asset_id: InputBytes::from(trim_unicode_whitespace(slice_string_value(asset_ids, index))),
            role: normalize_role_with(slice_string_value(roles, index), normalize_role),
            title_override: InputBytes::from(trim_unicode_whitespace(slice_string_value(title_overrides, index))),
            caption: InputBytes::from(trim_unicode_whitespace(slice_string_value(captions, index))),
            alt_override: InputBytes::from(trim_unicode_whitespace(slice_string_value(alt_overrides, index))),
        };
        if item.asset_id.is_empty() {
            continue;
        }
        let position =
            parse_attachment_position(slice_string_value(positions, index), index as i64);
        collected.push((position, index, item));
    }
    collected.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    collected.into_iter().map(|(_, _, item)| item).collect()
}

/// Derive deduplicated attachment inputs from presented form items.
///
/// Each item's asset identity is trimmed and a blank identity is skipped **without**
/// invoking the role normalizer; every kept row trims its title override, caption
/// and alternative-text override, normalizes its role, and then passes through the
/// same dedupe as [`dedupe_media_attachment_inputs`] — so the normalizer runs twice
/// for a kept row.
#[must_use]
pub fn media_attachment_inputs_from_items(
    form_items: &[MediaAttachmentFormItem],
    normalize_role: Option<RoleNormalizerRef<'_>>,
) -> Vec<MediaAttachmentInput> {
    let mut inputs = Vec::with_capacity(form_items.len());
    for item in form_items {
        let asset_id = item.asset_id.trim();
        if asset_id.is_empty() {
            continue;
        }
        inputs.push(MediaAttachmentInput {
            asset_id: asset_id.to_owned().into(),
            role: normalize_role_with(item.role.as_bytes(), normalize_role),
            title_override: item.title_override.trim().to_owned().into(),
            caption: item.caption.trim().to_owned().into(),
            alt_override: item.alt_override.trim().to_owned().into(),
        });
    }
    dedupe_media_attachment_inputs(&inputs, normalize_role)
}

/// Preserve the authoring role choices and the gallery default.
///
/// The value is trimmed and lowercased, and only the exact `hero`, `inline` and
/// `download` spellings are kept; every other value — including a consumer-defined
/// role — becomes `gallery`. This is the owner's own defaulting rule and is not the
/// generic trimming used elsewhere in this module.
#[must_use]
pub fn normalize_media_attachment_role(role: impl AsRef<[u8]>) -> String {
    match crate::byte_text::lowercase(trim_unicode_whitespace(role.as_ref())).as_str() {
        MEDIA_ATTACHMENT_ROLE_HERO => MEDIA_ATTACHMENT_ROLE_HERO.to_owned(),
        MEDIA_ATTACHMENT_ROLE_INLINE => MEDIA_ATTACHMENT_ROLE_INLINE.to_owned(),
        MEDIA_ATTACHMENT_ROLE_DOWNLOAD => MEDIA_ATTACHMENT_ROLE_DOWNLOAD.to_owned(),
        _ => MEDIA_ATTACHMENT_ROLE_GALLERY.to_owned(),
    }
}

/// Remove blank and repeated asset identities, keeping the first occurrence.
///
/// A blank trimmed asset identity is dropped. The first entry for each trimmed asset
/// identity wins and its identity is stored trimmed; the role is normalized with the
/// caller's normalizer (trimmed afterwards, or only trimmed when it is absent) and
/// the title override, caption and alternative-text override are **not** touched
/// here — the offering paths trim their own metadata first. A repeated asset
/// identity does not invoke the normalizer for the dropped entry.
#[must_use]
pub fn dedupe_media_attachment_inputs(
    inputs: &[MediaAttachmentInput],
    normalize_role: Option<RoleNormalizerRef<'_>>,
) -> Vec<MediaAttachmentInput> {
    let mut seen: BTreeSet<InputBytes> = BTreeSet::new();
    let mut deduped = Vec::with_capacity(inputs.len());
    for item in inputs {
        let asset_id = item.asset_id.trim();
        if asset_id.is_empty() {
            continue;
        }
        if !seen.insert(asset_id.to_owned()) {
            continue;
        }
        deduped.push(MediaAttachmentInput {
            asset_id: asset_id.to_owned().into(),
            role: normalize_role_with(item.role.as_bytes(), normalize_role),
            title_override: item.title_override.clone(),
            caption: item.caption.clone(),
            alt_override: item.alt_override.clone(),
        });
    }
    deduped
}

/// Add asset identities under one role.
///
/// The current inputs are deduped first and the requested role is normalized with
/// the same normalizer. For the configured hero role the first trimmed nonblank
/// supplied identity becomes a **fresh** hero entry — the selected entry's previous
/// metadata is not retained, and any previous entry with that identity is omitted —
/// while every older hero entry is downgraded to the configured gallery role. For
/// every other role each new identity is appended once, in the supplied order, when
/// it is not already present under any role. Both branches finish with the same
/// dedupe.
#[must_use]
pub fn attach_asset_ids_to_attachment_inputs(
    current: &[MediaAttachmentInput],
    asset_ids: &[InputBytes],
    role: impl AsRef<[u8]>,
    options: &MediaAttachmentRoleOptions,
) -> Vec<MediaAttachmentInput> {
    let normalize_role = options.normalize_role.as_deref();
    let current = dedupe_media_attachment_inputs(current, normalize_role);
    let role = normalize_role_with(role.as_ref(), normalize_role);
    let hero = InputBytes::from(media_attachment_hero_role(options));
    let gallery = InputBytes::from(media_attachment_gallery_role(options));

    if role == hero {
        let Some(selected) = asset_ids
            .iter()
            .map(|asset_id| asset_id.trim())
            .find(|asset_id| !asset_id.is_empty())
        else {
            return current;
        };
        let mut next = Vec::with_capacity(current.len() + 1);
        next.push(MediaAttachmentInput {
            asset_id: selected.to_owned().into(),
            role: hero.clone().into(),
            ..MediaAttachmentInput::default()
        });
        for mut item in current {
            if item.asset_id == selected {
                continue;
            }
            if item.role == hero {
                item.role = gallery.clone().into();
            }
            next.push(item);
        }
        return dedupe_media_attachment_inputs(&next, normalize_role);
    }

    let mut next = current;
    for asset_id in asset_ids {
        let asset_id = InputBytes::from(trim_unicode_whitespace(asset_id.as_ref()));
        if asset_id.is_empty() || media_attachment_input_index(&next, &asset_id).is_some() {
            continue;
        }
        next.push(MediaAttachmentInput {
            asset_id: asset_id.to_owned().into(),
            role: role.clone(),
            ..MediaAttachmentInput::default()
        });
    }
    dedupe_media_attachment_inputs(&next, normalize_role)
}

/// Promote one attachment to the configured hero role.
///
/// A blank supplied identity only dedupes the current inputs. Otherwise the current
/// inputs are deduped, the named entry is looked up and, when present, **keeps its
/// own metadata**, receives the configured hero role, moves to the front, and every
/// other entry that held the hero role is downgraded to the configured gallery role.
/// An absent identity leaves the deduped inputs unchanged. The result is deduped
/// once more.
#[must_use]
pub fn promote_attachment_input(
    current: &[MediaAttachmentInput],
    asset_id: impl AsRef<[u8]>,
    options: &MediaAttachmentRoleOptions,
) -> Vec<MediaAttachmentInput> {
    let normalize_role = options.normalize_role.as_deref();
    let asset_id = InputBytes::from(trim_unicode_whitespace(asset_id.as_ref()));
    if asset_id.is_empty() {
        return dedupe_media_attachment_inputs(current, normalize_role);
    }
    let current = dedupe_media_attachment_inputs(current, normalize_role);
    let Some(index) = media_attachment_input_index(&current, &asset_id) else {
        return current;
    };
    let hero = InputBytes::from(media_attachment_hero_role(options));
    let gallery = InputBytes::from(media_attachment_gallery_role(options));
    let Some(mut selected) = current.get(index).cloned() else {
        return current;
    };
    selected.role = hero.clone().into();
    let mut next = Vec::with_capacity(current.len());
    next.push(selected);
    for (position, mut item) in current.into_iter().enumerate() {
        if position == index {
            continue;
        }
        if item.role == hero {
            item.role = gallery.clone().into();
        }
        next.push(item);
    }
    dedupe_media_attachment_inputs(&next, normalize_role)
}

/// Remove one attachment by asset identity.
///
/// A blank supplied identity only dedupes the current inputs. Otherwise the deduped
/// inputs are filtered by the trimmed asset identity, so the remaining order and the
/// source's dedupe behavior are preserved.
#[must_use]
pub fn remove_attachment_input(
    current: &[MediaAttachmentInput],
    asset_id: impl AsRef<[u8]>,
    normalize_role: Option<RoleNormalizerRef<'_>>,
) -> Vec<MediaAttachmentInput> {
    let asset_id = InputBytes::from(trim_unicode_whitespace(asset_id.as_ref()));
    let deduped = dedupe_media_attachment_inputs(current, normalize_role);
    if asset_id.is_empty() {
        return deduped;
    }
    deduped
        .into_iter()
        .filter(|item| item.asset_id != asset_id)
        .collect()
}

/// Find the position of an asset identity, or absence.
///
/// Each stored asset identity is trimmed before comparison and the supplied identity
/// is compared as given; the first match wins. Absence is [`Option::None`].
#[must_use]
pub fn media_attachment_input_index(
    inputs: &[MediaAttachmentInput],
    asset_id: impl AsRef<[u8]>,
) -> Option<usize> {
    inputs
        .iter()
        .position(|item| item.asset_id.trim().as_bytes() == asset_id.as_ref())
}

/// Read the unique selected asset identities from decoded form values.
///
/// The primary `selected_asset_ids[]` column wins whenever it carries any entry,
/// including entries that are all blank, and only otherwise does the legacy
/// `selected_asset_id` column apply. Entries are trimmed, blank entries are dropped
/// and the first occurrence of each identity decides its position. `None` is an
/// absent request and yields no identities.
#[must_use]
pub fn media_selected_asset_ids(form: Option<&ByteFormData>) -> Vec<InputBytes> {
    let selected = form_values_with_fallback(form, "selected_asset_ids[]", "selected_asset_id");
    let mut ids = Vec::with_capacity(selected.len());
    let mut seen: BTreeSet<InputBytes> = BTreeSet::new();
    for item in selected {
        let id = InputBytes::from(trim_unicode_whitespace(item));
        if id.is_empty() {
            continue;
        }
        if seen.insert(id.to_owned()) {
            ids.push(id.to_owned());
        }
    }
    ids
}

/// Render the media shortcode text for one asset and role.
///
/// Both inputs are trimmed, a blank identity yields no shortcode, and a blank role
/// becomes `inline`. The result is the exact `[media id="<asset>" role=<role>]`
/// string: the role is unquoted and nothing is escaped or whitelisted. This is
/// shortcode text, not HTML, so Maud is not involved and no markup is constructed
/// here.
#[must_use]
pub fn media_shortcode(asset_id: impl AsRef<[u8]>, role: impl AsRef<[u8]>) -> InputBytes {
    let asset_id = trim_unicode_whitespace(asset_id.as_ref());
    if asset_id.is_empty() { return InputBytes::default(); }
    let role = trim_unicode_whitespace(role.as_ref());
    let role = if role.is_empty() { b"inline" } else { role };
    InputBytes::from([b"[media id=\"".as_slice(), asset_id, b"\" role=", role, b"]"].concat())
}

/// The configured hero role, or the declared `hero` name.
fn media_attachment_hero_role(options: &MediaAttachmentRoleOptions) -> String {
    first_non_blank(&[options.hero_role.as_str(), MEDIA_ATTACHMENT_ROLE_HERO])
}

/// The configured gallery role, or the declared `gallery` name.
fn media_attachment_gallery_role(options: &MediaAttachmentRoleOptions) -> String {
    first_non_blank(&[options.gallery_role.as_str(), MEDIA_ATTACHMENT_ROLE_GALLERY])
}

/// Apply the caller's role normalizer, or only trim when it is absent.
///
/// The callback receives the raw value and this owner trims its result, so a
/// consumer-defined role is preserved (trimmed) when no normalizer and no
/// [`normalize_media_attachment_role`] defaulting is involved.
fn normalize_role_with(raw: &[u8], normalize_role: Option<RoleNormalizerRef<'_>>) -> InputBytes {
    match normalize_role {
        Some(normalize_role) => normalize_role(raw).trim().to_owned(),
        None => InputBytes::from(trim_unicode_whitespace(raw)),
    }
}

/// Parse a trimmed signed integer position, falling back to the original index.
///
/// A trimmed signed integer is accepted; a malformed or negative value becomes the
/// row's original index, so neither a negative position nor a rejected value
/// reorders the row ahead of the parsed ones.
fn parse_attachment_position(raw: &[u8], fallback: i64) -> i64 {
    match std::str::from_utf8(trim_unicode_whitespace(raw)).ok().and_then(|value| value.parse::<i64>().ok()) {
        Some(value) if value >= 0 => value,
        _ => fallback,
    }
}

/// One column value at a position, or empty bytes when the column is shorter.
fn slice_string_value(values: &[Vec<u8>], index: usize) -> &[u8] {
    values.get(index).map_or(&[], Vec::as_slice)
}

/// The first trimmed non-blank value, or the empty string.
///
/// This is the neutral first-non-blank primitive; it carries no attachment policy
/// and is kept local to this owner rather than reached for across an owner boundary.
fn first_non_blank(values: &[&str]) -> String {
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    String::new()
}
