//! The authoring managed route.
//!
//! The source registered one `POST` authoring route with its name, path,
//! permissions and the `authoring` category through the shared routing owner. An
//! absent authoring service, name or path performs no registration.
//!
//! The Rust contract delegates to the routing owner's typed registration: the
//! consumer supplies the router, the mounter and the access control records, and
//! this owner supplies only the managed-route description and handler.

use std::sync::Arc;

use effect_server::Router;

use super::contracts::AuthoringHandlerError;
use super::runtime::AuthoringService;
use crate::routing::{
    AccessControl, ManagedRoute, ManagedRouteError, ManagedRouteMounter,
    ManagedRouteRegistration, register,
};

/// The managed-route category used by the source authoring route.
pub const AUTHORING_ROUTE_CATEGORY: &str = "authoring";

/// Configures the authoring route service.
pub struct AuthoringRouteServiceOptions<A, T, R>
where
    A: Send + 'static,
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The route name.
    pub route_name: String,
    /// The route path.
    pub path: String,
    /// The authoring service, or absent for a declaration without a handler.
    pub authoring: Option<Arc<dyn AuthoringService<A, T, R>>>,
    /// The managed access control.
    pub access: AccessControl<AuthoringHandlerError<A, T>, R>,
    /// The declared permissions.
    pub permissions: Vec<String>,
}

impl<A, T, R> Clone for AuthoringRouteServiceOptions<A, T, R>
where
    A: Send + 'static,
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            route_name: self.route_name.clone(),
            path: self.path.clone(),
            authoring: self.authoring.clone(),
            access: self.access.clone(),
            permissions: self.permissions.clone(),
        }
    }
}

/// The authoring route service.
pub struct AuthoringRouteService<A, T, R>
where
    A: Send + 'static,
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    options: AuthoringRouteServiceOptions<A, T, R>,
    routes: crate::routing::Service<AuthoringHandlerError<A,T>,R>,
}

impl<A, T, R> Clone for AuthoringRouteService<A, T, R>
where
    A: Send + 'static,
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            options: self.options.clone(),
            routes: self.routes.clone(),
        }
    }
}

impl<A, T, R> AuthoringRouteService<A, T, R>
where
    A: Send + 'static,
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Construct the route service from its complete configuration.
    #[must_use]
    pub fn new(options: AuthoringRouteServiceOptions<A, T, R>) -> Self {
        let routes=crate::routing::Service::new(crate::routing::Options { access:options.access.clone(), routes:Vec::new() });
        Self { options, routes }
    }

    /// Register and start metadata synchronization under the application's existing scope.
    pub fn register_and_start_routes<X>(
        &self, owner:&effect::Scope<X>,
        router:Option<Router<AuthoringHandlerError<A,T>,R>>,
        mounter:Option<&ManagedRouteMounter<AuthoringHandlerError<A,T>,R>>,
    )->effect::Effect<(crate::routing::RegisteredRoutes<AuthoringHandlerError<A,T>,R>,Option<effect::Fiber<(),X>>),X,R>
    where A:std::fmt::Display,T:std::fmt::Display,
        X:From<crate::routing::ManagedStartError<AuthoringHandlerError<A,T>>>+Send+Sync+'static,
        R:effect::HasClock+effect::HasConsole+effect::HasTaskSpawner,
    {
        let service=self.clone();let owner=owner.clone();let mounter=mounter.cloned();let has_router=router.is_some();
        effect::effect! {
            bind;
            program:effect::effect_try_sync! { once; body:move || {
                let mut routes=Vec::new();
                if has_router && !service.options.route_name.is_empty() && !service.options.path.is_empty() {
                    if let Some(authoring)=&service.options.authoring {
                        let mut route=ManagedRoute::new(&service.options.route_name,"POST",&service.options.path,authoring.translate_localized_content());
                        route.permissions=service.options.permissions.clone();route.category=AUTHORING_ROUTE_CATEGORY.to_owned();routes.push(route);
                    }
                }
                service.routes.with_routes(routes)
            }, on_error: |error| X::from(crate::routing::ManagedStartError::Registration(error)), },
            body:move |routes|routes.register_and_start_routes(&owner,router.clone(),mounter.as_ref()),
        }
    }

    /// Admit, protect and mount the authoring route.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedRouteError`] when the route cannot be protected or mounted.
    pub fn register_routes(
        &self,
        router: Option<Router<AuthoringHandlerError<A, T>, R>>,
        mounter: Option<&ManagedRouteMounter<AuthoringHandlerError<A, T>, R>>,
    ) -> Result<
        (
            Option<Router<AuthoringHandlerError<A, T>, R>>,
            Option<ManagedRouteRegistration<AuthoringHandlerError<A, T>, R>>,
        ),
        ManagedRouteError,
    > {
        if router.is_none() || self.options.route_name.is_empty() || self.options.path.is_empty() {
            return Ok((router, None));
        }
        let Some(authoring) = self.options.authoring.as_ref() else {
            return Ok((router, None));
        };
        let handler = authoring.translate_localized_content();
        let mut route = ManagedRoute::new(
            self.options.route_name.as_str(),
            "POST",
            self.options.path.as_str(),
            handler,
        );
        route.permissions = self.options.permissions.clone();
        route.category = AUTHORING_ROUTE_CATEGORY.to_owned();
        register(router, &self.options.access, mounter, &route)
    }
}
