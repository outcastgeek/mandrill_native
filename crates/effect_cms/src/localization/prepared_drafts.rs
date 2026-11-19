//! Region-backed localized authoring drafts.

use std::alloc::Layout;
use std::num::NonZeroUsize;
use std::ops::Range;

use effect::memory::{
    Buffer, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};

use crate::forms::PreparedMergedForm;
use crate::validation::trim_unicode_whitespace;

use super::{
    FormatNormalizerRef, LOCALIZED_CONTENT_FIELD_KEY, LOCALIZED_CONTENT_FORMAT_FIELD_KEY,
    LOCALIZED_DESCRIPTION_FIELD_KEY, NormalizerRef,
};

/// Concurrent snapshots and per-snapshot limits for localized draft output.
#[derive(Clone, Copy)]
pub struct LocalizedDraftCapacity {
    pub snapshots: NonZeroUsize,
    pub drafts: NonZeroUsize,
    pub bytes: Bytes,
}

/// Select content fields or one text field for each configured language.
#[derive(Clone, Copy)]
pub enum LocalizedDraftFields<'a> {
    Content {
        format: Option<FormatNormalizerRef<'a>>,
    },
    Text {
        prefix: &'a str,
    },
}

/// Startup workspace for localized drafts retaining their exact input bytes.
pub struct PreparedLocalizedDrafts {
    slots: SlotPool<'static, DraftStorage>,
    maximum_drafts: NonZeroUsize,
}

struct DraftStorage {
    records: FixedVector<'static, DraftRecord>,
    bytes: Buffer<'static>,
}

struct DraftRecord {
    language: Range<usize>,
    fields: DraftFieldRanges,
}

struct DraftFieldRanges {
    description: Range<usize>,
    content: Range<usize>,
    content_format: Range<usize>,
}

/// A localized draft collection that keeps its region backing checked out.
pub struct LocalizedDraftSnapshot {
    storage: SlotLease<'static, DraftStorage>,
}

/// Borrowed values of the corresponding localized authoring draft.
pub struct LocalizedDraftView<'a> {
    pub lang: &'a str,
    pub description: &'a [u8],
    pub content: &'a [u8],
    pub content_format: &'a [u8],
    pub required: bool,
}

impl PreparedLocalizedDrafts {
    /// Calculate record, byte and slot payload demand.
    pub fn storage_bytes(capacity: LocalizedDraftCapacity) -> Result<Bytes, MemoryError> {
        let records = Layout::array::<DraftRecord>(capacity.drafts.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let bytes = Bytes::new(records.size()).checked_add(capacity.bytes)?;
        SlotPool::<DraftStorage>::storage_bytes(capacity.snapshots.get())?
            .checked_add(bytes.checked_mul(capacity.snapshots.get())?)
    }

    /// Prepare all record and byte slots from the caller's selected region.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: LocalizedDraftCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(capacity)?;
        let slots = memory.slots(region, capacity.snapshots.get(), |_| {
            Ok(DraftStorage {
                records: FixedVector::prepare(memory.allocator(region)?, capacity.drafts.get())?,
                bytes: memory.buffer(region, capacity.bytes)?,
            })
        })?;
        Ok(Self {
            slots,
            maximum_drafts: capacity.drafts,
        })
    }

    /// Collect ordered draft rows using the prepared merged form's precedence.
    /// Storage is reserved before invoking either native normalization callback.
    pub fn collect(
        &self,
        form: Option<&PreparedMergedForm>,
        languages: &[String],
        fields: LocalizedDraftFields<'_>,
        normalize: Option<NormalizerRef<'_>>,
    ) -> Result<LocalizedDraftSnapshot, MemoryError> {
        if languages.len() > self.maximum_drafts.get() {
            return Err(MemoryError::NoSlot);
        }
        let mut storage = self.slots.try_checkout()?;
        storage.records.clear();
        storage.bytes.clear();
        for language in languages {
            let fields = match fields {
                LocalizedDraftFields::Content { format } => {
                    let description =
                        lookup(form, LOCALIZED_DESCRIPTION_FIELD_KEY, language, normalize);
                    let content = lookup(form, LOCALIZED_CONTENT_FIELD_KEY, language, normalize);
                    let raw_format = lookup(
                        form,
                        LOCALIZED_CONTENT_FORMAT_FIELD_KEY,
                        language,
                        normalize,
                    );
                    let normalized = format.map(|f| f(raw_format));
                    let format = normalized
                        .as_ref()
                        .map_or(raw_format, |value| value.as_bytes());
                    store_fields(&mut storage.bytes, description, content, format)?
                }
                LocalizedDraftFields::Text { prefix } => {
                    let description = lookup(form, prefix, language, normalize);
                    store_fields(&mut storage.bytes, description, &[], &[])?
                }
            };
            let language = append(&mut storage.bytes, language.as_bytes())?;
            storage
                .records
                .push(DraftRecord { language, fields })
                .map_err(|_| MemoryError::NoSlot)?;
        }
        Ok(LocalizedDraftSnapshot { storage })
    }
}

fn lookup<'a>(
    form: Option<&'a PreparedMergedForm>,
    prefix: &str,
    language: &str,
    normalize: Option<NormalizerRef<'_>>,
) -> &'a [u8] {
    let Some(form) = form else { return &[] };
    let normalized = normalize.map(|normalize| normalize(language));
    let suffix = normalized
        .as_deref()
        .unwrap_or(language)
        .trim()
        .bytes()
        .map(|byte| if byte == b'-' { b'_' } else { byte });
    let prefix = prefix.trim().as_bytes().iter().copied().chain(*b"_");
    let key = prefix.chain(suffix);
    form.pairs()
        .find_map(|(name, value)| {
            name.iter()
                .copied()
                .eq(key.clone())
                .then(|| trim_unicode_whitespace(value))
        })
        .unwrap_or_default()
}

fn append(output: &mut Buffer<'_>, bytes: &[u8]) -> Result<Range<usize>, MemoryError> {
    let start = output.len();
    output.extend_from_slice(bytes)?;
    Ok(start..output.len())
}

fn store_fields(
    output: &mut Buffer<'_>,
    description: &[u8],
    content: &[u8],
    format: &[u8],
) -> Result<DraftFieldRanges, MemoryError> {
    Ok(DraftFieldRanges {
        description: append(output, description)?,
        content: append(output, content)?,
        content_format: append(output, format)?,
    })
}

impl LocalizedDraftSnapshot {
    /// Borrow draft records in caller language order.
    pub fn drafts(&self) -> impl Iterator<Item = LocalizedDraftView<'_>> {
        let bytes = self.storage.bytes.as_slice();
        self.storage
            .records
            .as_slice()
            .iter()
            .filter_map(move |record| {
                let lang = std::str::from_utf8(bytes.get(record.language.clone())?).ok()?;
                Some(LocalizedDraftView {
                    lang,
                    description: bytes.get(record.fields.description.clone())?,
                    content: bytes.get(record.fields.content.clone())?,
                    content_format: bytes.get(record.fields.content_format.clone())?,
                    required: true,
                })
            })
    }
}
