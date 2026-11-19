//! Shared root ownership adapter for typed generated graph transitions.
use super::*;
#[path = "owned_input.rs"]
mod owned_input;

pub struct PreparedGraphPlan<F, D: Description> {
    factory: F,
    contract: DynamicRunContract,
    owned_environment: bool,
    shared_environment: bool,
    cleanup: Option<crate::PreparedCleanupConfig<D::Error>>,
    panic_pool: Option<crate::PreparedPanicPool>,
    prepared_trace: Option<PreparedTraceContext>,
    instrumentation: Option<PreparedPollInstrumentation>,
    marker: PhantomData<fn() -> D>,
}
pub struct PreparedGraph<F, D: Description> {
    recipe: SharedSlot<'static, F>,
    rows: SlotPool<'static, Root<D::Value, D::Error>>,
    frames: FramePool<'static>,
    storage: D::Storage,
    policy: MemoryPolicy,
    owned_environment: bool,
    shared_environment: bool,
    prepared_trace: Option<PreparedTraceContext>,
}
impl<F, I, D> PreparedGraphPlan<InputRecipe<F, I>, D>
where
    D: Description,
    I: Send + 'static,
    F: Fn(I) -> D + Send + Sync + 'static,
{
    /// Plan a shared graph recipe which consumes one late-published owned input.
    pub fn new_with_input(factory: F, contract: DynamicRunContract) -> Self {
        Self {
            factory: InputRecipe {
                factory,
                marker: PhantomData,
            },
            contract,
            owned_environment: false,
            shared_environment: false,
            cleanup: None,
            panic_pool: None,
            prepared_trace: None,
            instrumentation: None,
            marker: PhantomData,
        }
    }
}
/// Graph admission retains its input publication authority before route decoding.
pub struct PendingInputSharedGraph<
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    pending: PendingSharedGraphInvocation<A, E, R>,
    input: InputPublisher<I, A, E>,
}
pub struct PendingOwnedInputSharedGraph<
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    pending: PendingOwnedSharedGraph<A, E, R>,
    input: InputPublisher<I, A, E>,
}
impl<I: Send + 'static, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>
    PendingInputSharedGraph<I, A, E, R>
{
    pub fn reserve_output_origins(
        &self,
    ) -> Result<super::super::ReservedOutputOrigins<A, E>, MemoryError> {
        self.pending.reserve_output_origins()
    }
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub fn into_owned(
        self,
        origins: super::super::ReservedOutputOrigins<A, E>,
    ) -> Result<
        PendingOwnedInputSharedGraph<I, A, E, R>,
        (Self, super::super::ReservedOutputOrigins<A, E>),
    > {
        let Self { pending, input } = self;
        match pending.into_owned(origins) {
            Ok(pending) => Ok(PendingOwnedInputSharedGraph { pending, input }),
            Err((pending, origins)) => Err((Self { pending, input }, origins)),
        }
    }
}
impl<I: Send + 'static, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>
    PendingOwnedInputSharedGraph<I, A, E, R>
{
    /// Move the input before exposing the environment publication/driver state.
    pub fn publish_input(self, input: I) -> PendingOwnedSharedGraph<A, E, R> {
        self.input.publish(input);
        self.pending
    }
}
/// Fully admitted owned driver held until its input has been published.
/// Dropping this value retires the unused invocation and publication authority.
pub struct PendingOwnedInputGraph<I: Send + 'static, A: Send + 'static, E: Send + 'static> {
    driver: super::super::PreparedOwnedDriver<A, E>,
    input: InputPublisher<I, A, E>,
}
impl<I: Send + 'static, A: Send + 'static, E: Send + 'static> PendingOwnedInputGraph<I, A, E> {
    /// Publish the owned input before exposing the driver for polling.
    pub fn publish_input(self, input: I) -> super::super::PreparedOwnedDriver<A, E> {
        self.input.publish(input);
        self.driver
    }
}
impl<F, I, D> PreparedGraph<InputRecipe<F, I>, D>
where
    D: Description,
    I: Send + 'static,
    F: Fn(I) -> D + Send + Sync + 'static,
{
    /// Admit using an existing shared environment and reserve the unique output
    /// driver before accepting the late owned input. No factory runs here.
    pub fn admit_owned_input(
        &self,
        environment: Arc<D::Env>,
    ) -> Result<PendingOwnedInputGraph<I, D::Value, D::Error>, MemoryError> {
        if self.owned_environment {
            return Err(MemoryError::UnsupportedRunContract);
        }
        let (invocation, _, _, input) = self.admit_with(
            RootEnvironment::External(environment),
            |_| Ok(()),
            |_, (), _| Ok(()),
        )?;
        let origins = invocation.reserve_output_origins()?;
        let driver = invocation
            .into_owned_driver(origins)
            .map_err(|_| MemoryError::UnsupportedRunContract)?;
        Ok(PendingOwnedInputGraph { driver, input })
    }
    #[allow(clippy::type_complexity)]
    pub fn admit_input_shared(
        &self,
    ) -> Result<PendingInputSharedGraph<I, D::Value, D::Error, D::Env>, MemoryError> {
        if !self.shared_environment {
            return Err(MemoryError::UnsupportedRunContract);
        }
        let (invocation, _, publisher, input) = self.admit_with(
            RootEnvironment::UnpublishedShared,
            |_| Ok(()),
            |_, (), _| Ok(()),
        )?;
        match publisher {
            Some(PendingPublisher::Shared(publisher)) => Ok(PendingInputSharedGraph {
                pending: PendingSharedGraphInvocation {
                    invocation,
                    publisher,
                },
                input,
            }),
            _ => Err(MemoryError::UnsupportedRunContract),
        }
    }
}
pub type OwnedGraphInvocation<A, E, R> = (PreparedInvocation<A, E>, Environment<R>);
pub type OwnedGraphAdmission<D> = Result<
    OwnedGraphInvocation<<D as Spec>::Value, <D as Spec>::Error, <D as Spec>::Env>,
    MemoryError,
>;
/// Fully admitted body whose actual selected R is not yet published. No run
/// handle escapes before publication; dropping rolls back the unused body.
pub struct PendingGraphInvocation<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    invocation: PreparedInvocation<A, E>,
    publisher: super::environment::PreparedEnvironmentPublisher<R>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>
    PendingGraphInvocation<A, E, R>
{
    pub(super) fn has_panic_diagnostics(&self) -> bool {
        self.invocation.context.root.panic_pool.is_some()
    }
    pub fn publish(self, value: R) -> PreparedInvocation<A, E> {
        self.publisher.publish(value);
        self.invocation
    }
    pub(super) fn publish_from(
        self,
        value: R,
        origin: super::super::OriginSnapshot,
    ) -> PreparedInvocation<A, E> {
        self.publisher.publish_from(value, origin);
        self.invocation
    }
    /// Actual pre-admitted cancellation/trace context, without starting work.
    pub fn context(&self) -> Result<PreparedRunContext<A, E>, MemoryError> {
        self.invocation.context()
    }
}
/// Admitted graph awaiting an externally owned shared environment.
pub struct PendingSharedGraphInvocation<
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    invocation: PreparedInvocation<A, E>,
    publisher: super::environment::PreparedEnvironmentPublisher<Arc<R>>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>
    PendingSharedGraphInvocation<A, E, R>
{
    pub fn reserve_output_origins(
        &self,
    ) -> Result<super::super::ReservedOutputOrigins<A, E>, MemoryError> {
        self.invocation.reserve_output_origins()
    }
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub fn into_owned(
        self,
        origins: super::super::ReservedOutputOrigins<A, E>,
    ) -> Result<PendingOwnedSharedGraph<A, E, R>, (Self, super::super::ReservedOutputOrigins<A, E>)>
    {
        let Self {
            invocation,
            publisher,
        } = self;
        match invocation.into_owned_driver(origins) {
            Ok(driver) => Ok(PendingOwnedSharedGraph { driver, publisher }),
            Err((invocation, origins)) => Err((
                Self {
                    invocation,
                    publisher,
                },
                origins,
            )),
        }
    }
    pub fn publish_shared(self, value: Arc<R>) -> PreparedInvocation<A, E> {
        self.publisher.publish(value);
        self.invocation
    }
    pub fn context(&self) -> Result<PreparedRunContext<A, E>, MemoryError> {
        self.invocation.context()
    }
}
/// A pre-claimed unique driver awaiting selected shared R.
pub struct PendingOwnedSharedGraph<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    driver: super::super::PreparedOwnedDriver<A, E>,
    publisher: super::environment::PreparedEnvironmentPublisher<Arc<R>>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>
    PendingOwnedSharedGraph<A, E, R>
{
    pub fn publish_shared(self, value: Arc<R>) -> super::super::PreparedOwnedDriver<A, E> {
        self.publisher.publish(value);
        self.driver
    }
}
pub type DeferredSharedGraphAdmission<D> = Result<
    PendingSharedGraphInvocation<<D as Spec>::Value, <D as Spec>::Error, <D as Spec>::Env>,
    MemoryError,
>;
pub type DeferredGraphAdmission<D> = Result<
    PendingGraphInvocation<<D as Spec>::Value, <D as Spec>::Error, <D as Spec>::Env>,
    MemoryError,
>;
type RootAdmission<D, P> = Result<
    (
        PreparedInvocation<<D as Spec>::Value, <D as Spec>::Error>,
        Option<Environment<<D as Spec>::Env>>,
        Option<PendingPublisher<<D as Spec>::Env>>,
        P,
    ),
    MemoryError,
>;
enum PendingPublisher<R: Send + Sync + 'static> {
    Owned(super::environment::PreparedEnvironmentPublisher<R>),
    Shared(super::environment::PreparedEnvironmentPublisher<Arc<R>>),
}
enum RootEnvironment<R: Send + Sync + 'static> {
    External(Arc<R>),
    Owned(R),
    Unpublished,
    UnpublishedShared,
}
struct Factory<F: RootRecipe<D>, D: Description> {
    recipe: SharedSlot<'static, F>,
    input: F::Held,
    admission: D::Admission<D::Value>,
    failed: FrameReservation<'static, Invoke<D::Value, D::Error>>,
    finish: Finish<D::Value, D::Error>,
    context: PreparedRunContext<D::Value, D::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<F, D> Consume<(), Step> for Factory<F, D>
where
    D: Description,
    F: RootRecipe<D>,
{
    fn consume(self, (): ()) -> Step {
        self.context
            .root
            .contexts
            .protected_failure(self.context.id, false);
        if self.context.is_cancelled() {
            return Step::More(Once::prepared(
                self.failed,
                Invoke {
                    outcome: Exit::Interrupted,
                    finish: self.finish,
                },
            ));
        }
        match catch_unwind(AssertUnwindSafe(|| self.recipe.invoke(self.input))) {
            Ok(Ok(description)) => {
                description
                    .dispatch(self.admission, Continuation(self.finish))
                    .0
            }
            Ok(Err(defect)) => Step::More(Once::prepared(
                self.failed,
                Invoke {
                    outcome: Exit::Die(defect),
                    finish: self.finish,
                },
            )),
            Err(panic) => Step::More(Once::prepared(
                self.failed,
                Invoke {
                    outcome: Exit::Die(super::super::panic_defect(self.panic, panic)),
                    finish: self.finish,
                },
            )),
        }
    }
}
impl<F, D> PreparedGraphPlan<F, D>
where
    D: Description,
    F: RootRecipe<D>,
{
    pub fn new(factory: F, contract: DynamicRunContract) -> Self {
        Self {
            factory,
            contract,
            owned_environment: false,
            shared_environment: false,
            cleanup: None,
            panic_pool: None,
            prepared_trace: None,
            instrumentation: None,
            marker: PhantomData,
        }
    }
    /// Reserve the actual root environment value/owner in this plan's frames.
    /// This is distinct from the ordinary external Arc environment mode. No R
    /// clone or caller allocator is needed; admission returns a retained view.
    #[must_use]
    pub fn with_owned_environment(mut self) -> Self {
        self.owned_environment = true;
        self.shared_environment = false;
        self
    }
    /// Reserve a late-published external Arc owner without allocating its control block.
    #[must_use]
    pub fn with_shared_environment(mut self) -> Self {
        self.owned_environment = true;
        self.shared_environment = true;
        self
    }
    /// Supply separately prepared cleanup backing; no capacity is implicit.
    #[must_use]
    pub fn with_cleanup(mut self, cleanup: crate::PreparedCleanupConfig<D::Error>) -> Self {
        self.cleanup = Some(cleanup);
        self
    }
    /// Supply caller-prepared panic diagnostics; unconfigured graphs retain ordinary conversion.
    #[must_use]
    pub fn with_panic_diagnostics(mut self, pool: crate::PreparedPanicPool) -> Self {
        self.panic_pool = Some(pool);
        self
    }
    /// Retain a default trace snapshot for invocations of this graph.
    #[must_use]
    pub fn with_prepared_trace(mut self, trace: PreparedTraceContext) -> Self {
        self.prepared_trace = Some(trace);
        self
    }
    /// Select a startup-prepared native hook for the graph's poll transitions.
    #[must_use]
    pub fn with_prepared_poll_instrumentation(mut self, hook: PreparedPollInstrumentation) -> Self {
        self.instrumentation = Some(hook);
        self
    }
    pub fn demand(&self, capacity: usize) -> Result<Demand, MemoryError> {
        self.demand_for_layout(capacity, None)
    }
    /// Demand using the actual enclosing frame layout selected for preparation.
    pub fn demand_with_layout(
        &self,
        capacity: usize,
        layout: Layout,
    ) -> Result<Demand, MemoryError> {
        self.demand_for_layout(capacity, Some(layout))
    }
    fn demand_for_layout(
        &self,
        capacity: usize,
        selected: Option<Layout>,
    ) -> Result<Demand, MemoryError> {
        let DynamicRunContract::GeneratedGraph {
            clock_ticks,
            contexts,
            listeners,
            bridges,
        } = self.contract
        else {
            return Err(MemoryError::UnsupportedRunContract);
        };
        let mut demand = D::shape::<D::Value>()?
            .add(Demand::panic_sites(1))?
            .add(Demand::cell::<Factory<F, D>>(false))?
            .add(Demand::cell::<Invoke<D::Value, D::Error>>(true))?
            .add(Demand::cell::<Store<D::Value, D::Error>>(true))?
            .add(output_owner_shape::<
                SharedSlot<'static, F>,
                D::Value,
                D::Error,
            >())?;
        if let Some(input) = F::input_demand() {
            demand = demand.add(input)?;
        }
        if self.shared_environment {
            demand = demand.add(Environment::<D::Env>::shared_pending_demand::<
                D::Value,
                D::Error,
            >())?;
        } else if self.owned_environment {
            demand = demand.add(Environment::<D::Env>::pending_demand::<D::Value, D::Error>())?;
        }
        let layout = selected.unwrap_or(demand.frame_layout);
        if layout.align() < demand.frame_layout.align() {
            return Err(MemoryError::UnsupportedAlignment {
                requested: demand.frame_layout.align(),
                supported: layout.align(),
            });
        }
        if layout.size() < demand.frame_layout.size() {
            return Err(MemoryError::ValueTooLarge {
                requested: Bytes::new(demand.frame_layout.size()),
                capacity: Bytes::new(layout.size()),
            });
        }
        demand.record_payload = SlotPool::<Root<D::Value, D::Error>>::storage_bytes(capacity)?
            .checked_add(SlotPool::<F>::storage_bytes(1)?)?
            .checked_add(Domain::payload(contexts, listeners, clock_ticks)?.checked_mul(capacity)?)?
            .checked_add(
                super::super::bridge::Storage::payload::<D::Value, D::Error>(bridges)?
                    .checked_mul(capacity)?,
            )?;
        demand.record_payload = demand
            .record_payload
            .checked_add(D::storage_bytes::<D::Value>(capacity, layout)?)?;
        Ok(demand)
    }
    pub fn prepare(
        self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: FramePool<'static>,
    ) -> Result<PreparedGraph<F, D>, MemoryError> {
        let demand = self.demand_with_layout(capacity, frames.layout())?;
        let panic_total = demand
            .panic_sites
            .checked_mul(capacity)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        if let Some(pool) = &self.panic_pool
            && pool.capacity() < panic_total
        {
            return Err(MemoryError::NoSlot);
        }
        let cleanup_total = demand
            .cleanup_sites
            .checked_mul(capacity)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        match &self.cleanup {
            Some(cleanup) => {
                cleanup.validate_frames()?;
                if cleanup.frames.same_pool(&frames) {
                    return Err(MemoryError::ControlPoolAlias);
                }
                if cleanup.pool.capacity() < cleanup_total
                    || cleanup.frames.capacity() < cleanup_total
                {
                    return Err(MemoryError::NoSlot);
                }
            }
            None if cleanup_total != 0 => return Err(MemoryError::UnsupportedRunContract),
            None => {}
        }
        let DynamicRunContract::GeneratedGraph {
            clock_ticks,
            contexts,
            listeners,
            bridges,
        } = self.contract
        else {
            return Err(MemoryError::UnsupportedRunContract);
        };
        let rows = memory.slots(region, capacity, |_| {
            Ok(Root {
                output: Mutex::new(None),
                owners: Mutex::new(super::super::Owners::new()),
                construction: Mutex::new(None),
                consumers: AtomicUsize::new(0),
                output_readers: AtomicUsize::new(0),
                closed: AtomicBool::new(true),
                panic_pool: self
                    .panic_pool
                    .as_ref()
                    .map(crate::PreparedPanicPool::try_clone)
                    .transpose()?,
                contexts: Domain::prepare(memory, region, contexts, listeners, clock_ticks)?,
                bridges: super::super::bridge::Storage::prepare::<D::Value, D::Error>(
                    memory, region, bridges,
                )?,
                trace: TraceContext::default(),
                instrumentation: self
                    .instrumentation
                    .as_ref()
                    .map(PreparedPollInstrumentation::try_clone)
                    .transpose()?,
            })
        })?;
        let storage =
            D::provision::<D::Value>(memory, region, capacity, &frames, self.cleanup.as_ref())?;
        let mut factory = Some(self.factory);
        let recipes = memory.slots(region, 1, |_| factory.take().ok_or(MemoryError::NoSlot))?;
        Ok(PreparedGraph {
            recipe: recipes.try_checkout()?.share(),
            rows,
            frames,
            storage,
            policy: memory.policy(),
            owned_environment: self.owned_environment,
            shared_environment: self.shared_environment,
            prepared_trace: self.prepared_trace,
        })
    }
}
impl<F, D> PreparedGraph<F, D>
where
    D: Description,
    F: RootRecipe<D>,
{
    #[must_use]
    pub fn policy(&self) -> MemoryPolicy {
        self.policy
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.rows.active()
    }
    pub fn close(&self) {
        self.rows.close();
    }
    pub fn admit(
        &self,
        environment: Arc<D::Env>,
    ) -> Result<PreparedInvocation<D::Value, D::Error>, MemoryError>
    where
        F: RootRecipe<D, Publisher = ()>,
    {
        self.admit_external(environment, self.prepared_trace.as_ref())
    }
    /// Retain a caller-prepared trace for this invocation in place of the default.
    pub fn admit_with_prepared_trace(
        &self,
        environment: Arc<D::Env>,
        trace: &PreparedTraceContext,
    ) -> Result<PreparedInvocation<D::Value, D::Error>, MemoryError>
    where
        F: RootRecipe<D, Publisher = ()>,
    {
        self.admit_external(environment, Some(trace))
    }
    fn admit_external(
        &self,
        environment: Arc<D::Env>,
        trace: Option<&PreparedTraceContext>,
    ) -> Result<PreparedInvocation<D::Value, D::Error>, MemoryError>
    where
        F: RootRecipe<D, Publisher = ()>,
    {
        if self.owned_environment {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.admit_with_trace(
            RootEnvironment::External(environment),
            trace,
            |_| Ok(()),
            |_, (), _| Ok(()),
        )
        .map(|(invocation, _, _, ())| invocation)
    }
    /// Install an actual per-run non-Clone input/environment before any graph
    /// factory. The returned counted view allows synchronous input publication
    /// through the environment's own synchronization, without a new System Arc.
    pub fn admit_value(&self, environment: D::Env) -> OwnedGraphAdmission<D>
    where
        F: RootRecipe<D, Publisher = ()>,
    {
        if !self.owned_environment || self.shared_environment {
            return Err(MemoryError::UnsupportedRunContract);
        }
        let (invocation, view, _, ()) = self.admit_with(
            RootEnvironment::Owned(environment),
            |_| Ok(()),
            |_, (), _| Ok(()),
        )?;
        view.map(|view| (invocation, view))
            .ok_or(MemoryError::UnsupportedRunContract)
    }
    /// Reserve the whole body and its actual R cell before selecting R. Caller
    /// must publish the selected value before polling the body; admission never
    /// invokes a business projector/factory to obtain it.
    pub fn admit_unpublished(&self) -> DeferredGraphAdmission<D>
    where
        F: RootRecipe<D, Publisher = ()>,
    {
        if !self.owned_environment || self.shared_environment {
            return Err(MemoryError::UnsupportedRunContract);
        }
        let (invocation, _, publisher, ()) =
            self.admit_with(RootEnvironment::Unpublished, |_| Ok(()), |_, (), _| Ok(()))?;
        publisher
            .and_then(|publisher| match publisher {
                PendingPublisher::Owned(publisher) => Some(PendingGraphInvocation {
                    invocation,
                    publisher,
                }),
                PendingPublisher::Shared(_) => None,
            })
            .ok_or(MemoryError::UnsupportedRunContract)
    }
    /// Reserve the graph before the shared environment is selected.
    pub fn admit_unpublished_shared(&self) -> DeferredSharedGraphAdmission<D>
    where
        F: RootRecipe<D, Publisher = ()>,
    {
        if !self.shared_environment {
            return Err(MemoryError::UnsupportedRunContract);
        }
        let (invocation, _, publisher, ()) = self.admit_with(
            RootEnvironment::UnpublishedShared,
            |_| Ok(()),
            |_, (), _| Ok(()),
        )?;
        match publisher {
            Some(PendingPublisher::Shared(publisher)) => Ok(PendingSharedGraphInvocation {
                invocation,
                publisher,
            }),
            _ => Err(MemoryError::UnsupportedRunContract),
        }
    }
    fn admit_with<S>(
        &self,
        environment: RootEnvironment<D::Env>,
        setup: impl FnOnce(&PreparedRunContext<D::Value, D::Error>) -> Result<S, MemoryError>,
        preflight: impl FnOnce(
            &PreparedRunContext<D::Value, D::Error>,
            S,
            &Environment<D::Env>,
        ) -> Result<(), MemoryError>,
    ) -> RootAdmission<D, F::Publisher> {
        self.admit_with_trace(environment, self.prepared_trace.as_ref(), setup, preflight)
    }
    fn admit_with_trace<S>(
        &self,
        environment: RootEnvironment<D::Env>,
        trace: Option<&PreparedTraceContext>,
        setup: impl FnOnce(&PreparedRunContext<D::Value, D::Error>) -> Result<S, MemoryError>,
        preflight: impl FnOnce(
            &PreparedRunContext<D::Value, D::Error>,
            S,
            &Environment<D::Env>,
        ) -> Result<(), MemoryError>,
    ) -> RootAdmission<D, F::Publisher> {
        let recipe = self.recipe.try_clone()?;
        let mut row = self.rows.try_checkout()?;
        row.closed.store(true, Ordering::Release);
        let old = row
            .output
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if row.bridges.in_use() {
            return Err(MemoryError::Busy);
        }
        row.contexts.reset()?;
        drop(old);
        row.trace = TraceContext::default();
        row.closed.store(false, Ordering::Relaxed);
        *row.consumers.get_mut() = 1;
        *row.output_readers.get_mut() = 1;
        let factory = self.frames.reserve::<Factory<F, D>>()?;
        let failed = self.frames.reserve::<Invoke<D::Value, D::Error>>()?;
        let store = self.frames.reserve::<Store<D::Value, D::Error>>()?;
        let prepared_trace = trace.map(PreparedTraceContext::try_clone).transpose()?;
        let id = row.contexts.create(Kind::Root, None, None, None)?;
        let context = PreparedRunContext {
            root: row.share(),
            id,
            origin_group: None,
            prepared_trace,
        };
        let (input_publisher, input) = F::reserve_input(&self.frames, &context)?;
        let (environment, owned_view, publisher) = match environment {
            RootEnvironment::External(value) => (Environment::external(value), None, None),
            RootEnvironment::Owned(value) => {
                let (publisher, environment) = Environment::reserve(&self.frames, &context)?;
                let view = environment.try_clone()?;
                publisher.install(value);
                (environment, Some(view), None)
            }
            RootEnvironment::Unpublished => {
                let (publisher, environment) =
                    Environment::reserve_pending(&self.frames, &context)?;
                (environment, None, Some(PendingPublisher::Owned(publisher)))
            }
            RootEnvironment::UnpublishedShared => {
                let (publisher, environment) =
                    Environment::reserve_shared_pending(&self.frames, &context)?;
                (environment, None, Some(PendingPublisher::Shared(publisher)))
            }
        };
        let callback = context.root.try_clone()?;
        let loss = Loss {
            root: context.root.try_clone()?,
        };
        let output = PreparedOutput {
            reader: super::super::OutputReader {
                root: context.root.try_clone()?,
                armed: true,
            },
            root: super::super::OutputLease {
                root: context.root.try_clone()?,
            },
        };
        let recipe_owner = context.reserve_output_owner::<SharedSlot<'static, F>>(&self.frames)?;
        recipe_owner
            .attach(recipe.try_clone()?)
            .map_err(|(error, _)| error)?;
        let factory_context = context.try_clone()?;
        let panic = context.reserve_panic()?;
        let construction = setup(&context)?;
        // Recursively reserve real callback/capture/continuation/terminal frames
        // and owners for every concrete produced description BEFORE factory.
        let admission = D::reserve(&self.frames, &self.storage, &environment, &context)?;
        preflight(&context, construction, &environment)?;
        let finish = Once::prepared(store, Store { root: callback });
        let step = Step::More(Once::prepared(
            factory,
            Factory {
                recipe,
                input,
                admission,
                failed,
                finish,
                context: factory_context,
                panic,
            },
        ));
        Ok((
            PreparedInvocation {
                step,
                context,
                output,
                loss,
            },
            owned_view,
            publisher,
            input_publisher,
        ))
    }
}

/// An explicit native pre-business constructor over a finite typed site shape.
/// Base graph frames and every site vacancy are acquired before this callback;
/// actual foreign target construction finishes before description factories.
/// Native constructor over an actual region-owned environment view.
pub struct OwnedConstructor<G>(G);
/// Native construction over actual caller-supplied pre-business inputs; no
/// projected/unpublished target R is read to acquire its dependencies.
pub struct InputConstructor<I, G>(G, PhantomData<fn() -> I>);
pub struct ConstructedGraphPlan<F, D: Description, P, G> {
    plan: PreparedGraphPlan<F, D>,
    preflight: G,
    marker: PhantomData<fn() -> P>,
}
pub struct ConstructedGraph<F, D: Description, P, G: Send + Sync + 'static> {
    graph: PreparedGraph<F, D>,
    pool: super::super::PreparedConstructionPool,
    preflight: SharedSlot<'static, G>,
    marker: PhantomData<fn() -> P>,
}
impl<F, D> PreparedGraphPlan<F, D>
where
    D: Description,
    F: Fn() -> D + Send + Sync + 'static,
{
    /// Constructor companion for an unpublished target environment. Its native
    /// preflight receives actual per-admission inputs, not an early projection.
    #[must_use]
    pub fn with_inputs_construction<P, I, G>(
        self,
        _shape: P,
        preflight: G,
    ) -> ConstructedGraphPlan<F, D, P, InputConstructor<I, G>>
    where
        P: super::super::ConstructionShape,
        I: 'static,
        G: Fn(&I, P::Vacancies<D::Value, D::Error>) -> Result<(), MemoryError>
            + Send
            + Sync
            + 'static,
    {
        ConstructedGraphPlan {
            plan: if self.shared_environment {
                self
            } else {
                self.with_owned_environment()
            },
            preflight: InputConstructor(preflight, PhantomData),
            marker: PhantomData,
        }
    }
    /// Owned-input companion to the external-Arc construction API. The finite
    /// vacancies and actual root value are held before this native callback;
    /// projected/business factories have not run. Same construction registry.
    #[must_use]
    pub fn with_environment_construction<P, G>(
        self,
        _shape: P,
        preflight: G,
    ) -> ConstructedGraphPlan<F, D, P, OwnedConstructor<G>>
    where
        P: super::super::ConstructionShape,
        G: Fn(&Environment<D::Env>, P::Vacancies<D::Value, D::Error>) -> Result<(), MemoryError>
            + Send
            + Sync
            + 'static,
    {
        ConstructedGraphPlan {
            plan: self.with_owned_environment(),
            preflight: OwnedConstructor(preflight),
            marker: PhantomData,
        }
    }
    #[must_use]
    pub fn with_construction<P, G>(
        self,
        _shape: P,
        preflight: G,
    ) -> ConstructedGraphPlan<F, D, P, G>
    where
        P: super::super::ConstructionShape,
        G: Fn(&Arc<D::Env>, P::Vacancies<D::Value, D::Error>) -> Result<(), MemoryError>
            + Send
            + Sync
            + 'static,
    {
        ConstructedGraphPlan {
            plan: self,
            preflight,
            marker: PhantomData,
        }
    }
}
impl<F, D, P, G> ConstructedGraphPlan<F, D, P, G>
where
    D: Description,
    F: Fn() -> D + Send + Sync + 'static,
    P: super::super::ConstructionShape,
    G: Send + Sync + 'static,
{
    pub fn demand(&self, capacity: usize) -> Result<Demand, MemoryError> {
        self.demand_for_layout(capacity, None)
    }
    /// Demand using the actual enclosing frame layout selected for preparation.
    pub fn demand_with_layout(
        &self,
        capacity: usize,
        layout: Layout,
    ) -> Result<Demand, MemoryError> {
        self.demand_for_layout(capacity, Some(layout))
    }
    fn demand_for_layout(
        &self,
        capacity: usize,
        selected: Option<Layout>,
    ) -> Result<Demand, MemoryError> {
        let extra = P::demand::<D::Value, D::Error>()?;
        let extra_demand = Demand {
            frame_layout: extra.frame_layout,
            frames_per_run: extra.frames_per_run,
            continuations: 0,
            descriptions: 0,
            origin_frames: 0,
            environment_origin_frames: 0,
            record_payload: extra
                .record_payload
                .checked_add(super::super::PreparedConstructionPool::storage_bytes(
                    capacity,
                    P::sites()?,
                )?)?
                .checked_add(SlotPool::<G>::storage_bytes(1)?)?,
            cleanup_sites: 0,
            panic_sites: extra.panic_sites,
        };
        let callback = output_owner_shape::<SharedSlot<'static, G>, D::Value, D::Error>();
        let minimum = self
            .plan
            .demand(capacity)?
            .add(callback)?
            .add(extra_demand)?;
        let layout = selected.unwrap_or(minimum.frame_layout);
        if layout.align() < minimum.frame_layout.align() {
            return Err(MemoryError::UnsupportedAlignment {
                requested: minimum.frame_layout.align(),
                supported: layout.align(),
            });
        }
        if layout.size() < minimum.frame_layout.size() {
            return Err(MemoryError::ValueTooLarge {
                requested: Bytes::new(minimum.frame_layout.size()),
                capacity: Bytes::new(layout.size()),
            });
        }
        self.plan
            .demand_with_layout(capacity, layout)?
            .add(output_owner_shape::<
                SharedSlot<'static, G>,
                D::Value,
                D::Error,
            >())?
            .add(extra_demand)
    }
    pub fn prepare(
        self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: FramePool<'static>,
    ) -> Result<ConstructedGraph<F, D, P, G>, MemoryError> {
        let _demand = self.demand_with_layout(capacity, frames.layout())?;
        let graph = crate::effect_prepare! { prepare; plan:self.plan,memory:memory,region:region,capacity:capacity,frames:frames, }?;
        let pool =
            super::super::PreparedConstructionPool::prepare(memory, region, capacity, P::sites()?)?;
        let mut callback = Some(self.preflight);
        let callbacks = memory.slots(region, 1, |_| callback.take().ok_or(MemoryError::NoSlot))?;
        Ok(ConstructedGraph {
            graph,
            pool,
            preflight: callbacks.try_checkout()?.share(),
            marker: PhantomData,
        })
    }
}
impl<F, D, P, G> ConstructedGraph<F, D, P, G>
where
    D: Description,
    F: Fn() -> D + Send + Sync + 'static,
    P: super::super::ConstructionShape,
    G: Send + Sync + 'static,
{
    fn reserve_construction(
        &self,
        context: &PreparedRunContext<D::Value, D::Error>,
    ) -> Result<P::Vacancies<D::Value, D::Error>, MemoryError> {
        let callback_owner =
            context.reserve_output_owner::<SharedSlot<'static, G>>(&self.graph.frames)?;
        callback_owner
            .attach(self.preflight.try_clone()?)
            .map_err(|(error, _)| error)?;
        let lane = self.pool.checkout()?;
        *context
            .root
            .construction
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(lane);
        P::reserve(context, &self.graph.frames)
    }
}
impl<F, D, P, G> ConstructedGraph<F, D, P, G>
where
    D: Description,
    F: Fn() -> D + Send + Sync + 'static,
    P: super::super::ConstructionShape,
    G: Fn(&Arc<D::Env>, P::Vacancies<D::Value, D::Error>) -> Result<(), MemoryError>
        + Send
        + Sync
        + 'static,
{
    pub fn admit(
        &self,
        environment: Arc<D::Env>,
    ) -> Result<PreparedInvocation<D::Value, D::Error>, MemoryError> {
        self.admit_using_trace(environment, self.graph.prepared_trace.as_ref())
    }
    /// Retain a caller-prepared trace before invoking construction preflight.
    pub fn admit_with_prepared_trace(
        &self,
        environment: Arc<D::Env>,
        trace: &PreparedTraceContext,
    ) -> Result<PreparedInvocation<D::Value, D::Error>, MemoryError> {
        self.admit_using_trace(environment, Some(trace))
    }
    fn admit_using_trace(
        &self,
        environment: Arc<D::Env>,
        trace: Option<&PreparedTraceContext>,
    ) -> Result<PreparedInvocation<D::Value, D::Error>, MemoryError> {
        let native = Arc::clone(&environment);
        if self.graph.owned_environment {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.graph
            .admit_with_trace(
                RootEnvironment::External(environment),
                trace,
                |context| self.reserve_construction(context),
                |context, vacancies, _| {
                    (self.preflight)(&native, vacancies)?;
                    context.construction_ready()
                },
            )
            .map(|(invocation, _, _, ())| invocation)
    }
    pub fn close(&self) {
        self.graph.close();
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.graph.active()
    }
    #[must_use]
    pub fn policy(&self) -> MemoryPolicy {
        self.graph.policy()
    }
}
