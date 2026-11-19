//! Startup request controls and physically admitted native channels.
use crate::body::BodySignal;
use crate::endpoint::PipelineRun;
use crate::host::{NativeOutputState, NativeResponse, NativeSelection};
use crate::prepared_request_storage::{Pool, Vacancy};
use crate::{PreparedRequestControlCapacity, Request, RoutingRequest};
use axum::body::Body;
use effect::memory::{
    Admission, AdmissionBundle, Bytes, CapacityUnit, FixedVector, MemoryError, PreparedMemory,
    RegionId, SharedSlot, SlotPool,
};
use effect::{
    NativeContext, PreparedOneshotPool, PreparedReceiver, PreparedSender, PreparedWatchPool,
    PreparedWatchPublication, PreparedWatchReceiver, PreparedWatchSender,
};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Default)]
struct SignalState {
    handles: usize,
    signal: Option<BodySignal>,
}
impl SignalState {
    fn acquire(&mut self) {
        // Every handle already retained its own control Arc; its checked
        // reference-count limit is reached before this private count can wrap.
        self.handles += 1;
    }
    fn release(&mut self) -> Option<BodySignal> {
        self.handles -= 1;
        if self.handles == 0 {
            self.signal.take()
        } else {
            None
        }
    }
}

#[derive(Default)]
struct RequestShadowState {
    handles: usize,
    request: Option<Request>,
}
/// A shadow Request and every clone retain the actual outer control record.
pub(crate) struct RequestShadow<E: Send + 'static, R: Send + Sync + 'static>(
    RequestControlRef<E, R>,
);
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RequestShadow<E, R> {
    fn clone(&self) -> Self {
        let owner = self.0.clone();
        // Each handle already owns an Arc-backed control loan, whose limit is
        // reached before the number of shadow handles can overflow.
        owner
            .control
            .record
            .request_shadow
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .handles += 1;
        Self(owner)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for RequestShadow<E, R> {
    fn drop(&mut self) {
        let request = {
            let mut state = self
                .0
                .control
                .record
                .request_shadow
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.handles -= 1;
            if state.handles == 0 {
                state.request.take()
            } else {
                None
            }
        };
        drop(request);
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> RequestShadow<E, R> {
    pub(crate) fn with<T>(&self, access: impl FnOnce(&Request) -> T) -> Option<T> {
        let state = self
            .0
            .control
            .record
            .request_shadow
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.request.as_ref().map(access)
    }
    #[allow(clippy::result_large_err)] // Preserve the actual request on invariant failure.
    pub(crate) fn replace(&self, request: Request) -> Result<(), Request> {
        let previous = {
            let mut state = self
                .0
                .control
                .record
                .request_shadow
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.request.is_none() {
                return Err(request);
            }
            state.request.replace(request)
        };
        drop(previous);
        Ok(())
    }
}
pub(crate) struct GateSenders {
    pub start: PreparedSender<NativeContext>,
    pub selection: PreparedSender<NativeContext>,
}
struct Record<E: Send + 'static, R: Send + Sync + 'static> {
    configured: bool,
    quota: Mutex<Option<AdmissionBundle<'static, 1>>>,
    queue_permit: Mutex<Option<effect::PreparedSemaphorePermit>>,
    loans: Arc<Mutex<usize>>,
    producers: Mutex<usize>,
    output: Mutex<NativeOutputState<E, R>>,
    // A Fiber is an outcome/cancellation handle; the running task remains in
    // the existing scope. The slot never stores a RequestControlRef itself.
    inner_driver: Mutex<Option<effect::Fiber<(), E>>>,
    routing: Mutex<Option<RoutingRequest>>,
    body_signal: Mutex<SignalState>,
    request_shadow: Mutex<RequestShadowState>,
    gates: Mutex<Option<GateSenders>>,
    start: Mutex<Option<PreparedReceiver<NativeContext>>>,
    selection: Mutex<Option<PreparedReceiver<NativeContext>>>,
    responses: PreparedOneshotPool<NativeResponse<E, R>>,
    contexts: PreparedOneshotPool<NativeContext>,
    inner: Option<InnerPool<E, R>>,
    first_inner: Mutex<Option<InnerReservation<E, R>>>,
    pub(crate) classification: Option<PreparedWatchPool<Option<Request>>>,
    pub(crate) selection_watch: Option<PreparedWatchPool<Option<NativeSelection<R>>>>,
    watches: Mutex<Option<WatchPairs<R>>>,
    classification_sender: Mutex<Option<PreparedWatchSender<Option<Request>>>>,
    selection_sender: Mutex<Option<PreparedWatchSender<Option<NativeSelection<R>>>>>,
    classification_first: Mutex<Option<PreparedWatchPublication<Option<Request>>>>,
    selection_first: Mutex<Option<PreparedWatchPublication<Option<NativeSelection<R>>>>>,
    classification_producers: Mutex<usize>,
    selection_producers: Mutex<usize>,
}
pub(crate) struct WatchPairs<R: Send + Sync + 'static> {
    pub classification: PreparedWatchReceiver<Option<Request>>,
    pub selection: PreparedWatchReceiver<Option<NativeSelection<R>>>,
}
pub(crate) struct ClassificationProducer<E: Send + 'static, R: Send + Sync + 'static>(
    RequestControlRef<E, R>,
);
pub(crate) struct SelectionProducer<E: Send + 'static, R: Send + Sync + 'static>(
    RequestControlRef<E, R>,
);
pub(crate) struct ClassificationPublication<E: Send + 'static, R: Send + Sync + 'static>(
    PreparedWatchPublication<Option<Request>>,
    RequestControlRef<E, R>,
);
pub(crate) struct SelectionPublication<E: Send + 'static, R: Send + Sync + 'static>(
    PreparedWatchPublication<Option<NativeSelection<R>>>,
    RequestControlRef<E, R>,
);
struct InnerRecord<E: Send + 'static, R: Send + Sync + 'static> {
    continuations: [Mutex<ContinuationCell<E, R>>; 2],
    producers: Mutex<usize>,
    sender: Mutex<Option<PreparedSender<http::Response<Body>>>>,
    channel: PreparedOneshotPool<http::Response<Body>>,
    outer_wait: Mutex<Option<crate::body::BodySignalWait>>,
    body_vacancies: Mutex<[Option<crate::prepared_body_completion::ReplyBodyVacancy<E>>; 2]>,
    body_signal: Mutex<SignalState>,
    pipeline: Mutex<Option<Vacancy<PipelineRun<E, R>>>>,
}
struct ContinuationCell<E: Send + 'static, R: Send + Sync + 'static> {
    owners: usize,
    generation: usize,
    delivery: Option<crate::host::ReplyDelivery<E, R>>,
    root: Option<RequestControlRef<E, R>>,
}
struct InnerControl<E: Send + 'static, R: Send + Sync + 'static> {
    record: SharedSlot<'static, InnerRecord<E, R>>,
}
#[derive(Clone, Copy)]
enum ContinuationIndex { First, Second }
impl<E: Send + 'static, R: Send + Sync + 'static> InnerControl<E, R> {
    fn continuation(&self, index: ContinuationIndex) -> &Mutex<ContinuationCell<E, R>> {
        let [first, second] = &self.record.continuations;
        match index { ContinuationIndex::First => first, ContinuationIndex::Second => second }
    }
}
pub(crate) struct PreparedReplyContinuation<E: Send + 'static, R: Send + Sync + 'static> {
    inner: Arc<InnerControl<E, R>>,
    index: ContinuationIndex,
    generation: usize,
}
pub(crate) struct WeakPreparedReplyContinuation<E: Send + 'static, R: Send + Sync + 'static> {
    inner: std::sync::Weak<InnerControl<E, R>>,
    index: ContinuationIndex,
    generation: usize,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for WeakPreparedReplyContinuation<E, R> {
    fn clone(&self) -> Self { Self { inner: self.inner.clone(), index: self.index, generation: self.generation } }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for PreparedReplyContinuation<E, R> {
    fn clone(&self) -> Self {
        let inner = self.inner.clone();
        {
            let mut cell = inner.continuation(self.index).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            // Each strong continuation retains an inner Arc, whose own count
            // limit is reached before this count could overflow.
            cell.owners += 1;
        }
        Self { inner, index: self.index, generation: self.generation }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for PreparedReplyContinuation<E, R> {
    fn drop(&mut self) {
        let retired = {
            let mut cell = self.inner.continuation(self.index).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            cell.owners -= 1;
            if cell.owners == 0 { Some((cell.delivery.take(), cell.root.take())) } else { None }
        };
        drop(retired);
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> PreparedReplyContinuation<E, R> {
    pub(crate) fn downgrade(&self) -> WeakPreparedReplyContinuation<E, R> {
        WeakPreparedReplyContinuation { inner: Arc::downgrade(&self.inner), index: self.index, generation: self.generation }
    }
    pub(crate) fn delivery(&self) -> Option<crate::host::ReplyDelivery<E, R>> {
        self.inner.continuation(self.index).lock().unwrap_or_else(std::sync::PoisonError::into_inner).delivery.clone()
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> WeakPreparedReplyContinuation<E, R> {
    pub(crate) fn upgrade(&self) -> Option<PreparedReplyContinuation<E, R>> {
        let inner = self.inner.upgrade()?;
        {
            let mut cell = inner.continuation(self.index).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if cell.owners == 0 || cell.generation != self.generation { return None; }
            cell.owners += 1;
        }
        Some(PreparedReplyContinuation { inner, index: self.index, generation: self.generation })
    }
}
struct InnerPool<E: Send + 'static, R: Send + Sync + 'static> {
    controls: Mutex<FixedVector<'static, Arc<InnerControl<E, R>>>>,
    pipelines: Pool<PipelineRun<E, R>>,
}
pub(crate) struct InnerProducer<E: Send + 'static, R: Send + Sync + 'static> {
    inner: Arc<InnerControl<E, R>>,
    _root: RequestControlRef<E, R>,
}
pub(crate) struct InnerReceiver<E: Send + 'static, R: Send + Sync + 'static> {
    receiver: PreparedReceiver<http::Response<Body>>,
    _inner: Arc<InnerControl<E, R>>,
    _root: RequestControlRef<E, R>,
}
/// Holds the inner physical record independently of producer authority.
pub(crate) struct InnerBodyState<E: Send + 'static, R: Send + Sync + 'static> {
    inner: Arc<InnerControl<E, R>>,
    _root: RequestControlRef<E, R>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for InnerBodyState<E, R> {
    fn clone(&self) -> Self {
        let inner = Arc::clone(&self.inner);
        let root = self._root.clone();
        inner
            .record
            .body_signal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .acquire();
        Self { inner, _root: root }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for InnerBodyState<E, R> {
    fn drop(&mut self) {
        let signal = self
            .inner
            .record
            .body_signal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .release();
        drop(signal);
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> InnerBodyState<E, R> {
    fn state(&self) -> &Mutex<SignalState> {
        &self.inner.record.body_signal
    }
    pub(crate) fn is_none(&self) -> bool {
        self.state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .signal
            .is_none()
    }
    pub(crate) fn outcome(&self) -> crate::BodyOutcome {
        self.state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .signal
            .as_ref()
            .map(BodySignal::outcome)
            .unwrap_or(crate::BodyOutcome::Open)
    }
    pub(crate) fn replace(&self, value: BodySignal) {
        let old = self
            .state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .signal
            .replace(value);
        drop(old);
    }
}
/// Holds the outer request control independently of response-producer authority.
pub(crate) struct OuterBodyState<E: Send + 'static, R: Send + Sync + 'static>(
    RequestControlRef<E, R>,
);
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for OuterBodyState<E, R> {
    fn clone(&self) -> Self {
        let owner = self.0.clone();
        owner
            .control
            .record
            .body_signal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .acquire();
        Self(owner)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for OuterBodyState<E, R> {
    fn drop(&mut self) {
        let signal = self
            .0
            .control
            .record
            .body_signal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .release();
        drop(signal);
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> OuterBodyState<E, R> {
    fn state(&self) -> &Mutex<SignalState> {
        &self.0.control.record.body_signal
    }
    pub(crate) fn is_none(&self) -> bool {
        self.state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .signal
            .is_none()
    }
    pub(crate) fn outcome(&self) -> crate::BodyOutcome {
        self.state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .signal
            .as_ref()
            .map(BodySignal::outcome)
            .unwrap_or(crate::BodyOutcome::Open)
    }
    pub(crate) fn replace(&self, value: BodySignal) {
        let old = self
            .state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .signal
            .replace(value);
        drop(old);
    }
}
type InnerPair<E, R> = (InnerProducer<E, R>, InnerReceiver<E, R>, bool);
struct FirstInnerResources<E: Send + 'static> {
    outer_wait: crate::body::BodySignalWait,
    vacancies: [Option<crate::prepared_body_completion::ReplyBodyVacancy<E>>; 2],
}
// This pre-admitted owner deliberately contains no RequestControlRef: it may
// be stored in the root record without making that record own itself.
struct InnerReservation<E: Send + 'static, R: Send + Sync + 'static> {
    pipeline: Vacancy<PipelineRun<E, R>>,
    inner: Arc<InnerControl<E, R>>,
    sender: PreparedSender<http::Response<Body>>,
    receiver: PreparedReceiver<http::Response<Body>>,
    resources: Option<FirstInnerResources<E>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> InnerReservation<E, R> {
    fn publish(self, root: &RequestControlRef<E, R>) -> InnerPair<E, R> {
        let Self { pipeline, inner, sender, receiver, resources } = self;
        let prepared = resources.is_some();
        if let Some(resources) = resources {
            *inner.record.outer_wait.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(resources.outer_wait);
            *inner.record.body_vacancies.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = resources.vacancies;
        }
        *inner.record.sender.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sender);
        *inner.record.pipeline.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pipeline);
        *inner.record.producers.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = 1;
        let producer = InnerProducer { inner: inner.clone(), _root: root.clone() };
        (producer, InnerReceiver { receiver, _inner: inner, _root: root.clone() }, prepared)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> InnerPool<E, R> {
    fn storage_bytes(count: usize) -> Result<Bytes, MemoryError> {
        let vector = std::alloc::Layout::array::<Arc<InnerControl<E, R>>>(count)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<InnerRecord<E, R>>::storage_bytes(count)?
            .checked_add(Bytes::new(vector.size()))?
            .checked_add(
                PreparedOneshotPool::<http::Response<Body>>::storage_bytes(1)?
                    .checked_mul(count)?,
            )?
            .checked_add(Pool::<PipelineRun<E, R>>::storage_bytes(count)?)
    }
    fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
    ) -> Result<Self, MemoryError> {
        let records = memory.slots(region, count, |_| {
            Ok(InnerRecord {
                continuations: std::array::from_fn(|_| Mutex::new(ContinuationCell {
                    owners: 0, generation: 0, delivery: None, root: None,
                })),
                producers: Mutex::new(0),
                sender: Mutex::new(None),
                channel: PreparedOneshotPool::prepare(memory, region, 1)?,
                outer_wait: Mutex::new(None),
                body_vacancies: Mutex::new([None, None]),
                body_signal: Mutex::new(SignalState::default()),
                pipeline: Mutex::new(None),
            })
        })?;
        let mut controls = FixedVector::prepare(memory.allocator(region)?, count)?;
        for _ in 0..count {
            let control = Arc::new(InnerControl {
                record: records.checkout()?.share(),
            });
            if let Err(control) = controls.push(control) {
                drop(control);
                return Err(MemoryError::NoSlot);
            }
        }
        Ok(Self {
            controls: Mutex::new(controls),
            pipelines: Pool::prepare(memory, region, count)?,
        })
    }
    fn reserve(
        &self,
        root: &RequestControlRef<E, R>,
    ) -> Result<InnerPair<E, R>, MemoryError> {
        Ok(self.reserve_vacancy()?.publish(root))
    }
    fn reserve_vacancy(&self) -> Result<InnerReservation<E, R>, MemoryError> {
        let inner = {
            let mut controls = crate::try_admission_lock(&self.controls)?;
            let mut selected = None;
            for control in controls.as_mut_slice() {
                if Arc::get_mut(control).is_some() {
                    selected = Some(Arc::clone(control));
                    break;
                }
            }
            selected.ok_or(MemoryError::NoSlot)?
        };
        let (sender, receiver) = inner.record.channel.try_channel()?;
        let pipeline = self.pipelines.reserve(None)?;
        Ok(InnerReservation { pipeline, inner, sender, receiver, resources: None })
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for InnerProducer<E, R> {
    fn clone(&self) -> Self {
        let inner = Arc::clone(&self.inner);
        let root = self._root.clone();
        {
            let mut producers = inner
                .record
                .producers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Every producer already owns an inner Arc; Arc's limit is reached first.
            *producers += 1;
        }
        Self { inner, _root: root }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> InnerProducer<E, R> {
    pub(crate) fn take_pipeline(&self) -> Option<Vacancy<PipelineRun<E, R>>> {
        self.inner.record.pipeline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    pub(crate) fn try_continuation(
        &self, delivery: crate::host::ReplyDelivery<E, R>,
    ) -> Result<PreparedReplyContinuation<E, R>, MemoryError> {
        let inner = self.inner.clone();
        let root = self._root.clone();
        let mut exhausted = false;
        for index in [ContinuationIndex::First, ContinuationIndex::Second] {
            let generation = {
                let mut cell = inner.continuation(index).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if cell.owners != 0 { continue; }
                let Some(generation) = cell.generation.checked_add(1) else { exhausted = true; continue; };
                cell.generation = generation;
                cell.delivery = Some(delivery);
                cell.root = Some(root);
                cell.owners = 1;
                generation
            };
            return Ok(PreparedReplyContinuation { inner, index, generation });
        }
        drop(delivery);
        Err(if exhausted { MemoryError::OwnerLimit } else { MemoryError::NoSlot })
    }
    pub(crate) fn body_state(&self) -> InnerBodyState<E, R> {
        let inner = Arc::clone(&self.inner);
        let root = self._root.clone();
        inner
            .record
            .body_signal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .acquire();
        InnerBodyState { inner, _root: root }
    }
    pub(crate) fn install_body_vacancies(
        &self,
        vacancies: [Option<crate::prepared_body_completion::ReplyBodyVacancy<E>>; 2],
    ) {
        *self
            .inner
            .record
            .body_vacancies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = vacancies;
    }
    pub(crate) fn take_body_vacancy(
        &self,
    ) -> Option<crate::prepared_body_completion::ReplyBodyVacancy<E>> {
        let mut vacancies = self
            .inner
            .record
            .body_vacancies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        vacancies.iter_mut().find_map(Option::take)
    }
    pub(crate) fn install_outer_wait(&self, wait: crate::body::BodySignalWait) {
        *self
            .inner
            .record
            .outer_wait
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(wait);
    }
    pub(crate) fn take_outer_wait(&self) -> Option<crate::body::BodySignalWait> {
        self.inner
            .record
            .outer_wait
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
    pub(crate) fn take(&self) -> Option<(PreparedSender<http::Response<Body>>, Self)> {
        let sender = self
            .inner
            .record
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if sender.is_some() {
            let vacancies = {
                let mut vacancies = self
                    .inner
                    .record
                    .body_vacancies
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::take(&mut *vacancies)
            };
            drop(vacancies);
        }
        sender.map(|sender| (sender, self.clone()))
    }
    pub(crate) fn is_none(&self) -> bool {
        self.inner
            .record
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none()
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for InnerProducer<E, R> {
    fn drop(&mut self) {
        let last = {
            let mut producers = self
                .inner
                .record
                .producers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *producers -= 1;
            *producers == 0
        };
        if last {
            let sender = self
                .inner
                .record
                .sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let wait = self
                .inner
                .record
                .outer_wait
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let vacancies = {
                let mut vacancies = self
                    .inner
                    .record
                    .body_vacancies
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::take(&mut *vacancies)
            };
            let pipeline = self.take_pipeline();
            drop(sender);
            drop(wait);
            drop(vacancies);
            drop(pipeline);
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> std::future::Future for InnerReceiver<E, R> {
    type Output = Result<http::Response<Body>, effect::PreparedOneshotClosed>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.get_mut().receiver).poll(cx)
    }
}
struct Control<E: Send + 'static, R: Send + Sync + 'static> {
    record: SharedSlot<'static, Record<E, R>>,
}
type Controls<E, R> = Mutex<FixedVector<'static, Arc<Control<E, R>>>>;
pub(crate) struct RequestControlPool<E: Send + 'static, R: Send + Sync + 'static> {
    controls: SharedSlot<'static, Controls<E, R>>,
    admission: Option<Admission<'static, 1>>,
}
pub(crate) struct RequestControlRef<E: Send + 'static, R: Send + Sync + 'static> {
    control: ControlOwner<E, R>,
}
struct ControlOwner<E: Send + 'static, R: Send + Sync + 'static>(Option<Arc<Control<E, R>>>);
impl<E: Send + 'static, R: Send + Sync + 'static> std::ops::Deref for ControlOwner<E, R> {
    type Target = Arc<Control<E, R>>;
    #[expect(clippy::unreachable, reason = "a live request reference retains its control until Drop")]
    fn deref(&self) -> &Self::Target {
        let Some(control) = &self.0 else { unreachable!("live request control"); };
        control
    }
}
pub(crate) struct RequestChannels<E: Send + 'static, R: Send + Sync + 'static> {
    pub response_sender: PreparedSender<NativeResponse<E, R>>,
    pub response_receiver: PreparedReceiver<NativeResponse<E, R>>,
    pub control: RequestControlRef<E, R>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> RequestControlPool<E, R> {
    pub(crate) fn storage_bytes(count: usize) -> Result<Bytes, MemoryError> {
        Self::storage_bytes_with(count, None)
    }
    pub(crate) fn storage_bytes_with(
        count: usize,
        config: Option<PreparedRequestControlCapacity>,
    ) -> Result<Bytes, MemoryError> {
        let entries = std::alloc::Layout::array::<Arc<Control<E, R>>>(count)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let mut bytes = SlotPool::<Record<E, R>>::storage_bytes(count)?
            .checked_add(SlotPool::<Controls<E, R>>::storage_bytes(1)?)?
            .checked_add(Bytes::new(entries.size()))?
            .checked_add(
                PreparedOneshotPool::<NativeResponse<E, R>>::storage_bytes(1)?
                    .checked_mul(count)?,
            )?
            .checked_add(
                PreparedOneshotPool::<NativeContext>::storage_bytes(2)?.checked_mul(count)?,
            )?;
        if let Some(config) = config {
            let extra = InnerPool::<E, R>::storage_bytes(config.inner_responses)?
                .checked_add(PreparedWatchPool::<Option<Request>>::storage_bytes(
                    1,
                    config.classification_receivers,
                    config.classification_versions,
                )?)?
                .checked_add(
                    PreparedWatchPool::<Option<NativeSelection<R>>>::storage_bytes(
                        1,
                        config.selection_receivers,
                        config.selection_versions,
                    )?,
                )?;
            bytes = bytes.checked_add(extra.checked_mul(count)?)?;
        }
        Ok(bytes)
    }
    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
        config: Option<PreparedRequestControlCapacity>,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes_with(count, config)?;
        let records = memory.slots(region, count, |_| {
            Ok(Record {
                configured: config.is_some(),
                quota: Mutex::new(None),
                queue_permit: Mutex::new(None),
                loans: Arc::new(Mutex::new(0)),
                producers: Mutex::new(0),
                output: Mutex::new(NativeOutputState::empty()),
                inner_driver: Mutex::new(None),
                routing: Mutex::new(None),
                body_signal: Mutex::new(SignalState::default()),
                request_shadow: Mutex::new(RequestShadowState::default()),
                gates: Mutex::new(None),
                start: Mutex::new(None),
                selection: Mutex::new(None),
                responses: PreparedOneshotPool::prepare(memory, region, 1)?,
                contexts: PreparedOneshotPool::prepare(memory, region, 2)?,
                inner: config
                    .map(|config| InnerPool::<E, R>::prepare(memory, region, config.inner_responses))
                    .transpose()?,
                first_inner: Mutex::new(None),
                classification: config
                    .map(|config| {
                        PreparedWatchPool::prepare(
                            memory,
                            region,
                            1,
                            config.classification_receivers,
                            config.classification_versions,
                        )
                    })
                    .transpose()?,
                selection_watch: config
                    .map(|config| {
                        PreparedWatchPool::prepare(
                            memory,
                            region,
                            1,
                            config.selection_receivers,
                            config.selection_versions,
                        )
                    })
                    .transpose()?,
                watches: Mutex::new(None),
                classification_sender: Mutex::new(None),
                selection_sender: Mutex::new(None),
                classification_first: Mutex::new(None),
                selection_first: Mutex::new(None),
                classification_producers: Mutex::new(0),
                selection_producers: Mutex::new(0),
            })
        })?;
        let mut controls = FixedVector::prepare(memory.allocator(region)?, count)?;
        for _ in 0..count {
            let control = Arc::new(Control {
                record: records.checkout()?.share(),
            });
            if let Err(control) = controls.push(control) {
                drop(control);
                return Err(MemoryError::NoSlot);
            }
        }
        let mut controls = Some(controls);
        let registry = memory.slots(region, 1, |_| {
            Ok(Mutex::new(controls.take().ok_or(MemoryError::NoSlot)?))
        })?;
        Ok(Self {
            controls: registry.checkout()?.share(),
            admission: None,
        })
    }
    pub(crate) fn install_admission(
        &mut self,
        admission: Admission<'static, 1>,
    ) -> Result<(), MemoryError> {
        if admission.capacities().first().map(|capacity| capacity.unit)
            != Some(CapacityUnit::Slots)
        {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.admission = Some(admission);
        Ok(())
    }
    pub(crate) fn admit_quota(&self, control: &RequestControlRef<E, R>) -> Result<(), MemoryError> {
        let Some(admission) = &self.admission else {
            return Ok(());
        };
        let quota = admission.reserve([1], [0])?;
        let mut slot = control
            .control
            .record
            .quota
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_some() {
            drop(slot);
            drop(quota);
            return Err(MemoryError::Busy);
        }
        *slot = Some(quota);
        Ok(())
    }
    pub(crate) fn reserve(&self) -> Result<RequestChannels<E, R>, MemoryError> {
        let control = {
            let mut controls = crate::try_admission_lock(&self.controls)?;
            let mut selected = None;
            for control in controls.as_mut_slice() {
                if Arc::get_mut(control).is_some() {
                    let mut loans = control
                        .record
                        .loans
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    *loans = 1;
                    drop(loans);
                    selected = Some(RequestControlRef {
                        control: ControlOwner(Some(Arc::clone(control))),
                    });
                    break;
                }
            }
            selected.ok_or(MemoryError::NoSlot)?
        };
        let first_inner = control.control.record.inner.as_ref()
            .map(InnerPool::reserve_vacancy).transpose()?;
        let (response_sender, response_receiver) =
            control.control.record.responses.try_channel()?;
        let (start, start_receiver) = control.control.record.contexts.try_channel()?;
        let (selection, selection_receiver) = control.control.record.contexts.try_channel()?;
        let watches = match (
            &control.control.record.classification,
            &control.control.record.selection_watch,
        ) {
            (Some(classification), Some(selection)) => {
                let classification = classification.try_watch(None).map_err(|(error, _)| error)?;
                let selection = selection.try_watch(None).map_err(|(error, _)| error)?;
                let classification_first = classification.0.reserve_publication()?;
                let selection_first = selection.0.reserve_publication()?;
                Some((
                    classification,
                    selection,
                    classification_first,
                    selection_first,
                ))
            }
            (None, None) => None,
            _ => return Err(MemoryError::UnsupportedRunContract),
        };
        *control
            .control
            .record
            .start
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(start_receiver);
        *control
            .control
            .record
            .selection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(selection_receiver);
        *control
            .control
            .record
            .gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(GateSenders { start, selection });
        if let Some((
            (classification_sender, classification),
            (selection_sender, selection),
            classification_first,
            selection_first,
        )) = watches
        {
            *control
                .control
                .record
                .classification_sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(classification_sender);
            *control
                .control
                .record
                .selection_sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(selection_sender);
            *control
                .control
                .record
                .classification_first
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(classification_first);
            *control
                .control
                .record
                .selection_first
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(selection_first);
            *control
                .control
                .record
                .watches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(WatchPairs {
                classification,
                selection,
            });
        }
        *control.control.record.first_inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = first_inner;
        Ok(RequestChannels {
            control,
            response_sender,
            response_receiver,
        })
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> RequestControlRef<E, R> {
    /// The front bundle has not yet published its actual first inner owner.
    pub(crate) fn has_first_inner_reservation(&self) -> bool {
        self.control.record.first_inner.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }
    pub(crate) fn install_first_inner_resources(
        &self,
        outer_wait: crate::body::BodySignalWait,
        vacancies: [Option<crate::prepared_body_completion::ReplyBodyVacancy<E>>; 2],
    ) -> Result<(), MemoryError> {
        let mut resources = Some(FirstInnerResources { outer_wait, vacancies });
        let installed = {
            let mut first = self.control.record.first_inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match first.as_mut() {
                Some(first) if first.resources.is_none() => {
                    first.resources = resources.take();
                    true
                }
                _ => false,
            }
        };
        drop(resources);
        if installed { Ok(()) } else { Err(MemoryError::UnsupportedRunContract) }
    }
    #[allow(clippy::result_large_err)] // Preserve the actual request on invariant failure.
    pub(crate) fn request_shadow(&self, request: Request) -> Result<RequestShadow<E, R>, Request> {
        let owner = self.clone();
        {
            let mut state = owner
                .control
                .record
                .request_shadow
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.handles != 0 || state.request.is_some() {
                return Err(request);
            }
            state.handles = 1;
            state.request = Some(request);
        }
        Ok(RequestShadow(owner))
    }
    pub(crate) fn body_state(&self) -> OuterBodyState<E, R> {
        let owner = self.clone();
        owner
            .control
            .record
            .body_signal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .acquire();
        OuterBodyState(owner)
    }
    /// Install only in the pre-admitted vacant record. Return the actual owner
    /// unchanged if the private one-shot invariant is violated.
    #[allow(clippy::result_large_err)] // Preserve the actual routing owner on refusal.
    pub(crate) fn install_routing(&self, routing: RoutingRequest) -> Result<(), RoutingRequest> {
        let mut slot = self
            .control
            .record
            .routing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_some() {
            return Err(routing);
        }
        *slot = Some(routing);
        Ok(())
    }
    pub(crate) fn take_routing(&self) -> Option<RoutingRequest> {
        self.control
            .record
            .routing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
    pub(crate) fn replace_inner_driver(&self, fiber: effect::Fiber<(), E>) -> Option<effect::Fiber<(), E>> {
        self.control.record.inner_driver.lock().unwrap_or_else(std::sync::PoisonError::into_inner).replace(fiber)
    }
    pub(crate) fn take_inner_driver(&self) -> Option<effect::Fiber<(), E>> {
        self.control.record.inner_driver.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }
    pub(crate) fn reserve_inner(&self) -> Option<Result<InnerPair<E, R>, MemoryError>> {
        match &self.control.record.inner {
            Some(pool) => {
                let first = self.control.record.first_inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                Some(match first { Some(first) => Ok(first.publish(self)), None => pool.reserve(self) })
            },
            None if self.control.record.configured => {
                Some(Err(MemoryError::UnsupportedRunContract))
            }
            None => None,
        }
    }
    pub(crate) fn take_watches(&self) -> Option<WatchPairs<R>> {
        self.control
            .record
            .watches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
    pub(crate) fn watches_configured(&self) -> bool {
        self.control.record.configured
    }
    pub(crate) fn watch_producers(
        &self,
    ) -> (ClassificationProducer<E, R>, SelectionProducer<E, R>) {
        let classification = self.clone();
        let selection = self.clone();
        *self
            .control
            .record
            .classification_producers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        *self
            .control
            .record
            .selection_producers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        (
            ClassificationProducer(classification),
            SelectionProducer(selection),
        )
    }
    pub(crate) fn install_queue_permit(&self, permit: effect::PreparedSemaphorePermit) {
        *self.control.record.queue_permit.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(permit);
    }
    pub(crate) fn retains_queue_permit(&self) -> bool {
        self.control.record.queue_permit.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner).is_some()
    }
    pub(crate) fn lock(&self) -> std::sync::LockResult<MutexGuard<'_, NativeOutputState<E, R>>> {
        self.control.record.output.lock()
    }
    pub(crate) fn take_gates(&self) -> Option<GateSenders> {
        self.control
            .record
            .gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
    pub(crate) fn take_gate(&self, selection: bool) -> Option<PreparedReceiver<NativeContext>> {
        let gate = if selection {
            &self.control.record.selection
        } else {
            &self.control.record.start
        };
        gate.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
    pub(crate) fn add_producer(&self) {
        let mut count = self
            .control
            .record
            .producers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Each producer already owns a root loan, whose Arc count reaches its
        // own limit before this strictly smaller producer count can overflow.
        *count += 1;
    }
    pub(crate) fn retire_producer(&self) {
        let last = {
            let mut count = self
                .control
                .record
                .producers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *count -= 1;
            *count == 0
        };
        if last {
            // Release the active queue claim after the actual transport and
            // task producers, independently of retained diagnostic aliases.
            let queue_permit = self.control.record.queue_permit.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner).take();
            let (sender, retention, terminal) = {
                let mut output = self.control.record.output.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (output.sender.take(), output.retention.take(), output.terminal.take())
            };
            drop(sender);
            // The native task, response body and emitted data buffers have all
            // retired. A diagnostic scope alias may still retain the control.
            drop(retention);
            drop(terminal);
            drop(queue_permit);
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> ClassificationProducer<E, R> {
    pub(crate) fn reserve(&self) -> Result<ClassificationPublication<E, R>, MemoryError> {
        let record = &self.0.control.record;
        let first = record
            .classification_first
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let publication = match first {
            Some(first) => first,
            None => record
                .classification_sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .ok_or(MemoryError::Closed)?
                .reserve_publication()?,
        };
        Ok(ClassificationPublication(publication, self.0.clone()))
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> SelectionProducer<E, R> {
    pub(crate) fn reserve(&self) -> Result<SelectionPublication<E, R>, MemoryError> {
        let record = &self.0.control.record;
        let first = record
            .selection_first
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let publication = match first {
            Some(first) => first,
            None => record
                .selection_sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .ok_or(MemoryError::Closed)?
                .reserve_publication()?,
        };
        Ok(SelectionPublication(publication, self.0.clone()))
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> ClassificationPublication<E, R> {
    pub(crate) fn publish(self, request: Request) {
        let Self(publication, root) = self;
        publication.publish(Some(request));
        drop(root);
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> SelectionPublication<E, R> {
    pub(crate) fn publish(self, selection: NativeSelection<R>) {
        let Self(publication, root) = self;
        publication.publish(Some(selection));
        drop(root);
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for ClassificationProducer<E, R> {
    fn clone(&self) -> Self {
        let root = self.0.clone();
        *root
            .control
            .record
            .classification_producers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        Self(root)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for SelectionProducer<E, R> {
    fn clone(&self) -> Self {
        let root = self.0.clone();
        *root
            .control
            .record
            .selection_producers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        Self(root)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for ClassificationProducer<E, R> {
    fn drop(&mut self) {
        let record = &self.0.control.record;
        let last = {
            let mut count = record
                .classification_producers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *count -= 1;
            *count == 0
        };
        if last {
            let first = record
                .classification_first
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let sender = record
                .classification_sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(first);
            drop(sender);
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for SelectionProducer<E, R> {
    fn drop(&mut self) {
        let record = &self.0.control.record;
        let last = {
            let mut count = record
                .selection_producers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *count -= 1;
            *count == 0
        };
        if last {
            let first = record
                .selection_first
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let sender = record
                .selection_sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(first);
            drop(sender);
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RequestControlRef<E, R> {
    fn clone(&self) -> Self {
        let control = Arc::clone(&self.control);
        {
            let mut loans = control
                .record
                .loans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Arc cloning precedes this increment, and every loan owns an Arc.
            *loans += 1;
        }
        Self {
            control: ControlOwner(Some(control)),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for RequestControlRef<E, R> {
    fn drop(&mut self) {
        // The independent gate survives control retirement. Both admission
        // claims release after the request's resources, including on unwind.
        let queue_permit;
        let quota;
        let Some(control) = self.control.0.take() else { return; };
        let gate = Arc::clone(&control.record.loans);
        {
            let mut loans = gate.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if *loans != 1 {
                drop(control);
                *loans -= 1;
                return;
            }
            *loans = 0;
        }
        queue_permit = control.record.queue_permit.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner).take();
        quota = control
            .record
            .quota
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let first_inner = control.record.first_inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        let inner_driver = control
            .record
            .inner_driver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let output = {
            let mut output = control
                .record
                .output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::replace(&mut *output, NativeOutputState::empty())
        };
        let routing = control
            .record
            .routing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let gates = control
            .record
            .gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let start = control
            .record
            .start
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let selection = control
            .record
            .selection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let watches = control
            .record
            .watches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let classification_first = control
            .record
            .classification_first
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let selection_first = control
            .record
            .selection_first
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let classification_sender = control
            .record
            .classification_sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let selection_sender = control
            .record
            .selection_sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(first_inner);
        drop(inner_driver);
        drop(output);
        drop(routing);
        drop(gates);
        drop(start);
        drop(selection);
        drop(watches);
        drop(classification_first);
        drop(selection_first);
        drop(classification_sender);
        drop(selection_sender);
        drop(control);
        drop(quota);
        drop(queue_permit);
    }
}
