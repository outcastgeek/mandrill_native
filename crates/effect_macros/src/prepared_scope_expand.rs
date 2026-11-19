//! Explicit finite construction/target contracts inside existing ownership DSL.
use crate::callback_expand::Input;
use crate::prepared_program_expand::program_field;
use crate::prepared_resource_expand::release;
use proc_macro2::TokenStream;
use quote::quote;
pub(crate) fn scope_program(
    name: &str,
    input: Input,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let mode = input.mode(&["construction", "reserve_permit", "release"], "default")?;
    if name == "effect_scope" {
        let mut fields = input.fields;
        let graph = quote!(#path::__private::prepared_graph);
        let constructed = fields.contains("construction");
        let body = fields.transform("body", |expr, inline| {
            if inline {
                crate::prepared_program_expand::effect_callback_arity(
                    expr,
                    if constructed { 2 } else { 1 },
                    path,
                )
            } else {
                crate::prepared_program_expand::native_descriptor_callback(expr, path)
            }
        })?;
        if constructed {
            let construction = fields.transform("construction", |expr, _| {
                crate::prepared_program_expand::native_descriptors(expr, path)
            })?;
            return fields.finish_inferred(quote!(F,N,P),&[("body",quote!(F)),("construction",quote!(P))],quote!(impl #graph::Description<Value=N::Value,Error=N::Error,Env=N::Env>),quote!(N: #graph::Description,P: #graph::scope_binding::Construction,F: FnOnce(#path::PreparedScope<N::Error>,P::Permits<N::Error>)->N + Send + 'static),quote!(#graph::scope_construct(#body,#construction)));
        }
        return fields.finish_inferred(quote!(F,N),&[("body",quote!(F))],quote!(impl #graph::Description<Value=N::Value,Error=N::Error,Env=N::Env>),quote!(N: #graph::Description,F: FnOnce(#path::PreparedScope<N::Error>)->N + Send + 'static),quote!(#graph::scope(#body)));
    }
    let mut fields = input.fields;
    let graph = quote!(#path::__private::prepared_graph);
    let binding = quote!(#graph::scope_binding);
    if mode == "construction" {
        return fields.finish(quote!(#binding::construction()));
    }
    if mode == "reserve_permit" {
        let scope = fields.transform("scope", |expr, _| Ok(quote!(&(#expr))))?;
        return fields.finish(quote!(#binding::ScopePermit::reserve(#scope)));
    }
    let permit = fields.take("permit")?;
    let scope = if fields.contains("scope") {
        Some(fields.transform("scope", |expr, _| Ok(quote!(&(#expr))))?)
    } else {
        None
    };
    let checked_target = scope.is_some();
    let recipe = if name == "effect_finalizer" {
        let cleanup = if mode == "release" {
            let callback = fields.transform("release", |expr, inline| {
                let expr = if inline {
                    syn::parse2(crate::domain_expand::role(
                        expr,
                        0,
                        crate::domain_expand::BodyCategory::Native,
                    )?)?
                } else {
                    expr
                };
                crate::prepared_program_expand::native_descriptor_callback(expr, path)
            })?;
            quote!((#callback)())
        } else {
            let source = match (fields.contains("program"), fields.contains("body")) {
                (true, false) => "program",
                (false, true) => "body",
                _ => {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "expected exactly one program or body",
                    ));
                }
            };
            fields.transform(source, |expr, inline| program_field(expr, inline, path))?
        };
        // Cleanup construction stays at release invocation, not registration.
        quote!(#binding::finalizer_recipe(move||#cleanup))
    } else {
        let acquire =
            fields.transform("acquire", |expr, inline| program_field(expr, inline, path))?;
        let release = release(&mut fields, path)?;
        quote!(#binding::recipe(#acquire,#release))
    };
    let bound = if let Some(scope) = scope {
        quote!((#permit).try_bind(#scope,#recipe))
    } else {
        quote!((#permit).bind(#recipe))
    };
    let bound = if name == "effect_finalizer" {
        if checked_target {
            quote!((#bound).map(|bound|#graph::bind(bound,move|_|#graph::success(()))))
        } else {
            quote!(#graph::bind(#bound,move|_|#graph::success(())))
        }
    } else {
        bound
    };
    let (output, mapper) = if fields.contains("on_error") {
        if !checked_target {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "on_error requires an explicitly checked scope target",
            ));
        }
        let mapper = fields.role("on_error", 1, crate::domain_expand::BodyCategory::Native)?;
        (quote!((#bound).map_err(#mapper)), true)
    } else {
        (bound, false)
    };
    if name == "effect_scope_acquire" {
        let mismatch = quote!(#binding::TargetMismatch<A::Error,A,F,C>);
        let mut arguments = vec![
            ("permit", quote!(#binding::ScopePermit<A::Error>)),
            ("acquire", quote!(A)),
            ("release", quote!(F)),
        ];
        if checked_target {
            arguments.push(("scope", quote!(&#path::PreparedScope<A::Error>)));
        }
        let constraints = quote!(A: #graph::Description,A::Value: Sync,C: #graph::Description<Value=(),Error=A::Error,Env=A::Env>,F: for<'a> FnOnce(A::Value,#path::PrimaryOutcomeRef<'a,A::Error>)->C + Send + 'static);
        let (generics, return_type, constraints) = if mapper {
            arguments.push(("on_error", quote!(M)));
            (
                quote!(A, F, C, M, X),
                quote!(::core::result::Result<#binding::BoundScoped<A,F,C>,X>),
                quote!(#constraints,M: FnOnce(#mismatch)->X),
            )
        } else if checked_target {
            (
                quote!(A, F, C),
                quote!(::core::result::Result<#binding::BoundScoped<A,F,C>,#mismatch>),
                constraints,
            )
        } else {
            (
                quote!(A, F, C),
                quote!(#binding::BoundScoped<A,F,C>),
                constraints,
            )
        };
        fields.finish_inferred(generics, &arguments, return_type, constraints, output)
    } else {
        fields.finish(output)
    }
}
