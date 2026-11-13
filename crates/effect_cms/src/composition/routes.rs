//! Ordered registrar composition using the application's existing native scope.
use std::{fmt,sync::Arc};
use effect::{Effect,Scope,Fiber,HasClock,HasConsole,HasTaskSpawner,effect,effect_map};
use effect_server::Router;
use crate::routing::{RegisteredRoutes,ManagedRouteMounter,ManagedStartError};
pub struct RouteRegistrationBatch<E:Send+'static,X:Send+'static,R:Send+Sync+'static>{pub registered:RegisteredRoutes<E,R>,pub synchronizations:Vec<Fiber<(),X>>}
pub type RouteRegistrar<E,X,R>=Arc<dyn Fn(&Scope<X>,Option<Router<E,R>>,Option<&ManagedRouteMounter<E,R>>)->Effect<RouteRegistrationBatch<E,X,R>,X,R>+Send+Sync+'static>;
pub struct RouteService<E:Send+'static,X:Send+'static,R:Send+Sync+'static>{registrars:Vec<Option<RouteRegistrar<E,X,R>>>}
impl<E:Send+'static,X:Send+'static,R:Send+Sync+'static> Clone for RouteService<E,X,R>{fn clone(&self)->Self{Self{registrars:self.registrars.clone()}}}
pub fn new_route_service<E:Send+'static,X:Send+'static,R:Send+Sync+'static>(registrars:Vec<Option<RouteRegistrar<E,X,R>>>)->RouteService<E,X,R>{RouteService{registrars}}
impl<E:Send+'static,X:Send+'static,R:Send+Sync+'static> RouteService<E,X,R>{
    pub fn register_routes(&self,owner:&Scope<X>,router:Option<Router<E,R>>,mounter:Option<&ManagedRouteMounter<E,R>>)->Effect<RouteRegistrationBatch<E,X,R>,X,R>{
        let owner=owner.clone();let registrars=self.registrars.clone();let mounter=mounter.cloned();
        effect!{
            let owner=owner.clone();let registrars=registrars.clone();let mounter=mounter.clone();let mut current=router.clone();let mut routes=Vec::new();let mut synchronizations=Vec::new();
            if current.is_some(){for registrar in registrars.into_iter().flatten(){let batch=registrar(&owner,current.take(),mounter.as_ref())?;current=batch.registered.router;routes.extend(batch.registered.routes);synchronizations.extend(batch.synchronizations);}}
            RouteRegistrationBatch{registered:RegisteredRoutes{router:current,routes},synchronizations}
        }
    }
}
/// Adapt the canonical managed route service without deferring or dropping startup work.
pub fn managed_route_registrar<E,X,R>(service:crate::routing::Service<E,R>)->RouteRegistrar<E,X,R>
where E:fmt::Display+Send+'static,X:From<ManagedStartError<E>>+Send+Sync+'static,R:HasClock+HasConsole+HasTaskSpawner+Send+Sync+'static{
    Arc::new(move |owner,router,mounter|{effect_map!{program:service.register_and_start_routes(owner,router,mounter),map:|(registered,sync)|RouteRegistrationBatch{registered,synchronizations:sync.into_iter().collect()}}})
}
