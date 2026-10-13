//! D17–D19 typed Identity capability, definition and endpoint lowering.

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{
    Expr, Token,
    ext::IdentExt,
    parse::{Parse, ParseStream},
};
use syn::{braced, parenthesized};

/// One source-ordered field, group operation or selected Effect callback.
struct Entry {
    /// Domain operation/field name retains its caller diagnostic span.
    name: Ident,
    /// Native Rust expressions evaluated once in written order.
    values: Vec<Expr>,
    /// Explicit inline callback parameters and Effect body.
    inline: Option<Expr>,
    /// Nested typed route roles are parsed by this DSL, rather than as a Rust block.
    route: Option<Vec<Entry>>,
}
/// Ordered domain entries preserve configuration evaluation and group ordering.
struct Entries(Vec<Entry>);
impl Parse for Entries {
    /// Parse named fields, call-shaped operations and inline Effect error renderers.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut entries = Vec::new();
        while !input.is_empty() {
            let name: Ident = input.call(Ident::parse_any)?;
            let (values, inline, route) = if input.peek(Token![:]) {
                input.parse::<Token![:]>()?;
                (vec![input.parse()?], None, None)
            } else if input.peek(syn::token::Paren) {
                let content;
                parenthesized!(content in input);
                (
                    syn::punctuated::Punctuated::<Expr, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .collect(),
                    None,
                    None,
                )
            } else if input.peek(syn::token::Brace) {
                let content;
                braced!(content in input);
                let Entries(route) = content.parse()?;
                (Vec::new(), None, Some(route))
            } else {
                (
                    Vec::new(),
                    Some(crate::domain_expand::parse_role(input)?),
                    None,
                )
            };
            entries.push(Entry {
                name,
                values,
                inline,
                route,
            });
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if input.peek(Token![;]) {
                input.parse::<Token![;]>()?;
            } else if !input.is_empty()
                && entries
                    .last()
                    .is_none_or(|entry| entry.inline.is_none() && entry.route.is_none())
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
        if self.inline.is_some() || self.route.is_some() || self.values.len() != count {
            Err(syn::Error::new(
                self.name.span(),
                format!("{} requires {count} arguments", self.name),
            ))
        } else {
            Ok(&self.values)
        }
    }
    /// Use the shared role lowerer or preserve an extracted typed helper expression.
    fn role(&self, arity: usize) -> syn::Result<TokenStream> {
        if let Some(expression) = &self.inline {
            crate::domain_expand::role(
                expression.clone(),
                arity,
                crate::domain_expand::BodyCategory::Native,
            )
        } else {
            let value = self.value()?;
            Ok(quote!(#value))
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
    let entries: Entries = syn::parse2(input)?;
    match name {
        "identity_store" => stores(entries, &path),
        "identity" if entries.0.iter().any(|entry| entry.name == "identity") => {
            action(entries, &path)
        }
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
    path: &TokenStream,
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
        let value = if name == "errors" {
            let callback = entry.role(1)?;
            quote!(#path::__private::error_mapper(#callback))
        } else {
            let value = entry.value()?;
            quote!(#value)
        };
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
    let (evaluate, selected) = fields(entries, &allowed, path)?;
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
        quote!((||{#(#evaluate)* let __identity_stores=#path::capabilities::IdentityStores::new(#backend);#(#select)* ::core::result::Result::<_,#path::IdentityError>::Ok(__identity_stores)})()),
    )
}
/// Build an inert D18 definition with explicit selected native components/policies.
fn definition(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let mut factory = None;
    let mut configuration = Vec::new();
    for entry in entries {
        if entry.name == "native_factory" {
            if factory.is_some() {
                return Err(syn::Error::new(
                    entry.name.span(),
                    "duplicate Identity native factory",
                ));
            }
            let callback = entry.role(3)?;
            if entry.inline.is_some() {
                factory = Some(callback);
            } else {
                factory = Some(quote!(__identity_native_factory));
                configuration.push(entry);
            }
        } else {
            configuration.push(entry);
        }
    }
    let entries = configuration;
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
        "native_factory",
        "errors",
        "clock",
        "grant_policy",
        "admission",
    ]);
    let order = entries
        .iter()
        .map(|entry| entry.name.to_string())
        .collect::<Vec<_>>();
    let (evaluate, selected) = fields(entries, &allowed, path)?;
    let storage = required(&selected, "storage")?;
    let components = selected.get("configure_native");
    if factory.is_some() == components.is_some() {
        return Err(syn::Error::new(
            Span::call_site(),
            "select exactly one of configure_native or native_factory",
        ));
    }
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
    let factory_binding = selected
        .get("native_factory")
        .map(|local| quote!(let __identity_native_factory = #local;));
    let construct = if let Some(factory) = factory {
        quote!(#path::__private::definition_with_factory(#storage, #factory, __identity_policies, #errors, #clock, #grants, #admission))
    } else if let Some(components) = components {
        quote!(#path::IdentityDefinition::new(#storage, #components, __identity_policies, #errors, #clock, #grants, #admission))
    } else {
        return Err(syn::Error::new(
            Span::call_site(),
            "missing Identity native configuration",
        ));
    };
    Ok(
        quote!((|| { #(#evaluate)* #factory_binding let __identity_policies = #path::policies::IdentityPolicies::new(); #(#select)* #construct })()),
    )
}
/// Lower a native Result role through the shared grammar, or retain its typed helper.
fn future_role(entry: &Entry, arity: usize) -> syn::Result<TokenStream> {
    if let Some(expression) = &entry.inline {
        crate::domain_expand::role(
            expression.clone(),
            arity,
            crate::domain_expand::BodyCategory::NativeFuture,
        )
    } else {
        let value = entry.value()?;
        Ok(quote!(#value))
    }
}
/// Named native async roles and their checked parameter counts.
type CallbackRoles = &'static [(&'static str, usize)];
/// One existing method, ordered input count and its native callback contract.
type CallbackOperation = (&'static str, usize, CallbackRoles);
/// Finite callback-bearing Identity operations retain their actual typed native owners.
/// Ordinary Effect-returning workflows compose directly in Effect bodies; these forms
/// remove native async callback boilerplate without inventing another runtime or grant.
fn action(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let operations: &[CallbackOperation] = &[
        ("authorize_once", 1, &[("authorize", 3)]),
        ("authorization_stage", 1, &[]),
        ("authorization_guard", 1, &[("guard", 2)]),
        ("invoke", 0, &[("native", 4)]),
        ("invoke_once", 1, &[("native", 4)]),
        ("invoke_capability_once", 1, &[("native", 5)]),
        ("native_http", 1, &[("native", 4)]),
        ("accounts_page", 3, &[("decide", 3), ("allow", 2)]),
        ("roles_page", 3, &[("decide", 3), ("allow", 2)]),
        ("administer_account", 4, &[("decide", 3), ("operation", 3)]),
        ("administer_role", 3, &[("decide", 3), ("operation", 3)]),
        ("create_role", 4, &[("decide", 3)]),
        ("rename_role", 2, &[("decide", 3)]),
        ("delete_role", 1, &[("decide", 3)]),
        ("add_role_claim", 2, &[("decide", 3)]),
        ("remove_role_claim", 2, &[("decide", 3)]),
        ("add_account_to_role", 5, &[("decide", 3)]),
        ("remove_account_from_role", 5, &[("decide", 3)]),
        ("mutate_account_snapshot", 2, &[("transform", 3)]),
        ("deliver", 4, &[("transport", 1)]),
        ("notify", 4, &[("dispatch", 1)]),
        ("request_personal_data_job", 4, &[("accept", 1)]),
    ];
    let selected = entries
        .iter()
        .filter_map(|entry| operations.iter().find(|(name, _, _)| entry.name == *name))
        .collect::<Vec<_>>();
    let [selected] = selected.as_slice() else {
        return Err(syn::Error::new(
            Span::call_site(),
            "select exactly one Identity callback operation",
        ));
    };
    let (name, count, roles) = **selected;
    if matches!(name, "invoke_once" | "invoke_capability_once") {
        return invoke_action(entries, path, name);
    }
    let mut evaluate = Vec::new();
    let mut identity = None;
    let mut arguments = Vec::new();
    let mut callbacks = std::collections::BTreeMap::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let local = Ident::new(&format!("__identity_action_{index}"), Span::mixed_site());
        if entry.name == "identity" {
            if identity.is_some() {
                return Err(syn::Error::new(
                    entry.name.span(),
                    "duplicate Identity action definition",
                ));
            }
            let value = entry.value()?;
            evaluate.push(quote!(let #local = #value;));
            identity = Some(local);
        } else if entry.name == name {
            for (position, value) in entry.arguments(count)?.iter().enumerate() {
                let argument = Ident::new(
                    &format!("__identity_argument_{position}"),
                    Span::mixed_site(),
                );
                evaluate.push(quote!(let #argument = #value;));
                arguments.push(argument);
            }
        } else if let Some((role, arity)) = roles.iter().find(|(role, _)| entry.name == *role) {
            if callbacks.contains_key(*role) {
                return Err(syn::Error::new(
                    entry.name.span(),
                    "duplicate Identity callback role",
                ));
            }
            let callback = if name == "authorization_guard" {
                let value = if let Some(expression) = &entry.inline {
                    crate::domain_expand::role(
                        expression.clone(),
                        *arity,
                        crate::domain_expand::BodyCategory::Effect,
                    )?
                } else {
                    let value = entry.value()?;
                    quote!(#value)
                };
                quote!({
                    fn __identity_guard<D, E, R, F>(callback: F) -> F
                    where D: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static,
                        F: Fn(::std::sync::Arc<D>, #path::__private::effect_server::Request) -> #path::__private::effect::Effect<(), E, R> + Send + Sync + 'static {
                        callback
                    }
                    __identity_guard(#value)
                })
            } else {
                future_role(&entry, *arity)?
            };
            if entry.inline.is_some() {
                callbacks.insert(*role, callback);
            } else {
                evaluate.push(quote!(let #local = #callback;));
                callbacks.insert(*role, quote!(#local));
            }
        } else {
            return Err(unknown(&entry));
        }
    }
    let identity = identity
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing Identity action definition"))?;
    let callbacks = roles
        .iter()
        .map(|(role, _)| {
            callbacks.remove(role).ok_or_else(|| {
                syn::Error::new(
                    Span::call_site(),
                    format!("missing Identity callback: {role}"),
                )
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let method = Ident::new(name, Span::mixed_site());
    if name == "authorization_guard" {
        return Ok(
            quote!({#(#evaluate)* #path::endpoints::authorized_guard(#identity.authorization_stage(#(#arguments),*), #(#callbacks),*)}),
        );
    }
    Ok(quote!({ #(#evaluate)* #identity.#method(#(#arguments,)* #(#callbacks),*) }))
}

/// Type native invocation callbacks against the selected definition before
/// evaluating their bodies, retaining every expression's written argument order.
fn invoke_action(
    entries: Vec<Entry>,
    path: &TokenStream,
    method: &str,
) -> syn::Result<TokenStream> {
    // Canonical field order already reaches the native typed callback argument.
    // Keep the ordered helper below for callbacks written before their definition.
    if let [definition, input, callback] = entries.as_slice()
        && definition.name == "identity"
        && input.name == method
        && callback.name == "native"
    {
        let definition_value = definition.value()?;
        let input_values = input.arguments(1)?;
        let input_value = input_values.first().ok_or_else(|| {
            syn::Error::new(input.name.span(), "missing Identity invocation input")
        })?;
        let callback = future_role(
            callback,
            if method == "invoke_capability_once" {
                5
            } else {
                4
            },
        )?;
        let definition = Ident::new("__identity_definition", Span::mixed_site());
        let input = Ident::new("__identity_input", Span::mixed_site());
        let method = Ident::new(method, Span::mixed_site());
        return Ok(quote!({
            let #definition = &(#definition_value);
            let #input = #input_value;
            #definition.#method(#input, #callback)
        }));
    }
    let mut parameters = Vec::new();
    let mut values = Vec::new();
    let mut identity = None;
    let mut input = None;
    let mut native = None;
    let capability = method == "invoke_capability_once";
    for (index, entry) in entries.into_iter().enumerate() {
        let local = Ident::new(&format!("__identity_value_{index}"), Span::mixed_site());
        let (slot, ty, value) = if entry.name == "identity" {
            let value = entry.value()?;
            (
                &mut identity,
                quote!(&#path::IdentityDefinition<S, C, P, M>),
                quote!(&(#value)),
            )
        } else if entry.name == method {
            let arguments = entry.arguments(1)?;
            let value = arguments.first().ok_or_else(|| {
                syn::Error::new(entry.name.span(), "missing Identity invocation input")
            })?;
            (&mut input, quote!(I), quote!(#value))
        } else if entry.name == "native" {
            let value = future_role(&entry, if capability { 5 } else { 4 })?;
            (&mut native, quote!(F), value)
        } else {
            return Err(unknown(&entry));
        };
        if slot.replace(local.clone()).is_some() {
            return Err(syn::Error::new(
                entry.name.span(),
                "duplicate Identity invocation field",
            ));
        }
        parameters.push(quote!(#local: #ty));
        values.push(value);
    }
    let identity = identity
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing Identity action definition"))?;
    let input = input
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing Identity invocation input"))?;
    let native = native
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing Identity callback: native"))?;
    let storage = if capability {
        quote!(::std::sync::Arc<S>,)
    } else {
        quote!()
    };
    let method = Ident::new(method, Span::mixed_site());
    Ok(quote!({
        fn __identity_invoke<S, C, P, M, I, A, E, R, F, Pending>(#(#parameters),*) -> #path::__private::effect::Effect<A, E, R>
        where S: #path::capabilities::StoreSelection, C: #path::IdentityComponents<S::Backend, P>,
            P: Send + Sync + 'static, M: Fn(#path::IdentityError) -> E + Send + Sync + 'static,
            I: Send + 'static, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
            F: Fn(#storage C::Native, I, #path::authority::IdentityAuthority<S::Backend>, ::std::sync::Arc<P>) -> Pending + Send + Sync + 'static,
            Pending: ::core::future::Future<Output = ::core::result::Result<A, #path::IdentityError>> + Send + 'static {
            #identity.#method(#input, #native)
        }
        __identity_invoke(#(#values),*)
    }))
}
/// Parse route braces as the common ordered Identity entries without interpreting Rust bodies.
fn route_entries(
    entries: Vec<Entry>,
    kind: TokenStream,
    path: &TokenStream,
    route_index: usize,
) -> syn::Result<(Vec<TokenStream>, TokenStream)> {
    let mut evaluate = Vec::new();
    let mut fields = std::collections::BTreeMap::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let name = entry.name.to_string();
        if fields.contains_key(&name) {
            return Err(syn::Error::new(
                entry.name.span(),
                "duplicate Identity route field",
            ));
        }
        let (arity, effect) = match name.as_str() {
            "method" | "path" | "policy" => (0, false),
            "input" | "render" => (1, true),
            "workflow" | "guard" => (2, true),
            _ => return Err(unknown(&entry)),
        };
        let value = if effect && entry.inline.is_some() {
            let expression = entry.inline.as_ref().ok_or_else(|| unknown(&entry))?;
            let _ = crate::domain_expand::role(
                expression.clone(),
                arity,
                crate::domain_expand::BodyCategory::Native,
            )?;
            let Expr::Closure(closure) = expression else {
                return Err(unknown(&entry));
            };
            let Expr::Block(body) = closure.body.as_ref() else {
                return Err(unknown(&entry));
            };
            let inputs = &closure.inputs;
            let (prefix, statements) =
                crate::context_expand::split_prefix(body.block.stmts.clone());
            let body = crate::extension_support::effect_body_once(statements, path);
            quote!(move |#inputs| { #(#prefix)* #body })
        } else {
            let value = entry.value()?;
            quote!(#value)
        };
        if effect && entry.inline.is_some() {
            fields.insert(name, value);
        } else {
            let local = Ident::new(
                &format!("__identity_route_{route_index}_field_{index}"),
                Span::mixed_site(),
            );
            evaluate.push(quote!(let #local = #value;));
            fields.insert(name, quote!(#local));
        }
    }
    let method = fields
        .remove("method")
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing Identity route method"))?;
    let route = fields
        .remove("path")
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing Identity route path"))?;
    let input = fields
        .remove("input")
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing Identity route input"))?;
    let workflow = fields
        .remove("workflow")
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing Identity route workflow"))?;
    let render = fields
        .remove("render")
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing Identity route render"))?;
    let guard = fields
        .remove("guard")
        .unwrap_or_else(|| quote!(|_, _| #path::__private::effect::Effect::sync(|| ())));
    let guard = if let Some(policy) = fields.remove("policy") {
        quote!({
            let __identity_authorize = __identity_definition.authorization_stage(#policy);
            #path::endpoints::authorized_guard(__identity_authorize, #guard)
        })
    } else {
        guard
    };
    Ok((
        evaluate,
        quote!((__identity_endpoints).workflow_route(#kind, (#method, (#route).into()), #guard, #input, #workflow, #render)?),
    ))
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
                let inline = entry.inline.is_some();
                let value = if entry.route.is_some() {
                    TokenStream::new()
                } else {
                    entry.role(1)?
                };
                if !inline && entry.route.is_none() {
                    evaluate.push(quote!(let #local=#value;));
                }
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
                if let Some(route) = entry.route {
                    let (initializers, route) = route_entries(
                        route,
                        quote!(::core::option::Option::Some(#path::endpoints::EndpointGroup::#variant)),
                        path,
                        index,
                    )?;
                    evaluate.extend(initializers);
                    operations.push(quote!(let __identity_endpoints = #route;));
                } else if inline {
                    operations.push(quote!(let __identity_endpoints = __identity_endpoints.group_with(#path::endpoints::EndpointGroup::#variant, #value)?;));
                } else {
                    operations.push(quote!(let __identity_endpoints=__identity_endpoints.group(#path::endpoints::EndpointGroup::#variant,#local)?;));
                }
            }
            "route" => {
                let route = entry.route.ok_or_else(|| {
                    syn::Error::new(
                        entry.name.span(),
                        "route requires braced input/workflow/render roles",
                    )
                })?;
                let (initializers, route) =
                    route_entries(route, quote!(::core::option::Option::None), path, index)?;
                evaluate.extend(initializers);
                operations.push(quote!(let __identity_endpoints = #route;));
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
            "protect" => {
                let [policy, routes] = entry.arguments(2)? else {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "protect requires a native policy and a Router",
                    ));
                };
                evaluate.push(quote!(let #local = (#policy, #routes);));
                operations.push(quote! {
                    let (__identity_policy, __identity_routes) = #local;
                    let __identity_protected = #path::__private::effect_server::Router::new().pipe_through(
                        #path::__private::effect_server::Pipeline::new().before(
                            __identity_definition.authorization_stage(__identity_policy)
                        )
                    ).include(__identity_routes);
                    let __identity_endpoints = __identity_endpoints.include(__identity_protected);
                });
            }
            "include" | "pipe_through" => {
                let value = entry.value()?;
                evaluate.push(quote!(let #local=#value;));
                let method = Ident::new(
                    &entry.name.to_string(),
                    entry.name.span().resolved_at(Span::mixed_site()),
                );
                operations
                    .push(quote!(let __identity_endpoints=__identity_endpoints.#method(#local);));
            }
            "on_error" => {
                let value = if let Some(expression) = &entry.inline {
                    // Shared validation retains typed patterns; the extension path keeps
                    // owned callback lowering usable through renamed façade dependencies.
                    let _validated = entry.role(1)?;
                    let Expr::Closure(closure) = expression else {
                        return Err(syn::Error::new_spanned(
                            expression,
                            "on_error requires one error parameter",
                        ));
                    };
                    let Expr::Block(body) = closure.body.as_ref() else {
                        return Err(syn::Error::new_spanned(
                            expression,
                            "on_error body must be braced",
                        ));
                    };
                    let parameters = &closure.inputs;
                    let effect =
                        crate::extension_support::effect_body_once(body.block.stmts.clone(), path);
                    quote!(move |#parameters| #effect)
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
        quote!((||{#(#evaluate)* let __identity_endpoints=#path::endpoints::IdentityEndpoints::new(#identity,&#prefix)?;let __identity_definition=__identity_endpoints.definition();#(#operations)* __identity_endpoints.finish()})()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Ambiguous operations and malformed callback roles must fail before Rust execution.
    #[test]
    fn callback_operation_diagnostics() -> syn::Result<()> {
        let path = quote!(::identity);
        for source in [
            quote!(identity: selected, invoke(), invoke_once(value), native |a,b,c,d| { Ok(()) }),
            quote!(identity: selected, accounts_page(scope, None, limit), decide |a,b| { Ok(()) }, allow |a,b| { Ok(true) }),
            quote!(identity: selected, deliver(actor, id, None, request), transport |a| { Ok(()) }, transport: helper),
        ] {
            assert!(action(syn::parse2(source)?, &path).is_err());
        }
        Ok(())
    }
    /// Missing guards stay explicit no-op callbacks; malformed route roles never silently default.
    #[test]
    fn endpoint_role_diagnostics() -> syn::Result<()> {
        let path = quote!(::identity);
        let entries = syn::parse2(quote!(identity: selected, prefix: "/", route {
            method: Method::POST, path: "/collect", input |request| { request },
            guard |identity| { () }, workflow |identity,input| { input }, render |output| { output },
        }))?;
        assert!(endpoints(entries, &path).is_err());
        let entries =
            syn::parse2(quote!(identity: selected, prefix: "/", login: helper, login: other))?;
        assert!(endpoints(entries, &path).is_err());
        Ok(())
    }
}
