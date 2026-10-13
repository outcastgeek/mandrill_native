//! SQLx implementations of the pinned native Identity store contracts.
//! Each mutation owns its acknowledged transaction; native manager workflows can
//! still compose multiple separately committed operations. Persistence JSON is a
//! private versioned security DTO, never a row conversion or public view codec.

use super::schema::{collection_limit_sql, collection_lookahead_sql, recovery_lookahead_sql};
use identity_rs::identity::{
    StoreError,
    models::{
        AuthTokenName, AuthTokenValue, AuthenticatorKey, ConcurrencyStamp, EmailAddress,
        IdentityClaim, IdentityRole, IdentityUser, LoginProvider, NewRole, NewUser,
        NormalizedEmailAddress, NormalizedUserName, PasskeyChallenge, PasskeyChallengePurpose,
        PasskeyChallengeState, PasskeyCredentialId, PhoneNumber, ProviderKey, RecoveryCode,
        RecoveryCodeHash, RoleId, RoleUpdate, UserId, UserLoginInfo, UserPasskeyInfo, UserUpdate,
    },
    store::{
        PasskeyChallengeStore, QueryableRoleStore, QueryableUserStore, RoleClaimStore, RoleStore,
        UserAuthenticationTokenStore, UserAuthenticatorKeyStore, UserClaimStore, UserEmailStore,
        UserLockoutStore, UserLoginStore, UserPasskeyStore, UserPhoneNumberStore, UserRoleStore,
        UserStore, UserTwoFactorRecoveryCodeStore, UserTwoFactorStore,
    },
};
use time::OffsetDateTime;

use super::error::provider_category;
use super::executor::{StoreConnection, StoreRows};
use effect_sql::{SqlError, kernel::NativeOperation};
use identity_rs::identity::store::{BrowserContext, BrowserContextStore, RolePage, UserPage};
use sqlx::Row;

/// Native authenticator token namespace at the pinned Identity revision.
const AUTHENTICATOR_STORE_LOGIN_PROVIDER: &str = "[identity_rs.authenticator_store]";
/// Native authenticator-key name, not an application policy default.
const AUTHENTICATOR_KEY_TOKEN_NAME: &str = "AuthenticatorKey";

use super::SqlxIdentityStore;

impl UserStore for SqlxIdentityStore {
    /// Local topology is checked from the selected kernel, including read-only rejection.
    fn security_writes_supported(&self) -> bool {
        self.kernel.require_authoritative().is_ok()
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn create_user(&self, new_user: NewUser) -> Result<IdentityUser, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            new_user: NewUser,
        }
        impl NativeOperation<IdentityUser> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<IdentityUser, SqlError>
            where
                Self: 'lease,
            {
                let Self { new_user } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<IdentityUser, StoreError> = async move {
        let require_unique = new_user.require_unique_email();
        let user = new_user.into_user();
        let normalized_user_name = user
            .normalized_user_name()
            .cloned()
            .ok_or(StoreError::CorruptStoredData)?;
        let normalized_email = user.normalized_email().map(ToString::to_string);
        let payload = serialize_user(&user)?;



        let mut protected = connection
            .query("SELECT COUNT(*) FROM effect_identity_protected_users", ())
            .await
            .map_err(provider_category)?;
        let count = protected
            .next()
            .await
            .map_err(provider_category)?
            .ok_or(StoreError::CorruptStoredData)?
            .try_get::<i64, _>(0)
            .map_err(|_| StoreError::CorruptStoredData)?;
        consume_rows(&mut protected).await?;
        if count > 0 {
            return Err(StoreError::ConstraintViolation);
        }

        validate_email_claim(&connection, &user, require_unique).await?;

        connection
            .execute(
                "
                INSERT INTO effect_identity_users (
                    id,
                    normalized_user_name,
                    normalized_email,
                    concurrency_stamp,
                    payload
                ) VALUES (?1, ?2, ?3, ?4, ?5)
                ",
                (
                    user.id().to_string(),
                    normalized_user_name.to_string(),
                    normalized_email,
                    user.concurrency_stamp().to_string(),
                    payload,
                ),
            )
            .await
            .map_err(provider_category)?;

        connection
            .execute(
                "INSERT INTO effect_identity_user_email_policy (user_id, require_unique) VALUES (?1, ?2)",
                (user.id().to_string(), i64::from(require_unique)),
            )
            .await
            .map_err(provider_category)?;

        Ok(user)

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { new_user })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn update_user(&self, update: UserUpdate) -> Result<IdentityUser, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            update: UserUpdate,
        }
        impl NativeOperation<IdentityUser> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<IdentityUser, SqlError>
            where
                Self: 'lease,
            {
                let Self { update } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<IdentityUser, StoreError> = async move {
        let updated_user = update.user();
        updated_user.validate().map_err(|_| StoreError::CorruptStoredData)?;
        let id = updated_user.id();
        let expected = update.expected_concurrency_stamp();
        let existing = find_user_by_id(&connection,&id).await?.ok_or(StoreError::NotFound)?;
        if existing.concurrency_stamp()!=expected { return Err(StoreError::TransactionConflict); }
        let normalized_name=updated_user.normalized_user_name().ok_or(StoreError::CorruptStoredData)?;
        validate_email_claim(&connection,updated_user,false).await?;
        let revision=ConcurrencyStamp::new();
        let payload=serialize_refreshed_user(updated_user,&revision)?;
        let changed=connection.execute("UPDATE effect_identity_users SET normalized_user_name=?1,normalized_email=?2,concurrency_stamp=?3,payload=?4 WHERE id=?5 AND concurrency_stamp=?6",
            (normalized_name.as_str(),updated_user.normalized_email().map(ToString::to_string),revision.to_string(),payload,id.to_string(),expected.to_string())).await.map_err(provider_category)?;
        ensure_rows_changed(changed,StoreError::TransactionConflict)?;
        find_user_by_id(&connection,&id).await?.ok_or(StoreError::CorruptStoredData)

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { update })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn delete_user(&self, id: &UserId) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    connection
                        .execute(
                            "DELETE FROM effect_identity_users WHERE id = ?1",
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn find_user_by_id(&self, id: &UserId) -> Result<Option<IdentityUser>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<Option<IdentityUser>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<IdentityUser>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<IdentityUser>, StoreError> =
                    async move { find_user_by_id(&connection, id).await }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn find_user_by_normalized_name(
        &self,
        normalized_name: &NormalizedUserName,
    ) -> Result<Option<IdentityUser>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            normalized_name: &'input NormalizedUserName,
        }
        impl<'input> NativeOperation<Option<IdentityUser>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<IdentityUser>, SqlError>
            where
                Self: 'lease,
            {
                let Self { normalized_name } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<IdentityUser>, StoreError> = async move {
                    let mut rows = connection
            .query(
                "SELECT payload FROM effect_identity_users WHERE normalized_user_name = ?1",
                (normalized_name.to_string(),),
            )
            .await
            .map_err(provider_category)?;

                    read_optional_user(&mut rows).await
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { normalized_name })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn find_user_by_normalized_email_for_validation(
        &self,
        normalized_email: &NormalizedEmailAddress,
    ) -> Result<Option<IdentityUser>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            normalized_email: &'input NormalizedEmailAddress,
        }
        impl<'input> NativeOperation<Option<IdentityUser>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<IdentityUser>, SqlError>
            where
                Self: 'lease,
            {
                let Self { normalized_email } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<IdentityUser>, StoreError> = async move {
                    let mut rows = connection
            .query(
                "SELECT payload FROM effect_identity_users WHERE normalized_email = ?1 LIMIT 2",
                (normalized_email.to_string(),),
            )
            .await
            .map_err(provider_category)?;

                    read_unique_user(&mut rows).await
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { normalized_email })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl QueryableUserStore for SqlxIdentityStore {
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn list_users_page(
        &self,
        after: Option<UserId>,
        limit: std::num::NonZeroU16,
    ) -> Result<UserPage, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            after: Option<UserId>,
            /// Native method input retained by borrow or moved exactly once.
            limit: std::num::NonZeroU16,
        }
        impl NativeOperation<UserPage> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<UserPage, SqlError>
            where
                Self: 'lease,
            {
                let Self { after, limit } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<UserPage, StoreError> = async move {
                    if usize::from(limit.get()) > super::schema::NATIVE_COLLECTION_LIMIT {
                        return Err(StoreError::ConstraintViolation);
                    }

                    let mut rows = connection
            .query(
                "SELECT payload FROM effect_identity_users WHERE id > ?1 ORDER BY id LIMIT ?2",
                (
                    after.map_or_else(String::new, |id| id.to_string()),
                    i64::from(limit.get()) + 1,
                ),
            )
            .await
            .map_err(provider_category)?;
                    let mut users = read_users(&mut rows).await?;
                    let next = if users.len() > usize::from(limit.get()) {
                        users.pop();
                        users.last().map(IdentityUser::id)
                    } else {
                        None
                    };
                    UserPage::new(users, next)
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { after, limit })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl UserEmailStore for SqlxIdentityStore {
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_email(&self, id: &UserId) -> Result<Option<EmailAddress>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<Option<EmailAddress>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<EmailAddress>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<EmailAddress>, StoreError> = async move {
                    let user = find_user_by_id(&connection, id)
                        .await?
                        .ok_or(StoreError::NotFound)?;

                    Ok(user.email().cloned())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Uses the selected native account/token operation; no hidden provider is acquired.
    async fn set_email(
        &self,
        id: &UserId,
        email: Option<EmailAddress>,
    ) -> Result<IdentityUser, StoreError> {
        self.update_user_contact(id, |user| Ok(user.with_email(email)))
            .await
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_normalized_email(
        &self,
        id: &UserId,
    ) -> Result<Option<NormalizedEmailAddress>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<Option<NormalizedEmailAddress>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<NormalizedEmailAddress>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<NormalizedEmailAddress>, StoreError> = async move {
                    let user = find_user_by_id(&connection, id)
                        .await?
                        .ok_or(StoreError::NotFound)?;

                    Ok(user.normalized_email().cloned())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn find_user_by_normalized_email(
        &self,
        normalized_email: &NormalizedEmailAddress,
    ) -> Result<Option<IdentityUser>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            normalized_email: &'input NormalizedEmailAddress,
        }
        impl<'input> NativeOperation<Option<IdentityUser>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<IdentityUser>, SqlError>
            where
                Self: 'lease,
            {
                let Self { normalized_email } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<IdentityUser>, StoreError> = async move {
                    let mut rows = connection
            .query(
                "SELECT payload FROM effect_identity_users WHERE normalized_email = ?1 LIMIT 2",
                (normalized_email.to_string(),),
            )
            .await
            .map_err(provider_category)?;

                    read_unique_user(&mut rows).await
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { normalized_email })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Uses the selected native account/token operation; no hidden provider is acquired.
    async fn set_normalized_email(
        &self,
        id: &UserId,
        normalized_email: Option<NormalizedEmailAddress>,
    ) -> Result<IdentityUser, StoreError> {
        self.update_user_contact(id, |user| {
            user.with_normalized_email(normalized_email)
                .map_err(|_| StoreError::CorruptStoredData)
        })
        .await
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_email_confirmed(&self, id: &UserId) -> Result<bool, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<bool> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<bool, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<bool, StoreError> = async move {
                    let user = find_user_by_id(&connection, id)
                        .await?
                        .ok_or(StoreError::NotFound)?;

                    Ok(user.email_confirmed())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Uses the selected native account/token operation; no hidden provider is acquired.
    async fn set_email_confirmed(
        &self,
        id: &UserId,
        email_confirmed: bool,
    ) -> Result<IdentityUser, StoreError> {
        self.update_user_contact(id, |user| {
            user.with_email_confirmed(email_confirmed)
                .map_err(|_| StoreError::CorruptStoredData)
        })
        .await
    }
}
impl UserPhoneNumberStore for SqlxIdentityStore {
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_phone_number(&self, id: &UserId) -> Result<Option<PhoneNumber>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<Option<PhoneNumber>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<PhoneNumber>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<PhoneNumber>, StoreError> = async move {
                    let user = find_user_by_id(&connection, id)
                        .await?
                        .ok_or(StoreError::NotFound)?;

                    Ok(user.phone_number().cloned())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Uses the selected native account/token operation; no hidden provider is acquired.
    async fn set_phone_number(
        &self,
        id: &UserId,
        phone_number: Option<PhoneNumber>,
    ) -> Result<IdentityUser, StoreError> {
        self.update_user_contact(id, |user| Ok(user.with_phone_number(phone_number)))
            .await
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_phone_number_confirmed(&self, id: &UserId) -> Result<bool, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<bool> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<bool, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<bool, StoreError> = async move {
                    let user = find_user_by_id(&connection, id)
                        .await?
                        .ok_or(StoreError::NotFound)?;

                    Ok(user.phone_number_confirmed())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Uses the selected native account/token operation; no hidden provider is acquired.
    async fn set_phone_number_confirmed(
        &self,
        id: &UserId,
        phone_number_confirmed: bool,
    ) -> Result<IdentityUser, StoreError> {
        self.update_user_contact(id, |user| {
            user.with_phone_number_confirmed(phone_number_confirmed)
                .map_err(|_| StoreError::CorruptStoredData)
        })
        .await
    }
}
impl UserRoleStore for SqlxIdentityStore {
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn add_user_to_role(
        &self,
        id: &UserId,
        normalized_role_name: NormalizedUserName,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            normalized_role_name: NormalizedUserName,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    normalized_role_name,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {

        ensure_user_exists(&connection, id).await?;
        let role_id = find_role_id_by_normalized_name(&connection, &normalized_role_name)
            .await?
            .ok_or(StoreError::NotFound)?;

        let inserted = connection
            .execute(
                collection_limit_sql!("
                INSERT INTO effect_identity_user_roles (user_id, role_id)
                SELECT ?1, ?2 WHERE (SELECT COUNT(*) FROM effect_identity_user_roles WHERE user_id=?1)<","
                "),
                (id.to_string(), role_id.to_string()),
            )
            .await
            .map_err(provider_category)?;
        if inserted != 1 {
            return Err(StoreError::ConstraintViolation);
        }

        Ok(())

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                id,
                normalized_role_name,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn remove_user_from_role(
        &self,
        id: &UserId,
        normalized_role_name: &NormalizedUserName,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            normalized_role_name: &'input NormalizedUserName,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    normalized_role_name,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    let Some(role_id) =
                        find_role_id_by_normalized_name(&connection, normalized_role_name).await?
                    else {
                        return Ok(());
                    };

                    connection
                        .execute(
                            "
                DELETE FROM effect_identity_user_roles
                 WHERE user_id = ?1
                   AND role_id = ?2
                ",
                            (id.to_string(), role_id.to_string()),
                        )
                        .await
                        .map_err(provider_category)?;

                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                id,
                normalized_role_name,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_user_roles(&self, id: &UserId) -> Result<Vec<NormalizedUserName>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<Vec<NormalizedUserName>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Vec<NormalizedUserName>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Vec<NormalizedUserName>, StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;

                    let mut rows = connection
                        .query(
                            collection_lookahead_sql!(
                                "
                SELECT r.normalized_name
                  FROM effect_identity_user_roles ur
                  JOIN effect_identity_roles r ON r.id = ur.role_id
                 WHERE ur.user_id = ?1
                 ORDER BY r.normalized_name LIMIT ",
                                "
                "
                            ),
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    let mut roles = Vec::new();
                    while let Some(row) = rows.next().await.map_err(provider_category)? {
                        if roles.len() >= super::schema::NATIVE_COLLECTION_LIMIT {
                            return Err(StoreError::CorruptStoredData);
                        }
                        roles.push(
                            NormalizedUserName::parse(row_text_at(&row, 0)?)
                                .map_err(|_| StoreError::CorruptStoredData)?,
                        );
                    }

                    Ok(roles)
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn is_user_in_role(
        &self,
        id: &UserId,
        normalized_role_name: &NormalizedUserName,
    ) -> Result<bool, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            normalized_role_name: &'input NormalizedUserName,
        }
        impl<'input> NativeOperation<bool> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<bool, SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    normalized_role_name,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<bool, StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;

                    let mut rows = connection
                        .query(
                            "
                SELECT ur.user_id
                  FROM effect_identity_user_roles ur
                  JOIN effect_identity_roles r ON r.id = ur.role_id
                 WHERE ur.user_id = ?1
                   AND r.normalized_name = ?2
                 LIMIT 1
                ",
                            (id.to_string(), normalized_role_name.to_string()),
                        )
                        .await
                        .map_err(provider_category)?;

                    rows.next()
                        .await
                        .map(|row| row.is_some())
                        .map_err(provider_category)
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation {
                id,
                normalized_role_name,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_users_in_role_page(
        &self,
        normalized_role_name: &NormalizedUserName,
        after: Option<identity_rs::identity::models::UserId>,
        limit: std::num::NonZeroU16,
    ) -> Result<identity_rs::identity::store::UserPage, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            normalized_role_name: &'input NormalizedUserName,
            /// Native method input retained by borrow or moved exactly once.
            after: Option<identity_rs::identity::models::UserId>,
            /// Native method input retained by borrow or moved exactly once.
            limit: std::num::NonZeroU16,
        }
        impl<'input> NativeOperation<identity_rs::identity::store::UserPage> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<identity_rs::identity::store::UserPage, SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    normalized_role_name,
                    after,
                    limit,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<identity_rs::identity::store::UserPage, StoreError> = async move {
        if usize::from(limit.get()) > super::schema::NATIVE_COLLECTION_LIMIT {
            return Err(StoreError::ConstraintViolation);
        }

        let mut rows = connection.query("SELECT u.payload FROM effect_identity_user_roles ur JOIN effect_identity_roles r ON r.id=ur.role_id JOIN effect_identity_users u ON u.id=ur.user_id WHERE r.normalized_name=?1 AND u.id > ?2 ORDER BY u.id LIMIT ?3", (normalized_role_name.to_string(), after.map_or_else(String::new, |id| id.to_string()), i64::from(limit.get())+1)).await.map_err(provider_category)?;
        let mut users = read_users(&mut rows).await?;
        let next = if users.len() > usize::from(limit.get()) {
            users.pop();
            users.last().map(IdentityUser::id)
        } else {
            None
        };
        UserPage::new(users, next)

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation {
                normalized_role_name,
                after,
                limit,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl UserClaimStore for SqlxIdentityStore {
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_user_claims(&self, id: &UserId) -> Result<Vec<IdentityClaim>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<Vec<IdentityClaim>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Vec<IdentityClaim>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Vec<IdentityClaim>, StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    let mut rows = connection
                        .query(
                            collection_lookahead_sql!(
                                "
                SELECT claim_type, claim_value
                  FROM effect_identity_user_claims
                 WHERE user_id = ?1
                 ORDER BY id LIMIT ",
                                "
                "
                            ),
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    read_claims(&mut rows).await
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn add_user_claims(
        &self,
        id: &UserId,
        claims: Vec<IdentityClaim>,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            claims: Vec<IdentityClaim>,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id, claims } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    if claims.len() > super::schema::NATIVE_COLLECTION_LIMIT {
                        return Err(StoreError::ConstraintViolation);
                    }

                    ensure_user_exists(&connection, id).await?;

                    let mut count_rows = connection
                        .query(
                            "SELECT COUNT(*) FROM effect_identity_user_claims WHERE user_id=?1",
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;
                    let count = count_rows
                        .next()
                        .await
                        .map_err(provider_category)?
                        .ok_or(StoreError::CorruptStoredData)?
                        .try_get::<i64, _>(0)
                        .map_err(provider_category)?;
                    consume_rows(&mut count_rows).await?;
                    if !(0..=super::schema::NATIVE_COLLECTION_LIMIT as i64).contains(&count)
                        || usize::try_from(count).map_err(|_| StoreError::CorruptStoredData)?
                            + claims.len()
                            > super::schema::NATIVE_COLLECTION_LIMIT
                    {
                        return Err(StoreError::ConstraintViolation);
                    }
                    for claim in claims {
                        insert_user_claim(&connection, id, &claim).await?;
                    }

                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id, claims })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn replace_user_claim(
        &self,
        id: &UserId,
        claim: &IdentityClaim,
        replacement: IdentityClaim,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            claim: &'input IdentityClaim,
            /// Native method input retained by borrow or moved exactly once.
            replacement: IdentityClaim,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    claim,
                    replacement,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    connection
                        .execute(
                            "
                UPDATE effect_identity_user_claims
                   SET claim_type = ?1,
                       claim_value = ?2
                 WHERE user_id = ?3
                   AND claim_type = ?4
                   AND claim_value = ?5
                ",
                            (
                                replacement.claim_type().to_string(),
                                replacement.claim_value().to_string(),
                                id.to_string(),
                                claim.claim_type().to_string(),
                                claim.claim_value().to_string(),
                            ),
                        )
                        .await
                        .map_err(provider_category)?;

                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                id,
                claim,
                replacement,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn remove_user_claims(
        &self,
        id: &UserId,
        claims: &[IdentityClaim],
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            claims: &'input [IdentityClaim],
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id, claims } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    if claims.len() > super::schema::NATIVE_COLLECTION_LIMIT {
                        return Err(StoreError::ConstraintViolation);
                    }

                    ensure_user_exists(&connection, id).await?;

                    for claim in claims {
                        connection
                            .execute(
                                "
                    DELETE FROM effect_identity_user_claims
                     WHERE user_id = ?1
                       AND claim_type = ?2
                       AND claim_value = ?3
                    ",
                                (
                                    id.to_string(),
                                    claim.claim_type().to_string(),
                                    claim.claim_value().to_string(),
                                ),
                            )
                            .await
                            .map_err(provider_category)?;
                    }

                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id, claims })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_users_for_claim_page(
        &self,
        claim: &IdentityClaim,
        after: Option<identity_rs::identity::models::UserId>,
        limit: std::num::NonZeroU16,
    ) -> Result<identity_rs::identity::store::UserPage, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            claim: &'input IdentityClaim,
            /// Native method input retained by borrow or moved exactly once.
            after: Option<identity_rs::identity::models::UserId>,
            /// Native method input retained by borrow or moved exactly once.
            limit: std::num::NonZeroU16,
        }
        impl<'input> NativeOperation<identity_rs::identity::store::UserPage> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<identity_rs::identity::store::UserPage, SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    claim,
                    after,
                    limit,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<identity_rs::identity::store::UserPage, StoreError> = async move {
        if usize::from(limit.get()) > super::schema::NATIVE_COLLECTION_LIMIT {
            return Err(StoreError::ConstraintViolation);
        }

        let mut rows = connection.query("SELECT DISTINCT u.payload, u.id FROM effect_identity_user_claims uc JOIN effect_identity_users u ON u.id=uc.user_id WHERE uc.claim_type=?1 AND uc.claim_value=?2 AND u.id > ?3 ORDER BY u.id LIMIT ?4", (claim.claim_type().to_string(), claim.claim_value().to_string(), after.map_or_else(String::new, |id| id.to_string()), i64::from(limit.get())+1)).await.map_err(provider_category)?;
        let mut users = read_users(&mut rows).await?;
        let next = if users.len() > usize::from(limit.get()) {
            users.pop();
            users.last().map(IdentityUser::id)
        } else {
            None
        };
        UserPage::new(users, next)

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation {
                claim,
                after,
                limit,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl UserLoginStore for SqlxIdentityStore {
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn add_user_login(&self, id: &UserId, login: UserLoginInfo) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            login: UserLoginInfo,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id, login } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {

        ensure_user_exists(&connection, id).await?;
        let inserted = connection
            .execute(
                collection_limit_sql!("
                INSERT INTO effect_identity_user_logins (
                    user_id,
                    login_provider,
                    provider_key,
                    provider_display_name
                ) SELECT ?1, ?2, ?3, ?4 WHERE (SELECT COUNT(*) FROM effect_identity_user_logins WHERE user_id=?1)<","
                "),
                (
                    id.to_string(),
                    login.login_provider().to_string(),
                    login.provider_key().to_string(),
                    login.provider_display_name().map(ToOwned::to_owned),
                ),
            )
            .await
            .map_err(provider_category)?;
        if inserted != 1 {
            return Err(StoreError::ConstraintViolation);
        }

        Ok(())

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id, login })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn remove_user_login(
        &self,
        id: &UserId,
        login_provider: &LoginProvider,
        provider_key: &ProviderKey,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            login_provider: &'input LoginProvider,
            /// Native method input retained by borrow or moved exactly once.
            provider_key: &'input ProviderKey,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    login_provider,
                    provider_key,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    let changed = connection
                        .execute(
                            "
                DELETE FROM effect_identity_user_logins
                 WHERE user_id = ?1
                   AND login_provider = ?2
                   AND provider_key = ?3
                ",
                            (
                                id.to_string(),
                                login_provider.to_string(),
                                provider_key.to_string(),
                            ),
                        )
                        .await
                        .map_err(provider_category)?;

                    if changed != 0 {
                        rotate_security_stamp(&connection, id, false).await?;
                    }

                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                id,
                login_provider,
                provider_key,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_user_logins(&self, id: &UserId) -> Result<Vec<UserLoginInfo>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<Vec<UserLoginInfo>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Vec<UserLoginInfo>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Vec<UserLoginInfo>, StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    let mut rows = connection
                        .query(
                            collection_lookahead_sql!(
                                "
                SELECT login_provider, provider_key, provider_display_name
                  FROM effect_identity_user_logins
                 WHERE user_id = ?1
                 ORDER BY login_provider, provider_key LIMIT ",
                                "
                "
                            ),
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    read_logins(&mut rows).await
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn find_user_by_login(
        &self,
        login_provider: &LoginProvider,
        provider_key: &ProviderKey,
    ) -> Result<Option<IdentityUser>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            login_provider: &'input LoginProvider,
            /// Native method input retained by borrow or moved exactly once.
            provider_key: &'input ProviderKey,
        }
        impl<'input> NativeOperation<Option<IdentityUser>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<IdentityUser>, SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    login_provider,
                    provider_key,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<IdentityUser>, StoreError> = async move {
                    let mut rows = connection
                        .query(
                            "
                SELECT u.payload
                  FROM effect_identity_user_logins ul
                  JOIN effect_identity_users u ON u.id = ul.user_id
                 WHERE ul.login_provider = ?1
                   AND ul.provider_key = ?2
                ",
                            (login_provider.to_string(), provider_key.to_string()),
                        )
                        .await
                        .map_err(provider_category)?;

                    read_optional_user(&mut rows).await
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation {
                login_provider,
                provider_key,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl UserAuthenticationTokenStore for SqlxIdentityStore {
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_authentication_token(
        &self,
        id: &UserId,
        login_provider: &LoginProvider,
        name: &AuthTokenName,
    ) -> Result<Option<AuthTokenValue>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            login_provider: &'input LoginProvider,
            /// Native method input retained by borrow or moved exactly once.
            name: &'input AuthTokenName,
        }
        impl<'input> NativeOperation<Option<AuthTokenValue>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<AuthTokenValue>, SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    login_provider,
                    name,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<AuthTokenValue>, StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    let mut rows = connection
                        .query(
                            "
                SELECT value
                  FROM effect_identity_user_tokens
                 WHERE user_id = ?1
                   AND login_provider = ?2
                   AND name = ?3
                ",
                            (id.to_string(), login_provider.to_string(), name.to_string()),
                        )
                        .await
                        .map_err(provider_category)?;

                    let Some(row) = rows.next().await.map_err(provider_category)? else {
                        return Ok(None);
                    };

                    let value = optional_row_text_at(&row, 0)?
                        .map(AuthTokenValue::parse)
                        .transpose()
                        .map_err(|_| StoreError::CorruptStoredData)?;
                    consume_rows(&mut rows).await?;

                    Ok(value)
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation {
                id,
                login_provider,
                name,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn set_authentication_token(
        &self,
        id: &UserId,
        login_provider: LoginProvider,
        name: AuthTokenName,
        value: Option<AuthTokenValue>,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            login_provider: LoginProvider,
            /// Native method input retained by borrow or moved exactly once.
            name: AuthTokenName,
            /// Native method input retained by borrow or moved exactly once.
            value: Option<AuthTokenValue>,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    login_provider,
                    name,
                    value,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {

        ensure_user_exists(&connection, id).await?;
        let Some(value)=value else {
            connection.execute("DELETE FROM effect_identity_user_tokens WHERE user_id=?1 AND login_provider=?2 AND name=?3",(id.to_string(),login_provider.as_str(),name.as_str())).await.map_err(provider_category)?;
            return Ok(());
        };
        connection
            .execute(
                "
                INSERT INTO effect_identity_user_tokens (
                    user_id,
                    login_provider,
                    name,
                    value
                ) VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(user_id, login_provider, name)
                DO UPDATE SET value = excluded.value
                ",
                (
                    id.to_string(),
                    login_provider.to_string(),
                    name.to_string(),
                    value.to_string(),
                ),
            )
            .await
            .map_err(provider_category)?;

        Ok(())

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                id,
                login_provider,
                name,
                value,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn replace_authentication_token_if_matches(
        &self,
        id: &UserId,
        login_provider: &LoginProvider,
        name: &AuthTokenName,
        expected_value: &AuthTokenValue,
        new_value: AuthTokenValue,
    ) -> Result<bool, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            login_provider: &'input LoginProvider,
            /// Native method input retained by borrow or moved exactly once.
            name: &'input AuthTokenName,
            /// Native method input retained by borrow or moved exactly once.
            expected_value: &'input AuthTokenValue,
            /// Native method input retained by borrow or moved exactly once.
            new_value: AuthTokenValue,
        }
        impl<'input> NativeOperation<bool> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<bool, SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    login_provider,
                    name,
                    expected_value,
                    new_value,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<bool, StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    let affected_rows = connection
                        .execute(
                            "
                UPDATE effect_identity_user_tokens
                   SET value = ?1
                 WHERE user_id = ?2
                   AND login_provider = ?3
                   AND name = ?4
                   AND value = ?5
                ",
                            (
                                new_value.to_string(),
                                id.to_string(),
                                login_provider.to_string(),
                                name.to_string(),
                                expected_value.to_string(),
                            ),
                        )
                        .await
                        .map_err(provider_category)?;

                    Ok(affected_rows > 0)
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                id,
                login_provider,
                name,
                expected_value,
                new_value,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn remove_authentication_token(
        &self,
        id: &UserId,
        login_provider: &LoginProvider,
        name: &AuthTokenName,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            login_provider: &'input LoginProvider,
            /// Native method input retained by borrow or moved exactly once.
            name: &'input AuthTokenName,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    login_provider,
                    name,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    connection
                        .execute(
                            "
                DELETE FROM effect_identity_user_tokens
                 WHERE user_id = ?1
                   AND login_provider = ?2
                   AND name = ?3
                ",
                            (id.to_string(), login_provider.to_string(), name.to_string()),
                        )
                        .await
                        .map_err(provider_category)?;

                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                id,
                login_provider,
                name,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl UserPasskeyStore for SqlxIdentityStore {
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn add_or_update_passkey(
        &self,
        id: &UserId,
        passkey: UserPasskeyInfo,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            passkey: UserPasskeyInfo,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id, passkey } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
        passkey.validate()?;


        ensure_user_exists(&connection, id).await?;

        let credential_bytes = passkey.credential_id().as_bytes().to_vec();
        let mut rows = connection
            .query(
                "
                SELECT payload
                  FROM effect_identity_user_passkeys
                 WHERE credential_id = ?1
                ",
                (credential_bytes.as_slice(),),
            )
            .await
            .map_err(provider_category)?;
        let existing = read_optional_user_passkey(&mut rows).await?;

        if let Some(existing) = existing {
            if existing.user_id() != *id {
                return Err(StoreError::ConstraintViolation);
            }

            let expected_payload = serialize_user_passkey(&existing)?;
            let existing = existing.with_display_from_info(&passkey);
            let payload = serialize_user_passkey(&existing)?;
            let changed = connection
                .execute(
                    "UPDATE effect_identity_user_passkeys SET payload = ?2 WHERE credential_id = ?1 AND payload = ?3",
                    (credential_bytes, payload, expected_payload),
                )
                .await
                .map_err(provider_category)?;
            if changed != 1 {
                return Err(StoreError::TransactionConflict);
            }

            rotate_security_stamp(&connection, id, false).await?;
            return Ok(());
        }

        let user_passkey = StoredPasskey::new(*id, passkey);
        let payload = serialize_user_passkey(&user_passkey)?;
        let inserted = connection
            .execute(
                collection_limit_sql!("
                INSERT INTO effect_identity_user_passkeys (
                    user_id,
                    credential_id,
                    payload
                ) SELECT ?1, ?2, ?3 WHERE (SELECT COUNT(*) FROM effect_identity_user_passkeys WHERE user_id=?1)<","
                "),
                (id.to_string(), credential_bytes, payload),
            )
            .await
            .map_err(provider_category)?;
        if inserted != 1 {
            return Err(StoreError::ConstraintViolation);
        }

        rotate_security_stamp(&connection, id, false).await?;
        Ok(())

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id, passkey })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn update_passkey_if_matches(
        &self,
        id: &UserId,
        expected: &UserPasskeyInfo,
        updated: UserPasskeyInfo,
    ) -> Result<bool, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            expected: &'input UserPasskeyInfo,
            /// Native method input retained by borrow or moved exactly once.
            updated: UserPasskeyInfo,
        }
        impl<'input> NativeOperation<bool> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<bool, SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    expected,
                    updated,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<bool, StoreError> = async move {
        updated.validate()?;
        let updated = if updated.revision() == expected.revision() {
            let name = updated.name().cloned();
            updated.with_name(name)
        } else {
            updated
        };
        if expected.credential_id() != updated.credential_id() {
            return Err(StoreError::ConstraintViolation);
        }

        let old_payload = serialize_expected_passkey(*id, expected)?;
        let new_payload = serialize_user_passkey(&StoredPasskey::new(*id, updated))?;
        let changed = connection.execute("UPDATE effect_identity_user_passkeys SET payload = ?1 WHERE user_id = ?2 AND credential_id = ?3 AND payload = ?4",
            (new_payload, id.to_string(), expected.credential_id().as_bytes(), old_payload)).await.map_err(provider_category)?;
        Ok(changed == 1)

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                id,
                expected,
                updated,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_passkey(
        &self,
        id: &UserId,
        credential_id: &PasskeyCredentialId,
    ) -> Result<Option<UserPasskeyInfo>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            credential_id: &'input PasskeyCredentialId,
        }
        impl<'input> NativeOperation<Option<UserPasskeyInfo>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<UserPasskeyInfo>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id, credential_id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<UserPasskeyInfo>, StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    let mut rows = connection
                        .query(
                            "
                SELECT payload
                  FROM effect_identity_user_passkeys
                 WHERE user_id = ?1
                   AND credential_id = ?2
                ",
                            (id.to_string(), credential_id.as_bytes()),
                        )
                        .await
                        .map_err(provider_category)?;

                    Ok(read_optional_user_passkey(&mut rows)
                        .await?
                        .map(|passkey| passkey.into_user_passkey_info()))
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id, credential_id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_passkeys(&self, id: &UserId) -> Result<Vec<UserPasskeyInfo>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<Vec<UserPasskeyInfo>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Vec<UserPasskeyInfo>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Vec<UserPasskeyInfo>, StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    let mut rows = connection
                        .query(
                            collection_lookahead_sql!(
                                "
                SELECT payload
                  FROM effect_identity_user_passkeys
                 WHERE user_id = ?1
                 ORDER BY credential_id LIMIT ",
                                "
                "
                            ),
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    read_user_passkeys(&mut rows, id).await
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn find_user_by_passkey_credential_id(
        &self,
        credential_id: &PasskeyCredentialId,
    ) -> Result<Option<IdentityUser>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            credential_id: &'input PasskeyCredentialId,
        }
        impl<'input> NativeOperation<Option<IdentityUser>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<IdentityUser>, SqlError>
            where
                Self: 'lease,
            {
                let Self { credential_id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<IdentityUser>, StoreError> = async move {
                    let mut rows = connection
                        .query(
                            "
                SELECT u.payload
                  FROM effect_identity_user_passkeys up
                  JOIN effect_identity_users u ON u.id = up.user_id
                 WHERE up.credential_id = ?1
                ",
                            (credential_id.as_bytes(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    read_optional_user(&mut rows).await
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { credential_id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn remove_passkey(
        &self,
        id: &UserId,
        credential_id: &PasskeyCredentialId,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            credential_id: &'input PasskeyCredentialId,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id, credential_id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    let changed = connection
                        .execute(
                            "
                DELETE FROM effect_identity_user_passkeys
                 WHERE user_id = ?1
                   AND credential_id = ?2
                ",
                            (id.to_string(), credential_id.as_bytes()),
                        )
                        .await
                        .map_err(provider_category)?;

                    if changed > 0 {
                        rotate_security_stamp(&connection, id, false).await?;
                    }
                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id, credential_id })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl PasskeyChallengeStore for SqlxIdentityStore {
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn store_passkey_challenge(
        &self,
        challenge_state: PasskeyChallengeState,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            challenge_state: PasskeyChallengeState,
            /// Explicit installed capacity and UTC callable.
            policy: super::IdentityStoragePolicy,
        }
        impl NativeOperation<()> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    challenge_state,
                    policy,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    challenge_state.validate()?;
                    let now = (policy.utc_now)();
                    if challenge_state.expires_at() <= now {
                        return Err(StoreError::ConstraintViolation);
                    }
                    connection
                        .execute(
                            "DELETE FROM effect_identity_passkey_challenges WHERE expires_at<=?1",
                            (now.unix_timestamp(),),
                        )
                        .await
                        .map_err(provider_category)?;
                    let mut count = connection
                        .query(
                            "SELECT COUNT(*) FROM effect_identity_passkey_challenges",
                            (),
                        )
                        .await
                        .map_err(provider_category)?;
                    let row = count
                        .next()
                        .await
                        .map_err(provider_category)?
                        .ok_or(StoreError::CorruptStoredData)?;
                    if usize::try_from(row_integer_at(&row, 0)?)
                        .map_err(|_| StoreError::CorruptStoredData)?
                        >= policy.challenge_capacity.get()
                    {
                        return Err(StoreError::ConstraintViolation);
                    }
                    let payload = serialize_passkey_challenge_state(&challenge_state)?;

                    connection
                        .execute(
                            "
                INSERT INTO effect_identity_passkey_challenges (
                    challenge,
                    purpose,
                    expires_at,
                    payload
                ) VALUES (?1, ?2, ?3, ?4)
                ",
                            (
                                challenge_state.challenge().as_bytes(),
                                passkey_challenge_purpose_text(challenge_state.purpose()),
                                challenge_state.expires_at().unix_timestamp(),
                                payload,
                            ),
                        )
                        .await
                        .map_err(provider_category)?;

                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                challenge_state,
                policy: self.policy,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn take_passkey_challenge(
        &self,
        challenge: &PasskeyChallenge,
        purpose: PasskeyChallengePurpose,
        now: OffsetDateTime,
    ) -> Result<Option<PasskeyChallengeState>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            challenge: &'input PasskeyChallenge,
            /// Native method input retained by borrow or moved exactly once.
            purpose: PasskeyChallengePurpose,
            /// Native method input retained by borrow or moved exactly once.
            now: OffsetDateTime,
        }
        impl<'input> NativeOperation<Option<PasskeyChallengeState>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<PasskeyChallengeState>, SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    challenge,
                    purpose,
                    now,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<PasskeyChallengeState>, StoreError> = async move {
                    let mut rows = connection
                        .query(
                            "
                SELECT payload
                  FROM effect_identity_passkey_challenges
                 WHERE challenge = ?1
                ",
                            (challenge.as_bytes(),),
                        )
                        .await
                        .map_err(provider_category)?;
                    let challenge_state =
                        match read_optional_passkey_challenge_state(&mut rows).await {
                            Ok(challenge_state) => challenge_state,
                            Err(error) => {
                                return Err(error);
                            }
                        };

                    connection
                        .execute(
                            "
                DELETE FROM effect_identity_passkey_challenges
                 WHERE challenge = ?1
                ",
                            (challenge.as_bytes(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    let Some(challenge_state) = challenge_state else {
                        return Ok(None);
                    };
                    if challenge_state.purpose() != purpose || challenge_state.is_expired(now) {
                        return Ok(None);
                    }

                    Ok(Some(challenge_state))
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                challenge,
                purpose,
                now,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn remove_expired_passkey_challenges(
        &self,
        now: OffsetDateTime,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            now: OffsetDateTime,
        }
        impl NativeOperation<()> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { now } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    connection
                        .execute(
                            "
                DELETE FROM effect_identity_passkey_challenges
                 WHERE expires_at <= ?1
                ",
                            (now.unix_timestamp(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { now })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl UserLockoutStore for SqlxIdentityStore {
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_lockout_end_date(
        &self,
        id: &UserId,
    ) -> Result<Option<OffsetDateTime>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<Option<OffsetDateTime>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<OffsetDateTime>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<OffsetDateTime>, StoreError> = async move {
                    let user = find_user_by_id(&connection, id)
                        .await?
                        .ok_or(StoreError::NotFound)?;

                    Ok(user.lockout_end())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Uses the selected native account/token operation; no hidden provider is acquired.
    async fn set_lockout_end_date(
        &self,
        id: &UserId,
        lockout_end: Option<OffsetDateTime>,
    ) -> Result<IdentityUser, StoreError> {
        self.update_user_contact(id, |user| Ok(user.with_lockout_end(lockout_end)))
            .await
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn increment_access_failed_count(&self, id: &UserId) -> Result<u32, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<u32> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<u32, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<u32, StoreError> = async move {
                    let user = match find_user_by_id(&connection, id).await {
                        Ok(Some(user)) => user,
                        Ok(None) => {
                            return Err(StoreError::NotFound);
                        }
                        Err(error) => {
                            return Err(error);
                        }
                    };
                    let expected = user.concurrency_stamp().clone();
                    let access_failed_count = user.access_failed_count().saturating_add(1);
                    let updated_user = user
                        .with_access_failed_count(access_failed_count)
                        .with_refreshed_concurrency_stamp();
                    save_user(&connection, &updated_user, &expected).await?;

                    Ok(access_failed_count)
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Uses the selected native account/token operation; no hidden provider is acquired.
    async fn reset_access_failed_count(&self, id: &UserId) -> Result<IdentityUser, StoreError> {
        self.update_user_contact(id, |user| Ok(user.with_access_failed_count(0)))
            .await
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_access_failed_count(&self, id: &UserId) -> Result<u32, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<u32> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<u32, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<u32, StoreError> = async move {
                    let user = find_user_by_id(&connection, id)
                        .await?
                        .ok_or(StoreError::NotFound)?;

                    Ok(user.access_failed_count())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_lockout_enabled(&self, id: &UserId) -> Result<bool, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<bool> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<bool, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<bool, StoreError> = async move {
                    let user = find_user_by_id(&connection, id)
                        .await?
                        .ok_or(StoreError::NotFound)?;

                    Ok(user.lockout_enabled())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Uses the selected native account/token operation; no hidden provider is acquired.
    async fn set_lockout_enabled(
        &self,
        id: &UserId,
        enabled: bool,
    ) -> Result<IdentityUser, StoreError> {
        self.update_user_contact(id, |user| Ok(user.with_lockout_enabled(enabled)))
            .await
    }
}
impl UserTwoFactorStore for SqlxIdentityStore {
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_two_factor_enabled(&self, id: &UserId) -> Result<bool, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<bool> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<bool, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<bool, StoreError> = async move {
                    let user = find_user_by_id(&connection, id)
                        .await?
                        .ok_or(StoreError::NotFound)?;

                    Ok(user.two_factor_enabled())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Uses the selected native account/token operation; no hidden provider is acquired.
    async fn set_two_factor_enabled(
        &self,
        id: &UserId,
        enabled: bool,
    ) -> Result<IdentityUser, StoreError> {
        self.update_user_contact(id, |user| {
            Ok(user
                .with_two_factor_enabled(enabled)
                .with_refreshed_security_stamp())
        })
        .await
    }
}
impl UserAuthenticatorKeyStore for SqlxIdentityStore {
    /// Uses the selected native account/token operation; no hidden provider is acquired.
    async fn get_authenticator_key(
        &self,
        id: &UserId,
    ) -> Result<Option<AuthenticatorKey>, StoreError> {
        let provider = LoginProvider::parse(AUTHENTICATOR_STORE_LOGIN_PROVIDER)
            .map_err(|_| StoreError::CorruptStoredData)?;
        let name = AuthTokenName::parse(AUTHENTICATOR_KEY_TOKEN_NAME)
            .map_err(|_| StoreError::CorruptStoredData)?;

        self.get_authentication_token(id, &provider, &name)
            .await?
            .map(|value| AuthenticatorKey::parse(value.to_string()))
            .transpose()
            .map_err(|_| StoreError::CorruptStoredData)
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn set_authenticator_key(
        &self,
        id: &UserId,
        key: AuthenticatorKey,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            key: AuthenticatorKey,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id, key } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {


        ensure_user_exists(&connection, id).await?;
        let mut rows=connection.query("SELECT value FROM effect_identity_user_tokens WHERE user_id=?1 AND login_provider=?2 AND name=?3",(id.to_string(),AUTHENTICATOR_STORE_LOGIN_PROVIDER,AUTHENTICATOR_KEY_TOKEN_NAME)).await.map_err(provider_category)?;
        let replacing = rows.next().await.map_err(provider_category)?.is_some();
        consume_rows(&mut rows).await?;
        connection.execute("INSERT INTO effect_identity_user_tokens(user_id,login_provider,name,value) VALUES (?1,?2,?3,?4) ON CONFLICT(user_id,login_provider,name) DO UPDATE SET value=excluded.value",(id.to_string(),AUTHENTICATOR_STORE_LOGIN_PROVIDER,AUTHENTICATOR_KEY_TOKEN_NAME,key.to_string())).await.map_err(provider_category)?;
        if replacing {
            connection
                .execute(
                    "DELETE FROM effect_identity_user_recovery_code_hashes WHERE user_id=?1",
                    (id.to_string(),),
                )
                .await
                .map_err(provider_category)?;
            rotate_security_stamp(&connection, id, true).await?;
        }
        Ok(())

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id, key })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn reset_authenticator_key(&self, id: &UserId) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {


        ensure_user_exists(&connection, id).await?;
        connection.execute("DELETE FROM effect_identity_user_tokens WHERE user_id=?1 AND login_provider=?2 AND name=?3",(id.to_string(),AUTHENTICATOR_STORE_LOGIN_PROVIDER,AUTHENTICATOR_KEY_TOKEN_NAME)).await.map_err(provider_category)?;
        connection
            .execute(
                "DELETE FROM effect_identity_user_recovery_code_hashes WHERE user_id=?1",
                (id.to_string(),),
            )
            .await
            .map_err(provider_category)?;
        rotate_security_stamp(&connection, id, true).await?;
        Ok(())

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl UserTwoFactorRecoveryCodeStore for SqlxIdentityStore {
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn replace_recovery_codes(
        &self,
        id: &UserId,
        recovery_code_hashes: Vec<RecoveryCodeHash>,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            recovery_code_hashes: Vec<RecoveryCodeHash>,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    recovery_code_hashes,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    if recovery_code_hashes.len() > super::schema::NATIVE_RECOVERY_LIMIT {
                        return Err(StoreError::ConstraintViolation);
                    }
                    let serialized_hashes = recovery_code_hashes
                        .iter()
                        .map(serialize_recovery_code_hash)
                        .collect::<Result<Vec<_>, _>>()?;

                    ensure_user_exists(&connection, id).await?;

                    connection
            .execute(
                "DELETE FROM effect_identity_user_recovery_code_hashes WHERE user_id = ?1",
                (id.to_string(),),
            )
            .await
            .map_err(provider_category)?;

                    for hash in serialized_hashes {
                        connection
                            .execute(
                                "
                    INSERT INTO effect_identity_user_recovery_code_hashes (
                        user_id,
                        code_hash
                    ) VALUES (?1, ?2)
                    ",
                                (id.to_string(), hash),
                            )
                            .await
                            .map_err(provider_category)?;
                    }

                    rotate_security_stamp(&connection, id, false).await?;
                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                id,
                recovery_code_hashes,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn redeem_recovery_code(
        &self,
        id: &UserId,
        recovery_code: &RecoveryCode,
    ) -> Result<bool, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
            /// Native method input retained by borrow or moved exactly once.
            recovery_code: &'input RecoveryCode,
        }
        impl<'input> NativeOperation<bool> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<bool, SqlError>
            where
                Self: 'lease,
            {
                let Self { id, recovery_code } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<bool, StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;

                    let rows = match connection
                        .query(
                            recovery_lookahead_sql!(
                                "
                SELECT id, code_hash
                  FROM effect_identity_user_recovery_code_hashes
                 WHERE user_id = ?1
                 ORDER BY id LIMIT ",
                                "
                "
                            ),
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)
                    {
                        Ok(rows) => rows,
                        Err(error) => {
                            return Err(error);
                        }
                    };

                    let mut matched_row_id = None;
                    for row in recovery_code_rows(rows).await? {
                        let row_id = match row_integer_at(&row, 0) {
                            Ok(row_id) => row_id,
                            Err(error) => {
                                return Err(error);
                            }
                        };
                        let hash_payload = match row_text_at(&row, 1) {
                            Ok(hash_payload) => hash_payload,
                            Err(error) => {
                                return Err(error);
                            }
                        };
                        let hash = match deserialize_recovery_code_hash(&hash_payload) {
                            Ok(hash) => hash,
                            Err(error) => {
                                return Err(error);
                            }
                        };
                        if hash.matches(recovery_code) {
                            matched_row_id = Some(row_id);
                        }
                    }

                    let Some(row_id) = matched_row_id else {
                        return Ok(false);
                    };

                    let changed = match connection
                        .execute(
                            "
                DELETE FROM effect_identity_user_recovery_code_hashes
                 WHERE id = ?1
                   AND user_id = ?2
                ",
                            (row_id, id.to_string()),
                        )
                        .await
                        .map_err(provider_category)
                    {
                        Ok(changed) => changed,
                        Err(error) => {
                            return Err(error);
                        }
                    };

                    Ok(changed == 1)
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id, recovery_code })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn count_recovery_codes(&self, id: &UserId) -> Result<usize, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input> NativeOperation<usize> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<usize, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<usize, StoreError> = async move {
                    ensure_user_exists(&connection, id).await?;
                    let mut rows = connection
                        .query(
                            "
                SELECT COUNT(*)
                  FROM effect_identity_user_recovery_code_hashes
                 WHERE user_id = ?1
                ",
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    let row = rows
                        .next()
                        .await
                        .map_err(provider_category)?
                        .ok_or(StoreError::CorruptStoredData)?;
                    let count = usize::try_from(row_integer_at(&row, 0)?)
                        .map_err(|_| StoreError::CorruptStoredData)?;
                    consume_rows(&mut rows).await?;

                    Ok(count)
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl RoleStore for SqlxIdentityStore {
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn create_role(&self, new_role: NewRole) -> Result<IdentityRole, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            new_role: NewRole,
        }
        impl NativeOperation<IdentityRole> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<IdentityRole, SqlError>
            where
                Self: 'lease,
            {
                let Self { new_role } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<IdentityRole, StoreError> = async move {
                    let role = IdentityRole::from_new(new_role);
                    let normalized_name = role
                        .normalized_name()
                        .cloned()
                        .ok_or(StoreError::CorruptStoredData)?;
                    let payload = serialize_role(&role)?;

                    connection
                        .execute(
                            "
                INSERT INTO effect_identity_roles (
                    id,
                    normalized_name,
                    concurrency_stamp,
                    payload
                ) VALUES (?1, ?2, ?3, ?4)
                ",
                            (
                                role.id().to_string(),
                                normalized_name.to_string(),
                                role.concurrency_stamp().to_string(),
                                payload,
                            ),
                        )
                        .await
                        .map_err(provider_category)?;

                    Ok(role)
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { new_role })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn update_role(&self, update: RoleUpdate) -> Result<IdentityRole, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            update: RoleUpdate,
        }
        impl NativeOperation<IdentityRole> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<IdentityRole, SqlError>
            where
                Self: 'lease,
            {
                let Self { update } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<IdentityRole, StoreError> = async move {
        let role=update.role();
        let expected=update.expected_concurrency_stamp();
        let id=role.id();
        let existing=find_role_by_id(&connection,&id).await?.ok_or(StoreError::NotFound)?;
        if existing.concurrency_stamp()!=expected{return Err(StoreError::TransactionConflict);}
        let normalized_name=role.normalized_name().ok_or(StoreError::CorruptStoredData)?;
        let revision=ConcurrencyStamp::new();
        let payload=serialize_refreshed_role(role,&revision)?;
        let changed=connection.execute("UPDATE effect_identity_roles SET normalized_name=?1,concurrency_stamp=?2,payload=?3 WHERE id=?4 AND concurrency_stamp=?5",
            (normalized_name.as_str(),revision.to_string(),payload,id.to_string(),expected.to_string())).await.map_err(provider_category)?;
        ensure_rows_changed(changed,StoreError::TransactionConflict)?;
        find_role_by_id(&connection,&id).await?.ok_or(StoreError::CorruptStoredData)

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { update })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn delete_role(&self, id: &RoleId) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input RoleId,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    connection
                        .execute(
                            "DELETE FROM effect_identity_roles WHERE id = ?1",
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn find_role_by_id(&self, id: &RoleId) -> Result<Option<IdentityRole>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input RoleId,
        }
        impl<'input> NativeOperation<Option<IdentityRole>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<IdentityRole>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<IdentityRole>, StoreError> =
                    async move { find_role_by_id(&connection, id).await }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn find_role_by_normalized_name(
        &self,
        normalized_name: &NormalizedUserName,
    ) -> Result<Option<IdentityRole>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            normalized_name: &'input NormalizedUserName,
        }
        impl<'input> NativeOperation<Option<IdentityRole>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<IdentityRole>, SqlError>
            where
                Self: 'lease,
            {
                let Self { normalized_name } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<IdentityRole>, StoreError> = async move {
                    let mut rows = connection
                        .query(
                            "SELECT payload FROM effect_identity_roles WHERE normalized_name = ?1",
                            (normalized_name.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    read_optional_role(&mut rows).await
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { normalized_name })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl QueryableRoleStore for SqlxIdentityStore {
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn list_roles_page(
        &self,
        after: Option<RoleId>,
        limit: std::num::NonZeroU16,
    ) -> Result<RolePage, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            after: Option<RoleId>,
            /// Native method input retained by borrow or moved exactly once.
            limit: std::num::NonZeroU16,
        }
        impl NativeOperation<RolePage> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<RolePage, SqlError>
            where
                Self: 'lease,
            {
                let Self { after, limit } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<RolePage, StoreError> = async move {
                    if usize::from(limit.get()) > super::schema::NATIVE_COLLECTION_LIMIT {
                        return Err(StoreError::ConstraintViolation);
                    }

                    let mut rows = connection
            .query(
                "SELECT payload FROM effect_identity_roles WHERE id > ?1 ORDER BY id LIMIT ?2",
                (
                    after.map_or_else(String::new, |id| id.to_string()),
                    i64::from(limit.get()) + 1,
                ),
            )
            .await
            .map_err(provider_category)?;
                    let mut roles = read_roles(&mut rows).await?;
                    let next = if roles.len() > usize::from(limit.get()) {
                        roles.pop();
                        roles.last().map(IdentityRole::id)
                    } else {
                        None
                    };
                    RolePage::new(roles, next)
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { after, limit })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl RoleClaimStore for SqlxIdentityStore {
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn get_role_claims(&self, id: &RoleId) -> Result<Vec<IdentityClaim>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input RoleId,
        }
        impl<'input> NativeOperation<Vec<IdentityClaim>> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Vec<IdentityClaim>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Vec<IdentityClaim>, StoreError> = async move {
                    ensure_role_exists(&connection, id).await?;
                    let mut rows = connection
                        .query(
                            collection_lookahead_sql!(
                                "
                SELECT claim_type, claim_value
                  FROM effect_identity_role_claims
                 WHERE role_id = ?1
                 ORDER BY id LIMIT ",
                                "
                "
                            ),
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;

                    read_claims(&mut rows).await
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn add_role_claim(&self, id: &RoleId, claim: IdentityClaim) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input RoleId,
            /// Native method input retained by borrow or moved exactly once.
            claim: IdentityClaim,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id, claim } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {

        ensure_role_exists(&connection, id).await?;
        let inserted = connection
            .execute(
                collection_limit_sql!("
                INSERT INTO effect_identity_role_claims (
                    role_id,
                    claim_type,
                    claim_value
                ) SELECT ?1, ?2, ?3 WHERE (SELECT COUNT(*) FROM effect_identity_role_claims WHERE role_id=?1)<","
                "),
                (
                    id.to_string(),
                    claim.claim_type().to_string(),
                    claim.claim_value().to_string(),
                ),
            )
            .await
            .map_err(provider_category)?;
        if inserted != 1 {
            return Err(StoreError::ConstraintViolation);
        }

        Ok(())

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id, claim })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn remove_role_claim(
        &self,
        id: &RoleId,
        claim: &IdentityClaim,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input RoleId,
            /// Native method input retained by borrow or moved exactly once.
            claim: &'input IdentityClaim,
        }
        impl<'input> NativeOperation<()> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id, claim } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    ensure_role_exists(&connection, id).await?;
                    connection
                        .execute(
                            "
                DELETE FROM effect_identity_role_claims
                 WHERE role_id = ?1
                   AND claim_type = ?2
                   AND claim_value = ?3
                ",
                            (
                                id.to_string(),
                                claim.claim_type().to_string(),
                                claim.claim_value().to_string(),
                            ),
                        )
                        .await
                        .map_err(provider_category)?;

                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id, claim })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl identity_rs::identity::protection::storage::EncryptedPersonalDataStore for SqlxIdentityStore {
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn create_protected_user(
        &self,
        record: identity_rs::identity::protection::storage::ProtectedUserRecord,
    ) -> Result<identity_rs::identity::protection::storage::ProtectedUserRecord, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            record: identity_rs::identity::protection::storage::ProtectedUserRecord,
        }
        impl NativeOperation<identity_rs::identity::protection::storage::ProtectedUserRecord>
            for Operation
        {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<identity_rs::identity::protection::storage::ProtectedUserRecord, SqlError>
            where
                Self: 'lease,
            {
                let Self { record } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<identity_rs::identity::protection::storage::ProtectedUserRecord, StoreError> = async move {
        record.validate()?;


        let mut rows = connection.query("SELECT 1 FROM effect_identity_users u LEFT JOIN effect_identity_protected_users p ON p.user_id = u.id WHERE p.user_id IS NULL LIMIT 1", ()).await.map_err(provider_category)?;
        if rows.next().await.map_err(provider_category)?.is_some() {
            return Err(StoreError::CorruptStoredData);
        }
        consume_rows(&mut rows).await?;
        drop(rows);
        validate_protected_claims(&connection, &record).await?;
        connection.execute("INSERT INTO effect_identity_users(id, normalized_user_name, normalized_email, concurrency_stamp, payload) VALUES (?1, ?2, NULL, ?3, ?4)",
            (record.id().to_string(), format!("protected:{}", record.id()), record.metadata().concurrency_stamp().to_string(), serialize_user(record.metadata())?)).await.map_err(provider_category)?;
        connection.execute("INSERT INTO effect_identity_protected_users(user_id, require_unique, payload) VALUES (?1, ?2, ?3)",
            (record.id().to_string(), i64::from(record.require_unique_email()), super::schema::encode_security(&record)?)).await.map_err(provider_category)?;
        insert_protected_indexes(&connection, &record).await?;

        Ok(record)

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { record })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn update_protected_user(
        &self,
        record: identity_rs::identity::protection::storage::ProtectedUserRecord,
        expected: &ConcurrencyStamp,
    ) -> Result<identity_rs::identity::protection::storage::ProtectedUserRecord, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            record: identity_rs::identity::protection::storage::ProtectedUserRecord,
            /// Native method input retained by borrow or moved exactly once.
            expected: &'input ConcurrencyStamp,
        }
        impl<'input>
            NativeOperation<identity_rs::identity::protection::storage::ProtectedUserRecord>
            for Operation<'input>
        {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<identity_rs::identity::protection::storage::ProtectedUserRecord, SqlError>
            where
                Self: 'lease,
            {
                let Self { record, expected } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<identity_rs::identity::protection::storage::ProtectedUserRecord, StoreError> = async move {
        record.validate()?;


        let existing = read_protected_user(&connection, &record.id())
            .await?
            .ok_or(StoreError::NotFound)?;
        if existing.metadata().concurrency_stamp() != expected {
            return Err(StoreError::TransactionConflict);
        }
        if existing.require_unique_email() != record.require_unique_email() {
            return Err(StoreError::ConstraintViolation);
        }
        validate_protected_claims(&connection, &record).await?;
        let changed = connection.execute("UPDATE effect_identity_users SET concurrency_stamp = ?1, payload = ?2 WHERE id = ?3 AND concurrency_stamp = ?4",
            (record.metadata().concurrency_stamp().to_string(), serialize_user(record.metadata())?, record.id().to_string(), expected.to_string())).await.map_err(provider_category)?;
        ensure_rows_changed(changed, StoreError::TransactionConflict)?;
        connection
            .execute(
                "UPDATE effect_identity_protected_users SET payload = ?1 WHERE user_id = ?2",
                (
                    super::schema::encode_security(&record)?,
                    record.id().to_string(),
                ),
            )
            .await
            .map_err(provider_category)?;
        connection
            .execute(
                "DELETE FROM effect_identity_protected_indexes WHERE user_id = ?1",
                (record.id().to_string(),),
            )
            .await
            .map_err(provider_category)?;
        insert_protected_indexes(&connection, &record).await?;

        Ok(record)

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { record, expected })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn find_protected_user(
        &self,
        id: &UserId,
    ) -> Result<Option<identity_rs::identity::protection::storage::ProtectedUserRecord>, StoreError>
    {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: &'input UserId,
        }
        impl<'input>
            NativeOperation<Option<identity_rs::identity::protection::storage::ProtectedUserRecord>>
            for Operation<'input>
        {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<
                Option<identity_rs::identity::protection::storage::ProtectedUserRecord>,
                SqlError,
            >
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<
                    Option<identity_rs::identity::protection::storage::ProtectedUserRecord>,
                    StoreError,
                > = async move { read_protected_user(&connection, id).await }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn find_protected_users(
        &self,
        indexes: &[identity_rs::identity::protection::storage::ProtectedLookupKey],
    ) -> Result<Vec<identity_rs::identity::protection::storage::ProtectedUserRecord>, StoreError>
    {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            indexes: &'input [identity_rs::identity::protection::storage::ProtectedLookupKey],
        }
        impl<'input>
            NativeOperation<Vec<identity_rs::identity::protection::storage::ProtectedUserRecord>>
            for Operation<'input>
        {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<
                Vec<identity_rs::identity::protection::storage::ProtectedUserRecord>,
                SqlError,
            >
            where
                Self: 'lease,
            {
                let Self { indexes } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Vec<identity_rs::identity::protection::storage::ProtectedUserRecord>, StoreError> = async move {
        if indexes.len() > super::schema::NATIVE_PROTECTED_INDEX_LIMIT {
            return Err(StoreError::ConstraintViolation);
        }

        let mut ids = std::collections::HashSet::new();
        for index in indexes {
            let mut rows = connection.query("SELECT user_id FROM effect_identity_protected_indexes WHERE field = ?1 AND key_id = ?2 AND digest = ?3 LIMIT 2", (index.field(), index.key_id().to_string(), index.digest())).await.map_err(provider_category)?;
            while let Some(row) = rows.next().await.map_err(provider_category)? {
                ids.insert(
                    UserId::parse(row_text(&row)?).map_err(|_| StoreError::CorruptStoredData)?,
                );
                if ids.len() > 1 {
                    return Err(StoreError::ConstraintViolation);
                }
            }
        }
        let mut records = Vec::new();
        for id in ids {
            records.push(
                read_protected_user(&connection, &id)
                    .await?
                    .ok_or(StoreError::CorruptStoredData)?,
            );
        }
        Ok(records)

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { indexes })
            .await
            .map_err(|error| self.narrow(error))
    }
}
impl BrowserContextStore for SqlxIdentityStore {
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn create_browser_context(
        &self,
        context: BrowserContext,
        now: OffsetDateTime,
        capacity: usize,
    ) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            context: BrowserContext,
            /// Native method input retained by borrow or moved exactly once.
            now: OffsetDateTime,
            /// Native method input retained by borrow or moved exactly once.
            capacity: usize,
        }
        impl NativeOperation<()> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    context,
                    now,
                    capacity,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
        context.validate()?;
        if capacity == 0 || capacity > super::schema::NATIVE_BROWSER_LIMIT || context.expires_at() <= now {
            return Err(StoreError::ConstraintViolation);
        }


        let now =
            i64::try_from(now.unix_timestamp_nanos()).map_err(|_| StoreError::CorruptStoredData)?;
        connection
            .execute(
                "DELETE FROM effect_identity_browser_contexts WHERE expires_at <= ?1",
                (now,),
            )
            .await
            .map_err(provider_category)?;
        let mut rows = connection
            .query("SELECT COUNT(*) FROM effect_identity_browser_contexts", ())
            .await
            .map_err(provider_category)?;
        let row = rows
            .next()
            .await
            .map_err(provider_category)?
            .ok_or(StoreError::CorruptStoredData)?;
        if usize::try_from(row_integer_at(&row, 0)?).map_err(|_| StoreError::CorruptStoredData)?
            >= capacity
        {
            return Err(StoreError::ConstraintViolation);
        }
        consume_rows(&mut rows).await?;
        drop(rows);
        connection
            .execute(
                "INSERT INTO effect_identity_browser_contexts(id,expires_at,payload) VALUES(?1,?2,?3)",
                (
                    context.id().to_string(),
                    i64::try_from(context.expires_at().unix_timestamp_nanos())
                        .map_err(|_| StoreError::CorruptStoredData)?,
                    super::schema::encode_security(&context)?,
                ),
            )
            .await
            .map_err(provider_category)?;
        Ok(())

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                context,
                now,
                capacity,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Reads bounded native state through one operation-owned SQL lease; absence and backend failures remain distinct.
    async fn find_browser_context(
        &self,
        id: uuid::Uuid,
        now: OffsetDateTime,
    ) -> Result<Option<BrowserContext>, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            id: uuid::Uuid,
            /// Native method input retained by borrow or moved exactly once.
            now: OffsetDateTime,
        }
        impl NativeOperation<Option<BrowserContext>> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<Option<BrowserContext>, SqlError>
            where
                Self: 'lease,
            {
                let Self { id, now } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<Option<BrowserContext>, StoreError> = async move {
                    let mut rows = connection
                        .query(
                            "SELECT payload FROM effect_identity_browser_contexts WHERE id = ?1",
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;
                    let Some(row) = rows.next().await.map_err(provider_category)? else {
                        return Ok(None);
                    };
                    let context: BrowserContext = super::schema::decode_security(&row_text(&row)?)?;
                    context.validate()?;
                    if context.id() != id {
                        return Err(StoreError::CorruptStoredData);
                    }
                    consume_rows(&mut rows).await?;
                    Ok((context.expires_at() > now).then_some(context))
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .connection_operation(Operation { id, now })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn replace_browser_pending_authentication(
        &self,
        id: uuid::Uuid,
        expected_token: &AuthTokenValue,
        expected_pending: Option<&AuthTokenValue>,
        value: Option<AuthTokenValue>,
        now: OffsetDateTime,
    ) -> Result<bool, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation<'input> {
            /// Native method input retained by borrow or moved exactly once.
            id: uuid::Uuid,
            /// Native method input retained by borrow or moved exactly once.
            expected_token: &'input AuthTokenValue,
            /// Native method input retained by borrow or moved exactly once.
            expected_pending: Option<&'input AuthTokenValue>,
            /// Native method input retained by borrow or moved exactly once.
            value: Option<AuthTokenValue>,
            /// Native method input retained by borrow or moved exactly once.
            now: OffsetDateTime,
        }
        impl<'input> NativeOperation<bool> for Operation<'input> {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<bool, SqlError>
            where
                Self: 'lease,
            {
                let Self {
                    id,
                    expected_token,
                    expected_pending,
                    value,
                    now,
                } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<bool, StoreError> = async move {
        let Some(context) = find_browser_context(&connection, id, now).await? else {
            return Ok(false);
        };
        if context.token() != expected_token || context.pending_authentication() != expected_pending
        {
            return Ok(false);
        }
        let expected_payload =
            super::schema::encode_security(&context)?;
        let updated = context.with_pending_authentication(value)?;
        let payload = super::schema::encode_security(&updated)?;
        let changed = connection.execute(
            "UPDATE effect_identity_browser_contexts SET payload=?1 WHERE id=?2 AND payload=?3 AND expires_at>?4",
            (payload, id.to_string(), expected_payload, i64::try_from(now.unix_timestamp_nanos()).map_err(|_| StoreError::CorruptStoredData)?),
        ).await.map_err(provider_category)?;
        Ok(changed == 1)

                        }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation {
                id,
                expected_token,
                expected_pending,
                value,
                now,
            })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn remove_browser_context(&self, id: uuid::Uuid) -> Result<(), StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            id: uuid::Uuid,
        }
        impl NativeOperation<()> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<(), SqlError>
            where
                Self: 'lease,
            {
                let Self { id } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<(), StoreError> = async move {
                    connection
                        .execute(
                            "DELETE FROM effect_identity_browser_contexts WHERE id = ?1",
                            (id.to_string(),),
                        )
                        .await
                        .map_err(provider_category)?;
                    Ok(())
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { id })
            .await
            .map_err(|error| self.narrow(error))
    }
    /// Commits the native command in one operation-owned SQL transaction; acknowledgment failure remains typed.
    async fn remove_expired_browser_contexts(
        &self,
        now: OffsetDateTime,
    ) -> Result<usize, StoreError> {
        /// Borrowed/moved native invocation, separate from any reusable Effect recipe.
        struct Operation {
            /// Native method input retained by borrow or moved exactly once.
            now: OffsetDateTime,
        }
        impl NativeOperation<usize> for Operation {
            /// Execute under the shared kernel owner without cloning inputs or opening another runtime.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<usize, SqlError>
            where
                Self: 'lease,
            {
                let Self { now } = self;
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result: Result<usize, StoreError> = async move {
                    let now = i64::try_from(now.unix_timestamp_nanos())
                        .map_err(|_| StoreError::CorruptStoredData)?;
                    let changed = connection
                        .execute(
                            "DELETE FROM effect_identity_browser_contexts WHERE expires_at <= ?1",
                            (now,),
                        )
                        .await
                        .map_err(provider_category)?;
                    usize::try_from(changed).map_err(|_| StoreError::CorruptStoredData)
                }
                .await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Operation { now })
            .await
            .map_err(|error| self.narrow(error))
    }
}
/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) async fn find_user_by_id(
    connection: &StoreConnection<'_>,
    id: &UserId,
) -> Result<Option<IdentityUser>, StoreError> {
    let mut rows = connection
        .query(
            "SELECT payload FROM effect_identity_users WHERE id = ?1",
            (id.to_string(),),
        )
        .await
        .map_err(provider_category)?;

    read_optional_user(&mut rows).await
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn find_role_by_id(
    connection: &StoreConnection<'_>,
    id: &RoleId,
) -> Result<Option<IdentityRole>, StoreError> {
    let mut rows = connection
        .query(
            "SELECT payload FROM effect_identity_roles WHERE id = ?1",
            (id.to_string(),),
        )
        .await
        .map_err(provider_category)?;

    read_optional_role(&mut rows).await
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn find_role_id_by_normalized_name(
    connection: &StoreConnection<'_>,
    normalized_name: &NormalizedUserName,
) -> Result<Option<RoleId>, StoreError> {
    let mut rows = connection
        .query(
            "SELECT id FROM effect_identity_roles WHERE normalized_name = ?1",
            (normalized_name.to_string(),),
        )
        .await
        .map_err(provider_category)?;

    let Some(row) = rows.next().await.map_err(provider_category)? else {
        return Ok(None);
    };

    let id = RoleId::parse(row_text_at(&row, 0)?).map_err(|_| StoreError::CorruptStoredData)?;
    consume_rows(&mut rows).await?;

    Ok(Some(id))
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn ensure_user_exists(
    connection: &StoreConnection<'_>,
    id: &UserId,
) -> Result<(), StoreError> {
    if find_user_by_id(connection, id).await?.is_none() {
        return Err(StoreError::NotFound);
    }

    Ok(())
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) async fn ensure_role_exists(
    connection: &StoreConnection<'_>,
    id: &RoleId,
) -> Result<(), StoreError> {
    if find_role_by_id(connection, id).await?.is_none() {
        return Err(StoreError::NotFound);
    }

    Ok(())
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn save_user(
    connection: &StoreConnection<'_>,
    user: &IdentityUser,
    expected: &ConcurrencyStamp,
) -> Result<(), StoreError> {
    if user.user_name().is_none() {
        let mut rows = connection
            .query(
                "SELECT user_id FROM effect_identity_protected_users WHERE user_id=?1",
                (user.id().to_string(),),
            )
            .await
            .map_err(provider_category)?;
        if rows.next().await.map_err(provider_category)?.is_none() {
            return Err(StoreError::CorruptStoredData);
        }
        let changed=connection.execute("UPDATE effect_identity_users SET concurrency_stamp=?1,payload=?2 WHERE id=?3 AND concurrency_stamp=?4",(user.concurrency_stamp().to_string(),serialize_user(user)?,user.id().to_string(),expected.to_string())).await.map_err(provider_category)?;
        return ensure_rows_changed(changed, StoreError::TransactionConflict);
    }
    let normalized_user_name = user
        .normalized_user_name()
        .cloned()
        .ok_or(StoreError::CorruptStoredData)?;
    let normalized_email = user.normalized_email().map(ToString::to_string);
    let payload = serialize_user(user)?;
    let changed = connection
        .execute(
            "
            UPDATE effect_identity_users
               SET normalized_user_name = ?1,
                   normalized_email = ?2,
                   concurrency_stamp = ?3,
                   payload = ?4
             WHERE id = ?5 AND concurrency_stamp = ?6
            ",
            (
                normalized_user_name.to_string(),
                normalized_email,
                user.concurrency_stamp().to_string(),
                payload,
                user.id().to_string(),
                expected.to_string(),
            ),
        )
        .await
        .map_err(provider_category)?;

    ensure_rows_changed(changed, StoreError::TransactionConflict)?;

    Ok(())
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn insert_user_claim(
    connection: &StoreConnection<'_>,
    id: &UserId,
    claim: &IdentityClaim,
) -> Result<(), StoreError> {
    connection
        .execute(
            "
            INSERT INTO effect_identity_user_claims (
                user_id,
                claim_type,
                claim_value
            ) VALUES (?1, ?2, ?3)
            ",
            (
                id.to_string(),
                claim.claim_type().to_string(),
                claim.claim_value().to_string(),
            ),
        )
        .await
        .map_err(provider_category)?;

    Ok(())
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn read_optional_user(rows: &mut StoreRows) -> Result<Option<IdentityUser>, StoreError> {
    let Some(row) = rows.next().await.map_err(provider_category)? else {
        return Ok(None);
    };

    let payload = row_text(&row)?;
    consume_rows(rows).await?;
    deserialize_user(&payload).map(Some)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn read_unique_user(rows: &mut StoreRows) -> Result<Option<IdentityUser>, StoreError> {
    let Some(row) = rows.next().await.map_err(provider_category)? else {
        return Ok(None);
    };
    let user = deserialize_user(&row_text(&row)?)?;
    if rows.next().await.map_err(provider_category)?.is_some() {
        consume_rows(rows).await?;
        return Err(StoreError::ConstraintViolation);
    }
    Ok(Some(user))
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) async fn validate_email_claim(
    connection: &StoreConnection<'_>,
    user: &IdentityUser,
    requested_unique: bool,
) -> Result<(), StoreError> {
    let Some(email) = user.normalized_email() else {
        return Ok(());
    };
    let mut rows = connection.query(
        "SELECT 1 FROM effect_identity_users u LEFT JOIN effect_identity_user_email_policy p ON p.user_id = u.id
         WHERE u.normalized_email = ?1 AND u.id <> ?2
           AND (?3 = 1 OR COALESCE(p.require_unique, 0) = 1 OR EXISTS
             (SELECT 1 FROM effect_identity_user_email_policy own WHERE own.user_id = ?2 AND own.require_unique = 1))
         LIMIT 1", (email.to_string(), user.id().to_string(), i64::from(requested_unique)))
        .await.map_err(provider_category)?;
    let conflict = rows.next().await.map_err(provider_category)?.is_some();
    consume_rows(&mut rows).await?;
    if conflict {
        Err(StoreError::ConstraintViolation)
    } else {
        Ok(())
    }
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn read_users(rows: &mut StoreRows) -> Result<Vec<IdentityUser>, StoreError> {
    let mut users = Vec::new();
    while let Some(row) = rows.next().await.map_err(provider_category)? {
        if users.len() >= super::schema::NATIVE_COLLECTION_LOOKAHEAD {
            return Err(StoreError::CorruptStoredData);
        }
        users.push(deserialize_user(&row_text(&row)?)?);
    }

    Ok(users)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn read_optional_role(rows: &mut StoreRows) -> Result<Option<IdentityRole>, StoreError> {
    let Some(row) = rows.next().await.map_err(provider_category)? else {
        return Ok(None);
    };

    let payload = row_text(&row)?;
    consume_rows(rows).await?;
    deserialize_role(&payload).map(Some)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn read_roles(rows: &mut StoreRows) -> Result<Vec<IdentityRole>, StoreError> {
    let mut roles = Vec::new();
    while let Some(row) = rows.next().await.map_err(provider_category)? {
        if roles.len() >= super::schema::NATIVE_COLLECTION_LOOKAHEAD {
            return Err(StoreError::CorruptStoredData);
        }
        roles.push(deserialize_role(&row_text(&row)?)?);
    }

    Ok(roles)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) async fn read_claims(rows: &mut StoreRows) -> Result<Vec<IdentityClaim>, StoreError> {
    let mut claims = Vec::new();
    while let Some(row) = rows.next().await.map_err(provider_category)? {
        if claims.len() >= super::schema::NATIVE_COLLECTION_LIMIT {
            return Err(StoreError::CorruptStoredData);
        }
        let claim_type = row_text_at(&row, 0)?;
        let claim_value = row_text_at(&row, 1)?;
        claims.push(IdentityClaim::new(
            claim_type
                .parse()
                .map_err(|_| StoreError::CorruptStoredData)?,
            claim_value
                .parse()
                .map_err(|_| StoreError::CorruptStoredData)?,
        ));
    }

    Ok(claims)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) async fn read_logins(rows: &mut StoreRows) -> Result<Vec<UserLoginInfo>, StoreError> {
    let mut logins = Vec::new();
    while let Some(row) = rows.next().await.map_err(provider_category)? {
        if logins.len() >= super::schema::NATIVE_COLLECTION_LIMIT {
            return Err(StoreError::CorruptStoredData);
        }
        let login_provider = row_text_at(&row, 0)?;
        let provider_key = row_text_at(&row, 1)?;
        let provider_display_name = optional_row_text_at(&row, 2)?;
        logins.push(UserLoginInfo::new(
            login_provider
                .parse()
                .map_err(|_| StoreError::CorruptStoredData)?,
            provider_key
                .parse()
                .map_err(|_| StoreError::CorruptStoredData)?,
            provider_display_name,
        ));
    }

    Ok(logins)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn read_optional_user_passkey(
    rows: &mut StoreRows,
) -> Result<Option<StoredPasskey>, StoreError> {
    let Some(row) = rows.next().await.map_err(provider_category)? else {
        return Ok(None);
    };

    let payload = row_text(&row)?;
    consume_rows(rows).await?;
    deserialize_user_passkey(&payload).map(Some)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn read_optional_passkey_challenge_state(
    rows: &mut StoreRows,
) -> Result<Option<PasskeyChallengeState>, StoreError> {
    let Some(row) = rows.next().await.map_err(provider_category)? else {
        return Ok(None);
    };

    let payload = row_text(&row)?;
    consume_rows(rows).await?;
    deserialize_passkey_challenge_state(&payload).map(Some)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) async fn read_user_passkeys(
    rows: &mut StoreRows,
    owner: &UserId,
) -> Result<Vec<UserPasskeyInfo>, StoreError> {
    let mut passkeys = Vec::new();

    while let Some(row) = rows.next().await.map_err(provider_category)? {
        if passkeys.len() >= super::schema::NATIVE_COLLECTION_LIMIT {
            return Err(StoreError::CorruptStoredData);
        }
        let user_passkey = deserialize_user_passkey(&row_text(&row)?)?;
        if user_passkey.user_id() != *owner {
            return Err(StoreError::CorruptStoredData);
        }
        passkeys.push(user_passkey.into_user_passkey_info());
    }

    Ok(passkeys)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn consume_rows(rows: &mut StoreRows) -> Result<(), StoreError> {
    while rows.next().await.map_err(provider_category)?.is_some() {}

    Ok(())
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) fn row_text(row: &sqlx::TursoRow) -> Result<String, StoreError> {
    row_text_at(row, 0)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) fn row_text_at(row: &sqlx::TursoRow, column: usize) -> Result<String, StoreError> {
    optional_row_text_at(row, column)?.ok_or(StoreError::CorruptStoredData)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
fn optional_row_text_at(row: &sqlx::TursoRow, column: usize) -> Result<Option<String>, StoreError> {
    row.try_get(column)
        .map_err(|_| StoreError::CorruptStoredData)
}
/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) fn row_integer_at(row: &sqlx::TursoRow, column: usize) -> Result<i64, StoreError> {
    row.try_get(column)
        .map_err(|_| StoreError::CorruptStoredData)
}
/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) fn ensure_rows_changed(changed: u64, error: StoreError) -> Result<(), StoreError> {
    if changed == 0 {
        return Err(error);
    }

    Ok(())
}
/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) fn serialize_user(user: &IdentityUser) -> Result<String, StoreError> {
    super::schema::encode_security(user)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) fn deserialize_user(payload: &str) -> Result<IdentityUser, StoreError> {
    super::schema::decode_security(payload)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
fn serialize_role(role: &IdentityRole) -> Result<String, StoreError> {
    super::schema::encode_security(role)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
fn deserialize_role(payload: &str) -> Result<IdentityRole, StoreError> {
    super::schema::decode_security(payload)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) fn serialize_user_passkey(passkey: &StoredPasskey) -> Result<String, StoreError> {
    super::schema::encode_security(passkey)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) fn deserialize_user_passkey(payload: &str) -> Result<StoredPasskey, StoreError> {
    let record: StoredPasskey = super::schema::decode_security(payload)?;
    record.info.validate()?;
    Ok(record)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
fn serialize_passkey_challenge_state(
    challenge_state: &PasskeyChallengeState,
) -> Result<String, StoreError> {
    super::schema::encode_security(challenge_state)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
fn deserialize_passkey_challenge_state(payload: &str) -> Result<PasskeyChallengeState, StoreError> {
    super::schema::decode_security(payload)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
fn passkey_challenge_purpose_text(purpose: PasskeyChallengePurpose) -> &'static str {
    match purpose {
        PasskeyChallengePurpose::Registration => "registration",
        PasskeyChallengePurpose::Assertion => "assertion",
    }
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) fn serialize_recovery_code_hash(hash: &RecoveryCodeHash) -> Result<String, StoreError> {
    super::schema::encode_security(hash)
}

/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) fn deserialize_recovery_code_hash(
    payload: &str,
) -> Result<RecoveryCodeHash, StoreError> {
    super::schema::decode_security(payload)
}
/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
async fn recovery_code_rows(mut rows: StoreRows) -> Result<Vec<sqlx::TursoRow>, StoreError> {
    let mut collected = Vec::new();
    while let Some(row) = rows.next().await.map_err(provider_category)? {
        if collected.len() >= super::schema::NATIVE_RECOVERY_LIMIT {
            return Err(StoreError::CorruptStoredData);
        }
        collected.push(row);
    }
    Ok(collected)
}
/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) async fn read_protected_user(
    connection: &StoreConnection<'_>,
    id: &UserId,
) -> Result<Option<identity_rs::identity::protection::storage::ProtectedUserRecord>, StoreError> {
    let Some(metadata) = find_user_by_id(connection, id).await? else {
        return Ok(None);
    };
    let mut rows = connection
        .query(
            "SELECT payload FROM effect_identity_protected_users WHERE user_id = ?1",
            (id.to_string(),),
        )
        .await
        .map_err(provider_category)?;
    let row = rows
        .next()
        .await
        .map_err(provider_category)?
        .ok_or(StoreError::CorruptStoredData)?;
    let record: ProtectedPersistence = super::schema::decode_security(&row_text(&row)?)?;
    consume_rows(&mut rows).await?;
    record
        .metadata
        .validate()
        .map_err(|_| StoreError::CorruptStoredData)?;
    if record.metadata.id() != *id {
        return Err(StoreError::CorruptStoredData);
    }
    let record = identity_rs::identity::protection::storage::ProtectedUserRecord::new(
        metadata,
        record.values,
        record.indexes,
        record.email_confirmed,
        record.require_unique_email,
    )?;
    record.validate()?;
    Ok(Some(record))
}
/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) async fn insert_protected_indexes(
    connection: &StoreConnection<'_>,
    record: &identity_rs::identity::protection::storage::ProtectedUserRecord,
) -> Result<(), StoreError> {
    for index in record.indexes() {
        connection.execute("INSERT INTO effect_identity_protected_indexes(user_id, field, key_id, digest) VALUES (?1, ?2, ?3, ?4)",
            (record.id().to_string(), index.field(), index.key_id().to_string(), index.digest())).await.map_err(provider_category)?;
    }
    Ok(())
}
/// Validate native persisted state or bind its fields inside the caller's owned SQL operation.
pub(super) async fn validate_protected_claims(
    connection: &StoreConnection<'_>,
    record: &identity_rs::identity::protection::storage::ProtectedUserRecord,
) -> Result<(), StoreError> {
    for index in record.indexes() {
        let mut rows = connection.query("SELECT 1 FROM effect_identity_protected_indexes i JOIN effect_identity_protected_users p ON p.user_id = i.user_id WHERE i.field = ?1 AND i.key_id = ?2 AND i.digest = ?3 AND i.user_id <> ?4 AND (?1 = 'name' OR ?5 = 1 OR p.require_unique = 1) LIMIT 1",
            (index.field(), index.key_id().to_string(), index.digest(), record.id().to_string(), i64::from(record.require_unique_email()))).await.map_err(provider_category)?;
        let conflict = rows.next().await.map_err(provider_category)?.is_some();
        consume_rows(&mut rows).await?;
        if conflict {
            return Err(StoreError::ConstraintViolation);
        }
    }
    Ok(())
}

/// Owned native passkey persistence envelope. Conversion from public accessors is
/// kept here because upstream exposes no consuming persisted-envelope conversion.
/// Application credential payloads move into `info`; CAS serialization borrows them.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct StoredPasskey {
    /// Stable owner, verified against the requested user before updates.
    user_id: UserId,
    /// Validated native credential; no cryptographic algorithm is implemented here.
    info: UserPasskeyInfo,
}
impl StoredPasskey {
    /// Move a validated native credential into the private security envelope.
    pub(super) fn new(user_id: UserId, info: UserPasskeyInfo) -> Self {
        Self { user_id, info }
    }
    /// Return the stable owner without interpreting credential bytes as proof.
    fn user_id(&self) -> UserId {
        self.user_id
    }
    /// Move native credential data out of the persistence envelope without copying blobs.
    fn into_user_passkey_info(self) -> UserPasskeyInfo {
        self.info
    }
    /// Change display metadata through the native public lifecycle setter. Its
    /// accessor-only optional label requires a native owned label conversion.
    fn with_display_from_info(mut self, supplied: &UserPasskeyInfo) -> Self {
        self.info = self.info.with_name(supplied.name().cloned());
        self
    }
}
/// Borrowed serialization shape identical to StoredPasskey, used for full-value CAS.
#[derive(serde::Serialize)]
struct ExpectedPasskey<'a> {
    /// Account ID participating in the persisted envelope comparison.
    user_id: UserId,
    /// Borrowed full native credential, including counter/flags/revision.
    info: &'a UserPasskeyInfo,
}
/// Serialize a native expected credential by borrow; no public crypto payload is cloned.
pub(super) fn serialize_expected_passkey(
    id: UserId,
    expected: &UserPasskeyInfo,
) -> Result<String, StoreError> {
    super::schema::encode_security(&ExpectedPasskey {
        user_id: id,
        info: expected,
    })
}
/// Decode encrypted persisted fields into owned slots so refreshing metadata moves
/// ciphertext/indexes rather than cloning an accessor-only native record.
#[derive(serde::Deserialize)]
struct ProtectedPersistence {
    /// Old aggregate is validated by the public DTO construction on decode/use.
    metadata: IdentityUser,
    /// Four personal-data ciphertext fields in native documented order.
    values: [Option<identity_rs::identity::protection::storage::EncryptedPersonalValue>; 4],
    /// Retained-version keyed lookup digests, never plaintext lookup aliases.
    indexes: Vec<identity_rs::identity::protection::storage::ProtectedLookupKey>,
    /// Destination confirmation bound to the encrypted contact slots.
    email_confirmed: bool,
    /// Persisted alias policy retained across credential-only mutations.
    require_unique_email: bool,
}
/// Load/validate one browser context on the same operation owner used by pending CAS.
async fn find_browser_context(
    connection: &StoreConnection<'_>,
    id: uuid::Uuid,
    now: OffsetDateTime,
) -> Result<Option<BrowserContext>, StoreError> {
    let mut rows = connection
        .query(
            "SELECT payload FROM effect_identity_browser_contexts WHERE id=?1",
            (id.to_string(),),
        )
        .await
        .map_err(provider_category)?;
    let Some(row) = rows.next().await.map_err(provider_category)? else {
        return Ok(None);
    };
    let context: BrowserContext = super::schema::decode_security(&row_text(&row)?)?;
    context.validate()?;
    if context.id() != id {
        return Err(StoreError::CorruptStoredData);
    }
    Ok((context.expires_at() > now).then_some(context))
}

impl SqlxIdentityStore {
    /// Apply one native contact/security mutation under an operation-owned writer
    /// transaction; aliases and expected revision are persisted at the same boundary.
    async fn update_user_contact<F>(
        &self,
        id: &UserId,
        update: F,
    ) -> Result<IdentityUser, StoreError>
    where
        F: FnOnce(IdentityUser) -> Result<IdentityUser, StoreError> + Send,
    {
        /// Scoped mutation captures the account identity by borrow and consumes its callback.
        struct Contact<'input, F> {
            /// Stable user reference valid until this operation settles.
            id: &'input UserId,
            /// Native lifecycle mutation with no reusable/Clone payload requirement.
            update: F,
        }
        impl<F: FnOnce(IdentityUser) -> Result<IdentityUser, StoreError> + Send>
            NativeOperation<IdentityUser> for Contact<'_, F>
        {
            /// Run against the shared kernel owner and preserve rejection through rollback.
            async fn run<'lease>(
                self,
                native: &'lease mut sqlx::TursoConnection,
            ) -> Result<IdentityUser, SqlError>
            where
                Self: 'lease,
            {
                let connection = StoreConnection::new(native);
                let diagnostics = connection.diagnostics();
                let result = async {
                        let user = find_user_by_id(&connection, self.id).await?.ok_or(StoreError::NotFound)?;
                        let expected = user.concurrency_stamp().clone();
                        let updated = (self.update)(user)?.with_refreshed_concurrency_stamp();
                        updated.validate().map_err(|_| StoreError::CorruptStoredData)?;
                        if updated.user_name().is_none() {
                            let mut rows = connection.query("SELECT user_id FROM effect_identity_protected_users WHERE user_id=?1", (self.id.to_string(),)).await.map_err(provider_category)?;
                            if rows.next().await.map_err(provider_category)?.is_none() { return Err(StoreError::CorruptStoredData); }
                            let changed = connection.execute("UPDATE effect_identity_users SET concurrency_stamp=?2,payload=?3 WHERE id=?1 AND concurrency_stamp=?4", (self.id.to_string(), updated.concurrency_stamp().to_string(), serialize_user(&updated)?, expected.to_string())).await.map_err(provider_category)?;
                            ensure_rows_changed(changed, StoreError::TransactionConflict)?;
                        } else {
                            validate_email_claim(&connection,&updated,false).await?;
                            save_user(&connection,&updated,&expected).await?;
                        }
                        Ok(updated)
                    }.await;
                result.map_err(|error| diagnostics.command_error(error))
            }
        }
        self.kernel
            .transaction_operation(Contact { id, update })
            .await
            .map_err(|error| self.narrow(error))
    }
}
/// Rotate credential/security metadata inside the caller's existing writer transaction.
/// Protected lookup aliases remain unchanged; no read/write workflow atomicity is implied.
async fn rotate_security_stamp(
    connection: &StoreConnection<'_>,
    id: &UserId,
    disable_two_factor: bool,
) -> Result<(), StoreError> {
    let mut user = find_user_by_id(connection, id)
        .await?
        .ok_or(StoreError::NotFound)?;
    if disable_two_factor {
        user = user.with_two_factor_enabled(false);
    }
    user = user
        .with_refreshed_security_stamp()
        .with_refreshed_concurrency_stamp();
    let changed = connection
        .execute(
            "UPDATE effect_identity_users SET concurrency_stamp=?2,payload=?3 WHERE id=?1",
            (
                id.to_string(),
                user.concurrency_stamp().to_string(),
                serialize_user(&user)?,
            ),
        )
        .await
        .map_err(provider_category)?;
    ensure_rows_changed(changed, StoreError::NotFound)
}

/// Serialize a borrowed native aggregate with its newly committed revision.
/// The explicit private DTO follows v1 security fields; no JSON DOM or credential
/// clone is used to work around upstream's accessor-only update command.
pub(super) fn serialize_refreshed_user(
    user: &IdentityUser,
    stamp: &ConcurrencyStamp,
) -> Result<String, StoreError> {
    #[derive(serde::Serialize)]
    /// Borrowed private v1 account persistence representation, separate from views.
    struct Persistence<'a> {
        /// Stable native account identity.
        id: UserId,
        /// Optional validated display alias.
        user_name: Option<&'a identity_rs::identity::models::UserName>,
        /// Selected normalized lookup alias.
        normalized_user_name: Option<&'a NormalizedUserName>,
        /// Optional personal contact selected by the native manager.
        email: Option<&'a EmailAddress>,
        /// Selected-normalizer contact lookup.
        normalized_email: Option<&'a NormalizedEmailAddress>,
        /// Native destination confirmation, not independently minted proof.
        email_confirmed: bool,
        /// Optional validated phone contact.
        phone_number: Option<&'a PhoneNumber>,
        /// Native phone confirmation result.
        phone_number_confirmed: bool,
        /// Native validated hash, never public-view serialized.
        password_hash: Option<&'a identity_rs::identity::password::PasswordHashString>,
        /// Explicit UTC lockout deadline.
        lockout_end: Option<OffsetDateTime>,
        /// Selected account's persisted lockout participation.
        lockout_enabled: bool,
        /// Acknowledged attempt counter; only store increments are atomic.
        access_failed_count: u32,
        /// Persisted native factor policy.
        two_factor_enabled: bool,
        /// Native credential-invalidating version.
        security_stamp: &'a identity_rs::identity::models::SecurityStamp,
        /// New revision minted only after checking the expected version.
        concurrency_stamp: &'a ConcurrencyStamp,
    }
    super::schema::encode_security(&Persistence {
        id: user.id(),
        user_name: user.user_name(),
        normalized_user_name: user.normalized_user_name(),
        email: user.email(),
        normalized_email: user.normalized_email(),
        email_confirmed: user.email_confirmed(),
        phone_number: user.phone_number(),
        phone_number_confirmed: user.phone_number_confirmed(),
        password_hash: user.password_hash(),
        lockout_end: user.lockout_end(),
        lockout_enabled: user.lockout_enabled(),
        access_failed_count: user.access_failed_count(),
        two_factor_enabled: user.two_factor_enabled(),
        security_stamp: user.security_stamp(),
        concurrency_stamp: stamp,
    })
}
/// Serialize the borrowed role update using its stable ID and fresh persisted stamp.
/// Native public Deserialize validates the owned acknowledged result on reload.
fn serialize_refreshed_role(
    role: &IdentityRole,
    stamp: &ConcurrencyStamp,
) -> Result<String, StoreError> {
    #[derive(serde::Serialize)]
    /// Version-one role persistence fields, independent of HTTP account views.
    struct Persistence<'a> {
        /// Stable role identity preserved across rename.
        id: RoleId,
        /// Validated selected display name.
        name: Option<&'a identity_rs::identity::models::UserName>,
        /// Selected-normalizer equality key.
        normalized_name: Option<&'a NormalizedUserName>,
        /// Fresh committed revision following a successful expected-version check.
        concurrency_stamp: &'a ConcurrencyStamp,
    }
    super::schema::encode_security(&Persistence {
        id: role.id(),
        name: role.name(),
        normalized_name: role.normalized_name(),
        concurrency_stamp: stamp,
    })
}
