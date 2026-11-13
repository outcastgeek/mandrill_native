use std::{num::NonZeroUsize,sync::{Arc,Mutex},time::Duration};
use effect::{Exit,TestClock,effect_scope,effect_sync,effect_try_sync};
use effect_cms::{*,routing::{self,*}};
use super::support::*;
#[test]
fn root_registrars_are_lazy_ordered_skip_missing_router_and_stop_on_failure()->Result<(),String>{
    let calls=Arc::new(Mutex::new(Vec::new()));let mut registrars=Vec::new();
    for name in ["first","second","last"]{let calls=calls.clone();let registrar:RouteRegistrar<String,String,()>=Arc::new(move|_,router,_|{let calls=calls.clone();effect_try_sync!{once;body:move||{calls.lock().map_err(|_|"lock".to_owned())?.push(name);if name=="second"{Err("registration failed".into())}else{Ok(RouteRegistrationBatch{registered:RegisteredRoutes{router,routes:Vec::new()},synchronizations:Vec::new()})}}}});registrars.push(Some(registrar));}
    registrars.insert(1,None);let routes=new_route_service(registrars);let absent=routes.clone();
    let program=effect_scope!{body:move |scope| absent.register_routes(&scope,None,None)};let batch=run(program)?;assert!(batch.registered.router.is_none());assert!(calls.lock().map_err(|_|"lock")?.is_empty());
    let router:effect_server::Router<String,()>=effect_server::router!{}.map_err(|e|e.to_string())?;
    let program=effect_scope!{body:move |scope| routes.register_routes(&scope,Some(router.clone()),None)};
    assert!(calls.lock().map_err(|_|"lock")?.is_empty());let exit=futures_executor::block_on(program.run(Arc::new(())));assert!(matches!(exit,Exit::Fail(error)if error=="registration failed"));assert_eq!(*calls.lock().map_err(|_|"lock")?,vec!["first","second"]);Ok(())
}

#[test]
fn permission_any_all_normalize_and_wrap_actual_handler()->Result<(),String>{
    let observed=Arc::new(Mutex::new(Vec::new()));
    let requirement=|mode:&'static str|{let observed=observed.clone();Arc::new(move|permissions:&[String]|{observed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((mode,permissions.to_vec()));Arc::new(move|handler:RouteHandler<String,()>|handler)as PermissionGuard<String,()>})as PermissionRequirementBuilder<String,()>};
    let access=AccessControl{middleware:Some(PermissionMiddleware::new(requirement("any"),requirement("all"))),disable_metadata_sync:true,..Default::default()};
    let handler:RouteHandler<String,()>=Arc::new(|_|effect_sync!{body:||effect_server::reply!{text("protected".to_owned())}});
    let mut route=ManagedRoute::new("articles"," get ","/articles",handler);route.permissions=vec![" edit ".into(),"".into(),"edit".into(),"view".into()];
    let registration=access.protect(&route).map_err(|e|e.to_string())?.ok_or("registration")?;
    let request=request("/")?;let reply=run((registration.handler)(&request))?;let finite=reply.into_finite(NonZeroUsize::MIN.saturating_add(1023)).map_err(|e|e.to_string())?;assert_eq!(finite.bytes.as_ref(),b"protected");
    route.permission_mode=PermissionMode::new(PERMISSION_MODE_ALL);let _=access.protect(&route).map_err(|e|e.to_string())?;
    assert_eq!(*observed.lock().map_err(|_|"lock")?,vec![("any",vec!["edit".to_owned(),"view".to_owned()]),("all",vec!["edit".to_owned(),"view".to_owned()])]);Ok(())
}

#[test]
fn route_metadata_reconciles_create_conflict_and_does_not_duplicate_existing_permission()->Result<(),String>{
    type R=TestClock;let calls=Arc::new(Mutex::new(Vec::new()));let lookup_count=Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let record=RouteRecord{id:"route-1".into()};let permission=RoutePermission{id:"permission-1".into(),name:"view".into()};
    let seen=calls.clone();let lookup=lookup_count.clone();let record_clone=record.clone();
    let routes:RouteByPathAndMethod<String,R>=Arc::new(move|path,method|{assert_eq!(path,"/articles");assert_eq!(method,"GET");let seen=seen.clone();let lookup=lookup.clone();let record=record_clone.clone();effect_sync!{body:move||{seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push("lookup");if lookup.fetch_add(1,std::sync::atomic::Ordering::SeqCst)==0{None}else{Some(record.clone())}}}});
    let seen=calls.clone();let create=Arc::new(move|draft:&RouteDraft|{assert!(!draft.is_public);let seen=seen.clone();effect_try_sync!{body:move||{seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push("create");Err("exists".to_owned())}}});
    let seen=calls.clone();let permission_by_name:PermissionByName<String,R>=Arc::new(move|name|{assert_eq!(name,"view");let seen=seen.clone();let permission=permission.clone();effect_sync!{body:move||{seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push("permission");Some(permission.clone())}}});
    let catalogue=RouteCatalogue::new(permission_by_name,routes,create,Arc::new(|_|effect_sync!{body:||Some(RouteRequirement{required_permissions:vec![RoutePermission{id:"permission-1".into(),name:"view".into()}]})}),Arc::new(|_,_|effect_try_sync!{body:||Err("duplicate write".to_owned())}),Arc::new(|_,_|effect_sync!{body:||true}));
    let access=AccessControl{catalogue:Some(catalogue),..Default::default()};let handler:RouteHandler<String,R>=Arc::new(|_|effect_sync!{body:||effect_server::reply!{empty()}});let route=ManagedRoute::new("articles","get","/articles",handler);
    let exit=futures_executor::block_on(access.sync_metadata(&route,&["view".into()]).run(Arc::new(TestClock::new(Duration::ZERO))));assert!(matches!(exit,Exit::Success(())));assert_eq!(*calls.lock().map_err(|_|"lock")?,vec!["permission","lookup","create","lookup"]);Ok(())
}

#[test]
fn legacy_redirect_preserves_repeated_query_and_mutation_uses_live_sorted_params()->Result<(),String>{
    let request=request("/old?tag=a&tag=b")?;let path:RoutePathBuilder=Arc::new(|_,_,_|"/new".into());
    let redirect=routing::new_query_preserving_route_redirect_handler::<String,()>("new",Some(path),http::StatusCode::PERMANENT_REDIRECT);let finite=run(redirect(&request))?.into_finite(NonZeroUsize::MIN.saturating_add(4095)).map_err(|e|e.to_string())?;assert_eq!(finite.status,http::StatusCode::PERMANENT_REDIRECT);assert_eq!(finite.headers.get(http::header::LOCATION).and_then(|v|v.to_str().ok()),Some("/new?tag=a&tag=b"));
    let mut result=effect_cms::commands::MutationResult::default();result.redirect_route="article".into();result.redirect_language="fr".into();let _=result.redirect_params.insert("z".into(),"last".into());let _=result.redirect_params.insert("a".into(),"first".into());
    let builder:MutationResultRoutePathBuilder=Arc::new(|lang,route,params|format!("{lang}:{route}:{params:?}"));let target=routing::mutation_result_path(Some(&result),"en",Some(&builder));assert!(target.starts_with("fr:article:"));assert!(target.find("first")<target.find("last"));Ok(())
}
