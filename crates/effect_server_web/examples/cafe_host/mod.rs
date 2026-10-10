//! Concrete host wiring for `cafe_application`, using existing native services.
//!
//! Required environment: CAFE_LISTEN, CAFE_DATABASE, CAFE_KEY_RING (native XML),
//! CAFE_ORIGIN, CAFE_COOKIE, CAFE_COOKIE_SECURE, CAFE_CULTURE, CAFE_CATALOG,
//! CAFE_WORKERS, CAFE_CAPACITY, CAFE_BYTES, CAFE_DEPTH, CAFE_HASH_ITERATIONS,
//! CAFE_WINDOW_SECONDS, CAFE_RECENT_SECONDS and CAFE_RETRY_SECONDS.
//! CAPACITY bounds this small example's counts, including active spans, metric
//! descriptors and series per descriptor in both telemetry setups. BYTES bounds each finite resource,
//! key and response; WINDOW_SECONDS supplies its explicit TTL/reload/drain budget.
//! `/menu` and availability are public; `/pickup` requires the selected native
//! customer cookie. Native account workflows use the same database/key selection.
//! Edit the supplied catalogue file to observe complete generation reloads.

use super::cafe_telemetry;
use super::*;
use effect::{Cancellation, HasBlockingExecutor, HasTracer, Scope, Tracer};
use effect_otel::{
    BorrowedProviders, HasOtel, Otel, OtelBackend, OwnedProviders, Propagation, ProviderFactories,
    ShutdownPolicy,
};
use effect_server::{BodyPolicy, HostPolicy};
use effect_server_identity::authority::{GrantPolicy, IdentityClock};
use effect_server_identity::components::{
    NativeBrowserPolicy, NativeCookiePolicy, NativePrincipalPolicy, Retained,
};
use effect_server_identity::native::identity::{
    IdentityOptions, IdentityResult,
    error::DefaultIdentityErrorDescriber,
    manager::{SignInManager, SignInPolicy, UserManager},
    models::IdentityUser,
    normalization::UpperInvariantLookupNormalizer,
    password::{
        DefaultPasswordHasher, DefaultPasswordValidator, OsRandomBytes, PasswordHasherOptions,
    },
    principal::ClaimsIdentityOptions,
    ticket::{
        CookieSameSite, CookieSessionManager, CookieSessionOptions, DataProtectionTicketFormat,
        TicketDataFormatOptions, default_ticket_purposes,
    },
    tokens::{AuthenticatorTokenProvider, data_protection::DataProtectionKeyRing},
    validation::DefaultUserValidator,
};
use effect_server_identity::{
    ConfiguredComponents, IdentityStoragePolicy, SqlxIdentityStore, identity, identity_store,
};
use effect_sql::{SqlClient, sqlx};
use effect_tokio::{TokioBlocking, TokioClock, TokioTasks};
use std::{fs::File, io::Read, str::FromStr};
use time::OffsetDateTime;

/// Request projection shares the original provider/clock/tracer owner with the host.
struct Environment {
    /// Existing telemetry consumer's explicit native runtime/provider configuration.
    base: Arc<cafe_telemetry::Environment>,
    /// Exactly one request metadata/body/cancellation owner.
    request: Request,
    /// Declared UI culture selected before request environment projection.
    culture: CultureId,
}
impl HasRequest for Environment {
    /// Borrow the same request whose body survives through cleanup.
    fn request(&self) -> &Request {
        &self.request
    }
}
impl HasCancellation for Environment {
    /// Root and projected requests use their own cooperative cancellation tokens.
    fn cancellation(&self) -> &Cancellation {
        self.request.cancellation()
    }
}
impl HasClock for Environment {
    /// Existing monotonic provider, shared across request projections.
    type Clock = TokioClock;
    /// Preserve one time origin for request duration observation.
    fn clock(&self) -> &Self::Clock {
        self.base.clock.as_ref()
    }
}
impl HasTaskSpawner for Environment {
    /// Existing caller-owned native task provider.
    type Tasks = TokioTasks;
    /// Select managed tasks through this same R.
    fn task_spawner(&self) -> &Self::Tasks {
        &self.base.tasks
    }
}
impl HasBlockingExecutor for Environment {
    /// Existing independent blocking provider for files and SDK lifecycle work.
    type Executor = TokioBlocking;
    /// Reuse the selected service rather than creating another pool.
    fn blocking_executor(&self) -> &Self::Executor {
        &self.base.blocking
    }
}
impl HasTracer for Environment {
    /// Ordinary spans and OTel use the identical native span map.
    fn tracer(&self) -> &Tracer {
        &self.base.tracer
    }
}
impl HasOtel for Environment {
    /// Retain the same native backend across root/request scopes.
    fn otel(&self) -> &OtelBackend {
        &self.base.otel
    }
}
impl effect_server::HasTokio for Environment {
    /// Axum/SQLx use the one caller-owned reactor.
    fn tokio(&self) -> &tokio::runtime::Handle {
        self.base.runtime.handle()
    }
}
/// Native sign-in admission cannot waive password, factor or current-user checks.
#[derive(Debug)]
struct CafeAdmission;
impl SignInPolicy for CafeAdmission {
    /// This café accepts named accounts after native credential verification.
    fn allows(&self, user: &IdentityUser) -> Result<bool, IdentityResult> {
        Ok(user.user_name().is_some())
    }
}
/// Read typed caller policy with no fabricated default or secret printing.
fn configured<T: FromStr>(name: &str) -> Result<T, Box<dyn std::error::Error>>
where
    T::Err: std::error::Error + 'static,
{
    Ok(std::env::var(name)?.parse()?)
}
/// Load only the caller's bounded native XML key source; parsing/crypto stay native.
fn key_ring(maximum: NonZeroUsize) -> Result<DataProtectionKeyRing, Box<dyn std::error::Error>> {
    let mut text = String::new();
    let limit = maximum
        .get()
        .checked_add(1)
        .ok_or("key file bound overflow")?;
    File::open(std::env::var("CAFE_KEY_RING")?)?
        .take(u64::try_from(limit)?)
        .read_to_string(&mut text)?;
    if text.len() > maximum.get() {
        return Err("configured key file byte bound".into());
    }
    Ok(DataProtectionKeyRing::parse_xml(&text)?)
}
/// Run one configured SQLx-backed native Identity host and join its scoped providers.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let address: SocketAddr = configured("CAFE_LISTEN")?;
    let capacity: NonZeroUsize = configured("CAFE_CAPACITY")?;
    let bytes: NonZeroUsize = configured("CAFE_BYTES")?;
    let telemetry_capacity = cafe_telemetry::TelemetryCapacity {
        active_spans: capacity,
        metric_descriptors: capacity,
        series_per_descriptor: capacity,
    };
    let depth: NonZeroUsize = configured("CAFE_DEPTH")?;
    let seconds: NonZeroU64 = configured("CAFE_WINDOW_SECONDS")?;
    let window = Duration::from_secs(seconds.get());
    let native_window = time::Duration::seconds(i64::try_from(seconds.get())?);
    let culture = CultureId::new(&std::env::var("CAFE_CULTURE")?)?;
    let cookie = std::env::var("CAFE_COOKIE")?;
    let origin = std::env::var("CAFE_ORIGIN")?;
    let secure: bool = configured("CAFE_COOKIE_SECURE")?;
    let ticket_format = DataProtectionTicketFormat::with_options(
        key_ring(bytes)?,
        TicketDataFormatOptions::new()
            .with_purposes(default_ticket_purposes("Cafe.Customer"))
            .with_ticket_lifespan(window)
            .with_utc_now(OffsetDateTime::now_utc),
    );
    let tickets = NativeCookiePolicy::new(
        CookieSessionManager::with_options(
            ticket_format,
            CookieSessionOptions::new()
                .with_name(cookie.clone())
                .with_secure(secure)
                .with_http_only(true)
                .with_same_site(CookieSameSite::Strict)
                .with_utc_now(OffsetDateTime::now_utc),
        ),
        native_window,
        true,
    )?;
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(configured("CAFE_WORKERS")?)
            .enable_all()
            .build()?,
    );
    let options = sqlx::TursoConnectOptions::new()
        .filename(std::env::var("CAFE_DATABASE")?)
        .create_if_missing(true)
        .pragma("foreign_keys", Some("ON".into()));
    let pool = runtime.block_on(sqlx::TursoPool::connect_with(options))?;
    let sql = SqlClient::new(pool.clone());
    let backend = SqlxIdentityStore::from_kernel(
        sql.kernel().clone(),
        IdentityStoragePolicy {
            challenge_capacity: capacity,
            utc_now: OffsetDateTime::now_utc,
        },
    )?;
    runtime.block_on(backend.initialize())?;
    let browser = NativeBrowserPolicy::new(&backend, vec![origin], cookie)?;
    let validator = Retained::new(DefaultUserValidator::new());
    let passwords = Retained::new(DefaultPasswordValidator::new());
    let hasher = Retained::new(DefaultPasswordHasher::with_options_and_random(
        PasswordHasherOptions::new(configured("CAFE_HASH_ITERATIONS")?)?,
        OsRandomBytes,
    ));
    let normalizer = Retained::new(UpperInvariantLookupNormalizer::new());
    let authenticator = Retained::new(AuthenticatorTokenProvider::new());
    let admission = Retained::new(CafeAdmission);
    let components = ConfiguredComponents::new(
        (),
        move |_: Arc<()>, backend: SqlxIdentityStore, clock: IdentityClock, _: Arc<_>| {
            let users = UserManager::with_components_and_error_describer(
                backend,
                validator.clone(),
                hasher.clone(),
                passwords.clone(),
                normalizer.clone(),
                DefaultIdentityErrorDescriber,
            )
            .with_utc_now(clock.callable())
            .with_password_work_limit(capacity)
            .map_err(IdentityError::Native)?
            .with_options(IdentityOptions::default())
            .map_err(IdentityError::Native)?;
            Ok(SignInManager::new(users)
                .with_authenticator_token_provider(authenticator.clone())
                .with_sign_in_policy(admission.clone())
                .with_authentication_scheme("Cafe.Customer"))
        },
    );
    let recent_window = time::Duration::seconds(configured("CAFE_RECENT_SECONDS")?);
    let definition = identity! {
        storage: identity_store! { backend: backend }?, configure_native: components,
        tickets: tickets, browser: browser, principal_factory: NativePrincipalPolicy::new(ClaimsIdentityOptions::default()),
        errors: |error| cafe_error(CafeProblem::Identity(error)), clock: IdentityClock::new(OffsetDateTime::now_utc),
        grant_policy: GrantPolicy { self_lifetime: native_window, recent_lifetime: recent_window }, admission: capacity,
    }?;
    let mut selected_security = customer_security(&definition, || {
        Effect::try_sync(
            || ResponseNonce::new(uuid::Uuid::new_v4().simple().to_string()),
            |error| cafe_error(CafeProblem::Web(error)),
        )
    })?;
    let verify = selected_security
        .authentication
        .take()
        .ok_or("selected native authentication stage")?;
    selected_security.authentication = Some(Arc::new(move |request| {
        let public = matches!(
            request.uri().path(),
            "/menu" | "/health" | "/healthz" | "/ready" | "/readyz" | "/live" | "/livez"
        );
        verify(request).map(move |authenticated| public || authenticated)
    }));
    let traces = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let metrics = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
    let logs = opentelemetry_sdk::logs::SdkLoggerProvider::builder().build();
    let base = Arc::new(cafe_telemetry::Environment::new(
        runtime.clone(),
        BorrowedProviders::new(&traces, &metrics, &logs),
        telemetry_capacity,
    )?);
    let root_request = Request::from_native(
        http::Request::new(axum::body::Body::empty()),
        base.cancellation.clone(),
    );
    let root = Arc::new(Environment {
        base,
        request: root_request,
        culture: culture.clone(),
    });
    let observation = Observability::new(
        Propagation::new(opentelemetry::propagation::TextMapCompositePropagator::new(
            vec![
                Box::new(opentelemetry_sdk::propagation::TraceContextPropagator::new()),
                Box::new(opentelemetry_sdk::propagation::BaggagePropagator::new()),
            ],
        )),
        |_| effect_otel::server::RequestSpan {
            name: Arc::from("cafe.customer-request"),
            attributes: Vec::new(),
        },
        |summary| {
            Otel::log(
                effect::LogLevel::Info,
                format!(
                    "café response {:?}; cleanup_failed={}",
                    summary.body, summary.cleanup_failed
                ),
            )
        },
        |error| cafe_error(CafeProblem::Otel(error)),
    );
    let policy = CafePolicy {
        server: ServerPolicy::new(
            BodyPolicy {
                frames: capacity,
                chunk_bytes: bytes,
            },
            HostPolicy { drain: window },
            |error| cafe_error(CafeProblem::Http(error)),
            |_| {
                Effect::sync(
                    || reply! { text(String::from("The café cannot complete this request.")); },
                )
            },
        ),
        listen: address,
        protocols: HttpProtocols::Http1,
        retain: Retain::new(
            effect_server_web::cache::Retention::Bounded(capacity),
            Some(window),
        )?,
        loaders: capacity,
        dependencies: NonZeroUsize::MIN,
        catalog: CatalogLimits {
            bytes,
            depth,
            keys: capacity,
            key_bytes: bytes,
            value_bytes: bytes,
            message_bytes: bytes,
        },
        reload: window,
        probes: ProbeDeadlines {
            per_probe: window,
            overall: window,
        },
        probe_parallelism: capacity,
        retry_seconds: configured("CAFE_RETRY_SECONDS")?,
        output: FiniteOutputPolicy {
            name: Arc::from("cafe-public-menu"),
            response_bytes: bytes,
            key_bytes: bytes,
            entries: capacity,
            maximum_active: capacity,
            retain: window,
            vary_headers: Arc::from([]),
            vary_query: false,
        },
    };
    let resource = CatalogResource::file(
        "cafe-messages",
        culture.clone(),
        std::env::var("CAFE_CATALOG")?.into(),
        bytes,
    );
    let cultures = CultureSet::new(vec![culture.clone()], Vec::new(), culture.clone())?;
    let selected = culture.clone();
    let app = cafe_application(
        policy,
        Arc::new(MenuBoard {
            seasonal_available: AtomicBool::new(false),
        }),
        sql,
        selected_security,
        observation,
        Effect::service(|env: &Environment| !env.cancellation().is_requested()),
        vec![resource],
        cultures,
        move |request| {
            let selection = CatalogSelection {
                ui: selected.clone(),
                formatting: selected.clone(),
                vary: Arc::from([]),
            };
            request.insert_extension(selection.clone());
            Effect::from_result_once(Ok(selection))
        },
        |_, request| {
            Effect::service(move |env: &Environment| {
                Arc::new(Environment {
                    base: env.base.clone(),
                    culture: request
                        .with_extensions(|extensions| {
                            extensions
                                .get::<CatalogSelection>()
                                .map(|selection| selection.ui.clone())
                        })
                        .unwrap_or_else(|| env.culture.clone()),
                    request: request.clone(),
                })
            })
        },
        |env| MenuKey {
            cafe: Arc::from("neighborhood-cafe"),
            audience: Arc::from("customer-menu"),
            culture: env.culture.clone(),
            representation: Arc::from("text"),
        },
    )?;
    let factories = ProviderFactories::new(
        |builder| Ok(builder.build()),
        |builder| Ok(builder.build()),
        |builder| Ok(builder.build()),
    );
    let shutdown = ShutdownPolicy::new(window, true)?;
    let provider_runtime = runtime.clone();
    let root_for_run = root.clone();
    let program = Scope::scoped(move |scope| {
        let app = app.clone();
        let root = root_for_run.clone();
        let provider_runtime = provider_runtime.clone();
        let factories = factories.clone();
        effect! {
            let scope = scope.clone(); let app = app.clone(); let root = root.clone();
            let provider_runtime = provider_runtime.clone(); let factories = factories.clone();
            let providers = OwnedProviders::acquire_with_error(scope.clone(), factories, shutdown, |error| cafe_error(CafeProblem::Lifecycle(error)))?;
            let native = Effect::try_sync(move || providers.with(|owner| owner.borrowed()), |error| cafe_error(CafeProblem::Otel(effect_otel::NativeError::Native(error))))?;
            let mut base = Effect::from_result_once(cafe_telemetry::Environment::new(provider_runtime, native, telemetry_capacity).map_err(|error| cafe_error(CafeProblem::Otel(effect_otel::NativeError::Native(effect::Defect::new(error))))))?;
            // The signal backend changes; the inert constructor's unused providers
            // are replaced before work starts, preserving the original service owners.
            base.tasks = root.base.tasks.clone(); base.blocking = root.base.blocking.clone();
            base.clock = root.base.clock.clone(); base.cancellation = root.base.cancellation.clone();
            let environment = Arc::new(Environment { base: Arc::new(base), request: root.request.clone(), culture: root.culture.clone() });
            let cancellation = environment.cancellation().clone();
            scope.fork(Effect::try_future_once(move |_| async move { tokio::signal::ctrl_c().await?; let _ = cancellation.request(); Ok::<(), std::io::Error>(()) }, |error| cafe_error(CafeProblem::Http(ServerError::Io(error)))))?;
            run_cafe_application(app, environment)?;
        }
    });
    println!("Starting café at {address}; Ctrl-C drains requests and native providers.");
    let outcome = runtime.block_on(program.run(root.clone()));
    runtime.block_on(root.base.tasks.stop().wait());
    runtime.block_on(root.base.blocking.stop().wait());
    logs.shutdown()?;
    traces.shutdown()?;
    metrics.shutdown()?;
    runtime.block_on(pool.close());
    match outcome {
        effect::Exit::Success(()) => Ok(()),
        effect::Exit::Fail(_) => {
            Err("café operation failed; typed outcome retained through cleanup".into())
        }
        effect::Exit::Die(error) => Err(error.message().into()),
        effect::Exit::Interrupted => Err("café host interrupted".into()),
        effect::Exit::FinalizationFailed(_) => Err("café host cleanup failed".into()),
    }
}
