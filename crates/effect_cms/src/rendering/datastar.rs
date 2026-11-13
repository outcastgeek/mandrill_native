//! Shared Datastar framing; all element payloads are serialized Maud components.
use maud::Markup;
use effect_server::{Reply,reply};
use http::{header,HeaderName,HeaderValue};
fn field(event:&mut String,name:&str,value:&str){
    for line in value.replace("\r\n","\n").replace('\r',"\n").split('\n'){
        event.push_str("data: ");event.push_str(name);event.push(' ');event.push_str(line);event.push('\n');
    }
}
pub(crate) fn component_patch_event(selector:&str,mode:&str,component:&Markup)->String{
    let mut event=String::from("event: datastar-patch-elements\n");
    if !selector.is_empty(){field(&mut event,"selector",selector);}
    if !mode.is_empty(){field(&mut event,"mode",mode);}
    if !component.0.as_str().is_empty(){field(&mut event,"elements",component.0.as_str());}
    field(&mut event,"ifmissing","ignore");event.push('\n');event
}
pub(crate) fn signal_patch_event(signals:&str)->String{
    let mut event=String::from("event: datastar-patch-signals\n");
    if !signals.is_empty(){field(&mut event,"signals",signals);}
    event.push('\n');event
}
pub(crate) fn event_reply<E:Send+'static,R:Send+Sync+'static>(event:String)->Reply<E,R>{
    reply! {header(header::CONTENT_TYPE,HeaderValue::from_static("text/event-stream"));
        header(header::CACHE_CONTROL,HeaderValue::from_static("no-cache"));
        header(header::CONNECTION,HeaderValue::from_static("keep-alive"));
        header(HeaderName::from_static("x-accel-buffering"),HeaderValue::from_static("no"));text(event);}
}
/// Native body ownership preserves event write/flush ordering and terminal failures.
pub(crate) fn event_stream_reply<E:Send+'static,R:Send+Sync+'static>(events:Vec<String>)->Reply<E,R>{
    let body=effect_server::stream! {body |writer| {
        let events=events.clone();let writer=writer.clone();
        for event in events {if !event.is_empty(){writer.write(event.into())?;writer.flush()?;}}
    }};
    reply! {header(header::CACHE_CONTROL,HeaderValue::from_static("no-cache"));
        header(header::CONNECTION,HeaderValue::from_static("keep-alive"));
        header(HeaderName::from_static("x-accel-buffering"),HeaderValue::from_static("no"));sse(body);}
}
