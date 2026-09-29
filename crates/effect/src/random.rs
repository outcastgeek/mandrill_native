//! Injectable randomness for Effect and schedule policies.

use crate::{Defect, Effect, Exit, LibraryDefect};
use std::num::NonZeroU8;

#[derive(Clone, Copy, Debug, PartialEq)]
/// A validated random sample between zero and one for schedule jitter.
pub struct UnitInterval(f64);
impl UnitInterval {
    /// Smallest value in the unit interval.
    pub const MIN: f64 = u8::MIN as f64;
    /// Largest value in the unit interval.
    pub const MAX: f64 = NonZeroU8::MIN.get() as f64;
    /// # Parameters
    /// - `value` (`f64`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, LibraryDefect>`.
    pub const fn new(value: f64) -> Result<Self, LibraryDefect> {
        if value.is_finite() && value >= Self::MIN && value <= Self::MAX {
            Ok(Self(value))
        } else {
            Err(LibraryDefect::InvalidRandomSample)
        }
    }
    #[must_use]
    /// # Returns
    /// Type: `f64`.
    pub const fn get(self) -> f64 {
        self.0
    }
}
/// Supplies caller-injected random values without choosing a global generator.
pub trait Random: Send + Sync + 'static {
    /// # Returns
    /// Type: `Result<UnitInterval, Defect>`.
    fn sample(&self) -> Result<UnitInterval, Defect>;
}
/// Selects the random-value provider in an application environment.
pub trait HasRandom {
    /// Random-value provider selected by this environment.
    type Random: Random + ?Sized;
    /// # Returns
    /// Type: `&Self::Random`.
    fn random(&self) -> &Self::Random;
}
/// A random-sample provider built from a caller-supplied callback.
///
/// # Type parameters
/// - `F`: Callback or field-definition type.
pub struct RandomFn<F>(F);
impl<F> RandomFn<F> {
    #[must_use]
    /// # Type parameters
    /// - `F`: Field or callback type.
    ///
    /// # Parameters
    /// - `sample` (`F`): The sample supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(sample: F) -> Self {
        Self(sample)
    }
}
impl<F: Fn() -> Result<UnitInterval, Defect> + Send + Sync + 'static> Random for RandomFn<F> {
    ///
    /// # Returns
    /// Type: `Result<UnitInterval, Defect>`.
    fn sample(&self) -> Result<UnitInterval, Defect> {
        (self.0)()
    }
}
impl<R: HasRandom + Send + Sync + 'static, E: Send + 'static> Effect<UnitInterval, E, R> {
    /// Request a validated random sample from the environment capability.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `HasRandom + Send + Sync + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn random_sample() -> Self {
        Self::ready(|env, _| match env.random().sample() {
            Ok(sample) => Exit::Success(sample),
            Err(defect) => Exit::Die(defect),
        })
    }
}
