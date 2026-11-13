//! Eleven canonical platform capabilities; optional services retain source nil semantics.
use std::sync::Arc;
use crate::{authoring,commands,dashboards,media,queries,seo,validation};
use super::*;
/// Application-supplied routes, authoring and media remain their canonical owner types.
/// Distinct authoring/provider and route-start errors are retained without flattening exits.
pub struct Dependencies<E:Send+'static,A:Send+'static,T:Send+'static,X:Send+'static,R:Send+Sync+'static>{
    pub routes:Option<RouteService<E,X,R>>,pub resources:Option<ResourceRegistry>,pub queries:Option<queries::QueryService>,
    pub validation:Option<ValidationService<E,R>>,pub forms:Option<FormService<E,R>>,pub rendering:Option<RenderingService<E>>,
    pub commands:Option<commands::CommandService<E,R>>,pub dashboards:Option<dashboards::DashboardService<E>>,pub seo:Option<seo::Service<E>>,
    pub authoring:Option<Arc<dyn authoring::AuthoringService<A,T,R>>>,pub media_assets:Option<Arc<dyn media::runtime::MediaAssetService<E,R>+Send+Sync>>,
}
impl<E:Send+'static,A:Send+'static,T:Send+'static,X:Send+'static,R:Send+Sync+'static> Default for Dependencies<E,A,T,X,R>{fn default()->Self{Self{routes:None,resources:None,queries:None,validation:None,forms:None,rendering:None,commands:None,dashboards:None,seo:None,authoring:None,media_assets:None}}}
impl<E:Send+'static,A:Send+'static,T:Send+'static,X:Send+'static,R:Send+Sync+'static> Clone for Dependencies<E,A,T,X,R>{fn clone(&self)->Self{Self{routes:self.routes.clone(),resources:self.resources.clone(),queries:self.queries.clone(),validation:self.validation.clone(),forms:self.forms.clone(),rendering:self.rendering.clone(),commands:self.commands.clone(),dashboards:self.dashboards.clone(),seo:self.seo.clone(),authoring:self.authoring.clone(),media_assets:self.media_assets.clone()}}}
pub struct Platform<E:Send+'static,A:Send+'static,T:Send+'static,X:Send+'static,R:Send+Sync+'static>{dependencies:Dependencies<E,A,T,X,R>}
impl<E:Send+'static,A:Send+'static,T:Send+'static,X:Send+'static,R:Send+Sync+'static> Clone for Platform<E,A,T,X,R>{fn clone(&self)->Self{Self{dependencies:self.dependencies.clone()}}}
pub fn compose_platform<E:Send+'static,A:Send+'static,T:Send+'static,X:Send+'static,R:Send+Sync+'static>(dependencies:Dependencies<E,A,T,X,R>)->Platform<E,A,T,X,R>{Platform{dependencies}}
impl<E:Send+'static,A:Send+'static,T:Send+'static,X:Send+'static,R:Send+Sync+'static> Platform<E,A,T,X,R>{
    pub fn routes(&self)->Option<&RouteService<E,X,R>>{self.dependencies.routes.as_ref()}
    pub fn resources(&self)->Option<&ResourceRegistry>{self.dependencies.resources.as_ref()}
    pub fn queries(&self)->Option<&queries::QueryService>{self.dependencies.queries.as_ref()}
    pub fn validation(&self)->Option<&ValidationService<E,R>>{self.dependencies.validation.as_ref()}
    pub fn forms(&self)->Option<&FormService<E,R>>{self.dependencies.forms.as_ref()}
    pub fn rendering(&self)->Option<&RenderingService<E>>{self.dependencies.rendering.as_ref()}
    pub fn commands(&self)->Option<&commands::CommandService<E,R>>{self.dependencies.commands.as_ref()}
    pub fn dashboards(&self)->Option<&dashboards::DashboardService<E>>{self.dependencies.dashboards.as_ref()}
    pub fn seo(&self)->Option<&seo::Service<E>>{self.dependencies.seo.as_ref()}
    pub fn authoring(&self)->Option<&Arc<dyn authoring::AuthoringService<A,T,R>>>{self.dependencies.authoring.as_ref()}
    pub fn media_assets(&self)->Option<&Arc<dyn media::runtime::MediaAssetService<E,R>+Send+Sync>>{self.dependencies.media_assets.as_ref()}
}
impl<E:Send+'static,R:Send+Sync+'static> DefinitionServiceSet<E,R>{
    /// Like the source service set, this supplies eight definition services and the two
    /// explicit external services. `media_assets` remains available for caller assignment.
    pub fn dependencies<A:Send+'static,T:Send+'static,X:Send+'static>(&self,routes:Option<RouteService<E,X,R>>,authoring:Option<Arc<dyn authoring::AuthoringService<A,T,R>>>)->Dependencies<E,A,T,X,R>{
        Dependencies{routes,resources:Some(self.resources.clone()),queries:Some(self.queries.clone()),validation:Some(self.validation.clone()),forms:Some(self.forms.clone()),rendering:Some(self.rendering.clone()),commands:Some(self.commands.clone()),dashboards:Some(self.dashboards.clone()),seo:Some(self.seo.clone()),authoring,media_assets:None}
    }
}
/// Adapt the existing media service to both canonical binder traits. These bind into
/// the very same live maps consumed by validation and commands; no execution is copied.
pub struct MediaAssetServiceBinder<E:Send+'static,R:Send+Sync+'static>{service:Arc<dyn media::runtime::MediaAssetService<E,R>+Send+Sync>}
impl<E:Send+'static,R:Send+Sync+'static> MediaAssetServiceBinder<E,R>{pub fn new(service:Arc<dyn media::runtime::MediaAssetService<E,R>+Send+Sync>)->Self{Self{service}}}
impl<E:Send+'static,R:Send+Sync+'static> commands::ResourceCommandBinder<E,R> for MediaAssetServiceBinder<E,R>{fn bind_resource_commands(&self,bindings:&commands::ResourceCommandBindings<E,R>){self.service.bind_resource_commands(bindings)}}
impl<E:Send+'static,R:Send+Sync+'static> validation::Binder<commands::CommandInput,E,R> for MediaAssetServiceBinder<E,R>{fn bind_resource_validation(&self,bindings:&mut validation::Bindings<commands::CommandInput,E,R>){self.service.bind_resource_validation(bindings)}}
