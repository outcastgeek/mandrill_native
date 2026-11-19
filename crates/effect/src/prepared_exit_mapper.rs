//! Preadmitted, bounded error-channel mapping for prepared outcomes.
use crate::memory::MemoryError;
use crate::{
    CleanupFailure, CleanupFailures, Defect, DefectKind, Exit, Finalization, LibraryDefect,
    PreparedCleanupConfig, PrimaryOutcome,
};

/// An affine target carrier reserved before the operation producing an exit.
/// Its finite secondary-entry capacity and overflow summary are those selected
/// in the supplied prepared cleanup configuration.
pub struct PreparedExitMapper<E: Send + 'static> {
    rest: CleanupFailures<E>,
}
impl<E2: Send + 'static> PreparedExitMapper<E2> {
    pub(crate) fn into_rest(self) -> CleanupFailures<E2> {
        self.rest
    }
    /// Number of prepared cleanup carriers needed per simultaneous mapping.
    #[must_use]
    pub const fn cleanup_buffers_per_mapping() -> usize {
        1
    }
    /// Acquire real prepared target backing before any candidate or business factory.
    pub fn admit(config: &PreparedCleanupConfig<E2>) -> Result<Self, MemoryError> {
        Ok(Self {
            rest: config.reserve_failures()?,
        })
    }
    /// Map expected errors in original order, without a temporary Global collection.
    /// Callback panics propagate to the existing caller boundary. A non-finalization
    /// result retires the unused target carrier before this method returns.
    pub fn map<A, E1>(self, outcome: Exit<A, E1>, transform: impl FnMut(E1) -> E2) -> Exit<A, E2> {
        self.map_with_limit(outcome, None, transform)
    }

    // An admitted expected-value cell is needed only for a retained error.
    // Keep the original primary, first, and order of the retained suffix.
    pub(crate) fn map_retained<A, E1>(
        self,
        outcome: Exit<A, E1>,
        retained: usize,
        transform: impl FnMut(E1) -> E2,
    ) -> Exit<A, E2> {
        self.map_with_limit(outcome, Some(retained), transform)
    }

    fn map_with_limit<A, E1>(
        self,
        outcome: Exit<A, E1>,
        retained: Option<usize>,
        mut transform: impl FnMut(E1) -> E2,
    ) -> Exit<A, E2> {
        fn map_failure<E1, E2>(
            failure: CleanupFailure<E1>,
            transform: &mut impl FnMut(E1) -> E2,
        ) -> CleanupFailure<E2> {
            match failure {
                CleanupFailure::ReleaseFailure(error) => {
                    CleanupFailure::ReleaseFailure(transform(error))
                }
                CleanupFailure::ReleaseDefect(defect) => CleanupFailure::ReleaseDefect(defect),
                CleanupFailure::ReleaseInterrupted => CleanupFailure::ReleaseInterrupted,
            }
        }
        match outcome {
            Exit::Success(value) => Exit::Success(value),
            Exit::Fail(error) => Exit::Fail(transform(error)),
            Exit::Die(defect) => Exit::Die(defect),
            Exit::Interrupted => Exit::Interrupted,
            Exit::FinalizationFailed(details) => {
                let primary = match details.primary {
                    PrimaryOutcome::Succeeded => PrimaryOutcome::Succeeded,
                    PrimaryOutcome::Failed(error) => PrimaryOutcome::Failed(transform(error)),
                    PrimaryOutcome::Defected(defect) => PrimaryOutcome::Defected(defect),
                    PrimaryOutcome::WasInterrupted => PrimaryOutcome::WasInterrupted,
                };
                let first = map_failure(details.first, &mut transform);
                let mut rest = self.rest;
                let mut retained_count = 0;
                for failure in details.rest {
                    let summary = retained.is_some()
                        && matches!(&failure, CleanupFailure::ReleaseDefect(defect)
                        if matches!(defect.kind(), DefectKind::Library(LibraryDefect::CleanupOverflow { .. })));
                    if retained.is_none_or(|limit| retained_count < limit) || summary {
                        rest.push(map_failure(failure, &mut transform));
                        retained_count += usize::from(retained.is_some() && !summary);
                    } else {
                        rest.push(CleanupFailure::ReleaseDefect(Defect::library(
                            LibraryDefect::CleanupOverflow {
                                omitted: 1,
                                saturated: false,
                            },
                        )));
                        drop(failure);
                    }
                }
                Exit::FinalizationFailed(Finalization {
                    primary,
                    first,
                    rest,
                })
            }
        }
    }
}
