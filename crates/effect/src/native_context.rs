//! Provider-neutral access to an existing Effect execution from native adapters.

use crate::internal::runtime::{self, Context};
use crate::{Cancellation, Defect, Effect, Exit, HasCancellation, TraceContext};
use futures_util::FutureExt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Opaque context of one running Effect, retaining its cancellation and trace state.
///
/// Only a running Effect can supply this handle. It grants no SQL, request, grant,
/// or resource authority and cannot manufacture a new runtime. Native adapters
/// retain their own owners while awaiting nested programs through [`Self::run`].
#[derive(Clone)]
pub struct NativeContext {
    /// Interpreter state shared with the enclosing native operation.
    pub(crate) context: Context,
}

/// Affine ownership of the future, output, entry and instrumentation frames
/// for one contextual nested future. Dropping it unused returns the frames.
#[must_use = "dropping the reservation returns its unused frames"]
pub struct PreparedContextualFutureReservation<I, R, F, A, E>
where
    I: Send + 'static,
    R: Send + Sync + 'static,
    F: Future<Output = Exit<A, E>> + Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
{
    origin: NativeContext,
    frames: crate::internal::runtime_storage::FutureReservation<F, A, E>,
    nested: runtime::PreparedNestedReservation<A, E>,
    instrumentation: crate::internal::runtime_storage::InstrumentationReservation<A, E>,
    entry: crate::memory::FrameReservation<'static, PreparedContextualEntry<I, R, F, A, E>>,
}

/// Exact physical frame layouts retained by a contextual future reservation.
#[derive(Clone, Copy, Debug)]
pub struct PreparedContextualFutureLayouts {
    /// Pinned future frame.
    pub pending: std::alloc::Layout,
    /// Future completion transition frame.
    pub completion: std::alloc::Layout,
    /// Nested interpreter outcome frame.
    pub output: std::alloc::Layout,
    /// Nested interpreter terminal store frame.
    pub store: std::alloc::Layout,
    /// Typed operation entry frame.
    pub entry: std::alloc::Layout,
    /// Entry and restore instrumentation frame layouts.
    pub instrumentation: crate::memory::PreparedContextualDemand,
}

struct PreparedContextualEntry<I, R, F, A: Send + 'static, E: Send + 'static>
where
    I: Send + 'static,
    R: Send + Sync + 'static,
    F: Future<Output = Exit<A, E>> + Send + 'static,
{
    input: I,
    environment: Arc<R>,
    context: Context,
    finish: runtime::Finish<A, E>,
    frames: crate::internal::runtime_storage::FutureReservation<F, A, E>,
    factory: fn(I, Arc<R>, NativeContext) -> F,
}

impl<I, R, F, A, E> crate::internal::runtime_storage::Consume<(), runtime::Step>
    for PreparedContextualEntry<I, R, F, A, E>
where
    I: Send + 'static,
    R: Send + Sync + 'static,
    F: Future<Output = Exit<A, E>> + Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
{
    fn consume(self, (): ()) -> runtime::Step {
        let Self {
            input,
            environment,
            context,
            finish,
            frames,
            factory,
        } = self;
        if !runtime::begin_operation(&context) {
            return frames.complete(Exit::Interrupted, finish);
        }
        frames.start(
            || factory(input, environment, NativeContext { context }),
            finish,
        )
    }
}

impl<I, R, F, A, E> PreparedContextualFutureReservation<I, R, F, A, E>
where
    I: Send + 'static,
    R: Send + Sync + 'static,
    F: Future<Output = Exit<A, E>> + Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
{
    /// Exact layouts of the reserved frames; transitive pointees and the
    /// surrounding interpreter remain separate.
    #[must_use]
    pub fn layouts() -> PreparedContextualFutureLayouts {
        let (pending, completion) =
            crate::internal::runtime_storage::FutureReservation::<F, A, E>::layouts();
        let (output, store) = runtime::PreparedNestedReservation::<A, E>::layouts();
        PreparedContextualFutureLayouts {
            pending,
            completion,
            output,
            store,
            entry: std::alloc::Layout::new::<PreparedContextualEntry<I, R, F, A, E>>(),
            instrumentation:
                crate::internal::runtime_storage::InstrumentationReservation::<A, E>::layouts(),
        }
    }

    /// Consume the reservation on its original context using the existing
    /// nested interpreter, preserving its full Exit and publication behavior.
    pub async fn run(
        self,
        input: I,
        env: Arc<R>,
        factory: fn(I, Arc<R>, NativeContext) -> F,
    ) -> Exit<A, E> {
        let context = self.origin.context;
        let entry_context = context.clone();
        let frames = self.frames;
        let instrumentation = self.instrumentation;
        let entry = self.entry;
        runtime::evaluate_context_reserved(&context, self.nested, move |finish| {
            let (finish, tag) = instrumentation.finish(finish, entry_context.clone());
            let step = runtime::Step::More(crate::internal::runtime_storage::Once::prepared(
                entry,
                PreparedContextualEntry {
                    input,
                    environment: env,
                    context: entry_context.clone(),
                    finish,
                    frames,
                    factory,
                },
            ));
            runtime::instrument_step_reserved(step, entry_context, tag)
        })
        .await
    }
}

impl NativeContext {
    /// Claim the actual frames from this context before constructing the
    /// future. Missing or insufficient selected storage returns its typed cause.
    pub fn reserve_prepared_exit_future<I, R, F, A, E>(
        &self,
    ) -> Result<PreparedContextualFutureReservation<I, R, F, A, E>, crate::memory::MemoryError>
    where
        I: Send + 'static,
        R: Send + Sync + 'static,
        F: Future<Output = Exit<A, E>> + Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
    {
        let pool = self
            .context
            .frames()
            .ok_or(crate::memory::MemoryError::MissingRuntimeStorage)?;
        let frames = crate::internal::runtime_storage::FutureReservation::reserve(pool)?;
        let nested = runtime::PreparedNestedReservation::reserve(pool)?;
        let instrumentation =
            crate::internal::runtime_storage::InstrumentationReservation::reserve(pool)?;
        let entry = pool.reserve()?;
        Ok(PreparedContextualFutureReservation {
            origin: self.clone(),
            frames,
            nested,
            instrumentation,
            entry,
        })
    }
    /// Borrow the cooperative cancellation token of this execution.
    /// Dropping a future does not acknowledge native rollback or resource release.
    #[must_use]
    pub fn cancellation(&self) -> &Cancellation {
        &self.context.cancellation
    }

    /// Reserve the checked cancellation registration before polling admitted native work.
    pub fn try_cancelled(
        &self,
    ) -> Result<impl Future<Output = ()> + '_, crate::memory::MemoryError> {
        self.context.cancellation.try_cancelled()
    }

    /// Borrow the current Effect trace, whose IDs are distinct from native OTel IDs.
    #[must_use]
    pub fn trace(&self) -> &TraceContext {
        &self.context.trace
    }

    /// Evaluate a nested typed program through the existing interpreter and context.
    /// The caller supplies the original environment or its explicit projection;
    /// values and errors move into the returned exit without `Clone` bounds.
    /// This does not retain an abandoned native owner: the enclosing adapter must
    /// continue driving cancellation and asynchronous cleanup through settlement.
    pub async fn run<A, E, R>(&self, program: Effect<A, E, R>, env: Arc<R>) -> Exit<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        runtime::evaluate_context(program, env, &self.context).await
    }

    /// Evaluate native request work with the cancellation source selected by `R`.
    ///
    /// The current trace and poll instrumentation are retained, while the supplied
    /// environment's token replaces this handle's cancellation source. A request
    /// disconnect can therefore interrupt that request without cancelling its host
    /// or siblings. Host-to-request cancellation links, when needed, belong to the
    /// request environment's token ownership. Request work receives an independent
    /// failure/clock frontier through the existing context rebasing operation.
    ///
    /// Selection is lazy when this future is polled; an accessor panic becomes a
    /// defect. The adapter must keep awaiting through ordered finalization after
    /// requesting cancellation. This method creates no new runtime or executor.
    pub async fn run_cancellable<A, E, R>(
        &self,
        program: Effect<A, E, R>,
        env: Arc<R>,
    ) -> Exit<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: HasCancellation + Send + Sync + 'static,
    {
        let cancellation = match catch_unwind(AssertUnwindSafe(|| env.cancellation().clone())) {
            Ok(cancellation) => cancellation,
            Err(panic) => return Exit::Die(Defect::from_panic(panic)),
        };
        let context = self.context.parallel(cancellation);
        runtime::evaluate_context(program, env, &context).await
    }
}

/// A native instrumentation guard dropped before a poll returns or a callback ends.
///
/// Guards deliberately need not be `Send`: an ambient span/dispatcher/context
/// guard stays on the polling thread and is never retained across an await.
pub trait PollScope {}

/// Native context selected through the application's environment for an Effect.
/// Core knows no tracing subscriber, SDK provider, exporter or global singleton.
pub trait PollInstrumentation: Send + Sync + 'static {
    /// Enter native context for this trace on the current polling thread.
    /// The returned guard restores prior native context when dropped.
    fn enter(&self, trace: &TraceContext) -> Box<dyn PollScope>;
}

impl<A, E, R> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Return the actual pending and completion frame layouts for a contextual
    /// prepared future factory, without invoking that factory. The layouts
    /// cover these frames only, not captured pointees or interpreter storage.
    #[must_use]
    pub fn prepared_exit_future_with_context_layouts<F>(
        _factory: &impl Fn(Arc<R>, NativeContext) -> F,
    ) -> (std::alloc::Layout, std::alloc::Layout)
    where
        F: Future<Output = Exit<A, E>> + Send + 'static,
    {
        crate::internal::runtime_storage::FutureReservation::<F, A, E>::layouts()
    }

    /// Run a contextual, full-Exit future in the current run's selected work
    /// or cleanup frames. Both frames are reserved before the factory runs;
    /// missing or exhausted storage is mapped by `on_capacity` without fallback.
    /// The surrounding Effect interpreter and native allocations remain separate.
    pub fn from_prepared_exit_future_with_context<F>(
        factory: impl Fn(Arc<R>, NativeContext) -> F + Send + Sync + 'static,
        on_capacity: impl Fn(crate::memory::MemoryError) -> E + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = Exit<A, E>> + Send + 'static,
    {
        Self::future_exit(move |env, context, finish| {
            let failed = |finish, error| {
                let outcome = catch_unwind(AssertUnwindSafe(|| on_capacity(error)))
                    .map(Exit::Fail)
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            };
            let Some(pool) = context.frames() else {
                return failed(finish, crate::memory::MemoryError::MissingRuntimeStorage);
            };
            let native_context = NativeContext {
                context: context.clone(),
            };
            match crate::internal::runtime_storage::prepare_future(
                pool,
                || factory(env, native_context),
                finish,
            ) {
                Ok(step) => step,
                Err((error, finish)) => failed(finish, error),
            }
        })
    }

    /// Run this program with an explicitly selected Effect trace context.
    ///
    /// Selection is scoped to this lazy program and its nested work; the caller's
    /// trace is restored before its continuation resumes, including on failure or
    /// interruption. The environment, cancellation source, execution ownership
    /// and selected poll instrumentation are retained. Shared annotations and
    /// span identities are shared without copying application payloads.
    ///
    /// A context with no span detaches this work from an inherited local Effect
    /// span. Effect IDs remain distinct from native OTel IDs: native propagation
    /// adapters supply and validate their own remote context through `R`. This
    /// method neither creates a span nor installs a native provider or runtime.
    pub fn with_trace_context(self, trace: TraceContext) -> Self {
        Self::new(move |env, mut context, finish| {
            context.trace = trace.clone();
            self.clone().execute(env, context, finish)
        })
    }

    /// Construct a fresh native future with the current interpreter context each run.
    ///
    /// The future returns the complete typed exit, retaining defects, interruption,
    /// finalization failure and durable uncertainty represented by `E`. Panics
    /// become defects. Native futures keep their reactor requirements; this method
    /// creates no runtime and performs no native operation during construction.
    /// Shared configuration belongs in the factory; fresh payloads move inside it.
    pub fn from_exit_future_with_context<F>(
        factory: impl Fn(Arc<R>, NativeContext) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = Exit<A, E>> + Send + 'static,
    {
        let factory = Arc::new(factory);
        Self::new(move |env, context, finish| {
            let factory = Arc::clone(&factory);
            runtime::Step::suspended(Box::pin(async move {
                let outcome =
                    AssertUnwindSafe(async move { factory(env, NativeContext { context }).await })
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }

    /// Select poll-scoped native instrumentation from `R` for this program.
    ///
    /// The selected hook follows nested Effects and scoped children. Its guard
    /// surrounds synchronous transitions and each native future poll separately;
    /// it is dropped before returning `Pending`. Selection happens lazily per run.
    /// Native hooks must isolate best-effort telemetry failures themselves.
    pub fn with_poll_instrumentation(
        self,
        select: impl Fn(&R) -> Arc<dyn PollInstrumentation> + Send + Sync + 'static,
    ) -> Self {
        Self::new(move |env, mut context, finish| {
            context.instrumentation = Some(select(&env));
            self.clone().execute(env, context, finish)
        })
    }
}
