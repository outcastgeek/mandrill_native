//! Source media templates rendered through Maud and the shared component library.
use effect_server::Request;
use maud::{html, Markup};
use crate::localization::TranslateKeyFunc;
use web_components::components::shared;
use web_components::components::unstyled::{button,form};
use super::*;

/// Request-scoped localization shared by the media templates.
#[derive(Clone, Copy, Default)]
pub struct MediaRenderContext<'a> {
    pub request: Option<&'a Request>,
    pub translate: Option<&'a TranslateKeyFunc<Request>>,
    pub translate_args: Option<&'a (dyn Fn(&str, &[String]) -> String + Send + Sync)>,
}
impl MediaRenderContext<'_> {
    pub fn t_args(self,key:&str,args:&[String])->String { self.translate_args.map(|t| t(key,args)).unwrap_or_else(||self.t(key)) }
    pub fn t(self, key: &str) -> String {
        self.request.map(|r| media_translate(self.translate,r,key)).unwrap_or_else(|| key.to_owned())
    }
}
pub(super) fn attrs(values: &[(&str,&str)]) -> std::collections::BTreeMap<String,String> {
    values.iter().map(|(k,v)| ((*k).to_owned(),(*v).to_owned())).collect()
}
pub(super) fn first<'a>(values:&[&'a str])-> &'a str { values.iter().map(|v|v.trim()).find(|v|!v.is_empty()).unwrap_or("") }
pub(super) fn text(value: impl AsRef<[u8]>) -> String { crate::byte_text::encoding_text(value.as_ref()) }
pub(super) fn media_button(label: Markup, kind: &'static str, variant: &'static str, class: &str, attributes: std::collections::BTreeMap<String,String>, disabled: bool) -> Markup {
    button::Button(&button::ButtonProps { button_type:kind, variant, size:shared::SIZE_SM, class:class.into(), attributes,disabled,..Default::default()},label)
}
pub(super) fn hidden(name:&str,value:&str)->Markup { html!{ input type="hidden" name=(name) value=(value); } }
pub(super) fn get_form(url:&str,class:&str,id:&str,children:Markup)->Markup {
    form::Form(&form::FormProps {action:url.into(),method:"GET".into(),datastar_get:url.into(),class:class.into(),attributes:attrs(&[("id",id)]),..Default::default()},children)
}
pub(super) fn post_form(url:&str,class:&str,id:&str,token:&str,field:&str,children:Markup)->Markup {
    form::FormWithCSRF(&form::FormProps {action:url.into(),method:"POST".into(),datastar_post:url.into(),class:class.into(),attributes:attrs(&[("id",id)]),..Default::default()},token,field,children)
}
