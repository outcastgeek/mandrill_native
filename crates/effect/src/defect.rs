use std::any::Any;
use std::sync::Arc;

/// Typed identity of a library invariant or execution-ownership failure.
///
/// These conditions occupy the defect channel. Diagnostic wording is for
/// people; use this identity when inspecting or recording a library failure.
/// Serialized variant names are part of the stored-outcome version 1 protocol;
/// renaming one requires a deliberate format migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LibraryDefect {
    MissingEffectRecipe,
    MissingInterpreterOutcome,
    WorkflowConsumed,
    BindingConsumed,
    ResourceScopeEnded,
    ResourceReleasedTwice,
    ResourceBodyConsumed,
    ScopeFinalizerConsumed,
    ScopeRegistrationClosed,
    ScopeOutcomeConsumed,
    ScopeOwnerDropped,
    ScopeCloseDriverDropped,
    RunDriverDropped,
    RunOutcomeConsumed,
    ScheduledTaskDropped,
    TaskSubmissionRejected,
    FiberOutcomeConsumed,
    FiberOutcomeShared,
    FiberObservationOwnership,
    ParallelResultMissing,
    DeferredOutcomeConsumed,
    DeferredOutcomeShared,
    QueueOfferConsumed,
    ScheduledJobDropped,
    JobCallbackConsumed,
    JobSubmissionRejected,
    ClockDeadlineOverflow,
    ScheduleCountOverflow,
    InvalidSchedulePolicy,
    StreamCollectionLimitExceeded,
    MissingScheduleRecipe,
    ProviderClosed,
    ProviderCapacityFull,
    ProviderCountOverflow,
    ProviderWorkersInsufficient,
    NativeCompletionPanicked,
    PoolClosed,
    PoolCountOverflow,
    PoolLeaseEnded,
    PoolResourceDropped,
    PoolReleaseDriverDropped,
    PoolShutdownDriverDropped,
    CacheClosed,
    CacheShutdownDriverDropped,
    WeakCacheLoadConsumed,
    WeakCacheKeyDropped,
    StreamReaderStillOwned,
    StreamProducerSlotMissing,
    StreamBufferValueMissing,
    RequestResolverUnexpectedSuccess,
    InvalidRandomSample,
    SyncCallbackConsumed,
    TraceCountOverflow,
    FutureFactoryConsumed,
    ExternalIoFailure,
    InvalidUtf8,
}

impl LibraryDefect {
    // Each string presents the corresponding typed condition. No interpreter,
    // recovery, ownership, or provider decision uses this diagnostic text.
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
    pub const fn library(reason: LibraryDefect) -> Self {
        Self {
            cause: Cause::Library(reason),
        }
    }

    /// Construct an application- or adapter-supplied diagnostic.
    #[must_use]
    pub fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            cause: Cause::Custom(message.into()),
        }
    }

    #[must_use]
    pub fn from_io(error: std::io::Error) -> Self {
        Self {
            cause: Cause::LibraryDetail(
                LibraryDefect::ExternalIoFailure,
                Arc::from(error.to_string()),
            ),
        }
    }
    #[must_use]
    pub fn from_utf8(error: std::string::FromUtf8Error) -> Self {
        Self {
            cause: Cause::LibraryDetail(LibraryDefect::InvalidUtf8, Arc::from(error.to_string())),
        }
    }

    #[must_use]
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
    fn from(reason: LibraryDefect) -> Self {
        Self::library(reason)
    }
}

impl std::fmt::Debug for Defect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(Defect))
            .field(stringify!(kind), &self.kind())
            .field(stringify!(message), &self.message())
            .finish()
    }
}
