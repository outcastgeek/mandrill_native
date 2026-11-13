//! Derived mutation media state and the resource replacement entry point.
//!
//! Both state builders preview the request's attachment items through a supplied
//! runtime capability and then derive the stored values with the content-media
//! owner's transformations. They perform no persistence and produce no HTML.

use std::sync::Arc;

use effect::{Effect, effect, effect_try_sync};
use effect_server::Request;

use crate::attachments::MediaAttachmentFormItem;
use crate::media::contentmedia::{
    ContentMediaItem, ContentVideo, content_media_items_from_attachment_items,
    content_video_from_attachment_items, marshal_content_media_json,
};
use crate::commands::InputBytes;

use super::workflow::AttachmentPreviewer;

/// The derived hero image, video and presented attachments.
#[derive(Clone, Debug, Default)]
pub struct ContentVideoMutationMediaState {
    /// The derived hero image URL.
    pub image: InputBytes,
    /// The derived hero video, when the items contain one.
    pub video: Option<ContentVideo>,
    /// The previewed attachment items.
    pub attachments: Vec<MediaAttachmentFormItem>,
}

/// Options for the derived hero media state.
pub struct ContentVideoMutationMediaStateOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The runtime preview capability.
    runtime: Arc<dyn AttachmentPreviewer<E, R>>,
    /// The fallback image used after the derived image.
    pub fallback_image: String,
    /// The default image used after the fallback image.
    pub default_image: String,
}

impl<E, R> ContentVideoMutationMediaStateOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Construct the options with the required preview capability and image values.
    #[must_use]
    pub fn new(
        runtime: Arc<dyn AttachmentPreviewer<E, R>>,
        fallback_image: String,
        default_image: String,
    ) -> Self {
        Self {
            runtime,
            fallback_image,
            default_image,
        }
    }
}

/// The derived collection attachments, stored media items and JSON text.
#[derive(Clone, Debug, Default)]
pub struct ContentMediaCollectionMutationState {
    /// The previewed attachment items.
    pub attachments: Vec<MediaAttachmentFormItem>,
    /// The stored media items derived from the attachments.
    pub media: Vec<ContentMediaItem>,
    /// The stored media items encoded as JSON text.
    pub media_json: String,
}

/// Derive the hero image and video from the request's attachment selection.
///
/// The runtime preview supplies the items. The derived image wins over the
/// fallback image, which wins over the default image; blank values are skipped.
/// The effect is lazy and reusable, and a preview error is returned unchanged.
pub fn build_content_video_mutation_media_state<E, R>(
    request: Option<Request>,
    options: ContentVideoMutationMediaStateOptions<E, R>,
) -> Effect<ContentVideoMutationMediaState, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let ContentVideoMutationMediaStateOptions {
        runtime,
        fallback_image,
        default_image,
    } = options;
    effect! {
        let runtime = runtime.clone();
        let request = request.clone();
        let fallback_image = fallback_image.clone();
        let default_image = default_image.clone();
        let items = runtime.preview_attachment_items_from_request(request)?;
        let (image, video) = content_video_from_attachment_items(&items);
        let image = [image, fallback_image.into(), default_image.into()].into_iter().map(|value: InputBytes| value.trim()).find(|value| !value.is_empty()).unwrap_or_default();
        ContentVideoMutationMediaState {
            image,
            video,
            attachments: items,
        }
    }
}

/// Derive the stored media collection from the request's attachment selection.
///
/// The runtime preview supplies the items and the content-media owner derives the
/// stored items and their JSON text. `on_marshal_error` maps a JSON serialization
/// failure into the caller's error family; a preview error is returned unchanged.
/// The effect is lazy and reusable.
pub fn build_content_media_collection_mutation_state<E, R>(
    request: Option<Request>,
    runtime: Arc<dyn AttachmentPreviewer<E, R>>,
    on_marshal_error: impl Fn(serde_json::Error) -> E + Send + Sync + 'static,
) -> Effect<ContentMediaCollectionMutationState, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let on_marshal_error: Arc<dyn Fn(serde_json::Error) -> E + Send + Sync> =
        Arc::new(on_marshal_error);
    effect! {
        let runtime = runtime.clone();
        let request = request.clone();
        let on_marshal_error = on_marshal_error.clone();
        let items = runtime.preview_attachment_items_from_request(request)?;
        let media = content_media_items_from_attachment_items(&items);
        let media_for_json = media.clone();
        let media_json = effect_try_sync! {
            body: {
                let on_marshal_error = on_marshal_error.clone();
                move || {
                    marshal_content_media_json(&media_for_json)
                        .map_err(|error| on_marshal_error(error))
                }
            },
        }?;
        ContentMediaCollectionMutationState {
            attachments: items,
            media,
            media_json,
        }
    }
}
