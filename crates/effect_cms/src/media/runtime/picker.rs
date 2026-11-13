//! The private picker pipeline.
//!
//! The store list, the surface decode and the page publication run in the source
//! order. Each stage is a private prepared value consumed by the next stage; a stage
//! cannot be reused or substituted, and the presentation owner receives the decoded
//! state through the caller's publisher instead of a duplicated page model.

use std::sync::Arc;

use effect::{Effect, effect, effect_sync};

use effect_server::{ByteFormData, Request, ServerError};

use crate::attachments::{MediaAttachmentFormItem, MediaFormPolicy};
use crate::media::assets::MediaAssetListResult;
use crate::media::surface::{MediaQuery, MediaSurfaceRequest};

use super::{MediaAssetStoreRuntime, PickerPageState};

/// The listed-assets stage.
#[derive(Clone)]
struct PickerAssetsListed {
    request: Request,
    query: MediaQuery,
    error_msg: String,
    result: Option<MediaAssetListResult>,
}
/// The decoded-surface stage.
struct PickerSurfaceDecoded {
    listed: PickerAssetsListed,
    surface: MediaSurfaceRequest,
    current_attachments: Vec<MediaAttachmentFormItem>,
}

/// Run the picker pipeline and publish the presentation page.
pub(super) fn run_picker_pipeline<E, R, P>(
    runtime: MediaAssetStoreRuntime<E, R>,
    request: &Request,
    query: MediaQuery,
    error_msg: String,
    form: Option<Arc<ByteFormData>>,
    policy: MediaFormPolicy,
    on_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
    publish: Arc<dyn Fn(PickerPageState) -> P + Send + Sync + 'static>,
) -> Effect<P, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    P: Default + Send + 'static,
{
    if runtime.assets.is_none() {
        return effect_sync! { body { P::default() } };
    }
    let request = request.clone();
    effect! {
        let runtime = runtime.clone();
        let request = request.clone();
        let query = query.clone();
        let error_msg = error_msg.clone();
        let form = form.clone();
        let on_error = on_error.clone();
        let publish = Arc::clone(&publish);
        let listed = list_picker_assets(&runtime, request.clone(), query, error_msg)?;
        let decoded = decode_picker_surface(&runtime, listed, form, policy, on_error)?;
        let surface = decoded.surface.clone();
        let total = decoded.listed.result.as_ref().map(|result| result.total).unwrap_or(0);
        let state = PickerPageState {
            request: decoded.listed.request.clone(),
            surface: surface.clone(),
            query: decoded.listed.query.clone(),
            error_msg: decoded.listed.error_msg.clone(),
            result: decoded.listed.result.clone(),
            current_attachments: decoded.current_attachments.clone(),
            total,
            default_picker_root_id: crate::queries::first_non_blank(&[
                runtime.default_picker_root_id.clone(),
                crate::media::surface::DEFAULT_MEDIA_PICKER_ROOT_ID.to_owned(),
            ]),
            default_create_root_id: crate::queries::first_non_blank(&[
                runtime.default_create_root_id.clone(),
                crate::media::surface::DEFAULT_MEDIA_CREATE_ROOT_ID.to_owned(),
            ]),
            default_role: runtime.default_role(&surface),
            default_page_size: runtime.default_page_size,
            attach_url: runtime.request_url(
                &decoded.listed.request,
                runtime.picker_attach_url.as_ref(),
            ),
            create_base_path: runtime.request_url(
                &decoded.listed.request,
                runtime.create_base_path.as_ref().or(runtime.media_new_url.as_ref()),
            ),
            inline_surface: runtime.resolved_inline_surface(),
            context_label: runtime.context_label_for(&decoded.listed.request, &surface),
            filter_action: runtime.request_url(
                &decoded.listed.request,
                runtime.filter_action_url.as_ref().or(runtime.picker_url.as_ref()),
            ),
            image_kind: runtime.image_kind_name(),
            hero_role: runtime.hero_role_name(),
            resolve_url: runtime.request_asset_resolver(&decoded.listed.request),
            total_pages: runtime.total_pages.clone(),
        };
        publish(state)
    }
}

fn list_picker_assets<E, R>(
    runtime: &MediaAssetStoreRuntime<E, R>,
    request: Request,
    query: MediaQuery,
    error_msg: String,
) -> Effect<PickerAssetsListed, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let assets = runtime.assets.clone();
    effect! {
        let assets = assets.clone();
        let request = request.clone();
        let query = query.clone();
        let error_msg = error_msg.clone();
        let result = match assets.as_ref() {
            Some(assets) => assets.list(query.clone())?,
            None => None,
        };
        PickerAssetsListed {
            request,
            query,
            error_msg,
            result,
        }
    }
}

fn decode_picker_surface<E, R>(
    runtime: &MediaAssetStoreRuntime<E, R>,
    listed: PickerAssetsListed,
    form: Option<Arc<ByteFormData>>,
    policy: MediaFormPolicy,
    on_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
) -> Effect<PickerSurfaceDecoded, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let runtime = runtime.clone();
    effect! {
        let runtime = runtime.clone();
        let listed = listed.clone();
        let form = form.clone();
        let on_error = on_error.clone();
        let (mut surface, _form) = runtime.surface_request_from_request(
            &listed.request, form, policy, move |error| on_error(error),
        )?;
        let default_role = runtime.default_role(&surface);
        if surface.role.trim().is_empty() {
            surface.role = default_role.into();
        }
        let current_attachments = runtime
            .current_attachment_items(
                &listed.request,
                surface.scope.clone(),
                surface.owner_content_id.clone(),
            )?;
        PickerSurfaceDecoded {
            listed,
            surface,
            current_attachments,
        }
    }
}
