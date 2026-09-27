use crate::Defect;

/// The original outcome retained when cleanup also fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimaryOutcome<E> {
    Succeeded,
    Failed(E),
    Defected(Defect),
    WasInterrupted,
}

/// Borrowed view used by release callbacks without cloning the primary error.
pub enum PrimaryOutcomeRef<'outcome, E> {
    Succeeded,
    Failed(&'outcome E),
    Defected(&'outcome Defect),
    WasInterrupted,
}

/// One failure of an owned resource's release operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupFailure<E> {
    ReleaseFailure(E),
    ReleaseDefect(Defect),
    ReleaseInterrupted,
}

/// Ordered cleanup failures; `first` makes the collection nonempty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finalization<E> {
    pub primary: PrimaryOutcome<E>,
    pub first: CleanupFailure<E>,
    pub rest: Vec<CleanupFailure<E>>,
}

/// Complete terminal outcome. Recovery of expected errors leaves other channels intact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit<A, E> {
    Success(A),
    Fail(E),
    Die(Defect),
    Interrupted,
    FinalizationFailed(Finalization<E>),
}

impl<A, E> Exit<A, E> {
    #[must_use]
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
    pub fn map_error<E2>(self, mut transform: impl FnMut(E) -> E2) -> Exit<A, E2> {
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
