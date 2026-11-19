//! Existing Layer constructors/provision before ordinary Arc erasure.
use crate::callback_expand::Input;
use crate::domain_expand::BodyCategory;
use crate::prepared_program_expand::{effect_callback, program, program_field};
use crate::prepared_resource_expand::{native_release, release};
use proc_macro2::TokenStream;
use quote::quote;
pub(crate) fn layer_program(
    name: &str,
    input: Input,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let mode = input.mode(&["scoped", "merge", "provide", "try_build"], "default")?;
    let mut fields = input.fields;
    let graph = quote!(#path::__private::prepared_graph);
    let layers = quote!(#graph::layer);
    if name == "effect_with_layer" {
        let layer = fields.transform("layer", |expr, _| program(expr, path))?;
        let source_name = match (fields.contains("program"), fields.contains("body")) {
            (true, false) => "program",
            (false, true) => "body",
            _ => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "expected exactly one program or body",
                ));
            }
        };
        let source = fields.transform(source_name, |expr, inline| {
            program_field(expr, inline, path)
        })?;
        return fields.finish_inferred(quote!(L,D),&[("layer",quote!(L)),(source_name,quote!(D))],quote!(#layers::WithLayer<L,D>),quote!(L: #layers::LayerDescription,L::Output: Sync,D: #graph::Description<Env=L::Output,Error=L::Error>),quote!(#layers::with_layer(#layer,#source)));
    }
    if mode == "try_build" || mode == "default" && fields.contains("construction") {
        let checked = mode == "try_build";
        let construction = fields.transform("construction", |expr, _| {
            crate::prepared_program_expand::native_descriptors(expr, path)
        })?;
        let build = fields.transform("build", |expr, inline| {
            if checked {
                let expr = if inline {
                    syn::parse2(crate::domain_expand::role(expr, 2, BodyCategory::Native)?)?
                } else {
                    expr
                };
                crate::prepared_program_expand::native_descriptor_callback(expr, path)
            } else if inline {
                crate::prepared_program_expand::effect_callback_arity(expr, 2, path)
            } else {
                crate::prepared_program_expand::native_descriptor_callback(expr, path)
            }
        })?;
        let (wrapper, constructor, result) = if checked {
            (
                quote!(#layers::Checked<B>),
                quote!(try_construct),
                quote!(::core::result::Result<D,D::Error>),
            )
        } else {
            (quote!(#layers::Infallible<B>), quote!(construct), quote!(D))
        };
        return fields.finish_inferred(quote!(B,D,P),&[("build",quote!(B)),("construction",quote!(P))],quote!(#layers::Construct<#wrapper,D,P>),quote!(P: #graph::scope_binding::Construction,D: #graph::Description,B: FnOnce(#path::PreparedScope<D::Error>,P::Permits<D::Error>)->#result + Send + 'static),quote!(#layers::#constructor(#build,#construction)));
    }
    match mode.as_str() {
        "scoped" => {
            let acquire =
                fields.transform("acquire", |expr, inline| program_field(expr, inline, path))?;
            let release = release(&mut fields, path)?;
            fields.finish_inferred(quote!(A,F,C),&[("acquire",quote!(A)),("release",quote!(F))],quote!(#layers::Scoped<A,F,C>),quote!(A: #graph::Description,A::Value: Sync,C: #graph::Description<Value=(),Error=A::Error,Env=A::Env>,F: for<'a> FnOnce(A::Value,#path::PrimaryOutcomeRef<'a,A::Error>)->C + Send + 'static),quote!(#layers::scoped(#acquire,#release)))
        }
        "merge" => {
            let left = fields.transform("left", |expr, _| program(expr, path))?;
            let right = fields.transform("right", |expr, _| program(expr, path))?;
            let combine = fields.role("combine", 2, BodyCategory::Native)?;
            fields.finish_inferred(quote!(L,R,F,Q),&[("left",quote!(L)),("right",quote!(R)),("combine",quote!(F))],quote!(#layers::Merge<L,R,F,Q>),quote!(L: #layers::LayerDescription,R: #layers::LayerDescription<Input=L::Input,Error=L::Error>,F: FnOnce(L::Output,R::Output)->Q + Send + 'static,Q: Send + 'static),quote!(#layers::merge(#left,#right,#combine)))
        }
        "provide" => {
            let left = fields.transform("left", |expr, _| program(expr, path))?;
            let right = fields.transform("right", |expr, _| program(expr, path))?;
            fields.finish_inferred(quote!(L,R),&[("left",quote!(L)),("right",quote!(R))],quote!(#layers::Provide<L,R>),quote!(L: #layers::LayerDescription,L::Output: Sync,R: #layers::LayerDescription<Input=L::Output,Error=L::Error>),quote!(#layers::provide(#left,#right)))
        }
        "default" => {
            let build = fields.transform("build", |expr, inline| {
                if inline {
                    effect_callback(expr, path)
                } else {
                    native_release(expr, path)
                }
            })?;
            fields.finish_inferred(quote!(B,D),&[("build",quote!(B))],quote!(#layers::Build<B,D>),quote!(D: #graph::Description,B: FnOnce(#path::PreparedScope<D::Error>)->D + Send + 'static),quote!(#layers::build(#build)))
        }
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "unsupported Layer mode",
        )),
    }
}
