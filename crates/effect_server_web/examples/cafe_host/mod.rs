//! Concrete host wiring for `cafe_application`, using existing native services.
//!
//! Required environment: CAFE_LISTEN, CAFE_DATABASE, CAFE_KEY_RING (native XML),
//! CAFE_ORIGIN, CAFE_COOKIE, CAFE_COOKIE_SECURE, CAFE_CULTURE, CAFE_CATALOG,
//! CAFE_WORKERS, CAFE_CAPACITY, CAFE_BYTES, CAFE_DEPTH, CAFE_HASH_ITERATIONS,
//! CAFE_WINDOW_SECONDS, CAFE_SESSION_SECONDS, CAFE_RECENT_SECONDS and CAFE_RETRY_SECONDS.
//! CAFE_MEMORY_BYTES optionally fixes the shared Effectful arena total.
//! CAFE_QUEUE_CAPACITY optionally enables a bounded queue for parsed request heads.
//! CAPACITY bounds this small example's counts, including active spans, metric
//! descriptors and series per descriptor in both telemetry setups. BYTES bounds each finite resource,
//! key and response; WINDOW_SECONDS supplies its explicit TTL/reload/drain budget.
//! `/menu` and availability are public; `/pickup` requires the selected native
//! customer cookie issued by native sign-in. See `../README.md` for browser login,
//! owner-bound collection and observable localized pickup text.

use effect::effect_scope;
/// Concrete account, session and request-policy composition.
mod customer_identity;
/// Native account entry points and local pickup fixtures.
mod customer_routes;
mod prepared;
use prepared::{HostStorage, prepare_cafe_host};

use super::cafe_telemetry;
use super::*;
use effect::memory::{Bytes, FramePool, MemoryError, PreparedMemory, RegionId};
use effect::{
    Cancellation, HasBlockingExecutor, HasTracer, PreparedCacheCapacity, Tracer, effect_try_future,
    memory,
};
use effect::{
    PreparedAdmissionError, PreparedCleanupConfig, PreparedCleanupPool, PreparedProviderAdmission,
};
use effect_otel::{
    BorrowedProviders, HasOtel, Otel, OtelBackend, OwnedProviders, Propagation, ProviderFactories,
    ShutdownPolicy,
};
use effect_server::{BodyOutcome, BodyPolicy, HostPolicy};
use effect_server_identity::native::identity::tokens::data_protection::DataProtectionKeyRing;
use effect_server_identity::store::invocation::PreparedIdentityInvocations;
use effect_sql::{
    PreparedSqlRowLanes, PreparedSqlSessionCapacity, PreparedSqlSessionPool, SqlClient, SqlError,
    sqlx,
};
use effect_tokio::{TokioBlocking, TokioClock, TokioTasks};
use std::{fs::File, io::Read, str::FromStr};
use time::OffsetDateTime;

fn cafe_cache_limits(
    retain: Retain,
    capacity: NonZeroUsize,
) -> Result<PreparedCacheCapacity, MemoryError> {
    let scale = |factor| {
        capacity
            .get()
            .checked_mul(factor)
            .and_then(NonZeroUsize::new)
            .ok_or(MemoryError::ArithmeticOverflow)
    };
    Ok(PreparedCacheCapacity {
        retention: retain.cache_capacity(),
        active: capacity,
        lookups: capacity,
        diagnostics: capacity,
        observers: capacity,
        keys: scale(4)?,
        entries: scale(3)?,
        values: scale(3)?,
        errors: scale(4)?,
        cleanup_outcomes: scale(3)?,
        maintenance_outcomes: capacity,
        cleanup_entries: 2,
    })
}

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
struct CafeStartup {
    memory: PreparedMemory<'static>,
    prepared: CafeMemory,
    sessions: Arc<PreparedSqlSessionPool>,
    row_lanes: Arc<PreparedSqlRowLanes>,
    identity: Arc<PreparedProviderAdmission>,
    identity_invocations: Arc<PreparedIdentityInvocations>,
    telemetry: Arc<PreparedProviderAdmission>,
}

fn prepared_work_error(error: PreparedAdmissionError) -> CafeError {
    match error {
        PreparedAdmissionError::Memory(reason) => {
            cafe_error(CafeProblem::Web(WebError::Memory(reason)))
        }
        PreparedAdmissionError::Admission(reason) => cafe_error(CafeProblem::Busy(reason)),
    }
}

fn prepare_cafe_startup(
    capacity: NonZeroUsize,
    bytes: NonZeroUsize,
    retain: Retain,
    dependencies: NonZeroUsize,
    catalog_limits: CatalogLimits,
    output: &FiniteOutputPolicy,
    host: &HostStorage,
) -> Result<CafeStartup, Box<dyn std::error::Error>> {
    let cache_limits = cafe_cache_limits(retain, capacity)?;
    let probe_count = NonZeroUsize::MIN.saturating_add(1);
    let response = Bytes::new(bytes.get());
    let metadata = Bytes::new(64 * 1024);
    // Generation/listener/frame preparation performs multiple real allocations
    // per configured slot. This fixture budgets a header/padding allowance
    // separately from the payload sizing methods.
    let overhead = Bytes::new(4 * 1024).checked_mul(
        capacity
            .get()
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?,
    )?;
    let menu_bytes =
        PreparedCacheStorage::<MenuKey, Arc<str>, DependencyCacheError<CafeProblem>>::storage_bytes(
            cache_limits,
        )?
        .checked_add(CafeError::prepared_storage_bytes()?.checked_mul(2)?)?
        .checked_add(overhead)?;
    let seasonal_bytes = PreparedDependentReadThroughStorage::<
        MenuKey,
        MenuDependency,
        bool,
        CafeProblem,
    >::storage_bytes(
        cache_limits,
        cache_limits.values,
        cache_limits.entries,
        dependencies,
    )?
    .checked_add(overhead)?;
    let availability_bytes =
        PreparedAvailability::<CafeError>::storage_bytes(capacity, probe_count, response)?
            .checked_add(overhead)?;
    let output_limits = PreparedCacheCapacity {
        retention: effect::CacheCapacity::Bounded(output.entries),
        active: output.maximum_active,
        ..cache_limits
    };
    // Three menu dimensions, fixed identity fields and an optional raw query.
    // The physical key also holds nine lengths/counts and the query presence byte.
    let output_key_bytes = NonZeroUsize::new(
        output
            .key_bytes
            .get()
            .checked_add(9 * std::mem::size_of::<u64>() + 1)
            .ok_or(MemoryError::ArithmeticOverflow)?,
    )
    .ok_or(MemoryError::ArithmeticOverflow)?;
    let output_cache_bytes =
        PreparedOutputCacheStorage::<DependencyCacheError<CafeProblem>>::storage_bytes(
            output_limits,
            output_key_bytes,
        )?
        .checked_add(overhead)?;
    let host_bytes = host.region_bytes::<Environment>()?;
    let queue_bytes = host.queue_region_bytes()?;
    let generations = capacity
        .checked_add(2)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    let catalogue_sources = NonZeroUsize::MIN;
    let catalogue_bytes = PreparedCatalogGenerations::<CafeMessages>::storage_bytes(
        generations,
        catalog_limits,
        catalogue_sources,
    )?
    .checked_add(overhead)?;
    let identity_bytes = PreparedProviderAdmission::storage_bytes(0)?
        .checked_add(PreparedIdentityInvocations::storage_bytes(capacity.get())?)?
        .checked_add(overhead)?;
    let telemetry_bytes = PreparedProviderAdmission::storage_bytes(0)?.checked_add(overhead)?;
    let session_capacity = PreparedSqlSessionCapacity {
        sessions: capacity.get(),
        frames_per_session: 8,
    };
    let row_capacity = sqlx::PreparedTursoRowsCapacity {
        rows: capacity.get(),
        max_columns: 32,
        bytes_per_row: bytes.get(),
    };
    let retained_errors = 2;
    let cleanup_slots = capacity
        .get()
        .checked_mul(3)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    let cleanup_layout = PreparedCleanupPool::<SqlError>::frame_layout();
    // The locked driver prepares value, descriptor and byte vectors for each retained row.
    let row_overhead = Bytes::new(3 * 64)
        .checked_mul(row_capacity.rows)?
        .checked_mul(capacity.get())?;
    let sql_bytes = session_capacity
        .demand()?
        .region_bytes
        .checked_add(PreparedSqlRowLanes::demand(capacity.get(), row_capacity)?.region_bytes)?
        .checked_add(PreparedCleanupPool::<SqlError>::storage_bytes(
            cleanup_slots,
            retained_errors,
        )?)?
        .checked_add(FramePool::storage_bytes(cleanup_slots, cleanup_layout)?)?
        .checked_add(row_overhead)?
        .checked_add(overhead)?;
    let required = menu_bytes
        .checked_add(seasonal_bytes)?
        .checked_add(availability_bytes)?
        .checked_add(output_cache_bytes)?
        .checked_add(host_bytes)?
        .checked_add(queue_bytes)?
        .checked_add(metadata)?
        .checked_add(catalogue_bytes)?
        .checked_add(identity_bytes)?
        .checked_add(telemetry_bytes)?
        .checked_add(sql_bytes)?;
    let total = match std::env::var("CAFE_MEMORY_BYTES") {
        Ok(value) => Bytes::new(value.parse()?),
        Err(std::env::VarError::NotPresent) => required,
        Err(error) => return Err(error.into()),
    };
    let memory = memory! {
        total: total, mode: prepared, metadata: metadata, alignment: 64,
        partition metadata: metadata,
        partition menu: menu_bytes,
        partition seasonal: seasonal_bytes,
        partition availability: availability_bytes,
        partition output_cache: output_cache_bytes,
        partition web_host: host_bytes,
        partition request_queue: queue_bytes,
        partition catalogue: catalogue_bytes,
        partition identity: identity_bytes,
        partition telemetry: telemetry_bytes,
        partition sql: sql_bytes,
    }?
    .prepare()?;
    let menu_region = RegionId::named("menu");
    let busy = SharedValue::prepare_region(
        &memory,
        menu_region,
        DependencyCacheError::Load(CafeProblem::Busy(AdmissionError::Full)),
    )?;
    let closed = SharedValue::prepare_region(
        &memory,
        menu_region,
        DependencyCacheError::Load(CafeProblem::Busy(AdmissionError::Closed)),
    )?;
    let active = busy.clone();
    let refusals = PreparedCacheRefusals::new(
        move |reason| {
            if reason == MemoryError::Closed {
                closed.clone()
            } else {
                busy.clone()
            }
        },
        move || active.clone(),
    );
    let prepared = CafeMemory {
        menu: PreparedCacheStorage::prepare(&memory, menu_region, cache_limits)?,
        seasonal: PreparedDependentReadThroughStorage::prepare(
            &memory,
            RegionId::named("seasonal"),
            cache_limits,
            cache_limits.values,
            cache_limits.entries,
            dependencies,
        )?,
        refusals,
        availability: Arc::new(PreparedAvailability::prepare(
            &memory,
            RegionId::named("availability"),
            capacity,
            probe_count,
            response,
        )?),
        generations: PreparedCatalogGenerations::prepare(
            &memory,
            RegionId::named("catalogue"),
            generations,
            catalog_limits,
            catalogue_sources,
        )?,
        output: PreparedOutputCacheStorage::prepare(
            &memory,
            RegionId::named("output_cache"),
            output_limits,
            output_key_bytes,
        )?,
    };
    let sql_region = RegionId::named("sql");
    let sql_cleanup = PreparedCleanupConfig::new(
        PreparedCleanupPool::prepare(&memory, sql_region, cleanup_slots, retained_errors)?,
        memory.frames(sql_region, cleanup_slots, cleanup_layout)?,
    );
    let sessions = Arc::new(
        effect_sql::sql! {
            prepared_session_pool;
            memory: memory, region: sql_region, capacity: session_capacity,
        }?
        .with_cleanup(Arc::new(sql_cleanup)),
    );
    let row_lanes = Arc::new(effect_sql::sql! {
        prepared_native_rows;
        memory: memory, region: sql_region, lanes: capacity.get(), capacity: row_capacity,
    }?);
    let identity = Arc::new(PreparedProviderAdmission::prepare(
        &memory,
        RegionId::named("identity"),
        capacity,
        0,
    )?);
    let identity_invocations = Arc::new(PreparedIdentityInvocations::prepare(
        &memory,
        RegionId::named("identity"),
        capacity.get(),
    )?);
    let telemetry = Arc::new(PreparedProviderAdmission::prepare(
        &memory,
        RegionId::named("telemetry"),
        capacity,
        0,
    )?);
    Ok(CafeStartup {
        memory,
        prepared,
        sessions,
        row_lanes,
        identity,
        identity_invocations,
        telemetry,
    })
}

/// Run one configured SQLx-backed native Identity host and join its scoped providers.
/// Ctrl-C requests selected cancellation; request cleanup precedes provider drain.
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
    let queued = match std::env::var("CAFE_QUEUE_CAPACITY") {
        Ok(value) => Some(value.parse::<NonZeroUsize>()?),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let session_seconds: NonZeroU64 = configured("CAFE_SESSION_SECONDS")?;
    let culture = CultureId::new(&std::env::var("CAFE_CULTURE")?)?;
    let cookie = std::env::var("CAFE_COOKIE")?;
    let origin = std::env::var("CAFE_ORIGIN")?;
    let secure: bool = configured("CAFE_COOKIE_SECURE")?;
    let retain = Retain::new(
        effect_server_web::cache::Retention::Bounded(capacity),
        Some(window),
    )?;
    let dependencies = NonZeroUsize::MIN;
    let catalog_limits = CatalogLimits {
        bytes,
        depth,
        keys: capacity,
        key_bytes: bytes,
        value_bytes: bytes,
        message_bytes: bytes,
    };
    let output = FiniteOutputPolicy {
        name: Arc::from("cafe-public-menu"),
        response_bytes: bytes,
        key_bytes: bytes,
        entries: capacity,
        maximum_active: capacity,
        retain: window,
        vary_headers: Arc::from([]),
        vary_query: false,
    };
    let host_storage = HostStorage::new(
        capacity, bytes, output.entries, output.maximum_active, queued, window,
    )?;
    let CafeStartup {
        memory,
        prepared,
        sessions,
        row_lanes,
        identity,
        identity_invocations,
        telemetry,
    } = prepare_cafe_startup(
        capacity,
        bytes,
        retain,
        dependencies,
        catalog_limits,
        &output,
        &host_storage,
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
    let sql = effect_sql::sql! {
        client; pool: pool.clone(), row_lanes: row_lanes,
    }?
    .with_session_pool(sessions);
    let customers = runtime.block_on(customer_identity::configure(
        sql.clone(),
        customer_identity::CustomerPolicy {
            capacity,
            ring: key_ring(bytes)?,
            cookie,
            origin,
            secure,
            session_seconds,
            recent_seconds: configured("CAFE_RECENT_SECONDS")?,
            hash_iterations: configured("CAFE_HASH_ITERATIONS")?,
            bytes,
        },
        (identity, identity_invocations),
    ))?;
    match runtime.block_on(customers.fixture.run(Arc::new(()))) {
        effect::Exit::Success(()) => (),
        other @ (effect::Exit::Fail(_)
        | effect::Exit::Die(_)
        | effect::Exit::Interrupted
        | effect::Exit::FinalizationFailed(_)) => {
            runtime.block_on(pool.close());
            return Err(format!("café fixture acquisition: {other:?}").into());
        }
    }
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
    let span_name: Arc<str> = Arc::from("cafe.customer-request");
    let completion_messages = [
        BodyOutcome::Open,
        BodyOutcome::Completed,
        BodyOutcome::Disconnected,
        BodyOutcome::Failed,
    ].map(|body| {
        [false, true].map(|cleanup_failed| {
            Arc::<str>::from(format!("café response {body:?}; cleanup_failed={cleanup_failed}"))
        })
    });
    let observation = Observability::new(
        Propagation::new(opentelemetry::propagation::TextMapCompositePropagator::new(
            vec![
                Box::new(opentelemetry_sdk::propagation::TraceContextPropagator::new()),
                Box::new(opentelemetry_sdk::propagation::BaggagePropagator::new()),
            ],
        )),
        move |_| effect_otel::server::RequestSpan {
            name: span_name.clone(),
            attributes: Vec::new(),
        },
        move |summary| {
            let [open, completed, disconnected, failed] = &completion_messages;
            let [settled, cleanup_failed] = match summary.body {
                BodyOutcome::Open => open,
                BodyOutcome::Completed => completed,
                BodyOutcome::Disconnected => disconnected,
                BodyOutcome::Failed => failed,
            };
            let message = if summary.cleanup_failed { cleanup_failed } else { settled };
            Otel::log(effect::LogLevel::Info, message.clone())
        },
        |error| cafe_error(CafeProblem::Otel(error)),
    )
    .with_prepared_admission(telemetry, prepared_work_error);
    let policy = CafePolicy {
        server: ServerPolicy::new(
            BodyPolicy {
                frames: capacity,
                chunk_bytes: bytes,
            },
            HostPolicy { drain: window },
            |error| cafe_error(CafeProblem::Http(error)),
            customer_routes::error_reply,
        ),
        listen: address,
        protocols: HttpProtocols::Http1,
        retain,
        loaders: capacity,
        dependencies,
        catalog: catalog_limits,
        reload: window,
        probes: ProbeDeadlines {
            per_probe: window,
            overall: window,
        },
        probe_parallelism: capacity,
        retry_seconds: configured("CAFE_RETRY_SECONDS")?,
        output,
    };
    let resource = prepared.generations.file(
        "cafe-messages",
        culture.clone(),
        std::env::var("CAFE_CATALOG")?.into(),
        bytes,
    );
    let cultures = CultureSet::new(vec![culture.clone()], Vec::new(), culture.clone())?;
    let selected = culture.clone();
    let selection = Arc::new(CatalogSelection {
        ui: selected.clone(),
        formatting: selected,
        vary: Arc::from([]),
    });
    let menu_key = MenuKey {
        cafe: Arc::from("neighborhood-cafe"),
        audience: Arc::from("customer-menu"),
        culture: culture.clone(),
        representation: Arc::from("text"),
    };
    let menu_partition = MenuPartition::new(menu_key.clone());
    let host = prepare_cafe_host(memory, host_storage)?;
    let app = cafe_application(
        (
            policy,
            Arc::new(MenuBoard {
                seasonal_available: AtomicBool::new(false),
            }),
            sql,
        ),
        (
            customers.security,
            observation,
            service! { env |env: &Environment| { !env.cancellation().is_requested() }, },
            customers.routes,
        ),
        (vec![resource], cultures),
        move |request| {
            let selection = selection.clone();
            request.insert_extension(selection.as_ref().clone());
            effect_sync! { once; body { selection }, }
        },
        |_, request| {
            service! {
                env |env: &Environment| {
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
            }, }
        },
        (
            move |env: &Environment| MenuKey {
                culture: env.culture.clone(),
                ..menu_key.clone()
            },
            |env: &Environment| env.request.with_extensions(|extensions| {
                extensions.get::<effect_server_identity::authority::SelfGrant>().is_some()
            }),
            menu_partition,
        ),
        (prepared, host),
    )?;
    let factories = ProviderFactories::new(
        |builder| Ok(builder.build()),
        |builder| Ok(builder.build()),
        |builder| Ok(builder.build()),
    );
    let shutdown = ShutdownPolicy::new(window, true)?;
    let provider_runtime = runtime.clone();
    let root_for_run = root.clone();
    let program = effect_scope! {
        body: move |scope| {
        let app = app.clone();
        let root = root_for_run.clone();
        let provider_runtime = provider_runtime.clone();
        let factories = factories.clone();
        effect! {
            let scope = scope.clone(); let app = app.clone(); let root = root.clone();
            let provider_runtime = provider_runtime.clone(); let factories = factories.clone();
            let providers = OwnedProviders::acquire_with_error(scope.clone(), factories, shutdown, |error| cafe_error(CafeProblem::Lifecycle(error)))?;
            let native = effect_try_sync! { body { providers.with(|owner| owner.borrowed()) }, on_error |error| { cafe_error(CafeProblem::Otel(effect_otel::NativeError::Native(error))) }, }?;
            let mut base = effect_try_sync! { once; body { cafe_telemetry::Environment::new(provider_runtime, native, telemetry_capacity) }, on_error |error| { cafe_error(CafeProblem::Otel(effect_otel::NativeError::Native(effect::Defect::new(error)))) }, }?;
            // The signal backend changes; the inert constructor's unused providers
            // are replaced before work starts, preserving the original service owners.
            base.tasks = root.base.tasks.clone(); base.blocking = root.base.blocking.clone();
            base.clock = root.base.clock.clone(); base.cancellation = root.base.cancellation.clone();
            let environment = Arc::new(Environment { base: Arc::new(base), request: root.request.clone(), culture: root.culture.clone() });
            let cancellation = environment.cancellation().clone();
            scope.fork(effect_try_future! {
                once;
                body |cancel: Cancellation| {
                    async move {
                    tokio::select! {
                        signal = tokio::signal::ctrl_c() => {
                            signal?;
                            let _ = cancellation.request();
                        }
                        () = cancel.cancelled() => {}
                    }
                    Ok::<(), std::io::Error>(())
                } },
                on_error |error| { cafe_error(CafeProblem::Http(ServerError::Io(error))) },
            })?;
            run_cafe_application(app, environment)?
        }
        },
    };
    use std::io::Write;
    writeln!(
        std::io::stdout().lock(),
        "Café pickup demo: open http://{address}/ to sign in and collect your order. Ctrl-C drains requests and native providers."
    )?;
    let outcome = runtime.block_on(program.run(root.clone()));
    runtime.block_on(root.base.tasks.stop().wait());
    runtime.block_on(root.base.blocking.stop().wait());
    logs.shutdown()?;
    traces.shutdown()?;
    metrics.shutdown()?;
    runtime.block_on(pool.close());
    match &outcome {
        effect::Exit::Success(host) => finish_cafe(&**host),
        failure @ (effect::Exit::Fail(_)
        | effect::Exit::Die(_)
        | effect::Exit::Interrupted
        | effect::Exit::FinalizationFailed(_)) => finish_cafe(failure),
    }
}

fn finish_cafe<A>(outcome: &effect::Exit<A, CafeError>) -> Result<(), Box<dyn std::error::Error>> {
    match outcome {
        effect::Exit::Success(_) => Ok(()),
        effect::Exit::Fail(_) => {
            Err("café operation failed; typed outcome retained through cleanup".into())
        }
        effect::Exit::Die(error) => Err(error.message().into()),
        effect::Exit::Interrupted => Err("café host interrupted".into()),
        effect::Exit::FinalizationFailed(failure) => {
            let first = match &failure.first {
                effect::CleanupFailure::ReleaseFailure(error) => error
                    .with(|error| match error {
                        effect::DependencyCacheError::Load(CafeProblem::Http(error)) => {
                            error.to_string()
                        }
                        effect::DependencyCacheError::Load(problem) => problem.to_string(),
                        effect::DependencyCacheError::DependencyCapacity { .. } => {
                            "café dependency capacity".into()
                        }
                        effect::DependencyCacheError::ActiveCapacity { .. } => {
                            "café active loader capacity".into()
                        }
                        effect::DependencyCacheError::Memory(error) => error.to_string(),
                    })
                    .unwrap_or_else(|defect| defect.message().to_owned()),
                effect::CleanupFailure::ReleaseDefect(defect) => defect.message().to_owned(),
                effect::CleanupFailure::ReleaseInterrupted => "release interrupted".into(),
            };
            Err(format!("café host cleanup failed. First release cause: {first}. {failure:?}").into())
        }
    }
}
