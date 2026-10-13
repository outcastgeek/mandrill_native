//! Composable retry and repeat scheduling decisions.

use crate::internal::destruction::FlatDrop;
use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::schedule;
use crate::{
    Clock, Defect, Effect, Exit, HasClock, HasRandom, LibraryDefect, Random, UnitInterval,
};
use std::num::{NonZeroU8, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
/// Schedule sampling that adds no random jitter.
pub struct Deterministic;
#[derive(Clone, Copy, Debug)]
/// Schedule sampling that draws jitter from an environment random capability.
pub struct Randomized;
/// Supplies values used when a schedule requests randomized sampling.
pub trait ScheduleSampling<R>: Send + Sync + 'static {
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`&R`): The environment supplying this operation's capabilities.
    ///
    /// # Returns
    /// Type: `Result<Option<UnitInterval>, Defect>`.
    fn sample(env: &R) -> Result<Option<UnitInterval>, Defect>;
}
impl<R> ScheduleSampling<R> for Deterministic {
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Result<Option<UnitInterval>, Defect>`.
    fn sample(_: &R) -> Result<Option<UnitInterval>, Defect> {
        Ok(None)
    }
}
impl<R: HasRandom> ScheduleSampling<R> for Randomized {
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`&R`): The environment supplying this operation's capabilities.
    ///
    /// # Returns
    /// Type: `Result<Option<UnitInterval>, Defect>`.
    fn sample(env: &R) -> Result<Option<UnitInterval>, Defect> {
        env.random().sample().map(Some)
    }
}
/// Combines two schedule sampling requirements into one capability.
pub trait CombineSampling<S> {
    /// Combined sampling capability produced by this composition.
    type Combined: Send + Sync + 'static;
}
impl CombineSampling<Deterministic> for Deterministic {
    /// Type alias for combined in schedule.
    type Combined = Deterministic;
}
impl CombineSampling<Randomized> for Deterministic {
    /// Type alias for combined in schedule.
    type Combined = Randomized;
}
impl CombineSampling<Deterministic> for Randomized {
    /// Type alias for combined in schedule.
    type Combined = Randomized;
}
impl CombineSampling<Randomized> for Randomized {
    /// Type alias for combined in schedule.
    type Combined = Randomized;
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// A validated multiplier for exponential schedule delays.
pub struct GrowthFactor(f64);
impl GrowthFactor {
    /// # Parameters
    /// - `value` (`f64`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, LibraryDefect>`.
    pub const fn new(value: f64) -> Result<Self, LibraryDefect> {
        if value.is_finite() && value >= NonZeroU8::MIN.get() as f64 {
            Ok(Self(value))
        } else {
            Err(LibraryDefect::InvalidSchedulePolicy)
        }
    }
    #[must_use]
    /// # Returns
    /// Type: `f64`.
    pub const fn get(self) -> f64 {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
/// A validated nonnegative factor for schedule calculations.
pub struct NonnegativeFactor(f64);
impl NonnegativeFactor {
    /// # Parameters
    /// - `value` (`f64`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, LibraryDefect>`.
    pub const fn new(value: f64) -> Result<Self, LibraryDefect> {
        if value.is_finite() && value >= u8::MIN as f64 {
            Ok(Self(value))
        } else {
            Err(LibraryDefect::InvalidSchedulePolicy)
        }
    }
    #[must_use]
    /// # Returns
    /// Type: `f64`.
    pub const fn get(self) -> f64 {
        self.0
    }
}

/// A schedule step that either continues with its next delay or stops.
///
/// # Type parameters
/// - `I`: Input type. Bound or type: `'static`.
/// - `O`: Output type. Bound or type: `'static`.
/// - `S`: Service or storage type. Bound or type: `'static = Deterministic`.
pub enum ScheduleDecision<I: 'static, O: 'static, S: 'static = Deterministic> {
    /// End the schedule without producing another step. Contains `O`.
    Stop(O),
    /// Continue with the next schedule or workflow step.
    Continue {
        /// The output produced by this operation. Type: `O`.
        output: O,
        /// Time to wait before the next schedule decision. Type: `Duration`.
        delay: Duration,
        /// Next position after the current record. Type: `Schedule<I, O, S>`.
        next: Schedule<I, O, S>,
    },
    /// The  failed validation. Carries `Defect`.
    Invalid(Defect),
}
/// Type alias for policy in schedule.
///
/// # Type parameters
/// - `I`: Input type.
/// - `O`: Output type.
/// - `S`: Service or storage type.
type Policy<I, O, S> =
    Arc<dyn Fn(&I, Duration, Option<UnitInterval>) -> ScheduleDecision<I, O, S> + Send + Sync>;
/// Type alias for transition in schedule.
///
/// # Type parameters
/// - `State`: Stored state type.
/// - `I`: Input type.
/// - `O`: Output type.
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
/// A reusable policy for retries, repeats, and time-based recurrence.
///
/// # Type parameters
/// - `I`: Input type. Bound or type: `'static`.
/// - `O`: Output type. Bound or type: `'static`.
/// - `S`: Service or storage type. Bound or type: `'static = Deterministic`.
pub struct Schedule<I: 'static, O: 'static, S: 'static = Deterministic> {
    policy: FlatDrop<Policy<I, O, S>>,
}
impl<I: 'static, O: 'static, S: 'static> Clone for Schedule<I, O, S> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            policy: self.policy.clone(),
        }
    }
}
impl<I: 'static, O: 'static, S: Send + Sync + 'static> Schedule<I, O, S> {
    #[must_use]
    /// Construct a `Schedule` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `I`: Input type.
    /// - `O`: Output type.
    /// - `S`: Service or store type.
    ///
    /// # Parameters
    /// - `policy` (`impl Fn(&I, Duration, Option<UnitInterval>) -> ScheduleDecision<I, O, S> + Send + Sync + 'static`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Evaluate one scheduling decision from the current state and input.
    ///
    /// # Type parameters
    /// - `I`: Input type.
    /// - `O`: Output type.
    /// - `S`: Service or store type.
    ///
    /// # Parameters
    /// - `input` (`&I`): The input value or factory consumed by this operation.
    /// - `elapsed` (`Duration`): The elapsed supplied to this operation.
    /// - `sample` (`Option<UnitInterval>`): The sample supplied to this operation.
    ///
    /// # Returns
    /// Type: `ScheduleDecision<I, O, S>`.
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
    /// Build a schedule by repeatedly applying the supplied state transition.
    ///
    /// # Type parameters
    /// - `S`: Service or storage type. Bound: `Send + Sync + 'static`.
    /// - `State`: Bounds: `Send + Sync + 'static`.
    /// - `I`: Input type.
    /// - `O`: Output type.
    ///
    /// # Parameters
    /// - `state` (`State`): Current or replacement state owned by this operation.
    /// - `next` (`impl Fn( &State, &I, Duration, Option<UnitInterval>, ) -> Result<(O, Option<(Duration, State)>), Defect> + Send + Sync + 'static`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Self`.
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
    ///
    /// # Type parameters
    /// - `State`: Bounds: `Send + Sync + 'static`.
    /// - `I`: Input type.
    /// - `O`: Output type.
    ///
    /// # Parameters
    /// - `state` (`State`): Current or replacement state owned by this operation.
    /// - `next` (`Transition<State, I, O>`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Stop this schedule once its elapsed-time bound is reached.
    ///
    /// # Type parameters
    /// - `I`: Input type. Bound: `'static`.
    /// - `O`: Output type. Bound: `'static`.
    /// - `S`: Service or storage type. Bound: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `limit` (`Duration`): The caller-supplied limit.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Add environment-supplied random variation to the schedule delays.
    ///
    /// # Type parameters
    /// - `S`: Service or storage type. Bound: `Send + Sync + 'static`.
    /// - `I`: Input type.
    /// - `O`: Output type.
    ///
    /// # Parameters
    /// - `minimum` (`NonnegativeFactor`): Lower bound enforced by this policy.
    /// - `maximum` (`NonnegativeFactor`): Upper bound enforced by this policy.
    ///
    /// # Returns
    /// Type: `Result<Schedule<I, O, Randomized>, LibraryDefect>`.
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
    ///
    /// # Type parameters
    /// - `I`: Input type.
    /// - `O`: Output type.
    ///
    /// # Parameters
    /// - `minimum` (`NonnegativeFactor`): Lower bound enforced by this policy.
    /// - `maximum` (`NonnegativeFactor`): Upper bound enforced by this policy.
    ///
    /// # Returns
    /// Type: `Schedule<I, O, Randomized>`.
    /// Returns `Schedule<I, O, Randomized>`.
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
    /// Continue only while both schedules continue, combining their delays.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bounds: `'static`.
    /// - `T`: Value or item type. Bounds: `Send + Sync + 'static`.
    /// - `I`: Input type.
    /// - `O`: Output type.
    /// - `S`: Service or store type.
    ///
    /// # Parameters
    /// - `right` (`Schedule<I, P, T>`): Right-hand operation composed with this one.
    ///
    /// # Returns
    /// Type: `Schedule<I, (O, P), S::Combined>`.
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
    /// Continue for the caller-specified number of recurrences.
    ///
    /// # Type parameters
    /// - `I`: Input type. Bound: `'static`.
    ///
    /// # Parameters
    /// - `count` (`usize`): Caller-selected number of iterations, values, or permits.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn recurs(count: usize) -> Self {
        schedule! {
        state: usize::default(), step: move |index, _, _, _| {
                    let next = if *index < count {
                        let index = index
                            .checked_add(NonZeroUsize::MIN.get())
                            .ok_or_else(|| Defect::from(LibraryDefect::ScheduleCountOverflow))?;
                        Some((Duration::ZERO, index))
                    } else {
                        None
                    };
                    Ok((*index, next))
                },
        }
    }
    #[must_use]
    /// Continue with the caller-specified delay between decisions.
    ///
    /// # Type parameters
    /// - `I`: Input type. Bound: `'static`.
    ///
    /// # Parameters
    /// - `delay` (`Duration`): The delay supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn spaced(delay: Duration) -> Self {
        schedule! {
        state: usize::default(), step: move |index, _, _, _| {
                    let next = index
                        .checked_add(NonZeroUsize::MIN.get())
                        .ok_or_else(|| Defect::from(LibraryDefect::ScheduleCountOverflow))?;
                    Ok((*index, Some((delay, next))))
                },
        }
    }
    #[must_use]
    /// Grow each delay by a validated factor until the schedule stops.
    ///
    /// # Type parameters
    /// - `I`: Input type. Bound: `'static`.
    ///
    /// # Parameters
    /// - `initial` (`Duration`): Initial value used before any state transition.
    /// - `factor` (`GrowthFactor`): The factor supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn exponential(initial: Duration, factor: GrowthFactor) -> Self {
        schedule! {
        state: (usize::default(), initial), step: move |(index, delay), _, _, _| {
                        let next_index = index
                            .checked_add(NonZeroUsize::MIN.get())
                            .ok_or_else(|| Defect::from(LibraryDefect::ScheduleCountOverflow))?;
                        let next_delay = Duration::try_from_secs_f64(delay.as_secs_f64() * factor.get())
                            .map_err(|_| Defect::from(LibraryDefect::ClockDeadlineOverflow))?;
                        Ok((*index, Some((*delay, (next_index, next_delay)))))
                    },
        }
    }
}

/// Selects the failure for retry or the success for repetition from an exit.
trait Selection<A, E> {
    /// Type alias for input in schedule.
    type Input;
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`&Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Option<&Self::Input>`.
    /// Returns the value when present, or `None`.
    fn selected(outcome: &Exit<A, E>) -> Option<&Self::Input>;
}
/// Typed retry used by schedule.
struct Retry;
/// Typed repeat used by schedule.
struct Repeat;
impl<A, E> Selection<A, E> for Retry {
    /// Type alias for input in schedule.
    type Input = E;
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`&Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Option<&E>`.
    /// Returns the value when present, or `None`.
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
    /// Type alias for input in schedule.
    type Input = A;
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`&Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Option<&A>`.
    /// Returns the value when present, or `None`.
    fn selected(outcome: &Exit<A, E>) -> Option<&A> {
        match outcome {
            Exit::Success(value) => Some(value),
            Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => None,
        }
    }
}
/// Run one program/schedule step while retaining the previous full outcome; schedule failure is appended without dropping owned primary or cleanup failures.
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
                Ok(Err(defect)) => {
                    return runtime::finish(
                        finish,
                        outcome::append_cleanup(previous, Exit::Die(defect)),
                    );
                }
                Err(panic) => {
                    return runtime::finish(
                        finish,
                        outcome::append_cleanup(previous, Exit::Die(Defect::from_panic(panic))),
                    );
                }
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
                ScheduleDecision::Invalid(defect) => {
                    runtime::finish(finish, outcome::append_cleanup(previous, Exit::Die(defect)))
                }
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
///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `HasClock + Send + Sync + 'static`.
/// - `O`: Output or terminal value type. Additional bound: `Send + 'static`.
/// - `S`: State or store value type. Additional bound: `ScheduleSampling<R>`.
/// - `T`: Stored or transferred value type. Additional bound: `Selection<A, E> + Send + 'static`. Additional bound: `:Input: 'static`.
///
/// # Parameters
/// - `program` (`Effect<A, E, R>`): Lazy Effect description executed or transformed here.
/// - `schedule` (`Schedule<T::Input, O, S>`): The schedule supplied to this operation.
///
/// # Returns
/// Type: `Effect<A, E, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    /// Re-execute typed failures while the supplied schedule permits, using R's clock.
    ///
    /// A schedule failure retains the previous primary failure and ordered cleanup
    /// failures rather than replacing them. Interruption and defects keep their effect
    /// channels. The underlying program must be reusable or create fresh owned inputs;
    /// a schedule does not make a consumed one-shot input replayable.
    pub fn retry<O: Send + 'static, S: ScheduleSampling<R>>(
        self,
        schedule: Schedule<E, O, S>,
    ) -> Self {
        iterate::<A, E, R, O, S, Retry>(self, schedule)
    }
    /// Re-execute successful results while the schedule continues, using R's clock.
    ///
    /// Schedule evaluation and waiting remain cancellation-aware. Failure preserves
    /// existing primary/cleanup information; supply a reusable program because repeat
    /// does not clone consumed captures or choose an executor.
    pub fn repeat<O: Send + 'static, S: ScheduleSampling<R>>(
        self,
        schedule: Schedule<A, O, S>,
    ) -> Self {
        iterate::<A, E, R, O, S, Repeat>(self, schedule)
    }
}
