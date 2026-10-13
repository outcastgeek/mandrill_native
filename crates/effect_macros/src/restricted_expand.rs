//! Named adapters retain transaction journals, tracked read contexts and durable replay boundaries.
//! None of these bodies is silently lowered as a general application Effect.
use crate::callback_expand::Input;
use crate::domain_expand::BodyCategory;
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Expr;

/// Detect a supported leading mode without mistaking an ordinary statement for it.
pub(crate) fn has_mode(tokens: &TokenStream, modes: &[&str]) -> bool {
    let mut tokens = tokens.clone().into_iter();
    matches!((tokens.next(), tokens.next(), tokens.next(), tokens.next()),
        (Some(proc_macro2::TokenTree::Ident(name)), Some(proc_macro2::TokenTree::Punct(semi)), Some(proc_macro2::TokenTree::Ident(_)), Some(next))
        if semi.as_char() == ';' && modes.contains(&name.to_string().as_str()) &&
            matches!(next, proc_macro2::TokenTree::Punct(_) | proc_macro2::TokenTree::Group(_)))
}
/// Named map/modify/update syntax over the current transactional evaluator.
/// Both state roles use TxRef::update's borrowed snapshot and Result<(replacement,result),E>.
pub(crate) fn transaction(input: Input) -> syn::Result<TokenStream> {
    let mode = input.mode(&["map_error", "modify", "update"], "default")?;
    let mut fields = input.fields;
    let path = crate::expand::effect_crate_path()?;
    let output = if mode == "map_error" {
        let program = fields.take("program")?;
        let map = fields.role("map", 1, BodyCategory::Native)?;
        quote!((#program).map_error(#map))
    } else {
        let reference = fields.transform("reference", |reference, _| Ok(quote!(&(#reference))))?;
        let callback = fields.role(&mode, 1, BodyCategory::Native)?;
        if mode == "update" {
            quote!((#reference).update(move |state| (#callback)(state).map(|next| (next, ()))))
        } else {
            quote!((#reference).update(#callback))
        }
    };
    if mode == "map_error" {
        fields.finish(output)
    } else {
        let result = if mode == "update" {
            quote!(T)
        } else {
            quote!((T, B))
        };
        let value = if mode == "update" {
            quote!(())
        } else {
            quote!(B)
        };
        let generics = if mode == "update" {
            quote!(T, E, F)
        } else {
            quote!(T, B, E, F)
        };
        let extra = if mode == "update" {
            quote!()
        } else {
            quote!(B: Send + 'static,)
        };
        fields.finish_inferred(
            generics,
            &[
                ("reference", quote!(&#path::transactional::TxRef<T>)),
                (mode.as_str(), quote!(F)),
            ],
            quote!(#path::transactional::Transaction<#value, E>),
            quote!(T: #path::transactional::TxValue, E: Send + 'static, #extra
                F: Fn(&T) -> ::core::result::Result<#result, E> + Send + Sync + 'static),
            output,
        )
    }
}
/// Map a standalone tracked program before registration, keeping its dependency context.
pub(crate) fn atom(input: Input) -> syn::Result<TokenStream> {
    input.mode(&["map_error"], "map_error")?;
    let mut fields = input.fields;
    let program = fields.take("program")?;
    let map = fields.role("map", 1, BodyCategory::Native)?;
    fields.finish(quote!((#program).map_error(#map)))
}
/// Lower durable error mapping/traversal and delegate named retry/compensation operations.
/// Retry uses the existing branch/policy; compensation's body is an activity Effect
/// at its documented engine boundary, not an arbitrary durable coordination body.
pub(crate) fn workflow(input: Input) -> syn::Result<TokenStream> {
    let mode = input.mode(
        &["map_error", "for_each", "retry", "compensate", "result"],
        "default",
    )?;
    let mut fields = input.fields;
    let path = crate::expand::effect_crate_path()?;
    let output = match mode.as_str() {
        "result" => {
            let value = fields.take("value")?;
            quote!(#path::workflow::Durable::from_result_once(#value))
        }
        "map_error" => {
            let program = fields.take("program")?;
            let map = fields.role("map", 1, BodyCategory::Native)?;
            quote!((#program).map_error(#map))
        }
        "for_each" => {
            let values = fields.role("produce", 0, BodyCategory::Native)?;
            let callback = fields.transform("body", |expr, inline| {
                if !inline {
                    return Ok(quote!(#expr));
                }
                let Expr::Closure(closure) = expr else {
                    return Err(syn::Error::new_spanned(
                        expr,
                        "durable body requires one input",
                    ));
                };
                if closure.inputs.len() != 1
                    || closure.asyncness.is_some()
                    || !matches!(closure.output, syn::ReturnType::Default)
                {
                    return Err(syn::Error::new_spanned(
                        closure,
                        "durable body requires one synchronous parameter",
                    ));
                }
                let Expr::Block(block) = *closure.body else {
                    return Err(syn::Error::new_spanned(
                        closure.body,
                        "durable body must be braced",
                    ));
                };
                let inputs = closure.inputs;
                let body = crate::durable_expand::expand_once(block.block.stmts)?;
                Ok(quote!(move |#inputs| #body))
            })?;
            quote!(#path::workflow::Durable::for_each(#values, #callback))
        }
        "retry" => {
            let run = fields.take("run")?;
            let branch = fields.take("branch")?;
            let policy = fields.take("policy")?;
            quote!(#path::workflow::Durable::<(), (), _>::retry(#run, #branch, #policy))
        }
        "compensate" => {
            let compensation = fields.take("compensation")?;
            let run = fields.take("run")?;
            let callback = fields.role("body", 1, BodyCategory::Effect)?;
            if fields.contains("original") {
                let original = fields.take("original")?;
                quote!((#compensation).on_failure(#original, #run, #callback))
            } else {
                quote!((#compensation).run(#run, #callback))
            }
        }
        _ => return Err(syn::Error::new(Span::call_site(), "unknown workflow mode")),
    };
    fields.finish(output)
}
/// Context-explicit entity acquisition uses the current externally owned activation context.
pub(crate) fn entity_resource(input: Input) -> syn::Result<TokenStream> {
    input.mode(&[], "default")?;
    let mut fields = input.fields;
    let path = crate::expand::effect_crate_path()?;
    let acquire = fields.transform("acquire", |expr, inline| {
        if !inline {
            return Ok(quote!(#expr));
        }
        let Expr::Closure(closure) = expr else {
            return Err(syn::Error::new_spanned(
                expr,
                "acquire requires one activation context",
            ));
        };
        if closure.inputs.len() != 1
            || closure.asyncness.is_some()
            || !matches!(closure.output, syn::ReturnType::Default)
        {
            return Err(syn::Error::new_spanned(
                closure,
                "acquire requires one synchronous context parameter",
            ));
        }
        let pattern = closure
            .inputs
            .first()
            .ok_or_else(|| syn::Error::new_spanned(&closure, "missing activation context"))?;
        let Expr::Block(block) = *closure.body else {
            return Err(syn::Error::new_spanned(
                closure.body,
                "acquire must be braced",
            ));
        };
        crate::context_expand::named_callback(block.block.stmts, pattern)
    })?;
    fields.finish(quote!(#path::cluster::EntityResourcePlan::from_body(#acquire)))
}
