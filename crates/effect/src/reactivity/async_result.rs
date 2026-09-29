//! Observable initial, pending, success, and failure states.

use crate::{Exit, SharedValue};
use std::num::NonZeroU64;

/// Observable phase. A running refresh retains the last successful value;
/// a failed refresh retains it without conflating expected errors, defects,
/// interruption or finalization failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncResultPhase {
    /// This is the first recorded state of the operation.
    Initial,
    /// The operation has started and is not yet terminal.
    Running,
    /// The owned work has reached a terminal state.
    Settled,
}

/// The pending, successful, or failed state of an asynchronous reactive computation.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub enum AsyncResult<A: Send + 'static, E: Send + 'static> {
    /// This is the first recorded state of the operation.
    Initial,
    /// The operation has started and is not yet terminal.
    Running {
        /// The ownership generation used for fencing. Type: `NonZeroU64`.
        generation: NonZeroU64,
        /// Last successful value retained during refresh. Type: `Option<SharedValue<A>>`.
        previous_success: Option<SharedValue<A>>,
        /// Whether a newer evaluation is in progress. Type: `bool`.
        refreshing: bool,
    },
    /// The owned work has reached a terminal state.
    Settled {
        /// The ownership generation used for fencing. Type: `NonZeroU64`.
        generation: NonZeroU64,
        /// The recorded outcome of this operation. Type: `Exit<SharedValue<A>, SharedValue<E>>`.
        outcome: Exit<SharedValue<A>, SharedValue<E>>,
        /// Last successful value retained during refresh. Type: `Option<SharedValue<A>>`.
        previous_success: Option<SharedValue<A>>,
    },
}

impl<A: Send + 'static, E: Send + 'static> Clone for AsyncResult<A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        match self {
            Self::Initial => Self::Initial,
            Self::Running {
                generation,
                previous_success,
                refreshing,
            } => Self::Running {
                generation: *generation,
                previous_success: previous_success.clone(),
                refreshing: *refreshing,
            },
            Self::Settled {
                generation,
                outcome,
                previous_success,
            } => Self::Settled {
                generation: *generation,
                outcome: outcome.clone(),
                previous_success: previous_success.clone(),
            },
        }
    }
}

impl<A: Send + 'static, E: Send + 'static> AsyncResult<A, E> {
    #[must_use]
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `AsyncResultPhase`.
    pub const fn phase(&self) -> AsyncResultPhase {
        match self {
            Self::Initial => AsyncResultPhase::Initial,
            Self::Running { .. } => AsyncResultPhase::Running,
            Self::Settled { .. } => AsyncResultPhase::Settled,
        }
    }

    #[must_use]
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `Option<NonZeroU64>`.
    /// Returns the value when present, or `None`.
    pub const fn generation(&self) -> Option<NonZeroU64> {
        match self {
            Self::Initial => None,
            Self::Running { generation, .. } | Self::Settled { generation, .. } => {
                Some(*generation)
            }
        }
    }

    #[must_use]
    /// Borrow the last successful value retained during a later refresh.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Option<&SharedValue<A>>`.
    /// Returns the value when present, or `None`.
    pub fn previous_success(&self) -> Option<&SharedValue<A>> {
        match self {
            Self::Initial => None,
            Self::Running {
                previous_success, ..
            }
            | Self::Settled {
                previous_success, ..
            } => previous_success.as_ref(),
        }
    }

    #[must_use]
    /// Borrow the current successful value if evaluation has completed.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Option<&SharedValue<A>>`.
    /// Returns the value when present, or `None`.
    pub fn current_success(&self) -> Option<&SharedValue<A>> {
        match self {
            Self::Settled {
                outcome: Exit::Success(value),
                ..
            } => Some(value),
            Self::Initial => None,
            Self::Running {
                previous_success, ..
            }
            | Self::Settled {
                previous_success, ..
            } => previous_success.as_ref(),
        }
    }

    #[must_use]
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub const fn refreshing(&self) -> bool {
        matches!(
            self,
            Self::Running {
                refreshing: true,
                ..
            }
        )
    }

    #[must_use]
    /// Borrow the complete current asynchronous result state.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Option<&Exit<SharedValue<A>, SharedValue<E>>>`.
    /// Returns the value when present, or `None`.
    pub fn outcome(&self) -> Option<&Exit<SharedValue<A>, SharedValue<E>>> {
        match self {
            Self::Settled { outcome, .. } => Some(outcome),
            Self::Initial | Self::Running { .. } => None,
        }
    }

    #[must_use]
    /// Apply the selected terminal action to a fenced claim.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `generation` (`NonZeroU64`): The generation supplied to this operation.
    /// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    /// - `previous_success` (`Option<SharedValue<A>>`): The previous success supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn settle(
        generation: NonZeroU64,
        outcome: Exit<A, E>,
        previous_success: Option<SharedValue<A>>,
    ) -> Self {
        let outcome = match outcome.map_error(SharedValue::new) {
            Exit::Success(value) => Exit::Success(SharedValue::new(value)),
            Exit::Fail(error) => Exit::Fail(error),
            Exit::Die(defect) => Exit::Die(defect),
            Exit::Interrupted => Exit::Interrupted,
            Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
        };
        Self::Settled {
            generation,
            outcome,
            previous_success,
        }
    }
}
