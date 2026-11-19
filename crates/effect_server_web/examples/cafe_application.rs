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
    AdmissionError, CacheKey, DependencyCacheError, Effect, HasCancellation, HasClock,
    HasTaskSpawner, PreparedCacheRefusals, PreparedCacheStorage, PreparedDependencyWriter,
    SharedValue, effect, effect_map_error, effect_provide_some, effect_provided, effect_sync,
    effect_try_sync, service,
};
use effect_server::{
    HasRequest, HttpProtocols, PreparedEndpointBytePools, PreparedHostResult, PreparedServer,
    Reply, Request, Router, Server, ServerError, ServerPolicy, StaticOverloadReply, endpoint, reply,
    router,
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
    CatalogSelection, Contract, CultureId, CultureSet, Localization, LocalizationError, Message,
    MessageContract, MissingMessage, NamedArgument, PinnedCatalog, PreparedCatalogGenerations,
};
use effect_server_web::observability::Observability;
use effect_server_web::output_cache::{
    FiniteOutputPolicy, FiniteSource, OutputCache, OutputCacheDefinition, OutputPartition,
    PreparedOutputCacheStorage, PreparedOutputInput, PreparedOutputWriter,
};
use effect_server_web::security::Security;
use effect_server_web::{
    WebError, availability, dependent_read_through, message_arguments, read_through, web,
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

/// Prepared host inputs assembled from the same arena as the café services.
pub struct CafeHostStorage<F> {
    pub(crate) factory: F,
    pub(crate) pools: Arc<PreparedEndpointBytePools>,
    pub(crate) overload: StaticOverloadReply,
}

/// Serve the menu and localized pickup announcement with native account security. Request projection
/// receives verified Identity/culture metadata before producing the same selected R.
/// The caller's key selector must use that R's tenant/audience/culture/representation.
#[expect(clippy::result_large_err, reason = "startup preserves the owned café and SQL error without indirection")]
pub fn cafe_application<R, F>(
    (policy, board, sql): (CafePolicy<R>, Arc<MenuBoard>, effect_sql::SqlClient),
    (selected_security, observation, request_loop, customer_routes): CafeRequestServices<R>,
    (resources, cultures): (Vec<CatalogResource<R>>, CultureSet),
    choose_language: impl Fn(Request) -> Effect<Arc<CatalogSelection>, LocalizationError, R>
    + Send
    + Sync
    + 'static,
    bind_request: impl Fn(Arc<CafeCapabilities<R>>, Request) -> Effect<Arc<R>, CafeError, R>
    + Send
    + Sync
    + 'static,
    (menu_key, authenticated, prepared_partition): (
        impl Fn(&R) -> MenuKey + Send + Sync + 'static,
        impl Fn(&R) -> bool + Send + Sync + 'static,
        MenuPartition,
    ),
    (prepared, host): (CafeMemory, CafeHostStorage<F>),
) -> Result<PreparedWeb<CafeCapabilities<R>, CafeError, R>, CafeProblem>
where
    R: HasRequest + HasClock + HasTaskSpawner + effect_otel::HasOtel + Send + Sync + 'static,
    F: Fn(Server<CafeError, R>) -> Result<PreparedServer<CafeError, R>, ServerError>
        + Send + Sync + 'static,
{
    let catalogue = CatalogDefinition::<CafeMessages, R>::new(
        "cafe",
        policy.catalog,
        resources,
        vec![Contract::of::<OrderReady>()],
    )
    .map_err(CafeProblem::Language)?;
    let translations = Localization::new_prepared(
        catalogue,
        cultures,
        choose_language,
        MissingMessage::Fail,
        Some(policy.reload),
        prepared.generations,
    )
    .map_err(CafeProblem::Language)?;
    let maintenance = Maintenance::new(
        MaintenanceMode::Open,
        RetryAfter::seconds(policy.retry_seconds),
    );
    let prepared_availability = prepared.availability;
    let availability = availability! {
        service: "neighborhood-cafe", errors: |error| cafe_error(CafeProblem::Availability(error)),
        readiness("orders-database", effect_map_error! { program: effect_server_web::database::readiness_prepared(&sql), map |error| { cafe_error(CafeProblem::Sql(error)) }, });
        process_liveness("request-loop", request_loop);
        traffic_ready: maintenance.traffic_ready(), deadline: policy.probes, parallelism: policy.probe_parallelism,
    }.map(|definition| definition.with_prepared_workspace(prepared_availability));
    let menu_key = Arc::new(menu_key);
    let authenticated = Arc::new(authenticated);
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
    let app = web! {
        policy: policy.server, web_errors: |error| cafe_error(CafeProblem::Web(error)),
        startup |scope: effect::Scope<CafeError>| {
            let board = startup_board.clone(); let maintenance = startup_maintenance.clone(); let menu_key = menu_key.clone(); let customer_routes = customer_routes.clone();
            let prepared_menu = prepared_menu.clone(); let prepared_seasonal = prepared_seasonal.clone(); let prepared_output = prepared_output.clone();
            let refusals = refusals.clone(); let output_policy = output_policy.clone(); let authenticated = authenticated.clone();
            let prepared_partition = prepared_partition.clone();
            let menu_memory = prepared_menu?;
            let seasonal_memory = prepared_seasonal?;
            let output_memory = prepared_output?;
            let menu = read_through! {
                scope: scope.clone(), retain: retained,
                memory: menu_memory, refusals: refusals.clone(),
                admission: LoadAdmission { maximum: loaders, on_full: Arc::new(|error| DependencyCacheError::Load(CafeProblem::Busy(error))) },
                keys: |left: &MenuKey, right: &MenuKey| left == right,
                cache_failures: |_: &DependencyCacheError<CafeProblem>| false,
                loader |key: CacheKey<MenuKey>| { key.cafe.clone() }
            }?;
            let seasonal_board = board.clone();
            let seasonal = dependent_read_through! {
                scope: scope.clone(), retain: retained,
                memory: seasonal_memory, refusals: refusals.clone(),
                admission: LoadAdmission { maximum: loaders, on_full: Arc::new(CafeProblem::Busy) },
                maximum_dependencies: dependencies,
                keys: |left: &MenuKey, right: &MenuKey| left == right,
                dependencies: |left: &MenuDependency, right: &MenuDependency| left == right,
                cache_failures: |_: &CafeProblem| false,
                loader |key: CacheKey<MenuKey>, writer: PreparedDependencyWriter<MenuDependency>| {
                    let board = seasonal_board.clone();
                    seasonal_menu_prepared(board, key, writer)?
                },
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
            let key = menu_key.clone();
            let output = OutputCache::scoped_prepared(
                scope, output_policy, menu_source, (output_memory, refusals),
                |_| effect_sync! { body { () }, },
                move |env| authenticated(env),
                move |env, _| {
                    let key = key(env);
                    Ok(if key == prepared_partition.key { prepared_partition.output.clone() } else { menu_partition(&key) })
                },
                menu_output_error,
            )?;
            let menu_response = output.clone();
            let control = maintenance.clone();
            let menu_endpoint = endpoint! {
                effect(control.admit(AdmissionClass::Read, move || menu_response.reply(),
                |_| effect! {
                    reply! {
                    bytes(bytes::Bytes::from_static("The café is closing; please return after reopening.".as_bytes()), http::HeaderValue::from_static("text/plain; charset=utf-8"));
                } })); };
            let routes = effect_try_sync! {
                once;
                body {
                    router! {
                get("/menu", menu_endpoint);
                include(customer_routes);
            } }, on_error |error| { cafe_error(CafeProblem::Http(error)) }, }?;
            let capabilities = CafeCapabilities { menu, seasonal, output, board, maintenance, routes };
            let seasonal = capabilities.seasonal.clone();
            let initial_key = service! { env |env: &R| { menu_key(env) }, }?;
            seasonal.get(initial_key)?;
            update_seasonal_menu(&capabilities, true)?;
            capabilities
        }
        environment: bind_request,
        security: selected_security, localization: translations, otel: observation, availability: availability,
        output_cache: OutputCacheDefinition::new(vec![policy.output]),
        memory: host.factory, byte_pools: host.pools, overload: host.overload,
        listen: policy.listen, protocols: policy.protocols,
        routes: |capabilities: Arc<CafeCapabilities<R>>| capabilities.routes.clone(),
    };
    Ok(app)
}
/// Shared error ownership lets both real scoped cache engines retain non-Clone errors.
pub type CafeError = SharedValue<DependencyCacheError<CafeProblem>>;
/// Native request security, observability and liveness selected by the café host.
pub type CafeRequestServices<R> = (
    Security<CafeError, R>,
    Observability<CafeError, R>,
    Effect<bool, CafeError, R>,
    Router<CafeError, R>,
);
/// Café failures retain their original provider boundary rather than becoming denial.
#[derive(Debug)]
pub enum CafeProblem {
    /// No order is assigned to this verified customer.
    MissingOrder,
    /// Checked Web policy or middleware failure.
    Web(WebError),
    /// Safe catalogue acquisition/selection error.
    Language(LocalizationError),
    /// Original selected Identity/native SQL error.
    Identity(Box<IdentityError>),
    /// Native HTTP registration/input failure.
    Http(ServerError),
    /// Availability policy failure.
    Availability(AvailabilityError),
    /// Immediate producer capacity denial.
    Busy(AdmissionError),
    /// Original failure from a menu origin cache.
    MenuOrigin(CafeError),
    /// Original defect reading a shared menu value.
    MenuValue(effect::Defect),
    /// Original SQL readiness failure.
    Sql(effect_sql::SqlError),
    /// Native OTel context/signal failure.
    Otel(effect_otel::NativeError),
    /// Ordered native provider acquisition/shutdown failure.
    Lifecycle(effect_otel::LifecycleError),
}
impl std::fmt::Display for CafeProblem {
    /// Public launch reporting identifies the failed boundary without credentials.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MissingOrder => "café pickup order absent",
            Self::Web(_) => "café web policy",
            Self::Language(_) => "café catalogue",
            Self::Identity(_) => "café identity",
            Self::Http(_) => "café HTTP",
            Self::Availability(_) => "café readiness",
            Self::Busy(_) => "café producer capacity",
            Self::MenuOrigin(_) => "café menu origin",
            Self::MenuValue(_) => "café menu value",
            Self::Sql(_) => "café SQL",
            Self::Otel(_) => "café telemetry",
            Self::Lifecycle(_) => "café provider lifecycle",
        })
    }
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
pub struct CafeCapabilities<R: Send + Sync + 'static> {
    /// Ordinary menu read-through over the existing core cache.
    pub menu: ReadThrough<MenuKey, Arc<str>, DependencyCacheError<CafeProblem>, R>,
    /// Membership-aware seasonal menu over the existing dependency engine.
    pub seasonal: DependentReadThrough<MenuKey, MenuDependency, bool, CafeProblem, R>,
    output: OutputCache<DependencyCacheError<CafeProblem>, R>,
    /// Application state selected once at startup.
    pub board: Arc<MenuBoard>,
    /// Process-local admission policy, separate from distributed authorization.
    pub maintenance: Maintenance,
    /// Checked DSL registration assembled after acquiring the selected cache owners.
    pub routes: Router<CafeError, R>,
}

fn menu_output_error(error: WebError) -> DependencyCacheError<CafeProblem> {
    DependencyCacheError::Load(CafeProblem::Web(error))
}

type MenuModel = (SharedValue<Arc<str>>, SharedValue<bool>);
const SEASONAL_DRINK: &str = "Maple oat latte";

fn menu_model<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
    menu: ReadThrough<MenuKey, Arc<str>, DependencyCacheError<CafeProblem>, R>,
    seasonal: DependentReadThrough<MenuKey, MenuDependency, bool, CafeProblem, R>,
    prepared_partition: MenuPartition,
    input: PreparedOutputInput,
) -> Effect<MenuModel, DependencyCacheError<CafeProblem>, R> {
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
        let ordinary = effect_map_error! { program: menu.get(key.clone()), map |error| { DependencyCacheError::Load(CafeProblem::MenuOrigin(error)) }, }?;
        let seasonal = effect_map_error! { program: seasonal.get(key), map |error| { DependencyCacheError::Load(CafeProblem::MenuOrigin(error)) }, }?;
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
/// Explicit operational policy inputs; units/bounds come from the host configuration.
pub struct CafePolicy<R: Send + Sync + 'static> {
    /// Native body/drain/error policy, supplied by the owning Server consumer.
    pub server: ServerPolicy<CafeError, R>,
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

fn seasonal_menu_prepared<R: Send + Sync + 'static>(
    board: Arc<MenuBoard>,
    _key: CacheKey<MenuKey>,
    mut writer: PreparedDependencyWriter<MenuDependency>,
) -> Effect<(bool, PreparedDependencyWriter<MenuDependency>), CafeProblem, R> {
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
pub fn update_seasonal_menu<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
    capabilities: &CafeCapabilities<R>,
    available: bool,
) -> Effect<(), CafeError, R> {
    let board = capabilities.board.clone();
    let cache = capabilities.seasonal.clone();
    let output = capabilities.output.clone();
    effect! {
        let board = board.clone(); let cache = cache.clone(); let output = output.clone();
        effect_sync! {
            body { board.seasonal_available.store(available, Ordering::Release) },
        }?;
        cache.invalidate_dependency(MenuDependency::SeasonalMembership)?;
        output.clear()?;
    }
}
/// Manual operator refresh uses the same complete-validate-atomic-publish operation
/// as D27's scoped interval worker. A failed refresh retains the last good generation.
pub fn refresh_translations<R: Send + Sync + 'static>(
    catalog: &CatalogRuntime<CafeMessages, R>,
) -> Effect<(), LocalizationError, R> {
    catalog.reload()
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
pub fn run_cafe_application<R>(
    application: PreparedWeb<CafeCapabilities<R>, CafeError, R>,
    environment: Arc<R>,
) -> Effect<PreparedHostResult<(), CafeError, R>, CafeError, R>
where
    R: HasRequest
        + HasClock
        + HasTaskSpawner
        + HasCancellation
        + effect_server::HasTokio
        + effect_otel::HasOtel
        + Send
        + Sync
        + 'static,
{
    effect_provide_some! {
        program: effect_provided! { program: application.run(), environment: environment, },
        project |_outer: &R| { () },
    }
}
/// Execute the real configured host; native serving and all cleanup finish before return.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    cafe_host::run()
}
