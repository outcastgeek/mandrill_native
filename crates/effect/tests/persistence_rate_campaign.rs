#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::persistence::{
    AdaptiveFeedback, AdaptiveSignal, AdaptiveWindowPolicy, FeedbackOutcome, MemoryRateLimitError,
    MemoryRateLimitStore, RateLimitDecision, RateLimitPeriod, RateLimitPolicy, RateLimitStore,
    RateLimitStoreCapacity,
};
use effect::{Cancellation, Effect, Exit, JsonValue, UtcMillis};
use futures_executor::block_on;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn success<A: Send + 'static, E: Send + 'static>(effect: Effect<A, E>) -> A {
    match block_on(effect.run(Arc::new(()))) {
        Exit::Success(value) => value,
        _ => panic!("memory rate-limit operation must succeed"),
    }
}

struct FixedModel {
    window: i64,
    used: u64,
}

impl FixedModel {
    fn acquire(&mut self, now: i64, cost: u64, capacity: u64, period: u64) -> RateLimitDecision {
        let maximum = NonZeroU64::new(capacity).expect("positive configured capacity");
        let requested = NonZeroU64::new(cost).expect("positive generated cost");
        if cost > capacity {
            return RateLimitDecision::ImpossibleCost { requested, maximum };
        }
        let period = i64::try_from(period).expect("configured period fits clock");
        let observed = now.div_euclid(period);
        if observed != self.window {
            self.window = observed;
            self.used = 0;
        }
        let remaining = capacity - self.used;
        if cost <= remaining {
            self.used += cost;
            RateLimitDecision::Allowed {
                remaining: remaining - cost,
            }
        } else {
            RateLimitDecision::Denied {
                remaining,
                retry_after: Duration::from_millis(
                    u64::try_from((observed + 1) * period - now).expect("future window boundary"),
                ),
            }
        }
    }
}

// Rational credit is deliberately distinct from the implementation's token
// count plus remainder representation: one token is `period` credit units.
struct TokenModel {
    credit: u128,
    last: i64,
}

impl TokenModel {
    fn acquire(
        &mut self,
        now: i64,
        cost: u64,
        capacity: u64,
        refill: u64,
        period: u64,
    ) -> RateLimitDecision {
        let maximum = NonZeroU64::new(capacity).expect("positive configured capacity");
        let requested = NonZeroU64::new(cost).expect("positive generated cost");
        if cost > capacity {
            return RateLimitDecision::ImpossibleCost { requested, maximum };
        }
        let period = u128::from(period);
        let elapsed = u128::try_from(now - self.last).expect("monotone model clock");
        self.credit =
            (self.credit + elapsed * u128::from(refill)).min(u128::from(capacity) * period);
        self.last = now;
        let available = self.credit / period;
        if u128::from(cost) <= available {
            self.credit -= u128::from(cost) * period;
            RateLimitDecision::Allowed {
                remaining: u64::try_from(available - u128::from(cost))
                    .expect("bounded available credit"),
            }
        } else {
            let deficit = u128::from(cost) * period - self.credit;
            let delay = deficit.div_ceil(u128::from(refill));
            RateLimitDecision::Denied {
                remaining: u64::try_from(available).expect("bounded available credit"),
                retry_after: Duration::from_millis(
                    u64::try_from(delay).expect("bounded model delay"),
                ),
            }
        }
    }
}

struct AdaptiveModel {
    window: i64,
    used: u64,
    current: u64,
    last_feedback: Option<(i64, u64)>,
}

impl AdaptiveModel {
    fn acquire(&mut self, now: i64, cost: u64, maximum: u64, period: u64) -> RateLimitDecision {
        let requested = NonZeroU64::new(cost).expect("positive generated cost");
        if cost > maximum {
            return RateLimitDecision::ImpossibleCost {
                requested,
                maximum: NonZeroU64::new(maximum).expect("positive adaptive maximum"),
            };
        }
        if cost > self.current {
            return RateLimitDecision::AdaptationRequired {
                requested,
                current: NonZeroU64::new(self.current).expect("positive current limit"),
            };
        }
        let period = i64::try_from(period).expect("configured period fits clock");
        let observed = now.div_euclid(period);
        if observed != self.window {
            self.window = observed;
            self.used = 0;
        }
        let remaining = self.current.saturating_sub(self.used);
        if cost <= remaining {
            self.used += cost;
            RateLimitDecision::Allowed {
                remaining: remaining - cost,
            }
        } else {
            RateLimitDecision::Denied {
                remaining,
                retry_after: Duration::from_millis(
                    u64::try_from((observed + 1) * period - now).expect("future adaptive boundary"),
                ),
            }
        }
    }

    fn feedback(
        &mut self,
        report: AdaptiveFeedback,
        minimum: u64,
        maximum: u64,
        increase: u64,
        decrease: u64,
    ) -> FeedbackOutcome {
        let order = (report.instant.value(), report.ordinal.get());
        if self.last_feedback.is_some_and(|previous| order <= previous) {
            return FeedbackOutcome::IgnoredStale {
                current: NonZeroU64::new(self.current).expect("positive current limit"),
            };
        }
        self.current = match report.signal {
            AdaptiveSignal::Healthy => (self.current + increase).min(maximum),
            AdaptiveSignal::Overloaded => self.current.saturating_sub(decrease).max(minimum),
        };
        self.last_feedback = Some(order);
        FeedbackOutcome::Applied {
            current: NonZeroU64::new(self.current).expect("positive current limit"),
        }
    }
}

#[test]
fn configured_rate_histories_match_independent_models() {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/feature_campaigns.json")
        });
    let config: JsonValue = std::fs::read_to_string(configured)
        .expect("campaign configuration is readable")
        .parse()
        .expect("campaign configuration is JSON");
    let number = |path: &[&str]| -> u64 {
        path.iter()
            .fold(&config, |value, field| {
                value
                    .get(*field)
                    .unwrap_or_else(|| panic!("missing campaign field {field}"))
            })
            .as_u64()
            .expect("campaign field is a nonnegative integer")
    };
    let seeds = number(&["campaigns", "persistence", "seeds"]);
    let transitions = number(&["campaigns", "persistence", "transitions"]);
    let rate = |field| number(&["campaigns", "persistence", "rate_limit", field]);
    let capacity = rate("capacity");
    let refill = rate("refill");
    let period = rate("period_millis");
    let minimum = rate("adaptive_minimum");
    let initial = rate("adaptive_initial");
    let maximum = rate("adaptive_maximum");
    let increase = rate("adaptive_increase");
    let decrease = rate("adaptive_decrease");
    let simultaneous = usize::try_from(rate("simultaneous_callers"))
        .expect("configured native caller count fits usize");
    let seconds = number(&["budgets", "seconds"]);
    assert!(seeds >= 1_000 && transitions >= 100 && seconds > 0);
    assert!(capacity > 0 && refill > 0 && period > 0);
    assert!(minimum > 0 && minimum <= initial && initial <= maximum);
    assert!(simultaneous > usize::try_from(capacity).expect("capacity fits usize"));
    let positive = |value| NonZeroU64::new(value).expect("positive campaign policy");
    let interval = RateLimitPeriod::from_millis(positive(period));
    let fixed = RateLimitPolicy::FixedWindow {
        capacity: positive(capacity),
        period: interval,
    };
    let token = RateLimitPolicy::TokenBucket {
        capacity: positive(capacity),
        refill: positive(refill),
        period: interval,
    };
    let adaptive = AdaptiveWindowPolicy::new(
        interval,
        positive(minimum),
        positive(maximum),
        positive(initial),
        positive(increase),
        positive(decrease),
    )
    .expect("configured adaptive bounds");
    let invalid = AdaptiveWindowPolicy::new(
        interval,
        positive(maximum),
        positive(minimum),
        positive(initial),
        positive(increase),
        positive(decrease),
    );
    assert!(invalid.is_err());
    let started = Instant::now();
    let mut decisions = [0_u64; 4];
    let mut feedbacks = [0_u64; 2];
    for seed in 0..seeds {
        let store = MemoryRateLimitStore::new(RateLimitStoreCapacity::Bounded(
            NonZeroUsize::new(3).expect("three policy model keys"),
        ));
        let fixed_key: Arc<[u8]> = Arc::from(&b"fixed"[..]);
        let token_key: Arc<[u8]> = Arc::from(&b"token"[..]);
        let adaptive_key: Arc<[u8]> = Arc::from(&b"adaptive"[..]);
        let first = positive(1);
        assert_eq!(
            success(store.acquire(Arc::clone(&fixed_key), fixed, UtcMillis::new(0), first)),
            RateLimitDecision::Allowed {
                remaining: capacity - 1
            }
        );
        assert_eq!(
            success(store.acquire(Arc::clone(&token_key), token, UtcMillis::new(0), first)),
            RateLimitDecision::Allowed {
                remaining: capacity - 1
            }
        );
        assert_eq!(
            success(store.acquire(
                Arc::clone(&adaptive_key),
                RateLimitPolicy::AdaptiveWindow(adaptive),
                UtcMillis::new(0),
                first
            )),
            RateLimitDecision::Allowed {
                remaining: initial - 1
            }
        );
        let mut fixed_model = FixedModel { window: 0, used: 1 };
        let mut token_model = TokenModel {
            credit: u128::from(capacity - 1) * u128::from(period),
            last: 0,
        };
        let mut adaptive_model = AdaptiveModel {
            window: 0,
            used: 1,
            current: initial,
            last_feedback: None,
        };
        let mut generator = number(&["seed"]).wrapping_add(seed).max(1);
        let mut now = 0_i64;
        for transition in 0..transitions {
            now += i64::try_from(next(&mut generator) % (period + 2))
                .expect("bounded generated clock step");
            let fixed_cost = 1 + next(&mut generator) % (capacity + 1);
            let fixed_actual = success(store.acquire(
                Arc::clone(&fixed_key),
                fixed,
                UtcMillis::new(now),
                positive(fixed_cost),
            ));
            assert_eq!(
                fixed_actual,
                fixed_model.acquire(now, fixed_cost, capacity, period)
            );
            decisions[decision_index(fixed_actual)] += 1;

            let token_cost = 1 + next(&mut generator) % (capacity + 1);
            let token_actual = success(store.acquire(
                Arc::clone(&token_key),
                token,
                UtcMillis::new(now),
                positive(token_cost),
            ));
            assert_eq!(
                token_actual,
                token_model.acquire(now, token_cost, capacity, refill, period)
            );
            decisions[decision_index(token_actual)] += 1;

            if next(&mut generator) & 1 == 0 {
                let adaptive_cost = 1 + next(&mut generator) % (maximum + 1);
                let actual = success(store.acquire(
                    Arc::clone(&adaptive_key),
                    RateLimitPolicy::AdaptiveWindow(adaptive),
                    UtcMillis::new(now),
                    positive(adaptive_cost),
                ));
                assert_eq!(
                    actual,
                    adaptive_model.acquire(now, adaptive_cost, maximum, period)
                );
                decisions[decision_index(actual)] += 1;
            } else {
                let delayed = next(&mut generator) % (period + 1);
                let report = AdaptiveFeedback {
                    instant: UtcMillis::new(
                        now - i64::try_from(delayed).expect("bounded feedback delay"),
                    ),
                    ordinal: positive(transition + 1),
                    signal: if next(&mut generator) & 1 == 0 {
                        AdaptiveSignal::Healthy
                    } else {
                        AdaptiveSignal::Overloaded
                    },
                };
                let actual = success(store.feedback(Arc::clone(&adaptive_key), adaptive, report));
                let expected =
                    adaptive_model.feedback(report, minimum, maximum, increase, decrease);
                assert_eq!(actual, expected);
                feedbacks[usize::from(matches!(actual, FeedbackOutcome::IgnoredStale { .. }))] += 1;
            }
            assert!(
                started.elapsed().as_secs() <= seconds,
                "rate campaign timed out"
            );
        }
        assert!(matches!(
            block_on(
                store
                    .acquire(Arc::from(&b"fourth"[..]), fixed, UtcMillis::new(now), first,)
                    .run(Arc::new(()))
            ),
            Exit::Fail(MemoryRateLimitError::CapacityExceeded)
        ));
        let incompatible = RateLimitPolicy::FixedWindow {
            capacity: positive(capacity + 1),
            period: interval,
        };
        assert!(matches!(
            block_on(
                store
                    .acquire(fixed_key, incompatible, UtcMillis::new(now), first,)
                    .run(Arc::new(()))
            ),
            Exit::Fail(MemoryRateLimitError::PolicyConflict)
        ));
    }
    let cancelled_store = MemoryRateLimitStore::new(RateLimitStoreCapacity::Unbounded);
    let cancellation = Arc::new(Cancellation::new());
    assert!(cancellation.request());
    let cancellation_key: Arc<[u8]> = Arc::from(&b"precancel"[..]);
    let pre_cancelled = cancelled_store
        .acquire(
            Arc::clone(&cancellation_key),
            fixed,
            UtcMillis::new(0),
            positive(1),
        )
        .provide_some(|_: &Cancellation| ());
    assert_eq!(
        block_on(pre_cancelled.run_cancellable(cancellation)),
        Exit::Interrupted
    );
    assert_eq!(
        success(cancelled_store.acquire(cancellation_key, fixed, UtcMillis::new(0), positive(1),)),
        RateLimitDecision::Allowed {
            remaining: capacity - 1
        }
    );

    let contended = Arc::new(MemoryRateLimitStore::new(RateLimitStoreCapacity::Unbounded));
    let barrier = Arc::new(Barrier::new(simultaneous));
    let hot_key: Arc<[u8]> = Arc::from(&b"hot-window"[..]);
    let outcomes = std::thread::scope(|scope| {
        let mut threads = Vec::with_capacity(simultaneous);
        for _ in 0..simultaneous {
            let store = Arc::clone(&contended);
            let barrier = Arc::clone(&barrier);
            let key = Arc::clone(&hot_key);
            threads.push(scope.spawn(move || {
                barrier.wait();
                success(store.acquire(key, fixed, UtcMillis::new(0), positive(1)))
            }));
        }
        threads
            .into_iter()
            .map(|thread| thread.join().expect("native caller joined"))
            .collect::<Vec<_>>()
    });
    let mut allowed_remaining = Vec::new();
    let mut denied = 0_usize;
    for outcome in outcomes {
        match outcome {
            RateLimitDecision::Allowed { remaining } => allowed_remaining.push(remaining),
            RateLimitDecision::Denied {
                remaining,
                retry_after,
            } => {
                assert_eq!(remaining, 0);
                assert_eq!(retry_after, Duration::from_millis(period));
                denied += 1;
            }
            _ => panic!("unit cost under a fixed policy has only allowed/denied outcomes"),
        }
    }
    allowed_remaining.sort_unstable();
    assert_eq!(allowed_remaining, (0..capacity).collect::<Vec<_>>());
    assert_eq!(
        denied,
        simultaneous - usize::try_from(capacity).expect("capacity fits usize")
    );
    assert!(decisions.into_iter().all(|count| count > 0));
    assert!(feedbacks.into_iter().all(|count| count > 0));
    campaign_report::completed(
        "persistence_rate",
        serde_json::json!({
            "seeds": seeds, "transitions": transitions,
            "decisions": decisions, "feedbacks": feedbacks,
        }),
    );
    eprintln!(
        "rate model completed {seeds} seeds x {transitions} transitions; decisions={decisions:?}; feedbacks={feedbacks:?}"
    );
}

fn decision_index(decision: RateLimitDecision) -> usize {
    match decision {
        RateLimitDecision::Allowed { .. } => 0,
        RateLimitDecision::Denied { .. } => 1,
        RateLimitDecision::ImpossibleCost { .. } => 2,
        RateLimitDecision::AdaptationRequired { .. } => 3,
    }
}
