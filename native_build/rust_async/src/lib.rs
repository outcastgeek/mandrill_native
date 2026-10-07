//! Owned Tokio execution and cancellation for consumer-defined native futures.
//! No payload schema, database policy, global runtime or callback thread is imposed.
pub mod foreign;

use futures_util::FutureExt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::sync::Notify;

const CANCELLED: u8 = 1;
const TERMINAL: u8 = 2;

/// One workflow's persistent cancellation request. Ordinary Rust consumers may
/// observe/request cancellation but cannot prematurely mark native work finished.
///
/// ```compile_fail
/// use native_async_runtime::Cancellation;
/// let token = Cancellation::new();
/// token.finish(); // terminal arbitration is private to the FFI owner
/// ```
#[derive(Default)]
pub struct Cancellation {
    state: AtomicU8,
    changed: Notify,
}

impl Cancellation {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Atomically request cancellation only while completion has not claimed the terminal state.
    ///
    /// Returns false if terminal completion already won; accepted requests wake
    /// cancellation waiters. Repeated requests remain idempotent and do not acquire
    /// execution or drain ownership.
    #[must_use]
    pub fn request(&self) -> bool {
        let result = self
            .state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                if state & TERMINAL != 0 {
                    None
                } else {
                    Some(state | CANCELLED)
                }
            });
        if result.is_ok() {
            self.changed.notify_waiters();
        }
        result.is_ok()
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) & CANCELLED != 0
    }

    /// Lost-wakeup safe even when cancellation precedes future registration.
    pub async fn cancelled(&self) {
        loop {
            let notification = self.changed.notified();
            tokio::pin!(notification);
            notification.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notification.await;
        }
    }

    /// Called once by the owner after quiescence. The returned flag determines
    /// whether cancellation won before terminal completion was selected.
    #[must_use]
    fn finish(&self) -> bool {
        self.state.fetch_or(TERMINAL, Ordering::AcqRel) & CANCELLED != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Saturated,
    Stopping,
}

#[derive(Debug)]
pub enum Completion<T, E> {
    Success(T),
    Failure(E),
    Panicked,
}

struct Admission {
    // High bit stops admission; lower bits count accepted work through callback return.
    state: AtomicUsize,
    limit: usize,
    drained: Notify,
}
const STOPPING: usize = 1 << (usize::BITS - 1);

struct Permit(Arc<Admission>);
impl Permit {
    /// Atomically reserve one operation below the configured limit unless stopping has begun; the returned permit owns release and drain notification.
    fn acquire(admission: &Arc<Admission>) -> Result<Self, AdmissionError> {
        admission
            .state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                if state & STOPPING != 0 || state >= admission.limit {
                    None
                } else {
                    Some(state + 1)
                }
            })
            .map_err(|state| {
                if state & STOPPING != 0 {
                    AdmissionError::Stopping
                } else {
                    AdmissionError::Saturated
                }
            })?;
        Ok(Self(Arc::clone(admission)))
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let previous = self.0.state.fetch_sub(1, Ordering::AcqRel);
        if previous & !STOPPING == 1 {
            self.0.drained.notify_waiters();
        }
    }
}

/// Create at native initialization and retain across requests. Drop only from
/// the owner's shutdown context after `stop().wait()`; not a Tokio callback.
pub struct RuntimeOwner {
    runtime: Runtime,
    controller: RuntimeController,
}

/// Admission/shutdown authority, retained by the runtime's controller rather
/// than distributed to task submitters. This type is deliberately not Clone.
pub struct RuntimeController {
    handle: RuntimeHandle,
}

/// Cloneable submission authority for one designated owner. Clones share both
/// Tokio scheduling and admission accounting; dropping one never shuts it down.
/// A submitter cannot stop other consumers or invoke synchronous waiting:
/// ```compile_fail
/// use native_async_runtime::RuntimeHandle;
/// fn wrong(handle: &RuntimeHandle) { handle.stop(); }
/// ```
/// ```compile_fail
/// use native_async_runtime::RuntimeHandle;
/// fn wrong(handle: &RuntimeHandle) { handle.block_on_sync(async {}); }
/// ```
#[derive(Clone)]
pub struct RuntimeHandle {
    runtime: Handle,
    admission: Arc<Admission>,
    parent: Option<Arc<Admission>>,
}

/// Admission has stopped. This non-cloneable state is the only API for waiting
/// for quiescence; a running handle cannot accidentally await an open producer.
#[must_use = "await wait() or arrange notify() before destroying the owner"]
pub struct Draining {
    handle: RuntimeHandle,
}

impl RuntimeOwner {
    pub fn new(worker_threads: usize, max_operations: usize) -> std::io::Result<Self> {
        if worker_threads == 0 || max_operations == 0 || max_operations >= STOPPING {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid runtime capacity",
            ));
        }
        let runtime = Builder::new_multi_thread()
            .worker_threads(worker_threads)
            .enable_all()
            .build()?;
        let controller = RuntimeController::attach(runtime.handle().clone(), max_operations)?;
        Ok(Self {
            runtime,
            controller,
        })
    }

    /// Independent service admission/drain on this same runtime. Parent
    /// accounting includes every child operation; stopping one service does not
    /// stop others, while stopping the owner rejects new child operations too.
    pub fn controller(&self, max_operations: usize) -> std::io::Result<RuntimeController> {
        let mut controller =
            RuntimeController::attach(self.runtime.handle().clone(), max_operations)?;
        controller.handle.parent = Some(Arc::clone(&self.controller.handle.admission));
        Ok(controller)
    }

    #[must_use]
    pub fn handle(&self) -> RuntimeHandle {
        self.controller.handle()
    }

    #[must_use = "await the drain before destroying the runtime owner"]
    pub fn stop(&self) -> Draining {
        self.controller.stop()
    }

    #[must_use]
    pub fn active(&self) -> usize {
        self.controller.handle.active()
    }

    /// Only for an explicitly synchronous native contract or shutdown coordinator.
    /// Async bridges must use the submission handle instead.
    pub fn block_on_sync<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }
}

impl RuntimeController {
    /// Attach once to a consumer-owned Tokio runtime. Distribute `handle()`
    /// clones to submitters and retain this controller for shutdown. The external
    /// owner must keep its runtime alive through stop/wait and resource cleanup.
    pub fn attach(runtime: Handle, max_operations: usize) -> std::io::Result<Self> {
        if max_operations == 0 || max_operations >= STOPPING {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid runtime capacity",
            ));
        }
        Ok(Self {
            handle: RuntimeHandle {
                runtime,
                parent: None,
                admission: Arc::new(Admission {
                    state: AtomicUsize::new(0),
                    limit: max_operations,
                    drained: Notify::new(),
                }),
            },
        })
    }

    #[must_use]
    pub fn handle(&self) -> RuntimeHandle {
        self.handle.clone()
    }

    #[must_use = "observe quiescence through the returned drain state"]
    pub fn stop(&self) -> Draining {
        self.handle
            .admission
            .state
            .fetch_or(STOPPING, Ordering::AcqRel);
        Draining {
            handle: self.handle.clone(),
        }
    }
}

impl RuntimeHandle {
    #[must_use]
    pub fn worker_count(&self) -> usize {
        self.runtime.metrics().num_workers()
    }

    /// Admission never waits for capacity. `create` is invoked on a Tokio worker,
    /// and owns everything its future can reach. The completion callback must
    /// publish/enqueue bounded work and return without blocking the Tokio worker.
    /// Cancellation cleanup belongs in the consumer future: this method never
    /// drops a database future behind the consumer's back.
    ///
    /// Non-Send payloads cannot cross executor threads:
    /// ```compile_fail
    /// use native_async_runtime::{Cancellation, RuntimeHandle};
    /// use std::rc::Rc;
    /// fn wrong(handle: &RuntimeHandle) {
    ///     let data = Rc::new(42);
    ///     handle.submit(Cancellation::new(), move |_| async move {
    ///         Ok::<_, ()>(*data)
    ///     }, |_| {}).unwrap();
    /// }
    /// ```
    /// Borrowed stack storage cannot escape into an asynchronous operation:
    /// ```compile_fail
    /// use native_async_runtime::{Cancellation, RuntimeHandle};
    /// fn wrong(handle: &RuntimeHandle) {
    ///     let data = String::from("owned by this stack frame");
    ///     let borrowed = &data;
    ///     handle.submit(Cancellation::new(), move |_| async move {
    ///         Ok::<_, ()>(borrowed.len())
    ///     }, |_| {}).unwrap();
    /// }
    /// ```
    pub fn submit<Create, F, T, E, Done>(
        &self,
        cancel: Arc<Cancellation>,
        create: Create,
        done: Done,
    ) -> Result<(), AdmissionError>
    where
        Create: FnOnce(Arc<Cancellation>) -> F + Send + 'static,
        F: Future<Output = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
        Done: FnOnce(Completion<T, E>) + Send + 'static,
    {
        self.reserve(cancel)?.submit(create, done);
        Ok(())
    }

    /// Reserve before native preparation; consuming submit needs no second
    /// admission. Dropping unused preparation releases its capacity and drain.
    pub fn reserve(&self, cancel: Arc<Cancellation>) -> Result<ReservedOperation, AdmissionError> {
        let parent = self.parent.as_ref().map(Permit::acquire).transpose()?;
        let permit = Permit::acquire(&self.admission)?;
        Ok(ReservedOperation {
            runtime: self.runtime.clone(),
            permit,
            parent,
            cancel,
        })
    }

    #[must_use]
    pub fn active(&self) -> usize {
        self.admission.state.load(Ordering::Acquire) & !STOPPING
    }
}

/// Accepted operation ownership, including preparation before Rust submission.
/// It can be consumed once even after admission stops; shutdown waits for it.
/// ```compile_fail
/// use native_async_runtime::ReservedOperation;
/// fn wrong(reservation: ReservedOperation) {
///     reservation.submit(|_| async { Ok::<_, ()>(()) }, |_| {});
///     reservation.submit(|_| async { Ok::<_, ()>(()) }, |_| {});
/// }
/// ```
#[must_use = "submit the accepted operation or drop it to release admission"]
pub struct ReservedOperation {
    runtime: Handle,
    permit: Permit,
    parent: Option<Permit>,
    cancel: Arc<Cancellation>,
}

impl ReservedOperation {
    pub fn submit<Create, F, T, E, Done>(self, create: Create, done: Done)
    where
        Create: FnOnce(Arc<Cancellation>) -> F + Send + 'static,
        F: Future<Output = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
        Done: FnOnce(Completion<T, E>) + Send + 'static,
    {
        let Self {
            runtime,
            permit,
            parent,
            cancel,
        } = self;
        drop(runtime.spawn(async move {
            let result = AssertUnwindSafe(async move { create(cancel).await })
                .catch_unwind()
                .await;
            let outcome = match result {
                Ok(Ok(value)) => Completion::Success(value),
                Ok(Err(error)) => Completion::Failure(error),
                Err(_) => Completion::Panicked,
            };
            done(outcome);
            drop(permit);
            drop(parent);
        }));
    }
}

impl Draining {
    /// Consume shutdown observation exactly once. The returned future only
    /// becomes ready after accepted futures and completion callbacks return.
    ///
    /// ```compile_fail
    /// use native_async_runtime::RuntimeOwner;
    /// async fn wrong(owner: &RuntimeOwner) {
    ///     owner.handle().wait().await; // stop admission first
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use native_async_runtime::RuntimeOwner;
    /// async fn wrong(owner: &RuntimeOwner) {
    ///     let drain = owner.stop();
    ///     drain.wait().await;
    ///     drain.wait().await; // moved; no double consumption
    /// }
    /// ```
    pub async fn wait(self) {
        loop {
            let notification = self.handle.admission.drained.notified();
            tokio::pin!(notification);
            notification.as_mut().enable();
            if self.handle.active() == 0 {
                return;
            }
            notification.await;
        }
    }

    /// Consume the drain and notify the external shutdown context from Tokio.
    pub fn notify<Done>(self, done: Done)
    where
        Done: FnOnce() + Send + 'static,
    {
        let runtime = self.handle.runtime.clone();
        drop(runtime.spawn(async move {
            self.wait().await;
            done();
        }));
    }
}

// Opaque C handle is an Arc strong reference. All functions require a live owned
// reference for their entire call; concurrent release of that same reference is
// forbidden. Each foreign holder retains its own reference before publication.
#[unsafe(no_mangle)]
pub extern "C" fn native_cancel_create() -> *const Cancellation {
    Arc::into_raw(Cancellation::new())
}

/// # Safety
/// `handle` must be a live cancellation reference owned by the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn native_cancel_retain(handle: *const Cancellation) {
    if !handle.is_null() {
        unsafe { Arc::increment_strong_count(handle) };
    }
}

/// # Safety
/// Consumes one owned reference, after the caller's final use of that reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn native_cancel_release(handle: *const Cancellation) {
    if !handle.is_null() {
        drop(unsafe { Arc::from_raw(handle) });
    }
}

/// # Safety
/// `handle` must remain alive through the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn native_cancel_request(handle: *const Cancellation) -> bool {
    !handle.is_null() && unsafe { &*handle }.request()
}

/// # Safety
/// `handle` must remain alive through the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn native_cancel_requested(handle: *const Cancellation) -> bool {
    !handle.is_null() && unsafe { &*handle }.is_cancelled()
}

/// # Safety
/// Only the terminal workflow owner may call this, after children have stopped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn native_cancel_finish(handle: *const Cancellation) -> bool {
    !handle.is_null() && unsafe { &*handle }.finish()
}

/// # Safety
/// `handle` must be live while its independent reference is cloned.
pub unsafe fn retain_cancel(handle: *const Cancellation) -> Option<Arc<Cancellation>> {
    if handle.is_null() {
        return None;
    }
    unsafe { Arc::increment_strong_count(handle) };
    Some(unsafe { Arc::from_raw(handle) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn admission_cancel_and_drain_keep_one_owner() -> Result<(), Box<dyn std::error::Error>> {
        let owner = RuntimeOwner::new(2, 1)?;
        let cancel = Cancellation::new();
        let (tx, rx) = mpsc::channel();
        let accepted = owner.handle().submit(
            Arc::clone(&cancel),
            |cancel| async move {
                cancel.cancelled().await;
                Ok::<_, ()>(42)
            },
            move |result| {
                let _ = tx.send(result);
            },
        );
        assert_eq!(accepted, Ok(()));
        assert_eq!(owner.active(), 1);
        assert_eq!(
            owner
                .handle()
                .submit(Cancellation::new(), |_| async { Ok::<_, ()>(()) }, |_| {}),
            Err(AdmissionError::Saturated)
        );
        assert!(cancel.request());
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2))?,
            Completion::Success(42)
        ));
        let drain = owner.stop();
        owner.block_on_sync(drain.wait());
        assert_eq!(owner.active(), 0);
        assert!(cancel.finish());
        assert!(!cancel.request());
        assert_eq!(
            owner
                .handle()
                .submit(Cancellation::new(), |_| async { Ok::<_, ()>(()) }, |_| {}),
            Err(AdmissionError::Stopping)
        );
        Ok(())
    }

    #[test]
    fn pre_cancelled_and_many_owned_structures() -> Result<(), Box<dyn std::error::Error>> {
        let owner = RuntimeOwner::new(2, 64)?;
        let (tx, rx) = mpsc::channel();
        for index in 0..32 {
            let tx = tx.clone();
            let cancel = Cancellation::new();
            assert!(cancel.request());
            assert_eq!(
                owner.handle().submit(
                    cancel,
                    move |cancel| async move {
                        cancel.cancelled().await;
                        Ok::<_, ()>((index, vec![format!("task {index}")]))
                    },
                    move |result| {
                        let _ = tx.send(result);
                    }
                ),
                Ok(())
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..32 {
            if let Completion::Success((index, names)) = rx.recv_timeout(Duration::from_secs(2))? {
                assert_eq!(names, vec![format!("task {index}")]);
                assert!(seen.insert(index));
            } else {
                return Err("unexpected completion".into());
            }
        }
        let drain = owner.stop();
        owner.block_on_sync(drain.wait());
        assert_eq!(seen.len(), 32);
        Ok(())
    }
    #[test]
    fn external_owner_and_cloned_handles_share_admission() -> Result<(), Box<dyn std::error::Error>>
    {
        let external = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let controller = RuntimeController::attach(external.handle().clone(), 1)?;
        let first = controller.handle();
        let second = first.clone();
        let cancellation = Cancellation::new();
        let (tx, rx) = mpsc::channel();
        assert_eq!(
            first.submit(
                Arc::clone(&cancellation),
                |cancel| async move {
                    cancel.cancelled().await;
                    Ok::<_, ()>("finished")
                },
                move |result| {
                    let _ = tx.send(result);
                }
            ),
            Ok(())
        );
        assert_eq!(second.active(), 1);
        assert_eq!(
            second.submit(Cancellation::new(), |_| async { Ok::<_, ()>(()) }, |_| {}),
            Err(AdmissionError::Saturated)
        );
        drop(first);
        assert!(cancellation.request());
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2))?,
            Completion::Success("finished")
        ));
        let drain = controller.stop();
        external.block_on(drain.wait());
        assert_eq!(second.active(), 0);
        assert_eq!(
            second.submit(Cancellation::new(), |_| async { Ok::<_, ()>(()) }, |_| {}),
            Err(AdmissionError::Stopping)
        );
        drop(second);
        // Releasing our submission authority never destroys a consumer's runtime.
        assert_eq!(
            external.block_on(async { tokio::spawn(async { 42 }).await })?,
            42
        );
        Ok(())
    }
    #[test]
    fn shutdown_notification_waits_for_async_cleanup() -> Result<(), Box<dyn std::error::Error>> {
        let owner = RuntimeOwner::new(2, 2)?;
        let handle = owner.handle();
        let cancellation = Cancellation::new();
        let cleanup = Arc::new(Notify::new());
        let cleanup_future = Arc::clone(&cleanup);
        let (completed_tx, completed_rx) = mpsc::channel();
        assert_eq!(
            handle.submit(
                Arc::clone(&cancellation),
                move |cancel| async move {
                    cancel.cancelled().await;
                    // Cancellation alone is not quiescence. Simulate awaited rollback.
                    cleanup_future.notified().await;
                    Ok::<_, ()>(())
                },
                move |_| {
                    let _ = completed_tx.send(());
                }
            ),
            Ok(())
        );
        let (stopped_tx, stopped_rx) = mpsc::channel();
        owner.stop().notify(move || {
            let _ = stopped_tx.send(());
        });
        assert_eq!(
            handle.submit(Cancellation::new(), |_| async { Ok::<_, ()>(()) }, |_| {}),
            Err(AdmissionError::Stopping)
        );
        assert!(cancellation.request());
        assert!(matches!(
            stopped_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        cleanup.notify_one();
        stopped_rx.recv_timeout(Duration::from_secs(2))?;
        // Drained includes return from the ordinary completion callback.
        completed_rx.try_recv()?;
        assert_eq!(handle.active(), 0);
        drop(handle);
        // Explicit owner control context: never drop a Runtime on a Tokio task.
        drop(owner);
        Ok(())
    }
    #[test]
    fn service_drain_is_local_and_owner_accounts_for_children()
    -> Result<(), Box<dyn std::error::Error>> {
        let owner = RuntimeOwner::new(2, 2)?;
        let service = owner.controller(1)?;
        let other = owner.controller(1)?;
        let cancellation = Cancellation::new();
        assert_eq!(
            service.handle().submit(
                Arc::clone(&cancellation),
                |cancel| async move {
                    cancel.cancelled().await;
                    Ok::<_, ()>(())
                },
                |_| {}
            ),
            Ok(())
        );
        assert_eq!(owner.active(), 1);
        let drain = service.stop();
        assert_eq!(
            service
                .handle()
                .submit(Cancellation::new(), |_| async { Ok::<_, ()>(()) }, |_| {}),
            Err(AdmissionError::Stopping)
        );
        assert_eq!(owner.active(), 1); // failed child admission released parent permit
        assert_eq!(
            other
                .handle()
                .submit(Cancellation::new(), |_| async { Ok::<_, ()>(()) }, |_| {}),
            Ok(())
        );
        let owner_drain = owner.stop();
        assert_eq!(
            other
                .handle()
                .submit(Cancellation::new(), |_| async { Ok::<_, ()>(()) }, |_| {}),
            Err(AdmissionError::Stopping)
        );
        assert!(cancellation.request());
        owner.block_on_sync(drain.wait());
        owner.block_on_sync(owner_drain.wait());
        assert_eq!(owner.active(), 0);
        Ok(())
    }
    #[test]
    fn reservation_spans_preparation_and_submits_after_stop()
    -> Result<(), Box<dyn std::error::Error>> {
        let owner = RuntimeOwner::new(2, 1)?;
        let reservation = owner
            .handle()
            .reserve(Cancellation::new())
            .map_err(|_| "admission")?;
        assert_eq!(owner.active(), 1);
        let drain = owner.stop();
        let (tx, rx) = mpsc::channel();
        reservation.submit(
            |_| async { Ok::<_, ()>(42) },
            move |result| {
                let _ = tx.send(result);
            },
        );
        owner.block_on_sync(drain.wait());
        assert!(matches!(rx.try_recv()?, Completion::Success(42)));
        assert_eq!(owner.active(), 0);
        let another = RuntimeOwner::new(1, 1)?;
        let unused = another
            .handle()
            .reserve(Cancellation::new())
            .map_err(|_| "admission")?;
        drop(unused);
        assert_eq!(another.active(), 0);
        Ok(())
    }
}
