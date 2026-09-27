use crate::{Defect, Effect, Exit, LibraryDefect};
use std::num::NonZeroU8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitInterval(f64);
impl UnitInterval {
    pub const MIN: f64 = u8::MIN as f64;
    pub const MAX: f64 = NonZeroU8::MIN.get() as f64;
    pub const fn new(value: f64) -> Result<Self, LibraryDefect> {
        if value.is_finite() && value >= Self::MIN && value <= Self::MAX {
            Ok(Self(value))
        } else {
            Err(LibraryDefect::InvalidRandomSample)
        }
    }
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}
pub trait Random: Send + Sync + 'static {
    fn sample(&self) -> Result<UnitInterval, Defect>;
}
pub trait HasRandom {
    type Random: Random + ?Sized;
    fn random(&self) -> &Self::Random;
}
pub struct RandomFn<F>(F);
impl<F> RandomFn<F> {
    #[must_use]
    pub const fn new(sample: F) -> Self {
        Self(sample)
    }
}
impl<F: Fn() -> Result<UnitInterval, Defect> + Send + Sync + 'static> Random for RandomFn<F> {
    fn sample(&self) -> Result<UnitInterval, Defect> {
        (self.0)()
    }
}
impl<R: HasRandom + Send + Sync + 'static, E: Send + 'static> Effect<UnitInterval, E, R> {
    pub fn random_sample() -> Self {
        Self::ready(|env, _| match env.random().sample() {
            Ok(sample) => Exit::Success(sample),
            Err(defect) => Exit::Die(defect),
        })
    }
}
