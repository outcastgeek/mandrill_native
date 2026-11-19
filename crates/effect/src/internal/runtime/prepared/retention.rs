//! Pre-work typed transfer owners. The output consumer, not completion of a
//! callback or the root environment, determines final reclamation.
use super::*;
use crate::memory::{FrameReservation, RetainedFrameValue, RuntimeFrame};
use std::any::TypeId;
use std::ptr::NonNull;

/// Equality-only cookie. A separately retained Environment owns the actual
/// allocation throughout selection; this value never grants access to it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct EnvironmentKey {
    pub(super) kind: u8,
    pub(super) address: usize,
    pub(super) ty: TypeId,
}

pub(super) struct Owners {
    head: Option<RuntimeFrame>,
}
impl Owners {
    pub(super) fn new() -> Self {
        Self { head: None }
    }
    fn take(&mut self) -> Option<RuntimeFrame> {
        self.head.take()
    }
}
/// Actual region-backed iteration origin list. Context references do not count
/// as consumers; the final consumer breaks footer/context cycles by taking the
/// complete head before dropping it outside this gate.
pub(super) struct OriginGroup {
    owners: Mutex<Owners>,
    consumers: AtomicUsize,
    config: Mutex<Option<GroupConfig>>,
}
struct GroupConfig {
    base: EnvironmentKey,
    parent: Option<SharedSlot<'static, OriginGroup>>,
    frames: FramePool<'static>,
}
impl OriginGroup {
    pub(super) fn new() -> Self {
        Self {
            owners: Mutex::new(Owners::new()),
            consumers: AtomicUsize::new(0),
            config: Mutex::new(None),
        }
    }
    /// Claim an actual vacant group and install its initial counted iteration
    /// owner. All fallible clones precede publication of the new context.
    pub(super) fn begin<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
        pool: &SlotPool<'static, Self>,
        parent: &PreparedRunContext<A, E>,
        environment: &super::graph::Environment<R>,
        frames: &FramePool<'static>,
    ) -> Result<(PreparedRunContext<A, E>, GroupLease), MemoryError> {
        let group = pool.try_checkout()?.share();
        let selected = group.try_clone()?;
        let mut context = parent.try_clone()?;
        let (persistent, frames) = match parent.persistent_selection()? {
            Some(selection) => selection,
            None => (parent.try_clone()?, frames.try_clone()?),
        };
        let parent_group = persistent
            .origin_group
            .as_ref()
            .map(SharedSlot::try_clone)
            .transpose()?;
        let config = GroupConfig {
            base: environment.origin_key(),
            parent: parent_group,
            frames,
        };
        {
            let owners = group
                .owners
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if owners.head.is_some() || group.consumers.load(Ordering::Acquire) != 0 {
                return Err(MemoryError::Busy);
            }
            let mut existing = group
                .config
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if existing.is_some() {
                return Err(MemoryError::Busy);
            }
            *existing = Some(config);
            group.consumers.store(1, Ordering::Release);
        }
        context.origin_group = Some(selected);
        Ok((context, GroupLease { group }))
    }
}
pub(super) struct GroupLease {
    group: SharedSlot<'static, OriginGroup>,
}
impl GroupLease {
    fn acquire(group: SharedSlot<'static, OriginGroup>) -> Result<Self, MemoryError> {
        group
            .consumers
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                if count == 0 {
                    None
                } else {
                    count.checked_add(1)
                }
            })
            .map_err(|count| {
                if count == 0 {
                    MemoryError::Closed
                } else {
                    MemoryError::OwnerLimit
                }
            })?;
        Ok(Self { group })
    }
}
impl Drop for GroupLease {
    fn drop(&mut self) {
        if self.group.consumers.fetch_sub(1, Ordering::AcqRel) == 1 {
            let head = self
                .group
                .owners
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let config = self
                .group
                .config
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            // The prepared drain retires a queued node and its descendants
            // before later siblings. Thus this head and its descendants retire
            // before the root lease field can enqueue the root's origins.
            drop(head);
            drop(config);
        }
    }
}
enum OwnerConsumer<A: Send + 'static, E: Send + 'static> {
    Root(OutputLease<A, E>),
    // Field order retires the group head before releasing the root consumer.
    Group {
        group: GroupLease,
        _root: OutputLease<A, E>,
    },
}
impl<A: Send + 'static, E: Send + 'static> OwnerConsumer<A, E> {
    fn acquire(context: &PreparedRunContext<A, E>) -> Result<Self, MemoryError> {
        let root = OutputLease::acquire(context.root.try_clone()?)?;
        if let Some(group) = &context.origin_group {
            Ok(Self::Group {
                group: GroupLease::acquire(group.try_clone()?)?,
                _root: root,
            })
        } else {
            Ok(Self::Root(root))
        }
    }
    fn owners(&self) -> &Mutex<Owners> {
        match self {
            Self::Root(root) => &root.root.owners,
            Self::Group { group, .. } => &group.group.owners,
        }
    }
}
struct Footer<A: Send + 'static, E: Send + 'static> {
    _previous: Option<RuntimeFrame>,
    _context: PreparedRunContext<A, E>,
}
type Record<T, A, E> = RetainedFrameValue<T, Footer<A, E>>;
/// Immutable consumer-coupled view of an actual typed origin cell. The value
/// cannot be extracted or moved; this view is itself an output consumer.
pub struct PreparedOutputOwnerView<T: Send + 'static, A: Send + 'static, E: Send + 'static> {
    value: NonNull<T>,
    _consumer: OwnerConsumer<A, E>,
}
// SAFETY: the private constructor installs T in stable region storage, and the
// output consumer prevents any retirement while this view exists. No mutable
// projection exists; cross-thread borrowing additionally requires T: Sync.
unsafe impl<T: Send, A: Send, E: Send> Send for PreparedOutputOwnerView<T, A, E> {}
unsafe impl<T: Send + Sync, A: Send, E: Send> Sync for PreparedOutputOwnerView<T, A, E> {}
impl<T: Send, A: Send, E: Send> Deref for PreparedOutputOwnerView<T, A, E> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: exact constructor pairing; consumer keeps the origin live.
        unsafe { self.value.as_ref() }
    }
}
/// A concrete transfer cell acquired before business/native construction. It
/// cannot grow, allocate metadata, or strip the originating context authority.
pub struct PreparedOutputOwnerReservation<T: Send + 'static, A: Send + 'static, E: Send + 'static> {
    frame: FrameReservation<'static, Record<T, A, E>>,
    context: PreparedRunContext<A, E>,
    consumer: OwnerConsumer<A, E>,
}
impl<T: Send + 'static, A: Send + 'static, E: Send + 'static>
    PreparedOutputOwnerReservation<T, A, E>
{
    #[must_use]
    pub fn layout() -> Layout {
        Layout::new::<Record<T, A, E>>()
    }
    /// Transfer the real storage/budget owner into the originating run's output.
    /// The reservation already owns an output consumer; a closed output refuses
    /// reservation, not a late handoff. This does not detach native pointees.
    pub fn attach(self, value: T) -> Result<(), (MemoryError, T)> {
        self.attach_retained(value).map(drop)
    }
    /// Install the origin and return a pre-admitted consumer of that exact cell.
    /// No bare value is returned; dropping the original output cannot release
    /// this view's budget/backing. Even the consumer was acquired before work.
    pub fn attach_retained(
        self,
        value: T,
    ) -> Result<PreparedOutputOwnerView<T, A, E>, (MemoryError, T)> {
        Ok(self.install_for_invocation(value))
    }
    /// Private infallible install for the sealed engine handoff. The pre-admitted
    /// consumer stays live through the continuation call, and is NOT stored in
    /// its originating root outcome/origin value.
    pub(crate) fn install_for_invocation(self, value: T) -> PreparedOutputOwnerView<T, A, E> {
        let pointer = {
            let mut owners = self
                .consumer
                .owners()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let previous = owners.take();
            let frame = self.frame.retain_parts(
                value,
                Footer {
                    _previous: previous,
                    _context: self.context,
                },
            );
            // SAFETY: retain_parts installed exactly this Record; no public
            // mutation exists and the pre-held consumer prevents reclamation.
            let pointer = unsafe { Record::<T, A, E>::value_pointer(frame.pointer().cast()) };
            owners.head = Some(frame);
            pointer
        };
        PreparedOutputOwnerView {
            value: pointer,
            _consumer: self.consumer,
        }
    }
}
impl<A: Send + 'static, E: Send + 'static> PreparedRunContext<A, E> {
    /// Borrow the configured persistent parent authority, or leave the normal
    /// root path untouched when this context is not in an iteration group.
    pub(super) fn persistent_selection(
        &self,
    ) -> Result<Option<(Self, FramePool<'static>)>, MemoryError> {
        let Some(group) = &self.origin_group else {
            return Ok(None);
        };
        let lease = GroupLease::acquire(group.try_clone()?)?;
        let (parent, frames) = {
            let config = group
                .config
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let config = config.as_ref().ok_or(MemoryError::Closed)?;
            (
                config
                    .parent
                    .as_ref()
                    .map(SharedSlot::try_clone)
                    .transpose()?,
                config.frames.try_clone()?,
            )
        };
        let mut context = self.try_clone()?;
        context.origin_group = parent;
        drop(lease);
        Ok(Some((context, frames)))
    }
    pub fn reserve_output_owner<T: Send + 'static>(
        &self,
        frames: &FramePool<'static>,
    ) -> Result<PreparedOutputOwnerReservation<T, A, E>, MemoryError> {
        if let Some((context, frames)) = self.persistent_selection()? {
            return context.reserve_output_owner_here(&frames);
        }
        self.reserve_output_owner_here(frames)
    }
    fn reserve_output_owner_here<T: Send + 'static>(
        &self,
        frames: &FramePool<'static>,
    ) -> Result<PreparedOutputOwnerReservation<T, A, E>, MemoryError> {
        Ok(PreparedOutputOwnerReservation {
            frame: frames.reserve()?,
            context: self.try_clone()?,
            consumer: OwnerConsumer::acquire(self)?,
        })
    }
    pub(super) fn reserve_environment_owner<T: Send + 'static>(
        &self,
        frames: &FramePool<'static>,
        key: EnvironmentKey,
    ) -> Result<PreparedOutputOwnerReservation<T, A, E>, MemoryError> {
        if let Some(group) = &self.origin_group {
            let lease = GroupLease::acquire(group.try_clone()?)?;
            let matches = {
                let config = group
                    .config
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                config.as_ref().ok_or(MemoryError::Closed)?.base == key
            };
            if matches {
                let result = self.reserve_output_owner_here(frames);
                drop(lease);
                return result;
            }
            drop(lease);
        }
        self.reserve_output_owner(frames)
    }
}
/// A shared immutable PREFIX of actual origin records, never a consumer of its
/// own root output. A submitted job/result can outlive that output without a
/// root -> result/fiber -> counted-consumer cycle. Only the affine binder writes
/// the empty cell; private holders cannot inspect or extract origin payloads.
pub(crate) struct OriginSnapshot {
    owner: crate::memory::SharedRuntimeFrame,
}
pub(crate) struct OriginSnapshotReservation<A: Send + 'static, E: Send + 'static> {
    anchor: PreparedOutputOwnerReservation<OriginSnapshot, A, E>,
    snapshot: OriginSnapshot,
}
/// Actual D retirement storage is reserved before factories. Its payload is D;
/// its phased footer is the source origin prefix. Neither is pointer accounting.
pub(crate) struct NativeGraphOrigins<D> {
    frame: RuntimeFrame,
    install: fn(&mut RuntimeFrame, D),
}
impl<D> NativeGraphOrigins<D> {
    pub(crate) fn retire(mut self, budget: Option<D>) {
        if let Some(budget) = budget {
            (self.install)(&mut self.frame, budget);
        }
        drop(self.frame);
    }
}
impl<D: Send + 'static> NativeGraphOrigins<D> {
    pub(crate) fn layout() -> Layout {
        Layout::new::<RetainedFrameValue<Option<D>, OriginSnapshot>>()
    }
}
impl<A: Send + 'static, E: Send + 'static> OriginSnapshotReservation<A, E> {
    pub(crate) fn layout() -> Layout {
        Layout::new::<RetainedFrameValue<Mutex<Option<RuntimeFrame>>, PreparedRunContext<A, E>>>()
    }
    /// A real immutable prefix guard for late cross-root environment transfer.
    /// No placeholder D/byte allowance is created or inferred here.
    pub(crate) fn reserve_prefix(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<A, E>,
    ) -> Result<(Self, OriginSnapshot), MemoryError> {
        let selection = context.persistent_selection()?;
        let (context, frames) = selection
            .as_ref()
            .map_or((context, frames), |(context, frames)| (context, frames));
        let owner = frames
            .reserve::<RetainedFrameValue<Mutex<Option<RuntimeFrame>>, PreparedRunContext<A, E>>>()?
            .share_parts(Mutex::new(None), context.try_clone()?);
        let view = OriginSnapshot {
            owner: owner.try_clone()?,
        };
        Ok((
            Self {
                anchor: context.reserve_output_owner_here(frames)?,
                snapshot: OriginSnapshot { owner },
            },
            view,
        ))
    }
    pub(crate) fn reserve<D: Send + 'static>(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<A, E>,
    ) -> Result<(Self, NativeGraphOrigins<D>), MemoryError> {
        let selection = context.persistent_selection()?;
        let (context, frames) = selection
            .as_ref()
            .map_or((context, frames), |(context, frames)| (context, frames));
        fn install<D: Send + 'static>(frame: &mut RuntimeFrame, budget: D) {
            // SAFETY: paired only with the exact retained None<D> frame below;
            // private, unpinned, written once after all F/T descendants retire.
            let mut value = unsafe {
                RetainedFrameValue::<Option<D>, OriginSnapshot>::value_pointer(
                    frame.pointer().cast(),
                )
            };
            unsafe {
                *value.as_mut() = Some(budget);
            }
        }
        let owner = frames
            .reserve::<RetainedFrameValue<Mutex<Option<RuntimeFrame>>, PreparedRunContext<A, E>>>()?
            .share_parts(Mutex::new(None), context.try_clone()?);
        let job = OriginSnapshot {
            owner: owner.try_clone()?,
        };
        let frame = frames
            .reserve::<RetainedFrameValue<Option<D>, OriginSnapshot>>()?
            .retain_parts(None, job);
        let binder = Self {
            anchor: context.reserve_output_owner_here(frames)?,
            snapshot: OriginSnapshot { owner },
        };
        Ok((
            binder,
            NativeGraphOrigins {
                frame,
                install: install::<D>,
            },
        ))
    }
    pub(crate) fn bind(self) {
        let PreparedOutputOwnerReservation {
            frame,
            context,
            consumer,
        } = self.anchor;
        {
            let mut owners = consumer
                .owners()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let previous = owners.take();
            // SAFETY: this affine reservation created this exact shared cell;
            // its other owner is private retention only, never a reader/writer.
            let data = unsafe {
                RetainedFrameValue::<Mutex<Option<RuntimeFrame>>,PreparedRunContext<A,E>>::value_pointer(self.snapshot.owner.pointer().cast()).as_ref()
            };
            *data
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = previous;
            // The old head moved INTO the snapshot. Never also link it as this
            // anchor's predecessor, and never capture the new anchor in itself.
            owners.head = Some(frame.retain_parts(
                self.snapshot,
                Footer {
                    _previous: None,
                    _context: context,
                },
            ));
        }
        drop(consumer);
    }
}
/// Every public result/future view counts as a consumer, even after mapping or
/// Ready publication. Last release breaks deliberate root->origin->root links
/// outside all gates, after value destruction and before returning row capacity.
pub(super) struct OutputLease<A: Send + 'static, E: Send + 'static> {
    pub(super) root: SharedSlot<'static, Root<A, E>>,
}
impl<A: Send + 'static, E: Send + 'static> Deref for OutputLease<A, E> {
    type Target = Root<A, E>;
    fn deref(&self) -> &Self::Target {
        &self.root
    }
}
impl<A: Send + 'static, E: Send + 'static> OutputLease<A, E> {
    pub(super) fn acquire(root: SharedSlot<'static, Root<A, E>>) -> Result<Self, MemoryError> {
        root.consumers
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                if count == 0 {
                    None
                } else {
                    count.checked_add(1)
                }
            })
            .map_err(|count| {
                if count == 0 {
                    MemoryError::Closed
                } else {
                    MemoryError::OwnerLimit
                }
            })?;
        Ok(Self { root })
    }
    pub(super) fn try_clone(&self) -> Result<Self, MemoryError> {
        Self::acquire(self.root.try_clone()?)
    }
}
impl<A: Send + 'static, E: Send + 'static> Drop for OutputLease<A, E> {
    fn drop(&mut self) {
        if self.root.consumers.fetch_sub(1, Ordering::AcqRel) == 1 {
            // Every pending attachment/view already holds a consumer. At zero
            // none can race this final release. No user Drop runs under gates.
            // FIFO/postorder prepared retirement makes value descendants finish
            // before queued origins, including inside an existing active drain.
            let owners = self
                .root
                .owners
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let value = self
                .root
                .output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let construction = self
                .root
                .construction
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(value);
            drop(construction);
            drop(owners);
        }
    }
}
