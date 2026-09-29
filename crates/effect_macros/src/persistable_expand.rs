//! Expansion of typed persistable record declarations.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, Token, Type};

///
/// # Parameters
/// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
/// - `expected` (`&str`): Value or revision required for the proposed state transition.
///
/// # Returns
/// Type: `syn::Result<()>`.
fn label(input: ParseStream<'_>, expected: &str) -> syn::Result<()> {
    let name: Ident = input.parse()?;
    if name != expected {
        return Err(syn::Error::new(
            name.span(),
            format!("expected `{expected}`"),
        ));
    }
    input.parse::<Token![:]>()?;
    Ok(())
}

/// Typed persistable input used by persistable expand.
pub(crate) struct PersistableInput {
    namespace: Expr,
    request: Type,
    key: Expr,
    success: Expr,
    error: Expr,
    policy: Expr,
}

impl Parse for PersistableInput {
    ///
    /// # Parameters
    /// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
    ///
    /// # Returns
    /// Type: `syn::Result<Self>`.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        label(input, "namespace")?;
        let namespace = input.parse()?;
        input.parse::<Token![,]>()?;
        label(input, "request")?;
        let request = input.parse()?;
        input.parse::<Token![,]>()?;
        label(input, "key")?;
        let key = input.parse()?;
        input.parse::<Token![,]>()?;
        label(input, "success")?;
        let success = input.parse()?;
        input.parse::<Token![,]>()?;
        label(input, "error")?;
        let error = input.parse()?;
        input.parse::<Token![,]>()?;
        label(input, "version")?;
        let policy = input.parse()?;
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after persistable policy"));
        }
        Ok(Self {
            namespace,
            request,
            key,
            success,
            error,
            policy,
        })
    }
}

/// # Parameters
/// - `input` (`PersistableInput`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn expand(input: PersistableInput) -> syn::Result<TokenStream> {
    let effect = crate::expand::effect_crate_path()?;
    let PersistableInput {
        namespace,
        request,
        key,
        success,
        error,
        policy,
    } = input;
    Ok(quote!({
        let __effect_persist_namespace = #namespace;
        let __effect_persist_key = #key;
        let __effect_persist_success = #success;
        let __effect_persist_error = #error;
        let __effect_persist_policy = #policy;
        #effect::persistence::Persistable::<#request, _, _, _>::with_policy(
            __effect_persist_namespace,
            __effect_persist_policy,
            move |request: &#request| {
                let selected = (__effect_persist_key)(request);
                #effect::KeyParts::key_segments(selected)
            },
            __effect_persist_success,
            __effect_persist_error,
        )
    }))
}
