//! Owned state for managed effectful sequence directions.
use super::internal::parser::PreparedArrayAssembly;
use super::prepared_alternatives::Budget;
use super::{
    JsonWriter, ParseContext, ParsePolicy, PreparedIssueMemory, PreparedJsonArray,
    PreparedJsonArrayBuilder, PreparedJsonValidationPool, SchemaIssue, SharedPreparedJsonValue,
};
use crate::memory::{
    Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool,
};
use std::sync::{Arc, Mutex};

/// Physical simultaneous effectful-array operations.
#[derive(Clone, Copy, Debug)]
pub struct PreparedArrayOperationCapacity {
    /// Number of independently admitted run states.
    pub operations: usize,
}
impl PreparedArrayOperationCapacity {
    /// Exact region slot and state-record storage; owned child pools are separate.
    pub fn demand<A: Send + 'static>(self) -> Result<Bytes, MemoryError> {
        SlotPool::<Mutex<Option<RunState<A>>>>::storage_bytes(self.operations)
    }
}
struct RunState<A: Send + 'static> {
    input: Option<SharedPreparedJsonValue>,
    values: Option<PreparedJsonArray<A>>,
    assembly: Option<PreparedArrayAssembly>,
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
/// Region-backed operation controls for owned managed array directions.
pub struct PreparedArrayOperationPool<A: Send + 'static> {
    slots: SlotPool<'static, Mutex<Option<RunState<A>>>>,
}
impl<A: Send + 'static> PreparedArrayOperationPool<A> {
    /// Prepare all operation records before descriptions are run.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedArrayOperationCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand::<A>()?;
        Ok(Self {
            slots: memory.slots(region, capacity.operations, |_| Ok(Mutex::new(None)))?,
        })
    }
    fn reserve(&self) -> Result<SlotLease<'static, Mutex<Option<RunState<A>>>>, MemoryError> {
        self.slots.checkout()
    }
    fn publish(
        slot: SlotLease<'static, Mutex<Option<RunState<A>>>>,
        value: RunState<A>,
    ) -> RunHandle<A> {
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
struct RunHandle<A: Send + 'static> {
    slot: Option<SharedSlot<'static, Mutex<Option<RunState<A>>>>>,
}
impl<A: Send + 'static> RunHandle<A> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        let slot = self.slot.as_ref().ok_or(MemoryError::Closed)?.try_clone()?;
        Ok(Self { slot: Some(slot) })
    }
    fn with<T>(
        &self,
        f: impl FnOnce(&mut RunState<A>) -> Result<T, SchemaIssue>,
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
impl<A: Send + 'static> RunHandle<A> {
    fn prefix(&self, issue: SchemaIssue, index: usize) -> SchemaIssue {
        let mut issue = Some(issue);
        self.with(|state| {
            let selected = state
                .context
                .as_ref()
                .and_then(ParseContext::issue_memory)
                .or(Some(&state.issues));
            Ok(super::parse_context::selected_issue_at(
                selected,
                issue.take().ok_or_else(invalid)?,
                super::IssueSegment::Index(index),
            ))
        })
        .unwrap_or_else(|error| error)
    }
}
impl<A: Send + 'static> Drop for RunHandle<A> {
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
type Seeds<A> = (RunHandle<A>, RunHandle<A>, RunHandle<A>);
fn seeds<A: Send + 'static>(owner: RunHandle<A>) -> Result<Seeds<A>, SchemaIssue> {
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

impl<A: Send + 'static> RunState<A> {
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

/// Actual owners selected for managed effectful array operations.
pub struct PreparedJsonArrayCodecResources<A: Send + 'static> {
    /// Typed output arrays and their retained frames.
    pub arrays: Arc<super::PreparedJsonArrayPool<A>>,
    /// Managed output document and counting writer backing.
    pub output: Arc<super::PreparedJsonParserPool>,
    /// Finite operation-state controls.
    pub operations: Arc<PreparedArrayOperationPool<A>>,
    /// Issue paths and child-array backing.
    pub issues: Arc<PreparedIssueMemory>,
    /// Representation and issue traversal scratch.
    pub validation: Arc<PreparedJsonValidationPool>,
}
/// Managed array directions using the genuine owned child codec.
pub struct PreparedJsonArrayCodec<
    C,
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    F = fn(SchemaIssue) -> SchemaIssue,
    G = fn(SchemaIssue) -> Result<SchemaIssue, SchemaIssue>,
> {
    codec: Arc<C>,
    arrays: Arc<super::PreparedJsonArrayPool<A>>,
    output: Arc<super::PreparedJsonParserPool>,
    operations: Arc<PreparedArrayOperationPool<A>>,
    issues: Arc<PreparedIssueMemory>,
    validation: Arc<PreparedJsonValidationPool>,
    policy: ParsePolicy,
    maximum: Option<std::num::NonZeroUsize>,
    has_validator: bool,
    has_default_validator: bool,
    has_writer: bool,
    ast: super::SchemaAst,
    map: F,
    classify: G,
    marker: std::marker::PhantomData<fn() -> (D, E)>,
}
impl<C, A, D, E> PreparedJsonArrayCodec<C, A, D, E>
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    C: super::PreparedCodec<A, SharedPreparedJsonValue, D, E, Error = SchemaIssue>,
{
    /// Select the standard SchemaIssue root family.
    pub fn new(
        codec: Arc<C>,
        policy: ParsePolicy,
        maximum: Option<std::num::NonZeroUsize>,
        resources: PreparedJsonArrayCodecResources<A>,
    ) -> Self {
        Self::new_mapped(
            codec,
            policy,
            maximum,
            resources,
            std::convert::identity::<SchemaIssue>,
            Ok::<SchemaIssue, SchemaIssue>,
        )
    }
}
impl<C, A, D, E, F, G> PreparedJsonArrayCodec<C, A, D, E, F, G>
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    /// Select the root mapper and explicit schema-issue classification at startup.
    pub fn new_mapped<X>(
        codec: Arc<C>,
        policy: ParsePolicy,
        maximum: Option<std::num::NonZeroUsize>,
        resources: PreparedJsonArrayCodecResources<A>,
        map: F,
        classify: G,
    ) -> Self
    where
        X: Send + 'static,
        C: super::PreparedCodec<A, SharedPreparedJsonValue, D, E, Error = X>,
        F: Fn(SchemaIssue) -> X + Send + Sync + 'static,
        G: Fn(X) -> Result<SchemaIssue, X> + Send + Sync + 'static,
    {
        let has_validator = codec.has_validator();
        let has_default_validator = codec.has_default_validator();
        let has_writer = codec.has_writer();
        let ast = super::SchemaAst::new(super::SchemaShape::Sequence {
            element: codec.ast().clone(),
            maximum,
        });
        Self {
            codec,
            arrays: resources.arrays,
            output: resources.output,
            operations: resources.operations,
            issues: resources.issues,
            validation: resources.validation,
            policy,
            maximum,
            has_validator,
            has_default_validator,
            has_writer,
            ast,
            map,
            classify,
            marker: std::marker::PhantomData,
        }
    }
    fn admit_decode(
        &self,
        slot: SlotLease<'static, Mutex<Option<RunState<A>>>>,
        input: SharedPreparedJsonValue,
        context: Option<ParseContext>,
    ) -> Result<RunHandle<A>, SchemaIssue> {
        let policy = super::parse_context::policy(context.as_ref(), self.policy);
        self.validation.check_parse_limits(&input, &policy.limits)?;
        if input.view().kind() != Some(super::PreparedJsonKind::Array) {
            return Err(SchemaIssue::new(super::SchemaIssueKind::TypeMismatch {
                expected: super::ValueKind::Sequence,
                actual: super::prepared_view_kind(input.view())?,
            }));
        }
        let count = input.view().len();
        if count > policy.limits.collection.get()
            || self.maximum.is_some_and(|max| count > max.get())
        {
            return Err(SchemaIssue::new(super::SchemaIssueKind::LimitExceeded {
                resource: super::SchemaResource::Width,
            }));
        }
        let values = self
            .arrays
            .reserve()
            .map_err(super::prepared_alternatives::memory)?;
        if values.capacity() < count {
            return Err(super::prepared_alternatives::memory(MemoryError::NoSlot));
        }
        let errors = if count == 0 || policy.errors == super::SchemaErrorMode::First {
            None
        } else {
            Some(super::prepared_alternatives::error_array(
                &self.issues,
                count.min(policy.limits.issues.get()),
            )?)
        };
        let budget = Budget::new(context.as_ref(), policy);
        let initial = budget.used();
        Ok(PreparedArrayOperationPool::publish(
            slot,
            RunState {
                input: Some(input),
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
                issues: Arc::clone(&self.issues),
                validation: Arc::clone(&self.validation),
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
impl<C, A, D, E, F, G> PreparedJsonArrayCodec<C, A, D, E, F, G>
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    fn admit_values(
        &self,
        slot: SlotLease<'static, Mutex<Option<RunState<A>>>>,
        values: PreparedJsonArray<A>,
        context: Option<ParseContext>,
        direction: Direction,
        writer: Option<JsonWriter>,
    ) -> Result<RunHandle<A>, SchemaIssue> {
        if matches!(direction, Direction::Validate) && !self.has_validator {
            return Err(SchemaIssue::new(super::SchemaIssueKind::MissingValidator));
        }
        if matches!(direction, Direction::Write) && !self.has_writer {
            return Err(SchemaIssue::new(super::SchemaIssueKind::MissingWriter));
        }
        let policy = super::parse_context::policy(context.as_ref(), self.policy);
        let count = values.as_slice().len();
        if count > policy.limits.collection.get()
            || self.maximum.is_some_and(|max| count > max.get())
        {
            return Err(SchemaIssue::new(super::SchemaIssueKind::LimitExceeded {
                resource: super::SchemaResource::Width,
            }));
        }
        let assembly = if matches!(direction, Direction::Encode) {
            Some(self.output.admit_array(policy)?)
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
                &self.issues,
                count.min(policy.limits.issues.get()),
            )?)
        };
        let budget = Budget::new(context.as_ref(), policy);
        let initial = budget.used();
        Ok(PreparedArrayOperationPool::publish(
            slot,
            RunState {
                input: None,
                values: Some(values),
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
                issues: Arc::clone(&self.issues),
                validation: Arc::clone(&self.validation),
            },
        ))
    }
}

impl<C, A, D, E, F, G, X> super::PreparedCodec<PreparedJsonArray<A>, SharedPreparedJsonValue, D, E>
    for PreparedJsonArrayCodec<C, A, D, E, F, G>
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    F: Fn(SchemaIssue) -> X + Send + Sync + 'static,
    G: Fn(X) -> Result<SchemaIssue, X> + Send + Sync + 'static,
    C: super::PreparedCodec<A, SharedPreparedJsonValue, D, E, Error = X>,
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
    fn has_writer(&self) -> bool {
        self.has_writer
    }
    fn has_default_validator(&self) -> bool {
        self.has_default_validator
    }
    fn validate_default_ref(
        &self,
        values: &PreparedJsonArray<A>,
        context: Option<&ParseContext>,
    ) -> Result<(), X> {
        if !self.has_default_validator {
            return Err((self.map)(SchemaIssue::new(
                super::SchemaIssueKind::MissingValidator,
            )));
        }
        let policy = super::parse_context::policy(context, self.policy);
        let count = values.as_slice().len();
        if count > policy.limits.collection.get()
            || self.maximum.is_some_and(|max| count > max.get())
        {
            return Err((self.map)(SchemaIssue::new(
                super::SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                },
            )));
        }
        if let Some(context) = context {
            context.visit().map_err(&self.map)?;
        }
        for (index, value) in values.iter().enumerate() {
            let selected = context
                .and_then(ParseContext::issue_memory)
                .or(Some(&self.issues));
            let child = super::parse_context::child(context, super::IssueSegment::Index(index))
                .map_err(|issue| {
                    (self.map)(super::parse_context::selected_issue_at(
                        selected,
                        issue,
                        super::IssueSegment::Index(index),
                    ))
                })?;
            if let Err(error) = self.codec.validate_default_ref(value, child.as_ref()) {
                return Err(match (self.classify)(error) {
                    Ok(issue) => (self.map)(super::parse_context::selected_issue_at(
                        selected,
                        issue,
                        super::IssueSegment::Index(index),
                    )),
                    Err(error) => error,
                });
            }
        }
        Ok(())
    }
    fn decode(
        self: Arc<Self>,
        input: SharedPreparedJsonValue,
        context: Option<ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = PreparedJsonArray<A>,
        Error = X,
        Env = D,
    > {
        let build = Arc::clone(&self);
        let codec = Arc::clone(&self.codec);
        let body_codec = Arc::clone(&self);
        let capacity_codec = Arc::clone(&self);
        let final_codec = Arc::clone(&self);
        crate::effect_prepare! { description; program: effect! {
            let (handle,guard_seed,body_seed) = effect_try_sync! { once; body: move || {
                (|| -> Result<_,SchemaIssue> {
                    let slot=build.operations.reserve().map_err(super::prepared_alternatives::memory)?;
                    seeds(build.admit_decode(slot,input,context)?)
                })().map_err(|issue| (build.map)(issue))
            }, }?;
            let _loop = effect! { while_loop;
                guard: { let selected=guard_seed; move || selected.with(|s| Ok(s.more())).unwrap_or(true) },
                body: { let selected=body_seed; let codec=Arc::clone(&codec); let body_codec=Arc::clone(&body_codec); move || {
                    let child_handle=selected.try_clone();
                    let codec=Arc::clone(&codec);
                    let prepare=Arc::clone(&body_codec);
                    let work_codec=Arc::clone(&body_codec);
                    let mapper_codec=Arc::clone(&body_codec);
                    let complete=Arc::clone(&body_codec); let capture_codec=Arc::clone(&body_codec);
                    effect! {
                        let (owner,work,mapper,index,capture)=effect_try_sync! { once; body: move || {
                            (|| -> Result<_,SchemaIssue> {
                                let owner=child_handle.map_err(super::prepared_alternatives::memory)?;
                                let (index,capture)=owner.with(|s| {let index=s.begin_child()?; Ok((index,s.policy.errors==super::SchemaErrorMode::Accumulate))})?;
                                let work=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                let mapper=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                Ok((owner,work,mapper,index,capture))
                            })().map_err(|issue| (prepare.map)(issue))
                        }, }?;
                        let result=crate::effect_prepare! { description; program: effect_map_error! {
                            program: effect! {
                                let (value,context)=effect_try_sync! { once; body: move || work.with(|s| { let context = super::parse_context::child(s.context.as_ref(),super::IssueSegment::Index(s.index))?; let value = s.input.as_ref().ok_or_else(invalid)?.try_get_index(s.index).map_err(super::prepared_alternatives::memory)?.ok_or_else(invalid)?; Ok((value,context)) }).map_err(|issue| (work_codec.map)(issue)), }?;
                                let value=<C as super::PreparedCodec<A,SharedPreparedJsonValue,D,E>>::decode(codec,value,context)?;
                                value
                            },
                            map: move |error| match (mapper_codec.classify)(error) {
                                Ok(issue)=>(mapper_codec.map)(mapper.prefix(issue,index)),
                                Err(error)=>error,
                            },
                        }, capture_errors: capture, classify_error: move |error| (capture_codec.classify)(error), }?;
                        let _done=effect_try_sync! { once; body: move || match result {
                            Ok(value)=>owner.with(|s| { s.values.as_mut().ok_or_else(invalid)?.push(value).map_err(|_| super::prepared_alternatives::memory(MemoryError::NoSlot))?; s.index += 1; Ok(()) }).map_err(|issue| (complete.map)(issue)),
                            Err(issue) => owner.with(|s| s.record_failure(issue)).map_err(|issue| (complete.map)(issue)),
                        }, }?;
                        ()
                    }
                } },
                on_capacity: move |error| (capacity_codec.map)(super::prepared_alternatives::memory(error)),
            }?;
            let result=effect_try_sync! { once; body: move || handle.with(|s| { s.finish_errors()?; s.values.take().ok_or_else(invalid) }).map_err(|issue| (final_codec.map)(issue)), }?;
            result
        }, }
    }
    fn encode(
        self: Arc<Self>,
        values: PreparedJsonArray<A>,
        context: Option<ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = SharedPreparedJsonValue,
        Error = X,
        Env = E,
    > {
        let build = Arc::clone(&self);
        let codec = Arc::clone(&self.codec);
        let body_codec = Arc::clone(&self);
        let capacity_codec = Arc::clone(&self);
        let final_codec = Arc::clone(&self);
        crate::effect_prepare! { description; program: effect! {
            let (handle,guard_seed,body_seed) = effect_try_sync! { once; body: move || {
                (|| -> Result<_,SchemaIssue> {
                    let slot=build.operations.reserve().map_err(super::prepared_alternatives::memory)?;
                    seeds(build.admit_values(slot,values,context,Direction::Encode,None)?)
                })().map_err(|issue| (build.map)(issue))
            }, }?;
            let _loop = effect! { while_loop;
                guard: { let selected=guard_seed; move || selected.with(|s| Ok(s.more())).unwrap_or(true) },
                body: { let selected=body_seed; let codec=Arc::clone(&codec); let body_codec=Arc::clone(&body_codec); move || {
                    let child_handle=selected.try_clone();
                    let codec=Arc::clone(&codec);
                    let prepare=Arc::clone(&body_codec);
                    let work_codec=Arc::clone(&body_codec);
                    let mapper_codec=Arc::clone(&body_codec);
                    let complete=Arc::clone(&body_codec); let capture_codec=Arc::clone(&body_codec);
                    effect! {
                        let (owner,work,mapper,index,capture)=effect_try_sync! { once; body: move || {
                            (|| -> Result<_,SchemaIssue> {
                                let owner=child_handle.map_err(super::prepared_alternatives::memory)?;
                                let (index,capture)=owner.with(|s| { let index = s.begin_child()?; s.assembly.as_ref().ok_or_else(invalid)?.preflight()?; Ok((index,s.policy.errors==super::SchemaErrorMode::Accumulate)) })?;
                                let work=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                let mapper=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                Ok((owner,work,mapper,index,capture))
                            })().map_err(|issue| (prepare.map)(issue))
                        }, }?;
                        let result=crate::effect_prepare! { description; program: effect_map_error! {
                            program: effect! {
                                let (value,context)=effect_try_sync! { once; body: move || work.with(|s| { let context = super::parse_context::child(s.context.as_ref(),super::IssueSegment::Index(s.index))?; let value = s.values.as_mut().ok_or_else(invalid)?.remove_first().ok_or_else(invalid)?; Ok((value,context)) }).map_err(|issue| (work_codec.map)(issue)), }?;
                                let value=<C as super::PreparedCodec<A,SharedPreparedJsonValue,D,E>>::encode(codec,value,context)?;
                                value
                            },
                            map: move |error| match (mapper_codec.classify)(error) {
                                Ok(issue)=>(mapper_codec.map)(mapper.prefix(issue,index)),
                                Err(error)=>error,
                            },
                        }, capture_errors: capture, classify_error: move |error| (capture_codec.classify)(error), }?;
                        let _done=effect_try_sync! { once; body: move || match result {
                            Ok(value)=>owner.with(|s| { s.assembly.as_mut().ok_or_else(invalid)?.push_value(value)?; s.index += 1; Ok(()) }).map_err(|issue| (complete.map)(issue)),
                            Err(issue) => owner.with(|s| s.record_failure(issue)).map_err(|issue| (complete.map)(issue)),
                        }, }?;
                        ()
                    }
                } },
                on_capacity: move |error| (capacity_codec.map)(super::prepared_alternatives::memory(error)),
            }?;
            let result=effect_try_sync! { once; body: move || handle.with(|s| { s.finish_errors()?; s.assembly.take().ok_or_else(invalid)?.finish() }).map_err(|issue| (final_codec.map)(issue)), }?;
            result
        }, }
    }
    fn validate(
        self: Arc<Self>,
        values: PreparedJsonArray<A>,
        context: Option<ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = PreparedJsonArray<A>,
        Error = X,
        Env = E,
    > {
        let build = Arc::clone(&self);
        let codec = Arc::clone(&self.codec);
        let body_codec = Arc::clone(&self);
        let capacity_codec = Arc::clone(&self);
        let final_codec = Arc::clone(&self);
        crate::effect_prepare! { description; program: effect! {
            let (handle,guard_seed,body_seed) = effect_try_sync! { once; body: move || {
                (|| -> Result<_,SchemaIssue> {
                    let slot=build.operations.reserve().map_err(super::prepared_alternatives::memory)?;
                    seeds(build.admit_values(slot,values,context,Direction::Validate,None)?)
                })().map_err(|issue| (build.map)(issue))
            }, }?;
            let _loop = effect! { while_loop;
                guard: { let selected=guard_seed; move || selected.with(|s| Ok(s.more())).unwrap_or(true) },
                body: { let selected=body_seed; let codec=Arc::clone(&codec); let body_codec=Arc::clone(&body_codec); move || {
                    let child_handle=selected.try_clone();
                    let codec=Arc::clone(&codec);
                    let prepare=Arc::clone(&body_codec);
                    let work_codec=Arc::clone(&body_codec);
                    let mapper_codec=Arc::clone(&body_codec);
                    let complete=Arc::clone(&body_codec); let capture_codec=Arc::clone(&body_codec);
                    effect! {
                        let (owner,work,mapper,index,capture)=effect_try_sync! { once; body: move || {
                            (|| -> Result<_,SchemaIssue> {
                                let owner=child_handle.map_err(super::prepared_alternatives::memory)?;
                                let (index,capture)=owner.with(|s| {let index=s.begin_child()?; Ok((index,s.policy.errors==super::SchemaErrorMode::Accumulate))})?;
                                let work=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                let mapper=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                Ok((owner,work,mapper,index,capture))
                            })().map_err(|issue| (prepare.map)(issue))
                        }, }?;
                        let result=crate::effect_prepare! { description; program: effect_map_error! {
                            program: effect! {
                                let (value,context)=effect_try_sync! { once; body: move || work.with(|s| { let context = super::parse_context::child(s.context.as_ref(),super::IssueSegment::Index(s.index))?; let value = s.values.as_mut().ok_or_else(invalid)?.remove_first().ok_or_else(invalid)?; Ok((value,context)) }).map_err(|issue| (work_codec.map)(issue)), }?;
                                let value=<C as super::PreparedCodec<A,SharedPreparedJsonValue,D,E>>::validate(codec,value,context)?;
                                value
                            },
                            map: move |error| match (mapper_codec.classify)(error) {
                                Ok(issue)=>(mapper_codec.map)(mapper.prefix(issue,index)),
                                Err(error)=>error,
                            },
                        }, capture_errors: capture, classify_error: move |error| (capture_codec.classify)(error), }?;
                        let _done=effect_try_sync! { once; body: move || match result {
                            Ok(value)=>owner.with(|s| { s.values.as_mut().ok_or_else(invalid)?.push(value).map_err(|_| super::prepared_alternatives::memory(MemoryError::NoSlot))?; s.index += 1; Ok(()) }).map_err(|issue| (complete.map)(issue)),
                            Err(issue) => owner.with(|s| s.record_failure(issue)).map_err(|issue| (complete.map)(issue)),
                        }, }?;
                        ()
                    }
                } },
                on_capacity: move |error| (capacity_codec.map)(super::prepared_alternatives::memory(error)),
            }?;
            let result=effect_try_sync! { once; body: move || handle.with(|s| { s.finish_errors()?; s.values.take().ok_or_else(invalid) }).map_err(|issue| (final_codec.map)(issue)), }?;
            result
        }, }
    }
    fn write(
        self: Arc<Self>,
        values: PreparedJsonArray<A>,
        writer: JsonWriter,
    ) -> impl crate::__private::prepared_graph::Description<Value = JsonWriter, Error = X, Env = E>
    {
        let build = Arc::clone(&self);
        let codec = Arc::clone(&self.codec);
        let body_codec = Arc::clone(&self);
        let capacity_codec = Arc::clone(&self);
        let final_codec = Arc::clone(&self);
        crate::effect_prepare! { description; program: effect! {
            let (handle,guard_seed,body_seed) = effect_try_sync! { once; body: move || {
                (|| -> Result<_,SchemaIssue> {
                    let slot=build.operations.reserve().map_err(super::prepared_alternatives::memory)?;
                    { let owners = seeds(build.admit_values(slot,values,None,Direction::Write,Some(writer))?)?; owners.0.with(|s| s.writer.as_mut().ok_or_else(invalid)?.begin_array())?; Ok(owners) }
                })().map_err(|issue| (build.map)(issue))
            }, }?;
            let _loop = effect! { while_loop;
                guard: { let selected=guard_seed; move || selected.with(|s| Ok(s.more())).unwrap_or(true) },
                body: { let selected=body_seed; let codec=Arc::clone(&codec); let body_codec=Arc::clone(&body_codec); move || {
                    let child_handle=selected.try_clone();
                    let codec=Arc::clone(&codec);
                    let prepare=Arc::clone(&body_codec);
                    let work_codec=Arc::clone(&body_codec);
                    let mapper_codec=Arc::clone(&body_codec);
                    let complete=Arc::clone(&body_codec); let capture_codec=Arc::clone(&body_codec);
                    effect! {
                        let (owner,work,mapper,index,capture)=effect_try_sync! { once; body: move || {
                            (|| -> Result<_,SchemaIssue> {
                                let owner=child_handle.map_err(super::prepared_alternatives::memory)?;
                                let index=owner.with(|s| Ok(s.index))?; let capture=false;
                                let work=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                let mapper=owner.try_clone().map_err(super::prepared_alternatives::memory)?;
                                Ok((owner,work,mapper,index,capture))
                            })().map_err(|issue| (prepare.map)(issue))
                        }, }?;
                        let result=crate::effect_prepare! { description; program: effect_map_error! {
                            program: effect! {
                                let (value,context)=effect_try_sync! { once; body: move || work.with(|s| { let value = s.values.as_mut().ok_or_else(invalid)?.remove_first().ok_or_else(invalid)?; let writer = s.writer.take().ok_or_else(invalid)?; Ok((value,writer)) }).map_err(|issue| (work_codec.map)(issue)), }?;
                                let value=<C as super::PreparedCodec<A,SharedPreparedJsonValue,D,E>>::write(codec,value,context)?;
                                value
                            },
                            map: move |error| match (mapper_codec.classify)(error) {
                                Ok(issue)=>(mapper_codec.map)(mapper.prefix(issue,index)),
                                Err(error)=>error,
                            },
                        }, capture_errors: capture, classify_error: move |error| (capture_codec.classify)(error), }?;
                        let _done=effect_try_sync! { once; body: move || match result {
                            Ok(value)=>owner.with(|s| { s.writer = Some(value); s.index += 1; Ok(()) }).map_err(|issue| (complete.map)(issue)),
                            Err(issue) => Err((complete.map)(issue)),
                        }, }?;
                        ()
                    }
                } },
                on_capacity: move |error| (capacity_codec.map)(super::prepared_alternatives::memory(error)),
            }?;
            let result=effect_try_sync! { once; body: move || handle.with(|s| { s.finish_errors()?; { let mut writer=s.writer.take().ok_or_else(invalid)?; writer.end_array()?; Ok(writer) } }).map_err(|issue| (final_codec.map)(issue)), }?;
            result
        }, }
    }
}
