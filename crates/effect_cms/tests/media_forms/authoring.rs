use super::*;
use effect_cms::{authoring::*,commands::InputBytes};
use effect::{effect_sync,effect_future};
use std::{collections::BTreeMap,time::Duration,sync::atomic::{AtomicBool,AtomicUsize,Ordering}};
#[test]fn provider_json_shape_fences_escape_repair_and_custom_fallback_signals(){
 let field=AuthoringTranslationField{name:"content".into(),value:"Hello".into(),format:"markdown".into()};assert_eq!(serde_json::to_string(&vec![field]).unwrap(),r#"[{"name":"content","value":"Hello","format":"markdown"}]"#);
 let parsed=parse_authoring_translation_fields_json("```json\n{\"fields\":{\"body\":\"Use \\#tag\"}}\n```").unwrap();assert_eq!(parsed[&InputBytes::from("body")].as_bytes(),br"Use \#tag");assert!(parse_authoring_translation_fields_json("{broken").is_err());
 let signals=AuthoringTranslationSignals{translating:"customTranslating".into(),status_text:"customStatus".into(),..Default::default()};let value:serde_json::Value=serde_json::from_str(&authoring_translation_fallback_signals_json(&signals,"failed")).unwrap();assert_eq!(value["customTranslating"],false);assert_eq!(value["customStatus"],"failed");assert!(value.get("locTranslating").is_none());assert_eq!(DEFAULT_AUTHORING_TRANSLATION_TIMEOUT,Duration::from_secs(60));
}
#[test]fn normalization_dedupes_targets_and_keeps_exact_nonblank_field_values(){
 let normalizer=|s:&InputBytes|InputBytes::from(String::from_utf8(s.as_bytes().to_ascii_lowercase()).unwrap());let targets=normalize_target_languages(&" EN ".into(),&[" fr ".into(),"FR".into(),"en".into(),"".into(),"pt".into()],Some(&normalizer));assert_eq!(targets,vec![InputBytes::from("fr"),InputBytes::from("pt")]);
 let fields=normalize_authoring_fields(&[AuthoringTranslationField{name:" body ".into(),value:"\n line one\nline two \n".into(),format:" markdown ".into()},AuthoringTranslationField{name:"blank".into(),value:" \n ".into(),..Default::default()}]);assert_eq!(fields.len(),1);assert_eq!(fields[0].name.as_bytes(),b"body");assert_eq!(fields[0].value.as_bytes(),b"\n line one\nline two \n");assert_eq!(authoring_field_format_map(&fields)[&InputBytes::from("body")].as_bytes(),b"markdown");
}
fn initialized_form(targets:&[&str])->Arc<effect_server::ByteFormData>{Arc::new(effect_server::ByteFormData{values:[(b"scope".to_vec(),vec![b"primary".to_vec()]),(b"widget_key".to_vec(),vec![b"primary_body".to_vec()]),(b"source_language".to_vec(),vec![b"en".to_vec()]),(b"target_language".to_vec(),targets.iter().map(|v|v.as_bytes().to_vec()).collect()),(b"field_name".to_vec(),vec![b" content ".to_vec()]),(b"field_value".to_vec(),vec![b"hello\nworld".to_vec()]),(b"field_format".to_vec(),vec![b"markdown".to_vec()])].into()})}
type Env=super::native_http::Env;
fn options(translator:Option<AuthoringTranslator<String,Env>>,targets:&[&str])->LocalizedAuthoringTranslationHandlerOptions<String,String,Env>{LocalizedAuthoringTranslationHandlerOptions{translator,timeout:Duration::from_secs(1),require_access:None,translate_key:None,normalize_language:None,internal_error_status_code:500,deliver_patch:None,form_policy:form_policy(),form:Some(initialized_form(targets))}}
fn deliver(options:LocalizedAuthoringTranslationHandlerOptions<String,String,Env>)->super::native_http::Response {let route=new_localized_authoring_translation_handler(options);super::native_http::deliver(route,Arc::new(AuthoringHandlerError::Delivery)).unwrap()}
#[test]fn localized_translation_delivers_native_multiline_signal_patch_and_unavailable_branch(){
 let translator:AuthoringTranslator<String,Env>=Arc::new(|_,timeout,input|{assert_eq!(timeout,Duration::from_secs(1));assert_eq!(input.fields.with(|fields|fields[0].value.clone()).as_bytes(),b"hello\nworld");effect_sync!{once;body:move ||AuthoringTranslationResult{target_language:input.target_language,fields:effect_cms::definitions::SharedMap::new([("content".into(),"bonjour\nmonde".into())].into())}}});
 let response=deliver(options(Some(translator),&["fr"]));assert_eq!(response.status,http::StatusCode::OK);assert!(response.headers[http::header::CONTENT_TYPE].to_str().unwrap().contains("text/event-stream"));assert!(response.body.contains("datastar-patch-signals"));assert!(response.body.contains("bonjour\\nmonde"));assert!(response.body.contains("markdown"));
 let unavailable=deliver(options(None,&["fr"]));assert_eq!(unavailable.status,http::StatusCode::OK);assert!(unavailable.headers[http::header::CONTENT_TYPE].to_str().unwrap().contains("text/event-stream"));assert!(unavailable.body.contains("localized_tabs.auto_translate_unavailable"));assert!(!unavailable.body.contains("bonjour"));
}
#[test]fn translation_normalizes_targets_and_runs_distinct_targets_concurrently(){
 let barrier=Arc::new(tokio::sync::Barrier::new(2));let calls=Arc::new(AtomicUsize::new(0));let count=calls.clone();
 let translator:AuthoringTranslator<String,Env>=Arc::new(move|_,_,input|{let barrier=barrier.clone();let count=count.clone();effect_future!{once;body:move |_|async move{count.fetch_add(1,Ordering::SeqCst);barrier.wait().await;AuthoringTranslationResult{target_language:input.target_language.clone(),fields:effect_cms::definitions::SharedMap::new([("content".into(),input.target_language)].into())}}}});
 let mut config=options(Some(translator),&[" fr ","FR","en","pt"]);config.normalize_language=Some(Arc::new(|s|String::from_utf8(s.as_bytes().to_ascii_lowercase()).unwrap().into()));let response=deliver(config);assert_eq!(response.status,http::StatusCode::OK);assert!(response.headers.contains_key(http::header::CONTENT_TYPE));assert_eq!(calls.load(Ordering::SeqCst),2);assert!(response.body.contains("fr"));assert!(response.body.contains("pt"));
}
struct OnDrop(Arc<AtomicBool>);impl Drop for OnDrop{fn drop(&mut self){self.0.store(true,Ordering::SeqCst)}}
#[test]fn translator_deadline_cancels_hung_future_and_renders_failure_patch(){
 let dropped=Arc::new(AtomicBool::new(false));let guard=dropped.clone();let observed=Arc::new(std::sync::Mutex::new(None));let deadline=observed.clone();
 let translator:AuthoringTranslator<String,Env>=Arc::new(move|_,timeout,_|{*deadline.lock().unwrap()=Some(timeout);let guard=guard.clone();effect_future!{once;body:move |cancel|async move{let _guard=OnDrop(guard);cancel.cancelled().await;AuthoringTranslationResult::default()}}});
 let mut config=options(Some(translator),&["fr"]);config.timeout=Duration::from_millis(20);let response=deliver(config);assert_eq!(response.status,http::StatusCode::OK);assert!(response.headers[http::header::CONTENT_TYPE].to_str().unwrap().contains("text/event-stream"));assert!(response.body.contains("localized_tabs.auto_translate_failed"));assert_eq!(*observed.lock().unwrap(),Some(Duration::from_millis(20)));assert!(dropped.load(Ordering::SeqCst));
}
#[test]fn access_reply_precedes_translator_and_form_parsing(){
 let calls=Arc::new(AtomicUsize::new(0));let counter=calls.clone();let translator:AuthoringTranslator<String,Env>=Arc::new(move|_,_,_|{counter.fetch_add(1,Ordering::SeqCst);effect_sync!{body:||AuthoringTranslationResult::default()}});
 let mut config=options(Some(translator),&["fr"]);config.require_access=Some(Arc::new(|_|effect_sync!{body:||Some(effect_server::reply!{status(effect_server::BodyStatus::new(http::StatusCode::FORBIDDEN).unwrap());text("denied".to_owned())})}));
 let response=deliver(config);assert_eq!(response.status,http::StatusCode::FORBIDDEN);assert!(response.headers.contains_key(http::header::CONTENT_TYPE));assert_eq!(response.body,"denied");assert_eq!(calls.load(Ordering::SeqCst),0);
}
#[test]fn concurrent_translation_callbacks_share_the_normalized_field_backing_store(){
 let changed=Arc::new(tokio::sync::Notify::new());let observed=Arc::new(AtomicBool::new(false));let seen=observed.clone();
 let translator:AuthoringTranslator<String,Env>=Arc::new(move|_,_,input|{let changed=changed.clone();let seen=seen.clone();effect_future!{once;body:move |_|async move{
   if input.target_language==InputBytes::from("fr"){assert!(input.fields.with_element_mut(0,|field|field.value="updated".into()));changed.notify_one();}else{changed.notified().await;seen.store(input.fields.with(|fields|fields[0].value==InputBytes::from("updated")),Ordering::SeqCst);}
   AuthoringTranslationResult{target_language:input.target_language,fields:effect_cms::definitions::SharedMap::new(BTreeMap::from([("content".into(),input.fields.with(|fields|fields[0].value.clone()))]))}
 }}});
 let response=deliver(options(Some(translator),&["fr","pt"]));assert_eq!(response.status,http::StatusCode::OK);assert!(response.headers.contains_key(http::header::CONTENT_TYPE));assert!(observed.load(Ordering::SeqCst));assert!(response.body.contains("updated"));assert!(!response.body.contains("hello\\nworld"));
}
#[test]fn ready_patch_reads_returned_shared_result_after_language_normalizer_mutation(){
 let returned:Arc<std::sync::Mutex<Option<effect_cms::definitions::SharedMap<InputBytes,InputBytes>>>>=Arc::new(std::sync::Mutex::new(None));let stored=returned.clone();
 let translator:AuthoringTranslator<String,Env>=Arc::new(move|_,_,input|{let fields=effect_cms::definitions::SharedMap::new(BTreeMap::from([("content".into(),"original".into())]));*stored.lock().unwrap()=Some(fields.clone());effect_sync!{once;body:move||AuthoringTranslationResult{target_language:input.target_language,fields}}});
 let mut config=options(Some(translator),&["fr"]);config.normalize_language=Some(Arc::new(move|lang|{if let Some(fields)=returned.lock().unwrap().as_ref(){fields.insert("content".into(),"updated after return".into());}lang.clone()}));
 let response=deliver(config);assert_eq!(response.status,http::StatusCode::OK);assert!(response.headers.contains_key(http::header::CONTENT_TYPE));assert!(response.body.contains("updated after return"));assert!(!response.body.contains("original"));
}
#[derive(Default)]struct TestTasks(std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>);
impl effect::TaskSpawner for TestTasks{fn spawn(&self,task:effect::ScheduledTask)->Result<(),effect::Defect>{let worker=std::thread::spawn(move||block_on(task));self.0.lock().unwrap().push(worker);Ok(())}}
#[derive(Default)]struct CancelEnv{tasks:TestTasks,clock:effect::TestClock,cancel:effect::Cancellation}
impl effect::HasTaskSpawner for CancelEnv{type Tasks=TestTasks;fn task_spawner(&self)->&TestTasks{&self.tasks}}
impl effect::HasClock for CancelEnv{type Clock=effect::TestClock;fn clock(&self)->&effect::TestClock{&self.clock}}
impl effect::HasCancellation for CancelEnv{fn cancellation(&self)->&effect::Cancellation{&self.cancel}}
#[test]fn cancellation_interrupts_active_translation_and_releases_its_future(){
 let (started,receive)=std::sync::mpsc::channel();let started=Arc::new(std::sync::Mutex::new(started));let dropped=Arc::new(AtomicBool::new(false));let guard=dropped.clone();
 let translator:AuthoringTranslator<String,CancelEnv>=Arc::new(move|_,_,_|{let started=started.clone();let guard=guard.clone();effect_future!{once;body:move |cancel|async move{let _guard=OnDrop(guard);started.lock().unwrap().send(()).unwrap();cancel.cancelled().await;AuthoringTranslationResult::default()}}});
 let env=Arc::new(CancelEnv::default());let run_env=env.clone();let program=build_authoring_translation_patch(&request("/"),Some(translator),Duration::ZERO,AuthoringTranslationRequest{source_language:"en".into(),target_languages:vec!["fr".into()],fields:vec![AuthoringTranslationField{name:"body".into(),value:"source".into(),..Default::default()}],..Default::default()},AuthoringTranslationPatchOptions::default());
 let worker=std::thread::spawn(move||block_on(program.run_cancellable(run_env)));receive.recv_timeout(Duration::from_secs(3)).expect("translator must start before cancellation");assert!(env.cancel.request());assert!(matches!(worker.join().unwrap(),Exit::Interrupted));let tasks=std::mem::take(&mut *env.tasks.0.lock().unwrap());for task in tasks{task.join().unwrap();}assert!(dropped.load(Ordering::SeqCst));
}
