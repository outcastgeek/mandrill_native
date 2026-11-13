use std::{io,sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering}}};
use effect::{effect_sync,Exit};
use effect_cms::{*,definitions::*,queries,routing,validation,commands,seo};
use super::support::*;

#[test]
fn factory_constructs_reusable_real_services_and_full_runtime()->Result<(),String>{
    let calls=Arc::new(Mutex::new(Vec::new()));
    let mut options=DefinitionRuntimeFactoryOptions::<io::Error,()>::new(policy(),Arc::new(|_|effect_sync!{body:||None}));
    options.definitions=vec![definition()];options.default_page_size=9;
    options.request_language=Some(Arc::new(|_|"fr".into()));
    options.translate=Some(Arc::new(|_,key|format!("translated:{key}")));
    options.management_prefix="/manage".into();
    options.route_to_path=Some(Arc::new(|name,_|Some(format!("/{name}"))));
    options.prefix_path=Some(Arc::new(|language,path|format!("/{language}{path}")));
    let observed=calls.clone();options.validators.list_page=Some(Arc::new(move|page|{observed.lock().map_err(|_|io::Error::other("lock"))?.push("list");page.subtitle="validated list".into();Ok(())}));
    let observed=calls.clone();options.validators.dashboard_page=Some(Arc::new(move|page|{observed.lock().map_err(|_|io::Error::other("lock"))?.push("dashboard");page.subtitle="validated dashboard".into();Ok(())}));
    let factory=new_definition_runtime_factory(options);
    let request=request("/manage/articles?search=words&page=2")?;let kind=ResourceKind::new("article");
    assert_eq!((factory.route_path)(&request,"articles",&[]),"/fr/articles");assert!((factory.is_management)(&request));
    assert_eq!((factory.translate_key)(&request,"articles.title"),"translated:articles.title");
    let services=factory.new_service_set(None,Default::default(),Default::default());
    let query=run(services.queries.list_query::<io::Error,()>(&request,&kind))?;
    assert_eq!(query.search,"words");
    let runtime=factory.new_runtime(services.clone());
    let callbacks=runtime.frame_builder_options().rendering.ok_or("missing full rendering")?;
    let list=callbacks.list_detail.list_page.ok_or("missing list callback")?;
    assert!(calls.lock().map_err(|_|"lock")?.is_empty());
    for _ in 0..2{let page=run(list(&request,&kind,query.clone()))?;assert_eq!(page.subtitle,"validated list");}
    let dashboard=run((callbacks.dashboard_page)(&request,&kind,query))?;assert_eq!(dashboard.subtitle,"validated dashboard");
    assert_eq!(*calls.lock().map_err(|_|"lock")?,vec!["list","list","dashboard"]);
    let page_options=runtime.resource_page_builder_options();assert!(page_options.list_seo.is_some());assert!(page_options.detail_seo.is_some());assert!(page_options.pages.dashboards.is_some());
    let deps:Dependencies<io::Error,io::Error,io::Error,io::Error,()>=services.dependencies(None,None);
    let platform=compose_platform(deps);
    assert!(Arc::ptr_eq(platform.resources().ok_or("resources")?,&services.resources));
    assert!(platform.queries().is_some()&&platform.validation().is_some()&&platform.commands().is_some()&&platform.forms().is_some()&&platform.rendering().is_some()&&platform.dashboards().is_some()&&platform.seo().is_some());
    assert!(platform.routes().is_none()&&platform.authoring().is_none()&&platform.media_assets().is_none());
    Ok(())
}

struct ValidationBinder(Arc<AtomicUsize>);
impl validation::Binder<commands::CommandInput,io::Error,()> for ValidationBinder{fn bind_resource_validation(&self,_:&mut validation::Bindings<commands::CommandInput,io::Error,()>){self.0.fetch_add(1,Ordering::SeqCst);}}
struct CommandBinder(Arc<AtomicUsize>);
impl commands::ResourceCommandBinder<io::Error,()> for CommandBinder{fn bind_resource_commands(&self,_:&commands::ResourceCommandBindings<io::Error,()>){self.0.fetch_add(1,Ordering::SeqCst);}}
#[test]
fn existing_service_resolution_does_not_collect_binders()->Result<(),String>{
    let mut options=DefinitionServiceSetOptions::<io::Error,()>::new(policy());options.definitions=vec![definition()];
    let services=new_definition_service_set(options.clone());let count=Arc::new(AtomicUsize::new(0));
    let validator=ValidationBinder(count.clone());let commander=CommandBinder(count.clone());
    let _=resolve_definition_validation_service(Some(services.validation),options.clone(),&[&validator]);
    let _=resolve_definition_command_service(Some(services.commands),options,&[&commander]);assert_eq!(count.load(Ordering::SeqCst),0);
    let supplied=queries::CanonicalRouteParams{id_param:"custom".into(),..Default::default()};let resolved=defaulted_canonical_route_params(supplied);assert_eq!(resolved.id_param,"custom");assert_eq!(resolved.slug_param,routing::default_canonical_route_params().slug_param);Ok(())
}

#[test]
fn validators_publish_mutations_and_preserve_original_failure()->Result<(),String>{
    let request=request("/")?;let registry=Arc::new(new_definition_registry([definition()]));
    let count=Arc::new(AtomicUsize::new(0));let observed=count.clone();
    let service=seo::Service::<io::Error>::new(registry.clone(),seo::Options{validate_seo:Some(Arc::new(move|page|{observed.fetch_add(1,Ordering::SeqCst);page.title="normalized".into();Ok(())})),..Default::default()});
    let effect=service.page::<()>(&request,&ResourceKind::new("article"),&ResourceViewMode::new(RESOURCE_VIEW_MODE_LIST));
    assert_eq!(count.load(Ordering::SeqCst),0);assert_eq!(run(effect)?.title,"normalized");assert_eq!(count.load(Ordering::SeqCst),1);
    let service=seo::Service::<io::Error>::new(registry,seo::Options{validate_seo:Some(Arc::new(|_|Err(io::Error::new(io::ErrorKind::PermissionDenied,"sentinel")))),..Default::default()});
    let exit=futures_executor::block_on(service.page::<()>(&request,&ResourceKind::new("article"),&ResourceViewMode::new("list")).run(Arc::new(())));
    match exit{Exit::Fail(seo::SEOError::Validation(error))=>{assert_eq!(error.kind(),io::ErrorKind::PermissionDenied);assert_eq!(error.to_string(),"sentinel");},_=>return Err("validator failure was replaced".into())};
    Ok(())
}
