//! Compose existing CMS services and transport-owned replies.

use crate::callback_expand::Input;
use proc_macro2::{Span, TokenStream};
use quote::quote;

pub(crate) fn expand(input: Input) -> syn::Result<TokenStream> {
    let mode = input.mode(
        &[
            "register_routes",
            "definition_services",
            "definition_list_reply",
            "prepared_definition_reply",
            "prepared_search_stream",
            "prepared_resource_search_stream",
        ],
        "",
    )?;
    let mut fields = input.fields;
    let output = match mode.as_str() {
        "definition_services" => {
            let cms = crate::extension_support::crate_path("effect_cms")?;
            let options = fields.take("options")?;
            quote!(#cms::new_definition_service_set(#options))
        }
        "register_routes" => {
            let service = fields.transform("service", |value, _| Ok(quote!(&(#value))))?;
            let scope = fields.take("scope")?;
            let router = fields.take("router")?;
            let mounter = fields.take("mounter")?;
            quote!(#service.register_and_start_routes(#scope, #router, #mounter))
        }
        "definition_list_reply" => {
            let renderer = fields.transform("renderer", |value, _| Ok(quote!(&(#value))))?;
            let services = fields.take("services")?;
            let kind = fields.take("kind")?;
            quote!(#renderer.definition_list_reply(#services, #kind))
        }
        "prepared_definition_reply" => {
            let renderer = fields.transform("renderer", |value, _| Ok(quote!(&(#value))))?;
            let services = fields.take("services")?;
            let prepared = fields.take("prepared")?;
            let kind = fields.take("kind")?;
            let surface = fields.take("surface")?;
            let limits = fields.take("limits")?;
            let on_error = fields.take("on_error")?;
            quote!(#renderer.definition_reply_prepared(#services, #prepared, #kind, #surface, #limits, #on_error))
        }
        "prepared_search_stream" => {
            let cms = crate::extension_support::crate_path("effect_cms")?;
            let patches = fields.take("patches")?;
            let maximum = fields.take("maximum_components")?;
            let on_error = fields.take("on_error")?;
            quote!(#cms::resourcepages::prepared_cms_search_stream(#patches, #maximum, #on_error))
        }
        "prepared_resource_search_stream" => {
            let cms = crate::extension_support::crate_path("effect_cms")?;
            let patches = fields.take("patches")?;
            let frames = fields.take("frames")?;
            let maximum = fields.take("maximum_components")?;
            let on_error = fields.take("on_error")?;
            quote!(#cms::resourcepages::prepared_resource_search_stream(#patches, #frames, #maximum, #on_error))
        }
        _ => {
            return Err(syn::Error::new(
                Span::call_site(),
                "expected a CMS composition mode",
            ));
        }
    };
    fields.finish(output)
}
