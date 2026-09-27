use crate::internal::destruction::FlatDrop;
use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::{
    Clock, Defect, Effect, Exit, HasClock, HasRandom, LibraryDefect, Random, UnitInterval,
};
use std::num::{NonZeroU8, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct Deterministic;
#[derive(Clone, Copy, Debug)]
pub struct Randomized;
pub trait ScheduleSampling<R>: Send + Sync + 'static {
    fn sample(env: &R) -> Result<Option<UnitInterval>, Defect>;
}
impl<R> ScheduleSampling<R> for Deterministic {
    fn sample(_: &R) -> Result<Option<UnitInterval>, Defect> {
        Ok(None)
    }
}
impl<R: HasRandom> ScheduleSampling<R> for Randomized {
    fn sample(env: &R) -> Result<Option<UnitInterval>, Defect> {
        env.random().sample().map(Some)
    }
}
pub trait CombineSampling<S> {
    type Combined: Send + Sync + 'static;
}
impl CombineSampling<Deterministic> for Deterministic {
    type Combined = Deterministic;
}
impl CombineSampling<Randomized> for Deterministic {
    type Combined = Randomized;
}
impl CombineSampling<Deterministic> for Randomized {
    type Combined = Randomized;
}
impl CombineSampling<Randomized> for Randomized {
    type Combined = Randomized;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrowthFactor(f64);
impl GrowthFactor {
    pub const fn new(value: f64) -> Result<Self, LibraryDefect> {
        if value.is_finite() && value >= NonZeroU8::MIN.get() as f64 {
            Ok(Self(value))
        } else {
            Err(LibraryDefect::InvalidSchedulePolicy)
        }
    }
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NonnegativeFactor(f64);
impl NonnegativeFactor {
    pub const fn new(value: f64) -> Result<Self, LibraryDefect> {
        if value.is_finite() && value >= u8::MIN as f64 {
            Ok(Self(value))
        } else {
            Err(LibraryDefect::InvalidSchedulePolicy)
        }
    }
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

pub enum ScheduleDecision<I: 'static, O: 'static, S: 'static = Deterministic> {
    Stop(O),
    Continue {
        output: O,
        delay: Duration,
        next: Schedule<I, O, S>,
    },
    Invalid(Defect),
}
type Policy<I, O, S> =
    Arc<dyn Fn(&I, Duration, Option<UnitInterval>) -> ScheduleDecision<I, O, S> + Send + Sync>;
type Transition<State, I, O> = Arc<
    dyn Fn(
            &State,
            &I,
            Duration,
            Option<UnitInterval>,
        ) -> Result<(O, Option<(Duration, State)>), Defect>
        + Send
        + Sync,
>;
pub struct Schedule<I: 'static, O: 'static, S: 'static = Deterministic> {
    policy: FlatDrop<Policy<I, O, S>>,
}
impl<I: 'static, O: 'static, S: 'static> Clone for Schedule<I, O, S> {
    fn clone(&self) -> Self {
        Self {
            policy: self.policy.clone(),
        }
    }
}
impl<I: 'static, O: 'static, S: Send + Sync + 'static> Schedule<I, O, S> {
    #[must_use]
    pub fn new(
        policy: impl Fn(&I, Duration, Option<UnitInterval>) -> ScheduleDecision<I, O, S>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            policy: FlatDrop::new(Arc::new(policy)),
        }
    }
    pub fn step(
        &self,
        input: &I,
        elapsed: Duration,
        sample: Option<UnitInterval>,
    ) -> ScheduleDecision<I, O, S> {
        match self.policy.as_ref() {
            Some(policy) => policy(input, elapsed, sample),
            None => ScheduleDecision::Invalid(Defect::from(LibraryDefect::MissingScheduleRecipe)),
        }
    }
    #[must_use]
    pub fn unfold<State: Send + Sync + 'static>(
        state: State,
        next: impl Fn(
            &State,
            &I,
            Duration,
            Option<UnitInterval>,
        ) -> Result<(O, Option<(Duration, State)>), Defect>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self::unfold_shared(state, Arc::new(next))
    }
    fn unfold_shared<State: Send + Sync + 'static>(
        state: State,
        next: Transition<State, I, O>,
    ) -> Self {
        Self::new(
            move |input, elapsed, sample| match next(&state, input, elapsed, sample) {
                Ok((output, None)) => ScheduleDecision::Stop(output),
                Ok((output, Some((delay, state)))) => ScheduleDecision::Continue {
                    output,
                    delay,
                    next: Self::unfold_shared(state, Arc::clone(&next)),
                },
                Err(defect) => ScheduleDecision::Invalid(defect),
            },
        )
    }
    #[must_use]
    pub fn up_to(self, limit: Duration) -> Self {
        Self::new(
            move |input, elapsed, sample| match self.step(input, elapsed, sample) {
                ScheduleDecision::Continue {
                    output,
                    delay,
                    next,
                } => match elapsed.checked_add(delay) {
                    Some(deadline) if deadline <= limit => ScheduleDecision::Continue {
                        output,
                        delay,
                        next: next.up_to(limit),
                    },
                    Some(_) => ScheduleDecision::Stop(output),
                    None => ScheduleDecision::Invalid(Defect::from(
                        LibraryDefect::ClockDeadlineOverflow,
                    )),
                },
                other @ (ScheduleDecision::Stop(_) | ScheduleDecision::Invalid(_)) => other,
            },
        )
    }
    pub fn jitter(
        self,
        minimum: NonnegativeFactor,
        maximum: NonnegativeFactor,
    ) -> Result<Schedule<I, O, Randomized>, LibraryDefect> {
        if minimum.get() > maximum.get() {
            return Err(LibraryDefect::InvalidSchedulePolicy);
        }
        Ok(self.jitter_validated(minimum, maximum))
    }
    fn jitter_validated(
        self,
        minimum: NonnegativeFactor,
        maximum: NonnegativeFactor,
    ) -> Schedule<I, O, Randomized> {
        Schedule::new(move |input, elapsed, sample| {
            let Some(sample) = sample else {
                return ScheduleDecision::Invalid(Defect::from(
                    LibraryDefect::InvalidSchedulePolicy,
                ));
            };
            match self.step(input, elapsed, Some(sample)) {
                ScheduleDecision::Continue {
                    output,
                    delay,
                    next,
                } => {
                    let factor = minimum.get() + sample.get() * (maximum.get() - minimum.get());
                    match Duration::try_from_secs_f64(delay.as_secs_f64() * factor) {
                        Ok(delay) => ScheduleDecision::Continue {
                            output,
                            delay,
                            next: next.jitter_validated(minimum, maximum),
                        },
                        Err(_) => ScheduleDecision::Invalid(Defect::from(
                            LibraryDefect::ClockDeadlineOverflow,
                        )),
                    }
                }
                ScheduleDecision::Stop(output) => ScheduleDecision::Stop(output),
                ScheduleDecision::Invalid(defect) => ScheduleDecision::Invalid(defect),
            }
        })
    }
    #[must_use]
    pub fn intersect<P: 'static, T: Send + Sync + 'static>(
        self,
        right: Schedule<I, P, T>,
    ) -> Schedule<I, (O, P), S::Combined>
    where
        S: CombineSampling<T>,
    {
        Schedule::new(move |input, elapsed, sample| {
            let left = self.step(input, elapsed, sample);
            let right = right.step(input, elapsed, sample);
            match (left, right) {
                (ScheduleDecision::Invalid(defect), _) | (_, ScheduleDecision::Invalid(defect)) => {
                    ScheduleDecision::Invalid(defect)
                }
                (ScheduleDecision::Stop(a), ScheduleDecision::Stop(b))
                | (ScheduleDecision::Stop(a), ScheduleDecision::Continue { output: b, .. })
                | (ScheduleDecision::Continue { output: a, .. }, ScheduleDecision::Stop(b)) => {
                    ScheduleDecision::Stop((a, b))
                }
                (
                    ScheduleDecision::Continue {
                        output: a,
                        delay: ad,
                        next: an,
                    },
                    ScheduleDecision::Continue {
                        output: b,
                        delay: bd,
                        next: bn,
                    },
                ) => ScheduleDecision::Continue {
                    output: (a, b),
                    delay: ad.max(bd),
                    next: an.intersect(bn),
                },
            }
        })
    }
}
impl<I: 'static> Schedule<I, usize> {
    #[must_use]
    pub fn recurs(count: usize) -> Self {
        Self::unfold(usize::default(), move |index, _, _, _| {
            let next = if *index < count {
                let index = index
                    .checked_add(NonZeroUsize::MIN.get())
                    .ok_or_else(|| Defect::from(LibraryDefect::ScheduleCountOverflow))?;
                Some((Duration::ZERO, index))
            } else {
                None
            };
            Ok((*index, next))
        })
    }
    #[must_use]
    pub fn spaced(delay: Duration) -> Self {
        Self::unfold(usize::default(), move |index, _, _, _| {
            let next = index
                .checked_add(NonZeroUsize::MIN.get())
                .ok_or_else(|| Defect::from(LibraryDefect::ScheduleCountOverflow))?;
            Ok((*index, Some((delay, next))))
        })
    }
    #[must_use]
    pub fn exponential(initial: Duration, factor: GrowthFactor) -> Self {
        Self::unfold(
            (usize::default(), initial),
            move |(index, delay), _, _, _| {
                let next_index = index
                    .checked_add(NonZeroUsize::MIN.get())
                    .ok_or_else(|| Defect::from(LibraryDefect::ScheduleCountOverflow))?;
                let next_delay = Duration::try_from_secs_f64(delay.as_secs_f64() * factor.get())
                    .map_err(|_| Defect::from(LibraryDefect::ClockDeadlineOverflow))?;
                Ok((*index, Some((*delay, (next_index, next_delay)))))
            },
        )
    }
}

trait Selection<A, E> {
    type Input;
    fn selected(outcome: &Exit<A, E>) -> Option<&Self::Input>;
}
struct Retry;
struct Repeat;
impl<A, E> Selection<A, E> for Retry {
    type Input = E;
    fn selected(outcome: &Exit<A, E>) -> Option<&E> {
        match outcome {
            Exit::Fail(error) => Some(error),
            Exit::Success(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => {
                None
            }
        }
    }
}
impl<A, E> Selection<A, E> for Repeat {
    type Input = A;
    fn selected(outcome: &Exit<A, E>) -> Option<&A> {
        match outcome {
            Exit::Success(value) => Some(value),
            Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => None,
        }
    }
}
fn attempt<A, E, R, O, S, T>(
    program: Effect<A, E, R>,
    schedule: Schedule<T::Input, O, S>,
    started: Duration,
    env: Arc<R>,
    context: Context,
    finish: Finish<A, E>,
) -> Step
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasClock + Send + Sync + 'static,
    O: Send + 'static,
    S: ScheduleSampling<R>,
    T: Selection<A, E> + Send + 'static,
    T::Input: 'static,
{
    let repeated = program.clone();
    program.execute(
        Arc::clone(&env),
        context.clone(),
        runtime::continuation(move |raw| {
            let previous = runtime::publish(raw, &context);
            let Some(input) = T::selected(&previous) else {
                return runtime::finish(finish, previous);
            };
            let decision = catch_unwind(AssertUnwindSafe(|| {
                let elapsed = env
                    .clock()
                    .now()
                    .checked_sub(started)
                    .ok_or_else(|| Defect::from(LibraryDefect::ClockDeadlineOverflow))?;
                let sample = S::sample(&env)?;
                Ok::<_, Defect>(schedule.step(input, elapsed, sample))
            }));
            let decision = match decision {
                Ok(Ok(decision)) => decision,
                Ok(Err(defect)) => return runtime::finish(finish, Exit::Die(defect)),
                Err(panic) => return runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            };
            match decision {
                ScheduleDecision::Stop(output) => {
                    match catch_unwind(AssertUnwindSafe(|| drop(output))) {
                        Ok(()) => runtime::finish(finish, previous),
                        Err(panic) => runtime::finish(
                            finish,
                            outcome::append_cleanup(previous, Exit::Die(Defect::from_panic(panic))),
                        ),
                    }
                }
                ScheduleDecision::Invalid(defect) => runtime::finish(finish, Exit::Die(defect)),
                ScheduleDecision::Continue {
                    output,
                    delay,
                    next,
                } => {
                    if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(output))) {
                        return runtime::finish(
                            finish,
                            outcome::append_cleanup(previous, Exit::Die(Defect::from_panic(panic))),
                        );
                    }
                    Effect::<(), E, R>::sleep(delay).execute(
                        Arc::clone(&env),
                        context.clone(),
                        runtime::continuation(move |slept| {
                            let discarded = catch_unwind(AssertUnwindSafe(|| drop(previous)));
                            match slept.into_failure() {
                                Ok(()) => match discarded {
                                    Ok(()) => attempt::<A, E, R, O, S, T>(
                                        repeated, next, started, env, context, finish,
                                    ),
                                    Err(panic) => runtime::finish(
                                        finish,
                                        Exit::Die(Defect::from_panic(panic)),
                                    ),
                                },
                                Err(failure) => runtime::finish(
                                    finish,
                                    match discarded {
                                        Ok(()) => failure,
                                        Err(panic) => outcome::append_cleanup(
                                            failure,
                                            Exit::Die(Defect::from_panic(panic)),
                                        ),
                                    },
                                ),
                            }
                        }),
                    )
                }
            }
        }),
    )
}
fn iterate<A, E, R, O, S, T>(
    program: Effect<A, E, R>,
    schedule: Schedule<T::Input, O, S>,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasClock + Send + Sync + 'static,
    O: Send + 'static,
    S: ScheduleSampling<R>,
    T: Selection<A, E> + Send + 'static,
    T::Input: 'static,
{
    Effect::new(move |env: Arc<R>, context, finish| {
        let started = match catch_unwind(AssertUnwindSafe(|| env.clock().now())) {
            Ok(started) => started,
            Err(panic) => return runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
        };
        attempt::<A, E, R, O, S, T>(
            program.clone(),
            schedule.clone(),
            started,
            env,
            context,
            finish,
        )
    })
}
impl<A: Send + 'static, E: Send + 'static, R: HasClock + Send + Sync + 'static> Effect<A, E, R> {
    pub fn retry<O: Send + 'static, S: ScheduleSampling<R>>(
        self,
        schedule: Schedule<E, O, S>,
    ) -> Self {
        iterate::<A, E, R, O, S, Retry>(self, schedule)
    }
    pub fn repeat<O: Send + 'static, S: ScheduleSampling<R>>(
        self,
        schedule: Schedule<A, O, S>,
    ) -> Self {
        iterate::<A, E, R, O, S, Repeat>(self, schedule)
    }
}
