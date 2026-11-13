//! Media inline-create presentation props.

use super::view_text;

use crate::media::assets::{
    MEDIA_KIND_DOCUMENT, MEDIA_KIND_IMAGE, MEDIA_KIND_VIDEO, MEDIA_SOURCE_KIND_REMOTE,
    MediaAssetURLResolver, normalize_media_kind, normalize_media_source_kind,
};
use crate::media::presentation::contracts::{
    MediaInlineCreatePreviewOptions, MediaInlineCreatePreviewProps, MediaInlineCreateProps,
    MediaInlineCreatePropsOptions,
};
use crate::media::presentation::picker::media_form_or_query_value;
use crate::queries::first_non_blank;

/// Build the inline-create props.
#[must_use]
pub fn build_media_inline_create_props(options: &MediaInlineCreatePropsOptions) -> MediaInlineCreateProps {
    let root_id = first_non_blank(&[
        view_text(&options.surface.root_id),
        options.default_root_id.clone(),
    ]);
    let role_default = first_non_blank(&[
        view_text(&options.surface.role),
        options.default_role.clone(),
    ]);
    let duration = match options.input.duration.as_ref().map(|value|value.load(std::sync::atomic::Ordering::SeqCst)) {
        Some(value) if value > 0 => value.to_string(),
        _ => String::new(),
    };
    MediaInlineCreateProps {
        dialog_id: format!("{root_id}-dialog"),
        preview_root_id: media_inline_create_preview_root_id(&root_id),
        preview: build_media_inline_create_preview_props(&MediaInlineCreatePreviewOptions {
            request: options.request.clone(),
            form: options.form.clone(),
            input: options.input.clone(),
            root_id: root_id.clone(),
            resolve_url: options.resolve_url.clone(),
        }),
        root_id,
        picker_root_id: view_text(&options.surface.picker_root_id),
        scope: view_text(&options.surface.scope),
        role_default,
        owner_form_id: view_text(&options.surface.owner_form_id),
        owner_content_id: view_text(&options.surface.owner_content_id),
        attachment_id: view_text(&options.surface.target_id),
        action_url: options.action_url.trim().to_owned(),
        preview_url: options.preview_url.trim().to_owned(),
        error_msg: options.error_msg.trim().to_owned(),
        csrf_token: options.csrf_token.trim().to_owned(),
        csrf_field_name: options.csrf_field_name.trim().to_owned(),
        attachments: options.attachments.clone(),
        title: view_text(options.input.title.trim()),
        kind: normalize_media_kind(&options.input.kind),
        source_kind: normalize_media_source_kind(&options.input.source_kind),
        url: view_text(options.input.url.trim()),
        description: view_text(options.input.description.trim()),
        thumbnail_url: view_text(options.input.thumbnail_url.trim()),
        alt: view_text(options.input.alt.trim()),
        duration,
    }
}

/// Build the inline-create preview props.
#[must_use]
pub fn build_media_inline_create_preview_props(
    options: &MediaInlineCreatePreviewOptions,
) -> MediaInlineCreatePreviewProps {
    let request = options.request.as_ref();
    let root_id = first_non_blank(&[
        media_form_or_query_value(request, options.form.as_deref(), "preview_root_id"),
        media_inline_create_preview_root_id(&first_non_blank(&[
            options.root_id.clone(),
            media_form_or_query_value(request, options.form.as_deref(), "root_id"),
        ])),
    ]);
    let kind = normalize_media_kind(&options.input.kind);
    let source_kind = normalize_media_source_kind(&options.input.source_kind);
    let show_remote_image = kind == MEDIA_KIND_IMAGE
        && source_kind == MEDIA_SOURCE_KIND_REMOTE
        && !options.input.url.trim().is_empty();
    let resolved_image_url = if show_remote_image {
        resolve_url(options.resolve_url.as_ref(), &options.input.url)
    } else {
        String::new()
    };
    MediaInlineCreatePreviewProps {
        root_id,
        provider: view_text(crate::media::assets::resolve_media_asset_form_provider(&options.input)),
        resolved_image_url,
        show_remote_image,
        show_video_hint: kind == MEDIA_KIND_VIDEO,
        show_document_hint: kind == MEDIA_KIND_DOCUMENT,
    }
}

/// Derive the inline-create preview root identity.
#[must_use]
pub fn media_inline_create_preview_root_id(root_id: &str) -> String {
    let root_id = first_non_blank(&[root_id.to_owned(), "media-create-root".to_owned()]);
    format!("{root_id}-preview")
}

fn resolve_url(resolve: Option<&MediaAssetURLResolver>, raw: impl AsRef<[u8]>) -> String {
    let raw = crate::validation::trim_unicode_whitespace(raw.as_ref());
    match resolve {
        Some(resolve) => view_text(resolve(raw).trim()),
        None => view_text(raw),
    }
}
