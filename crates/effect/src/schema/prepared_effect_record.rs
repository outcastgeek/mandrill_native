//! Required-field owned record descriptions on the existing prepared graph.
use super::internal::parser::PreparedObjectAssembly;
use super::prepared_alternatives::Budget;
use super::{
    JsonWriter, ParseContext, ParsePolicy, PreparedIssueMemory, PreparedJsonArrayBuilder,
    PreparedJsonValidationPool, SchemaIssue, SharedPreparedJsonValue,
};
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool,
};
use std::sync::{Arc, Mutex};

/// Actual input/output owners selected for a prepared record representation.
#[doc(hidden)]
pub trait RecordBacking: Send + Sync + 'static {
    /// Encoded child representation.
    type Encoded: Send + 'static;
    /// Admitted and validated input owner.
    type Input: Send + 'static;
    /// Admitted output owner.
    type Output: Send + 'static;
    /// Admit and check input before field callbacks.
    fn read(
        &self,
        input: Self::Encoded,
        policy: ParsePolicy,
        names: &[Arc<str>],
        context: Option<&ParseContext>,
        issues: &PreparedIssueMemory,
        validation: &PreparedJsonValidationPool,
    ) -> Result<Self::Input, SchemaIssue>;
    /// Take one named value from the actual input owner.
    fn take(
        &self,
        input: &mut Self::Input,
        name: &str,
    ) -> Result<Option<Self::Encoded>, SchemaIssue>;
    /// Admit actual output before the splitter and field callbacks.
    fn output(
        &self,
        guaranteed: &[Arc<str>],
        maximum: usize,
        policy: ParsePolicy,
    ) -> Result<Self::Output, SchemaIssue>;
    /// Check a field before calling its encoder.
    fn preflight(&self, output: &Self::Output, name: &str) -> Result<(), SchemaIssue>;
    /// Move one encoded field into actual output backing.
    fn push(
        &self,
        output: &mut Self::Output,
        name: &str,
        value: Self::Encoded,
    ) -> Result<(), SchemaIssue>;
    /// Publish the actual owned output.
    fn finish(&self, output: Self::Output) -> Result<Self::Encoded, SchemaIssue>;
}

/// Managed JSON record backing used by the compatibility constructor.
pub struct JsonRecordBacking {
    output: Arc<super::PreparedJsonParserPool>,
}
impl RecordBacking for JsonRecordBacking {
    type Encoded = SharedPreparedJsonValue;
    type Input = SharedPreparedJsonValue;
    type Output = PreparedObjectAssembly;
    fn read(
        &self,
        input: Self::Encoded,
        policy: ParsePolicy,
        names: &[Arc<str>],
        context: Option<&ParseContext>,
        issues: &PreparedIssueMemory,
        validation: &PreparedJsonValidationPool,
    ) -> Result<Self::Input, SchemaIssue> {
        validation.check_parse_limits(&input, &policy.limits)?;
        if input.view().kind() != Some(super::PreparedJsonKind::Object) {
            return Err(SchemaIssue::new(super::SchemaIssueKind::TypeMismatch {
                expected: super::ValueKind::Record,
                actual: super::prepared_view_kind(input.view())?,
            }));
        }
        let mut current = input.view().first_child();
        let mut position = 0usize;
        while let Some(child) = current {
            let name = child.property_name().ok_or_else(invalid)?;
            let mut prior = input.view().first_child();
            for _ in 0..position {
                let previous = prior.ok_or_else(invalid)?;
                if previous.property_name().ok_or_else(invalid)? == name {
                    return Err(super::schema_issue::prepared_duplicate_field(
                        context
                            .and_then(ParseContext::issue_memory)
                            .or(Some(issues)),
                        name,
                    ));
                }
                prior = previous.next_sibling();
            }
            position = position.checked_add(1).ok_or_else(|| {
                super::prepared_alternatives::memory(MemoryError::ArithmeticOverflow)
            })?;
            current = child.next_sibling();
        }
        if policy.unknown_fields == super::UnknownFields::Reject {
            let mut current = input.view().first_child();
            let mut unknown: Option<&str> = None;
            while let Some(child) = current {
                let name = child.property_name().ok_or_else(invalid)?;
                if !names.iter().any(|known| known.as_ref() == name)
                    && unknown.is_none_or(|old| name < old)
                {
                    unknown = Some(name);
                }
                current = child.next_sibling();
            }
            if let Some(name) = unknown {
                return Err(super::prepared_unknown_field(context, Some(issues), name));
            }
        }
        Ok(input)
    }
    fn take(
        &self,
        input: &mut Self::Input,
        name: &str,
    ) -> Result<Option<Self::Encoded>, SchemaIssue> {
        input
            .try_get(name)
            .map_err(super::prepared_alternatives::memory)
    }
    fn output(
        &self,
        guaranteed: &[Arc<str>],
        _: usize,
        policy: ParsePolicy,
    ) -> Result<Self::Output, SchemaIssue> {
        let output = self.output.admit_object(policy)?;
        output.preflight_names(guaranteed.iter().map(AsRef::as_ref))?;
        Ok(output)
    }
    fn preflight(&self, output: &Self::Output, name: &str) -> Result<(), SchemaIssue> {
        output.preflight(name)
    }
    fn push(
        &self,
        output: &mut Self::Output,
        name: &str,
        value: Self::Encoded,
    ) -> Result<(), SchemaIssue> {
        output.insert_value(name, value)
    }
    fn finish(&self, output: Self::Output) -> Result<Self::Encoded, SchemaIssue> {
        output.finish()
    }
}

/// Generic record backing with actual caller-selected entry and representation owners.
pub struct PreparedGenericRecordBacking<
    Encoded: Send + 'static,
    R: super::PreparedRecordRepresentation<Encoded>,
> {
    representation: Arc<R>,
    fields: Arc<super::PreparedJsonArrayPool<super::PreparedObjectEntry<Encoded>>>,
}
impl<Encoded: Send + 'static, R: super::PreparedRecordRepresentation<Encoded>>
    PreparedGenericRecordBacking<Encoded, R>
{
    /// Select an actual object representation and owned input-entry pool.
    #[must_use]
    pub fn new(
        representation: Arc<R>,
        fields: Arc<super::PreparedJsonArrayPool<super::PreparedObjectEntry<Encoded>>>,
    ) -> Self {
        Self {
            representation,
            fields,
        }
    }
}
impl<Encoded: Send + 'static, R: super::PreparedRecordRepresentation<Encoded>> RecordBacking
    for PreparedGenericRecordBacking<Encoded, R>
{
    type Encoded = Encoded;
    type Input = super::PreparedObjectValue<Encoded>;
    type Output = R::WriteAdmission;
    fn read(
        &self,
        input: Encoded,
        policy: ParsePolicy,
        names: &[Arc<str>],
        context: Option<&ParseContext>,
        issues: &PreparedIssueMemory,
        _: &PreparedJsonValidationPool,
    ) -> Result<Self::Input, SchemaIssue> {
        use super::PreparedObjectCursor;
        let mut fields = self
            .fields
            .reserve()
            .map_err(super::prepared_alternatives::memory)?;
        let mut cursor = self.representation.read(input, policy)?;
        let count = cursor.remaining();
        if count > policy.limits.collection.get() {
            return Err(SchemaIssue::new(super::SchemaIssueKind::LimitExceeded {
                resource: super::SchemaResource::Width,
            }));
        }
        if fields.capacity() < count {
            return Err(super::prepared_alternatives::memory(MemoryError::NoSlot));
        }
        for index in 0..count {
            let before = cursor.remaining();
            if count.checked_sub(index) != Some(before) {
                return Err(invalid());
            }
            let (name, value) = cursor.take_next()?.ok_or_else(invalid)?;
            if before.checked_sub(1) != Some(cursor.remaining()) {
                return Err(invalid());
            }
            if fields
                .as_slice()
                .iter()
                .any(|entry| entry.name() == name.as_str())
            {
                return Err(super::schema_issue::prepared_duplicate_field(
                    context
                        .and_then(ParseContext::issue_memory)
                        .or(Some(issues)),
                    name.as_str(),
                ));
            }
            fields
                .push(super::PreparedObjectEntry::new(name, value))
                .map_err(|_| super::prepared_alternatives::memory(MemoryError::NoSlot))?;
        }
        if cursor.remaining() != 0 {
            return Err(invalid());
        }
        if policy.unknown_fields == super::UnknownFields::Reject {
            let unknown = fields
                .as_slice()
                .iter()
                .map(super::PreparedObjectEntry::name)
                .filter(|name| !names.iter().any(|known| known.as_ref() == *name))
                .min();
            if let Some(name) = unknown {
                return Err(super::prepared_unknown_field(context, Some(issues), name));
            }
        }
        Ok(super::PreparedObjectValue::from_entries(fields))
    }
    fn take(&self, input: &mut Self::Input, name: &str) -> Result<Option<Encoded>, SchemaIssue> {
        Ok(input.take_named(name))
    }
    fn output(
        &self,
        guaranteed: &[Arc<str>],
        maximum: usize,
        policy: ParsePolicy,
    ) -> Result<Self::Output, SchemaIssue> {
        self.representation.admit_record_write(
            guaranteed.iter().map(AsRef::as_ref),
            maximum,
            policy,
        )
    }
    fn preflight(&self, output: &Self::Output, name: &str) -> Result<(), SchemaIssue> {
        self.representation.preflight(output, name)
    }
    fn push(
        &self,
        output: &mut Self::Output,
        name: &str,
        value: Encoded,
    ) -> Result<(), SchemaIssue> {
        self.representation.push(output, name, value)
    }
    fn finish(&self, output: Self::Output) -> Result<Encoded, SchemaIssue> {
        self.representation.finish_record(output)
    }
}

#[doc(hidden)]
pub trait ErrorPolicy<X>: Send + Sync + 'static {
    fn map(&self, issue: SchemaIssue) -> X;
    fn classify(&self, error: X) -> Result<SchemaIssue, X>;
}
struct SelectedPolicy<F, G> {
    map: F,
    classify: G,
}
impl<
    X: Send + 'static,
    F: Fn(SchemaIssue) -> X + Send + Sync + 'static,
    G: Fn(X) -> Result<SchemaIssue, X> + Send + Sync + 'static,
> ErrorPolicy<X> for SelectedPolicy<F, G>
{
    fn map(&self, issue: SchemaIssue) -> X {
        (self.map)(issue)
    }
    fn classify(&self, error: X) -> Result<SchemaIssue, X> {
        (self.classify)(error)
    }
}
#[doc(hidden)]
pub type ErrorOwner<X> = Arc<dyn ErrorPolicy<X>>;
type IssueArray = PreparedJsonArrayBuilder<SchemaIssue>;
/// Explicit simultaneous operation records and retained issue-builder slots.
#[derive(Clone, Copy, Debug)]
pub struct PreparedRecordOperationCapacity {
    /// Simultaneous record operations.
    pub operations: usize,
    /// Physically backed issue builders per operation.
    pub issue_builders: usize,
}
impl PreparedRecordOperationCapacity {
    /// Region state/slot and actual builder-index bytes; issue arrays and frames are separate.
    pub fn demand<X: Send + 'static>(self) -> Result<Bytes, MemoryError> {
        self.demand_for::<X, JsonRecordBacking>()
    }
    /// Actual region demand for the selected input and output owner types.
    pub fn demand_for<X: Send + 'static, B: RecordBacking>(self) -> Result<Bytes, MemoryError> {
        let layout = std::alloc::Layout::array::<IssueArray>(self.issue_builders)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Cell<X, B>>::storage_bytes(self.operations)?
            .checked_add(Bytes::new(layout.size()).checked_mul(self.operations)?)
    }
}
struct RecordState<X: Send + 'static, B: RecordBacking> {
    input: Option<B::Input>,
    assembly: Option<B::Output>,
    backing: Arc<B>,
    writer: Option<JsonWriter>,
    builders: Option<FixedVector<'static, IssueArray>>,
    context: Option<ParseContext>,
    policy: ParsePolicy,
    budget: Budget,
    _errors: ErrorOwner<X>,
    issues: Arc<PreparedIssueMemory>,
    validation: Arc<PreparedJsonValidationPool>,
}
struct CellData<X: Send + 'static, B: RecordBacking> {
    state: Option<RecordState<X, B>>,
    builders: Option<FixedVector<'static, IssueArray>>,
}
type Cell<X, B> = Mutex<CellData<X, B>>;
type Slot<X, B> = SlotLease<'static, Cell<X, B>>;
type Shared<X, B> = SharedSlot<'static, Cell<X, B>>;
/// Fixed region-owned operation records for required-field prepared records.
pub struct PreparedRecordOperationPool<X: Send + 'static, B: RecordBacking = JsonRecordBacking> {
    slots: SlotPool<'static, Cell<X, B>>,
}

impl<X: Send + 'static, B: RecordBacking> PreparedRecordOperationPool<X, B> {
    /// Prepare the actual operation records and their fixed builder indexes.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedRecordOperationCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand_for::<X, B>()?;
        let slots = memory.slots(region, capacity.operations, |_| {
            Ok(Mutex::new(CellData {
                state: None,
                builders: Some(FixedVector::prepare(
                    memory.allocator(region)?,
                    capacity.issue_builders,
                )?),
            }))
        })?;
        Ok(Self { slots })
    }
    fn reserve(&self) -> Result<BuildLease<X, B>, MemoryError> {
        let slot = self.slots.checkout()?;
        let builders = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .builders
            .take()
            .ok_or(MemoryError::Closed)?;
        Ok(BuildLease {
            slot: Some(slot),
            builders: Some(builders),
        })
    }
}
fn drain_builders(builders: &mut FixedVector<'static, IssueArray>) {
    let mut continuation = std::mem::DropGuard::new(builders, |builders| {
        while let Some(builder) = builders.pop() {
            drop(builder)
        }
    });
    while let Some(builder) = continuation.pop() {
        drop(builder)
    }
    let _ = std::mem::DropGuard::dismiss(continuation);
}
struct BuildLease<X: Send + 'static, B: RecordBacking> {
    slot: Option<Slot<X, B>>,
    builders: Option<FixedVector<'static, IssueArray>>,
}
impl<X: Send + 'static, B: RecordBacking> BuildLease<X, B> {
    fn capacity(&self) -> usize {
        self.builders.as_ref().map_or(0, FixedVector::capacity)
    }
    fn fill(
        &mut self,
        pool: &super::PreparedJsonArrayPool<SchemaIssue>,
        count: usize,
    ) -> Result<(), MemoryError> {
        if self.capacity() < count {
            return Err(MemoryError::NoSlot);
        }
        for _ in 0..count {
            let builder = pool.reserve_shared()?;
            if builder.capacity() < 2 {
                drop(builder);
                return Err(MemoryError::NoSlot);
            }
            self.builders
                .as_mut()
                .ok_or(MemoryError::Closed)?
                .push(builder)
                .map_err(|_| MemoryError::NoSlot)?;
        }
        Ok(())
    }
    fn publish(mut self, mut state: RecordState<X, B>) -> Result<RecordHandle<X, B>, MemoryError> {
        state.builders = self.builders.take();
        let slot = self.slot.take().ok_or(MemoryError::Closed)?;
        let old = {
            let mut cell = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cell.state.replace(state)
        };
        drop(old);
        Ok(RecordHandle {
            slot: Some(slot.share()),
        })
    }
}
impl<X: Send + 'static, B: RecordBacking> Drop for BuildLease<X, B> {
    fn drop(&mut self) {
        if let (Some(slot), Some(builders)) = (self.slot.take(), self.builders.take()) {
            let mut restore = std::mem::DropGuard::new((slot, builders), |(slot, builders)| {
                let old = {
                    let mut cell = slot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    cell.builders.replace(builders)
                };
                drop(old);
                drop(slot);
            });
            drain_builders(&mut restore.1);
        }
    }
}
/// Private-construction shared owner of one actual prepared record operation.
#[doc(hidden)]
pub struct RecordHandle<X: Send + 'static, B: RecordBacking = JsonRecordBacking> {
    slot: Option<Shared<X, B>>,
}
impl<X: Send + 'static, B: RecordBacking> RecordHandle<X, B> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            slot: Some(self.slot.as_ref().ok_or(MemoryError::Closed)?.try_clone()?),
        })
    }
    fn with<T>(
        &self,
        f: impl FnOnce(&mut RecordState<X, B>) -> Result<T, SchemaIssue>,
    ) -> Result<T, SchemaIssue> {
        let slot = self.slot.as_ref().ok_or_else(invalid)?;
        let state = {
            let mut cell = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cell.state.take().ok_or_else(invalid)?
        };
        let mut restore = std::mem::DropGuard::new((slot, state), |(slot, state)| {
            let old = {
                let mut cell = slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                cell.state.replace(state)
            };
            drop(old);
        });
        f(&mut restore.1)
    }
}
impl<X: Send + 'static, B: RecordBacking> Drop for RecordHandle<X, B> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take()
            && let Some(slot) = slot.into_last()
        {
            let state = {
                let mut cell = slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                cell.state.take()
            };
            if let Some(mut state) = state
                && let Some(builders) = state.builders.take()
            {
                let cleanup = BuildLease {
                    slot: Some(slot),
                    builders: Some(builders),
                };
                drop(state);
                drop(cleanup);
            }
        }
    }
}
impl<X: Send + 'static, B: RecordBacking> RecordState<X, B> {
    fn begin_pair(&mut self) -> Result<(bool, usize), SchemaIssue> {
        let first = self.policy.errors == super::SchemaErrorMode::First;
        if !first {
            self.budget.available()?;
        }
        Ok((first, self.budget.used()))
    }
    fn before_rest(&self) -> usize {
        self.budget.used()
    }
    fn account(
        &mut self,
        before: usize,
        issue: SchemaIssue,
        more: bool,
    ) -> Result<SchemaIssue, SchemaIssue> {
        if super::prepared_alternatives::root_resource(&issue) {
            return Err(issue);
        }
        let scan = self.validation.scan_issue(&issue)?;
        if scan.resource {
            return Err(issue);
        }
        self.budget.record(before, scan.count)?;
        if more {
            self.budget.available()?;
        }
        Ok(issue)
    }
    fn combine<A, Z>(
        &mut self,
        first_mode: bool,
        before: usize,
        before_rest: usize,
        first: Result<A, SchemaIssue>,
        second: Result<Z, SchemaIssue>,
    ) -> Result<Result<(A, Z), SchemaIssue>, SchemaIssue> {
        if first_mode {
            return Ok(first.and_then(|a| second.map(|b| (a, b))));
        }
        let second = match second {
            Ok(value) => Ok(value),
            Err(issue) => Err(self.account(before_rest, issue, false)?),
        };
        let combined = match (first, second) {
            (Ok(a), Ok(b)) => Ok((a, b)),
            (Err(issue), Ok(_)) | (Ok(_), Err(issue)) => Err(issue),
            (Err(a), Err(b)) => {
                let mut builder = self
                    .builders
                    .as_mut()
                    .and_then(FixedVector::pop)
                    .ok_or_else(|| super::prepared_alternatives::memory(MemoryError::NoSlot))?;
                builder
                    .try_push(a)
                    .map_err(|_| super::prepared_alternatives::memory(MemoryError::NoSlot))?;
                builder
                    .try_push(b)
                    .map_err(|_| super::prepared_alternatives::memory(MemoryError::NoSlot))?;
                Err(SchemaIssue::new(super::SchemaIssueKind::Multiple)
                    .with_prepared_children(builder.publish()))
            }
        };
        if let Err(issue) = &combined {
            let count = self.validation.count_issue(issue)?;
            self.budget.record(before, count)?;
        }
        Ok(combined)
    }
}
impl<X: Send + 'static, B: RecordBacking> RecordHandle<X, B> {
    fn prefix(&self, issue: SchemaIssue, name: &Arc<str>) -> SchemaIssue {
        let mut original = Some(issue);
        let result = self.with(|state| {
            let selected = state
                .context
                .as_ref()
                .and_then(ParseContext::issue_memory)
                .or(Some(&state.issues));
            let issue = original.take().ok_or_else(invalid)?;
            Ok(super::parse_context::selected_issue_at(
                selected,
                issue,
                super::IssueSegment::Field(Arc::clone(name)),
            ))
        });
        match result {
            Ok(issue) => issue,
            Err(decoration) => match original {
                Some(issue)
                    if super::prepared_json_validation::physical_cause(&issue).is_some() =>
                {
                    issue
                }
                Some(_) | None => decoration,
            },
        }
    }
}
fn invalid() -> SchemaIssue {
    SchemaIssue::new(super::SchemaIssueKind::InvalidDefinition)
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::PreparedEffectRecordNil {}
    impl<T, A, C, H, R: Sealed, P> Sealed for super::PreparedEffectRecordField<T, A, C, H, R, P> {}
}
/// Empty required-field list; its owned field tuple is `()`.
pub struct PreparedEffectRecordNil;
/// A required field, always emitted.
pub struct Required;
/// An absent-value factory and conditional omission rule.
pub struct Optional<F, N, O> {
    factory: F,
    omit: O,
    annotation: Option<Arc<serde_json::Value>>,
    marker: std::marker::PhantomData<fn() -> N>,
}
/// An absent-value factory with an independent borrowed validator.
pub struct Defaulted<F, N, V> {
    factory: F,
    validate: V,
    annotation: Option<Arc<serde_json::Value>>,
    marker: std::marker::PhantomData<fn() -> N>,
}
/// Uses the child's genuine borrowed default witness.
pub struct ChildDefaultValidator;
/// Uses a caller-supplied borrowed validator.
pub struct ContextDefaultValidator<V>(V);
#[doc(hidden)]
pub trait DefaultValidator<A, C, D, E, X, B: RecordBacking>: Send + Sync + 'static
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    C: super::PreparedCodec<A, B::Encoded, D, E, Error = X>,
{
    fn validate(&self, codec: &C, value: &A, context: Option<&ParseContext>) -> Result<(), X>;
}
impl<A, C, D, E, X, B: RecordBacking> DefaultValidator<A, C, D, E, X, B> for ChildDefaultValidator
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    C: super::PreparedCodec<A, B::Encoded, D, E, Error = X>,
{
    fn validate(&self, codec: &C, value: &A, context: Option<&ParseContext>) -> Result<(), X> {
        codec.validate_default_ref(value, context)
    }
}
impl<A, C, D, E, X, V, B: RecordBacking> DefaultValidator<A, C, D, E, X, B>
    for ContextDefaultValidator<V>
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    C: super::PreparedCodec<A, B::Encoded, D, E, Error = X>,
    V: Fn(&A, Option<&ParseContext>) -> Result<(), X> + Send + Sync + 'static,
{
    fn validate(&self, _: &C, value: &A, context: Option<&ParseContext>) -> Result<(), X> {
        (self.0)(value, context)
    }
}
#[doc(hidden)]
pub trait FieldPolicy<A, C, D, E, X, B: RecordBacking>: Send + Sync + 'static
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    C: super::PreparedCodec<A, B::Encoded, D, E, Error = X>,
{
    fn required(&self) -> bool;
    fn always_emitted(&self) -> bool;
    fn omit(&self, value: &A) -> bool;
    fn annotation(&self) -> Option<Arc<serde_json::Value>>;
    fn missing(
        self: Arc<Self>,
        codec: Arc<C>,
        context: Option<ParseContext>,
        name: Arc<str>,
        errors: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<Value = A, Error = X, Env = D>;
}
impl<A, C, D, E, X, B: RecordBacking> FieldPolicy<A, C, D, E, X, B> for Required
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    C: super::PreparedCodec<A, B::Encoded, D, E, Error = X>,
{
    fn required(&self) -> bool {
        true
    }
    fn always_emitted(&self) -> bool {
        true
    }
    fn omit(&self, _: &A) -> bool {
        false
    }
    fn annotation(&self) -> Option<Arc<serde_json::Value>> {
        None
    }
    fn missing(
        self: Arc<Self>,
        _: Arc<C>,
        _: Option<ParseContext>,
        name: Arc<str>,
        errors: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<Value = A, Error = X, Env = D> {
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: move || Err(errors.map(SchemaIssue::new(super::SchemaIssueKind::MissingField(name)))), }, }
    }
}
impl<A, C, D, E, X, F, N, O, B: RecordBacking> FieldPolicy<A, C, D, E, X, B> for Optional<F, N, O>
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    C: super::PreparedCodec<A, B::Encoded, D, E, Error = X>,
    F: Fn() -> N + Send + Sync + 'static,
    O: Fn(&A) -> bool + Send + Sync + 'static,
    N: crate::__private::prepared_graph::Description<Value = A, Error = X, Env = D>,
{
    fn required(&self) -> bool {
        false
    }
    fn always_emitted(&self) -> bool {
        false
    }
    fn omit(&self, value: &A) -> bool {
        (self.omit)(value)
    }
    fn annotation(&self) -> Option<Arc<serde_json::Value>> {
        self.annotation.clone()
    }
    fn missing(
        self: Arc<Self>,
        _: Arc<C>,
        _: Option<ParseContext>,
        _: Arc<str>,
        _: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<Value = A, Error = X, Env = D> {
        crate::effect_prepare! { description; program: effect! { let value=(self.factory)()?; value }, }
    }
}
impl<A, C, D, E, X, F, N, V, B: RecordBacking> FieldPolicy<A, C, D, E, X, B> for Defaulted<F, N, V>
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    C: super::PreparedCodec<A, B::Encoded, D, E, Error = X>,
    F: Fn() -> N + Send + Sync + 'static,
    V: DefaultValidator<A, C, D, E, X, B>,
    N: crate::__private::prepared_graph::Description<Value = A, Error = X, Env = D>,
{
    fn required(&self) -> bool {
        false
    }
    fn always_emitted(&self) -> bool {
        true
    }
    fn omit(&self, _: &A) -> bool {
        false
    }
    fn annotation(&self) -> Option<Arc<serde_json::Value>> {
        self.annotation.clone()
    }
    fn missing(
        self: Arc<Self>,
        codec: Arc<C>,
        context: Option<ParseContext>,
        _: Arc<str>,
        _: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<Value = A, Error = X, Env = D> {
        let factory = Arc::clone(&self);
        crate::effect_prepare! { description; program: effect! {
            let value=(factory.factory)()?;
            let value=effect_try_sync! { once; body: move || { self.validate.validate(codec.as_ref(),&value,context.as_ref())?; Ok(value) }, }?;
            value
        }, }
    }
}
/// One field with an owned codec, policy and typed remaining list.
pub struct PreparedEffectRecordField<T, A, C, H, Rest, P = Required> {
    name: Arc<str>,
    codec: Arc<C>,
    getter: H,
    rest: Arc<Rest>,
    policy: Arc<P>,
    marker: std::marker::PhantomData<fn(&T) -> &A>,
}
/// Sealed statically typed required-field list.
pub trait PreparedEffectRecordFields<T, D, E, X, B: RecordBacking = JsonRecordBacking>:
    sealed::Sealed + Send + Sync + 'static
where
    T: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
{
    /// The actual nested owned `(Field, Rest)` tuple.
    type Values: Send + 'static;
    /// Owned fields paired with their already-computed omission decisions.
    type PreparedValues: Send + 'static;
    #[doc(hidden)]
    fn prepare_values(
        &self,
        values: Self::Values,
        context: Option<&ParseContext>,
        issues: &PreparedIssueMemory,
    ) -> Self::PreparedValues;
    #[doc(hidden)]
    fn decode(
        self: Arc<Self>,
        handle: RecordHandle<X, B>,
        errors: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = Result<Self::Values, SchemaIssue>,
        Error = X,
        Env = D,
    >;
    #[doc(hidden)]
    fn encode(
        self: Arc<Self>,
        values: Self::PreparedValues,
        handle: RecordHandle<X, B>,
        errors: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = Result<(), SchemaIssue>,
        Error = X,
        Env = E,
    >;
    #[doc(hidden)]
    fn validate(
        self: Arc<Self>,
        values: Self::PreparedValues,
        handle: RecordHandle<X, B>,
        errors: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = Result<Self::Values, SchemaIssue>,
        Error = X,
        Env = E,
    >;
    #[doc(hidden)]
    fn write(
        self: Arc<Self>,
        values: Self::Values,
        handle: RecordHandle<X, B>,
        errors: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<Value = (), Error = X, Env = E>;
    #[doc(hidden)]
    fn properties(&self, output: &mut Vec<super::SchemaProperty>) -> Result<(), SchemaIssue>;
    #[doc(hidden)]
    fn emitted_names(&self, names: &mut Vec<Arc<str>>);
    #[doc(hidden)]
    fn has_validator(&self) -> bool;
    #[doc(hidden)]
    fn has_default_validator(&self) -> bool;
    #[doc(hidden)]
    fn has_writer(&self) -> bool;
    #[doc(hidden)]
    fn validate_borrowed(
        &self,
        value: &T,
        context: Option<&ParseContext>,
        issues: &PreparedIssueMemory,
        policy: &dyn ErrorPolicy<X>,
    ) -> Result<(), X>;
}
impl<T, D, E, X, B: RecordBacking> PreparedEffectRecordFields<T, D, E, X, B>
    for PreparedEffectRecordNil
where
    T: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
{
    type Values = ();
    type PreparedValues = ();
    fn prepare_values(&self, (): (), _: Option<&ParseContext>, _: &PreparedIssueMemory) {}
    fn decode(
        self: Arc<Self>,
        _: RecordHandle<X, B>,
        _: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = Result<(), SchemaIssue>,
        Error = X,
        Env = D,
    > {
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: || Ok::<_,X>(Ok(())), }, }
    }
    fn encode(
        self: Arc<Self>,
        _: (),
        _: RecordHandle<X, B>,
        _: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = Result<(), SchemaIssue>,
        Error = X,
        Env = E,
    > {
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: || Ok::<_,X>(Ok(())), }, }
    }
    fn validate(
        self: Arc<Self>,
        _: (),
        _: RecordHandle<X, B>,
        _: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = Result<(), SchemaIssue>,
        Error = X,
        Env = E,
    > {
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: || Ok::<_,X>(Ok(())), }, }
    }
    fn write(
        self: Arc<Self>,
        _: (),
        _: RecordHandle<X, B>,
        _: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<Value = (), Error = X, Env = E> {
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: || Ok::<_,X>(()), }, }
    }
    fn properties(&self, _: &mut Vec<super::SchemaProperty>) -> Result<(), SchemaIssue> {
        Ok(())
    }
    fn emitted_names(&self, _: &mut Vec<Arc<str>>) {}
    fn has_validator(&self) -> bool {
        true
    }
    fn has_default_validator(&self) -> bool {
        true
    }
    fn has_writer(&self) -> bool {
        true
    }
    fn validate_borrowed(
        &self,
        _: &T,
        _: Option<&ParseContext>,
        _: &PreparedIssueMemory,
        _: &dyn ErrorPolicy<X>,
    ) -> Result<(), X> {
        Ok(())
    }
}
impl<T, A, C, H, Rest, P, D, E, X, B: RecordBacking> PreparedEffectRecordFields<T, D, E, X, B>
    for PreparedEffectRecordField<T, A, C, H, Rest, P>
where
    T: Send + 'static,
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    C: super::PreparedCodec<A, B::Encoded, D, E, Error = X>,
    H: for<'a> Fn(&'a T) -> &'a A + Send + Sync + 'static,
    Rest: PreparedEffectRecordFields<T, D, E, X, B>,
    P: FieldPolicy<A, C, D, E, X, B>,
{
    type Values = (A, Rest::Values);
    type PreparedValues =
        Result<((A, bool, Option<ParseContext>), Rest::PreparedValues), SchemaIssue>;
    fn prepare_values(
        &self,
        (field, tail): Self::Values,
        context: Option<&ParseContext>,
        issues: &PreparedIssueMemory,
    ) -> Self::PreparedValues {
        let omitted = <P as FieldPolicy<A, C, D, E, X, B>>::omit(&self.policy, &field);
        let child = if omitted {
            None
        } else {
            let segment = super::IssueSegment::Field(Arc::clone(&self.name));
            match super::parse_context::child(context, segment) {
                Ok(child) => child,
                Err(issue) => {
                    return Err(super::parse_context::selected_issue_at(
                        context
                            .and_then(ParseContext::issue_memory)
                            .or(Some(issues)),
                        issue,
                        super::IssueSegment::Field(Arc::clone(&self.name)),
                    ));
                }
            }
        };
        let tail = self.rest.prepare_values(tail, context, issues);
        Ok(((field, omitted, child), tail))
    }
    fn decode(
        self: Arc<Self>,
        handle: RecordHandle<X, B>,
        errors: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = Result<Self::Values, SchemaIssue>,
        Error = X,
        Env = D,
    > {
        let codec = Arc::clone(&self.codec);
        let missing_codec = Arc::clone(&self.codec);
        let policy = Arc::clone(&self.policy);
        let required = <P as FieldPolicy<A, C, D, E, X, B>>::required(&self.policy);
        let missing_name = Arc::clone(&self.name);
        let missing_errors = Arc::clone(&errors);
        let rest = Arc::clone(&self.rest);
        let name = Arc::clone(&self.name);
        let prefix_name = Arc::clone(&self.name);
        let initial = handle.try_clone();
        let work = handle.try_clone();
        let mapper = handle.try_clone();
        let after = handle.try_clone();
        let rest_handle = handle.try_clone();
        let combine = handle.try_clone();
        let error_initial = Arc::clone(&errors);
        let error_work = Arc::clone(&errors);
        let error_mapper = Arc::clone(&errors);
        let error_capture = Arc::clone(&errors);
        let error_after = Arc::clone(&errors);
        let error_rest = Arc::clone(&errors);
        let error_combine = Arc::clone(&errors);
        crate::effect_prepare! { description; program: effect! {
            let (first_mode,before,work,mapper,after,rest_handle,combine)=effect_try_sync! { once; body: move || {
                let initial=initial.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let work=work.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let mapper=mapper.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let after=after.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let rest_handle=rest_handle.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let combine=combine.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let (first_mode,before)=initial.with(|state| { let pair=state.begin_pair()?;  Ok(pair) }).map_err(|issue|error_initial.map(issue))?;
                Ok((first_mode,before,work,mapper,after,rest_handle,combine))
            }, }?;
            let result=crate::effect_prepare! { description; program: effect_map_error! {
                program: effect! { let (encoded,context)=effect_try_sync! { once; body: move || work.with(|state| { let encoded=state.backing.take(state.input.as_mut().ok_or_else(invalid)?, &name)?; let context=if encoded.is_some() || !required { super::parse_context::child(state.context.as_ref(),super::IssueSegment::Field(Arc::clone(&name)))? } else { None }; Ok((encoded,context)) }).map_err(|issue| error_work.map(issue)), }?; match encoded { Some(encoded) => { let field=<C as super::PreparedCodec<A,B::Encoded,D,E>>::decode(codec,encoded,context)?; field }, None => { let field=<P as FieldPolicy<A,C,D,E,X,B>>::missing(policy,missing_codec,context,missing_name,missing_errors)?; field } } },
                map: move |error| match error_mapper.classify(error) { Ok(issue)=>error_mapper.map(mapper.prefix(issue,&prefix_name)),Err(error)=>error },
            }, capture_errors: !first_mode, classify_error: move |error| error_capture.classify(error), }?;
            let (first,before_rest)=effect_try_sync! { once; body: move || {
                let first=result;
                let first=if !first_mode {match first {Ok(field)=>Ok(field),Err(issue)=>Err(after.with(|state|state.account(before,issue,true)).map_err(|issue|error_after.map(issue))?)}} else {first};
                let before_rest=after.with(|state|Ok(state.before_rest())).map_err(|issue|error_after.map(issue))?;
                Ok((first,before_rest))
            }, }?;
            match first {
                Err(issue) if first_mode => { let value=effect_try_sync! { once; body: move || Ok::<_,X>(Err(issue)), }?; value },
                first => {
                    let first=effect_try_sync! { once; body: move || {
                        match first { Ok(field)=>{  Ok::<_,X>(Ok(field)) },Err(issue)=>Ok(Err(issue)) }
                    }, }?;
                    let second=rest.decode(rest_handle,Arc::clone(&error_rest))?;
                    let combined=effect_try_sync! { once; body: move || combine.with(|state|state.combine(first_mode,before,before_rest,first,second)).map_err(|issue|error_combine.map(issue)), }?;
                    combined
                },
            }
        }, }
    }
    fn encode(
        self: Arc<Self>,
        values: Self::PreparedValues,
        handle: RecordHandle<X, B>,
        errors: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = Result<(), SchemaIssue>,
        Error = X,
        Env = E,
    > {
        let codec = Arc::clone(&self.codec);
        let rest = Arc::clone(&self.rest);
        let preflight_name = Arc::clone(&self.name);
        let output_name = Arc::clone(&self.name);
        let prefix_name = Arc::clone(&self.name);
        let initial = handle.try_clone();
        let mapper = handle.try_clone();
        let after = handle.try_clone();
        let rest_handle = handle.try_clone();
        let combine = handle.try_clone();
        let work_output = handle.try_clone();
        let error_initial = Arc::clone(&errors);
        let error_mapper = Arc::clone(&errors);
        let error_capture = Arc::clone(&errors);
        let error_after = Arc::clone(&errors);
        let error_rest = Arc::clone(&errors);
        let error_rest_classify = Arc::clone(&errors);
        let error_combine = Arc::clone(&errors);
        let error_output = Arc::clone(&errors);
        crate::effect_prepare! { description; program: effect! {
            let (first_mode,before,mapper,after,rest_handle,combine,work_output,omitted,field,context,tail)=effect_try_sync! { once; body: move || {
                let ((field,omitted,context),tail)=values.map_err(|issue|error_initial.map(issue))?;
                let initial=initial.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let mapper=mapper.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let after=after.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let rest_handle=rest_handle.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let combine=combine.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let work_output=work_output.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let (first_mode,before)=initial.with(|state| { let pair=state.begin_pair()?; if !omitted { state.backing.preflight(state.assembly.as_ref().ok_or_else(invalid)?, &preflight_name)?; } Ok(pair) }).map_err(|issue|error_initial.map(issue))?;
                Ok((first_mode,before,mapper,after,rest_handle,combine,work_output,omitted,field,context,tail))
            }, }?;
            let result=crate::effect_prepare! { description; program: effect_map_error! {
                program: effect! { if omitted { let value=effect_sync! { once; body: move || { drop(field); None }, }?; value } else { let field=<C as super::PreparedCodec<A,B::Encoded,D,E>>::encode(codec,field,context)?; Some(field) } },
                map: move |error| match error_mapper.classify(error) { Ok(issue)=>error_mapper.map(mapper.prefix(issue,&prefix_name)),Err(error)=>error },
            }, capture_errors: !first_mode, classify_error: move |error| error_capture.classify(error), }?;
            let (first,before_rest)=effect_try_sync! { once; body: move || {
                let first=result;
                let first=if !first_mode {match first {Ok(field)=>Ok(field),Err(issue)=>Err(after.with(|state|state.account(before,issue,true)).map_err(|issue|error_after.map(issue))?)}} else {first};
                let before_rest=after.with(|state|Ok(state.before_rest())).map_err(|issue|error_after.map(issue))?;
                Ok((first,before_rest))
            }, }?;
            match first {
                Err(issue) if first_mode => { let value=effect_try_sync! { once; body: move || Ok::<_,X>(Err(issue)), }?; value },
                first => {
                    let first=effect_try_sync! { once; body: move || {
                        match first { Ok(field)=>{ if let Some(field)=field { work_output.with(|state| state.backing.push(state.assembly.as_mut().ok_or_else(invalid)?, &output_name, field)).map_err(|issue| error_output.map(issue))?; } Ok::<_,X>(Ok(())) },Err(issue)=>Ok(Err(issue)) }
                    }, }?;
                    let second=crate::effect_prepare! { description; program: rest.encode(tail,rest_handle,Arc::clone(&error_rest)), capture_errors: !first_mode, classify_error: move |error| error_rest_classify.classify(error), }?;
                    let second=second.and_then(std::convert::identity);
                    let combined=effect_try_sync! { once; body: move || combine.with(|state|state.combine(first_mode,before,before_rest,first,second)).map_err(|issue|error_combine.map(issue)), }?;
                    combined.map(|_|())
                },
            }
        }, }
    }
    fn validate(
        self: Arc<Self>,
        values: Self::PreparedValues,
        handle: RecordHandle<X, B>,
        errors: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = Result<Self::Values, SchemaIssue>,
        Error = X,
        Env = E,
    > {
        let codec = Arc::clone(&self.codec);
        let rest = Arc::clone(&self.rest);
        let prefix_name = Arc::clone(&self.name);
        let initial = handle.try_clone();
        let mapper = handle.try_clone();
        let after = handle.try_clone();
        let rest_handle = handle.try_clone();
        let combine = handle.try_clone();
        let error_initial = Arc::clone(&errors);
        let error_mapper = Arc::clone(&errors);
        let error_capture = Arc::clone(&errors);
        let error_after = Arc::clone(&errors);
        let error_rest = Arc::clone(&errors);
        let error_rest_classify = Arc::clone(&errors);
        let error_combine = Arc::clone(&errors);
        crate::effect_prepare! { description; program: effect! {
            let (first_mode,before,mapper,after,rest_handle,combine,omitted,field,context,tail)=effect_try_sync! { once; body: move || {
                let ((field,omitted,context),tail)=values.map_err(|issue|error_initial.map(issue))?;
                let initial=initial.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let mapper=mapper.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let after=after.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let rest_handle=rest_handle.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let combine=combine.map_err(|error|error_initial.map(super::prepared_alternatives::memory(error)))?;
                let (first_mode,before)=initial.with(|state| { let pair=state.begin_pair()?;  Ok(pair) }).map_err(|issue|error_initial.map(issue))?;
                Ok((first_mode,before,mapper,after,rest_handle,combine,omitted,field,context,tail))
            }, }?;
            let result=crate::effect_prepare! { description; program: effect_map_error! {
                program: effect! { if omitted { let field=effect_sync! { once; body: move || field, }?; field } else { let field=<C as super::PreparedCodec<A,B::Encoded,D,E>>::validate(codec,field,context)?; field } },
                map: move |error| match error_mapper.classify(error) { Ok(issue)=>error_mapper.map(mapper.prefix(issue,&prefix_name)),Err(error)=>error },
            }, capture_errors: !first_mode, classify_error: move |error| error_capture.classify(error), }?;
            let (first,before_rest)=effect_try_sync! { once; body: move || {
                let first=result;
                let first=if !first_mode {match first {Ok(field)=>Ok(field),Err(issue)=>Err(after.with(|state|state.account(before,issue,true)).map_err(|issue|error_after.map(issue))?)}} else {first};
                let before_rest=after.with(|state|Ok(state.before_rest())).map_err(|issue|error_after.map(issue))?;
                Ok((first,before_rest))
            }, }?;
            match first {
                Err(issue) if first_mode => { let value=effect_try_sync! { once; body: move || Ok::<_,X>(Err(issue)), }?; value },
                first => {
                    let first=effect_try_sync! { once; body: move || {
                        match first { Ok(field)=>{  Ok::<_,X>(Ok(field)) },Err(issue)=>Ok(Err(issue)) }
                    }, }?;
                    let second=crate::effect_prepare! { description; program: rest.validate(tail,rest_handle,Arc::clone(&error_rest)), capture_errors: !first_mode, classify_error: move |error| error_rest_classify.classify(error), }?;
                    let second=second.and_then(std::convert::identity);
                    let combined=effect_try_sync! { once; body: move || combine.with(|state|state.combine(first_mode,before,before_rest,first,second)).map_err(|issue|error_combine.map(issue)), }?;
                    combined
                },
            }
        }, }
    }

    fn write(
        self: Arc<Self>,
        values: Self::Values,
        handle: RecordHandle<X, B>,
        errors: ErrorOwner<X>,
    ) -> impl crate::__private::prepared_graph::Description<Value = (), Error = X, Env = E> {
        let (field, tail) = values;
        let codec = Arc::clone(&self.codec);
        let policy = Arc::clone(&self.policy);
        let rest = Arc::clone(&self.rest);
        let name = Arc::clone(&self.name);
        let prefix_name = Arc::clone(&self.name);
        let work = handle.try_clone();
        let mapper = handle.try_clone();
        let save = handle.try_clone();
        let rest_handle = handle.try_clone();
        let error_work = Arc::clone(&errors);
        let error_mapper = Arc::clone(&errors);
        let error_save = Arc::clone(&errors);
        let error_rest = Arc::clone(&errors);
        crate::effect_prepare! { description; program: effect! {
            let (writer,mapper,save,rest_handle,omitted,field)=effect_try_sync! { once; body: move || {
                let work=work.map_err(|error|error_work.map(super::prepared_alternatives::memory(error)))?;
                let mapper=mapper.map_err(|error|error_work.map(super::prepared_alternatives::memory(error)))?;
                let save=save.map_err(|error|error_work.map(super::prepared_alternatives::memory(error)))?;
                let rest_handle=rest_handle.map_err(|error|error_work.map(super::prepared_alternatives::memory(error)))?;
                let omitted=<P as FieldPolicy<A,C,D,E,X,B>>::omit(&policy,&field);
                let writer=work.with(|state| {let mut writer=state.writer.take().ok_or_else(invalid)?;if !omitted { writer.property(&name)?; } Ok(writer)}).map_err(|issue|error_work.map(issue))?;
                Ok((writer,mapper,save,rest_handle,omitted,field))
            }, }?;
            let writer=if omitted { let writer=effect_sync! { once; body: move || { drop(field); writer }, }?; writer } else { let writer=effect_map_error! { program: <C as super::PreparedCodec<A,B::Encoded,D,E>>::write(codec,field,writer),
                map: move |error| match error_mapper.classify(error) {Ok(issue)=>error_mapper.map(mapper.prefix(issue,&prefix_name)),Err(error)=>error},
            }?; writer };
            let _saved=effect_try_sync! { once; body: move || save.with(|state| {state.writer=Some(writer);Ok(())}).map_err(|issue|error_save.map(issue)), }?;
            let _done=rest.write(tail,rest_handle,error_rest)?;
            ()
        }, }
    }
    fn properties(&self, output: &mut Vec<super::SchemaProperty>) -> Result<(), SchemaIssue> {
        output.push(super::SchemaProperty::new(
            Arc::clone(&self.name),
            self.codec.ast().clone(),
            <P as FieldPolicy<A, C, D, E, X, B>>::required(&self.policy),
            <P as FieldPolicy<A, C, D, E, X, B>>::annotation(&self.policy),
        )?);
        self.rest.properties(output)
    }
    fn emitted_names(&self, names: &mut Vec<Arc<str>>) {
        if <P as FieldPolicy<A, C, D, E, X, B>>::always_emitted(&self.policy) {
            names.push(Arc::clone(&self.name));
        }
        self.rest.emitted_names(names);
    }
    fn has_validator(&self) -> bool {
        self.codec.has_validator() && self.rest.has_validator()
    }
    fn has_default_validator(&self) -> bool {
        self.codec.has_default_validator() && self.rest.has_default_validator()
    }
    fn has_writer(&self) -> bool {
        self.codec.has_writer() && self.rest.has_writer()
    }
    fn validate_borrowed(
        &self,
        value: &T,
        context: Option<&ParseContext>,
        issues: &PreparedIssueMemory,
        policy: &dyn ErrorPolicy<X>,
    ) -> Result<(), X> {
        if <P as FieldPolicy<A, C, D, E, X, B>>::omit(&self.policy, (self.getter)(value)) {
            return self.rest.validate_borrowed(value, context, issues, policy);
        }
        let selected = context
            .and_then(ParseContext::issue_memory)
            .or(Some(issues));
        let segment = || super::IssueSegment::Field(Arc::clone(&self.name));
        let child = super::parse_context::child(context, segment()).map_err(|issue| {
            policy.map(super::parse_context::selected_issue_at(
                selected,
                issue,
                segment(),
            ))
        })?;
        if let Err(error) = self
            .codec
            .validate_default_ref((self.getter)(value), child.as_ref())
        {
            return Err(match policy.classify(error) {
                Ok(issue) => policy.map(super::parse_context::selected_issue_at(
                    selected,
                    issue,
                    segment(),
                )),
                Err(error) => error,
            });
        }
        self.rest.validate_borrowed(value, context, issues, policy)
    }
}
/// Builder of a typed, required-field prepared record description.
pub struct PreparedEffectRecordBuilder<T, D, E, X, Fields = PreparedEffectRecordNil> {
    fields: Fields,
    marker: std::marker::PhantomData<fn() -> (T, X)>,
    environments: std::marker::PhantomData<fn() -> (D, E)>,
}
impl<T, D, E, X> PreparedEffectRecordBuilder<T, D, E, X> {
    /// Start an empty record definition.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            fields: PreparedEffectRecordNil,
            marker: std::marker::PhantomData,
            environments: std::marker::PhantomData,
        }
    }
}
impl<T, D, E, X, Fields> PreparedEffectRecordBuilder<T, D, E, X, Fields> {
    /// Add one required field without cloning its owned value.
    pub fn field<A, C, H>(
        self,
        name: impl Into<Arc<str>>,
        getter: H,
        codec: Arc<C>,
    ) -> PreparedEffectRecordAdded<T, D, E, X, A, C, H, Fields>
    where
        A: Send + 'static,
        H: for<'a> Fn(&'a T) -> &'a A + Send + Sync + 'static,
    {
        PreparedEffectRecordBuilder {
            fields: PreparedEffectRecordField {
                name: name.into(),
                codec,
                getter,
                rest: Arc::new(self.fields),
                policy: Arc::new(Required),
                marker: std::marker::PhantomData,
            },
            marker: std::marker::PhantomData,
            environments: std::marker::PhantomData,
        }
    }

    /// Add an absent-value factory and a conditional omission rule.
    pub fn optional<A, C, H, F, N, O>(
        self,
        name: impl Into<Arc<str>>,
        getter: H,
        codec: Arc<C>,
        factory: F,
        omit: O,
        annotation: Option<Arc<serde_json::Value>>,
    ) -> PreparedOptionalRecord<T, D, E, X, A, C, H, Fields, F, N, O>
    where
        A: Send + 'static,
        H: for<'a> Fn(&'a T) -> &'a A + Send + Sync + 'static,
        F: Fn() -> N + Send + Sync + 'static,
        O: Fn(&A) -> bool + Send + Sync + 'static,
    {
        PreparedEffectRecordBuilder {
            fields: PreparedEffectRecordField {
                name: name.into(),
                codec,
                getter,
                rest: Arc::new(self.fields),
                policy: Arc::new(Optional {
                    factory,
                    omit,
                    annotation,
                    marker: std::marker::PhantomData,
                }),
                marker: std::marker::PhantomData,
            },
            marker: std::marker::PhantomData,
            environments: std::marker::PhantomData,
        }
    }

    /// Add a lazy absent default validated by the child's borrowed witness.
    pub fn default_with<A, C, H, F, N>(
        self,
        name: impl Into<Arc<str>>,
        getter: H,
        codec: Arc<C>,
        factory: F,
        annotation: Option<Arc<serde_json::Value>>,
    ) -> PreparedDefaultRecord<T, D, E, X, A, C, H, Fields, F, N>
    where
        A: Send + 'static,
        H: for<'a> Fn(&'a T) -> &'a A + Send + Sync + 'static,
        F: Fn() -> N + Send + Sync + 'static,
    {
        PreparedEffectRecordBuilder {
            fields: PreparedEffectRecordField {
                name: name.into(),
                codec,
                getter,
                rest: Arc::new(self.fields),
                policy: Arc::new(Defaulted {
                    factory,
                    validate: ChildDefaultValidator,
                    annotation,
                    marker: std::marker::PhantomData,
                }),
                marker: std::marker::PhantomData,
            },
            marker: std::marker::PhantomData,
            environments: std::marker::PhantomData,
        }
    }

    /// Add a lazy absent default with a context-aware borrowed validator.
    pub fn default_with_context<A, C, H, F, N, V>(
        self,
        name: impl Into<Arc<str>>,
        getter: H,
        codec: Arc<C>,
        factory: F,
        validate: V,
        annotation: Option<Arc<serde_json::Value>>,
    ) -> PreparedContextDefaultRecord<T, D, E, X, A, C, H, Fields, F, N, V>
    where
        A: Send + 'static,
        H: for<'a> Fn(&'a T) -> &'a A + Send + Sync + 'static,
        F: Fn() -> N + Send + Sync + 'static,
        V: Fn(&A, Option<&ParseContext>) -> Result<(), X> + Send + Sync + 'static,
    {
        PreparedEffectRecordBuilder {
            fields: PreparedEffectRecordField {
                name: name.into(),
                codec,
                getter,
                rest: Arc::new(self.fields),
                policy: Arc::new(Defaulted {
                    factory,
                    validate: ContextDefaultValidator(validate),
                    annotation,
                    marker: std::marker::PhantomData,
                }),
                marker: std::marker::PhantomData,
            },
            marker: std::marker::PhantomData,
            environments: std::marker::PhantomData,
        }
    }
}

impl<T, D, E, X> Default for PreparedEffectRecordBuilder<T, D, E, X> {
    fn default() -> Self {
        Self {
            fields: PreparedEffectRecordNil,
            marker: std::marker::PhantomData,
            environments: std::marker::PhantomData,
        }
    }
}
/// Builder after adding a field with a typed policy.
pub type PreparedEffectRecordWithPolicy<T, D, E, X, A, C, H, Fields, P> =
    PreparedEffectRecordBuilder<T, D, E, X, PreparedEffectRecordField<T, A, C, H, Fields, P>>;
/// Builder after adding an optional field.
pub type PreparedOptionalRecord<T, D, E, X, A, C, H, R, F, N, O> =
    PreparedEffectRecordWithPolicy<T, D, E, X, A, C, H, R, Optional<F, N, O>>;
/// Builder after adding a default validated by the child.
pub type PreparedDefaultRecord<T, D, E, X, A, C, H, R, F, N> =
    PreparedEffectRecordWithPolicy<T, D, E, X, A, C, H, R, Defaulted<F, N, ChildDefaultValidator>>;
/// Builder after adding a context-validated default.
pub type PreparedContextDefaultRecord<T, D, E, X, A, C, H, R, F, N, V> =
    PreparedEffectRecordWithPolicy<
        T,
        D,
        E,
        X,
        A,
        C,
        H,
        R,
        Defaulted<F, N, ContextDefaultValidator<V>>,
    >;
/// Builder after adding one required field.
pub type PreparedEffectRecordAdded<T, D, E, X, A, C, H, Fields> =
    PreparedEffectRecordBuilder<T, D, E, X, PreparedEffectRecordField<T, A, C, H, Fields>>;
/// Actual finite operation, output and diagnostic owners for required records.
pub struct PreparedEffectRecordResources<X: Send + 'static> {
    /// Region operation controls and per-pair builder indices.
    pub operations: Arc<PreparedRecordOperationPool<X>>,
    /// Managed output document/counting writer backing.
    pub output: Arc<super::PreparedJsonParserPool>,
    /// Actual issue paths and pair child arrays.
    pub issues: Arc<PreparedIssueMemory>,
    /// Managed representation and issue traversal scratch.
    pub validation: Arc<PreparedJsonValidationPool>,
}
/// Actual generic record operation, representation and diagnostic owners.
pub struct PreparedGenericRecordResources<X: Send + 'static, B: RecordBacking> {
    /// Region operations with the actual generic state layout.
    pub operations: Arc<PreparedRecordOperationPool<X, B>>,
    /// Explicit input and output representation backing.
    pub backing: Arc<B>,
    /// Diagnostic backing.
    pub issues: Arc<PreparedIssueMemory>,
    /// Issue traversal scratch.
    pub validation: Arc<PreparedJsonValidationPool>,
}
/// Concrete owned record codec. Field values remain in their statically typed graph frames.
pub struct PreparedEffectRecordCodec<
    T: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    Fields: PreparedEffectRecordFields<T, D, E, X, B>,
    K,
    S,
    B: RecordBacking = JsonRecordBacking,
> {
    fields: Arc<Fields>,
    constructor: Arc<K>,
    splitter: Arc<S>,
    errors: ErrorOwner<X>,
    resources: PreparedGenericRecordResources<X, B>,
    policy: ParsePolicy,
    ast: super::SchemaAst,
    names: Arc<[Arc<str>]>,
    emitted_names: Arc<[Arc<str>]>,
    marker: std::marker::PhantomData<fn() -> T>,
    environments: std::marker::PhantomData<fn() -> (D, E)>,
}
/// Result of finishing a typed required-field record.
pub type PreparedEffectRecordBuilt<T, D, E, X, Fields, K, S, B = JsonRecordBacking> =
    Result<PreparedEffectRecordCodec<T, D, E, X, Fields, K, S, B>, SchemaIssue>;
impl<T, D, E, X, Fields> PreparedEffectRecordBuilder<T, D, E, X, Fields>
where
    T: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    Fields: PreparedEffectRecordFields<T, D, E, X>,
{
    /// Finish a required-field definition with actual owners and same-root error classification.
    pub fn finish<K, S, F, G>(
        self,
        policy: ParsePolicy,
        resources: PreparedEffectRecordResources<X>,
        constructor: K,
        splitter: S,
        map: F,
        classify: G,
    ) -> PreparedEffectRecordBuilt<T, D, E, X, Fields, K, S>
    where
        K: Fn(Fields::Values) -> T + Send + Sync + 'static,
        S: Fn(T) -> Fields::Values + Send + Sync + 'static,
        F: Fn(SchemaIssue) -> X + Send + Sync + 'static,
        G: Fn(X) -> Result<SchemaIssue, X> + Send + Sync + 'static,
    {
        let backing = Arc::new(JsonRecordBacking {
            output: resources.output,
        });
        self.finish_in(
            policy,
            PreparedGenericRecordResources {
                operations: resources.operations,
                backing,
                issues: resources.issues,
                validation: resources.validation,
            },
            constructor,
            splitter,
            map,
            classify,
        )
    }
}
impl<T, D, E, X, Fields> PreparedEffectRecordBuilder<T, D, E, X, Fields>
where
    T: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
{
    /// Finish using caller-selected generic representation backing.
    pub fn finish_in<B, K, S, F, G>(
        self,
        policy: ParsePolicy,
        resources: PreparedGenericRecordResources<X, B>,
        constructor: K,
        splitter: S,
        map: F,
        classify: G,
    ) -> PreparedEffectRecordBuilt<T, D, E, X, Fields, K, S, B>
    where
        B: RecordBacking,
        Fields: PreparedEffectRecordFields<T, D, E, X, B>,
        K: Fn(<Fields as PreparedEffectRecordFields<T, D, E, X, B>>::Values) -> T
            + Send
            + Sync
            + 'static,
        S: Fn(T) -> <Fields as PreparedEffectRecordFields<T, D, E, X, B>>::Values
            + Send
            + Sync
            + 'static,
        F: Fn(SchemaIssue) -> X + Send + Sync + 'static,
        G: Fn(X) -> Result<SchemaIssue, X> + Send + Sync + 'static,
    {
        let mut properties = Vec::new();
        <Fields as PreparedEffectRecordFields<T, D, E, X, B>>::properties(
            &self.fields,
            &mut properties,
        )?;
        let names: Arc<[Arc<str>]> = properties
            .iter()
            .map(super::SchemaProperty::shared_name)
            .collect();
        let mut emitted_names = Vec::new();
        <Fields as PreparedEffectRecordFields<T, D, E, X, B>>::emitted_names(
            &self.fields,
            &mut emitted_names,
        );
        let emitted_names = emitted_names.into();
        let ast = super::SchemaAst::object(properties, policy.unknown_fields)?;
        ast.prepare(policy.limits)?;
        Ok(PreparedEffectRecordCodec {
            fields: Arc::new(self.fields),
            constructor: Arc::new(constructor),
            splitter: Arc::new(splitter),
            errors: Arc::new(SelectedPolicy { map, classify }),
            resources,
            policy,
            ast,
            names,
            emitted_names,
            marker: std::marker::PhantomData,
            environments: std::marker::PhantomData,
        })
    }
}
#[derive(Clone, Copy)]
enum Direction {
    Decode,
    Encode,
    Validate,
    Write,
}
impl<T, D, E, X, Fields, K, S, B: RecordBacking>
    PreparedEffectRecordCodec<T, D, E, X, Fields, K, S, B>
where
    T: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    Fields: PreparedEffectRecordFields<T, D, E, X, B>,
    K: Fn(Fields::Values) -> T + Send + Sync + 'static,
    S: Fn(T) -> Fields::Values + Send + Sync + 'static,
{
    fn admit(
        &self,
        input: Option<B::Encoded>,
        context: Option<ParseContext>,
        direction: Direction,
        writer: Option<JsonWriter>,
    ) -> Result<RecordHandle<X, B>, SchemaIssue> {
        let mut lease = self
            .resources
            .operations
            .reserve()
            .map_err(super::prepared_alternatives::memory)?;
        let mut policy = record_policy::<B::Encoded>(context.as_ref(), self.policy);
        policy.unknown_fields = self.policy.unknown_fields;
        if matches!(direction, Direction::Validate) && !self.fields.has_validator() {
            return Err(SchemaIssue::new(super::SchemaIssueKind::MissingValidator));
        }
        if matches!(direction, Direction::Write) && !self.fields.has_writer() {
            return Err(SchemaIssue::new(super::SchemaIssueKind::MissingWriter));
        }
        let builders = if policy.errors == super::SchemaErrorMode::Accumulate
            && !matches!(direction, Direction::Write)
        {
            self.names.len().saturating_sub(1)
        } else {
            0
        };
        lease
            .fill(self.resources.issues.child_pool(), builders)
            .map_err(super::prepared_alternatives::memory)?;
        let input = input
            .map(|input| {
                self.resources.backing.read(
                    input,
                    policy,
                    &self.names,
                    context.as_ref(),
                    &self.resources.issues,
                    &self.resources.validation,
                )
            })
            .transpose()?;
        let assembly = if matches!(direction, Direction::Encode) {
            Some(
                self.resources
                    .backing
                    .output(&self.emitted_names, self.names.len(), policy)?,
            )
        } else {
            None
        };
        let budget = Budget::new(context.as_ref(), policy);
        let state = RecordState {
            input,
            assembly,
            backing: Arc::clone(&self.resources.backing),
            writer,
            builders: None,
            context,
            policy,
            budget,
            _errors: Arc::clone(&self.errors),
            issues: Arc::clone(&self.resources.issues),
            validation: Arc::clone(&self.resources.validation),
        };
        lease
            .publish(state)
            .map_err(super::prepared_alternatives::memory)
    }
}
impl<T, D, E, X, Fields, K, S, B: RecordBacking> super::PreparedCodec<T, B::Encoded, D, E>
    for PreparedEffectRecordCodec<T, D, E, X, Fields, K, S, B>
where
    T: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    Fields: PreparedEffectRecordFields<T, D, E, X, B>,
    K: Fn(Fields::Values) -> T + Send + Sync + 'static,
    S: Fn(T) -> Fields::Values + Send + Sync + 'static,
{
    type Error = X;
    fn ast(&self) -> &super::SchemaAst {
        &self.ast
    }
    fn map_schema_issue(&self, issue: SchemaIssue) -> X {
        self.errors.map(issue)
    }
    fn has_validator(&self) -> bool {
        self.fields.has_validator()
    }
    fn has_default_validator(&self) -> bool {
        self.fields.has_default_validator()
    }
    fn has_writer(&self) -> bool {
        self.fields.has_writer()
    }
    fn validate_default_ref(&self, value: &T, context: Option<&ParseContext>) -> Result<(), X> {
        if !self.fields.has_default_validator() {
            return Err(self
                .errors
                .map(SchemaIssue::new(super::SchemaIssueKind::MissingValidator)));
        }
        if let Some(context) = context {
            context.visit().map_err(|issue| self.errors.map(issue))?;
        }
        self.fields
            .validate_borrowed(value, context, &self.resources.issues, self.errors.as_ref())
    }
    fn decode(
        self: Arc<Self>,
        input: B::Encoded,
        context: Option<ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<Value = T, Error = X, Env = D> {
        let build = Arc::clone(&self);
        let fields = Arc::clone(&self.fields);
        let errors = Arc::clone(&self.errors);
        let constructor = Arc::clone(&self.constructor);
        let final_errors = Arc::clone(&self.errors);
        crate::effect_prepare! { description; program: effect! {
            let handle=effect_try_sync! { once; body: move || build.admit(Some(input),context,Direction::Decode,None).map_err(|issue|build.errors.map(issue)), }?;
            let result=fields.decode(handle,errors)?;
            let values=effect_try_sync! { once; body: move || result.map_err(|issue|final_errors.map(issue)), }?;
            let value=effect_sync! { once; body: move || constructor(values), }?;
            value
        }, }
    }
    fn encode(
        self: Arc<Self>,
        value: T,
        context: Option<ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<Value = B::Encoded, Error = X, Env = E>
    {
        let build = Arc::clone(&self);
        let fields = Arc::clone(&self.fields);
        let prepare = Arc::clone(&self.fields);
        let prepare_context = context.clone();
        let prepare_issues = Arc::clone(&self.resources.issues);
        let errors = Arc::clone(&self.errors);
        let splitter = Arc::clone(&self.splitter);
        let final_errors = Arc::clone(&self.errors);
        let handle_errors = Arc::clone(&self.errors);
        crate::effect_prepare! { description; program: effect! {
            let (handle,field_handle)=effect_try_sync! { once; body: move || {
                let handle=build.admit(None,context,Direction::Encode,None).map_err(|issue|build.errors.map(issue))?;
                let field_handle=handle.try_clone().map_err(|error|handle_errors.map(super::prepared_alternatives::memory(error)))?;
                Ok((handle,field_handle))
            }, }?;
            let values=effect_sync! { once; body: move || { let values=splitter(value); prepare.prepare_values(values,prepare_context.as_ref(),&prepare_issues) }, }?;
            let result=fields.encode(values,field_handle,errors)?;
            let output=effect_try_sync! { once; body: move || {
                result.map_err(|issue|final_errors.map(issue))?;
                handle.with(|state| { let output=state.assembly.take().ok_or_else(invalid)?; state.backing.finish(output) }).map_err(|issue|final_errors.map(issue))
            }, }?;
            output
        }, }
    }
    fn validate(
        self: Arc<Self>,
        value: T,
        context: Option<ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<Value = T, Error = X, Env = E> {
        let build = Arc::clone(&self);
        let fields = Arc::clone(&self.fields);
        let prepare = Arc::clone(&self.fields);
        let prepare_context = context.clone();
        let prepare_issues = Arc::clone(&self.resources.issues);
        let errors = Arc::clone(&self.errors);
        let splitter = Arc::clone(&self.splitter);
        let constructor = Arc::clone(&self.constructor);
        let final_errors = Arc::clone(&self.errors);
        crate::effect_prepare! { description; program: effect! {
            let handle=effect_try_sync! { once; body: move || build.admit(None,context,Direction::Validate,None).map_err(|issue|build.errors.map(issue)), }?;
            let values=effect_sync! { once; body: move || { let values=splitter(value); prepare.prepare_values(values,prepare_context.as_ref(),&prepare_issues) }, }?;
            let result=fields.validate(values,handle,errors)?;
            let values=effect_try_sync! { once; body: move || result.map_err(|issue|final_errors.map(issue)), }?;
            let value=effect_sync! { once; body: move || constructor(values), }?;
            value
        }, }
    }
    fn write(
        self: Arc<Self>,
        value: T,
        writer: JsonWriter,
    ) -> impl crate::__private::prepared_graph::Description<Value = JsonWriter, Error = X, Env = E>
    {
        let build = Arc::clone(&self);
        let fields = Arc::clone(&self.fields);
        let errors = Arc::clone(&self.errors);
        let splitter = Arc::clone(&self.splitter);
        let final_errors = Arc::clone(&self.errors);
        let open_errors = Arc::clone(&self.errors);
        crate::effect_prepare! { description; program: effect! {
            let (handle,field_handle)=effect_try_sync! { once; body: move || {
                let handle=build.admit(None,None,Direction::Write,Some(writer)).map_err(|issue|build.errors.map(issue))?;
                let field_handle=handle.try_clone().map_err(|error|open_errors.map(super::prepared_alternatives::memory(error)))?;
                handle.with(|state|state.writer.as_mut().ok_or_else(invalid)?.begin_object()).map_err(|issue|open_errors.map(issue))?;
                Ok((handle,field_handle))
            }, }?;
            let values=effect_sync! { once; body: move || splitter(value), }?;
            let _done=fields.write(values,field_handle,errors)?;
            let writer=effect_try_sync! { once; body: move || handle.with(|state| {let mut writer=state.writer.take().ok_or_else(invalid)?;writer.end_object()?;Ok(writer)}).map_err(|issue|final_errors.map(issue)), }?;
            writer
        }, }
    }
}

fn record_policy<Encoded: 'static>(
    context: Option<&ParseContext>,
    fallback: ParsePolicy,
) -> ParsePolicy {
    context.map_or(fallback, |context| {
        let mut policy = context.policy().parse;
        if std::any::TypeId::of::<Encoded>()
            == std::any::TypeId::of::<super::SharedPreparedJsonValue>()
            || std::any::TypeId::of::<Encoded>() == std::any::TypeId::of::<super::JsonValue>()
        {
            policy.limits.depth = context.policy().json_depth;
        }
        policy
    })
}
