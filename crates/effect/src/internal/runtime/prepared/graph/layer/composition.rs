//! Sequential Layer merge/provide retain one enclosing canonical scope.
use super::*;
pub struct Merge<L, R, F, Q> {
    left: L,
    right: R,
    combine: F,
    marker: PhantomData<fn() -> Q>,
}
pub fn merge<L, R, F, Q>(left: L, right: R, combine: F) -> Merge<L, R, F, Q>
where
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: FnOnce(L::Output, R::Output) -> Q + Send + 'static,
    Q: Send + 'static,
{
    Merge {
        left,
        right,
        combine,
        marker: PhantomData,
    }
}
pub struct MergeAdmission<
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: Send + 'static,
    Q: Send + 'static,
    V: Send + 'static,
> {
    entry: FrameReservation<'static, MergeEntry<L, R, F, Q, V>>,
    left: L::Admission<V>,
    right: R::Admission<V>,
    first: FrameReservation<'static, MergeLeft<L, R, F, Q, V>>,
    second: FrameReservation<'static, MergeRight<L, R, F, Q, V>>,
    combine_frame: FrameReservation<'static, Combine<L, R, F, Q, V>>,
    terminal: FrameReservation<'static, Guarded<Q, L::Error, V>>,
    context: PreparedRunContext<V, L::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct MergeEntry<
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: Send + 'static,
    Q: Send + 'static,
    V: Send + 'static,
> {
    description: Merge<L, R, F, Q>,
    finish: Finish<Q, L::Error>,
    ready: MergeReady<L, R, F, Q, V>,
}
struct MergeReady<
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: Send + 'static,
    Q: Send + 'static,
    V: Send + 'static,
> {
    left: L::Admission<V>,
    right: R::Admission<V>,
    first: FrameReservation<'static, MergeLeft<L, R, F, Q, V>>,
    second: FrameReservation<'static, MergeRight<L, R, F, Q, V>>,
    combine_frame: FrameReservation<'static, Combine<L, R, F, Q, V>>,
    terminal: FrameReservation<'static, Guarded<Q, L::Error, V>>,
    context: PreparedRunContext<V, L::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct MergeLeft<
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: Send + 'static,
    Q: Send + 'static,
    V: Send + 'static,
> {
    right: R,
    combine: F,
    admission: R::Admission<V>,
    second: FrameReservation<'static, MergeRight<L, R, F, Q, V>>,
    combine_frame: FrameReservation<'static, Combine<L, R, F, Q, V>>,
    terminal: FrameReservation<'static, Guarded<Q, L::Error, V>>,
    finish: Finish<Q, L::Error>,
    context: PreparedRunContext<V, L::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct MergeRight<
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: Send + 'static,
    Q: Send + 'static,
    V: Send + 'static,
> {
    left: L::Output,
    combine: F,
    combine_frame: FrameReservation<'static, Combine<L, R, F, Q, V>>,
    terminal: FrameReservation<'static, Guarded<Q, L::Error, V>>,
    finish: Finish<Q, L::Error>,
    context: PreparedRunContext<V, L::Error>,
    panic: Option<crate::PreparedPanicReservation>,
    marker: PhantomData<fn() -> R>,
}
impl<L, R, F, Q, V> Consume<(), Step> for MergeEntry<L, R, F, Q, V>
where
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: FnOnce(L::Output, R::Output) -> Q + Send + 'static,
    Q: Send + 'static,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let ready = self.ready;
        if ready.context.is_cancelled() {
            return Step::More(Once::prepared(
                ready.terminal,
                Guarded {
                    outcome: Exit::Interrupted,
                    finish: self.finish,
                    _context: ready.context,
                },
            ));
        }
        let first = Once::prepared(
            ready.first,
            MergeLeft::<L, R, F, Q, V> {
                right: self.description.right,
                combine: self.description.combine,
                admission: ready.right,
                second: ready.second,
                combine_frame: ready.combine_frame,
                terminal: ready.terminal,
                finish: self.finish,
                context: ready.context,
                panic: ready.panic,
            },
        );
        lower(self.description.left, ready.left, first)
    }
}
impl<L, R, F, Q, V> Consume<Exit<L::Output, L::Error>, Step> for MergeLeft<L, R, F, Q, V>
where
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: FnOnce(L::Output, R::Output) -> Q + Send + 'static,
    Q: Send + 'static,
    V: Send + 'static,
{
    fn consume(self, result: Exit<L::Output, L::Error>) -> Step {
        match failure(result) {
            Ok(left) => {
                if self.context.is_cancelled() {
                    return Step::More(Once::prepared(
                        self.terminal,
                        Guarded {
                            outcome: Exit::Interrupted,
                            finish: self.finish,
                            _context: self.context,
                        },
                    ));
                }
                let second = Once::prepared(
                    self.second,
                    MergeRight::<L, R, F, Q, V> {
                        left,
                        combine: self.combine,
                        combine_frame: self.combine_frame,
                        terminal: self.terminal,
                        finish: self.finish,
                        context: self.context,
                        panic: self.panic,
                        marker: PhantomData,
                    },
                );
                lower(self.right, self.admission, second)
            }
            Err(outcome) => Step::More(Once::prepared(
                self.terminal,
                Guarded {
                    outcome,
                    finish: self.finish,
                    _context: self.context,
                },
            )),
        }
    }
}
impl<L, R, F, Q, V> Consume<Exit<R::Output, R::Error>, Step> for MergeRight<L, R, F, Q, V>
where
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: FnOnce(L::Output, R::Output) -> Q + Send + 'static,
    Q: Send + 'static,
    V: Send + 'static,
{
    fn consume(self, result: Exit<R::Output, R::Error>) -> Step {
        match failure(result) {
            Ok(right) => Step::More(Once::prepared(
                self.combine_frame,
                Combine::<L, R, F, Q, V> {
                    left: self.left,
                    right,
                    combine: self.combine,
                    terminal: self.terminal,
                    finish: self.finish,
                    context: self.context,
                    panic: self.panic,
                },
            )),
            Err(outcome) => Step::More(Once::prepared(
                self.terminal,
                Guarded {
                    outcome,
                    finish: self.finish,
                    _context: self.context,
                },
            )),
        }
    }
}
struct Combine<
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F,
    Q: Send + 'static,
    V: Send + 'static,
> {
    left: L::Output,
    right: R::Output,
    combine: F,
    terminal: FrameReservation<'static, Guarded<Q, L::Error, V>>,
    finish: Finish<Q, L::Error>,
    context: PreparedRunContext<V, L::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<L, R, F, Q, V> Consume<(), Step> for Combine<L, R, F, Q, V>
where
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: FnOnce(L::Output, R::Output) -> Q + Send + 'static,
    Q: Send + 'static,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let outcome = if self.context.is_cancelled() {
            Exit::Interrupted
        } else {
            match catch_unwind(AssertUnwindSafe(|| (self.combine)(self.left, self.right))) {
                Ok(value) => Exit::Success(value),
                Err(panic) => Exit::Die(super::super::super::panic_defect(self.panic, panic)),
            }
        };
        Step::More(Once::prepared(
            self.terminal,
            Guarded {
                outcome,
                finish: self.finish,
                _context: self.context,
            },
        ))
    }
}
impl<L, R, F, Q> LayerDescription for Merge<L, R, F, Q>
where
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: FnOnce(L::Output, R::Output) -> Q + Send + 'static,
    Q: Send + 'static,
{
}
impl<L, R, F, Q> LayerSpec for Merge<L, R, F, Q>
where
    L: LayerDescription,
    R: LayerDescription<Input = L::Input, Error = L::Error>,
    F: FnOnce(L::Output, R::Output) -> Q + Send + 'static,
    Q: Send + 'static,
{
    type Input = L::Input;
    type Error = L::Error;
    type Output = Q;
    type Storage = (L::Storage, R::Storage);
    type Admission<V: Send + 'static> = MergeAdmission<L, R, F, Q, V>;
    fn finalizers() -> Result<usize, MemoryError> {
        L::finalizers()?
            .checked_add(R::finalizers()?)
            .ok_or(MemoryError::ArithmeticOverflow)
    }
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        Ok((
            L::provision::<V>(memory, region, capacity, frames, cleanup)?,
            R::provision::<V>(memory, region, capacity, frames, cleanup)?,
        ))
    }
    fn storage_bytes<V: Send + 'static>(capacity: usize, layout: Layout) -> Result<Bytes, MemoryError> {
        L::storage_bytes::<V>(capacity, layout)?.checked_add(R::storage_bytes::<V>(capacity, layout)?)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = (|| {
            L::shape::<V>()?
                .add(R::shape::<V>()?)?
                .add(Demand::cell::<MergeEntry<L, R, F, Q, V>>(false))?
                .add(Demand::cell::<MergeLeft<L, R, F, Q, V>>(true))?
                .add(Demand::cell::<MergeRight<L, R, F, Q, V>>(true))?
                .add(Demand::cell::<Combine<L, R, F, Q, V>>(false))?
                .add(Demand::cell::<Guarded<Q, L::Error, V>>(true))?
                .add(Demand::panic_sites(1))
        })();
        demand?.add(output_origin_shape::<Self::Input, V, Self::Error>())
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Input>,
        context: &PreparedRunContext<V, Self::Error>,
        scope: &PreparedScope<Self::Error>,
        protected: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Input, V, Self::Error>(frames, environment, context)?;
        let admitted = (|| {
            Ok(MergeAdmission {
                entry: frames.reserve()?,
                left: L::reserve(frames, &storage.0, environment, context, scope, protected)?,
                right: R::reserve(frames, &storage.1, environment, context, scope, protected)?,
                first: frames.reserve()?,
                second: frames.reserve()?,
                combine_frame: frames.reserve()?,
                terminal: frames.reserve()?,
                context: context.try_clone()?,
                panic: context.reserve_panic()?,
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
        finish: Continuation<Self::Output, Self::Error>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.entry,
            MergeEntry {
                description: self,
                ready: MergeReady {
                    left: admission.left,
                    right: admission.right,
                    first: admission.first,
                    second: admission.second,
                    combine_frame: admission.combine_frame,
                    terminal: admission.terminal,
                    context: admission.context,
                    panic: admission.panic,
                },
                finish: finish.0,
            },
        )))
    }
}
pub struct Provide<L, R> {
    left: L,
    right: R,
}
pub fn provide<L, R>(left: L, right: R) -> Provide<L, R>
where
    L: LayerDescription,
    L::Output: Sync,
    R: LayerDescription<Input = L::Output, Error = L::Error>,
{
    Provide { left, right }
}
pub struct ProvideAdmission<
    L: LayerDescription,
    R: LayerDescription<Input = L::Output, Error = L::Error>,
    V: Send + 'static,
> where
    L::Output: Sync,
{
    entry: FrameReservation<'static, ProvideEntry<L, R, V>>,
    ready: ProvideReady<L, R, V>,
}
struct ProvideReady<
    L: LayerDescription,
    R: LayerDescription<Input = L::Output, Error = L::Error>,
    V: Send + 'static,
> where
    L::Output: Sync,
{
    left: L::Admission<V>,
    right: R::Admission<V>,
    publisher: Publisher<L::Output>,
    first: FrameReservation<'static, ProvideLeft<L, R, V>>,
    terminal: FrameReservation<'static, Guarded<R::Output, L::Error, V>>,
    context: PreparedRunContext<V, L::Error>,
}
struct ProvideEntry<
    L: LayerDescription,
    R: LayerDescription<Input = L::Output, Error = L::Error>,
    V: Send + 'static,
> where
    L::Output: Sync,
{
    description: Provide<L, R>,
    finish: Finish<R::Output, L::Error>,
    ready: ProvideReady<L, R, V>,
}
impl<L, R, V> Consume<(), Step> for ProvideEntry<L, R, V>
where
    L: LayerDescription,
    L::Output: Sync,
    R: LayerDescription<Input = L::Output, Error = L::Error>,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let ready = self.ready;
        if ready.context.is_cancelled() {
            return Step::More(Once::prepared(
                ready.terminal,
                Guarded {
                    outcome: Exit::Interrupted,
                    finish: self.finish,
                    _context: ready.context,
                },
            ));
        }
        let first = Once::prepared(
            ready.first,
            ProvideLeft::<L, R, V> {
                right: self.description.right,
                admission: ready.right,
                publisher: ready.publisher,
                terminal: ready.terminal,
                finish: self.finish,
                context: ready.context,
            },
        );
        lower(self.description.left, ready.left, first)
    }
}
struct ProvideLeft<
    L: LayerDescription,
    R: LayerDescription<Input = L::Output, Error = L::Error>,
    V: Send + 'static,
> where
    L::Output: Sync,
{
    right: R,
    admission: R::Admission<V>,
    publisher: Publisher<L::Output>,
    terminal: FrameReservation<'static, Guarded<R::Output, L::Error, V>>,
    finish: Finish<R::Output, L::Error>,
    context: PreparedRunContext<V, L::Error>,
}
impl<L, R, V> Consume<Exit<L::Output, L::Error>, Step> for ProvideLeft<L, R, V>
where
    L: LayerDescription,
    L::Output: Sync,
    R: LayerDescription<Input = L::Output, Error = L::Error>,
    V: Send + 'static,
{
    fn consume(self, result: Exit<L::Output, L::Error>) -> Step {
        match failure(result) {
            Ok(output) => {
                if self.context.is_cancelled() {
                    return Step::More(Once::prepared(
                        self.terminal,
                        Guarded {
                            outcome: Exit::Interrupted,
                            finish: self.finish,
                            _context: self.context,
                        },
                    ));
                }
                self.publisher.install(output);
                lower(self.right, self.admission, self.finish)
            }
            Err(outcome) => Step::More(Once::prepared(
                self.terminal,
                Guarded {
                    outcome,
                    finish: self.finish,
                    _context: self.context,
                },
            )),
        }
    }
}
impl<L, R> LayerDescription for Provide<L, R>
where
    L: LayerDescription,
    L::Output: Sync,
    R: LayerDescription<Input = L::Output, Error = L::Error>,
{
}
impl<L, R> LayerSpec for Provide<L, R>
where
    L: LayerDescription,
    L::Output: Sync,
    R: LayerDescription<Input = L::Output, Error = L::Error>,
{
    type Input = L::Input;
    type Error = L::Error;
    type Output = R::Output;
    type Storage = (L::Storage, R::Storage);
    type Admission<V: Send + 'static> = ProvideAdmission<L, R, V>;
    fn finalizers() -> Result<usize, MemoryError> {
        L::finalizers()?
            .checked_add(R::finalizers()?)
            .ok_or(MemoryError::ArithmeticOverflow)
    }
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        Ok((
            L::provision::<V>(memory, region, capacity, frames, cleanup)?,
            R::provision::<V>(memory, region, capacity, frames, cleanup)?,
        ))
    }
    fn storage_bytes<V: Send + 'static>(capacity: usize, layout: Layout) -> Result<Bytes, MemoryError> {
        L::storage_bytes::<V>(capacity, layout)?.checked_add(R::storage_bytes::<V>(capacity, layout)?)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = (|| {
            L::shape::<V>()?
                .add(R::shape::<V>()?)?
                .add(Environment::<L::Output>::demand::<V, L::Error>())?
                .add(Demand::cell::<ProvideEntry<L, R, V>>(false))?
                .add(Demand::cell::<ProvideLeft<L, R, V>>(true))?
                .add(Demand::cell::<Guarded<R::Output, L::Error, V>>(true))
        })();
        demand?.add(output_origin_shape::<Self::Input, V, Self::Error>())
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Input>,
        context: &PreparedRunContext<V, Self::Error>,
        scope: &PreparedScope<Self::Error>,
        protected: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Input, V, Self::Error>(frames, environment, context)?;
        let admitted = (|| {
            let (publisher, projected) = Environment::<L::Output>::reserve(frames, context)?;
            Ok(ProvideAdmission {
                entry: frames.reserve()?,
                ready: ProvideReady {
                    left: L::reserve(frames, &storage.0, environment, context, scope, protected)?,
                    right: R::reserve(frames, &storage.1, &projected, context, scope, protected)?,
                    publisher,
                    first: frames.reserve()?,
                    terminal: frames.reserve()?,
                    context: context.try_clone()?,
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
        finish: Continuation<Self::Output, Self::Error>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.entry,
            ProvideEntry {
                description: self,
                ready: admission.ready,
                finish: finish.0,
            },
        )))
    }
}
