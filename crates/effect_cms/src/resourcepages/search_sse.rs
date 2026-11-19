//! Search replacement and ordered pagination patches over the native reply body.
use std::sync::Arc;
use maud::Markup;
use effect_server::{Reply,Request};
use crate::{commands::InputBytes,definitions::{Registry,ResourceKind},routing::RoutePathBuilder};
use crate::rendering::datastar::{component_patch_event,signal_patch_event,event_stream_reply};
pub const RESOURCE_SEARCH_PATCH_MODE_QUERY:&str="patch";
pub const RESOURCE_SEARCH_PATCH_MODE_REPLACE:&str="replace";
fn patch_url(raw:&str,replace:bool)->String{
    if !crate::media::assets::media_url_is_valid(raw){return raw.to_owned();}
    let Some(encoded)=crate::content::encode_reference_url(raw) else{return raw.to_owned()};
    let (base,fragment)=encoded.split_once('#').map(|(b,f)|(b,Some(f))).unwrap_or((&encoded,None));
    let (path,query)=base.split_once('?').unwrap_or((base,""));
    let mut values:crate::queries::QueryValues=effect_server::decode_url_encoded_pairs_partial(query.as_bytes()).values.into_iter().map(|(k,v)|(k.into(),v.into_iter().map(InputBytes::from).collect())).collect();
    if replace{values.insert(RESOURCE_SEARCH_PATCH_MODE_QUERY.into(),vec![RESOURCE_SEARCH_PATCH_MODE_REPLACE.into()]);}else{values.remove(RESOURCE_SEARCH_PATCH_MODE_QUERY.as_bytes());}
    let mut result=crate::queries::append_encoded_query(path,&values);
    if values.is_empty()&&base.ends_with('?'){result.push('?');}
    if let Some(fragment)=fragment{result.push('#');result.push_str(fragment);}result
}
pub fn resource_search_sse_url(raw:&str)->String{patch_url(raw,true)}
pub fn resource_append_sse_url(raw:&str)->String{patch_url(raw,false)}
pub fn is_resource_search_replace_request(request:Option<&Request>)->bool{
    request.map(|r|crate::queries::request_query_value(r,RESOURCE_SEARCH_PATCH_MODE_QUERY).trim().as_bytes().eq_ignore_ascii_case(RESOURCE_SEARCH_PATCH_MODE_REPLACE.as_bytes())).unwrap_or(false)
}
pub fn write_resource_search_replacement<E:Send+'static,R:Send+Sync+'static>(selector:&str,component:Markup,url_signal:&str,next_url:&str,loading_signal:&str,has_more_signal:&str,has_more:bool)->Reply<E,R>{
    write_resource_search_replacement_with_mode(selector,component,url_signal,next_url,loading_signal,has_more_signal,has_more,"outer")
}
pub fn write_resource_search_replacement_with_mode<E:Send+'static,R:Send+Sync+'static>(selector:&str,component:Markup,url_signal:&str,next_url:&str,loading_signal:&str,has_more_signal:&str,has_more:bool,mode:&str)->Reply<E,R>{
    let mut signals=std::collections::BTreeMap::new();
    signals.insert(url_signal.to_owned(),serde_json::Value::String(resource_append_sse_url(next_url)));
    signals.insert(loading_signal.to_owned(),serde_json::Value::Bool(false));signals.insert(has_more_signal.to_owned(),serde_json::Value::Bool(has_more));
    let payload=serde_json::to_string(&signals).unwrap_or_default();
    event_stream_reply(vec![component_patch_event(selector,mode,&component),signal_patch_event(&payload)])
}
pub fn write_paginated_component_stream<E:Send+'static,R:Send+Sync+'static>(selector:&str,components:&[Markup],next_url:&str,url_signal:&str,loading_signal:&str,has_more_signal:&str,has_more:bool)->Reply<E,R>{
    let mut events:Vec<_>=components.iter().map(|c|component_patch_event(selector,"append",c)).collect();
    let payload=if has_more{format!("{{\"{url_signal}\":\"{next_url}\",\"{loading_signal}\":false}}")}
        else{format!("{{\"{has_more_signal}\":false,\"{loading_signal}\":false}}")};
    events.push(signal_patch_event(&payload));event_stream_reply(events)
}
pub struct ResourcePaginatedComponentStreamOptions{
    pub request:Request,pub resources:Option<Arc<dyn Registry+Send+Sync>>,pub kind:ResourceKind,
    pub manage:bool,pub page:i64,pub page_size:i64,pub total:i64,pub selector:String,pub components:Vec<Markup>,
    pub url_signal:String,pub loading_signal:String,pub has_more_signal:String,pub route_path:Option<RoutePathBuilder>,
}
pub fn write_resource_paginated_component_stream<E:Send+'static,R:Send+Sync+'static>(options:ResourcePaginatedComponentStreamOptions)->Reply<E,R>{
    let page=options.page.max(1);let pages=if options.total<=0||options.page_size<=0{1}else{options.total/options.page_size+i64::from(options.total%options.page_size!=0)};
    let has_more=page<pages;
    let next=if has_more{resource_append_sse_url(&crate::routing::build_resource_sse_url_for_kind(&options.request,options.resources.as_deref(),&options.kind,options.manage,page.wrapping_add(1),options.route_path.as_ref()))}else{String::new()};
    write_paginated_component_stream(&options.selector,&options.components,&next,&options.url_signal,&options.loading_signal,&options.has_more_signal,has_more)
}
pub fn write_resource_list_component_stream<E:Send+'static,R:Send+Sync+'static>(options:ResourcePaginatedComponentStreamOptions)->Reply<E,R>{write_resource_paginated_component_stream(options)}

/// Stream a bounded sequence of existing Maud components and signal data through
/// prepared native DATA backing. Each frame retains its lease through delivery.
pub fn prepared_resource_search_stream<E: Send + 'static, R: Send + Sync + 'static>(
    (selector, mode, components, signals): (Arc<str>, Arc<str>, Arc<[Markup]>, Arc<str>),
    frames: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
    maximum_components: std::num::NonZeroUsize,
    on_error: impl Fn(effect::memory::MemoryError) -> E + Send + Sync + 'static,
) -> effect::Effect<Reply<E, R>, E, R> {
    crate::rendering::datastar::prepared_event_stream(
        (selector, mode, components, signals),
        frames,
        maximum_components,
        on_error,
    )
}

/// Render an ordered search event stream using the current prepared CMS environment.
pub fn prepared_cms_search_stream<E, R>(
    patches: (Arc<str>, Arc<str>, Arc<[Markup]>, Arc<str>),
    maximum_components: std::num::NonZeroUsize,
    on_error: impl Fn(effect::memory::MemoryError) -> E + Send + Sync + 'static,
) -> effect::Effect<Reply<E, R>, E, R>
where
    E: Send + 'static,
    R: crate::HasCmsMemory + Send + Sync + 'static,
{
    let on_error = Arc::new(on_error);
    effect::effect! {
        let on_error = on_error.clone();
        let patches = patches.clone();
        let frames = effect::service! { env: |env: &R| env.cms_memory().event_frames().clone(), }?;
        prepared_resource_search_stream(patches, frames, maximum_components, move |error| on_error(error))?
    }
}
