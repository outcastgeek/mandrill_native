//! Typed diagnostics for failures outside the domain-error channel.

use std::any::Any;
use std::sync::Arc;

/// Stable identities for infrastructure failures raised by the Effect implementation.
///
/// These conditions occupy the defect channel. Diagnostic wording is for
/// people; use this identity when inspecting or recording a library failure.
/// Serialized variant names are part of the stored-outcome version 1 protocol;
/// renaming one requires a deliberate format migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LibraryDefect {
    /// The required effect recipe is missing.
    MissingEffectRecipe,
    /// Prepared subtree execution preceded its owned environment publication.
    PreparedEnvironmentUnpublished,
    /// Two prepared views claimed the same backing with incompatible types.
    PreparedEnvironmentAliasMismatch,
    /// Accepted physical provider preparation lost its guaranteed queue handoff.
    PreparedProviderHandoffRefused,
    /// A validated affine construction value was missing at execution.
    PreparedConstructionUnbound,
    /// The required interpreter outcome is missing.
    MissingInterpreterOutcome,
    /// The workflow was already consumed by a previous execution.
    WorkflowConsumed,
    /// The binding was already consumed by a previous execution.
    BindingConsumed,
    /// The resource was used after its owning scope closed.
    ResourceScopeEnded,
    /// A release callback attempted to settle the resource twice.
    ResourceReleasedTwice,
    /// The resource body was already consumed by a previous execution.
    ResourceBodyConsumed,
    /// The scope finalizer was already consumed by a previous execution.
    ScopeFinalizerConsumed,
    /// The scope registration has closed admission for new work.
    ScopeRegistrationClosed,
    /// The scope outcome was already consumed by a previous execution.
    ScopeOutcomeConsumed,
    /// The scope owner owner was dropped before settlement.
    ScopeOwnerDropped,
    /// The scope close driver owner was dropped before settlement.
    ScopeCloseDriverDropped,
    /// The run driver owner was dropped before settlement.
    RunDriverDropped,
    /// The run outcome was already consumed by a previous execution.
    RunOutcomeConsumed,
    /// The scheduled task owner was dropped before settlement.
    ScheduledTaskDropped,
    /// The task submission was rejected before ownership transfer.
    TaskSubmissionRejected,
    /// The fiber outcome was already consumed by a previous execution.
    FiberOutcomeConsumed,
    /// The fiber outcome still has shared observers, so owned transfer is unavailable.
    FiberOutcomeShared,
    /// The fiber outcome cannot be moved while another observer owns it.
    FiberObservationOwnership,
    /// The required parallel result was absent.
    ParallelResultMissing,
    /// The deferred outcome was already consumed by a previous execution.
    DeferredOutcomeConsumed,
    /// The deferred outcome still has shared observers, so owned transfer is unavailable.
    DeferredOutcomeShared,
    /// The queue offer was already consumed by a previous execution.
    QueueOfferConsumed,
    /// The scheduled job owner was dropped before settlement.
    ScheduledJobDropped,
    /// The job callback was already consumed by a previous execution.
    JobCallbackConsumed,
    /// The job submission was rejected before ownership transfer.
    JobSubmissionRejected,
    /// Secondary cleanup entries exceeded their reserved capacity.
    CleanupOverflow { omitted: usize, saturated: bool },
    /// Computing clock deadline exceeded its supported range.
    ClockDeadlineOverflow,
    /// Computing schedule count exceeded its supported range.
    ScheduleCountOverflow,
    /// The schedule policy failed validation.
    InvalidSchedulePolicy,
    /// Collecting the stream exceeded the caller’s bound.
    StreamCollectionLimitExceeded,
    /// The required schedule recipe is missing.
    MissingScheduleRecipe,
    /// The provider has closed admission for new work.
    ProviderClosed,
    /// The provider’s admission bound rejected new work.
    ProviderCapacityFull,
    /// Computing provider count exceeded its supported range.
    ProviderCountOverflow,
    /// The provider cannot drive child work independently.
    ProviderWorkersInsufficient,
    /// A native completion callback unwound while settling work.
    NativeCompletionPanicked,
    /// The pool has closed admission for new work.
    PoolClosed,
    /// Computing pool count exceeded its supported range.
    PoolCountOverflow,
    /// The leased pool resource is no longer valid.
    PoolLeaseEnded,
    /// The pool resource owner was dropped before settlement.
    PoolResourceDropped,
    /// The pool release driver owner was dropped before settlement.
    PoolReleaseDriverDropped,
    /// The pool shutdown driver owner was dropped before settlement.
    PoolShutdownDriverDropped,
    /// The cache has closed admission for new work.
    CacheClosed,
    /// The cache shutdown driver owner was dropped before settlement.
    CacheShutdownDriverDropped,
    /// The weak cache load was already consumed by a previous execution.
    WeakCacheLoadConsumed,
    /// The weak cache key owner was dropped before settlement.
    WeakCacheKeyDropped,
    /// A stream reader remains live at the close boundary.
    StreamReaderStillOwned,
    /// The required stream producer slot was absent.
    StreamProducerSlotMissing,
    /// The required stream buffer value was absent.
    StreamBufferValueMissing,
    /// A resolver returned success where failure was required.
    RequestResolverUnexpectedSuccess,
    /// The random sample failed validation.
    InvalidRandomSample,
    /// The sync callback was already consumed by a previous execution.
    SyncCallbackConsumed,
    /// Computing trace count exceeded its supported range.
    TraceCountOverflow,
    /// The future factory was already consumed by a previous execution.
    FutureFactoryConsumed,
    /// An external I/O provider failed.
    ExternalIoFailure,
    /// The utf8 failed validation.
    InvalidUtf8,
}

impl LibraryDefect {
    // Each string presents the corresponding typed condition. No interpreter,
    // recovery, ownership, or provider decision uses this diagnostic text.
    ///
    /// # Returns
    /// Type: `&'static str`.
    const fn message(self) -> &'static str {
        match self {
            Self::MissingEffectRecipe => "Missing Effect recipe",
            Self::PreparedEnvironmentUnpublished => "Prepared environment has not been published",
            Self::PreparedEnvironmentAliasMismatch => "Prepared environment alias type mismatch",
            Self::PreparedProviderHandoffRefused => "Prepared provider handoff invariant violated",
            Self::PreparedConstructionUnbound => "Prepared construction invariant violated",
            Self::MissingInterpreterOutcome => "Interpreter completed without an outcome",
            Self::WorkflowConsumed => "Workflow was consumed",
            Self::BindingConsumed => "Binding was consumed",
            Self::ResourceScopeEnded => "Resource scope has ended",
            Self::ResourceReleasedTwice => "Resource released twice",
            Self::ResourceBodyConsumed => "Resource body was consumed",
            Self::ScopeFinalizerConsumed => "Scope finalizer consumed",
            Self::ScopeRegistrationClosed => "Cannot register in a closing or closed scope",
            Self::ScopeOutcomeConsumed => "Scope cleanup outcome already consumed",
            Self::ScopeOwnerDropped => "Scope owner dropped before closing",
            Self::ScopeCloseDriverDropped => "Scope close driver dropped before settlement",
            Self::RunDriverDropped => "Owned run driver dropped before settlement",
            Self::RunOutcomeConsumed => "Owned run outcome already consumed",
            Self::ScheduledTaskDropped => "Scheduled task dropped before settlement",
            Self::TaskSubmissionRejected => "Task submission rejected by its provider",
            Self::FiberOutcomeConsumed => "Fiber outcome already consumed by an observer or scope",
            Self::FiberOutcomeShared => "Fiber outcome still has shared owners",
            Self::FiberObservationOwnership => "Fiber observation ownership transition was invalid",
            Self::ParallelResultMissing => "Parallel result slot was missing or already filled",
            Self::DeferredOutcomeConsumed => "Deferred outcome already consumed by an observer",
            Self::DeferredOutcomeShared => "Deferred outcome still has shared owners",
            Self::QueueOfferConsumed => "Owned queue offer candidate already consumed",
            Self::ScheduledJobDropped => "Scheduled worker job dropped before settlement",
            Self::JobCallbackConsumed => "Owned worker callback already consumed",
            Self::JobSubmissionRejected => "Worker job submission rejected by its provider",
            Self::CleanupOverflow { .. } => "Cleanup failures exceeded reserved capacity",
            Self::ClockDeadlineOverflow => "Clock deadline exceeds its representable time range",
            Self::ScheduleCountOverflow => "Schedule recurrence count overflowed",
            Self::InvalidSchedulePolicy => "Schedule policy is invalid",
            Self::StreamCollectionLimitExceeded => "Stream exceeded the caller's collection limit",
            Self::StreamProducerSlotMissing => {
                "Stream producer slot was missing during observation"
            }
            Self::StreamBufferValueMissing => "Stream buffer value was missing during admission",
            Self::RequestResolverUnexpectedSuccess => {
                "Failure-only request conversion received success"
            }
            Self::MissingScheduleRecipe => "Schedule recipe was missing",
            Self::ProviderClosed => "Provider admission is closed",
            Self::ProviderCapacityFull => "Provider admission capacity is full",
            Self::ProviderCountOverflow => "Provider operation count overflowed",
            Self::ProviderWorkersInsufficient => "Provider has insufficient independent workers",
            Self::NativeCompletionPanicked => "Native completion callback panicked",
            Self::PoolClosed => "Pool is closed",
            Self::PoolCountOverflow => "Pool resource count overflowed",
            Self::PoolLeaseEnded => "Pool lease has ended",
            Self::PoolResourceDropped => "Pool resource dropped without awaited disposal",
            Self::PoolReleaseDriverDropped => "Pool release driver dropped before settlement",
            Self::PoolShutdownDriverDropped => "Pool shutdown driver dropped before settlement",
            Self::CacheClosed => "Cache is closed",
            Self::CacheShutdownDriverDropped => "Cache shutdown driver dropped before settlement",
            Self::WeakCacheLoadConsumed => "Weak cache miss loader already consumed",
            Self::WeakCacheKeyDropped => "Weak cache key no longer has an owner",
            Self::StreamReaderStillOwned => "Async stream reader is still owned during release",
            Self::InvalidRandomSample => "Random sample is outside the unit interval",
            Self::SyncCallbackConsumed => "Owned synchronous callback already consumed",
            Self::TraceCountOverflow => "Tracer span count overflowed",
            Self::FutureFactoryConsumed => "Owned future factory already consumed",
            Self::ExternalIoFailure => "An external I/O operation failed",
            Self::InvalidUtf8 => "Input is not valid UTF-8",
        }
    }
}

/// Classification independent of a defect's human-readable diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DefectKind {
    /// Explicit diagnostic supplied by application or adapter code.
    Custom,
    /// A caught unwind, including a panic with no string payload.
    CallbackPanic,
    /// A specific library invariant or execution-ownership failure.
    Library(LibraryDefect),
}

#[derive(Clone)]
/// Variants of cause used by defect.
enum Cause {
    Custom(Arc<str>),
    CallbackPanic(Option<Arc<str>>),
    PreparedPanic(Arc<DiagnosticCell>),
    Library(LibraryDefect),
    LibraryDetail(LibraryDefect, Arc<str>),
}

impl PartialEq for Cause {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Custom(a), Self::Custom(b)) => a == b,
            (Self::CallbackPanic(a), Self::CallbackPanic(b)) => a == b,
            (Self::PreparedPanic(a), Self::PreparedPanic(b)) => {
                a.message() == b.message() && a.metadata() == b.metadata()
            }
            (Self::CallbackPanic(Some(a)), Self::PreparedPanic(b))
            | (Self::PreparedPanic(b), Self::CallbackPanic(Some(a))) => {
                !b.metadata().truncated && a.as_ref() == b.message()
            }
            (Self::Library(a), Self::Library(b)) => a == b,
            (Self::LibraryDetail(a, x), Self::LibraryDetail(b, y)) => a == b && x == y,
            _ => false,
        }
    }
}
impl Eq for Cause {}

/// An unexpected failure with a typed identity and a presentation diagnostic.
///
/// Equality includes the identity. A custom diagnostic cannot impersonate a
/// library failure or a panic by using the same message.
#[derive(Clone, PartialEq, Eq)]
pub struct Defect {
    cause: Cause,
}

impl Defect {
    /// Construct a typed library condition, including in a const context.
    #[must_use]
    /// # Parameters
    /// - `reason` (`LibraryDefect`): The reason supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn library(reason: LibraryDefect) -> Self {
        Self {
            cause: Cause::Library(reason),
        }
    }

    /// Construct an application- or adapter-supplied diagnostic.
    #[must_use]
    /// # Parameters
    /// - `message` (`impl Into<Arc<str>>`): Text or payload transferred to the selected consumer.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            cause: Cause::Custom(message.into()),
        }
    }

    #[must_use]
    /// Classify an I/O failure as a defect while retaining its diagnostic message.
    ///
    /// # Parameters
    /// - `error` (`std::io::Error`): The error to propagate or record.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_io(error: std::io::Error) -> Self {
        Self {
            cause: Cause::LibraryDetail(
                LibraryDefect::ExternalIoFailure,
                Arc::from(error.to_string()),
            ),
        }
    }
    #[must_use]
    /// Classify a UTF-8 conversion failure as a defect.
    ///
    /// # Parameters
    /// - `error` (`std::string::FromUtf8Error`): The error to propagate or record.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_utf8(error: std::string::FromUtf8Error) -> Self {
        Self {
            cause: Cause::LibraryDetail(LibraryDefect::InvalidUtf8, Arc::from(error.to_string())),
        }
    }

    #[must_use]
    /// # Returns
    /// Type: `DefectKind`.
    pub const fn kind(&self) -> DefectKind {
        match &self.cause {
            Cause::Custom(_) => DefectKind::Custom,
            Cause::CallbackPanic(_) | Cause::PreparedPanic(_) => DefectKind::CallbackPanic,
            Cause::Library(reason) | Cause::LibraryDetail(reason, _) => {
                DefectKind::Library(*reason)
            }
        }
    }

    /// Human-readable presentation only; inspect `kind()` for semantic identity.
    #[must_use]
    /// # Returns
    /// Type: `&str`.
    pub fn message(&self) -> &str {
        match &self.cause {
            Cause::Custom(message)
            | Cause::CallbackPanic(Some(message))
            | Cause::LibraryDetail(_, message) => message,
            Cause::CallbackPanic(None) => "User callback panicked with a non-string payload",
            Cause::PreparedPanic(cell) => cell.message(),
            Cause::Library(reason) => reason.message(),
        }
    }

    /// Whether a caller-selected prepared overflow policy lost message bytes.
    #[must_use]
    pub fn panic_message_truncated(&self) -> bool {
        match &self.cause {
            Cause::PreparedPanic(cell) => cell.metadata().truncated,
            Cause::Custom(_)
            | Cause::CallbackPanic(_)
            | Cause::Library(_)
            | Cause::LibraryDetail(_, _) => false,
        }
    }
    /// Original string payload size, when the panic payload contained a string.
    #[must_use]
    pub fn panic_original_length(&self) -> Option<usize> {
        match &self.cause {
            Cause::CallbackPanic(Some(message)) => Some(message.len()),
            Cause::PreparedPanic(cell) => Some(cell.metadata().original_length),
            Cause::Custom(_)
            | Cause::CallbackPanic(None)
            | Cause::Library(_)
            | Cause::LibraryDetail(_, _) => None,
        }
    }

    /// Translate a caught unwind into the panic defect channel.
    ///
    /// The payload is inspected only for a diagnostic. It never transports an
    /// Effect success or expected error, and does not select library behavior.
    #[must_use]
    /// # Parameters
    /// - `payload` (`Box<dyn Any + Send>`): Application data moved into the message or stored record.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_panic(payload: Box<dyn Any + Send>) -> Self {
        let message = if let Some(message) = payload.downcast_ref::<String>() {
            Some(Arc::<str>::from(message.as_str()))
        } else {
            payload
                .downcast_ref::<&str>()
                .map(|message| Arc::<str>::from(*message))
        };
        Self {
            cause: Cause::CallbackPanic(message),
        }
    }
}

impl From<LibraryDefect> for Defect {
    ///
    /// # Parameters
    /// - `reason` (`LibraryDefect`): The reason supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    fn from(reason: LibraryDefect) -> Self {
        Self::library(reason)
    }
}

impl std::fmt::Debug for Defect {
    ///
    /// # Parameters
    /// - `f` (`&mut std::fmt::Formatter<'_>`): The f supplied to this operation.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(Defect))
            .field(stringify!(kind), &self.kind())
            .field(stringify!(message), &self.message())
            .finish()
    }
}

/// Explicit caller-selected treatment for a panic message exceeding its buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanicMessageOverflow {
    /// Use a static summary and record the original byte length.
    Summary,
    /// Retain the largest valid UTF-8 prefix fitting the selected buffer.
    Utf8Prefix,
}
/// No capacity or overflow policy is selected implicitly.
#[derive(Clone, Copy, Debug)]
pub struct PreparedPanicConfig {
    pub slots: usize,
    pub bytes_per_message: crate::memory::Bytes,
    pub overflow: PanicMessageOverflow,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DiagnosticMetadata {
    truncated: bool,
    original_length: usize,
    summary: bool,
}
struct DiagnosticData {
    text: crate::memory::Text<'static>,
    metadata: DiagnosticMetadata,
}
struct DiagnosticCell {
    data: std::cell::UnsafeCell<DiagnosticData>,
}
// SAFETY: the private registry grants one affine writer only when its Arc is the
// sole strong owner. After publication every external owner sees immutable data;
// no weak handle or mutable cell reference is exposed, and reuse waits for all
// external strong owners to retire. Arc::get_mut supplies the uniqueness check
// and acquire synchronization with earlier owners before each new writer.
unsafe impl Sync for DiagnosticCell {}
impl DiagnosticCell {
    fn metadata(&self) -> DiagnosticMetadata {
        // SAFETY: only frozen cells are reachable from Defect; the retained Arc
        // prevents a new reservation/writer while this reference is live.
        unsafe { (*self.data.get()).metadata }
    }
    fn message(&self) -> &str {
        // SAFETY: same frozen-cell lifetime as metadata, tied to retained Arc.
        let data = unsafe { &*self.data.get() };
        if data.metadata.summary {
            "Panic message exceeded prepared diagnostic capacity"
        } else {
            data.text.as_str()
        }
    }
}
struct PanicRegistry {
    cells: crate::memory::FixedVector<'static, Arc<DiagnosticCell>>,
    overflow: PanicMessageOverflow,
}
/// One selected-region registry with separately allocated startup System Arc controls.
pub struct PreparedPanicPool {
    registry: crate::memory::SharedSlot<'static, std::sync::Mutex<PanicRegistry>>,
}
/// An exclusive pre-admitted writer; it cannot be cloned.
pub struct PreparedPanicReservation {
    cell: Arc<DiagnosticCell>,
    overflow: PanicMessageOverflow,
}
/// Prepared layout accounting; external System allocator overhead is not included.
#[derive(Clone, Copy, Debug)]
pub struct PreparedPanicDemand {
    pub region_payload: crate::memory::Bytes,
    pub startup_system_controls: usize,
    /// Pinned Rust 1.100 ArcInner requested-layout estimate, not a stable ABI.
    pub startup_system_requested_layout_estimate: crate::memory::Bytes,
}
// Pinned std 1.100 ArcInner is repr(C, align(2)) with two AtomicUsize headers.
#[repr(C, align(2))]
struct EstimatedArcCell {
    strong: std::sync::atomic::AtomicUsize,
    weak: std::sync::atomic::AtomicUsize,
    value: DiagnosticCell,
}
impl PreparedPanicPool {
    /// Calculate selected-region payload plus separately classified System controls.
    pub fn demand(
        config: PreparedPanicConfig,
    ) -> Result<PreparedPanicDemand, crate::memory::MemoryError> {
        use crate::memory::{Bytes, MemoryError, SlotPool};
        let registry = std::alloc::Layout::array::<Arc<DiagnosticCell>>(config.slots)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let region_payload = SlotPool::<std::sync::Mutex<PanicRegistry>>::storage_bytes(1)?
            .checked_add(Bytes::new(registry.size()))?
            .checked_add(config.bytes_per_message.checked_mul(config.slots)?)?;
        Ok(PreparedPanicDemand {
            region_payload,
            startup_system_controls: config.slots,
            startup_system_requested_layout_estimate: Bytes::new(std::mem::size_of::<
                EstimatedArcCell,
            >())
            .checked_mul(config.slots)?,
        })
    }
    /// Prepare all actual text buffers and one startup System Arc control per slot.
    pub fn prepare(
        memory: &crate::memory::PreparedMemory<'static>,
        region: crate::memory::RegionId,
        config: PreparedPanicConfig,
    ) -> Result<Self, crate::memory::MemoryError> {
        let _ = Self::demand(config)?;
        let mut cells =
            crate::memory::FixedVector::prepare(memory.allocator(region)?, config.slots)?;
        for _ in 0..config.slots {
            let text = memory.text(region, config.bytes_per_message)?;
            cells.push_reserved(Arc::new(DiagnosticCell {
                data: std::cell::UnsafeCell::new(DiagnosticData {
                    text,
                    metadata: DiagnosticMetadata {
                        truncated: false,
                        original_length: 0,
                        summary: false,
                    },
                }),
            }));
        }
        let mut registry = Some(PanicRegistry {
            cells,
            overflow: config.overflow,
        });
        let pool = memory.slots(region, 1, |_| {
            Ok(std::sync::Mutex::new(
                registry.take().ok_or(crate::memory::MemoryError::NoSlot)?,
            ))
        })?;
        Ok(Self {
            registry: pool.checkout()?.share(),
        })
    }
    /// Number of physically prepared diagnostic slots.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cells
            .capacity()
    }
    /// Fallibly retain this prepared registry without allocating a new control.
    pub fn try_clone(&self) -> Result<Self, crate::memory::MemoryError> {
        Ok(Self {
            registry: self.registry.try_clone()?,
        })
    }
    /// Claim a concrete slot before a callback that can panic is invoked.
    pub fn try_reserve(&self) -> Result<PreparedPanicReservation, crate::memory::MemoryError> {
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let overflow = registry.overflow;
        for cell in registry.cells.as_mut_slice() {
            if Arc::get_mut(cell).is_some() {
                return Ok(PreparedPanicReservation {
                    cell: Arc::clone(cell),
                    overflow,
                });
            }
        }
        Err(crate::memory::MemoryError::NoSlot)
    }

    /// Claim a fixed group of actual slots under one nonblocking registry gate.
    /// On success every entry is `Some`; a short group is retired after the
    /// gate is released. Contention is reported as `MemoryError::Busy`.
    pub fn try_reserve_group<const N: usize>(
        &self,
    ) -> Result<[Option<PreparedPanicReservation>; N], crate::memory::MemoryError> {
        use crate::memory::MemoryError;
        use std::sync::TryLockError;

        let mut reservations = std::array::from_fn(|_| None);
        let mut registry = match self.registry.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(MemoryError::Busy),
        };
        let overflow = registry.overflow;
        let mut count = 0;
        let mut vacant = reservations.iter_mut();
        for cell in registry.cells.as_mut_slice() {
            if Arc::get_mut(cell).is_some() {
                let Some(target) = vacant.next() else {
                    break;
                };
                *target = Some(PreparedPanicReservation {
                    cell: Arc::clone(cell),
                    overflow,
                });
                count += 1;
            }
        }
        drop(registry);
        if count == N {
            Ok(reservations)
        } else {
            Err(MemoryError::NoSlot)
        }
    }
}
impl PreparedPanicReservation {
    /// Freeze a caught panic into this admitted slot. The runtime's panic Box is
    /// externally allocated and is not charged as prepared diagnostic backing.
    #[must_use]
    pub fn capture(self, payload: Box<dyn Any + Send>) -> Defect {
        let defect = self.capture_borrowed(payload.as_ref());
        drop(payload);
        defect
    }
    /// Capture supported panic text without moving the provider-owned payload.
    /// The caller retains the original payload and its working-set owner.
    #[must_use]
    pub fn capture_borrowed(self, payload: &(dyn Any + Send)) -> Defect {
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied());
        let Some(message) = message else {
            return Defect {
                cause: Cause::CallbackPanic(None),
            };
        };
        // SAFETY: reservation is the only external Arc and is affine; registry
        // never reads or writes data while this reservation/its Defect survives.
        let data = unsafe { &mut *self.cell.data.get() };
        data.text.clear();
        let capacity = data.text.capacity().get();
        let truncated = message.len() > capacity;
        let summary = truncated && self.overflow == PanicMessageOverflow::Summary;
        let mut end = message.len().min(capacity);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        if !summary {
            // SAFETY: end is within the message and is a UTF-8 boundary.
            let prefix = unsafe { message.get_unchecked(..end) };
            // SAFETY: the prefix length is bounded by this Text's capacity.
            unsafe { data.text.replace_prevalidated(prefix) };
        }
        data.metadata = DiagnosticMetadata {
            truncated,
            original_length: message.len(),
            summary,
        };
        Defect {
            cause: Cause::PreparedPanic(self.cell),
        }
    }
}
