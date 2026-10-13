//! Finite V04 grouped fixtures for native atomic contracts and selected capabilities.
//! These use real local SQLx/Turso transactions and native validators/providers;
//! no live provider, crypto certification, or alternative persistence is involved.
#![cfg(feature = "sqlx-turso")]

use effect::{effect, effect_sync};
use effect_server::{reply, router};
use effect_server_identity::{
    IdentityError, IdentityStoragePolicy, SqlxIdentityStore,
    authority::{
        GrantPolicy, IdentityAuthority, IdentityClock, PermissionDecision, PermissionOperation,
        PermissionTarget,
    },
    capabilities::StorageBound,
    store::{
        components::{KeyDocument, KeyRepository},
        records::{RecordCategory, RecordCodec, RecordError, RecordPolicy, SqlRecordAdapter},
    },
};
use effect_server_identity::{identity_endpoints, identity_store};
use identity_rs::identity::{
    StoreError,
    models::*,
    store::*,
    tokens::{UserTokenProvider, data_protection::DataProtectorTokenProvider},
};
use std::num::{NonZeroU16, NonZeroUsize};
use time::{Duration, OffsetDateTime};
use tokio::join;
use uuid::Uuid;

/// One isolated local database, with actual schema and independent pool leases.
async fn backend() -> Result<SqlxIdentityStore, Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!("effect-identity-v04-{}.sqlite", Uuid::new_v4()));
    let options = sqlx::TursoConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .pragma("foreign_keys", Some("ON".into()));
    let pool = sqlx::TursoPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;
    let store = SqlxIdentityStore::new(
        pool,
        IdentityStoragePolicy {
            challenge_capacity: NonZeroUsize::MIN,
            utc_now: OffsetDateTime::now_utc,
        },
    )?;
    store.initialize().await?;
    Ok(store)
}
/// Prepare only validated creation fields; persistence supplies actual atomic state.
async fn account(
    store: &SqlxIdentityStore,
    name: &str,
) -> Result<IdentityUser, Box<dyn std::error::Error>> {
    let user = IdentityUser::with_user_name(UserName::parse(name)?)
        .with_normalized_user_name(Some(NormalizedUserName::parse(name.to_ascii_uppercase())?))?;
    Ok(store
        .create_user(NewUser::from_aggregate(user, false)?)
        .await?)
}
/// Exercise normalized uniqueness, expected revisions and serialized counters.
#[tokio::test]
async fn aggregate_uniqueness_revision_and_counter() -> Result<(), Box<dyn std::error::Error>> {
    let store = backend().await?;
    let user = account(&store, "Maya").await?;
    let id = user.id();
    assert!(account(&store, "Maya").await.is_err());
    let previous = user.concurrency_stamp().clone();
    let current = store
        .update_user(UserUpdate::new(user, previous.clone())?)
        .await?;
    assert_ne!(current.concurrency_stamp(), &previous);
    assert!(matches!(
        store.update_user(UserUpdate::new(current, previous)?).await,
        Err(StoreError::TransactionConflict)
    ));
    let (a, b) = join!(
        store.increment_access_failed_count(&id),
        store.increment_access_failed_count(&id)
    );
    let mut counts = [a?, b?];
    counts.sort();
    assert_eq!(counts, [1, 2]);
    account(&store, "SecondCustomer").await?;
    let page = store.list_users_page(None, NonZeroU16::MIN).await?;
    assert_eq!(page.users().len(), 1);
    let cursor = page.next().ok_or("missing bounded continuation")?;
    let next = store.list_users_page(Some(cursor), NonZeroU16::MIN).await?;
    assert_eq!(next.users().len(), 1);
    assert!(next.next().is_none());
    Ok(())
}
/// Exercise exact token replacement, recovery consumption and native passkey CAS.
#[tokio::test]
async fn exact_tokens_recovery_and_passkey_revision() -> Result<(), Box<dyn std::error::Error>> {
    let store = backend().await?;
    let id = account(&store, "ReceiptCustomer").await?.id();
    let provider = LoginProvider::parse("Cafe")?;
    let name = AuthTokenName::parse("refresh")?;
    store
        .set_authentication_token(
            &id,
            LoginProvider::parse("Cafe")?,
            AuthTokenName::parse("refresh")?,
            Some(AuthTokenValue::parse("old")?),
        )
        .await?;
    assert!(
        store
            .replace_authentication_token_if_matches(
                &id,
                &provider,
                &name,
                &AuthTokenValue::parse("old")?,
                AuthTokenValue::parse("new")?
            )
            .await?
    );
    assert!(
        !store
            .replace_authentication_token_if_matches(
                &id,
                &provider,
                &name,
                &AuthTokenValue::parse("old")?,
                AuthTokenValue::parse("replay")?
            )
            .await?
    );
    store
        .set_authentication_token(&id, provider, name, None)
        .await?;
    let code = RecoveryCode::parse("CAFE-RECOVERY-ONE")?;
    store
        .replace_recovery_codes(&id, vec![RecoveryCodeHash::hash(&code)])
        .await?;
    assert!(store.redeem_recovery_code(&id, &code).await?);
    assert!(!store.redeem_recovery_code(&id, &code).await?);
    let credential = PasskeyCredentialId::parse(vec![1, 2, 3])?;
    let material = PasskeyCredentialMaterial::new(
        PasskeyPublicKey::parse(vec![1])?,
        PasskeyAttestationObject::parse(vec![2])?,
        PasskeyClientDataJson::parse(vec![3])?,
    );
    let passkey = UserPasskeyInfo::new(
        credential,
        material,
        OffsetDateTime::now_utc(),
        0,
        None,
        PasskeyFlags::new(true, false, false)?,
    )?;
    store.add_or_update_passkey(&id, passkey).await?;
    let credential = PasskeyCredentialId::parse(vec![1, 2, 3])?;
    let expected = store
        .get_passkey(&id, &credential)
        .await?
        .ok_or("missing passkey")?;
    let update = store
        .get_passkey(&id, &credential)
        .await?
        .ok_or("missing passkey")?
        .with_name(Some(PasskeyName::parse("Maya's phone")?));
    assert!(
        store
            .update_passkey_if_matches(&id, &expected, update)
            .await?
    );
    let stale = store
        .get_passkey(&id, &credential)
        .await?
        .ok_or("missing passkey")?;
    assert!(
        !store
            .update_passkey_if_matches(&id, &expected, stale)
            .await?
    );
    Ok(())
}
/// Exercise one-use ceremony and browser pending consumption at actual CAS boundaries.
#[tokio::test]
async fn challenges_and_browser_pending_are_consumed_once() -> Result<(), Box<dyn std::error::Error>>
{
    let store = backend().await?;
    let id = account(&store, "BrowserCustomer").await?.id();
    let now = OffsetDateTime::now_utc();
    let challenge = PasskeyChallenge::parse(vec![9; 32])?;
    store
        .store_passkey_challenge(PasskeyChallengeState::new(
            challenge,
            PasskeyChallengePurpose::Registration,
            Some(id),
            "https://cafe.example",
            "cafe.example",
            true,
            now + Duration::minutes(5),
        )?)
        .await?;
    let challenge = PasskeyChallenge::parse(vec![9; 32])?;
    assert!(
        store
            .take_passkey_challenge(&challenge, PasskeyChallengePurpose::Registration, now)
            .await?
            .is_some()
    );
    assert!(
        store
            .take_passkey_challenge(&challenge, PasskeyChallengePurpose::Registration, now)
            .await?
            .is_none()
    );
    let context_id = Uuid::new_v4();
    let token = AuthTokenValue::parse("ab".repeat(32))?;
    store
        .create_browser_context(
            BrowserContext::new(
                context_id,
                AuthTokenValue::parse("ab".repeat(32))?,
                [7; 32],
                now + Duration::minutes(5),
            )?,
            now,
            2,
        )
        .await?;
    let pending = AuthTokenValue::parse("native-purpose-protected-proof")?;
    assert!(
        store
            .replace_browser_pending_authentication(
                context_id,
                &token,
                None,
                Some(AuthTokenValue::parse("native-purpose-protected-proof")?),
                now
            )
            .await?
    );
    assert!(
        store
            .replace_browser_pending_authentication(context_id, &token, Some(&pending), None, now)
            .await?
    );
    assert!(
        !store
            .replace_browser_pending_authentication(context_id, &token, Some(&pending), None, now)
            .await?
    );
    Ok(())
}
/// Owned non-Clone auxiliary value proves the codec/adapter imposes no payload Clone.
struct Receipt(bool);
/// Explicit fixture codec/version; malformed values fail distinctly from absence.
struct ReceiptCodec;
impl RecordCodec<Receipt> for ReceiptCodec {
    /// One private selected document format.
    fn version(&self) -> NonZeroU16 {
        NonZeroU16::MIN
    }
    /// Encode by borrow, preserving original callback ownership.
    fn encode(&self, value: &Receipt) -> Result<Vec<u8>, RecordError> {
        Ok(vec![u8::from(value.0)])
    }
    /// Reject unknown versions and malformed receipt data.
    fn decode(&self, version: NonZeroU16, bytes: &[u8]) -> Result<Receipt, RecordError> {
        match (version, bytes) {
            (NonZeroU16::MIN, [0]) => Ok(Receipt(false)),
            (NonZeroU16::MIN, [1]) => Ok(Receipt(true)),
            _ => Err(RecordError::Corrupt),
        }
    }
}
/// Exercise key acknowledgment/reload, real native verification and ownership fences.
#[tokio::test]
async fn keys_grants_and_selected_auxiliary_ownership() -> Result<(), Box<dyn std::error::Error>> {
    let store = backend().await?;
    let other = backend().await?;
    let user = account(&store, "KeyCustomer").await?;
    let policy = RecordPolicy {
        max_bytes: NonZeroUsize::new(8192).ok_or("byte bound")?,
        page_limit: NonZeroU16::MIN,
    };
    let id = Uuid::new_v4();
    let keys = KeyRepository::new(store.clone(), "cafe-keys".into(), id, policy);
    let xml = format!(
        r#"<key id="{}" version="1"><creationDate>2020-01-01T00:00:00Z</creationDate><activationDate>2020-01-01T00:00:00Z</activationDate><expirationDate>2030-01-01T00:00:00Z</expirationDate><descriptor deserializerType="AuthenticatedEncryptorDescriptorDeserializer"><descriptor><encryption algorithm="AES_256_CBC"/><validation algorithm="HMACSHA256"/><masterKey><value>{}==</value></masterKey></descriptor></descriptor></key>"#,
        Uuid::new_v4(),
        "A".repeat(86)
    );
    let published = keys.persist(None, KeyDocument::parse(xml)?).await?;
    assert_eq!(
        keys.reload(OffsetDateTime::now_utc()).await?.revision(),
        published.revision()
    );
    let provider = DataProtectorTokenProvider::new(published);
    let token = provider
        .generate(identity_rs::identity::tokens::ACCESS_TOKEN_PURPOSE, &user)
        .await?;
    let token = format!("v1.{}.{}", user.id(), token);
    let policy_grants = GrantPolicy {
        self_lifetime: Duration::minutes(10),
        recent_lifetime: Duration::minutes(2),
    };
    let authority = IdentityAuthority::new(
        store.clone(),
        IdentityClock::new(OffsetDateTime::now_utc),
        policy_grants,
    )?;
    let grant = authority
        .bearer(&provider, &token)
        .await?
        .ok_or("native proof rejected")?;
    let foreign = IdentityAuthority::new(
        other.clone(),
        IdentityClock::new(OffsetDateTime::now_utc),
        policy_grants,
    )?;
    assert!(matches!(
        foreign.revalidate(&grant).await,
        Err(IdentityError::WrongConfiguration)
    ));
    let receipts =
        SqlRecordAdapter::new(store.clone(), RecordCategory::Consent, policy, ReceiptCodec);
    assert!(!StorageBound::belongs_to(&receipts, &other.configuration()));
    assert!(
        identity_store! {
            backend: other,
            consent: receipts,
        }
        .is_err()
    );
    let permission_revision = Uuid::new_v4();
    let target = user.id();
    let expires_at = OffsetDateTime::now_utc() + Duration::minutes(1);
    let permission = authority
        .permission(
            &grant,
            std::sync::Arc::from("cafe.receipts.edit"),
            std::sync::Arc::from("Maya.receipts"),
            move |_, _, _| async move {
                Ok(PermissionDecision {
                    allowed: true,
                    revision: permission_revision,
                    expires_at,
                    target: PermissionTarget::Account(target),
                    operation: PermissionOperation::AccountMutation,
                })
            },
        )
        .await?;
    assert!(matches!(
        authority
            .revalidate_permission(
                &permission,
                "cafe.receipts.read",
                "Maya.receipts",
                move |_, _, _| async move {
                    Ok(PermissionDecision {
                        allowed: true,
                        revision: permission_revision,
                        expires_at,
                        target: PermissionTarget::Account(target),
                        operation: PermissionOperation::AccountMutation,
                    })
                }
            )
            .await,
        Err(IdentityError::StaleGrant)
    ));
    assert!(matches!(
        authority
            .revalidate_permission(
                &permission,
                "cafe.receipts.edit",
                "Maya.receipts",
                move |_, _, _| async move {
                    Ok(PermissionDecision {
                        allowed: true,
                        revision: permission_revision,
                        expires_at,
                        target: PermissionTarget::Accounts,
                        operation: PermissionOperation::AccountsRead,
                    })
                }
            )
            .await,
        Err(IdentityError::StaleGrant)
    ));
    // Callback DSL still acknowledges native persistence before moving the original payload.
    use effect::{Effect, Exit};
    use effect_server_identity::{
        identity, identity_store,
        services::{DeliverySelection, NotificationSelection},
        store::auxiliary::{RequestKind, RequestRepository},
    };
    use std::sync::Arc;
    let requests = identity! {
        storage: identity_store! {
            backend: store.clone(),
            delivery: RequestRepository::new(store.clone(), RequestKind::Delivery, policy, ReceiptCodec),
            notifications: RequestRepository::new(store.clone(), RequestKind::Notification, policy, ReceiptCodec),
        }?,
        native_factory |backend, _clock, _policies| { Ok(backend) },
        errors |error| { error }, clock: IdentityClock::new(OffsetDateTime::now_utc),
        grant_policy: policy_grants, admission: NonZeroUsize::MIN,
    }?;
    let actor = requests
        .authority()
        .bearer(&provider, &token)
        .await?
        .ok_or("callback actor proof")?;
    let delivery_id = Uuid::new_v4();
    let delivery: Effect<Uuid, IdentityError> = identity! {
        identity: requests.clone(), deliver(actor.clone(), delivery_id, None, Receipt(true)),
        transport |accepted| { assert!(accepted.value.0); Ok(accepted.id) },
    };
    assert!(
        requests
            .storage()
            .delivery_store()?
            .find(
                &actor.user_id().to_string(),
                delivery_id,
                OffsetDateTime::now_utc()
            )
            .await?
            .is_none()
    );
    assert!(matches!(delivery.run(Arc::new(())).await, Exit::Success(id) if id == delivery_id));
    assert!(
        requests
            .storage()
            .delivery_store()?
            .find(
                &actor.user_id().to_string(),
                delivery_id,
                OffsetDateTime::now_utc()
            )
            .await?
            .is_some()
    );
    let notification_id = Uuid::new_v4();
    let notification: Effect<(), IdentityError> = identity! {
        identity: requests.clone(), notify(actor.clone(), notification_id, None, Receipt(false)),
        dispatch |accepted| {
            assert!(!accepted.value.0);
            Err(IdentityError::Configuration("fixture dispatch rejected"))
        },
    };
    assert!(matches!(
        notification.run(Arc::new(())).await,
        Exit::Fail(IdentityError::Configuration("fixture dispatch rejected"))
    ));
    assert!(
        requests
            .storage()
            .notification_store()?
            .find(
                &actor.user_id().to_string(),
                notification_id,
                OffsetDateTime::now_utc()
            )
            .await?
            .is_some()
    );
    let current = store
        .find_user_by_id(&user.id())
        .await?
        .ok_or("missing current user")?;
    let revision = current.concurrency_stamp().clone();
    store
        .update_user(UserUpdate::new(
            current.with_refreshed_security_stamp(),
            revision,
        )?)
        .await?;
    assert!(matches!(
        authority.revalidate(&grant).await,
        Err(IdentityError::StaleGrant)
    ));
    let receipts = SqlRecordAdapter::new(store, RecordCategory::Consent, policy, ReceiptCodec);
    let key = Uuid::new_v4();
    let revision = receipts.create("Maya", key, None, Receipt(true)).await?;
    assert!(
        receipts
            .replace("Maya", key, Uuid::new_v4(), None, Receipt(false))
            .await
            .is_err()
    );
    assert!(receipts.remove("Maya", key, revision).await?);
    Ok(())
}

/// Extracted factory expressions retain field order; native work and owned inputs stay lazy.
#[tokio::test]
async fn dsl_factory_order_and_once_ownership() -> Result<(), Box<dyn std::error::Error>> {
    use effect::{Effect, Exit};
    use effect_server_identity::{identity, policies::IdentityPolicies};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    let store = backend().await?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = calls.clone();
    let selected = identity! {
        storage: { events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(0); identity_store! { backend: store }? },
        native_factory: {
            events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(1);
            move |backend: SqlxIdentityStore, _clock: IdentityClock, _policies: Arc<IdentityPolicies>| {
                factory_calls.fetch_add(1, Ordering::SeqCst); Ok(backend)
            }
        },
        errors: { events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(2); std::convert::identity::<IdentityError> },
        clock: IdentityClock::new(OffsetDateTime::now_utc),
        grant_policy: GrantPolicy { self_lifetime: Duration::minutes(5), recent_lifetime: Duration::minutes(1) },
        admission: NonZeroUsize::MIN,
    }?;
    assert_eq!(
        *observed.lock().map_err(|_| "field order mutex")?,
        vec![0, 1, 2]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let route_events = observed.clone();
    let _routes: effect_server::Router<IdentityError, effect_server::RequestEnv<()>> = identity_endpoints! {
        identity: selected.clone(), prefix: "/",
        route {
            method: { route_events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(3); http::Method::GET },
            path: "/first",
            input: { route_events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(4); identity_request },
            workflow |_identity, request| { request },
            render |_request| { reply! { native(http::Response::new(axum::body::Body::empty())) } },
        },
        include: { route_events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(5); router! {}? },
        route {
            method: { route_events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(6); http::Method::POST },
            path: "/second", input: identity_request,
            workflow |_identity, request| { request },
            render |_request| { reply! { native(http::Response::new(axum::body::Body::empty())) } },
        },
    }?;
    assert_eq!(
        *observed.lock().map_err(|_| "field order mutex")?,
        vec![0, 1, 2, 3, 4, 5, 6]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    /// This owned input intentionally does not implement Clone.
    struct Receipt(String);
    let read: Effect<String, IdentityError> = identity! {
        identity: selected.clone(), invoke_once(Receipt("one receipt".into())),
        native |_backend, receipt, _authority, _policies| { Ok(receipt.0) },
    };
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        matches!(read.clone().run(Arc::new(())).await, Exit::Success(value) if value == "one receipt")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(read.run(Arc::new(())).await, Exit::Die(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let rejected: Effect<(), IdentityError> = identity! {
        identity: selected, authorize_once(()),
        authorize |_input, _authority, _policies| { Err(IdentityError::Unauthorized) },
    };
    assert!(matches!(
        rejected.run(Arc::new(())).await,
        Exit::Fail(IdentityError::Unauthorized)
    ));
    Ok(())
}

/// Extracted typed request recipe; its payload transfers only when the route runs.
fn identity_request(
    request: effect_server::Request,
) -> effect::Effect<effect_server::Request, IdentityError, effect_server::RequestEnv<()>> {
    effect_sync! { once; body { request }, }
}

/// Native role/permission policies use current verified claims and skip denied guards.
#[tokio::test]
async fn native_authorization_roles_permissions_and_guard_order()
-> Result<(), Box<dyn std::error::Error>> {
    use effect::{Cancellation, Exit};
    use effect_server::Request;
    use effect_server_identity::{
        components::{NativePrincipalPolicy, Retained},
        identity, identity_store,
    };
    use identity_rs::identity::{
        axum::{AuthenticatedIdentity, RoleClaimAuthorizationPolicy},
        error::DefaultIdentityErrorDescriber,
        manager::{SignInManager, UserManager},
        normalization::UpperInvariantLookupNormalizer,
        password::{
            DefaultPasswordHasher, DefaultPasswordValidator, OsRandomBytes, PasswordHasherOptions,
        },
        principal::{ClaimsIdentity, ClaimsIdentityOptions, ClaimsPrincipal},
        ticket::AuthenticationTicket,
        validation::DefaultUserValidator,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let store = backend().await?;
    let user = account(&store, "PolicyCustomer").await?;
    store
        .create_role(NewRole::new(UserName::parse("Customer")?)?)
        .await?;
    let role = NormalizedUserName::parse("CUSTOMER")?;
    store.add_user_to_role(&user.id(), role.clone()).await?;
    let read_claim = IdentityClaim::new(
        ClaimType::parse("permission")?,
        ClaimValue::parse("pickup.read")?,
    );
    store
        .add_user_claims(&user.id(), vec![read_claim.clone()])
        .await?;
    let keys = KeyRepository::new(
        store.clone(),
        "policy-keys".into(),
        Uuid::new_v4(),
        RecordPolicy {
            max_bytes: NonZeroUsize::new(8192).ok_or("key bytes")?,
            page_limit: NonZeroU16::MIN,
        },
    );
    let xml = format!(
        r#"<key id="{}" version="1"><creationDate>2020-01-01T00:00:00Z</creationDate><activationDate>2020-01-01T00:00:00Z</activationDate><expirationDate>2030-01-01T00:00:00Z</expirationDate><descriptor deserializerType="AuthenticatedEncryptorDescriptorDeserializer"><descriptor><encryption algorithm="AES_256_CBC"/><validation algorithm="HMACSHA256"/><masterKey><value>{}==</value></masterKey></descriptor></descriptor></key>"#,
        Uuid::new_v4(),
        "A".repeat(86)
    );
    let provider =
        DataProtectorTokenProvider::new(keys.persist(None, KeyDocument::parse(xml)?).await?);
    let user = store
        .find_user_by_id(&user.id())
        .await?
        .ok_or("policy account")?;
    let token = provider
        .generate(identity_rs::identity::tokens::ACCESS_TOKEN_PURPOSE, &user)
        .await?;
    let token = format!("v1.{}.{}", user.id(), token);
    let validator = Retained::new(DefaultUserValidator::new());
    let hasher = Retained::new(DefaultPasswordHasher::with_options_and_random(
        PasswordHasherOptions::new(100_000)?,
        OsRandomBytes,
    ));
    let passwords = Retained::new(DefaultPasswordValidator::new());
    let normalizer = Retained::new(UpperInvariantLookupNormalizer::new());
    let selected = identity! {
        storage: identity_store! { backend: store.clone() }?,
        native_factory |backend, clock, _policies| {
            let users = UserManager::with_components_and_error_describer(backend, validator.clone(), hasher.clone(), passwords.clone(), normalizer.clone(), DefaultIdentityErrorDescriber)
                .with_utc_now(clock.callable()).with_password_work_limit(NonZeroUsize::MIN).map_err(IdentityError::Native)?;
            Ok(SignInManager::new(users))
        },
        principal_factory: NativePrincipalPolicy::new(ClaimsIdentityOptions::default()),
        errors |error| { error }, clock: IdentityClock::new(OffsetDateTime::now_utc),
        grant_policy: GrantPolicy { self_lifetime: Duration::minutes(5), recent_lifetime: Duration::minutes(1) }, admission: NonZeroUsize::MIN,
    }?;
    let request = Request::from_native(
        http::Request::builder()
            .uri("/pickup")
            .body(axum::body::Body::empty())?,
        Cancellation::new(),
    );
    // A public native ticket extension cannot supply this definition's private proof.
    request.insert_extension(AuthenticatedIdentity::new(AuthenticationTicket::new(
        ClaimsPrincipal::new(ClaimsIdentity::new("forged", "name", "role")),
    )));
    let group = identity! {
        identity: &selected,
        authorization_stage(RoleClaimAuthorizationPolicy::require_role("CUSTOMER")),
    };
    assert!(matches!(
        group(request.clone()).run(Arc::new(())).await,
        Exit::Fail(IdentityError::Policy(http::StatusCode::UNAUTHORIZED))
    ));
    let actor = selected
        .authority()
        .bearer(&provider, &token)
        .await?
        .ok_or("policy proof")?;
    request.insert_extension(actor);
    assert!(matches!(
        group(request.clone()).run(Arc::new(())).await,
        Exit::Success(())
    ));
    let read = identity! {
        identity: &selected,
        authorization_stage(RoleClaimAuthorizationPolicy::require_permission("pickup.read")),
    };
    assert!(matches!(
        read(request.clone()).run(Arc::new(())).await,
        Exit::Success(())
    ));
    let collect = identity! {
        identity: &selected,
        authorization_stage(RoleClaimAuthorizationPolicy::require_permission("pickup.collect")),
    };
    assert!(matches!(
        collect(request.clone()).run(Arc::new(())).await,
        Exit::Fail(IdentityError::Policy(http::StatusCode::FORBIDDEN))
    ));
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let guarded = identity! {
        identity: &selected,
        authorization_guard(RoleClaimAuthorizationPolicy::require_role("ADMIN")),
        guard: move |_: Arc<()>, _request| {
            observed.fetch_add(1, Ordering::SeqCst);
            effect! { () }
        },
    };
    assert!(matches!(
        guarded(Arc::new(()), request.clone())
            .run(Arc::new(()))
            .await,
        Exit::Fail(IdentityError::Policy(http::StatusCode::FORBIDDEN))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // Revocation is read freshly, even when the original credential/grant is retained.
    store.remove_user_claims(&user.id(), &[read_claim]).await?;
    assert!(matches!(
        read(request.clone()).run(Arc::new(())).await,
        Exit::Fail(IdentityError::Policy(http::StatusCode::FORBIDDEN))
    ));
    store.remove_user_from_role(&user.id(), &role).await?;
    assert!(matches!(
        group(request).run(Arc::new(())).await,
        Exit::Fail(IdentityError::Policy(http::StatusCode::FORBIDDEN))
    ));
    Ok(())
}

/// A native defect stays Die, bypasses expected-error recovery, and evaluates its expression once.
#[tokio::test]
async fn defect_macro_retains_the_original_channel() {
    use effect::{Defect, Effect, Exit, effect_die, effect_recover, effect_sync};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let constructed = Arc::new(AtomicUsize::new(0));
    let witness = constructed.clone();
    let program: Effect<(), IdentityError> = effect_die! {
        defect: {
        witness.fetch_add(1, Ordering::SeqCst);
        Defect::new("native fixture defect")
    }, };
    assert_eq!(constructed.load(Ordering::SeqCst), 1);
    let recovered = Arc::new(AtomicUsize::new(0));
    let called = recovered.clone();
    let program = effect_recover! {
        program: program, recover |_error| {
        let called = called.clone();
        effect_sync! { body { called.fetch_add(1, Ordering::SeqCst); () }, }?
    }, };
    assert!(matches!(
        program.clone().run(Arc::new(())).await,
        Exit::Die(_)
    ));
    assert!(matches!(program.run(Arc::new(())).await, Exit::Die(_)));
    assert_eq!(constructed.load(Ordering::SeqCst), 1);
    assert_eq!(recovered.load(Ordering::SeqCst), 0);
}
