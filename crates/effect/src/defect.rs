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

#[derive(Clone, PartialEq, Eq)]
/// Variants of cause used by defect.
enum Cause {
    Custom(Arc<str>),
    CallbackPanic(Option<Arc<str>>),
    Library(LibraryDefect),
    LibraryDetail(LibraryDefect, Arc<str>),
}

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
            Cause::CallbackPanic(_) => DefectKind::CallbackPanic,
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
            Cause::Library(reason) => reason.message(),
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
