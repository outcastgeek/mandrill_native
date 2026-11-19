//! The selected attachment replacement working set.

use super::{MediaAttachmentItemLoader, MediaAttachmentListOptions, MediaAttachmentPreviewOptions};
use crate::attachments::{
    MediaAttachmentFormItem, MediaAttachmentStoreRecord, PreparedAttachmentInput,
    PreparedAttachmentPool, RoleNormalizer,
};
use crate::commands::InputBytes;
use effect::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
use effect::{Effect, effect, effect_sync, effect_try_sync};
use effect_server::{ByteFormData, Request};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// The replacement batch retains its actual regional rows until the consumer settles.
#[must_use]
pub struct PreparedAttachmentRows<Row: Send + 'static> {
    rows: SlotLease<'static, FixedVector<'static, Row>>,
}

impl<Row: Send + 'static> PreparedAttachmentRows<Row> {
    /// Borrow accepted rows in their original selection order.
    #[must_use]
    pub fn as_slice(&self) -> &[Row] {
        self.rows.as_slice()
    }

    pub(crate) fn take_consumer_rows(&mut self) -> Vec<Row> {
        let mut rows = Vec::with_capacity(self.rows.len());
        while let Some(row) = self.rows.pop() {
            rows.push(row);
        }
        rows.reverse();
        rows
    }
}
impl<Row: Send + 'static> Drop for PreparedAttachmentRows<Row> {
    fn drop(&mut self) {
        self.rows.clear();
    }
}

/// Reusable physical replacement batches for an existing consumer row type.
pub struct PreparedAttachmentRowPool<Row: Send + 'static> {
    batches: SlotPool<'static, FixedVector<'static, Row>>,
}
impl<Row: Send + 'static> PreparedAttachmentRowPool<Row> {
    /// Checked regional payload bytes for the batch and row storage.
    pub fn storage_bytes(calls: NonZeroUsize, rows: NonZeroUsize) -> Result<Bytes, MemoryError> {
        let layout =
            std::alloc::Layout::array::<Row>(rows.get()).map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<FixedVector<'static, Row>>::storage_bytes(calls.get())?
            .checked_add(Bytes::new(layout.size()).checked_mul(calls.get())?)
    }

    /// Prepare complete replacement batches before work may reach the consumer.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        calls: NonZeroUsize,
        rows: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(calls, rows)?;
        Ok(Self {
            batches: memory.slots(region, calls.get(), |_| {
                FixedVector::prepare(memory.allocator(region)?, rows.get())
            })?,
        })
    }
}

/// Build a consumer-owned row from one admitted attachment view.
pub type PreparedAttachmentRowBuilder<Row> =
    Arc<dyn for<'row> Fn(PreparedAttachmentInput<'row>) -> Option<Row> + Send + Sync>;

/// Transfer the admitted replacement batch to the existing consumer authority.
pub type PreparedAttachmentRowReplacer<Row, E, R> =
    Arc<dyn Fn(PreparedAttachmentRows<Row>) -> Effect<(), E, R> + Send + Sync>;

/// Selected request input, workspaces and replacement capabilities.
pub struct PreparedAttachmentReplaceOptions<
    Row: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    /// The request whose form selection is replaced.
    pub request: Option<Request>,
    /// The existing initialized merged form.
    pub form: Option<Arc<ByteFormData>>,
    /// The selected optional role normalizer.
    pub normalize_role: Option<RoleNormalizer>,
    /// Physical storage for the input snapshot.
    pub inputs: Arc<PreparedAttachmentPool>,
    /// Physical storage for the consumer replacement rows.
    pub rows: Arc<PreparedAttachmentRowPool<Row>>,
    /// Optional consumer row builder.
    pub build_row: Option<PreparedAttachmentRowBuilder<Row>>,
    /// Optional consumer row replacer.
    pub replace_rows: Option<PreparedAttachmentRowReplacer<Row, E, R>>,
    /// Optional loader for the consumer-owned presented items.
    pub load_replaced_items: Option<MediaAttachmentItemLoader<E, R>>,
}

/// Admit the input snapshot and complete replacement batch before building rows.
/// An absent builder or replacer still reads nothing. A successful empty batch
/// still invokes the replacer once, and loading is subsequent to its success.
pub fn replace_prepared_media_attachments_from_request<Row, E, R>(
    options: PreparedAttachmentReplaceOptions<Row, E, R>,
    on_memory: impl Fn(MemoryError) -> E + Send + Sync + 'static,
) -> Effect<Vec<MediaAttachmentFormItem>, E, R>
where
    Row: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let PreparedAttachmentReplaceOptions {
        request,
        form,
        normalize_role,
        inputs,
        rows,
        build_row,
        replace_rows,
        load_replaced_items,
    } = options;
    let (Some(build_row), Some(replace_rows)) = (build_row, replace_rows) else {
        return effect_sync! { body: Vec::new };
    };
    let build = effect_try_sync! {
        body: move || {
            let mut result = PreparedAttachmentRows { rows: rows.batches.try_checkout()? };
            let selected = inputs.collect(request.as_ref().and(form.as_deref()), normalize_role.as_deref())?;
            let inputs = selected.iter();
            if inputs.len() > result.rows.capacity() {
                return Err(MemoryError::NoSlot);
            }
            for input in inputs {
                if let Some(row) = build_row(input) {
                    result.rows.push(row).map_err(|_| MemoryError::NoSlot)?;
                }
            }
            Ok(result)
        },
        on_error: on_memory,
    };
    effect! {
        let build = build.clone();
        let replace_rows = replace_rows.clone();
        let load_replaced_items = load_replaced_items.clone();
        let rows = build?;
        replace_rows(rows)?;
        match load_replaced_items {
            Some(load) => load()?,
            None => Vec::new(),
        }
    }
}

/// Preview the initialized request form with selected input and output working sets.
///
/// The consumer's asset lookup receives the same ordered identity list. Its native
/// asset map remains provider-owned. The returned items occupy the actual output table.
pub fn preview_prepared_media_attachment_form_items_from_request<Asset, E, R>(
    options: MediaAttachmentPreviewOptions<Asset, E, R>,
    inputs: Arc<PreparedAttachmentPool>,
    output: Arc<PreparedAttachmentRowPool<MediaAttachmentFormItem>>,
    metadata_bytes: Bytes,
    on_memory: impl Fn(MemoryError) -> E + Send + Sync + 'static,
) -> Effect<PreparedAttachmentRows<MediaAttachmentFormItem>, E, R>
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
    let input_normalizer = normalize_role.clone();
    let on_memory = Arc::new(on_memory);
    let admit_error = on_memory.clone();
    let admit = effect_try_sync! {
        body: move || {
            let output = PreparedAttachmentRows { rows: output.batches.try_checkout()? };
            let inputs = inputs.collect(request.as_ref().and(form.as_deref()), input_normalizer.as_deref())?;
            if inputs.iter().len() > output.rows.capacity() {
                return Err(MemoryError::NoSlot);
            }
            Ok((inputs, output))
        },
        on_error: move |error| admit_error(error),
    };
    effect! {
        let admit = admit.clone();
        let load_assets_by_id = load_assets_by_id.clone();
        let build_item = build_item.clone();
        let normalize_role = normalize_role.clone();
        let on_memory = on_memory.clone();
        let (inputs, output) = admit?;
        match (load_assets_by_id, build_item) {
            (Some(load_assets), Some(build_item)) => {
                let asset_ids: Vec<_> = inputs.iter().map(|row| InputBytes::from(row.asset_id)).collect();
                if asset_ids.is_empty() {
                    output
                } else {
                    let assets = load_assets(asset_ids)?;
                    effect_try_sync! {
                        once;
                        body: move || {
                            let mut output = output;
                            let mut retained = 0usize;
                            for (index, input) in inputs.iter().enumerate() {
                                let Some(asset) = assets.get(input.asset_id) else { continue };
                                let mut id = format!("preview-{index}-").into_bytes();
                                id.extend_from_slice(input.asset_id);
                                let role = normalize_role.as_ref().map_or_else(
                                    || InputBytes::from(input.role),
                                    |normalize| normalize(input.role).trim(),
                                );
                                check_metadata(role.len(), metadata_bytes)?;
                                let record = MediaAttachmentStoreRecord {
                                    id: InputBytes::from(id),
                                    media_asset_id: InputBytes::from(input.asset_id),
                                    role,
                                    title_override: InputBytes::from(input.title_override),
                                    caption: InputBytes::from(input.caption),
                                    alt_override: InputBytes::from(input.alt_override),
                                };
                                if let Some(item) = build_item(index, asset.clone(), record) {
                                    retained = retained.checked_add(item_metadata(&item)?)
                                        .ok_or(MemoryError::ArithmeticOverflow)?;
                                    check_metadata(retained, metadata_bytes)?;
                                    output.rows.push(item).map_err(|_| MemoryError::NoSlot)?;
                                }
                            }
                            Ok(output)
                        },
                        on_error: move |error| on_memory(error),
                    }?
                }
            }
            _ => output,
        }
    }
}

fn item_metadata(item: &MediaAttachmentFormItem) -> Result<usize, MemoryError> {
    [
        item.key.len(),
        item.asset_id.len(),
        item.kind.len(),
        item.role.len(),
        item.status.len(),
        item.title.len(),
        item.description.len(),
        item.provider.len(),
        item.url.len(),
        item.thumbnail_url.len(),
        item.resolved_url.len(),
        item.resolved_thumbnail.len(),
        item.alt.len(),
        item.title_override.len(),
        item.caption.len(),
        item.alt_override.len(),
        item.shortcode.len(),
        item.edit_url.len(),
    ]
    .into_iter()
    .try_fold(0usize, |total, bytes| total.checked_add(bytes))
    .ok_or(MemoryError::ArithmeticOverflow)
}

/// Admit row projection and presented-item storage before asking the existing
/// provider for an owner's attachments. Provider row and asset owners retain
/// their existing contracts.
pub fn list_prepared_media_attachment_form_items<Asset, Row, E, R>(
    options: MediaAttachmentListOptions<Asset, Row, E, R>,
    records: Arc<PreparedAttachmentRowPool<MediaAttachmentStoreRecord>>,
    output: Arc<PreparedAttachmentRowPool<MediaAttachmentFormItem>>,
    metadata_bytes: Bytes,
    on_memory: impl Fn(MemoryError) -> E + Send + Sync + 'static,
) -> Effect<PreparedAttachmentRows<MediaAttachmentFormItem>, E, R>
where
    Asset: Clone + Send + 'static,
    Row: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let on_memory = Arc::new(on_memory);
    let admission_error = on_memory.clone();
    let admit = effect_try_sync! {
        body: move || reserve_attachment_list(&records, &output),
        on_error: move |error| admission_error(error),
    };
    list_prepared_admitted(options, admit, metadata_bytes, on_memory)
}

pub(crate) struct PreparedAttachmentListBatch {
    records: PreparedAttachmentRows<MediaAttachmentStoreRecord>,
    output: PreparedAttachmentRows<MediaAttachmentFormItem>,
}

pub(crate) fn reserve_attachment_list(
    records: &PreparedAttachmentRowPool<MediaAttachmentStoreRecord>,
    output: &PreparedAttachmentRowPool<MediaAttachmentFormItem>,
) -> Result<PreparedAttachmentListBatch, MemoryError> {
    Ok(PreparedAttachmentListBatch {
        records: PreparedAttachmentRows {
            rows: records.batches.try_checkout()?,
        },
        output: PreparedAttachmentRows {
            rows: output.batches.try_checkout()?,
        },
    })
}

pub(crate) fn list_prepared_admitted<Asset, Row, E, R>(
    options: MediaAttachmentListOptions<Asset, Row, E, R>,
    admit: Effect<PreparedAttachmentListBatch, E, R>,
    metadata_bytes: Bytes,
    on_memory: Arc<dyn Fn(MemoryError) -> E + Send + Sync>,
) -> Effect<PreparedAttachmentRows<MediaAttachmentFormItem>, E, R>
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
    let owner_present =
        !crate::validation::trim_unicode_whitespace(owner_content_id.as_bytes()).is_empty();
    effect! {
        let admit = admit.clone();
        let list_rows = list_rows.clone();
        let row_record = row_record.clone();
        let load_assets_by_id = load_assets_by_id.clone();
        let build_item = build_item.clone();
        let on_memory = on_memory.clone();
        let batch = admit?;
        let PreparedAttachmentListBatch { records, output } = batch;
        match (owner_present, list_rows, row_record, load_assets_by_id, build_item) {
            (true, Some(list), Some(project), Some(load_assets), Some(build_item)) => {
                let rows = list()?;
                let project_error = on_memory.clone();
                let records = effect_try_sync! {
                    once;
                    body: move || {
                        let mut records = records;
                        if rows.len() > records.rows.capacity().min(output.rows.capacity()) {
                            return Err(MemoryError::NoSlot);
                        }
                        let mut bytes = 0usize;
                        for row in rows {
                            let Some(mut record) = project(row) else { continue };
                            let asset_id = record.media_asset_id.trim();
                            if asset_id.is_empty() { continue; }
                            record.media_asset_id = asset_id;
                            let lengths = [
                                record.id.len(), record.media_asset_id.len(), record.role.len(),
                                record.title_override.len(), record.caption.len(), record.alt_override.len(),
                            ];
                            bytes = lengths.into_iter().try_fold(bytes, |total, field| total.checked_add(field))
                                .ok_or(MemoryError::ArithmeticOverflow)?;
                            check_metadata(bytes, metadata_bytes)?;
                            records.rows.push(record).map_err(|_| MemoryError::NoSlot)?;
                        }
                        Ok((records, output))
                    },
                    on_error: move |error| project_error(error),
                }?;
                let (records, output) = records;
                if records.as_slice().is_empty() {
                    output
                } else {
                    let ids = records.as_slice().iter().map(|record| record.media_asset_id.clone()).collect();
                    let assets = load_assets(ids)?;
                    effect_try_sync! {
                        once;
                        body: move || {
                            let mut output = output;
                            let mut retained = 0usize;
                            for (index, record) in records.as_slice().iter().enumerate() {
                                let Some(asset) = assets.get(&record.media_asset_id) else { continue };
                                if let Some(item) = build_item(index, asset.clone(), record.clone()) {
                                    retained = retained.checked_add(item_metadata(&item)?)
                                        .ok_or(MemoryError::ArithmeticOverflow)?;
                                    check_metadata(retained, metadata_bytes)?;
                                    output.rows.push(item).map_err(|_| MemoryError::NoSlot)?;
                                }
                            }
                            Ok(output)
                        },
                        on_error: move |error| on_memory(error),
                    }?
                }
            }
            _ => output,
        }
    }
}

fn check_metadata(bytes: usize, limit: Bytes) -> Result<(), MemoryError> {
    if bytes > limit.get() {
        return Err(MemoryError::ValueTooLarge {
            requested: Bytes::new(bytes),
            capacity: limit,
        });
    }
    Ok(())
}
