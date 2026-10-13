//! Typed wall-clock instants, deadlines, and elapsed durations.

use crate::effect_sync;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::internal::runtime;
use crate::{Defect, Effect, Exit};

// SI unit conversion, not a scheduling quantum or configurable policy.
const NANOS_PER_MILLISECOND: u32 = 1_000_000;

/// Signed Unix epoch milliseconds, independent of a process-local Instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct UtcMillis(i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A typed failure converting or calculating an instant or duration.
pub enum TimeError {
    /// The supplied value lies outside the supported range.
    OutOfRange,
    /// The value cannot be represented at the required time precision.
    SubmillisecondPrecision,
}

impl std::fmt::Display for TimeError {
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

impl std::error::Error for TimeError {}

impl UtcMillis {
    #[must_use]
    /// # Parameters
    /// - `value` (`i64`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    #[must_use]
    /// # Returns
    /// Type: `i64`.
    pub const fn value(self) -> i64 {
        self.0
    }

    /// Convert a native wall-clock value without silently losing precision.
    ///
    /// # Parameters
    /// - `value` (`SystemTime`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, TimeError>`.
    pub fn from_system_time(value: SystemTime) -> Result<Self, TimeError> {
        let (before_epoch, duration) = match value.duration_since(UNIX_EPOCH) {
            Ok(duration) => (false, duration),
            Err(error) => (true, error.duration()),
        };
        if duration.subsec_nanos() % NANOS_PER_MILLISECOND != 0 {
            return Err(TimeError::SubmillisecondPrecision);
        }
        let magnitude = i128::try_from(duration.as_millis()).map_err(|_| TimeError::OutOfRange)?;
        let signed = if before_epoch { -magnitude } else { magnitude };
        i64::try_from(signed)
            .map(Self)
            .map_err(|_| TimeError::OutOfRange)
    }

    /// Convert the checked UTC instant to the host system-time representation.
    ///
    /// # Returns
    /// Type: `Result<SystemTime, TimeError>`.
    pub fn to_system_time(self) -> Result<SystemTime, TimeError> {
        let magnitude = Duration::from_millis(self.0.unsigned_abs());
        if self.0.is_negative() {
            UNIX_EPOCH.checked_sub(magnitude)
        } else {
            UNIX_EPOCH.checked_add(magnitude)
        }
        .ok_or(TimeError::OutOfRange)
    }

    /// Add a duration or offset, returning an error if the range is exceeded.
    ///
    /// # Parameters
    /// - `duration` (`Duration`): The elapsed-time duration to use.
    ///
    /// # Returns
    /// Type: `Result<Self, TimeError>`.
    pub fn checked_add(self, duration: Duration) -> Result<Self, TimeError> {
        if !duration
            .subsec_nanos()
            .is_multiple_of(NANOS_PER_MILLISECOND)
        {
            return Err(TimeError::SubmillisecondPrecision);
        }
        let delta = i128::try_from(duration.as_millis()).map_err(|_| TimeError::OutOfRange)?;
        i128::from(self.0)
            .checked_add(delta)
            .and_then(|value| i64::try_from(value).ok())
            .map(Self)
            .ok_or(TimeError::OutOfRange)
    }

    /// Subtract a duration or offset, returning an error if the range is exceeded.
    ///
    /// # Parameters
    /// - `duration` (`Duration`): The elapsed-time duration to use.
    ///
    /// # Returns
    /// Type: `Result<Self, TimeError>`.
    pub fn checked_sub(self, duration: Duration) -> Result<Self, TimeError> {
        if !duration
            .subsec_nanos()
            .is_multiple_of(NANOS_PER_MILLISECOND)
        {
            return Err(TimeError::SubmillisecondPrecision);
        }
        let delta = i128::try_from(duration.as_millis()).map_err(|_| TimeError::OutOfRange)?;
        i128::from(self.0)
            .checked_sub(delta)
            .and_then(|value| i64::try_from(value).ok())
            .map(Self)
            .ok_or(TimeError::OutOfRange)
    }

    /// A stable signed epoch representation; byte order is explicit so a
    /// persisted timestamp never depends on the host machine's endianness.
    #[must_use]
    /// # Returns
    /// Type: `[u8; std::mem::size_of::<i64>()]`.
    pub const fn to_be_bytes(self) -> [u8; std::mem::size_of::<i64>()] {
        self.0.to_be_bytes()
    }

    #[must_use]
    /// # Parameters
    /// - `bytes` (`[u8; std::mem::size_of::<i64>()]`): Encoded bytes to validate, decode, write, or transfer.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn from_be_bytes(bytes: [u8; std::mem::size_of::<i64>()]) -> Self {
        Self(i64::from_be_bytes(bytes))
    }
}

/// Persistable wall-clock deadline. A process-local monotonic ClockTick is a
/// different type and cannot be substituted or serialized as this value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Deadline(UtcMillis);

impl Deadline {
    #[must_use]
    /// # Parameters
    /// - `instant` (`UtcMillis`): The instant supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn at(instant: UtcMillis) -> Self {
        Self(instant)
    }

    /// Construct a deadline relative to the current clock reading.
    ///
    /// # Parameters
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    /// - `duration` (`Duration`): The elapsed-time duration to use.
    ///
    /// # Returns
    /// Type: `Result<Self, TimeError>`.
    pub fn after(now: UtcMillis, duration: Duration) -> Result<Self, TimeError> {
        now.checked_add(duration).map(Self)
    }

    #[must_use]
    /// # Returns
    /// Type: `UtcMillis`.
    pub const fn instant(self) -> UtcMillis {
        self.0
    }

    #[must_use]
    /// Check whether this deadline has passed at the supplied instant.
    ///
    /// # Parameters
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub fn has_elapsed(self, now: UtcMillis) -> bool {
        now >= self.0
    }

    /// Calculate the nonnegative time left before this deadline.
    ///
    /// # Parameters
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Result<Option<Duration>, TimeError>`.
    pub fn remaining(self, now: UtcMillis) -> Result<Option<Duration>, TimeError> {
        if self.has_elapsed(now) {
            return Ok(None);
        }
        let millis = i128::from(self.0.value()) - i128::from(now.value());
        let millis = u64::try_from(millis).map_err(|_| TimeError::OutOfRange)?;
        Ok(Some(Duration::from_millis(millis)))
    }
}

/// Portable wall-clock time. A monotonic ClockTick is intentionally not a
/// durable timestamp and must not be persisted as a deadline.
pub trait WallClock: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;
    /// # Returns
    /// Type: `crate::Effect<UtcMillis, Self::Error>`.
    fn now(&self) -> crate::Effect<UtcMillis, Self::Error>;
}

/// Selects a wall-clock provider for recoverable deadlines.
pub trait HasWallClock {
    /// Clock selected by this environment.
    type Clock: WallClock + ?Sized;
    /// # Returns
    /// Type: `&Self::Clock`.
    fn wall_clock(&self) -> &Self::Clock;
}

/// A controllable, shareable wall-clock capability. Advancing it never starts
/// a timer or runtime; callers supply it through their environment.
#[derive(Clone)]
pub struct TestWallClock {
    millis: Arc<AtomicI64>,
}

impl TestWallClock {
    #[must_use]
    /// Construct a `TestWallClock` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `initial` (`UtcMillis`): Initial value used before any state transition.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(initial: UtcMillis) -> Self {
        Self {
            millis: Arc::new(AtomicI64::new(initial.value())),
        }
    }

    #[must_use]
    /// Return the absolute instant represented by this deadline.
    ///
    /// # Returns
    /// Type: `UtcMillis`.
    pub fn instant(&self) -> UtcMillis {
        UtcMillis::new(self.millis.load(Ordering::Acquire))
    }

    /// Replace the current value through the declared state capability.
    ///
    /// # Parameters
    /// - `instant` (`UtcMillis`): The instant supplied to this operation.
    ///
    /// # Returns
    /// Type: `()`.
    pub fn set(&self, instant: UtcMillis) {
        self.millis.store(instant.value(), Ordering::Release);
    }

    /// Move the recorded position forward after validating the intervening state.
    ///
    /// # Parameters
    /// - `duration` (`Duration`): The elapsed-time duration to use.
    ///
    /// # Returns
    /// Type: `Result<UtcMillis, TimeError>`.
    pub fn advance(&self, duration: Duration) -> Result<UtcMillis, TimeError> {
        loop {
            let current = self.millis.load(Ordering::Acquire);
            let next = UtcMillis::new(current).checked_add(duration)?;
            if self
                .millis
                .compare_exchange(current, next.value(), Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(next);
            }
        }
    }
}

impl WallClock for TestWallClock {
    /// Type alias for error in duration.
    type Error = std::convert::Infallible;

    ///
    /// # Returns
    /// Type: `Effect<UtcMillis, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn now(&self) -> Effect<UtcMillis, Self::Error> {
        let millis = Arc::clone(&self.millis);
        effect_sync! { body: move || UtcMillis::new(millis.load(Ordering::Acquire)), }
    }
}

impl HasWallClock for TestWallClock {
    /// Type alias for clock in duration.
    type Clock = Self;

    ///
    /// # Returns
    /// Type: `&Self::Clock`.
    fn wall_clock(&self) -> &Self::Clock {
        self
    }
}

impl<E, R> Effect<UtcMillis, E, R>
where
    E: Send + 'static,
    R: HasWallClock + Send + Sync + 'static,
    R::Clock: WallClock<Error = E>,
{
    /// Read the application-supplied wall clock through R. The concrete clock
    /// and its scheduling/runtime dependencies are constructed by a Layer.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn wall_now() -> Self {
        Self::new(move |env, context, finish| {
            match catch_unwind(AssertUnwindSafe(|| env.wall_clock().now())) {
                Ok(program) => program.execute(Arc::new(()), context, finish),
                Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            }
        })
    }
}
