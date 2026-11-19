//! Region-owned root input companion; construction and execution use the same
//! graph registry, reservation order and Step driver as external environments.
use super::*;

impl<F, D, P, G> ConstructedGraph<F, D, P, OwnedConstructor<G>>
where
    D: Description,
    F: Fn() -> D + Send + Sync + 'static,
    P: crate::internal::runtime::prepared::ConstructionShape,
    G: Fn(&Environment<D::Env>, P::Vacancies<D::Value, D::Error>) -> Result<(), MemoryError>
        + Send
        + Sync
        + 'static,
{
    pub fn admit_value(&self, value: D::Env) -> OwnedGraphAdmission<D> {
        if !self.graph.owned_environment || self.graph.shared_environment {
            return Err(MemoryError::UnsupportedRunContract);
        }
        let (invocation, view, _, ()) = self.graph.admit_with(
            RootEnvironment::Owned(value),
            |context| self.reserve_construction(context),
            |context, vacancies, environment| {
                (self.preflight.0)(environment, vacancies)?;
                context.construction_ready()
            },
        )?;
        view.map(|view| (invocation, view))
            .ok_or(MemoryError::UnsupportedRunContract)
    }
    pub fn close(&self) {
        self.graph.close();
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.graph.active()
    }
    #[must_use]
    pub fn policy(&self) -> MemoryPolicy {
        self.graph.policy()
    }
}

impl<F, D, P, I, G> ConstructedGraph<F, D, P, InputConstructor<I, G>>
where
    D: Description,
    F: Fn() -> D + Send + Sync + 'static,
    P: crate::internal::runtime::prepared::ConstructionShape,
    I: 'static,
    G: Fn(&I, P::Vacancies<D::Value, D::Error>) -> Result<(), MemoryError> + Send + Sync + 'static,
{
    /// Acquire the real whole target and construct its actual foreign/native
    /// dependencies using available inputs BEFORE business R selection.
    pub fn admit_unpublished(&self, inputs: &I) -> DeferredGraphAdmission<D> {
        if !self.graph.owned_environment || self.graph.shared_environment {
            return Err(MemoryError::UnsupportedRunContract);
        }
        let (invocation, _, publisher, ()) = self.graph.admit_with(
            RootEnvironment::Unpublished,
            |context| self.reserve_construction(context),
            |context, vacancies, _| {
                (self.preflight.0)(inputs, vacancies)?;
                context.construction_ready()
            },
        )?;
        publisher
            .and_then(|publisher| match publisher {
                PendingPublisher::Owned(publisher) => Some(PendingGraphInvocation {
                    invocation,
                    publisher,
                }),
                PendingPublisher::Shared(_) => None,
            })
            .ok_or(MemoryError::UnsupportedRunContract)
    }
    /// Admit the constructed graph before selecting its externally owned Arc environment.
    pub fn admit_unpublished_shared(&self, inputs: &I) -> DeferredSharedGraphAdmission<D> {
        if !self.graph.shared_environment {
            return Err(MemoryError::UnsupportedRunContract);
        }
        let (invocation, _, publisher, ()) = self.graph.admit_with(
            RootEnvironment::UnpublishedShared,
            |context| self.reserve_construction(context),
            |context, vacancies, _| {
                (self.preflight.0)(inputs, vacancies)?;
                context.construction_ready()
            },
        )?;
        match publisher {
            Some(PendingPublisher::Shared(publisher)) => Ok(PendingSharedGraphInvocation {
                invocation,
                publisher,
            }),
            _ => Err(MemoryError::UnsupportedRunContract),
        }
    }
    pub fn close(&self) {
        self.graph.close();
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.graph.active()
    }
    #[must_use]
    pub fn policy(&self) -> MemoryPolicy {
        self.graph.policy()
    }
}
