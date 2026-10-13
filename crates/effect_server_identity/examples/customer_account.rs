//! Register Maya, verify her password with native Identity, then save her receipt preference.
//!
//! Run: `CAFE_DATABASE=target/receipt-accounts.db cargo +1.99.0 run -p effect-server-identity --features sqlx-turso --example customer_account`.
//! Expected: `Maya registered, logged in, and saved email_receipts=true.`
//! The native manager enforces its account/password policy, OS-backed password salt,
//! two concurrent password operations and 100,000 hash iterations. The local SQLx
//! store holds accounts and the private versioned preference. Each run creates a
//! fresh alias; no HTTP listener, cookie keys, passkeys or external provider is needed.

#[cfg(feature = "sqlx-turso")]
mod account {
    use effect::{Effect, Exit, effect, effect_die, effect_try_sync};
    use effect_server_identity::{
        IdentityError, IdentityStoragePolicy, SqlxIdentityStore,
        authority::{
            GrantPolicy, IdentityClock, PermissionDecision, PermissionOperation, PermissionTarget,
        },
        components::{NativePrincipalPolicy, Retained},
        definition::{LoginInput, LoginOutcome, RegistrationInput},
        identity, identity_store,
        store::{
            accounts::AtomicAccounts,
            records::{RecordCategory, RecordCodec, RecordError, RecordPolicy, SqlRecordAdapter},
        },
    };
    use identity_rs::identity::{
        IdentityOptions, IdentityResult,
        error::DefaultIdentityErrorDescriber,
        manager::{SignInManager, SignInPolicy, UserManager},
        models::{IdentityUser, UserId, UserName},
        normalization::UpperInvariantLookupNormalizer,
        password::{
            DefaultPasswordHasher, DefaultPasswordValidator, OsRandomBytes, PasswordHasherOptions,
        },
        principal::ClaimsIdentityOptions,
        tokens::AuthenticatorTokenProvider,
        validation::DefaultUserValidator,
    };
    use std::{
        num::{NonZeroU16, NonZeroUsize},
        sync::Arc,
    };
    use time::{Duration, OffsetDateTime};

    /// Native registration/login grant the authority required to persist a preference.
    pub async fn register_and_save() -> Result<(), Box<dyn std::error::Error>> {
        let pool = sqlx::TursoPool::connect_with(
            sqlx::TursoConnectOptions::new()
                .filename(std::env::var("CAFE_DATABASE")?)
                .create_if_missing(true)
                .pragma("foreign_keys", Some("ON".into())),
        )
        .await?;
        let result = async {
            let backend = SqlxIdentityStore::new(pool.clone(), IdentityStoragePolicy {
                challenge_capacity: NonZeroUsize::MIN.saturating_add(15),
                utc_now: OffsetDateTime::now_utc,
            })?;
            backend.initialize().await?;
            let document = RecordPolicy { max_bytes: NonZeroUsize::MIN.saturating_add(1023),
                page_limit: NonZeroU16::MIN.saturating_add(15) };
            let stores = identity_store! {
                backend: backend.clone(),
                accounts: AtomicAccounts::new(backend.clone(), NonZeroUsize::MIN.saturating_add(15)),
                consent: SqlRecordAdapter::new(backend, RecordCategory::Consent, document, PreferenceCodec),
            }?;
            let validator = Retained::new(DefaultUserValidator::new());
            let hasher = Retained::new(DefaultPasswordHasher::with_options_and_random(PasswordHasherOptions::new(100_000)?, OsRandomBytes));
            let password_validator = Retained::new(DefaultPasswordValidator::new());
            let normalizer = Retained::new(UpperInvariantLookupNormalizer::new());
            let authenticator = Retained::new(AuthenticatorTokenProvider::new());
            let admission = Retained::new(AccountAdmission);
            let accounts = identity! {
                storage: stores,
                native_factory |backend, clock, _policies| {
                    let users = UserManager::with_components_and_error_describer(
                        backend, validator.clone(), hasher.clone(), password_validator.clone(), normalizer.clone(), DefaultIdentityErrorDescriber,
                    ).with_utc_now(clock.callable())
                     .with_password_work_limit(NonZeroUsize::MIN.saturating_add(1)).map_err(IdentityError::Native)?
                     .with_options(IdentityOptions::default()).map_err(IdentityError::Native)?;
                    Ok(SignInManager::new(users).with_authenticator_token_provider(authenticator.clone())
                        .with_sign_in_policy(admission.clone()).with_authentication_scheme("Receipt.Account"))
                },
                grant_policy: GrantPolicy { self_lifetime: Duration::seconds(300), recent_lifetime: Duration::seconds(120) },
                clock: IdentityClock::new(OffsetDateTime::now_utc), errors |error| { error },
                admission: NonZeroUsize::MIN.saturating_add(15),
                principal_factory: NativePrincipalPolicy::new(ClaimsIdentityOptions::default()),
            }?;
            let program: Effect<(), IdentityError> = effect! {
                let accounts = accounts.clone();
                let name = match UserName::parse(format!("maya-{}", uuid::Uuid::new_v4())) {
                    Ok(name) => name,
                    Err(error) => { let defect = effect::Defect::new(format!("customer alias: {error:?}")); effect_die! { defect: defect, }? },
                };
                let registered = accounts.register(RegistrationInput {
                    user: IdentityUser::with_user_name(name), password: "Cafe-Demo-Receipt-42!".into(),
                })?;
                let user_name = match registered.user_name {
                    Some(name) => name,
                    None => effect_try_sync! { body { Err(IdentityError::Unauthorized) }, }?,
                };
                let actor = match accounts.login(LoginInput {
                    user_name,
                    password: "Cafe-Demo-Receipt-42!".into(), persistent: false, lockout_on_failure: true,
                })? {
                    LoginOutcome::Completed { actor, .. } => actor,
                    LoginOutcome::Rejected(_) | LoginOutcome::Pending(_) =>
                        effect_try_sync! { body { Err(IdentityError::Unauthorized) }, }?,
                };
                // Scope permits this collection; the per-record rule permits only Maya.
                let scope = identity! {
                    identity: accounts.clone(), authorize_once(actor.clone()),
                    authorize |actor, authority, _policies| {
                        let permission = authority.permission(&actor, Arc::from("receipts.read"), Arc::from("accounts"), receipt_visibility).await?;
                        authority.scope(permission)
                    },
                }?;
                let visible = identity! {
                    identity: accounts.clone(), accounts_page(scope, None, NonZeroU16::MIN.saturating_add(15)),
                    decide: receipt_visibility,
                    allow |actor, target| { Ok(actor == target) },
                }?;
                assert!(visible.accounts.iter().all(|account| account.id == actor.user_id()));
                let preference_id = uuid::Uuid::new_v4();
                accounts.update_preferences(actor.clone(), preference_id, None, None,
                    ReceiptPreference { email_receipts: true })?;
                let saved = accounts.preferences(actor, preference_id)?;
                assert!(saved.is_some_and(|saved| saved.value.email_receipts));
                use std::io::Write;
                let written = writeln!(std::io::stdout().lock(), "Maya registered, logged in, and saved email_receipts=true.");
                match written { Ok(()) => (), Err(error) => { let defect = effect::Defect::from_io(error); effect_die! { defect: defect, }? }, };
            };
            match program.run(Arc::new(())).await {
                Exit::Success(()) => Ok(()),
                other @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => Err(format!("account workflow failed: {other:?}").into()),
            }
        }.await;
        pool.close().await;
        result
    }
    /// One private, versioned preference; it is not a security-account codec.
    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct ReceiptPreference {
        /// Maya's explicit receipt choice.
        pub email_receipts: bool,
    }
    /// Café's selected version-one preference codec.
    pub struct PreferenceCodec;
    impl RecordCodec<ReceiptPreference> for PreferenceCodec {
        /// Stable private document generation.
        fn version(&self) -> NonZeroU16 {
            NonZeroU16::MIN
        }
        /// Encode only the café preference, without native credentials.
        fn encode(&self, value: &ReceiptPreference) -> Result<Vec<u8>, RecordError> {
            serde_json::to_vec(value).map_err(|_| RecordError::Codec("receipt preference".into()))
        }
        /// Reject unknown versions/fields rather than create default consent.
        fn decode(
            &self,
            version: NonZeroU16,
            bytes: &[u8],
        ) -> Result<ReceiptPreference, RecordError> {
            if version != NonZeroU16::MIN {
                return Err(RecordError::Corrupt);
            }
            serde_json::from_slice(bytes).map_err(|_| RecordError::Corrupt)
        }
    }
    /// Native admission still requires successful password/factor/eligibility verification.
    #[derive(Debug)]
    pub struct AccountAdmission;
    impl SignInPolicy for AccountAdmission {
        /// The café accepts currently named accounts; this cannot waive native verification.
        fn allows(&self, user: &IdentityUser) -> Result<bool, IdentityResult> {
            Ok(user.user_name().is_some())
        }
    }
    /// Reusable application policy for the receipt account collection.
    /// The workflow rechecks its stable revision and applies visibility per record.
    async fn receipt_visibility(
        _actor: UserId,
        action: Arc<str>,
        resource: Arc<str>,
    ) -> Result<PermissionDecision, IdentityError> {
        Ok(PermissionDecision {
            allowed: action.as_ref() == "receipts.read" && resource.as_ref() == "accounts",
            revision: uuid::Uuid::from_u128(1),
            expires_at: OffsetDateTime::now_utc() + Duration::seconds(300),
            target: PermissionTarget::Accounts,
            operation: PermissionOperation::AccountsRead,
        })
    }
}

/// The caller owns Tokio; SQL/native cleanup completes before returning.
#[cfg(feature = "sqlx-turso")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    account::register_and_save().await
}

/// An unavailable optional SQL backend leaves the example inert.
#[cfg(not(feature = "sqlx-turso"))]
fn main() {}
