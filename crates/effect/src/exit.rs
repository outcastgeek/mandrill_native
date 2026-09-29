//! Success, failure, interruption, and finalization outcomes.

use crate::Defect;

/// The original outcome retained when cleanup also fails.
#[derive(Debug, Clone, PartialEq, Eq)]
///
/// # Type parameters
/// - `E`: Domain-error type.
pub enum PrimaryOutcome<E> {
    /// The primary operation completed successfully before cleanup.
    Succeeded,
    /// The primary operation failed with an expected application error.
    Failed(E),
    /// The primary operation ended in a defect outside the expected-error channel.
    Defected(Defect),
    /// The primary operation was interrupted before cleanup.
    WasInterrupted,
}

/// Borrowed view used by release callbacks without cloning the primary error.
///
/// # Type parameters
/// - `'outcome`: Lifetime parameter.
/// - `E`: Domain-error type.
pub enum PrimaryOutcomeRef<'outcome, E> {
    /// The primary operation succeeded; no error is borrowed.
    Succeeded,
    /// Borrows the expected application error without cloning it.
    Failed(&'outcome E),
    /// Borrows the defect without cloning it.
    Defected(&'outcome Defect),
    /// The primary operation was interrupted; no error is borrowed.
    WasInterrupted,
}

/// One failure of an owned resource's release operation.
#[derive(Debug, Clone, PartialEq, Eq)]
///
/// # Type parameters
/// - `E`: Domain-error type.
pub enum CleanupFailure<E> {
    /// Resource release failed with an expected application error.
    ReleaseFailure(E),
    /// Resource release ended in a defect.
    ReleaseDefect(Defect),
    /// Resource release was interrupted.
    ReleaseInterrupted,
}

/// Ordered cleanup failures; `first` makes the collection nonempty.
#[derive(Debug, Clone, PartialEq, Eq)]
///
/// # Type parameters
/// - `E`: Domain-error type.
pub struct Finalization<E> {
    /// Original body outcome, retained even when cleanup fails. Type: `PrimaryOutcome<E>`.
    pub primary: PrimaryOutcome<E>,
    /// First cleanup failure; makes this collection nonempty. Type: `CleanupFailure<E>`.
    pub first: CleanupFailure<E>,
    /// Later cleanup failures in observation order. Type: `Vec<CleanupFailure<E>>`.
    pub rest: Vec<CleanupFailure<E>>,
}

/// Complete terminal outcome. Recovery of expected errors leaves other channels intact.
#[derive(Debug, Clone, PartialEq, Eq)]
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub enum Exit<A, E> {
    /// The operation succeeded. Carries `A`.
    Success(A),
    /// Expected application failure; `catch_all` can recover this channel.
    Fail(E),
    /// Defect outside the expected `E` channel, such as a caught panic.
    Die(Defect),
    /// Execution was interrupted.
    Interrupted,
    /// Cleanup failed; retains both the original outcome and cleanup failures.
    FinalizationFailed(Finalization<E>),
}

impl<A, E> Exit<A, E> {
    #[must_use]
    /// Borrow the original outcome, including when cleanup also failed.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `PrimaryOutcomeRef<'_, E>`.
    pub fn primary(&self) -> PrimaryOutcomeRef<'_, E> {
        match self {
            Self::Success(_) => PrimaryOutcomeRef::Succeeded,
            Self::Fail(error) => PrimaryOutcomeRef::Failed(error),
            Self::Die(defect) => PrimaryOutcomeRef::Defected(defect),
            Self::Interrupted => PrimaryOutcomeRef::WasInterrupted,
            Self::FinalizationFailed(details) => match &details.primary {
                PrimaryOutcome::Succeeded => PrimaryOutcomeRef::Succeeded,
                PrimaryOutcome::Failed(error) => PrimaryOutcomeRef::Failed(error),
                PrimaryOutcome::Defected(defect) => PrimaryOutcomeRef::Defected(defect),
                PrimaryOutcome::WasInterrupted => PrimaryOutcomeRef::WasInterrupted,
            },
        }
    }

    #[must_use]
    /// Map every expected error in the exit, including retained cleanup errors.
    ///
    /// # Type parameters
    /// - `E2`: Transformed expected-error type.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `transform` (`impl FnMut(E) -> E2`): Callback applied to the selected value or error.
    ///
    /// # Returns
    /// Type: `Exit<A, E2>`.
    pub fn map_error<E2>(self, mut transform: impl FnMut(E) -> E2) -> Exit<A, E2> {
        ///
        /// # Type parameters
        /// - `E`: Expected application-error type.
        /// - `E2`: Transformed expected-error type.
        ///
        /// # Parameters
        /// - `failure` (`CleanupFailure<E>`): Expected error or cleanup failure retained in this outcome.
        /// - `transform` (`&mut impl FnMut(E) -> E2`): Callback applied to the selected value or error.
        ///
        /// # Returns
        /// Type: `CleanupFailure<E2>`.
        /// Returns `CleanupFailure<E2>`.
        fn cleanup<E, E2>(
            failure: CleanupFailure<E>,
            transform: &mut impl FnMut(E) -> E2,
        ) -> CleanupFailure<E2> {
            match failure {
                CleanupFailure::ReleaseFailure(error) => {
                    CleanupFailure::ReleaseFailure(transform(error))
                }
                CleanupFailure::ReleaseDefect(defect) => CleanupFailure::ReleaseDefect(defect),
                CleanupFailure::ReleaseInterrupted => CleanupFailure::ReleaseInterrupted,
            }
        }
        match self {
            Self::Success(value) => Exit::Success(value),
            Self::Fail(error) => Exit::Fail(transform(error)),
            Self::Die(defect) => Exit::Die(defect),
            Self::Interrupted => Exit::Interrupted,
            Self::FinalizationFailed(details) => {
                let primary = match details.primary {
                    PrimaryOutcome::Succeeded => PrimaryOutcome::Succeeded,
                    PrimaryOutcome::Failed(error) => PrimaryOutcome::Failed(transform(error)),
                    PrimaryOutcome::Defected(defect) => PrimaryOutcome::Defected(defect),
                    PrimaryOutcome::WasInterrupted => PrimaryOutcome::WasInterrupted,
                };
                Exit::FinalizationFailed(Finalization {
                    primary,
                    first: cleanup(details.first, &mut transform),
                    rest: details
                        .rest
                        .into_iter()
                        .map(|failure| cleanup(failure, &mut transform))
                        .collect(),
                })
            }
        }
    }

    /// # Type parameters
    /// - `B`: Transformed value type.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Result<A, Exit<B, E>>`.
    pub(crate) fn into_failure<B>(self) -> Result<A, Exit<B, E>> {
        match self {
            Self::Success(value) => Ok(value),
            Self::Fail(error) => Err(Exit::Fail(error)),
            Self::Die(defect) => Err(Exit::Die(defect)),
            Self::Interrupted => Err(Exit::Interrupted),
            Self::FinalizationFailed(details) => Err(Exit::FinalizationFailed(details)),
        }
    }
}
