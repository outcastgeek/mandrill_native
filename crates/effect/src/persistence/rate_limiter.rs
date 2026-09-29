use super::UtcMillis;
use crate::{Effect, KeyDomain, StableKey};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::time::Duration;

// Exact SI conversion; it is not a rate-limit policy value.
const NANOS_PER_MILLISECOND: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitDefinitionError {
    SubmillisecondPeriod,
    PeriodOverflow,
    InvalidAdaptiveBounds,
}

/// A caller-selected whole-millisecond interval. It is data, not a timer or
/// scheduler quantum. All comparisons use explicit UtcMillis arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimitPeriod(NonZeroU64);

impl RateLimitPeriod {
    #[must_use]
    pub const fn from_millis(millis: NonZeroU64) -> Self {
        Self(millis)
    }

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
    pub const fn period(self) -> RateLimitPeriod {
        self.period
    }
    #[must_use]
    pub const fn minimum(self) -> NonZeroU64 {
        self.minimum
    }
    #[must_use]
    pub const fn maximum(self) -> NonZeroU64 {
        self.maximum
    }
    #[must_use]
    pub const fn initial(self) -> NonZeroU64 {
        self.initial
    }
    #[must_use]
    pub const fn increase(self) -> NonZeroU64 {
        self.increase
    }
    #[must_use]
    pub const fn decrease(self) -> NonZeroU64 {
        self.decrease
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitPolicy {
    /// Windows are anchored to the Unix epoch. Negative instants use
    /// Euclidean division, so every instant belongs to exactly one window.
    FixedWindow {
        capacity: NonZeroU64,
        period: RateLimitPeriod,
    },
    /// Unused fractional refill is carried between calls until capacity is
    /// reached; surplus above capacity is discarded, never banked.
    TokenBucket {
        capacity: NonZeroU64,
        refill: NonZeroU64,
        period: RateLimitPeriod,
    },
    AdaptiveWindow(AdaptiveWindowPolicy),
}

impl RateLimitPolicy {
    #[must_use]
    pub const fn maximum(self) -> NonZeroU64 {
        match self {
            Self::FixedWindow { capacity, .. } | Self::TokenBucket { capacity, .. } => capacity,
            Self::AdaptiveWindow(policy) => policy.maximum,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allowed {
        remaining: u64,
    },
    Denied {
        remaining: u64,
        retry_after: Duration,
    },
    /// This request cannot fit under the declared maximum even after time or
    /// adaptive feedback changes. Retrying unchanged cannot make it valid.
    ImpossibleCost {
        requested: NonZeroU64,
        maximum: NonZeroU64,
    },
    /// Adaptive feedback must raise the current limit before this cost fits.
    AdaptationRequired {
        requested: NonZeroU64,
        current: NonZeroU64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdaptiveSignal {
    Healthy,
    Overloaded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdaptiveFeedback {
    pub instant: UtcMillis,
    pub ordinal: NonZeroU64,
    pub signal: AdaptiveSignal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackOutcome {
    Applied { current: NonZeroU64 },
    IgnoredStale { current: NonZeroU64 },
}

/// Atomic per-key allowance and feedback. A shared provider may apply the
/// contract across processes; the core memory implementation is local only.
/// A policy change under the same key is an error, never an implicit reset.
pub trait RateLimitStore: Send + Sync + 'static {
    type Error: Send + 'static;

    fn acquire(
        &self,
        key: Arc<[u8]>,
        policy: RateLimitPolicy,
        now: UtcMillis,
        cost: NonZeroU64,
    ) -> Effect<RateLimitDecision, Self::Error>;

    fn feedback(
        &self,
        key: Arc<[u8]>,
        policy: AdaptiveWindowPolicy,
        feedback: AdaptiveFeedback,
    ) -> Effect<FeedbackOutcome, Self::Error>;
}

pub trait HasRateLimitStore {
    type Store: RateLimitStore;
    fn rate_limit_store(&self) -> Arc<Self::Store>;
}

#[derive(Debug)]
pub enum RateLimitError<Backend> {
    UnsupportedFeedback,
    Uncertain(Backend),
}

/// A typed rule with no captured clock or provider. The same definition can
/// run with a local memory store or an unrelated conforming shared provider.
pub struct RateLimiter<D: KeyDomain> {
    policy: RateLimitPolicy,
    domain: std::marker::PhantomData<fn() -> D>,
}

impl<D: KeyDomain> Clone for RateLimiter<D> {
    fn clone(&self) -> Self {
        Self {
            policy: self.policy,
            domain: std::marker::PhantomData,
        }
    }
}

impl<D: KeyDomain> RateLimiter<D> {
    #[must_use]
    pub const fn new(policy: RateLimitPolicy) -> Self {
        Self {
            policy,
            domain: std::marker::PhantomData,
        }
    }

    #[must_use]
    pub const fn policy(&self) -> RateLimitPolicy {
        self.policy
    }

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
        Effect::service(|env: &R| env.rate_limit_store()).flat_map(move |store| {
            store
                .acquire(key.clone().into_bytes(), policy, now, cost)
                .map_error(RateLimitError::Uncertain)
                .provide_some(|_: &R| ())
        })
    }

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
            return Effect::fail_with(|| RateLimitError::UnsupportedFeedback);
        };
        Effect::service(|env: &R| env.rate_limit_store()).flat_map(move |store| {
            store
                .feedback(key.clone().into_bytes(), policy, feedback)
                .map_error(RateLimitError::Uncertain)
                .provide_some(|_: &R| ())
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitStoreCapacity {
    Unbounded,
    Bounded(NonZeroUsize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryRateLimitError {
    CapacityExceeded,
    PolicyConflict,
    ClockRegression,
    ArithmeticOverflow,
}

impl std::fmt::Display for MemoryRateLimitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for MemoryRateLimitError {}
