//! SQL-backed selected extension records with typed codecs and acknowledgment.
//!
//! Session tickets/key documents/consent/delivery/notifications/jobs are stored
//! independently unless a caller explicitly supplies a coupled operation. Durable
//! acceptance is not delivery completion or a job execution engine. Every adapter
//! is fenced to the selected account-store configuration.

mod prepared;

pub use prepared::{PreparedRecordCodec, PreparedRecordMemory, RecordWriter};

use super::SqlxIdentityStore;
use crate::{effect_future, effect_uninterruptible};
use effect::{Effect, memory::MemoryError};
use effect_sql::{NativeOperation, SqlError, kernel::NativeTransactionReservation, sql};
pub(super) use prepared::RecordBuffer;
use prepared::{PayloadBinding, PreparedCodec};
use sqlx::{AssertSqlSafe, Row};
use std::{
    marker::PhantomData,
    num::{NonZeroU16, NonZeroUsize},
    sync::Arc,
};
use time::OffsetDateTime;
use uuid::Uuid;

/// Versioned storage category identities; these are schema contracts rather than
/// hidden policy strings. Public account views have separate application codecs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordCategory {
    /// Native serialized stored ticket/session bytes with expiry/revocation.
    Session,
    /// Validated native Data Protection XML key documents.
    Key,
    /// Application permission/grant decision records; writing one does not authorize.
    Permission,
    /// Purpose/owner/expiry-bound generic one-use challenge records.
    Challenge,
    /// Application consent/preference state.
    Consent,
    /// Durable transport acceptance/state; actual transport is application-owned.
    Delivery,
    /// Durable notification state, independently acknowledged.
    Notification,
    /// Acknowledged export/delete request, without a job execution engine.
    Job,
    /// Application-selected personal-data hook state, not an invented inventory.
    PersonalData,
}
impl RecordCategory {
    /// Stable SQL discriminator in the v1 schema.
    fn key(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Key => "key",
            Self::Permission => "permission",
            Self::Challenge => "challenge",
            Self::Consent => "consent",
            Self::Delivery => "delivery",
            Self::Notification => "notification",
            Self::Job => "job",
            Self::PersonalData => "personal-data",
        }
    }
}

/// Explicit encoded-payload and page limits; all values carry native units.
#[derive(Clone, Copy)]
pub struct RecordPolicy {
    /// Maximum encoded bytes for one selected private security record.
    pub max_bytes: NonZeroUsize,
    /// Maximum records requested/allocated in a single incremental page.
    pub page_limit: NonZeroU16,
}
/// Bind the selected byte ceiling before returning stored payload cells. Oversize
/// corrupt payloads become NULL/decode failure instead of allocating their bytes.
#[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
fn max_bytes_sql(policy: RecordPolicy) -> Result<i64, SqlError> {
    i64::try_from(policy.max_bytes.get())
        .map_err(|_| SqlError::Binding(Box::new(RecordError::Bound)))
}
/// Preserve native expiry precision and reject instants outside SQLite's integer
/// nanosecond representation instead of silently truncating or changing expiry.
#[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
fn utc_nanos(value: OffsetDateTime) -> Result<i64, RecordError> {
    i64::try_from(value.unix_timestamp_nanos()).map_err(|_| RecordError::Bound)
}

/// Typed private persistence codec. It validates decoded data and refuses unknown
/// versions; neither HTTP views nor serde::Value are implicit persistence codecs.
pub trait RecordCodec<A>: Send + Sync + 'static {
    /// Stable version identifying this private record representation.
    fn version(&self) -> NonZeroU16;
    /// Encode by borrow into the provider's necessary owned byte representation.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    fn encode(&self, value: &A) -> Result<Vec<u8>, RecordError>;
    /// Validate and decode exactly the selected acknowledged version.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    fn decode(&self, version: NonZeroU16, bytes: &[u8]) -> Result<A, RecordError>;
}

/// Record failures retain native SQL diagnostics and uncertain acknowledgment.
#[derive(Debug)]
pub enum RecordError {
    /// Selected handle does not belong to the definition's storage configuration.
    WrongConfiguration,
    /// Caller policy/input would exceed the selected byte/page bound.
    Bound,
    /// Persisted record version or representation failed validation.
    Corrupt,
    /// Expected revision did not match or the record is missing.
    Conflict,
    /// Codec rejected the private representation with a safe application reason.
    Codec(String),
    /// Native SQL primary/cleanup/commit uncertainty tree; do not blindly retry.
    Sql(SqlError),
}
impl std::fmt::Display for RecordError {
    /// Safe category display excludes record bytes, keys and SQL diagnostics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::WrongConfiguration => "identity storage configuration mismatch",
            Self::Bound => "identity record bound exceeded",
            Self::Corrupt => "corrupt identity record",
            Self::Conflict => "identity record revision conflict",
            Self::Codec(_) => "identity record codec failed",
            Self::Sql(_) => "identity record persistence failed",
        })
    }
}
impl std::error::Error for RecordError {
    /// Trusted callers can inspect richer SQL categories without exposing them publicly.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sql(error) => Some(error),
            Self::WrongConfiguration
            | Self::Bound
            | Self::Corrupt
            | Self::Conflict
            | Self::Codec(_) => None,
        }
    }
}
impl From<SqlError> for RecordError {
    /// Preserve acknowledgment uncertainty and cleanup as the same typed SQL error tree.
    fn from(value: SqlError) -> Self {
        Self::Sql(value)
    }
}
impl RecordError {
    /// Restore native defects/interruption/ordered finalization at an Effect
    /// boundary; ordinary codec/conflict failures stay in the expected channel.
    pub fn into_exit<A>(self) -> effect::Exit<A, Self> {
        match self {
            Self::Sql(error) => error.into_exit().map_error(Self::Sql),
            error @ (Self::WrongConfiguration
            | Self::Bound
            | Self::Corrupt
            | Self::Conflict
            | Self::Codec(_)) => effect::Exit::Fail(error),
        }
    }
}

/// Acknowledged owned value with its explicit conditional-mutation revision.
pub struct Record<A> {
    /// Stable record identity, independent of owner and storage category.
    pub id: Uuid,
    /// Committed revision to use for the next conditional mutation.
    pub revision: Uuid,
    /// Optional expiry; None is deliberately nonexpiring caller policy.
    pub expires_at: Option<OffsetDateTime>,
    /// Validated native/application value moved out of its codec.
    pub value: A,
}
/// A finite page retains no SQL transaction during application/browser consumption.
pub struct RecordPage<A> {
    /// Strictly ascending record IDs returned under the selected finite limit.
    pub records: Vec<Record<A>>,
    /// Exclusive continuation if more records exist at this query boundary.
    pub next: Option<Uuid>,
}

pub(super) struct RecordWriteAdmission<'a> {
    buffer: Option<RecordBuffer>,
    transaction: Option<NativeTransactionReservation<'a>>,
}

impl<'a> RecordWriteAdmission<'a> {
    #[expect(
        clippy::result_large_err,
        reason = "preserves the established typed record ABI"
    )]
    fn reserve(
        backend: &'a SqlxIdentityStore,
        buffer: Option<RecordBuffer>,
    ) -> Result<Self, RecordError> {
        let transaction = buffer
            .as_ref()
            .map(|_| sql! { reserve_transaction_operation; kernel: backend.kernel, })
            .transpose()?;
        Ok(Self {
            buffer,
            transaction,
        })
    }
}

/// Selected SQL adapter for exactly one storage category and typed private codec.
pub struct SqlRecordAdapter<A, C> {
    /// Actual account backend, shared as immutable service configuration.
    backend: SqlxIdentityStore,
    /// Stable schema discriminator, never inferred from a type's debug name.
    category: RecordCategory,
    /// Byte/page policy selected at installation.
    policy: RecordPolicy,
    /// Typed codec generation shared without copying record payloads.
    codec: Arc<C>,
    prepared: Option<Arc<PreparedCodec<A, C>>>,
    /// Application payload remains statically typed; it is not retained in the handle.
    value: PhantomData<fn() -> A>,
}
impl<A, C> Clone for SqlRecordAdapter<A, C> {
    /// Clone shares configuration/codec handles, with no Clone requirement on A.
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            category: self.category,
            policy: self.policy,
            codec: self.codec.clone(),
            prepared: self.prepared.clone(),
            value: PhantomData,
        }
    }
}
impl<A: Send + 'static, C: RecordCodec<A>> SqlRecordAdapter<A, C> {
    /// Select one application codec/policy against the existing authoritative backend.
    pub fn new(
        backend: SqlxIdentityStore,
        category: RecordCategory,
        policy: RecordPolicy,
        codec: C,
    ) -> Self {
        Self {
            backend,
            category,
            policy,
            codec: Arc::new(codec),
            prepared: None,
            value: PhantomData,
        }
    }
    /// Select preadmitted record bytes for every read and write on this adapter.
    /// Exhaustion uses the existing record bound failure before codec invocation.
    pub fn with_prepared_memory(
        mut self,
        memory: Arc<PreparedRecordMemory>,
    ) -> Result<Self, MemoryError>
    where
        C: PreparedRecordCodec<A>,
    {
        self.prepared = Some(Arc::new(PreparedCodec::new(memory, self.policy)?));
        Ok(self)
    }
    fn prepared_codec(&self) -> Option<&PreparedCodec<A, C>> {
        self.prepared.as_deref()
    }
    #[expect(
        clippy::result_large_err,
        reason = "preserves the established typed record ABI"
    )]
    pub(super) fn reserve_buffer(&self) -> Result<Option<RecordBuffer>, RecordError> {
        prepared::reserve(self.prepared_codec()).map_err(|_| RecordError::Bound)
    }
    #[expect(
        clippy::result_large_err,
        reason = "preserves the established typed record ABI"
    )]
    pub(super) fn reserve_write(&self) -> Result<RecordWriteAdmission<'_>, RecordError> {
        self.reserve_write_with(self.reserve_buffer()?)
    }
    #[expect(
        clippy::result_large_err,
        reason = "preserves the established typed record ABI"
    )]
    pub(super) fn reserve_write_with(
        &self,
        buffer: Option<RecordBuffer>,
    ) -> Result<RecordWriteAdmission<'_>, RecordError> {
        RecordWriteAdmission::reserve(&self.backend, buffer)
    }
    #[expect(
        clippy::result_large_err,
        reason = "preserves the established typed record ABI"
    )]
    pub(super) fn check_selected_size(&self, bytes: usize) -> Result<(), RecordError> {
        if self.prepared.is_some() && bytes > self.policy.max_bytes.get() {
            return Err(RecordError::Bound);
        }
        Ok(())
    }
    /// Check the handle's private configuration identity before a selected workflow.
    #[must_use]
    pub fn belongs_to(&self, configuration: &crate::capabilities::StorageConfiguration) -> bool {
        Arc::ptr_eq(&self.backend.identity, &configuration.identity)
    }
    /// Exact selected private persistence domain; consent/request services verify it.
    #[must_use]
    pub fn category(&self) -> RecordCategory {
        self.category
    }
    /// Read current unexpired state; storage/codec errors are never absent state.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub async fn find(
        &self,
        owner: &str,
        id: Uuid,
        now: OffsetDateTime,
    ) -> Result<Option<Record<A>>, RecordError> {
        let mut buffer = self.reserve_buffer()?;
        self.find_reserved(owner, id, now, buffer.as_mut()).await
    }
    #[expect(
        clippy::result_large_err,
        reason = "preserves the established typed record ABI"
    )]
    pub(super) async fn find_reserved(
        &self,
        owner: &str,
        id: Uuid,
        now: OffsetDateTime,
        buffer: Option<&mut RecordBuffer>,
    ) -> Result<Option<Record<A>>, RecordError> {
        /// Finite borrowed native read executed under the shared connection owner.
        struct Find<'a, A, C> {
            /// Selected typed adapter and codec generation.
            adapter: &'a SqlRecordAdapter<A, C>,
            /// Owner partition borrowed until query settlement.
            owner: &'a str,
            /// Exact record identity, independent of authorization.
            id: Uuid,
            /// Caller-selected UTC value for expiry.
            now: OffsetDateTime,
            buffer: Option<&'a mut RecordBuffer>,
        }
        impl<A: Send + 'static, C: RecordCodec<A>> NativeOperation<Option<Record<A>>> for Find<'_, A, C> {
            /// Decode owned native cells under the kernel lease, retaining typed failures.
            #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
            async fn run<'a>(
                self,
                connection: &'a mut sqlx::TursoConnection,
            ) -> Result<Option<Record<A>>, SqlError>
            where
                Self: 'a,
            {
                let row=sqlx::query(AssertSqlSafe("SELECT record_id,revision,codec_version,expires_at,CASE WHEN length(payload)<=? THEN payload ELSE NULL END AS payload FROM effect_identity_records WHERE category=? AND owner=? AND record_id=? AND (expires_at IS NULL OR expires_at>?)"))
                    .bind(max_bytes_sql(self.adapter.policy)?).bind(self.adapter.category.key()).bind(self.owner).bind(self.id.to_string()).bind(utc_nanos(self.now).map_err(|error|SqlError::Binding(Box::new(error)))?).fetch_optional(connection).await?;
                row.map(|row| self.adapter.decode(row, self.buffer))
                    .transpose()
                    .map_err(|error| SqlError::Binding(Box::new(error)))
            }
        }
        sql! {
            connection_operation; kernel: self.backend.kernel, operation: Find {
                adapter: self,
                owner,
                id,
                now,
                buffer,
            },
        }
        .await
        .map_err(record_sql_error)
    }
    /// Create a fresh record atomically; uniqueness/commit uncertainty propagate.
    /// This returns the committed revision only after native settlement.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub async fn create(
        &self,
        owner: &str,
        id: Uuid,
        expires_at: Option<OffsetDateTime>,
        value: A,
    ) -> Result<Uuid, RecordError> {
        self.write_reserved(
            owner,
            id,
            (None, expires_at),
            value,
            self.reserve_write()?,
            None,
        )
        .await
        .map(|(revision, _)| revision)
    }
    /// Replace exactly the expected revision. This does not imply a caller's
    /// surrounding account/session workflow is a single SQL transaction.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub async fn replace(
        &self,
        owner: &str,
        id: Uuid,
        expected: Uuid,
        expires_at: Option<OffsetDateTime>,
        value: A,
    ) -> Result<Uuid, RecordError> {
        self.write_reserved(
            owner,
            id,
            (Some(expected), expires_at),
            value,
            self.reserve_write()?,
            None,
        )
        .await
        .map(|(revision, _)| revision)
    }
    /// Acknowledge acceptance while returning the original moved payload for an
    /// explicitly selected application transport/job callback; no Clone is needed.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub async fn create_returned(
        &self,
        owner: &str,
        id: Uuid,
        expires_at: Option<OffsetDateTime>,
        value: A,
    ) -> Result<(Uuid, A), RecordError> {
        self.write_reserved(
            owner,
            id,
            (None, expires_at),
            value,
            self.reserve_write()?,
            None,
        )
        .await
    }
    /// Create one expiring record under a caller-selected live owner quota. Cleanup,
    /// quota check and insertion share one acknowledged transaction across clients.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub async fn create_bounded(
        &self,
        owner: &str,
        id: Uuid,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
        capacity: NonZeroUsize,
        value: A,
    ) -> Result<Uuid, RecordError> {
        if expires_at <= now {
            return Err(RecordError::Bound);
        }
        let capacity = i64::try_from(capacity.get()).map_err(|_| RecordError::Bound)?;
        let now = utc_nanos(now)?;
        self.write_reserved(
            owner,
            id,
            (None, Some(expires_at)),
            value,
            self.reserve_write()?,
            Some((capacity, now)),
        )
        .await
        .map(|(revision, _)| revision)
    }
    /// Encode and move one admitted operation directly into the established SQL settlement path.
    #[expect(
        clippy::result_large_err,
        reason = "preserves the established typed record ABI"
    )]
    pub(super) async fn write_reserved(
        &self,
        owner: &str,
        id: Uuid,
        (expected, expires_at): (Option<Uuid>, Option<OffsetDateTime>),
        value: A,
        admission: RecordWriteAdmission<'_>,
        capacity: Option<(i64, i64)>,
    ) -> Result<(Uuid, A), RecordError> {
        let mut payload = prepared::encode(
            &*self.codec,
            self.prepared_codec(),
            self.policy,
            &value,
            admission.buffer,
        )?;
        let version = self.codec.version().get();
        let expires_at = expires_at.map(utc_nanos).transpose()?;
        /// One borrowed-owner/moved-payload invocation; SQL's cleanup implementation owns settlement.
        struct Mutation<'a> {
            /// Category/version are stable codec/schema metadata.
            category: &'static str,
            /// Owner is borrowed until acknowledged completion, not copied for a reusable Effect.
            owner: &'a str,
            /// Record identity and optional expected committed revision.
            id: Uuid,
            /// None means insert; Some means compare-and-replace.
            expected: Option<Uuid>,
            /// Explicit caller expiry in epoch nanoseconds, matching the private schema.
            expires_at: Option<i64>,
            /// Codec generation version, independent of public-view schemas.
            version: u16,
            /// Encoded native/provider ownership conversion, moved once.
            payload: PayloadBinding<'a>,
            /// Optional live-record quota and explicit UTC nanosecond instant.
            capacity: Option<(i64, i64)>,
        }
        impl NativeOperation<Uuid> for Mutation<'_> {
            /// Commit one conditional statement through the common operation-owned transaction.
            async fn run<'a>(
                self,
                connection: &'a mut sqlx::TursoConnection,
            ) -> Result<Uuid, SqlError>
            where
                Self: 'a,
            {
                let revision = Uuid::new_v4();
                let result = if let Some(expected) = self.expected {
                    self.payload.bind(sqlx::query(AssertSqlSafe("UPDATE effect_identity_records SET revision=?,codec_version=?,expires_at=?,payload=? WHERE category=? AND owner=? AND record_id=? AND revision=?"))
                            .bind(revision.to_string()).bind(i64::from(self.version)).bind(self.expires_at)).bind(self.category).bind(self.owner).bind(self.id.to_string()).bind(expected.to_string()).execute(connection).await?
                } else if let Some((capacity, now)) = self.capacity {
                    sqlx::query(AssertSqlSafe("DELETE FROM effect_identity_records WHERE category=? AND owner=? AND expires_at<=?"))
                            .bind(self.category).bind(self.owner).bind(now).execute(&mut *connection).await?;
                    self.payload.bind(sqlx::query(AssertSqlSafe("INSERT INTO effect_identity_records(category,owner,record_id,revision,codec_version,expires_at,payload) SELECT ?,?,?,?,?,?,? WHERE (SELECT COUNT(*) FROM effect_identity_records WHERE category=? AND owner=?)<?"))
                            .bind(self.category).bind(self.owner).bind(self.id.to_string()).bind(revision.to_string()).bind(i64::from(self.version)).bind(self.expires_at)).bind(self.category).bind(self.owner).bind(capacity).execute(connection).await?
                } else {
                    self.payload.bind(sqlx::query(AssertSqlSafe("INSERT INTO effect_identity_records(category,owner,record_id,revision,codec_version,expires_at,payload) VALUES(?,?,?,?,?,?,?)"))
                            .bind(self.category).bind(self.owner).bind(self.id.to_string()).bind(revision.to_string()).bind(i64::from(self.version)).bind(self.expires_at)).execute(connection).await?
                };
                if result.rows_affected() != 1 {
                    return Err(SqlError::Binding(Box::new(RecordError::Conflict)));
                }
                Ok(revision)
            }
        }
        let operation = Mutation {
            category: self.category.key(),
            owner,
            id,
            expected,
            expires_at,
            version,
            payload: payload.binding(),
            capacity,
        };
        let result = match admission.transaction {
            Some(transaction) => sql! { run_transaction_operation; reservation: transaction, operation: operation, }.await,
            None => sql! { transaction_operation; kernel: self.backend.kernel, operation: operation, }.await,
        };
        let revision = result.map_err(record_sql_error)?;
        Ok((revision, value))
    }
    /// Consume one matching purpose/owner-bound unexpired record at one durable
    /// linearization point. The record ID supplied by the selected protocol binds purpose.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub async fn take(
        &self,
        owner: &str,
        id: Uuid,
        expected: Uuid,
        now: OffsetDateTime,
    ) -> Result<Option<Record<A>>, RecordError> {
        self.take_inner(owner, id, expected, now, false).await
    }
    /// Consume an explicitly expiring protocol record. A null persisted expiry is
    /// corruption, never a permanent challenge; validation occurs before delete.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub async fn take_expiring(
        &self,
        owner: &str,
        id: Uuid,
        expected: Uuid,
        now: OffsetDateTime,
    ) -> Result<Option<Record<A>>, RecordError> {
        self.take_inner(owner, id, expected, now, true).await
    }
    /// Share the same durable consume path while retaining the protocol expiry rule.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    async fn take_inner(
        &self,
        owner: &str,
        id: Uuid,
        expected: Uuid,
        now: OffsetDateTime,
        require_expiry: bool,
    ) -> Result<Option<Record<A>>, RecordError> {
        /// Read/delete under one authoritative transaction; decode occurs before acknowledgment.
        struct Take<'a, A, C> {
            /// Borrowed typed adapter retains selected category/codec bounds.
            adapter: &'a SqlRecordAdapter<A, C>,
            /// Protocol-selected owner/purpose namespace.
            owner: &'a str,
            /// Exact record and revision identifying one consumption attempt.
            id: Uuid,
            /// Prior committed revision; a copied handle grants no extra consumption.
            expected: Uuid,
            /// Selected UTC clock value for expiry comparison.
            now: OffsetDateTime,
            /// Protocol consumption requires a persisted finite expiry.
            require_expiry: bool,
            buffer: Option<&'a mut RecordBuffer>,
        }
        impl<A: Send + 'static, C: RecordCodec<A>> NativeOperation<Option<Record<A>>> for Take<'_, A, C> {
            /// Decode before deleting so corrupt state fails closed and rolls back.
            async fn run<'a>(
                self,
                connection: &'a mut sqlx::TursoConnection,
            ) -> Result<Option<Record<A>>, SqlError>
            where
                Self: 'a,
            {
                let row=sqlx::query(AssertSqlSafe("SELECT record_id,revision,codec_version,expires_at,CASE WHEN length(payload)<=? THEN payload ELSE NULL END AS payload FROM effect_identity_records WHERE category=? AND owner=? AND record_id=? AND revision=? AND (expires_at IS NULL OR expires_at>?)"))
                        .bind(max_bytes_sql(self.adapter.policy)?).bind(self.adapter.category.key()).bind(self.owner).bind(self.id.to_string()).bind(self.expected.to_string()).bind(utc_nanos(self.now).map_err(|error|SqlError::Binding(Box::new(error)))?).fetch_optional(&mut *connection).await?;
                let Some(row) = row else { return Ok(None) };
                let record = self
                    .adapter
                    .decode(row, self.buffer)
                    .map_err(|error| SqlError::Binding(Box::new(error)))?;
                if self.require_expiry && record.expires_at.is_none() {
                    return Err(SqlError::Binding(Box::new(RecordError::Corrupt)));
                }
                let changed=sqlx::query(AssertSqlSafe("DELETE FROM effect_identity_records WHERE category=? AND owner=? AND record_id=? AND revision=?"))
                        .bind(self.adapter.category.key()).bind(self.owner).bind(self.id.to_string()).bind(self.expected.to_string()).execute(connection).await?;
                if changed.rows_affected() != 1 {
                    return Err(SqlError::Binding(Box::new(RecordError::Conflict)));
                }
                Ok(Some(record))
            }
        }
        let mut buffer = self.reserve_buffer()?;
        sql! {
            transaction_operation; kernel: self.backend.kernel, operation: Take {
                adapter: self,
                owner,
                id,
                expected,
                now,
                require_expiry,
                buffer: buffer.as_mut(),
            },
        }
        .await
        .map_err(record_sql_error)
    }
    /// Revoke/remove exactly the expected record revision; failed acknowledgment
    /// cannot establish logout, delivery cancellation or completed deletion.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub async fn remove(&self, owner: &str, id: Uuid, expected: Uuid) -> Result<bool, RecordError> {
        /// Borrowed conditional deletion shares the same native transaction semantics.
        struct Remove<'a> {
            /// Stable schema category.
            category: &'static str,
            /// Selected owner namespace.
            owner: &'a str,
            /// Record selected for conditional removal.
            id: Uuid,
            /// Expected committed revision, independent of security proof.
            expected: Uuid,
        }
        impl NativeOperation<bool> for Remove<'_> {
            /// Return the acknowledged compare/delete outcome, never a success on backend failure.
            async fn run<'a>(self, c: &'a mut sqlx::TursoConnection) -> Result<bool, SqlError>
            where
                Self: 'a,
            {
                let result=sqlx::query(AssertSqlSafe("DELETE FROM effect_identity_records WHERE category=? AND owner=? AND record_id=? AND revision=?"))
                    .bind(self.category).bind(self.owner).bind(self.id.to_string()).bind(self.expected.to_string()).execute(c).await?;
                Ok(result.rows_affected() == 1)
            }
        }
        sql! {
            transaction_operation; kernel: self.backend.kernel, operation: Remove {
                category: self.category.key(),
                owner,
                id,
                expected,
            },
        }
        .await
        .map_err(Into::into)
    }
    /// Read one finite page; no transaction or request is retained by its consumer.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub async fn page(
        &self,
        owner: &str,
        after: Option<Uuid>,
        limit: NonZeroU16,
        now: OffsetDateTime,
    ) -> Result<RecordPage<A>, RecordError> {
        if limit.get() > self.policy.page_limit.get() {
            return Err(RecordError::Bound);
        }
        /// Bounded page query borrowing the selected codec and owner partition.
        struct Page<'a, A, C> {
            /// Exact configured adapter generation.
            adapter: &'a SqlRecordAdapter<A, C>,
            /// Application-selected owner namespace.
            owner: &'a str,
            /// Exclusive record ID continuation.
            after: Option<Uuid>,
            /// Caller finite page count, checked against policy before execution.
            limit: NonZeroU16,
            /// Explicit native UTC instant for expiry filtering.
            now: OffsetDateTime,
            buffer: Option<&'a mut RecordBuffer>,
        }
        impl<A: Send + 'static, C: RecordCodec<A>> NativeOperation<RecordPage<A>> for Page<'_, A, C> {
            /// Decode only one finite native page and release the lease before returning it.
            #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
            async fn run<'a>(
                self,
                c: &'a mut sqlx::TursoConnection,
            ) -> Result<RecordPage<A>, SqlError>
            where
                Self: 'a,
            {
                let rows=sqlx::query(AssertSqlSafe("SELECT record_id,revision,codec_version,expires_at,CASE WHEN length(payload)<=? THEN payload ELSE NULL END AS payload FROM effect_identity_records WHERE category=? AND owner=? AND record_id>? AND (expires_at IS NULL OR expires_at>?) ORDER BY record_id LIMIT ?"))
                    .bind(max_bytes_sql(self.adapter.policy)?).bind(self.adapter.category.key()).bind(self.owner).bind(self.after.map_or_else(String::new,|v|v.to_string())).bind(utc_nanos(self.now).map_err(|error|SqlError::Binding(Box::new(error)))?).bind(i64::from(self.limit.get())+1).fetch_all(c).await?;
                let mut buffer = self.buffer;
                let mut records = rows
                    .into_iter()
                    .map(|row| self.adapter.decode(row, buffer.as_deref_mut()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| SqlError::Binding(Box::new(error)))?;
                let next = if records.len() > usize::from(self.limit.get()) {
                    records.pop();
                    records.last().map(|r| r.id)
                } else {
                    None
                };
                Ok(RecordPage { records, next })
            }
        }
        let mut buffer = self.reserve_buffer()?;
        sql! {
            connection_operation; kernel: self.backend.kernel, operation: Page {
                adapter: self,
                owner,
                after,
                limit,
                now,
                buffer: buffer.as_mut(),
            },
        }
        .await
        .map_err(record_sql_error)
    }
    /// Decode native SQL cells and validate both codec version and policy bound.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    fn decode(
        &self,
        row: sqlx::TursoRow,
        buffer: Option<&mut RecordBuffer>,
    ) -> Result<Record<A>, RecordError> {
        let id: String = row.try_get("record_id").map_err(|_| RecordError::Corrupt)?;
        let revision: String = row.try_get("revision").map_err(|_| RecordError::Corrupt)?;
        let version: i64 = row
            .try_get("codec_version")
            .map_err(|_| RecordError::Corrupt)?;
        let expires_at: Option<i64> = row
            .try_get("expires_at")
            .map_err(|_| RecordError::Corrupt)?;
        let payload: &[u8] = row.try_get("payload").map_err(|_| RecordError::Corrupt)?;
        let version = u16::try_from(version)
            .ok()
            .and_then(NonZeroU16::new)
            .ok_or(RecordError::Corrupt)?;
        Ok(Record {
            id: Uuid::parse_str(&id).map_err(|_| RecordError::Corrupt)?,
            revision: Uuid::parse_str(&revision).map_err(|_| RecordError::Corrupt)?,
            expires_at: expires_at
                .map(|value| OffsetDateTime::from_unix_timestamp_nanos(i128::from(value)))
                .transpose()
                .map_err(|_| RecordError::Corrupt)?,
            value: prepared::decode(
                &*self.codec,
                self.prepared_codec(),
                self.policy,
                version,
                payload,
                buffer,
            )?,
        })
    }
    /// Lazy reusable read selects owner/clock from R on each run; no runtime is created.
    pub fn find_effect<R: Send + Sync + 'static>(
        &self,
        id: Uuid,
        owner: impl Fn(&R) -> String + Send + Sync + 'static,
        now: impl Fn(&R) -> OffsetDateTime + Send + Sync + 'static,
    ) -> Effect<Option<Record<A>>, RecordError, R> {
        let adapter = self.clone();
        let owner = Arc::new(owner);
        let now = Arc::new(now);
        effect_uninterruptible! {
            program: effect_future! {
            exit_with_context; body: move |env: Arc<R>, _| {
            let adapter = adapter.clone();
            let owner = owner(&env);
            let now = now(&env);
            async move {
                match adapter.find(&owner, id, now).await {
                    Ok(value) => effect::Exit::Success(value),
                    Err(error) => error.into_exit(),
                }
            }
        }, }, }
    }
    /// Move one write payload into its Effect. Re-execution follows core one-shot
    /// semantics instead of cloning secrets; terminal native settlement is shielded.
    pub fn create_once<R: Send + Sync + 'static>(
        &self,
        owner: String,
        id: Uuid,
        expires_at: Option<OffsetDateTime>,
        value: A,
    ) -> Effect<Uuid, RecordError, R> {
        let adapter = self.clone();
        let pending = std::sync::Mutex::new(Some((owner, value)));
        effect_uninterruptible! {
            program: effect_future! {
            exit_with_context; body: move |_: Arc<R>, _| {
            let candidate = pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let adapter = adapter.clone();
            async move {
                match candidate {
                    Some((owner, value)) => {
                        match adapter.create(&owner, id, expires_at, value).await {
                            Ok(revision) => effect::Exit::Success(revision),
                            Err(error) => error.into_exit(),
                        }
                    }
                    None => effect::Exit::Die(effect::Defect::from(
                        effect::LibraryDefect::FutureFactoryConsumed,
                    )),
                }
            }
        }, }, }
    }
}

/// Restore an invocation-local typed rejection after kernel rollback; provider
/// failures retain the whole SQL primary/cleanup/uncertainty tree.
fn record_sql_error(error: SqlError) -> RecordError {
    match error {
        SqlError::Binding(error) => match error.downcast::<RecordError>() {
            Ok(error) => *error,
            Err(error) => RecordError::Sql(SqlError::Binding(error)),
        },
        error @ (SqlError::Configuration(_)
        | SqlError::Identifier(_)
        | SqlError::Provider(_)
        | SqlError::RowAbsent
        | SqlError::Decode(_)
        | SqlError::Memory(_)
        | SqlError::RowLimit { .. }
        | SqlError::PreparedStatement(_)
        | SqlError::PreparedRow(_)
        | SqlError::InactiveSession
        | SqlError::CommitOutcomeUnknown(_)
        | SqlError::Cleanup { .. }
        | SqlError::Migration { .. }
        | SqlError::MaintenanceRequired
        | SqlError::ReplicaNotAuthoritative
        | SqlError::NativeInvocation(_)
        | SqlError::Consumed) => RecordError::Sql(error),
    }
}

/// Typed independently acknowledged audit event; it is not an implicit account transaction.
pub struct AuditEntry<A> {
    /// Monotonic database sequence used as an exclusive incremental cursor.
    pub sequence: i64,
    /// Unique caller event ID, supporting inspection after acknowledgment uncertainty.
    pub event_id: Uuid,
    /// Validated application-selected audit payload.
    pub value: A,
}
/// A finite audit page owns values and retains no native connection/transaction.
pub struct AuditPage<A> {
    /// Ascending sequence records, bounded by the selected policy.
    pub entries: Vec<AuditEntry<A>>,
    /// Exclusive sequence boundary if another record existed at query time.
    pub next: Option<i64>,
}
/// SQL-backed append/read audit capability using an explicit private payload codec.
pub struct SqlAuditAdapter<A, C> {
    /// Selected authoritative account backend.
    backend: SqlxIdentityStore,
    /// Application-owned codec retained once; no untyped event interpretation.
    codec: Arc<C>,
    prepared: Option<Arc<PreparedCodec<A, C>>>,
    /// Explicit encoded byte/page bounds.
    policy: RecordPolicy,
    /// Payload type remains static without retained active values.
    value: PhantomData<fn() -> A>,
}
impl<A, C> Clone for SqlAuditAdapter<A, C> {
    /// Share provider/codec configuration without requiring event payload Clone.
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            codec: self.codec.clone(),
            prepared: self.prepared.clone(),
            policy: self.policy,
            value: PhantomData,
        }
    }
}
impl<A: Send + 'static, C: RecordCodec<A>> SqlAuditAdapter<A, C> {
    /// Select an inert SQL audit backend; no automatic account/audit coupling is claimed.
    pub fn new(backend: SqlxIdentityStore, policy: RecordPolicy, codec: C) -> Self {
        Self {
            backend,
            codec: Arc::new(codec),
            prepared: None,
            policy,
            value: PhantomData,
        }
    }
    /// Select preadmitted record bytes for every read and write on this adapter.
    /// Exhaustion uses the existing record bound failure before codec invocation.
    pub fn with_prepared_memory(
        mut self,
        memory: Arc<PreparedRecordMemory>,
    ) -> Result<Self, MemoryError>
    where
        C: PreparedRecordCodec<A>,
    {
        self.prepared = Some(Arc::new(PreparedCodec::new(memory, self.policy)?));
        Ok(self)
    }
    fn prepared_codec(&self) -> Option<&PreparedCodec<A, C>> {
        self.prepared.as_deref()
    }
    #[expect(
        clippy::result_large_err,
        reason = "preserves the established typed record ABI"
    )]
    fn reserve_buffer(&self) -> Result<Option<RecordBuffer>, RecordError> {
        prepared::reserve(self.prepared_codec()).map_err(|_| RecordError::Bound)
    }
    /// Reject a selected handle from another storage definition.
    #[must_use]
    pub fn belongs_to(&self, configuration: &crate::capabilities::StorageConfiguration) -> bool {
        Arc::ptr_eq(&self.backend.identity, &configuration.identity)
    }
    /// Append exactly one moved event and return its acknowledged sequence. Lost
    /// acknowledgment is inspected by event ID instead of blindly retried.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub async fn append(&self, owner: &str, event_id: Uuid, value: A) -> Result<i64, RecordError> {
        let admission = RecordWriteAdmission::reserve(&self.backend, self.reserve_buffer()?)?;
        let mut payload = prepared::encode(
            &*self.codec,
            self.prepared_codec(),
            self.policy,
            &value,
            admission.buffer,
        )?;
        /// Borrow owner while moving one encoded event under kernel settlement.
        struct Append<'a> {
            /// Selected administrative owner namespace, not authority by itself.
            owner: &'a str,
            /// Caller-generated inspection/idempotency identity.
            event_id: Uuid,
            /// Stable private codec version.
            version: u16,
            /// Encoded moved event bytes, bounded before native execution.
            payload: PayloadBinding<'a>,
        }
        impl NativeOperation<i64> for Append<'_> {
            /// Native INSERT RETURNING acknowledges this event's actual sequence.
            async fn run<'a>(self, c: &'a mut sqlx::TursoConnection) -> Result<i64, SqlError>
            where
                Self: 'a,
            {
                let row=self.payload.bind(sqlx::query(AssertSqlSafe("INSERT INTO effect_identity_audit(owner,event_id,codec_version,payload) VALUES(?,?,?,?) RETURNING sequence"))
                    .bind(self.owner).bind(self.event_id.to_string()).bind(i64::from(self.version))).fetch_one(c).await?;
                row.try_get("sequence").map_err(Into::into)
            }
        }
        let operation = Append {
            owner,
            event_id,
            version: self.codec.version().get(),
            payload: payload.binding(),
        };
        let result = match admission.transaction {
            Some(transaction) => sql! { run_transaction_operation; reservation: transaction, operation: operation, }.await,
            None => sql! { transaction_operation; kernel: self.backend.kernel, operation: operation, }.await,
        };
        result.map_err(Into::into)
    }
    /// Incrementally read a finite page for application authority checks before each
    /// emitted record. This store method alone does not establish operator permission.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub async fn page(
        &self,
        owner: &str,
        after: Option<i64>,
        limit: NonZeroU16,
    ) -> Result<AuditPage<A>, RecordError> {
        if limit.get() > self.policy.page_limit.get() || after.is_some_and(|v| v < 0) {
            return Err(RecordError::Bound);
        }
        /// Borrow the owner/codec while the native kernel owns one finite query.
        struct Read<'a, A, C> {
            /// Selected typed audit adapter.
            adapter: &'a SqlAuditAdapter<A, C>,
            /// Selected owner partition, validated by the authorization workflow.
            owner: &'a str,
            /// Exclusive prior sequence; zero is the schema AUTOINCREMENT start boundary.
            after: i64,
            /// Explicit finite query limit with one continuation lookahead.
            limit: u16,
            buffer: Option<&'a mut RecordBuffer>,
        }
        impl<A: Send + 'static, C: RecordCodec<A>> NativeOperation<AuditPage<A>> for Read<'_, A, C> {
            /// Consume native cells under the lease; no native row escapes into Effect.
            #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
            async fn run<'a>(
                self,
                c: &'a mut sqlx::TursoConnection,
            ) -> Result<AuditPage<A>, SqlError>
            where
                Self: 'a,
            {
                let rows=sqlx::query(AssertSqlSafe("SELECT sequence,event_id,codec_version,CASE WHEN length(payload)<=? THEN payload ELSE NULL END AS payload FROM effect_identity_audit WHERE owner=? AND sequence>? ORDER BY sequence LIMIT ?"))
                    .bind(max_bytes_sql(self.adapter.policy)?).bind(self.owner).bind(self.after).bind(i64::from(self.limit)+1).fetch_all(c).await?;
                let mut buffer = self.buffer;
                let mut entries = Vec::with_capacity(rows.len());
                for row in rows {
                    let sequence: i64 = row.try_get("sequence")?;
                    let event_id: String = row.try_get("event_id")?;
                    let version: i64 = row.try_get("codec_version")?;
                    let payload: &[u8] = row.try_get("payload")?;
                    let mut decode = || -> Result<AuditEntry<A>, RecordError> {
                        let version = u16::try_from(version)
                            .ok()
                            .and_then(NonZeroU16::new)
                            .ok_or(RecordError::Corrupt)?;
                        Ok(AuditEntry {
                            sequence,
                            event_id: Uuid::parse_str(&event_id)
                                .map_err(|_| RecordError::Corrupt)?,
                            value: prepared::decode(
                                &*self.adapter.codec,
                                self.adapter.prepared_codec(),
                                self.adapter.policy,
                                version,
                                payload,
                                buffer.as_deref_mut(),
                            )?,
                        })
                    };
                    entries.push(decode().map_err(|error| SqlError::Binding(Box::new(error)))?);
                }
                let next = if entries.len() > usize::from(self.limit) {
                    entries.pop();
                    entries.last().map(|entry| entry.sequence)
                } else {
                    None
                };
                Ok(AuditPage { entries, next })
            }
        }
        let mut buffer = self.reserve_buffer()?;
        sql! {
            connection_operation; kernel: self.backend.kernel, operation: Read {
                adapter: self,
                owner,
                after: after.unwrap_or(0),
                limit: limit.get(),
                buffer: buffer.as_mut(),
            },
        }
        .await
        .map_err(record_sql_error)
    }
}
