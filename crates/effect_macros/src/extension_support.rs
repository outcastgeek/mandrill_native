//! Hygienic crate resolution shared by the five optional extension DSLs.

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;

/// Resolve an extension's Cargo package through the consumer's dependency alias.
/// Each extension declares `extern crate self as its_underscore_name`, allowing
/// the same expansion in its own documentation and in renamed downstream crates.
pub(crate) fn crate_path(package: &str) -> syn::Result<TokenStream> {
    let name =
        match crate_name(package).map_err(|error| syn::Error::new(Span::call_site(), error))? {
            FoundCrate::Itself => package.replace('-', "_"),
            FoundCrate::Name(name) => name,
        };
    let ident = Ident::new(&name, Span::call_site());
    Ok(quote!(::#ident))
}

/// Lower an ordinary Effect body through the extension's hidden Effect re-export.
/// The consumer need not declare an additional direct Effect dependency. Domain
/// binders instead use `context_expand` where their own scoped context is needed.
pub(crate) fn effect_body(
    statements: Vec<syn::Stmt>,
    extension: &TokenStream,
) -> syn::Result<TokenStream> {
    crate::expand::expand_with_path(statements, quote!(#extension::__private::effect))
}

/// Lower one callback invocation's owned inputs through the existing one-shot binder.
/// The enclosing callback remains reusable and creates fresh invocation inputs;
/// the returned Effect consumes those inputs once without adding Clone bounds.
/// Use this for owned error/loader/request callback parameters, while ordinary
/// reusable `body` descriptions keep `effect_body` and its existing semantics.
pub(crate) fn effect_body_once(statements: Vec<syn::Stmt>, extension: &TokenStream) -> TokenStream {
    let private = quote!(#extension::__private::effect::__private);
    crate::async_expand::expand_once(statements, &private)
}
