//! OpenTelemetry convenience lowering onto the existing typed Effect span path.

use crate::domain_expand::Fields;
use proc_macro2::{Span, TokenStream};
use quote::quote;

/// Validate D01 fields and lower inline binds using the extension's Effect export.
///
/// Configuration expressions evaluate once in written order. `body { ... }`
/// and `effect: helper()` are mutually exclusive; attributes/links remain typed
/// native vectors and are emitted only when the lazy span executes.
pub(crate) fn expand(name: &str, input: TokenStream) -> syn::Result<TokenStream> {
    if name != "otel" {
        return Err(syn::Error::new(
            Span::call_site(),
            "unknown OpenTelemetry macro",
        ));
    }
    let path = crate::extension_support::crate_path("effect-otel")?;
    let mut fields: Fields = syn::parse2(input)?;
    let name = fields.take("name")?;
    let body = match (fields.contains("body"), fields.contains("effect")) {
        (true, false) => fields.transform("body", |expr, _| {
            let syn::Expr::Block(block) = expr else {
                return Err(syn::Error::new_spanned(expr, "otel body must be braced"));
            };
            crate::extension_support::effect_body(block.block.stmts, &path)
        })?,
        (false, true) => fields.take("effect")?,
        _ => {
            return Err(syn::Error::new(
                Span::call_site(),
                "otel requires exactly one of body or effect",
            ));
        }
    };
    let kind = if fields.contains("kind") {
        fields.take("kind")?
    } else {
        quote!(#path::SpanKind::Internal)
    };
    let parent = if fields.contains("parent") {
        let parent = fields.take("parent")?;
        quote!(::core::option::Option::Some(#parent))
    } else {
        quote!(::core::option::Option::None)
    };
    let attributes = if fields.contains("attributes") {
        fields.take("attributes")?
    } else {
        quote!(::std::vec::Vec::new())
    };
    let links = if fields.contains("links") {
        fields.take("links")?
    } else {
        quote!(::std::vec::Vec::new())
    };
    fields.finish(quote!(#path::Otel::span(#name, #path::SpanOptions { kind: #kind, parent: #parent, attributes: #attributes, links: #links }, #body)))
}
