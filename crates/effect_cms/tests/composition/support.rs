use std::{fmt::Debug,num::NonZeroUsize,sync::Arc};
use effect::{Cancellation,Effect,Exit};
use effect_server::{Request,ByteFormPolicy,ByteFieldBounds,BodyLimit,native::axum::body::Body};
pub fn run<A:Send+'static,E:Debug+Send+'static>(program:Effect<A,E>)->Result<A,String>{
    match futures_executor::block_on(program.run(Arc::new(()))){Exit::Success(value)=>Ok(value),other=>Err(format!("unexpected exit: {}",exit_name(other)))}
}
fn exit_name<A,E:Debug>(exit:Exit<A,E>)->String{match exit{Exit::Success(_)=>"success".into(),Exit::Fail(e)=>format!("failure: {e:?}"),Exit::Die(d)=>format!("defect: {d:?}"),Exit::Interrupted=>"interrupted".into(),Exit::FinalizationFailed(f)=>format!("finalization: {f:?}")}}
pub fn request(uri:&str)->Result<Request,String>{Ok(Request::from_native(http::Request::builder().uri(uri).body(Body::empty()).map_err(|e|e.to_string())?,Cancellation::new()))}
pub fn policy()->ByteFormPolicy{ByteFormPolicy{body:BodyLimit::new(NonZeroUsize::MIN.saturating_add(4095)),fields:NonZeroUsize::MIN.saturating_add(31),values:ByteFieldBounds{count:NonZeroUsize::MIN.saturating_add(15),bytes:NonZeroUsize::MIN.saturating_add(1023)}}}
pub fn definition()->effect_cms::definitions::ResourceDefinition{
    use effect_cms::definitions::*;
    ResourceDefinition{kind:ResourceKind::new("article"),public_routes:ResourceRouteSet{list:"articles".into(),detail:"article".into(),..Default::default()},management_routes:ResourceRouteSet{list:"manage-articles".into(),dashboard:"manage-dashboard".into(),new:"article-new".into(),..Default::default()},display:ResourceDisplayDefinition{list_title_key:"articles.title".into(),dashboard_title_key:"articles.dashboard".into(),..Default::default()},seo:ResourceSEODefinition{list_title_key:"articles.title".into(),list_description_key:"articles.description".into(),indexable_list:true,indexable_detail:true,..Default::default()},sitemap:ResourceSitemapDefinition{enabled:true,route_name:"sitemap-articles".into(),priority:0.7,..Default::default()},..Default::default()}
}
