//! Source media fragment actions; strings here are JavaScript, never HTML.
use super::*;
use crate::attachments::MediaAttachmentManagerProps;

pub fn media_form_request_action(method:&str,url:&str,form_id:&str)->String {
    let url=url.trim();let form_id=form_id.trim(); if url.is_empty(){return String::new()}
    let action=if method=="post"{"@post"}else{"@get"};
    if form_id.is_empty(){return format!("{action}('{url}')")}
    format!("const form = document.querySelector('#{form_id}'); if (form) {{ const previousNoValidate = form.noValidate; form.noValidate = true; {action}('{url}', {{contentType: 'form', selector: '#{form_id}'}}); form.noValidate = previousNoValidate; }} else {{ {action}('{url}'); }}")
}
pub fn media_form_get_action(url:&str,form_id:&str)->String {media_form_request_action("get",url,form_id)}
pub fn media_mutation_action(url:&str,owner_form_id:&str)->String {media_form_request_action("post",url,owner_form_id)}
pub fn open_media_fragment_action(url:&str,owner_form_id:&str)->String {
    let url=url.trim();let id=owner_form_id.trim();if url.is_empty(){return String::new()}
    let root=crate::media::assets::media_action_url_root_id(url);
    let clear=media_mount_clear_only_action(&root);
    let request=if id.is_empty(){format!("@get('{url}')")}else{format!("const form = document.querySelector('#{id}'); if (form) {{ const mediaStateForm = document.createElement('form'); const mediaStateFormId = 'media-state-' + Math.random().toString(36).slice(2); mediaStateForm.id = mediaStateFormId; mediaStateForm.noValidate = true; mediaStateForm.style.display = 'none'; const mediaStateNames = new Set(['attachment_ids[]', 'attachment_positions[]', 'attachment_roles[]', 'attachment_titles[]', 'attachment_captions[]', 'attachment_alts[]']); form.querySelectorAll('input, select, textarea').forEach((field) => {{ const name = field.getAttribute('name') || ''; if (!mediaStateNames.has(name)) return; const clone = document.createElement('input'); clone.type = 'hidden'; clone.name = name; clone.value = field.value; mediaStateForm.appendChild(clone); }}); document.body.appendChild(mediaStateForm); try {{ @get('{url}', {{contentType: 'form', selector: '#' + mediaStateFormId}}); }} finally {{ mediaStateForm.remove(); }} }} else {{ @get('{url}'); }}")};
    let prefix=if clear.is_empty(){String::new()}else{format!("{clear}; ")};
    format!("{}; {prefix}{request}",media_scroll_lock_action())
}
pub fn media_action_url(base:&str,role:&str)->String {let base=base.trim();let role=role.trim();if base.is_empty()||role.is_empty(){base.into()}else{format!("{base}{}role={role}",if base.contains('?'){"&"}else{"?"})}}
pub fn media_mutation_url(p:&MediaAttachmentManagerProps,action:&str,asset_id:impl AsRef<[u8]>)->String {
    let trim=crate::validation::trim_unicode_whitespace;
    let mut parts=vec![format!("mutation={}",media_query_escape(trim(action.as_bytes()))),format!("asset_id={}",media_query_escape(trim(asset_id.as_ref())))];
    for (key,value) in [("scope",p.scope.as_bytes()),("target",p.mounts.section_id.as_bytes()),("owner_form_id",p.mounts.owner_form_id.as_bytes()),("owner_content_id",p.owner_content_id.as_bytes())] {
        let value=trim(value);if !value.is_empty(){parts.push(format!("{key}={}",media_query_escape(value)))}
    }
    format!("{}{}{}",p.mutation_url.trim(),if p.mutation_url.trim().contains('?'){"&"}else{"?"},parts.join("&"))
}
fn media_query_escape(value:&[u8])->String {
    let mut encoded=String::new();
    for &byte in value {match byte {
        b'A'..=b'Z'|b'a'..=b'z'|b'0'..=b'9'|b'-'|b'_'|b'.'|b'~'=>encoded.push(byte as char),
        b' '=>encoded.push('+'), other=>encoded.push_str(&format!("%{other:02X}")),
    }}
    encoded
}

pub fn media_fragment_button(p:&MediaFragmentButtonProps)->maud::Markup {
    let mut attributes:std::collections::BTreeMap<String,String>=p.button_attributes.iter().cloned().collect();
    let action=open_media_fragment_action(p.url.trim(),p.owner_form_id.trim());
    let extra=attributes.get("data-on:click").map(|v|v.trim()).unwrap_or("");
    let action=if extra.is_empty(){action}else{format!("{extra}; {action}")};attributes.insert("data-on:click".into(),action);
    let variant=match p.variant.trim(){"primary"=>"primary","outline"=>"outline","ghost"=>"ghost","destructive"=>"destructive","secondary"=>"secondary","success"=>"success","warning"=>"warning","info"=>"info","link"=>"link",_=>"default"};
    let size=match p.size.trim(){"xs"=>"xs","md"=>"md","lg"=>"lg","xl"=>"xl",_=>"sm"};
    web_components::components::unstyled::button::Button(&web_components::components::unstyled::button::ButtonProps{button_type:"button",variant,size,class:p.button_class.trim().into(),attributes,..Default::default()},maud::html!{(p.label.trim())})
}
