//! Preview, listing, current-input selection and owner replacement workflows.
//!
//! Every workflow here coordinates supplied capabilities: asset lookup, attachment
//! row listing, row building, row replacement and final item loading. The owner
//! performs no persistence itself, and every capability that performs I/O returns
//! a lazy [`Effect`], so constructing a workflow performs no read or write.
//!
//! The workflows preserve these rules:
//!
//! - collection is position ordered and never deduplicated; deduplication belongs
//!   to current-input selection;
//! - a role normalizer receives the supplied role before this owner trims it, and
//!   an absent normalizer trims only;
//! - an absent required callback is a no-op terminal, and an empty selection still
//!   replaces the owner's rows once, which clears the selection;
//! - a replacement error and a final-load error are returned unchanged, and final
//!   items are loaded only after the replacement succeeds;
//! - asset rows absent from an asset lookup are skipped rather than failing.
//!
//! A consumed replacement stage is a private by-value value: it is created only
//! inside this module, is not cloneable, and hands its rows to the replacement
//! capability exactly once per run. Each run builds a fresh stage, so the public
//! workflow remains reusable without a cross-run one-shot restriction.

use std::collections::BTreeMap;
use std::sync::Arc;

use effect::{Effect, effect, effect_sync};
use effect_server::{ByteFormData, Request};

use crate::attachments::{
    MediaAttachmentFormItem, MediaAttachmentInput, MediaAttachmentStoreRecord, RoleNormalizer,
    RoleNormalizerRef, collect_media_attachment_inputs, dedupe_media_attachment_inputs,
    media_attachment_inputs_from_items,
};
use crate::definitions::ResourceKind;
use crate::commands::InputBytes;


/// Loads assets by identity for presented attachment rows.
pub type MediaAttachmentAssetLoader<Asset, E, R> =
    Arc<dyn Fn(Vec<InputBytes>) -> Effect<BTreeMap<InputBytes, Asset>, E, R> + Send + Sync + 'static>;

/// Builds one presented item from an asset and an attachment row.
///
/// The boolean result of the corresponding source contract is represented by
/// `Option`: [`None`] omits the row, and [`Some`] keeps it.
pub type MediaAttachmentItemBuilder<Asset> = Arc<
    dyn Fn(usize, Asset, MediaAttachmentStoreRecord) -> Option<MediaAttachmentFormItem>
        + Send
        + Sync
        + 'static,
>;

/// Loads the owner's current presented items.
pub type MediaAttachmentItemLoader<E, R> =
    Arc<dyn Fn() -> Effect<Vec<MediaAttachmentFormItem>, E, R> + Send + Sync + 'static>;

/// Builds one stored row from one collected input.
pub type MediaAttachmentRowBuilder<Row> =
    Arc<dyn Fn(MediaAttachmentInput) -> Option<Row> + Send + Sync + 'static>;

/// Lists stored attachment rows for one owner.
pub type MediaAttachmentRowLister<Row, E, R> =
    Arc<dyn Fn() -> Effect<Vec<Row>, E, R> + Send + Sync + 'static>;

/// Converts one stored row into its attachment record, or omits the row.
pub type MediaAttachmentRowRecord<Row> =
    Arc<dyn Fn(Row) -> Option<MediaAttachmentStoreRecord> + Send + Sync + 'static>;

/// Replaces every stored attachment row for one owner.
pub type MediaAttachmentRowReplacer<Row, E, R> =
    Arc<dyn Fn(Vec<Row>) -> Effect<(), E, R> + Send + Sync + 'static>;

/// Options for an attachment preview.
pub struct MediaAttachmentPreviewOptions<
    Asset: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    /// The request whose form selection is previewed.
    pub request: Option<Request>,
    /// The initialized merged form, when available.
    pub form: Option<Arc<ByteFormData>>,
    /// The already-collected inputs previewed by the input-only variant.
    pub inputs: Vec<MediaAttachmentInput>,
    /// Optional attachment-role normalizer.
    pub normalize_role: Option<RoleNormalizer>,
    /// Optional asset loader.
    pub load_assets_by_id: Option<MediaAttachmentAssetLoader<Asset, E, R>>,
    /// Optional presented-item builder.
    pub build_item: Option<MediaAttachmentItemBuilder<Asset>>,
}

impl<Asset: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone
    for MediaAttachmentPreviewOptions<Asset, E, R>
{
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            form: self.form.clone(),
            inputs: self.inputs.clone(),
            normalize_role: self.normalize_role.clone(),
            load_assets_by_id: self.load_assets_by_id.clone(),
            build_item: self.build_item.clone(),
        }
    }
}

impl<Asset: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Default
    for MediaAttachmentPreviewOptions<Asset, E, R>
{
    fn default() -> Self {
        Self {
            request: None,
            form: None,
            inputs: Vec::new(),
            normalize_role: None,
            load_assets_by_id: None,
            build_item: None,
        }
    }
}

/// Options for listing an owner's stored attachments.
pub struct MediaAttachmentListOptions<
    Asset: Send + 'static,
    Row: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    /// The owning content identity.
    pub owner_content_id: InputBytes,
    /// Optional stored-row lister.
    pub list_rows: Option<MediaAttachmentRowLister<Row, E, R>>,
    /// Optional stored-row record conversion.
    pub row_record: Option<MediaAttachmentRowRecord<Row>>,
    /// Optional asset loader.
    pub load_assets_by_id: Option<MediaAttachmentAssetLoader<Asset, E, R>>,
    /// Optional presented-item builder.
    pub build_item: Option<MediaAttachmentItemBuilder<Asset>>,
}

impl<
    Asset: Send + 'static,
    Row: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> Clone for MediaAttachmentListOptions<Asset, Row, E, R>
{
    fn clone(&self) -> Self {
        Self {
            owner_content_id: self.owner_content_id.clone(),
            list_rows: self.list_rows.clone(),
            row_record: self.row_record.clone(),
            load_assets_by_id: self.load_assets_by_id.clone(),
            build_item: self.build_item.clone(),
        }
    }
}

impl<
    Asset: Send + 'static,
    Row: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> Default for MediaAttachmentListOptions<Asset, Row, E, R>
{
    fn default() -> Self {
        Self {
            owner_content_id: InputBytes::default(),
            list_rows: None,
            row_record: None,
            load_assets_by_id: None,
            build_item: None,
        }
    }
}

/// Options for reading the owner's current attachment inputs.
pub struct CurrentMediaAttachmentInputOptions<E: Send + 'static, R: Send + Sync + 'static> {
    /// The request whose form selection wins when it carries one.
    pub request: Option<Request>,
    /// The initialized merged form, when available.
    pub form: Option<Arc<ByteFormData>>,
    /// Optional attachment-role normalizer.
    pub normalize_role: Option<RoleNormalizer>,
    /// Optional loader for the owner's presented items.
    pub load_current_items: Option<MediaAttachmentItemLoader<E, R>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone
    for CurrentMediaAttachmentInputOptions<E, R>
{
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            form: self.form.clone(),
            normalize_role: self.normalize_role.clone(),
            load_current_items: self.load_current_items.clone(),
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default
    for CurrentMediaAttachmentInputOptions<E, R>
{
    fn default() -> Self {
        Self {
            request: None,
            form: None,
            normalize_role: None,
            load_current_items: None,
        }
    }
}

/// Options for replacing an owner's attachments.
pub struct MediaAttachmentReplaceOptions<
    Row: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    /// The request whose form selection is replaced.
    pub request: Option<Request>,
    /// The initialized merged form, when available.
    pub form: Option<Arc<ByteFormData>>,
    /// Optional attachment-role normalizer.
    pub normalize_role: Option<RoleNormalizer>,
    /// Optional stored-row builder.
    pub build_row: Option<MediaAttachmentRowBuilder<Row>>,
    /// Optional stored-row replacer.
    pub replace_rows: Option<MediaAttachmentRowReplacer<Row, E, R>>,
    /// Optional loader for the replacement's presented items.
    pub load_replaced_items: Option<MediaAttachmentItemLoader<E, R>>,
}

impl<Row: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone
    for MediaAttachmentReplaceOptions<Row, E, R>
{
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            form: self.form.clone(),
            normalize_role: self.normalize_role.clone(),
            build_row: self.build_row.clone(),
            replace_rows: self.replace_rows.clone(),
            load_replaced_items: self.load_replaced_items.clone(),
        }
    }
}

impl<Row: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Default
    for MediaAttachmentReplaceOptions<Row, E, R>
{
    fn default() -> Self {
        Self {
            request: None,
            form: None,
            normalize_role: None,
            build_row: None,
            replace_rows: None,
            load_replaced_items: None,
        }
    }
}

/// The runtime capability that replaces one owner's attachments from a request.
pub trait AttachmentReplacer<E, R>: Send + Sync + 'static
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Replace one owner's attachments from the request form selection.
    fn replace_attachments_from_request(
        &self,
        request: Option<Request>,
        owner_type: String,
        owner_content_id: InputBytes,
    ) -> Effect<Vec<MediaAttachmentFormItem>, E, R>;
}

/// The runtime capability that previews request attachment items.
pub trait AttachmentPreviewer<E, R>: Send + Sync + 'static
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Preview the request form selection.
    fn preview_attachment_items_from_request(
        &self,
        request: Option<Request>,
    ) -> Effect<Vec<MediaAttachmentFormItem>, E, R>;
}

/// Preview already-collected inputs.
///
/// An empty selection, an absent asset loader or an absent item builder yields an
/// empty result without calling a capability. Identities are trimmed and blank
/// identities are skipped; a row whose asset is absent from the lookup is skipped.
/// The preview keeps the input order and does not deduplicate.
pub fn preview_media_attachment_form_items_from_inputs<Asset, E, R>(
    options: MediaAttachmentPreviewOptions<Asset, E, R>,
) -> Effect<Vec<MediaAttachmentFormItem>, E, R>
where
    Asset: Clone + Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let MediaAttachmentPreviewOptions {
        inputs,
        normalize_role,
        load_assets_by_id,
        build_item,
        ..
    } = options;
    let (Some(load_assets_by_id), Some(build_item)) = (load_assets_by_id, build_item) else {
        return effect_sync! { body: || Vec::new() };
    };
    preview_form_items_from_inputs(
        inputs,
        normalize_role,
        load_assets_by_id,
        build_item,
    )
}

/// Preview the request form selection.
///
/// The initialized form selection is collected first, including role-normalizer
/// callbacks. An absent asset loader or item builder then yields an empty result.
/// Otherwise the position-ordered selection is previewed by
/// [`preview_media_attachment_form_items_from_inputs`].
pub fn preview_media_attachment_form_items_from_request<Asset, E, R>(
    options: MediaAttachmentPreviewOptions<Asset, E, R>,
) -> Effect<Vec<MediaAttachmentFormItem>, E, R>
where
    Asset: Clone + Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let MediaAttachmentPreviewOptions {
        request,
        form,
        normalize_role,
        load_assets_by_id,
        build_item,
        ..
    } = options;
    effect! {
        let request = request.clone();
        let form = form.clone();
        let normalize_role = normalize_role.clone();
        let load_assets_by_id = load_assets_by_id.clone();
        let build_item = build_item.clone();
        let inputs = collect_media_attachment_inputs(request.as_ref().and(form.as_deref()), normalize_role.as_deref());
        match (load_assets_by_id, build_item) {
            (Some(load_assets_by_id), Some(build_item)) => {
                let items = preview_form_items_from_inputs(
                    inputs,
                    normalize_role,
                    load_assets_by_id,
                    build_item,
                )?;
                items
            }
            _ => Vec::new(),
        }
    }
}

/// List an owner's stored attachments as presented items.
///
/// A blank owner identity, an absent row lister, row conversion, asset loader or
/// item builder yields an empty result without calling a capability. Rows whose
/// conversion is absent or whose trimmed asset identity is blank are skipped, and
/// an asset absent from the lookup is skipped rather than failing the listing.
pub fn list_media_attachment_form_items<Asset, Row, E, R>(
    options: MediaAttachmentListOptions<Asset, Row, E, R>,
) -> Effect<Vec<MediaAttachmentFormItem>, E, R>
where
    Asset: Clone + Send + 'static,
    Row: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let MediaAttachmentListOptions {
        owner_content_id,
        list_rows,
        row_record,
        load_assets_by_id,
        build_item,
    } = options;
    if owner_content_id.trim().is_empty() {
        return effect_sync! { body: || Vec::new() };
    }
    let (Some(list_rows), Some(row_record), Some(load_assets_by_id), Some(build_item)) =
        (list_rows, row_record, load_assets_by_id, build_item)
    else {
        return effect_sync! { body: || Vec::new() };
    };
    effect! {
        let list_rows = list_rows.clone();
        let row_record = row_record.clone();
        let load_assets_by_id = load_assets_by_id.clone();
        let build_item = build_item.clone();
        let rows = list_rows()?;
        let mut records = Vec::with_capacity(rows.len());
        let mut asset_ids = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(record) = row_record(row) else {
                continue;
            };
            let media_asset_id = record.media_asset_id.trim().to_owned();
            if media_asset_id.is_empty() {
                continue;
            }
            let mut record = record;
            record.media_asset_id = media_asset_id.clone();
            records.push(record);
            asset_ids.push(media_asset_id);
        }
        if records.is_empty() {
            Vec::new()
        } else {
            let assets = load_assets_by_id(asset_ids)?;
            let mut items = Vec::with_capacity(records.len());
            for (index, record) in records.iter().enumerate() {
                let Some(asset) = assets.get(&record.media_asset_id) else {
                    continue;
                };
                if let Some(item) = build_item(index, asset.clone(), record.clone()) {
                    items.push(item);
                }
            }
            items
        }
    }
}

/// Read the owner's current attachment inputs.
///
/// The request's deduplicated form selection wins whenever it is non-empty.
/// Otherwise the supplied item loader, when present, is called and its presented
/// items are converted to deduplicated inputs with the same role normalizer. An
/// absent loader yields an empty result after the request selection is read.
pub fn current_media_attachment_inputs<E, R>(
    options: CurrentMediaAttachmentInputOptions<E, R>,
) -> Effect<Vec<MediaAttachmentInput>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let CurrentMediaAttachmentInputOptions {
        request,
        form,
        normalize_role,
        load_current_items,
    } = options;
    effect! {
        let request = request.clone();
        let form = form.clone();
        let normalize_role = normalize_role.clone();
        let load_current_items = load_current_items.clone();
        let requested = dedupe_media_attachment_inputs(
            &collect_media_attachment_inputs(request.as_ref().and(form.as_deref()), normalize_role.as_deref()),
            normalize_role.as_deref(),
        );
        if requested.is_empty() {
            match load_current_items {
                Some(load_current_items) => {
                    let items = load_current_items()?;
                    media_attachment_inputs_from_items(&items, normalize_role.as_deref())
                }
                None => Vec::new(),
            }
        } else {
            requested
        }
    }
}

/// Replace an owner's attachments from the request form selection.
///
/// An absent row builder or replacement capability yields an empty result without
/// reading the request. Otherwise the merged form selection is collected in
/// position order, the row builder is called once per collected input, and the
/// accepted rows replace the owner's rows in one call. An empty selection still
/// performs that one replacement with an empty batch. A replacement failure is
/// returned unchanged and prevents loading; on success the optional item loader
/// supplies the returned items, and an absent loader yields an empty result.
pub fn replace_media_attachments_from_request<Row, E, R>(
    options: MediaAttachmentReplaceOptions<Row, E, R>,
) -> Effect<Vec<MediaAttachmentFormItem>, E, R>
where
    Row: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let MediaAttachmentReplaceOptions {
        request,
        form,
        normalize_role,
        build_row,
        replace_rows,
        load_replaced_items,
    } = options;
    let (Some(build_row), Some(replace_rows)) = (build_row, replace_rows) else {
        return effect_sync! { body: || Vec::new() };
    };
    effect! {
        let request = request.clone();
        let form = form.clone();
        let normalize_role = normalize_role.clone();
        let build_row = build_row.clone();
        let replace_rows = replace_rows.clone();
        let load_replaced_items = load_replaced_items.clone();
        let inputs = collect_media_attachment_inputs(request.as_ref().and(form.as_deref()), normalize_role.as_deref());
        let admitted = AdmittedAttachmentRows::build(inputs, build_row.as_ref());
        let rows = admitted.into_rows();
        replace_rows(rows)?;
        match load_replaced_items {
            Some(load_replaced_items) => {
                let items = load_replaced_items()?;
                items
            }
            None => Vec::new(),
        }
    }
}

/// Replace one owner's attachments through a runtime capability.
///
/// The runtime owns the replacement details; this owner passes the exact request,
/// owner type and owner identity and returns the runtime's error unchanged. The
/// runtime's returned items are not part of this operation's result.
pub fn replace_resource_media_attachments_from_request<E, R>(
    runtime: Arc<dyn AttachmentReplacer<E, R>>,
    request: Option<Request>,
    owner_type: &ResourceKind,
    owner_content_id: &InputBytes,
) -> Effect<(), E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let owner_type = owner_type.as_str().to_owned();
    let owner_content_id = owner_content_id.to_owned();
    effect! {
        let runtime = runtime.clone();
        let request = request.clone();
        let owner_type = owner_type.clone();
        let owner_content_id = owner_content_id.clone();
        let _items = runtime.replace_attachments_from_request(
            request,
            owner_type,
            owner_content_id,
        )?;
        ()
    }
}

/// Accepted rows waiting for the one replacement call.
struct AdmittedAttachmentRows<Row> {
    /// Accepted rows in input order.
    rows: Vec<Row>,
}

impl<Row> AdmittedAttachmentRows<Row> {
    /// Build one fresh stage from accepted inputs.
    fn build(
        inputs: Vec<MediaAttachmentInput>,
        build_row: &(dyn Fn(MediaAttachmentInput) -> Option<Row> + Send + Sync),
    ) -> Self {
        let mut rows = Vec::with_capacity(inputs.len());
        for input in inputs {
            if let Some(row) = build_row(input) {
                rows.push(row);
            }
        }
        Self { rows }
    }

    /// Consume the stage and transfer its rows to the replacement capability.
    fn into_rows(self) -> Vec<Row> {
        self.rows
    }
}

/// Preview inputs with required capabilities.
fn preview_form_items_from_inputs<Asset, E, R>(
    inputs: Vec<MediaAttachmentInput>,
    normalize_role: Option<RoleNormalizer>,
    load_assets_by_id: MediaAttachmentAssetLoader<Asset, E, R>,
    build_item: MediaAttachmentItemBuilder<Asset>,
) -> Effect<Vec<MediaAttachmentFormItem>, E, R>
where
    Asset: Clone + Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    effect! {
        let inputs = inputs.clone();
        let normalize_role = normalize_role.clone();
        let load_assets_by_id = load_assets_by_id.clone();
        let build_item = build_item.clone();
        let asset_ids = attachment_asset_ids(&inputs);
        if asset_ids.is_empty() {
            Vec::new()
        } else {
            let assets = load_assets_by_id(asset_ids)?;
            let mut items = Vec::with_capacity(inputs.len());
            for (index, input) in inputs.iter().enumerate() {
                let asset_id = input.asset_id.trim();
                if asset_id.is_empty() {
                    continue;
                }
                let Some(asset) = assets.get(&asset_id) else {
                    continue;
                };
                let record = MediaAttachmentStoreRecord {
                    id: InputBytes::from([format!("preview-{index}-").as_bytes(), asset_id.as_bytes()].concat()),
                    media_asset_id: asset_id.to_owned(),
                    role: normalize_media_attachment_role_with(
                        &input.role,
                        normalize_role.as_deref(),
                    ),
                    title_override: input.title_override.trim().to_owned(),
                    caption: input.caption.trim().to_owned(),
                    alt_override: input.alt_override.trim().to_owned(),
                };
                if let Some(item) = build_item(index, asset.clone(), record) {
                    items.push(item);
                }
            }
            items
        }
    }
}

/// Collect the nonblank trimmed identities in input order.
fn attachment_asset_ids(inputs: &[MediaAttachmentInput]) -> Vec<InputBytes> {
    let mut asset_ids = Vec::with_capacity(inputs.len());
    for input in inputs {
        let asset_id = input.asset_id.trim();
        if !asset_id.is_empty() {
            asset_ids.push(asset_id.to_owned());
        }
    }
    asset_ids
}

/// Normalize one role with the caller's optional normalizer and trim the result.
fn normalize_media_attachment_role_with(
    raw: &InputBytes,
    normalize_role: Option<RoleNormalizerRef<'_>>,
) -> InputBytes {
    match normalize_role {
        Some(normalize_role) => normalize_role(raw.as_bytes()).trim().to_owned(),
        None => raw.trim().to_owned(),
    }
}
