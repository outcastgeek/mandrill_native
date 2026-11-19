//! Managed route registration, permission protection and permission metadata
//! synchronization.
//!
//! # What this owner decides
//!
//! - the normalized method, display name and category of a route description;
//! - the normalized, order-preserving permission list and whether the route requires
//!   any one permission or every permission;
//! - whether a route description is admissible for registration;
//! - the additive permission metadata that a route's declared permissions require,
//!   including reconciliation after a failed write;
//! - when and how the pending metadata synchronizations are executed.
//!
//! # What the consumer supplies
//!
//! The permission middleware that enforces a requirement, the route/permission
//! catalogue that resolves permissions, resolves or creates route metadata and holds the
//! route's required permissions, and the typed server mounter that installs one
//! protected route in the consumer's own router. The CMS owns no permission record,
//! route record, database, SQL statement, schema, migration, pool or router, serves no
//! listener and keeps no process-global state.
//!
//! # Registration
//!
//! `register` admits a route description (blank name or pattern, or no handler, is
//! skipped exactly as declared), protects it, and hands it to the consumer's mounter
//! together with the normalized method, the declared pattern, the resolved display
//! name/category and the declared description. A missing router is a no-op before any
//! protection runs. A protected route that must be mounted without a supplied mounter is
//! a configuration failure.
//!
//! # Protection
//!
//! A route without a handler is not protected. Otherwise protection normalizes the
//! declared permissions, then selects the consumer's any-permission or all-permission
//! requirement and applies it once to the handler. A route without permissions, or
//! without a permission middleware, is a configuration failure rather than a silently
//! unprotected route.
//!
//! # Permission metadata
//!
//! Protection also prepares the additive metadata synchronization for its normalized
//! permissions: it resolves every named permission, resolves the route by method and
//! pattern, creates the route metadata when it is missing, and adds the permission
//! associations the route does not already require. Nothing is removed.
//!
//! A failed route creation is reconciled by looking the route up again; a failed
//! association write is reconciled by re-reading the route's requirement. The desired
//! state is accepted only when it now exists; otherwise the original write failure is
//! reported unchanged. Reconciliation adds no bound on the consumer's error type.
//!
//! # Execution
//!
//! Preparation and mounting start no metadata tasks on their own. Operational
//! registration starts each admitted route's synchronization before its middleware
//! wrapping and mount. Tasks belong to the supplied application-lifetime scope and
//! survive a later registration failure; the returned fiber observes the started batch.
//! Each synchronization holds one slot of the service's single two-slot limit from its
//! first attempt through its retries, delays and terminal report, and the limit is shared
//! by every operational registration of that service. An expected synchronization failure
//! is reported to the environment's console and does not fail registration; a refused
//! submission, a closed or exhausted slot, a defect, interruption or finalization failure
//! keeps its own exit.
//!
//! # Failure branches
//!
//! - An empty pattern and an empty permission list are reported before any capability
//!   runs.
//! - A permission that the catalogue cannot resolve is reported with the permission
//!   name.
//! - A route that cannot be resolved, created or reconciled is reported with the method
//!   and pattern; the consumer's failure cause is preserved.
//! - The route's requirement is read once; a missing requirement means no association is
//!   required yet, and a failed association write keeps the route's own write failure.

use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use effect::{
    ConsoleChannel, Effect, Fiber, HasClock, HasConsole, HasTaskSpawner, Schedule, Scope,
    Semaphore, SemaphoreError, effect, effect_all_concurrent, effect_map, effect_map_error,
    effect_permit, effect_recover, effect_retry, effect_sync, effect_try_sync,
};
use effect::memory::MemoryError;
use effect_server::{Reply, Request, Router, ServerError};
use http::Method;

mod prepared;
pub use prepared::{PreparedManagedRouteCapacity, PreparedManagedRouteStorage};
use prepared::RouteTable;

/// Produces the response for one registered route.
///
/// The consumer mounts this capability in its own router; registration time is request
/// free and the effect runs only when the consumer invokes it for a request.
pub type RouteHandler<E, R> =
    Arc<dyn Fn(&Request) -> Effect<Reply<E, R>, E, R> + Send + Sync + 'static>;

/// Requires a permission list on one handler.
pub type PermissionGuard<E, R> =
    Arc<dyn Fn(RouteHandler<E, R>) -> RouteHandler<E, R> + Send + Sync + 'static>;

/// Builds the requirement for one permission list.
pub type PermissionRequirementBuilder<E, R> =
    Arc<dyn Fn(&[String]) -> PermissionGuard<E, R> + Send + Sync + 'static>;

/// Resolves one permission by name.
pub type PermissionByName<E, R> =
    Arc<dyn Fn(&str) -> Effect<Option<RoutePermission>, E, R> + Send + Sync + 'static>;

/// Resolves one route by pattern and normalized method.
pub type RouteByPathAndMethod<E, R> =
    Arc<dyn Fn(&str, &str) -> Effect<Option<RouteRecord>, E, R> + Send + Sync + 'static>;

/// Creates the metadata of one route.
pub type CreateRouteMetadata<E, R> =
    Arc<dyn Fn(&RouteDraft) -> Effect<RouteRecord, E, R> + Send + Sync + 'static>;

/// Reads the permissions one route requires.
pub type RouteRequirementLookup<E, R> =
    Arc<dyn Fn(&str) -> Effect<Option<RouteRequirement>, E, R> + Send + Sync + 'static>;

/// Adds one permission association to a route.
pub type AddRoutePermission<E, R> =
    Arc<dyn Fn(&str, &str) -> Effect<(), E, R> + Send + Sync + 'static>;

/// Reports whether one route already requires one permission.
pub type RoutePermissionLookup<E, R> =
    Arc<dyn Fn(&str, &str) -> Effect<bool, E, R> + Send + Sync + 'static>;

/// Mounts one protected route in the consumer's own typed router.
///
/// The mounter receives the consumer's router and the admitted route description, and
/// returns the router extended with that route. It owns the typed capture shape; this
/// owner owns admission, protection, ordering and the metadata handed to it.
pub type ManagedRouteMounter<E, R> = Arc<
    dyn Fn(Router<E, R>, &ManagedRouteRegistration<E, R>) -> Result<Router<E, R>, ServerError>
        + Send
        + Sync
        + 'static,
>;

/// The default number of metadata-synchronization attempts.
pub const DEFAULT_ASYNC_SYNC_ATTEMPTS: usize = 30;
/// The default delay between metadata-synchronization attempts.
pub const DEFAULT_ASYNC_SYNC_RETRY_DELAY: Duration = Duration::from_secs(2);
/// The default number of metadata synchronizations that run at the same time.
pub const DEFAULT_ASYNC_SYNC_SLOTS: usize = 2;
/// The default route category.
pub const DEFAULT_ROUTE_CATEGORY: &str = "content_management";
/// The any-permission mode spelling.
pub const PERMISSION_MODE_ANY: &str = "any";
/// The all-permission mode spelling.
pub const PERMISSION_MODE_ALL: &str = "all";

/// Selects any-permission or all-permission enforcement.
///
/// The value is an open string: only [`PERMISSION_MODE_ALL`] selects all-permission
/// enforcement, so the `any` spelling, an empty value and any other value select
/// any-permission enforcement.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PermissionMode(String);

impl PermissionMode {
    /// Admit the caller-supplied mode exactly as written.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The mode exactly as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for PermissionMode {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for PermissionMode {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl AsRef<str> for PermissionMode {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for PermissionMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0.as_str())
    }
}

/// One managed route description.
///
/// The handler is optional because a route description may be declared for permission or
/// metadata purposes only; such a route is skipped by registration and protection.
pub struct ManagedRoute<E: Send + 'static, R: Send + Sync + 'static> {
    /// The route name used for registration and filtering.
    pub name: String,
    /// The declared method; registration normalizes it.
    pub method: String,
    /// The declared route pattern.
    pub pattern: String,
    /// The request handler the consumer mounts, when one is declared.
    pub handler: Option<RouteHandler<E, R>>,
    /// The declared permissions; protection normalizes them.
    pub permissions: Vec<String>,
    /// The permission mode.
    pub permission_mode: PermissionMode,
    /// The display name, or empty to derive it.
    pub display_name: String,
    /// The route description.
    pub description: String,
    /// The category, or empty for the default category.
    pub category: String,
}

impl<E: Send + 'static, R: Send + Sync + 'static> ManagedRoute<E, R> {
    /// Declare one routable managed route.
    ///
    /// The remaining description fields are left empty for the caller to set.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        method: impl Into<String>,
        pattern: impl Into<String>,
        handler: RouteHandler<E, R>,
    ) -> Self {
        Self {
            name: name.into(),
            method: method.into(),
            pattern: pattern.into(),
            handler: Some(handler),
            permissions: Vec::new(),
            permission_mode: PermissionMode::default(),
            display_name: String::new(),
            description: String::new(),
            category: String::new(),
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for ManagedRoute<E, R> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            method: self.method.clone(),
            pattern: self.pattern.clone(),
            handler: self.handler.clone(),
            permissions: self.permissions.clone(),
            permission_mode: self.permission_mode.clone(),
            display_name: self.display_name.clone(),
            description: self.description.clone(),
            category: self.category.clone(),
        }
    }
}

/// A permission known to the consumer's catalogue.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RoutePermission {
    /// The permission identifier.
    pub id: String,
    /// The permission name.
    pub name: String,
}

/// A route known to the consumer's catalogue.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RouteRecord {
    /// The route identifier.
    pub id: String,
}

/// The permissions a route currently requires.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RouteRequirement {
    /// The required permissions.
    pub required_permissions: Vec<RoutePermission>,
}

/// The metadata describing one route to create.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RouteDraft {
    /// The route pattern.
    pub pattern: String,
    /// The normalized method.
    pub method: String,
    /// The resolved display name.
    pub display_name: String,
    /// The trimmed description.
    pub description: String,
    /// The resolved category.
    pub category: String,
    /// Whether the route is public metadata.
    pub is_public: bool,
}

/// The consumer's route and permission catalogue.
///
/// Every operation is a required constructor argument; the value is a capability
/// aggregate, not a service locator. A route/requirement lookup reports an absent record
/// as no value, so a not-found answer is not confused with a failure.
pub struct RouteCatalogue<E: Send + 'static, R: Send + Sync + 'static> {
    /// Resolve one permission by name.
    pub permission_by_name: PermissionByName<E, R>,
    /// Resolve one route by pattern and normalized method.
    pub route_by_path_and_method: RouteByPathAndMethod<E, R>,
    /// Create the route metadata.
    pub create_route: CreateRouteMetadata<E, R>,
    /// Read the route's required permissions.
    pub route_requirement: RouteRequirementLookup<E, R>,
    /// Add one permission association to a route.
    pub add_permission: AddRoutePermission<E, R>,
    /// Report whether the route already requires one permission.
    pub route_has_permission: RoutePermissionLookup<E, R>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> RouteCatalogue<E, R> {
    /// Construct the catalogue from its complete capability set.
    #[must_use]
    pub fn new(
        permission_by_name: PermissionByName<E, R>,
        route_by_path_and_method: RouteByPathAndMethod<E, R>,
        create_route: CreateRouteMetadata<E, R>,
        route_requirement: RouteRequirementLookup<E, R>,
        add_permission: AddRoutePermission<E, R>,
        route_has_permission: RoutePermissionLookup<E, R>,
    ) -> Self {
        Self {
            permission_by_name,
            route_by_path_and_method,
            create_route,
            route_requirement,
            add_permission,
            route_has_permission,
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RouteCatalogue<E, R> {
    fn clone(&self) -> Self {
        Self {
            permission_by_name: Arc::clone(&self.permission_by_name),
            route_by_path_and_method: Arc::clone(&self.route_by_path_and_method),
            create_route: Arc::clone(&self.create_route),
            route_requirement: Arc::clone(&self.route_requirement),
            add_permission: Arc::clone(&self.add_permission),
            route_has_permission: Arc::clone(&self.route_has_permission),
        }
    }
}

/// Enforces one permission requirement on a handler.
pub struct PermissionMiddleware<E: Send + 'static, R: Send + Sync + 'static> {
    require_any: PermissionRequirementBuilder<E, R>,
    require_all: PermissionRequirementBuilder<E, R>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> PermissionMiddleware<E, R> {
    /// Construct the middleware from both requirement builders.
    #[must_use]
    pub fn new(
        require_any: PermissionRequirementBuilder<E, R>,
        require_all: PermissionRequirementBuilder<E, R>,
    ) -> Self {
        Self {
            require_any,
            require_all,
        }
    }

    /// Build the any-permission requirement for a normalized permission list.
    #[must_use]
    pub fn require_any(&self, permissions: &[String]) -> PermissionGuard<E, R> {
        (self.require_any)(permissions)
    }

    /// Build the all-permission requirement for a normalized permission list.
    #[must_use]
    pub fn require_all(&self, permissions: &[String]) -> PermissionGuard<E, R> {
        (self.require_all)(permissions)
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for PermissionMiddleware<E, R> {
    fn clone(&self) -> Self {
        Self {
            require_any: Arc::clone(&self.require_any),
            require_all: Arc::clone(&self.require_all),
        }
    }
}

/// Configures managed-route protection and metadata synchronization.
///
/// Both capabilities are genuinely optional: a configuration may guard routes without a
/// catalogue, or synchronize metadata without a middleware.
pub struct AccessControl<E: Send + 'static, R: Send + Sync + 'static> {
    /// The permission middleware used to protect handlers.
    pub middleware: Option<PermissionMiddleware<E, R>>,
    /// The permission metadata catalogue.
    pub catalogue: Option<RouteCatalogue<E, R>>,
    /// Whether metadata synchronization is disabled.
    pub disable_metadata_sync: bool,
    /// The metadata-synchronization attempt count; a non-positive value selects the
    /// default.
    pub async_sync_attempts: i64,
    /// The delay between metadata-synchronization attempts; a zero value selects the
    /// default.
    pub async_sync_retry_delay: Duration,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for AccessControl<E, R> {
    fn clone(&self) -> Self {
        Self {
            middleware: self.middleware.clone(),
            catalogue: self.catalogue.clone(),
            disable_metadata_sync: self.disable_metadata_sync,
            async_sync_attempts: self.async_sync_attempts,
            async_sync_retry_delay: self.async_sync_retry_delay,
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default for AccessControl<E, R> {
    fn default() -> Self {
        Self {
            middleware: None,
            catalogue: None,
            disable_metadata_sync: false,
            async_sync_attempts: 0,
            async_sync_retry_delay: Duration::ZERO,
        }
    }
}

/// The deferred permission synchronization for one route.
///
/// The program runs the additive synchronization once per retry; the route identity is
/// retained for the failure report.
pub struct MetadataSync<E: Send + 'static, R: Send + Sync + 'static> {
    route_name: String,
    method: String,
    pattern: String,
    program: Arc<dyn Fn() -> Effect<(), MetadataSyncError<E>, R> + Send + Sync + 'static>,
    retries: usize,
    delay: Duration,
}

impl<E: Send + 'static, R: Send + Sync + 'static> MetadataSync<E, R> {
    fn new(
        route_name: String,
        method: String,
        pattern: String,
        program: Arc<dyn Fn() -> Effect<(), MetadataSyncError<E>, R> + Send + Sync + 'static>,
        retries: usize,
        delay: Duration,
    ) -> Self {
        Self {
            route_name,
            method,
            pattern,
            program,
            retries,
            delay,
        }
    }

    /// The route name this synchronization describes.
    #[must_use]
    pub fn route_name(&self) -> &str {
        self.route_name.as_str()
    }

    /// The normalized method this synchronization describes.
    #[must_use]
    pub fn method(&self) -> &str {
        self.method.as_str()
    }

    /// The trimmed pattern this synchronization describes.
    #[must_use]
    pub fn pattern(&self) -> &str {
        self.pattern.as_str()
    }

    /// Build the retrying synchronization program.
    #[must_use]
    pub fn program(&self) -> Effect<(), MetadataSyncError<E>, R>
    where
        R: HasClock,
    {
        let program = (self.program)();
        let schedule = Schedule::<MetadataSyncError<E>, usize>::recurs(self.retries)
            .intersect(Schedule::spaced(self.delay));
        effect_retry! {
            program: program,
            schedule: schedule,
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for MetadataSync<E, R> {
    fn clone(&self) -> Self {
        Self {
            route_name: self.route_name.clone(),
            method: self.method.clone(),
            pattern: self.pattern.clone(),
            program: Arc::clone(&self.program),
            retries: self.retries,
            delay: self.delay,
        }
    }
}

/// One admissible managed route with its protected handler and pending metadata
/// synchronization.
pub struct ManagedRouteRegistration<E: Send + 'static, R: Send + Sync + 'static> {
    /// The route name.
    pub name: String,
    /// The normalized method.
    pub method: String,
    /// The declared pattern.
    pub pattern: String,
    /// The declared permissions, unchanged.
    pub permissions: Vec<String>,
    /// The permission mode.
    pub permission_mode: PermissionMode,
    /// The resolved display name.
    pub display_name: String,
    /// The declared description.
    pub description: String,
    /// The resolved category.
    pub category: String,
    /// The protected handler.
    pub handler: RouteHandler<E, R>,
    /// The deferred metadata synchronization, when one is configured.
    pub metadata_sync: Option<MetadataSync<E, R>>,
}

/// The outcome of registering the configured routes.
pub struct RegisteredRoutes<E: Send + 'static, R: Send + Sync + 'static> {
    /// The consumer's router, extended with every mounted route. Absent when no router was
    /// supplied.
    pub router: Option<Router<E, R>>,
    /// The admitted routes in configuration order.
    pub routes: Vec<ManagedRouteRegistration<E, R>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> RegisteredRoutes<E, R> {
    /// The deferred synchronizations of the admitted routes, in configuration order.
    #[must_use]
    pub fn pending_metadata_syncs(&self) -> Vec<MetadataSync<E, R>> {
        self.routes
            .iter()
            .filter_map(|route| route.metadata_sync.clone())
            .collect()
    }
}

/// Configures the managed route service.
pub struct Options<E: Send + 'static, R: Send + Sync + 'static> {
    /// Protection and metadata configuration.
    pub access: AccessControl<E, R>,
    /// The managed route descriptions.
    pub routes: Vec<ManagedRoute<E, R>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for Options<E, R> {
    fn clone(&self) -> Self {
        Self {
            access: self.access.clone(),
            routes: self.routes.clone(),
        }
    }
}

/// Registers the configured managed routes and runs their synchronization.
///
/// The synchronization slot limit belongs to the service: every operational registration
/// performed by the same service shares it, so concurrent registration calls cannot
/// exceed it.
pub struct Service<E: Send + 'static, R: Send + Sync + 'static> {
    options: Arc<Options<E, R>>,
    slots: Arc<Semaphore>,
    prepared: Option<PreparedRoutes<E, R>>,
}

type PreparedRoutes<E, R> = (Arc<PreparedManagedRouteStorage<E, R>>, Arc<RouteTable<E, R>>);

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for Service<E, R> {
    fn clone(&self) -> Self {
        Self {
            options: self.options.clone(),
            slots: Arc::clone(&self.slots),
            prepared: self.prepared.clone(),
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Service<E, R> {
    /// Construct the service from its complete configuration.
    #[must_use]
    pub fn new(options: Options<E, R>) -> Self {
        let capacity = NonZeroUsize::new(DEFAULT_ASYNC_SYNC_SLOTS).unwrap_or(NonZeroUsize::MIN);
        Self {
            options: Arc::new(options),
            slots: Arc::new(Semaphore::full(capacity)),
            prepared: None,
        }
    }

    /// Move configured route descriptions into one admitted startup table.
    pub fn new_prepared(
        options: Options<E, R>,
        storage: Arc<PreparedManagedRouteStorage<E, R>>,
    ) -> Result<Self, MemoryError> {
        let Options { access, routes } = options;
        let table = storage.store(routes)?;
        let mut service = Self::new(Options { access, routes: Vec::new() });
        service.prepared = Some((storage, Arc::new(table)));
        Ok(service)
    }

    fn routes(&self) -> &[ManagedRoute<E, R>] {
        match &self.prepared {
            Some((_, table)) => table.as_slice(),
            None => &self.options.routes,
        }
    }

    /// Reuse this service's synchronization capacity for a newly resolved route set.
    pub(crate) fn with_routes(&self, routes: Vec<ManagedRoute<E, R>>) -> Result<Self, ManagedRouteError> {
        let (routes, prepared) = if let Some((storage, _)) = &self.prepared {
            let table = storage.store(routes).map_err(registration_memory)?;
            (Vec::new(), Some((storage.clone(), Arc::new(table))))
        } else {
            (routes, None)
        };
        Ok(Self {
            options: Arc::new(Options { access: self.options.access.clone(), routes }),
            slots: Arc::clone(&self.slots),
            prepared,
        })
    }

    /// Admit, protect and mount every configured route in configuration order.
    ///
    /// An absent router performs no work at all. An absent mounter leaves no way to
    /// install an admitted route, which is a configuration failure.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedRouteError`] when an admissible route cannot be protected or
    /// mounted, or when an admitted route has no mounter.
    pub fn register_routes(
        &self,
        router: Option<Router<E, R>>,
        mounter: Option<&ManagedRouteMounter<E, R>>,
    ) -> Result<RegisteredRoutes<E, R>, ManagedRouteError> {
        let mut routes = Vec::new();
        let mut current = router;
        if current.is_some() {
            if self.prepared.is_some() {
                routes.try_reserve_exact(self.routes().len()).map_err(|_| registration_memory(MemoryError::CollectionReservationFailed))?;
            } else {
                routes.reserve(self.routes().len());
            }
        }
        for route in self.routes() {
            let (next, registration) =
                register(current.take(), &self.options.access, mounter, route)?;
            if next.is_none() {
                return Ok(RegisteredRoutes {
                    router: None,
                    routes: Vec::new(),
                });
            }
            current = next;
            if let Some(registration) = registration {
                routes.push(registration);
            }
        }
        Ok(RegisteredRoutes {
            router: current,
            routes,
        })
    }

    /// Admit, protect, mount and start the synchronization of every configured route.
    ///
    /// This is the operational registration: each admitted route starts permission
    /// synchronization in the application-lifetime scope before protection and mounting.
    /// A later registration failure does not discard work already started. The returned fiber observes the started batch; a rejected or
    /// unwinding submission keeps its own full exit instead of being reported as success.
    ///
    /// Preparation is used unchanged, so a configuration without an admitted
    /// synchronization (no router, no catalogue, disabled synchronization, no admitted
    /// route) starts nothing and acquires no slot.
    ///
    /// # Errors
    ///
    /// Returns the caller's own error family when a route cannot be admitted or mounted, or
    /// when a synchronization slot cannot be acquired, so the started work belongs to the
    /// application-lifetime scope it runs in.
    pub fn register_and_start_routes<X>(
        &self,
        owner: &Scope<X>,
        router: Option<Router<E, R>>,
        mounter: Option<&ManagedRouteMounter<E, R>>,
    ) -> Effect<(RegisteredRoutes<E, R>, Option<Fiber<(), X>>), X, R>
    where
        E: fmt::Display + Send + 'static,
        X: From<ManagedStartError<E>> + Send + Sync + 'static,
        R: HasClock + HasConsole + HasTaskSpawner + Send + Sync + 'static,
    {
        let service = self.clone();
        let owner = owner.clone();
        let router = router.clone();
        let mounter = mounter.cloned();
        effect! {
            let service = service.clone();
            let slots = Arc::clone(&service.slots);
            let owner = owner.clone();
            let router = router.clone();
            let mounter = mounter.clone();
            let mut current = router;
            let mut registrations = Vec::new();
            let mut children: Vec<Effect<(), X, R>> = Vec::new();
            if current.is_some() && service.prepared.is_some() {
                let count = service.routes().len();
                let reserved = registrations.try_reserve_exact(count)
                    .and_then(|()| children.try_reserve_exact(count))
                    .map_err(|_| registration_memory(MemoryError::CollectionReservationFailed));
                effect_try_sync! {
                    once;
                    body: move || reserved,
                    on_error: |error| X::from(ManagedStartError::Registration(error)),
                }?;
            }
            for route in service.routes() {
                let route = route.clone();
                let access = service.options.access.clone();
                let preflight_route = route.clone();
                let has_router = current.is_some();
                let sync = effect_try_sync! {
                    once;
                    body: move || {
                        if !has_router || preflight_route.name.trim().is_empty()
                            || preflight_route.pattern.trim().is_empty() {
                            return Ok(None);
                        }
                        let Some(permissions) = access.checked_permissions(&preflight_route)? else {
                            return Ok(None);
                        };
                        Ok::<_, ManagedRouteError>(access.metadata_sync(&preflight_route, &permissions))
                    },
                    on_error: |error| X::from(ManagedStartError::Registration(error)),
                }?;
                if let Some(sync) = sync {
                    let child = owner.fork(sync_child::<E, X, R>(Arc::clone(&slots), sync))?;
                    children.push(child.join::<R>());
                }
                let access = service.options.access.clone();
                let mounter = mounter.clone();
                let mount_router = current.take();
                let (next, registration) = effect_try_sync! {
                    once;
                    body: move || register(mount_router, &access, mounter.as_ref(), &route),
                    on_error: |error| X::from(ManagedStartError::Registration(error)),
                }?;
                current = next;
                if let Some(registration) = registration { registrations.push(registration); }
            }
            let prepared = RegisteredRoutes { router: current, routes: registrations };
            let started = if children.is_empty() {
                None
            } else {
                let workers = NonZeroUsize::new(children.len()).unwrap_or(NonZeroUsize::MIN);
                let batch = effect! {
                    let children = children.clone();
                    let completed = effect_all_concurrent! {
                        items: children,
                        concurrency: workers,
                    }?;
                    drop(completed);
                    ()
                };
                effect_map! {
                    program: owner.fork(batch),
                    map: |fiber| { Some(fiber) },
                }?
            };
            (prepared, started)
        }
    }
}

/// Admit, protect and mount one managed route.
///
/// A blank name or pattern, or a route without a handler, is skipped. An absent router is
/// a no-op before any protection runs. The first value is the router to continue with
/// (absent only when no router was supplied) and the second is the admitted route, when
/// this route produced one.
///
/// # Errors
///
/// Returns [`ManagedRouteError`] when the route cannot be protected, no mounter is
/// available for an admitted route, or the server refuses the mounted route.
pub fn register<E, R>(
    router: Option<Router<E, R>>,
    access: &AccessControl<E, R>,
    mounter: Option<&ManagedRouteMounter<E, R>>,
    route: &ManagedRoute<E, R>,
) -> Result<(Option<Router<E, R>>, Option<ManagedRouteRegistration<E, R>>), ManagedRouteError>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let Some(router) = router else {
        return Ok((None, None));
    };
    if route.name.trim().is_empty() || route.pattern.trim().is_empty() {
        return Ok((Some(router), None));
    }
    let Some(registration) = access.protect(route)? else {
        return Ok((Some(router), None));
    };
    let Some(mounter) = mounter else {
        return Err(ManagedRouteError::MissingMounter);
    };
    let router = mounter(router, &registration).map_err(ManagedRouteError::Server)?;
    Ok((Some(router), Some(registration)))
}

impl<E: Send + 'static, R: Send + Sync + 'static> AccessControl<E, R> {
    /// The configured metadata-synchronization attempt count.
    #[must_use]
    pub fn sync_attempts(&self) -> usize {
        if self.async_sync_attempts <= 0 {
            return DEFAULT_ASYNC_SYNC_ATTEMPTS;
        }
        usize::try_from(self.async_sync_attempts).unwrap_or(DEFAULT_ASYNC_SYNC_ATTEMPTS)
    }

    /// The configured delay between metadata-synchronization attempts.
    #[must_use]
    pub fn sync_retry_delay(&self) -> Duration {
        if self.async_sync_retry_delay.is_zero() {
            DEFAULT_ASYNC_SYNC_RETRY_DELAY
        } else {
            self.async_sync_retry_delay
        }
    }

    /// Protect one route and prepare its metadata synchronization.
    ///
    /// A route without a handler is not protected and produces no registration.
    ///
    /// # Errors
    ///
    /// Returns [`AccessControlError::MissingPermissions`] when the normalized permission
    /// list is empty, and [`AccessControlError::MissingMiddleware`] when no permission
    /// middleware is configured.
    pub fn protect(
        &self,
        route: &ManagedRoute<E, R>,
    ) -> Result<Option<ManagedRouteRegistration<E, R>>, AccessControlError> {
        let Some(handler) = route.handler.as_ref() else {
            return Ok(None);
        };
        let Some(permissions) = self.checked_permissions(route)? else { return Ok(None); };
        let middleware = self.middleware.as_ref().expect("permission preflight checked middleware");
        let metadata_sync = self.metadata_sync(route, permissions.as_slice());
        let guard = if route.permission_mode.as_str() == PERMISSION_MODE_ALL {
            middleware.require_all(permissions.as_slice())
        } else {
            middleware.require_any(permissions.as_slice())
        };
        let protected = guard(Arc::clone(handler));
        Ok(Some(ManagedRouteRegistration {
            name: route.name.clone(),
            method: route_method(route.method.as_str()),
            pattern: route.pattern.clone(),
            permissions: route.permissions.clone(),
            permission_mode: route.permission_mode.clone(),
            display_name: route_display_name(route),
            description: route.description.clone(),
            category: route_category(route),
            handler: protected,
            metadata_sync,
        }))
    }

    /// Validate protection prerequisites before starting any synchronization.
    fn checked_permissions(&self, route: &ManagedRoute<E, R>) -> Result<Option<Vec<String>>, AccessControlError> {
        if route.handler.is_none() { return Ok(None); }
        let permissions = route_permissions(route);
        if permissions.is_empty() {
            return Err(AccessControlError::MissingPermissions { name: route.name.clone() });
        }
        if self.middleware.is_none() {
            return Err(AccessControlError::MissingMiddleware { name: route.name.clone() });
        }
        Ok(Some(permissions))
    }

    /// Prepare the metadata synchronization for one route.
    ///
    /// An absent catalogue or disabled synchronization prepares nothing.
    #[must_use]
    pub fn metadata_sync(
        &self,
        route: &ManagedRoute<E, R>,
        permissions: &[String],
    ) -> Option<MetadataSync<E, R>> {
        if self.catalogue.is_none() || self.disable_metadata_sync {
            return None;
        }
        let program = self.sync_metadata(route, permissions);
        Some(MetadataSync::new(
            route.name.clone(),
            route_method(route.method.as_str()),
            route.pattern.trim().to_owned(),
            Arc::new(move || program.clone()),
            self.sync_attempts().saturating_sub(1),
            self.sync_retry_delay(),
        ))
    }

    /// Synchronize one route's permission metadata once.
    ///
    /// An absent catalogue or disabled synchronization performs no work.
    #[must_use]
    pub fn sync_metadata(
        &self,
        route: &ManagedRoute<E, R>,
        permissions: &[String],
    ) -> Effect<(), MetadataSyncError<E>, R> {
        let Some(catalogue) = self.catalogue.clone() else {
            return effect_sync! { body: || () };
        };
        if self.disable_metadata_sync {
            return effect_sync! { body: || () };
        }
        let name = route.name.clone();
        let method = route_method(route.method.as_str());
        let pattern = route.pattern.trim().to_owned();
        let display_name = route_display_name(route);
        let description = route.description.trim().to_owned();
        let category = route_category(route);
        let names = normalize_permissions(permissions);
        effect! {
            let catalogue = catalogue.clone();
            let method = method.clone();
            let pattern = pattern.clone();
            let display_name = display_name.clone();
            let description = description.clone();
            let category = category.clone();
            let names = names.clone();
            let check_name = name.clone();
            let check_pattern = pattern.clone();
            let check_names = names.clone();
            effect_try_sync! {
                once;
                body: move || {
                    if check_pattern.is_empty() {
                        return Err(MetadataSyncError::EmptyPattern {
                            name: check_name.clone(),
                        });
                    }
                    if check_names.is_empty() {
                        return Err(MetadataSyncError::NoPermissions { name: check_name });
                    }
                    Ok(())
                },
            }?;
            let mut resolved: Vec<RoutePermission> = Vec::with_capacity(names.len());
            for permission_name in names.iter() {
                let lookup = catalogue.clone();
                let requested = permission_name.clone();
                let map_name = requested.clone();
                let found = effect_map_error! {
                    program: (lookup.permission_by_name)(requested.as_str()),
                    map move |source| {
                        MetadataSyncError::ResolvePermission {
                            name: map_name.clone(),
                            source,
                        }
                    },
                }?;
                let missing_name = requested.clone();
                let permission = effect_try_sync! {
                    once;
                    body: move || match found {
                        Some(permission) => Ok(permission),
                        None => Err(MetadataSyncError::PermissionNotFound { name: missing_name }),
                    },
                }?;
                resolved.push(permission);
            }
            let lookup_method = method.clone();
            let lookup_pattern = pattern.clone();
            let existing = effect_map_error! {
                program: (catalogue.route_by_path_and_method)(pattern.as_str(), method.as_str()),
                map move |source| {
                    MetadataSyncError::ResolveRoute {
                        method: lookup_method.clone(),
                        pattern: lookup_pattern.clone(),
                        source,
                    }
                },
            }?;
            let record = match existing {
                Some(record) => record,
                None => {
                    let draft = RouteDraft {
                        pattern: pattern.clone(),
                        method: method.clone(),
                        display_name: display_name.clone(),
                        description: description.clone(),
                        category: category.clone(),
                        is_public: false,
                    };
                    let create_method = method.clone();
                    let create_pattern = pattern.clone();
                    let create_route = catalogue.clone();
                    let create_program = effect_map_error! {
                        program: (create_route.create_route)(&draft),
                        map move |source| {
                            MetadataSyncError::CreateRoute {
                                method: create_method.clone(),
                                pattern: create_pattern.clone(),
                                source,
                            }
                        },
                    };
                    let create_outcome = create_program.either()?;
                    match create_outcome {
                        Ok(record) => record,
                        Err(failure) => {
                            let rechecked = lenient_route_lookup::<E, R>(
                                catalogue.clone(),
                                method.clone(),
                                pattern.clone(),
                                method.clone(),
                                pattern.clone(),
                            )?;
                            effect_try_sync! {
                                once;
                                body: move || match rechecked {
                                    Some(record) => Ok(record),
                                    None => Err(failure),
                                },
                            }?
                        }
                    }
                }
            };
            let requirement_method = method.clone();
            let requirement_pattern = pattern.clone();
            let requirement = effect_map_error! {
                program: (catalogue.route_requirement)(record.id.as_str()),
                map move |source| {
                    MetadataSyncError::ResolveRoutePermissions {
                        method: requirement_method.clone(),
                        pattern: requirement_pattern.clone(),
                        source,
                    }
                },
            }?;
            for permission in resolved {
                if requirement.as_ref().is_some_and(|requirement| {
                    requirement.required_permissions.iter().any(|existing| {
                        !existing.id.trim().is_empty() && existing.id == permission.id
                    })
                }) {
                    continue;
                }
                let add_permission_name = permission.name.clone();
                let add_method = method.clone();
                let add_pattern = pattern.clone();
                let add_catalogue = catalogue.clone();
                let add_route_id = record.id.clone();
                let add_permission_id = permission.id.clone();
                let add_program = effect_map_error! {
                    program: (add_catalogue.add_permission)(
                        add_route_id.as_str(),
                        add_permission_id.as_str(),
                    ),
                    map move |source| {
                        MetadataSyncError::AddPermission {
                            permission: add_permission_name.clone(),
                            method: add_method.clone(),
                            pattern: add_pattern.clone(),
                            source,
                        }
                    },
                };
                let add_outcome = add_program.either()?;
                match add_outcome {
                    Ok(()) => (),
                    Err(failure) => {
                        let required_now = lenient_route_permission::<E, R>(
                            catalogue.clone(),
                            record.id.clone(),
                            permission.id.clone(),
                            permission.name.clone(),
                            method.clone(),
                            pattern.clone(),
                        )?;
                        effect_try_sync! {
                            once;
                            body: move || match required_now {
                                true => Ok(()),
                                false => Err(failure),
                            },
                        }?
                    }
                }
            }
            ()
        }
    }
}

/// A managed route configuration failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccessControlError {
    /// The route declares no permissions.
    MissingPermissions {
        /// The route name.
        name: String,
    },
    /// No permission middleware is configured.
    MissingMiddleware {
        /// The route name.
        name: String,
    },
}

impl fmt::Display for AccessControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPermissions { name } => {
                write!(
                    formatter,
                    "contentmgmt managed route \"{name}\" has no configured permissions"
                )
            }
            Self::MissingMiddleware { name } => {
                write!(
                    formatter,
                    "contentmgmt managed route \"{name}\" has no permission middleware"
                )
            }
        }
    }
}

impl std::error::Error for AccessControlError {}

/// A managed route registration failure.
#[derive(Debug)]
pub enum ManagedRouteError {
    /// The route description was refused by protection.
    Access(AccessControlError),
    /// No mounter was supplied for an admitted route.
    MissingMounter,
    /// The consumer's server refused the mounted route.
    Server(ServerError),
}

fn registration_memory(error: MemoryError) -> ManagedRouteError {
    ManagedRouteError::Server(ServerError::Memory(error))
}

impl fmt::Display for ManagedRouteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Access(error) => fmt::Display::fmt(error, formatter),
            Self::MissingMounter => {
                formatter.write_str("contentmgmt managed route registration has no mounter")
            }
            Self::Server(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for ManagedRouteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Access(error) => Some(error),
            Self::MissingMounter => None,
            Self::Server(error) => Some(error),
        }
    }
}

impl From<AccessControlError> for ManagedRouteError {
    fn from(error: AccessControlError) -> Self {
        Self::Access(error)
    }
}

/// An operational managed-route registration or synchronization-start failure.
///
/// This is the error family of [`Service::register_and_start_routes`]: admission and
/// mounting failures keep their own cause, a synchronization failure keeps the
/// synchronization's own cause, and a slot failure stays an operational failure instead of
/// being reported as a catalogue failure.
#[derive(Debug)]
pub enum ManagedStartError<E> {
    /// A route could not be admitted or mounted.
    Registration(ManagedRouteError),
    /// A synchronization reported an expected failure.
    Sync(MetadataSyncError<E>),
    /// A synchronization slot could not be acquired or released.
    SyncSlots(SemaphoreError),
}

impl<E: fmt::Display> fmt::Display for ManagedStartError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registration(error) => fmt::Display::fmt(error, formatter),
            Self::Sync(error) => fmt::Display::fmt(error, formatter),
            Self::SyncSlots(error) => write!(formatter, "metadata sync slot: {error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for ManagedStartError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Registration(error) => Some(error),
            Self::Sync(error) => Some(error),
            Self::SyncSlots(error) => Some(error),
        }
    }
}

impl<E> From<ManagedRouteError> for ManagedStartError<E> {
    fn from(error: ManagedRouteError) -> Self {
        Self::Registration(error)
    }
}

/// A permission-metadata synchronization failure.
///
/// The consumer's own failure cause is preserved in every variant that a capability
/// reported.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetadataSyncError<E> {
    /// The route declares no pattern to synchronize.
    EmptyPattern {
        /// The route name.
        name: String,
    },
    /// The route declares no permission names to synchronize.
    NoPermissions {
        /// The route name.
        name: String,
    },
    /// A declared permission could not be resolved.
    ResolvePermission {
        /// The permission name.
        name: String,
        /// The catalogue's failure.
        source: E,
    },
    /// A declared permission does not exist.
    PermissionNotFound {
        /// The permission name.
        name: String,
    },
    /// The route metadata could not be resolved.
    ResolveRoute {
        /// The normalized method.
        method: String,
        /// The route pattern.
        pattern: String,
        /// The catalogue's failure.
        source: E,
    },
    /// The route metadata could not be created and could not be reconciled.
    CreateRoute {
        /// The normalized method.
        method: String,
        /// The route pattern.
        pattern: String,
        /// The catalogue's failure.
        source: E,
    },
    /// The route's required permissions could not be resolved.
    ResolveRoutePermissions {
        /// The normalized method.
        method: String,
        /// The route pattern.
        pattern: String,
        /// The catalogue's failure.
        source: E,
    },
    /// A permission association could not be added and its absence was confirmed.
    AddPermission {
        /// The permission name.
        permission: String,
        /// The normalized method.
        method: String,
        /// The route pattern.
        pattern: String,
        /// The catalogue's failure.
        source: E,
    },
}

impl<E> MetadataSyncError<E> {
    /// The consumer's failure cause, when this variant carries one.
    #[must_use]
    pub fn source_error(&self) -> Option<&E> {
        match self {
            Self::ResolvePermission { source, .. }
            | Self::ResolveRoute { source, .. }
            | Self::CreateRoute { source, .. }
            | Self::ResolveRoutePermissions { source, .. }
            | Self::AddPermission { source, .. } => Some(source),
            Self::EmptyPattern { .. }
            | Self::NoPermissions { .. }
            | Self::PermissionNotFound { .. } => None,
        }
    }
}

impl<E: fmt::Display> fmt::Display for MetadataSyncError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPattern { name } => {
                write!(formatter, "managed route \"{name}\" has an empty pattern")
            }
            Self::NoPermissions { name } => {
                write!(
                    formatter,
                    "managed route \"{name}\" has no permission names to sync"
                )
            }
            Self::ResolvePermission { name, source } => {
                write!(formatter, "resolve permission \"{name}\": {source}")
            }
            Self::PermissionNotFound { name } => {
                write!(
                    formatter,
                    "resolve permission \"{name}\": permission not found"
                )
            }
            Self::ResolveRoute {
                method,
                pattern,
                source,
            } => write!(formatter, "resolve route {method} {pattern}: {source}"),
            Self::CreateRoute {
                method,
                pattern,
                source,
            } => write!(formatter, "create route {method} {pattern}: {source}"),
            Self::ResolveRoutePermissions {
                method,
                pattern,
                source,
            } => write!(
                formatter,
                "resolve route permissions for {method} {pattern}: {source}"
            ),
            Self::AddPermission {
                permission,
                method,
                pattern,
                source,
            } => write!(
                formatter,
                "add permission \"{permission}\" to route {method} {pattern}: {source}"
            ),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for MetadataSyncError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source_error().map(|source| source as _)
    }
}

/// Run one pending synchronization under one shared slot.
///
/// The synchronization acquires one slot permit before its first attempt and holds it
/// through the retries, their delays and the terminal failure report. The binding releases
/// the permit through its own finalization, including cancellation and failure.
fn sync_child<E, X, R>(slots: Arc<Semaphore>, sync: MetadataSync<E, R>) -> Effect<(), X, R>
where
    E: fmt::Display + Send + 'static,
    X: From<ManagedStartError<E>> + Send + Sync + 'static,
    R: HasClock + HasConsole + Send + Sync + 'static,
{
    let route_name = sync.route_name().to_owned();
    let method = sync.method().to_owned();
    let pattern = sync.pattern().to_owned();
    let program = sync.program();
    let reported = effect_recover! {
        program: effect_map_error! {
            program: program,
            map: move |error| ManagedStartError::Sync(error),
        },
        recover: move |error| {
            let message = format!(
                "contentmgmt authz metadata sync failed route={route_name} method={method} pattern={pattern} error={error}"
            );
            effect! {
                let message = message.clone();
                Effect::console_write(ConsoleChannel::Error, message)?;
                ()
            }
        },
    };
    effect_map_error! {
        program: effect_permit! {
            semaphore: slots,
            permits: NonZeroUsize::MIN,
            on_error: ManagedStartError::SyncSlots,
            body {
                let reported = reported.clone();
                reported?;
                ()
            },
        },
        map: move |error| X::from(error),
    }
}

/// Look a route up again without letting its own failure escape.
///
/// A reconciliation read reports "not found" for an absent record and for a failed read,
/// because the caller then reports the original write failure.
fn lenient_route_lookup<E, R>(
    catalogue: RouteCatalogue<E, R>,
    method: String,
    pattern: String,
    map_method: String,
    map_pattern: String,
) -> Effect<Option<RouteRecord>, MetadataSyncError<E>, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let lookup = effect_map_error! {
        program: (catalogue.route_by_path_and_method)(pattern.as_str(), method.as_str()),
        map move |source| {
            MetadataSyncError::ResolveRoute {
                method: map_method.clone(),
                pattern: map_pattern.clone(),
                source,
            }
        },
    };
    effect_recover! {
        program: lookup,
        recover: |_lookup| effect_sync! { body: || None },
    }
}

/// Re-read one route's requirement without letting the read failure escape.
///
/// A reconciliation read reports "not required" for a failed read, because the caller then
/// reports the original write failure.
fn lenient_route_permission<E, R>(
    catalogue: RouteCatalogue<E, R>,
    route_id: String,
    permission_id: String,
    permission: String,
    method: String,
    pattern: String,
) -> Effect<bool, MetadataSyncError<E>, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let check = effect_map_error! {
        program: (catalogue.route_has_permission)(route_id.as_str(), permission_id.as_str()),
        map move |source| {
            MetadataSyncError::AddPermission {
                permission: permission.clone(),
                method: method.clone(),
                pattern: pattern.clone(),
                source,
            }
        },
    };
    effect_recover! {
        program: check,
        recover: |_lookup| effect_sync! { body: || false },
    }
}

/// Remove the named routes from a route list.
///
/// Names are trimmed; blank names are ignored. An empty route list or an empty name list
/// returns the route list unchanged, and the order of the retained routes is preserved.
#[must_use]
pub fn without_names<E: Send + 'static, R: Send + Sync + 'static>(
    routes: &[ManagedRoute<E, R>],
    names: &[String],
) -> Vec<ManagedRoute<E, R>> {
    if routes.is_empty() || names.is_empty() {
        return routes.to_vec();
    }
    let mut excluded: BTreeSet<&str> = BTreeSet::new();
    for name in names {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        excluded.insert(name);
    }
    if excluded.is_empty() {
        return routes.to_vec();
    }
    routes
        .iter()
        .filter(|route| !excluded.contains(route.name.trim()))
        .cloned()
        .collect()
}

/// The normalized permissions of a route.
#[must_use]
pub fn route_permissions<E: Send + 'static, R: Send + Sync + 'static>(
    route: &ManagedRoute<E, R>,
) -> Vec<String> {
    normalize_permissions(route.permissions.as_slice())
}

/// Trim and deduplicate permission names, preserving declaration order.
#[must_use]
pub fn normalize_permissions(permissions: &[String]) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut normalized = Vec::with_capacity(permissions.len());
    for permission in permissions {
        let permission = permission.trim().to_owned();
        if permission.is_empty() {
            continue;
        }
        if seen.insert(permission.clone()) {
            normalized.push(permission);
        }
    }
    normalized
}

/// Normalize a route method.
///
/// The value is trimmed and upper-cased; an empty value becomes the read method.
#[must_use]
pub fn route_method(method: &str) -> String {
    let method: String = method.trim().chars().map(|ch| {
        // Simple uppercase keeps one scalar; full uppercase expands these Greek
        // forms instead of retaining their capital prosgegrammeni counterpart.
        match ch {
            '\u{1f80}'..='\u{1f87}' | '\u{1f90}'..='\u{1f97}' | '\u{1fa0}'..='\u{1fa7}' =>
                char::from_u32(ch as u32 + 8).expect("Greek uppercase scalar"),
            '\u{1fb3}' | '\u{1fc3}' | '\u{1ff3}' =>
                char::from_u32(ch as u32 + 9).expect("Greek uppercase scalar"),
            _ => {
                let mut upper = ch.to_uppercase();
                let first = upper.next().unwrap_or(ch);
                if upper.next().is_none() { first } else { ch }
            }
        }
    }).collect();
    if method.is_empty() {
        return Method::GET.as_str().to_owned();
    }
    method
}

/// Resolve a route's display name.
///
/// The declared display name wins, then the route name, then the normalized method and
/// trimmed pattern.
#[must_use]
pub fn route_display_name<E: Send + 'static, R: Send + Sync + 'static>(
    route: &ManagedRoute<E, R>,
) -> String {
    let display_name = route.display_name.trim();
    if !display_name.is_empty() {
        return display_name.to_owned();
    }
    let name = route.name.trim();
    if !name.is_empty() {
        return name.to_owned();
    }
    format!(
        "{} {}",
        route_method(route.method.as_str()),
        route.pattern.trim()
    )
}

/// Resolve a route's category.
#[must_use]
pub fn route_category<E: Send + 'static, R: Send + Sync + 'static>(
    route: &ManagedRoute<E, R>,
) -> String {
    let category = route.category.trim();
    if !category.is_empty() {
        return category.to_owned();
    }
    DEFAULT_ROUTE_CATEGORY.to_owned()
}
