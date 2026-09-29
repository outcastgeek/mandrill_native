//! Source Outcome.fs semantics without cloning owned application errors.
use crate::{CleanupFailure, Exit, Finalization, PrimaryOutcome};

/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
///
/// # Parameters
/// - `outcome` (`&Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
///
/// # Returns
/// Type: `bool`.
/// Returns whether the requested condition holds.
pub(crate) fn interrupted<A, E>(outcome: &Exit<A, E>) -> bool {
    matches!(
        outcome,
        Exit::Interrupted
            | Exit::FinalizationFailed(Finalization {
                primary: PrimaryOutcome::WasInterrupted,
                ..
            })
    )
}

/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
///
/// # Parameters
/// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
///
/// # Returns
/// Type: `Vec<CleanupFailure<E>>`.
pub(crate) fn cleanup_failures<A, E>(outcome: Exit<A, E>) -> Vec<CleanupFailure<E>> {
    match outcome {
        Exit::Success(_) => Vec::new(),
        Exit::Fail(error) => vec![CleanupFailure::ReleaseFailure(error)],
        Exit::Die(defect) => vec![CleanupFailure::ReleaseDefect(defect)],
        Exit::Interrupted => vec![CleanupFailure::ReleaseInterrupted],
        Exit::FinalizationFailed(details) => {
            let mut failures = match details.primary {
                PrimaryOutcome::Succeeded => Vec::new(),
                PrimaryOutcome::Failed(error) => vec![CleanupFailure::ReleaseFailure(error)],
                PrimaryOutcome::Defected(defect) => vec![CleanupFailure::ReleaseDefect(defect)],
                PrimaryOutcome::WasInterrupted => vec![CleanupFailure::ReleaseInterrupted],
            };
            failures.push(details.first);
            failures.extend(details.rest);
            failures
        }
    }
}

/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
///
/// # Parameters
/// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
/// - `cleanup` (`Exit<(), E>`): The cleanup supplied to this operation.
///
/// # Returns
/// Type: `Exit<A, E>`.
pub(crate) fn append_cleanup<A, E>(outcome: Exit<A, E>, cleanup: Exit<(), E>) -> Exit<A, E> {
    append_failures(outcome, cleanup_failures(cleanup))
}

/// Parallel losers retain their cleanup failures, not their discarded primary.
/// The caller owns destruction and must invoke this outside bookkeeping gates.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
///
/// # Parameters
/// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
///
/// # Returns
/// Type: `Vec<CleanupFailure<E>>`.
pub(crate) fn failures<A, E>(outcome: Exit<A, E>) -> Vec<CleanupFailure<E>> {
    match outcome {
        Exit::FinalizationFailed(details) => {
            std::iter::once(details.first).chain(details.rest).collect()
        }
        Exit::Success(_) | Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted => Vec::new(),
    }
}

/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
///
/// # Parameters
/// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
/// - `additional` (`impl IntoIterator<Item = CleanupFailure<E>>`): The additional supplied to this operation.
///
/// # Returns
/// Type: `Exit<A, E>`.
pub(crate) fn append_failures<A, E>(
    outcome: Exit<A, E>,
    additional: impl IntoIterator<Item = CleanupFailure<E>>,
) -> Exit<A, E> {
    let mut additional = additional.into_iter();
    let Some(first) = additional.next() else {
        return outcome;
    };
    let primary = match outcome {
        Exit::Success(_) => PrimaryOutcome::Succeeded,
        Exit::Fail(error) => PrimaryOutcome::Failed(error),
        Exit::Die(defect) => PrimaryOutcome::Defected(defect),
        Exit::Interrupted => PrimaryOutcome::WasInterrupted,
        Exit::FinalizationFailed(mut details) => {
            details.rest.push(first);
            details.rest.extend(additional);
            return Exit::FinalizationFailed(details);
        }
    };
    Exit::FinalizationFailed(Finalization {
        primary,
        first,
        rest: additional.collect(),
    })
}
