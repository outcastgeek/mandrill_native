//! Byte-preserving authoring form admission using the canonical cached input owner.
use super::contracts::{AuthoringTranslationField, AuthoringTranslationRequest};
use crate::commands::InputBytes;
use crate::form_media::{InitializeForm, MergedFormPolicy, initialize_merged_form_inner};
use crate::forms::{PreparedMergedForm, PreparedMergedForms};
use effect::{Effect, effect, effect_future, effect_sync, effect_try_sync};
use effect_server::{ByteFormData, Request, ServerError};
use std::sync::Arc;

pub(crate) enum AuthoringForm {
    Ordinary {
        values: ByteFormData,
        error: Option<ServerError>,
    },
    Prepared(PreparedMergedForm),
}
impl AuthoringForm {
    pub(crate) fn first(&self, key: &str) -> InputBytes {
        self.values(key.as_bytes())
            .next()
            .map(InputBytes::from)
            .unwrap_or_default()
    }
    pub(crate) fn error(&self) -> Option<&ServerError> {
        match self {
            Self::Ordinary { error, .. } => error.as_ref(),
            Self::Prepared(form) => form.diagnostic(),
        }
    }

    fn take_diagnostic(&mut self) -> Option<ServerError> {
        match self {
            Self::Ordinary { error, .. } => error.take(),
            Self::Prepared(form) => form.take_diagnostic(),
        }
    }
}

trait AuthoringValues {
    fn values<'a>(&'a self, key: &'a [u8]) -> impl Iterator<Item = &'a [u8]> + 'a;
}
impl AuthoringValues for AuthoringForm {
    fn values<'a>(&'a self, key: &'a [u8]) -> impl Iterator<Item = &'a [u8]> + 'a {
        let ordinary = match self {
            Self::Ordinary { values, .. } => values.values.get(key),
            Self::Prepared(_) => None,
        };
        let prepared = match self {
            Self::Ordinary { .. } => None,
            Self::Prepared(form) => Some(form.values(key)),
        };
        ordinary
            .into_iter()
            .flatten()
            .map(Vec::as_slice)
            .chain(prepared.into_iter().flatten())
    }
}
impl AuthoringValues for PreparedMergedForm {
    fn values<'a>(&'a self, key: &'a [u8]) -> impl Iterator<Item = &'a [u8]> + 'a {
        self.values(key)
    }
}

/// The original native form diagnostic.
#[derive(Debug)]
pub enum AuthoringTranslationRequestError {
    Form(ServerError),
}
impl std::fmt::Display for AuthoringTranslationRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Form(error) => std::fmt::Display::fmt(error, f),
        }
    }
}
impl std::error::Error for AuthoringTranslationRequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Form(error) => Some(error),
        }
    }
}

pub(crate) fn read_authoring_form<E, R>(
    request: Request,
    policy: MergedFormPolicy,
    form: Option<Arc<ByteFormData>>,
    mode: InitializeForm,
) -> Effect<AuthoringForm, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    effect_future! {
        body: move |_| {
            let request=request.clone();let form=form.clone();
            async move {
                if let Some(form)=form { return AuthoringForm::Ordinary {values:ByteFormData { values:form.values.clone() },error:None}; }
                match initialize_merged_form_inner(&request,policy,mode).await {
                    Ok((values,error))=>AuthoringForm::Ordinary {values,error},
                    Err(error)=>AuthoringForm::Ordinary {values:ByteFormData {values:Default::default()},error:Some(error)},
                }
            }
        },
    }
}

pub(crate) fn parse_authoring_translation_request(
    form: &AuthoringForm,
    widget_key_field_name: &str,
) -> AuthoringTranslationRequest {
    project(form, widget_key_field_name)
}

fn project(form: &impl AuthoringValues, widget_key: &str) -> AuthoringTranslationRequest {
    let first_trimmed = |key: &[u8]| {
        form.values(key)
            .next()
            .map(crate::validation::trim_unicode_whitespace)
            .map(InputBytes::from)
            .unwrap_or_default()
    };
    let mut formats = form.values(b"field_format");
    AuthoringTranslationRequest {
        scope: first_trimmed(b"scope"),
        widget_key: first_trimmed(widget_key.as_bytes()),
        source_language: first_trimmed(b"source_language"),
        target_languages: form
            .values(b"target_language")
            .map(InputBytes::from)
            .collect(),
        fields: form
            .values(b"field_name")
            .zip(form.values(b"field_value"))
            .map(|(name, value)| AuthoringTranslationField {
                name: InputBytes::from(crate::validation::trim_unicode_whitespace(name)),
                value: InputBytes::from(value),
                format: formats
                    .next()
                    .map(crate::validation::trim_unicode_whitespace)
                    .map(InputBytes::from)
                    .unwrap_or_default(),
            })
            .collect(),
    }
}

/// Admit and parse the authoring form through the selected merged form storage.
/// Projection copies only the bounded fields forming the existing caller-owned DTO.
pub fn parse_authoring_translation_request_prepared<R: Send + Sync + 'static>(
    request: &Request,
    widget_key_field_name: Arc<str>,
    policy: MergedFormPolicy,
    storage: &Arc<PreparedMergedForms>,
) -> Effect<AuthoringTranslationRequest, AuthoringTranslationRequestError, R> {
    let form = storage.read(
        request,
        policy,
        InitializeForm::ParseForm,
        AuthoringTranslationRequestError::Form,
    );
    effect! {
        bind;
        program: form,
        body: move |form: PreparedMergedForm| {
            let name = widget_key_field_name.clone();
            effect_try_sync! {
                once;
                body: move || project_prepared_authoring_request(form, &name),
            }
        },
    }
}

/// Project an already admitted form into the existing consumer request.
pub fn project_prepared_authoring_request(
    mut form: PreparedMergedForm,
    widget_key_field_name: &str,
) -> Result<AuthoringTranslationRequest, AuthoringTranslationRequestError> {
    match form.take_diagnostic() {
        Some(error) => Err(AuthoringTranslationRequestError::Form(error)),
        None => Ok(project(&form, widget_key_field_name)),
    }
}

/// Parse a request with explicit URL-encoded and multipart limits.
pub fn parse_authoring_translation_request_from_form<R>(
    request: &Request,
    widget_key_field_name: &str,
    policy: MergedFormPolicy,
) -> Effect<AuthoringTranslationRequest, AuthoringTranslationRequestError, R>
where
    R: Send + Sync + 'static,
{
    parse_authoring_translation_request_with_form(request, widget_key_field_name, policy, None)
}
/// An initialized form bypasses body parsing and retains the caller's exact values.
pub fn parse_authoring_translation_request_with_form<R>(
    request: &Request,
    widget_key_field_name: &str,
    policy: MergedFormPolicy,
    form: Option<Arc<ByteFormData>>,
) -> Effect<AuthoringTranslationRequest, AuthoringTranslationRequestError, R>
where
    R: Send + Sync + 'static,
{
    let name = widget_key_field_name.to_owned();
    effect! {
        bind;
        program:read_authoring_form(request.clone(),policy,form,InitializeForm::ParseForm),
        body:move |mut form: AuthoringForm| match form.take_diagnostic() {
            Some(error)=>effect_try_sync! { once; body: move || Err::<AuthoringTranslationRequest,_>(AuthoringTranslationRequestError::Form(error)), },
            None=>{ let parsed=parse_authoring_translation_request(&form,&name);effect_sync! {once;body:move || parsed} },
        },
    }
}
