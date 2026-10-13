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
    native_key: bool,
    namespace: Expr,
    request: Type,
    key: Expr,
    success: Expr,
    error: Expr,
    policy: Expr,
    limits: Option<Expr>,
}

impl Parse for PersistableInput {
    ///
    /// # Parameters
    /// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
    ///
    /// # Returns
    /// Type: `syn::Result<Self>`.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let native_key = if input.peek(Ident)
            && input
                .fork()
                .parse::<Ident>()
                .is_ok_and(|name| name == "native")
        {
            input.parse::<Ident>()?;
            input.parse::<Token![;]>()?;
            true
        } else {
            false
        };
        label(input, "namespace")?;
        let namespace = input.parse()?;
        input.parse::<Token![,]>()?;
        label(input, "request")?;
        let request = input.parse()?;
        input.parse::<Token![,]>()?;
        let native_version = if native_key {
            label(input, "version")?;
            let version = input.parse::<Expr>()?;
            input.parse::<Token![,]>()?;
            Some(version)
        } else {
            None
        };
        label(input, "key")?;
        let key = input.parse()?;
        input.parse::<Token![,]>()?;
        label(input, "success")?;
        let success = input.parse()?;
        input.parse::<Token![,]>()?;
        label(input, "error")?;
        let error = input.parse()?;
        input.parse::<Token![,]>()?;
        let (policy, limits) = if let Some(version) = native_version {
            label(input, "limits")?;
            (version, Some(input.parse()?))
        } else {
            label(input, "version")?;
            (input.parse()?, None)
        };
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after persistable policy"));
        }
        Ok(Self {
            native_key,
            namespace,
            request,
            key,
            success,
            error,
            policy,
            limits,
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
        native_key,
        namespace,
        request,
        key,
        success,
        error,
        policy,
        limits,
    } = input;
    let key = if native_key {
        quote!({
            fn __effect_key<I, F>(key: F) -> F
            where F: Fn(&I) -> ::core::result::Result<::std::vec::Vec<#effect::KeySegment>, #effect::KeyError>
                + Send + Sync + 'static
            { key }
            __effect_key::<#request, _>(#key)
        })
    } else {
        quote!(#key)
    };
    let selected_key = if native_key {
        quote!(__effect_persist_key)
    } else {
        quote!(move |request: &#request| {
            let selected = (__effect_persist_key)(request);
            #effect::KeyParts::key_segments(selected)
        })
    };
    let construct = if let Some(limits) = limits {
        quote!({
            let __effect_persist_limits = #limits;
            #effect::persistence::Persistable::<#request, _, _, _>::new(
                __effect_persist_namespace, __effect_persist_policy, #selected_key,
                __effect_persist_success, __effect_persist_error, __effect_persist_limits,
            )
        })
    } else {
        quote!(#effect::persistence::Persistable::<#request, _, _, _>::with_policy(
            __effect_persist_namespace, __effect_persist_policy, #selected_key,
            __effect_persist_success, __effect_persist_error,
        ))
    };
    let native_version = native_key.then(|| quote!(let __effect_persist_policy = #policy;));
    let record_policy = (!native_key).then(|| quote!(let __effect_persist_policy = #policy;));
    Ok(quote!({
        let __effect_persist_namespace = #namespace;
        #native_version
        let __effect_persist_key = #key;
        let __effect_persist_success = #success;
        let __effect_persist_error = #error;
        #record_policy
        #construct
    }))
}
