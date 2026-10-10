//! Provider-neutral contracts for selected extension persistence capabilities.
//! Native crypto/tickets stay native types; reads distinguish absence from failure,
//! and revisions describe acknowledged writes rather than authentication proof.

use crate::{
    IdentityError,
    capabilities::{CapabilitySlot, IdentityStores, StorageBound, StoreSelection},
};
use identity_rs::identity::{models::UserId, ticket::AuthenticationTicket};
use std::future::Future;
use time::OffsetDateTime;
use uuid::Uuid;

/// Acknowledged application document, separate from account security serialization.
pub struct VersionedDocument<A> {
    /// Stable application key selected by the owning feature.
    pub id: Uuid,
    /// Actual acknowledged optimistic revision.
    pub revision: Uuid,
    /// Explicit application expiry; None deliberately means nonexpiring.
    pub expires_at: Option<OffsetDateTime>,
    /// Original decoded application payload, never required to implement Clone.
    pub value: A,
}
/// Configuration-owned consent/preferences persistence.
pub trait ConsentStore: StorageBound {
    /// Actual application-selected private document representation.
    type Document: Send + 'static;
    /// Read one owner's selected preference; failures cannot become default state.
    fn read<'a>(
        &'a self,
        owner: UserId,
        key: Uuid,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<VersionedDocument<Self::Document>>, IdentityError>> + Send + 'a;
    /// Create or replace exactly the supplied revision and acknowledge before returning.
    fn write<'a>(
        &'a self,
        owner: UserId,
        key: Uuid,
        expected: Option<Uuid>,
        expires_at: Option<OffsetDateTime>,
        value: Self::Document,
    ) -> impl Future<Output = Result<Uuid, IdentityError>> + Send + 'a;
}
/// Configuration-owned stored mode; independent of native cookie/stamp mode.
pub trait StoredSessions: StorageBound {
    /// Native ticket protection precedes actual persistence acknowledgment.
    fn store<'a>(
        &'a self,
        owner: UserId,
        id: Uuid,
        expires_at: OffsetDateTime,
        ticket: &'a AuthenticationTicket,
    ) -> impl Future<Output = Result<Uuid, IdentityError>> + Send + 'a;
    /// Native protection/expiry validation follows the current unrevoked durable read.
    fn load<'a>(
        &'a self,
        owner: UserId,
        id: Uuid,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<AuthenticationTicket>, IdentityError>> + Send + 'a;
    /// Revoke exactly an acknowledged owner/revision, preserving uncertainty.
    fn revoke<'a>(
        &'a self,
        owner: UserId,
        id: Uuid,
        expected: Uuid,
    ) -> impl Future<Output = Result<bool, IdentityError>> + Send + 'a;
}
/// Security snapshot metadata used to enforce exact native owner/version binding.
/// This is an internal persistence contract and never a public account-view codec.
pub trait AccountSnapshotMetadata: Send + 'static {
    /// Native validated metadata associated with the complete snapshot.
    fn metadata(&self) -> &identity_rs::identity::models::IdentityUser;
}
/// One internally consistent security snapshot and its aggregate mutation revision.
pub struct VersionedSnapshot<A, V> {
    /// Complete owned native/application security representation.
    pub snapshot: A,
    /// Actual aggregate CAS revision, including granular child changes.
    pub revision: V,
}
/// Optional complete account aggregate and its exact selected provider revision.
/// Absence remains distinct from an IdentityError in the surrounding result.
pub type AccountSnapshot<S> =
    Option<VersionedSnapshot<<S as AccountSnapshots>::Snapshot, <S as AccountSnapshots>::Revision>>;

/// Provider-neutral atomic account snapshot persistence, separate from granular managers.
pub trait AccountSnapshots: StorageBound {
    /// Actual private security aggregate, with no blanket Clone requirement.
    type Snapshot: AccountSnapshotMetadata;
    /// Actual provider acknowledgment/CAS representation.
    type Revision: Send + 'static;
    /// Read one consistent complete aggregate, preserving full provider outcomes.
    fn load<'a>(
        &'a self,
        id: UserId,
    ) -> impl Future<Output = Result<AccountSnapshot<Self>, IdentityError>> + Send + 'a;
    /// Replace exactly an expected full aggregate revision atomically.
    fn replace<'a>(
        &'a self,
        expected: Self::Revision,
        snapshot: Self::Snapshot,
    ) -> impl Future<Output = Result<Self::Revision, IdentityError>> + Send + 'a;
    /// Delete exactly one expected complete aggregate; absence/conflict remains failure.
    fn delete<'a>(
        &'a self,
        id: UserId,
        expected: Self::Revision,
    ) -> impl Future<Output = Result<(), IdentityError>> + Send + 'a;
}
/// Atomic account capability access on the exact installed D17 generation.
pub trait AccountSelection: StoreSelection {
    /// Actual selected aggregate provider.
    type Accounts: AccountSnapshots;
    /// Missing atomic capability does not silently substitute granular operations.
    fn accounts_store(&self) -> Result<&Self::Accounts, IdentityError>;
}
/// Request persistence domain, independently selected from any transport/executor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RequestDomain {
    /// Acknowledged delivery acceptance/state.
    Delivery,
    /// Acknowledged notification acceptance/state.
    Notification,
    /// Acknowledged export/delete job request/state.
    Job,
}
/// Durable acceptance with the original moved callback payload.
pub struct AcceptedDocument<A> {
    /// Explicit reconciliation/deduplication request identity.
    pub id: Uuid,
    /// Actual acknowledged persistence revision.
    pub revision: Uuid,
    /// Original request moved to application execution without payload cloning.
    pub value: A,
}
/// Configuration-owned durable request/state service; acceptance is not completion.
pub trait DurableRequests: StorageBound {
    /// Actual application request/state codec representation.
    type Request: Send + 'static;
    /// Exact selected persistence partition, preventing accidental cross-slot wiring.
    fn domain(&self) -> RequestDomain;
    /// Persist before an application-supplied callback may receive the request.
    fn accept<'a>(
        &'a self,
        owner: UserId,
        id: Uuid,
        expires_at: Option<OffsetDateTime>,
        value: Self::Request,
    ) -> impl Future<Output = Result<AcceptedDocument<Self::Request>, IdentityError>> + Send + 'a;
    /// Inspect durable state after uncertain acknowledgment or callback failure.
    fn find<'a>(
        &'a self,
        owner: UserId,
        id: Uuid,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<VersionedDocument<Self::Request>>, IdentityError>> + Send + 'a;
    /// Acknowledge a caller-defined state transition at its exact previous revision.
    fn update<'a>(
        &'a self,
        owner: UserId,
        id: Uuid,
        expected: Uuid,
        expires_at: Option<OffsetDateTime>,
        value: Self::Request,
    ) -> impl Future<Output = Result<Uuid, IdentityError>> + Send + 'a;
}
/// Access to the exact installed delivery capability.
pub trait DeliverySelection: StoreSelection {
    /// Actual selected durable acceptance/state service.
    type Delivery: DurableRequests;
    /// Missing selection remains a typed result.
    fn delivery_store(&self) -> Result<&Self::Delivery, IdentityError>;
}
/// Access to the exact installed notification capability.
pub trait NotificationSelection: StoreSelection {
    /// Actual selected durable acceptance/state service.
    type Notifications: DurableRequests;
    /// Missing selection remains a typed result.
    fn notification_store(&self) -> Result<&Self::Notifications, IdentityError>;
}
/// Access to the exact installed export/delete request capability.
pub trait JobSelection: StoreSelection {
    /// Actual selected durable request/state service; no job engine is implied.
    type Jobs: DurableRequests;
    /// Missing selection remains a typed result.
    fn job_store(&self) -> Result<&Self::Jobs, IdentityError>;
}
impl<B: crate::capabilities::NativeBackend, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
    DeliverySelection for IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
where
    Self: StoreSelection,
    D: CapabilitySlot,
    D::Service: DurableRequests,
{
    type Delivery = D::Service;
    /// Borrow the actual durable delivery acceptance service or preserve typed absence.
    fn delivery_store(&self) -> Result<&Self::Delivery, IdentityError> {
        self.delivery()
    }
}
impl<B: crate::capabilities::NativeBackend, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
    NotificationSelection for IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
where
    Self: StoreSelection,
    N: CapabilitySlot,
    N::Service: DurableRequests,
{
    type Notifications = N::Service;
    /// Borrow the actual durable notification acceptance service or preserve typed absence.
    fn notification_store(&self) -> Result<&Self::Notifications, IdentityError> {
        self.notifications()
    }
}
impl<B: crate::capabilities::NativeBackend, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
    JobSelection for IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
where
    Self: StoreSelection,
    J: CapabilitySlot,
    J::Service: DurableRequests,
{
    type Jobs = J::Service;
    /// Borrow the actual durable request/state capability or preserve typed absence.
    fn job_store(&self) -> Result<&Self::Jobs, IdentityError> {
        self.jobs()
    }
}
impl<B: crate::capabilities::NativeBackend, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
    AccountSelection for IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
where
    Self: StoreSelection,
    A: CapabilitySlot,
    A::Service: AccountSnapshots,
{
    type Accounts = A::Service;
    /// Borrow the exact selected atomic account snapshot service or preserve typed absence.
    fn accounts_store(&self) -> Result<&Self::Accounts, IdentityError> {
        self.accounts()
    }
}
/// Consent capability access on a typed D17 bundle.
pub trait ConsentSelection: StoreSelection {
    /// Actual selected application persistence implementation.
    type Consent: ConsentStore;
    /// Missing selection remains the precise D17 expected result.
    fn consent_store(&self) -> Result<&Self::Consent, IdentityError>;
}
/// Stored-session capability access on a typed D17 bundle.
pub trait SessionSelection: StoreSelection {
    /// Actual selected stored native ticket implementation.
    type Sessions: StoredSessions;
    /// Native cookie mode cannot masquerade as selected stored mode.
    fn sessions_store(&self) -> Result<&Self::Sessions, IdentityError>;
}
impl<B: crate::capabilities::NativeBackend, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
    ConsentSelection for IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
where
    Self: StoreSelection,
    Co: CapabilitySlot,
    Co::Service: ConsentStore,
{
    type Consent = Co::Service;
    /// Borrow the configuration-owned consent service or preserve typed absence.
    fn consent_store(&self) -> Result<&Self::Consent, IdentityError> {
        self.consent()
    }
}
impl<B: crate::capabilities::NativeBackend, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
    SessionSelection for IdentityStores<B, A, S, K, Ro, P, Ch, Q, Au, Co, D, N, J, Pd, Uq, Rq>
where
    Self: StoreSelection,
    S: CapabilitySlot,
    S::Service: StoredSessions,
{
    type Sessions = S::Service;
    fn sessions_store(&self) -> Result<&Self::Sessions, IdentityError> {
        self.sessions()
    }
}
#[cfg(feature = "sqlx-turso")]
impl<A: Send + 'static, C: crate::store::records::RecordCodec<A>> ConsentStore
    for crate::store::records::SqlRecordAdapter<A, C>
{
    type Document = A;
    /// Read the exact current owner/key through the selected adapter; absence remains distinct from failure.
    async fn read(
        &self,
        owner: UserId,
        key: Uuid,
        now: OffsetDateTime,
    ) -> Result<Option<VersionedDocument<A>>, IdentityError> {
        if self.category() != crate::store::records::RecordCategory::Consent {
            return Err(IdentityError::Configuration("consent record category"));
        }
        Ok(self
            .find(&owner.to_string(), key, now)
            .await?
            .map(|record| VersionedDocument {
                id: record.id,
                revision: record.revision,
                expires_at: record.expires_at,
                value: record.value,
            }))
    }
    /// Acknowledge the selected exact-revision create/update without replacing provider diagnostics.
    async fn write(
        &self,
        owner: UserId,
        key: Uuid,
        expected: Option<Uuid>,
        expires_at: Option<OffsetDateTime>,
        value: A,
    ) -> Result<Uuid, IdentityError> {
        if self.category() != crate::store::records::RecordCategory::Consent {
            return Err(IdentityError::Configuration("consent record category"));
        }
        let owner = owner.to_string();
        match expected {
            Some(expected) => self
                .replace(&owner, key, expected, expires_at, value)
                .await
                .map_err(IdentityError::from),
            None => self
                .create(&owner, key, expires_at, value)
                .await
                .map_err(IdentityError::from),
        }
    }
}
#[cfg(feature = "sqlx-turso")]
impl<T: identity_rs::identity::ticket::AuthenticationTicketFormat + 'static> StoredSessions
    for crate::store::components::SessionRepository<T>
{
    /// Protect the native ticket and acknowledge its selected owner/session persistence.
    async fn store<'a>(
        &'a self,
        owner: UserId,
        id: Uuid,
        expires_at: OffsetDateTime,
        ticket: &'a AuthenticationTicket,
    ) -> Result<Uuid, IdentityError> {
        self.store(&owner.to_string(), id, expires_at, ticket)
            .await
            .map_err(IdentityError::from)
    }
    /// Load the exact current owner/session or aggregate while preserving provider and validation outcomes.
    async fn load(
        &self,
        owner: UserId,
        id: Uuid,
        now: OffsetDateTime,
    ) -> Result<Option<AuthenticationTicket>, IdentityError> {
        self.load(&owner.to_string(), id, now)
            .await
            .map_err(IdentityError::from)
    }
    /// Acknowledge only the exact owner/session revision; uncertainty is not reported as absence.
    async fn revoke(&self, owner: UserId, id: Uuid, expected: Uuid) -> Result<bool, IdentityError> {
        self.revoke(&owner.to_string(), id, expected)
            .await
            .map_err(IdentityError::from)
    }
}
#[cfg(feature = "sqlx-turso")]
impl<A: Send + 'static, C: crate::store::records::RecordCodec<A>> DurableRequests
    for crate::store::auxiliary::RequestRepository<A, C>
{
    type Request = A;
    /// Return the originally selected durable request partition without inferring it from payloads.
    fn domain(&self) -> RequestDomain {
        match self.kind() {
            crate::store::auxiliary::RequestKind::Delivery => RequestDomain::Delivery,
            crate::store::auxiliary::RequestKind::Notification => RequestDomain::Notification,
            crate::store::auxiliary::RequestKind::Job => RequestDomain::Job,
        }
    }
    /// Acknowledge durable acceptance before moving the original payload to application execution.
    async fn accept(
        &self,
        owner: UserId,
        id: Uuid,
        expires_at: Option<OffsetDateTime>,
        value: A,
    ) -> Result<AcceptedDocument<A>, IdentityError> {
        let accepted = self
            .accept(&owner.to_string(), id, expires_at, value)
            .await?;
        Ok(AcceptedDocument {
            id: accepted.id,
            revision: accepted.revision,
            value: accepted.value,
        })
    }
    /// Read current durable state, distinguishing absent or expired records from provider failure.
    async fn find(
        &self,
        owner: UserId,
        id: Uuid,
        now: OffsetDateTime,
    ) -> Result<Option<VersionedDocument<A>>, IdentityError> {
        Ok(self
            .find(&owner.to_string(), id, now)
            .await?
            .map(|record| VersionedDocument {
                id: record.id,
                revision: record.revision,
                expires_at: record.expires_at,
                value: record.value,
            }))
    }
    /// Acknowledge the exact previous revision through the original selected request partition.
    async fn update(
        &self,
        owner: UserId,
        id: Uuid,
        expected: Uuid,
        expires_at: Option<OffsetDateTime>,
        value: A,
    ) -> Result<Uuid, IdentityError> {
        self.update(&owner.to_string(), id, expected, expires_at, value)
            .await
            .map_err(IdentityError::from)
    }
}
