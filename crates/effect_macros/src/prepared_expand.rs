//! Concrete generated native-root layout/ownership entry points. Opaque Effect
//! graphs are not asserted prepared; dynamic companion adapters remain explicit.
use crate::callback_expand::Input;
use crate::domain_expand::BodyCategory;
use proc_macro2::TokenStream;
use quote::quote;
pub(crate) fn expand(input: Input) -> syn::Result<TokenStream> {
    let mode = input.mode(
        &[
            "plan",
            "graph",
            "description",
            "prepare",
            "admit",
            "native",
            "reserve_native",
            "native_reserved",
            "provider_result",
            "import",
            "scheduled",
            "fork",
            "race",
        ],
        "plan",
    )?;
    let mut fields = input.fields;
    let path = crate::expand::effect_crate_path()?;
    if mode == "description" {
        let description = fields.transform("program", |expression, _| {
            if let syn::Expr::Macro(invocation) = &expression
                && !crate::prepared_program_expand::is_owned_macro_path(&invocation.mac.path, &path)
            {
                return Ok(quote!(#expression));
            }
            crate::prepared_program_expand::program(expression, &path)
        })?;
        let output = match (
            fields.contains("capture_errors"),
            fields.contains("classify_error"),
        ) {
            (false, false) => quote!(#description),
            (true, true) => {
                let capture = fields.take("capture_errors")?;
                let classify = fields.role(
                    "classify_error",
                    1,
                    crate::domain_expand::BodyCategory::Native,
                )?;
                quote!(#path::__private::prepared_graph::capture(#description, #capture, #classify))
            }
            _ => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "capture_errors and classify_error must be supplied together",
                ));
            }
        };
        return fields.finish(output);
    }
    if mode == "graph" || mode == "plan" && (fields.contains("program") || fields.contains("input"))
    {
        let has_input = fields.contains("input");
        if has_input == fields.contains("program") {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "specify exactly one of input or program",
            ));
        }
        let recipe = if has_input {
            fields.transform("input", |expression, inline| {
                if !inline {
                    return Ok(quote!(#expression));
                }
                let syn::Expr::Closure(closure) = expression else {
                    return Err(syn::Error::new_spanned(
                        expression,
                        "input requires one parameter and a braced body",
                    ));
                };
                if closure.inputs.len() != 1
                    || closure.asyncness.is_some()
                    || !matches!(closure.output, syn::ReturnType::Default)
                {
                    return Err(syn::Error::new_spanned(
                        closure,
                        "input requires one parameter and a braced body",
                    ));
                }
                let syn::Expr::Block(block) = *closure.body else {
                    return Err(syn::Error::new_spanned(
                        closure.body,
                        "input body must be braced",
                    ));
                };
                let inputs = closure.inputs;
                let expression: syn::Expr = syn::parse2(quote!(#path::effect! #block))?;
                let body = crate::prepared_program_expand::root(expression, &path)?;
                Ok(quote!(move |#inputs| { #body }))
            })?
        } else {
            fields.transform("program", |expression, _| {
                let body = crate::prepared_program_expand::root(expression, &path)?;
                Ok(quote!(move || { #body }))
            })?
        };
        let contract = fields.take("contract")?;
        let plan = if has_input {
            quote!(#path::__private::prepared_graph::PreparedGraphPlan::new_with_input(#recipe,#contract))
        } else {
            quote!(#path::__private::prepared_graph::PreparedGraphPlan::new(#recipe,#contract))
        };
        let plan = if fields.contains("cleanup") {
            let cleanup = fields.take("cleanup")?;
            quote!((#plan).with_cleanup(#cleanup))
        } else {
            plan
        };
        let plan = if fields.contains("panic_diagnostics") {
            let diagnostics = fields.take("panic_diagnostics")?;
            quote!((#plan).with_panic_diagnostics(#diagnostics))
        } else {
            plan
        };
        let plan = instrumentation(&mut fields, plan)?;
        if fields.contains("construction") {
            let shape = fields.take("construction")?;
            let reserve = fields.role("reserve", 2, BodyCategory::Native)?;
            return fields.finish(quote!((#plan).with_construction(#shape,#reserve)));
        }
        return fields.finish(plan);
    }
    if mode == "fork" {
        let permit = fields.take("permit")?;
        let program = fields.transform("program", |expr, _| {
            crate::prepared_program_expand::root(expr, &path)
        })?;
        let kind = if fields.contains("provider_kind") {
            let kind = fields.raw("provider_kind")?;
            quote!(#kind)
        } else {
            quote!(#path::prepared_provider::TaskProvider)
        };
        let kind = if fields.contains("construction_key") {
            let key = fields.raw("construction_key")?;
            quote!(#path::prepared_provider::ConstructionProvider<#kind,{#key}>)
        } else {
            kind
        };
        let graph = quote!(#path::__private::prepared_graph);
        return fields.finish_inferred_selected(quote!(N,K),&[("permit",quote!(#graph::scope_binding::ScopePermit<N::Error>)),("program",quote!(N))],quote!(#graph::Fork<N,K>),quote!(N: #graph::Description,K: Send + 'static,N::Env: #path::prepared_provider::HasPreparedProvider<N::Value,N::Error,K>),quote!((#permit).fork_on::<N,K>(#program)),&[(quote!(K),kind)]);
    }
    if mode == "race" {
        let left = fields.transform("left", |expr, _| {
            crate::prepared_program_expand::root(expr, &path)
        })?;
        let right = fields.transform("right", |expr, _| {
            crate::prepared_program_expand::root(expr, &path)
        })?;
        let left_kind = if fields.contains("left_provider_kind") {
            let kind = fields.raw("left_provider_kind")?;
            quote!(#kind)
        } else {
            quote!(#path::prepared_provider::TaskProvider)
        };
        let right_kind = if fields.contains("right_provider_kind") {
            let kind = fields.raw("right_provider_kind")?;
            quote!(#kind)
        } else {
            quote!(#path::prepared_provider::TaskProvider)
        };
        let left_kind = if fields.contains("left_construction_key") {
            let key = fields.raw("left_construction_key")?;
            quote!(#path::prepared_provider::ConstructionProvider<#left_kind,{#key}>)
        } else {
            left_kind
        };
        let right_kind = if fields.contains("right_construction_key") {
            let key = fields.raw("right_construction_key")?;
            quote!(#path::prepared_provider::ConstructionProvider<#right_kind,{#key}>)
        } else {
            right_kind
        };
        let graph = quote!(#path::__private::prepared_graph);
        return fields.finish_inferred_selected(quote!(L,R,KL,KR),&[("left",quote!(L)),("right",quote!(R))],quote!(#graph::Race<L,R,KL,KR>),quote!(L: #graph::Description,R: #graph::Description<Value=L::Value,Error=L::Error,Env=L::Env>,KL: Send + 'static,KR: Send + 'static,L::Env: #path::prepared_provider::HasPreparedProvider<L::Value,L::Error,KL> + #path::prepared_provider::HasPreparedProvider<L::Value,L::Error,KR>),quote!(#graph::race_on::<L,R,KL,KR>(#left,#right)),&[(quote!(KL),left_kind),(quote!(KR),right_kind)]);
    }
    if mode == "import" {
        let key = fields.raw("key")?;
        if fields.contains("environment") {
            let environment = fields.transform("environment", |expression, inline| {
                crate::prepared_program_expand::program_field(expression, inline, &path)
            })?;
            return fields.finish(quote!(#path::__private::prepared_graph::import_with_environment::<_,_,{#key}>(#environment)));
        }
        return fields.finish(quote!(#path::__private::prepared_graph::import::<_,_,_,{#key}>()));
    }
    if mode == "scheduled" {
        let kind = fields.raw("provider_kind")?;
        let kind = if fields.contains("construction_key") {
            let key = fields.raw("construction_key")?;
            quote!(#path::prepared_provider::ConstructionProvider<#kind,{#key}>)
        } else {
            quote!(#kind)
        };
        let body = fields.role("body", 2, BodyCategory::Native)?;
        let graph = quote!(#path::__private::prepared_graph);
        return fields.finish_inferred_selected(quote!(F,P,A,E,R,K),&[("body",quote!(F))],
            quote!(#graph::Scheduled<F,P,A,E,R,K>),
            quote!(A: Send + 'static,E: Send + 'static,K: Send + 'static,R: #path::prepared_provider::HasPreparedProvider<A,E,K>,F: FnOnce(#graph::Environment<R>,#graph::FutureContext<E>)->P + Send + 'static,P: ::core::future::Future<Output=#path::Exit<A,E>> + Send + 'static),
            quote!(#graph::scheduled(#body)),&[(quote!(K),kind)]);
    }
    if mode == "provider_result" {
        let result = fields.take("result")?;
        let graph = quote!(#path::__private::prepared_graph);
        return fields.finish_inferred(quote!(F,A,D,E,R),
            &[("result",quote!(#path::prepared_provider::PreparedOwnedProviderResult<F,A,D>))],
            quote!(#graph::ProviderResultDescription<F,A,D,E,R>),
            quote!(F: ::core::future::Future<Output=A> + Send + 'static,A: Send + 'static,D: #path::prepared_provider::ProviderWorkingSet,E: Send + 'static,R: Send + Sync + 'static),
            quote!(#graph::provider_result(#result)));
    }
    if mode == "reserve_native" {
        let context = fields.take("context")?;
        return fields.finish_inferred(quote!(A,E),
            &[("context",quote!(#path::PreparedRunContext<A,E>))],
            quote!(::core::result::Result<#path::PreparedNativeReservation<A,E>,(#path::memory::MemoryError,#path::PreparedRunContext<A,E>)>),
            quote!(A: Send + 'static,E: Send + 'static),
            quote!((#context).reserve_native()));
    }
    if mode == "native_reserved" {
        let reservation = fields.take("reservation")?;
        let body = fields.role("body", 1, BodyCategory::Native)?;
        return fields.finish_inferred(quote!(T,A,E,F,Pending),
            &[("reservation",quote!(#path::PreparedNativeReservation<A,E>)),("body",quote!(F))],
            quote!(#path::PreparedNativeFuture<T,A,E,F,Pending>),
            quote!(A: Send + 'static,E: Send + 'static,F: FnOnce(#path::PreparedRunContext<A,E>) -> Pending,Pending: ::core::future::Future<Output = #path::Exit<T,E>>),
            quote!((#reservation).native(#body)));
    }
    if mode == "native" {
        let context = fields.take("context")?;
        let body = fields.role("body", 1, BodyCategory::Native)?;
        return fields.finish_inferred(quote!(T,A,E,F,Pending),
            &[("context",quote!(#path::PreparedRunContext<A,E>)),("body",quote!(F))],
            quote!(::core::result::Result<#path::PreparedNativeFuture<T,A,E,F,Pending>,(#path::memory::MemoryError,#path::PreparedRunContext<A,E>)>),
            quote!(A: Send + 'static, E: Send + 'static, F: FnOnce(#path::PreparedRunContext<A,E>) -> Pending, Pending: ::core::future::Future<Output = #path::Exit<T,E>>),
            quote!((#context).native(#body)));
    }
    let bounds = quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
        Pending: ::core::future::Future<Output = #path::Exit<A,E>> + Send + 'static,
        F: Fn(::std::sync::Arc<R>, #path::PreparedRunContext<A,E>, L) -> Pending + Send + Sync + 'static, G: Fn(&::std::sync::Arc<R>, &#path::PreparedRunContext<A,E>) -> ::core::result::Result<L,#path::memory::MemoryError> + Send + Sync + 'static, L: Send + 'static);
    if mode == "plan" {
        let body = fields.role("body", 3, BodyCategory::Native)?;
        let reserve = fields.role("reserve", 2, BodyCategory::Native)?;
        let contract = fields.take("contract")?;
        let types = [
            ("body", quote!(F)),
            ("reserve", quote!(G)),
            ("contract", quote!(#path::DynamicRunContract)),
            ("panic_diagnostics", quote!(#path::PreparedPanicPool)),
            ("trace", quote!(#path::PreparedTraceContext)),
            ("poll_instrumentation", quote!(#path::PreparedPollInstrumentation)),
        ];
        let plan = if fields.contains("panic_diagnostics") {
            let diagnostics = fields.take("panic_diagnostics")?;
            quote!(#path::PreparedRunPlan::new(#body,#reserve,#contract).with_panic_diagnostics(#diagnostics))
        } else {
            quote!(#path::PreparedRunPlan::new(#body,#reserve,#contract))
        };
        let plan = instrumentation(&mut fields, plan)?;
        return fields.finish_inferred(
            quote!(A, E, R, F, Pending, G, L),
            &types,
            quote!(#path::PreparedRunPlan<A,E,R,F,Pending,G,L>),
            bounds,
            plan,
        );
    }
    if mode == "prepare" {
        let plan = fields.take("plan")?;
        let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
        let region = fields.take("region")?;
        let capacity = fields.take("capacity")?;
        let frames = fields.take("frames")?;
        return fields.finish(quote!((#plan).prepare(#memory,#region,#capacity,#frames)));
    }
    let run = fields.take("run")?;
    let environment = fields.take("environment")?;
    if fields.contains("trace") {
        let trace = fields.transform("trace", |value, _| Ok(quote!(&(#value))))?;
        return fields.finish(quote!((#run).admit_with_prepared_trace(#environment,#trace)));
    }
    fields.finish(quote!((#run).admit(#environment)))
}

fn instrumentation(
    fields: &mut crate::domain_expand::Fields,
    mut plan: TokenStream,
) -> syn::Result<TokenStream> {
    if fields.contains("trace") {
        let trace = fields.take("trace")?;
        plan = quote!((#plan).with_prepared_trace(#trace));
    }
    if fields.contains("poll_instrumentation") {
        let hook = fields.take("poll_instrumentation")?;
        plan = quote!((#plan).with_prepared_poll_instrumentation(#hook));
    }
    Ok(plan)
}
