//! Form-value helpers over the canonical initialized form snapshot.
use std::{collections::BTreeMap,sync::Arc};
use effect::{Effect,effect_future,effect_map};
use effect_server::{Request,ByteFormData};
use crate::{commands::InputBytes,form_media::{MergedFormPolicy,InitializeForm,initialize_merged_form_inner}};
use super::{FORM_SUBMIT_ACTION_FIELD_NAME,FORM_SUBMIT_ACTION_VALUE_DRAFT,FORM_SUBMIT_ACTION_VALUE_PUBLISH};
fn form_values<E:Send+'static,R:Send+Sync+'static>(request:Option<&Request>,form:Option<Arc<ByteFormData>>,policy:MergedFormPolicy)->Effect<ByteFormData,E,R>{
    let request=request.cloned();
    effect_future! {body:move |_|{let request=request.clone();let form=form.clone();async move{
        if let Some(form)=form{return ByteFormData{values:form.values.clone()};}
        if let Some(request)=request{if let Ok((form,_))=initialize_merged_form_inner(&request,policy,InitializeForm::FormValue).await{return form;}}
        ByteFormData{values:BTreeMap::new()}
    }}}
}
fn first(form:&ByteFormData,key:&[u8])->InputBytes{form.values.get(key).and_then(|v|v.first()).cloned().map(InputBytes::from).unwrap_or_default().trim()}
pub fn requested_form_value<E:Send+'static,R:Send+Sync+'static>(request:Option<&Request>,name:impl Into<InputBytes>,fallback:impl Into<InputBytes>,form:Option<Arc<ByteFormData>>,policy:MergedFormPolicy)->Effect<InputBytes,E,R>{
    let name=name.into();let fallback=fallback.into();
    effect_map!{program:form_values(request,form,policy),map:move |form|{let value=first(&form,name.as_bytes());if value.is_empty(){fallback.trim()}else{value}}}
}
pub fn requested_content_status<E:Send+'static,R:Send+Sync+'static>(request:Option<&Request>,fallback:&str,form:Option<Arc<ByteFormData>>,policy:MergedFormPolicy)->Effect<String,E,R>{
    let fallback=fallback.to_owned();
    effect_map!{program:form_values(request,form,policy),map:move |form|{
        let action=first(&form,FORM_SUBMIT_ACTION_FIELD_NAME.as_bytes());
        if action==FORM_SUBMIT_ACTION_VALUE_DRAFT{return "draft".to_owned()}
        if action==FORM_SUBMIT_ACTION_VALUE_PUBLISH{return "published".to_owned()}
        let status=first(&form,b"status");if status=="published"{return "published".to_owned()}if status=="draft"{return "draft".to_owned()}
        if fallback.trim()=="published"{"published".to_owned()}else{"draft".to_owned()}
    }}
}
pub type ResourceSlugifier=Arc<dyn Fn(&InputBytes)->InputBytes+Send+Sync>;
pub fn requested_slug<E:Send+'static,R:Send+Sync+'static>(request:Option<&Request>,title:InputBytes,fallback:InputBytes,slugify:Option<ResourceSlugifier>,form:Option<Arc<ByteFormData>>,policy:MergedFormPolicy)->Effect<InputBytes,E,R>{
    effect_map!{program:requested_form_value(request,"slug",fallback,form,policy),map:move |value|{let raw=if value.trim().is_empty(){title.trim()}else{value.trim()};slugify.as_ref().map(|f|f(&raw)).unwrap_or(raw)}}
}
/// Update supplied initialized views; an absent merged view makes this a no-op.
pub fn set_request_form_value(form:Option<&mut ByteFormData>,post_form:Option<&mut ByteFormData>,key:impl AsRef<[u8]>,value:impl AsRef<[u8]>){
    let Some(form)=form else{return};let key=crate::validation::trim_unicode_whitespace(key.as_ref());if key.is_empty(){return}
    let value=crate::validation::trim_unicode_whitespace(value.as_ref()).to_vec();
    form.values.insert(key.to_vec(),vec![value.clone()]);if let Some(post)=post_form{post.values.insert(key.to_vec(),vec![value]);}
}
pub fn set_request_form_values(mut form:Option<&mut ByteFormData>,mut post_form:Option<&mut ByteFormData>,values:&BTreeMap<InputBytes,InputBytes>){
    for (key,value) in values{set_request_form_value(form.as_deref_mut(),post_form.as_deref_mut(),key,value);}
}
