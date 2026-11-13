use super::run;
use effect::effect_sync;
use effect_cms::{definitions::*,queries as q,validation as v,mutations as m};
use std::{collections::BTreeMap,sync::{Arc,atomic::{AtomicUsize,Ordering}}};
type Error=std::fmt::Error;
fn request()->effect_server::Request {effect_server::Request::from_native(http::Request::builder().method("POST").uri("/source").body(effect_server::native::axum::body::Body::empty()).unwrap(),effect::Cancellation::new())}
fn relation()->ResourceRelationDefinition {ResourceRelationDefinition {name:"target".into(),target:ResourceKind::new("target"),cardinality:"many_to_one".into(),keys:vec![ResourceRelationKeyDefinition {source:ResourceFieldKey::new("target_id"),target:ResourceFieldKey::new("id"),r#type:"string".into()}],operations:vec!["select".into()],..Default::default()}}
fn registry()->Arc<dyn Registry+Send+Sync>{Arc::new(new_definition_registry([ResourceDefinition {kind:ResourceKind::new("source"),relations:vec![relation()],..Default::default()}]))}
fn selection()->q::RelationSelection {let descriptor=new_resource_relation_descriptor(ResourceKind::new("source"),relation()).unwrap();q::new_relation_selection(&descriptor,"",vec![q::new_relation_string_key(ResourceFieldKey::new("id"),"selected").unwrap()]).unwrap()}

#[test]
fn relation_zero_services_publish_nothing_and_factories_refuse_unbound_capabilities() {
    assert!(run(q::RelationLookupService::<Error,()>::default().lookup(q::RelationLookupInput::default())).unwrap().is_none());
    assert!(run(v::RelationValidationService::<Error,()>::default().validate(v::RelationValidationInput::default())).unwrap().is_none());
    assert!(run(m::RelationMutationService::<Error,()>::default().execute(m::RelationMutationInput::default())).unwrap().is_none());
    assert!(q::new_relation_lookup_service::<Error,()>(registry(),q::RelationLookupBinding::default(),q::RelationLookupOptions::default()).is_err());
    assert!(v::new_relation_validation_service::<Error,()>(registry(),v::RelationValidationBinding::default()).is_err());
    assert!(m::new_relation_mutation_service::<Error,()>(m::RelationMutationBinding::default()).is_err());
}

#[test]
fn relation_descriptor_and_selection_reject_wrong_identity_key_shape_and_operations() {
    let descriptor=new_resource_relation_descriptor(ResourceKind::new("source"),relation()).unwrap();
    assert!(descriptor.is_valid()&&descriptor.allows("select"));assert!(!descriptor.allows("delete"));
    assert!(q::new_relation_selection(&descriptor,"",vec![q::new_relation_int_key(ResourceFieldKey::new("id"),1).unwrap()]).is_err());
    assert!(q::new_relation_selection(&descriptor,"",vec![]).is_err());
    let selected=selection();assert!(selected.belongs_to(&descriptor));assert_eq!(selected.keys()[0].string_value(),Some(b"selected".as_slice()));
    assert!(new_resource_relation_descriptor(ResourceKind::new("source"),ResourceRelationDefinition {cardinality:"unknown".into(),..relation()}).is_err());
    assert!(!ResourceRelationDescriptor::default().is_valid());assert!(!q::RelationSelection::default().is_valid());
}

#[test]
fn relation_lookup_is_lazy_reusable_and_publishes_hydrated_selection() {
    let admitted=Arc::new(AtomicUsize::new(0));let looked=Arc::new(AtomicUsize::new(0));let count=admitted.clone();let observe=looked.clone();
    let record=q::new_relation_lookup_record(selection(),"Selected".into(),"/selected".into()).unwrap();
    let binding=q::new_relation_lookup_binding::<Error,()>(ResourceKind::new("source"),"target",Arc::new(move |_| {let count=count.clone();effect_sync! {body:move || {count.fetch_add(1,Ordering::SeqCst);()},}}),Arc::new(move |query| {let query=query.clone();let record=record.clone();let observe=observe.clone();effect_sync! {body:move || {observe.fetch_add(1,Ordering::SeqCst);q::observe_relation_lookup(&query,vec![record.clone()],vec![record.clone()],vec![],1,query.revision()).unwrap()},}})).unwrap();
    let service=q::new_relation_lookup_service(registry(),binding,q::RelationLookupOptions {default_page_size:10,..Default::default()}).unwrap();
    assert!(matches!(run(service.lookup(q::RelationLookupInput::default())).unwrap(),Some(q::RelationLookupOutcome::Rejected(_))));
    assert_eq!(admitted.load(Ordering::SeqCst),0);
    let program=service.lookup(q::RelationLookupInput {request:Some(request()),source:ResourceKind::new("source"),relation:"target".into(),mode:q::RelationLookupMode::new(q::RELATION_LOOKUP_CHOICES),selected:vec![selection()],..Default::default()});
    assert_eq!(looked.load(Ordering::SeqCst),0);
    for _ in 0..2 {let outcome=run(program.clone()).unwrap().unwrap();assert!(matches!(outcome,q::RelationLookupOutcome::Found(_)));let result=outcome.result().unwrap();assert!(result.is_valid());assert_eq!(result.selected().len(),1);}
    assert_eq!(admitted.load(Ordering::SeqCst),2);assert_eq!(looked.load(Ordering::SeqCst),2);
}

#[test]
fn relation_validation_detaches_rows_and_mutation_consumes_claim_once() {
    let checked=Arc::new(AtomicUsize::new(0));let application=Arc::new(AtomicUsize::new(0));let count=checked.clone();let applied=application.clone();
    let binding=v::new_relation_validation_binding::<Error,()>(ResourceKind::new("source"),"target",Arc::new(move |draft|{let count=count.clone();effect_sync! {body:move || {count.fetch_add(1,Ordering::SeqCst);v::observe_relation_rows_validated(draft.clone()).unwrap()},}}),Arc::new(move |_|{let applied=applied.clone();effect_sync! {body:move || {applied.fetch_add(1,Ordering::SeqCst);v::RelationRowsApplicationResult::default()},}})).unwrap();
    let service=v::new_relation_validation_service(registry(),binding).unwrap();
    assert!(matches!(run(service.validate(v::RelationValidationInput::default())).unwrap(),Some(v::RelationValidationOutcome::Rejected(_))));
    assert_eq!(checked.load(Ordering::SeqCst),0);
    let input=v::RelationValidationInput {request:Some(request()),source:ResourceKind::new("source"),relation:"target".into(),source_id:"parent".into(),operation:"select".into(),rows:vec![v::RelationRowInput {key:"row".into(),selection:selection(),operation:"select".into(),values:BTreeMap::from([(ResourceFieldKey::new("caption"),"original".into())]),..Default::default()}],..Default::default()};
    let outcome=run(service.validate(input)).unwrap().unwrap();let v::RelationValidationOutcome::Accepted(rows)=outcome else {panic!("expected admitted rows")};
    assert!(rows.is_valid());assert_eq!(checked.load(Ordering::SeqCst),1);assert_eq!(application.load(Ordering::SeqCst),1);
    let mut projection=rows.rows();projection[0].values.insert(ResourceFieldKey::new("caption"),"changed".into());assert_eq!(rows.rows()[0].values[&ResourceFieldKey::new("caption")],"original");
    let before=Arc::new(AtomicUsize::new(0));let written=Arc::new(AtomicUsize::new(0));let b=before.clone();let w=written.clone();
    let binding=m::new_relation_mutation_binding::<Error,()>(ResourceKind::new("source"),"target",Arc::new(move |draft:v::RelationRowsDraft|{let b=b.clone();effect_sync! {body:move ||{b.fetch_add(1,Ordering::SeqCst);m::observe_relation_before_accepted(draft.clone())},}}),Arc::new(move |request:m::RelationMutationRequest|{let w=w.clone();effect_sync! {body:move ||{w.fetch_add(1,Ordering::SeqCst);m::observe_relation_mutation_applied(&request,"revision")},}})).unwrap();
    let mutation=m::new_relation_mutation_service(binding).unwrap();
    assert!(matches!(run(mutation.execute(m::RelationMutationInput::default())).unwrap(),Some(m::RelationMutationOutcome::Rejected(_))));
    let input=m::RelationMutationInput {source:ResourceKind::new("source"),relation:"target".into(),source_id:"parent".into(),rows,..Default::default()};
    let outcome=run(mutation.execute(input.clone())).unwrap().unwrap();let m::RelationMutationOutcome::Applied(applied)=outcome else {panic!("expected applied mutation")};assert_eq!(applied.revision(),"revision");
    assert!(matches!(run(mutation.execute(input)).unwrap(),Some(m::RelationMutationOutcome::Rejected(_))));
    assert_eq!(before.load(Ordering::SeqCst),1);assert_eq!(written.load(Ordering::SeqCst),1);
}
