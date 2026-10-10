//! Local SQLx persistence replacing native Identity's default stores.
//!
//! Native managers use this same implementation. Each store mutation owns one
//! acknowledged transaction, independently of an ambient application SQL scope.
//! Manager workflows can therefore contain multiple commits. No memory, native
//! Turso or synchronized-replica persistence is selected as a fallback.

pub mod accounts;
pub mod auxiliary;
pub mod components;
mod error;
mod executor;
pub mod invocation;
mod native;
pub mod records;
pub mod schema;

use effect_sql::{
    MigrationCatalog, MigrationStep, Rollback, SQLX_JOURNAL, SqlError, SqlKernel, SqlMigrator,
};
use std::sync::Arc;
use time::OffsetDateTime;

/// Explicit store capacity and UTC callable selected by application configuration.
/// The native challenge API has no `now`/capacity arguments, so these are retained
/// at installation; they are never reacquired from a hidden default/global clock.
#[derive(Clone, Copy)]
pub struct IdentityStoragePolicy {
    /// Maximum concurrent unexpired passkey ceremonies; zero rejects installation.
    pub challenge_capacity: std::num::NonZeroUsize,
    /// Clock compatible with native fn-pointer setters; arbitrary R captures are
    /// not claimed to fit that upstream signature.
    pub utc_now: fn() -> OffsetDateTime,
}

/// Shared storage configuration identity for selected auxiliary handles and grants.
/// Construction is private: copies share a configuration, never one-use authority.
pub(crate) use crate::capabilities::StorageIdentity;

/// Actual native replacement store over the local shared SQLx pool.
#[derive(Clone)]
pub struct SqlxIdentityStore {
    /// Kernel is responsible for authoritative writes and terminal settlement.
    pub(crate) kernel: SqlKernel,
    /// Explicit caller policy retained across every native operation.
    pub(crate) policy: IdentityStoragePolicy,
    /// Optional-capability selection fence, private to this configuration.
    pub(crate) identity: Arc<StorageIdentity>,
    /// Present only on a private invocation clone, never a process/global service.
    pub(crate) invocation: Option<Arc<invocation::InvocationDiagnostics>>,
}
impl SqlxIdentityStore {
    /// Select an authoritative local pool and explicit policy without I/O. Apply
    /// the catalog before accepting traffic; this constructor creates no schema.
    pub fn new(pool: sqlx::TursoPool, policy: IdentityStoragePolicy) -> Result<Self, SqlError> {
        Self::from_kernel(SqlKernel::new(pool), policy)
    }
    /// Retain an application's existing kernel; no nested runtime/pool is created.
    pub fn from_kernel(kernel: SqlKernel, policy: IdentityStoragePolicy) -> Result<Self, SqlError> {
        kernel.require_authoritative()?;
        let options = kernel.pool().connect_options();
        let foreign_keys = options
            .pragmas()
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("foreign_keys"))
            .and_then(|(_, value)| value.as_deref());
        if !foreign_keys.is_some_and(|value| {
            matches!(
                value.trim_matches('\'').to_ascii_uppercase().as_str(),
                "ON" | "1" | "TRUE" | "YES"
            )
        }) {
            return Err(SqlError::Configuration(
                "identity persistence requires native foreign-key enforcement".into(),
            ));
        }
        Ok(Self {
            kernel,
            policy,
            identity: Arc::new(StorageIdentity),
            invocation: None,
        })
    }
    /// Share the same store/backend configuration with native browser and selected
    /// extension adapters; this moves no account or credential payload.
    #[must_use]
    pub fn shared_backend(&self) -> Self {
        self.clone()
    }
    /// Borrow the actual kernel for checked SQLx interoperability and migration composition.
    #[must_use]
    pub fn kernel(&self) -> &SqlKernel {
        &self.kernel
    }
    /// Retain this provider's private composition identity for neutral capability selection.
    #[must_use]
    pub fn configuration(&self) -> crate::capabilities::StorageConfiguration {
        crate::capabilities::StorageConfiguration {
            identity: self.identity.clone(),
        }
    }
    /// Begin a private diagnostic scope. Build selected native components with
    /// its store handle and settle that scope at the outer Effect boundary.
    #[must_use]
    pub fn invocation(&self) -> invocation::StoreInvocation {
        invocation::StoreInvocation::new(self)
    }
    /// Retain the full settled SQL outcome before returning a native category.
    pub(crate) fn narrow(&self, error: SqlError) -> identity_rs::identity::StoreError {
        let category = error::sql_category(&error);
        if let Some(invocation) = &self.invocation {
            invocation.retain(error);
        }
        category
    }
    /// Return this schema's catalog entry for composition with application steps
    /// in the same SQLx journal. Identity does not own a parallel migration history.
    pub fn migration_step() -> MigrationStep {
        MigrationStep::transactional(
            schema::SCHEMA_MIGRATION_VERSION,
            "effect identity security schema v1",
            schema::SCHEMA_UP,
            Rollback::Irreversible,
        )
    }
    /// Build the standalone Identity catalog. Applications sharing a journal with
    /// business SQL should instead include migration_step in their complete catalog.
    pub fn migration_catalog() -> Result<MigrationCatalog, SqlError> {
        MigrationCatalog::new(SQLX_JOURNAL, vec![Self::migration_step()])
    }
    /// Apply only the standalone catalog through the shared SQLx journal/authority.
    /// Rejection/dirty history/commit uncertainty is returned without automatic replay.
    pub async fn initialize(&self) -> Result<(), SqlError> {
        let migrator = SqlMigrator::new(self.kernel.clone(), Self::migration_catalog()?)?;
        migrator.initialize().await?;
        migrator.up().await?;
        Ok(())
    }
}

/// Server-backed browser store retaining the exact account backend generation.
#[derive(Clone)]
pub struct SqlxBrowserContextStore {
    /// Same SQLx store; browser CAS uses its operation-owned transaction path.
    backend: SqlxIdentityStore,
}
impl SqlxBrowserContextStore {
    /// Select the existing backend without creating another native provider.
    #[must_use]
    pub fn new(backend: SqlxIdentityStore) -> Self {
        Self { backend }
    }
    /// Borrow native interoperation for selected browser request-security services.
    #[must_use]
    pub fn backend(&self) -> &SqlxIdentityStore {
        &self.backend
    }
}
impl identity_rs::identity::store::BrowserContextStore for SqlxBrowserContextStore {
    /// Delegate capacity/expiry/uniqueness checks to the same SQL transaction owner.
    async fn create_browser_context(
        &self,
        context: identity_rs::identity::store::BrowserContext,
        now: OffsetDateTime,
        capacity: usize,
    ) -> Result<(), identity_rs::identity::StoreError> {
        self.backend
            .create_browser_context(context, now, capacity)
            .await
    }
    /// Read validated unexpired state without treating unavailable storage as absence.
    async fn find_browser_context(
        &self,
        id: uuid::Uuid,
        now: OffsetDateTime,
    ) -> Result<
        Option<identity_rs::identity::store::BrowserContext>,
        identity_rs::identity::StoreError,
    > {
        self.backend.find_browser_context(id, now).await
    }
    /// Compare token and pending proof at the same durable update boundary.
    async fn replace_browser_pending_authentication(
        &self,
        id: uuid::Uuid,
        expected_token: &identity_rs::identity::models::AuthTokenValue,
        expected_pending: Option<&identity_rs::identity::models::AuthTokenValue>,
        value: Option<identity_rs::identity::models::AuthTokenValue>,
        now: OffsetDateTime,
    ) -> Result<bool, identity_rs::identity::StoreError> {
        self.backend
            .replace_browser_pending_authentication(
                id,
                expected_token,
                expected_pending,
                value,
                now,
            )
            .await
    }
    /// Removal acknowledgment is required before a security transition succeeds.
    async fn remove_browser_context(
        &self,
        id: uuid::Uuid,
    ) -> Result<(), identity_rs::identity::StoreError> {
        self.backend.remove_browser_context(id).await
    }
    /// Cleanup is bounded by the installed browser capacity and returns its actual count.
    async fn remove_expired_browser_contexts(
        &self,
        now: OffsetDateTime,
    ) -> Result<usize, identity_rs::identity::StoreError> {
        self.backend.remove_expired_browser_contexts(now).await
    }
}
