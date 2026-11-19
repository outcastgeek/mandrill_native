//! Validated culture identities and bounded JSON catalogue resources.

use super::message::{visit_template, write_bounded};
use super::{ArgumentList, LocalizationError, MessageContract, Template};
use crate::{effect_future, effect_try_blocking};
use bytes::Bytes;
use effect::memory::{
    Buffer, BufferView, Bytes as MemoryBytes, FixedVector, MemoryError, PreparedMemory, RegionId,
    SlotLease, SlotPool,
};
use effect::{Effect, Exit};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::Arc;

/// Syntactically validated BCP 47 culture identity. Selection additionally checks
/// the application's supported culture set; no process-global locale is consulted.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub struct CultureId(Arc<str>);
impl CultureId {
    /// Validate a language tag and normalize case for stable equality. Actual
    /// culture parents are declared explicitly by CultureSet, not guessed by truncation.
    pub fn new(value: &str) -> Result<Self, LocalizationError> {
        let normalized = value.to_ascii_lowercase();
        if !valid_language_tag(&normalized) {
            return Err(LocalizationError::Definition("culture".into()));
        }
        Ok(Self(value.to_ascii_lowercase().into()))
    }
    /// Borrow normalized culture metadata for native formatting/response headers.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Validate RFC 5646 subtag structure without claiming registry membership or
/// guessing a culture's parent. Applications explicitly select supported tags.
fn valid_language_tag(value: &str) -> bool {
    // Grandfathered registered syntax has no ordinary language-tag decomposition.
    if matches!(
        value,
        "art-lojban"
            | "cel-gaulish"
            | "en-gb-oed"
            | "i-ami"
            | "i-bnn"
            | "i-default"
            | "i-enochian"
            | "i-hak"
            | "i-klingon"
            | "i-lux"
            | "i-mingo"
            | "i-navajo"
            | "i-pwn"
            | "i-tao"
            | "i-tay"
            | "i-tsu"
            | "no-bok"
            | "no-nyn"
            | "sgn-be-fr"
            | "sgn-be-nl"
            | "sgn-ch-de"
            | "zh-guoyu"
            | "zh-hakka"
            | "zh-min"
            | "zh-min-nan"
            | "zh-xiang"
    ) {
        return true;
    }
    let mut parts = value.split('-').peekable();
    let Some(language) = parts.next() else {
        return false;
    };
    if language == "x" {
        let mut count = 0usize;
        for part in parts {
            if !(1..=8).contains(&part.len())
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                return false;
            }
            count += 1;
        }
        return count != 0;
    }
    if !(2..=8).contains(&language.len())
        || !language.bytes().all(|byte| byte.is_ascii_alphabetic())
    {
        return false;
    }
    if language.len() <= 3 {
        for _ in 0..3 {
            if parts.peek().is_some_and(|part| {
                part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_alphabetic())
            }) {
                parts.next();
            } else {
                break;
            }
        }
    }
    if parts
        .peek()
        .is_some_and(|part| part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
    {
        parts.next();
    }
    if parts.peek().is_some_and(|part| {
        (part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
            || (part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) {
        parts.next();
    }
    let mut variants = BTreeSet::new();
    while let Some(part) = parts.peek().copied() {
        if ((5..=8).contains(&part.len())
            || (part.len() == 4 && part.as_bytes().first().is_some_and(u8::is_ascii_digit)))
            && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            if !variants.insert(part) {
                return false;
            }
            parts.next();
        } else {
            break;
        }
    }
    let mut extensions = BTreeSet::new();
    while let Some(part) = parts.next() {
        if part == "x" {
            let mut count = 0usize;
            for part in parts {
                if !(1..=8).contains(&part.len())
                    || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
                {
                    return false;
                }
                count += 1;
            }
            return count != 0;
        }
        if part.len() != 1
            || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
            || !extensions.insert(part)
        {
            return false;
        }
        let mut count = 0usize;
        while let Some(part) = parts.peek() {
            if !(2..=8).contains(&part.len())
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                break;
            }
            parts.next();
            count += 1;
        }
        if count == 0 {
            return false;
        }
    }
    true
}
/// Explicit supported cultures, their actual declared parents and default fallback.
#[derive(Clone)]
#[must_use]
pub struct CultureSet {
    /// Selection allowlist, including configured parents and default.
    supported: Arc<[CultureId]>,
    /// Exact declared parent relation; cycles/missing parents are rejected.
    metadata: Arc<BTreeMap<CultureId, CultureMetadata>>,
    /// Last lookup fallback selected by the application.
    fallback: CultureId,
}

#[derive(Clone)]
pub(super) struct CultureMetadata {
    pub(super) precedence: Arc<[CultureId]>,
    pub(super) content_language: http::HeaderValue,
}

impl CultureSet {
    /// Prepare a nonempty culture set. The parent relation is explicit because
    /// locale inheritance can differ from blindly removing a hyphen segment.
    pub fn new(
        supported: Vec<CultureId>,
        parents: Vec<(CultureId, CultureId)>,
        fallback: CultureId,
    ) -> Result<Self, LocalizationError> {
        if supported.is_empty()
            || !supported.contains(&fallback)
            || supported.iter().collect::<BTreeSet<_>>().len() != supported.len()
        {
            return Err(LocalizationError::Definition(
                "supported cultures/default".into(),
            ));
        }
        let mut relation = BTreeMap::new();
        for (child, parent) in parents {
            if !supported.contains(&child)
                || !supported.contains(&parent)
                || relation.insert(child, parent).is_some()
            {
                return Err(LocalizationError::Definition("culture parents".into()));
            }
        }
        let mut metadata = BTreeMap::new();
        for culture in &supported {
            let mut seen = BTreeSet::new();
            let mut current = culture;
            let mut chain = vec![culture.clone()];
            while let Some(parent) = relation.get(current) {
                if !seen.insert(current) {
                    return Err(LocalizationError::Definition("culture parent cycle".into()));
                }
                chain.push(parent.clone());
                current = parent;
            }
            if !chain.contains(&fallback) {
                chain.push(fallback.clone());
            }
            let content_language = http::HeaderValue::from_str(culture.as_str())
                .map_err(|_| LocalizationError::Definition("content language".into()))?;
            metadata.insert(
                culture.clone(),
                CultureMetadata {
                    precedence: chain.into(),
                    content_language,
                },
            );
        }
        Ok(Self {
            supported: supported.into(),
            metadata: Arc::new(metadata),
            fallback,
        })
    }
    /// Whether this UI/formatting culture is explicitly selectable.
    #[must_use]
    pub fn contains(&self, culture: &CultureId) -> bool {
        self.supported.contains(culture)
    }
    /// Borrow the application's default culture.
    #[must_use]
    pub fn fallback(&self) -> &CultureId {
        &self.fallback
    }
    /// Return exact/actual-parent/default precedence without duplicate fallback entries.
    pub fn precedence(&self, culture: &CultureId) -> Result<Vec<CultureId>, LocalizationError> {
        Ok(self.metadata(culture)?.precedence.to_vec())
    }

    pub(super) fn metadata(
        &self,
        culture: &CultureId,
    ) -> Result<CultureMetadata, LocalizationError> {
        self.metadata
            .get(culture)
            .cloned()
            .ok_or_else(|| LocalizationError::Definition("unsupported culture".into()))
    }
}
/// Explicit resource and formatted-output limits, with no hidden catalogue defaults.
#[derive(Clone, Copy)]
#[must_use]
pub struct CatalogLimits {
    /// Maximum complete encoded resource bytes before parsing.
    pub bytes: NonZeroUsize,
    /// Maximum nested object/array depth, checked by the seed before descent.
    pub depth: NonZeroUsize,
    /// Maximum object keys and array positions examined per resource.
    pub keys: NonZeroUsize,
    /// Maximum encoded bytes in one flattened key.
    pub key_bytes: NonZeroUsize,
    /// Maximum encoded bytes in one string template.
    pub value_bytes: NonZeroUsize,
    /// Maximum encoded bytes in one formatted message result.
    pub message_bytes: NonZeroUsize,
}
/// Missing-message behavior is explicit; Key returns the typed contract identity,
/// never an unvalidated raw template or substitute English text.
#[derive(Clone, Copy)]
#[must_use]
pub enum MissingMessage {
    /// Missing lookup is a typed failure.
    Fail,
    /// Return the known message key for intentionally selected diagnostic UI.
    Key,
}
/// Catalogue resource loader. The Effect acquires/reads its real owner when run;
/// no filesystem/network source is queried by definition construction.
#[must_use]
pub struct CatalogResource<R: Send + Sync + 'static> {
    /// Safe resource identity used by protected status.
    pub identity: Arc<str>,
    /// Culture whose entries this resource supplies.
    pub culture: CultureId,
    /// Owned finite source callback, retaining typed loader failures.
    pub load: Arc<dyn Fn() -> Effect<Bytes, LocalizationError, R> + Send + Sync>,
}
impl<R: effect::HasBlockingExecutor + Send + Sync + 'static> CatalogResource<R> {
    /// Select a real file whose native owner closes on the blocking provider before
    /// publication. Read at most maximum + one sentinel byte; reload never first
    /// allocates an unbounded complete file and then discovers the selected limit.
    pub fn file(
        identity: impl Into<Arc<str>>,
        culture: CultureId,
        path: std::path::PathBuf,
        maximum: NonZeroUsize,
    ) -> Self {
        let identity = identity.into();
        let source_identity = identity.clone();
        Self {
            identity,
            culture,
            load: Arc::new(move || {
                let path = path.clone();
                let identity = source_identity.clone();
                effect_try_blocking! {
                body: move |_| {
                    use std::io::Read;
                    let length = maximum
                        .get()
                        .checked_add(1)
                        .and_then(|length| u64::try_from(length).ok())
                        .ok_or_else(|| LocalizationError::Limit("file read bytes".into()))?;
                    let file = std::fs::File::open(&path)
                        .map_err(|_| LocalizationError::Load(identity.to_string()))?;
                    let mut bytes = Vec::new();
                    file.take(length)
                        .read_to_end(&mut bytes)
                        .map_err(|_| LocalizationError::Load(identity.to_string()))?;
                    if bytes.len() > maximum.get() {
                        return Err(LocalizationError::Limit("resource bytes".into()));
                    }
                    Ok(Bytes::from(bytes))
                }, on_error: |error| error, }
            }),
        }
    }

    pub(super) fn prepared_file(
        identity: impl Into<Arc<str>>,
        culture: CultureId,
        path: std::path::PathBuf,
        maximum: NonZeroUsize,
        buffers: Arc<SlotPool<'static, Buffer<'static>>>,
    ) -> Self {
        let identity = identity.into();
        let source_identity = identity.clone();
        let path = Arc::new(path);
        Self {
            identity,
            culture,
            load: Arc::new(move || {
                let path = path.clone();
                let identity = source_identity.clone();
                let buffers = buffers.clone();
                effect_try_blocking! {
                    body: move |_| read_prepared_file(&path, &identity, maximum, &buffers),
                    on_error: |error| error,
                }
            }),
        }
    }
}

fn read_prepared_file(
    path: &std::path::Path,
    identity: &str,
    maximum: NonZeroUsize,
    buffers: &SlotPool<'static, Buffer<'static>>,
) -> Result<Bytes, LocalizationError> {
    use std::io::Read;
    let mut output = buffers.try_checkout().map_err(LocalizationError::Memory)?;
    let maximum = maximum.get();
    let ceiling = maximum
        .checked_add(1)
        .ok_or_else(|| LocalizationError::Limit("file read bytes".into()))?;
    let mut file =
        std::fs::File::open(path).map_err(|_| LocalizationError::Load(identity.into()))?;
    let mut scratch = [0u8; 4096];
    output.clear();
    loop {
        let available = ceiling - output.len();
        let count = available.min(scratch.len());
        let window = scratch
            .get_mut(..count)
            .ok_or_else(|| LocalizationError::Limit("file read bytes".into()))?;
        match file.read(window) {
            Ok(0) => return Ok(Bytes::from_owner(BufferView::new(output.share()))),
            Ok(read) => {
                let chunk = window
                    .get(..read)
                    .ok_or_else(|| LocalizationError::Load(identity.into()))?;
                if read > maximum - output.len() {
                    return Err(LocalizationError::Limit("resource bytes".into()));
                }
                output
                    .extend_from_slice(chunk)
                    .map_err(LocalizationError::Memory)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => (),
            Err(_) => return Err(LocalizationError::Load(identity.into())),
        }
    }
}
/// Static message metadata registered for resource validation.
#[derive(Clone)]
#[must_use]
pub struct Contract {
    /// Catalogue family identity supplied by MessageContract.
    pub catalogue: &'static str,
    /// Flattened message key.
    pub key: &'static str,
    /// Exact named field contract.
    pub fields: &'static [&'static str],
}
impl Contract {
    /// Extract compile-time catalogue/key/argument names without constructing payloads.
    pub const fn of<M: MessageContract>() -> Self {
        Self {
            catalogue: M::CATALOGUE,
            key: M::KEY,
            fields: M::FIELDS,
        }
    }
}
/// Typed reusable resource description and complete message contracts.
#[must_use]
pub struct CatalogDefinition<C: Send + Sync + 'static, R: Send + Sync + 'static> {
    /// Safe identity checked against each MessageContract family.
    pub(crate) identity: Arc<str>,
    /// Explicit parser/output resource limits.
    pub(crate) limits: CatalogLimits,
    /// Shared immutable source callbacks.
    resources: Arc<[CatalogResource<R>]>,
    /// Complete key-to-field contract map.
    contracts: Arc<BTreeMap<&'static str, &'static [&'static str]>>,
    /// Catalogue family relationship, without an instance/global lookup.
    marker: PhantomData<fn() -> C>,
}
impl<C: Send + Sync + 'static, R: Send + Sync + 'static> Clone for CatalogDefinition<C, R> {
    /// Share resource/contract metadata only, never loaded bytes or request state.
    fn clone(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            limits: self.limits,
            resources: self.resources.clone(),
            contracts: self.contracts.clone(),
            marker: PhantomData,
        }
    }
}
impl<C: Send + Sync + 'static, R: Send + Sync + 'static> CatalogDefinition<C, R> {
    /// Validate static identities and contracts before acquiring any source. Empty
    /// resources deliberately represent a valid empty replacement catalogue.
    pub fn new(
        identity: impl Into<Arc<str>>,
        limits: CatalogLimits,
        resources: Vec<CatalogResource<R>>,
        contracts: Vec<Contract>,
    ) -> Result<Self, LocalizationError> {
        let identity = identity.into();
        if identity.is_empty() {
            return Err(LocalizationError::Definition("catalogue identity".into()));
        }
        let mut entries = BTreeMap::new();
        for contract in contracts {
            if contract.key.len() > limits.key_bytes.get() {
                return Err(LocalizationError::Limit(
                    "catalogue contract key bytes".into(),
                ));
            }
            if contract.catalogue != identity.as_ref()
                || contract.key.is_empty()
                || contract.fields.iter().collect::<BTreeSet<_>>().len() != contract.fields.len()
                || entries.insert(contract.key, contract.fields).is_some()
            {
                return Err(LocalizationError::Definition("message contracts".into()));
            }
        }
        let mut sources = BTreeSet::new();
        for resource in &resources {
            if resource.identity.is_empty() || !sources.insert(resource.identity.clone()) {
                return Err(LocalizationError::Definition("resource identities".into()));
            }
        }
        Ok(Self {
            identity,
            limits,
            resources: resources.into(),
            contracts: Arc::new(entries),
            marker: PhantomData,
        })
    }
    /// Validate every source's UI language against the actual supported set before
    /// any loader runs. Unsupported resource languages cannot silently become dead entries.
    pub(crate) fn validate_cultures(&self, cultures: &CultureSet) -> Result<(), LocalizationError> {
        if self
            .resources
            .iter()
            .any(|resource| !cultures.contains(&resource.culture))
        {
            return Err(LocalizationError::Definition("resource culture".into()));
        }
        Ok(())
    }

    pub(super) fn resource_count(&self) -> usize {
        self.resources.len()
    }
    /// Read and validate every source before returning one replacement. No partial
    /// catalogue is published; ownership/cleanup failures remain Effect outcomes.
    pub fn load(&self) -> Effect<Catalog<C>, LocalizationError, R> {
        self.load_with(None)
    }

    pub(super) fn load_with(
        &self,
        prepared: Option<Arc<effect::memory::SlotPool<'static, PreparedCatalogData>>>,
    ) -> Effect<Catalog<C>, LocalizationError, R> {
        let definition = self.clone();
        effect_future! {
            exit_with_context; body: move |env: Arc<R>, context| {
            let definition = definition.clone();
            let prepared = prepared.clone();
            async move {
                let mut entries = match prepared {
                    Some(pool) => match pool.try_checkout() {
                        Ok(slot) => CatalogEntries::Prepared(PreparedCatalogLease(slot)),
                        Err(error) => return Exit::Fail(LocalizationError::Memory(error)),
                    },
                    None => CatalogEntries::Ordinary(BTreeMap::new()),
                };
                for resource in definition.resources.iter() {
                    let bytes = match effect::effect! {
                        run_in_context;
                        context: context,
                        program: (resource.load)(),
                        environment: env.clone(),
                    }.await {
                        Exit::Success(bytes) => bytes,
                        Exit::Fail(error) => return Exit::Fail(error),
                        Exit::Die(defect) => return Exit::Die(defect),
                        Exit::Interrupted => return Exit::Interrupted,
                        Exit::FinalizationFailed(failure) => {
                            return Exit::FinalizationFailed(failure);
                        }
                    };
                    if let Err(error) = entries.parse(
                        &resource.culture, &bytes, definition.limits, &definition.contracts,
                    ) {
                        return Exit::Fail(error);
                    }
                }
                Exit::Success(Catalog {
                    identity: definition.identity.clone(),
                    entries,
                    limits: definition.limits,
                    marker: PhantomData,
                })
            }
        }, }
    }
}
/// Complete immutable prepared catalogue generation. Equality compares content,
/// permitting unchanged reloads to retain their generation counter.
#[must_use]
pub struct Catalog<C: Send + Sync + 'static> {
    /// Prepared family identity, retained in each generation.
    pub(crate) identity: Arc<str>,
    /// Checked exact culture/key templates.
    entries: CatalogEntries,
    /// Explicit formatted-output byte limit.
    pub(crate) limits: CatalogLimits,
    /// Compile-time catalogue family marker.
    marker: PhantomData<fn() -> C>,
}

struct PreparedCatalogEntry {
    culture: CultureId,
    key: &'static str,
    fields: &'static [&'static str],
    source: Range<usize>,
}

pub(super) struct PreparedCatalogData {
    source: Buffer<'static>,
    entries: FixedVector<'static, PreparedCatalogEntry>,
}
impl PreparedCatalogData {
    pub(super) fn storage_bytes(
        entries: usize,
        bytes: MemoryBytes,
    ) -> Result<MemoryBytes, MemoryError> {
        let layout = std::alloc::Layout::array::<PreparedCatalogEntry>(entries)
            .map_err(|_| MemoryError::InvalidLayout)?;
        MemoryBytes::new(layout.size()).checked_add(bytes)
    }

    pub(super) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        entries: usize,
        bytes: MemoryBytes,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            source: memory.buffer(region, bytes)?,
            entries: FixedVector::prepare(memory.allocator(region)?, entries)?,
        })
    }

    #[allow(clippy::expect_used)] // Insertion stores only complete UTF-8 strings and their exact bounds.
    fn source(&self, entry: &PreparedCatalogEntry) -> &str {
        let source = self
            .source
            .as_slice()
            .get(entry.source.clone())
            .expect("prepared catalogue source range");
        std::str::from_utf8(source).expect("prepared catalogue source UTF-8")
    }

    fn find(&self, culture: &CultureId, key: &'static str) -> Option<&PreparedCatalogEntry> {
        let entries = self.entries.as_slice();
        let index = entries
            .binary_search_by(|entry| (&entry.culture, entry.key).cmp(&(culture, key)))
            .ok()?;
        entries.get(index)
    }
}

struct PreparedCatalogLease(SlotLease<'static, PreparedCatalogData>);
impl Drop for PreparedCatalogLease {
    fn drop(&mut self) {
        self.0.entries.clear();
        self.0.source.clear();
    }
}

enum CatalogEntries {
    Ordinary(BTreeMap<(CultureId, &'static str), Template>),
    Prepared(PreparedCatalogLease),
}
impl CatalogEntries {
    fn parse(
        &mut self,
        culture: &CultureId,
        bytes: &[u8],
        limits: CatalogLimits,
        contracts: &BTreeMap<&'static str, &'static [&'static str]>,
    ) -> Result<(), LocalizationError> {
        match self {
            Self::Ordinary(entries) => {
                let mut parsed = BTreeMap::new();
                parse_resource(bytes, limits, contracts, |key, fields, source| {
                    let template = Template::parse(source, fields)?;
                    if parsed.insert(key, template).is_some() {
                        return Err(LocalizationError::Key(key.into()));
                    }
                    Ok(())
                })?;
                for (key, template) in parsed {
                    if entries.insert((culture.clone(), key), template).is_some() {
                        return Err(LocalizationError::Key(key.into()));
                    }
                }
            }
            Self::Prepared(PreparedCatalogLease(data)) => {
                let first = data.entries.as_slice().len();
                parse_resource(bytes, limits, contracts, |key, fields, source| {
                    visit_template(source, fields, |_, _| Ok(()))?;
                    let current = data
                        .entries
                        .as_slice()
                        .get(first..)
                        .ok_or(LocalizationError::Admission)?;
                    if current.iter().any(|entry| entry.key == key) {
                        return Err(LocalizationError::Key(key.into()));
                    }
                    let start = data.source.len();
                    data.source
                        .extend_from_slice(source.as_bytes())
                        .map_err(LocalizationError::Memory)?;
                    let end = data.source.len();
                    data.entries
                        .push(PreparedCatalogEntry {
                            culture: culture.clone(),
                            key,
                            fields,
                            source: start..end,
                        })
                        .map_err(|_| LocalizationError::Memory(MemoryError::NoSlot))?;
                    Ok(())
                })?;
                let entries = data.entries.as_mut_slice();
                entries.sort_unstable_by(|left, right| {
                    (&left.culture, left.key).cmp(&(&right.culture, right.key))
                });
                if let Some(duplicate) = entries.windows(2).find_map(|pair| match pair {
                    [left, right] if left.culture == right.culture && left.key == right.key => {
                        Some(left.key)
                    }
                    [_left, _right] => None,
                    _ => None,
                }) {
                    return Err(LocalizationError::Key(duplicate.into()));
                }
            }
        }
        Ok(())
    }
}

impl<C: Send + Sync + 'static> Catalog<C> {
    /// Compare prepared content without cloning templates or argument payloads.
    pub(crate) fn same(&self, other: &Self) -> bool {
        self.identity == other.identity
            && match (&self.entries, &other.entries) {
                (CatalogEntries::Ordinary(left), CatalogEntries::Ordinary(right)) => left == right,
                (
                    CatalogEntries::Prepared(PreparedCatalogLease(left)),
                    CatalogEntries::Prepared(PreparedCatalogLease(right)),
                ) => {
                    let left_entries = left.entries.as_slice();
                    let right_entries = right.entries.as_slice();
                    left_entries.len() == right_entries.len()
                        && left_entries
                            .iter()
                            .zip(right_entries)
                            .all(|(entry, candidate)| {
                                entry.culture == candidate.culture
                                    && entry.key == candidate.key
                                    && left.source(entry) == right.source(candidate)
                            })
                }
                (CatalogEntries::Ordinary(_), CatalogEntries::Prepared(_))
                | (CatalogEntries::Prepared(_), CatalogEntries::Ordinary(_)) => false,
            }
    }

    pub(super) fn write(
        &self,
        culture: &CultureId,
        key: &'static str,
        arguments: &impl ArgumentList,
        formatting: &CultureId,
        emit: impl FnMut(&str) -> Result<(), LocalizationError>,
    ) -> Result<bool, LocalizationError> {
        let maximum = self.limits.message_bytes.get();
        match &self.entries {
            CatalogEntries::Ordinary(entries) => {
                let Some(template) = entries.get(&(culture.clone(), key)) else {
                    return Ok(false);
                };
                template.write(arguments, formatting, maximum, emit)?;
            }
            CatalogEntries::Prepared(PreparedCatalogLease(data)) => {
                let Some(entry) = data.find(culture, key) else {
                    return Ok(false);
                };
                write_bounded(maximum, emit, |append| {
                    visit_template(data.source(entry), entry.fields, |field, value| {
                        if field {
                            arguments.write_value(value, formatting, append)
                        } else {
                            append(value)
                        }
                    })
                })?;
            }
        }
        Ok(true)
    }

    #[allow(clippy::expect_used)] // Prepared source entered the generation only after contract validation.
    pub(super) fn enumerate(&self, culture: &CultureId, output: &mut BTreeMap<String, Template>) {
        match &self.entries {
            CatalogEntries::Ordinary(entries) => {
                for ((entry_culture, key), template) in entries {
                    if entry_culture == culture {
                        output.insert((*key).into(), template.clone());
                    }
                }
            }
            CatalogEntries::Prepared(PreparedCatalogLease(data)) => {
                for entry in data
                    .entries
                    .as_slice()
                    .iter()
                    .filter(|entry| entry.culture == *culture)
                {
                    let template = Template::parse(data.source(entry), entry.fields)
                        .expect("validated template");
                    output.insert(entry.key.into(), template);
                }
            }
        }
    }
}

type StoreTemplate<'a> =
    dyn FnMut(&'static str, &'static [&'static str], &str) -> Result<(), LocalizationError> + 'a;
/// One bounded parse budget shared by recursive serde seeds.
struct Budget<'a> {
    /// Explicit caller parser limits.
    limits: CatalogLimits,
    /// Number of key/array positions consumed.
    keys: usize,
    /// Complete contract map; unknown resource messages fail validation.
    contracts: &'a BTreeMap<&'static str, &'static [&'static str]>,
    /// Checked flattened output; all leaves must be string templates.
    store: &'a mut StoreTemplate<'a>,
    /// Original safe domain category, retained instead of serde's parser text.
    failure: Option<LocalizationError>,
}
/// Recursive deserialization seed validates depth before visiting descendants.
struct Node<'a, 'b> {
    /// Shared budget/output borrowed during this parse only.
    budget: &'a mut Budget<'b>,
    /// Current flattened path, rejecting ambiguous source keys.
    path: String,
    /// Explicit object/array nesting depth.
    depth: usize,
    /// A real array member consumes its budget before parsing descendants.
    array_member: bool,
}
impl<'de> DeserializeSeed<'de> for Node<'_, '_> {
    /// Parsing contributes only checked entries to the borrowed budget.
    type Value = ();
    /// Dispatch checked string/object/array nodes only; no intermediate JSON DOM.
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        if self.array_member {
            let next = self.budget.keys.checked_add(1);
            if next.is_none_or(|next| next > self.budget.limits.keys.get()) {
                self.budget.failure = Some(LocalizationError::Limit("catalogue key count".into()));
                return Err(serde::de::Error::custom("catalogue key count"));
            }
            if let Some(next) = next {
                self.budget.keys = next;
            }
        }
        if self.depth > self.budget.limits.depth.get() {
            self.budget.failure = Some(LocalizationError::Limit("catalogue depth".into()));
            return Err(serde::de::Error::custom("catalogue depth"));
        }
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Node<'_, '_> {
    /// Parsing contributes only checked entries to the borrowed budget.
    type Value = ();
    /// Describe the source contract to serde without reproducing resource data.
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded catalogue string/object/array")
    }
    /// Validate a finite string leaf against its declared named message contract.
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<(), E> {
        if value.len() > self.budget.limits.value_bytes.get()
            || self.path.len() > self.budget.limits.key_bytes.get()
        {
            self.budget.failure =
                Some(LocalizationError::Limit("catalogue key/value bytes".into()));
            return Err(E::custom("catalogue key/value bytes"));
        }
        let Some((&key, fields)) = self.budget.contracts.get_key_value(self.path.as_str()) else {
            self.budget.failure = Some(LocalizationError::Template(self.path.clone()));
            return Err(E::custom("unknown message contract"));
        };
        if let Err(error) = (self.budget.store)(key, fields, value) {
            self.budget.failure = Some(error);
            return Err(E::custom("message template"));
        }
        Ok(())
    }
    /// Owned strings use exactly the same contract as borrowed serde strings.
    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<(), E> {
        self.visit_str(&value)
    }
    /// Reject duplicate/ambiguous object keys before child processing.
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        let mut seen = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if key.is_empty() || key.contains(['.', '[', ']', '\\']) || !seen.insert(key.clone()) {
                self.budget.failure = Some(LocalizationError::Key(self.path.clone()));
                return Err(serde::de::Error::custom(
                    "ambiguous/duplicate catalogue key",
                ));
            }
            self.budget.keys = self
                .budget
                .keys
                .checked_add(1)
                .ok_or_else(|| serde::de::Error::custom("catalogue key count"))?;
            let length = self
                .path
                .len()
                .checked_add(key.len())
                .and_then(|length| length.checked_add(usize::from(!self.path.is_empty())));
            if self.budget.keys > self.budget.limits.keys.get()
                || length.is_none_or(|length| length > self.budget.limits.key_bytes.get())
            {
                self.budget.failure = Some(LocalizationError::Limit("catalogue key bound".into()));
                return Err(serde::de::Error::custom("catalogue key bound"));
            }
            let path = if self.path.is_empty() {
                key
            } else {
                format!("{}.{}", self.path, key)
            };
            map.next_value_seed(Node {
                budget: self.budget,
                path,
                depth: self.depth + 1,
                array_member: false,
            })?;
        }
        Ok(())
    }
    /// Arrays flatten as `[index]` and consume the same explicit node/key budget.
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        let mut index = 0usize;
        loop {
            let path = format!("{}[{index}]", self.path);
            if path.len() > self.budget.limits.key_bytes.get() {
                self.budget.failure = Some(LocalizationError::Limit("catalogue key bytes".into()));
                return Err(serde::de::Error::custom("catalogue key bytes"));
            }
            if seq
                .next_element_seed(Node {
                    budget: self.budget,
                    path,
                    depth: self.depth + 1,
                    array_member: true,
                })?
                .is_none()
            {
                break;
            }
            index = index
                .checked_add(1)
                .ok_or_else(|| serde::de::Error::custom("catalogue array index"))?;
        }
        Ok(())
    }
}
/// Parse one bounded finite source directly, rejecting duplicate JSON keys and
/// non-string leaves rather than converting an already-lossy serde_json Value.
fn parse_resource(
    bytes: &[u8],
    limits: CatalogLimits,
    contracts: &BTreeMap<&'static str, &'static [&'static str]>,
    mut store: impl FnMut(&'static str, &'static [&'static str], &str) -> Result<(), LocalizationError>,
) -> Result<(), LocalizationError> {
    if bytes.len() > limits.bytes.get() {
        return Err(LocalizationError::Limit("resource bytes".into()));
    }
    let mut budget = Budget {
        limits,
        keys: 0,
        contracts,
        store: &mut store,
        failure: None,
    };
    let mut parser = serde_json::Deserializer::from_slice(bytes);
    parser.disable_recursion_limit();
    if (Node {
        budget: &mut budget,
        path: String::new(),
        depth: 0,
        array_member: false,
    })
    .deserialize(&mut parser)
    .is_err()
    {
        return Err(budget
            .failure
            .unwrap_or_else(|| LocalizationError::Resource("JSON syntax/leaf type".into())));
    }
    parser
        .end()
        .map_err(|_| LocalizationError::Resource("trailing JSON".into()))
}
