//! D20–D27 lowering over real Server/core owners and the existing Effect grammar.
//!
//! Descriptor literals are checked during expansion; dynamic policies retain
//! typed runtime validation. Inline callbacks use the existing one-shot interpreter
//! binder for each fresh invocation, retaining reusable outer descriptions without
//! cloning owned payloads. Explicit shared inputs retain their original owner.

use proc_macro2::{Ident, Span, TokenStream};
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    Expr, ExprClosure, Token,
    ext::IdentExt,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};
use syn::{parenthesized, parse_quote};

/// One inert named field, ordered call or explicit inline callback.
enum Entry {
    /// Named typed configuration expression, including extracted callback functions.
    Field(Ident, Expr),
    /// Ordered repeated call-shaped descriptor operation.
    Call(Ident, Vec<Expr>),
    /// Domain callback syntax without a colon; only effectful labels lower Effects.
    Inline(Ident, ExprClosure),
}
/// Ordered syntax with original identifiers/spans preserved for diagnostics.
struct Description(Vec<Entry>);
impl Parse for Description {
    /// Parse mixed fields/calls/inline callbacks without interpreting application values.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut entries = Vec::new();
        while !input.is_empty() {
            let name = Ident::parse_any(input)?;
            if input.peek(Token![:]) {
                input.parse::<Token![:]>()?;
                entries.push(Entry::Field(name, input.parse()?));
                if !input.is_empty() {
                    if input.peek(Token![,]) {
                        input.parse::<Token![,]>()?;
                    } else {
                        input.parse::<Token![;]>()?;
                    }
                }
            } else if input.peek(syn::token::Paren) {
                let values;
                parenthesized!(values in input);
                entries.push(Entry::Call(
                    name,
                    Punctuated::<Expr, Token![,]>::parse_terminated(&values)?
                        .into_iter()
                        .collect(),
                ));
                input.parse::<Token![;]>()?;
            } else {
                entries.push(Entry::Inline(name, input.parse()?));
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                } else if input.peek(Token![;]) {
                    input.parse::<Token![;]>()?;
                }
            }
        }
        Ok(Self(entries))
    }
}
/// Consume one descriptor's unique named fields and retain calls in written order.
struct Fields {
    /// Unique typed/inline fields with their source identifier.
    values: BTreeMap<String, (Ident, Expr, bool)>,
    /// Repeated call-shaped operations, never confused with scalar configuration.
    calls: Vec<(Ident, Vec<Expr>, usize)>,
    /// Source positions survive keyed extraction so evaluation never follows lookup order.
    positions: BTreeMap<String, usize>,
    /// Lowered, hygienic bindings emitted once in declaration order.
    bindings: BTreeMap<usize, TokenStream>,
}
impl Fields {
    /// Reject duplicate fields even when one uses inline and another extracted syntax.
    fn parse(input: TokenStream) -> syn::Result<Self> {
        let Description(entries) = syn::parse2(input)?;
        let mut values = BTreeMap::new();
        let mut calls = Vec::new();
        let mut positions = BTreeMap::new();
        for (position, entry) in entries.into_iter().enumerate() {
            match entry {
                Entry::Field(name, value) => {
                    let key = name.to_string();
                    positions.insert(key.clone(), position);
                    if values.insert(key, (name.clone(), value, false)).is_some() {
                        return Err(syn::Error::new(
                            name.span(),
                            "duplicate Web descriptor field",
                        ));
                    }
                }
                Entry::Inline(name, callback) => {
                    let key = name.to_string();
                    positions.insert(key.clone(), position);
                    if values
                        .insert(key, (name.clone(), Expr::Closure(callback), true))
                        .is_some()
                    {
                        return Err(syn::Error::new(
                            name.span(),
                            "duplicate inline/extracted Web callback",
                        ));
                    }
                }
                Entry::Call(name, values) => calls.push((name, values, position)),
            }
        }
        Ok(Self {
            values,
            calls,
            positions,
            bindings: BTreeMap::new(),
        })
    }
    /// Require a named field while preserving its callback lowering marker.
    fn take(&mut self, name: &str) -> syn::Result<(Expr, bool)> {
        self.values
            .remove(name)
            .map(|(_, value, inline)| (value, inline))
            .ok_or_else(|| {
                syn::Error::new(
                    Span::call_site(),
                    format!("missing required Web field `{name}`"),
                )
            })
    }
    /// Take an optional field, distinguishing absent from an explicitly typed None.
    fn optional(&mut self, name: &str) -> Option<(Expr, bool)> {
        self.values
            .remove(name)
            .map(|(_, value, inline)| (value, inline))
    }
    /// Require an ordinary inert expression; callback inline syntax is not inferred.
    fn expression(&mut self, name: &str) -> syn::Result<Expr> {
        let (value, inline) = self.take(name)?;
        if inline {
            return Err(syn::Error::new_spanned(
                value,
                format!("`{name}` requires a typed expression"),
            ));
        }
        Ok(Expr::Verbatim(self.bind(name, quote!(#value))?))
    }
    /// Bind a lowered scalar or callback without executing its deferred body.
    fn bind(&mut self, name: &str, value: TokenStream) -> syn::Result<TokenStream> {
        let position = self.positions.get(name).copied().ok_or_else(|| {
            syn::Error::new(Span::call_site(), format!("missing Web binding `{name}`"))
        })?;
        let local = Ident::new(&format!("__web_field_{position}"), Span::mixed_site());
        self.bindings.insert(position, quote!(let #local = #value;));
        Ok(quote!(#local))
    }
    /// Bind repeated-call arguments left to right at that call's source position.
    fn bind_call(&mut self, position: usize, values: &[Expr]) -> Vec<TokenStream> {
        let mut bindings = Vec::new();
        let mut arguments = Vec::new();
        for (argument, value) in values.iter().enumerate() {
            let local = Ident::new(
                &format!("__web_call_{position}_{argument}"),
                Span::mixed_site(),
            );
            bindings.push(quote!(let #local = #value;));
            arguments.push(quote!(#local));
        }
        self.bindings.insert(position, quote!(#(#bindings)*));
        arguments
    }
    /// Reject unknown configuration and return the source-ordered inert bindings.
    fn finish(self) -> syn::Result<Vec<TokenStream>> {
        if let Some((_, (name, _, _))) = self.values.into_iter().next() {
            return Err(syn::Error::new(name.span(), "unknown Web descriptor field"));
        }
        if let Some((name, _, _)) = self.calls.into_iter().next() {
            return Err(syn::Error::new(
                name.span(),
                "unexpected Web descriptor operation",
            ));
        }
        Ok(self.bindings.into_values().collect())
    }
}
/// Check fixed descriptor arity at its call-site span.
fn arity(name: &Ident, values: &[Expr], expected: usize) -> syn::Result<()> {
    if values.len() == expected {
        Ok(())
    } else {
        Err(syn::Error::new(
            name.span(),
            format!("`{name}` requires {expected} argument(s)"),
        ))
    }
}
/// Lower a reusable callback to one fresh, one-shot Effect per invocation. Its pure
/// prefix executes in source order before the owned body is deferred, permitting
/// non-Clone inputs/results to move through the existing interpreter. Explicit
/// shared input copies are owner handles (Arc key/Request), never payload clones.
/// Named inputs may carry native Rust type annotations to anchor deferred inference.
fn callback(
    value: Expr,
    inline: bool,
    path: &TokenStream,
    shared_input: bool,
) -> syn::Result<TokenStream> {
    if !inline {
        return Ok(quote!(#value));
    }
    let Expr::Closure(mut closure) = value else {
        return Err(syn::Error::new_spanned(
            value,
            "inline callback requires `|arguments| { body }`",
        ));
    };
    let Expr::Block(block) = closure.body.as_mut() else {
        return Err(syn::Error::new_spanned(
            &closure.body,
            "inline callback requires a block",
        ));
    };
    let mut statements = std::mem::take(&mut block.block.stmts);
    if shared_input {
        for pattern in closure.inputs.iter().rev() {
            let pattern = if let syn::Pat::Type(typed) = pattern {
                typed.pat.as_ref()
            } else {
                pattern
            };
            let syn::Pat::Ident(pattern) = pattern else {
                return Err(syn::Error::new_spanned(
                    pattern,
                    "inline shared inputs require named bindings",
                ));
            };
            let name = &pattern.ident;
            statements.insert(0, parse_quote!(let #name = #name.clone();));
        }
    }
    let (prefix, statements) = crate::context_expand::split_prefix(statements);
    let body = crate::extension_support::effect_body_once(statements, path);
    closure.capture = Some(parse_quote!(move));
    *closure.body = Expr::Verbatim(quote!({ #(#prefix)* #body }));
    Ok(quote!(#closure))
}
/// Borrowed terminal observation evaluates its pure metadata prefix at callback
/// invocation. Only explicitly owned projected values enter its one-shot deferred
/// Effect; these values move once, and R/E/completion payloads acquire no blanket
/// Clone requirement. The enclosing observer remains reusable for future requests.
fn observer_callback(value: Expr, inline: bool, path: &TokenStream) -> syn::Result<TokenStream> {
    if !inline {
        return Ok(quote!(#value));
    }
    let Expr::Closure(mut closure) = value else {
        return Err(syn::Error::new_spanned(
            value,
            "inline observer requires a closure",
        ));
    };
    let Expr::Block(block) = closure.body.as_mut() else {
        return Err(syn::Error::new_spanned(
            &closure.body,
            "inline observer requires a block",
        ));
    };
    let (prefix, body) =
        crate::context_expand::split_prefix(std::mem::take(&mut block.block.stmts));
    let body = crate::extension_support::effect_body_once(body, path);
    closure.capture = Some(parse_quote!(move));
    *closure.body = Expr::Verbatim(quote!({ #(#prefix)* #body }));
    Ok(quote!(#closure))
}
/// Inline equality/selection callbacks are pure typed native closures, not Effects.
fn pure_callback(value: Expr, _: bool) -> TokenStream {
    quote!(#value)
}
/// D22 checked CSP, including literal exclusive-none and duplicate-directive diagnostics.
fn csp(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let Description(entries) = syn::parse2(input)?;
    let builder = Ident::new("__web_csp", Span::mixed_site());
    let mut steps = Vec::new();
    let mut directives = BTreeMap::<String, bool>::new();
    for entry in entries {
        let Entry::Call(name, values) = entry else {
            return Err(syn::Error::new(
                Span::call_site(),
                "CSP uses directive/append/replace call-shaped entries",
            ));
        };
        arity(&name, &values, 2)?;
        let mut values = values.iter();
        let directive = values
            .next()
            .ok_or_else(|| syn::Error::new(name.span(), "missing CSP directive"))?;
        let sources = values
            .next()
            .ok_or_else(|| syn::Error::new(name.span(), "missing CSP sources"))?;
        let key = directive.to_token_stream().to_string();
        let source = sources.to_token_stream().to_string();
        let none = source.ends_with(":: none ()");
        match name.to_string().as_str() {
            "directive" => {
                if directives.insert(key, none).is_some() {
                    return Err(syn::Error::new(
                        name.span(),
                        "duplicate CSP directive; use replace explicitly",
                    ));
                }
            }
            "replace" => {
                directives.insert(key, none);
            }
            "append" => {
                if let Some(previous_none) = directives.get(&key)
                    && (*previous_none || none)
                {
                    return Err(syn::Error::new(
                        name.span(),
                        "CSP none cannot be appended to ordinary sources; use replace",
                    ));
                }
                directives.insert(key, none);
            }
            _ => {
                return Err(syn::Error::new(
                    name.span(),
                    "expected directive/append/replace",
                ));
            }
        }
        let method = Ident::new(
            &name.to_string(),
            name.span().resolved_at(Span::mixed_site()),
        );
        steps.push(quote!(let #builder = #builder.#method(#directive, #sources)?;));
    }
    Ok(
        quote!((|| -> ::core::result::Result<#path::csp::Csp, #path::WebError> { let #builder = #path::csp::Csp::new(); #(#steps)* Ok(#builder) })()),
    )
}
/// D21 native/typed security selections; duplicate/unknown fields fail statically.
fn security(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let mut fields = Fields::parse(input)?;
    let policy = Ident::new("__web_security", Span::mixed_site());
    let mut assignments = Vec::new();
    for name in [
        "browser_defaults",
        "forwarded_from",
        "cors",
        "compression",
        "static_files",
        "body_limit",
        "method_override",
        "hsts",
    ] {
        if let Some((value, inline)) = fields.optional(name) {
            if inline {
                return Err(syn::Error::new_spanned(
                    value,
                    "security policy requires a typed expression",
                ));
            }
            let value = fields.bind(name, quote!(#value))?;
            let name = Ident::new(name, Span::mixed_site());
            assignments.push(quote!(#policy.#name = Some(#value);));
        }
    }
    if let Some((value, inline)) = fields.optional("csp") {
        if inline {
            return Err(syn::Error::new_spanned(
                value,
                "CSP requires a typed description",
            ));
        }
        let value = fields.bind("csp", quote!(#value))?;
        assignments.push(quote!(#policy.csp = Some(#path::csp::IntoCsp::into_csp(#value)?);));
    }
    for name in ["nonce", "authentication", "authorization", "antiforgery"] {
        if let Some((value, inline)) = fields.optional(name) {
            let callback = callback(value, inline, path, name != "nonce")?;
            let callback = fields.bind(name, callback)?;
            let name = Ident::new(name, Span::mixed_site());
            assignments.push(quote!(#policy.#name = Some(::std::sync::Arc::new(#callback));));
        }
    }
    for (name, values, position) in std::mem::take(&mut fields.calls) {
        if name != "header" {
            return Err(syn::Error::new(
                name.span(),
                "security supports header(name,value);",
            ));
        }
        arity(&name, &values, 2)?;
        let values = fields.bind_call(position, &values);
        let mut values = values.iter();
        let header = values.next();
        let value = values.next();
        assignments.push(quote!(#policy.headers.append(#header, #value);));
    }
    let bindings = fields.finish()?;
    Ok(
        quote!((|| { #(#bindings)* let mut #policy = #path::security::Security::new(); #(#assignments)* #policy.validate() })()),
    )
}
/// D23/D24 core-backed scoped construction with complete policy/equality/loader forms.
fn read_through(
    input: TokenStream,
    path: &TokenStream,
    dependent: bool,
) -> syn::Result<TokenStream> {
    let mut fields = Fields::parse(input)?;
    let scope = fields.expression("scope")?;
    let retain = fields.expression("retain")?;
    let admission = fields.expression("admission")?;
    let (keys, keys_inline) = fields.take("keys")?;
    let keys = pure_callback(keys, keys_inline);
    let keys = fields.bind("keys", keys)?;
    let (loader, inline) = fields.take("loader")?;
    let loader = callback(loader, inline, path, true)?;
    let loader = fields.bind("loader", loader)?;
    let (failures, inline) = fields.take("cache_failures")?;
    let failures = pure_callback(failures, inline);
    let failures = fields.bind("cache_failures", failures)?;
    let projection = match fields.optional("project") {
        Some((value, inline)) => Some(fields.bind("project", pure_callback(value, inline))?),
        None => None,
    };
    let result = if dependent {
        let (dependencies, inline) = fields.take("dependencies")?;
        let dependencies = pure_callback(dependencies, inline);
        let dependencies = fields.bind("dependencies", dependencies)?;
        let maximum = fields.expression("maximum_dependencies")?;
        match fields.optional("select_dependencies") {
            Some((select, inline)) => {
                let select = pure_callback(select, inline);
                let select = fields.bind("select_dependencies", select)?;
                quote!(#path::cache::DependentReadThrough::scoped_by(#scope, #retain, #admission, #maximum, (#keys, #dependencies, #failures), #loader, #select))
            }
            None => {
                quote!(#path::cache::DependentReadThrough::scoped(#scope, #retain, #admission, #maximum, (#keys, #dependencies, #failures), #loader))
            }
        }
    } else {
        quote!(#path::cache::ReadThrough::scoped(#scope, #keys, #retain, #admission, #failures, #loader))
    };
    let bindings = fields.finish()?;
    let result = match projection {
        Some(project) => quote!((#result).map(#project)),
        None => result,
    };
    Ok(quote!({ #(#bindings)* #result }))
}
/// D26 typed field tuples. Literal names are unique; M::Arguments enforces the
/// complete field-value relationship while Message::bind validates dynamic names.
fn message_arguments(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let mut fields = Fields::parse(input)?;
    let message = fields.expression("message")?;
    let mut names = BTreeSet::new();
    let mut arguments = Vec::new();
    for (name, values, position) in std::mem::take(&mut fields.calls) {
        if name != "field" {
            return Err(syn::Error::new(
                name.span(),
                "message_arguments supports field(name,value);",
            ));
        }
        arity(&name, &values, 2)?;
        let field = values
            .first()
            .ok_or_else(|| syn::Error::new(name.span(), "missing message field"))?;
        let Expr::Lit(literal) = field else {
            return Err(syn::Error::new_spanned(
                field,
                "message field name must be a string literal",
            ));
        };
        let syn::Lit::Str(field) = &literal.lit else {
            return Err(syn::Error::new_spanned(
                field,
                "message field name must be a string literal",
            ));
        };
        if field.value().is_empty() || !names.insert(field.value()) {
            return Err(syn::Error::new(
                field.span(),
                "empty/duplicate message field",
            ));
        }
        let values = fields.bind_call(position, &values);
        arguments.push(quote!((#(#values),*)));
    }
    let bindings = fields.finish()?;
    let list = arguments.into_iter().rev().fold(
        quote!(()),
        |tail, field| quote!(#path::localization::NamedArgument::new(#field, #tail)),
    );
    Ok(quote!({ #(#bindings)* (#message).bind(#list) }))
}
/// D27 complete catalogue/culture/selection/missing policy with explicit optional reload.
fn localization(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let mut fields = Fields::parse(input)?;
    let catalogue = fields.expression("catalogue")?;
    let cultures = fields.expression("cultures")?;
    let (select, inline) = fields.take("select_by")?;
    let select = callback(select, inline, path, true)?;
    let missing = fields.expression("missing")?;
    let select = fields.bind("select_by", select)?;
    let reload = match fields.optional("reload") {
        Some((value, _)) => {
            let value = fields.bind("reload", quote!(#value))?;
            quote!(Some(#value))
        }
        None => quote!(None),
    };
    let bindings = fields.finish()?;
    Ok(
        quote!({ #(#bindings)* #path::localization::Localization::new(#catalogue, #cultures, #select, #missing, #reload) }),
    )
}
/// D25 typed required/optional probe construction; readiness must be declared.
fn availability(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let mut fields = Fields::parse(input)?;
    let service = fields.expression("service")?;
    let (errors, inline) = fields.take("errors")?;
    if inline {
        return Err(syn::Error::new_spanned(
            errors,
            "availability errors requires a typed injection callback",
        ));
    }
    let errors = fields.bind("errors", quote!(#errors))?;
    let deadline = fields.expression("deadline")?;
    let parallelism = fields.expression("parallelism")?;
    let traffic = match fields.optional("traffic_ready") {
        Some((value, _)) => {
            let value = fields.bind("traffic_ready", quote!(#value))?;
            quote!(Some(#value))
        }
        None => quote!(None),
    };
    let dashboard = match fields.optional("dashboard") {
        Some((value, _)) => Some(fields.bind("dashboard", quote!(#value))?),
        None => None,
    };
    let mut probes = Vec::new();
    let mut ready = false;
    let mut literals = BTreeSet::new();
    for (name, values, position) in std::mem::take(&mut fields.calls) {
        let (role, required) = match name.to_string().as_str() {
            "readiness" => {
                ready = true;
                (quote!(Readiness), true)
            }
            "optional_readiness" => {
                ready = true;
                (quote!(Readiness), false)
            }
            "health" => (quote!(Health), true),
            "optional_health" => (quote!(Health), false),
            "liveness" | "process_liveness" => (quote!(Liveness), true),
            "optional_liveness" => (quote!(Liveness), false),
            _ => {
                return Err(syn::Error::new(
                    name.span(),
                    "expected readiness/health/liveness/process_liveness or optional probe operation",
                ));
            }
        };
        arity(&name, &values, 2)?;
        if let Some(Expr::Lit(literal)) = values.first()
            && let syn::Lit::Str(identity) = &literal.lit
            && !literals.insert((role.to_string(), identity.value()))
        {
            return Err(syn::Error::new(
                identity.span(),
                "duplicate availability probe",
            ));
        }
        let values = fields.bind_call(position, &values);
        let mut values = values.iter();
        let identity = values.next();
        let sample = values.next();
        probes.push(quote!(#path::availability::Probe::new(#identity, #path::availability::ProbeRole::#role, #required, #sample)?));
    }
    if !ready {
        return Err(syn::Error::new(
            Span::call_site(),
            "availability requires nonempty readiness",
        ));
    }
    let bindings = fields.finish()?;
    let constructed = quote!(#path::availability::Availability::new(#service, #errors, vec![#(#probes),*], #deadline, #parallelism, #traffic));
    let result = match dashboard {
        Some(dashboard) => {
            quote!((#constructed).map(|availability| availability.with_dashboard(#dashboard)))
        }
        None => constructed,
    };
    Ok(quote!((|| { #(#bindings)* #result })()))
}
/// D20 complete root assembly. Static required fields are checked here while
/// application/native policy validation/acquisition belongs to Web startup.
fn web(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let mut fields = Fields::parse(input)?;
    let policy = fields.expression("policy")?;
    let errors = fields.expression("web_errors")?;
    let (startup, inline) = fields.take("startup")?;
    let startup = callback(startup, inline, path, true)?;
    let startup = fields.bind("startup", startup)?;
    let (environment, inline) = fields.take("environment")?;
    let environment = callback(environment, inline, path, true)?;
    let environment = fields.bind("environment", environment)?;
    let listen = fields.expression("listen")?;
    let protocols = fields.expression("protocols")?;
    let (routes, inline) = fields.take("routes")?;
    let routes = fields.bind("routes", pure_callback(routes, inline))?;
    let builder = Ident::new("__web_application", Span::mixed_site());
    let mut methods = Vec::new();
    for name in [
        "security",
        "localization",
        "observe",
        "otel",
        "availability",
        "output_cache",
        "configure_builder",
        "configure_app",
    ] {
        if let Some((value, inline)) = fields.optional(name) {
            let value = if name == "observe" {
                observer_callback(value, inline, path)?
            } else if matches!(
                name,
                "configure_builder"
                    | "configure_app"
                    | "localization"
                    | "security"
                    | "output_cache"
                    | "availability"
                    | "otel"
            ) {
                if inline {
                    pure_callback(value, true)
                } else {
                    quote!(#value)
                }
            } else {
                callback(value, inline, path, true)?
            };
            let value = fields.bind(name, value)?;
            let method = Ident::new(name, Span::mixed_site());
            methods.push(quote!(let #builder = #builder.#method(#value);));
        }
    }
    let bindings = fields.finish()?;
    Ok(
        quote!({ #(#bindings)* let #builder = #path::application::Web::new(#policy, #errors, #startup, #environment, #listen, #protocols, #routes); #(#methods)* #builder }),
    )
}
/// Existing shared dispatcher; runtime owners are always resolved through the
/// consumer's actual renamed effect-server-web package dependency.
pub(crate) fn expand(name: &str, input: TokenStream) -> syn::Result<TokenStream> {
    let path = crate::extension_support::crate_path("effect-server-web")?;
    match name {
        "web" => web(input, &path),
        "security" => security(input, &path),
        "csp" => csp(input, &path),
        "read_through" => read_through(input, &path, false),
        "dependent_read_through" => read_through(input, &path, true),
        "availability" => availability(input, &path),
        "message_arguments" => message_arguments(input, &path),
        "localization" => localization(input, &path),
        _ => Err(syn::Error::new(
            Span::call_site(),
            "unknown Web extension macro",
        )),
    }
}
