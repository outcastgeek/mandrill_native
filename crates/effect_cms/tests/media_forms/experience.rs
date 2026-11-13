//! Source media_asset_experience_test.go generic mutation pipeline through real services.
use super::*;
use super::native_http::{self,Env,Handler};
use effect::{AtomicRef,effect,effect_sync,effect_try_sync,effect_map,effect_map_error};
use effect_cms::{*,commands,definitions::*,forms,media::{assets::*,flows::*,management::*,runtime::*,surface::MediaQuery},validation};
use effect_server::{Request,ServerError,Route,Scalar};
use std::{collections::BTreeMap,sync::{Mutex,atomic::{AtomicBool,AtomicI64,Ordering}}};

#[derive(Debug)]
enum Error{Validation(validation::ValidationError),Other(String)}
impl Error{fn other(message:impl std::fmt::Display)->Self{Self::Other(message.to_string())}}
impl std::fmt::Display for Error{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{match self{Self::Validation(error)=>write!(f,"{error}"),Self::Other(message)=>f.write_str(message)}}}
impl std::error::Error for Error{fn source(&self)->Option<&(dyn std::error::Error+'static)>{match self{Self::Validation(error)=>Some(error),Self::Other(_)=>None}}}

#[derive(Clone,Default)]
struct Store{records:Arc<Mutex<BTreeMap<String,MediaAssetRecord>>>,trace:Arc<Mutex<Vec<String>>>,fail_create:Arc<AtomicBool>}
impl MediaAssetStore<Error,Env> for Store{
    fn get_by_id(&self,id:commands::InputBytes)->Effect<Option<MediaAssetRecord>,Error,Env>{let store=self.clone();effect_sync!{body:move||{store.trace.lock().unwrap().push(format!("read:{id:?}"));store.records.lock().unwrap().get(&String::from_utf8_lossy(id.as_bytes()).into_owned()).cloned()}}}
    fn get_by_ids(&self,ids:Vec<commands::InputBytes>)->Effect<BTreeMap<commands::InputBytes,MediaAssetRecord>,Error,Env>{let store=self.clone();effect_sync!{body:move||{let records=store.records.lock().unwrap();ids.iter().filter_map(|id|records.get(&String::from_utf8_lossy(id.as_bytes()).into_owned()).map(|record|(id.clone(),record.clone()))).collect()}}}
    fn list(&self,query:MediaQuery)->Effect<Option<MediaAssetListResult>,Error,Env>{let store=self.clone();effect_sync!{body:move||{let items:Vec<_>=store.records.lock().unwrap().values().filter(|record|query.search.is_empty()||record.title.as_bytes().windows(query.search.as_bytes().len()).any(|value|value==query.search.as_bytes())).cloned().collect();Some(MediaAssetListResult{total:items.len() as i64,items})}}}
    fn create(&self,record:AtomicRef<MediaAssetRecord>)->Effect<(),Error,Env>{let store=self.clone();effect!{let store=store.clone();let record=record.clone();let value=record.with(Clone::clone)?;effect_try_sync!{body:move||{store.trace.lock().unwrap().push("store:create".into());if store.fail_create.load(Ordering::SeqCst){return Err(Error::other("consumer-create-failed"))}store.records.lock().unwrap().insert(value.id.clone(),value.clone());Ok(())}}?;}}
    fn update(&self,record:AtomicRef<MediaAssetRecord>)->Effect<(),Error,Env>{let store=self.clone();effect!{let store=store.clone();let record=record.clone();let value=record.with(Clone::clone)?;effect_sync!{body:move||{store.trace.lock().unwrap().push("store:update".into());store.records.lock().unwrap().insert(value.id.clone(),value.clone());}}?;}}
    fn delete(&self,id:commands::InputBytes)->Effect<(),Error,Env>{let store=self.clone();effect_sync!{body:move||{store.trace.lock().unwrap().push("store:delete".into());store.records.lock().unwrap().remove(&String::from_utf8_lossy(id.as_bytes()).into_owned());}}}
}

type Experience=MediaAssetServiceBindings<Error,Env>;
struct Fixture{experience:Experience,store:Arc<Store>,clock:Arc<AtomicI64>}
fn native_error(error:ServerError)->Error{Error::other(error.to_string())}
fn route_path(_request:&Request,route:&str,params:&[(String,String)])->String{match route{"library"=>"/en/content/media/library".into(),"new"|"new_post"=>"/en/content/media/new".into(),"edit"|"edit_post"=>format!("/en/content/media/{}/edit",params.iter().find(|(name,_)|name=="id").map(|(_,id)|id.as_str()).unwrap_or("")),"delete"=>format!("/en/content/media/{}/delete",params.iter().find(|(name,_)|name=="id").map(|(_,id)|id.as_str()).unwrap_or("")),_=>format!("/{route}")}}
fn source_forms(service:FormService<Error,Env>)->forms::FormPageSource<(),Error,Env>{
    let new=service.clone();let edit=service.clone();let rehydrate=service.clone();
    forms::bind_form_page_source(Some(Arc::new(move|_,request,kind|effect_map!{program:effect_map_error!{program:new.new_page(Arc::new(CMSRequestContext::new(request,None)),request,kind),map:|error|Error::other(error.to_string())},map:Some})),Some(Arc::new(move|_,request,kind|effect_map!{program:effect_map_error!{program:edit.edit_page(Arc::new(CMSRequestContext::new(request,None)),request,kind),map:|error|Error::other(error.to_string())},map:Some})),Some(Arc::new(move|_,request,kind,mode,result|effect_map!{program:effect_map_error!{program:rehydrate.rehydrate(Arc::new(CMSRequestContext::new(request,None)),request,kind,mode,result.clone()),map:|error|Error::other(error.to_string())},map:Some})),Some(Arc::new(move|_,request,kind,mode|effect_map!{program:effect_map_error!{program:service.page(Arc::new(CMSRequestContext::new(request,None)),request,kind,mode),map:|error|Error::other(error.to_string())},map:Some})))
}
fn fixture()->Fixture{
    let store=Arc::new(Store::default());let clock=Arc::new(AtomicI64::new(100));
    let routes=ResourceRouteSet{list:"library".into(),new:"new".into(),new_post:"new_post".into(),edit:"edit".into(),edit_post:"edit_post".into(),delete:"delete".into(),..Default::default()};
    let resources:ResourceRegistry=Arc::new(new_definition_registry([new_media_resource_definition(MediaResourceDefinitionOptions{kind:ResourceKind::new("media"),management_routes:routes,..Default::default()})]));
    let mut set_options=DefinitionServiceSetOptions::<Error,Env>::new(form_policy().urlencoded);set_options.resources=Some(resources.clone());set_options.default_page_size=24;
    let trace=store.trace.clone();set_options.validators.command_input=Some(Arc::new(move|input|{trace.lock().unwrap().push(format!("input:{}",input.action.as_str()));Ok(())}));
    let trace=store.trace.clone();set_options.validators.validation=Some(Arc::new(move|result|{trace.lock().unwrap().push(format!("validation:{}",result.code.as_str()));Ok(())}));
    let trace=store.trace.clone();set_options.validators.mutation=Some(Arc::new(move|result|{trace.lock().unwrap().push(format!("result:{}",result.action.as_str()));Ok(())}));
    let validation_bindings=set_options.validation_bindings.clone();let command_bindings=set_options.command_bindings.clone();
    let services=new_definition_service_set(set_options);
    let now=clock.clone();let runtime=new_media_asset_store_runtime(MediaAssetStoreRuntimeOptions{assets:Some(store.clone()),current_user_id:Some(Arc::new(|_|"editor-1".into())),new_id:Some(Arc::new(||"asset-created".into())),now_unix:Some(Arc::new(move||now.load(Ordering::SeqCst))),route_path:Some(Arc::new(route_path)),routes:MediaAssetStoreRuntimeRoutes{library:"library".into(),new:"new".into(),edit:"edit".into(),..Default::default()},..Default::default()});
    let validation=services.validation.clone();let commands=services.commands.clone();
    let management=MediaAssetManagementOptions{runtime:runtime.clone(),resource_kind:ResourceKind::new("media"),library_page_size:24,picker_page_size:8,shortcode_page_size:4,io:MediaFlowIO{form:None,form_policy:form_policy(),on_form_error:Arc::new(native_error),write_error:None,write_patch:None,write_patches:None,write_html_status:None,write_html:None},require_access:None,request_language:Some(Arc::new(|_|"en".into())),mutation_route_path:Some(Arc::new(|language,route,_|format!("/{language}/content/media/{route}"))),csrf_token:None,csrf_field_name:None,render_html:Some(Arc::new(|_,markup|effect_sync!{once;body:move||effect_server::reply!{html(effect_server::HtmlContent::trusted(markup.unwrap_or_default().into_string()))}})),render_picker:None,list_page:None,form_page:None,validation:Some(Arc::new(move||Some(validation.clone()))),commands:Some(Arc::new(move||Some(commands.clone()))),on_mutation_error:Arc::new(|failure|Error::other(failure.to_string())),validate_input:None,translate_key:None,translate_args:None,url_required_key:"URL required".into(),bad_request_key:"Bad request".into(),internal_error_key:"Internal error".into(),not_found_key:"Not found".into()};
    let policy=form_policy();let experience=new_media_asset_service(MediaAssetServiceOptions{bindings:MediaAssetManagementControllerBindingOptions{management,resources,route_path:Some(Arc::new(route_path)),forms:source_forms(services.forms.clone()),context:Arc::new(()),render_page:None,render_form_page:Some(Arc::new(|request,page|effect_cms::resourcepages::new_resource_page_renderer(Default::default()).form_page(request,page).unwrap())),resolve_url:None,state:None,page_options:Default::default(),invalid_request_error:Arc::new(|message|Error::other(message.to_owned()))},runtime_bindings:Default::default(),route_names:Default::default(),route_permissions:Default::default(),access:Default::default(),commands:MediaAssetCommandBindingOptions{runtime,kind:ResourceKind::new("media"),redirect_route:"library".into(),invalid_request_error:Arc::new(|message|Error::other(message.to_owned())),form:None,form_policy:policy},validation:MediaAssetValidationBindingOptions{kind:ResourceKind::new("media"),url_required_message:Some(Arc::new(|_|"media URL is required".into())),invalid_request_error:Arc::new(||Error::other("invalid media identity")),form:None,validation_error:Arc::new(Error::Validation),on_form_error:Arc::new(native_error),form_policy:policy.urlencoded,multipart_policy:policy.multipart,multipart_fields:policy.multipart_fields,multipart_values:policy.multipart_values,file_memory:policy.file_memory}}).unwrap();
    experience.bind_resource_validation(&validation_bindings);
    experience.bind_resource_commands(&command_bindings);
    Fixture{experience,store,clock}
}
fn capture_route(action:&str)->Route<String>{
    let scalar=Scalar::new(|raw|Ok::<String,ServerError>(raw.to_owned()),|value:&String|Ok(value.clone()));
    let suffix=format!("/{action}");
    effect_server::route!{literal("/en/content/media");capture("id",scalar);literal(suffix.as_str());}.unwrap()
}
fn send(fixture:&Fixture,action:&str,id:Option<&str>,body:&str)->native_http::Response{
    let controller=fixture.experience.controller();let action=action.to_owned();let which=action.clone();
    let handler:Handler<Error>=Arc::new(move|request|match which.as_str(){"new"=>controller.create(request),"edit"=>controller.update(request),"delete"=>controller.delete(request),_=>unreachable!()});
    let uri=id.map(|id|format!("/en/content/media/{id}/{action}")).unwrap_or_else(||"/en/content/media/new".into());
    native_http::deliver_at(handler,Arc::new(native_error),http::Method::POST,uri,body.to_owned(),id.map(|_|capture_route(&action))).unwrap()
}
fn form(url:&str,title:&str,kind:&str,status:&str)->String{format!("kind={kind}&source_kind=remote&url={url}&status={status}&title={title}")}
fn assert_redirect(response:&native_http::Response){assert_eq!(response.status,http::StatusCode::SEE_OTHER);assert_eq!(response.headers.get(http::header::LOCATION).and_then(|value|value.to_str().ok()),Some("/en/content/media/library"));}

#[test]
fn media_experience_create_update_delete_use_generic_command_pipeline(){
    let fixture=fixture();
    let created=send(&fixture,"new",None,&form("https%3A%2F%2Fexample.test%2Fnew.jpg","New+Media+Asset","image","ready"));assert_redirect(&created);
    let initial=fixture.store.records.lock().unwrap().get("asset-created").cloned().expect("consumer create persisted record");
    assert_eq!(initial.title,"New Media Asset");assert_eq!(initial.url,"https://example.test/new.jpg");assert_eq!(initial.created_by,"editor-1");assert_eq!((initial.created_at,initial.updated_at),(100,100));
    fixture.clock.store(200,Ordering::SeqCst);
    let updated=send(&fixture,"edit",Some("asset-created"),&form("https%3A%2F%2Fexample.test%2Fupdated.mp4","Updated+Media+Asset","video","archived"));assert_redirect(&updated);
    let record=fixture.store.records.lock().unwrap().get("asset-created").cloned().expect("consumer update retained record");assert_eq!(record.kind,"video");assert_eq!(record.status,"archived");assert_eq!(record.title,"Updated Media Asset");assert_eq!(record.url,"https://example.test/updated.mp4");assert_eq!((record.created_at,record.updated_at),(100,200));
    let deleted=send(&fixture,"delete",Some("asset-created"),"");assert_redirect(&deleted);assert!(fixture.store.records.lock().unwrap().is_empty());
    let trace=fixture.store.trace.lock().unwrap();let writes:Vec<_>=trace.iter().filter(|entry|entry.starts_with("store:")).cloned().collect();assert_eq!(writes,vec!["store:create","store:update","store:delete"]);
    for action in ["create","update","delete"]{let input=format!("input:{action}");assert_eq!(trace.iter().filter(|entry|**entry==input).count(),2,"validation and command owners both validated {action}");let write=format!("store:{action}");let result=format!("result:{action}");assert!(trace.iter().position(|entry|entry==&write)<trace.iter().position(|entry|entry==&result));}
}

#[test]
fn media_experience_rejects_missing_url_before_generic_executor_or_store_write(){
    let fixture=fixture();let response=send(&fixture,"new",None,&form("","Rejected+Asset","image","ready"));
    assert_eq!(response.status,http::StatusCode::OK);assert!(response.body.contains("media URL is required"),"rejected body: {}",response.body);assert!(response.headers.get(http::header::LOCATION).is_none());assert!(fixture.store.records.lock().unwrap().is_empty());
    let trace=fixture.store.trace.lock().unwrap();assert_eq!(trace.iter().filter(|entry|entry.as_str()=="input:create").count(),1);assert!(!trace.iter().any(|entry|entry.starts_with("store:")||entry.starts_with("result:")));
}

#[test]
fn media_experience_consumer_create_failure_never_publishes_success_or_mutation_result(){
    let fixture=fixture();fixture.store.fail_create.store(true,Ordering::SeqCst);
    let response=send(&fixture,"new",None,&form("https%3A%2F%2Fexample.test%2Fnew.jpg","Failed+Asset","image","ready"));
    assert_eq!(response.status,http::StatusCode::INTERNAL_SERVER_ERROR);assert!(response.headers.get(http::header::LOCATION).is_none());assert!(fixture.store.records.lock().unwrap().is_empty());
    let trace=fixture.store.trace.lock().unwrap();assert_eq!(trace.iter().filter(|entry|entry.as_str()=="store:create").count(),1);assert!(!trace.iter().any(|entry|entry.starts_with("result:")||entry=="store:update"||entry=="store:delete"));
}
