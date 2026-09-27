use crate::internal::gate::{Gate, GateError};
use crate::internal::notification::notify_all;
use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Step};
use crate::scope::Registered;
use crate::{Cancellation, Clock, Defect, Effect, Exit, HasClock, LibraryDefect, Scope};
use event_listener::Event;
use futures_util::{
    FutureExt,
    future::{Either, select},
};
use std::collections::VecDeque;
use std::future::Future;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

type OwnedFuture<T, E> = Pin<Box<dyn Future<Output = Exit<T, E>> + Send>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolCapacity {
    Unbounded,
    Bounded(NonZeroUsize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolEndStatus {
    Clean,
    Failed,
}
struct State<T: Send + Sync + 'static, E: Send + 'static> {
    closed: bool,
    resources: usize,
    capacity: PoolCapacity,
    idle: VecDeque<Arc<Entry<T, E>>>,
    diagnostics: Vec<Exit<(), E>>,
    shutdown: Shutdown<E>,
    population: Vec<Arc<Population<E>>>,
}
enum Shutdown<E> {
    Open,
    Closing,
    Closed(Option<Exit<(), E>>),
}
type Validate<T, E> = Arc<dyn Fn(&T, Context) -> OwnedFuture<bool, E> + Send + Sync>;
struct Inner<T: Send + Sync + 'static, E: Send + 'static> {
    state: Mutex<State<T, E>>,
    changed: Event,
    acquiring: Gate,
    cancellation: Cancellation,
    ttl: Option<Duration>,
    now: Arc<dyn Fn() -> Duration + Send + Sync>,
    create: Arc<dyn Fn(Context) -> OwnedFuture<T, E> + Send + Sync>,
    validate: Validate<T, E>,
    dispose: Arc<dyn Fn(T, Context) -> OwnedFuture<(), E> + Send + Sync>,
    spawn: Arc<dyn Fn(crate::ScheduledTask) -> Result<(), Defect> + Send + Sync>,
}
pub struct Pool<T: Send + Sync + 'static, E: Send + 'static> {
    inner: Arc<Inner<T, E>>,
}
impl<T: Send + Sync + 'static, E: Send + 'static> Clone for Pool<T, E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
struct Slot<T: Send + Sync + 'static, E: Send + 'static> {
    pool: Weak<Inner<T, E>>,
}
impl<T: Send + Sync + 'static, E: Send + 'static> Drop for Slot<T, E> {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade() {
            pool.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .resources -= NonZeroUsize::MIN.get();
            notify_all(&pool.changed);
        }
    }
}
struct Entry<T: Send + Sync + 'static, E: Send + 'static> {
    value: RwLock<Option<T>>,
    returned: Mutex<Duration>,
    slot: Slot<T, E>,
}
impl<T: Send + Sync + 'static, E: Send + 'static> Drop for Entry<T, E> {
    fn drop(&mut self) {
        if self
            .value
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
            && let Some(pool) = self.slot.pool.upgrade()
        {
            pool.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .diagnostics
                .push(Exit::Die(Defect::from(LibraryDefect::PoolResourceDropped)));
        }
    }
}
struct LeaseState<T: Send + Sync + 'static, E: Send + 'static> {
    entry: Option<Arc<Entry<T, E>>>,
    phase: LeasePhase<E>,
}
enum LeasePhase<E> {
    Open,
    Closing,
    Closed(Option<Exit<(), E>>),
}
struct LeaseInner<T: Send + Sync + 'static, E: Send + 'static> {
    pool: Pool<T, E>,
    state: Mutex<LeaseState<T, E>>,
    changed: Event,
}
pub struct PoolLease<T: Send + Sync + 'static, E: Send + 'static> {
    inner: Arc<LeaseInner<T, E>>,
}
impl<T: Send + Sync + 'static, E: Send + 'static> Clone for PoolLease<T, E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static> PoolLease<T, E> {
    pub fn with<A>(&self, inspect: impl FnOnce(&T) -> A) -> Result<A, Defect> {
        let entry = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| Defect::from(LibraryDefect::PoolLeaseEnded))?;
        let value = entry
            .value
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match value.as_ref() {
            Some(value) => {
                catch_unwind(AssertUnwindSafe(|| inspect(value))).map_err(Defect::from_panic)
            }
            None => Err(Defect::from(LibraryDefect::PoolLeaseEnded)),
        }
    }
    fn abandon(&self) {
        let entry = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !matches!(state.phase, LeasePhase::Open) {
                return;
            }
            state.phase = LeasePhase::Closed(Some(Exit::Die(Defect::from(
                LibraryDefect::ScopeOwnerDropped,
            ))));
            state.entry.take()
        };
        // Retain ownership for an explicit recovery drain; Drop cannot
        // invoke and await the native disposer.
        self.inner.pool.abandon_entry(entry);
        notify_all(&self.inner.changed);
        notify_all(&self.inner.pool.inner.changed);
    }
    fn status(&self) -> PoolEndStatus {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &state.phase {
            LeasePhase::Closed(Some(Exit::Success(()))) | LeasePhase::Closed(None) => {
                PoolEndStatus::Clean
            }
            LeasePhase::Closed(Some(
                Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_),
            ))
            | LeasePhase::Open
            | LeasePhase::Closing => PoolEndStatus::Failed,
        }
    }
    async fn end(&self, retire: bool, context: Context) {
        let entry = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match state.phase {
                LeasePhase::Open => {
                    state.phase = LeasePhase::Closing;
                    state.entry.take()
                }
                LeasePhase::Closing | LeasePhase::Closed(_) => None,
            }
        };
        if let Some(entry) = entry {
            let mut guard = LeaseDriver {
                lease: self.clone(),
                settled: false,
            };
            let outcome = self
                .inner
                .pool
                .return_entry(entry, retire, context.protected())
                .await;
            self.inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .phase = LeasePhase::Closed(Some(outcome));
            guard.settled = true;
            notify_all(&self.inner.changed);
        } else {
            loop {
                let changed = self.inner.changed.listen();
                if matches!(
                    self.inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .phase,
                    LeasePhase::Closed(_)
                ) {
                    break;
                }
                changed.await;
            }
        }
    }
    pub fn release<R: Send + Sync + 'static>(&self) -> Effect<PoolEndStatus, E, R> {
        self.end_effect(false)
    }
    pub fn discard<R: Send + Sync + 'static>(&self) -> Effect<PoolEndStatus, E, R> {
        self.end_effect(true)
    }
    fn end_effect<R: Send + Sync + 'static>(&self, retire: bool) -> Effect<PoolEndStatus, E, R> {
        let lease = self.clone();
        Effect::new(move |_, context, finish| {
            let lease = lease.clone();
            Step::Suspended(Box::pin(async move {
                lease.end(retire, context).await;
                runtime::finish(finish, Exit::Success(lease.status()))
            }))
        })
        .uninterruptible()
    }
    async fn finalize(&self, context: Context) -> Exit<(), E> {
        self.end(false, context).await;
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut state.phase {
            LeasePhase::Closed(outcome) => outcome.take().unwrap_or(Exit::Success(())),
            LeasePhase::Open | LeasePhase::Closing => {
                Exit::Die(Defect::from(LibraryDefect::PoolLeaseEnded))
            }
        }
    }
}
struct LeaseDriver<T: Send + Sync + 'static, E: Send + 'static> {
    lease: PoolLease<T, E>,
    settled: bool,
}
impl<T: Send + Sync + 'static, E: Send + 'static> Drop for LeaseDriver<T, E> {
    fn drop(&mut self) {
        if !self.settled {
            self.lease
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .phase = LeasePhase::Closed(Some(Exit::Die(Defect::from(
                LibraryDefect::PoolReleaseDriverDropped,
            ))));
            notify_all(&self.lease.inner.changed);
        }
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static> Pool<T, E> {
    pub fn make<R: HasClock + crate::HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<E>,
        capacity: PoolCapacity,
        idle_ttl: Option<Duration>,
        create: Effect<T, E, R>,
        validate: impl Fn(&T) -> Effect<bool, E, R> + Send + Sync + 'static,
        dispose: impl Fn(T) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Effect<Self, E, R> {
        let validate = Arc::new(validate);
        let dispose = Arc::new(dispose);
        Effect::new(move |env: Arc<R>, context, finish| {
            let create_env = Arc::clone(&env);
            let validate_env = Arc::clone(&env);
            let dispose_env = Arc::clone(&env);
            let spawn_env = Arc::clone(&env);
            let now_env = env;
            let create = create.clone();
            let validate = Arc::clone(&validate);
            let dispose = Arc::clone(&dispose);
            let pool = Self {
                inner: Arc::new(Inner {
                    state: Mutex::new(State {
                        closed: false,
                        resources: usize::default(),
                        capacity,
                        idle: VecDeque::new(),
                        diagnostics: Vec::new(),
                        shutdown: Shutdown::Open,
                        population: Vec::new(),
                    }),
                    changed: Event::new(),
                    acquiring: Gate::new(),
                    cancellation: context.cancellation.child(),
                    ttl: idle_ttl,
                    now: Arc::new(move || now_env.clock().now()),
                    spawn: Arc::new(move |task| {
                        use crate::TaskSpawner;
                        spawn_env.task_spawner().spawn(task)
                    }),
                    create: Arc::new(move |context| {
                        let env = Arc::clone(&create_env);
                        let create = create.clone();
                        Box::pin(async move { runtime::evaluate_raw(create, env, &context).await })
                    }),
                    validate: Arc::new(move |value, context| {
                        let program = validate(value);
                        let env = Arc::clone(&validate_env);
                        Box::pin(
                            async move { runtime::evaluate_context(program, env, &context).await },
                        )
                    }),
                    dispose: Arc::new(move |value, context| {
                        let program = dispose(value);
                        let env = Arc::clone(&dispose_env);
                        Box::pin(
                            async move { runtime::evaluate_context(program, env, &context).await },
                        )
                    }),
                }),
            };
            let close = pool.clone();
            let close_context = context.protected();
            let abandoned = pool.clone();
            let finalizer = Registered::with_abandon(
                move |finish| {
                    Step::Suspended(Box::pin(async move {
                        let cleanup = close.shutdown_owned(close_context).await;
                        runtime::finish(finish, cleanup)
                    }))
                },
                move || abandoned.abandon(),
            );
            scope.register_or_release(finalizer, pool, finish)
        })
    }
    fn abandon(&self) {
        self.abandon_entry(None);
    }
    fn abandon_entry(&self, entry: Option<Arc<Entry<T, E>>>) {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;
            state
                .diagnostics
                .push(Exit::Die(Defect::from(LibraryDefect::ScopeOwnerDropped)));
            if let Some(entry) = entry {
                state.idle.push_back(entry);
            }
        }
        self.inner.acquiring.close();
        let _ = self.inner.cancellation.request();
        notify_all(&self.inner.changed);
    }
    async fn dispose_entry(&self, entry: Arc<Entry<T, E>>, context: Context) -> Exit<(), E> {
        let value = entry
            .value
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let result = match value {
            Some(value) => {
                AssertUnwindSafe(async { (self.inner.dispose)(value, context.protected()).await })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)))
            }
            None => Exit::Success(()),
        };
        drop(entry);
        result
    }
    async fn return_entry(
        &self,
        entry: Arc<Entry<T, E>>,
        retire: bool,
        context: Context,
    ) -> Exit<(), E> {
        let returned = catch_unwind(AssertUnwindSafe(|| (self.inner.now)()));
        if !retire && let Ok(returned_time) = returned.as_ref() {
            *entry
                .returned
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = *returned_time;
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let excess = match state.capacity {
                PoolCapacity::Unbounded => false,
                PoolCapacity::Bounded(limit) => state.resources > limit.get(),
            };
            if !state.closed && !excess {
                state.idle.push_back(entry);
                drop(state);
                notify_all(&self.inner.changed);
                return Exit::Success(());
            }
        }
        let cleanup = self.dispose_entry(entry, context).await;
        match returned {
            Ok(_) => cleanup,
            Err(panic) => outcome::append_cleanup(Exit::Die(Defect::from_panic(panic)), cleanup),
        }
    }
    async fn acquire_entry(&self, context: Context) -> Exit<Arc<Entry<T, E>>, E> {
        if self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed
        {
            return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
        }
        let mut turn = None;
        loop {
            if turn.is_none() {
                turn = Some(
                    match self
                        .inner
                        .acquiring
                        .enter(context.cancellation.clone())
                        .await
                    {
                        Ok(permit) => permit,
                        Err(GateError::Interrupted) => return Exit::Interrupted,
                        Err(GateError::Closed) => {
                            return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
                        }
                    },
                );
            }
            if context.cancelled() {
                return Exit::Interrupted;
            }
            let changed = self.inner.changed.listen();
            let selection = {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed {
                    return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
                }
                if let Some(entry) = state.idle.pop_front() {
                    Some(Ok(entry))
                } else {
                    let room = match state.capacity {
                        PoolCapacity::Unbounded => true,
                        PoolCapacity::Bounded(limit) => state.resources < limit.get(),
                    };
                    if room {
                        let Some(count) = state.resources.checked_add(NonZeroUsize::MIN.get())
                        else {
                            return Exit::Die(Defect::from(LibraryDefect::PoolCountOverflow));
                        };
                        state.resources = count;
                        Some(Err(Slot {
                            pool: Arc::downgrade(&self.inner),
                        }))
                    } else {
                        None
                    }
                }
            };
            match selection {
                Some(Err(slot)) => {
                    drop(turn.take());
                    let created =
                        AssertUnwindSafe(async { (self.inner.create)(context.clone()).await })
                            .catch_unwind()
                            .await
                            .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    match created.into_failure() {
                        Ok(value) => {
                            return Exit::Success(Arc::new(Entry {
                                value: RwLock::new(Some(value)),
                                returned: Mutex::new(Duration::ZERO),
                                slot,
                            }));
                        }
                        Err(failure) => {
                            drop(slot);
                            return failure;
                        }
                    }
                }
                Some(Ok(entry)) => {
                    drop(turn.take());
                    let validation = AssertUnwindSafe(async {
                        let expired = self.inner.ttl.is_some_and(|ttl| {
                            (self.inner.now)().saturating_sub(
                                *entry
                                    .returned
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                            ) >= ttl
                        });
                        if expired {
                            Exit::Success(false)
                        } else {
                            let future = {
                                let value = entry
                                    .value
                                    .read()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                match value.as_ref() {
                                    Some(value) => (self.inner.validate)(value, context.clone()),
                                    None => {
                                        return Exit::Die(Defect::from(
                                            LibraryDefect::PoolLeaseEnded,
                                        ));
                                    }
                                }
                            };
                            future.await
                        }
                    })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    match validation {
                        Exit::Success(true) => return Exit::Success(entry),
                        Exit::Success(false) => {
                            let cleanup = self.dispose_entry(entry, context.clone()).await;
                            if let Err(failure) = cleanup.into_failure() {
                                return failure;
                            }
                        }
                        failure @ (Exit::Fail(_)
                        | Exit::Die(_)
                        | Exit::Interrupted
                        | Exit::FinalizationFailed(_)) => {
                            let cleanup = self.dispose_entry(entry, context.clone()).await;
                            return outcome::append_cleanup(
                                failure.into_failure().err().unwrap_or(Exit::Interrupted),
                                cleanup,
                            );
                        }
                    }
                }
                None => match select(
                    Box::pin(changed),
                    Box::pin(context.cancellation.cancelled()),
                )
                .await
                {
                    Either::Left(((), _)) => {}
                    Either::Right(((), _)) => return Exit::Interrupted,
                },
            }
        }
    }
    pub fn acquire<R: Send + Sync + 'static>(
        &self,
        scope: Scope<E>,
    ) -> Effect<PoolLease<T, E>, E, R> {
        let pool = self.clone();
        Effect::new(move |_, context, finish| {
            let pool = pool.clone();
            let scope = scope.clone();
            Step::Suspended(Box::pin(async move {
                let operation = pool.inner.cancellation.child();
                let child = context.parallel(operation.clone());
                let acquired = match select(
                    Box::pin(pool.acquire_entry(child)),
                    Box::pin(context.cancellation.cancelled()),
                )
                .await
                {
                    Either::Left((outcome, _)) => outcome,
                    Either::Right(((), remaining)) => {
                        let _ = operation.request();
                        remaining.await
                    }
                };
                operation.unlink();
                match acquired.into_failure() {
                    Err(failure) => runtime::finish(finish, failure),
                    Ok(entry) => {
                        let lease = PoolLease {
                            inner: Arc::new(LeaseInner {
                                pool: pool.clone(),
                                state: Mutex::new(LeaseState {
                                    entry: Some(entry),
                                    phase: LeasePhase::Open,
                                }),
                                changed: Event::new(),
                            }),
                        };
                        let owned = lease.clone();
                        let cleanup_context = context.protected();
                        let abandoned = lease.clone();
                        let finalizer = Registered::with_abandon(
                            move |finish| {
                                Step::Suspended(Box::pin(async move {
                                    let result = owned.finalize(cleanup_context).await;
                                    runtime::finish(finish, result)
                                }))
                            },
                            move || abandoned.abandon(),
                        );
                        match scope.try_register(finalizer) {
                            Err(finalizer) => {
                                finalizer.execute(runtime::continuation(move |cleanup| {
                                    runtime::finish(
                                        finish,
                                        outcome::append_cleanup(
                                            Exit::Die(Defect::from(
                                                LibraryDefect::ScopeRegistrationClosed,
                                            )),
                                            cleanup,
                                        ),
                                    )
                                }))
                            }
                            Ok(())
                                if context.cancelled()
                                    || pool
                                        .inner
                                        .state
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                                        .closed =>
                            {
                                let cleanup = lease.finalize(context.protected()).await;
                                runtime::finish(
                                    finish,
                                    outcome::append_cleanup(Exit::Interrupted, cleanup),
                                )
                            }
                            Ok(()) => runtime::finish(finish, Exit::Success(lease)),
                        }
                    }
                }
            }))
        })
    }
    pub fn set_capacity<R: Send + Sync + 'static>(
        &self,
        capacity: PoolCapacity,
    ) -> Effect<(), E, R> {
        let pool = self.clone();
        Effect::ready(move |_, _| {
            let mut state = pool
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed {
                return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
            }
            state.capacity = capacity;
            drop(state);
            notify_all(&pool.inner.changed);
            Exit::Success(())
        })
    }
    pub fn exists_idle<R: Send + Sync + 'static>(
        &self,
        predicate: impl Fn(&T) -> bool + Send + Sync + 'static,
    ) -> Effect<bool, E, R> {
        let pool = self.clone();
        Effect::ready(move |_, context| {
            let idle = {
                let state = pool
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed {
                    return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
                }
                state.idle.iter().map(Arc::clone).collect::<Vec<_>>()
            };
            let found = idle.iter().any(|entry| {
                entry
                    .value
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .as_ref()
                    .is_some_and(&predicate)
            });
            if context.cancelled() {
                Exit::Interrupted
            } else {
                Exit::Success(found)
            }
        })
    }
    pub fn ensure_size<R: Send + Sync + 'static>(&self, target: usize) -> Effect<(), E, R> {
        let pool = self.clone();
        Effect::new(move |_, context, finish| {
            let pool = pool.clone();
            Step::Composed(Box::pin(async move {
                let work = async {
                    let (relevant, jobs) = {
                        let mut state = pool
                            .inner
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if state.closed {
                            return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
                        }
                        let maximum = match state.capacity {
                            PoolCapacity::Unbounded => target,
                            PoolCapacity::Bounded(limit) => target.min(limit.get()),
                        };
                        let mut relevant =
                            state.population.iter().map(Arc::clone).collect::<Vec<_>>();
                        let mut jobs = Vec::new();
                        while state.resources < maximum {
                            state.resources += NonZeroUsize::MIN.get();
                            let job = Arc::new(Population {
                                state: Mutex::new(PopulationState::Pending),
                                changed: Event::new(),
                            });
                            state.population.push(Arc::clone(&job));
                            relevant.push(Arc::clone(&job));
                            jobs.push((
                                job,
                                Slot {
                                    pool: Arc::downgrade(&pool.inner),
                                },
                            ));
                        }
                        (relevant, jobs)
                    };
                    // All real slots and completion owners exist before dispatch.
                    for (job, slot) in jobs {
                        let operation = pool.inner.cancellation.child();
                        let child_context = context.parallel(operation.clone());
                        let child_pool = pool.clone();
                        let completed = Arc::clone(&job);
                        let lost = Arc::clone(&job);
                        let task = crate::ScheduledTask::new(
                            move |settlement| {
                                Box::pin(async move {
                                    let work = async {
                                        let created =
                                            (child_pool.inner.create)(child_context.clone()).await;
                                        match created.into_failure() {
                                            Ok(value) => {
                                                child_pool
                                                    .return_entry(
                                                        Arc::new(Entry {
                                                            value: RwLock::new(Some(value)),
                                                            returned: Mutex::new(Duration::ZERO),
                                                            slot,
                                                        }),
                                                        child_context.cancelled(),
                                                        child_context,
                                                    )
                                                    .await
                                            }
                                            Err(failure) => {
                                                drop(slot);
                                                failure
                                            }
                                        }
                                    };
                                    let outcome =
                                        AssertUnwindSafe(work).catch_unwind().await.unwrap_or_else(
                                            |panic| Exit::Die(Defect::from_panic(panic)),
                                        );
                                    operation.unlink();
                                    settlement.release();
                                    completed.publish(outcome, false);
                                })
                            },
                            move |event| match event {
                                crate::execution::TaskEvent::Started => {}
                                crate::execution::TaskEvent::Lost(defect) => {
                                    lost.publish(Exit::Die(defect), false)
                                }
                            },
                        );
                        match catch_unwind(AssertUnwindSafe(|| (pool.inner.spawn)(task))) {
                            Ok(Ok(())) => {}
                            Ok(Err(defect)) => job.publish(Exit::Die(defect), true),
                            Err(panic) => job.publish(Exit::Die(Defect::from_panic(panic)), true),
                        }
                    }
                    let mut outcome = Exit::Success(());
                    for job in relevant {
                        loop {
                            let changed = job.changed.listen();
                            let completed = job.take();
                            if let Some(completed) = completed {
                                outcome = match outcome {
                                    Exit::Success(()) => completed,
                                    primary @ (Exit::Fail(_)
                                    | Exit::Die(_)
                                    | Exit::Interrupted
                                    | Exit::FinalizationFailed(_)) => {
                                        outcome::append_cleanup(primary, completed)
                                    }
                                };
                                break;
                            }
                            match select(
                                Box::pin(changed),
                                Box::pin(context.cancellation.cancelled()),
                            )
                            .await
                            {
                                Either::Left(((), _)) => {}
                                Either::Right(((), _)) => {
                                    return outcome::append_cleanup(Exit::Interrupted, outcome);
                                }
                            }
                        }
                    }
                    let retired = {
                        let mut state = pool
                            .inner
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let mut retired = Vec::new();
                        state.population.retain(|job| {
                            if job.observed() {
                                retired.push(Arc::clone(job));
                                false
                            } else {
                                true
                            }
                        });
                        retired
                    };
                    drop(retired);
                    outcome
                };
                let outcome = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }
    async fn shutdown_start(&self, context: Context) {
        let idle = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match state.shutdown {
                Shutdown::Open => {
                    state.closed = true;
                    state.shutdown = Shutdown::Closing;
                    Some(std::mem::take(&mut state.idle))
                }
                Shutdown::Closing | Shutdown::Closed(_) => None,
            }
        };
        if let Some(idle) = idle {
            let mut guard = ShutdownDriver {
                pool: self.clone(),
                settled: false,
            };
            let _ = self.inner.cancellation.request();
            self.inner.acquiring.close();
            notify_all(&self.inner.changed);
            let mut cleanup = Exit::Success(());
            for entry in idle {
                cleanup = outcome::append_cleanup(
                    cleanup,
                    self.dispose_entry(entry, context.clone()).await,
                );
            }
            loop {
                let changed = self.inner.changed.listen();
                let abandoned = std::mem::take(
                    &mut self
                        .inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .idle,
                );
                for entry in abandoned {
                    cleanup = outcome::append_cleanup(
                        cleanup,
                        self.dispose_entry(entry, context.clone()).await,
                    );
                }
                let done = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .resources
                    == usize::default();
                if done {
                    break;
                }
                changed.await;
            }
            let population = std::mem::take(
                &mut self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .population,
            );
            for job in population {
                loop {
                    let changed = job.changed.listen();
                    if let Some(outcome) = job.take() {
                        cleanup = outcome::append_cleanup(cleanup, outcome);
                        break;
                    }
                    changed.await;
                }
            }
            let diagnostics = std::mem::take(
                &mut self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .diagnostics,
            );
            for failure in diagnostics {
                cleanup = outcome::append_cleanup(cleanup, failure);
            }
            self.inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .shutdown = Shutdown::Closed(Some(cleanup));
            guard.settled = true;
            self.inner.cancellation.unlink();
            notify_all(&self.inner.changed);
        } else {
            loop {
                let changed = self.inner.changed.listen();
                if matches!(
                    self.inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .shutdown,
                    Shutdown::Closed(_)
                ) {
                    break;
                }
                changed.await;
            }
        }
    }
    async fn shutdown_owned(&self, context: Context) -> Exit<(), E> {
        self.shutdown_start(context).await;
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut state.shutdown {
            Shutdown::Closed(outcome) => outcome.take().unwrap_or(Exit::Success(())),
            Shutdown::Open | Shutdown::Closing => {
                Exit::Die(Defect::from(LibraryDefect::PoolShutdownDriverDropped))
            }
        }
    }
    pub fn shutdown<R: Send + Sync + 'static>(&self) -> Effect<PoolEndStatus, E, R> {
        let pool = self.clone();
        Effect::new(move |_, context, finish| {
            let pool = pool.clone();
            Step::Suspended(Box::pin(async move {
                pool.shutdown_start(context.protected()).await;
                let status = {
                    let state = pool
                        .inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    match &state.shutdown {
                        Shutdown::Closed(Some(Exit::Success(()))) | Shutdown::Closed(None) => {
                            PoolEndStatus::Clean
                        }
                        Shutdown::Open
                        | Shutdown::Closing
                        | Shutdown::Closed(Some(
                            Exit::Fail(_)
                            | Exit::Die(_)
                            | Exit::Interrupted
                            | Exit::FinalizationFailed(_),
                        )) => PoolEndStatus::Failed,
                    }
                };
                runtime::finish(finish, Exit::Success(status))
            }))
        })
        .uninterruptible()
    }
}
struct ShutdownDriver<T: Send + Sync + 'static, E: Send + 'static> {
    pool: Pool<T, E>,
    settled: bool,
}
impl<T: Send + Sync + 'static, E: Send + 'static> Drop for ShutdownDriver<T, E> {
    fn drop(&mut self) {
        if !self.settled {
            self.pool
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .shutdown = Shutdown::Closed(Some(Exit::Die(Defect::from(
                LibraryDefect::PoolShutdownDriverDropped,
            ))));
            notify_all(&self.pool.inner.changed);
        }
    }
}

enum PopulationState<E> {
    Pending,
    Settled(Option<Exit<(), E>>),
}
struct Population<E> {
    state: Mutex<PopulationState<E>>,
    changed: Event,
}
impl<E> Population<E> {
    fn publish(&self, outcome: Exit<(), E>, override_rejection: bool) {
        let discarded = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if matches!(*state, PopulationState::Pending) || override_rejection {
                Some(std::mem::replace(
                    &mut *state,
                    PopulationState::Settled(Some(outcome)),
                ))
            } else {
                None
            }
        };
        drop(discarded);
        notify_all(&self.changed);
    }
    fn take(&self) -> Option<Exit<(), E>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut *state {
            PopulationState::Pending => None,
            PopulationState::Settled(outcome) => Some(outcome.take().unwrap_or(Exit::Success(()))),
        }
    }
    fn observed(&self) -> bool {
        matches!(
            *self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            PopulationState::Settled(None)
        )
    }
}
