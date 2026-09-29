use super::super::rate_limiter::{
    AdaptiveFeedback, AdaptiveSignal, AdaptiveWindowPolicy, FeedbackOutcome, HasRateLimitStore,
    MemoryRateLimitError, RateLimitDecision, RateLimitPolicy, RateLimitStore,
    RateLimitStoreCapacity,
};
use crate::{Effect, Layer, UtcMillis};
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

enum Counter {
    Fixed {
        window: i128,
        used: u64,
    },
    Token {
        tokens: u64,
        remainder: u128,
        last: UtcMillis,
    },
    Adaptive {
        window: i128,
        used: u64,
        current: NonZeroU64,
        last_feedback: Option<(UtcMillis, NonZeroU64)>,
    },
}

struct TokenBucketSpec {
    capacity: NonZeroU64,
    refill: NonZeroU64,
    period: NonZeroU64,
}

struct Entry {
    policy: RateLimitPolicy,
    counter: Counter,
}

impl Entry {
    fn new(policy: RateLimitPolicy, now: UtcMillis) -> Self {
        let counter = match policy {
            RateLimitPolicy::FixedWindow { period, .. } => Counter::Fixed {
                window: window_index(now, period.millis()),
                used: 0,
            },
            RateLimitPolicy::TokenBucket { capacity, .. } => Counter::Token {
                tokens: capacity.get(),
                remainder: 0,
                last: now,
            },
            RateLimitPolicy::AdaptiveWindow(policy) => Counter::Adaptive {
                window: window_index(now, policy.period().millis()),
                used: 0,
                current: policy.initial(),
                last_feedback: None,
            },
        };
        Self { policy, counter }
    }

    fn acquire(
        &mut self,
        now: UtcMillis,
        cost: NonZeroU64,
    ) -> Result<RateLimitDecision, MemoryRateLimitError> {
        match (self.policy, &mut self.counter) {
            (
                RateLimitPolicy::FixedWindow { capacity, period },
                Counter::Fixed { window, used },
            ) => window_allowance(window, used, capacity, period.millis(), now, cost),
            (
                RateLimitPolicy::AdaptiveWindow(policy),
                Counter::Adaptive {
                    window,
                    used,
                    current,
                    ..
                },
            ) => {
                if window_index(now, policy.period().millis()) < *window {
                    return Err(MemoryRateLimitError::ClockRegression);
                }
                if cost > *current {
                    return Ok(RateLimitDecision::AdaptationRequired {
                        requested: cost,
                        current: *current,
                    });
                }
                window_allowance(window, used, *current, policy.period().millis(), now, cost)
            }
            (
                RateLimitPolicy::TokenBucket {
                    capacity,
                    refill,
                    period,
                },
                Counter::Token {
                    tokens,
                    remainder,
                    last,
                },
            ) => token_allowance(
                tokens,
                remainder,
                last,
                TokenBucketSpec {
                    capacity,
                    refill,
                    period: period.millis(),
                },
                now,
                cost,
            ),
            _ => Err(MemoryRateLimitError::PolicyConflict),
        }
    }

    fn feedback(
        &mut self,
        report: AdaptiveFeedback,
    ) -> Result<FeedbackOutcome, MemoryRateLimitError> {
        let RateLimitPolicy::AdaptiveWindow(policy) = self.policy else {
            return Err(MemoryRateLimitError::PolicyConflict);
        };
        let Counter::Adaptive {
            current,
            last_feedback,
            ..
        } = &mut self.counter
        else {
            return Err(MemoryRateLimitError::PolicyConflict);
        };
        let order = (report.instant, report.ordinal);
        if last_feedback.is_some_and(|previous| order <= previous) {
            return Ok(FeedbackOutcome::IgnoredStale { current: *current });
        }
        let next = match report.signal {
            AdaptiveSignal::Healthy => {
                let room = policy.maximum().get() - current.get();
                current.get() + room.min(policy.increase().get())
            }
            AdaptiveSignal::Overloaded => current
                .get()
                .saturating_sub(policy.decrease().get())
                .max(policy.minimum().get()),
        };
        *current = NonZeroU64::new(next).ok_or(MemoryRateLimitError::ArithmeticOverflow)?;
        *last_feedback = Some(order);
        Ok(FeedbackOutcome::Applied { current: *current })
    }
}

fn window_index(now: UtcMillis, period: NonZeroU64) -> i128 {
    i128::from(now.value()).div_euclid(i128::from(period.get()))
}

fn window_retry(now: UtcMillis, period: NonZeroU64) -> Result<Duration, MemoryRateLimitError> {
    let length = i128::from(period.get());
    let next = window_index(now, period)
        .checked_add(1)
        .and_then(|window| window.checked_mul(length))
        .ok_or(MemoryRateLimitError::ArithmeticOverflow)?;
    let wait = next
        .checked_sub(i128::from(now.value()))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(MemoryRateLimitError::ArithmeticOverflow)?;
    Ok(Duration::from_millis(wait))
}

fn window_allowance(
    window: &mut i128,
    used: &mut u64,
    capacity: NonZeroU64,
    period: NonZeroU64,
    now: UtcMillis,
    cost: NonZeroU64,
) -> Result<RateLimitDecision, MemoryRateLimitError> {
    let observed = window_index(now, period);
    if observed < *window {
        return Err(MemoryRateLimitError::ClockRegression);
    }
    let current_used = if observed == *window { *used } else { 0 };
    let remaining = capacity.get().saturating_sub(current_used);
    let decision = if cost.get() <= remaining {
        RateLimitDecision::Allowed {
            remaining: remaining - cost.get(),
        }
    } else {
        RateLimitDecision::Denied {
            remaining,
            retry_after: window_retry(now, period)?,
        }
    };
    *window = observed;
    *used = if cost.get() <= remaining {
        current_used
            .checked_add(cost.get())
            .ok_or(MemoryRateLimitError::ArithmeticOverflow)?
    } else {
        current_used
    };
    Ok(decision)
}

fn token_allowance(
    tokens: &mut u64,
    remainder: &mut u128,
    last: &mut UtcMillis,
    spec: TokenBucketSpec,
    now: UtcMillis,
    cost: NonZeroU64,
) -> Result<RateLimitDecision, MemoryRateLimitError> {
    let TokenBucketSpec {
        capacity,
        refill,
        period,
    } = spec;
    let elapsed = i128::from(now.value()) - i128::from(last.value());
    if elapsed < 0 {
        return Err(MemoryRateLimitError::ClockRegression);
    }
    let elapsed = u128::try_from(elapsed).map_err(|_| MemoryRateLimitError::ArithmeticOverflow)?;
    let produced = elapsed
        .checked_mul(u128::from(refill.get()))
        .and_then(|value| value.checked_add(*remainder))
        .ok_or(MemoryRateLimitError::ArithmeticOverflow)?;
    let period_value = u128::from(period.get());
    let whole = produced / period_value;
    let room = capacity
        .get()
        .checked_sub(*tokens)
        .ok_or(MemoryRateLimitError::ArithmeticOverflow)?;
    let (available, next_remainder) = if whole >= u128::from(room) {
        (capacity.get(), 0)
    } else {
        let minted = u64::try_from(whole).map_err(|_| MemoryRateLimitError::ArithmeticOverflow)?;
        (
            tokens
                .checked_add(minted)
                .ok_or(MemoryRateLimitError::ArithmeticOverflow)?,
            produced % period_value,
        )
    };
    let decision = if available >= cost.get() {
        RateLimitDecision::Allowed {
            remaining: available - cost.get(),
        }
    } else {
        let deficit = u128::from(cost.get() - available);
        let needed = deficit
            .checked_mul(period_value)
            .and_then(|value| value.checked_sub(next_remainder))
            .ok_or(MemoryRateLimitError::ArithmeticOverflow)?;
        let delay = needed.div_ceil(u128::from(refill.get()));
        let millis = u64::try_from(delay).map_err(|_| MemoryRateLimitError::ArithmeticOverflow)?;
        RateLimitDecision::Denied {
            remaining: available,
            retry_after: Duration::from_millis(millis),
        }
    };
    *tokens = match decision {
        RateLimitDecision::Allowed { remaining } => remaining,
        RateLimitDecision::Denied { .. }
        | RateLimitDecision::ImpossibleCost { .. }
        | RateLimitDecision::AdaptationRequired { .. } => available,
    };
    *remainder = next_remainder;
    *last = now;
    Ok(decision)
}

struct State {
    entries: BTreeMap<Arc<[u8]>, Entry>,
    capacity: RateLimitStoreCapacity,
}

/// One process-local atomic store. It performs no background work and never
/// pretends that a local counter is a distributed rate limit.
pub struct MemoryRateLimitStore {
    state: Arc<Mutex<State>>,
}

impl MemoryRateLimitStore {
    #[must_use]
    pub fn new(capacity: RateLimitStoreCapacity) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                entries: BTreeMap::new(),
                capacity,
            })),
        }
    }

    #[must_use]
    pub fn layer(capacity: RateLimitStoreCapacity) -> Layer<(), MemoryRateLimitError, Arc<Self>> {
        Layer::make_with(move || Arc::new(Self::new(capacity)))
    }

    fn has_room(state: &State) -> bool {
        match state.capacity {
            RateLimitStoreCapacity::Unbounded => true,
            RateLimitStoreCapacity::Bounded(maximum) => state.entries.len() < maximum.get(),
        }
    }
}

impl HasRateLimitStore for Arc<MemoryRateLimitStore> {
    type Store = MemoryRateLimitStore;

    fn rate_limit_store(&self) -> Arc<Self::Store> {
        Arc::clone(self)
    }
}

impl RateLimitStore for MemoryRateLimitStore {
    type Error = MemoryRateLimitError;

    fn acquire(
        &self,
        key: Arc<[u8]>,
        policy: RateLimitPolicy,
        now: UtcMillis,
        cost: NonZeroU64,
    ) -> Effect<RateLimitDecision, Self::Error> {
        let state = Arc::clone(&self.state);
        Effect::try_sync_once(
            move || {
                if cost > policy.maximum() {
                    return Ok(RateLimitDecision::ImpossibleCost {
                        requested: cost,
                        maximum: policy.maximum(),
                    });
                }
                let mut state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(entry) = state.entries.get_mut(&key) {
                    if entry.policy != policy {
                        return Err(MemoryRateLimitError::PolicyConflict);
                    }
                    return entry.acquire(now, cost);
                }
                if !Self::has_room(&state) {
                    return Err(MemoryRateLimitError::CapacityExceeded);
                }
                let mut entry = Entry::new(policy, now);
                let decision = entry.acquire(now, cost)?;
                state.entries.insert(key, entry);
                Ok(decision)
            },
            std::convert::identity,
        )
    }

    fn feedback(
        &self,
        key: Arc<[u8]>,
        policy: AdaptiveWindowPolicy,
        report: AdaptiveFeedback,
    ) -> Effect<FeedbackOutcome, Self::Error> {
        let state = Arc::clone(&self.state);
        Effect::try_sync_once(
            move || {
                let mut state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let expected = RateLimitPolicy::AdaptiveWindow(policy);
                if let Some(entry) = state.entries.get_mut(&key) {
                    if entry.policy != expected {
                        return Err(MemoryRateLimitError::PolicyConflict);
                    }
                    return entry.feedback(report);
                }
                if !Self::has_room(&state) {
                    return Err(MemoryRateLimitError::CapacityExceeded);
                }
                let mut entry = Entry::new(expected, report.instant);
                let outcome = entry.feedback(report)?;
                state.entries.insert(key, entry);
                Ok(outcome)
            },
            std::convert::identity,
        )
    }
}
