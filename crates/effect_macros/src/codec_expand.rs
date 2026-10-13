//! Directional schema callback blocks without fabricated witnesses or representation conversion.
//! Pure inputs borrow, effectful inputs own; contextual directions forward ParseContext.

use crate::callback_expand::Input;
use crate::domain_expand::BodyCategory;
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::parse_quote;

/// Lower pure/effectful/transform/recursive/lazy modes through the genuine codec constructors.
pub(crate) fn codec(input: Input) -> syn::Result<TokenStream> {
    let mode = input.mode(
        &[
            "pure",
            "effectful",
            "contextual",
            "transform",
            "transform_pure",
            "recursive",
            "recursive_pure",
            "lazy",
        ],
        "effectful",
    )?;
    let mut fields = input.fields;
    let path = crate::expand::effect_crate_path()?;
    let pure = mode == "pure" || mode == "recursive_pure" || mode == "transform_pure";
    let owner = if pure {
        quote!(#path::PureCodec)
    } else {
        quote!(#path::Codec)
    };
    let output = match mode.as_str() {
        "lazy" => {
            let factory = fields.role("lazy", 0, BodyCategory::Native)?;
            quote!(::std::sync::LazyLock::new(#factory))
        }
        "recursive" | "recursive_pure" => {
            let name = fields.take("name")?;
            let shape = fields.role("shape", 1, BodyCategory::Native)?;
            let build = fields.role("build", 1, BodyCategory::Native)?;
            quote!(#owner::recursive(#name, #shape, #build))
        }
        "transform" | "transform_pure" => {
            let source = fields.take("source")?;
            let category = if pure {
                BodyCategory::Native
            } else {
                BodyCategory::Effect
            };
            let decode = fields.role("decode", 1, category)?;
            let encode = fields.role("encode", 1, category)?;
            let method = if pure {
                quote!(transform)
            } else {
                quote!(transform_effectful)
            };
            quote!((#source).#method(#decode, #encode))
        }
        _ => {
            let arity = if mode == "contextual" { 2 } else { 1 };
            let category = if pure {
                BodyCategory::Native
            } else {
                BodyCategory::Effect
            };
            let decode = shared_direction(&mut fields, "decode", arity, category)?;
            let encode = shared_direction(&mut fields, "encode", arity, category)?;
            let ast = if fields.contains("ast") {
                fields.take("ast")?
            } else if mode == "contextual" {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "contextual codec requires ast",
                ));
            } else {
                quote!(#path::SchemaAst::opaque())
            };
            quote!(#owner::from_shared_context(#ast, #decode, #encode))
        }
    };
    finish_codec(fields, &mode, &path, output)
}

/// Place each directional recipe at its native shared callback argument boundary.
/// Closures receive context syntactically; extracted expressions are captured once
/// during description construction, retaining the native single allocation.
fn shared_direction(
    fields: &mut crate::domain_expand::Fields,
    name: &str,
    arity: usize,
    category: BodyCategory,
) -> syn::Result<TokenStream> {
    fields.transform(name, |expression, inline| {
        let callback = if inline {
            crate::domain_expand::role(expression, arity, category)?
        } else {
            quote!(#expression)
        };
        let mut expression: syn::Expr = syn::parse2(callback)?;
        if arity == 1
            && let syn::Expr::Block(block) = &mut expression
            && let Some(syn::Stmt::Expr(syn::Expr::Closure(closure), None)) =
                block.block.stmts.last_mut()
        {
            closure.inputs.push(parse_quote!(_));
            return Ok(quote!(::std::sync::Arc::new(#expression)));
        }
        let direction = if let syn::Expr::Closure(mut closure) = expression {
            if arity == 1 {
                closure.inputs.push(parse_quote!(_));
            }
            quote!(#closure)
        } else if arity == 1 {
            quote!({
                let callback = #expression;
                move |value, _| callback(value)
            })
        } else {
            quote!(#expression)
        };
        Ok(quote!(::std::sync::Arc::new(#direction)))
    })
}

/// Check directional callbacks at their native constructor's argument boundary,
/// retaining borrowed pure inputs, owned contextual inputs and written field order.
fn finish_codec(
    fields: crate::domain_expand::Fields,
    mode: &str,
    path: &TokenStream,
    output: TokenStream,
) -> syn::Result<TokenStream> {
    let ordinary = quote!(A: Send + 'static, Encoded: Send + 'static);
    let environments = quote!(D: Send + Sync + 'static, E: Send + Sync + 'static);
    let types = |decode, encode| {
        vec![
            ("ast", quote!(#path::SchemaAst)),
            ("decode", decode),
            ("encode", encode),
        ]
    };
    match mode {
        "lazy" => fields.finish_inferred(
            quote!(T, F),
            &[("lazy", quote!(F))],
            quote!(::std::sync::LazyLock<T, F>),
            quote!(F: FnOnce() -> T),
            output,
        ),
        "recursive" | "recursive_pure" => {
            let pure = mode == "recursive_pure";
            let owner = if pure { quote!(#path::PureCodec<A, #path::JsonValue>) } else { quote!(#path::Codec<A, #path::JsonValue, D, E>) };
            let generics = if pure { quote!(A, N, F, G) } else { quote!(A, D, E, N, F, G) };
            let environments = if pure { quote!() } else { quote!(#environments,) };
            fields.finish_inferred(
                generics,
                &[("name", quote!(N)), ("shape", quote!(F)), ("build", quote!(G))],
                quote!(::core::result::Result<#owner, #path::SchemaIssue>),
                quote!(A: Send + 'static, #environments N: Into<::std::sync::Arc<str>>, F: FnOnce(#path::SchemaReference) -> #path::SchemaShape, G: FnOnce(#owner) -> ::core::result::Result<#owner, #path::SchemaIssue>),
                output,
            )
        }
        "transform_pure" => fields.finish_inferred(
            quote!(A, B, F, G),
            &[("source", quote!(#path::PureCodec<A, #path::JsonValue>)), ("decode", quote!(F)), ("encode", quote!(G))],
            quote!(#path::PureCodec<B, #path::JsonValue>),
            quote!(A: Send + 'static, B: Send + 'static, F: Fn(A) -> ::core::result::Result<B, #path::SchemaIssue> + Send + Sync + 'static, G: Fn(&B) -> ::core::result::Result<A, #path::SchemaIssue> + Send + Sync + 'static),
            output,
        ),
        "transform" => fields.finish_inferred(
            quote!(A, B, Encoded, D, E, F, G),
            &[("source", quote!(#path::Codec<A, Encoded, D, E>)), ("decode", quote!(F)), ("encode", quote!(G))],
            quote!(#path::Codec<B, Encoded, D, E>),
            quote!(#ordinary, #environments, B: Send + 'static, F: Fn(A) -> #path::Effect<B, #path::SchemaIssue, D> + Send + Sync + 'static, G: Fn(B) -> #path::Effect<A, #path::SchemaIssue, E> + Send + Sync + 'static),
            output,
        ),
        "pure" => fields.finish_inferred(
            quote!(A, Encoded),
            &types(
                quote!(::std::sync::Arc<dyn Fn(&Encoded, ::core::option::Option<&#path::ParseContext>) -> ::core::result::Result<A, #path::SchemaIssue> + Send + Sync>),
                quote!(::std::sync::Arc<dyn Fn(&A, ::core::option::Option<&#path::ParseContext>) -> ::core::result::Result<Encoded, #path::SchemaIssue> + Send + Sync>),
            ),
            quote!(#path::PureCodec<A, Encoded>),
            ordinary,
            output,
        ),
        _ => {
            fields.finish_inferred(
                quote!(A, Encoded, D, E),
                &types(
                    quote!(::std::sync::Arc<dyn Fn(Encoded, ::core::option::Option<#path::ParseContext>) -> #path::Effect<A, #path::SchemaIssue, D> + Send + Sync>),
                    quote!(::std::sync::Arc<dyn Fn(A, ::core::option::Option<#path::ParseContext>) -> #path::Effect<Encoded, #path::SchemaIssue, E> + Send + Sync>),
                ),
                quote!(#path::Codec<A, Encoded, D, E>),
                quote!(#ordinary, #environments),
                output,
            )
        }
    }
}

/// Parse text with a ConfiguredCodec, preserving its decode environment and
/// schema/JSON depth, pattern and cumulative issue budgets. Optional mapping
/// changes SchemaIssue only; encoding remains a separate directional operation.
pub(crate) fn parse(input: Input) -> syn::Result<TokenStream> {
    input.mode(&[], "default")?;
    let mut fields = input.fields;
    let codec = fields.transform("codec", |codec, _| Ok(quote!(&(#codec))))?;
    let text = fields.take("text")?;
    let output = if fields.contains("policy") {
        let policy = fields.take("policy")?;
        quote!((#codec).parse(#text, #policy))
    } else {
        quote!((#codec).parse(#text))
    };
    let output = if fields.contains("on_error") {
        let error = fields.role("on_error", 1, BodyCategory::Native)?;
        quote!((#output).map_error(#error))
    } else {
        output
    };
    fields.finish(output)
}
