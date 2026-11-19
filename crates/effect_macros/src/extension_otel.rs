//! OpenTelemetry convenience lowering onto the existing typed Effect span path.

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
    let input: crate::callback_expand::Input = syn::parse2(input)?;
    let mode = input.mode(&["acquire", "tag", "attribute", "event", "log", "current_context", "envelope", "close", "install_request_hooks", "start", "start_prepared", "instrumented_span"], "span")?;
    let mut fields = input.fields;
    if mode == "current_context" {
        return fields.finish(quote!(#path::Otel::current_context()));
    }
    if mode == "envelope" {
        let context = fields.take("context")?;
        let program = fields.take("program")?;
        return fields.finish(quote!(#path::Otel::envelope(#context, #program)));
    }
    if mode == "close" {
        let source = fields.transform("source", |value, _| Ok(quote!(&(#value))))?;
        return fields.finish(quote!((#source).close()));
    }
    if mode == "install_request_hooks" {
        let hooks = fields.transform("hooks", |value, _| Ok(quote!(&(#value))))?;
        let server = fields.take("server")?;
        let propagation = fields.take("propagation")?;
        return fields.finish(quote!((#hooks).install(#server, #propagation)));
    }
    if mode == "start" || mode == "start_prepared" {
        let shutdown = fields.take("shutdown")?;
        let output = if mode == "start" {
            let factories = fields.take("factories")?;
            quote!(#path::OwnedProviders::start(#factories, #shutdown))
        } else {
            let prepared = fields.take("prepared")?;
            quote!(#path::PreparedOwnedProviders::start(#prepared, #shutdown))
        };
        return fields.finish(output);
    }
    if mode == "acquire" {
        let scope = fields.take("scope")?;
        let factories = fields.take("factories")?;
        let shutdown = fields.take("shutdown")?;
        let on_error = fields.role("on_error", 1, crate::domain_expand::BodyCategory::Native)?;
        return fields.finish(quote!(#path::OwnedProviders::acquire_with_error(#scope, #factories, #shutdown, #on_error)));
    }
    if mode == "tag" || mode == "attribute" {
        let key = fields.take("key")?;
        let value = fields.take("value")?;
        let method = proc_macro2::Ident::new(&mode, Span::mixed_site());
        return fields.finish(quote!(#path::Otel::#method(#key, #value)));
    }
    if mode == "log" {
        let level = fields.take("level")?;
        let message = fields.take("message")?;
        return fields.finish(quote!(#path::Otel::log(#level, #message)));
    }
    let name = fields.take("name")?;
    if mode == "instrumented_span" {
        let core = quote!(#path::__private::effect);
        let program = fields.take("program")?;
        let select = fields.role("select", 1, crate::domain_expand::BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(A, E, R, N, F),
            &[("name", quote!(N)), ("program", quote!(#core::Effect<A, E, R>)), ("select", quote!(F))],
            quote!(#core::Effect<A, E, R>),
            quote!(A: Send + 'static, E: Send + 'static, R: #core::HasTracer + #core::HasClock + Send + Sync + 'static,
                N: Into<::std::sync::Arc<str>>, F: Fn(&R) -> ::std::sync::Arc<dyn #core::PollInstrumentation> + Send + Sync + 'static),
            quote!(#path::__private::tracing_span(#name, #program, #select)),
        );
    }
    if mode == "event" {
        let output = if fields.contains("attributes") {
            let attributes = fields.take("attributes")?;
            quote!(#path::Otel::event_with(#name, #attributes))
        } else {
            quote!(#path::Otel::event(#name))
        };
        return fields.finish(output);
    }
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
    if fields.contains("parent") && fields.contains("parent_option") {
        return Err(syn::Error::new(
            Span::call_site(),
            "parent conflicts with parent_option",
        ));
    }
    let parent = if fields.contains("parent_option") {
        fields.take("parent_option")?
    } else if fields.contains("parent") {
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
