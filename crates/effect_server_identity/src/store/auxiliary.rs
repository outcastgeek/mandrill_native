//! Typed selected auxiliary repositories beside native Identity persistence.
//!
//! These records establish storage acknowledgment only. Native protocol validators
//! establish proof; selected application callbacks establish transport/execution
//! outcomes. Every repository retains the original account-storage configuration.

use super::{
    SqlxIdentityStore,
    records::{Record, RecordCategory, RecordCodec, RecordError, RecordPolicy, SqlRecordAdapter},
};
use identity_rs::identity::models::UserId;
use std::num::{NonZeroU16, NonZeroUsize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Selected application protocol purpose, validated against its explicit key bound.
/// A purpose string is a storage discriminator and never protocol validation proof.
pub struct ChallengePurpose(String);
impl ChallengePurpose {
    /// Reject empty/oversize discriminators before persistence or allocation by SQL.
    pub fn new(value: String, max_bytes: NonZeroU16) -> Result<Self, RecordError> {
        if value.trim().is_empty() || value.len() > usize::from(max_bytes.get()) {
            return Err(RecordError::Bound);
        }
        Ok(Self(value))
    }
    /// Borrow the selected discriminator without interpreting it as authority.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Owner/purpose/expiry-bound challenges with a selected live per-partition capacity.
pub struct ChallengeRepository<A, C> {
    /// Typed native/application challenge payload codec; no HTTP view schema is reused.
    records: SqlRecordAdapter<A, C>,
    /// Caller-selected maximum simultaneous unexpired challenges for one purpose/owner.
    capacity: NonZeroUsize,
}
impl<A: Send + 'static, C: RecordCodec<A>> ChallengeRepository<A, C> {
    /// Select the real SQL backend, finite payload/page policy and live challenge quota.
    pub fn new(
        backend: SqlxIdentityStore,
        policy: RecordPolicy,
        capacity: NonZeroUsize,
        codec: C,
    ) -> Self {
        Self {
            records: SqlRecordAdapter::new(backend, RecordCategory::Challenge, policy, codec),
            capacity,
        }
    }
    /// Check this selected capability against a definition's private storage fence.
    #[must_use]
    pub fn belongs_to(&self, configuration: &crate::capabilities::StorageConfiguration) -> bool {
        self.records.belongs_to(configuration)
    }
    /// Encode the stable typed owner and purpose into an unambiguous private partition.
    fn partition(owner: UserId, purpose: &ChallengePurpose) -> String {
        format!("{owner}:{}:{}", purpose.as_str().len(), purpose.as_str())
    }
    /// Acknowledge one purpose-bound challenge only after expiry/quota/unique-ID insertion.
    pub async fn issue(
        &self,
        owner: UserId,
        purpose: &ChallengePurpose,
        id: Uuid,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
        value: A,
    ) -> Result<Uuid, RecordError> {
        self.records
            .create_bounded(
                &Self::partition(owner, purpose),
                id,
                expires_at,
                now,
                self.capacity,
                value,
            )
            .await
    }
    /// Consume exactly one matching unexpired owner/purpose/revision at the durable
    /// boundary. Protocol verification must still validate this typed native payload.
    pub async fn consume(
        &self,
        owner: UserId,
        purpose: &ChallengePurpose,
        id: Uuid,
        expected: Uuid,
        now: OffsetDateTime,
    ) -> Result<Option<Record<A>>, RecordError> {
        self.records
            .take_expiring(&Self::partition(owner, purpose), id, expected, now)
            .await
    }
}

/// Typed accepted-request domain; acceptance is separate from completed processing.
#[derive(Clone, Copy)]
pub enum RequestKind {
    /// Application-supplied transport owns actual delivery and subsequent state.
    Delivery,
    /// Application-supplied notifier owns dispatch/completion semantics.
    Notification,
    /// Application-supplied export/delete worker owns inventory and job execution.
    Job,
}
impl RequestKind {
    /// Map stable request domains to distinct persisted category partitions.
    fn category(self) -> RecordCategory {
        match self {
            Self::Delivery => RecordCategory::Delivery,
            Self::Notification => RecordCategory::Notification,
            Self::Job => RecordCategory::Job,
        }
    }
}
/// Durable acceptance receipt retaining the original moved callback payload.
/// Reconciliation uses id/revision; no payload Clone or automatic replay is implied.
pub struct AcceptedRequest<A> {
    /// Caller-selected request identity for uncertainty inspection/deduplication.
    pub id: Uuid,
    /// Actual acknowledged storage revision, required for conditional state updates.
    pub revision: Uuid,
    /// Original owned payload for a selected application transport/worker callback.
    pub value: A,
}
/// SQL-backed delivery/notification/job acceptance and conditional state repository.
pub struct RequestRepository<A, C> {
    /// Typed records in one explicit request domain and owner namespace.
    records: SqlRecordAdapter<A, C>,
    /// Exact selected request domain; cannot be rebound into another D17 slot.
    kind: RequestKind,
}
impl<A: Send + 'static, C: RecordCodec<A>> RequestRepository<A, C> {
    /// Select the request domain and private payload codec without performing work.
    pub fn new(
        backend: SqlxIdentityStore,
        kind: RequestKind,
        policy: RecordPolicy,
        codec: C,
    ) -> Self {
        Self {
            records: SqlRecordAdapter::new(backend, kind.category(), policy, codec),
            kind,
        }
    }
    /// Borrow the actual selected domain without inferring it from a generic payload.
    #[must_use]
    pub fn kind(&self) -> RequestKind {
        self.kind
    }
    /// Reject selection in a definition using another account-storage configuration.
    #[must_use]
    pub fn belongs_to(&self, configuration: &crate::capabilities::StorageConfiguration) -> bool {
        self.records.belongs_to(configuration)
    }
    /// Acknowledge durable acceptance before a transport/worker receives the payload.
    pub async fn accept(
        &self,
        owner: &str,
        id: Uuid,
        expires_at: Option<OffsetDateTime>,
        value: A,
    ) -> Result<AcceptedRequest<A>, RecordError> {
        let (revision, value) = self
            .records
            .create_returned(owner, id, expires_at, value)
            .await?;
        Ok(AcceptedRequest {
            id,
            revision,
            value,
        })
    }
    /// Inspect acknowledged state after uncertainty; absence is distinct from outage.
    pub async fn find(
        &self,
        owner: &str,
        id: Uuid,
        now: OffsetDateTime,
    ) -> Result<Option<Record<A>>, RecordError> {
        self.records.find(owner, id, now).await
    }
    /// Record caller-defined status only at the exact prior accepted revision.
    /// A callback result's meaning belongs to the selected application service.
    pub async fn update(
        &self,
        owner: &str,
        id: Uuid,
        expected: Uuid,
        expires_at: Option<OffsetDateTime>,
        value: A,
    ) -> Result<Uuid, RecordError> {
        self.records
            .replace(owner, id, expected, expires_at, value)
            .await
    }
}
