//! Media content view derivation over stored hero/collection values.

use super::view_text;


use crate::attachments::{MEDIA_ATTACHMENT_ROLE_HERO, normalize_media_attachment_role};
use crate::media::assets::{
    MEDIA_KIND_IMAGE, MEDIA_KIND_VIDEO, MediaAssetURLResolver, media_shortcode_asset_ids,
};
use crate::media::contentmedia::{
    ContentMediaItem, ContentVideo, parse_content_media_json, parse_content_video_json,
};
use crate::media::presentation::contracts::{
    ContentMediaCollectionView, ContentMediaCollectionViewOptions,
    ContentMediaDetailSectionOptions,
    ContentMediaDetailSectionProps, ContentVideoDetailSectionProps, ContentVideoMediaView,
    ContentVideoMediaViewOptions, HeroMediaProps, MediaSlideDeckProps, MediaSlideItem,
    MediaSlideItemsOptions, MediaSlideSectionProps, VideoPlayerProps,
};
use crate::queries::first_non_blank;
use crate::seo::StructuredVideo;

/// The default slide state variable.
pub const DEFAULT_CONTENT_MEDIA_SLIDE_STATE_VAR: &str = "contentMediaSlide";

/// Derive player props from a stored hero video.
#[must_use]
pub fn content_video_player_props(
    video: Option<&ContentVideo>,
    fallback_title: &str,
    resolve: Option<&MediaAssetURLResolver>,
) -> Option<VideoPlayerProps> {
    let video = video?;
    Some(VideoPlayerProps {
        url: view_text(video.url.trim()),
        title: first_non_blank(&[view_text(&video.title), fallback_title.to_owned()]),
        provider: view_text(video.provider.name.trim()),
        thumbnail: resolve_asset_url(resolve, &video.metadata.thumbnail),
        description: view_text(video.metadata.description.trim()),
        ..VideoPlayerProps::default()
    })
}

/// Derive structured video values from a stored hero video.
#[must_use]
pub fn content_video_structured_video(
    video: Option<&ContentVideo>,
    fallback_title: &str,
    resolve_absolute: Option<&MediaAssetURLResolver>,
) -> Option<StructuredVideo> {
    let video = video?;
    Some(StructuredVideo {
        title: first_non_blank(&[view_text(&video.title), fallback_title.to_owned()]),
        description: view_text(video.metadata.description.trim()),
        url: view_text(video.url.trim()),
        thumbnail_url: resolve_asset_url(resolve_absolute, &video.metadata.thumbnail),
        published_at: video.metadata.published_at.trim().to_owned(),
        duration_seconds: video.metadata.duration,
        width: video.metadata.width,
        height: video.metadata.height,
    })
}

/// Select the hero item index.
#[must_use]
pub fn select_content_media_hero_index(items: &[ContentMediaItem]) -> i64 {
    for (index, item) in items.iter().enumerate() {
        if normalize_media_attachment_role(&item.role) == MEDIA_ATTACHMENT_ROLE_HERO {
            return i64::try_from(index).unwrap_or(i64::MAX);
        }
    }
    if items.is_empty() { -1 } else { 0 }
}

/// Derive hero props from a content media item.
#[must_use]
pub fn content_hero_media_props(
    item: Option<&ContentMediaItem>,
    fallback_title: &str,
    resolve: Option<&MediaAssetURLResolver>,
) -> Option<HeroMediaProps> {
    let item = item?;
    let mut props = HeroMediaProps {
        kind: item.kind.trim().to_ascii_lowercase(),
        url: view_text(item.url.trim()),
        thumbnail: resolve_asset_url(resolve, &item.thumbnail),
        alt: first_non_blank(&[view_text(&item.alt), fallback_title.to_owned()]),
        title: first_non_blank(&[view_text(&item.title), fallback_title.to_owned()]),
        description: view_text(item.description.trim()),
        provider: view_text(item.provider.trim()),
    };
    if props.kind == MEDIA_KIND_IMAGE {
        props.url = resolve_asset_url(resolve, &item.url);
    }
    Some(props)
}

/// Derive the collection share image URL.
#[must_use]
pub fn content_media_share_image_url(
    item: Option<&ContentMediaItem>,
    resolve_absolute: Option<&MediaAssetURLResolver>,
) -> String {
    let Some(item) = item else {
        return String::new();
    };
    if item.kind.trim().eq_ignore_ascii_case(MEDIA_KIND_IMAGE) {
        resolve_asset_url(resolve_absolute, &item.url)
    } else {
        resolve_asset_url(resolve_absolute, &item.thumbnail)
    }
}

/// Derive structured video values for a video item.
#[must_use]
pub fn content_media_structured_video(
    item: Option<&ContentMediaItem>,
    fallback_title: &str,
    resolve_absolute: Option<&MediaAssetURLResolver>,
) -> Option<StructuredVideo> {
    let item = item?;
    if !item.kind.trim().eq_ignore_ascii_case(MEDIA_KIND_VIDEO) {
        return None;
    }
    Some(StructuredVideo {
        title: first_non_blank(&[view_text(&item.title), fallback_title.to_owned()]),
        description: view_text(item.description.trim()),
        url: view_text(item.url.trim()),
        thumbnail_url: resolve_asset_url(resolve_absolute, &item.thumbnail),
        duration_seconds: item.duration,
        ..StructuredVideo::default()
    })
}

/// Derive slide items from content media items.
#[must_use]
pub fn content_media_slide_items(
    items: &[ContentMediaItem],
    options: &MediaSlideItemsOptions,
) -> Vec<MediaSlideItem> {
    let embedded = media_shortcode_asset_ids(&options.content);
    let mut slides = Vec::with_capacity(items.len());
    for (index,item) in items.iter().enumerate() {
        if i64::try_from(index).ok() == Some(options.hero_index) { continue; }
        if item.id.to_str().ok().is_some_and(|id| !id.trim().is_empty() && embedded.contains(id.trim())) { continue; }
        let preview = if item.thumbnail.trim().is_empty() { &item.url } else { &item.thumbnail };
        slides.push(MediaSlideItem {
            id: view_text(item.id.trim()), kind: item.kind.trim().to_owned(),
            url: resolve_asset_url(options.resolve_url.as_ref(), &item.url),
            thumbnail: resolve_asset_url(options.resolve_url.as_ref(), preview),
            alt: view_text(item.alt.trim()), title: first_non_blank(&[view_text(&item.title),view_text(&item.id)]),
            description: view_text(item.description.trim()), provider: view_text(item.provider.trim()),
            ..MediaSlideItem::default()
        });
    }
    slides
}

/// Filter embedded/hero slides and resolve their URLs.
#[must_use]
pub fn build_media_slide_items(
    items: &[MediaSlideItem],
    options: &MediaSlideItemsOptions,
) -> Vec<MediaSlideItem> {
    let embedded = media_shortcode_asset_ids(&options.content);
    let mut slides = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        if i64::try_from(index).ok() == Some(options.hero_index) {
            continue;
        }
        let id = item.id.trim();
        if !id.is_empty() && embedded.contains(id) {
            continue;
        }
        let mut slide = item.clone();
        slide.url = resolve_slide_url(options.resolve_url.as_ref(), &item.url);
        slide.thumbnail = resolve_slide_url(options.resolve_url.as_ref(), &slide_preview_url(item));
        slides.push(slide);
    }
    slides
}

/// Build the hero video view.
#[must_use]
pub fn build_content_video_media_view(options: &ContentVideoMediaViewOptions) -> ContentVideoMediaView {
    let video = options
        .video_json
        .as_deref()
        .and_then(|raw| parse_content_video_json(Some(raw)).ok())
        .flatten();
    let hero_image_url = resolve_asset_url(options.resolve_url.as_ref(), &options.image);
    let mut share_image_url = resolve_asset_url(options.resolve_absolute_url.as_ref(), &options.image);
    if share_image_url.is_empty()
        && let Some(video) = video.as_ref()
    {
        share_image_url = resolve_asset_url(
            options.resolve_absolute_url.as_ref(),
            &video.metadata.thumbnail,
        );
    }
    ContentVideoMediaView {
        video_block: content_video_player_props(
            video.as_ref(),
            &options.title,
            options.resolve_url.as_ref(),
        ),
        video_model: video,
        hero_image_url,
        share_image_url,
    }
}

/// Build the media collection view.
#[must_use]
pub fn build_content_media_collection_view(
    options: &ContentMediaCollectionViewOptions,
) -> ContentMediaCollectionView {
    let items = parse_content_media_json(&options.media_json)
        .ok()
        .flatten()
        .unwrap_or_default();
    let hero_index = select_content_media_hero_index(&items);
    let hero = usize::try_from(hero_index)
        .ok()
        .and_then(|index| items.get(index))
        .cloned();
    let slide_items = content_media_slide_items(
        &items,
        &MediaSlideItemsOptions {
            content: options.description.trim().to_owned(),
            hero_index,
            resolve_url: options.resolve_url.clone(),
        },
    );
    let share_image_url = content_media_share_image_url(
        hero.as_ref(),
        options.resolve_absolute_url.as_ref(),
    );
    ContentMediaCollectionView {
        hero_block: content_hero_media_props(
            hero.as_ref(),
            &options.title,
            options.resolve_url.as_ref(),
        ),
        hero_model: hero,
        media_models: items,
        slide_items,
        share_image_url,
    }
}

/// Build the hero detail section props.
#[must_use]
pub fn build_content_video_detail_section_props(
    view: &ContentVideoMediaView,
) -> ContentVideoDetailSectionProps {
    ContentVideoDetailSectionProps {
        video: view.video_block.clone(),
    }
}

/// Build the collection detail section props.
#[must_use]
pub fn build_content_media_detail_section_props(
    view: &ContentMediaCollectionView,
    options: &ContentMediaDetailSectionOptions,
) -> ContentMediaDetailSectionProps {
    ContentMediaDetailSectionProps {
        hero: view.hero_block.clone(),
        slide_title: options.slide_title.clone(),
        slide_eyebrow: options.slide_eyebrow.clone(),
        slide_subtitle: options.slide_subtitle.clone(),
        slide_state_var: first_non_blank(&[
            options.slide_state_var.clone(),
            DEFAULT_CONTENT_MEDIA_SLIDE_STATE_VAR.to_owned(),
        ]),
        slides: view.slide_items.clone(),
    }
}

/// Build the slide deck props for one collection view.
#[must_use]
pub fn media_slide_deck_props(
    view: &ContentMediaCollectionView,
    state_var: &str,
) -> MediaSlideDeckProps {
    MediaSlideDeckProps {
        state_var: first_non_blank(&[
            state_var.to_owned(),
            DEFAULT_CONTENT_MEDIA_SLIDE_STATE_VAR.to_owned(),
        ]),
        items: view.slide_items.clone(),
    }
}

/// Build the slide section props for one collection view.
#[must_use]
pub fn media_slide_section_props(
    view: &ContentMediaCollectionView,
    options: &ContentMediaDetailSectionOptions,
) -> MediaSlideSectionProps {
    let props = build_content_media_detail_section_props(view, options);
    MediaSlideSectionProps {
        title: props.slide_title,
        eyebrow: props.slide_eyebrow,
        subtitle: props.slide_subtitle,
        state_var: props.slide_state_var,
        items: props.slides,
    }
}

/// Build a keyed preview image map.
#[must_use]
pub fn build_resource_preview_image_map<Row>(
    rows: &[Row],
    key: Option<&(dyn Fn(&Row) -> String + Send + Sync)>,
    preview_image: Option<&(dyn Fn(&Row) -> String + Send + Sync)>,
) -> std::collections::BTreeMap<String, String> {
    let (Some(key),Some(preview_image)) = (key,preview_image) else { return Default::default(); };
    let mut out = std::collections::BTreeMap::new();
    for row in rows {
        let row_key = key(row).trim().to_owned();
        if row_key.is_empty() { continue; }
        out.insert(row_key,preview_image(row).trim().to_owned());
    }
    out
}

/// Parse stored video JSON through the content-media owner.
///
/// # Errors
///
/// Returns the content-media parse failure unchanged.
pub fn parse_content_video(
    raw: Option<&str>,
) -> Result<Option<ContentVideo>, serde_json::Error> {
    parse_content_video_json(raw)
}

/// Parse stored collection JSON through the content-media owner.
///
/// # Errors
///
/// Returns the content-media parse failure unchanged.
pub fn parse_content_media(
    raw: &str,
) -> Result<Option<Vec<ContentMediaItem>>, serde_json::Error> {
    parse_content_media_json(raw)
}

fn resolve_asset_url(resolve: Option<&MediaAssetURLResolver>, raw: impl AsRef<[u8]>) -> String {
    let raw = crate::validation::trim_unicode_whitespace(raw.as_ref());
    match resolve {
        Some(resolve) => view_text(resolve(raw).trim()),
        None => view_text(raw),
    }
}

fn resolve_slide_url(resolve: Option<&MediaAssetURLResolver>, raw: &str) -> String {
    resolve_asset_url(resolve,raw)
}

fn slide_preview_url(item: &MediaSlideItem) -> String {
    if !item.thumbnail.trim().is_empty() {
        return item.thumbnail.clone();
    }
    item.url.clone()
}
