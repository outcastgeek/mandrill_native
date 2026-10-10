//! D17–D19 typed Identity capability, definition and endpoint lowering.

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{
    Expr, Token,
    ext::IdentExt,
    parse::{Parse, ParseStream},
};

/// One source-ordered field, group operation or selected Effect callback.
struct Entry {
    /// Domain operation/field name retains its caller diagnostic span.
    name: Ident,
    /// Native Rust expressions evaluated once in written order.
    values: Vec<Expr>,
    /// Explicit inline callback parameters and Effect body.
    inline: Option<(Vec<syn::Pat>, syn::Block)>,
}
/// Ordered domain entries preserve configuration evaluation and group ordering.
struct Entries(Vec<Entry>);
impl Parse for Entries {
    /// Parse named fields, call-shaped operations and inline Effect error renderers.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut entries = Vec::new();
        while !input.is_empty() {
            let name: Ident = input.call(Ident::parse_any)?;
            let (values, inline) = if input.peek(Token![:]) {
                input.parse::<Token![:]>()?;
                (vec![input.parse()?], None)
            } else if input.peek(syn::token::Paren) {
                let content;
                syn::parenthesized!(content in input);
                (
                    syn::punctuated::Punctuated::<Expr, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .collect(),
                    None,
                )
            } else {
                let mut parameters = Vec::new();
                if input.peek(Token![|]) {
                    input.parse::<Token![|]>()?;
                    while !input.peek(Token![|]) {
                        parameters.push(input.call(syn::Pat::parse_single)?);
                        if input.peek(Token![,]) {
                            input.parse::<Token![,]>()?;
                        } else {
                            break;
                        }
                    }
                    input.parse::<Token![|]>()?;
                }
                (Vec::new(), Some((parameters, input.parse()?)))
            };
            entries.push(Entry {
                name,
                values,
                inline,
            });
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if input.peek(Token![;]) {
                input.parse::<Token![;]>()?;
            } else if !input.is_empty() && entries.last().is_none_or(|entry| entry.inline.is_none())
            {
                return Err(input.error("expected comma or semicolon after Identity entry"));
            }
        }
        Ok(Self(entries))
    }
}
impl Entry {
    /// Diagnose incompatible inline forms and argument counts at the operation span.
    fn arguments(&self, count: usize) -> syn::Result<&[Expr]> {
        if self.inline.is_some() || self.values.len() != count {
            Err(syn::Error::new(
                self.name.span(),
                format!("{} requires {count} arguments", self.name),
            ))
        } else {
            Ok(&self.values)
        }
    }
    /// Borrow the one selected typed field without changing its Rust semantics.
    fn value(&self) -> syn::Result<&Expr> {
        self.arguments(1)?
            .first()
            .ok_or_else(|| syn::Error::new(self.name.span(), "missing Identity value"))
    }
}
/// Resolve renamed extension dependencies and dispatch the exact required DSL.
pub(crate) fn expand(name: &str, input: TokenStream) -> syn::Result<TokenStream> {
    let path = crate::extension_support::crate_path("effect-server-identity")?;
    let entries = syn::parse2(input)?;
    match name {
        "identity_store" => stores(entries, &path),
        "identity" => definition(entries, &path),
        "identity_endpoints" => endpoints(entries, &path),
        _ => Err(syn::Error::new(Span::call_site(), "unknown Identity macro")),
    }
}
/// Reject unknown fields/operations instead of dynamically dispatching method names.
fn unknown(entry: &Entry) -> syn::Error {
    syn::Error::new(
        entry.name.span(),
        format!("unknown Identity entry: {}", entry.name),
    )
}
/// Retain unique field expressions in declaration order with hygienic local names.
fn fields(
    entries: Vec<Entry>,
    allowed: &[&str],
) -> syn::Result<(Vec<TokenStream>, std::collections::BTreeMap<String, Ident>)> {
    let mut statements = Vec::new();
    let mut selected = std::collections::BTreeMap::new();
    for entry in entries {
        let name = entry.name.to_string();
        if !allowed.contains(&name.as_str()) {
            return Err(unknown(&entry));
        }
        if selected.contains_key(&name) {
            return Err(syn::Error::new(
                entry.name.span(),
                "duplicate Identity field",
            ));
        }
        let value = entry.value()?;
        let local = Ident::new(
            &format!("__identity_field_{}", selected.len()),
            Span::mixed_site(),
        );
        statements.push(quote!(let #local=#value;));
        selected.insert(name, local);
    }
    Ok((statements, selected))
}
/// Require a concrete selection; no macro-invented provider/clock/policy defaults exist.
fn required<'a>(
    fields: &'a std::collections::BTreeMap<String, Ident>,
    name: &str,
) -> syn::Result<&'a Ident> {
    fields.get(name).ok_or_else(|| {
        syn::Error::new(Span::call_site(), format!("missing Identity field: {name}"))
    })
}
/// Lower every F# D17 capability onto one typed configuration-fenced selection bundle.
fn stores(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let capabilities = [
        "accounts",
        "sessions",
        "keys",
        "roles",
        "permissions",
        "challenges",
        "queries",
        "audit",
        "consent",
        "delivery",
        "notifications",
        "jobs",
        "personal_data",
        "native_user_queries",
        "native_role_queries",
    ];
    let mut allowed = capabilities.to_vec();
    allowed.push("backend");
    // Selection methods follow source order; locals evaluate every expression once.
    let order = entries
        .iter()
        .map(|entry| entry.name.to_string())
        .collect::<Vec<_>>();
    let (evaluate, selected) = fields(entries, &allowed)?;
    let backend = required(&selected, "backend")?;
    let mut select = Vec::new();
    for name in order {
        if name == "backend" {
            continue;
        }
        let value = required(&selected, &name)?;
        let method = Ident::new(&format!("with_{name}"), Span::mixed_site());
        select.push(quote!(let __identity_stores=__identity_stores.#method(#value)?;));
    }
    Ok(
        quote!((move||{#(#evaluate)* let __identity_stores=#path::capabilities::IdentityStores::new(#backend);#(#select)* ::core::result::Result::<_,#path::IdentityError>::Ok(__identity_stores)})()),
    )
}
/// Build an inert D18 definition with explicit selected native components/policies.
fn definition(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let policies = [
        "passwords",
        "authenticator",
        "external",
        "passkeys",
        "email_confirmation",
        "browser",
        "renderers",
        "validators",
        "normalizer",
        "principal_factory",
        "tokens",
        "tickets",
        "key_ring",
    ];
    let mut allowed = policies.to_vec();
    allowed.extend([
        "storage",
        "configure_native",
        "errors",
        "clock",
        "grant_policy",
        "admission",
    ]);
    let order = entries
        .iter()
        .map(|entry| entry.name.to_string())
        .collect::<Vec<_>>();
    let (evaluate, selected) = fields(entries, &allowed)?;
    let storage = required(&selected, "storage")?;
    let components = required(&selected, "configure_native")?;
    let errors = required(&selected, "errors")?;
    let clock = required(&selected, "clock")?;
    let grants = required(&selected, "grant_policy")?;
    let admission = required(&selected, "admission")?;
    let mut select = Vec::new();
    for name in order {
        if policies.contains(&name.as_str()) {
            let value = required(&selected, &name)?;
            let method = Ident::new(&format!("with_{name}"), Span::mixed_site());
            select.push(quote!(let __identity_policies=__identity_policies.#method(#value);));
        }
    }
    Ok(
        quote!((move||{#(#evaluate)* let __identity_policies=#path::policies::IdentityPolicies::new();#(#select)* #path::IdentityDefinition::new(#storage,#components,__identity_policies,#errors,#clock,#grants,#admission)})()),
    )
}
/// Assemble required semantic groups/custom typed routes without evaluating request Effects.
fn endpoints(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let mut evaluate = Vec::new();
    let mut operations = Vec::new();
    let mut identity = None;
    let mut prefix = None;
    let mut groups = std::collections::BTreeSet::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let name = entry.name.to_string();
        let local = Ident::new(&format!("__identity_endpoint_{index}"), Span::mixed_site());
        match name.as_str() {
            "identity" | "prefix" => {
                let selected = if name == "identity" {
                    &mut identity
                } else {
                    &mut prefix
                };
                if selected.is_some() {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "duplicate Identity endpoint field",
                    ));
                }
                let value = entry.value()?;
                evaluate.push(quote!(let #local=#value;));
                *selected = Some(local);
            }
            "registration" | "login" | "recovery" | "authenticator" | "passkeys" | "external"
            | "management" | "logout" | "refresh" | "antiforgery" => {
                if !groups.insert(name.clone()) {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "duplicate Identity endpoint group",
                    ));
                }
                let value = entry.value()?;
                evaluate.push(quote!(let #local=#value;));
                let variant = Ident::new(
                    match name.as_str() {
                        "registration" => "Registration",
                        "login" => "Login",
                        "recovery" => "Recovery",
                        "authenticator" => "Authenticator",
                        "passkeys" => "Passkeys",
                        "external" => "External",
                        "management" => "Management",
                        "logout" => "Logout",
                        "refresh" => "Refresh",
                        _ => "Antiforgery",
                    },
                    entry.name.span(),
                );
                operations.push(quote!(let __identity_endpoints=__identity_endpoints.group(#path::endpoints::EndpointGroup::#variant,#local)?;));
            }
            "get" | "post" => {
                let [route, handler] = entry.arguments(2)? else {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        format!("{} requires 2 arguments", entry.name),
                    ));
                };
                evaluate.push(quote!(let #local=(#route,#handler);));
                let method = Ident::new(&name.to_uppercase(), entry.name.span());
                operations.push(quote!(let (__identity_route,__identity_handler)=#local;let __identity_custom=#path::__private::effect_server::mount_route(#path::__private::effect_server::Router::new(),#path::__private::effect_server::HttpMethodSet::new(::std::vec![#path::native::http::Method::#method])?,__identity_route,__identity_handler)?;let __identity_endpoints=__identity_endpoints.include(__identity_custom);));
            }
            "include" | "pipe_through" => {
                let value = entry.value()?;
                evaluate.push(quote!(let #local=#value;));
                let method = &entry.name;
                operations
                    .push(quote!(let __identity_endpoints=__identity_endpoints.#method(#local);));
            }
            "on_error" => {
                let value = if let Some((parameters, body)) = &entry.inline {
                    if parameters.len() != 1 {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "on_error requires one error parameter",
                        ));
                    }
                    let effect =
                        crate::extension_support::effect_body_once(body.stmts.clone(), path);
                    quote!(move|#(#parameters),*|#effect)
                } else {
                    let value = entry.value()?;
                    quote!(#value)
                };
                evaluate.push(quote!(let #local=#value;));
                operations
                    .push(quote!(let __identity_endpoints=__identity_endpoints.on_error(#local);));
            }
            _ => return Err(unknown(&entry)),
        }
    }
    let identity = identity.ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            "missing Identity endpoint field: identity",
        )
    })?;
    let prefix = prefix.ok_or_else(|| {
        syn::Error::new(Span::call_site(), "missing Identity endpoint field: prefix")
    })?;
    Ok(
        quote!((move||{#(#evaluate)* let __identity_endpoints=#path::endpoints::IdentityEndpoints::new(#identity,&#prefix)?;#(#operations)* __identity_endpoints.finish()})()),
    )
}
