use super::run;
use effect::{effect, effect_sync};
use effect_cms::{access as a, commands as c, definitions::*, queries as q, validation as v, mutations as m};
use effect_server::{Request, ByteFormPolicy, BodyLimit, ByteFieldBounds};
use std::{num::NonZeroUsize,sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering}}};

#[derive(Clone,Debug,Eq,PartialEq)]
struct Failure(&'static str);
impl std::fmt::Display for Failure {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str(self.0)}}
impl std::error::Error for Failure {}
fn request(uri:&str)->Request {
    Request::from_native(http::Request::builder().method("POST").uri(uri).body(effect_server::native::axum::body::Body::empty()).unwrap(),effect::Cancellation::new())
}
fn policy()->ByteFormPolicy {
    ByteFormPolicy {body:BodyLimit::new(NonZeroUsize::new(4096).unwrap()),fields:NonZeroUsize::new(32).unwrap(),values:ByteFieldBounds {count:NonZeroUsize::new(8).unwrap(),bytes:NonZeroUsize::new(1024).unwrap()}}
}
fn registry()->Arc<dyn Registry+Send+Sync> {Arc::new(new_definition_registry([ResourceDefinition {kind:ResourceKind::new("sample"),actions:SharedList::new(vec![ResourceAction::new(RESOURCE_ACTION_CREATE)]),..Default::default()}]))}

#[test]
fn list_query_admits_repeated_filters_after_page_callback_and_reuses_description() {
    let filters=SharedList::new(vec![ResourceFilterDefinition {key:ResourceFilterKey::new(RESOURCE_FILTER_TAG),param:"tag".into(),mode:ResourceFilterMode::new(RESOURCE_FILTER_MODE_MULTI_ALL),..Default::default()},ResourceFilterDefinition {key:ResourceFilterKey::new(RESOURCE_FILTER_STATUS),param:"filter".into(),default_value:"before".into(),..Default::default()}]);
    let resources=Arc::new(new_definition_registry([ResourceDefinition {kind:ResourceKind::new("sample"),filters:filters.clone(),..Default::default()}]));
    let calls=Arc::new(AtomicUsize::new(0)); let seen=calls.clone();
    let service=q::new_query_service(resources,q::Options {default_page_size:20,parse_named_page:Some(Arc::new(move |_,name| {assert_eq!(name,"page");seen.fetch_add(1,Ordering::SeqCst);assert!(filters.with_element_mut(1,|f|f.default_value="after".into()));3})),..Default::default()});
    let program=service.list_query::<Failure,()>(&request("/resources?tag=rust&tag=ffi&tag=rust&tag="),&ResourceKind::new("sample"));
    assert_eq!(calls.load(Ordering::SeqCst),0);
    for _ in 0..2 {let query=run(program.clone()).unwrap();assert_eq!((query.page,query.page_size),(3,20));assert_eq!(query.values_for_filter(&ResourceFilterKey::new(RESOURCE_FILTER_TAG)),vec![c::InputBytes::from("ffi"),"rust".into()]);assert_eq!(query.filter_value(&ResourceFilterKey::new(RESOURCE_FILTER_STATUS)),"after");assert!(query.filtered());}
    assert_eq!(calls.load(Ordering::SeqCst),2);
    let unknown=run(service.list_query::<Failure,()>(&request("/?search=x"),&ResourceKind::new("unknown"))).unwrap();
    assert_eq!((unknown.page,unknown.page_size),(1,20)); assert!(unknown.search.is_empty());
}

#[test]
fn filter_toggle_and_search_token_contracts_preserve_encoding_order_and_limits() {
    let filter=ResourceFilterDefinition {key:ResourceFilterKey::new(RESOURCE_FILTER_TAG),param:"tag".into(),mode:ResourceFilterMode::new(RESOURCE_FILTER_MODE_MULTI_ALL),..Default::default()};
    let options=q::FilterToggleURLOptions {request:Some(request("/resources?tag=rust&search=ffi&page=4")),base_path:"/resources".into(),definition:filter,value:"zig".into(),reset_page:true,sort_values:true};
    assert_eq!(q::resource_filter_toggle_url(&options),"/resources?search=ffi&tag=rust&tag=zig");
    assert_eq!(q::resource_filter_toggle_url(&q::FilterToggleURLOptions {value:"rust".into(),..options}),"/resources?search=ffi");
    assert_eq!(q::resource_filter_url("/resources","tag","go templ"),"/resources?tag=go+templ");
    assert_eq!(q::resource_filter_url("/resources?page=2","tag","go"),"/resources?page=2&tag=go");
    assert_eq!(q::resource_filter_url("/resources","","go"),"/resources");
    assert_eq!(q::search_tokens("Rust, rust! ffi · CAFÉ 42"),["rust","ffi","café","42"]);
    assert_eq!(q::search_tokens_with_options("one two three two",&q::SearchTokenOptions {max_tokens:2}),["one","two"]);
}

#[test]
fn default_detail_service_admits_query_values_without_registry_or_language() {
    let service=q::DetailService::default();
    let query=run(service.detail_query::<Failure,()>(&request("/?id=42&slug=hello"),&ResourceKind::new("unknown"))).unwrap();
    assert_eq!(query.id,"42"); assert_eq!(query.slug,"hello"); assert!(query.parent.is_empty()&&query.locale.is_empty());
}

#[test]
fn access_public_bypass_skips_resolver_and_principal_rules_keep_configured_roles() {
    let principal=a::AccessPrincipal::new("id","user","email",vec![" editor ".into()],"stamp","session");
    assert!(a::principal_has_any_role(&[" editor ".into()])(&principal));
    assert!(!a::principal_has_any_role(&[] )(&principal));
    assert!(!a::principal_has_any_role(&["Editor".into()])(&principal));
    let calls=Arc::new(AtomicUsize::new(0));let seen=calls.clone();
    let resolver:a::PrincipalResolver<Failure,()>=Arc::new(move |_| {let seen=seen.clone();effect_sync! {body:move || {seen.fetch_add(1,Ordering::SeqCst);None},}});
    let public=run(a::require_managed_principal_access(&request("/"),false,a::PrincipalAccessOptions::default(),&resolver)).unwrap();
    assert!(public.is_bypassed()&&public.proceeds());assert_eq!(calls.load(Ordering::SeqCst),0);
    let denied=run(a::require_principal_access(&request("/"),false,a::PrincipalAccessOptions::default(),&resolver)).unwrap();
    assert!(!denied.proceeds());assert!(denied.into_response().is_some());assert_eq!(calls.load(Ordering::SeqCst),1);
    let allowed=run(a::require_principal_access(&request("/"),true,a::PrincipalAccessOptions {principal:Some(principal),..Default::default()},&resolver)).unwrap();
    assert!(allowed.is_allowed());assert_eq!(calls.load(Ordering::SeqCst),1);
}

#[test]
fn command_execution_uses_mutated_identity_live_bindings_and_result_map_aliases() {
    let bindings=c::ResourceCommandBindings::<Failure,()>::default();
    let result_params=SharedMap::default(); let shared=result_params.clone();
    let validation_calls=Arc::new(AtomicUsize::new(0));let count=validation_calls.clone();
    let service=c::CommandService::new(registry(),c::Options {executors:bindings.clone(),validate_command_input:Some(Arc::new(|input|{input.kind=ResourceKind::new("sample");input.action=ResourceAction::new(RESOURCE_ACTION_CREATE);input.form_values.insert("stage".into(),"validated".into());Ok(())})),validate_mutation_result:Some(Arc::new(move |mut result| {count.fetch_add(1,Ordering::SeqCst);assert_eq!(result.kind.as_str(),"sample");assert_eq!(result.action.as_str(),RESOURCE_ACTION_CREATE);result.redirect_route="local scalar".into();result.redirect_params.insert("edited".into(),"yes".into());Ok(())})),..Default::default()});
    let executed=Arc::new(AtomicUsize::new(0));let calls=executed.clone();
    bindings.set(ResourceKind::new("sample"),ResourceAction::new(RESOURCE_ACTION_CREATE),Some(Arc::new(move |_,input| {let params=shared.clone();let calls=calls.clone();effect! {let params=params.clone();let calls=calls.clone();let values=input.with(|i|i.form_values.clone())?;assert_eq!(values[&c::InputBytes::from("stage")],"validated");calls.fetch_add(1,Ordering::SeqCst);c::MutationResult {redirect_route:"published".into(),redirect_params:params,..Default::default()}}})));
    let handle=c::CommandInputHandle::new(c::CommandInput {kind:ResourceKind::new("wrong"),action:ResourceAction::new("wrong"),..Default::default()});
    let program=service.execute(&request("/"),Some(handle.clone()));assert_eq!(executed.load(Ordering::SeqCst),0);
    let result=run(program.clone()).unwrap();
    assert_eq!(result.redirect_route,"published"); assert_eq!(result_params.get(&"edited".to_owned()).as_deref(),Some("yes"));
    assert_eq!(run(handle.with::<_,Failure,()>(|input|input.action.clone())).unwrap().as_str(),RESOURCE_ACTION_CREATE);
    assert_eq!(executed.load(Ordering::SeqCst),1);assert_eq!(validation_calls.load(Ordering::SeqCst),1);
    assert!(run(program).is_ok());assert_eq!(executed.load(Ordering::SeqCst),2);
    assert!(matches!(run(service.execute(&request("/"),None)),Err(c::CommandError::InvalidRequest(_))));
}

#[test]
fn command_missing_executor_and_refused_action_never_invoke_result_validator() {
    let calls=Arc::new(AtomicUsize::new(0));let seen=calls.clone();
    let service=c::CommandService::<Failure,()>::new(registry(),c::Options {validate_mutation_result:Some(Arc::new(move |_|{seen.fetch_add(1,Ordering::SeqCst);Ok(())})),..Default::default()});
    for (action,unsupported) in [(RESOURCE_ACTION_CREATE,false),(RESOURCE_ACTION_DELETE,true)] {let result=run(service.execute(&request("/"),Some(c::CommandInputHandle::new(c::CommandInput {kind:ResourceKind::new("sample"),action:ResourceAction::new(action),..Default::default()}))));assert!(if unsupported {matches!(result,Err(c::CommandError::UnsupportedAction(_)))}else{matches!(result,Err(c::CommandError::InvalidRequest(_)))});}
    assert_eq!(calls.load(Ordering::SeqCst),0);
}

#[test]
fn validation_result_error_takes_precedence_over_supported_and_refused_actions() {
    let validation=v::ActionValidationService::<Failure,()>::new(registry(),None,policy(),v::Options {validators:v::Validators {validation_result:Some(Arc::new(|_|Err(Failure("result rejected")))),..Default::default()},..Default::default()});
    for action in [RESOURCE_ACTION_CREATE,RESOURCE_ACTION_UPDATE] {
        let outcome=run(validation.validate_action_input(Some(&request("/")),&ResourceKind::new("sample"),&ResourceAction::new(action))).unwrap();
        let (input,_,cause)=outcome.into_projection(); assert!(input.is_none()); assert!(matches!(cause,Some(v::ActionValidationCause::Consumer(Failure("result rejected")))));
    }
}

#[test]
fn mutation_runs_validation_before_command_and_stops_on_before_failure() {
    let seen=Arc::new(Mutex::new(Vec::new()));let log=seen.clone();
    let validation=v::ActionValidationService::<Failure,()>::new(registry(),Some(q::new_query_service(registry(),q::Options {default_page_size:20,..Default::default()})),policy(),v::Options {validators:v::Validators {command_input:Some(Arc::new(move |input| {log.lock().unwrap().push("validate");input.form_values.insert("validation".into(),"present".into());Ok(())})),..Default::default()},..Default::default()});
    let bindings=c::ResourceCommandBindings::default();let log=seen.clone();
    bindings.set(ResourceKind::new("sample"),ResourceAction::new(RESOURCE_ACTION_CREATE),Some(Arc::new(move |_,input| {let log=log.clone();effect! {let log=log.clone();let values=input.with(|i|i.form_values.clone())?;assert_eq!(values[&c::InputBytes::from("stage")],"before");assert_eq!(values[&c::InputBytes::from("validation")],"present");log.lock().unwrap().push("command");c::MutationResult::default()}})));
    let commands=c::CommandService::new(registry(),c::Options {executors:bindings,..Default::default()});let log=seen.clone();
    let before:m::BeforeExecute<Failure>=Arc::new(move |input| {log.lock().unwrap().push("before");assert_eq!(input.form_values[&c::InputBytes::from("validation")],"present");input.form_values.insert("stage".into(),"before".into());(v::ValidationResult::default(),None)});
    let program=m::execute_resource_mutation_with_before(&request("/"),&ResourceKind::new("sample"),&ResourceAction::new(RESOURCE_ACTION_CREATE),&validation,&commands,Some(before));
    assert!(seen.lock().unwrap().is_empty());let applied=run(program).unwrap();assert_eq!(applied.result().unwrap().kind.as_str(),"sample");assert_eq!(*seen.lock().unwrap(),["validate","before","command"]);
    seen.lock().unwrap().clear();let log=seen.clone();
    let result=run(m::execute_resource_mutation_with_before(&request("/"),&ResourceKind::new("sample"),&ResourceAction::new(RESOURCE_ACTION_CREATE),&validation,&commands,Some(Arc::new(move |_|{log.lock().unwrap().push("before");(v::ValidationResult::default(),Some(Failure("before failed")))}))));
    assert!(matches!(result,Err(ref error) if matches!(error.cause(),m::MutationCause::Before(Failure("before failed")))));assert_eq!(*seen.lock().unwrap(),["validate","before"]);
}

#[test]
fn binder_collections_keep_live_maps_and_ordered_callback_replacement() {
    let visited=Mutex::new(Vec::new());
    let second=Mutex::new("original");
    let bindings=v::new_bindings_from::<c::CommandInput,Failure,()>(2,|index,b| {if index==0 {*second.lock().unwrap()="replacement";visited.lock().unwrap().push("first");}else{visited.lock().unwrap().push(*second.lock().unwrap());b.set(ResourceKind::new("sample"),ResourceAction::new("create"),Some(Arc::new(|_,_|effect_sync! {body:|| (),})));}});
    assert_eq!(*visited.lock().unwrap(),["first","replacement"]);
    let copy=bindings.clone();
    bindings.set(ResourceKind::new("sample"),ResourceAction::new("update"),Some(Arc::new(|_,_|effect_sync! {body:|| (),})));
    assert!(copy.get(&ResourceKind::new("sample"),&ResourceAction::new("update")).is_some());
}

fn schema_policy()->effect::SchemaPolicy {
    fn nz(n:usize)->NonZeroUsize {NonZeroUsize::new(n).unwrap()}
    effect::SchemaPolicy {parse:effect::ParsePolicy {limits:effect::ParseLimits {bytes:nz(65536),depth:nz(128),nodes:nz(32768),collection:nz(256),issues:nz(128)},errors:effect::SchemaErrorMode::Accumulate,unknown_fields:effect::UnknownFields::Reject,duplicate_fields:effect::DuplicateFields::Reject},json_depth:nz(64),string_length:nz(8192),integer_digits:nz(32),pattern_steps:nz(65536)}
}
#[test]
fn native_contract_routes_caller_selector_and_preserves_setup_error_identity() {
    let policy=schema_policy();let context=effect::ParseContext::new(policy);
    let builtin=Arc::new(v::NativeContractValidator::builtin(policy.parse).unwrap());
    let platform=v::NativeContractHandle {validator:Some(builtin.clone()),error:None};
    let project=v::NativeContractHandle::default();
    let document=serde_json::json!({"Kind":"sample","Action":"create","RedirectRoute":"resource.detail"});
    v::route_native_contract(v::SELECTOR_MUTATION_RESULT,&document,&platform,&project,None,&context).unwrap();
    assert!(matches!(v::route_native_contract("#CallerInput",&document,&platform,&project,None,&context),Err(v::NativeContractError::MissingValidator(_))));
    // Caller selection is supplied as a native selector mapping; no CUE runtime or stack inspection.
    v::route_native_contract(v::SELECTOR_MUTATION_RESULT,&document,&project,&platform,Some(&|_|false),&effect::ParseContext::new(policy)).unwrap();
    let cause:Arc<dyn std::error::Error+Send+Sync>=Arc::new(Failure("setup failed"));
    let failed=v::NativeContractHandle {validator:None,error:Some(cause.clone())};
    let error=v::route_native_contract(v::SELECTOR_MUTATION_RESULT,&document,&failed,&project,None,&effect::ParseContext::new(policy)).unwrap_err();
    assert!(matches!(error,v::NativeContractError::Setup(ref retained) if Arc::ptr_eq(retained,&cause)));
    assert!(v::validate_native_value(None,"#CallerInput",&document,&effect::ParseContext::new(policy)).is_ok());
    assert!(matches!(v::validate_native_command_input(None,None,&effect::ParseContext::new(policy)),Err(v::NativeContractError::MissingInput)));
}

#[test]
fn native_contract_reports_selector_relative_field_path_and_rejects_unknown_or_malformed_values() {
    let policy=schema_policy();
    let error=v::validate_builtin_contract(v::SELECTOR_MUTATION_RESULT,&serde_json::json!({"Kind":"sample","Action":"create","RedirectRoute":""}),policy).unwrap_err();
    assert!(!error.issues().is_empty());
    assert!(error.issues().iter().any(|issue|issue.path.contains("RedirectRoute")&&!issue.path.contains("MutationResultVM")));
    assert!(v::validate_builtin_contract("#DoesNotExist",&serde_json::json!({}),policy).is_err());
    assert!(v::validate_builtin_contract_serialized(v::SELECTOR_MUTATION_RESULT,"{invalid",policy).is_err());
    assert!(v::validate_builtin_contract(v::SELECTOR_MUTATION_RESULT,&serde_json::json!({"Kind":"sample","Action":"unsupported","RedirectRoute":"x"}),policy).is_err());
}

#[test]
fn command_input_form_parser_trims_values_rejects_malformed_and_honors_initialized_form() {
    fn form(body:&str)->Request {Request::from_native(http::Request::builder().method("POST").uri("/resource?title=query").header("Content-Type","application/x-www-form-urlencoded").body(effect_server::native::axum::body::Body::from(body.to_owned())).unwrap(),effect::Cancellation::new())}
    let kind=ResourceKind::new("sample");let action=ResourceAction::new(RESOURCE_ACTION_CREATE);
    let error=run(v::build_command_input::<()>(None,Some(&form("title=++hello++&bad=%zz")),&kind,&action,policy())).unwrap_err();
    assert!(error.to_string().contains("parse form:"));
    let input=run(v::build_command_input::<()>(None,Some(&form("title=++hello++")),&kind,&action,policy())).unwrap();
    assert_eq!(input.form_values[&c::InputBytes::from("title")],"hello");
    let initialized=effect_server::ByteFormData {values:std::collections::BTreeMap::from([(b"title".to_vec(),vec![b"current initialized".to_vec()])])};
    let input=run(v::build_command_input_from_form::<()>(None,Some(&form("bad=%zz")),&kind,&action,policy(),Some(initialized))).unwrap();
    assert_eq!(input.form_values[&c::InputBytes::from("title")],"current initialized");
}

#[test]
fn access_redirect_forbidden_and_unauthorized_publish_exact_dispositions() {
    let resolver:a::PrincipalResolver<Failure,()>=Arc::new(|_|effect_sync! {body:||None,});
    let redirect=a::new_redirect_access_response(Some(Arc::new(|_|"/login".into())),effect_server::BodyStatus::new(http::StatusCode::FOUND).unwrap());
    let outcome=run(a::require_managed_principal_access(&request("/"),true,a::PrincipalAccessOptions {redirect_unauthenticated:Some(redirect),..Default::default()},&resolver)).unwrap();
    assert!(!outcome.proceeds());let response=outcome.into_response().unwrap().into_finite(NonZeroUsize::new(4096).unwrap()).unwrap();assert_eq!(response.status,http::StatusCode::FOUND);assert_eq!(response.headers[http::header::LOCATION],"/login");
    let forbidden=a::new_status_key_access_response(effect_server::BodyStatus::new(http::StatusCode::FORBIDDEN).unwrap(),"forbidden".into(),None);
    let member=a::AccessPrincipal::new("id","user","",vec!["MEMBER".into()],"","");
    let outcome=run(a::require_managed_principal_access(&request("/"),true,a::PrincipalAccessOptions {principal:Some(member),can_access:Some(a::principal_has_any_role(&["ADMIN".into()])),render_forbidden:Some(forbidden),..Default::default()},&resolver)).unwrap();
    assert!(!outcome.proceeds());let response=outcome.into_response().unwrap().into_finite(NonZeroUsize::new(4096).unwrap()).unwrap();assert_eq!(response.status,http::StatusCode::FORBIDDEN);assert_eq!(response.bytes.as_ref(),b"Forbidden\n");
    let unauthorized=a::new_status_key_access_response(effect_server::BodyStatus::new(http::StatusCode::UNAUTHORIZED).unwrap(),"unauthorized".into(),None);
    let outcome=run(a::require_principal_access(&request("/"),false,a::PrincipalAccessOptions {render_unauthorized:Some(unauthorized),..Default::default()},&resolver)).unwrap();
    assert!(!outcome.proceeds());let response=outcome.into_response().unwrap().into_finite(NonZeroUsize::new(4096).unwrap()).unwrap();assert_eq!(response.status,http::StatusCode::UNAUTHORIZED);
}

#[test]
fn command_action_availability_is_ordered_reusable_and_appends_translation_once() {
    let resource=ResourceDefinition {kind:ResourceKind::new("sample"),actions:SharedList::new(vec![ResourceAction::new(RESOURCE_ACTION_CREATE),ResourceAction::new(RESOURCE_ACTION_DELETE)]),widgets:SharedList::new(vec![LocalizedWidgetVM::default()]),..Default::default()};
    let service=c::CommandService::<Failure,()>::new(Arc::new(new_definition_registry([resource])),c::Options::default());
    assert!(service.supports(&ResourceKind::new("sample"),&ResourceAction::new(RESOURCE_ACTION_TRANSLATE)));
    assert!(!service.supports(&ResourceKind::new("unknown"),&ResourceAction::new(RESOURCE_ACTION_CREATE)));
    let program=service.actions(&request("/"),&ResourceKind::new("sample"));
    for _ in 0..2 {let actions=run(program.clone()).unwrap();assert_eq!(actions.iter().map(|a|a.action.as_str()).collect::<Vec<_>>(),[RESOURCE_ACTION_CREATE,RESOURCE_ACTION_DELETE,RESOURCE_ACTION_TRANSLATE]);}
    assert!(run(service.actions(&request("/"),&ResourceKind::new("unknown"))).is_err());
}
