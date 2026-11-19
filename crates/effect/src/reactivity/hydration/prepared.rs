//! Bounded wire and snapshot ownership for the existing hydration protocol.

use super::{
    AtomId, AtomRef, AtomRefError, AtomRegistry, HydrationConflictPolicy, HydrationError,
    HydrationImportOutcome, wire,
};
use crate::internal::runtime::{self, PreparedNestedReservation};
use crate::memory::{
    BufferView, Bytes, FixedVector, FramePool, MemoryError, PreparedMemory, RegionId,
};
use crate::persistence::RecordVersion;
use crate::schema::internal::parser::{PreparedJsonParseReservation, strict_wire_policy};
use crate::stream::{PreparedChunkPool, PreparedChunkValues};
use crate::{
    Effect, Exit, JsonWriter, JsonWriterPolicy, ParseLimits, PreparedJsonKind, PreparedJsonOutput,
    PreparedJsonValue, PreparedSchemaMemory, SchemaIssue, SchemaIssueKind,
};
use crate::{
    effect, effect_future, effect_map_error, effect_resource, effect_sync, effect_try_sync,
};
use std::alloc::Layout;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Mutex, PoisonError};

/// Typed codec at the prepared writer and borrowed JSON value boundary.
pub trait PreparedHydrationCodec<A>: Send + Sync {
    /// Write exactly one value through the caller-admitted writer.
    fn write(&self, value: &A, writer: &mut JsonWriter) -> Result<(), SchemaIssue>;

    /// Decode an admitted, borrowed value without a native JSON DOM.
    fn read(&self, value: PreparedJsonValue<'_>) -> Result<A, SchemaIssue>;
}

/// An error from prepared hydration or the existing atom lifecycle.
#[derive(Debug)]
pub enum PreparedHydrationError {
    /// The selected physical workspace could not admit the operation.
    Memory(MemoryError),
    /// The wire or atom transition failed its existing checks.
    Hydration(HydrationError),
}

impl From<HydrationError> for PreparedHydrationError {
    fn from(error: HydrationError) -> Self {
        Self::Hydration(error)
    }
}

impl From<MemoryError> for PreparedHydrationError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

fn schema_error(error: SchemaIssue) -> PreparedHydrationError {
    if let SchemaIssueKind::Memory(memory) = error.kind() {
        PreparedHydrationError::Memory(*memory)
    } else {
        PreparedHydrationError::Hydration(HydrationError::Schema(error))
    }
}

/// A complete record retaining its actual input or writer byte backing.
pub enum PreparedHydrationBytes {
    /// A received record in a counted input buffer.
    Buffer(BufferView<'static>),
    /// A completed prepared JSON writer output.
    Json(PreparedJsonOutput),
}

impl AsRef<[u8]> for PreparedHydrationBytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Buffer(value) => value.as_ref(),
            Self::Json(value) => value.as_bytes(),
        }
    }
}

impl From<BufferView<'static>> for PreparedHydrationBytes {
    fn from(value: BufferView<'static>) -> Self {
        Self::Buffer(value)
    }
}

impl From<PreparedJsonOutput> for PreparedHydrationBytes {
    fn from(value: PreparedJsonOutput) -> Self {
        Self::Json(value)
    }
}

/// Versioned prepared hydration schema for a selected source.
pub struct PreparedHydrationSchema<A> {
    atom: AtomId,
    version: RecordVersion,
    limits: ParseLimits,
    codec: Arc<dyn PreparedHydrationCodec<A>>,
}

impl<A> Clone for PreparedHydrationSchema<A> {
    fn clone(&self) -> Self {
        Self {
            atom: self.atom.clone(),
            version: self.version.clone(),
            limits: self.limits,
            codec: Arc::clone(&self.codec),
        }
    }
}

struct PreparedHydrationEncode {
    writer: JsonWriter,
    validate: PreparedJsonParseReservation,
}

impl<A> PreparedHydrationSchema<A> {
    /// Bind a codec and format limits to one atom and compatibility version.
    pub fn new(
        atom: AtomId,
        version: RecordVersion,
        limits: ParseLimits,
        codec: impl PreparedHydrationCodec<A> + 'static,
    ) -> Self {
        Self {
            atom,
            version,
            limits,
            codec: Arc::new(codec),
        }
    }

    fn reserve_encode(
        &self,
        memory: &PreparedSchemaMemory,
    ) -> Result<PreparedHydrationEncode, MemoryError> {
        let writer = memory.writers().admit_serde_buffered(JsonWriterPolicy {
            bytes: self.limits.bytes,
            depth: self.limits.depth,
        })?;
        let validate = memory
            .parser()
            .reserve_parse(strict_wire_policy(self.limits))?;
        Ok(PreparedHydrationEncode { writer, validate })
    }

    /// Encode the declared wire record into an independently retained writer slot.
    pub fn encode(
        &self,
        memory: &PreparedSchemaMemory,
        value: &A,
        revision: NonZeroU64,
    ) -> Result<PreparedJsonOutput, PreparedHydrationError> {
        let PreparedHydrationEncode {
            mut writer,
            validate,
        } = self.reserve_encode(memory)?;
        self.write_record(value, revision, &mut writer)
            .map_err(schema_error)?;
        let output = writer.finish_prepared().map_err(schema_error)?;
        drop(validate.parse(output.as_bytes()).map_err(schema_error)?);
        Ok(output)
    }

    fn write_record(
        &self,
        value: &A,
        revision: NonZeroU64,
        writer: &mut JsonWriter,
    ) -> Result<(), SchemaIssue> {
        writer.begin_object()?;
        writer.property(wire::ATOM)?;
        writer.begin_array()?;
        for byte in self.atom.as_bytes() {
            writer.u64(u64::from(*byte))?;
        }
        writer.end_array()?;
        writer.property(wire::REVISION)?;
        writer.u64(revision.get())?;
        writer.property(wire::VALUE)?;
        self.codec.write(value, writer)?;
        writer.property(wire::VERSION)?;
        writer.string(self.version.as_str())?;
        writer.end_object()
    }

    /// Decode the declared record through a physical parser workspace.
    pub fn decode(
        &self,
        memory: &PreparedSchemaMemory,
        bytes: &[u8],
    ) -> Result<(A, NonZeroU64), PreparedHydrationError> {
        let reservation = memory
            .parser()
            .reserve_parse(strict_wire_policy(self.limits))?;
        let document = reservation.parse(bytes).map_err(schema_error)?;
        let root = document.root();
        if root.kind() != Some(PreparedJsonKind::Object)
            || root.len() != wire::FIELDS.len()
            || wire::FIELDS.iter().any(|field| root.get(field).is_none())
        {
            return Err(HydrationError::InvalidRecord.into());
        }
        let atom = record_atom(root)?;
        if !matches_atom(atom, self.atom.as_bytes()) {
            return Err(HydrationError::WrongAtom.into());
        }
        let version = root
            .get(wire::VERSION)
            .and_then(PreparedJsonValue::as_str)
            .ok_or(HydrationError::InvalidRecord)?;
        if version != self.version.as_str() {
            return Err(HydrationError::VersionMismatch.into());
        }
        let revision = root
            .get(wire::REVISION)
            .and_then(PreparedJsonValue::number_token)
            .and_then(|token| token.parse().ok())
            .and_then(NonZeroU64::new)
            .ok_or(HydrationError::InvalidRecord)?;
        let encoded = root.get(wire::VALUE).ok_or(HydrationError::InvalidRecord)?;
        let value = self.codec.read(encoded).map_err(schema_error)?;
        Ok((value, revision))
    }
}

fn record_atom(root: PreparedJsonValue<'_>) -> Result<PreparedJsonValue<'_>, HydrationError> {
    root.get(wire::ATOM)
        .filter(|value| value.kind() == Some(PreparedJsonKind::Array))
        .ok_or(HydrationError::InvalidRecord)
}

fn atom_bytes(value: PreparedJsonValue<'_>) -> impl Iterator<Item = Option<u8>> + '_ {
    let mut next = value.first_child();
    std::iter::from_fn(move || {
        let byte = next?;
        next = byte.next_sibling();
        Some(
            byte.number_token()
                .and_then(|token| token.parse::<u8>().ok()),
        )
    })
}

fn matches_atom(value: PreparedJsonValue<'_>, expected: &[u8]) -> bool {
    value.len() == expected.len() && atom_bytes(value).eq(expected.iter().copied().map(Some))
}

impl<A: Send + Sync + 'static> AtomRef<A> {
    /// Export one consistent value/revision using prepared wire storage.
    pub fn export_prepared<R: Send + Sync + 'static>(
        &self,
        schema: PreparedHydrationSchema<A>,
        memory: Arc<PreparedSchemaMemory>,
    ) -> Effect<PreparedJsonOutput, PreparedHydrationError, R> {
        if schema.atom != self.id {
            return effect_try_sync! { body: || Err(HydrationError::WrongAtom.into()), on_error: std::convert::identity, };
        }
        let source = self.clone();
        let acquire = effect_map_error! {
            program: self.hydrate_permit(),
            map: |error| PreparedHydrationError::from(HydrationError::Source(AtomRefError::Admission(error))),
        };
        effect! {
            let source = source.clone();
            let schema = schema.clone();
            let memory = Arc::clone(&memory);
            let permit = acquire.clone()?;
            {
                let read = effect_map_error! {
                    program: source.value.read(),
                    map: |error| PreparedHydrationError::from(HydrationError::Source(AtomRefError::Publication(error))),
                };
                let source_for_snapshot = source.clone();
                let schema = schema.clone();
                let memory = Arc::clone(&memory);
                let snapshot = effect! {
                    let source_for_snapshot = source_for_snapshot.clone();
                    let schema = schema.clone();
                    let memory = Arc::clone(&memory);
                    let value = read.clone()?;
                    {
                        let revision = *source_for_snapshot.revision.lock().unwrap_or_else(PoisonError::into_inner);
                        effect_try_sync! {
                            once; body: {
                                let result = schema.encode(&memory, &value, revision);
                                move || result
                            },
                            on_error: std::convert::identity,
                        }
                    }?
                };
                let release = effect_sync! { once; body: move || drop(permit), };
                effect_resource! {
                    acquire: effect_sync! { body: || (), },
                    release: move |(), _| release.clone(),
                    body: move |_| snapshot.clone(),
                }
            }?
        }
    }

    /// Decode retained bytes before applying the existing atom revision fence.
    pub fn import_prepared<R: Send + Sync + 'static>(
        &self,
        schema: PreparedHydrationSchema<A>,
        memory: Arc<PreparedSchemaMemory>,
        bytes: PreparedHydrationBytes,
        policy: HydrationConflictPolicy,
    ) -> Effect<HydrationImportOutcome, PreparedHydrationError, R> {
        if schema.atom != self.id {
            return effect_try_sync! { body: || Err(HydrationError::WrongAtom.into()), on_error: std::convert::identity, };
        }
        let decoded = effect_try_sync! {
            once; body: move || schema.decode(&memory, bytes.as_ref()),
            on_error: std::convert::identity,
        };
        self.apply_decoded_hydration(decoded, policy)
    }
}

/// Startup bounds for selected hydration bindings and independently retained pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedHydrationSetCapacity {
    /// Registered atom schemas.
    pub bindings: NonZeroUsize,
    /// Page slots, including pages retained by downstream readers.
    pub pages: NonZeroUsize,
    /// Maximum records in one page.
    pub records_per_page: NonZeroUsize,
    /// Logical bytes in each retained caller-owned atom identity.
    pub bytes_per_atom: Bytes,
}

type ImportFactory<R> = dyn Fn(
        PreparedHydrationBytes,
        HydrationConflictPolicy,
    ) -> Effect<HydrationImportOutcome, PreparedHydrationError, R>
    + Send
    + Sync;

struct PreparedBinding<R: Send + Sync + 'static> {
    atom: AtomId,
    export: Effect<PreparedJsonOutput, PreparedHydrationError, R>,
    import: Arc<ImportFactory<R>>,
}

/// A complete snapshot of one atom in a leased hydration page.
pub struct PreparedHydrationRecord {
    /// Stable registered identity.
    pub atom: AtomId,
    /// Physical JSON bytes for that atom's value and revision.
    pub bytes: PreparedJsonOutput,
}

/// One page of independent atom snapshots in stable identity order.
pub struct PreparedHydrationPage {
    records: PreparedChunkValues<PreparedHydrationRecord>,
    next: Option<AtomId>,
}

impl PreparedHydrationPage {
    /// Borrow the records without copying their byte backing.
    #[must_use]
    pub fn records(&self) -> &[PreparedHydrationRecord] {
        self.records.as_slice()
    }

    /// Borrow the exclusive lower bound for the next page, if present.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&AtomId> {
        self.next.as_ref()
    }

    /// Transfer record owners into a transport or another prepared input page.
    pub fn take_records(&mut self) -> impl Iterator<Item = PreparedHydrationRecord> + '_ {
        self.records.take_items()
    }

    /// Return this page's slots and retain its continuation identity.
    #[must_use]
    pub fn into_next_cursor(self) -> Option<AtomId> {
        self.next
    }
}

struct PreparedInput {
    bytes: PreparedHydrationBytes,
    binding: Option<usize>,
}

/// An affine input page holding only caller-admitted byte owners.
pub struct PreparedHydrationInputPage(PreparedChunkValues<PreparedInput>);

impl PreparedHydrationInputPage {
    /// Add another retained record, returning it to the caller on saturation.
    pub fn try_push(
        &mut self,
        bytes: PreparedHydrationBytes,
    ) -> Result<(), PreparedHydrationBytes> {
        self.0
            .try_push(PreparedInput {
                bytes,
                binding: None,
            })
            .map_err(|input| input.bytes)
    }
}

/// A selected import's outcome for one resolved atom identity.
pub struct PreparedHydrationRecordResult {
    /// Stable identity that received the record.
    pub atom: AtomId,
    /// Individual revision and publication result.
    pub outcome: Result<HydrationImportOutcome, PreparedHydrationError>,
}

/// Independently retained outcomes in the input order.
pub struct PreparedHydrationImportPage(PreparedChunkValues<PreparedHydrationRecordResult>);

impl PreparedHydrationImportPage {
    /// Borrow recorded per-atom outcomes.
    #[must_use]
    pub fn results(&self) -> &[PreparedHydrationRecordResult] {
        self.0.as_slice()
    }
}

/// Fixed startup bindings with a leased selected snapshot path. Each record
/// uses its source's value/revision gate and owns its encoded bytes.
pub struct PreparedHydrationSet<R: Send + Sync + 'static> {
    registry: AtomRegistry,
    schemas: Arc<PreparedSchemaMemory>,
    bindings: FixedVector<'static, PreparedBinding<R>>,
    pages: PreparedChunkPool<PreparedHydrationRecord>,
    inputs: PreparedChunkPool<PreparedInput>,
    outcomes: PreparedChunkPool<PreparedHydrationRecordResult>,
    routing_limits: ParseLimits,
    bridge: FramePool<'static>,
    capacity: PreparedHydrationSetCapacity,
}

impl<R: Send + Sync + 'static> PreparedHydrationSet<R> {
    fn bridge_demand(pages: NonZeroUsize) -> Result<(usize, Layout), MemoryError> {
        let (export, export_finish) =
            PreparedNestedReservation::<PreparedJsonOutput, PreparedHydrationError>::layouts();
        let (import, import_finish) =
            PreparedNestedReservation::<HydrationImportOutcome, PreparedHydrationError>::layouts();
        let layouts = [export, export_finish, import, import_finish];
        let size = layouts.iter().map(Layout::size).max().unwrap_or_default();
        let align = layouts.iter().map(Layout::align).max().unwrap_or(1);
        let layout =
            Layout::from_size_align(size, align).map_err(|_| MemoryError::InvalidLayout)?;
        let count = pages
            .get()
            .checked_mul(4)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok((count, layout))
    }

    /// Region demand for bindings, returned pages and nested effect bridges.
    /// Supply schema parser/writer backing and region overhead separately.
    pub fn storage_bytes(capacity: PreparedHydrationSetCapacity) -> Result<Bytes, MemoryError> {
        let bindings = Layout::array::<PreparedBinding<R>>(capacity.bindings.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let (bridge_count, bridge_layout) = Self::bridge_demand(capacity.pages)?;
        Bytes::new(bindings.size())
            .checked_add(PreparedChunkPool::<PreparedHydrationRecord>::storage_bytes(
                capacity.pages,
                capacity.records_per_page,
            )?)?
            .checked_add(PreparedChunkPool::<PreparedInput>::storage_bytes(
                capacity.pages,
                capacity.records_per_page,
            )?)?
            .checked_add(
                PreparedChunkPool::<PreparedHydrationRecordResult>::storage_bytes(
                    capacity.pages,
                    capacity.records_per_page,
                )?,
            )?
            .checked_add(FramePool::storage_bytes(bridge_count, bridge_layout)?)
    }

    /// Provision fixed binding and page storage before the supplied region seals.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        registry: AtomRegistry,
        schemas: Arc<PreparedSchemaMemory>,
        routing_limits: ParseLimits,
        capacity: PreparedHydrationSetCapacity,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(capacity)?;
        let (bridge_count, bridge_layout) = Self::bridge_demand(capacity.pages)?;
        Ok(Self {
            registry,
            schemas,
            bindings: FixedVector::prepare(memory.allocator(region)?, capacity.bindings.get())?,
            pages: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.pages,
                capacity.records_per_page,
            )?,
            inputs: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.pages,
                capacity.records_per_page,
            )?,
            outcomes: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.pages,
                capacity.records_per_page,
            )?,
            routing_limits,
            bridge: memory.frames(region, bridge_count, bridge_layout)?,
            capacity,
        })
    }

    /// Register the existing source and its selected wire codec at startup.
    pub fn register<A: Send + Sync + 'static>(
        &mut self,
        source: AtomRef<A>,
        schema: PreparedHydrationSchema<A>,
    ) -> Result<(), PreparedHydrationError> {
        if !self.registry.same_instance(&source.registry) {
            return Err(HydrationError::ForeignRegistry.into());
        }
        if source.id != schema.atom {
            return Err(HydrationError::WrongAtom.into());
        }
        let position = match self
            .bindings
            .as_slice()
            .binary_search_by(|binding| binding.atom.cmp(&source.id))
        {
            Ok(_) => return Err(HydrationError::DuplicateAtom.into()),
            Err(position) => position,
        };
        let requested = Bytes::new(source.id.as_bytes().len());
        if requested > self.capacity.bytes_per_atom {
            return Err(MemoryError::ValueTooLarge {
                requested,
                capacity: self.capacity.bytes_per_atom,
            }
            .into());
        }
        if self.bindings.len() == self.bindings.capacity() {
            return Err(MemoryError::NoSlot.into());
        }
        let export = source.export_prepared(schema.clone(), Arc::clone(&self.schemas));
        let atom = source.id.clone();
        let memory = Arc::clone(&self.schemas);
        let import = Arc::new(move |record, policy| {
            source.import_prepared(schema.clone(), Arc::clone(&memory), record, policy)
        });
        self.bindings.push_reserved(PreparedBinding {
            atom,
            export,
            import,
        });
        self.bindings
            .as_mut_slice()
            .split_at_mut(position)
            .1
            .rotate_right(1);
        Ok(())
    }

    /// Export a bounded page without a detached output vector. A failed or
    /// cancelled invocation returns its incomplete page and all its byte slots.
    /// The source gate is per atom, so the page is a set of independent snapshots.
    pub fn export_page(
        self: &Arc<Self>,
        after: Option<AtomId>,
        limit: NonZeroUsize,
    ) -> Effect<PreparedHydrationPage, PreparedHydrationError, R> {
        let set = Arc::clone(self);
        effect_future! {
            exit_with_context; body: move |env: Arc<R>, context| {
                let set = Arc::clone(&set);
                let after = after.clone();
                async move { set.run_page(after, limit, env, &context.context).await }
            },
        }
    }

    async fn run_page(
        &self,
        after: Option<AtomId>,
        limit: NonZeroUsize,
        env: Arc<R>,
        context: &runtime::Context,
    ) -> Exit<PreparedHydrationPage, PreparedHydrationError> {
        if limit > self.capacity.records_per_page {
            return Exit::Fail(MemoryError::CollectionReservationFailed.into());
        }
        let mut records = match self.pages.try_checkout() {
            Ok(records) => records,
            Err(error) => return Exit::Fail(error.into()),
        };
        let all = self.bindings.as_slice();
        let first = after.map_or(0, |after| {
            all.partition_point(|binding| binding.atom <= after)
        });
        let remaining = all.split_at(first).1;
        for binding in remaining.iter().take(limit.get()) {
            if context.cancelled() {
                return Exit::Interrupted;
            }
            let reservation = match PreparedNestedReservation::reserve(&self.bridge) {
                Ok(reservation) => reservation,
                Err(error) => return Exit::Fail(error.into()),
            };
            let outcome = runtime::evaluate_context_reserved(context, reservation, |finish| {
                binding
                    .export
                    .clone()
                    .execute(Arc::clone(&env), context.clone(), finish)
            })
            .await;
            let bytes = match outcome {
                Exit::Success(bytes) => bytes,
                Exit::Fail(error) => return Exit::Fail(error),
                Exit::Die(defect) => return Exit::Die(defect),
                Exit::Interrupted => return Exit::Interrupted,
                Exit::FinalizationFailed(failure) => return Exit::FinalizationFailed(failure),
            };
            records.push_reserved(PreparedHydrationRecord {
                atom: binding.atom.clone(),
                bytes,
            });
        }
        let next = if remaining.len() > limit.get() {
            records.as_slice().last().map(|record| record.atom.clone())
        } else {
            None
        };
        Exit::Success(PreparedHydrationPage { records, next })
    }
}

impl<R: Send + Sync + 'static> PreparedHydrationSet<R> {
    /// Reserve the actual table for a finite batch before receiving its records.
    pub fn reserve_input_page(&self) -> Result<PreparedHydrationInputPage, MemoryError> {
        self.inputs.try_checkout().map(PreparedHydrationInputPage)
    }

    /// Route every atom and reject duplicates before applying any record. Then
    /// apply records independently with their existing cancellation and commit fence.
    pub fn import_page(
        self: &Arc<Self>,
        input: PreparedHydrationInputPage,
        policy: HydrationConflictPolicy,
    ) -> Effect<PreparedHydrationImportPage, PreparedHydrationError, R> {
        let set = Arc::clone(self);
        let input = Mutex::new(Some(input));
        effect_future! {
            exit_with_context; body: move |env: Arc<R>, context| {
                let set = Arc::clone(&set);
                let input = input.lock().unwrap_or_else(PoisonError::into_inner).take();
                async move {
                    let Some(input) = input else {
                        return Exit::Fail(HydrationError::InputConsumed.into());
                    };
                    set.run_import(input, policy, env, &context.context).await
                }
            },
        }
    }

    fn route(&self, records: &mut [PreparedInput]) -> Result<(), PreparedHydrationError> {
        for index in 0..records.len() {
            let (previous, remaining) = records.split_at_mut(index);
            let record = remaining.first_mut().ok_or(HydrationError::InvalidRecord)?;
            let parser = self
                .schemas
                .parser()
                .reserve_parse(strict_wire_policy(self.routing_limits))?;
            let document = parser.parse(record.bytes.as_ref()).map_err(schema_error)?;
            let atom = record_atom(document.root())?;
            if atom_bytes(atom).any(|byte| byte.is_none()) {
                return Err(HydrationError::InvalidRecord.into());
            }
            let binding = self
                .bindings
                .as_slice()
                .iter()
                .position(|binding| matches_atom(atom, binding.atom.as_bytes()))
                .ok_or(HydrationError::UnknownAtom)?;
            if previous.iter().any(|input| input.binding == Some(binding)) {
                return Err(HydrationError::DuplicateRecord.into());
            }
            record.binding = Some(binding);
        }
        Ok(())
    }

    async fn run_import(
        &self,
        mut input: PreparedHydrationInputPage,
        policy: HydrationConflictPolicy,
        env: Arc<R>,
        context: &runtime::Context,
    ) -> Exit<PreparedHydrationImportPage, PreparedHydrationError> {
        if input.0.len() > self.capacity.records_per_page.get() {
            return Exit::Fail(MemoryError::CollectionReservationFailed.into());
        }
        let mut results = match self.outcomes.try_checkout() {
            Ok(results) => results,
            Err(error) => return Exit::Fail(error.into()),
        };
        if let Err(error) = self.route(input.0.as_mut_slice()) {
            return Exit::Fail(error);
        }
        for record in input.0.take_items() {
            if context.cancelled() {
                return Exit::Interrupted;
            }
            let Some(binding) = record
                .binding
                .and_then(|index| self.bindings.as_slice().get(index))
            else {
                return Exit::Fail(HydrationError::UnknownAtom.into());
            };
            let reservation = match PreparedNestedReservation::reserve(&self.bridge) {
                Ok(reservation) => reservation,
                Err(error) => return Exit::Fail(error.into()),
            };
            let program = (binding.import)(record.bytes, policy);
            let outcome = runtime::evaluate_context_reserved(context, reservation, |finish| {
                program.execute(Arc::clone(&env), context.clone(), finish)
            })
            .await;
            let outcome = match outcome {
                Exit::Success(value) => Ok(value),
                Exit::Fail(error) => Err(error),
                Exit::Die(defect) => return Exit::Die(defect),
                Exit::Interrupted => return Exit::Interrupted,
                Exit::FinalizationFailed(failure) => return Exit::FinalizationFailed(failure),
            };
            results.push_reserved(PreparedHydrationRecordResult {
                atom: binding.atom.clone(),
                outcome,
            });
        }
        Exit::Success(PreparedHydrationImportPage(results))
    }
}
