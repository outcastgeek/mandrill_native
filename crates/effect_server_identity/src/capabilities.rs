//! Typed D17 capability selection with storage-configuration ownership fences.
use crate::{CapabilityName, IdentityError};
use std::sync::Arc;

/// Private composition identity carried by actual providers and selected hooks.
pub(crate) struct StorageIdentity;
/// Provider-neutral configuration ownership; cloning retains one generation.
#[derive(Clone)]
pub struct StorageConfiguration {
    /// Unforgeable allocation shared with actual selected provider adapters.
    pub(crate) identity: Arc<StorageIdentity>,
}
impl Default for StorageConfiguration {
    /// Establish a fresh private composition identity, exactly as explicit construction does.
    fn default() -> Self {
        Self::new()
    }
}
impl StorageConfiguration {
    /// Establish a new explicitly selected provider composition identity without I/O.
    #[must_use]
    pub fn new() -> Self {
        Self {
            identity: Arc::new(StorageIdentity),
        }
    }
    /// Compare two retained configurations without exposing a forgeable identifier.
    #[must_use]
    pub fn same_configuration(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
}
/// Native manager/store interoperability independent of a SQL provider feature.
/// Custom native providers retain their documented authority/diagnostic contracts;
/// the SQLx implementation preserves original failures through its private carrier.
pub trait NativeBackend:
    identity_rs::identity::store::UserStore + identity_rs::identity::store::RoleStore + Clone + 'static
{
    /// Retain the actual provider's private configuration identity.
    fn configuration(&self) -> StorageConfiguration;
    /// Establish an owned native diagnostic scope without acquiring a provider or runtime.
    fn scoped(&self) -> Self;
    /// Restore a completed native result after its narrower store calls have settled.
    fn capture_outcome<A>(&self, result: Result<A, IdentityError>) -> Result<A, IdentityError>;
    /// Report a provider-retained fn-pointer UTC clock where its native store API
    /// requires one; None makes no claim about an application's stateful clock.
    fn utc_clock(&self) -> Option<fn() -> time::OffsetDateTime> {
        None
    }
}
#[cfg(feature = "sqlx-turso")]
impl NativeBackend for crate::SqlxIdentityStore {
    /// Retain the actual local SQLx store generation.
    fn configuration(&self) -> StorageConfiguration {
        self.configuration()
    }
    /// Create the private invocation store clone sharing the installed pool/policies.
    fn scoped(&self) -> Self {
        self.invocation().store()
    }
    /// Preserve native defects/interruption/ordered SQL cleanup before narrowing returns.
    fn capture_outcome<A>(&self, result: Result<A, IdentityError>) -> Result<A, IdentityError> {
        match &self.invocation {
            Some(diagnostics) => crate::error::restore_sql_result(diagnostics.take(), result),
            None => result,
        }
    }
    /// Native passkey storage uses this explicitly installed UTC callable.
    fn utc_clock(&self) -> Option<fn() -> time::OffsetDateTime> {
        Some(self.policy.utc_now)
    }
}

/// Selected persistence services expose their actual private configuration fence.
/// Application callbacks use SelectedHook; SQL adapters never accept rebinding.
pub trait StorageBound: Send + Sync + 'static {
    /// Check exact selected storage ownership, not connection strings or table names.
    fn belongs_to(&self, configuration: &StorageConfiguration) -> bool;
}
/// Explicitly absent optional selection; no native memory/default service is created.
pub struct Missing;
/// A retained selected service handle; Clone copies only its Arc configuration.
pub struct Selected<T>(Arc<T>);
impl<T> Selected<T> {
    /// Retain an explicitly selected component once without performing provider work.
    pub(crate) fn new(value: T) -> Self {
        Self(Arc::new(value))
    }
    /// Borrow the original selected component generation for native factory composition.
    #[must_use]
    pub fn service(&self) -> &T {
        &self.0
    }
}
impl<T> Clone for Selected<T> {
    /// Retain the selected service without cloning its application/credential payload.
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
/// Typed optional retrieval whose missing result remains an expected error.
pub trait CapabilitySlot {
    /// Actual selected service type; absence has the uninhabited Infallible type.
    type Service;
    /// Borrow the installed service or return its precise missing-selection category.
    fn selected(&self, name: CapabilityName) -> Result<&Self::Service, IdentityError>;
}
impl CapabilitySlot for Missing {
    type Service = std::convert::Infallible;
    /// Return typed absence without constructing a fallback service.
    fn selected(&self, name: CapabilityName) -> Result<&Self::Service, IdentityError> {
        Err(IdentityError::MissingCapability(name))
    }
}
impl<T> CapabilitySlot for Selected<T> {
    type Service = T;
    /// Borrow exactly the installed service generation.
    fn selected(&self, _: CapabilityName) -> Result<&T, IdentityError> {
        Ok(&self.0)
    }
}
/// Configuration-owned application callback, policy, renderer or external verifier.
/// SQL persistence adapters expose their own actual fence instead of using this wrapper.
pub struct SelectedHook<T> {
    /// Original selected storage owner.
    identity: Arc<StorageIdentity>,
    /// Application service selected once; no hidden implementation is reacquired.
    service: T,
}
impl<T> SelectedHook<T> {
    /// Bind an application-owned hook to the definition's selected storage domain.
    pub fn new(configuration: &StorageConfiguration, service: T) -> Self {
        Self {
            identity: configuration.identity.clone(),
            service,
        }
    }
    /// Borrow the actual selected callback/policy without performing work.
    pub fn service(&self) -> &T {
        &self.service
    }
}
impl<T: Send + Sync + 'static> StorageBound for SelectedHook<T> {
    /// Compare the callback's original private storage owner.
    fn belongs_to(&self, configuration: &StorageConfiguration) -> bool {
        Arc::ptr_eq(&self.identity, &configuration.identity)
    }
}
#[cfg(feature = "sqlx-turso")]
impl StorageBound for crate::SqlxIdentityStore {
    /// Check native store/backend identity retained by every granular SQLx method.
    fn belongs_to(&self, configuration: &StorageConfiguration) -> bool {
        Arc::ptr_eq(&self.identity, &configuration.identity)
    }
}
#[cfg(feature = "sqlx-turso")]
impl StorageBound for crate::store::SqlxBrowserContextStore {
    /// Compare the browser store's real native backend generation.
    fn belongs_to(&self, configuration: &StorageConfiguration) -> bool {
        StorageBound::belongs_to(self.backend(), configuration)
    }
}
#[cfg(feature = "sqlx-turso")]
impl StorageBound for crate::store::accounts::AtomicAccounts {
    /// Compare the aggregate adapter's actual private configuration owner.
    fn belongs_to(&self, configuration: &StorageConfiguration) -> bool {
        self.belongs_to(configuration)
    }
}
#[cfg(feature = "sqlx-turso")]
impl<A: Send + 'static, C: crate::store::records::RecordCodec<A>> StorageBound
    for crate::store::records::SqlRecordAdapter<A, C>
{
    /// Compare the private owner retained by the actual SQL record adapter.
    fn belongs_to(&self, configuration: &StorageConfiguration) -> bool {
        self.belongs_to(configuration)
    }
}
#[cfg(feature = "sqlx-turso")]
impl<A: Send + 'static, C: crate::store::records::RecordCodec<A>> StorageBound
    for crate::store::records::SqlAuditAdapter<A, C>
{
    /// Compare the private owner retained by the actual SQL audit adapter.
    fn belongs_to(&self, configuration: &StorageConfiguration) -> bool {
        self.belongs_to(configuration)
    }
}
#[cfg(feature = "sqlx-turso")]
impl StorageBound for crate::store::components::KeyRepository {
    /// Compare the acknowledged key repository's underlying SQL record generation.
    fn belongs_to(&self, configuration: &StorageConfiguration) -> bool {
        self.records().belongs_to(configuration)
    }
}
#[cfg(feature = "sqlx-turso")]
impl<T: identity_rs::identity::ticket::AuthenticationTicketFormat + 'static> StorageBound
    for crate::store::components::SessionRepository<T>
{
    /// Compare the actual selected session persistence generation.
    fn belongs_to(&self, configuration: &StorageConfiguration) -> bool {
        self.records().belongs_to(configuration)
    }
}
#[cfg(feature = "sqlx-turso")]
impl<A: Send + 'static, C: crate::store::records::RecordCodec<A>> StorageBound
    for crate::store::auxiliary::ChallengeRepository<A, C>
{
    /// Compare the challenge repository's actual private owner.
    fn belongs_to(&self, configuration: &StorageConfiguration) -> bool {
        self.belongs_to(configuration)
    }
}
#[cfg(feature = "sqlx-turso")]
impl<A: Send + 'static, C: crate::store::records::RecordCodec<A>> StorageBound
    for crate::store::auxiliary::RequestRepository<A, C>
{
    /// Compare the durable acceptance repository's actual private owner.
    fn belongs_to(&self, configuration: &StorageConfiguration) -> bool {
        self.belongs_to(configuration)
    }
}
/// Typed mandatory storage selection used by inert Identity definitions.
pub trait StoreSelection: Send + Sync + 'static {
    /// Native provider type selected explicitly by this composition.
    type Backend: NativeBackend;
    /// Borrow the exact backend generation passed into invocation-owned native factories.
    fn backend(&self) -> &Self::Backend;
}
/// Inert typed selection bundle for every F# Identity storage capability.
pub struct IdentityStores<
    B,
    A = Missing,
    S = Missing,
    K = Missing,
    Ro = Missing,
    P = Missing,
    Ch = Missing,
    Q = Missing,
    Au = Missing,
    Co = Missing,
    D = Missing,
    N = Missing,
    J = Missing,
    Pd = Missing,
    Uq = Missing,
    Rq = Missing,
> {
    /// Mandatory explicitly selected native account/role backend; SQLx supplies the production adapter.
    backend: B,
    /// Explicit optional accounts selection.
    accounts: A,
    /// Explicit optional sessions selection.
    sessions: S,
    /// Explicit optional keys selection.
    keys: K,
    /// Explicit optional roles selection.
    roles: Ro,
    /// Explicit optional permissions selection.
    permissions: P,
    /// Explicit optional challenges selection.
    challenges: Ch,
    /// Explicit optional queries selection.
    queries: Q,
    /// Explicit optional audit selection.
    audit: Au,
    /// Explicit optional consent selection.
    consent: Co,
    /// Explicit optional delivery selection.
    delivery: D,
    /// Explicit optional notifications selection.
    notifications: N,
    /// Explicit optional jobs selection.
    jobs: J,
    /// Explicit optional personal data selection.
    personal_data: Pd,
    /// Explicit optional native user queries selection.
    native_user_queries: Uq,
    /// Explicit optional native role queries selection.
    native_role_queries: Rq,
}
/// Compile-time return types for replacing one storage slot; projection performs no provider work.
pub trait StoreSelections {
    /// Exact builder result after replacing Accounts; every other slot retains its type.
    type Accounts<Next>;
    /// Exact builder result after replacing Sessions; every other slot retains its type.
    type Sessions<Next>;
    /// Exact builder result after replacing Keys; every other slot retains its type.
    type Keys<Next>;
    /// Exact builder result after replacing Roles; every other slot retains its type.
    type Roles<Next>;
    /// Exact builder result after replacing Permissions; every other slot retains its type.
    type Permissions<Next>;
    /// Exact builder result after replacing Challenges; every other slot retains its type.
    type Challenges<Next>;
    /// Exact builder result after replacing Queries; every other slot retains its type.
    type Queries<Next>;
    /// Exact builder result after replacing Audit; every other slot retains its type.
    type Audit<Next>;
    /// Exact builder result after replacing Consent; every other slot retains its type.
    type Consent<Next>;
    /// Exact builder result after replacing Delivery; every other slot retains its type.
    type Delivery<Next>;
    /// Exact builder result after replacing Notifications; every other slot retains its type.
    type Notifications<Next>;
    /// Exact builder result after replacing Jobs; every other slot retains its type.
    type Jobs<Next>;
    /// Exact builder result after replacing PersonalData; every other slot retains its type.
    type PersonalData<Next>;
    /// Exact builder result after replacing NativeUserQueries; every other slot retains its type.
    type NativeUserQueries<Next>;
    /// Exact builder result after replacing NativeRoleQueries; every other slot retains its type.
    type NativeRoleQueries<Next>;
}
impl<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq> StoreSelections
    for IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
{
    /// Preserve the concrete bag and its existing slots when replacing Accounts.
    type Accounts<Next> = Result<
        IdentityStores<B, Selected<Next>, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing Sessions.
    type Sessions<Next> = Result<
        IdentityStores<B, A, Selected<Next>, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing Keys.
    type Keys<Next> = Result<
        IdentityStores<B, A, S, Selected<Next>, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing Roles.
    type Roles<Next> = Result<
        IdentityStores<B, A, S, K, Selected<Next>, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing Permissions.
    type Permissions<Next> = Result<
        IdentityStores<B, A, S, K, Ro, Selected<Next>, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing Challenges.
    type Challenges<Next> = Result<
        IdentityStores<B, A, S, K, Ro, P, Selected<Next>, Q, Au, Co, D, N, J, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing Queries.
    type Queries<Next> = Result<
        IdentityStores<B, A, S, K, Ro, P, Ch, Selected<Next>, Au, Co, D, N, J, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing Audit.
    type Audit<Next> = Result<
        IdentityStores<B, A, S, K, Ro, P, Ch, Q, Selected<Next>, Co, D, N, J, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing Consent.
    type Consent<Next> = Result<
        IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Selected<Next>, D, N, J, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing Delivery.
    type Delivery<Next> = Result<
        IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, Selected<Next>, N, J, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing Notifications.
    type Notifications<Next> = Result<
        IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, Selected<Next>, J, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing Jobs.
    type Jobs<Next> = Result<
        IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, Selected<Next>, Pd, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing PersonalData.
    type PersonalData<Next> = Result<
        IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Selected<Next>, Uq, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing NativeUserQueries.
    type NativeUserQueries<Next> = Result<
        IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Selected<Next>, Rq>,
        IdentityError,
    >;
    /// Preserve the concrete bag and its existing slots when replacing NativeRoleQueries.
    type NativeRoleQueries<Next> = Result<
        IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Selected<Next>>,
        IdentityError,
    >;
}
/// Checked result of selecting configuration-owned accounts; all other slots retain their exact types.
pub type WithAccounts<Current, Next> = <Current as StoreSelections>::Accounts<Next>;

/// Checked result of selecting configuration-owned sessions; all other slots retain their exact types.
pub type WithSessions<Current, Next> = <Current as StoreSelections>::Sessions<Next>;

/// Checked result of selecting configuration-owned keys; all other slots retain their exact types.
pub type WithKeys<Current, Next> = <Current as StoreSelections>::Keys<Next>;

/// Checked result of selecting configuration-owned roles; all other slots retain their exact types.
pub type WithRoles<Current, Next> = <Current as StoreSelections>::Roles<Next>;

/// Checked result of selecting configuration-owned permissions; all other slots retain their exact types.
pub type WithPermissions<Current, Next> = <Current as StoreSelections>::Permissions<Next>;

/// Checked result of selecting configuration-owned challenges; all other slots retain their exact types.
pub type WithChallenges<Current, Next> = <Current as StoreSelections>::Challenges<Next>;

/// Checked result of selecting configuration-owned queries; all other slots retain their exact types.
pub type WithQueries<Current, Next> = <Current as StoreSelections>::Queries<Next>;

/// Checked result of selecting configuration-owned audit; all other slots retain their exact types.
pub type WithAudit<Current, Next> = <Current as StoreSelections>::Audit<Next>;

/// Checked result of selecting configuration-owned consent; all other slots retain their exact types.
pub type WithConsent<Current, Next> = <Current as StoreSelections>::Consent<Next>;

/// Checked result of selecting configuration-owned delivery; all other slots retain their exact types.
pub type WithDelivery<Current, Next> = <Current as StoreSelections>::Delivery<Next>;

/// Checked result of selecting configuration-owned notifications; all other slots retain their exact types.
pub type WithNotifications<Current, Next> = <Current as StoreSelections>::Notifications<Next>;

/// Checked result of selecting configuration-owned jobs; all other slots retain their exact types.
pub type WithJobs<Current, Next> = <Current as StoreSelections>::Jobs<Next>;

/// Checked result of selecting configuration-owned personal data; all other slots retain their exact types.
pub type WithPersonalData<Current, Next> = <Current as StoreSelections>::PersonalData<Next>;

/// Checked result of selecting configuration-owned native user queries; all other slots retain their exact types.
pub type WithNativeUserQueries<Current, Next> =
    <Current as StoreSelections>::NativeUserQueries<Next>;

/// Checked result of selecting configuration-owned native role queries; all other slots retain their exact types.
pub type WithNativeRoleQueries<Current, Next> =
    <Current as StoreSelections>::NativeRoleQueries<Next>;

impl<B: NativeBackend> IdentityStores<B> {
    /// Select the mandatory SQLx backend with every auxiliary capability explicitly absent.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            accounts: Missing,
            sessions: Missing,
            keys: Missing,
            roles: Missing,
            permissions: Missing,
            challenges: Missing,
            queries: Missing,
            audit: Missing,
            consent: Missing,
            delivery: Missing,
            notifications: Missing,
            jobs: Missing,
            personal_data: Missing,
            native_user_queries: Missing,
            native_role_queries: Missing,
        }
    }
}
impl<B: NativeBackend, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
    IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
{
    /// Borrow the actual native SQLx backend shared by this capability generation.
    pub fn backend(&self) -> &B {
        &self.backend
    }
    /// Select configuration-owned accounts without I/O or default acquisition.
    pub fn with_accounts<Next: StorageBound>(self, value: Next) -> WithAccounts<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts: Selected(Arc::new(value)),
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected accounts service; missing selection is typed.
    pub fn accounts(&self) -> Result<&<A as CapabilitySlot>::Service, IdentityError>
    where
        A: CapabilitySlot,
    {
        self.accounts.selected(CapabilityName::Accounts)
    }
    /// Select configuration-owned sessions without I/O or default acquisition.
    pub fn with_sessions<Next: StorageBound>(self, value: Next) -> WithSessions<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions: Selected(Arc::new(value)),
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected sessions service; missing selection is typed.
    pub fn sessions(&self) -> Result<&<S as CapabilitySlot>::Service, IdentityError>
    where
        S: CapabilitySlot,
    {
        self.sessions.selected(CapabilityName::Sessions)
    }
    /// Select configuration-owned keys without I/O or default acquisition.
    pub fn with_keys<Next: StorageBound>(self, value: Next) -> WithKeys<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys: Selected(Arc::new(value)),
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected keys service; missing selection is typed.
    pub fn keys(&self) -> Result<&<K as CapabilitySlot>::Service, IdentityError>
    where
        K: CapabilitySlot,
    {
        self.keys.selected(CapabilityName::Keys)
    }
    /// Select configuration-owned roles without I/O or default acquisition.
    pub fn with_roles<Next: StorageBound>(self, value: Next) -> WithRoles<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles: Selected(Arc::new(value)),
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected roles service; missing selection is typed.
    pub fn roles(&self) -> Result<&<Ro as CapabilitySlot>::Service, IdentityError>
    where
        Ro: CapabilitySlot,
    {
        self.roles.selected(CapabilityName::Roles)
    }
    /// Select configuration-owned permissions without I/O or default acquisition.
    pub fn with_permissions<Next: StorageBound>(self, value: Next) -> WithPermissions<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions: Selected(Arc::new(value)),
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected permissions service; missing selection is typed.
    pub fn permissions(&self) -> Result<&<P as CapabilitySlot>::Service, IdentityError>
    where
        P: CapabilitySlot,
    {
        self.permissions.selected(CapabilityName::Permissions)
    }
    /// Select configuration-owned challenges without I/O or default acquisition.
    pub fn with_challenges<Next: StorageBound>(self, value: Next) -> WithChallenges<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges: Selected(Arc::new(value)),
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected challenges service; missing selection is typed.
    pub fn challenges(&self) -> Result<&<Ch as CapabilitySlot>::Service, IdentityError>
    where
        Ch: CapabilitySlot,
    {
        self.challenges.selected(CapabilityName::Challenges)
    }
    /// Select configuration-owned queries without I/O or default acquisition.
    pub fn with_queries<Next: StorageBound>(self, value: Next) -> WithQueries<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries: Selected(Arc::new(value)),
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected queries service; missing selection is typed.
    pub fn queries(&self) -> Result<&<Q as CapabilitySlot>::Service, IdentityError>
    where
        Q: CapabilitySlot,
    {
        self.queries.selected(CapabilityName::Queries)
    }
    /// Select configuration-owned audit without I/O or default acquisition.
    pub fn with_audit<Next: StorageBound>(self, value: Next) -> WithAudit<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit: Selected(Arc::new(value)),
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected audit service; missing selection is typed.
    pub fn audit(&self) -> Result<&<Au as CapabilitySlot>::Service, IdentityError>
    where
        Au: CapabilitySlot,
    {
        self.audit.selected(CapabilityName::Audit)
    }
    /// Select configuration-owned consent without I/O or default acquisition.
    pub fn with_consent<Next: StorageBound>(self, value: Next) -> WithConsent<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent: Selected(Arc::new(value)),
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected consent service; missing selection is typed.
    pub fn consent(&self) -> Result<&<Co as CapabilitySlot>::Service, IdentityError>
    where
        Co: CapabilitySlot,
    {
        self.consent.selected(CapabilityName::Consent)
    }
    /// Select configuration-owned delivery without I/O or default acquisition.
    pub fn with_delivery<Next: StorageBound>(self, value: Next) -> WithDelivery<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery: Selected(Arc::new(value)),
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected delivery service; missing selection is typed.
    pub fn delivery(&self) -> Result<&<D as CapabilitySlot>::Service, IdentityError>
    where
        D: CapabilitySlot,
    {
        self.delivery.selected(CapabilityName::Delivery)
    }
    /// Select configuration-owned notifications without I/O or default acquisition.
    pub fn with_notifications<Next: StorageBound>(
        self,
        value: Next,
    ) -> WithNotifications<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications: Selected(Arc::new(value)),
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected notifications service; missing selection is typed.
    pub fn notifications(&self) -> Result<&<N as CapabilitySlot>::Service, IdentityError>
    where
        N: CapabilitySlot,
    {
        self.notifications.selected(CapabilityName::Notifications)
    }
    /// Select configuration-owned jobs without I/O or default acquisition.
    pub fn with_jobs<Next: StorageBound>(self, value: Next) -> WithJobs<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            personal_data,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs: Selected(Arc::new(value)),
            personal_data,
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected jobs service; missing selection is typed.
    pub fn jobs(&self) -> Result<&<J as CapabilitySlot>::Service, IdentityError>
    where
        J: CapabilitySlot,
    {
        self.jobs.selected(CapabilityName::Jobs)
    }
    /// Select configuration-owned personal data without I/O or default acquisition.
    pub fn with_personal_data<Next: StorageBound>(
        self,
        value: Next,
    ) -> WithPersonalData<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            native_user_queries,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data: Selected(Arc::new(value)),
            native_user_queries,
            native_role_queries,
        })
    }
    /// Borrow the selected personal data service; missing selection is typed.
    pub fn personal_data(&self) -> Result<&<Pd as CapabilitySlot>::Service, IdentityError>
    where
        Pd: CapabilitySlot,
    {
        self.personal_data.selected(CapabilityName::PersonalData)
    }
    /// Select configuration-owned native user queries without I/O or default acquisition.
    pub fn with_native_user_queries<Next: StorageBound>(
        self,
        value: Next,
    ) -> WithNativeUserQueries<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_role_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries: Selected(Arc::new(value)),
            native_role_queries,
        })
    }
    /// Borrow the selected native user queries service; missing selection is typed.
    pub fn native_user_queries(&self) -> Result<&<Uq as CapabilitySlot>::Service, IdentityError>
    where
        Uq: CapabilitySlot,
    {
        self.native_user_queries
            .selected(CapabilityName::NativeUserQueries)
    }
    /// Select configuration-owned native role queries without I/O or default acquisition.
    pub fn with_native_role_queries<Next: StorageBound>(
        self,
        value: Next,
    ) -> WithNativeRoleQueries<Self, Next> {
        if !value.belongs_to(&self.backend.configuration()) {
            return Err(IdentityError::WrongConfiguration);
        }
        let Self {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            ..
        } = self;
        Ok(IdentityStores {
            backend,
            accounts,
            sessions,
            keys,
            roles,
            permissions,
            challenges,
            queries,
            audit,
            consent,
            delivery,
            notifications,
            jobs,
            personal_data,
            native_user_queries,
            native_role_queries: Selected(Arc::new(value)),
        })
    }
    /// Borrow the selected native role queries service; missing selection is typed.
    pub fn native_role_queries(&self) -> Result<&<Rq as CapabilitySlot>::Service, IdentityError>
    where
        Rq: CapabilitySlot,
    {
        self.native_role_queries
            .selected(CapabilityName::NativeRoleQueries)
    }
}

impl<
    B: NativeBackend,
    A: Send + Sync + 'static,
    S: Send + Sync + 'static,
    K: Send + Sync + 'static,
    Ro: Send + Sync + 'static,
    P: Send + Sync + 'static,
    Ch: Send + Sync + 'static,
    Q: Send + Sync + 'static,
    Au: Send + Sync + 'static,
    Co: Send + Sync + 'static,
    D: Send + Sync + 'static,
    N: Send + Sync + 'static,
    J: Send + Sync + 'static,
    Pd: Send + Sync + 'static,
    Uq: Send + Sync + 'static,
    Rq: Send + Sync + 'static,
> StoreSelection for IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
{
    /// Borrow the mandatory native account/browser SQLx backend of this generation.
    type Backend = B;
    fn backend(&self) -> &B {
        &self.backend
    }
}
