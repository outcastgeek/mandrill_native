use crate::{Exit, SharedValue};
use std::num::NonZeroU64;

/// Observable phase. A running refresh retains the last successful value;
/// a failed refresh retains it without conflating expected errors, defects,
/// interruption or finalization failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncResultPhase {
    Initial,
    Running,
    Settled,
}

pub enum AsyncResult<A: Send + 'static, E: Send + 'static> {
    Initial,
    Running {
        generation: NonZeroU64,
        previous_success: Option<SharedValue<A>>,
        refreshing: bool,
    },
    Settled {
        generation: NonZeroU64,
        outcome: Exit<SharedValue<A>, SharedValue<E>>,
        previous_success: Option<SharedValue<A>>,
    },
}

impl<A: Send + 'static, E: Send + 'static> Clone for AsyncResult<A, E> {
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
    pub const fn phase(&self) -> AsyncResultPhase {
        match self {
            Self::Initial => AsyncResultPhase::Initial,
            Self::Running { .. } => AsyncResultPhase::Running,
            Self::Settled { .. } => AsyncResultPhase::Settled,
        }
    }

    #[must_use]
    pub const fn generation(&self) -> Option<NonZeroU64> {
        match self {
            Self::Initial => None,
            Self::Running { generation, .. } | Self::Settled { generation, .. } => {
                Some(*generation)
            }
        }
    }

    #[must_use]
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
    pub fn outcome(&self) -> Option<&Exit<SharedValue<A>, SharedValue<E>>> {
        match self {
            Self::Settled { outcome, .. } => Some(outcome),
            Self::Initial | Self::Running { .. } => None,
        }
    }

    #[must_use]
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
