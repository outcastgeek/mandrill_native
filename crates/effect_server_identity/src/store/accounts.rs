//! Atomic complete security-account snapshots beside native granular stores.
//!
//! Native managers may commit granular operations independently. Database triggers
//! maintain an aggregate revision for every related mutation so replacing a stale
//! whole snapshot cannot silently lose a claim/token/passkey/membership update.
//! Snapshot operations own their transaction and do not enroll ambient business SQL.

use super::{
    SqlxIdentityStore,
    error::{command_error, provider_category},
    executor::StoreConnection,
    native::*,
};
use effect_sql::{NativeOperation, SqlError};
use identity_rs::identity::{StoreError, models::*};
use sqlx::Row;

/// Exact provider revision for whole-security-snapshot CAS. It is not an
/// authentication grant or the native manager's public concurrency stamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountRevision(i64);
impl AccountRevision {
    /// Inspect the acknowledged nonnegative logical sequence for reconciliation.
    #[must_use]
    pub fn value(self) -> i64 {
        self.0
    }
}
/// Secret native token entry in a private security snapshot; Debug is omitted.
pub struct AccountToken {
    /// Validated provider discriminator selected by native algorithms.
    pub provider: LoginProvider,
    /// Validated purpose/name key, independent of its secret value.
    pub name: AuthTokenName,
    /// Native validated secret token, never part of a public account view.
    pub value: AuthTokenValue,
}
/// Account persistence policy selected explicitly for one full security aggregate.
/// Ciphertext and keyed indexes stay separate from plaintext native snapshots.
pub enum AccountState {
    /// Owned native personal fields, boxed to keep the snapshot's state discriminator small.
    Plain(Box<IdentityUser>),
    /// Owned provider-protected fields and indexes, boxed without copying their payload.
    Protected(Box<identity_rs::identity::protection::storage::ProtectedUserRecord>),
}
impl AccountState {
    /// Borrow validated security metadata; protected metadata contains no personal aliases.
    #[must_use]
    pub fn metadata(&self) -> &IdentityUser {
        match self {
            Self::Plain(user) => user,
            Self::Protected(record) => record.metadata(),
        }
    }
    /// Validate the exact selected native persistence policy before SQL mutation.
    fn validate(&self) -> Result<(), StoreError> {
        match self {
            Self::Plain(user) => user.validate().map_err(|_| StoreError::CorruptStoredData),
            Self::Protected(record) => record.validate(),
        }
    }
}
/// Full bounded native security snapshot, moved once into atomic persistence.
/// Application profile/personal-data inventories remain selected independent hooks.
pub struct AccountSnapshot {
    /// Native validated account metadata including lockout/password/security state.
    pub user: AccountState,
    /// Stable role IDs, preserving membership across native role rename.
    pub roles: Vec<RoleId>,
    /// Native validated claims, bounded at the pinned native capacity.
    pub claims: Vec<IdentityClaim>,
    /// Unique native external associations; these records do not establish provider proof.
    pub logins: Vec<UserLoginInfo>,
    /// Private native token records, including selected authenticator-key namespace.
    pub tokens: Vec<AccountToken>,
    /// Native one-use hashes only; plaintext recovery secrets are never accepted.
    pub recovery_hashes: Vec<RecoveryCodeHash>,
    /// Native validated passkeys, whose algorithm verification is selected separately.
    pub passkeys: Vec<UserPasskeyInfo>,
}
/// Acknowledged complete account state with its exact aggregate replacement revision.
pub struct VersionedAccount {
    /// Revision fencing every native granular security child mutation.
    pub revision: AccountRevision,
    /// Owned bounded security state, containing no borrowed SQL owner.
    pub snapshot: AccountSnapshot,
}
impl AccountSnapshot {
    /// Check native aggregate/collection bounds before starting SQL. Token count
    /// is explicitly bounded by the selected snapshot policy, not an invented default.
    fn validate(&self, token_capacity: usize) -> Result<(), StoreError> {
        self.user.validate()?;
        if self.roles.len() > super::schema::NATIVE_COLLECTION_LIMIT
            || self.claims.len() > super::schema::NATIVE_COLLECTION_LIMIT
            || self.logins.len() > super::schema::NATIVE_COLLECTION_LIMIT
            || self.passkeys.len() > super::schema::NATIVE_COLLECTION_LIMIT
            || self.recovery_hashes.len() > super::schema::NATIVE_RECOVERY_LIMIT
            || self.tokens.len() > token_capacity
        {
            return Err(StoreError::ConstraintViolation);
        }
        for passkey in &self.passkeys {
            passkey.validate()?;
        }
        Ok(())
    }
}
/// Selected atomic-snapshot capability with explicit token collection capacity.
#[derive(Clone)]
pub struct AtomicAccounts {
    /// Actual account backend, not a wrapper around memory/native-Turso storage.
    backend: SqlxIdentityStore,
    /// Maximum tokens allocated/persisted by this complete-snapshot API.
    token_capacity: std::num::NonZeroUsize,
}
impl AtomicAccounts {
    /// Select the same native backend and finite complete-snapshot token bound.
    #[must_use]
    pub fn new(backend: SqlxIdentityStore, token_capacity: std::num::NonZeroUsize) -> Self {
        Self {
            backend,
            token_capacity,
        }
    }
    /// Check an optional/selected capability's definition ownership fence.
    #[must_use]
    pub fn belongs_to(&self, configuration: &crate::capabilities::StorageConfiguration) -> bool {
        std::sync::Arc::ptr_eq(&self.backend.identity, &configuration.identity)
    }
    /// Load one internally consistent complete security snapshot under native write
    /// authority, releasing the transaction before application/HTTP consumption.
    pub async fn find(&self, id: &UserId) -> Result<Option<VersionedAccount>, StoreError> {
        /// Borrow the caller identity until one complete read settles.
        struct Read<'a> {
            /// Exact native user identity.
            id: &'a UserId,
            /// Explicit token allocation bound for this snapshot interface.
            capacity: usize,
        }
        impl NativeOperation<Option<VersionedAccount>> for Read<'_> {
            /// Serialize aggregate reads under the same authoritative transaction as mutations.
            async fn run<'a>(
                self,
                c: &'a mut sqlx::TursoConnection,
            ) -> Result<Option<VersionedAccount>, SqlError>
            where
                Self: 'a,
            {
                let connection = StoreConnection::new(c);
                let result = load(&connection, self.id, self.capacity).await;
                result.map_err(|error| connection.diagnostics().command_error(error))
            }
        }
        self.backend
            .kernel
            .transaction_operation(Read {
                id,
                capacity: self.token_capacity.get(),
            })
            .await
            .map_err(|error| self.backend.narrow(error))
    }
    /// Create metadata/aliases/all bounded security children atomically and return
    /// the actual committed aggregate revision. Duplicate/uncertain outcomes propagate.
    pub async fn create(
        &self,
        snapshot: AccountSnapshot,
        require_unique_email: bool,
    ) -> Result<AccountRevision, StoreError> {
        snapshot.validate(self.token_capacity.get())?;
        /// Move a complete aggregate into one native acknowledged create operation.
        struct Create {
            /// Owned native state and security children, with no payload Clone bound.
            snapshot: AccountSnapshot,
            /// Native selected uniqueness policy, atomically persisted with aliases.
            unique: bool,
        }
        impl NativeOperation<AccountRevision> for Create {
            /// Insert the account and all relations under one shared kernel transaction.
            async fn run<'a>(
                self,
                c: &'a mut sqlx::TursoConnection,
            ) -> Result<AccountRevision, SqlError>
            where
                Self: 'a,
            {
                let connection = StoreConnection::new(c);
                let result=async{
                    let user=self.snapshot.user.metadata();let id=user.id();
                    match &self.snapshot.user {
                        AccountState::Plain(user)=>{
                            let name=user.normalized_user_name().ok_or(StoreError::CorruptStoredData)?;
                            let mut protected=connection.query("SELECT user_id FROM effect_identity_protected_users LIMIT 1",()).await.map_err(provider_category)?;
                            if protected.next().await.map_err(provider_category)?.is_some(){return Err(StoreError::ConstraintViolation)}
                            validate_email_claim(&connection,user,self.unique).await?;
                            connection.execute("INSERT INTO effect_identity_users(id,normalized_user_name,normalized_email,concurrency_stamp,payload) VALUES(?1,?2,?3,?4,?5)",(id.to_string(),name.as_str(),user.normalized_email().map(ToString::to_string),user.concurrency_stamp().to_string(),serialize_user(user)?)).await.map_err(provider_category)?;
                            connection.execute("INSERT INTO effect_identity_user_email_policy(user_id,require_unique) VALUES(?1,?2)",(id.to_string(),i64::from(self.unique))).await.map_err(provider_category)?;
                        },
                        AccountState::Protected(record)=>{
                            if record.require_unique_email()!=self.unique{return Err(StoreError::ConstraintViolation)}
                            let mut plain=connection.query("SELECT 1 FROM effect_identity_users u LEFT JOIN effect_identity_protected_users p ON p.user_id=u.id WHERE p.user_id IS NULL LIMIT 1",()).await.map_err(provider_category)?;
                            if plain.next().await.map_err(provider_category)?.is_some(){return Err(StoreError::ConstraintViolation)}
                            validate_protected_claims(&connection,record).await?;
                            connection.execute("INSERT INTO effect_identity_users(id,normalized_user_name,normalized_email,concurrency_stamp,payload) VALUES(?1,?2,NULL,?3,?4)",(id.to_string(),format!("protected:{id}"),user.concurrency_stamp().to_string(),serialize_user(user)?)).await.map_err(provider_category)?;
                            connection.execute("INSERT INTO effect_identity_protected_users(user_id,require_unique,payload) VALUES(?1,?2,?3)",(id.to_string(),i64::from(self.unique),super::schema::encode_security(record)?)).await.map_err(provider_category)?;
                            insert_protected_indexes(&connection,record).await?;
                        },
                    }
                    write_children(&connection,&self.snapshot).await?;
                    revision(&connection,&id).await
                }.await;
                result.map_err(|error| connection.diagnostics().command_error(error))
            }
        }
        self.backend
            .kernel
            .transaction_operation(Create {
                snapshot,
                unique: require_unique_email,
            })
            .await
            .map_err(|error| self.backend.narrow(error))
    }
    /// Replace metadata/aliases/security children only at the exact full aggregate
    /// revision, then return its acknowledged new revision. No blind retry is performed.
    pub async fn replace(
        &self,
        expected: AccountRevision,
        snapshot: AccountSnapshot,
    ) -> Result<AccountRevision, StoreError> {
        snapshot.validate(self.token_capacity.get())?;
        /// One moved replacement and its immutable expected aggregate fence.
        struct Replace {
            /// Expected full aggregate revision, not merely metadata's native stamp.
            expected: AccountRevision,
            /// Complete owned snapshot whose relations replace the prior set.
            snapshot: AccountSnapshot,
        }
        impl NativeOperation<AccountRevision> for Replace {
            /// Locked revision check and every relation mutation share one durable boundary.
            async fn run<'a>(
                self,
                c: &'a mut sqlx::TursoConnection,
            ) -> Result<AccountRevision, SqlError>
            where
                Self: 'a,
            {
                let connection = StoreConnection::new(c);
                let result=async{
                    let id=self.snapshot.user.metadata().id();
                    if revision(&connection,&id).await?!=self.expected{return Err(StoreError::TransactionConflict)}
                    let stamp=ConcurrencyStamp::new();
                    let user=self.snapshot.user.metadata();
                    let mut rows=connection.query("SELECT require_unique FROM effect_identity_protected_users WHERE user_id=?1",(id.to_string(),)).await.map_err(provider_category)?;
                    let protected=rows.next().await.map_err(provider_category)?;
                    match &self.snapshot.user {
                        AccountState::Plain(user)=>{
                            if protected.is_some(){return Err(StoreError::ConstraintViolation)}
                            let name=user.normalized_user_name().ok_or(StoreError::CorruptStoredData)?;
                            validate_email_claim(&connection,user,false).await?;
                            let changed=connection.execute("UPDATE effect_identity_users SET normalized_user_name=?1,normalized_email=?2,concurrency_stamp=?3,payload=?4 WHERE id=?5 AND aggregate_revision=?6",(name.as_str(),user.normalized_email().map(ToString::to_string),stamp.to_string(),serialize_refreshed_user(user,&stamp)?,id.to_string(),self.expected.0)).await.map_err(provider_category)?;
                            ensure_rows_changed(changed,StoreError::TransactionConflict)?;
                        },
                        AccountState::Protected(record)=>{
                            let row=protected.ok_or(StoreError::ConstraintViolation)?;
                            if row_integer_at(&row,0)?!=i64::from(record.require_unique_email()){return Err(StoreError::ConstraintViolation)}
                            validate_protected_claims(&connection,record).await?;
                            let changed=connection.execute("UPDATE effect_identity_users SET concurrency_stamp=?1,payload=?2 WHERE id=?3 AND aggregate_revision=?4",(stamp.to_string(),serialize_refreshed_user(user,&stamp)?,id.to_string(),self.expected.0)).await.map_err(provider_category)?;
                            ensure_rows_changed(changed,StoreError::TransactionConflict)?;
                            connection.execute("UPDATE effect_identity_protected_users SET payload=?1 WHERE user_id=?2",(super::schema::encode_security(record)?,id.to_string())).await.map_err(provider_category)?;
                            connection.execute("DELETE FROM effect_identity_protected_indexes WHERE user_id=?1",(id.to_string(),)).await.map_err(provider_category)?;
                            insert_protected_indexes(&connection,record).await?;
                        },
                    }
                    delete_children(&connection,&id).await?;
                    write_children(&connection,&self.snapshot).await?;
                    revision(&connection,&id).await
                }.await;
                result.map_err(|error| connection.diagnostics().command_error(error))
            }
        }
        self.backend
            .kernel
            .transaction_operation(Replace { expected, snapshot })
            .await
            .map_err(|error| self.backend.narrow(error))
    }
    /// Delete only at the expected aggregate revision; foreign-key cascades remove
    /// related security state. Failed acknowledgment is not successful invalidation.
    pub async fn delete(&self, id: &UserId, expected: AccountRevision) -> Result<(), StoreError> {
        /// Borrow identity while conditionally deleting one full aggregate.
        struct Delete<'a> {
            /// Exact native account identity.
            id: &'a UserId,
            /// Expected revision binding this destructive account command.
            expected: AccountRevision,
        }
        impl NativeOperation<()> for Delete<'_> {
            /// Compare/delete at the same SQL linearization point with real cascade ownership.
            async fn run<'a>(self, c: &'a mut sqlx::TursoConnection) -> Result<(), SqlError>
            where
                Self: 'a,
            {
                let connection = StoreConnection::new(c);
                let changed = connection
                    .execute(
                        "DELETE FROM effect_identity_users WHERE id=?1 AND aggregate_revision=?2",
                        (self.id.to_string(), self.expected.0),
                    )
                    .await
                    .map_err(SqlError::from)?;
                ensure_rows_changed(changed, StoreError::TransactionConflict).map_err(command_error)
            }
        }
        self.backend
            .kernel
            .transaction_operation(Delete { id, expected })
            .await
            .map_err(|error| self.backend.narrow(error))
    }
}
impl crate::services::AccountSnapshotMetadata for AccountSnapshot {
    /// Borrow only security metadata; encrypted fields retain their actual native representation.
    fn metadata(&self) -> &IdentityUser {
        self.user.metadata()
    }
}
impl crate::services::AccountSnapshots for AtomicAccounts {
    type Snapshot = AccountSnapshot;
    type Revision = AccountRevision;
    /// Scope the selected native snapshot interface so narrowed StoreError retains
    /// the original SQL failure/defect/interruption/cleanup at its Effect boundary.
    async fn load(
        &self,
        id: UserId,
    ) -> Result<
        Option<crate::services::VersionedSnapshot<AccountSnapshot, AccountRevision>>,
        crate::IdentityError,
    > {
        use crate::capabilities::NativeBackend;
        let backend = self.backend.scoped();
        let adapter = Self {
            backend: backend.clone(),
            token_capacity: self.token_capacity,
        };
        let result = adapter
            .find(&id)
            .await
            .map(|value| {
                value.map(|value| crate::services::VersionedSnapshot {
                    snapshot: value.snapshot,
                    revision: value.revision,
                })
            })
            .map_err(crate::IdentityError::Store);
        backend.capture_outcome(result)
    }
    /// Atomically acknowledge the expected aggregate revision and restore full scoped diagnostics.
    async fn replace(
        &self,
        expected: AccountRevision,
        snapshot: AccountSnapshot,
    ) -> Result<AccountRevision, crate::IdentityError> {
        use crate::capabilities::NativeBackend;
        let backend = self.backend.scoped();
        let adapter = Self {
            backend: backend.clone(),
            token_capacity: self.token_capacity,
        };
        let result = AtomicAccounts::replace(&adapter, expected, snapshot)
            .await
            .map_err(crate::IdentityError::Store);
        backend.capture_outcome(result)
    }
    /// Acknowledge deletion of the exact expected aggregate revision and restore scoped diagnostics.
    async fn delete(
        &self,
        id: UserId,
        expected: AccountRevision,
    ) -> Result<(), crate::IdentityError> {
        use crate::capabilities::NativeBackend;
        let backend = self.backend.scoped();
        let adapter = Self {
            backend: backend.clone(),
            token_capacity: self.token_capacity,
        };
        let result = AtomicAccounts::delete(&adapter, &id, expected)
            .await
            .map_err(crate::IdentityError::Store);
        backend.capture_outcome(result)
    }
}
/// Read the nonnegative v1 aggregate sequence without interpreting absence as success.
async fn revision(
    connection: &StoreConnection<'_>,
    id: &UserId,
) -> Result<AccountRevision, StoreError> {
    let mut rows = connection
        .query(
            "SELECT aggregate_revision FROM effect_identity_users WHERE id=?1",
            (id.to_string(),),
        )
        .await
        .map_err(provider_category)?;
    let row = rows
        .next()
        .await
        .map_err(provider_category)?
        .ok_or(StoreError::NotFound)?;
    let revision = row_integer_at(&row, 0)?;
    if revision < 0 {
        return Err(StoreError::CorruptStoredData);
    }
    Ok(AccountRevision(revision))
}
/// Read bounded native aggregate relations while the caller retains one writer snapshot.
async fn load(
    c: &StoreConnection<'_>,
    id: &UserId,
    token_capacity: usize,
) -> Result<Option<VersionedAccount>, StoreError> {
    let Some(user) = find_user_by_id(c, id).await? else {
        return Ok(None);
    };
    let user = if user.user_name().is_none() {
        AccountState::Protected(Box::new(
            read_protected_user(c, id)
                .await?
                .ok_or(StoreError::CorruptStoredData)?,
        ))
    } else {
        AccountState::Plain(Box::new(user))
    };
    let revision = revision(c, id).await?;
    let mut rows=c.query(super::schema::collection_lookahead_sql!("SELECT role_id FROM effect_identity_user_roles WHERE user_id=?1 ORDER BY role_id LIMIT ",""),(id.to_string(),)).await.map_err(provider_category)?;
    let mut roles = Vec::new();
    while let Some(row) = rows.next().await.map_err(provider_category)? {
        if roles.len() >= super::schema::NATIVE_COLLECTION_LIMIT {
            return Err(StoreError::CorruptStoredData);
        }
        roles.push(RoleId::parse(row_text(&row)?).map_err(|_| StoreError::CorruptStoredData)?);
    }
    let mut rows=c.query(super::schema::collection_lookahead_sql!("SELECT claim_type,claim_value FROM effect_identity_user_claims WHERE user_id=?1 ORDER BY id LIMIT ",""),(id.to_string(),)).await.map_err(provider_category)?;
    let claims = read_claims(&mut rows).await?;
    let mut rows=c.query(super::schema::collection_lookahead_sql!("SELECT login_provider,provider_key,provider_display_name FROM effect_identity_user_logins WHERE user_id=?1 ORDER BY login_provider,provider_key LIMIT ",""),(id.to_string(),)).await.map_err(provider_category)?;
    let logins = read_logins(&mut rows).await?;
    let mut rows=c.query("SELECT login_provider,name,value FROM effect_identity_user_tokens WHERE user_id=?1 AND value IS NOT NULL ORDER BY login_provider,name LIMIT ?2",(id.to_string(),i64::try_from(token_capacity).map_err(|_|StoreError::ConstraintViolation)?.checked_add(1).ok_or(StoreError::ConstraintViolation)?)).await.map_err(provider_category)?;
    let mut tokens = Vec::new();
    while let Some(row) = rows.next().await.map_err(provider_category)? {
        if tokens.len() >= token_capacity {
            return Err(StoreError::CorruptStoredData);
        }
        let value: Option<String> = row.try_get(2).map_err(|_| StoreError::CorruptStoredData)?;
        if let Some(value) = value {
            tokens.push(AccountToken {
                provider: LoginProvider::parse(row_text_at(&row, 0)?)
                    .map_err(|_| StoreError::CorruptStoredData)?,
                name: AuthTokenName::parse(row_text_at(&row, 1)?)
                    .map_err(|_| StoreError::CorruptStoredData)?,
                value: AuthTokenValue::parse(value).map_err(|_| StoreError::CorruptStoredData)?,
            });
        }
    }
    let mut rows=c.query(super::schema::recovery_lookahead_sql!("SELECT code_hash FROM effect_identity_user_recovery_code_hashes WHERE user_id=?1 ORDER BY id LIMIT ",""),(id.to_string(),)).await.map_err(provider_category)?;
    let mut recovery_hashes = Vec::new();
    while let Some(row) = rows.next().await.map_err(provider_category)? {
        if recovery_hashes.len() >= super::schema::NATIVE_RECOVERY_LIMIT {
            return Err(StoreError::CorruptStoredData);
        }
        recovery_hashes.push(deserialize_recovery_code_hash(&row_text(&row)?)?);
    }
    let mut rows=c.query(super::schema::collection_lookahead_sql!("SELECT payload FROM effect_identity_user_passkeys WHERE user_id=?1 ORDER BY credential_id LIMIT ",""),(id.to_string(),)).await.map_err(provider_category)?;
    let passkeys = read_user_passkeys(&mut rows, id).await?;
    Ok(Some(VersionedAccount {
        revision,
        snapshot: AccountSnapshot {
            user,
            roles,
            claims,
            logins,
            tokens,
            recovery_hashes,
            passkeys,
        },
    }))
}
/// Remove only this owner's credential/authorization children before a checked replacement.
async fn delete_children(c: &StoreConnection<'_>, id: &UserId) -> Result<(), StoreError> {
    for text in [
        "DELETE FROM effect_identity_user_roles WHERE user_id=?1",
        "DELETE FROM effect_identity_user_claims WHERE user_id=?1",
        "DELETE FROM effect_identity_user_logins WHERE user_id=?1",
        "DELETE FROM effect_identity_user_tokens WHERE user_id=?1",
        "DELETE FROM effect_identity_user_recovery_code_hashes WHERE user_id=?1",
        "DELETE FROM effect_identity_user_passkeys WHERE user_id=?1",
    ] {
        c.execute(text, (id.to_string(),))
            .await
            .map_err(provider_category)?;
    }
    Ok(())
}
/// Insert the validated bounded relation set inside the caller's existing transaction.
/// Database uniqueness/FKs protect aliases and stable membership identity across clients.
async fn write_children(
    c: &StoreConnection<'_>,
    snapshot: &AccountSnapshot,
) -> Result<(), StoreError> {
    let id = snapshot.user.metadata().id();
    for role in &snapshot.roles {
        ensure_role_exists(c, role).await?;
        c.execute(
            "INSERT INTO effect_identity_user_roles(user_id,role_id) VALUES(?1,?2)",
            (id.to_string(), role.to_string()),
        )
        .await
        .map_err(provider_category)?;
    }
    for claim in &snapshot.claims {
        c.execute("INSERT INTO effect_identity_user_claims(user_id,claim_type,claim_value) VALUES(?1,?2,?3)",(id.to_string(),claim.claim_type().as_str(),claim.claim_value().as_str())).await.map_err(provider_category)?;
    }
    for login in &snapshot.logins {
        c.execute("INSERT INTO effect_identity_user_logins(user_id,login_provider,provider_key,provider_display_name) VALUES(?1,?2,?3,?4)",(id.to_string(),login.login_provider().as_str(),login.provider_key().as_str(),login.provider_display_name().map(str::to_owned))).await.map_err(provider_category)?;
    }
    for token in &snapshot.tokens {
        c.execute("INSERT INTO effect_identity_user_tokens(user_id,login_provider,name,value) VALUES(?1,?2,?3,?4)",(id.to_string(),token.provider.as_str(),token.name.as_str(),token.value.as_str())).await.map_err(provider_category)?;
    }
    for hash in &snapshot.recovery_hashes {
        c.execute("INSERT INTO effect_identity_user_recovery_code_hashes(user_id,code_hash) VALUES(?1,?2)",(id.to_string(),serialize_recovery_code_hash(hash)?)).await.map_err(provider_category)?;
    }
    for passkey in &snapshot.passkeys {
        c.execute("INSERT INTO effect_identity_user_passkeys(user_id,credential_id,payload) VALUES(?1,?2,?3)",(id.to_string(),passkey.credential_id().as_bytes(),super::native::serialize_expected_passkey(id,passkey)?)).await.map_err(provider_category)?;
    }
    Ok(())
}
