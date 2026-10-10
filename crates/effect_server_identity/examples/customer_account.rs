//! Maya creates a café account and saves her receipt preference in local SQLx/Turso.
//! Run with `cargo run -p effect-server-identity --features sqlx-turso --example customer_account`.
//! Caller configuration is required: CAFE_DATABASE, CAFE_BODY_BYTES, CAFE_DOCUMENT_BYTES,
//! CAFE_PAGE_SIZE, CAFE_CHALLENGE_CAPACITY, CAFE_TOKEN_CAPACITY, CAFE_HASH_ITERATIONS,
//! CAFE_PASSWORD_WORK, CAFE_ADMISSION, CAFE_SELF_SECONDS, CAFE_RECENT_SECONDS.
//! Integer limits must be positive; grant durations are seconds. The database path
//! is caller-selected. Native IdentityOptions::default selects the documented upstream
//! account/password policy; the native manager retains its OS-backed enrollment generator.
//! D17 selects real native/auxiliary persistence, D18 selects actual native managers,
//! and D19 serves all ten semantic groups plus extracted/custom routes during one
//! finite local cookie-login/account-view exchange. No external provider is started;
//! external challenge metadata never proves identity.
//! Browser mode additionally selects NativeBrowserPolicy and NativeCookiePolicy;
//! stored mode selects SessionRepository with an acknowledged KeyRepository snapshot;
//! bearer mode selects NativeBearerPolicy and persists refresh state before returning it.
//! The primary login uses bearer issuance; custom cookie/stored login routes select
//! their native modes. The scoped Server host mounts these descriptions for the exchange.
//! Cookie-authenticated unsafe requests require native browser/antiforgery proof;
//! public entrypoints remain available without a pre-existing authenticated grant.
//! Additional caller configuration: CAFE_KEY_ID (UUID), CAFE_KEY_XML (native XML file
//! used on first installation), CAFE_COOKIE_NAME, CAFE_ORIGIN, CAFE_RP_ID,
//! CAFE_USER_VERIFICATION (required/preferred/discouraged), CAFE_SESSION_SECONDS,
//! CAFE_ALLOW_REFRESH (bool), CAFE_EXTERNAL_SCHEME. Keys persist in the same SQL database.
//! Host configuration: CAFE_WORKERS, CAFE_TASK_CAPACITY, CAFE_CHUNK_BYTES, CAFE_FRAMES,
//! CAFE_DRAIN_MS and CAFE_LISTEN (a local SocketAddr, with port 0 for an allocated port).

#[cfg(feature = "sqlx-turso")]
mod cafe {
    use axum::response::IntoResponse;
    use effect::{
        AdmissionCapacity, Cancellation, Effect, Exit, HasCancellation, HasTaskSpawner, effect,
    };
    use effect_server::{
        BodyPolicy, HasRequest, HasTokio, HostPolicy, HttpProtocols, Reply, Request,
        RequestBinding, ServerPolicy, endpoint, pipeline, reply, server,
    };
    use effect_server_identity::{
        IdentityError, IdentityStoragePolicy, SqlxIdentityStore,
        authority::{GrantPolicy, IdentityClock, SelfGrant},
        components::{
            NativeBearerPolicy, NativeBrowserPolicy, NativeCookiePolicy, NativePrincipalPolicy,
            Retained,
        },
        definition::{ConfiguredComponents, LoginInput, LoginOutcome, RegistrationInput},
        endpoints::{EndpointError, WorkflowGroup, WorkflowRoutes},
        identity, identity_endpoints, identity_store,
        store::{
            accounts::AtomicAccounts,
            components::{KeyDocument, KeyRepository, SessionRepository},
            records::{RecordCategory, RecordCodec, RecordError, RecordPolicy, SqlRecordAdapter},
        },
        workflows::{AccountView, AuthenticatorCompletion, IssuedBearer, PasswordRecovery},
    };
    use effect_tokio::TokioTasks;
    use identity_rs::identity::{
        IdentityOptions, IdentityResult,
        axum::dto::PublicKeyCredentialRequestOptionsResponse,
        error::DefaultIdentityErrorDescriber,
        manager::{SignInManager, SignInPolicy, UserManager},
        models::{IdentityUser, LoginProvider, PasskeyCredentialId, UserId, UserName},
        normalization::UpperInvariantLookupNormalizer,
        passkeys::{
            PasskeyAssertionResponse, PasskeyUserVerificationRequirement, StaticPasskeyPolicy,
        },
        password::{
            DefaultPasswordHasher, DefaultPasswordValidator, OsRandomBytes, PasswordHasherOptions,
        },
        principal::ClaimsIdentityOptions,
        store::UserStore,
        ticket::{
            CookieSessionManager, CookieSessionOptions, DataProtectionTicketFormat,
            TicketDataFormatOptions, default_ticket_purposes,
        },
        tokens::{
            AuthenticatorTokenProvider, DEFAULT_BEARER_ACCESS_TOKEN_EXPIRES_IN_SECONDS,
            data_protection::DataProtectorTokenProvider,
        },
        validation::DefaultUserValidator,
    };
    use std::{
        num::{NonZeroU16, NonZeroU64, NonZeroUsize},
        sync::Arc,
    };
    use time::{Duration, OffsetDateTime};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Café-owned expected errors retain their actual domain and transport sources.
    #[derive(Debug)]
    pub enum CafeError {
        /// Native account or selected persistence failed.
        Identity(IdentityError),
        /// HTTP input/body ownership or syntax was invalid.
        Input(effect_server::ServerError),
        /// Public registration fields failed validation.
        Form,
    }
    impl std::fmt::Display for CafeError {
        /// Public feedback exposes categories while retaining typed causes.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Identity(error) => write!(f, "{error}"),
                Self::Input(error) => write!(f, "{error}"),
                Self::Form => f.write_str("invalid café registration"),
            }
        }
    }
    impl std::error::Error for CafeError {
        /// Original typed diagnostics remain available to trusted application reporting.
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            match self {
                Self::Identity(error) => Some(error),
                Self::Input(error) => Some(error),
                Self::Form => None,
            }
        }
    }
    /// Request environment selected by the Server host for this route.
    pub struct CafeRequest {
        /// Native request shared with the selected scoped route.
        request: Request,
        /// Caller-owned multithread runtime retained through host/provider drain.
        runtime: Arc<tokio::runtime::Runtime>,
        /// Existing admitted task adapter; scheduling remains an R capability.
        tasks: TokioTasks,
    }
    impl HasRequest for CafeRequest {
        /// Borrow the same single-owner native request used by the endpoint.
        fn request(&self) -> &Request {
            &self.request
        }
    }
    impl HasTokio for CafeRequest {
        /// Borrow the reactor selected at the application's outer boundary.
        fn tokio(&self) -> &tokio::runtime::Handle {
            self.runtime.handle()
        }
    }
    impl HasCancellation for CafeRequest {
        /// Select this request's cancellation authority without cancelling siblings.
        fn cancellation(&self) -> &Cancellation {
            self.request.cancellation()
        }
    }
    impl HasTaskSpawner for CafeRequest {
        /// Reuse the existing bounded native scheduling adapter.
        type Tasks = TokioTasks;
        /// Borrow the same admitted provider retained by the application.
        fn task_spawner(&self) -> &TokioTasks {
            &self.tasks
        }
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
    pub struct CafeAdmission;
    impl SignInPolicy for CafeAdmission {
        /// The café accepts currently named accounts; this cannot waive native verification.
        fn allows(&self, user: &IdentityUser) -> Result<bool, IdentityResult> {
            Ok(user.user_name().is_some())
        }
    }
    /// Retained native component selections in constructor order; no request payloads.
    type NativeConfig = (
        Retained<DefaultUserValidator>,
        Retained<DefaultPasswordHasher>,
        Retained<DefaultPasswordValidator>,
        Retained<UpperInvariantLookupNormalizer>,
        Retained<AuthenticatorTokenProvider>,
        Retained<CafeAdmission>,
        NonZeroUsize,
    );
    /// Actual native user manager parameterized by the local replacement store.
    type NativeUsers = UserManager<
        SqlxIdentityStore,
        Retained<DefaultUserValidator>,
        Retained<DefaultPasswordHasher>,
        Retained<DefaultPasswordValidator>,
        Retained<UpperInvariantLookupNormalizer>,
        DefaultIdentityErrorDescriber,
    >;
    /// Actual native sign-in manager; it owns credential/factor/admission verification.
    type NativeSignIn = SignInManager<NativeUsers, Retained<AuthenticatorTokenProvider>>;
    /// Pure extracted factory: retain selected components and bind the invocation store.
    /// The native constructor retains its existing OS-backed enrollment generator.
    fn select_native<P>(
        config: Arc<NativeConfig>,
        backend: SqlxIdentityStore,
        clock: IdentityClock,
        _policies: Arc<P>,
    ) -> Result<NativeSignIn, IdentityError> {
        let (
            validator,
            hasher,
            password_validator,
            normalizer,
            authenticator,
            admission,
            password_work,
        ) = config.as_ref();
        let users = UserManager::with_components_and_error_describer(
            backend,
            validator.clone(),
            hasher.clone(),
            password_validator.clone(),
            normalizer.clone(),
            DefaultIdentityErrorDescriber,
        )
        .with_utc_now(clock.callable())
        .with_password_work_limit(*password_work)
        .map_err(IdentityError::Native)?
        .with_options(IdentityOptions::default())
        .map_err(IdentityError::Native)?;
        Ok(SignInManager::new(users)
            .with_authenticator_token_provider(authenticator.clone())
            .with_sign_in_policy(admission.clone())
            .with_authentication_scheme("Cafe.Customer"))
    }
    /// Read one explicit application policy; missing/invalid configuration rejects startup.
    fn configured<T: std::str::FromStr>(name: &str) -> Result<T, Box<dyn std::error::Error>>
    where
        T::Err: std::error::Error + 'static,
    {
        Ok(std::env::var(name)?.parse()?)
    }
    /// Public request shape accepts no native security/confirmation/concurrency fields.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RegistrationForm {
        /// Customer-selected bounded native alias.
        name: String,
        /// Secret moved once into native hashing.
        password: String,
    }
    /// Decode one strict public DTO and retain native headers under the caller's byte bound.
    fn input<A: serde::de::DeserializeOwned + Send + 'static>(
        request: Request,
        limit: effect_server::BodyLimit,
    ) -> Effect<(A, http::HeaderMap), CafeError, CafeRequest> {
        Effect::try_future_once(
            move |_| async move {
                let bytes = request.read_bytes(limit).await.map_err(CafeError::Input)?;
                Ok((
                    serde_json::from_slice(&bytes).map_err(|_| CafeError::Form)?,
                    request.headers().clone(),
                ))
            },
            |error| error,
        )
    }
    /// Prepare only public registration fields, never trusted confirmation/security state.
    fn registration_input(
        request: Request,
        limit: effect_server::BodyLimit,
    ) -> Effect<RegistrationInput, CafeError, CafeRequest> {
        input::<RegistrationForm>(request, limit).flat_map(|(form, _)| {
            Effect::from_result_once(
                UserName::parse(form.name)
                    .map(|name| RegistrationInput {
                        user: IdentityUser::with_user_name(name),
                        password: form.password,
                    })
                    .map_err(|_| CafeError::Form),
            )
        })
    }
    /// Original password request; every policy preference is explicit in the submitted DTO.
    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct LoginForm {
        /// Lookup alias; the native manager applies its selected normalization.
        name: String,
        /// Owned secret consumed by native verification.
        password: String,
        /// Explicit native session persistence preference.
        persistent: bool,
        /// Explicit native lockout participation for this attempt.
        lockout_on_failure: bool,
    }
    impl LoginForm {
        /// Move public input into the real native password workflow.
        fn native(self) -> LoginInput {
            LoginInput {
                user_name: self.name,
                password: self.password,
                persistent: self.persistent,
                lockout_on_failure: self.lockout_on_failure,
            }
        }
    }
    /// Bounded example assertion wire input uses JSON byte arrays, not native credential internals.
    /// A browser client supplies its ArrayBuffer bytes; native code verifies the ceremony.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PasskeyForm {
        /// Untrusted credential selector, validated by the native constructor.
        credential_id: Vec<u8>,
        /// Original client JSON bytes awaiting native challenge/origin/type verification.
        client_data_json: Vec<u8>,
        /// Original authenticator bytes awaiting native flags/RP/counter verification.
        authenticator_data: Vec<u8>,
        /// Original signature bytes awaiting native cryptographic verification.
        signature: Vec<u8>,
        /// Optional observed account handle; never an authenticated owner claim.
        user_handle: Option<Vec<u8>>,
    }
    impl PasskeyForm {
        /// Move body-bounded buffers into native input after validating the credential selector.
        fn native(self) -> Result<PasskeyAssertionResponse, CafeError> {
            let id = PasskeyCredentialId::parse(self.credential_id).map_err(|_| CafeError::Form)?;
            Ok(PasskeyAssertionResponse::new(
                id,
                self.client_data_json,
                self.authenticator_data,
                self.signature,
                self.user_handle,
            ))
        }
    }
    /// Recovery lookup is not proof; the native reset-purpose token is verified before mutation.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RecoveryForm {
        /// Current account lookup identity.
        user_id: UserId,
        /// Native reset-purpose evidence.
        token: String,
        /// Secret validated/hashed under the selected native policy.
        new_password: String,
    }
    /// Factor answer consumes previously retained, genuine native primary evidence.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct FactorForm {
        /// Selected authenticator answer, not an assurance flag.
        code: String,
        /// Explicit session persistence preference.
        persistent: bool,
    }
    /// Native refresh evidence; exact stored digest replacement establishes one-use.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RefreshForm {
        /// Original protected refresh credential.
        token: String,
    }
    /// Only completed native verification yields credentials; pending is not authentication.
    #[derive(serde::Serialize)]
    enum LoginReply {
        /// Native provider credentials returned only after refresh-state acknowledgment.
        Bearer(IssuedBearer),
        /// Server-backed native primary evidence still requires a factor.
        FactorRequired,
    }
    /// Require private authority installed by the native verification pipeline.
    fn actor_input(request: Request) -> Effect<SelfGrant, CafeError, CafeRequest> {
        Effect::from_result_once(
            request
                .with_extensions(|extensions| extensions.get::<SelfGrant>().cloned())
                .ok_or(CafeError::Identity(IdentityError::Unauthorized)),
        )
    }
    /// Preserve native JSON serialization for explicitly selected public DTOs/credentials.
    fn json_reply<A: serde::Serialize>(
        value: A,
    ) -> Effect<Reply<CafeError, CafeRequest>, CafeError, CafeRequest> {
        Effect::from_result_once(Ok(reply! {native(axum::Json(value).into_response());}))
    }
    /// Retain a native response and its body without another encoder.
    fn native_reply(
        response: http::Response<axum::body::Body>,
    ) -> Effect<Reply<CafeError, CafeRequest>, CafeError, CafeRequest> {
        Effect::from_result_once(Ok(reply! {native(response);}))
    }
    /// Emit a cookie only after the selected native workflow returns its acknowledged header.
    fn cookie_reply(
        header: http::HeaderValue,
    ) -> Effect<Reply<CafeError, CafeRequest>, CafeError, CafeRequest> {
        Effect::from_result_once(Ok(
            reply! {append_header(http::header::SET_COOKIE,header);no_content();},
        ))
    }
    /// Public error status/category; original typed causes remain on CafeError.
    fn error_reply(
        error: CafeError,
    ) -> Effect<Reply<CafeError, CafeRequest>, CafeError, CafeRequest> {
        let status = match &error {
            CafeError::Identity(IdentityError::Unauthorized | IdentityError::StaleGrant) => {
                http::StatusCode::UNAUTHORIZED
            }
            CafeError::Identity(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
            CafeError::Input(_) | CafeError::Form => http::StatusCode::BAD_REQUEST,
        };
        native_reply((status, error.to_string()).into_response())
    }
    /// Render only the explicitly safe account view; no native aggregate is serialized.
    fn welcome(view: AccountView) -> Effect<Reply<CafeError, CafeRequest>, CafeError, CafeRequest> {
        Effect::from_result_once(Ok(
            reply! {text(format!("Welcome to the café, {}",view.user_name.as_deref().unwrap_or("customer")));},
        ))
    }
    /// Exchange one finite native HTTP/1 request, consuming the full response before return.
    /// Credentials remain local and are never printed; this is client transport, not verification.
    async fn exchange(address: std::net::SocketAddr, request: String) -> Result<String, CafeError> {
        let mut socket = tokio::net::TcpStream::connect(address)
            .await
            .map_err(|error| CafeError::Input(effect_server::ServerError::Io(error)))?;
        socket
            .write_all(request.as_bytes())
            .await
            .map_err(|error| CafeError::Input(effect_server::ServerError::Io(error)))?;
        let mut response = Vec::new();
        socket
            .read_to_end(&mut response)
            .await
            .map_err(|error| CafeError::Input(effect_server::ServerError::Io(error)))?;
        String::from_utf8(response).map_err(|_| CafeError::Form)
    }
    /// Construct D17–D19, save Maya's preference, then serve her native cookie/account exchange.
    async fn story(root: Arc<CafeRequest>) -> Result<(), Box<dyn std::error::Error>> {
        let options = sqlx::TursoConnectOptions::new()
            .filename(std::env::var("CAFE_DATABASE")?)
            .create_if_missing(true)
            .pragma("foreign_keys", Some("ON".into()));
        let pool = sqlx::TursoPool::connect_with(options).await?;
        let backend = SqlxIdentityStore::new(
            pool,
            IdentityStoragePolicy {
                challenge_capacity: configured::<NonZeroUsize>("CAFE_CHALLENGE_CAPACITY")?,
                utc_now: OffsetDateTime::now_utc,
            },
        )?;
        backend.initialize().await?;
        let policy = RecordPolicy {
            max_bytes: configured::<NonZeroUsize>("CAFE_DOCUMENT_BYTES")?,
            page_limit: configured::<NonZeroU16>("CAFE_PAGE_SIZE")?,
        };
        let key_id = configured::<uuid::Uuid>("CAFE_KEY_ID")?;
        let keys = KeyRepository::new(backend.clone(), "Cafe.Customer".into(), key_id, policy);
        let snapshot = if keys
            .records()
            .find("Cafe.Customer", key_id, OffsetDateTime::now_utc())
            .await?
            .is_some()
        {
            keys.reload(OffsetDateTime::now_utc()).await?
        } else {
            keys.persist(
                None,
                KeyDocument::parse(std::fs::read_to_string(std::env::var("CAFE_KEY_XML")?)?)?,
            )
            .await?
        };
        let format = DataProtectionTicketFormat::with_options(
            snapshot.clone(),
            TicketDataFormatOptions::new().with_purposes(default_ticket_purposes("Cafe.Customer")),
        );
        let cookie_name = std::env::var("CAFE_COOKIE_NAME")?;
        let origin = std::env::var("CAFE_ORIGIN")?;
        let browser =
            NativeBrowserPolicy::new(&backend, vec![origin.clone()], cookie_name.clone())?;
        let verification = match std::env::var("CAFE_USER_VERIFICATION")?.as_str() {
            "required" => PasskeyUserVerificationRequirement::Required,
            "preferred" => PasskeyUserVerificationRequirement::Preferred,
            "discouraged" => PasskeyUserVerificationRequirement::Discouraged,
            _ => return Err("invalid CAFE_USER_VERIFICATION".into()),
        };
        let passkeys = StaticPasskeyPolicy::new(
            std::env::var("CAFE_RP_ID")?,
            "Neighbourhood Cafe",
            vec![origin],
            verification,
        )?;
        // The native provider documents its dedicated bearer access/refresh lifetimes;
        // its default account-token lifetime is one day, independent of those purposes.
        let tokens = NativeBearerPolicy::new(
            DataProtectorTokenProvider::new(snapshot.clone()),
            NonZeroU64::new(DEFAULT_BEARER_ACCESS_TOKEN_EXPIRES_IN_SECONDS)
                .ok_or("native bearer lifetime")?,
        );
        let cookies = NativeCookiePolicy::new(
            CookieSessionManager::with_options(
                format.clone(),
                CookieSessionOptions::new()
                    .with_name(cookie_name)
                    .with_path("/account"),
            ),
            Duration::seconds(configured::<i64>("CAFE_SESSION_SECONDS")?),
            configured::<bool>("CAFE_ALLOW_REFRESH")?,
        )?;
        let token_capacity = configured::<NonZeroUsize>("CAFE_TOKEN_CAPACITY")?;
        let storage_backend = backend.clone();
        let stores = identity_store! {backend:storage_backend.clone(),accounts:AtomicAccounts::new(storage_backend.clone(),token_capacity),sessions:SessionRepository::new(storage_backend.clone(),policy,format),keys:keys,consent:SqlRecordAdapter::new(storage_backend.clone(),RecordCategory::Consent,policy,PreferenceCodec)}?;
        let config = (
            Retained::new(DefaultUserValidator::new()),
            Retained::new(DefaultPasswordHasher::with_options_and_random(
                PasswordHasherOptions::new(configured::<u32>("CAFE_HASH_ITERATIONS")?)?,
                OsRandomBytes,
            )),
            Retained::new(DefaultPasswordValidator::new()),
            Retained::new(UpperInvariantLookupNormalizer::new()),
            Retained::new(AuthenticatorTokenProvider::new()),
            Retained::new(CafeAdmission),
            configured::<NonZeroUsize>("CAFE_PASSWORD_WORK")?,
        );
        let authenticator = config.4.clone();
        let grant_policy = GrantPolicy {
            self_lifetime: Duration::seconds(configured::<i64>("CAFE_SELF_SECONDS")?),
            recent_lifetime: Duration::seconds(configured::<i64>("CAFE_RECENT_SECONDS")?),
        };
        let admission = configured::<NonZeroUsize>("CAFE_ADMISSION")?;
        let accounts = identity! {storage:stores,configure_native:ConfiguredComponents::new(config,select_native::<_>),errors:CafeError::Identity,clock:IdentityClock::new(OffsetDateTime::now_utc),grant_policy:grant_policy,admission:admission,tokens:tokens,tickets:cookies,browser:browser,passkeys:passkeys,authenticator:authenticator,principal_factory:NativePrincipalPolicy::new(ClaimsIdentityOptions::default()),key_ring:snapshot}?;
        let body_limit =
            effect_server::BodyLimit::new(configured::<NonZeroUsize>("CAFE_BODY_BYTES")?);
        // Definition and supplied Effect clones retain configuration/one-shot ownership;
        // credential inputs themselves move once and are never cloned.
        let definition = accounts.clone();
        let registration = WorkflowGroup::new(
            http::Method::POST,
            "/register".into(),
            move |request| registration_input(request, body_limit),
            move |_, input| definition.register(input),
            welcome,
        );
        let definition = accounts.clone();
        let login = WorkflowGroup::new(
            http::Method::POST,
            "/login".into(),
            move |request| input::<LoginForm>(request, body_limit),
            move |_, (form, headers)| {
                let definition = definition.clone();
                let supplied = Effect::from_result_once(Ok((form, headers)));
                effect! {
                    let definition=definition.clone();let supplied=supplied.clone();
                    let (form,headers)=supplied?;
                    let outcome=definition.login(form.native())?;
                    match outcome {
                        LoginOutcome::Pending(pending)=>{definition.retain_pending_browser(pending,headers)?;LoginReply::FactorRequired},
                        outcome=>{let issued=definition.sign_in_bearer(outcome)?;LoginReply::Bearer(issued)},
                    }
                }
            },
            json_reply,
        );
        let definition = accounts.clone();
        let recovery = WorkflowGroup::new(
            http::Method::POST,
            "/recover".into(),
            move |request| input::<RecoveryForm>(request, body_limit),
            move |_, (form, _)| {
                definition.recover_password(PasswordRecovery {
                    user_id: form.user_id,
                    token: form.token,
                    new_password: form.new_password,
                })
            },
            welcome,
        );
        let definition = accounts.clone();
        let authenticator = WorkflowGroup::new(
            http::Method::POST,
            "/factor".into(),
            move |request| input::<FactorForm>(request, body_limit),
            move |_, (form, headers)| {
                let definition = definition.clone();
                let supplied = Effect::from_result_once(Ok((form, headers)));
                effect! {
                    let definition=definition.clone();let supplied=supplied.clone();
                    let (form,headers)=supplied?;
                    let pending=definition.resume_pending_browser(headers)?;
                    let pending=Effect::from_result_once(pending.ok_or(CafeError::Identity(IdentityError::Unauthorized)))?;
                    let outcome=definition.complete_authenticator(AuthenticatorCompletion{pending,code:form.code,persistent:form.persistent})?;
                    definition.sign_in_bearer(outcome)?
                }
            },
            json_reply,
        );
        let definition = accounts.clone();
        let options = WorkflowGroup::new(
            http::Method::GET,
            "/passkeys/options".into(),
            |_| Effect::from_result_once(Ok(())),
            move |_, ()| definition.passkey_login_options(None),
            |options| {
                json_reply(PublicKeyCredentialRequestOptionsResponse::from_options(
                    &options,
                ))
            },
        );
        let definition = accounts.clone();
        let assertion = WorkflowGroup::new(
            http::Method::POST,
            "/passkeys/login".into(),
            move |request| input::<PasskeyForm>(request, body_limit),
            move |_, (form, _)| {
                let definition = definition.clone();
                let supplied = Effect::from_result_once(Ok(form));
                effect! {let definition=definition.clone();let supplied=supplied.clone();let form=supplied?;let response=Effect::from_result_once(form.native())?;let outcome=definition.passkey_login(response)?;definition.sign_in_bearer(outcome)?}
            },
            json_reply,
        );
        let definition = accounts.clone();
        let external_scheme = LoginProvider::parse(std::env::var("CAFE_EXTERNAL_SCHEME")?)?;
        let external = WorkflowGroup::new(
            http::Method::GET,
            "/external/start".into(),
            |_| Effect::from_result_once(Ok(())),
            move |_, ()| {
                let definition = definition.clone();
                let provider = external_scheme.clone();
                effect! {
                    let definition=definition.clone();let provider=provider.clone();
                    definition.invoke_once(provider,|native,provider,_,_|async move{Ok(native.configure_external_authentication_properties(provider,None,None).provider().as_str().to_owned())})?
                }
            },
            json_reply,
        );
        let definition = accounts.clone();
        let management = WorkflowGroup::new(
            http::Method::GET,
            "/me".into(),
            actor_input,
            move |_, actor| definition.account(actor),
            json_reply,
        );
        let definition = accounts.clone();
        let logout = WorkflowGroup::new(
            http::Method::POST,
            "/logout".into(),
            actor_input,
            move |_, actor| definition.logout_bearer(actor),
            |()| Effect::from_result_once(Ok(reply! {no_content();})),
        );
        let definition = accounts.clone();
        let refresh = WorkflowGroup::new(
            http::Method::POST,
            "/refresh".into(),
            move |request| input::<RefreshForm>(request, body_limit),
            move |_, (form, _)| {
                definition
                    .refresh_bearer(form.token)
                    .map(|(issued, _)| issued)
            },
            json_reply,
        );
        let definition = accounts.clone();
        let antiforgery = WorkflowGroup::new(
            http::Method::GET,
            "/antiforgery".into(),
            |request: Request| Effect::from_result_once(Ok(request.headers().clone())),
            move |_, headers| definition.issue_antiforgery(headers),
            native_reply,
        );
        let definition = accounts.clone();
        let cookie_login = endpoint! {body {
            let definition=definition.clone();
            let request=Effect::service(|env:&CafeRequest|env.request().clone())?;
            let (form,_)=input::<LoginForm>(request,body_limit)?;
            let outcome=definition.login(form.native())?;
            let header=definition.sign_in_cookie(outcome)?;
            cookie_reply(header)?
        }};
        let definition = accounts.clone();
        let stored_login = endpoint! {body {
            let definition=definition.clone();
            let request=Effect::service(|env:&CafeRequest|env.request().clone())?;
            let (form,_)=input::<LoginForm>(request,body_limit)?;
            let outcome=definition.login(form.native())?;
            let session_id=uuid::Uuid::new_v4();
            let owner=match &outcome{LoginOutcome::Completed{actor,..}=>Some(actor.user_id()),LoginOutcome::Rejected(_)|LoginOutcome::Pending(_)=>None};
            let revision=definition.store_session(outcome,session_id)?;
            json_reply((owner,session_id,revision))?
        }};
        let bearer = accounts.selected_bearer_stage();
        let cookie = accounts.cookie_stage();
        let antiforgery_stage = Arc::new(accounts.selected_antiforgery_stage());
        // A supplied Authorization header selects bearer validation exclusively;
        // otherwise a verified cookie selects native browser proof before handlers.
        // Missing/rejected cookies grant nothing: protected inputs still require SelfGrant.
        let pipeline = pipeline! {before(move|request:Request|if request.headers().contains_key(http::header::AUTHORIZATION){bearer(request).map(|_|())}else{
            let authenticated=cookie(request.clone());
            let antiforgery_stage=antiforgery_stage.clone();
            effect! {
                let request=request.clone();let authenticated=authenticated.clone();let antiforgery_stage=antiforgery_stage.clone();
                let authenticated=authenticated?;
                if authenticated {
                    let allowed=antiforgery_stage(request)?;
                    Effect::from_result_once(if allowed{Ok(())}else{Err(CafeError::Identity(IdentityError::Unauthorized))})?
                }else{()}
            }
        });};
        let routes_accounts = accounts.clone();
        let routes:effect_server::Router<CafeError,CafeRequest>=identity_endpoints!{
            identity:routes_accounts,prefix:"/account",
            registration(registration);login(login);recovery(recovery);authenticator(authenticator);
            passkeys(WorkflowRoutes::new(options).route(assertion));external(external);management(management);
            logout(logout);refresh(refresh);antiforgery(antiforgery);
            get("/status",endpoint! {body {reply! {text(String::from("Cafe account service"));}}});
            post("/cookie/login",cookie_login);post("/stored/login",stored_login);
            pipe_through(pipeline);on_error:error_reply,
        }.map_err(|error:EndpointError|Box::<dyn std::error::Error>::from(error))?;
        let program: Effect<AccountView, CafeError, ()> = effect! {
            let definition=accounts.clone();
            let name=Effect::from_result_once(UserName::parse(format!("maya-{}",uuid::Uuid::new_v4())).map_err(|_|CafeError::Form))?;
            let view=definition.register(RegistrationInput{user:IdentityUser::with_user_name(name),password:"Cafe-Demo-Receipt-42!".into()})?;
            let name=Effect::from_result_once(view.user_name.as_ref().ok_or(CafeError::Form).map(|name|name.to_owned()))?;
            let outcome=definition.login(LoginInput{user_name:name,password:"Cafe-Demo-Receipt-42!".into(),persistent:false,lockout_on_failure:true})?;
            let actor=Effect::from_result_once(match outcome{LoginOutcome::Completed{actor,..}=>Ok(actor),LoginOutcome::Rejected(_)|LoginOutcome::Pending(_)=>Err(CafeError::Identity(IdentityError::Unauthorized))})?;
            let key=uuid::Uuid::new_v4();
            definition.update_preferences(actor.clone(),key,None,None,ReceiptPreference{email_receipts:true})?;
            let preference=definition.preferences(actor,key)?;
            Effect::from_result_once(preference.filter(|document|document.value.email_receipts).ok_or(CafeError::Form))?;
            view
        };
        let view = match program.run(Arc::new(())).await {
            Exit::Success(view) => {
                // Direct native interoperability reads the identical SQLx-backed account.
                let persisted = backend.find_user_by_id(&view.id).await?;
                if persisted.is_none() {
                    return Err("native manager registration was not persisted".into());
                }
                view
            }
            outcome => {
                return Err(format!("café registration did not succeed: {outcome:?}").into());
            }
        };
        let binding = RequestBinding::new(|request: Request| {
            Effect::service(move |root: &CafeRequest| {
                Arc::new(CafeRequest {
                    request: request.clone(),
                    runtime: root.runtime.clone(),
                    tasks: root.tasks.clone(),
                })
            })
        });
        let body_policy = BodyPolicy {
            frames: configured::<NonZeroUsize>("CAFE_FRAMES")?,
            chunk_bytes: configured::<NonZeroUsize>("CAFE_CHUNK_BYTES")?,
        };
        let host_policy = HostPolicy {
            drain: std::time::Duration::from_millis(configured::<u64>("CAFE_DRAIN_MS")?),
        };
        let listen = configured::<std::net::SocketAddr>("CAFE_LISTEN")?;
        let host = server! {
            policy:ServerPolicy::new(body_policy,host_policy,CafeError::Input,error_reply),
            environment:binding,listen:listen,protocols:HttpProtocols::Http1,routes:routes,
        }?;
        let name: Arc<str> = view
            .user_name
            .ok_or("registered account has no alias")?
            .into();
        let execution=host.with_host(move|host|{
            let name=name.clone();
            Effect::try_future_once(move |_|async move{
                let body=serde_json::to_string(&LoginForm{name:name.to_string(),password:"Cafe-Demo-Receipt-42!".into(),persistent:false,lockout_on_failure:true}).map_err(|_|CafeError::Form)?;
                let response=exchange(host.address,format!("POST /account/cookie/login HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len())).await?;
                if !response.starts_with("HTTP/1.1 204 "){return Err(CafeError::Form)}
                // Forward only the native issued cookie to this scoped local host.
                let cookie=response.lines().take_while(|line|!line.is_empty()).find_map(|line|{
                    let (name,value)=line.split_once(':')?;
                    if name.eq_ignore_ascii_case("set-cookie"){value.trim().split(';').next()}else{None}
                }).ok_or(CafeError::Form)?;
                let response=exchange(host.address,format!("GET /account/me HTTP/1.1\r\nHost: localhost\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n")).await?;
                if !response.starts_with("HTTP/1.1 200 "){return Err(CafeError::Form)}
                println!("Maya's saved café account was served under native cookie authority.");
                Ok(())
            },|error|error)
        });
        match execution.run(root).await {
            Exit::Success(()) => Ok(()),
            outcome => Err(format!("café account routes did not succeed: {outcome:?}").into()),
        }
    }
    /// Select one caller-owned runtime/provider at the outer boundary and drain it after the story.
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(configured::<NonZeroUsize>("CAFE_WORKERS")?.get())
                .enable_all()
                .build()?,
        );
        let tasks = TokioTasks::new(
            runtime.clone(),
            AdmissionCapacity::Bounded(configured::<NonZeroUsize>("CAFE_TASK_CAPACITY")?),
        )
        .map_err(|error| format!("café task provider: {error:?}"))?;
        let root = Arc::new(CafeRequest {
            request: Request::from_native(
                http::Request::new(axum::body::Body::empty()),
                Cancellation::new(),
            ),
            runtime: runtime.clone(),
            tasks,
        });
        let result = runtime.block_on(story(root.clone()));
        runtime.block_on(root.tasks.stop().wait());
        result
    }
}
/// Run the feature-selected local café demonstration on the application's Tokio runtime.
#[cfg(feature = "sqlx-turso")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    cafe::run()
}
/// Default-feature builds remain inert; the documented runnable example selects SQLx/Turso.
#[cfg(not(feature = "sqlx-turso"))]
fn main() {}
