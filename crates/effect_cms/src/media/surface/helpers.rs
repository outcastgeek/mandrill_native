//! Media surface query and state values shared by media workflows.
//!
//! These are plain caller-supplied values. They carry no store, request adapter or
//! HTML. Media picker/panel rendering and request parsing belong to later media
//! owners and use these values without redefining them.

use std::sync::Arc;
use crate::commands::InputBytes;
use crate::validation::trim_unicode_whitespace;

use effect_server::{ByteFormData, Request};

use crate::attachments::{MediaAttachmentFormItem, MediaAttachmentManagerProps, MediaSurfaceMounts};
use crate::queries::request_query_value;

use super::{MediaQuery, MediaSurfaceOptions, MediaSurfaceRequest, MountsResolver};

/// Parse a media surface request from merged form values and the query.
///
/// The decoded form value wins when it is non-blank; otherwise the request query is
/// read. Scope and role pass through their optional normalizers.
#[must_use]
pub fn parse_media_surface_request(
    request: &Request,
    form: Option<&ByteFormData>,
    options: &MediaSurfaceOptions,
) -> MediaSurfaceRequest {
    let value = |key: &str| -> InputBytes {
        if let Some(form) = form
            && let Some(values) = form.values.get(key.as_bytes())
            && let Some(value) = values.first()
            && !trim_unicode_whitespace(value).is_empty()
        {
            return InputBytes::from(trim_unicode_whitespace(value));
        }
        query_bytes(request, key)
    };
    let normalize = |raw: InputBytes, normalizer: Option<&Arc<dyn Fn(&[u8]) -> InputBytes + Send + Sync>>| {
        match normalizer {
            Some(normalizer) => normalizer(raw.trim().as_bytes()),
            None => raw.trim().to_owned(),
        }
        .trim()
        .to_owned()
    };
    MediaSurfaceRequest {
        scope: normalize(value("scope"), options.normalize_scope.as_ref()),
        target_id: value("target"),
        owner_form_id: value("owner_form_id"),
        owner_content_id: value("owner_content_id"),
        role: normalize(value("role"), options.normalize_role.as_ref()),
        root_id: value("root_id"),
        create_root_id: value("create_root_id"),
        picker_root_id: value("picker_root_id"),
    }
}

/// Whether the request selects the inline surface.
#[must_use]
pub fn media_surface_is_inline(request: &Request, inline_surface: &str) -> bool {
    request_query_value(request, "surface")
        .as_bytes().eq_ignore_ascii_case(inline_surface.trim().as_bytes())
}

/// Parse a media query from the request and an optional merged form.
#[must_use]
pub fn media_query_from_request(
    request: &Request,
    form: Option<&ByteFormData>,
    limit: i64,
) -> MediaQuery {
    let limit = if limit <= 0 { 1 } else { limit };
    let value = |key: &str| -> InputBytes {
        if let Some(form) = form
            && let Some(values) = form.values.get(key.as_bytes())
            && let Some(value) = values.first()
            && !trim_unicode_whitespace(value).is_empty()
        {
            return InputBytes::from(trim_unicode_whitespace(value));
        }
        query_bytes(request, key)
    };
    let page = value("page").to_str().ok().and_then(|value| value.parse::<i64>().ok())
        .filter(|page| *page > 0)
        .unwrap_or(1);
    MediaQuery {
        search: value("search"),
        kind: value("kind"),
        status: value("status"),
        sort: value("sort"),
        limit,
        offset: (page - 1).wrapping_mul(limit),
    }
}

/// Resolve the surface mounts, applying request overrides in source order.
#[must_use]
pub fn resolve_media_surface_mounts(
    request: Option<&Request>,
    form: Option<&ByteFormData>,
    defaults: MediaSurfaceMounts,
) -> MediaSurfaceMounts {
    let Some(request) = request else {
        return defaults;
    };
    let value = |key: &str| -> InputBytes {
        if let Some(form) = form
            && let Some(values) = form.values.get(key.as_bytes())
            && let Some(value) = values.first()
            && !trim_unicode_whitespace(value).is_empty()
        {
            return InputBytes::from(trim_unicode_whitespace(value));
        }
        query_bytes(request, key)
    };
    let mut mounts = defaults;
    let mut root_id = value("root_id");
    let mut create_root_id = value("create_root_id");
    let picker_root_id = value("picker_root_id");
    if !picker_root_id.is_empty() {
        create_root_id = first_non_blank(&[create_root_id, root_id.clone()]);
        root_id = picker_root_id.clone();
    }
    let mut shortcode_root_id = value("shortcode_root_id");
    if shortcode_root_id.is_empty() && picker_root_id.is_empty() && create_root_id.is_empty() {
        shortcode_root_id = query_bytes(request, "root_id");
    }
    let shortcode_dialog_id = value("dialog_id");
    let owner_form_id = value("owner_form_id");
    let section_id = first_non_blank(&[value("target"), value("section_id")]);
    if !owner_form_id.is_empty() {
        mounts.owner_form_id = owner_form_id;
    }
    if !section_id.is_empty() {
        mounts.section_id = section_id;
    }
    if !root_id.is_empty() {
        mounts.picker_root_id = root_id;
    }
    if !create_root_id.is_empty() {
        mounts.create_root_id = create_root_id;
    }
    if !shortcode_root_id.is_empty() {
        mounts.shortcode_root_id = shortcode_root_id;
    }
    if !shortcode_dialog_id.is_empty() {
        mounts.shortcode_dialog_id = shortcode_dialog_id;
    }
    mounts
}

/// Build one media fragment path with the declared query parameters.
#[must_use]
pub fn media_fragment_path(base_path: &str, params: &[(String, InputBytes)]) -> String {
    let base_path = base_path.trim();
    let mut params = params.iter().collect::<Vec<_>>();
    params.sort_by(|left, right| left.0.cmp(&right.0));
    let query = params
        .into_iter()
        .map(|(key, value)| format!("{}={}", encode(key.as_bytes()), encode(value.trim().as_bytes())))
        .collect::<Vec<_>>()
        .join("&");
    if base_path.is_empty() {
        return format!("?{query}");
    }
    let separator = if base_path.contains('?') { '&' } else { '?' };
    format!("{base_path}{separator}{query}")
}

/// Build the picker fragment target for a surface.
#[must_use]
pub fn media_create_fragment_path(
    base_path: &str,
    surface: impl AsRef<[u8]>,
    root_id: impl AsRef<[u8]>,
    picker_root_id: impl AsRef<[u8]>,
    scope: impl AsRef<[u8]>,
    section_id: impl AsRef<[u8]>,
    owner_form_id: impl AsRef<[u8]>,
    owner_content_id: impl AsRef<[u8]>,
    role: impl AsRef<[u8]>,
) -> String {
    let mut params = vec![
        ("surface".to_owned(), InputBytes::from(surface.as_ref())),
        ("root_id".to_owned(), InputBytes::from(root_id.as_ref())),
        ("picker_root_id".to_owned(), InputBytes::from(picker_root_id.as_ref())),
        ("scope".to_owned(), InputBytes::from(scope.as_ref())),
        ("target".to_owned(), InputBytes::from(section_id.as_ref())),
        ("owner_form_id".to_owned(), InputBytes::from(owner_form_id.as_ref())),
        ("owner_content_id".to_owned(), InputBytes::from(owner_content_id.as_ref())),
    ];
    if !trim_unicode_whitespace(role.as_ref()).is_empty() {
        params.push(("role".to_owned(), InputBytes::from(role.as_ref())));
    }
    media_fragment_path(base_path, &params)
}

/// Build the picker fragment target for a surface.
#[must_use]
pub fn media_picker_fragment_path(
    base_path: &str,
    root_id: impl AsRef<[u8]>,
    create_root_id: impl AsRef<[u8]>,
    scope: impl AsRef<[u8]>,
    section_id: impl AsRef<[u8]>,
    owner_form_id: impl AsRef<[u8]>,
    owner_content_id: impl AsRef<[u8]>,
    role: impl AsRef<[u8]>,
) -> String {
    let mut params = vec![
        ("root_id".to_owned(), InputBytes::from(root_id.as_ref())),
        ("create_root_id".to_owned(), InputBytes::from(create_root_id.as_ref())),
        ("scope".to_owned(), InputBytes::from(scope.as_ref())),
        ("target".to_owned(), InputBytes::from(section_id.as_ref())),
        ("owner_form_id".to_owned(), InputBytes::from(owner_form_id.as_ref())),
        ("owner_content_id".to_owned(), InputBytes::from(owner_content_id.as_ref())),
    ];
    if !trim_unicode_whitespace(role.as_ref()).is_empty() {
        params.push(("role".to_owned(), InputBytes::from(role.as_ref())));
    }
    media_fragment_path(base_path, &params)
}

/// Build the attachment manager props from the request and resolved mounts.
#[must_use]
pub fn build_media_attachment_manager_from_request(
    request: &Request,
    form: Option<&effect_server::ByteFormData>,
    scope: &InputBytes,
    owner_content_id: &InputBytes,
    attachments: Vec<MediaAttachmentFormItem>,
    mounts_for_scope: Option<&MountsResolver>,
    picker_base_path: &str,
    create_base_path: &str,
    mutation_url: &str,
    inline_surface: &str,
) -> MediaAttachmentManagerProps {
    let defaults = mounts_for_scope.map(|resolve| resolve(scope.as_bytes())).unwrap_or_default();
    let mounts = resolve_media_surface_mounts(Some(request), form, defaults);
    MediaAttachmentManagerProps {
        scope: scope.trim().to_owned(),
        owner_content_id: owner_content_id.trim().to_owned(),
        picker_url: media_picker_fragment_path(
            picker_base_path,
            &mounts.picker_root_id,
            &mounts.create_root_id,
            scope,
            &mounts.section_id,
            &mounts.owner_form_id,
            owner_content_id,
            "",
        ),
        create_url: media_create_fragment_path(
            create_base_path,
            inline_surface,
            &mounts.create_root_id,
            &mounts.picker_root_id,
            scope,
            &mounts.section_id,
            &mounts.owner_form_id,
            owner_content_id,
            "",
        ),
        mounts,
        mutation_url: mutation_url.trim().to_owned(),
        attachments,
    }
}

fn encode(value: &[u8]) -> String {
    let mut encoded = String::new();
    for &byte in value {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

fn first_non_blank(values: &[InputBytes]) -> InputBytes {
    values.iter().map(InputBytes::trim).find(|v| !v.is_empty()).unwrap_or_default()
}
fn query_bytes(request: &Request, key: &str) -> InputBytes {
    request_query_value(request, key)
}
