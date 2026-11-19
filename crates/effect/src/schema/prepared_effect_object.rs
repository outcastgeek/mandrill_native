//! Generic prepared object codec with actual representation owners.
use super::prepared_alternatives::Budget;
use super::prepared_object::{PreparedObjectCursor, PreparedObjectRepresentation};
use super::{
    JsonWriter, ParseContext, ParsePolicy, PreparedIssueMemory, PreparedJsonArray,
    PreparedJsonArrayBuilder, PreparedJsonString, PreparedJsonValidationPool, PreparedObjectEntry,
    PreparedObjectValue, SchemaIssue,
};
use crate::memory::{
    Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool,
};
use std::sync::{Arc, Mutex};

/// Physical simultaneous prepared-object operations.
#[derive(Clone, Copy, Debug)]
pub struct PreparedObjectOperationCapacity {
    /// Number of independently admitted run states.
    pub operations: usize,
}
impl PreparedObjectOperationCapacity {
    /// Exact region slot and state-record storage; owned child pools are separate.
    pub fn demand<A: Send + 'static, Encoded: Send + 'static, W: Send + 'static>(
        self,
    ) -> Result<Bytes, MemoryError> {
        SlotPool::<Mutex<Option<RunState<A, Encoded, W>>>>::storage_bytes(self.operations)
    }
}
struct RunState<A: Send + 'static, Encoded: Send + 'static, W: Send + 'static> {
    fields: Option<PreparedJsonArray<PreparedObjectEntry<Encoded>>>,
    name: Option<PreparedJsonString>,
    values: Option<PreparedJsonArray<PreparedObjectEntry<A>>>,
    assembly: Option<W>,
    writer: Option<JsonWriter>,
    errors: Option<PreparedJsonArrayBuilder<SchemaIssue>>,
    context: Option<ParseContext>,
    policy: ParsePolicy,
    budget: Budget,
    initial: usize,
    before: usize,
    index: usize,
    count: usize,
    stop: Option<SchemaIssue>,
    issues: Arc<PreparedIssueMemory>,
    validation: Arc<PreparedJsonValidationPool>,
}
type StateCell<A, Encoded, W> = Mutex<Option<RunState<A, Encoded, W>>>;
type StateSlot<A, Encoded, W> = SlotLease<'static, StateCell<A, Encoded, W>>;
type SharedStateSlot<A, Encoded, W> = SharedSlot<'static, StateCell<A, Encoded, W>>;
/// Region-backed operation controls for owned generic object directions.
pub struct PreparedObjectOperationPool<
    A: Send + 'static,
    Encoded: Send + 'static,
    W: Send + 'static,
> {
    slots: SlotPool<'static, Mutex<Option<RunState<A, Encoded, W>>>>,
}
impl<A: Send + 'static, Encoded: Send + 'static, W: Send + 'static>
    PreparedObjectOperationPool<A, Encoded, W>
{
    /// Prepare all operation records before descriptions are run.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedObjectOperationCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand::<A, Encoded, W>()?;
        Ok(Self {
            slots: memory.slots(region, capacity.operations, |_| Ok(Mutex::new(None)))?,
        })
    }
    fn reserve(&self) -> Result<StateSlot<A, Encoded, W>, MemoryError> {
        self.slots.checkout()
    }
    fn publish(
        slot: StateSlot<A, Encoded, W>,
        value: RunState<A, Encoded, W>,
    ) -> RunHandle<A, Encoded, W> {
        let old = {
            let mut guard = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.replace(value)
        };
        drop(old);
        RunHandle {
            slot: Some(slot.share()),
        }
    }
}
struct RunHandle<A: Send + 'static, Encoded: Send + 'static, W: Send + 'static> {
    slot: Option<SharedStateSlot<A, Encoded, W>>,
}
impl<A: Send + 'static, Encoded: Send + 'static, W: Send + 'static> RunHandle<A, Encoded, W> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        let slot = self.slot.as_ref().ok_or(MemoryError::Closed)?.try_clone()?;
        Ok(Self { slot: Some(slot) })
    }
    fn with<T>(
        &self,
        f: impl FnOnce(&mut RunState<A, Encoded, W>) -> Result<T, SchemaIssue>,
    ) -> Result<T, SchemaIssue> {
        let slot = self.slot.as_ref().ok_or_else(invalid)?;
        let mut state = {
            let mut guard = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.take().ok_or_else(invalid)?
        };
        let result = f(&mut state);
        {
            let mut guard = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard = Some(state);
        }
        result
    }
}
impl<A: Send + 'static, Encoded: Send + 'static, W: Send + 'static> RunHandle<A, Encoded, W> {
    fn prefix(&self, issue: SchemaIssue) -> SchemaIssue {
        let mut original = Some(issue);
        let decorated = self.with(|state| {
            let selected = state
                .context
                .as_ref()
                .and_then(ParseContext::issue_memory)
                .or(Some(&state.issues));
            let name = state.name.as_ref().ok_or_else(invalid)?.as_str();
            let text =
                super::schema_issue::format_diagnostic_text(selected, format_args!("{name}"))?;
            let issue = original.take().ok_or_else(invalid)?;
            Ok(super::parse_context::selected_issue_at(
                selected,
                issue,
                super::IssueSegment::PreparedField(text),
            ))
        });
        match decorated {
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
impl<A: Send + 'static, Encoded: Send + 'static, W: Send + 'static> Drop
    for RunHandle<A, Encoded, W>
{
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take()
            && let Some(slot) = slot.into_last()
        {
            let value = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(value);
            drop(slot);
        }
    }
}
type Seeds<A, Encoded, W> = (
    RunHandle<A, Encoded, W>,
    RunHandle<A, Encoded, W>,
    RunHandle<A, Encoded, W>,
);
fn seeds<A: Send + 'static, Encoded: Send + 'static, W: Send + 'static>(
    owner: RunHandle<A, Encoded, W>,
) -> Result<Seeds<A, Encoded, W>, SchemaIssue> {
    let guard = owner
        .try_clone()
        .map_err(super::prepared_alternatives::memory)?;
    let body = owner
        .try_clone()
        .map_err(super::prepared_alternatives::memory)?;
    Ok((owner, guard, body))
}
fn invalid() -> SchemaIssue {
    SchemaIssue::new(super::SchemaIssueKind::InvalidDefinition)
}

impl<A: Send + 'static, Encoded: Send + 'static, W: Send + 'static> RunState<A, Encoded, W> {
    fn child_context(&self) -> Result<Option<ParseContext>, SchemaIssue> {
        let Some(context) = self.context.as_ref() else {
            return Ok(None);
        };
        let name = self.name.as_ref().ok_or_else(invalid)?.as_str();
        let memory = context.issue_memory().or(Some(&self.issues));
        let text = super::schema_issue::format_diagnostic_text(memory, format_args!("{name}"))?;
        context
            .child(super::IssueSegment::PreparedField(text))
            .map(Some)
    }

    fn more(&self) -> bool {
        self.stop.is_none() && self.index < self.count
    }
    fn begin_child(&mut self) -> Result<usize, SchemaIssue> {
        if self.policy.errors != super::SchemaErrorMode::First {
            self.budget.available()?;
        }
        self.before = self.budget.used();
        Ok(self.index)
    }
    fn record_failure(&mut self, issue: SchemaIssue) -> Result<(), SchemaIssue> {
        if self.policy.errors == super::SchemaErrorMode::First
            || super::prepared_alternatives::root_resource(&issue)
        {
            self.stop = Some(issue);
        } else {
            let scan = self.validation.scan_issue(&issue)?;
            if scan.resource {
                self.stop = Some(issue);
            } else {
                self.budget.record(self.before, scan.count)?;
                self.errors
                    .as_mut()
                    .ok_or_else(invalid)?
                    .try_push(issue)
                    .map_err(|_| super::prepared_alternatives::memory(MemoryError::NoSlot))?;
            }
        }
        self.index += 1;
        Ok(())
    }
    fn finish_errors(&mut self) -> Result<(), SchemaIssue> {
        if let Some(issue) = self.stop.take() {
            return Err(issue);
        }
        if let Some(mut errors) = self.errors.take()
            && !errors.is_empty()
        {
            let issue = if errors.len() == 1 {
                errors.remove_first().ok_or_else(invalid)?
            } else {
                SchemaIssue::new(super::SchemaIssueKind::Multiple)
                    .with_prepared_children(errors.publish())
            };
            let count = self.validation.count_issue(&issue)?;
            self.budget.record(self.initial, count)?;
            return Err(issue);
        }
        Ok(())
    }
}

/// Actual input/result arrays, operation records and diagnostics for one object codec.
pub struct PreparedObjectCodecResources<
    A: Send + 'static,
    Encoded: Send + 'static,
    W: Send + 'static,
> {
    /// Result entries retain original names and typed values.
    pub entries: Arc<super::PreparedJsonArrayPool<PreparedObjectEntry<A>>>,
    /// Input entries retain original names and owned encoded values.
    pub fields: Arc<super::PreparedJsonArrayPool<PreparedObjectEntry<Encoded>>>,
    /// Region operation records containing actual output write owners.
    pub operations: Arc<PreparedObjectOperationPool<A, Encoded, W>>,
    /// Prepared field text and issue trees.
    pub issues: Arc<PreparedIssueMemory>,
    /// Prepared issue traversal scratch.
    pub validation: Arc<PreparedJsonValidationPool>,
}
/// Owned effectful object codec over a caller-selected prepared representation.
pub struct PreparedObjectCodec<
    C,
    R,
    A: Send + 'static,
    Encoded: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    F = fn(SchemaIssue) -> SchemaIssue,
    G = fn(SchemaIssue) -> Result<SchemaIssue, SchemaIssue>,
> where
    R: PreparedObjectRepresentation<Encoded>,
{
    codec: Arc<C>,
    representation: Arc<R>,
    resources: PreparedObjectCodecResources<A, Encoded, R::WriteAdmission>,
    policy: ParsePolicy,
    has_validator: bool,
    has_default_validator: bool,
    has_writer: bool,
    ast: super::SchemaAst,
    map: F,
    classify: G,
    encoded: std::marker::PhantomData<fn() -> Encoded>,
    environments: std::marker::PhantomData<fn() -> (D, E)>,
}
impl<C, R, A, Encoded, D, E> PreparedObjectCodec<C, R, A, Encoded, D, E>
where
    A: Send + 'static,
    Encoded: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    R: PreparedObjectRepresentation<Encoded>,
    C: super::PreparedCodec<A, Encoded, D, E, Error = SchemaIssue>,
{
    /// Select actual representation owners and the SchemaIssue family.
    pub fn new(
        codec: Arc<C>,
        representation: Arc<R>,
        policy: ParsePolicy,
        resources: PreparedObjectCodecResources<A, Encoded, R::WriteAdmission>,
    ) -> Self {
        Self::new_mapped(
            codec,
            representation,
            policy,
            resources,
            std::convert::identity::<SchemaIssue>,
            Ok::<SchemaIssue, SchemaIssue>,
        )
    }
}
impl<C, R, A, Encoded, D, E, F, G> PreparedObjectCodec<C, R, A, Encoded, D, E, F, G>
where
    A: Send + 'static,
    Encoded: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    R: PreparedObjectRepresentation<Encoded>,
{
    /// Select actual owners and explicit schema-error classification.
    pub fn new_mapped<X>(
        codec: Arc<C>,
        representation: Arc<R>,
        policy: ParsePolicy,
        resources: PreparedObjectCodecResources<A, Encoded, R::WriteAdmission>,
        map: F,
        classify: G,
    ) -> Self
    where
        X: Send + 'static,
        C: super::PreparedCodec<A, Encoded, D, E, Error = X>,
        F: Fn(SchemaIssue) -> X + Send + Sync + 'static,
        G: Fn(X) -> Result<SchemaIssue, X> + Send + Sync + 'static,
    {
        let has_validator = codec.has_validator();
        let has_default_validator = codec.has_default_validator();
        let has_writer = codec.has_writer();
        Self {
            codec,
            representation,
            resources,
            policy,
            has_validator,
            has_default_validator,
            has_writer,
            ast: super::SchemaAst::opaque(),
            map,
            classify,
            encoded: std::marker::PhantomData,
            environments: std::marker::PhantomData,
        }
    }
    fn admit_decode(
        &self,
        slot: StateSlot<A, Encoded, R::WriteAdmission>,
        input: Encoded,
        context: Option<ParseContext>,
    ) -> Result<RunHandle<A, Encoded, R::WriteAdmission>, SchemaIssue> {
        let policy = object_policy::<Encoded>(context.as_ref(), self.policy);
        let values = self
            .resources
            .entries
            .reserve()
            .map_err(super::prepared_alternatives::memory)?;
        let mut fields = self
            .resources
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
        if values.capacity() < count || fields.capacity() < count {
            return Err(super::prepared_alternatives::memory(MemoryError::NoSlot));
        }
        let errors = if count == 0 || policy.errors == super::SchemaErrorMode::First {
            None
        } else {
            Some(super::prepared_alternatives::error_array(
                &self.resources.issues,
                count.min(policy.limits.issues.get()),
            )?)
        };
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
                let memory = context
                    .as_ref()
                    .and_then(ParseContext::issue_memory)
                    .or(Some(&self.resources.issues));
                return Err(super::schema_issue::prepared_duplicate_field(
                    memory,
                    name.as_str(),
                ));
            }
            fields
                .push(PreparedObjectEntry::new(name, value))
                .map_err(|_| super::prepared_alternatives::memory(MemoryError::NoSlot))?;
        }
        if cursor.remaining() != 0 {
            return Err(invalid());
        }
        drop(cursor);
        let budget = Budget::new(context.as_ref(), policy);
        let initial = budget.used();
        Ok(PreparedObjectOperationPool::publish(
            slot,
            RunState {
                fields: Some(fields),
                name: None,
                values: Some(values),
                assembly: None,
                writer: None,
                errors,
                context,
                policy,
                budget,
                initial,
                before: 0,
                index: 0,
                count,
                stop: None,
                issues: Arc::clone(&self.resources.issues),
                validation: Arc::clone(&self.resources.validation),
            },
        ))
    }
    fn admit_values(
        &self,
        slot: StateSlot<A, Encoded, R::WriteAdmission>,
        values: PreparedObjectValue<A>,
        context: Option<ParseContext>,
        direction: Direction,
        writer: Option<JsonWriter>,
    ) -> Result<RunHandle<A, Encoded, R::WriteAdmission>, SchemaIssue> {
        if matches!(direction, Direction::Validate) && !self.has_validator {
            return Err(SchemaIssue::new(super::SchemaIssueKind::MissingValidator));
        }
        if matches!(direction, Direction::Write) && !self.has_writer {
            return Err(SchemaIssue::new(super::SchemaIssueKind::MissingWriter));
        }
        let policy = object_policy::<Encoded>(context.as_ref(), self.policy);
        let count = values.entries().len();
        if count > policy.limits.collection.get() {
            return Err(SchemaIssue::new(super::SchemaIssueKind::LimitExceeded {
                resource: super::SchemaResource::Width,
            }));
        }
        let assembly = if matches!(direction, Direction::Encode) {
            Some(self.representation.admit_write(
                values.entries().iter().map(PreparedObjectEntry::name),
                policy,
            )?)
        } else {
            None
        };
        let errors = if count == 0
            || matches!(direction, Direction::Write)
            || policy.errors == super::SchemaErrorMode::First
        {
            None
        } else {
            Some(super::prepared_alternatives::error_array(
                &self.resources.issues,
                count.min(policy.limits.issues.get()),
            )?)
        };
        let budget = Budget::new(context.as_ref(), policy);
        let initial = budget.used();
        Ok(PreparedObjectOperationPool::publish(
            slot,
            RunState {
                fields: None,
                name: None,
                values: Some(values.into_entries()),
                assembly,
                writer,
                errors,
                context,
                policy,
                budget,
                initial,
                before: 0,
                index: 0,
                count,
                stop: None,
                issues: Arc::clone(&self.resources.issues),
                validation: Arc::clone(&self.resources.validation),
            },
        ))
    }
}
#[derive(Clone, Copy)]
enum Direction {
    Encode,
    Validate,
    Write,
}
fn object_policy<Encoded: 'static>(
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
impl<C, R, A, Encoded, D, E, F, G, X> super::PreparedCodec<PreparedObjectValue<A>, Encoded, D, E>
    for PreparedObjectCodec<C, R, A, Encoded, D, E, F, G>
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    Encoded: Send + 'static,
    R: PreparedObjectRepresentation<Encoded>,
    X: Send + 'static,
    F: Fn(SchemaIssue) -> X + Send + Sync + 'static,
    G: Fn(X) -> Result<SchemaIssue, X> + Send + Sync + 'static,
    C: super::PreparedCodec<A, Encoded, D, E, Error = X>,
{
    type Error = X;
    fn ast(&self) -> &super::SchemaAst {
        &self.ast
    }
    fn map_schema_issue(&self, issue: SchemaIssue) -> X {
        (self.map)(issue)
    }
    fn has_validator(&self) -> bool {
        self.has_validator
    }
    fn has_default_validator(&self) -> bool {
        self.has_default_validator
    }
    fn has_writer(&self) -> bool {
        self.has_writer
    }
    fn validate_default_ref(
        &self,
        values: &PreparedObjectValue<A>,
        context: Option<&ParseContext>,
    ) -> Result<(), X> {
        if !self.has_default_validator {
            return Err((self.map)(SchemaIssue::new(
                super::SchemaIssueKind::MissingValidator,
            )));
        }
        let policy = object_policy::<Encoded>(context, self.policy);
        if values.entries().len() > policy.limits.collection.get() {
            return Err((self.map)(SchemaIssue::new(
                super::SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                },
            )));
        }
        if let Some(context) = context {
            context.visit().map_err(&self.map)?;
        }
        for entry in values.entries() {
            let name = entry.name();
            let selected = context
                .and_then(ParseContext::issue_memory)
                .or(Some(&self.resources.issues));
            let segment = || {
                super::schema_issue::format_diagnostic_text(selected, format_args!("{name}"))
                    .map(super::IssueSegment::PreparedField)
            };
            let decorate = |issue: SchemaIssue| match segment() {
                Ok(segment) => super::parse_context::selected_issue_at(selected, issue, segment),
                Err(decoration) => {
                    if super::prepared_json_validation::physical_cause(&issue).is_some() {
                        issue
                    } else {
                        decoration
                    }
                }
            };
            let child = match context {
                Some(context) => Some(
                    context
                        .child(segment().map_err(&self.map)?)
                        .map_err(|issue| (self.map)(decorate(issue)))?,
                ),
                None => None,
            };
            if let Err(error) = self
                .codec
                .validate_default_ref(entry.value(), child.as_ref())
            {
                return Err(match (self.classify)(error) {
                    Ok(issue) => (self.map)(decorate(issue)),
                    Err(error) => error,
                });
            }
        }
        Ok(())
    }
    fn decode(
        self: Arc<Self>,
        input: Encoded,
        context: Option<ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = PreparedObjectValue<A>,
        Error = X,
        Env = D,
    > {
        let build = Arc::clone(&self);
        let codec = Arc::clone(&self.codec);
        let body_codec = Arc::clone(&self);
        let capacity_codec = Arc::clone(&self);
        let final_codec = Arc::clone(&self);
        crate::effect_prepare! { description; program: effect! {
            let (handle,guard_seed,body_seed)=effect_try_sync! { once; body: move || {
                (|| -> Result<_,SchemaIssue> { let slot=build.resources.operations.reserve().map_err(super::prepared_alternatives::memory)?; seeds(build.admit_decode(slot,input,context)?) })().map_err(|issue| (build.map)(issue))
            }, }?;
            let _loop=effect! { while_loop;
                guard: { let selected=guard_seed; move || selected.with(|s| Ok(s.more())).unwrap_or(true) },
                body: { let selected=body_seed; let codec=Arc::clone(&codec); let body_codec=Arc::clone(&body_codec); move || {
                    let child_handle=selected.try_clone(); let codec=Arc::clone(&codec); let prepare=Arc::clone(&body_codec); let work_codec=Arc::clone(&body_codec); let mapper_codec=Arc::clone(&body_codec); let complete=Arc::clone(&body_codec); let capture_codec=Arc::clone(&body_codec);
                    effect! {
                        let (owner,work,mapper,value,_writer,capture)=effect_try_sync! { once; body: move || {
                            (|| -> Result<_,SchemaIssue> {
                                let owner=child_handle.map_err(super::prepared_alternatives::memory)?;
                                let capture=owner.with(|s| { let _index=s.begin_child()?; Ok(s.policy.errors==super::SchemaErrorMode::Accumulate) })?;
                                let work=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                let mapper=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                let (value,writer)=owner.with(|s| { let (name,value)=s.fields.as_mut().ok_or_else(invalid)?.remove_first().ok_or_else(invalid)?.into_parts();  s.name=Some(name); Ok((value,None::<JsonWriter>)) })?;
                                Ok((owner,work,mapper,value,writer,capture))
                            })().map_err(|issue| (prepare.map)(issue))
                        }, }?;
                        let result=crate::effect_prepare! { description; program: effect_map_error! {
                            program: effect! { let context=effect_try_sync! { once; body: move || work.with(|s| s.child_context()).map_err(|issue| (work_codec.map)(issue)), }?;
                                let value=<C as super::PreparedCodec<A,Encoded,D,E>>::decode(codec,value,context)?; value },
                            map: move |error| match (mapper_codec.classify)(error) { Ok(issue)=>(mapper_codec.map)(mapper.prefix(issue)),Err(error)=>error },
                        }, capture_errors: capture, classify_error: move |error| (capture_codec.classify)(error), }?;
                        let _done=effect_try_sync! { once; body: move || match result {
                            Ok(value)=>owner.with(|s| { let name=s.name.take().ok_or_else(invalid)?; s.values.as_mut().ok_or_else(invalid)?.push(PreparedObjectEntry::new(name,value)).map_err(|_| super::prepared_alternatives::memory(MemoryError::NoSlot))?; s.index+=1; Ok(()) }).map_err(|issue| (complete.map)(issue)),
                            Err(issue) => owner.with(|s| { drop(s.name.take()); s.record_failure(issue) }).map_err(|issue| (complete.map)(issue)),
                        }, }?;
                        ()
                    }
                } },
                on_capacity: move |error| (capacity_codec.map)(super::prepared_alternatives::memory(error)),
            }?;
            let result=effect_try_sync! { once; body: move || handle.with(|s| { s.finish_errors()?; Ok(PreparedObjectValue::from_entries(s.values.take().ok_or_else(invalid)?)) }).map_err(|issue| (final_codec.map)(issue)), }?;
            result
        }, }
    }
    fn encode(
        self: Arc<Self>,
        values: PreparedObjectValue<A>,
        context: Option<ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<Value = Encoded, Error = X, Env = E>
    {
        let build = Arc::clone(&self);
        let codec = Arc::clone(&self.codec);
        let body_codec = Arc::clone(&self);
        let capacity_codec = Arc::clone(&self);
        let final_codec = Arc::clone(&self);
        crate::effect_prepare! { description; program: effect! {
            let (handle,guard_seed,body_seed)=effect_try_sync! { once; body: move || {
                (|| -> Result<_,SchemaIssue> { let slot=build.resources.operations.reserve().map_err(super::prepared_alternatives::memory)?; seeds(build.admit_values(slot,values,context,Direction::Encode,None)?) })().map_err(|issue| (build.map)(issue))
            }, }?;
            let _loop=effect! { while_loop;
                guard: { let selected=guard_seed; move || selected.with(|s| Ok(s.more())).unwrap_or(true) },
                body: { let selected=body_seed; let codec=Arc::clone(&codec); let body_codec=Arc::clone(&body_codec); move || {
                    let child_handle=selected.try_clone(); let codec=Arc::clone(&codec); let prepare=Arc::clone(&body_codec); let work_codec=Arc::clone(&body_codec); let mapper_codec=Arc::clone(&body_codec); let complete=Arc::clone(&body_codec); let capture_codec=Arc::clone(&body_codec);
                    effect! {
                        let (owner,work,mapper,value,_writer,capture)=effect_try_sync! { once; body: move || {
                            (|| -> Result<_,SchemaIssue> {
                                let owner=child_handle.map_err(super::prepared_alternatives::memory)?;
                                let capture=owner.with(|s| { let _index=s.begin_child()?; Ok(s.policy.errors==super::SchemaErrorMode::Accumulate) })?;
                                let work=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                let mapper=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                let (value,writer)=owner.with(|s| { let (name,value)=s.values.as_mut().ok_or_else(invalid)?.remove_first().ok_or_else(invalid)?.into_parts(); prepare.representation.preflight(s.assembly.as_ref().ok_or_else(invalid)?,name.as_str())?; s.name=Some(name); Ok((value,None::<JsonWriter>)) })?;
                                Ok((owner,work,mapper,value,writer,capture))
                            })().map_err(|issue| (prepare.map)(issue))
                        }, }?;
                        let result=crate::effect_prepare! { description; program: effect_map_error! {
                            program: effect! { let context=effect_try_sync! { once; body: move || work.with(|s| s.child_context()).map_err(|issue| (work_codec.map)(issue)), }?;
                                let value=<C as super::PreparedCodec<A,Encoded,D,E>>::encode(codec,value,context)?; value },
                            map: move |error| match (mapper_codec.classify)(error) { Ok(issue)=>(mapper_codec.map)(mapper.prefix(issue)),Err(error)=>error },
                        }, capture_errors: capture, classify_error: move |error| (capture_codec.classify)(error), }?;
                        let _done=effect_try_sync! { once; body: move || match result {
                            Ok(value)=>owner.with(|s| { let name=s.name.take().ok_or_else(invalid)?; complete.representation.push(s.assembly.as_mut().ok_or_else(invalid)?,name.as_str(),value)?; s.index+=1; Ok(()) }).map_err(|issue| (complete.map)(issue)),
                            Err(issue) => owner.with(|s| { drop(s.name.take()); s.record_failure(issue) }).map_err(|issue| (complete.map)(issue)),
                        }, }?;
                        ()
                    }
                } },
                on_capacity: move |error| (capacity_codec.map)(super::prepared_alternatives::memory(error)),
            }?;
            let result=effect_try_sync! { once; body: move || handle.with(|s| { s.finish_errors()?; final_codec.representation.finish(s.assembly.take().ok_or_else(invalid)?) }).map_err(|issue| (final_codec.map)(issue)), }?;
            result
        }, }
    }
    fn validate(
        self: Arc<Self>,
        values: PreparedObjectValue<A>,
        context: Option<ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = PreparedObjectValue<A>,
        Error = X,
        Env = E,
    > {
        let build = Arc::clone(&self);
        let codec = Arc::clone(&self.codec);
        let body_codec = Arc::clone(&self);
        let capacity_codec = Arc::clone(&self);
        let final_codec = Arc::clone(&self);
        crate::effect_prepare! { description; program: effect! {
            let (handle,guard_seed,body_seed)=effect_try_sync! { once; body: move || {
                (|| -> Result<_,SchemaIssue> { let slot=build.resources.operations.reserve().map_err(super::prepared_alternatives::memory)?; seeds(build.admit_values(slot,values,context,Direction::Validate,None)?) })().map_err(|issue| (build.map)(issue))
            }, }?;
            let _loop=effect! { while_loop;
                guard: { let selected=guard_seed; move || selected.with(|s| Ok(s.more())).unwrap_or(true) },
                body: { let selected=body_seed; let codec=Arc::clone(&codec); let body_codec=Arc::clone(&body_codec); move || {
                    let child_handle=selected.try_clone(); let codec=Arc::clone(&codec); let prepare=Arc::clone(&body_codec); let work_codec=Arc::clone(&body_codec); let mapper_codec=Arc::clone(&body_codec); let complete=Arc::clone(&body_codec); let capture_codec=Arc::clone(&body_codec);
                    effect! {
                        let (owner,work,mapper,value,_writer,capture)=effect_try_sync! { once; body: move || {
                            (|| -> Result<_,SchemaIssue> {
                                let owner=child_handle.map_err(super::prepared_alternatives::memory)?;
                                let capture=owner.with(|s| { let _index=s.begin_child()?; Ok(s.policy.errors==super::SchemaErrorMode::Accumulate) })?;
                                let work=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                let mapper=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                let (value,writer)=owner.with(|s| { let (name,value)=s.values.as_mut().ok_or_else(invalid)?.remove_first().ok_or_else(invalid)?.into_parts();  s.name=Some(name); Ok((value,None::<JsonWriter>)) })?;
                                Ok((owner,work,mapper,value,writer,capture))
                            })().map_err(|issue| (prepare.map)(issue))
                        }, }?;
                        let result=crate::effect_prepare! { description; program: effect_map_error! {
                            program: effect! { let context=effect_try_sync! { once; body: move || work.with(|s| s.child_context()).map_err(|issue| (work_codec.map)(issue)), }?;
                                let value=<C as super::PreparedCodec<A,Encoded,D,E>>::validate(codec,value,context)?; value },
                            map: move |error| match (mapper_codec.classify)(error) { Ok(issue)=>(mapper_codec.map)(mapper.prefix(issue)),Err(error)=>error },
                        }, capture_errors: capture, classify_error: move |error| (capture_codec.classify)(error), }?;
                        let _done=effect_try_sync! { once; body: move || match result {
                            Ok(value)=>owner.with(|s| { let name=s.name.take().ok_or_else(invalid)?; s.values.as_mut().ok_or_else(invalid)?.push(PreparedObjectEntry::new(name,value)).map_err(|_| super::prepared_alternatives::memory(MemoryError::NoSlot))?; s.index+=1; Ok(()) }).map_err(|issue| (complete.map)(issue)),
                            Err(issue) => owner.with(|s| { drop(s.name.take()); s.record_failure(issue) }).map_err(|issue| (complete.map)(issue)),
                        }, }?;
                        ()
                    }
                } },
                on_capacity: move |error| (capacity_codec.map)(super::prepared_alternatives::memory(error)),
            }?;
            let result=effect_try_sync! { once; body: move || handle.with(|s| { s.finish_errors()?; Ok(PreparedObjectValue::from_entries(s.values.take().ok_or_else(invalid)?)) }).map_err(|issue| (final_codec.map)(issue)), }?;
            result
        }, }
    }
    fn write(
        self: Arc<Self>,
        values: PreparedObjectValue<A>,
        writer: JsonWriter,
    ) -> impl crate::__private::prepared_graph::Description<Value = JsonWriter, Error = X, Env = E>
    {
        let build = Arc::clone(&self);
        let codec = Arc::clone(&self.codec);
        let body_codec = Arc::clone(&self);
        let capacity_codec = Arc::clone(&self);
        let final_codec = Arc::clone(&self);
        crate::effect_prepare! { description; program: effect! {
            let (handle,guard_seed,body_seed)=effect_try_sync! { once; body: move || {
                (|| -> Result<_,SchemaIssue> { let slot=build.resources.operations.reserve().map_err(super::prepared_alternatives::memory)?; { let owners=seeds(build.admit_values(slot,values,None,Direction::Write,Some(writer))?)?; owners.0.with(|s| s.writer.as_mut().ok_or_else(invalid)?.begin_object())?; Ok(owners) } })().map_err(|issue| (build.map)(issue))
            }, }?;
            let _loop=effect! { while_loop;
                guard: { let selected=guard_seed; move || selected.with(|s| Ok(s.more())).unwrap_or(true) },
                body: { let selected=body_seed; let codec=Arc::clone(&codec); let body_codec=Arc::clone(&body_codec); move || {
                    let child_handle=selected.try_clone(); let codec=Arc::clone(&codec); let prepare=Arc::clone(&body_codec); let mapper_codec=Arc::clone(&body_codec); let complete=Arc::clone(&body_codec); let capture_codec=Arc::clone(&body_codec);
                    effect! {
                        let (owner,mapper,value,writer,capture)=effect_try_sync! { once; body: move || {
                            (|| -> Result<_,SchemaIssue> {
                                let owner=child_handle.map_err(super::prepared_alternatives::memory)?;
                                owner.with(|s| { let _index=s.index; Ok(()) })?; let capture=false;
                                let mapper=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                let (value,writer)=owner.with(|s| { let (name,value)=s.values.as_mut().ok_or_else(invalid)?.remove_first().ok_or_else(invalid)?.into_parts();  let mut writer=s.writer.take().ok_or_else(invalid)?; writer.property(name.as_str())?; s.name=Some(name); Ok((value,writer)) })?;
                                Ok((owner,mapper,value,writer,capture))
                            })().map_err(|issue| (prepare.map)(issue))
                        }, }?;
                        let result=crate::effect_prepare! { description; program: effect_map_error! {
                            program: effect! {
                                let value=<C as super::PreparedCodec<A,Encoded,D,E>>::write(codec,value,writer)?; value },
                            map: move |error| match (mapper_codec.classify)(error) { Ok(issue)=>(mapper_codec.map)(mapper.prefix(issue)),Err(error)=>error },
                        }, capture_errors: capture, classify_error: move |error| (capture_codec.classify)(error), }?;
                        let _done=effect_try_sync! { once; body: move || match result {
                            Ok(value)=>owner.with(|s| { drop(s.name.take()); s.writer=Some(value); s.index+=1; Ok(()) }).map_err(|issue| (complete.map)(issue)),
                            Err(issue)=>Err((complete.map)(issue)),
                        }, }?;
                        ()
                    }
                } },
                on_capacity: move |error| (capacity_codec.map)(super::prepared_alternatives::memory(error)),
            }?;
            let result=effect_try_sync! { once; body: move || handle.with(|s| { s.finish_errors()?; { let mut writer=s.writer.take().ok_or_else(invalid)?; writer.end_object()?; Ok(writer) } }).map_err(|issue| (final_codec.map)(issue)), }?;
            result
        }, }
    }
}
