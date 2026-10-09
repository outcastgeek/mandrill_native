//! Typed pure projection cases deriving identity acceptance and decoding from one event definition.

use super::{
    EventCodecError, EventDefinitionView, EventGroupId, EventTag, EventVersion, JournalEntry,
    Projection, ProjectionKey, ProjectionVersion,
};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Failure constructing an unambiguous typed projection dispatch table.
#[derive(Debug)]
pub enum ProjectionCasesError {
    /// Two cases claim the same group/tag, including conflicting versions.
    Duplicate {
        /// Conflicting event group identity.
        group: EventGroupId,
        /// Conflicting event tag.
        tag: EventTag,
    },
    /// A typed-case description must supply at least one event definition.
    Empty,
}
/// Decoding and pure fold failures remain distinct; none advances the projection checkpoint.
#[derive(Debug)]
pub enum ProjectionCaseError<F> {
    /// The envelope does not have a registered group/tag identity.
    Unregistered,
    /// The accepted identity's version differs from this definition.
    VersionMismatch,
    /// The authoritative payload/outcome decoder rejected the encoded entry.
    Decode(EventCodecError),
    /// The typed application fold returned its expected error.
    Fold(F),
}
/// Existing projection-engine fold callback after static typed case registration.
type Fold<S, F> = dyn Fn(&S, &JournalEntry) -> Result<S, ProjectionCaseError<F>> + Send + Sync;
/// Checked assembly of typed case callbacks; runtime dispatch uses the existing journal's
/// stable group/tag identities, while each registered decoder and fold retain their types.
pub struct ProjectionCases<S, F> {
    /// One accepted version per registered group/tag, sourced from the same definition as decoding.
    accepted: BTreeMap<(EventGroupId, EventTag), EventVersion>,
    /// Process-local typed decoder/fold callbacks, never stored in event records.
    folds: BTreeMap<(EventGroupId, EventTag), Arc<Fold<S, F>>>,
}
impl<S: Send + Sync + 'static, F: Send + 'static> ProjectionCases<S, F> {
    /// Start a checked case description; finish rejects an empty table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            accepted: BTreeMap::new(),
            folds: BTreeMap::new(),
        }
    }
    /// Register one definition and typed payload/outcome fold. Values decode without cloning or
    /// serializing an intermediate representation; identity/version and codec share one authority.
    pub fn register<D: EventDefinitionView>(
        &mut self,
        definition: &D,
        fold: impl Fn(&S, D::Payload, Result<D::Success, D::Error>) -> Result<S, F>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), ProjectionCasesError> {
        let definition = definition.definition().clone();
        let key = (definition.group.clone(), definition.tag.clone());
        if self.accepted.contains_key(&key) {
            return Err(ProjectionCasesError::Duplicate {
                group: key.0,
                tag: key.1,
            });
        }
        self.accepted.insert(key.clone(), definition.version);
        self.folds.insert(
            key,
            Arc::new(move |state, entry| {
                if entry.event.version != definition.version {
                    return Err(ProjectionCaseError::VersionMismatch);
                }
                let payload = definition
                    .decode_payload(&entry.event)
                    .map_err(ProjectionCaseError::Decode)?;
                let outcome = definition
                    .decode_outcome(&entry.event)
                    .map_err(ProjectionCaseError::Decode)?;
                fold(state, payload, outcome).map_err(ProjectionCaseError::Fold)
            }),
        );
        Ok(())
    }
    /// Produce the existing Projection with checked identity/decoding dispatch. Advance remains
    /// the ordinary Effect boundary, and the raw-fold constructor remains available unchanged.
    pub fn finish(
        self,
        key: ProjectionKey,
        version: ProjectionVersion,
        initial: impl Fn() -> S + Send + Sync + 'static,
    ) -> Result<Projection<S, ProjectionCaseError<F>>, ProjectionCasesError> {
        if self.accepted.is_empty() {
            return Err(ProjectionCasesError::Empty);
        }
        let folds = self.folds;
        Ok(Projection::new(
            key,
            version,
            self.accepted,
            initial,
            move |state, entry| match folds
                .get(&(entry.event.group.clone(), entry.event.tag.clone()))
            {
                Some(fold) => fold(state, entry),
                None => Err(ProjectionCaseError::Unregistered),
            },
        ))
    }
}
impl<S: Send + Sync + 'static, F: Send + 'static> Default for ProjectionCases<S, F> {
    /// Begin empty checked assembly; finish still requires a registered case.
    fn default() -> Self {
        Self::new()
    }
}
