//! Media assets: asset/form/list values, consumer store contracts, preparation
//! options, normalization and the default media resource declaration.
//!
//! # Consumer-owned persistence
//!
//! [`MediaAssetStore`] and [`ContentMediaAttachmentStore`] are consumer-supplied
//! capabilities. Their methods return lazy effects; the CMS never creates a table,
//! SQL statement, schema, migration, pool, connection, backend or storage lifecycle.
//!
//! # Nullability and mutation
//!
//! A single-asset lookup is nullable and a list result may be absent. Create and
//! update receive a shared mutable asset handle, so the consumer store can assign an
//! identity or fields that remain visible after the effect completes, without a
//! borrowed reference crossing an asynchronous boundary.
//!
//! # Scope
//!
//! This foundation owns records, vocabularies, normalization, record preparation and
//! default definitions. Media forms, routes, runtime flows, presentation and demo
//! behavior are later production work. No HTML is produced here.
//!
//! The owner also projects stored asset and attachment records into presented
//! attachment items and exposes the store-backed owner attachment listing and
//! replacement adapters. Those adapters invoke the media attachment workflow over
//! the supplied stores; they open no store of their own.

use crate::commands::InputBytes;
use std::sync::Arc;

use effect::memory::MemoryError;
use effect::{Effect, effect, effect_sync, effect_try_sync};
use crate::content::PreparedCmsContent;

use crate::attachments::{MediaAttachmentFormItem, MediaAttachmentStoreRecord, media_shortcode};


use super::attachmentflow::{
    MediaAttachmentAssetLoader, MediaAttachmentItemBuilder, MediaAttachmentItemLoader,
    MediaAttachmentListOptions, MediaAttachmentReplaceOptions, MediaAttachmentRowBuilder,
    MediaAttachmentRowLister, MediaAttachmentRowReplacer,
    PreparedAttachmentReplaceOptions, PreparedAttachmentRowBuilder, PreparedAttachmentRowReplacer,
    PreparedAttachmentRows, list_media_attachment_form_items, list_prepared_admitted,
    list_prepared_media_attachment_form_items, replace_media_attachments_from_request,
    replace_prepared_media_attachments_from_request, reserve_attachment_list,
};

mod definition;
mod form;
mod model;
mod normalize;
mod url;

pub use self::definition::{
    MediaResourceDefinitionOptions, default_media_resource_dashboard_actions,
    default_media_resource_filters, default_media_resource_form_definition,
    default_media_resource_list_definition, new_media_resource_definition,
};
pub use self::form::{
    default_media_asset_form_input, media_asset_form_input_from_form,
    media_asset_form_input_from_prepared_form, media_asset_form_input_from_prepared_request, media_asset_form_input_from_record,
    media_asset_form_input_from_request,
};
pub(crate) use self::form::media_request_form;
pub use self::model::{
    ContentMediaAttachmentStore, DEFAULT_MEDIA_LIBRARY_PAGE_SIZE, MEDIA_KIND_DOCUMENT,
    MEDIA_KIND_IMAGE, MEDIA_KIND_VIDEO, MEDIA_SORT_ALPHA, MEDIA_SORT_NEWEST, MEDIA_SORT_OLDEST,
    MEDIA_SOURCE_KIND_EMBED, MEDIA_SOURCE_KIND_REMOTE, MEDIA_SOURCE_KIND_UPLOAD, MEDIA_STATUS_ARCHIVED,
    MEDIA_STATUS_DRAFT, MEDIA_STATUS_READY, MediaAssetEditUrl, MediaAssetFormInput,
    MediaAssetIdGenerator, MediaAssetListResult, MediaAssetNowUnix, MediaAssetRecord,
    MediaAssetStore, MediaAssetURLResolver, MediaStoreAttachmentOptions,
    PreparedMediaAttachmentStoreRecord,
};
pub use self::normalize::{
    apply_media_asset_form_record, media_looks_renderable_as_image, normalize_media_asset_ids,
    normalize_media_asset_store_record, normalize_media_kind, normalize_media_provider_name,
    normalize_media_query, normalize_media_source_kind, normalize_media_status,
    parse_optional_positive_int, parse_positive_int,
    prepare_media_attachment_store_records_for_replace, prepare_record_for_create,
    prepare_record_for_update, record_from_form, resolve_media_asset_form_provider,
    resolve_media_provider, validate_media_asset_url,
};

/// Project shortcode assets in their stored order.
#[must_use]
pub fn media_shortcode_items_from_result(
    result: Option<&MediaAssetListResult>,
    role: &str,
    image_kind: &str,
    resolve: Option<&MediaAssetURLResolver>,
    edit_url: &(dyn Fn(&MediaAssetRecord) -> String + Send + Sync),
) -> Vec<MediaAttachmentFormItem> {
    let Some(result) = result else { return Vec::new(); };
    let mut items = Vec::with_capacity(result.items.len());
    for (index, asset) in result.items.iter().enumerate() {
        let id = asset.id.trim();
        if id.is_empty() { continue; }
        let record = MediaAttachmentStoreRecord {
            id: id.to_owned().into(),
            media_asset_id: id.to_owned().into(),
            role: role.to_owned().into(),
            ..MediaAttachmentStoreRecord::default()
        };
        if let Some(item) = media_attachment_form_item_from_records(
            index, asset, &record, &edit_url(asset), image_kind, resolve,
        ) {
            items.push(item);
        }
    }
    items
}

/// Resolve an asset's primary URL.
///
/// A nonblank CDN URL wins; otherwise the source URL is passed through the optional
/// resolver and trimmed. With no resolver the trimmed source URL is returned.
#[must_use]
pub fn media_asset_primary_url(
    asset: &MediaAssetRecord,
    resolve: Option<&MediaAssetURLResolver>,
) -> InputBytes {
    let cdn_url = asset.cdn_url.trim();
    if !cdn_url.is_empty() {
        return cdn_url.to_owned();
    }
    resolve_media_asset_url(resolve, &asset.url)
}

/// Resolve an asset's thumbnail URL.
///
/// A nonblank thumbnail that resolves to an image URL wins. Otherwise an asset whose
/// trimmed kind matches the supplied image kind (ASCII case-insensitive) uses its
/// primary URL; every other asset has no thumbnail.
#[must_use]
pub fn media_asset_thumbnail_url(
    asset: &MediaAssetRecord,
    image_kind: &str,
    resolve: Option<&MediaAssetURLResolver>,
) -> InputBytes {
    let thumbnail_url = asset.thumbnail_url.trim();
    if !thumbnail_url.is_empty() {
        let thumbnail = resolve_media_asset_url(resolve, &thumbnail_url);
        if media_looks_renderable_as_image(&thumbnail) {
            return thumbnail;
        }
    }
    if asset.kind.trim().eq_ignore_ascii_case(image_kind.trim()) {
        return media_asset_primary_url(asset, resolve);
    }
    InputBytes::default()
}

/// Resolve an asset's preview URL.
///
/// The thumbnail URL wins when it is nonblank; otherwise an asset whose trimmed kind
/// matches the supplied image kind uses its primary URL, and every other asset has no
/// preview URL.
#[must_use]
pub fn media_asset_preview_url(
    asset: &MediaAssetRecord,
    image_kind: &str,
    resolve: Option<&MediaAssetURLResolver>,
) -> InputBytes {
    let preview = media_asset_thumbnail_url(asset, image_kind, resolve);
    if !preview.is_empty() {
        return preview;
    }
    if asset.kind.trim().eq_ignore_ascii_case(image_kind.trim()) {
        return media_asset_primary_url(asset, resolve);
    }
    InputBytes::default()
}

/// Resolve an asset's display name.
///
/// The trimmed title wins, then the trimmed provider name, then the trimmed source
/// URL.
#[must_use]
pub fn media_asset_display_name(asset: &MediaAssetRecord) -> InputBytes {
    first_non_blank(&[
        asset.title.clone(),
        asset.provider.clone(),
        asset.url.clone(),
    ])
}

/// Build one presented attachment item from an asset and an attachment row.
///
/// A blank trimmed asset identity yields [`None`]. Every text field is trimmed, the
/// resolved URLs use the supplied resolver, the title and alternative text use the
/// existing first-nonblank precedence, and the shortcode text is derived from the
/// asset identity and stored role. A blank row identity receives
/// `<asset identity>-<index>` as its form key.
#[must_use]
pub fn media_attachment_form_item_from_records(
    index: usize,
    asset: &MediaAssetRecord,
    attachment: &MediaAttachmentStoreRecord,
    edit_url: &str,
    image_kind: &str,
    resolve: Option<&MediaAssetURLResolver>,
) -> Option<MediaAttachmentFormItem> {
    if asset.id.trim().is_empty() {
        return None;
    }
    let mut item = MediaAttachmentFormItem {
        key: attachment.id.trim().to_owned(),
        position: i64::try_from(index).unwrap_or(i64::MAX),
        asset_id: asset.id.trim().to_owned().into(),
        kind: asset.kind.trim().to_owned(),
        role: attachment.role.trim().to_owned(),
        status: asset.status.trim().to_owned(),
        title: first_non_blank(&[
            asset.title.clone(),
            attachment.title_override.clone(),
            media_asset_display_name(asset),
        ]),
        description: asset.description.trim().to_owned(),
        provider: asset.provider.trim().to_owned(),
        url: asset.url.trim().to_owned(),
        thumbnail_url: asset.thumbnail_url.trim().to_owned(),
        resolved_url: media_asset_primary_url(asset, resolve),
        resolved_thumbnail: media_asset_thumbnail_url(asset, image_kind, resolve),
        alt: first_non_blank(&[attachment.alt_override.clone(), asset.alt.clone()]),
        title_override: attachment.title_override.trim().to_owned(),
        caption: attachment.caption.trim().to_owned(),
        alt_override: attachment.alt_override.trim().to_owned(),
        shortcode: media_shortcode(&asset.id, &attachment.role),
        edit_url: edit_url.trim().to_owned(),
    };
    if item.key.is_empty() {
        item.key = InputBytes::from([asset.id.as_bytes(), format!("-{index}").as_bytes()].concat());
    }
    Some(item)
}

/// List one owner's stored attachments as presented items.
///
/// A blank owner identity or an absent asset or attachment store yields an empty
/// result. Otherwise the owner's rows are listed, converted to attachment records,
/// resolved by identity and projected through
/// [`media_attachment_form_item_from_records`]. Rows whose conversion is absent,
/// whose trimmed asset identity is blank, or whose asset is absent from the lookup
/// are skipped rather than failing the listing.
pub fn list_media_attachment_form_items_from_store<E, R>(
    options: &MediaStoreAttachmentOptions<E, R>,
) -> Effect<Vec<MediaAttachmentFormItem>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    if options.assets.is_none() || options.attachments.is_none() || options.owner_content_id.is_empty() {
        return effect_sync! { body: Vec::new };
    }
    list_media_attachment_form_items(store_list_options(options))
}

fn store_list_options<E, R>(
    options: &MediaStoreAttachmentOptions<E, R>,
) -> MediaAttachmentListOptions<MediaAssetRecord, MediaAttachmentStoreRecord, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let kind = options.owner_type.clone();
    let owner = options.owner_content_id.clone();
    let list_rows = options.attachments.as_ref().map(|attachments| {
        let attachments = attachments.clone();
        Arc::new(move || attachments.list_by_owner(kind.clone(), owner.clone()))
            as MediaAttachmentRowLister<MediaAttachmentStoreRecord, E, R>
    });
    let load_assets_by_id = options.assets.as_ref().map(|assets| {
        let assets = assets.clone();
        Arc::new(move |ids| assets.get_by_ids(ids)) as MediaAttachmentAssetLoader<MediaAssetRecord, E, R>
    });
    MediaAttachmentListOptions {
        owner_content_id: options.owner_content_id.clone(),
        list_rows,
        row_record: Some(Arc::new(|row| (!row.media_asset_id.is_empty()).then_some(row))),
        load_assets_by_id,
        build_item: Some(store_media_attachment_item_builder(options)),
    }
}

/// List the owner's attachments with the grouped CMS workspace and existing stores.
pub fn list_prepared_media_attachment_form_items_from_store<E, R>(
    options: &MediaStoreAttachmentOptions<E, R>,
    memory: &PreparedCmsContent,
    on_memory: impl Fn(MemoryError) -> E + Send + Sync + 'static,
) -> Effect<PreparedAttachmentRows<MediaAttachmentFormItem>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    list_prepared_media_attachment_form_items(
        store_list_options(options),
        memory.attachment_records.clone(),
        memory.attachment_items.clone(),
        memory.attachment_metadata,
        on_memory,
    )
}

/// Replace one owner's stored attachments from the request form selection.
///
/// An absent asset or attachment store yields an empty result without reading the
/// request. Otherwise every collected input becomes one stored attachment record,
/// the records replace the owner's rows in one store call (including an empty
/// batch), and a successful replacement loads the owner's presented items through
/// [`list_media_attachment_form_items_from_store`]. A replacement or final-load
/// failure is returned unchanged. The caller supplies an initialized form when one
/// exists; this workflow does not parse the request.
pub fn replace_media_attachments_from_store_request<E, R>(
    options: &MediaStoreAttachmentOptions<E, R>,
) -> Effect<Vec<MediaAttachmentFormItem>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let (Some(_assets), Some(attachments)) =
        (options.assets.clone(), options.attachments.clone())
    else {
        return effect_sync! { body: || Vec::new() };
    };
    let owner_type = options.owner_type.clone();
    let owner_content_id = options.owner_content_id.clone();
    let replace_rows: MediaAttachmentRowReplacer<MediaAttachmentStoreRecord, E, R> =
        Arc::new(move |rows| {
            attachments.replace_by_owner(
                owner_type.clone(),
                owner_content_id.clone(),
                rows,
            )
        });
    let build_row: MediaAttachmentRowBuilder<MediaAttachmentStoreRecord> =
        Arc::new(|input| {
            Some(MediaAttachmentStoreRecord {
                id: InputBytes::default(),
                media_asset_id: input.asset_id,
                role: input.role,
                title_override: input.title_override,
                caption: input.caption,
                alt_override: input.alt_override,
            })
        });
    let list_options = options.clone();
    let load_replaced_items: MediaAttachmentItemLoader<E, R> = Arc::new(move || {
        list_media_attachment_form_items_from_store(&list_options)
    });
    replace_media_attachments_from_request(
        MediaAttachmentReplaceOptions {
            request: options.request.clone(),
            form: options.form.clone(),
            normalize_role: options.normalize_role.clone(),
            build_row: Some(build_row),
            replace_rows: Some(replace_rows),
            load_replaced_items: Some(load_replaced_items),
        },
    )
}

/// Reserve all local replacement and presentation storage before calling the existing stores.
pub fn replace_prepared_media_attachments_from_store_request<E, R>(
    options: &MediaStoreAttachmentOptions<E, R>,
    memory: &PreparedCmsContent,
    on_memory: impl Fn(MemoryError) -> E + Send + Sync + 'static,
) -> Effect<PreparedAttachmentRows<MediaAttachmentFormItem>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let (Some(_assets), Some(attachments)) = (options.assets.clone(), options.attachments.clone()) else {
        return list_prepared_media_attachment_form_items_from_store(options, memory, on_memory);
    };
    let on_memory = Arc::new(on_memory);
    let kind = options.owner_type.clone();
    let owner = options.owner_content_id.clone();
    let replace: PreparedAttachmentRowReplacer<MediaAttachmentStoreRecord, E, R> = Arc::new(move |rows| {
        let rows = effect_sync! { once; body: move || rows, };
        let attachments = attachments.clone();
        let kind = kind.clone();
        let owner = owner.clone();
        effect! {
            let rows = rows.clone();
            let attachments = attachments.clone();
            let kind = kind.clone();
            let owner = owner.clone();
            let mut rows = rows?;
            attachments.replace_by_owner(kind, owner, rows.take_consumer_rows())?;
            drop(rows);
        }
    });
    let build: PreparedAttachmentRowBuilder<MediaAttachmentStoreRecord> = Arc::new(|row| {
        Some(MediaAttachmentStoreRecord {
            id: InputBytes::default(),
            media_asset_id: InputBytes::from(row.asset_id),
            role: InputBytes::from(row.role),
            title_override: InputBytes::from(row.title_override),
            caption: InputBytes::from(row.caption),
            alt_override: InputBytes::from(row.alt_override),
        })
    });
    let replace_error = on_memory.clone();
    let replace = replace_prepared_media_attachments_from_request(
        PreparedAttachmentReplaceOptions {
            request: options.request.clone(),
            form: options.form.clone(),
            normalize_role: options.normalize_role.clone(),
            inputs: memory.attachments.clone(),
            rows: memory.attachment_replacements.clone(),
            build_row: Some(build),
            replace_rows: Some(replace),
            load_replaced_items: None,
        },
        move |error| replace_error(error),
    );
    let records = memory.attachment_records.clone();
    let items = memory.attachment_items.clone();
    let list_error = on_memory.clone();
    let admit = effect_try_sync! {
        body: move || reserve_attachment_list(&records, &items),
        on_error: move |error| list_error(error),
    };
    let list = store_list_options(options);
    let metadata_bytes = memory.attachment_metadata;
    effect! {
        let admit = admit.clone();
        let replace = replace.clone();
        let list = list.clone();
        let on_memory = on_memory.clone();
        let batch = admit?;
        let _ = replace?;
        let batch = effect_sync! { once; body: move || batch, };
        list_prepared_admitted(list, batch, metadata_bytes, on_memory)?
    }
}

/// Resolve one asset URL through the optional resolver and trim the result.
fn resolve_media_asset_url(
    resolve: Option<&MediaAssetURLResolver>,
    raw: &InputBytes,
) -> InputBytes {
    let raw = raw.trim();
    match resolve {
        Some(resolve) => resolve(raw.as_bytes()).trim().to_owned(),
        None => raw.to_owned(),
    }
}

/// Build the store-backed presented-item builder for one option set.
fn store_media_attachment_item_builder<E, R>(
    options: &MediaStoreAttachmentOptions<E, R>,
) -> MediaAttachmentItemBuilder<MediaAssetRecord>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request = options.request.clone();
    let edit_url = options.edit_url.clone();
    let image_kind = options.image_kind.clone();
    let resolve_url = options.resolve_url.clone();
    Arc::new(move |index, asset, record| {
        let edit = match edit_url.as_ref() {
            Some(edit_url) => edit_url(request.as_ref(), &asset),
            None => String::new(),
        };
        media_attachment_form_item_from_records(
            index,
            &asset,
            &record,
            &edit,
            &image_kind,
            resolve_url.as_ref(),
        )
    })
}

fn first_non_blank(values: &[InputBytes]) -> InputBytes { values.iter().map(InputBytes::trim).find(|v| !v.is_empty()).unwrap_or_default() }

/// Collect asset identities embedded in media shortcodes.
#[must_use]
pub fn media_shortcode_asset_ids(content: &str) -> std::collections::BTreeSet<String> {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(||
        regex::Regex::new(r#"(?i)\[media[^\]]*\bid="([^"]+)"[^\]]*\]"#)
            .expect("valid media shortcode pattern"));
    PATTERN.captures_iter(content).filter_map(|capture| capture.get(1))
        .map(|value| value.as_str().trim().to_owned()).filter(|value| !value.is_empty()).collect()
}
/// Render the full-width embedded media shortcode.
#[must_use]
pub fn embedded_media_shortcode(asset_id: &str) -> String {
    let asset_id = asset_id.trim();
    if asset_id.is_empty() { String::new() } else { format!(r#"[media id="{asset_id}" size="full" align="none"]"#) }
}

/// Resolve the root query field of a valid media action URL.
pub(crate) fn media_action_url_root_id(raw: &str) -> String {
    let raw=raw.trim();
    if url::parse(raw.as_bytes()).is_none(){return String::new();}
    let query=raw.split('#').next().unwrap_or_default().split_once('?').map(|(_,query)|query).unwrap_or_default();
    let values=effect_server::decode_url_encoded_pairs_partial(query.as_bytes());
    values.values.get(b"root_id".as_slice()).and_then(|values|values.first())
        .map(|value|crate::byte_text::encoding_text(crate::validation::trim_unicode_whitespace(value)))
        .unwrap_or_default()
}

pub(crate) fn media_url_is_valid(raw:&str)->bool { url::parse(raw.as_bytes()).is_some() }
