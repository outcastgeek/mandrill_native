//! Lexical resources over the existing prepared Scope phase/registered engine.
//! Empty guarded cells and actual finalizer/drain working sets are committed
//! BEFORE the root factory/acquisition. Unacquired entries never invoke release.
mod access;
mod closing;
mod spec;
use super::environment::Publisher;
use super::*;
use crate::internal::runtime_storage::Settlement;
use crate::scope::Registered;
use crate::{PreparedScopeClose, PreparedScopeLimits, PreparedScopePool, PrimaryOutcomeRef};
pub use access::Access;
pub(super) use closing::Closing;
use std::sync::Mutex;

pub struct ResourceValue<T: Send + Sync + 'static> {
    pub(super) view: Environment<T>,
}
impl<T: Send + Sync + 'static> ResourceValue<T> {
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            view: self.view.try_clone()?,
        })
    }
    pub fn with<A>(&self, access: impl FnOnce(&T) -> A) -> Result<A, Defect> {
        self.view
            .with(access)
            .map_err(|_| Defect::from(crate::LibraryDefect::ResourceScopeEnded))
    }
}
pub(super) struct ValueOwner<T: Send + Sync + 'static> {
    pub(super) value: Publisher<T>,
}
impl<T: Send + Sync + 'static> Drop for ValueOwner<T> {
    fn drop(&mut self) {
        drop(self.value.take());
    }
}
pub(super) struct Abandon<T: Send + Sync + 'static> {
    pub(super) value: Publisher<T>,
}
impl<T: Send + Sync + 'static> Consume<(), ()> for Abandon<T> {
    fn consume(self, (): ()) {
        drop(self.value.take());
    }
}
pub(super) struct Slot<T: Send + 'static> {
    value: Environment<Mutex<Option<T>>>,
}
impl<T: Send + 'static> Slot<T> {
    pub(super) fn prepare<V: Send + 'static, E: Send + 'static>(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self, MemoryError> {
        let (publisher, value) = Environment::reserve(frames, context)?;
        publisher.install(Mutex::new(None));
        Ok(Self { value })
    }
    pub(super) fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            value: self.value.try_clone()?,
        })
    }
    pub(super) fn put(&self, value: T) -> Result<(), Defect> {
        let old = self.value.with(|cell| {
            cell.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .replace(value)
        })?;
        drop(old);
        Ok(())
    }
    pub(super) fn take(&self) -> Result<Option<T>, Defect> {
        self.value.with(|cell| {
            cell.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
        })
    }
}
pub struct Resource<A, F, C> {
    acquire: A,
    release: F,
    marker: PhantomData<fn() -> C>,
}
pub fn resource<A, F, C>(acquire: A, release: F) -> Resource<A, F, C>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
{
    Resource {
        acquire,
        release,
        marker: PhantomData,
    }
}
impl<A, F, C> binding_sealed::Sealed for Resource<A, F, C>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
{
}
impl<A, F, C> Bindable for Resource<A, F, C>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
{
    type Value = ResourceValue<A::Value>;
    type Error = A::Error;
    type Env = A::Env;
    type Bound<B, N>
        = Using<A, F, C, B, N>
    where
        N: Description<Error = A::Error, Env = A::Env>,
        B: FnOnce(Self::Value) -> N + Send + 'static;
    fn bind<B, N>(self, body: B) -> Self::Bound<B, N>
    where
        N: Description<Error = A::Error, Env = A::Env>,
        B: FnOnce(Self::Value) -> N + Send + 'static,
    {
        Using {
            resource: self,
            body,
            marker: PhantomData,
        }
    }
}
pub struct Using<A, F, C, B, N> {
    resource: Resource<A, F, C>,
    body: B,
    marker: PhantomData<fn() -> N>,
}
pub struct Storage<AS, NS, CS, E: Send + 'static> {
    acquire: AS,
    body: NS,
    cleanup: CS,
    scopes: PreparedScopePool<E>,
}

struct Release<
    A: Description,
    F: Send + 'static,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    N: Description<Error = A::Error, Env = A::Env>,
    V: Send + 'static,
> where
    A::Value: Sync,
{
    value: Publisher<A::Value>,
    factory: Slot<F>,
    primary: Slot<Exit<N::Value, N::Error>>,
    admission: C::Admission<V>,
    completed: FrameReservation<'static, ReleaseFinish<N, V>>,
    failed: FrameReservation<'static, Invoke<(), N::Error>>,
    context: PreparedRunContext<V, N::Error>,
    panic: Option<crate::PreparedPanicReservation>,
    cleanup_buffer: crate::CleanupFailures<N::Error>,
}
struct ReleaseFinish<N: Description, V: Send + 'static> {
    primary: Exit<N::Value, N::Error>,
    slot: Slot<Exit<N::Value, N::Error>>,
    finish: Finish<(), N::Error>,
    _context: PreparedRunContext<V, N::Error>,
    cleanup_buffer: crate::CleanupFailures<N::Error>,
}
impl<N: Description, V: Send + 'static> Consume<Exit<(), N::Error>, Step> for ReleaseFinish<N, V> {
    fn consume(self, cleanup: Exit<(), N::Error>) -> Step {
        let cleanup = match self.slot.put(self.primary) {
            Ok(()) => cleanup,
            Err(defect) => {
                let mut accumulator =
                    crate::PreparedCleanupAccumulator::from_reserved(cleanup, self.cleanup_buffer);
                accumulator.append_cleanup(Exit::Die(defect));
                accumulator.finish()
            }
        };
        self.finish.call(cleanup).unwrap_or(Step::Done)
    }
}
impl<A, F, C, N, V> Consume<Finish<(), N::Error>, Step> for Release<A, F, C, N, V>
where
    A: Description,
    A::Value: Sync,
    N: Description<Error = A::Error, Env = A::Env>,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
    V: Send + 'static,
{
    fn consume(self, finish: Finish<(), N::Error>) -> Step {
        let Some(value) = self.value.take() else {
            // No acquisition committed a value; canonical empty-entry cleanup is
            // a no-op, not ResourceReleasedTwice and never a release invocation.
            return Step::More(Once::prepared(
                self.failed,
                Invoke {
                    outcome: Exit::Success(()),
                    finish,
                },
            ));
        };
        let primary = match self.primary.take() {
            Ok(Some(primary)) => primary,
            Ok(None) => {
                return Step::More(Once::prepared(
                    self.failed,
                    Invoke {
                        outcome: Exit::Die(Defect::from(
                            crate::LibraryDefect::MissingInterpreterOutcome,
                        )),
                        finish,
                    },
                ));
            }
            Err(defect) => {
                return Step::More(Once::prepared(
                    self.failed,
                    Invoke {
                        outcome: Exit::Die(defect),
                        finish,
                    },
                ));
            }
        };
        let factory = self.factory.take();
        let produced = match factory {
            Ok(Some(factory)) => {
                catch_unwind(AssertUnwindSafe(|| factory(value, primary.primary())))
                    .map_err(|panic| super::super::panic_defect(self.panic, panic))
            }
            Ok(None) => Err(Defect::from(crate::LibraryDefect::FutureFactoryConsumed)),
            Err(defect) => Err(defect),
        };
        let completed = Once::prepared(
            self.completed,
            ReleaseFinish {
                primary,
                slot: self.primary,
                finish,
                _context: self.context,
                cleanup_buffer: self.cleanup_buffer,
            },
        );
        match produced {
            Ok(cleanup) => cleanup.dispatch(self.admission, Continuation(completed)).0,
            Err(defect) => Step::More(Once::prepared(
                self.failed,
                Invoke {
                    outcome: Exit::Die(defect),
                    finish: completed,
                },
            )),
        }
    }
}
// Retain the actual context NODE, not only a root-row/output owner, through
// queued outcome/callback destruction. Transfer the pre-held owner, never clone.
struct Terminal<N: Description, V: Send + 'static> {
    outcome: Exit<N::Value, N::Error>,
    finish: Finish<N::Value, N::Error>,
    _context: PreparedRunContext<V, N::Error>,
}
impl<N: Description, V: Send + 'static> Consume<(), Step> for Terminal<N, V> {
    fn consume(self, (): ()) -> Step {
        self.finish.call(self.outcome).unwrap_or(Step::Done)
    }
}
struct FinishBody<N: Description, V: Send + 'static> {
    slot: Slot<Exit<N::Value, N::Error>>,
    close: PreparedScopeClose<(), N::Error, V>,
    after: FrameReservation<'static, AfterClose<N, V>>,
    terminal: FrameReservation<'static, Terminal<N, V>>,
    finish: Finish<N::Value, N::Error>,
    context: PreparedRunContext<V, N::Error>,
    cleanup_buffer: crate::CleanupFailures<N::Error>,
}
struct AfterClose<N: Description, V: Send + 'static> {
    slot: Slot<Exit<N::Value, N::Error>>,
    terminal: FrameReservation<'static, Terminal<N, V>>,
    finish: Finish<N::Value, N::Error>,
    context: PreparedRunContext<V, N::Error>,
    cleanup_buffer: crate::CleanupFailures<N::Error>,
}
impl<N: Description, V: Send + 'static> Consume<Exit<N::Value, N::Error>, Step>
    for FinishBody<N, V>
{
    fn consume(self, result: Exit<N::Value, N::Error>) -> Step {
        // Publication follows guarded raw success, before cleanup; the body exit
        // remains owned (not cloned) so release may inspect borrowed primary.
        let result = self.context.publish(result);
        if let Err(defect) = self.slot.put(result) {
            return Step::More(Once::prepared(
                self.terminal,
                Terminal {
                    outcome: Exit::Die(defect),
                    finish: self.finish,
                    _context: self.context,
                },
            ));
        }
        let after = Once::prepared(
            self.after,
            AfterClose {
                slot: self.slot,
                terminal: self.terminal,
                finish: self.finish,
                context: self.context,
                cleanup_buffer: self.cleanup_buffer,
            },
        );
        self.close
            .dispatch(Exit::Success(()), Continuation(after))
            .0
    }
}
impl<N: Description, V: Send + 'static> Consume<Exit<(), N::Error>, Step> for AfterClose<N, V> {
    fn consume(self, cleanup: Exit<(), N::Error>) -> Step {
        let primary = match self.slot.take() {
            Ok(Some(primary)) => primary,
            Ok(None) => Exit::Die(Defect::from(
                crate::LibraryDefect::MissingInterpreterOutcome,
            )),
            Err(defect) => Exit::Die(defect),
        };
        let mut accumulator = crate::PreparedCleanupAccumulator::from_reserved(
            self.context.publish(primary),
            self.cleanup_buffer,
        );
        accumulator.append_cleanup(cleanup);
        let outcome = accumulator.finish();
        Step::More(Once::prepared(
            self.terminal,
            Terminal {
                outcome,
                finish: self.finish,
                _context: self.context,
            },
        ))
    }
}
struct Acquired<
    A: Description,
    B: Send + 'static,
    N: Description<Error = A::Error, Env = A::Env>,
    V: Send + 'static,
> where
    A::Value: Sync,
{
    body: B,
    value: Publisher<A::Value>,
    handle: ResourceValue<A::Value>,
    admission: N::Admission<V>,
    start: FrameReservation<'static, StartBody<A, B, N, V>>,
    failed: FrameReservation<'static, Invoke<N::Value, N::Error>>,
    finish: Finish<N::Value, N::Error>,
    context: PreparedRunContext<V, N::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct StartBody<
    A: Description,
    B: Send + 'static,
    N: Description<Error = A::Error, Env = A::Env>,
    V: Send + 'static,
> where
    A::Value: Sync,
{
    body: B,
    handle: ResourceValue<A::Value>,
    admission: N::Admission<V>,
    failed: FrameReservation<'static, Invoke<N::Value, N::Error>>,
    finish: Finish<N::Value, N::Error>,
    context: PreparedRunContext<V, N::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<A, B, N, V> Consume<Exit<A::Value, A::Error>, Step> for Acquired<A, B, N, V>
where
    A: Description,
    A::Value: Sync,
    N: Description<Error = A::Error, Env = A::Env>,
    B: FnOnce(ResourceValue<A::Value>) -> N + Send + 'static,
    V: Send + 'static,
{
    fn consume(self, acquired: Exit<A::Value, A::Error>) -> Step {
        let failure = match acquired {
            Exit::Success(value) => {
                // Install actual native T into its pre-owned guarded cell BEFORE
                // cancellation, publication or user body construction.
                self.value.install(value);
                return Step::More(Once::prepared(
                    self.start,
                    StartBody::<A, B, N, V> {
                        body: self.body,
                        handle: self.handle,
                        admission: self.admission,
                        failed: self.failed,
                        finish: self.finish,
                        context: self.context,
                        panic: self.panic,
                    },
                ));
            }
            Exit::Fail(error) => Exit::Fail(error),
            Exit::Die(defect) => Exit::Die(defect),
            Exit::Interrupted => Exit::Interrupted,
            Exit::FinalizationFailed(failure) => Exit::FinalizationFailed(failure),
        };
        Step::More(Once::prepared(
            self.failed,
            Invoke {
                outcome: failure,
                finish: self.finish,
            },
        ))
    }
}
impl<A, B, N, V> Consume<(), Step> for StartBody<A, B, N, V>
where
    A: Description,
    A::Value: Sync,
    N: Description<Error = A::Error, Env = A::Env>,
    B: FnOnce(ResourceValue<A::Value>) -> N + Send + 'static,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let body = if self.context.is_cancelled() {
            Err(Exit::Interrupted)
        } else {
            catch_unwind(AssertUnwindSafe(|| (self.body)(self.handle)))
                .map_err(|panic| Exit::Die(super::super::panic_defect(self.panic, panic)))
        };
        match body {
            Ok(body) => body.dispatch(self.admission, Continuation(self.finish)).0,
            Err(outcome) => Step::More(Once::prepared(
                self.failed,
                Invoke {
                    outcome,
                    finish: self.finish,
                },
            )),
        }
    }
}
pub struct Admission<
    A: Description,
    F: Send + 'static,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    B: Send + 'static,
    N: Description<Error = A::Error, Env = A::Env>,
    V: Send + 'static,
> where
    A::Value: Sync,
{
    entry: FrameReservation<'static, Entry<A, F, C, B, N, V>>,
    controls: Controls<A, F, C, B, N, V>,
}
struct Controls<
    A: Description,
    F: Send + 'static,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    B: Send + 'static,
    N: Description<Error = A::Error, Env = A::Env>,
    V: Send + 'static,
> where
    A::Value: Sync,
{
    acquire: A::Admission<V>,
    body: N::Admission<V>,
    factory: Slot<F>,
    primary: Slot<Exit<N::Value, N::Error>>,
    value: Publisher<A::Value>,
    handle: ResourceValue<A::Value>,
    acquired: FrameReservation<'static, Acquired<A, B, N, V>>,
    start: FrameReservation<'static, StartBody<A, B, N, V>>,
    failed: FrameReservation<'static, Invoke<N::Value, N::Error>>,
    finished: FrameReservation<'static, FinishBody<N, V>>,
    after: FrameReservation<'static, AfterClose<N, V>>,
    terminal: FrameReservation<'static, Terminal<N, V>>,
    close: PreparedScopeClose<(), N::Error, V>,
    cleanup_buffer: crate::CleanupFailures<N::Error>,
    acquire_context: PreparedRunContext<V, N::Error>,
    body_panic: Option<crate::PreparedPanicReservation>,
    context: PreparedRunContext<V, N::Error>,
    marker: PhantomData<fn() -> C>,
}
struct Entry<
    A: Description,
    F: Send + 'static,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    B: Send + 'static,
    N: Description<Error = A::Error, Env = A::Env>,
    V: Send + 'static,
> where
    A::Value: Sync,
{
    description: Using<A, F, C, B, N>,
    finish: Finish<N::Value, N::Error>,
    controls: Controls<A, F, C, B, N, V>,
}
impl<A, F, C, B, N, V> Consume<(), Step> for Entry<A, F, C, B, N, V>
where
    A: Description,
    A::Value: Sync,
    N: Description<Error = A::Error, Env = A::Env>,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
    B: FnOnce(ResourceValue<A::Value>) -> N + Send + 'static,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let controls = self.controls;
        let finished = Once::prepared(
            controls.finished,
            FinishBody {
                slot: controls.primary,
                close: controls.close,
                after: controls.after,
                terminal: controls.terminal,
                cleanup_buffer: controls.cleanup_buffer,
                finish: self.finish,
                context: controls.context,
            },
        );
        if let Err(defect) = controls.factory.put(self.description.resource.release) {
            return Step::More(Once::prepared(
                controls.failed,
                Invoke {
                    outcome: Exit::Die(defect),
                    finish: finished,
                },
            ));
        }
        let acquired = Once::prepared(
            controls.acquired,
            Acquired::<A, B, N, V> {
                body: self.description.body,
                value: controls.value,
                handle: controls.handle,
                admission: controls.body,
                start: controls.start,
                failed: controls.failed,
                finish: finished,
                context: controls.acquire_context,
                panic: controls.body_panic,
            },
        );
        self.description
            .resource
            .acquire
            .dispatch(controls.acquire, Continuation(acquired))
            .0
    }
}
