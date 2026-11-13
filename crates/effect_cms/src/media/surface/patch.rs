//! Native Datastar replies carrying Maud-authored fragments.
use effect_server::{Reply,Request};
use http::header;
use crate::rendering::datastar::{component_patch_event,event_reply};
use maud::Markup;
use crate::commands::InputBytes;

#[derive(Clone)]
pub struct MediaPatch { pub root_id: InputBytes, pub mode: String, pub component: Markup }

pub fn media_datastar_request(request: &Request) -> bool {
    request.headers().get("Datastar-Request").is_some_and(|value| !value.as_bytes().is_empty())
        || request.headers().get(header::ACCEPT).is_some_and(|value| value.as_bytes().windows(17).any(|part| part == b"text/event-stream"))
}

pub fn write_media_patch<E:Send+'static,R:Send+Sync+'static>(
    root_id: impl AsRef<[u8]>, mode: &str, component: Markup, default_root_id: &str,
) -> Reply<E,R> {
    let root = crate::validation::trim_unicode_whitespace(root_id.as_ref());
    let root = if root.is_empty() { default_root_id.trim().as_bytes() } else { root };
    event_reply(element_event(root,mode,&component))
}
pub fn write_media_patches<E:Send+'static,R:Send+Sync+'static>(patches: &[MediaPatch]) -> Reply<E,R> {
    let mut events = String::new();
    for patch in patches {
        if !patch.root_id.trim().is_empty() {
            events.push_str(&element_event(patch.root_id.trim().as_bytes(),patch.mode.trim(),&patch.component));
        }
    }
    event_reply(events)
}
fn element_event(root: &[u8], mode: &str, component: &Markup) -> String {
    component_patch_event(&format!("#{}",crate::byte_text::encoding_text(root)),if mode.trim().is_empty(){"outer"}else{mode},component)
}
