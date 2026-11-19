//! Fixed backing for the existing JSON writer and its codec boundaries.
use super::{JsonWriterSink, SchemaIssue};
use crate::memory::{
    Admission, AdmissionBundle, Buffer, Bytes, CapacityUnit, FixedVector, FramePool, MemoryError,
    PreparedMemory, RegionId, RetainedFrameValue, RuntimeFrame, SharedSlot, SlotLease, SlotPool,
};

/// Caller-selected physical JSON writer and codec-boundary backing.
/// Storage exhaustion reports a `SchemaIssueKind::Memory` cause, distinct from
/// the logical byte and depth limits in `JsonWriterPolicy`; boundary snapshot
/// scarcity retains its `WriterBoundaries` category.
#[derive(Clone, Copy, Debug)]
pub struct PreparedJsonWriterCapacity {
    /// Simultaneously admitted writers.
    pub writers: usize,
    /// Buffered output bytes per writer (unused by sink-mode writers).
    pub output_bytes: usize,
    /// Retained property-name bytes per writer.
    pub name_bytes: usize,
    /// Concurrent retained property names per writer.
    pub names: usize,
    /// Concurrent open container frames per writer.
    pub depth: usize,
    /// Fixed tree traversal depth per writer.
    pub tree_frames: usize,
    /// Concurrent codec boundary snapshots across this pool.
    pub boundaries: usize,
}
#[derive(Clone, Copy)]
pub(super) struct TreeFrame {
    pub next: usize,
}
#[derive(Clone, Copy)]
pub(super) struct NameSpan {
    pub start: usize,
    pub end: usize,
}
#[derive(Clone, Copy)]
pub(super) enum Container {
    Array {
        serial: u64,
        count: usize,
    },
    Object {
        serial: u64,
        names_base: usize,
        bytes_base: usize,
        count: usize,
        pending: bool,
    },
}
#[derive(Clone, Copy)]
pub(super) enum BoundaryFrame {
    Array {
        serial: u64,
        count: usize,
    },
    Object {
        serial: u64,
        count: usize,
        pending: bool,
    },
}
pub(super) struct Storage {
    pub output: Buffer<'static>,
    pub names: Buffer<'static>,
    pub index: FixedVector<'static, NameSpan>,
    pub containers: FixedVector<'static, Container>,
    pub tree: FixedVector<'static, TreeFrame>,
    pub serial: u64,
}
pub(super) struct Snapshot {
    pub frames: FixedVector<'static, BoundaryFrame>,
}
pub(crate) struct Boundary {
    pub(super) snapshot: SlotLease<'static, Snapshot>,
    pub(super) token: SharedSlot<'static, ()>,
    pub(super) root: bool,
}
impl Drop for Boundary {
    fn drop(&mut self) {
        self.snapshot.frames.clear();
    }
}

/// Physically prepared finite writers and snapshot records. Sink frame pool is caller-owned.
pub struct PreparedJsonWriterPool {
    pub(super) storage: SlotPool<'static, Storage>,
    pub(super) tokens: SlotPool<'static, ()>,
    pub(super) snapshots: SlotPool<'static, Snapshot>,
    pub(super) frames: FramePool<'static>,
    pub(super) issues: Option<std::sync::Arc<super::PreparedIssueMemory>>,
    admission: Option<Admission<'static, 1>>,
}
/// A fully admitted sink writer, before its sink value is constructed.
pub struct PreparedJsonWriterSinkReservation<T: JsonWriterSink> {
    frame: crate::memory::FrameReservation<'static, RetainedFrameValue<T, Core>>,
    core: Core,
    policy: super::JsonWriterPolicy,
}
impl<T: JsonWriterSink> PreparedJsonWriterSinkReservation<T> {
    /// Publish the concrete sink into its already-reserved frame without further admission.
    #[must_use]
    pub fn publish(self, sink: T) -> super::JsonWriter {
        let sink = Sink::new(self.frame, sink, self.core);
        super::JsonWriter::from_prepared(
            self.policy,
            Owner {
                backing: Backing::Sink(sink),
            },
        )
    }
}
/// A completed buffered JSON output retaining its actual region-backed writer slot.
pub struct PreparedJsonOutput {
    pub(super) owner: Owner,
}
impl PreparedJsonOutput {
    /// Borrow the retained complete JSON bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.owner.storage.output.as_slice()
    }
}
impl PreparedJsonWriterCapacity {
    /// Actual region payload and pool bookkeeping; frame pool backing is separately supplied.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let names = std::alloc::Layout::array::<NameSpan>(self.names)
            .map_err(|_| MemoryError::InvalidLayout)?
            .size();
        let containers = std::alloc::Layout::array::<Container>(self.depth)
            .map_err(|_| MemoryError::InvalidLayout)?
            .size();
        let tree = std::alloc::Layout::array::<TreeFrame>(self.tree_frames)
            .map_err(|_| MemoryError::InvalidLayout)?
            .size();
        let snapshot = std::alloc::Layout::array::<BoundaryFrame>(self.depth)
            .map_err(|_| MemoryError::InvalidLayout)?
            .size();
        let writer = Bytes::new(self.output_bytes)
            .checked_add(Bytes::new(self.name_bytes))?
            .checked_add(Bytes::new(names))?
            .checked_add(Bytes::new(containers))?
            .checked_add(Bytes::new(tree))?;
        SlotPool::<Storage>::storage_bytes(self.writers)?
            .checked_add(SlotPool::<()>::storage_bytes(self.writers)?)?
            .checked_add(SlotPool::<Snapshot>::storage_bytes(self.boundaries)?)?
            .checked_add(writer.checked_mul(self.writers)?)?
            .checked_add(Bytes::new(snapshot).checked_mul(self.boundaries)?)
    }
}
impl PreparedJsonWriterPool {
    pub(super) fn issue_memory(&self) -> Option<&super::PreparedIssueMemory> {
        self.issues.as_deref()
    }
    /// Provision all fixed writer and boundary records before writer admission.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedJsonWriterCapacity,
        frames: FramePool<'static>,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.storage_bytes()?;
        let storage = memory.slots(region, capacity.writers, |_| {
            Ok(Storage {
                output: memory.buffer(region, Bytes::new(capacity.output_bytes))?,
                names: memory.buffer(region, Bytes::new(capacity.name_bytes))?,
                index: FixedVector::prepare(memory.allocator(region)?, capacity.names)?,
                containers: FixedVector::prepare(memory.allocator(region)?, capacity.depth)?,
                tree: FixedVector::prepare(memory.allocator(region)?, capacity.tree_frames)?,
                serial: 0,
            })
        })?;
        let tokens = memory.slots(region, capacity.writers, |_| Ok(()))?;
        let snapshots = memory.slots(region, capacity.boundaries, |_| {
            Ok(Snapshot {
                frames: FixedVector::prepare(memory.allocator(region)?, capacity.depth)?,
            })
        })?;
        Ok(Self {
            storage,
            tokens,
            snapshots,
            frames,
            issues: None,
            admission: None,
        })
    }
    /// Select the caller-owned issue backing retained by each admitted writer.
    #[must_use]
    pub fn with_issue_memory(mut self, issues: std::sync::Arc<super::PreparedIssueMemory>) -> Self {
        self.issues = Some(issues);
        self
    }
    /// Couple each actual writer storage slot to one Slots work unit. Frames,
    /// snapshots and native sink allocations are separate.
    pub fn with_admission(mut self, admission: Admission<'static, 1>) -> Result<Self, MemoryError> {
        if admission.capacities()[0].unit != CapacityUnit::Slots {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.admission = Some(admission);
        Ok(self)
    }
    fn reserve_physical(&self) -> Result<Core, MemoryError> {
        let storage = self.storage.checkout()?;
        let token = self.tokens.checkout()?.share();
        let snapshots = self.snapshots.try_clone()?;
        Ok(Core {
            storage,
            token,
            snapshots,
            issues: self.issues.clone(),
            quota: None,
        })
    }
    fn admit_quota(&self, core: &mut Core) -> Result<(), MemoryError> {
        if let Some(admission) = &self.admission {
            core.quota = Some(admission.reserve([1], [0])?);
        }
        Ok(())
    }
    fn reserve(&self) -> Result<Core, MemoryError> {
        let mut core = self.reserve_physical()?;
        self.admit_quota(&mut core)?;
        Ok(core)
    }
    /// Admit a buffered writer whose finished bytes retain the actual prepared slot.
    pub fn admit_buffered(
        &self,
        policy: super::JsonWriterPolicy,
    ) -> Result<super::JsonWriter, MemoryError> {
        Ok(super::JsonWriter::from_prepared(
            policy,
            Owner {
                backing: Backing::Buffered(self.reserve()?),
            },
        ))
    }
    /// Admit a counting writer using the same prepared structural state without output or sink backing.
    pub fn admit_counting(
        &self,
        policy: super::JsonWriterPolicy,
    ) -> Result<super::JsonWriter, MemoryError> {
        Ok(super::JsonWriter::from_prepared(
            policy,
            Owner {
                backing: Backing::Counting(self.reserve()?),
            },
        ))
    }
    pub(crate) fn admit_serde_counting(
        &self,
        policy: super::JsonWriterPolicy,
    ) -> Result<super::JsonWriter, MemoryError> {
        Ok(super::JsonWriter::from_serde_counting(
            policy,
            Owner {
                backing: Backing::Counting(self.reserve()?),
            },
        ))
    }
    pub(crate) fn admit_serde_buffered(
        &self,
        policy: super::JsonWriterPolicy,
    ) -> Result<super::JsonWriter, MemoryError> {
        Ok(super::JsonWriter::from_serde_counting(
            policy,
            Owner {
                backing: Backing::Buffered(self.reserve()?),
            },
        ))
    }
    /// Exact caller-frame layout required for this sink and its retained writer footer.
    #[must_use]
    pub fn sink_layout<T: JsonWriterSink>() -> std::alloc::Layout {
        std::alloc::Layout::new::<RetainedFrameValue<T, Core>>()
    }
    /// Reserve writer, identity, snapshot-pool ownership and concrete sink frame before sink creation.
    /// Each codec boundary subsequently checks out its own actual snapshot before its callback.
    pub fn reserve_sink<T: JsonWriterSink>(
        &self,
        policy: super::JsonWriterPolicy,
    ) -> Result<PreparedJsonWriterSinkReservation<T>, MemoryError> {
        let mut core = self.reserve_physical()?;
        let frame = self.frames.reserve::<RetainedFrameValue<T, Core>>()?;
        self.admit_quota(&mut core)?;
        Ok(PreparedJsonWriterSinkReservation {
            frame,
            core,
            policy,
        })
    }
    /// Admit every physical owner before running the sink factory.
    pub fn admit_sink<T, F>(
        &self,
        policy: super::JsonWriterPolicy,
        factory: F,
    ) -> Result<super::JsonWriter, (MemoryError, F)>
    where
        T: JsonWriterSink,
        F: FnOnce() -> T,
    {
        let reservation = match self.reserve_sink::<T>(policy) {
            Ok(value) => value,
            Err(error) => return Err((error, factory)),
        };
        Ok(reservation.publish(factory()))
    }
}
struct Sink {
    frame: RuntimeFrame,
    core: std::ptr::NonNull<Core>,
    write: fn(&mut RuntimeFrame, &[u8]) -> Result<(), SchemaIssue>,
    finish: fn(&mut RuntimeFrame) -> Result<(), SchemaIssue>,
}
impl Sink {
    fn new<T: JsonWriterSink>(
        reservation: crate::memory::FrameReservation<'static, RetainedFrameValue<T, Core>>,
        value: T,
        core: Core,
    ) -> Self {
        fn write<T: JsonWriterSink>(
            frame: &mut RuntimeFrame,
            bytes: &[u8],
        ) -> Result<(), SchemaIssue> {
            // SAFETY: this vtable is paired with the exact initialized retained T/Core frame;
            // exclusive Sink access prevents aliasing and T is not structurally pinned.
            let value =
                unsafe { RetainedFrameValue::<T, Core>::value_pointer(frame.pointer().cast()) };
            unsafe { value.as_ptr().as_mut() }
                .ok_or_else(|| SchemaIssue::new(super::SchemaIssueKind::InvalidDefinition))?
                .write(bytes)
        }
        fn finish<T: JsonWriterSink>(frame: &mut RuntimeFrame) -> Result<(), SchemaIssue> {
            // SAFETY: the exact same concrete frame and exclusive access proof applies.
            let value =
                unsafe { RetainedFrameValue::<T, Core>::value_pointer(frame.pointer().cast()) };
            unsafe { value.as_ptr().as_mut() }
                .ok_or_else(|| SchemaIssue::new(super::SchemaIssueKind::InvalidDefinition))?
                .finish()
        }
        let frame = reservation.retain_parts(value, core);
        // SAFETY: frame was just initialized with exactly T/Core and owns its live footer.
        let core = unsafe { RetainedFrameValue::<T, Core>::footer_pointer(frame.pointer().cast()) };
        Self {
            frame,
            core,
            write: write::<T>,
            finish: finish::<T>,
        }
    }
}
// SAFETY: the unique frame owns both Send sink and Send footer; the pointer addresses
// that footer only while its frame is alive, and cannot be accessed concurrently.
unsafe impl Send for Sink {}
// SAFETY: construction requires T: JsonWriterSink (therefore Sync); Core is Sync,
// shared access exposes only an immutable footer, and invocation needs &mut Sink.
unsafe impl Sync for Sink where Core: Sync {}
pub(super) struct Core {
    pub storage: SlotLease<'static, Storage>,
    pub token: SharedSlot<'static, ()>,
    pub snapshots: SlotPool<'static, Snapshot>,
    pub issues: Option<std::sync::Arc<super::PreparedIssueMemory>>,
    quota: Option<AdmissionBundle<'static, 1>>,
}
impl Drop for Core {
    fn drop(&mut self) {
        self.storage.output.clear();
        self.storage.names.clear();
        self.storage.index.clear();
        self.storage.containers.clear();
        self.storage.tree.clear();
        self.storage.serial = 0;
    }
}
enum Backing {
    Buffered(Core),
    Counting(Core),
    Sink(Sink),
}
pub(super) struct Owner {
    backing: Backing,
}
impl std::ops::Deref for Owner {
    type Target = Core;
    fn deref(&self) -> &Core {
        match &self.backing {
            Backing::Buffered(core) | Backing::Counting(core) => core,
            Backing::Sink(sink) => {
                // SAFETY: the frame owns its initialized Core footer until its own retirement.
                unsafe { sink.core.as_ref() }
            }
        }
    }
}
impl std::ops::DerefMut for Owner {
    fn deref_mut(&mut self) -> &mut Core {
        match &mut self.backing {
            Backing::Buffered(core) | Backing::Counting(core) => core,
            Backing::Sink(sink) => {
                // SAFETY: exclusive Owner access is the only path to the footer and frame is live.
                unsafe { sink.core.as_mut() }
            }
        }
    }
}
impl Owner {
    pub fn has_sink(&self) -> bool {
        matches!(self.backing, Backing::Sink(_) | Backing::Counting(_))
    }
    pub fn write_sink(&mut self, bytes: &[u8]) -> Option<Result<(), SchemaIssue>> {
        match &mut self.backing {
            Backing::Buffered(_) => None,
            Backing::Counting(_) => Some(Ok(())),
            Backing::Sink(sink) => Some((sink.write)(&mut sink.frame, bytes)),
        }
    }
    pub fn finish_sink(&mut self) -> Option<Result<(), SchemaIssue>> {
        match &mut self.backing {
            Backing::Buffered(_) => None,
            Backing::Counting(_) => Some(Ok(())),
            Backing::Sink(sink) => Some((sink.finish)(&mut sink.frame)),
        }
    }
}
