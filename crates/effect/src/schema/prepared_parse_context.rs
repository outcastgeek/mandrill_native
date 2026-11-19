//! Finite backing for existing ParseContext roots and immutable path snapshots.
use super::{IssueBudget, IssueSegment, ParseContext, SchemaPolicy};
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use std::cell::UnsafeCell;
use std::sync::{Arc, Mutex, MutexGuard};

/// Physical invocation roots and independently retained immutable paths.
#[derive(Clone, Copy, Debug)]
pub struct PreparedParseContextCapacity {
    /// Simultaneously retained root counter records.
    pub roots: usize,
    /// Simultaneously retained path snapshots.
    pub paths: usize,
    /// Maximum segments retained in each path snapshot.
    pub segments_per_path: usize,
}
/// Region bytes and separately classified startup native controls.
#[derive(Clone, Copy, Debug)]
pub struct PreparedParseContextDemand {
    /// Region payload and metadata; allocator overhead is separate.
    pub region_bytes: Bytes,
    /// Root/path controls and the shared path registry Arc.
    pub startup_arc_controls: usize,
    /// Arc payload bytes, excluding Arc headers and allocator rounding.
    pub startup_arc_payload_bytes: Bytes,
}
#[derive(Default)]
pub(super) struct Counts {
    pub nodes: usize,
    pub pattern: usize,
    pub issues: usize,
}
struct RootControl {
    counts: SharedSlot<'static, Mutex<Counts>>,
}
type RootRegistry = Mutex<FixedVector<'static, Arc<RootControl>>>;
#[derive(Clone)]
pub(super) struct RootRef {
    control: Arc<RootControl>,
    validation: Option<Arc<super::PreparedJsonValidationPool>>,
    issues: Option<Arc<super::PreparedIssueMemory>>,
}
impl RootRef {
    pub fn issue_memory(&self) -> Option<&super::PreparedIssueMemory> {
        self.issues.as_deref()
    }
    pub fn validation(&self) -> Option<&super::PreparedJsonValidationPool> {
        self.validation.as_deref()
    }
    pub fn lock(&self) -> MutexGuard<'_, Counts> {
        self.control
            .counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
struct PathPayload {
    loans: Mutex<usize>,
    values: UnsafeCell<FixedVector<'static, IssueSegment>>,
}
// SAFETY: only an exclusive unpublished vacancy writes; after publication every
// reader retains a loan. Last-loan cleanup starts only after all readers retire.
unsafe impl Sync for PathPayload {}
struct PathControl {
    payload: SharedSlot<'static, PathPayload>,
}
pub(super) enum PathError {
    Resource(super::SchemaResource),
    Memory(MemoryError),
}
type PathRegistry = Mutex<FixedVector<'static, Arc<PathControl>>>;
struct Paths {
    registry: SharedSlot<'static, PathRegistry>,
}
pub(super) struct PathRef {
    control: Arc<PathControl>,
    paths: Arc<Paths>,
}
impl Clone for PathRef {
    fn clone(&self) -> Self {
        let control = Arc::clone(&self.control);
        *control
            .payload
            .loans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        Self {
            control,
            paths: Arc::clone(&self.paths),
        }
    }
}
impl PathRef {
    fn values(&self) -> &[IssueSegment] {
        // SAFETY: a live loan prevents any mutation or final cleanup of this snapshot.
        unsafe { (&*self.control.payload.values.get()).as_slice() }
    }
    pub fn child(&self, segment: IssueSegment) -> Result<Self, PathError> {
        let capacity = self
            .values()
            .len()
            .checked_add(1)
            .ok_or(PathError::Resource(super::SchemaResource::Depth))?;
        // SAFETY: this live immutable loan prevents path mutation or reclamation.
        let limit = unsafe { (&*self.control.payload.values.get()).capacity() };
        if capacity > limit {
            return Err(PathError::Resource(super::SchemaResource::Depth));
        }
        let mut vacancy = self
            .paths
            .reserve()
            .map_err(|_| PathError::Resource(super::SchemaResource::ContextPaths))?;
        for item in self.values() {
            vacancy
                .push(item.try_clone_prepared().map_err(PathError::Memory)?)
                .map_err(|_| PathError::Resource(super::SchemaResource::Depth))?;
        }
        vacancy
            .push(segment)
            .map_err(|_| PathError::Resource(super::SchemaResource::Depth))?;
        Ok(vacancy.publish(Arc::clone(&self.paths)))
    }
}
impl Drop for PathRef {
    fn drop(&mut self) {
        let last = {
            let mut loans = self
                .control
                .payload
                .loans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *loans -= 1;
            *loans == 0
        };
        if last {
            // SAFETY: the last loan is gone and this Arc prevents registry reuse;
            // each segment is removed and dropped without holding the loan gate.
            let values = unsafe { &mut *self.control.payload.values.get() };
            while let Some(value) = values.remove_first() {
                drop(value);
            }
        }
    }
}
struct PathVacancy {
    control: Arc<PathControl>,
    published: bool,
}
impl PathVacancy {
    fn push(&mut self, value: IssueSegment) -> Result<(), IssueSegment> {
        // SAFETY: the sole unpublished vacancy owns this initialized fixed backing.
        let values = unsafe { &mut *self.control.payload.values.get() };
        values.push(value)
    }
    fn publish(mut self, paths: Arc<Paths>) -> PathRef {
        *self
            .control
            .payload
            .loans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 1;
        self.published = true;
        PathRef {
            control: Arc::clone(&self.control),
            paths,
        }
    }
}
impl Drop for PathVacancy {
    fn drop(&mut self) {
        if !self.published {
            // SAFETY: unpublished vacancy exclusively owns its physical cell.
            let values = unsafe { &mut *self.control.payload.values.get() };
            while let Some(value) = values.remove_first() {
                drop(value);
            }
        }
    }
}
impl Paths {
    fn reserve(&self) -> Result<PathVacancy, MemoryError> {
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for control in registry.as_mut_slice() {
            if Arc::get_mut(control).is_some() {
                return Ok(PathVacancy {
                    control: Arc::clone(control),
                    published: false,
                });
            }
        }
        Err(MemoryError::NoSlot)
    }
}
/// Caller-prepared roots and paths; context clones allocate no control blocks.
/// Pass an admitted context to `PureCodec::decode_ref_in` or `encode_ref_in`.
/// Path segments retain their existing owners; SchemaIssue payloads and JSON
/// representation traversal remain separately allocated by ordinary paths.
pub struct PreparedParseContextPool {
    roots: SharedSlot<'static, RootRegistry>,
    paths: Arc<Paths>,
    validation: Option<Arc<super::PreparedJsonValidationPool>>,
    issues: Option<Arc<super::PreparedIssueMemory>>,
}
impl PreparedParseContextCapacity {
    /// Physical selected-region bytes and separately requested startup Arc payloads.
    pub fn demand(self) -> Result<PreparedParseContextDemand, MemoryError> {
        let root_index = std::alloc::Layout::array::<Arc<RootControl>>(self.roots)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let path_index = std::alloc::Layout::array::<Arc<PathControl>>(self.paths)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let segments = std::alloc::Layout::array::<IssueSegment>(self.segments_per_path)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let region_bytes = SlotPool::<Mutex<Counts>>::storage_bytes(self.roots)?
            .checked_add(SlotPool::<PathPayload>::storage_bytes(self.paths)?)?
            .checked_add(SlotPool::<RootRegistry>::storage_bytes(1)?)?
            .checked_add(SlotPool::<PathRegistry>::storage_bytes(1)?)?
            .checked_add(Bytes::new(root_index.size()))?
            .checked_add(Bytes::new(path_index.size()))?
            .checked_add(Bytes::new(segments.size()).checked_mul(self.paths)?)?;
        let controls = self
            .roots
            .checked_add(self.paths)
            .and_then(|n| n.checked_add(1))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let payload = Bytes::new(std::mem::size_of::<RootControl>())
            .checked_mul(self.roots)?
            .checked_add(Bytes::new(std::mem::size_of::<PathControl>()).checked_mul(self.paths)?)?
            .checked_add(Bytes::new(std::mem::size_of::<Paths>()))?;
        Ok(PreparedParseContextDemand {
            region_bytes,
            startup_arc_controls: controls,
            startup_arc_payload_bytes: payload,
        })
    }
}
impl PreparedParseContextPool {
    /// Prepare all selected-region records before allocating startup Arc controls.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedParseContextCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand()?;
        let roots = memory.slots(region, capacity.roots, |_| {
            Ok(Mutex::new(Counts::default()))
        })?;
        let paths = memory.slots(region, capacity.paths, |_| {
            Ok(PathPayload {
                loans: Mutex::new(0),
                values: UnsafeCell::new(FixedVector::prepare(
                    memory.allocator(region)?,
                    capacity.segments_per_path,
                )?),
            })
        })?;
        let mut root_index = Some(FixedVector::prepare(
            memory.allocator(region)?,
            capacity.roots,
        )?);
        let mut path_index = Some(FixedVector::prepare(
            memory.allocator(region)?,
            capacity.paths,
        )?);
        let root_registry = memory.slots(region, 1, |_| {
            Ok(Mutex::new(root_index.take().ok_or(MemoryError::NoSlot)?))
        })?;
        let path_registry = memory.slots(region, 1, |_| {
            Ok(Mutex::new(path_index.take().ok_or(MemoryError::NoSlot)?))
        })?;
        let mut root_registry = root_registry.checkout()?;
        let mut path_registry = path_registry.checkout()?;
        for _ in 0..capacity.roots {
            root_registry
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(Arc::new(RootControl {
                    counts: roots.checkout()?.share(),
                }))
                .map_err(|_| MemoryError::NoSlot)?;
        }
        for _ in 0..capacity.paths {
            path_registry
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(Arc::new(PathControl {
                    payload: paths.checkout()?.share(),
                }))
                .map_err(|_| MemoryError::NoSlot)?;
        }
        Ok(Self {
            roots: root_registry.share(),
            paths: Arc::new(Paths {
                registry: path_registry.share(),
            }),
            validation: None,
            issues: None,
        })
    }
    /// Select prepared representation-check scratch and counting writer backing.
    #[must_use]
    pub fn with_json_validation(
        mut self,
        validation: Arc<super::PreparedJsonValidationPool>,
    ) -> Self {
        self.validation = Some(validation);
        self
    }
    /// Select actual prepared path and child ownership for diagnostic operations.
    #[must_use]
    pub fn with_issue_memory(mut self, issues: Arc<super::PreparedIssueMemory>) -> Self {
        self.issues = Some(issues);
        self
    }
    /// Admit a context only when prepared JSON representation validation was selected.
    pub fn try_json_context(&self, policy: SchemaPolicy) -> Result<ParseContext, MemoryError> {
        if self.validation.is_none() {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.try_context(policy)
    }
    /// Admit both a physical root and initial immutable path before publication.
    pub fn try_context(&self, policy: SchemaPolicy) -> Result<ParseContext, MemoryError> {
        let root = {
            let mut roots = self
                .roots
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let root = roots
                .as_mut_slice()
                .iter_mut()
                .find_map(|control| {
                    if Arc::get_mut(control).is_some() {
                        Some(Arc::clone(control))
                    } else {
                        None
                    }
                })
                .ok_or(MemoryError::NoSlot)?;
            RootRef {
                control: root,
                validation: self.validation.clone(),
                issues: self.issues.clone(),
            }
        };
        *root.lock() = Counts::default();
        let path = self.paths.reserve()?.publish(Arc::clone(&self.paths));
        let issues = IssueBudget::prepared(policy.parse.limits.issues, root.clone());
        Ok(ParseContext::prepared(policy, root, path, issues))
    }
}
