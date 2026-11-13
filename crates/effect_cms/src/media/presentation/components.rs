//! Small media HTML primitives authored with Maud.
//!
//! These preserve the source element identities, classes and hidden payload field
//! names for the mount root, attachment payload inputs, preview frame, type badge
//! and provider chip. The complete picker/library/manager templates compose these
//! primitives and the existing shared component props.

use super::view_text;

use effect_server::Request;
use maud::{Markup, html};

use crate::attachments::MediaAttachmentFormItem;
use crate::attachments::normalize_media_attachment_role;
use crate::localization::TranslateKeyFunc;
use crate::media::assets::{MEDIA_KIND_DOCUMENT, MEDIA_KIND_IMAGE, MEDIA_KIND_VIDEO};
use crate::media::presentation::labels::media_kind_label;

/// Render one empty mount root.
#[must_use]
pub fn media_mount_root(id: &str) -> Markup {
    let id = id.trim();
    if id.is_empty() {
        return Markup::default();
    }
    html! { div id=(id) {} }
}

/// Render the hidden attachment payload fields.
#[must_use]
pub fn media_attachment_payload_inputs(items: &[MediaAttachmentFormItem]) -> Markup {
    html! {
        @for (index, item) in items.iter().enumerate() {
            input type="hidden" name="attachment_ids[]" value=(view_text(item.asset_id.trim()));
            input type="hidden" name="attachment_positions[]" value=(index.to_string());
            input type="hidden" name="attachment_roles[]" value=(media_payload_role(&item.role));
            input type="hidden" name="attachment_titles[]" value=(view_text(item.title_override.trim()));
            input type="hidden" name="attachment_captions[]" value=(view_text(item.caption.trim()));
            input type="hidden" name="attachment_alts[]" value=(view_text(item.alt_override.trim()));
        }
    }
}

/// Render the media preview frame or its kind fallback.
#[must_use]
pub fn media_preview_frame(
    request: Option<&Request>,
    translate: Option<&TranslateKeyFunc<Request>>,
    preview_url: &str,
    alt: &str,
    kind: &str,
    class: &str,
    alt_fallback: &str,
) -> Markup {
    let preview_url = preview_url.trim();
    if !preview_url.is_empty() {
        let alt = if alt.trim().is_empty() {
            alt_fallback.trim().to_owned()
        } else {
            alt.trim().to_owned()
        };
        let class = format!(
            "aspect-video w-full rounded-2xl border border-border bg-muted object-cover {}",
            class.trim()
        );
        return html! { img src=(preview_url) alt=(alt) class=(class.trim()); };
    }
    let label = request
        .map(|request| media_kind_label(translate, request, kind))
        .unwrap_or_else(|| normalized_kind_label(translate, kind));
    let class = format!(
        "flex aspect-video w-full items-center justify-center rounded-2xl border border-dashed border-border bg-muted/20 px-4 text-center text-xs font-semibold uppercase tracking-[0.22em] text-muted-foreground {}",
        class.trim()
    );
    html! { div class=(class.trim()) { (label) } }
}

/// Render the media kind badge.
#[must_use]
pub fn media_type_badge(
    request: Option<&Request>,
    translate: Option<&TranslateKeyFunc<Request>>,
    kind: &str,
    class: &str,
) -> Markup {
    let label = request
        .map(|request| media_kind_label(translate, request, kind))
        .unwrap_or_else(|| normalized_kind_label(translate, kind));
    let base = match kind.trim().to_ascii_lowercase().as_str() {
        MEDIA_KIND_VIDEO => "border-border bg-info-muted text-info-foreground",
        MEDIA_KIND_DOCUMENT => "border-border bg-warning-muted text-warning-foreground",
        _ => "border-border bg-success-muted text-success-foreground",
    };
    let class = format!("{base} {}", class.trim());
    web_components::components::unstyled::badge::Badge(
        &web_components::components::unstyled::badge::BadgeProps {
            variant: web_components::components::shared::VARIANT_OUTLINE,
            size: web_components::components::shared::SIZE_SM, pill: true,
            class: class.trim().into(), ..Default::default()
        }, html!{(label)},
    )
}

/// Render one provider chip.
#[must_use]
pub fn provider_chip(provider: &str, class: &str) -> Markup {
    let provider = provider.trim();
    if provider.is_empty() {
        return Markup::default();
    }
    let class = format!(
        "border-primary/30 bg-primary/10 text-primary {}",
        class.trim()
    );
    web_components::components::unstyled::badge::Badge(
        &web_components::components::unstyled::badge::BadgeProps {
            variant: web_components::components::shared::VARIANT_OUTLINE,
            size: web_components::components::shared::SIZE_SM, pill: true,
            class: class.trim().into(), ..Default::default()
        }, html!{(provider)},
    )
}

/// Render the kind label through the source key.
#[must_use]
pub fn normalized_kind_label(
    translate: Option<&TranslateKeyFunc<Request>>,
    kind: &str,
) -> String {
    let key = match kind.trim().to_ascii_lowercase().as_str() {
        MEDIA_KIND_VIDEO => "media.kind.video",
        MEDIA_KIND_DOCUMENT => "media.kind.document",
        _ => "media.kind.image",
    };
    match translate {
        Some(translate) => translate_without_request(translate, key),
        None => key.to_owned(),
    }
}

fn translate_without_request(translate: &TranslateKeyFunc<Request>, key: &str) -> String {
    // Translation requires the request-scoped port; callers that need localization
    // pass the request-aware functions above. This fallback keeps the key stable.
    let _ = translate;
    key.to_owned()
}

fn media_payload_role(role: impl AsRef<[u8]>) -> String {
    let role = normalize_media_attachment_role(role);
    if role.is_empty() {
        "gallery".to_owned()
    } else {
        role
    }
}

/// Keep the source media-kind import referenced in the primitive label rule.
const _: &str = MEDIA_KIND_IMAGE;
