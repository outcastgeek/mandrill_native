//! D07–D16 parsing and lowering onto typed Server descriptions and common Effect binds.

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream, Parser};
use syn::{Expr, Token};
use syn::{parenthesized, parse_quote};

/// One source-ordered descriptor operation, configuration field or inline Effect callback.
struct Entry {
    /// User-visible operation identity and its diagnostic span.
    name: Ident,
    /// Arguments retained as native Rust expressions, evaluated once in written order.
    args: Vec<Expr>,
    /// Optional inline body and explicitly declared callback parameters.
    inline: Option<(Vec<syn::Pat>, syn::Block)>,
    /// Colon form denotes a unique configuration field instead of a repeated operation.
    field: bool,
}
/// Ordered domain syntax keeps duplicate fields distinct from valid repeated operations.
struct Entries(Vec<Entry>);
impl Parse for Entries {
    /// Parse call-shaped operations, colon fields and inline callback bodies at source spans.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut entries: Vec<Entry> = Vec::new();
        while !input.is_empty() {
            let name: Ident = input.call(Ident::parse_any)?;
            let field = input.peek(Token![:]);
            let (args, inline) = if field {
                input.parse::<Token![:]>()?;
                (vec![input.parse()?], None)
            } else if input.peek(syn::token::Paren) {
                let content;
                parenthesized!(content in input);
                let args =
                    syn::punctuated::Punctuated::<Expr, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .collect();
                (args, None)
            } else {
                let mut patterns = Vec::new();
                if input.peek(Token![|]) {
                    input.parse::<Token![|]>()?;
                    while !input.peek(Token![|]) {
                        let pattern = input.call(syn::Pat::parse_single)?;
                        let pattern = if name == "prepared" && input.peek(Token![:]) {
                            input.parse::<Token![:]>()?;
                            let ty: syn::Type = input.parse()?;
                            parse_quote!(#pattern: #ty)
                        } else {
                            pattern
                        };
                        patterns.push(pattern);
                        if input.peek(Token![,]) {
                            input.parse::<Token![,]>()?;
                        } else {
                            break;
                        }
                    }
                    input.parse::<Token![|]>()?;
                }
                let body: syn::Block = input.parse()?;
                (Vec::new(), Some((patterns, body)))
            };
            if field
                && entries
                    .iter()
                    .any(|entry| entry.field && entry.name == name)
            {
                return Err(syn::Error::new(
                    name.span(),
                    "duplicate server descriptor field",
                ));
            }
            entries.push(Entry {
                name,
                args,
                inline,
                field,
            });
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if input.peek(Token![;]) {
                input.parse::<Token![;]>()?;
            } else if !input.is_empty() && entries.last().is_none_or(|entry| entry.inline.is_none())
            {
                return Err(input.error("expected comma or semicolon after server operation"));
            }
        }
        Ok(Self(entries))
    }
}
use syn::ext::IdentExt;
impl Entry {
    /// Validate operation arity and reject accidentally supplied body/field forms.
    fn arguments(&self, count: usize) -> syn::Result<&[Expr]> {
        if self.inline.is_some() || self.args.len() != count {
            Err(syn::Error::new(
                self.name.span(),
                format!("{} requires {count} arguments", self.name),
            ))
        } else {
            Ok(&self.args)
        }
    }
    /// Return one source expression while retaining its type and ownership.
    fn value(&self) -> syn::Result<&Expr> {
        self.arguments(1)?
            .first()
            .ok_or_else(|| syn::Error::new(self.name.span(), "missing value"))
    }
    /// Lower an inline Effect callback with the common binder; extracted callbacks remain typed.
    fn callback(&self, path: &TokenStream, count: usize) -> syn::Result<TokenStream> {
        match &self.inline {
            Some((patterns, body)) if patterns.len() == count => {
                let effect = callback_effect(body.stmts.clone(), path);
                Ok(quote!(move |#(#patterns),*| #effect))
            }
            Some(_) => Err(syn::Error::new(
                self.name.span(),
                "wrong inline callback parameter count",
            )),
            None => {
                let value = self.value()?;
                Ok(quote!(#value))
            }
        }
    }
}
/// Each domain callback invocation owns its inputs once, using the core native-control
/// lowering instead of requiring Clone payloads in a reusable inner description.
fn callback_effect(statements: Vec<syn::Stmt>, path: &TokenStream) -> TokenStream {
    let private = quote!(#path::__private::effect::__private);
    crate::async_expand::expand_once(statements, &private)
}
/// Diagnose unknown operations instead of silently treating names as native methods.
fn unknown(entry: &Entry) -> syn::Error {
    syn::Error::new(
        entry.name.span(),
        format!("unknown server operation: {}", entry.name),
    )
}
/// Dispatch each required public macro onto the corresponding typed domain lowering.
pub(crate) fn expand(name: &str, input: TokenStream) -> syn::Result<TokenStream> {
    let path = crate::extension_support::crate_path("effect-server")?;
    match name {
        "endpoint" => endpoint(input, &path),
        "server_stream" => stream(input, &path),
        "reply" => reply(syn::parse2(input)?, &path),
        "route" => route(syn::parse2(input)?, &path),
        "router" | "routes" => router(syn::parse2(input)?, &path),
        "pipeline" => pipeline(syn::parse2(input)?, &path),
        "registered" => registered(syn::parse2(input)?, &path),
        "representations" => representations(syn::parse2(input)?, &path),
        "server" => server(syn::parse2(input)?, &path),
        _ => Err(syn::Error::new(Span::call_site(), "unknown HTTP macro")),
    }
}
/// Lower direct endpoint statements, an existing source, or one inline/extracted body.
fn endpoint(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let endpoint_ident = Ident::new("__endpoint", Span::mixed_site());
    let endpoint_body_ident = Ident::new("__endpoint_body", Span::mixed_site());

    let descriptor = syn::parse2::<Entries>(input.clone());
    if let Ok(Entries(entries)) = &descriptor
        && entries.iter().any(|entry| entry.name == "prepared")
    {
        let effect = crate::expand::effect_crate_path()?;
        let mut program = None;
        let mut contract = None;
        let mut cleanup = None;
        let mut panic_diagnostics = None;
        let mut on_capacity = None;
        let mut capacity = None;
        let mut byte_pools = None;
        let mut input_pattern = None;
        let mut bindings = Vec::new();
        let mut bind = |value: &Expr, ty: TokenStream| {
            let local = Ident::new(
                &format!("__prepared_endpoint_field_{}", bindings.len()),
                Span::mixed_site(),
            );
            bindings.push(quote!(let #local: #ty = #value;));
            local
        };
        for entry in entries {
            match entry.name.to_string().as_str() {
                "prepared" => {
                    if program.is_some() {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "duplicate prepared endpoint body",
                        ));
                    }
                    let expression = if let Some((patterns, block)) = &entry.inline {
                        if patterns.len() > 1 {
                            return Err(syn::Error::new(
                                entry.name.span(),
                                "prepared endpoint body accepts at most one input pattern",
                            ));
                        }
                        input_pattern = patterns.first().cloned();
                        syn::parse2(quote!(#effect::effect! #block))?
                    } else {
                        entry.value()?.clone()
                    };
                    program = Some(crate::prepared_program_expand::root(expression, &effect)?);
                }
                "contract" => {
                    if contract.is_some() {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "duplicate prepared endpoint contract",
                        ));
                    }
                    contract = Some(bind(entry.value()?, quote!(#effect::DynamicRunContract)));
                }
                "panic_diagnostics" => {
                    if panic_diagnostics.is_some() {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "duplicate prepared endpoint panic diagnostics",
                        ));
                    }
                    panic_diagnostics = Some(bind(entry.value()?, quote!(#effect::PreparedPanicPool)));
                }
                "cleanup" => {
                    if cleanup.is_some() {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "duplicate prepared endpoint cleanup",
                        ));
                    }
                    cleanup = Some(bind(entry.value()?, quote!(#effect::PreparedCleanupConfig<_>)));
                }
                "on_capacity" => {
                    if on_capacity.is_some() {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "duplicate prepared endpoint capacity reply",
                        ));
                    }
                    on_capacity = Some(bind(entry.value()?, quote!(#path::StaticOverloadReply)));
                }
                "capacity" => {
                    if capacity.is_some() {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "duplicate prepared endpoint capacity",
                        ));
                    }
                    let types = [
                        quote!(&#effect::memory::PreparedMemory<'static>),
                        quote!(#effect::memory::RegionId),
                        quote!(usize),
                        quote!(#effect::memory::FramePool<'static>),
                    ];
                    capacity = Some(entry.arguments(4)?.iter().zip(types)
                        .map(|(argument, ty)| bind(argument, ty))
                        .collect::<Vec<_>>());
                }
                "byte_pools" => {
                    if byte_pools.is_some() {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "duplicate prepared endpoint byte pools",
                        ));
                    }
                    byte_pools = Some(bind(
                        entry.value()?,
                        quote!(::std::sync::Arc<#path::PreparedEndpointBytePools>),
                    ));
                }
                _ => return Err(unknown(entry)),
            }
        }
        let program = program
            .ok_or_else(|| syn::Error::new(Span::call_site(), "missing prepared endpoint body"))?;
        let contract = contract.ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "missing prepared endpoint execution contract",
            )
        })?;
        let plan = match input_pattern {
            Some(input) => {
                quote!(#path::PreparedInputEndpointPlan::from_graph(#effect::__private::prepared_graph::PreparedGraphPlan::new_with_input(move |#input| {#program}, #contract)))
            }
            None => {
                quote!(#path::PreparedEndpointPlan::from_graph(#effect::__private::prepared_graph::PreparedGraphPlan::new(move || {#program}, #contract)))
            }
        };
        let plan = match cleanup {
            Some(cleanup) => quote!((#plan).with_cleanup(#cleanup)),
            None => plan,
        };
        let plan = match panic_diagnostics {
            Some(diagnostics) => quote!((#plan).with_panic_diagnostics(#diagnostics)),
            None => plan,
        };
        let plan = match on_capacity {
            Some(reply) => quote!((#plan).on_capacity(#reply)),
            None => plan,
        };
        let plan = match byte_pools {
            Some(pools) => quote!((#plan).with_byte_pools(#pools)),
            None => plan,
        };
        let endpoint = match capacity {
            Some(args) => quote!((#plan).prepare(#(#args),*)),
            None => plan,
        };
        return Ok(quote!({ #(#bindings)* #endpoint }));
    }
    if let Ok(Entries(entries)) = descriptor
        && entries.iter().any(|entry| {
            matches!(
                entry.name.to_string().as_str(),
                "body" | "effect" | "source"
            )
        })
    {
        let mut body = None;
        let mut config = Vec::new();
        let mut bindings = Vec::new();
        for entry in entries {
            match entry.name.to_string().as_str() {
                "source" => {
                    if body.is_some() {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "endpoint source/body/effect are mutually exclusive",
                        ));
                    }
                    let source = entry.value()?;
                    bindings.push(quote!(let #endpoint_body_ident=#source;));
                    body = Some(quote!(#endpoint_body_ident));
                }
                "body" => {
                    if body.is_some() {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "endpoint body/effect are mutually exclusive",
                        ));
                    }
                    let Some((patterns, block)) = &entry.inline else {
                        let callback = entry.value()?;
                        bindings.push(
                            quote!(let #endpoint_body_ident=#path::Endpoint::from_body(#callback);),
                        );
                        body = Some(quote!(#endpoint_body_ident));
                        continue;
                    };
                    if !patterns.is_empty() {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "endpoint body has no callback parameters",
                        ));
                    }
                    let callback = crate::context_expand::reusable_callback(block.stmts.clone());
                    bindings.push(
                        quote!(let #endpoint_body_ident=#path::Endpoint::from_body(#callback);),
                    );
                    body = Some(quote!(#endpoint_body_ident));
                }
                "effect" => {
                    if body.is_some() {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "endpoint body/effect are mutually exclusive",
                        ));
                    }
                    let effect = entry.value()?;
                    bindings.push(
                        quote!(let #endpoint_body_ident=#path::Endpoint::from_effect(#effect);),
                    );
                    body = Some(quote!(#endpoint_body_ident));
                }
                "pipe_through" => {
                    let value = entry.value()?;
                    let local = Ident::new(
                        &format!("__endpoint_config_{}", bindings.len()),
                        Span::mixed_site(),
                    );
                    bindings.push(quote!(let #local=#value;));
                    config.push(quote!(let #endpoint_ident=#endpoint_ident.pipe_through(#local);));
                }
                "on_error" => {
                    let value = entry.callback(path, 1)?;
                    let local = Ident::new(
                        &format!("__endpoint_config_{}", bindings.len()),
                        Span::mixed_site(),
                    );
                    bindings.push(quote!(let #local=#value;));
                    config.push(quote!(let #endpoint_ident=#endpoint_ident.on_error(#local);));
                }
                "preconditions" => {
                    let args = entry.arguments(2)?;
                    let first = args.first().ok_or_else(|| unknown(&entry))?;
                    let second = args.get(1).ok_or_else(|| unknown(&entry))?;
                    let local = Ident::new(
                        &format!("__endpoint_config_{}", bindings.len()),
                        Span::mixed_site(),
                    );
                    bindings.push(quote!(let #local=(#first,#second);));
                    config.push(quote!(let #endpoint_ident=#endpoint_ident.preconditions(#local.0,#local.1);));
                }
                _ => return Err(unknown(&entry)),
            }
        }
        let body =
            body.ok_or_else(|| syn::Error::new(Span::call_site(), "missing endpoint body"))?;
        return Ok(quote!({#(#bindings)* let #endpoint_ident=#body;#(#config)* #endpoint_ident}));
    }
    let statements = syn::Block::parse_within.parse2(input)?;
    let callback = crate::context_expand::reusable_callback(statements);
    Ok(quote!(#path::Endpoint::from_body(#callback)))
}
/// Enforce exactly one terminal reply and validated body/status relationships statically.
fn reply(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let draft_ident = Ident::new("__reply_draft", Span::mixed_site());

    let mut operations = Vec::new();
    let mut terminal = None;
    let mut bodyless = false;
    let mut has_status = false;
    let mut has_trailers = false;
    for entry in entries {
        let name = entry.name.to_string();
        match name.as_str() {
            "status" | "header" | "append_header" | "remove_header" | "location" | "created"
            | "content_language" | "cache_control" | "vary" | "validators" | "conditional_get"
            | "trailer" | "set_cookie" | "delete_cookie" => {
                if terminal.is_some() {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "reply metadata must precede its terminal body",
                    ));
                }
                let count = if matches!(name.as_str(), "header" | "append_header" | "trailer") {
                    2
                } else {
                    1
                };
                let args = entry.arguments(count)?;
                if matches!(name.as_str(), "status" | "created") {
                    has_status = true;
                }
                if name == "trailer" {
                    has_trailers = true;
                }
                let method = Ident::new(
                    &entry.name.to_string(),
                    entry.name.span().resolved_at(Span::mixed_site()),
                );
                operations.push(quote!(let #draft_ident=#draft_ident.#method(#(#args),*);));
            }
            "text" | "html" | "render" | "json" | "json_codec" | "problem" | "validation"
            | "bytes" | "stream" | "sse" | "negotiate" | "native" | "redirect" | "file_path"
            | "no_content" | "empty" | "source" | "finite" | "lease" | "frame" => {
                if terminal.is_some() {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "reply requires exactly one terminal body",
                    ));
                }
                let count = match name.as_str() {
                    "empty" | "no_content" => 0,
                    "bytes" | "stream" | "negotiate" | "redirect" | "file_path" | "lease"
                    | "frame" => 2,
                    "json_codec" => 3,
                    _ => 1,
                };
                let args = entry.arguments(count)?;
                bodyless = matches!(name.as_str(), "no_content" | "redirect");
                let method = Ident::new(
                    if name == "finite" {
                        "from_finite"
                    } else {
                        &name
                    },
                    entry.name.span().resolved_at(Span::mixed_site()),
                );
                terminal = Some(if name == "source" {
                    quote!(#(#args)*)
                } else {
                    quote!(#path::Reply::#method(#(#args),*))
                });
            }
            _ => return Err(unknown(&entry)),
        }
    }
    if bodyless && has_status {
        return Err(syn::Error::new(
            Span::call_site(),
            "bodyless terminal controls its own status",
        ));
    }
    if bodyless && has_trailers {
        return Err(syn::Error::new(
            Span::call_site(),
            "bodyless terminal cannot emit trailers",
        ));
    }
    let terminal = terminal
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing reply terminal body"))?;
    let finish = if bodyless {
        quote!(#draft_ident.finish_bodyless(#terminal))
    } else {
        quote!(#draft_ident.finish(#terminal))
    };
    Ok(quote!({let #draft_ident=#path::ReplyDraft::new();#(#operations)* #finish}))
}
/// Compile literal route grammar and capture shape; dynamic descriptions keep typed validation.
fn route(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let route_ident = Ident::new("__route", Span::mixed_site());

    let mut operations = Vec::new();
    let mut initialized = false;
    let mut captured = false;
    let mut metadata = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    let mut optional = false;
    for entry in entries {
        let name = entry.name.to_string();
        match name.as_str() {
            "literal" => {
                let value = entry.value()?;
                if let Expr::Lit(literal) = value
                    && let syn::Lit::Str(path) = &literal.lit
                    && (!path.value().starts_with('/')
                        || path.value().contains(['{', '}', '?', '#']))
                {
                    return Err(syn::Error::new_spanned(
                        value,
                        "literal requires a native absolute path without captures",
                    ));
                }
                if !initialized {
                    operations.push(quote!(let #route_ident=#path::Route::literal(#value)?;));
                    initialized = true;
                } else {
                    operations.push(quote!(let #route_ident=#route_ident.segment(#value)?;));
                }
            }
            "prefix" => {
                if initialized {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "prefix must begin the route",
                    ));
                }
                let value = entry.value()?;
                operations.push(quote!(let #route_ident=#value;));
                initialized = true;
                captured = true;
            }
            "capture"
            | "capture_constrained"
            | "optional_capture"
            | "default_capture"
            | "catch_all" => {
                if !initialized {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "route needs literal or prefix before captures",
                    ));
                }
                if optional {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "optional/default/catch-all capture must be terminal",
                    ));
                }
                let count = match name.as_str() {
                    "catch_all" => 1,
                    "capture_constrained" | "default_capture" => 3,
                    _ => 2,
                };
                let args = entry.arguments(count)?;
                if let Some(Expr::Lit(literal)) = args.first()
                    && let syn::Lit::Str(value) = &literal.lit
                    && !names.insert(value.value())
                {
                    return Err(syn::Error::new_spanned(value, "duplicate route capture"));
                }
                let method = match (name.as_str(), captured) {
                    ("capture", false) => "capture_first",
                    ("capture_constrained", false) => "constrained_first",
                    ("optional_capture", false) => "optional_first",
                    ("default_capture", false) => "default_first",
                    ("catch_all", false) => "catch_all_first",
                    ("capture", true) => "capture",
                    ("capture_constrained", true) => "capture_constrained",
                    ("optional_capture", true) => "optional_capture",
                    ("default_capture", true) => "default_capture",
                    ("catch_all", true) => "catch_all",
                    _ => return Err(unknown(&entry)),
                };
                let method = Ident::new(method, entry.name.span().resolved_at(Span::mixed_site()));
                operations.push(quote!(let #route_ident=#route_ident.#method(#(#args),*)?;));
                captured = true;
                optional = matches!(
                    name.as_str(),
                    "optional_capture" | "default_capture" | "catch_all"
                );
            }
            "name" | "tags" | "configure" => {
                let value = entry.value()?;
                let method = Ident::new(
                    &entry.name.to_string(),
                    entry.name.span().resolved_at(Span::mixed_site()),
                );
                let local = Ident::new(
                    &format!("__route_metadata_{}", metadata.len()),
                    Span::mixed_site(),
                );
                operations.push(quote!(let #local=#value;));
                metadata.push(quote!(let #route_ident=#route_ident.#method(#local);));
            }
            _ => return Err(unknown(&entry)),
        }
    }
    if !initialized {
        return Err(syn::Error::new(
            Span::call_site(),
            "route needs literal or prefix",
        ));
    }
    Ok(
        quote!((move || {#(#operations)* #(#metadata)* ::core::result::Result::<_,#path::ServerError>::Ok(#route_ident)})()),
    )
}
/// Assemble router methods/groups/native hooks in source order without running endpoint bodies.
fn router(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let router_ident = Ident::new("__router", Span::mixed_site());
    let value_ident = Ident::new("__value", Span::mixed_site());
    let values_ident = Ident::new("__values", Span::mixed_site());
    let make_ident = Ident::new("__make", Span::mixed_site());

    let mut operations = Vec::new();
    let mut source = None;
    for entry in entries {
        match entry.name.to_string().as_str() {
            "source" => {
                if source.is_some() || !operations.is_empty() {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "router source must appear once before routes",
                    ));
                }
                let value = entry.value()?;
                source = Some(quote!(#value));
            }
            "get" | "post" | "put" | "patch" | "delete" | "head" | "options" => {
                let args = entry.arguments(2)?;
                let route = args.first().ok_or_else(|| unknown(&entry))?;
                let handler = args.get(1).ok_or_else(|| unknown(&entry))?;
                let method = Ident::new(&entry.name.to_string().to_uppercase(), entry.name.span());
                operations.push(quote!(let #router_ident=#path::mount_route(#router_ident,#path::HttpMethodSet::new(::std::vec![#path::native::http::Method::#method])?,#route,#handler)?;));
            }
            "methods" => {
                let args = entry.arguments(3)?;
                let methods = args.first().ok_or_else(|| unknown(&entry))?;
                let route = args.get(1).ok_or_else(|| unknown(&entry))?;
                let handler = args.get(2).ok_or_else(|| unknown(&entry))?;
                operations.push(quote!(let #router_ident=#path::mount_route(#router_ident,#methods,#route,#handler)?;));
            }
            "include" => {
                let value = entry.value()?;
                operations.push(quote!(let #router_ident=#router_ident.include(#path::IntoRouter::into_router(#value)?);));
            }
            "group" | "group_configured" => {
                let args = entry.arguments(if entry.name == "group" { 2 } else { 3 })?;
                let prefix = args.first().ok_or_else(|| unknown(&entry))?;
                let child = args.get(1).ok_or_else(|| unknown(&entry))?;
                if entry.name == "group" {
                    operations.push(quote!(let #router_ident=#router_ident.group(#prefix,#path::IntoRouter::into_router(#child)?)?;));
                } else {
                    let configure = args.get(2).ok_or_else(|| unknown(&entry))?;
                    operations.push(quote!(let #router_ident=#router_ident.group_configured(#prefix,#configure,#path::IntoRouter::into_router(#child)?)?;));
                }
            }
            "native" | "fallback" | "pipe_through" | "on_error" => {
                let value = if entry.name == "on_error" {
                    entry.callback(path, 1)?
                } else {
                    let value = entry.value()?;
                    quote!(#value)
                };
                let method = Ident::new(
                    &entry.name.to_string(),
                    entry.name.span().resolved_at(Span::mixed_site()),
                );
                operations.push(quote!(let #router_ident=#router_ident.#method(#value);));
            }
            "websocket" => {
                let args = entry.arguments(3)?;
                operations.push(quote!(let #router_ident=#router_ident.websocket(#(#args),*)?;));
            }
            "when" => {
                let args = entry.arguments(2)?;
                let condition = args.first().ok_or_else(|| unknown(&entry))?;
                let routes = args.get(1).ok_or_else(|| unknown(&entry))?;
                operations.push(quote!(let #router_ident=if #condition {#router_ident.include(#path::IntoRouter::into_router(#routes)?)} else {#router_ident};));
            }
            "each" => {
                let args = entry.arguments(2)?;
                let values = args.first().ok_or_else(|| unknown(&entry))?;
                let make = args.get(1).ok_or_else(|| unknown(&entry))?;
                operations.push(quote!(let mut #router_ident=#router_ident;let #values_ident=#values;let #make_ident=#make;for #value_ident in #values_ident {#router_ident=#router_ident.include(#path::IntoRouter::into_router(#make_ident(#value_ident))?);}));
            }
            _ => return Err(unknown(&entry)),
        }
    }
    let source = source.unwrap_or_else(|| quote!(#path::Router::new()));
    Ok(
        quote!((|| {let #router_ident=#source;#(#operations)* ::core::result::Result::<_,#path::ServerError>::Ok(#router_ident)})()),
    )
}

/// Preserve ordered middleware stages, optionally extending an existing owned source.
/// A source precedes stages and is evaluated once; it adds no empty pipeline allocation.
fn pipeline(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let pipeline_ident = Ident::new("__pipeline", Span::mixed_site());

    let mut operations = Vec::new();
    let mut source = None;
    for entry in entries {
        match entry.name.to_string().as_str() {
            "source" => {
                if source.is_some() || !operations.is_empty() {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "pipeline source must appear once before stages",
                    ));
                }
                let value = entry.value()?;
                source = Some(quote!(#value));
            }
            "before" | "after" | "respond" => {
                let callback = entry.callback(path, 1)?;
                let method = Ident::new(
                    &entry.name.to_string(),
                    entry.name.span().resolved_at(Span::mixed_site()),
                );
                operations.push(quote!(let #pipeline_ident=#pipeline_ident.#method(#callback);));
            }
            "on_exit" => {
                let callback = match &entry.inline {
                    Some((patterns, block)) if patterns.len() == 2 => {
                        let (prefix, body) =
                            crate::context_expand::split_prefix(block.stmts.clone());
                        let effect = callback_effect(body, path);
                        quote!(move |#(#patterns),*| {#(#prefix)* #path::__private::effect::Release::from_effect(#effect)})
                    }
                    Some(_) => {
                        return Err(syn::Error::new(
                            entry.name.span(),
                            "on_exit requires request and outcome parameters",
                        ));
                    }
                    None => {
                        let value = entry.value()?;
                        quote!(#value)
                    }
                };
                operations.push(quote!(let #pipeline_ident=#pipeline_ident.on_exit(#callback);));
            }
            "guard" => {
                let args = entry.arguments(2)?;
                operations.push(quote!(let #pipeline_ident=#pipeline_ident.guard(#(#args),*);));
            }
            "set_header" => {
                let args = entry.arguments(2)?;
                operations
                    .push(quote!(let #pipeline_ident=#pipeline_ident.set_header(#(#args),*);));
            }
            "stage" => {
                let callback = entry.callback(path, 2)?;
                operations.push(quote!(let #pipeline_ident=#pipeline_ident.stage(#callback);));
            }
            "prepared_stage" => {
                let callback = entry.callback(path, 2)?;
                operations.push(quote!(let #pipeline_ident=#pipeline_ident.prepared_stage(#callback);));
            }
            "include" => {
                let value = entry.value()?;
                operations.push(quote!(let #pipeline_ident=#pipeline_ident.append(#value);));
            }
            _ => return Err(unknown(&entry)),
        }
    }
    let source = source.map_or_else(|| quote!(#path::Pipeline::new()), |value| quote!(#value));
    Ok(quote!({let #pipeline_ident=#source;#(#operations)* #pipeline_ident}))
}
/// Build registration metadata separately from the endpoint's per-request values.
fn registered(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let metadata_ident = Ident::new("__metadata", Span::mixed_site());
    let registered_ident = Ident::new("__registered", Span::mixed_site());
    let configured_ident = Ident::new("__configured", Span::mixed_site());

    let mut endpoint = None;
    let mut operations = Vec::new();
    let mut finalizers = Vec::new();
    let mut authorized = false;
    let mut anonymous = false;
    for entry in entries {
        match entry.name.to_string().as_str() {
            "endpoint" => {
                if endpoint.is_some() {
                    return Err(syn::Error::new(entry.name.span(), "duplicate endpoint"));
                }
                let value = entry.value()?;
                let local = Ident::new("__registered_endpoint", Span::mixed_site());
                operations.push(quote!(let #local=#value;));
                endpoint = Some(quote!(#local));
            }
            "name" | "summary" | "description" => {
                let value = entry.value()?;
                let name = &entry.name;
                operations.push(quote!(#metadata_ident.#name=::core::option::Option::Some(::std::convert::Into::<::std::sync::Arc<str>>::into(#value));));
            }
            "tags" | "accepts" | "produces" | "hosts" => {
                let value = entry.value()?;
                let name = &entry.name;
                operations.push(quote!(#metadata_ident.#name=#value;));
            }
            "authorize" => {
                if anonymous {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "authorize conflicts with anonymous",
                    ));
                }
                authorized = true;
                let value = entry.value()?;
                operations.push(quote!(#metadata_ident.authorization=::core::option::Option::Some(::std::convert::Into::<::std::sync::Arc<str>>::into(#value));));
            }
            "allow_anonymous" => {
                if authorized {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "anonymous conflicts with authorize",
                    ));
                }
                anonymous = true;
                let value = entry.value()?;
                operations.push(quote!(#metadata_ident.allow_anonymous=#value;));
            }
            "metadata" => {
                let value = entry.value()?;
                operations.push(quote!(#metadata_ident.extensions.insert(#value);));
            }
            "configure" => {
                let value = entry.value()?;
                operations.push(quote!((#value)(&mut #metadata_ident);));
            }
            "pipe_through" | "filter" => {
                let value = entry.value()?;
                let local = Ident::new(
                    &format!("__registration_pipe_{}", finalizers.len()),
                    Span::mixed_site(),
                );
                operations.push(quote!(let #local=#value;));
                finalizers
                    .push(quote!(let #registered_ident=#registered_ident.pipe_through(#local);));
            }
            "on_error" => {
                let callback = entry.callback(path, 1)?;
                let local = Ident::new(
                    &format!("__registration_policy_{}", finalizers.len()),
                    Span::mixed_site(),
                );
                operations.push(quote!(let #local=#callback;));
                finalizers.push(quote!(let #registered_ident={let mut #configured_ident=#registered_ident;#configured_ident.endpoint=#configured_ident.endpoint.on_error(#local);#configured_ident};));
            }
            _ => return Err(unknown(&entry)),
        }
    }
    let endpoint = endpoint
        .ok_or_else(|| syn::Error::new(Span::call_site(), "registered requires endpoint"))?;
    Ok(
        quote!({let mut #metadata_ident=#path::Registration::default();#(#operations)* let #registered_ident=#path::Registered::new(#endpoint,#metadata_ident);#(#finalizers)* #registered_ident}),
    )
}
/// Construct checked media/language alternatives without evaluating any renderer.
fn representations(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let representations_ident = Ident::new("__representations", Span::mixed_site());

    if entries.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "representations requires an alternative",
        ));
    }
    let mut operations = Vec::new();
    for entry in entries {
        let constructor = match entry.name.to_string().as_str() {
            "variant" => {
                let args = entry.arguments(3)?;
                quote!(#path::Representation::new(#(#args),*)?)
            }
            "text" | "html" => {
                let args = entry.arguments(1)?;
                let method = Ident::new(
                    &entry.name.to_string(),
                    entry.name.span().resolved_at(Span::mixed_site()),
                );
                quote!(#path::Representation::#method(#(#args),*))
            }
            "stream" => {
                let args = entry.arguments(2)?;
                quote!(#path::Representation::stream(#(#args),*)?)
            }
            "json_codec" => {
                let args = entry.arguments(2)?;
                quote!(#path::Representation::json_codec(#(#args),*))
            }
            "language" => {
                let args = entry.arguments(2)?;
                let language = args.first().ok_or_else(|| unknown(&entry))?;
                let representation = args.get(1).ok_or_else(|| unknown(&entry))?;
                quote!((#representation).language(#language)?)
            }
            _ => return Err(unknown(&entry)),
        };
        operations.push(quote!(#representations_ident.push(#constructor);));
    }
    Ok(
        quote!((move || {let mut #representations_ident=::std::vec::Vec::new();#(#operations)* #path::Representations::new(#representations_ident)})()),
    )
}
/// Construct inert host/configuration fields once in declaration order, keeping native hooks explicit.
fn server(Entries(entries): Entries, path: &TokenStream) -> syn::Result<TokenStream> {
    let server_ident = Ident::new("__server", Span::mixed_site());

    let mut fields = std::collections::BTreeMap::new();
    let mut prepared = std::collections::BTreeMap::new();
    let mut bindings = Vec::new();
    let mut operations = Vec::new();
    let mut source = None;
    let mut overload = false;
    for entry in entries {
        let name = entry.name.to_string();
        let local = Ident::new(
            &format!("__server_field_{}", bindings.len()),
            Span::mixed_site(),
        );
        let value = entry.value()?;
        match name.as_str() {
            "source" => {
                if source.is_some() || !bindings.is_empty() {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "server source must appear once before configuration",
                    ));
                }
                source = Some(local.clone());
                bindings.push(quote!(let #local=#value;));
            }
            "policy" | "environment" | "listen" | "protocols" | "routes" => {
                if fields.insert(name, local.clone()).is_some() {
                    return Err(syn::Error::new(entry.name.span(), "duplicate server field"));
                }
                bindings.push(quote!(let #local=#value;));
            }
            "memory" | "region" | "capacity" | "result_frames" | "cleanup" | "request_controls"
            | "admission" => {
                let local = Ident::new(
                    &format!("__server_prepared_{}", prepared.len()),
                    Span::mixed_site(),
                );
                if prepared.insert(name, local.clone()).is_some() {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "duplicate prepared server field",
                    ));
                }
                let binding = quote!(let #local=#value;);
                if source.is_some() {
                    operations.push(binding);
                } else {
                    bindings.push(binding);
                }
            }
            "configure_app"
            | "configure_builder"
            | "middleware"
            | "before_environment"
            | "before_routing"
            | "after_response"
            | "around_request"
            | "around_selection"
            | "websockets"
            | "http1_header_timeout"
            | "overload" => {
                if name == "overload" && std::mem::replace(&mut overload, true) {
                    return Err(syn::Error::new(entry.name.span(), "duplicate server overload"));
                }
                let method_name = match name.as_str() {
                    "http1_header_timeout" => "with_http1_header_timeout",
                    "overload" => "with_overload_reply",
                    _ => &name,
                };
                let method = Ident::new(
                    method_name,
                    entry.name.span().resolved_at(Span::mixed_site()),
                );
                if source.is_some() {
                    // The existing Server supplies E/R at the native callback
                    // boundary; an untyped closure local would lose inference.
                    operations.push(quote!(let #server_ident=#server_ident.#method(#value);));
                } else {
                    bindings.push(quote!(let #local=#value;));
                    operations.push(quote!(let #server_ident=#server_ident.#method(#local);));
                }
            }
            _ => return Err(unknown(&entry)),
        }
    }
    let finish = if prepared.is_empty() {
        quote!(::core::result::Result::<_,#path::ServerError>::Ok(#server_ident))
    } else {
        let args = ["memory", "region", "capacity", "result_frames", "cleanup", "request_controls"]
            .into_iter()
            .map(|name| {
                prepared.get(name).ok_or_else(|| {
                    syn::Error::new(Span::call_site(), format!("prepared server requires {name}"))
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let prepare = quote!(#path::PreparedServer::prepare_with_request_controls(#server_ident,#(#args),*)
            .map_err(#path::ServerError::Memory));
        match prepared.get("admission") {
            Some(admission) => quote!((#prepare?).with_request_control_admission(#admission)
                .map_err(#path::ServerError::Memory)),
            None => prepare,
        }
    };
    if let Some(source) = source {
        if !fields.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "server source conflicts with construction fields",
            ));
        }
        let body = quote!(#(#bindings)* let #server_ident=#source;#(#operations)*);
        return if prepared.is_empty() {
            Ok(quote!({#body #server_ident}))
        } else {
            Ok(quote!((|| {#body #finish})()))
        };
    }
    let mut required = Vec::new();
    for name in ["policy", "environment", "listen", "protocols", "routes"] {
        required.push(fields.get(name).cloned().ok_or_else(|| {
            syn::Error::new(Span::call_site(), format!("server requires {name}"))
        })?);
    }
    let policy = required
        .first()
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing policy"))?;
    let environment = required
        .get(1)
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing environment"))?;
    let listen = required
        .get(2)
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing listener"))?;
    let protocols = required
        .get(3)
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing protocols"))?;
    let routes = required
        .get(4)
        .ok_or_else(|| syn::Error::new(Span::call_site(), "missing routes"))?;
    Ok(
        quote!((|| {#(#bindings)* let #server_ident=#path::Server::new(#policy,#path::IntoRequestBinding::into_binding(#environment),#listen,#protocols,#path::IntoRouter::into_router(#routes)?);#(#operations)* #finish})()),
    )
}
/// Connect inline/extracted writer callbacks to common Effect and scoped stream production semantics.
/// Inline bodies prepare explicit captures in the reusable factory before a one-shot future owns them.
/// The writer parameter retains its native type so prefix method calls infer before the body future.
/// The immediately invoked binder closure names MacroContext explicitly for the same receiver inference.
fn stream(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let source_ident = Ident::new("__server_source", Span::mixed_site());
    let writer_ident = Ident::new("__server_writer", Span::mixed_site());
    let body_ident = Ident::new("__server_body", Span::mixed_site());
    let context_ident = Ident::new("__context", Span::mixed_site());

    let Entries(entries) = syn::parse2(input)?;
    if let Some(owned) = entries
        .iter()
        .find(|entry| matches!(entry.name.to_string().as_str(), "leases" | "frames"))
    {
        if entries.len() != 1 {
            return Err(syn::Error::new(
                owned.name.span(),
                "stream leases or frames must be the only operation",
            ));
        }
        let source = owned.value()?;
        let method = Ident::new(
            &owned.name.to_string(),
            owned.name.span().resolved_at(Span::mixed_site()),
        );
        return Ok(quote!(#path::BodyProgram::#method(#source)));
    }
    let mut source = None;
    let mut writer = None;
    let mut body = None;
    let mut bindings = Vec::new();
    let mut source_parameters = Vec::new();
    let mut source_values = Vec::new();
    for entry in entries {
        match entry.name.to_string().as_str() {
            "source" => {
                if source.is_some() {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "duplicate stream source",
                    ));
                }
                let value = entry.value()?;
                bindings.push(quote!(let #source_ident=#value;));
                source_parameters
                    .push(quote!(#source_ident: #path::__private::effect::Stream<A, E, R>));
                source_values.push(quote!(#value));
                source = Some(quote!(#source_ident));
            }
            "write" => {
                if writer.is_some() {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "duplicate stream writer",
                    ));
                }
                let callback = entry.callback(path, 2)?;
                bindings.push(quote!(let #writer_ident=#callback;));
                source_parameters.push(quote!(#writer_ident: F));
                source_values.push(callback);
                writer = Some(quote!(#writer_ident));
            }
            "body" => {
                if body.is_some() {
                    return Err(syn::Error::new(entry.name.span(), "duplicate stream body"));
                }
                let Some((patterns, block)) = entry.inline else {
                    let value = entry.value()?;
                    // Keep the extracted callback at the native writer boundary,
                    // evaluating it once in the entry's written position.
                    bindings.push(quote! {
                        fn __server_native_body<E, R>(
                            body: impl Fn(#path::BodyWriter<E, R>) -> #path::__private::effect::Effect<(), E, R> + Send + Sync + 'static,
                        ) -> #path::BodyProgram<E, R>
                        where E: Send + 'static, R: Send + Sync + 'static {
                            #path::BodyProgram::new(body)
                        }
                        let #body_ident = __server_native_body(#value);
                    });
                    body = Some(quote!(#body_ident));
                    continue;
                };
                if patterns.len() != 1 {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        "stream body requires one writer parameter",
                    ));
                }
                let writer_name = patterns
                    .first()
                    .ok_or_else(|| syn::Error::new(entry.name.span(), "missing writer"))?;
                let (prefix, statements) = crate::context_expand::split_prefix(block.stmts);
                let mut callback: syn::ExprClosure =
                    syn::parse2(crate::context_expand::callback(statements))?;
                let argument = callback.inputs.first_mut().ok_or_else(|| {
                    syn::Error::new(entry.name.span(), "missing stream body context")
                })?;
                *argument = syn::Pat::Type(syn::PatType {
                    attrs: Vec::new(),
                    pat: Box::new(argument.clone()),
                    colon_token: Default::default(),
                    ty: Box::new(parse_quote!(
                        #path::__private::effect::__private::MacroContext<_,_>
                    )),
                });
                body = Some(
                    quote!(#path::BodyProgram::from_body(move |#writer_name: #path::BodyWriter<_,_>,#context_ident| {#(#prefix)* (#callback)(#context_ident)})),
                );
            }
            _ => return Err(unknown(&entry)),
        }
    }
    match (source, writer, body) {
        (Some(source), Some(writer), None) => Ok(quote!({
            fn __server_native_source<A, E, R, F>(#(#source_parameters),*) -> #path::BodyProgram<E, R>
            where A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
                F: Fn(A, #path::BodyWriter<E, R>) -> #path::__private::effect::Effect<(), E, R> + Send + Sync + 'static {
                #path::BodyProgram::source(#source, #writer)
            }
            __server_native_source(#(#source_values),*)
        })),
        (None, None, Some(body)) => Ok(quote!({#(#bindings)* #body})),
        _ => Err(syn::Error::new(
            Span::call_site(),
            "stream requires source+write or one body; forms are mutually exclusive",
        )),
    }
}
