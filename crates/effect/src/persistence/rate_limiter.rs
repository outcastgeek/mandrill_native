//! Fixed-window, token-bucket, and adaptive limits.

use super::UtcMillis;
use crate::{Effect, KeyDomain, StableKey};
use crate::{effect, effect_map_error, effect_provide_some, effect_try_sync, service};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::time::Duration;

// Exact SI conversion; it is not a rate-limit policy value.
const NANOS_PER_MILLISECOND: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// An invalid rate-limit identity or policy detected before execution.
pub enum RateLimitDefinitionError {
    /// The requested period is below the supported scheduling precision.
    SubmillisecondPeriod,
    /// Computing period exceeded its supported range.
    PeriodOverflow,
    /// The adaptive bounds failed validation.
    InvalidAdaptiveBounds,
}

/// A caller-selected whole-millisecond interval. It is data, not a timer or
/// scheduler quantum. All comparisons use explicit UtcMillis arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimitPeriod(NonZeroU64);

impl RateLimitPeriod {
    #[must_use]
    /// # Parameters
    /// - `millis` (`NonZeroU64`): The millis supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn from_millis(millis: NonZeroU64) -> Self {
        Self(millis)
    }

    /// Construct a rate-limit interval from a validated duration.
    ///
    /// # Parameters
    /// - `duration` (`Duration`): The elapsed-time duration to use.
    ///
    /// # Returns
    /// Type: `Result<Self, RateLimitDefinitionError>`.
    pub fn from_duration(duration: Duration) -> Result<Self, RateLimitDefinitionError> {
        if !duration
            .subsec_nanos()
            .is_multiple_of(NANOS_PER_MILLISECOND)
        {
            return Err(RateLimitDefinitionError::SubmillisecondPeriod);
        }
        let millis = u64::try_from(duration.as_millis())
            .map_err(|_| RateLimitDefinitionError::PeriodOverflow)?;
        NonZeroU64::new(millis)
            .map(Self)
            .ok_or(RateLimitDefinitionError::PeriodOverflow)
    }

    #[must_use]
    /// # Returns
    /// Type: `NonZeroU64`.
    pub const fn millis(self) -> NonZeroU64 {
        self.0
    }
}

/// Feedback changes the next allowance inside explicitly selected bounds.
/// Feedback order is `(instant, ordinal)`; older or repeated reports are
/// ignored atomically. There is no hidden sampling or smoothing constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdaptiveWindowPolicy {
    period: RateLimitPeriod,
    minimum: NonZeroU64,
    maximum: NonZeroU64,
    initial: NonZeroU64,
    increase: NonZeroU64,
    decrease: NonZeroU64,
}

impl AdaptiveWindowPolicy {
    /// Construct a `AdaptiveWindowPolicy` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `period` (`RateLimitPeriod`): The period supplied to this operation.
    /// - `minimum` (`NonZeroU64`): Lower bound enforced by this policy.
    /// - `maximum` (`NonZeroU64`): Upper bound enforced by this policy.
    /// - `initial` (`NonZeroU64`): Initial value used before any state transition.
    /// - `increase` (`NonZeroU64`): The increase supplied to this operation.
    /// - `decrease` (`NonZeroU64`): The decrease supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, RateLimitDefinitionError>`.
    pub fn new(
        period: RateLimitPeriod,
        minimum: NonZeroU64,
        maximum: NonZeroU64,
        initial: NonZeroU64,
        increase: NonZeroU64,
        decrease: NonZeroU64,
    ) -> Result<Self, RateLimitDefinitionError> {
        if minimum > initial || initial > maximum {
            return Err(RateLimitDefinitionError::InvalidAdaptiveBounds);
        }
        Ok(Self {
            period,
            minimum,
            maximum,
            initial,
            increase,
            decrease,
        })
    }

    #[must_use]
    /// # Returns
    /// Type: `RateLimitPeriod`.
    pub const fn period(self) -> RateLimitPeriod {
        self.period
    }
    #[must_use]
    /// # Returns
    /// Type: `NonZeroU64`.
    pub const fn minimum(self) -> NonZeroU64 {
        self.minimum
    }
    #[must_use]
    /// # Returns
    /// Type: `NonZeroU64`.
    pub const fn maximum(self) -> NonZeroU64 {
        self.maximum
    }
    #[must_use]
    /// # Returns
    /// Type: `NonZeroU64`.
    pub const fn initial(self) -> NonZeroU64 {
        self.initial
    }
    #[must_use]
    /// # Returns
    /// Type: `NonZeroU64`.
    pub const fn increase(self) -> NonZeroU64 {
        self.increase
    }
    #[must_use]
    /// # Returns
    /// Type: `NonZeroU64`.
    pub const fn decrease(self) -> NonZeroU64 {
        self.decrease
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The capacity, refill, and feedback rules used to admit requests.
pub enum RateLimitPolicy {
    /// Windows are anchored to the Unix epoch. Negative instants use
    /// Euclidean division, so every instant belongs to exactly one window.
    FixedWindow {
        /// The maximum admitted capacity. Type: `NonZeroU64`.
        capacity: NonZeroU64,
        /// Interval between scheduled occurrences. Type: `RateLimitPeriod`.
        period: RateLimitPeriod,
    },
    /// Unused fractional refill is carried between calls until capacity is
    /// reached; surplus above capacity is discarded, never banked.
    TokenBucket {
        /// The maximum admitted capacity. Type: `NonZeroU64`.
        capacity: NonZeroU64,
        /// Units restored during one rate-limit refill period. Type: `NonZeroU64`.
        refill: NonZeroU64,
        /// Interval between scheduled occurrences. Type: `RateLimitPeriod`.
        period: RateLimitPeriod,
    },
    /// Adjust the rate-limit window from recorded feedback. Contains `AdaptiveWindowPolicy`.
    AdaptiveWindow(AdaptiveWindowPolicy),
}

impl RateLimitPolicy {
    #[must_use]
    /// # Returns
    /// Type: `NonZeroU64`.
    pub const fn maximum(self) -> NonZeroU64 {
        match self {
            Self::FixedWindow { capacity, .. } | Self::TokenBucket { capacity, .. } => capacity,
            Self::AdaptiveWindow(policy) => policy.maximum,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The permit or rejection returned after checking one request against a rate limit.
pub enum RateLimitDecision {
    /// The proposed operation passed its admission policy.
    Allowed {
        /// Capacity or time remaining after this step. Type: `u64`.
        remaining: u64,
    },
    /// The admission policy rejected this request.
    Denied {
        /// Capacity or time remaining after this step. Type: `u64`.
        remaining: u64,
        /// Time before a rejected request may be retried. Type: `Duration`.
        retry_after: Duration,
    },
    /// This request cannot fit under the declared maximum even after time or
    /// adaptive feedback changes. Retrying unchanged cannot make it valid.
    ImpossibleCost {
        /// Identity or quantity requested by the caller. Type: `NonZeroU64`.
        requested: NonZeroU64,
        /// Largest value permitted by this policy. Type: `NonZeroU64`.
        maximum: NonZeroU64,
    },
    /// Adaptive feedback must raise the current limit before this cost fits.
    AdaptationRequired {
        /// Identity or quantity requested by the caller. Type: `NonZeroU64`.
        requested: NonZeroU64,
        /// Current recorded value or position. Type: `NonZeroU64`.
        current: NonZeroU64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A recorded signal used to adjust an adaptive rate limit.
pub enum AdaptiveSignal {
    /// The provider is available for new work.
    Healthy,
    /// The provider cannot admit more work at its configured capacity.
    Overloaded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The feedback supplied after an admitted operation settles.
pub struct AdaptiveFeedback {
    /// Resolved absolute instant. Type: `UtcMillis`.
    pub instant: UtcMillis,
    /// Stable order of this recorded feedback event. Type: `NonZeroU64`.
    pub ordinal: NonZeroU64,
    /// Identity of the workflow signal. Type: `AdaptiveSignal`.
    pub signal: AdaptiveSignal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The limiter’s decision after incorporating adaptive feedback.
pub enum FeedbackOutcome {
    /// Adaptive feedback updated the stored rate state.
    Applied {
        /// Current feedback revision after the update. Type: `NonZeroU64`.
        current: NonZeroU64,
    },
    /// Adaptive feedback was older than the stored state.
    IgnoredStale {
        /// Current feedback revision that was retained. Type: `NonZeroU64`.
        current: NonZeroU64,
    },
}

/// Atomic per-key allowance and feedback. A shared provider may apply the
/// contract across processes; the core memory implementation is local only.
/// A policy change under the same key is an error, never an implicit reset.
pub trait RateLimitStore: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;

    /// # Parameters
    /// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
    /// - `policy` (`RateLimitPolicy`): The caller-supplied behavior policy.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    /// - `cost` (`NonZeroU64`): Capacity units charged to this request.
    ///
    /// # Returns
    /// Type: `Effect<RateLimitDecision, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn acquire(
        &self,
        key: Arc<[u8]>,
        policy: RateLimitPolicy,
        now: UtcMillis,
        cost: NonZeroU64,
    ) -> Effect<RateLimitDecision, Self::Error>;

    /// # Parameters
    /// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
    /// - `policy` (`AdaptiveWindowPolicy`): The caller-supplied behavior policy.
    /// - `feedback` (`AdaptiveFeedback`): The feedback supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<FeedbackOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn feedback(
        &self,
        key: Arc<[u8]>,
        policy: AdaptiveWindowPolicy,
        feedback: AdaptiveFeedback,
    ) -> Effect<FeedbackOutcome, Self::Error>;
}

/// Selects the atomic rate-limit store in an application environment.
pub trait HasRateLimitStore {
    /// Store selected by this environment; its trait bound defines the required guarantees.
    type Store: RateLimitStore;
    /// # Returns
    /// Type: `Arc<Self::Store>`.
    fn rate_limit_store(&self) -> Arc<Self::Store>;
}

#[derive(Debug)]
/// A typed policy or backing-store failure during rate-limit admission.
///
/// # Type parameters
/// - `Backend`: Backend-specific error or provider type.
pub enum RateLimitError<Backend> {
    /// The feedback is not supported.
    UnsupportedFeedback,
    /// The operation may have committed before acknowledgement was lost. Carries `Backend`.
    Uncertain(Backend),
}

/// A typed rule with no captured clock or provider. The same definition can
/// run with a local memory store or an unrelated conforming shared provider.
///
/// # Type parameters
/// - `D`: Identity-domain type. Bound or type: `KeyDomain`.
pub struct RateLimiter<D: KeyDomain> {
    policy: RateLimitPolicy,
    domain: std::marker::PhantomData<fn() -> D>,
}

impl<D: KeyDomain> Clone for RateLimiter<D> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            policy: self.policy,
            domain: std::marker::PhantomData,
        }
    }
}

impl<D: KeyDomain> RateLimiter<D> {
    #[must_use]
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    ///
    /// # Parameters
    /// - `policy` (`RateLimitPolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(policy: RateLimitPolicy) -> Self {
        Self {
            policy,
            domain: std::marker::PhantomData,
        }
    }

    #[must_use]
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    ///
    /// # Returns
    /// Type: `RateLimitPolicy`.
    pub const fn policy(&self) -> RateLimitPolicy {
        self.policy
    }

    /// Acquire a resource and register the matching release obligation.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasRateLimitStore + Send + Sync + 'static`.
    /// - `D`: Stable-key domain type.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    /// - `cost` (`NonZeroU64`): Capacity units charged to this request.
    ///
    /// # Returns
    /// Type: `Effect<RateLimitDecision, RateLimitError<<<R as HasRateLimitStore>::Store as RateLimitStore>::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn acquire<R>(
        &self,
        key: StableKey<D>,
        now: UtcMillis,
        cost: NonZeroU64,
    ) -> Effect<
        RateLimitDecision,
        RateLimitError<<<R as HasRateLimitStore>::Store as RateLimitStore>::Error>,
        R,
    >
    where
        R: HasRateLimitStore + Send + Sync + 'static,
    {
        let policy = self.policy;
        {
            let __dsl_program = service! { env: |env: &R| env.rate_limit_store(), };
            effect! { let key = key.clone(); let store = __dsl_program.clone()?; {
                effect_provide_some! { program: effect_map_error! { program: store
                    .acquire(key.clone().into_bytes(), policy, now, cost), map: RateLimitError::Uncertain, }, project: |_: &R| (), }
            }? }
        }
    }

    /// Apply observed operation feedback to an adaptive rate-limit decision.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasRateLimitStore + Send + Sync + 'static`.
    /// - `D`: Stable-key domain type.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    /// - `feedback` (`AdaptiveFeedback`): The feedback supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<FeedbackOutcome, RateLimitError<<<R as HasRateLimitStore>::Store as RateLimitStore>::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn feedback<R>(
        &self,
        key: StableKey<D>,
        feedback: AdaptiveFeedback,
    ) -> Effect<
        FeedbackOutcome,
        RateLimitError<<<R as HasRateLimitStore>::Store as RateLimitStore>::Error>,
        R,
    >
    where
        R: HasRateLimitStore + Send + Sync + 'static,
    {
        let RateLimitPolicy::AdaptiveWindow(policy) = self.policy else {
            return effect_try_sync! { body: { let __dsl_error = || RateLimitError::UnsupportedFeedback; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
        };
        {
            let __dsl_program = service! { env: |env: &R| env.rate_limit_store(), };
            effect! { let key = key.clone(); let store = __dsl_program.clone()?; {
                effect_provide_some! { program: effect_map_error! { program: store
                    .feedback(key.clone().into_bytes(), policy, feedback), map: RateLimitError::Uncertain, }, project: |_: &R| (), }
            }? }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit resource bounds for persistence rate limiter.
pub enum RateLimitStoreCapacity {
    /// No explicit capacity bound is imposed.
    Unbounded,
    /// The supplied positive capacity bound is enforced. Contains `NonZeroUsize`.
    Bounded(NonZeroUsize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A capacity or state failure in the process-local rate-limit store.
pub enum MemoryRateLimitError {
    /// The configured capacity bound was exceeded.
    CapacityExceeded,
    /// An incompatible policy is already recorded.
    PolicyConflict,
    /// The proposed clock moves backward from recorded state.
    ClockRegression,
    /// Computing arithmetic exceeded its supported range.
    ArithmeticOverflow,
}

impl std::fmt::Display for MemoryRateLimitError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for MemoryRateLimitError {}
