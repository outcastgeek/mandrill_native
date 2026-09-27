use crate::LibraryDefect;
use crate::internal::destruction::FlatDrop;
use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::internal::workflow::{self, Workflow};
use crate::{
    Cancellation, Defect, Exit, HasCancellation, PrimaryOutcomeRef, Release, ResourceBinding,
    RunDriver, RunObserver,
};
use futures_util::FutureExt;
use std::future::Future;
use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

type Operation<A, E, R> = Arc<dyn Fn(Arc<R>, Context, Finish<A, E>) -> Step + Send + Sync>;

/// An opaque, lazy recipe. Cloning shares the recipe, not a running future.
#[must_use = "an Effect is a description; call run to execute it"]
pub struct Effect<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static = ()> {
    operation: FlatDrop<Operation<A, E, R>>,
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone for Effect<A, E, R> {
    fn clone(&self) -> Self {
        Self {
            operation: self.operation.clone(),
        }
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Effect<A, E, R> {
    pub(crate) fn new(
        operation: impl Fn(Arc<R>, Context, Finish<A, E>) -> Step + Send + Sync + 'static,
    ) -> Self {
        Self {
            operation: FlatDrop::new(Arc::new(operation)),
        }
    }

    pub(crate) fn execute(self, env: Arc<R>, context: Context, finish: Finish<A, E>) -> Step {
        runtime::more(move || {
            context.begin_operation();
            if context.cancelled() {
                return runtime::finish(finish, Exit::Interrupted);
            }
            match self.operation.as_ref() {
                Some(operation) => operation(env, context, finish),
                None => runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::MissingEffectRecipe)),
                ),
            }
        })
    }

    pub(crate) fn ready(
        callback: impl Fn(&R, &Context) -> Exit<A, E> + Send + Sync + 'static,
    ) -> Self {
        Self::new(move |env, context, finish| {
            let outcome = catch_unwind(AssertUnwindSafe(|| callback(&env, &context)))
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
            runtime::finish(finish, outcome)
        })
    }

    /// Fresh owned values can be produced without a Clone bound on A.
    pub fn sync(callback: impl Fn() -> A + Send + Sync + 'static) -> Self {
        Self::ready(move |_, _| Exit::Success(callback()))
    }

    pub fn succeed(value: A) -> Self
    where
        A: Clone + Sync,
    {
        Self::sync(move || value.clone())
    }

    pub fn fail(error: E) -> Self
    where
        E: Clone + Sync,
    {
        Self::fail_with(move || error.clone())
    }

    pub fn fail_with(callback: impl Fn() -> E + Send + Sync + 'static) -> Self {
        Self::ready(move |_, _| Exit::Fail(callback()))
    }

    pub fn die(defect: Defect) -> Self {
        Self::ready(move |_, _| Exit::Die(defect.clone()))
    }

    /// Result errors are mapped explicitly; unwinding panics remain defects.
    pub fn try_sync<X>(
        callback: impl Fn() -> Result<A, X> + Send + Sync + 'static,
        on_error: impl Fn(X) -> E + Send + Sync + 'static,
    ) -> Self {
        Self::ready(move |_, _| match callback() {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(on_error(error)),
        })
    }

    pub fn defer(factory: impl Fn() -> Self + Send + Sync + 'static) -> Self {
        Self::new(
            move |env, context, finish| match catch_unwind(AssertUnwindSafe(&factory)) {
                Ok(program) => program.execute(env, context, finish),
                Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            },
        )
    }

    /// Await protected cleanup after every started body outcome.
    pub fn ensuring(self, finalizer: Effect<(), E, R>) -> Self {
        self.ensuring_with(move |_| Release::from_effect(finalizer.clone()))
    }

    /// Inspect the borrowed primary outcome when constructing single-use
    /// cleanup, without requiring Clone on an expected error.
    pub fn ensuring_with(
        self,
        finalizer: impl for<'outcome> Fn(PrimaryOutcomeRef<'outcome, E>) -> Release<E, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        ResourceBinding::new(Effect::succeed(()), move |(), reason| finalizer(reason))
            .use_with(move |_| self.clone())
    }

    /// The factory receives cooperative cancellation and runs once per execution.
    /// Wrapped futures may still require their own reactor/runtime services.
    pub fn from_future<F: Future<Output = A> + Send + 'static>(
        factory: impl Fn(Cancellation) -> F + Send + Sync + 'static,
    ) -> Self {
        Self::future_exit(move |token| {
            let pending = factory(token);
            async move { Exit::Success(pending.await) }
        })
    }

    pub fn try_future<X: Send + 'static, F: Future<Output = Result<A, X>> + Send + 'static>(
        factory: impl Fn(Cancellation) -> F + Send + Sync + 'static,
        on_error: impl Fn(X) -> E + Send + Sync + 'static,
    ) -> Self {
        let on_error = Arc::new(on_error);
        Self::future_exit(move |token| {
            let pending = factory(token);
            let on_error = Arc::clone(&on_error);
            async move {
                match pending.await {
                    Ok(value) => Exit::Success(value),
                    Err(error) => Exit::Fail(on_error(error)),
                }
            }
        })
    }

    fn future_exit<F: Future<Output = Exit<A, E>> + Send + 'static>(
        factory: impl Fn(Cancellation) -> F + Send + Sync + 'static,
    ) -> Self {
        Self::new(move |_, context, finish| {
            match catch_unwind(AssertUnwindSafe(|| factory(context.cancellation.clone()))) {
                Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
                Ok(pending) => Step::Suspended(Box::pin(async move {
                    let outcome = AssertUnwindSafe(pending)
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    runtime::finish(finish, outcome)
                })),
            }
        })
    }

    pub fn service(project: impl Fn(&R) -> A + Send + Sync + 'static) -> Self {
        Self::ready(move |env, _| Exit::Success(project(env)))
    }

    pub fn provide(self, env: Arc<R>) -> Effect<A, E, ()> {
        Effect::new(move |_, context, finish| {
            self.clone().execute(Arc::clone(&env), context, finish)
        })
    }

    pub fn provide_some<L: Send + Sync + 'static>(
        self,
        project: impl Fn(&L) -> R + Send + Sync + 'static,
    ) -> Effect<A, E, L> {
        Effect::new(move |env, context, finish| {
            match catch_unwind(AssertUnwindSafe(|| project(&env))) {
                Ok(env) => self.clone().execute(Arc::new(env), context, finish),
                Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            }
        })
    }

    pub fn flat_map<B: Send + 'static>(
        self,
        next: impl Fn(A) -> Effect<B, E, R> + Send + Sync + 'static,
    ) -> Effect<B, E, R> {
        let next = Arc::new(next);
        workflow::reusable(move || {
            let next = Arc::clone(&next);
            workflow::bind(self.clone(), move |value| {
                let program = next(value);
                Workflow::new(move |env, context, finish| program.execute(env, context, finish))
            })
        })
    }

    pub fn map<B: Send + 'static>(
        self,
        transform: impl Fn(A) -> B + Send + Sync + 'static,
    ) -> Effect<B, E, R> {
        let transform = Arc::new(transform);
        workflow::reusable(move || {
            let transform = Arc::clone(&transform);
            workflow::bind(self.clone(), move |value| {
                workflow::success(transform(value))
            })
        })
    }

    fn bind_exit<B: Send + 'static, E2: Send + 'static>(
        self,
        next: impl Fn(Exit<A, E>) -> Workflow<B, E2, R> + Send + Sync + 'static,
    ) -> Effect<B, E2, R> {
        let next = Arc::new(next);
        Effect::new(move |env, context, finish| {
            let next = Arc::clone(&next);
            self.clone().execute(
                Arc::clone(&env),
                context.clone(),
                runtime::continuation(move |outcome| {
                    runtime::more(
                        move || match catch_unwind(AssertUnwindSafe(|| next(outcome))) {
                            Ok(program) => program.execute(env, context, finish),
                            Err(panic) => {
                                runtime::finish(finish, Exit::Die(Defect::from_panic(panic)))
                            }
                        },
                    )
                }),
            )
        })
    }

    pub fn map_error<E2: Send + 'static>(
        self,
        transform: impl Fn(E) -> E2 + Send + Sync + 'static,
    ) -> Effect<A, E2, R> {
        self.bind_exit(move |outcome| Workflow::from_exit(outcome.map_error(&transform)))
    }

    pub fn catch_all(self, recover: impl Fn(E) -> Self + Send + Sync + 'static) -> Self {
        let recover = Arc::new(recover);
        self.bind_exit(move |outcome| match outcome {
            Exit::Fail(error) => {
                let recover = Arc::clone(&recover);
                Workflow::new(move |env, context, finish| {
                    match catch_unwind(AssertUnwindSafe(|| recover(error))) {
                        Ok(program) => program.execute(env, context, finish),
                        Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
                    }
                })
            }
            other @ (Exit::Success(_)
            | Exit::Die(_)
            | Exit::Interrupted
            | Exit::FinalizationFailed(_)) => Workflow::from_exit(other),
        })
    }

    pub fn either(self) -> Effect<Result<A, E>, E, R> {
        self.bind_exit(|outcome| match outcome {
            Exit::Success(value) => workflow::success(Ok(value)),
            Exit::Fail(error) => workflow::success(Err(error)),
            Exit::Die(defect) => Workflow::from_exit(Exit::Die(defect)),
            Exit::Interrupted => Workflow::from_exit(Exit::Interrupted),
            Exit::FinalizationFailed(details) => {
                Workflow::from_exit(Exit::FinalizationFailed(details))
            }
        })
    }

    pub fn result(self) -> Effect<Result<A, E>, E, R> {
        self.either()
    }
    pub fn option(self) -> Effect<Option<A>, E, R> {
        self.either().map(Result::ok)
    }

    pub fn sandbox(self, on_defect: impl Fn(Defect) -> E + Send + Sync + 'static) -> Self {
        self.bind_exit(move |outcome| {
            Workflow::from_exit(match outcome {
                Exit::Die(defect) => Exit::Fail(on_defect(defect)),
                other @ (Exit::Success(_)
                | Exit::Fail(_)
                | Exit::Interrupted
                | Exit::FinalizationFailed(_)) => other,
            })
        })
    }

    /// Mask cancellation once this operation starts, then publish normally to
    /// its parent. A pre-cancelled run still does not start protected work.
    pub fn uninterruptible(self) -> Self {
        Self::new(move |env, context, finish| {
            self.clone().execute(
                env,
                context.protected(),
                runtime::continuation(move |outcome| {
                    context.record_protected_failure(&outcome);
                    runtime::finish(finish, outcome)
                }),
            )
        })
    }

    /// Await protected cleanup if this operation's primary outcome is interrupted.
    /// Cleanup failures are appended without replacing that primary outcome.
    pub fn on_interrupt(self, finalizer: Effect<(), E, R>) -> Self {
        Self::new(move |env, context, finish| {
            let finalizer = finalizer.clone();
            self.clone().execute(
                Arc::clone(&env),
                context.clone(),
                runtime::continuation(move |result| {
                    let result = runtime::publish(result, &context);
                    if !outcome::interrupted(&result) {
                        return runtime::finish(finish, result);
                    }
                    finalizer.execute(
                        env,
                        context.protected(),
                        runtime::continuation(move |cleanup| {
                            runtime::finish(finish, outcome::append_cleanup(result, cleanup))
                        }),
                    )
                }),
            )
        })
    }

    pub fn zip<B: Send + 'static>(self, right: Effect<B, E, R>) -> Effect<(A, B), E, R> {
        workflow::reusable(move || {
            let right = right.clone();
            workflow::bind(self.clone(), move |left| {
                workflow::bind(right, move |right| workflow::success((left, right)))
            })
        })
    }

    pub fn all(programs: Vec<Self>) -> Effect<Vec<A>, E, R> {
        workflow::reusable(move || collect(programs.clone().into_iter(), Vec::new()))
    }

    pub fn for_each<T: Clone + Send + Sync + 'static>(
        values: Vec<T>,
        transform: impl Fn(T) -> Self + Send + Sync + 'static,
    ) -> Effect<Vec<A>, E, R> {
        let transform = Arc::new(transform);
        workflow::reusable(move || {
            traverse(
                values.clone().into_iter(),
                Arc::clone(&transform),
                Vec::new(),
            )
        })
    }

    pub fn from_result(value: Result<A, E>) -> Self
    where
        A: Clone + Sync,
        E: Clone + Sync,
    {
        Self::ready(move |_, _| match &value {
            Ok(value) => Exit::Success(value.clone()),
            Err(error) => Exit::Fail(error.clone()),
        })
    }

    pub fn from_option(value: Option<A>, on_none: impl Fn() -> E + Send + Sync + 'static) -> Self
    where
        A: Clone + Sync,
    {
        Self::ready(move |_, _| match &value {
            Some(value) => Exit::Success(value.clone()),
            None => Exit::Fail(on_none()),
        })
    }

    /// Execution starts when polled. Dropping this future abandons execution;
    /// use [`Self::run_cancellable`] when the host needs to request cancellation.
    /// This boundary creates a fresh, internal request source and does not read
    /// cancellation from the environment. It requires no execution services.
    pub fn run(
        &self,
        env: Arc<R>,
    ) -> impl Future<Output = Exit<A, E>> + Send + 'static + use<A, E, R> {
        runtime::evaluate(self.clone(), env, Cancellation::new())
    }

    /// Run with the external cancellation source supplied by `R`.
    ///
    /// The accessor and all work are lazy. Request cancellation through the
    /// environment and continue awaiting to observe settlement. Environment
    /// projection within a run preserves its existing cancellation state.
    pub fn run_cancellable(
        &self,
        env: Arc<R>,
    ) -> impl Future<Output = Exit<A, E>> + Send + 'static + use<A, E, R>
    where
        R: HasCancellation,
    {
        let program = self.clone();
        async move {
            let cancellation = match catch_unwind(AssertUnwindSafe(|| env.cancellation().clone())) {
                Ok(cancellation) => cancellation,
                Err(panic) => return Exit::Die(Defect::from_panic(panic)),
            };
            runtime::evaluate(program, env, cancellation).await
        }
    }

    /// Separate the host's execution ownership from observation of the exit.
    /// The host polls the driver even if the observer is dropped.
    pub fn owned_run(&self, env: Arc<R>) -> (RunDriver<A, E>, RunObserver<A, E>) {
        crate::owned_run::owned(self.run(env))
    }

    /// An owned run whose cancellation source is supplied solely through `R`.
    pub fn owned_run_cancellable(&self, env: Arc<R>) -> (RunDriver<A, E>, RunObserver<A, E>)
    where
        R: HasCancellation,
    {
        crate::owned_run::owned(self.run_cancellable(env))
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Effect<(), E, R> {
    /// Check a lazy guard before each iteration and create a fresh body only
    /// when it succeeds. Failures and cancellation stop further iterations.
    pub fn while_loop(
        guard: impl Fn() -> bool + Send + Sync + 'static,
        body: impl Fn() -> Self + Send + Sync + 'static,
    ) -> Self {
        let step = Arc::new(move |()| {
            if guard() {
                body().map(|()| ControlFlow::Continue(()))
            } else {
                Effect::succeed(ControlFlow::Break(()))
            }
        });
        workflow::reusable(move || iterate((), Arc::clone(&step)))
    }
}

fn iterate<S, A, E, R, F>(state: S, step: Arc<F>) -> Workflow<A, E, R>
where
    S: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(S) -> Effect<ControlFlow<A, S>, E, R> + Send + Sync + 'static,
{
    let program = step(state);
    workflow::bind(program, move |next| match next {
        ControlFlow::Continue(state) => iterate(state, step),
        ControlFlow::Break(value) => workflow::success(value),
    })
}

fn collect<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    mut programs: std::vec::IntoIter<Effect<A, E, R>>,
    mut values: Vec<A>,
) -> Workflow<Vec<A>, E, R> {
    match programs.next() {
        Some(program) => workflow::bind(program, move |value| {
            values.push(value);
            collect(programs, values)
        }),
        None => workflow::success(values),
    }
}

fn traverse<T: Send + 'static, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, F>(
    mut input: std::vec::IntoIter<T>,
    transform: Arc<F>,
    mut values: Vec<A>,
) -> Workflow<Vec<A>, E, R>
where
    F: Fn(T) -> Effect<A, E, R> + Send + Sync + 'static,
{
    match input.next() {
        Some(value) => {
            let program = transform(value);
            workflow::bind(program, move |value| {
                values.push(value);
                traverse(input, transform, values)
            })
        }
        None => workflow::success(values),
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Effect<A, E, R> {
    /// An explicitly consumed callback for owned captures. Construct inside a
    /// reusable effect! factory to get fresh inputs on each application run.
    pub fn sync_once(callback: impl FnOnce() -> A + Send + 'static) -> Self {
        Self::try_sync_once(
            move || Ok::<A, std::convert::Infallible>(callback()),
            |never| match never {},
        )
    }
    pub fn try_sync_once<X>(
        callback: impl FnOnce() -> Result<A, X> + Send + 'static,
        on_error: impl FnOnce(X) -> E + Send + 'static,
    ) -> Self {
        let candidate = std::sync::Mutex::new(Some((callback, on_error)));
        Self::ready(move |_, _| {
            let candidate = candidate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            match candidate {
                Some((callback, on_error)) => match callback() {
                    Ok(value) => Exit::Success(value),
                    Err(error) => Exit::Fail(on_error(error)),
                },
                None => Exit::Die(Defect::from(LibraryDefect::SyncCallbackConsumed)),
            }
        })
    }
    pub fn from_result_once(value: Result<A, E>) -> Self {
        Self::try_sync_once(move || value, |error| error)
    }
    pub fn from_option_once(
        value: Option<A>,
        on_none: impl FnOnce() -> E + Send + 'static,
    ) -> Self {
        Self::try_sync_once(move || value.ok_or_else(on_none), |error| error)
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Effect<A, E, R> {
    pub fn from_future_with_env<F: Future<Output = A> + Send + 'static>(
        factory: impl Fn(Arc<R>, Cancellation) -> F + Send + Sync + 'static,
    ) -> Self {
        Self::try_future_with_env(
            move |env, cancellation| {
                let future = factory(env, cancellation);
                async move { Ok::<A, std::convert::Infallible>(future.await) }
            },
            |never| match never {},
        )
    }
    pub fn try_future_with_env<
        X: Send + 'static,
        F: Future<Output = Result<A, X>> + Send + 'static,
    >(
        factory: impl Fn(Arc<R>, Cancellation) -> F + Send + Sync + 'static,
        on_error: impl Fn(X) -> E + Send + Sync + 'static,
    ) -> Self {
        let factory = Arc::new(factory);
        let on_error = Arc::new(on_error);
        Self::new(move |env, context, finish| {
            let factory = Arc::clone(&factory);
            let on_error = Arc::clone(&on_error);
            Step::Suspended(Box::pin(async move {
                let outcome = AssertUnwindSafe(async {
                    match factory(Arc::clone(&env), context.cancellation.clone()).await {
                        Ok(value) => Exit::Success(value),
                        Err(error) => Exit::Fail(on_error(error)),
                    }
                })
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }
    pub fn from_future_once<F: Future<Output = A> + Send + 'static>(
        factory: impl FnOnce(Cancellation) -> F + Send + 'static,
    ) -> Self {
        Self::try_future_once(
            move |cancel| {
                let future = factory(cancel);
                async move { Ok::<A, std::convert::Infallible>(future.await) }
            },
            |never| match never {},
        )
    }
    pub fn try_future_once<X: Send + 'static, F: Future<Output = Result<A, X>> + Send + 'static>(
        factory: impl FnOnce(Cancellation) -> F + Send + 'static,
        on_error: impl FnOnce(X) -> E + Send + 'static,
    ) -> Self {
        let candidate = std::sync::Mutex::new(Some((factory, on_error)));
        Self::new(move |_, context, finish| {
            let candidate = candidate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            match candidate {
                Some((factory, on_error)) => Step::Suspended(Box::pin(async move {
                    let outcome = AssertUnwindSafe(async {
                        match factory(context.cancellation).await {
                            Ok(value) => Exit::Success(value),
                            Err(error) => Exit::Fail(on_error(error)),
                        }
                    })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    runtime::finish(finish, outcome)
                })),
                None => runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::FutureFactoryConsumed)),
                ),
            }
        })
    }
}
