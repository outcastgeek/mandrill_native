//! Media labels, reference counts and stable presenter identities.

use effect_server::Request;

use crate::localization::TranslateKeyFunc;
use crate::media::assets::{normalize_media_kind, normalize_media_status};

/// The picker attach-selected label key.
pub const MEDIA_PICKER_ATTACH_SELECTED_LABEL_KEY: &str = "media.picker.attach_selected";
/// The attachments section title key.
pub const MEDIA_ATTACHMENTS_SECTION_TITLE_KEY: &str = "media.attachments.section_title";
/// The dashboard tools title key.
pub const MEDIA_DASHBOARD_TOOLS_TITLE_KEY: &str = "contentmgmt.media_dashboard.tools";
/// The dashboard recent-page query parameter.
pub const MEDIA_DASHBOARD_RECENT_PAGE_PARAM: &str = "recent_media_page";
/// The picker hero context key.
pub const MEDIA_PICKER_CONTEXT_HERO_KEY: &str = "media.picker.context.hero";

/// Resolve one translation key with the source's trimmed-key fallback.
#[must_use]
pub fn media_translate(
    translate: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    key: &str,
) -> String {
    let key = key.trim();
    if key.is_empty() {
        return String::new();
    }
    match translate {
        Some(translate) => translate(request, key),
        None => key.to_owned(),
    }
}

/// Resolve a kind label through its canonical key.
#[must_use]
pub fn media_kind_label(
    translate: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    kind: &str,
) -> String {
    media_translate(
        translate,
        request,
        &format!("media.kind.{}", normalize_media_kind(kind)),
    )
}

/// Resolve a status label through its canonical key.
#[must_use]
pub fn media_status_label(
    translate: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    status: &str,
) -> String {
    media_translate(
        translate,
        request,
        &format!("media.status.{}", normalize_media_status(status)),
    )
}

/// Resolve the reference-count label.
#[must_use]
pub fn media_reference_count_label(
    translate: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    count: i64,
) -> String {
    let key = if count == 1 {
        "media.reference_count.one"
    } else {
        "media.reference_count.many"
    };
    media_translate(translate, request, key)
}
