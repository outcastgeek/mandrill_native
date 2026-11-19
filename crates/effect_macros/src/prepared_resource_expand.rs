//! Existing ownership grammar lowered before ordinary ResourceBinding erasure.
use crate::callback_expand::Input;
use crate::domain_expand::BodyCategory;
use crate::prepared_program_expand::{effect_callback, program};
use proc_macro2::TokenStream;
use quote::quote;
use syn::Expr;

pub(crate) fn resource_program(
    name: &str,
    input: Input,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let mode = input.mode(
        &[
            "binding",
            "binding_native",
            "release_sync",
            "release_try_sync",
            "release_effect",
            "use",
        ],
        "default",
    )?;
    let mut fields = input.fields;
    let graph = quote!(#path::__private::prepared_graph);
    if mode == "release_effect" {
        let source = fields.transform("program", |expr, _| program(expr, path))?;
        return fields.finish(quote!(#source));
    }
    if mode == "release_sync" || mode == "release_try_sync" {
        let callback = fields.role("body", 0, BodyCategory::Native)?;
        let (bound, lowered) = if mode == "release_sync" {
            (
                quote!(F: FnOnce() + Send + 'static),
                quote!(#graph::sync(#callback)),
            )
        } else {
            (
                quote!(F: FnOnce()->::core::result::Result<(),E> + Send + 'static),
                quote!(#graph::try_sync(#callback,::core::convert::identity)),
            )
        };
        return fields.finish_inferred(
            quote!(F, E, R),
            &[("body", quote!(F))],
            quote!(impl #graph::Description<Value=(),Error=E,Env=R>),
            quote!(#bound,E: Send + 'static,R: Send + Sync + 'static),
            lowered,
        );
    }
    if mode == "use" {
        let binding = fields.take("binding")?;
        let callback = fields.transform("body", |expr, inline| {
            if inline {
                effect_callback(expr, path)
            } else {
                native_release(expr, path)
            }
        })?;
        return fields.finish_inferred(quote!(P,B,N),&[("binding",quote!(P)),("body",quote!(B))],quote!(P::Bound<B,N>),quote!(P: #graph::Bindable,N: #graph::Description<Error=P::Error,Env=P::Env>,B: FnOnce(P::Value)->N + Send + 'static),quote!(#graph::bind(#binding,#callback)));
    }
    let acquire = fields.transform("acquire", |expr, inline| {
        crate::prepared_program_expand::program_field(expr, inline, path)
    })?;
    let drop_backed = name == "effect_using" && !fields.contains("release");
    let release = if drop_backed {
        quote!(move|resource,_primary|#graph::sync(move||{::core::mem::drop(resource)}))
    } else {
        release(&mut fields, path)?
    };
    let description = quote!(#graph::resource(#acquire,#release));
    let bounds = quote!(A: #graph::Description,A::Value: Sync,C: #graph::Description<Value=(),Error=A::Error,Env=A::Env>,F: for<'a> FnOnce(A::Value,#path::PrimaryOutcomeRef<'a,A::Error>)->C + Send + 'static);
    if mode == "binding" || mode == "binding_native" {
        if drop_backed {
            return fields.finish(description);
        }
        fields.finish_inferred(
            quote!(A, F, C),
            &[("acquire", quote!(A)), ("release", quote!(F))],
            quote!(#graph::Resource<A,F,C>),
            bounds,
            description,
        )
    } else {
        let callback = fields.transform("body", |expr, inline| {
            if inline {
                effect_callback(expr, path)
            } else {
                native_release(expr, path)
            }
        })?;
        // Drop-backed using has no release field/type parameter. Its generated
        // closure still infers concrete C from this same sealed resource bind.
        if drop_backed {
            return fields.finish(quote!(#graph::bind(#description,#callback)));
        }
        let arguments = [("acquire", quote!(A)), ("release", quote!(F)), ("body", quote!(B))];
        fields.finish_inferred(quote!(A,F,C,B,N),&arguments,quote!(#graph::Using<A,F,C,B,N>),quote!(#bounds,N: #graph::Description<Error=A::Error,Env=A::Env>,B: FnOnce(#graph::ResourceValue<A::Value>)->N + Send + 'static),quote!(#graph::bind(#description,#callback)))
    }
}
/// Primary-aware factories keep native prefix statements/immediate borrowing;
/// only their returned constructor is lowered, never reinterpreted as a DSL.
pub(crate) fn native_release(expression: Expr, path: &TokenStream) -> syn::Result<TokenStream> {
    let Expr::Closure(mut callback) = expression else {
        return Ok(quote!(#expression));
    };
    if let Expr::Block(block) = callback.body.as_mut() {
        if let Some(tail) = block.block.stmts.last_mut() {
            let value = if let syn::Stmt::Expr(value, None) = tail {
                Some(value.clone())
            } else if let syn::Stmt::Macro(invocation) = tail
                && invocation.semi_token.is_none()
            {
                Some(Expr::Macro(syn::ExprMacro {
                    attrs: invocation.attrs.clone(),
                    mac: invocation.mac.clone(),
                }))
            } else {
                None
            };
            if let Some(value) = value {
                *tail = syn::Stmt::Expr(syn::parse2(program(value, path)?)?, None);
            }
        }
    } else {
        *callback.body = syn::parse2(program(*callback.body, path)?)?;
    }
    Ok(quote!(#callback))
}

pub(crate) fn release(
    fields: &mut crate::domain_expand::Fields,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    if !fields.inline("release") {
        // Extracted callbacks already return a concrete descriptor. An
        // explicitly supplied native closure can lower its returned expression.
        return fields.transform("release", |expr, _| native_release(expr, path));
    }
    if fields.arity("release")? == 1 {
        return fields.transform("release", |expr, _| {
            let callback = effect_callback(expr, path)?;
            Ok(quote!(move|resource,_primary|(#callback)(resource)))
        });
    }
    fields.transform("release", |expr, _| {
        let callback = crate::domain_expand::role(expr, 2, BodyCategory::Native)?;
        native_release(syn::parse2(callback)?, path)
    })
}
