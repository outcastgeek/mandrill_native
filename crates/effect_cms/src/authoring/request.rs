//! Byte-preserving authoring form admission using the canonical cached input owner.
use std::sync::Arc;
use effect::{Effect, effect_future, effect, effect_sync, effect_try_sync};
use effect_server::{ByteFormData, Request, ServerError};
use crate::commands::InputBytes;
use crate::form_media::{MergedFormPolicy, InitializeForm, initialize_merged_form_inner};
use super::contracts::{AuthoringTranslationField, AuthoringTranslationRequest};

#[derive(Debug)]
pub(crate) struct AuthoringForm {
    values: ByteFormData,
    error: Option<ServerError>,
}
impl AuthoringForm {
    pub(crate) fn first(&self, key: &str) -> InputBytes {
        self.all(key).first().cloned().unwrap_or_default()
    }
    pub(crate) fn all(&self, key: &str) -> Vec<InputBytes> {
        self.values.values.get(key.as_bytes()).map(|values| values.iter().cloned().map(InputBytes::from).collect()).unwrap_or_default()
    }
    pub(crate) fn error(&self) -> Option<&ServerError> { self.error.as_ref() }
}

/// The original native form diagnostic.
#[derive(Debug)]
pub enum AuthoringTranslationRequestError { Form(ServerError) }
impl std::fmt::Display for AuthoringTranslationRequestError {
    fn fmt(&self, f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { match self { Self::Form(error)=>std::fmt::Display::fmt(error,f) } }
}
impl std::error::Error for AuthoringTranslationRequestError {
    fn source(&self)->Option<&(dyn std::error::Error+'static)> { match self { Self::Form(error)=>Some(error) } }
}

pub(crate) fn read_authoring_form<E,R>(request:Request, policy:MergedFormPolicy, form:Option<Arc<ByteFormData>>, mode:InitializeForm)->Effect<AuthoringForm,E,R>
where E:Send+'static,R:Send+Sync+'static {
    effect_future! {
        body: move |_| {
            let request=request.clone();let form=form.clone();
            async move {
                if let Some(form)=form { return AuthoringForm {values:ByteFormData { values:form.values.clone() },error:None}; }
                match initialize_merged_form_inner(&request,policy,mode).await {
                    Ok((values,error))=>AuthoringForm{values,error},
                    Err(error)=>AuthoringForm{values:ByteFormData {values:Default::default()},error:Some(error)},
                }
            }
        },
    }
}

pub(crate) fn parse_authoring_translation_request(form:&AuthoringForm,widget_key_field_name:&str)->AuthoringTranslationRequest {
    let names=form.all("field_name");let values=form.all("field_value");let formats=form.all("field_format");
    AuthoringTranslationRequest {
        scope:form.first("scope").trim(), widget_key:form.first(widget_key_field_name).trim(),
        source_language:form.first("source_language").trim(), target_languages:form.all("target_language"),
        fields:names.into_iter().zip(values).enumerate().map(|(index,(name,value))|AuthoringTranslationField {
            name:name.trim(),value,format:formats.get(index).cloned().unwrap_or_default().trim(),
        }).collect(),
    }
}

/// Parse a request with explicit URL-encoded and multipart limits.
pub fn parse_authoring_translation_request_from_form<R>(request:&Request,widget_key_field_name:&str,policy:MergedFormPolicy)
    ->Effect<AuthoringTranslationRequest,AuthoringTranslationRequestError,R>
where R:Send+Sync+'static {
    parse_authoring_translation_request_with_form(request,widget_key_field_name,policy,None)
}
/// An initialized form bypasses body parsing and retains the caller's exact values.
pub fn parse_authoring_translation_request_with_form<R>(request:&Request,widget_key_field_name:&str,policy:MergedFormPolicy,form:Option<Arc<ByteFormData>>)
    ->Effect<AuthoringTranslationRequest,AuthoringTranslationRequestError,R>
where R:Send+Sync+'static {
    let name=widget_key_field_name.to_owned();
    effect! {
        bind;
        program:read_authoring_form(request.clone(),policy,form,InitializeForm::ParseForm),
        body:move |form| match form.error {
            Some(error)=>effect_try_sync! { once; body: move || Err::<AuthoringTranslationRequest,_>(AuthoringTranslationRequestError::Form(error)), on_error: ::core::convert::identity, },
            None=>{ let parsed=parse_authoring_translation_request(&form,&name);effect_sync! {once;body:move || parsed} },
        },
    }
}
