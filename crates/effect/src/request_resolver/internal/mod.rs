use super::{BatchRunner, HasRequestScope, RequestResolverError, RequestScope};
use crate::internal::notification::notify_all;
use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Step};
use crate::{
    Defect, Effect, Exit, Fiber, HasTaskSpawner, LibraryDefect, Request, RequestGroup, RequestKey,
    Scope, SharedValue,
};
use event_listener::Event;
use futures_util::{
    FutureExt,
    future::{Either, select},
};
use std::collections::{BTreeMap, HashMap};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, Weak};

type SharedResult<A, E> = Exit<SharedValue<A>, RequestResolverError<E>>;
type GroupedRequests<Q, A, E> = BTreeMap<RequestGroup, Vec<(Q, Arc<Pending<A, E>>)>>;

struct Registry<A: Send + 'static, E: Send + 'static> {
    closed: bool,
    pending: HashMap<RequestKey, Arc<Pending<A, E>>>,
}

struct PendingState<A: Send + 'static, E: Send + 'static> {
    outcome: Option<SharedResult<A, E>>,
    fiber: Option<Fiber<(), RequestResolverError<E>>>,
    admission_done: bool,
}

struct Pending<A: Send + 'static, E: Send + 'static> {
    key: RequestKey,
    group: RequestGroup,
    state: Mutex<PendingState<A, E>>,
    changed: Event,
}

impl<A: Send + 'static, E: Send + 'static> Pending<A, E> {
    fn new(key: RequestKey, group: RequestGroup) -> Self {
        Self {
            key,
            group,
            state: Mutex::new(PendingState {
                outcome: None,
                fiber: None,
                admission_done: false,
            }),
            changed: Event::new(),
        }
    }

    fn complete(&self, outcome: SharedResult<A, E>) {
        let discarded = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.outcome.is_none() {
                state.outcome = Some(outcome);
                None
            } else {
                Some(outcome)
            }
        };
        if let Some(discarded) = discarded {
            drop(discarded);
        } else {
            notify_all(&self.changed);
        }
    }

    fn assign_fiber(&self, fiber: Fiber<(), RequestResolverError<E>>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.fiber = Some(fiber);
        state.admission_done = true;
        drop(state);
        notify_all(&self.changed);
    }

    fn reject(&self, outcome: SharedResult<A, E>) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .admission_done = true;
        self.complete(outcome);
    }

    fn completed(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .outcome
            .is_some()
    }

    async fn observe<R: Send + Sync + 'static>(
        &self,
        env: Arc<R>,
        context: &Context,
    ) -> SharedResult<A, E> {
        loop {
            let changed = self.changed.listen();
            if context.cancelled() {
                return Exit::Interrupted;
            }
            let (outcome, fiber, admission_done) = {
                let state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (
                    state.outcome.clone(),
                    state.fiber.clone(),
                    state.admission_done,
                )
            };
            if let Some(outcome) = outcome {
                // A batch-level error is owned by its child until one observer
                // marks it observed. Request-level errors are data in a
                // successful batch and need no child failure observation.
                let child_failure = !matches!(
                    &outcome,
                    Exit::Success(_) | Exit::Fail(RequestResolverError::RequestFailed(_))
                );
                if !child_failure || admission_done {
                    if child_failure && let Some(fiber) = &fiber {
                        let protected = context.protected();
                        let seen = runtime::evaluate_context(
                            fiber.await_exit_shared::<R>(),
                            env,
                            &protected,
                        )
                        .await;
                        return match seen.into_failure::<()>() {
                            Ok(_) => outcome,
                            Err(failure) => outcome::append_cleanup(outcome, failure),
                        };
                    }
                    return outcome;
                }
            }
            if let Some(fiber) = fiber {
                if fiber.settled().now_or_never().is_some() {
                    let protected = context.protected();
                    let seen =
                        runtime::evaluate_context(fiber.await_exit_shared::<R>(), env, &protected)
                            .await;
                    let result = match seen.into_failure::<SharedValue<A>>() {
                        Ok(shared) => match shared.with(Clone::clone) {
                            Ok(Exit::Success(())) => Exit::Fail(RequestResolverError::Uncompleted),
                            Ok(Exit::Fail(error)) => Exit::Fail(error),
                            Ok(Exit::Die(defect)) => Exit::Die(defect),
                            Ok(Exit::Interrupted) => Exit::Interrupted,
                            Ok(Exit::FinalizationFailed(details)) => {
                                Exit::FinalizationFailed(details)
                            }
                            Err(defect) => Exit::Die(defect),
                        },
                        Err(failure) => failure,
                    };
                    self.complete(result.clone());
                    return result;
                }
                let finished = std::pin::pin!(fiber.settled());
                let wake = std::pin::pin!(select(changed, finished));
                let cancelled = std::pin::pin!(context.cancellation.cancelled());
                if matches!(select(wake, cancelled).await, Either::Right(_)) {
                    return Exit::Interrupted;
                }
            } else {
                let cancelled = std::pin::pin!(context.cancellation.cancelled());
                if matches!(select(changed, cancelled).await, Either::Right(_)) {
                    return Exit::Interrupted;
                }
            }
        }
    }
}

pub(super) struct ResolverState<
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
> {
    scope: Scope<RequestResolverError<E>>,
    request_scope: RequestScope,
    run: Arc<BatchRunner<Q, A, E, R>>,
    registry: Mutex<Registry<A, E>>,
}

impl<
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
> ResolverState<Q, A, E, R>
{
    pub(super) fn new(
        scope: Scope<RequestResolverError<E>>,
        request_scope: RequestScope,
        run: Arc<BatchRunner<Q, A, E, R>>,
    ) -> Self {
        Self {
            scope,
            request_scope,
            run,
            registry: Mutex::new(Registry {
                closed: false,
                pending: HashMap::new(),
            }),
        }
    }

    pub(super) fn close(&self) {
        let pending = {
            let mut registry = self
                .registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registry.closed = true;
            std::mem::take(&mut registry.pending)
        };
        for entry in pending.into_values() {
            entry.reject(Exit::Fail(RequestResolverError::Closed));
        }
    }

    fn forget(&self, entry: &Arc<Pending<A, E>>) {
        let removed = {
            let mut registry = self
                .registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if registry
                .pending
                .get(&entry.key)
                .is_some_and(|current| Arc::ptr_eq(current, entry))
            {
                registry.pending.remove(&entry.key)
            } else {
                None
            }
        };
        drop(removed);
    }

    pub(super) async fn resolve(
        self: Arc<Self>,
        requests: Vec<Request<Q, A, E>>,
        env: Arc<R>,
        context: Context,
    ) -> Exit<Vec<SharedResult<A, E>>, RequestResolverError<E>> {
        if !self.request_scope.same(env.request_scope()) {
            return Exit::Fail(RequestResolverError::WrongEnvironment);
        }
        if context.cancelled() {
            return Exit::Interrupted;
        }
        let (selected, groups, discard) = {
            let mut registry = self
                .registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if registry.closed {
                return Exit::Fail(RequestResolverError::Closed);
            }
            let mut seen = HashMap::new();
            for request in &requests {
                if let Some(group) = seen.insert(request.key().clone(), request.group().clone())
                    && group != *request.group()
                {
                    return Exit::Fail(RequestResolverError::ConflictingGroup(
                        request.key().clone(),
                    ));
                }
                if registry
                    .pending
                    .get(request.key())
                    .is_some_and(|entry| entry.group != *request.group())
                {
                    return Exit::Fail(RequestResolverError::ConflictingGroup(
                        request.key().clone(),
                    ));
                }
            }
            let mut selected = Vec::with_capacity(requests.len());
            let mut groups: GroupedRequests<Q, A, E> = BTreeMap::new();
            let mut discard = Vec::new();
            for request in requests {
                let (key, group, payload) = request.into_parts();
                let entry = match registry.pending.get(&key) {
                    Some(entry) => {
                        discard.push(payload);
                        Arc::clone(entry)
                    }
                    None => {
                        let entry = Arc::new(Pending::new(key.clone(), group.clone()));
                        registry.pending.insert(key, Arc::clone(&entry));
                        groups
                            .entry(group)
                            .or_default()
                            .push((payload, Arc::clone(&entry)));
                        entry
                    }
                };
                selected.push(entry);
            }
            (selected, groups, discard)
        };

        for (group, batch) in groups {
            let entries = batch
                .iter()
                .map(|(_, entry)| Arc::clone(entry))
                .collect::<Vec<_>>();
            let program = batch_effect(group, batch, Arc::clone(&self.run), Arc::downgrade(&self));
            let protected = context.protected();
            let spawned =
                runtime::evaluate_context(self.scope.fork(program), Arc::clone(&env), &protected)
                    .await;
            match spawned.into_failure::<SharedValue<A>>() {
                Ok(fiber) => {
                    for entry in &entries {
                        entry.assign_fiber(fiber.clone());
                    }
                }
                Err(failure) => {
                    for entry in &entries {
                        entry.reject(failure.clone());
                        self.forget(entry);
                    }
                }
            }
        }
        drop(discard);

        let mut results = Vec::with_capacity(selected.len());
        for entry in selected {
            let result = entry.observe(Arc::clone(&env), &context).await;
            if entry.completed() {
                self.forget(&entry);
            }
            results.push(result);
        }
        Exit::Success(results)
    }
}

fn share_request<A: Send + 'static, E: Send + 'static>(outcome: Exit<A, E>) -> SharedResult<A, E> {
    match outcome.map_error(|error| RequestResolverError::RequestFailed(SharedValue::new(error))) {
        Exit::Success(value) => Exit::Success(SharedValue::new(value)),
        Exit::Fail(error) => Exit::Fail(error),
        Exit::Die(defect) => Exit::Die(defect),
        Exit::Interrupted => Exit::Interrupted,
        Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
    }
}

fn failed<B, C, E>(outcome: Exit<B, E>) -> Exit<C, E> {
    match outcome.into_failure() {
        Ok(_) => Exit::Die(Defect::from(
            LibraryDefect::RequestResolverUnexpectedSuccess,
        )),
        Err(failure) => failure,
    }
}

fn batch_effect<Q, A, E, R>(
    group: RequestGroup,
    batch: Vec<(Q, Arc<Pending<A, E>>)>,
    run: Arc<BatchRunner<Q, A, E, R>>,
    owner: Weak<ResolverState<Q, A, E, R>>,
) -> Effect<(), RequestResolverError<E>, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
{
    let batch = Mutex::new(Some(batch));
    Effect::new(move |env, context, finish| {
        let batch = batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(batch) = batch else {
            return runtime::finish(finish, Exit::Fail(RequestResolverError::InputConsumed));
        };
        let entries = batch
            .iter()
            .map(|(_, entry)| Arc::clone(entry))
            .collect::<Vec<_>>();
        let payloads = batch
            .into_iter()
            .map(|(payload, _)| payload)
            .collect::<Vec<_>>();
        let group = group.clone();
        let run = Arc::clone(&run);
        let owner = owner.clone();
        Step::Suspended(Box::pin(async move {
            let work = async {
                let program = run(group, payloads);
                let outcome = runtime::evaluate_context(program, env, &context).await;
                match outcome {
                    Exit::Success(outcomes) if outcomes.len() == entries.len() => {
                        for (entry, result) in entries.iter().zip(outcomes) {
                            entry.complete(share_request(result));
                        }
                        Exit::Success(())
                    }
                    Exit::Success(outcomes) => {
                        let error = RequestResolverError::CardinalityMismatch {
                            expected: entries.len(),
                            actual: outcomes.len(),
                        };
                        for entry in &entries {
                            entry.complete(Exit::Fail(error.clone()));
                        }
                        Exit::Fail(error)
                    }
                    failure @ (Exit::Fail(_)
                    | Exit::Die(_)
                    | Exit::Interrupted
                    | Exit::FinalizationFailed(_)) => {
                        let failure = failed::<_, (), _>(failure).map_error(|error| {
                            RequestResolverError::BatchFailed(SharedValue::new(error))
                        });
                        for entry in &entries {
                            entry.complete(failed::<_, SharedValue<A>, _>(failure.clone()));
                        }
                        failure
                    }
                }
            };
            let outcome = AssertUnwindSafe(work)
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| {
                    let defect = Defect::from_panic(panic);
                    for entry in &entries {
                        entry.complete(Exit::Die(defect.clone()));
                    }
                    Exit::Die(defect)
                });
            if let Some(owner) = owner.upgrade() {
                for entry in &entries {
                    owner.forget(entry);
                }
            }
            runtime::finish(finish, outcome)
        }))
    })
}
