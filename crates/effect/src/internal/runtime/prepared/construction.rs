//! Explicit pre-business construction sites. The registry stores actual typed
//! region owners, not authority inferred from keys or an unpublished R value.
use super::*;
use crate::memory::{FixedVector, RetainedFrameValue, SharedRuntimeFrame, SlotLease};
use std::any::TypeId;

pub(super) struct Site {
    key: u64,
    kind: TypeId,
    claimed: bool,
    owner: SharedRuntimeFrame,
    bound: fn(&SharedRuntimeFrame) -> bool,
}
pub(super) struct Lane {
    slots: SlotLease<'static, FixedVector<'static, Option<Site>>>,
}
impl Deref for Lane {
    type Target = FixedVector<'static, Option<Site>>;
    fn deref(&self) -> &Self::Target {
        &self.slots
    }
}
impl std::ops::DerefMut for Lane {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slots
    }
}
/// Borrows the still-owned lane storage. Iterator advancement and Option::take
/// both happen before user destruction, so one unwind resumes at the NEXT empty-
/// safe slot. Array length/capacity are unchanged and no allocation is needed.
struct LaneDrain<'a> {
    remaining: std::slice::IterMut<'a, Option<Site>>,
}
impl LaneDrain<'_> {
    fn drain(&mut self) {
        for slot in self.remaining.by_ref() {
            let site = slot.take();
            drop(site);
        }
    }
}
impl Drop for LaneDrain<'_> {
    fn drop(&mut self) {
        // Continue after one payload panic; a second panic during this unwind
        // retains ordinary Rust double-panic behavior. Do not catch or swallow.
        self.drain();
    }
}
impl Drop for Lane {
    fn drop(&mut self) {
        // This local guard is destroyed before Lane's SlotLease field, including
        // on unwind. Every remaining site is taken before the reusable lease can
        // return. Callers have already detached the lane from metadata gates.
        let mut drain = LaneDrain {
            remaining: self.slots.as_mut_slice().iter_mut(),
        };
        drain.drain();
    }
}
/// Startup-provisioned per-invocation construction registry. Site count/capacity
/// is explicitly supplied; no ambient registry or post-seal growth exists.
pub struct PreparedConstructionPool {
    lanes: SlotPool<'static, FixedVector<'static, Option<Site>>>,
}
impl PreparedConstructionPool {
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        runs: usize,
        sites: usize,
    ) -> Result<Self, MemoryError> {
        let lanes = memory.slots(region, runs, |_| {
            let mut slots = FixedVector::prepare(memory.allocator(region)?, sites)?;
            for _ in 0..sites {
                slots.push_reserved(None);
            }
            Ok(slots)
        })?;
        Ok(Self { lanes })
    }
    pub fn storage_bytes(runs: usize, sites: usize) -> Result<Bytes, MemoryError> {
        SlotPool::<FixedVector<'static, Option<Site>>>::storage_bytes(runs)?.checked_add(
            Bytes::new(
                std::alloc::Layout::array::<Option<Site>>(sites)
                    .map_err(|_| MemoryError::InvalidLayout)?
                    .size(),
            )
            .checked_mul(runs)?,
        )
    }
    pub(super) fn checkout(&self) -> Result<Lane, MemoryError> {
        Ok(Lane {
            slots: self.lanes.try_checkout()?,
        })
    }
}
type Validator<T> = fn(&T) -> Result<(), MemoryError>;
struct ConstructionCell<T> {
    value: Option<T>,
    validator: Option<Validator<T>>,
}
type Record<T, A, E> = RetainedFrameValue<Mutex<ConstructionCell<T>>, PreparedRunContext<A, E>>;
fn cell<T: Send + 'static, A: Send + 'static, E: Send + 'static>(
    owner: &SharedRuntimeFrame,
) -> &Mutex<ConstructionCell<T>> {
    // SAFETY: constructors pair exact Record type with this accessor. Registry
    // lookup checks T, and each registry belongs to one actual typed root A/E.
    unsafe { Record::<T, A, E>::value_pointer(owner.pointer().cast()).as_ref() }
}
fn bound<T: Send + 'static, A: Send + 'static, E: Send + 'static>(
    owner: &SharedRuntimeFrame,
) -> bool {
    cell::<T, A, E>(owner)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .value
        .is_some()
}
/// Linear vacancy for actual foreign construction. Capacity and node/consumer
/// owners exist before constructing T. Install does not perform another checkout.
pub struct ConstructionVacancy<T: Send + 'static, A: Send + 'static, E: Send + 'static> {
    owner: SharedRuntimeFrame,
    context: PreparedRunContext<A, E>,
    _consumer: retention::OutputLease<A, E>,
    marker: PhantomData<fn() -> T>,
}
impl<T: Send + 'static, A: Send + 'static, E: Send + 'static> ConstructionVacancy<T, A, E> {
    pub fn install(self, value: T) -> Result<(), (MemoryError, T)> {
        if self.context.root.closed.load(Ordering::Acquire) {
            return Err((MemoryError::Closed, value));
        }
        let mut slot = cell::<T, A, E>(&self.owner)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.value.is_some() {
            return Err((MemoryError::ConstructionAlreadyBound, value));
        }
        if let Some(validate) = slot.validator
            && let Err(error) = validate(&value)
        {
            return Err((error, value));
        }
        slot.value = Some(value);
        Ok(())
    }
}
pub(crate) struct ConstructionValue<T: Send + 'static> {
    owner: SharedRuntimeFrame,
    cell: for<'a> fn(&'a SharedRuntimeFrame) -> &'a Mutex<ConstructionCell<T>>,
}
impl<T: Send + 'static> ConstructionValue<T> {
    pub(crate) fn layout<A: Send + 'static, E: Send + 'static>() -> Layout {
        Layout::new::<Record<T, A, E>>()
    }
    pub(crate) fn native<A: Send + 'static, E: Send + 'static>(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<A, E>,
        factory: impl FnOnce() -> Result<T, MemoryError>,
    ) -> Result<Self, MemoryError> {
        let frame = frames.reserve::<Record<T, A, E>>()?;
        let context = context.try_clone()?;
        let value = factory()?;
        Ok(Self {
            owner: frame.share_parts(
                Mutex::new(ConstructionCell {
                    value: Some(value),
                    validator: None,
                }),
                context,
            ),
            cell: cell::<T, A, E>,
        })
    }
    pub(crate) fn validate_with(
        &self,
        validator: fn(&T) -> Result<(), MemoryError>,
    ) -> Result<(), MemoryError> {
        let mut slot = (self.cell)(&self.owner)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(value) = &slot.value {
            validator(value)?;
        }
        if slot.validator.is_some() {
            return Err(MemoryError::UnsupportedRunContract);
        }
        slot.validator = Some(validator);
        Ok(())
    }
    pub(crate) fn take(self) -> Result<T, MemoryError> {
        let value = (self.cell)(&self.owner)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .value
            .take();
        value.ok_or(MemoryError::ConstructionUnbound)
    }
}
/// Type-level site declaration; tuples form finite native construction shapes.
pub struct ConstructionSite<T, const KEY: u64>(PhantomData<fn() -> T>);
#[must_use]
pub fn construction_site<T, const KEY: u64>() -> ConstructionSite<T, KEY> {
    ConstructionSite(PhantomData)
}
mod sealed {
    pub trait Sealed {}
}
pub trait ConstructionShape: sealed::Sealed + Send + Sync + 'static {
    type Vacancies<A: Send + 'static, E: Send + 'static>;
    fn sites() -> Result<usize, MemoryError>;
    fn demand<A: Send + 'static, E: Send + 'static>() -> Result<PreparedRunDemand, MemoryError>;
    fn reserve<A: Send + 'static, E: Send + 'static>(
        context: &PreparedRunContext<A, E>,
        frames: &FramePool<'static>,
    ) -> Result<Self::Vacancies<A, E>, MemoryError>;
}
impl<T: Send + 'static, const KEY: u64> sealed::Sealed for ConstructionSite<T, KEY> {}
impl<T: Send + 'static, const KEY: u64> ConstructionShape for ConstructionSite<T, KEY> {
    type Vacancies<A: Send + 'static, E: Send + 'static> = ConstructionVacancy<T, A, E>;
    fn sites() -> Result<usize, MemoryError> {
        Ok(1)
    }
    fn demand<A: Send + 'static, E: Send + 'static>() -> Result<PreparedRunDemand, MemoryError> {
        Ok(PreparedRunDemand {
            frame_layout: Layout::new::<Record<T, A, E>>(),
            frames_per_run: 1,
            record_payload: Bytes::new(0),
            panic_sites: 0,
        })
    }
    fn reserve<A: Send + 'static, E: Send + 'static>(
        context: &PreparedRunContext<A, E>,
        frames: &FramePool<'static>,
    ) -> Result<Self::Vacancies<A, E>, MemoryError> {
        let record = frames.reserve::<Record<T, A, E>>()?;
        let context = context.try_clone()?;
        let consumer = retention::OutputLease::acquire(context.root.try_clone()?)?;
        let owner = record.share_parts(
            Mutex::new(ConstructionCell {
                value: None,
                validator: None,
            }),
            context.try_clone()?,
        );
        let bank_owner = owner.try_clone()?;
        {
            let mut registry = context
                .root
                .construction
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let lane = registry
                .as_mut()
                .ok_or(MemoryError::MissingRuntimeStorage)?;
            if lane.as_slice().iter().flatten().any(|site| site.key == KEY) {
                return Err(MemoryError::ConstructionDuplicateKey(KEY));
            }
            let slot = lane
                .as_mut_slice()
                .iter_mut()
                .find(|site| site.is_none())
                .ok_or(MemoryError::NoSlot)?;
            *slot = Some(Site {
                key: KEY,
                kind: TypeId::of::<T>(),
                claimed: false,
                owner: bank_owner,
                bound: bound::<T, A, E>,
            });
        }
        Ok(ConstructionVacancy {
            owner,
            context,
            _consumer: consumer,
            marker: PhantomData,
        })
    }
}
impl sealed::Sealed for () {}
impl ConstructionShape for () {
    type Vacancies<A: Send + 'static, E: Send + 'static> = ();
    fn sites() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn demand<A: Send + 'static, E: Send + 'static>() -> Result<PreparedRunDemand, MemoryError> {
        Ok(PreparedRunDemand {
            frame_layout: Layout::new::<()>(),
            frames_per_run: 0,
            record_payload: Bytes::new(0),
            panic_sites: 0,
        })
    }
    fn reserve<A: Send + 'static, E: Send + 'static>(
        _context: &PreparedRunContext<A, E>,
        _frames: &FramePool<'static>,
    ) -> Result<(), MemoryError> {
        Ok(())
    }
}
impl<L: ConstructionShape, R: ConstructionShape> sealed::Sealed for (L, R) {}
impl<L: ConstructionShape, R: ConstructionShape> ConstructionShape for (L, R) {
    type Vacancies<A: Send + 'static, E: Send + 'static> = (L::Vacancies<A, E>, R::Vacancies<A, E>);
    fn sites() -> Result<usize, MemoryError> {
        L::sites()?
            .checked_add(R::sites()?)
            .ok_or(MemoryError::ArithmeticOverflow)
    }
    fn demand<A: Send + 'static, E: Send + 'static>() -> Result<PreparedRunDemand, MemoryError> {
        let left = L::demand::<A, E>()?;
        let right = R::demand::<A, E>()?;
        Ok(PreparedRunDemand {
            frame_layout: Layout::from_size_align(
                left.frame_layout.size().max(right.frame_layout.size()),
                left.frame_layout.align().max(right.frame_layout.align()),
            )
            .map_err(|_| MemoryError::InvalidLayout)?
            .pad_to_align(),
            frames_per_run: left
                .frames_per_run
                .checked_add(right.frames_per_run)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            record_payload: left.record_payload.checked_add(right.record_payload)?,
            panic_sites: left
                .panic_sites
                .checked_add(right.panic_sites)
                .ok_or(MemoryError::ArithmeticOverflow)?,
        })
    }
    fn reserve<A: Send + 'static, E: Send + 'static>(
        context: &PreparedRunContext<A, E>,
        frames: &FramePool<'static>,
    ) -> Result<Self::Vacancies<A, E>, MemoryError> {
        Ok((L::reserve(context, frames)?, R::reserve(context, frames)?))
    }
}
impl<A: Send + 'static, E: Send + 'static> PreparedRunContext<A, E> {
    pub(crate) fn construction_value<T: Send + 'static>(
        &self,
        key: u64,
    ) -> Result<ConstructionValue<T>, MemoryError> {
        let mut registry = self
            .root
            .construction
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let lane = registry
            .as_mut()
            .ok_or(MemoryError::MissingRuntimeStorage)?;
        let site = lane
            .as_mut_slice()
            .iter_mut()
            .flatten()
            .find(|site| site.key == key)
            .ok_or(MemoryError::ConstructionMissingKey(key))?;
        if site.kind != TypeId::of::<T>() {
            return Err(MemoryError::ConstructionTypeMismatch(key));
        }
        if site.claimed {
            return Err(MemoryError::ConstructionAlreadyClaimed(key));
        }
        let owner = site.owner.try_clone()?;
        site.claimed = true;
        Ok(ConstructionValue {
            owner,
            cell: cell::<T, A, E>,
        })
    }
    pub(super) fn construction_ready(&self) -> Result<(), MemoryError> {
        let registry = self
            .root
            .construction
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let lane = registry
            .as_ref()
            .ok_or(MemoryError::MissingRuntimeStorage)?;
        for site in lane.as_slice().iter().flatten() {
            if !(site.bound)(&site.owner) {
                return Err(MemoryError::ConstructionUnbound);
            }
        }
        Ok(())
    }
}
