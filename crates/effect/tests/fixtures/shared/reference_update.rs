#![deny(semicolon_in_expressions_from_macros)]
use fx::*;
use std::{num::NonZeroUsize,sync::Arc};
fn nz(n:usize)->NonZeroUsize{NonZeroUsize::new(n).expect("positive fixture budget")}
fn policy()->ParsePolicy{ParsePolicy{limits:ParseLimits{bytes:nz(4096),depth:nz(16),nodes:nz(128),collection:nz(16),issues:nz(8)},errors:SchemaErrorMode::First,unknown_fields:UnknownFields::Reject,duplicate_fields:DuplicateFields::Reject}}
fn success<A,E>(exit:Exit<A,E>)->A{match exit{Exit::Success(value)=>value,_=>panic!("reference consumer outcome")}}
struct Decode {value:i64}
struct Encode;
struct Record {value:i64,name:String}
pub async fn exercise<R:HasClock+HasTaskSpawner+Send+Sync+'static>(environment:Arc<R>){
    let field=json_i64().into_effect::<Decode,Encode>();
    let codec=schema!{struct Record{#[schema(default_with = ||Effect::service(|env:&Decode|env.value))]value:field,name:json_string().into_effect::<Decode,Encode>()}policy:policy(),representation:JsonRepresentation,unknown_fields:UnknownFields::Reject}.expect("native definition");
    let record=success(codec.decode_once(JsonValue::Object([("name".to_owned(),JsonValue::String("owned".to_owned()))].into_iter().collect())).run(Arc::new(Decode{value:42})).await);
    assert_eq!(record.value,42);assert_eq!(record.name,"owned");
    let bytes=success(codec.write_once(record,JsonWriterPolicy{bytes:nz(4096),depth:nz(16)}).run(Arc::new(Encode)).await);
    assert!(std::str::from_utf8(&bytes).expect("UTF8").contains("42"));
    struct Owned(String);
    let items:Box<[Owned;2]>=Box::new([Owned("first".to_owned()),Owned("second".to_owned())]);
    let moved:Vec<_>=items.into_iter().map(|value|value.0).collect();assert_eq!(moved.len(),2);
    let program:Effect<(),SharedValue<DependencyCacheError<()>>,R>=Scope::scoped(move |scope|effect!{
        let cache=DependencyCache::make(scope.clone(),DependencyCachePolicy{retention:nz(2),maximum_active:nz(2),maximum_dependencies:nz(2),key_equal:Arc::new(|left:&usize,right|left==right),dependency_equal:Arc::new(|left:&String,right|left==right),cache_failure:Arc::new(|_:&()|false),ttl:CacheTtl::Fixed(None)},|key|{let key=*key;Effect::sync(move ||(key,vec!["schema".to_owned()]))})?;
        let first=cache.get_observed(1)?;assert_eq!(first.observation,CacheObservation::Loaded);
        let retained=cache.get_observed(1)?;assert_eq!(retained.observation,CacheObservation::Retained);
        cache.set(1,7,vec!["schema".to_owned()])?;
        let set=cache.try_get(1)?;assert_eq!(set.expect("set retained").with(|value|*value),Ok(7));
        cache.invalidate_dependency("schema".to_owned())?;let missing=cache.try_get(1)?;assert!(missing.is_none());
    });
    success(program.run(environment).await);
}
