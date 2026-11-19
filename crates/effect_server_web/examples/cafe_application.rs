//! Sign in with native Identity, inspect your ready order and record collection once.
//!
//! Compile/run with `--features identity,otel,sqlx-turso --example cafe_application`.
//! `main` binds the configured listener, SQLx Identity managers and scoped native
//! OTel providers, then serves `/menu`, `/pickup` and availability until Ctrl-C.
//! `cafe_host` supplies explicit environment inputs and reuses the existing native
//! telemetry setup; request projections share the same runtime, clock and span map.
//! Startup automatically enables seasonal membership after caching an empty menu,
//! and invalidates that dependency; `/menu` then loads the updated seasonal drinks.
//!
//! See `README.md` for the local browser sign-in and pickup walkthrough.

use effect::{
    AdmissionError, CacheKey, DependencyCacheError, Effect, PreparedCacheRefusals,
    PreparedCacheStorage, PreparedDependencyWriter,
    SharedValue, effect, effect_map_error, effect_provide_some, effect_sync,
    effect_try_sync, service,
};
use effect_server::{
    HasRequest, HttpProtocols, PreparedHostResult, Reply, Request, Router, ServerError, ServerPolicy,
    endpoint, reply, router,
};
use effect_server_identity::IdentityError;
use effect_server_web::admission::{AdmissionClass, Maintenance, MaintenanceMode, RetryAfter};
use effect_server_web::application::PreparedWeb;
use effect_server_web::availability::{AvailabilityError, PreparedAvailability, ProbeDeadlines};
use effect_server_web::cache::{
    DependentReadThrough, LoadAdmission, PreparedDependentReadThroughStorage, ReadThrough, Retain,
};
use effect_server_web::localization::{
    BoundMessage, CatalogDefinition, CatalogLimits, CatalogResource, CatalogRuntime,
    CatalogSelection, Contract, CultureId, CultureSet, LocalizationError, Message,
    MessageContract, MissingMessage, NamedArgument, PinnedCatalog, PreparedCatalogGenerations,
};
use effect_server_web::observability::Observability;
use effect_server_web::output_cache::{
    FiniteOutputPolicy, FiniteSource, OutputCache, OutputCacheDefinition, OutputPartition,
    PreparedOutputCacheStorage, PreparedOutputInput, PreparedOutputWriter,
};
use effect_server_web::security::Security;
use effect_server_web::{
    WebError, availability, dependent_read_through, localization, message_arguments, read_through, web,
};
use std::net::SocketAddr;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

/// Example-local native configuration and the executable host boundary.
mod cafe_host;
use cafe_host::{CafeHostStorage, Environment};
/// Reuse the caller-owned native provider wiring from the telemetry consumer.
#[path = "../../effect_otel/examples/cafe_telemetry/mod.rs"]
pub mod cafe_telemetry;
/// Customer-owned persisted pickup workflow and browser pages.
mod pickup;

/// Startup owners provisioned from the once-prepared café host budget.
pub struct CafeMemory {
    menu: PreparedCacheStorage<MenuKey, Arc<str>, DependencyCacheError<CafeProblem>>,
    seasonal:
        PreparedDependentReadThroughStorage<MenuKey, MenuDependency, bool, CafeProblem>,
    refusals: PreparedCacheRefusals<DependencyCacheError<CafeProblem>>,
    availability: Arc<PreparedAvailability<CafeError>>,
    output: PreparedOutputCacheStorage<DependencyCacheError<CafeProblem>>,
    pub generations: PreparedCatalogGenerations<CafeMessages>,
}

/// Serve the menu and localized pickup announcement with native account security. Request projection
/// receives verified Identity and culture metadata before producing the selected environment.
/// The key selector uses that environment's tenant, audience, culture, and representation.
#[expect(clippy::result_large_err, reason = "startup preserves the owned café and SQL error without indirection")]
fn cafe_application(
    (policy, board, sql): (CafePolicy, Arc<MenuBoard>, effect_sql::SqlClient),
    (selected_security, observation, request_loop, customer_routes): CafeRequestServices,
    (resources, cultures, selection): (
        Vec<CatalogResource<Environment>>,
        CultureSet,
        Arc<CatalogSelection>,
    ),
    (base_menu_key, prepared_partition): (MenuKey, MenuPartition),
    (prepared, host): (CafeMemory, CafeHostStorage),
) -> Result<PreparedWeb<CafeCapabilities, CafeError, Environment>, CafeProblem> {
    let catalogue = CatalogDefinition::<CafeMessages, Environment>::new(
        "cafe",
        policy.catalog,
        resources,
        vec![Contract::of::<OrderReady>()],
    )
    .map_err(CafeProblem::Language)?;
    let translations = localization! {
        catalogue: catalogue, cultures: cultures,
        select_by: move |request: Request| {
            let selection = selection.clone();
            request.insert_extension(selection.as_ref().clone());
            effect_sync! { once; body { selection }, }
        },
        missing: MissingMessage::Fail, reload: policy.reload, generations: prepared.generations,
    }
    .map_err(CafeProblem::Language)?;
    let maintenance = Maintenance::new(
        MaintenanceMode::Open,
        RetryAfter::seconds(policy.retry_seconds),
    );
    let prepared_availability = prepared.availability;
    let database = match policy.database_readiness {
        DatabaseReadiness::Ordinary => effect_server_web::database::readiness(&sql),
        DatabaseReadiness::Prepared => effect_server_web::database::readiness_prepared(&sql),
    };
    let availability = availability! {
        service: "neighborhood-cafe", errors: |error| cafe_error(CafeProblem::Availability(error)),
        readiness("orders-database", effect_map_error! { program: database, map |error| { cafe_error(CafeProblem::Sql(error)) }, });
        process_liveness("request-loop", request_loop);
        traffic_ready: web! { traffic_ready; source: maintenance, }, deadline: policy.probes, parallelism: policy.probe_parallelism,
        workspace: prepared_availability,
    };
    let retained = policy.retain;
    let loaders = policy.loaders;
    let dependencies = policy.dependencies;
    let prepared_menu = effect_sync! { once; body: move || prepared.menu, };
    let prepared_seasonal = effect_sync! { once; body: move || prepared.seasonal, };
    let prepared_output = effect_sync! { once; body: move || prepared.output, };
    let refusals = prepared.refusals;
    let output_policy = policy.output.clone();
    let startup_board = board.clone();
    let startup_maintenance = maintenance.clone();
    let CafeHostStorage {
        preparation,
        pools,
        overload,
    } = host;
    let app = web! {
        policy: policy.server, web_errors: |error| cafe_error(CafeProblem::Web(error)),
        startup |scope: effect::Scope<CafeError>| {
            let board = startup_board.clone(); let maintenance = startup_maintenance.clone(); let base_menu_key = base_menu_key.clone(); let customer_routes = customer_routes.clone();
            let prepared_menu = prepared_menu.clone(); let prepared_seasonal = prepared_seasonal.clone(); let prepared_output = prepared_output.clone();
            let refusals = refusals.clone(); let output_policy = output_policy.clone();
            let prepared_partition = prepared_partition.clone();
            let menu_memory = prepared_menu?;
            let seasonal_memory = prepared_seasonal?;
            let output_memory = prepared_output?;
            let menu = read_through! {
                scope: scope.clone(), retain: retained,
                memory: menu_memory, refusals: refusals.clone(),
                admission: LoadAdmission::new(loaders, |error| DependencyCacheError::Load(CafeProblem::Busy(error))),
                keys: PartialEq::eq,
                cache_failures: |_: &DependencyCacheError<CafeProblem>| false,
                loader |key: CacheKey<MenuKey>| { key.cafe.clone() }
            }?;
            let seasonal_board = board.clone();
            let seasonal = dependent_read_through! {
                scope: scope.clone(), retain: retained,
                memory: seasonal_memory, refusals: refusals.clone(),
                admission: LoadAdmission::new(loaders, CafeProblem::Busy),
                maximum_dependencies: dependencies,
                keys: PartialEq::eq,
                dependencies: PartialEq::eq,
                cache_failures: |_: &CafeProblem| false,
                loader: move |_, writer| seasonal_menu_prepared(seasonal_board.clone(), writer),
            }?;
            let menu_source = effect_try_sync! {
                once;
                body: {
                    let menu = menu.clone(); let seasonal = seasonal.clone();
                    let prepared_partition = prepared_partition.clone();
                    move || FiniteSource::prepared_encoded(
                        http::HeaderValue::from_static("text/plain; charset=utf-8"),
                        http::HeaderMap::new(),
                        move |input| menu_model(menu.clone(), seasonal.clone(), prepared_partition.clone(), input),
                        write_menu,
                    )
                }, on_error |error| { cafe_error(CafeProblem::Web(error)) },
            }?;
            let key = base_menu_key.clone();
            let output = web! {
                prepared_output_cache;
                scope: scope, policy: output_policy, source: menu_source,
                storage: output_memory, refusals: refusals,
                gate: |_| effect_sync! { body { () }, },
                authenticated: authenticated,
                partition: move |env, _| {
                    let key = menu_key(&key, env);
                    Ok(if key == prepared_partition.key { prepared_partition.output.clone() } else { menu_partition(&key) })
                },
                on_error: menu_output_error,
            }?;
            let menu_response = output.clone();
            let control = maintenance.clone();
            let menu_endpoint = endpoint! {
                effect: web! {
                    admit; source: control, class: AdmissionClass::Read,
                    body: move || web! { output_reply; source: menu_response, },
                    on_refusal: |_| effect! {
                        reply! {
                            bytes(bytes::Bytes::from_static("The café is closing; please return after reopening.".as_bytes()), http::HeaderValue::from_static("text/plain; charset=utf-8"));
                        }
                    },
                },
            };
            let routes = effect_try_sync! {
                once;
                body {
                    router! {
                get("/menu", menu_endpoint);
                include(customer_routes);
            } }, on_error |error| { cafe_error(CafeProblem::Http(error)) }, }?;
            let capabilities = CafeCapabilities { seasonal, output, board, routes };
            let seasonal = capabilities.seasonal.clone();
            let initial_key = service! { env |env: &Environment| { menu_key(&base_menu_key, env) }, }?;
            dependent_read_through! { get; cache: seasonal, key: initial_key, }?;
            update_seasonal_menu(&capabilities, true)?;
            capabilities
        }
        environment: |_, request: Request| {
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
                },
            }
        },
        security: selected_security, localization: translations, otel: observation, availability: availability,
        output_cache: OutputCacheDefinition::new(vec![policy.output]),
        memory: move |server| preparation.prepare(server), byte_pools: pools, overload: overload,
        listen: policy.listen, protocols: policy.protocols,
        routes: |capabilities: Arc<CafeCapabilities>| capabilities.routes.clone(),
    };
    Ok(app)
}
/// Shared error ownership lets both real scoped cache engines retain non-Clone errors.
pub type CafeError = SharedValue<DependencyCacheError<CafeProblem>>;
/// Native request security, observability and liveness selected by the café host.
type CafeRequestServices = (
    Security<CafeError, Environment>,
    Observability<CafeError, Environment>,
    Effect<bool, CafeError, Environment>,
    Router<CafeError, Environment>,
);
/// Café failures retain their original provider boundary rather than becoming denial.
#[derive(Debug, effect_server_web::EffectErrorLabel)]
pub enum CafeProblem {
    /// No order is assigned to this verified customer.
    #[effect_error_label("café pickup order absent")]
    MissingOrder,
    /// Checked Web policy or middleware failure.
    #[effect_error_label("café web policy")]
    Web(WebError),
    /// Safe catalogue acquisition/selection error.
    #[effect_error_label("café catalogue")]
    Language(LocalizationError),
    /// Original selected Identity/native SQL error.
    #[effect_error_label("café identity")]
    Identity(Box<IdentityError>),
    /// Native HTTP registration/input failure.
    #[effect_error_label("café HTTP")]
    Http(ServerError),
    /// Availability policy failure.
    #[effect_error_label("café readiness")]
    Availability(AvailabilityError),
    /// Immediate producer capacity denial.
    #[effect_error_label("café producer capacity")]
    Busy(AdmissionError),
    /// Original failure from a menu origin cache.
    #[effect_error_label("café menu origin")]
    MenuOrigin(CafeError),
    /// Original defect reading a shared menu value.
    #[effect_error_label("café menu value")]
    MenuValue(effect::Defect),
    /// Original SQL readiness failure.
    #[effect_error_label("café SQL")]
    Sql(effect_sql::SqlError),
    /// Native OTel context/signal failure.
    #[effect_error_label("café telemetry")]
    Otel(effect_otel::NativeError),
    /// Ordered native provider acquisition/shutdown failure.
    #[effect_error_label("café provider lifecycle")]
    Lifecycle(effect_otel::LifecycleError),
}
impl std::error::Error for CafeProblem {}
/// Inject one owned failure; no payload cloning or provider error stringification.
#[must_use]
pub fn cafe_error(problem: CafeProblem) -> CafeError {
    SharedValue::new(DependencyCacheError::Load(problem))
}
/// Catalogue family prevents mixing messages from another application's resources.
pub struct CafeMessages;
/// Pickup announcement has one explicitly typed customer argument.
pub struct OrderReady;
impl MessageContract for OrderReady {
    /// Only café catalogue generations may format this identity.
    type Catalogue = CafeMessages;
    /// Own one customer argument without an argument Clone requirement.
    type Arguments = NamedArgument<String, ()>;
    /// Stable application resource family.
    const CATALOGUE: &'static str = "cafe";
    /// Prepared flattened pickup-message key.
    const KEY: &'static str = "orders.ready";
    /// Exact typed argument names/order.
    const FIELDS: &'static [&'static str] = &["customer"];
}
/// Every value-affecting partition is an application-owned cache identity.
#[derive(Clone, PartialEq, Eq)]
pub struct MenuKey {
    /// Café/tenant identity, never an authentication credential.
    pub cafe: Arc<str>,
    /// Verified customer/staff visibility policy identity.
    pub audience: Arc<str>,
    /// Selected request UI culture, supplied after localization projection.
    pub culture: CultureId,
    /// Selected representation identity.
    pub representation: Arc<str>,
}

fn menu_key(base: &MenuKey, env: &Environment) -> MenuKey {
    MenuKey {
        cafe: base.cafe.clone(),
        audience: base.audience.clone(),
        culture: env.culture.clone(),
        representation: base.representation.clone(),
    }
}

fn authenticated(env: &Environment) -> bool {
    env.request.with_extensions(|extensions| {
        extensions
            .get::<effect_server_identity::authority::SelfGrant>()
            .is_some()
    })
}

/// An already validated origin key and output identity for a configured menu variant.
#[derive(Clone)]
pub struct MenuPartition {
    key: MenuKey,
    output: OutputPartition,
}

impl MenuPartition {
    /// Build the named menu variant before accepting requests.
    #[must_use]
    pub fn new(key: MenuKey) -> Self {
        let output = menu_partition(&key);
        Self { key, output }
    }
}

fn menu_partition(key: &MenuKey) -> OutputPartition {
    OutputPartition {
        namespace: key.cafe.clone(),
        dimensions: Arc::from([
            key.audience.clone(),
            Arc::from(key.culture.as_str()),
            key.representation.clone(),
        ]),
    }
}
/// Membership is invalidated even when the seasonal menu is empty.
#[derive(PartialEq, Eq)]
pub enum MenuDependency {
    /// Staff-controlled seasonal menu membership.
    SeasonalMembership,
}
/// Real café state; sharing this handle does not clone returned menu payloads.
pub struct MenuBoard {
    /// Staff controls whether the seasonal drink is available.
    pub seasonal_available: AtomicBool,
}
/// Host scope owns both cache engines until all native requests finish cleanup.
struct CafeCapabilities {
    /// Membership-aware seasonal menu over the existing dependency engine.
    seasonal: DependentReadThrough<MenuKey, MenuDependency, bool, CafeProblem, Environment>,
    output: OutputCache<DependencyCacheError<CafeProblem>, Environment>,
    /// Application state selected once at startup.
    board: Arc<MenuBoard>,
    /// Checked DSL registration assembled after acquiring the selected cache owners.
    routes: Router<CafeError, Environment>,
}

fn menu_output_error(error: WebError) -> DependencyCacheError<CafeProblem> {
    DependencyCacheError::Load(CafeProblem::Web(error))
}

type MenuModel = (SharedValue<Arc<str>>, SharedValue<bool>);
const SEASONAL_DRINK: &str = "Maple oat latte";

fn menu_model(
    menu: ReadThrough<MenuKey, Arc<str>, DependencyCacheError<CafeProblem>, Environment>,
    seasonal: DependentReadThrough<MenuKey, MenuDependency, bool, CafeProblem, Environment>,
    prepared_partition: MenuPartition,
    input: PreparedOutputInput,
) -> Effect<MenuModel, DependencyCacheError<CafeProblem>, Environment> {
    let key = effect_try_sync! {
        once;
        body: move || {
            let partition = input.partition();
            if *partition == prepared_partition.output {
                return Ok(prepared_partition.key);
            }
            let [audience, culture, representation] = partition.dimensions.as_ref() else {
                return Err(WebError::Configuration("café menu partition".into()));
            };
            Ok(MenuKey {
                cafe: partition.namespace.clone(),
                audience: audience.clone(),
                culture: CultureId::new(culture).map_err(WebError::Localization)?,
                representation: representation.clone(),
            })
        },
        on_error: |error| {
            let problem = if let WebError::Localization(error) = error {
                CafeProblem::Language(error)
            } else {
                CafeProblem::Web(error)
            };
            DependencyCacheError::Load(problem)
        },
    };
    effect! {
        let key = key.clone(); let menu = menu.clone(); let seasonal = seasonal.clone();
        let key = key?;
        let menu_origin = |error| DependencyCacheError::Load(CafeProblem::MenuOrigin(error));
        let ordinary = effect_map_error! { program: read_through! { get; cache: menu, key: key.clone(), }, map: menu_origin, }?;
        let seasonal = effect_map_error! { program: dependent_read_through! { get; cache: seasonal, key: key, }, map: menu_origin, }?;
        (ordinary, seasonal)
    }
}

#[expect(clippy::result_large_err, reason = "the finite cache writer returns the original unboxed error domain")]
fn write_menu(
    (ordinary, seasonal): MenuModel,
    writer: &mut PreparedOutputWriter<'_>,
) -> Result<(), DependencyCacheError<CafeProblem>> {
    ordinary
        .with(|cafe| {
            writer.write(cafe.as_bytes())?;
            writer.write(b": espresso, oat latte")
        })
        .map_err(|error| DependencyCacheError::Load(CafeProblem::MenuValue(error)))?
        .map_err(menu_output_error)?;
    seasonal
        .with(|available| {
            if *available {
                writer.write(b"; seasonal: ")?;
                writer.write(SEASONAL_DRINK.as_bytes())?;
            }
            Ok(())
        })
        .map_err(|error| DependencyCacheError::Load(CafeProblem::MenuValue(error)))?
        .map_err(menu_output_error)
}
/// Choose the SQL interpretation of the same readiness handler.
pub enum DatabaseReadiness {
    /// Use the ordinary native scalar query.
    Ordinary,
    /// Preadmit prepared row, cleanup and session backing for the borrowed-row query.
    Prepared,
}

/// Explicit operational policy inputs; units/bounds come from the host configuration.
struct CafePolicy {
    /// Native body/drain/error policy, supplied by the owning Server consumer.
    pub server: ServerPolicy<CafeError, Environment>,
    /// Deliberately selected listener/protocols, with no library-selected port.
    pub listen: SocketAddr,
    /// Compiled native HTTP transport selection.
    pub protocols: HttpProtocols,
    /// Settled menu retention policy.
    pub retain: Retain,
    /// Maximum active menu producers, independent from retained entries.
    pub loaders: NonZeroUsize,
    /// Maximum dependencies per seasonal result, including empty membership.
    pub dependencies: NonZeroUsize,
    /// Resource and rendered-message byte/depth bounds.
    pub catalog: CatalogLimits,
    /// Explicit refresh interval for complete catalogue reloads.
    pub reload: Duration,
    /// Per-probe/whole-readiness elapsed bounds.
    pub probes: ProbeDeadlines,
    /// Maximum simultaneous readiness checks.
    pub probe_parallelism: NonZeroUsize,
    /// SQL admission selected for the existing orders database probe.
    pub database_readiness: DatabaseReadiness,
    /// Local write-hold Retry-After guidance in whole seconds.
    pub retry_seconds: NonZeroU64,
    /// Explicit finite response policies published to eligible sources only.
    pub output: FiniteOutputPolicy,
}
/// Extracted loader shares only the board/key handles. The original Vec moves once;
/// every result records membership, including the deliberately empty seasonal menu.
pub fn seasonal_menu<R: Send + Sync + 'static>(
    board: Arc<MenuBoard>,
    _key: Arc<MenuKey>,
) -> Effect<(Vec<String>, Vec<MenuDependency>), CafeProblem, R> {
    effect_sync! {
        body {
            let menu = if board.seasonal_available.load(Ordering::Acquire) {
                vec![String::from(SEASONAL_DRINK)]
            } else { Vec::new() };
            (menu, vec![MenuDependency::SeasonalMembership])
        },
    }
}

fn seasonal_menu_prepared(
    board: Arc<MenuBoard>,
    mut writer: PreparedDependencyWriter<MenuDependency>,
) -> Effect<(bool, PreparedDependencyWriter<MenuDependency>), CafeProblem, Environment> {
    effect_sync! {
        once;
        body: move || {
            let _ = writer.push(MenuDependency::SeasonalMembership);
            (board.seasonal_available.load(Ordering::Acquire), writer)
        },
    }
}
/// Staff publishes the actual membership change then fences/joins active loaders
/// through the existing core dependency owner. This is not a second cache index.
fn update_seasonal_menu(
    capabilities: &CafeCapabilities,
    available: bool,
) -> Effect<(), CafeError, Environment> {
    let board = capabilities.board.clone();
    let cache = capabilities.seasonal.clone();
    let output = capabilities.output.clone();
    effect! {
        let board = board.clone(); let cache = cache.clone(); let output = output.clone();
        effect_sync! {
            body { board.seasonal_available.store(available, Ordering::Release) },
        }?;
        dependent_read_through! { invalidate_dependency; cache: cache, dependency: MenuDependency::SeasonalMembership, }?;
        web! { clear_output_cache; source: output, }?;
    }
}
/// Manual operator refresh uses the same complete-validate-atomic-publish operation
/// as D27's scoped interval worker. A failed refresh retains the last good generation.
pub fn refresh_translations<R: Send + Sync + 'static>(
    catalog: &CatalogRuntime<CafeMessages, R>,
) -> Effect<(), LocalizationError, R> {
    localization! { reload; catalog: catalog, }
}
/// A fresh request moves its customer's typed argument into the pinned catalogue;
/// native finite HTML escaping handles the rendered text without raw interpolation.
pub fn pickup_message<R: HasRequest + Send + Sync + 'static>(
    customer: String,
) -> Effect<(PinnedCatalog<CafeMessages>, BoundMessage<OrderReady>), CafeError, R> {
    let customer = effect_sync! { once; body { customer }, };
    effect! {
        let customer = customer.clone();
        let customer = customer?;
        let message = effect_try_sync! { once; body { message_arguments! { message: Message::<OrderReady>::new(), field("customer", customer); } }, on_error |error| { cafe_error(CafeProblem::Language(error)) }, }?;
        let catalog = service! { env |env: &R| { env.request().with_extensions(|extensions| extensions.get::<PinnedCatalog<CafeMessages>>().cloned()) }, }?;
        let catalog = effect_try_sync! {
            once;
            body { catalog.ok_or_else(|| LocalizationError::Definition("request catalogue".into())) },
            on_error |error| { cafe_error(CafeProblem::Language(error)) },
        }?;
        (catalog, message)
    }
}
/// Execute the integrated recipe on the caller's reactor until selected cancellation.
/// Native request/body owners and startup resources drain before this Effect settles.
/// The enclosing host scope then drains its owned providers. This boundary retains
/// the caller's interpreter context, SQLx Identity and OTel services throughout.
fn run_cafe_application(
    application: PreparedWeb<CafeCapabilities, CafeError, Environment>,
    environment: Arc<Environment>,
) -> Effect<PreparedHostResult<(), CafeError, Environment>, CafeError, Environment> {
    effect_provide_some! {
        shared;
        program: web! { run; source: application, },
        project: move |_: &Environment| Arc::clone(&environment),
    }
}
/// Execute the real configured host; native serving and all cleanup finish before return.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    cafe_host::run()
}
