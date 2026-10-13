//! Resource-aware entity definitions and checked activation catalogs over the existing dispatcher.

use super::{
    ClusterEntityType, ClusterOwner, EntityDispatcher, EntityRegistrationError, EntityResourcePlan,
};
use crate::rpc::RpcEnvelopePolicy;
use crate::{Effect, Scope};
use crate::{
    effect, effect_map, effect_map_error, effect_provide_some, effect_sync, effect_try_sync,
    service,
};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Static registration factory shares acquired state only within one activation.
type Register<S, R> = dyn Fn(Arc<S>, RpcEnvelopePolicy) -> Result<EntityDispatcher<R>, EntityRegistrationError>
    + Send
    + Sync;
/// Inert entity description. A supplied activation scope owns its resources; registration
/// callbacks receive shared live state without copying it into requests or durable records.
pub struct EntityDefinition<S: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    /// Stable entity type supplied by the application.
    entity_type: ClusterEntityType,
    /// Deferred acquisitions under the activation's owner and scope.
    resources: EntityResourcePlan<S, E, R>,
    /// Typed handler registration, invoked after acquisition rather than during declaration.
    register: Arc<Register<S, R>>,
}
impl<S: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>
    EntityDefinition<S, E, R>
{
    /// Associate a plan and typed registration factory without acquiring any live state.
    #[must_use]
    pub fn new(
        entity_type: ClusterEntityType,
        resources: EntityResourcePlan<S, E, R>,
        register: impl Fn(
            Arc<S>,
            RpcEnvelopePolicy,
        ) -> Result<EntityDispatcher<R>, EntityRegistrationError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            entity_type,
            resources,
            register: Arc::new(register),
        }
    }
    /// Borrow the explicit stable type identity used by the catalog and dispatcher.
    #[must_use]
    pub fn entity_type(&self) -> &ClusterEntityType {
        &self.entity_type
    }
    /// Acquire under the supplied scope, then register handlers which retain the resulting state.
    /// Construction failures remain typed; the activation scope owns partial acquisitions.
    fn build(
        &self,
        scope: Scope<E>,
        owner: ClusterOwner,
        policy: RpcEnvelopePolicy,
    ) -> Effect<EntityDispatcher<R>, EntityDefinitionError<E>, R> {
        if owner.address.entity_type != self.entity_type {
            return effect_try_sync! { body: { let __dsl_error = || EntityDefinitionError::AddressMismatch; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
        }
        let register = Arc::clone(&self.register);
        {
            let __dsl_program = effect_map_error! {
            program: self.resources
            .acquire(scope, owner), map: EntityDefinitionError::Resource, };
            effect! {
                let register = register.clone(); let state = __dsl_program.clone()?; {
                effect_try_sync! { once; body: { let __dsl_result = register(Arc::new(state), policy).map_err(EntityDefinitionError::Registration); move || __dsl_result }, on_error: ::core::convert::identity, }
            }? }
        }
    }
}
/// Failure before a resource-aware activation can publish a dispatcher.
#[derive(Debug)]
pub enum EntityDefinitionError<E> {
    /// The owner is for another entity type.
    AddressMismatch,
    /// A plan acquisition failed; its supplied scope still owns prior acquisitions.
    Resource(E),
    /// Existing dispatcher duplicate/registration validation failed.
    Registration(EntityRegistrationError),
    /// The catalog does not contain the requested entity type.
    UnknownEntity,
}
/// Checked catalog construction error before any resources are acquired.
#[derive(Debug)]
pub enum ClusterDefinitionError {
    /// Two definitions use the same explicit stable type.
    Duplicate(ClusterEntityType),
    /// A catalog must contain at least one definition or registration.
    Empty,
}
/// Erased only across live state types; success, errors, environment and owner remain typed.
type Activation<E, R> = dyn Fn(
        Scope<E>,
        ClusterOwner,
        RpcEnvelopePolicy,
    ) -> Effect<EntityDispatcher<R>, EntityDefinitionError<E>, R>
    + Send
    + Sync;
/// Checked resource-aware catalog using the current dispatcher. It creates no runner or store;
/// callers acquire an activation dispatcher, then use it with the existing cluster execution.
pub struct ClusterDefinition<E: Send + 'static, R: Send + Sync + 'static> {
    /// Explicit protocol envelope/framing policy shared by catalog registrations.
    policy: RpcEnvelopePolicy,
    /// Deferred activation operations keyed by stable entity type.
    entities: BTreeMap<ClusterEntityType, Arc<Activation<E, R>>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> ClusterDefinition<E, R> {
    /// Begin checked assembly with an explicit policy; finish rejects an empty catalog.
    #[must_use]
    pub fn new(policy: RpcEnvelopePolicy) -> Self {
        Self {
            policy,
            entities: BTreeMap::new(),
        }
    }
    /// Register a typed resource-aware entity without executing its plan.
    pub fn register<S: Send + Sync + 'static>(
        &mut self,
        definition: EntityDefinition<S, E, R>,
    ) -> Result<(), ClusterDefinitionError> {
        let identity = definition.entity_type.clone();
        if self.entities.contains_key(&identity) {
            return Err(ClusterDefinitionError::Duplicate(identity));
        }
        self.entities.insert(
            identity,
            Arc::new(move |scope, owner, policy| definition.build(scope, owner, policy)),
        );
        Ok(())
    }
    /// Register an existing dispatcher as an explicitly identified entity catalog entry.
    /// Its own addressed acceptance and handler conflict checks remain authoritative.
    pub fn register_dispatcher(
        &mut self,
        identity: ClusterEntityType,
        dispatcher: EntityDispatcher<R>,
    ) -> Result<(), ClusterDefinitionError> {
        if self.entities.contains_key(&identity) {
            return Err(ClusterDefinitionError::Duplicate(identity));
        }
        self.entities.insert(
            identity.clone(),
            Arc::new(move |_, owner, _| {
                if owner.address.entity_type != identity {
                    return effect_try_sync! { body: { let __dsl_error = || EntityDefinitionError::AddressMismatch; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
                }
                let dispatcher = dispatcher.clone();
                effect_sync! { body: move || dispatcher.clone(), }
            }),
        );
        Ok(())
    }
    /// Finish checked assembly without acquiring resources or dispatching messages.
    pub fn finish(self) -> Result<Self, ClusterDefinitionError> {
        if self.entities.is_empty() {
            Err(ClusterDefinitionError::Empty)
        } else {
            Ok(self)
        }
    }
    /// Build the existing dispatcher lazily under the exact activation owner and supplied scope.
    fn build(
        &self,
        scope: Scope<E>,
        owner: ClusterOwner,
    ) -> Effect<EntityDispatcher<R>, EntityDefinitionError<E>, R> {
        match self.entities.get(&owner.address.entity_type) {
            Some(activate) => activate(scope, owner, self.policy),
            None => {
                effect_try_sync! { body: { let __dsl_error = || EntityDefinitionError::UnknownEntity; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }
            }
        }
    }
}
/// A catalog accepts resource definitions or explicitly paired legacy registrations.
/// Implementations are sealed to retain the same dispatch/runtime boundary.
pub trait ClusterEntry<E: Send + 'static, R: Send + Sync + 'static>: entry_sealed::Sealed {
    /// Register without execution, retaining typed resource/registration validation.
    fn register_in(
        self,
        catalog: &mut ClusterDefinition<E, R>,
    ) -> Result<(), ClusterDefinitionError>;
}
/// Sealing restricts catalog registration to the supported typed descriptors.
mod entry_sealed {
    /// Internal catalog admission marker.
    pub trait Sealed {}
    impl<S: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for super::EntityDefinition<S, E, R>
    {
    }
    impl<R: Send + Sync + 'static> Sealed for (super::ClusterEntityType, super::EntityDispatcher<R>) {}
}
impl<S: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> ClusterEntry<E, R>
    for EntityDefinition<S, E, R>
{
    /// Preserve the definition's acquisition/error context through catalog registration.
    fn register_in(
        self,
        catalog: &mut ClusterDefinition<E, R>,
    ) -> Result<(), ClusterDefinitionError> {
        catalog.register(self)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> ClusterEntry<E, R>
    for (ClusterEntityType, EntityDispatcher<R>)
{
    /// Retain an already checked legacy dispatcher under its explicit stable identity.
    fn register_in(
        self,
        catalog: &mut ClusterDefinition<E, R>,
    ) -> Result<(), ClusterDefinitionError> {
        catalog.register_dispatcher(self.0, self.1)
    }
}

/// Typed activation validation and dispatch failures, retaining provider errors without erasure.
#[derive(Debug)]
pub enum EntityActivationError<E, S, C> {
    /// Acquisition or registration failed before live state could be published.
    Definition(EntityDefinitionError<E>),
    /// The activation provider failed while checking the current fence.
    Store(S),
    /// The environment wall clock could not supply a check instant.
    Clock(C),
    /// The activation owner expired, was replaced or is no longer active.
    StaleOwnership,
    /// A dispatch address differs from the activated resource owner.
    AddressMismatch,
    /// Existing typed request/reply framing, codec or handler dispatch failed.
    Dispatch(super::EntityDispatchError),
}
/// A typed live dispatcher associated with one exact activation owner. Resources stay in the
/// caller's activation scope; every dispatch rechecks ownership before invoking live handlers.
pub struct ActivatedEntity<R: Send + Sync + 'static> {
    /// Existing handler registry retaining shared live state for this activation.
    dispatcher: EntityDispatcher<R>,
    /// Exact activation fence checked before publication and each dispatch.
    owner: ClusterOwner,
}
impl<R: Send + Sync + 'static> Clone for ActivatedEntity<R> {
    /// Share handler descriptions and the owner identity without copying live state payloads.
    fn clone(&self) -> Self {
        Self {
            dispatcher: self.dispatcher.clone(),
            owner: self.owner.clone(),
        }
    }
}
/// Provider-typed outcome of activation or resource-aware dispatch.
pub type EntityActivationEffect<A, E, R> = Effect<
    A,
    EntityActivationError<
        E,
        <<R as super::HasClusterActivationStore>::Store as super::ClusterActivationStore>::Error,
        <<R as crate::HasWallClock>::Clock as crate::WallClock>::Error,
    >,
    R,
>;
impl<R> ActivatedEntity<R>
where
    R: super::HasClusterActivationStore + crate::HasWallClock + Send + Sync + 'static,
{
    /// Validate the exact active owner and lease using the existing environment authorities.
    fn validate<E: Send + 'static>(&self) -> EntityActivationEffect<(), E, R> {
        use super::ClusterActivationStore;
        let owner = self.owner.clone();
        {
            let __dsl_program = effect_map_error! { program: Effect::wall_now(), map: EntityActivationError::Clock, };
            effect! {
            let owner = owner.clone(); let now = __dsl_program.clone()?; {
                let owner = owner.clone();
                let inspect_owner = owner.clone();
                {
                    let __dsl_program = effect_map_error! {
                    program: {
                    let __dsl_program = service! { env: |environment: &R| environment.cluster_activation_store(), }; effect! {
                    let inspect_owner = inspect_owner.clone(); let store = __dsl_program.clone()?; {
                        effect_provide_some! {
                            program: store
                            .inspect_activation(inspect_owner.address.clone()), project: |_: &R| (), }
                    }? } }, map: EntityActivationError::Store, }; let owner = owner.clone(); effect! {
                        let __dsl_program = __dsl_program.clone(); let owner = owner.clone(); let status = __dsl_program?; {
                            effect_try_sync! {
                                once; body: {
                                let __dsl_result = if status.is_some_and(|status| {
                                    status.phase == super::ActivationPhase::Active
                                        && status.lease.owner == owner
                                        && now < status.lease.lease_until
                                }) {
                                    Ok(())
                                } else {
                                    Err(EntityActivationError::StaleOwnership)
                                }; move || __dsl_result }, on_error: ::core::convert::identity, }
                        }? } }
            }? }
        }
    }
    /// Execute an addressed request after checking the exact owner fence under the same R.
    pub fn execute(
        &self,
        address: super::ClusterAddress,
        request: Arc<[u8]>,
    ) -> EntityActivationEffect<super::SavedReply, std::convert::Infallible, R> {
        use super::ClusterExecutor;
        if address != self.owner.address {
            return effect_try_sync! { body: { let __dsl_error = || EntityActivationError::AddressMismatch; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
        }
        let dispatcher = self.dispatcher.clone();
        {
            let __dsl_program = self.validate();
            effect! {
                let __dsl_program = __dsl_program.clone(); let address = address.clone(); let dispatcher = dispatcher.clone(); let request = request.clone(); let () = __dsl_program?; {
                effect_map_error! {
                    program: dispatcher
                    .execute(address.clone(), Arc::clone(&request)), map: EntityActivationError::Dispatch, }
            }? }
        }
    }
}
impl<S: Send + Sync + 'static, E: Send + 'static, R> EntityDefinition<S, E, R>
where
    R: super::HasClusterActivationStore + crate::HasWallClock + Send + Sync + 'static,
{
    /// Acquire and register live state, then validate ownership before publishing its dispatcher.
    /// The external scope retains partial acquisitions even if registration or fencing fails.
    pub fn activate(
        &self,
        scope: Scope<E>,
        owner: ClusterOwner,
        policy: RpcEnvelopePolicy,
    ) -> EntityActivationEffect<ActivatedEntity<R>, E, R> {
        let live_owner = owner.clone();
        {
            let __dsl_program = effect_map_error! { program: self.build(scope, owner, policy), map: EntityActivationError::Definition, };
            effect! {
            let live_owner = live_owner.clone(); let dispatcher = __dsl_program.clone()?; {
                let active = ActivatedEntity {
                    dispatcher,
                    owner: live_owner.clone(),
                };
                let result = active.clone();
                effect_map! { program: active.validate(), map: move |()| result.clone(), }
            }? }
        }
    }
}
impl<E: Send + 'static, R> ClusterDefinition<E, R>
where
    R: super::HasClusterActivationStore + crate::HasWallClock + Send + Sync + 'static,
{
    /// Activate the selected definition and publish only a currently fenced dispatcher.
    pub fn activate(
        &self,
        scope: Scope<E>,
        owner: ClusterOwner,
    ) -> EntityActivationEffect<ActivatedEntity<R>, E, R> {
        let live_owner = owner.clone();
        {
            let __dsl_program = effect_map_error! { program: self.build(scope, owner), map: EntityActivationError::Definition, };
            effect! {
            let live_owner = live_owner.clone(); let dispatcher = __dsl_program.clone()?; {
                let active = ActivatedEntity {
                    dispatcher,
                    owner: live_owner.clone(),
                };
                let result = active.clone();
                effect_map! { program: active.validate(), map: move |()| result.clone(), }
            }? }
        }
    }
}
impl<R> super::ClusterExecutor<R> for ActivatedEntity<R>
where
    R: super::HasClusterActivationStore + crate::HasWallClock + Send + Sync + 'static,
{
    /// Provider failures and stale fencing remain typed alongside existing dispatch failures.
    type Error = EntityActivationError<
        std::convert::Infallible,
        <R::Store as super::ClusterActivationStore>::Error,
        <R::Clock as crate::WallClock>::Error,
    >;
    /// Route the existing cluster executor contract through exact-owner validation.
    fn execute(
        &self,
        address: super::ClusterAddress,
        request: Arc<[u8]>,
    ) -> Effect<super::SavedReply, Self::Error, R> {
        self.execute(address, request)
    }
}
