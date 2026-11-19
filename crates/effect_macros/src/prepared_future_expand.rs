//! Native future fields lower to concrete graph storage, never ordinary Effect.
use crate::callback_expand::Input;
use crate::domain_expand::BodyCategory;
use proc_macro2::TokenStream;
use quote::quote;

pub(crate) fn expand(name: &str, input: Input, path: &TokenStream) -> syn::Result<TokenStream> {
    if is_prepared_operation(&input) {
        return operation(name, input, path);
    }
    let mode = input.mode(&["once", "with_env", "exit_with_context"], "reusable")?;
    let mut fields = input.fields;
    let graph = quote!(#path::__private::prepared_graph);
    let contextual = mode == "with_env" || mode == "exit_with_context";
    let callback = fields.role("body", if contextual { 2 } else { 1 }, BodyCategory::Native)?;
    let arguments = if contextual {
        quote!(#graph::Environment<R>, #graph::FutureContext<E>)
    } else {
        quote!(#graph::FutureContext<E>)
    };
    let invoke = if contextual {
        quote!((#callback)(_environment, context))
    } else {
        quote!((#callback)(context))
    };
    if mode == "exit_with_context" {
        if name == "effect_try_future" {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "exit_with_context supplies Exit directly; use effect_future",
            ));
        }
        return fields.finish_inferred(quote!(F,P,A,E,R),&[("body",quote!(F))],
            quote!(impl #graph::Description<Value=A,Error=E,Env=R>),
            quote!(A: Send + 'static,E: Send + 'static,R: Send + Sync + 'static,F: FnOnce(#arguments)->P + Send + 'static,P: ::core::future::Future<Output=#path::Exit<A,E>> + Send + 'static),
            quote!(#graph::native_future(#callback)));
    }
    if name == "effect_try_future" {
        let explicit = fields.contains("on_error");
        let mapper = if explicit {
            fields.role("on_error", 1, BodyCategory::Native)?
        } else {
            quote!(::core::convert::identity)
        };
        // Keep mapper as a separate inferred field only when the caller wrote it.
        let mut inputs = vec![("body", quote!(F))];
        if explicit {
            inputs.push(("on_error", quote!(M)));
        }
        if explicit {
            return fields.finish_inferred(quote!(F,P,M,A,X,E,R),&inputs,
                quote!(impl #graph::Description<Value=A,Error=E,Env=R>),
                quote!(A: Send + 'static,E: Send + 'static,R: Send + Sync + 'static,F: FnOnce(#arguments)->P + Send + 'static,P: ::core::future::Future<Output=::core::result::Result<A,X>> + Send + 'static,M: FnOnce(X)->E + Send + 'static),
                quote!(#graph::native_future(move|_environment,context|async move {
                    match #invoke.await { Ok(value)=>#path::Exit::Success(value),Err(error)=>#path::Exit::Fail((#mapper)(error)) }
                })));
        }
        return fields.finish_inferred(quote!(F,P,A,E,R),&inputs,
            quote!(impl #graph::Description<Value=A,Error=E,Env=R>),
            quote!(A: Send + 'static,E: Send + 'static,R: Send + Sync + 'static,F: FnOnce(#arguments)->P + Send + 'static,P: ::core::future::Future<Output=::core::result::Result<A,E>> + Send + 'static),
            quote!(#graph::native_future(move|_environment,context|async move {
                match #invoke.await { Ok(value)=>#path::Exit::Success(value),Err(error)=>#path::Exit::Fail(error) }
            })));
    }
    fields.finish_inferred(quote!(F,P,A,E,R),&[("body",quote!(F))],
        quote!(impl #graph::Description<Value=A,Error=E,Env=R>),
        quote!(A: Send + 'static,E: Send + 'static,R: Send + Sync + 'static,F: FnOnce(#arguments)->P + Send + 'static,P: ::core::future::Future<Output=A> + Send + 'static),
        quote!(#graph::native_future(move|_environment,context|async move {#path::Exit::Success(#invoke.await)})))
}

/// CPU/blocking bodies remain synchronous native code on their selected real
/// prepared worker; the async wrapper has no native await or erased spawn.
pub(crate) fn job(name: &str, input: Input, path: &TokenStream) -> syn::Result<TokenStream> {
    let mode = input.mode(&["once", "with_context", "once_with_context"], "reusable")?;
    let fallible = name.starts_with("effect_try_");
    if fallible && mode == "once_with_context" {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "mode is not supported by this native constructor",
        ));
    }
    let contextual = mode.ends_with("with_context");
    let mut fields = input.fields;
    let graph = quote!(#path::__private::prepared_graph);
    let kind = if name.ends_with("cpu") {
        quote!(#path::prepared_provider::CpuProvider)
    } else {
        quote!(#path::prepared_provider::BlockingProvider)
    };
    let kind = if fields.contains("construction_key") {
        let key = fields.raw("construction_key")?;
        quote!(#path::prepared_provider::ConstructionProvider<#kind,{#key}>)
    } else {
        kind
    };
    let callback = fields.role("body", if contextual { 2 } else { 1 }, BodyCategory::Native)?;
    let arguments = if contextual {
        quote!(#graph::FutureContext<E>,#path::TraceContext)
    } else {
        quote!(#graph::FutureContext<E>)
    };
    let invoke = if contextual {
        quote!({let trace=context.trace().clone();(#callback)(context,trace)})
    } else {
        quote!((#callback)(context))
    };
    if !fallible {
        return fields.finish_inferred_selected(quote!(F,A,E,R,K),&[("body",quote!(F))],quote!(impl #graph::Description<Value=A,Error=E,Env=R>),quote!(A: Send + 'static,E: Send + 'static,K: Send + 'static,R: #path::prepared_provider::HasPreparedProvider<A,E,K>,F: FnOnce(#arguments)->A + Send + 'static),quote!(#graph::scheduled::<_,_,A,E,R,K>(move|_environment,context|async move {#path::Exit::Success(#invoke)})),&[(quote!(K),kind)]);
    }
    let explicit = fields.contains("on_error");
    let mapper = if explicit {
        fields.role("on_error", 1, BodyCategory::Native)?
    } else {
        quote!(::core::convert::identity)
    };
    if explicit {
        fields.finish_inferred_selected(quote!(F,M,A,X,E,R,K),&[("body",quote!(F)),("on_error",quote!(M))],quote!(impl #graph::Description<Value=A,Error=E,Env=R>),quote!(A: Send + 'static,E: Send + 'static,K: Send + 'static,R: #path::prepared_provider::HasPreparedProvider<A,E,K>,F: FnOnce(#arguments)->::core::result::Result<A,X> + Send + 'static,M: FnOnce(X)->E + Send + 'static),quote!(#graph::scheduled::<_,_,A,E,R,K>(move|_environment,context|async move {match #invoke {Ok(value)=>#path::Exit::Success(value),Err(error)=>#path::Exit::Fail((#mapper)(error))}})),&[(quote!(K),kind)])
    } else {
        fields.finish_inferred_selected(quote!(F,A,E,R,K),&[("body",quote!(F))],quote!(impl #graph::Description<Value=A,Error=E,Env=R>),quote!(A: Send + 'static,E: Send + 'static,K: Send + 'static,R: #path::prepared_provider::HasPreparedProvider<A,E,K>,F: FnOnce(#arguments)->::core::result::Result<A,E> + Send + 'static),quote!(#graph::scheduled::<_,_,A,E,R,K>(move|_environment,context|async move {match #invoke {Ok(value)=>#path::Exit::Success(value),Err(error)=>#path::Exit::Fail(error)}})),&[(quote!(K),kind)])
    }
}

pub(crate) fn is_prepared_operation(input: &Input) -> bool {
    input.mode.as_ref().is_some_and(|mode| {
        matches!(
            mode.to_string().as_str(),
            "prepared_queue_offer"
                | "prepared_queue_take"
                | "prepared_pubsub_publish"
                | "prepared_subscription_take"
                | "prepared_subscription_set"
        )
    })
}
/// Typed native operations retain the selected prepared owner across every await.
pub(crate) fn operation(name: &str, input: Input, path: &TokenStream) -> syn::Result<TokenStream> {
    if name != "effect_future" {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "prepared operation modes require effect_future",
        ));
    }
    let mode = input.mode(
        &[
            "prepared_queue_offer",
            "prepared_queue_take",
            "prepared_pubsub_publish",
            "prepared_subscription_take",
            "prepared_subscription_set",
        ],
        "prepared_queue_take",
    )?;
    let mut fields = input.fields;
    let selection = if mode.starts_with("prepared_queue") {
        "queue"
    } else if mode == "prepared_pubsub_publish" {
        "channel"
    } else if mode == "prepared_subscription_set" {
        "reference"
    } else {
        "subscription"
    };
    let selector = fields.role(selection, 1, BodyCategory::Native)?;
    let cleanup = fields.role("cleanup", 1, BodyCategory::Native)?;
    let capacity = fields.role("on_capacity", 1, BodyCategory::Native)?;
    let mapper = fields.role("on_error", 1, BodyCategory::Native)?;
    let candidate = if matches!(
        mode.as_str(),
        "prepared_queue_offer" | "prepared_pubsub_publish" | "prepared_subscription_set"
    ) {
        Some(fields.role("body", 1, BodyCategory::Native)?)
    } else {
        None
    };
    let graph = quote!(#path::__private::prepared_graph);
    let (selected, domain, value, state_bound, operation) = match mode.as_str() {
        "prepared_queue_offer" => {
            let body = candidate
                .as_ref()
                .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing body"))?;
            (
                quote!(#path::PreparedQueue<T>),
                quote!(#path::QueueError),
                quote!(()),
                quote!(T: Send + 'static,),
                quote! {
                    let queue = match environment.with(|env| (#selector)(env).try_clone()) {
                        Ok(Ok(queue)) => queue, Ok(Err(error)) => return #path::Exit::Fail((#capacity)(error)), Err(defect) => return #path::Exit::Die(defect),
                    };
                    let candidate_env = match environment.try_clone() { Ok(env) => env, Err(error) => return #path::Exit::Fail((#capacity)(error)) };
                    let factory = move || candidate_env.with(|env| (#body)(env));
                    let pending = match queue.offer_try_with_context(context, factory) { Ok(pending) => pending, Err((error, _context, _factory)) => return #path::Exit::Fail((#capacity)(error)) };
                    pending.await
                },
            )
        }
        "prepared_queue_take" => (
            quote!(#path::PreparedQueue<T>),
            quote!(#path::QueueError),
            quote!(#path::PreparedQueueItem<T>),
            quote!(T: Send + 'static,),
            quote! {
                let pending = match environment.with(|env| (#selector)(env).take_with_context(context)) {
                    Ok(Ok(pending)) => pending, Ok(Err((error, _context))) => return #path::Exit::Fail((#capacity)(error)), Err(defect) => return #path::Exit::Die(defect),
                };
                pending.await
            },
        ),
        "prepared_pubsub_publish" => {
            let body = candidate
                .as_ref()
                .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing body"))?;
            (
                quote!(#path::PreparedPubSub<T>),
                quote!(#path::PreparedPubSubError),
                quote!(#path::PublishReport),
                quote!(T: Send + Sync + 'static,),
                quote! {
                    let publication = match environment.with(|env| (#selector)(env).admit_publication()) {
                        Ok(Ok(publication)) => publication, Ok(Err(error)) => return #path::Exit::Fail((#capacity)(error)), Err(defect) => return #path::Exit::Die(defect),
                    };
                    let candidate_env = match environment.try_clone() { Ok(env) => env, Err(error) => return #path::Exit::Fail((#capacity)(error)) };
                    let factory = move || candidate_env.with(|env| (#body)(env)).map_err(#path::PreparedCandidateFailure::Defect);
                    publication.run_candidate_with_context(context, factory).await
                },
            )
        }
        "prepared_subscription_take" => (
            quote!(#path::PreparedSubscription<T>),
            quote!(#path::PubSubError),
            quote!(#path::PreparedSubscriptionEvent<T>),
            quote!(T: Send + Sync + 'static,),
            quote! {
                let pending = match environment.with(|env| (#selector)(env).take_with_context(context)) {
                    Ok(Ok(pending)) => pending, Ok(Err((error, _context))) => return #path::Exit::Fail((#capacity)(error)), Err(defect) => return #path::Exit::Die(defect),
                };
                pending.await
            },
        ),
        "prepared_subscription_set" => {
            let body = candidate
                .as_ref()
                .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing body"))?;
            (
                quote!(#path::PreparedSubscriptionRef<T>),
                quote!(#path::PreparedPubSubError),
                quote!(#path::PublishReport),
                quote!(T: Send + Sync + 'static,),
                quote! {
                    let publication = match environment.with(|env| (#selector)(env).admit_set()) {
                        Ok(Ok(publication)) => publication, Ok(Err(error)) => return #path::Exit::Fail((#capacity)(error)), Err(defect) => return #path::Exit::Die(defect),
                    };
                    let candidate_env = match environment.try_clone() { Ok(env) => env, Err(error) => return #path::Exit::Fail((#capacity)(error)) };
                    let factory = move || candidate_env.with(|env| (#body)(env)).map_err(#path::PreparedCandidateFailure::Defect);
                    publication.run_candidate_with_context(context, factory).await
                },
            )
        }
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "invalid prepared operation mode",
            ));
        }
    };
    let mut input_types = vec![
        (selection, quote!(S)),
        ("cleanup", quote!(C)),
        ("on_capacity", quote!(K)),
        ("on_error", quote!(M)),
    ];
    let (generics, candidate_bound) = if candidate.is_some() {
        input_types.push(("body", quote!(F)));
        (
            quote!(S, C, K, M, F, T, E, R),
            quote!(F: FnOnce(&R)->T + Send + 'static,),
        )
    } else {
        (quote!(S, C, K, M, T, E, R), quote!())
    };
    fields.finish_inferred(generics, &input_types, quote!(impl #graph::Description<Value=#value,Error=E,Env=R>),
        quote!(#state_bound E: Send + 'static, R: Send + Sync + 'static, S: for<'a> FnOnce(&'a R)-> &'a #selected + Send + 'static, C: for<'a> FnOnce(&'a R)-> &'a #path::PreparedCleanupConfig<E> + Send + 'static, K: FnOnce(#path::memory::MemoryError)->E + Send + 'static, M: FnMut(#domain)->E + Send + 'static, #candidate_bound),
        quote!(#graph::native_future(move |environment: #graph::Environment<R>, context: #graph::FutureContext<E>| async move {
            let target = match environment.with(|env| #path::PreparedExitMapper::admit((#cleanup)(env))) {
                Ok(Ok(target)) => target, Ok(Err(error)) => return #path::Exit::Fail((#capacity)(error)), Err(defect) => return #path::Exit::Die(defect),
            };
            let outcome = { #operation };
            target.map(outcome, #mapper)
        })))
}
