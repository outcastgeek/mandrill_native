//! Counted immutable environment records for prepared projection/Layer seams.
//! Publication is once, before subtree work. Sharing is checked during admission;
//! last payload destruction uses the common intrusive frame retirement lane.
use super::super::OriginSnapshot;
use super::*;
use crate::memory::{RetainedFrameValue, SharedRuntimeFrame};
use std::cell::UnsafeCell;
use std::sync::RwLock;

// Payload descendants complete before the actual source node footer, including
// when the environment is retained from a different root's output chain.
type Record<R, V, E> = RetainedFrameValue<RwLock<Option<R>>, PreparedRunContext<V, E>>;
struct PendingFooter<V: Send + 'static, E: Send + 'static> {
    origin: UnsafeCell<Option<OriginSnapshot>>,
    _context: PreparedRunContext<V, E>,
}
// SAFETY: origin is installed only through the sole publisher, under the
// value-publication gate; footer destruction starts only after the last view.
unsafe impl<V: Send, E: Send> Send for PendingFooter<V, E> {}
unsafe impl<V: Send, E: Send> Sync for PendingFooter<V, E> {}
type PendingRecord<R, V, E> = RetainedFrameValue<RwLock<Option<R>>, PendingFooter<V, E>>;
type InstallOrigin = fn(&SharedRuntimeFrame, OriginSnapshot) -> Option<OriginSnapshot>;
struct SharedRecord<R: Send + Sync + 'static> {
    owner: SharedRuntimeFrame,
    access: for<'a> fn(&'a SharedRuntimeFrame) -> &'a RwLock<Option<R>>,
}
fn access<R: Send + Sync + 'static, V: Send + 'static, E: Send + 'static>(
    owner: &SharedRuntimeFrame,
) -> &RwLock<Option<R>> {
    // SAFETY: reserve pairs this function only with the exact typed Record cell;
    // it is immutable except for the value's own lock and retained through views.
    unsafe { Record::<R, V, E>::value_pointer(owner.pointer().cast()).as_ref() }
}
impl<R: Send + Sync + 'static> SharedRecord<R> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            owner: self.owner.try_clone()?,
            access: self.access,
        })
    }
    fn get(&self) -> &RwLock<Option<R>> {
        (self.access)(&self.owner)
    }
}
pub struct Environment<R: Send + Sync + 'static> {
    owner: Owner<R>,
}
enum Owner<R: Send + Sync + 'static> {
    External(Arc<R>),
    Prepared(SharedRecord<R>),
    Shared(SharedRecord<Arc<R>>),
}
pub(super) struct Publisher<R: Send + Sync + 'static> {
    record: SharedRecord<R>,
}
/// One-shot publication authority for an actual pre-held typed environment
/// record. Publishing moves R; no R clone or new Arc allocation is performed.
pub struct PreparedEnvironmentPublisher<R: Send + Sync + 'static> {
    record: SharedRecord<R>,
    install_origin: InstallOrigin,
}
impl<R: Send + Sync + 'static> PreparedEnvironmentPublisher<R> {
    /// Install the selected value before polling the admitted body. Its producer
    /// must transfer actual transitive ownership, not a pointer-sized allowance.
    pub fn publish(self, value: R) {
        let old = self
            .record
            .get()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(value);
        drop(old);
    }
    pub(super) fn publish_from(self, value: R, origin: OriginSnapshot) {
        let mut gate = self
            .record
            .get()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous_origin = (self.install_origin)(&self.record.owner, origin);
        let previous = gate.replace(value);
        drop(gate);
        drop(previous);
        drop(previous_origin);
    }
}
impl<R: Send + Sync + 'static> Environment<R> {
    pub(super) fn external(value: Arc<R>) -> Self {
        Self {
            owner: Owner::External(value),
        }
    }
    pub(super) fn demand<V: Send + 'static, E: Send + 'static>() -> Demand {
        let mut demand = Demand::cell::<Record<R, V, E>>(false);
        demand.origin_frames = 1;
        demand
    }
    pub(super) fn reserve<V: Send + 'static, E: Send + 'static>(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<(Publisher<R>, Self), MemoryError> {
        let selection = context.persistent_selection()?;
        let (context, frames) = selection
            .as_ref()
            .map_or((context, frames), |(context, frames)| (context, frames));
        let frame = frames.reserve::<Record<R, V, E>>()?;
        let record = SharedRecord {
            owner: frame.share_parts(RwLock::new(None), context.try_clone()?),
            access: access::<R, V, E>,
        };
        let view = record.try_clone()?;
        Ok((
            Publisher { record },
            Self {
                owner: Owner::Prepared(view),
            },
        ))
    }
    pub(super) fn pending_demand<V: Send + 'static, E: Send + 'static>() -> Demand {
        let mut demand = Demand::cell::<PendingRecord<R, V, E>>(false);
        demand.origin_frames = 1;
        demand
    }
    pub(super) fn reserve_pending<V: Send + 'static, E: Send + 'static>(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<(PreparedEnvironmentPublisher<R>, Self), MemoryError> {
        let selection = context.persistent_selection()?;
        let (context, frames) = selection
            .as_ref()
            .map_or((context, frames), |(context, frames)| (context, frames));
        fn pending_access<R: Send + Sync + 'static, V: Send + 'static, E: Send + 'static>(
            owner: &SharedRuntimeFrame,
        ) -> &RwLock<Option<R>> {
            // SAFETY: exact private PendingRecord constructor/vtable pairing.
            unsafe { PendingRecord::<R, V, E>::value_pointer(owner.pointer().cast()).as_ref() }
        }
        fn install<R: Send + Sync + 'static, V: Send + 'static, E: Send + 'static>(
            owner: &SharedRuntimeFrame,
            origin: OriginSnapshot,
        ) -> Option<OriginSnapshot> {
            // SAFETY: exact pending-record pair, sole publisher holds its value
            // publication gate. Footer is live; no mutable payload projection.
            let footer = unsafe {
                PendingRecord::<R, V, E>::footer_pointer(owner.pointer().cast()).as_ref()
            };
            unsafe { (*footer.origin.get()).replace(origin) }
        }
        let record = SharedRecord {
            owner: frames.reserve::<PendingRecord<R, V, E>>()?.share_parts(
                RwLock::new(None),
                PendingFooter {
                    origin: UnsafeCell::new(None),
                    _context: context.try_clone()?,
                },
            ),
            access: pending_access::<R, V, E>,
        };
        let view = record.try_clone()?;
        Ok((
            PreparedEnvironmentPublisher {
                record,
                install_origin: install::<R, V, E>,
            },
            Self {
                owner: Owner::Prepared(view),
            },
        ))
    }
    pub(super) fn shared_pending_demand<V: Send + 'static, E: Send + 'static>() -> Demand {
        Environment::<Arc<R>>::pending_demand::<V, E>()
    }
    pub(super) fn reserve_shared_pending<V: Send + 'static, E: Send + 'static>(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<(PreparedEnvironmentPublisher<Arc<R>>, Self), MemoryError> {
        let (publisher, environment) = Environment::<Arc<R>>::reserve_pending(frames, context)?;
        match environment.owner {
            Owner::Prepared(record) => Ok((
                publisher,
                Self {
                    owner: Owner::Shared(record),
                },
            )),
            Owner::External(_) | Owner::Shared(_) => Err(MemoryError::UnsupportedRunContract),
        }
    }
    /// Retain the same external or prepared backing without converting a
    /// projected environment to a System Arc. Native futures use this checked
    /// companion when more than one owned environment view is required.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            owner: match &self.owner {
                Owner::External(value) => Owner::External(Arc::clone(value)),
                Owner::Prepared(record) => Owner::Prepared(record.try_clone()?),
                Owner::Shared(record) => Owner::Shared(record.try_clone()?),
            },
        })
    }
    fn identity(&self) -> (u8, usize) {
        match &self.owner {
            Owner::External(value) => (0, Arc::as_ptr(value).addr()),
            Owner::Prepared(record) => (1, record.owner.pointer().as_ptr().addr()),
            Owner::Shared(record) => (2, record.owner.pointer().as_ptr().addr()),
        }
    }
    pub(in crate::internal::runtime::prepared) fn origin_key(
        &self,
    ) -> super::super::retention::EnvironmentKey {
        let (kind, address) = self.identity();
        super::super::retention::EnvironmentKey {
            kind,
            address,
            ty: std::any::TypeId::of::<R>(),
        }
    }
    /// Borrow two retained environments without cloning or allocating. Aliases
    /// take their publication lock only once; distinct records are locked in
    /// stable address order. References cannot escape the callback. As with
    /// `with`, the callback must not await or retire either borrowed record.
    pub fn with_pair<S: Send + Sync + 'static, T>(
        &self,
        other: &Environment<S>,
        access: impl FnOnce(&R, &S) -> T,
    ) -> Result<T, Defect> {
        let left = self.identity();
        let right = other.identity();
        if left == right {
            // A safe shared record/Arc can alias only its original concrete
            // type. Checked Any borrowing avoids a raw cross-type projection.
            return self.with(|value| {
                (value as &dyn std::any::Any)
                    .downcast_ref::<S>()
                    .map(|other| access(value, other))
                    .ok_or_else(|| {
                        Defect::from(crate::LibraryDefect::PreparedEnvironmentAliasMismatch)
                    })
            })?;
        }
        if left < right {
            self.with(|left| other.with(|right| access(left, right)))?
        } else {
            other.with(|right| self.with(|left| access(left, right)))?
        }
    }
    /// Borrow the selected environment synchronously. A projected value remains
    /// behind its publication gate; no borrowed reference can escape this call.
    /// The callback must not await or attempt to retire this same environment.
    pub fn with<T>(&self, access: impl FnOnce(&R) -> T) -> Result<T, Defect> {
        match &self.owner {
            Owner::External(value) => Ok(access(value)),
            Owner::Shared(record) => {
                let value = record
                    .get()
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                value
                    .as_ref()
                    .map(|value| access(value.as_ref()))
                    .ok_or_else(|| {
                        Defect::from(crate::LibraryDefect::PreparedEnvironmentUnpublished)
                    })
            }
            Owner::Prepared(record) => {
                let value = record
                    .get()
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                value.as_ref().map(access).ok_or_else(|| {
                    Defect::from(crate::LibraryDefect::PreparedEnvironmentUnpublished)
                })
            }
        }
    }
}
impl<R: Send + Sync + 'static> Publisher<R> {
    pub(super) fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            record: self.record.try_clone()?,
        })
    }
    pub(super) fn take(&self) -> Option<R> {
        self.record
            .get()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
    pub(super) fn install(&self, value: R) {
        let old = self
            .record
            .get()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(value);
        // Any owned environment destructor runs outside the record gate.
        drop(old);
    }
}

pub struct Provided<N, F, I> {
    program: N,
    project: F,
    marker: PhantomData<fn() -> I>,
}
pub fn provide_some<N, F, I>(program: N, project: F) -> Provided<N, F, I>
where
    N: Description,
    F: FnOnce(&I) -> N::Env + Send + 'static,
    I: Send + Sync + 'static,
{
    Provided {
        program,
        project,
        marker: PhantomData,
    }
}
impl<N, F, I> Description for Provided<N, F, I>
where
    N: Description,
    F: FnOnce(&I) -> N::Env + Send + 'static,
    I: Send + Sync + 'static,
{
}
pub struct ProvidedAdmission<N: Description, F, I: Send + Sync + 'static, V: Send + 'static>
where
    F: FnOnce(&I) -> N::Env + Send + 'static,
{
    entry: FrameReservation<'static, ProvidedEntry<N, F, I, V>>,
    controls: ProvidedControls<N, I, V>,
}
struct ProvidedControls<N: Description, I: Send + Sync + 'static, V: Send + 'static> {
    environment: Environment<I>,
    publisher: Publisher<N::Env>,
    body: N::Admission<V>,
    failed: FrameReservation<'static, Invoke<N::Value, N::Error>>,
    context: PreparedRunContext<V, N::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct ProvidedEntry<N: Description, F, I: Send + Sync + 'static, V: Send + 'static>
where
    F: FnOnce(&I) -> N::Env + Send + 'static,
{
    description: Provided<N, F, I>,
    finish: Finish<N::Value, N::Error>,
    controls: ProvidedControls<N, I, V>,
}
impl<N, F, I, V> Consume<(), Step> for ProvidedEntry<N, F, I, V>
where
    N: Description,
    F: FnOnce(&I) -> N::Env + Send + 'static,
    I: Send + Sync + 'static,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        self.controls
            .context
            .root
            .contexts
            .protected_failure(self.controls.context.id, false);
        let result = if self.controls.context.is_cancelled() {
            Err(Exit::Interrupted)
        } else {
            catch_unwind(AssertUnwindSafe(|| {
                self.controls.environment.with(self.description.project)
            }))
            .unwrap_or_else(|panic| Err(super::super::panic_defect(self.controls.panic, panic)))
            .map_err(Exit::Die)
        };
        match result {
            Ok(environment) => {
                self.controls.publisher.install(environment);
                self.description
                    .program
                    .dispatch(self.controls.body, Continuation(self.finish))
                    .0
            }
            Err(outcome) => Step::More(Once::prepared(
                self.controls.failed,
                Invoke {
                    outcome,
                    finish: self.finish,
                },
            )),
        }
    }
}
impl<N, F, I> Spec for Provided<N, F, I>
where
    N: Description,
    F: FnOnce(&I) -> N::Env + Send + 'static,
    I: Send + Sync + 'static,
{
    type Value = N::Value;
    type Error = N::Error;
    type Env = I;
    type Storage = N::Storage;
    type Admission<V: Send + 'static> = ProvidedAdmission<N, F, I, V>;
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        N::provision::<V>(memory, region, capacity, frames, cleanup)
    }
    fn storage_bytes<V: Send + 'static>(
        capacity: usize,
        layout: Layout,
    ) -> Result<Bytes, MemoryError> {
        N::storage_bytes::<V>(capacity, layout)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = (|| {
            N::shape::<V>()?
                .add(Environment::<N::Env>::demand::<V, N::Error>())?
                .add(Demand::cell::<ProvidedEntry<N, F, I, V>>(false))?
                .add(Demand::cell::<Invoke<N::Value, N::Error>>(true))?
                .add(Demand::panic_sites(1))
        })();
        demand?.add(output_origin_shape::<Self::Env, V, Self::Error>())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        N::scope_finalizers()
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
        enclosing: &crate::PreparedScope<Self::Error>,
        enclosing_cleanup: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = (|| {
            // Full owned O record and ALL subtree owners exist before projection.
            let (publisher, body_environment) = Environment::<N::Env>::reserve(frames, context)?;
            Ok(ProvidedAdmission {
                entry: frames.reserve()?,
                controls: ProvidedControls {
                    environment: environment.try_clone()?,
                    publisher,
                    body: N::reserve_scoped(
                        frames,
                        storage,
                        &body_environment,
                        context,
                        enclosing,
                        enclosing_cleanup,
                    )?,
                    failed: frames.reserve()?,
                    context: context.try_clone()?,
                    panic: context.reserve_panic()?,
                },
            })
        })();
        match admitted {
            Ok(admitted) => {
                origin
                    .attach(environment.try_clone()?)
                    .map_err(|(error, _)| error)?;
                Ok(admitted)
            }
            Err(error) => Err(error),
        }
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<I>,
        context: &PreparedRunContext<V, N::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = (|| {
            // Full owned O record and ALL subtree owners exist before projection.
            let (publisher, body_environment) = Environment::<N::Env>::reserve(frames, context)?;
            Ok(ProvidedAdmission {
                entry: frames.reserve()?,
                controls: ProvidedControls {
                    environment: environment.try_clone()?,
                    publisher,
                    body: N::reserve(frames, storage, &body_environment, context)?,
                    failed: frames.reserve()?,
                    context: context.try_clone()?,
                    panic: context.reserve_panic()?,
                },
            })
        })();
        match admitted {
            Ok(admitted) => {
                origin
                    .attach(environment.try_clone()?)
                    .map_err(|(error, _)| error)?;
                Ok(admitted)
            }
            Err(error) => Err(error),
        }
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<Self::Value, Self::Error>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.entry,
            ProvidedEntry {
                description: self,
                finish: finish.0,
                controls: admission.controls,
            },
        )))
    }
}
