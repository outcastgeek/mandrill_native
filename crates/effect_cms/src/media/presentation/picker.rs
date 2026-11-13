//! Media picker view model and item projection.

use super::view_text;

use effect_server::{ByteFormData, Request};

use crate::attachments::{
    MediaAttachmentFormItem, MEDIA_ATTACHMENT_ROLE_HERO, normalize_media_attachment_role,
};
use crate::localization::TranslateKeyFunc;
use crate::media::assets::{
    MEDIA_KIND_IMAGE, MediaAssetListResult, MediaAssetRecord, MediaAssetURLResolver,
    media_asset_display_name, media_asset_preview_url,
    normalize_media_kind,
};
use crate::media::presentation::actions::js_string;
use crate::media::presentation::contracts::{
    MediaAssetListItemOptions, MediaPickerAssetRecord, MediaPickerFilterPillVM, MediaPickerItemVM,
    MediaPickerPageOptions, MediaPickerPageStateOptions, MediaPickerPageVM,
    MediaPickerPaginationProps,
};
use crate::media::presentation::labels::{media_status_label};
use crate::media::surface::media_create_fragment_path;
use crate::queries::{first_non_blank, request_query_value};
use crate::rendering::{
    ResourceListItemState,
    resource_list_current_page,
};

/// The newest sort value.
pub const MEDIA_PICKER_SORT_NEWEST: &str = "newest";
/// The oldest sort value.
pub const MEDIA_PICKER_SORT_OLDEST: &str = "oldest";
/// The alphabetical sort value.
pub const MEDIA_PICKER_SORT_ALPHA: &str = "alpha";

/// Build the picker page from explicit options.
#[must_use]
pub fn build_media_picker_page_vm(options: &MediaPickerPageOptions) -> MediaPickerPageVM {
    let mut surface = options.surface.clone();
    surface.root_id = first_input(&surface.root_id,&options.default_root_id);
    surface.create_root_id =
        first_input(&surface.create_root_id,&options.default_create_root_id);
    surface.role = first_input(&surface.role,&options.default_role);
    let mut limit = options.query.limit;
    if limit <= 0 {
        limit = options.default_page_size;
    }
    if limit <= 0 {
        limit = 1;
    }
    let total_pages = options
        .total_pages
        .as_ref()
        .map(|resolve| resolve(options.total, limit))
        .unwrap_or(0);
    let filter_form_id = format!("{}-filters", view_text(surface.root_id.trim()));
    let request = options.request.clone();
    let pills = match request.as_ref() {
        Some(request) => media_picker_filter_pills(options.translate_key.as_ref(), request, &view_text(&options.query.kind)),
        None => picker_filter_pills(&view_text(&options.query.kind), |key| key.to_owned()),
    };
    MediaPickerPageVM {
        request: request.clone(),
        root_id: view_text(&surface.root_id),
        create_root_id: view_text(&surface.create_root_id),
        scope: view_text(&surface.scope),
        role_default: view_text(&surface.role),
        target_id: view_text(&surface.target_id),
        owner_form_id: view_text(&surface.owner_form_id),
        owner_content_id: view_text(&surface.owner_content_id),
        attach_url: options.attach_url.trim().to_owned(),
        error_msg: options.error_msg.trim().to_owned(),
        search: view_text(&options.query.search),
        kind: view_text(&options.query.kind),
        status: view_text(&options.query.status),
        sort: view_text(&options.query.sort),
        create_url: options.create_url.trim().to_owned(),
        context_label: options.context_label.trim().to_owned(),
        signals: first_non_blank(&[
            options.signals.clone(),
            "{ mediaPickerOpen: true }".to_owned(),
        ]),
        filter_action: options.filter_action.trim().to_owned(),
        filter_form_id,
        attach_form_id: format!("{}-attach", view_text(surface.root_id.trim())),
        filter_submit_action: options.filter_submit_action.trim().to_owned(),
        filter_pills: pills,
        items: options.items.clone(),
        current_attachments: options.current_attachments.clone(),
        page: resource_list_current_page(
            options.query.offset,
            limit,
            options.default_page_size,
        ),
        page_size: limit,
        total: options.total,
        pages: total_pages,
    }
}

/// Build the picker page from state options.
#[must_use]
pub fn build_media_picker_page_from_state(
    options: &MediaPickerPageStateOptions,
) -> MediaPickerPageVM {
    let mut surface = options.surface.clone();
    surface.root_id = first_input(&surface.root_id,&options.default_root_id);
    surface.create_root_id =
        first_input(&surface.create_root_id,&options.default_create_root_id);
    surface.role = first_input(&surface.role,&options.default_role);
    let filter_form_id = format!("{}-filters", view_text(surface.root_id.trim()));
    let create_url = media_create_fragment_path(
        &options.create_base_path,
        &options.inline_surface,
        &surface.create_root_id,
        &surface.root_id,
        &surface.scope,
        &surface.target_id,
        &surface.owner_form_id,
        &surface.owner_content_id,
        &surface.role,
    );
    build_media_picker_page_vm(&MediaPickerPageOptions {
        request: options.request.clone(),
        surface,
        query: options.query.clone(),
        error_msg: options.error_msg.clone(),
        total: options.total,
        default_root_id: options.default_root_id.clone(),
        default_create_root_id: options.default_create_root_id.clone(),
        default_role: options.default_role.clone(),
        default_page_size: options.default_page_size,
        attach_url: options.attach_url.clone(),
        create_url,
        filter_action: options.filter_action.clone(),
        filter_submit_action: media_picker_filter_submit_action(
            &options.filter_action,
            &filter_form_id,
        ),
        context_label: options.context_label.clone(),
        signals: String::new(),
        current_attachments: options.current_attachments.clone(),
        items: options.items.clone(),
        total_pages: options.total_pages.clone(),
        translate_key: options.translate_key.clone(),
    })
}

/// Build the picker filter-submit Datastar action.
#[must_use]
pub fn media_picker_filter_submit_action(action: &str, form_id: &str) -> String {
    format!(
        "@get('{}', {{contentType: 'form', selector: '#{}'}})",
        js_string(action.trim()),
        js_string(form_id.trim())
    )
}

/// The picker sort options.
#[must_use]
pub fn media_picker_sort_options(
    translate: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
) -> Vec<web_components::components::unstyled::select::SelectOption> {
    vec![
        web_components::components::unstyled::select::SelectOption {
            value: MEDIA_PICKER_SORT_NEWEST.to_owned(),
            label: media_kind_sort_label(translate, request, "media.picker.sort_newest"),
            ..Default::default()
        },
        web_components::components::unstyled::select::SelectOption {
            value: MEDIA_PICKER_SORT_OLDEST.to_owned(),
            label: media_kind_sort_label(translate, request, "media.picker.sort_oldest"),
            ..Default::default()
        },
        web_components::components::unstyled::select::SelectOption {
            value: MEDIA_PICKER_SORT_ALPHA.to_owned(),
            label: media_kind_sort_label(translate, request, "media.picker.sort_alpha"),
            ..Default::default()
        },
    ]
}

fn media_kind_sort_label(
    translate: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    key: &str,
) -> String {
    crate::media::presentation::labels::media_translate(translate, request, key)
}

/// Normalize the picker sort value.
#[must_use]
pub fn media_picker_sort_value(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        MEDIA_PICKER_SORT_OLDEST => MEDIA_PICKER_SORT_OLDEST.to_owned(),
        MEDIA_PICKER_SORT_ALPHA => MEDIA_PICKER_SORT_ALPHA.to_owned(),
        _ => MEDIA_PICKER_SORT_NEWEST.to_owned(),
    }
}

/// Build the picker filter pills for the selected kind.
#[must_use]
pub fn media_picker_filter_pills(
    translate: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    selected_kind: &str,
) -> Vec<MediaPickerFilterPillVM> {
    picker_filter_pills(selected_kind, |key| crate::media::presentation::labels::media_translate(translate, request, key))
}

fn picker_filter_pills(selected_kind: &str, translate: impl Fn(&str) -> String) -> Vec<MediaPickerFilterPillVM> {
    let mut pills = Vec::with_capacity(4);
    for value in ["", "image", "video", "document"] {
        let key = match value {
            "image" => "media.filter.images",
            "video" => "media.filter.videos",
            "document" => "media.filter.documents",
            _ => "media.filter.kinds_all",
        };
        let label = translate(key);
        let selected = selected_kind.trim() == value.trim();
        let class = if selected {
            "rounded-full border-primary bg-primary text-primary-foreground"
        } else {
            "rounded-full border-border bg-background text-muted-foreground hover:border-primary/35 hover:bg-primary/5 hover:text-primary"
        };
        pills.push(MediaPickerFilterPillVM {
            value: value.to_owned(),
            label,
            class: class.to_owned(),
        });
    }
    pills
}

/// Resolve the picker selection input configuration.
#[must_use]
pub fn media_picker_selection_config(role: &str, hero_role: &str) -> (String, String, bool) {
    let hero = role.trim() == hero_role.trim();
    if hero {
        (
            "radio".to_owned(),
            "selected_asset_id".to_owned(),
            true,
        )
    } else {
        (
            "checkbox".to_owned(),
            "selected_asset_ids[]".to_owned(),
            false,
        )
    }
}

/// Resolve the checked/disabled state for one picker asset.
#[must_use]
pub fn media_picker_attachment_state(
    asset_id: &str,
    current_attachments: &[MediaAttachmentFormItem],
    hero: bool,
    hero_role: &str,
) -> (bool, bool) {
    let asset_id = asset_id.trim();
    let mut checked = false;
    let mut disabled = false;
    for current in current_attachments {
        if current.asset_id.trim() != asset_id {
            continue;
        }
        if hero {
            if current.role.trim() == hero_role.trim() {
                checked = true;
            }
            continue;
        }
        disabled = true;
        break;
    }
    (checked, disabled)
}

/// Build the picker item view models.
#[must_use]
pub fn build_media_picker_items(
    translate: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    assets: &[MediaPickerAssetRecord],
    current_attachments: &[MediaAttachmentFormItem],
    role: &str,
    hero_role: &str,
) -> Vec<MediaPickerItemVM> {
    if assets.is_empty() {
        return Vec::new();
    }
    let (selection_type, selection_name, hero) =
        media_picker_selection_config(role, hero_role);
    let mut items = Vec::with_capacity(assets.len());
    for asset in assets {
        let id = asset.id.trim();
        if id.is_empty() {
            continue;
        }
        let (checked, disabled) =
            media_picker_attachment_state(id, current_attachments, hero, hero_role);
        items.push(MediaPickerItemVM {
            id: id.to_owned(),
            kind: asset.kind.trim().to_owned(),
            status: asset.status.trim().to_owned(),
            title: asset.title.trim().to_owned(),
            resolved_preview: asset.resolved_preview.trim().to_owned(),
            meta: media_picker_item_meta(translate, request, asset),
            card_class: media_picker_item_card_class(checked, disabled),
            selected_attr: media_picker_selected_attr(checked),
            selection_type: selection_type.clone(),
            selection_name: selection_name.clone(),
            checked,
            disabled,
        });
    }
    items
}

/// Build the picker meta text.
#[must_use]
pub fn media_picker_item_meta(
    translate: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    asset: &MediaPickerAssetRecord,
) -> String {
    let mut parts = Vec::with_capacity(2);
    let provider = asset.provider.trim();
    if !provider.is_empty() {
        parts.push(provider.to_owned());
    } else if !asset.status.trim().is_empty() {
        parts.push(media_status_label(translate, request, &asset.status));
    }
    let short_id = media_picker_short_id(&asset.id, 10);
    if !short_id.is_empty() {
        parts.push(view_text(short_id));
    }
    parts.join(" • ")
}

/// Truncate a picker identity.
#[must_use]
pub fn media_picker_short_id(id: impl AsRef<[u8]>, max_length: i64) -> crate::commands::InputBytes {
    let id = crate::validation::trim_unicode_whitespace(id.as_ref());
    if max_length > 0 && id.len() > max_length as usize {
        return [id[..max_length as usize].to_vec(), b"...".to_vec()].concat().into();
    }
    id.into()
}

/// Resolve the picker card class.
#[must_use]
pub fn media_picker_item_card_class(checked: bool, disabled: bool) -> String {
    let base = "media-ui-picker-card flex h-full flex-col overflow-hidden rounded-[0.4rem] border bg-card transition focus-within:ring-2 focus-within:ring-primary/20";
    if disabled {
        format!("{base} cursor-not-allowed border-border/60 opacity-60")
    } else if checked {
        format!(
            "{base} cursor-pointer border-primary shadow-[0_0_0_2px_color-mix(in_srgb,var(--primary)_18%,transparent)]"
        )
    } else {
        format!("{base} cursor-pointer border-border hover:border-primary/35 hover:bg-primary/5")
    }
}

/// Resolve the selected attribute value.
#[must_use]
pub fn media_picker_selected_attr(checked: bool) -> String {
    if checked { "true".to_owned() } else { "false".to_owned() }
}

/// Build picker asset records from stored records.
#[must_use]
pub fn media_picker_asset_records(
    records: &[MediaAssetRecord],
    image_kind: &str,
    resolve: Option<&MediaAssetURLResolver>,
) -> Vec<MediaPickerAssetRecord> {
    let mut out = Vec::with_capacity(records.len());
    for asset in records {
        if asset.id.trim().is_empty() {
            continue;
        }
        out.push(MediaPickerAssetRecord {
            id: asset.id.trim().to_owned(),
            kind: asset.kind.trim().to_owned(),
            status: asset.status.trim().to_owned(),
            title: view_text(media_asset_display_name(asset)),
            resolved_preview: view_text(media_asset_preview_url(asset, image_kind, resolve)),
            provider: view_text(asset.provider.trim()),
        });
    }
    out
}

/// Build picker items from a store result.
#[must_use]
pub fn media_picker_items_from_result(
    translate: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    result: Option<&MediaAssetListResult>,
    current_attachments: &[MediaAttachmentFormItem],
    role: &str,
    image_kind: &str,
    resolve: Option<&MediaAssetURLResolver>,
    hero_role: &str,
) -> Vec<MediaPickerItemVM> {
    let Some(result) = result else {
        return Vec::new();
    };
    if result.items.is_empty() {
        return Vec::new();
    }
    let assets = media_picker_asset_records(&result.items, image_kind, resolve);
    build_media_picker_items(
        translate,
        request,
        &assets,
        current_attachments,
        role,
        hero_role,
    )
}

/// Build the canonical media list item states.
#[must_use]
pub fn media_asset_list_item_states(
    translate: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    records: &[MediaAssetRecord],
    options: &MediaAssetListItemOptions,
) -> Vec<ResourceListItemState> {
    let detail = options.detail_href.clone().map(|resolve| std::sync::Arc::new(move |_request: &Request, asset: &MediaAssetRecord| resolve(asset)) as crate::media::management::MediaAssetDetailHref);
    let delete = options.delete_href.clone().map(|resolve| std::sync::Arc::new(move |_request: &Request, asset: &MediaAssetRecord| resolve(asset)) as crate::media::management::MediaAssetDetailHref);
    crate::media::management::media_asset_list_item_states(request,
        &MediaAssetListResult { items: records.to_vec(), total: 0 }, detail.as_ref(), delete.as_ref(),
        if options.image_kind.trim().is_empty() { MEDIA_KIND_IMAGE } else { &options.image_kind },
        options.resolve_url.as_ref(), translate)
}

/// Build pagination props from a picker page.
#[must_use]
pub fn media_picker_pagination_props(
    page: &MediaPickerPageVM,
    previous_label: &str,
    next_label: &str,
    class: &str,
) -> MediaPickerPaginationProps {
    MediaPickerPaginationProps {
        form_id: page.filter_form_id.clone(),
        page: page.page,
        page_size: page.page_size,
        total: page.total,
        pages: page.pages,
        item_count: i64::try_from(page.items.len()).unwrap_or(i64::MAX),
        previous_label: previous_label.to_owned(),
        next_label: next_label.to_owned(),
        class: class.to_owned(),
    }
}

/// Resolve a form-or-query value from a decoded form and request.
#[must_use]
pub fn media_form_or_query_value(
    request: Option<&Request>,
    form: Option<&ByteFormData>,
    key: &str,
) -> String {
    if let Some(form) = form
        && let Some(values) = form.values.get(key.as_bytes())
        && let Some(value) = values.first()
        && !crate::validation::trim_unicode_whitespace(value).is_empty()
    {
        return view_text(crate::validation::trim_unicode_whitespace(value));
    }
    request
        .map(|request| view_text(request_query_value(request, key)))
        .unwrap_or_default()
}

/// Resolve the normalized kind for a picker kind filter.
#[must_use]
pub fn media_picker_normalize_kind(kind: &str) -> String {
    normalize_media_kind(kind)
}

/// The default hero role used by the picker.
#[must_use]
pub fn media_picker_default_hero_role() -> String {
    normalize_media_attachment_role(MEDIA_ATTACHMENT_ROLE_HERO)
}

/// Bind the source picker card, payload and pagination components.
pub fn default_media_picker_render_options<'a>(context: super::MediaRenderContext<'a>,token:&str,field:&str)->super::MediaPickerRenderOptions<'a> {
    super::MediaPickerRenderOptions {
        csrf_token:token.into(),csrf_field_name:field.into(),
        attachment_payload_inputs:Some(std::sync::Arc::new(super::media_attachment_payload_inputs)),
        item_card:Some(std::sync::Arc::new(move|item|super::media_library_card(context,&super::MediaLibraryCardProps{
            title:item.title.clone(),meta:item.meta,preview_url:item.resolved_preview,alt:item.title,kind:item.kind,..Default::default()
        }))),
        pagination:Some(std::sync::Arc::new(move|props|super::media_library_pagination(context,&super::MediaLibraryPaginationProps{
            form_id:props.form_id,page:props.page,page_size:props.page_size,total:props.total,pages:props.pages,item_count:props.item_count,
            previous_label:props.previous_label,next_label:props.next_label,class:props.class,
        }))),
    }
}
/// Render the picker with the canonical library components.
pub fn default_media_picker(context:super::MediaRenderContext<'_>,page:&MediaPickerPageVM,token:&str,field:&str)->maud::Markup {
    super::media_picker(context,page,&default_media_picker_render_options(context,token,field))
}

fn first_input(value:&crate::commands::InputBytes,fallback:&str)->crate::commands::InputBytes {
    if value.trim().is_empty(){fallback.trim().into()}else{value.trim()}
}
