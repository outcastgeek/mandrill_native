//! Ordered field parsing and construction for the cross-cutting domain DSLs.

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Token};
use syn::{bracketed, parse_quote};

/// A field retains its source position and distinguishes lowering from typed callback reuse.
struct Field {
    /// User-visible field name and diagnostic span.
    name: Ident,
    /// Original expression; pair lists retain their own grammar until the owning expander consumes them.
    value: Expr,
    /// Colon-free body syntax opts into lowering instead of passing an already typed expression.
    inline: bool,
    /// Lowered runtime initializer, emitted in source order after validation.
    binding: Option<(Ident, TokenStream)>,
    /// Whether the owning expander consumed the field.
    taken: bool,
}
/// Ordered fields. Expressions evaluate once in written order; generated identities are hygienic locals only.
pub(crate) struct Fields {
    /// Fields in source order, with no sorting or duplicate overwrites.
    fields: Vec<Field>,
}
impl Parse for Fields {
    /// Validate duplicate fields at their own spans while preserving runtime expression order.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut fields: Vec<Field> = Vec::new();
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            if fields.iter().any(|field| field.name == name) {
                return Err(syn::Error::new(name.span(), "duplicate descriptor field"));
            }
            let inline = if input.peek(Token![:]) {
                input.parse::<Token![:]>()?;
                false
            } else {
                true
            };
            if inline && !inline_role(&name.to_string()) {
                return Err(syn::Error::new(
                    name.span(),
                    "expected `:` after metadata field",
                ));
            }
            let value = if name == "bindings" || name == "cases" {
                let content;
                bracketed!(content in input);
                let pairs = content.parse::<Pairs>()?;
                let tokens = pairs.tokens();
                Expr::Verbatim(quote!([#tokens]))
            } else if name == "handlers" && input.peek(syn::token::Bracket) {
                let content;
                bracketed!(content in input);
                let tokens: TokenStream = content.parse()?;
                syn::parse2::<Expr>(quote!([#tokens]))
                    .unwrap_or_else(|_| Expr::Verbatim(quote!([#tokens])))
            } else if inline {
                parse_role(input)?
            } else {
                input.parse()?
            };
            fields.push(Field {
                name,
                value,
                inline,
                binding: None,
                taken: false,
            });
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(Self { fields })
    }
}
impl Fields {
    /// Test presence without consuming or evaluating a field.
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.fields
            .iter()
            .any(|field| field.name == name && !field.taken)
    }
    /// Consume a runtime expression and reference its ordered, once-evaluated local.
    pub(crate) fn take(&mut self, name: &str) -> syn::Result<TokenStream> {
        self.transform(name, |value, _| Ok(quote!(#value)))
    }
    /// Consume a syntax-only value such as an environment type; runtime lists use ordered bindings.
    pub(crate) fn raw(&mut self, name: &str) -> syn::Result<Expr> {
        let field = self
            .fields
            .iter_mut()
            .find(|field| field.name == name && !field.taken)
            .ok_or_else(|| {
                syn::Error::new(
                    Span::call_site(),
                    format!("missing descriptor field: {name}"),
                )
            })?;
        field.taken = true;
        Ok(field.value.clone())
    }
    /// Transform a field before its runtime initializer is emitted; body lowering stays static.
    pub(crate) fn transform(
        &mut self,
        name: &str,
        lower: impl FnOnce(Expr, bool) -> syn::Result<TokenStream>,
    ) -> syn::Result<TokenStream> {
        let (index, field) = self
            .fields
            .iter_mut()
            .enumerate()
            .find(|(_, field)| field.name == name && !field.taken)
            .ok_or_else(|| {
                syn::Error::new(
                    Span::call_site(),
                    format!("missing descriptor field: {name}"),
                )
            })?;
        let value = lower(field.value.clone(), field.inline)?;
        let local = Ident::new(&format!("__domain_field_{index}"), Span::mixed_site());
        field.binding = Some((local.clone(), value));
        field.taken = true;
        Ok(quote!(#local))
    }
    /// Lower an inline callback while passing colon-form reusable callbacks unchanged.
    pub(crate) fn callback(&mut self, name: &str, parameters: usize) -> syn::Result<TokenStream> {
        self.transform(name, |expr, inline| {
            if inline {
                callback(expr, parameters)
            } else {
                Ok(quote!(#expr))
            }
        })
    }
    /// Lower one named role at its actual Rust boundary. Colon forms retain the
    /// extracted callback's type; inline patterns are checked before code generation.
    pub(crate) fn role(
        &mut self,
        name: &str,
        arity: usize,
        category: BodyCategory,
    ) -> syn::Result<TokenStream> {
        self.transform(name, |expression, inline| {
            if !inline {
                if matches!(category, BodyCategory::SyncEffect) && arity == 2 {
                    let path = crate::expand::effect_crate_path()?;
                    return Ok(quote!(#path::__private::sync_callback2(#expression)));
                }
                return Ok(quote!(#expression));
            }
            role(expression, arity, category)
        })
    }
    /// Distinguish an inline role from a colon-form expression without evaluating it.
    pub(crate) fn inline(&self, name: &str) -> bool {
        self.fields
            .iter()
            .any(|field| field.name == name && !field.taken && field.inline)
    }
    /// Observe callback syntax without evaluating it, for explicit arity-selected modes.
    pub(crate) fn arity(&self, name: &str) -> syn::Result<usize> {
        let field = self
            .fields
            .iter()
            .find(|field| field.name == name && !field.taken)
            .ok_or_else(|| syn::Error::new(Span::call_site(), format!("missing role: {name}")))?;
        if let Expr::Block(_) = &field.value {
            Ok(0)
        } else if let Expr::Closure(closure) = &field.value {
            Ok(closure.inputs.len())
        } else if !field.inline {
            Err(syn::Error::new(
                field.name.span(),
                "extracted role requires an explicit mode",
            ))
        } else {
            Err(syn::Error::new_spanned(
                &field.value,
                "role requires a braced body",
            ))
        }
    }
    /// Validate unconsumed fields and emit initializer bindings in the original source order.
    pub(crate) fn finish(self, output: TokenStream) -> syn::Result<TokenStream> {
        if let Some(field) = self.fields.iter().find(|field| !field.taken) {
            return Err(syn::Error::new(
                field.name.span(),
                format!("unknown or conflicting descriptor field: {}", field.name),
            ));
        }
        let bindings = self
            .fields
            .into_iter()
            .filter_map(|field| field.binding)
            .map(|(local, value)| quote!(let #local = #value;));
        Ok(quote!({ #(#bindings)* #output }))
    }

    /// Keep callback expressions at a typed argument boundary. Arguments follow
    /// written field order, including callbacks placed before their program.
    pub(crate) fn finish_inferred(
        self,
        generics: TokenStream,
        types: &[(&str, TokenStream)],
        returns: TokenStream,
        bounds: TokenStream,
        output: TokenStream,
    ) -> syn::Result<TokenStream> {
        if let Some(field) = self.fields.iter().find(|field| !field.taken) {
            return Err(syn::Error::new(
                field.name.span(),
                format!("unknown or conflicting descriptor field: {}", field.name),
            ));
        }
        let mut parameters = Vec::new();
        let mut arguments = Vec::new();
        for field in self.fields {
            if let Some((local, value)) = field.binding {
                let ty = types
                    .iter()
                    .find(|(name, _)| field.name == *name)
                    .map(|(_, ty)| ty)
                    .ok_or_else(|| {
                        syn::Error::new(field.name.span(), "missing typed role boundary")
                    })?;
                parameters.push(quote!(#local: #ty));
                arguments.push(value);
            }
        }
        let apply = Ident::new("__effect_operator", Span::mixed_site());
        Ok(quote!({
            fn #apply<#generics>(#(#parameters),*) -> #returns where #bounds { #output }
            #apply(#(#arguments),*)
        }))
    }
}
/// Typed definition/fold or source/schema pair; the two expressions stay statically typed.
pub(crate) struct Pair {
    /// Definition or source expression evaluated once.
    pub(crate) left: Expr,
    /// Typed callback or schema associated with the left expression.
    pub(crate) right: Expr,
}
/// Ordered pair-list syntax used by hydration and typed projection cases.
pub(crate) struct Pairs {
    /// Pairs in source order, retaining their expression spans.
    pub(crate) values: Vec<Pair>,
}
impl Parse for Pairs {
    /// Consume `expression => expression` entries without reinterpreting nested macro tokens.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut values = Vec::new();
        while !input.is_empty() {
            let left = input.parse()?;
            input.parse::<Token![=>]>()?;
            let right = input.parse()?;
            values.push(Pair { left, right });
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(Self { values })
    }
}
impl Pairs {
    /// Preserve the grammar as syntax until the owning domain assigns typed constructor operations.
    fn tokens(&self) -> TokenStream {
        let entries = self.values.iter().map(|pair| {
            let left = &pair.left;
            let right = &pair.right;
            quote!(#left => #right)
        });
        quote!(#(#entries),*)
    }
}
/// Parse an explicitly bracketed pair-list expression retained by Fields.
pub(crate) fn pairs(expression: Expr) -> syn::Result<Pairs> {
    /// Bracket wrapper for parsing without exposing it as stable Rust expression syntax.
    struct Bracketed(
        /// Ordered pairs retained from the enclosing brackets for the owning descriptor.
        Pairs,
    );
    impl Parse for Bracketed {
        /// Delegate entries to the ordered pair grammar.
        fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
            let content;
            bracketed!(content in input);
            Ok(Self(content.parse()?))
        }
    }
    syn::parse2::<Bracketed>(quote!(#expression)).map(|value| value.0)
}
/// Parse the common inline role expression, preserving Rust typed/destructuring patterns.
pub(crate) fn parse_role(input: ParseStream<'_>) -> syn::Result<Expr> {
    input.parse()
}
/// Lower a Rust callback body to a fresh one-shot Effect for each owned input invocation.
/// Explicit handle copies in the pure prefix run at callback execution before owned locals
/// enter the future, retaining a reusable callback without fabricating capture clones.
pub(crate) fn callback(expression: Expr, parameters: usize) -> syn::Result<TokenStream> {
    let Expr::Closure(closure) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "inline body requires a Rust closure",
        ));
    };
    if closure.asyncness.is_some()
        || closure.inputs.len() != parameters
        || !matches!(closure.output, syn::ReturnType::Default)
    {
        return Err(syn::Error::new_spanned(
            closure,
            "body closure has the wrong parameters or return annotation",
        ));
    }
    let Expr::Block(block) = *closure.body else {
        return Err(syn::Error::new_spanned(
            closure.body,
            "inline body must be braced",
        ));
    };
    let inputs = closure.inputs;
    let (prefix, statements) = crate::context_expand::split_prefix(block.block.stmts);
    let body = crate::expand::expand_once(statements)?;
    let path = crate::expand::effect_crate_path()?;
    let adapter = Ident::new(&format!("defer_callback{parameters}"), Span::mixed_site());
    Ok(quote!(#path::__private::#adapter(move |#inputs| {#(#prefix)* #body})))
}
/// Lower exactly one inline body or reuse a typed Effect expression.
fn effect_body(fields: &mut Fields) -> syn::Result<TokenStream> {
    match (fields.contains("body"), fields.contains("effect")) {
        (true, false) => fields.transform("body", |expr, _| {
            let Expr::Block(block) = expr else {
                return Err(syn::Error::new_spanned(expr, "body must be braced"));
            };
            crate::expand::expand(block.block.stmts)
        }),
        (false, true) => fields.take("effect"),
        _ => Err(syn::Error::new(
            Span::call_site(),
            "specify exactly one of body or effect",
        )),
    }
}
/// Instrument an ordinary Effect, installing annotations around span creation and execution.
pub(crate) fn span(mut fields: Fields) -> syn::Result<TokenStream> {
    let name = if fields.contains("name") {
        Some(fields.take("name")?)
    } else {
        None
    };
    let body = effect_body(&mut fields)?;
    let output = if let Some(name) = name {
        quote!((#body).with_span(#name))
    } else if fields.contains("annotations") {
        quote!(#body)
    } else {
        return Err(syn::Error::new(
            Span::call_site(),
            "span requires name or annotations",
        ));
    };
    let output = if fields.contains("annotations") {
        let annotations = fields.take("annotations")?;
        quote!((#output).annotate(#annotations))
    } else {
        output
    };
    fields.finish(output)
}
/// Select an application service or explicitly project its environment, with no separate runtime.
pub(crate) fn service(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    if fields.contains("env") {
        // Preserve an explicit native environment type without annotating the
        // consumer callback; the adapter evaluates the factory exactly once.
        let environment = if fields.contains("environment") {
            Some(fields.raw("environment")?)
        } else {
            None
        };
        let select = fields.transform("env", |expression, inline| {
            let expression = if inline {
                role(expression, 1, BodyCategory::Native)?
            } else {
                quote!(#expression)
            };
            Ok(if let Some(environment) = environment {
                let select = Ident::new("__service_select", Span::mixed_site());
                quote!({
                    fn #select<R, A, F>(select: F) -> F where F: Fn(&R) -> A { select }
                    #select::<#environment, _, _>(#expression)
                })
            } else {
                expression
            })
        })?;
        return fields.finish_inferred(
            quote!(A, E, R, F),
            &[("env", quote!(F))],
            quote!(#path::Effect<A, E, R>),
            quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
                F: Fn(&R) -> A + Send + Sync + 'static),
            quote!(#path::Effect::service(#select)),
        );
    }
    if fields.contains("operation") {
        let operation = fields.role("operation", 1, BodyCategory::EffectFactory)?;
        return fields.finish(quote!(#path::ServiceOperation::new(#operation)));
    }
    if fields.contains("sync") || fields.contains("future") {
        let native = if fields.contains("sync") {
            "sync"
        } else {
            "future"
        };
        let callback = fields.role(native, 1, BodyCategory::Native)?;
        let mapper = if fields.contains("on_error") {
            fields.role("on_error", 1, BodyCategory::Native)?
        } else {
            quote!(::core::convert::identity)
        };
        let method = if native == "sync" {
            quote!(try_sync)
        } else {
            quote!(try_future)
        };
        return fields.finish(quote!(#path::ServiceOperation::#method(#callback, #mapper)));
    }
    if fields.contains("acquire") {
        let acquire = fields.role("acquire", 1, BodyCategory::EffectFactory)?;
        let release = fields.role("release", 1, BodyCategory::Effect)?;
        return fields.finish(quote!(#path::ServiceResource::new(#acquire, #release)));
    }
    if fields.contains("project") {
        let project = fields.role("project", 1, BodyCategory::Native)?;
        let body = effect_body(&mut fields)?;
        return fields.finish(quote!((#body).provide_some(#project)));
    }
    let selected = match (fields.contains("select"), fields.contains("handle")) {
        (true, false) => {
            let select = fields.role("select", 1, BodyCategory::Native)?;
            quote!(#path::Effect::service(#select))
        }
        (false, true) => {
            let handle = fields.take("handle")?;
            quote!(#path::Effect::sync_once(move || #handle))
        }
        _ => {
            return Err(syn::Error::new(
                Span::call_site(),
                "specify exactly one of select or handle, or use project",
            ));
        }
    };
    let mapped = fields.contains("on_error");
    let mapper = if mapped {
        fields.role("on_error", 1, BodyCategory::Native)?
    } else {
        quote!(::core::convert::identity)
    };
    let inline = fields
        .fields
        .iter()
        .any(|field| field.name == "run" && field.inline);
    if !inline {
        let run = fields.take("run")?;
        // A typed callback already owns its application error boundary. Merely declaring a
        // provider mapper must not transform unrelated errors returned by that callback.
        return fields.finish(quote!({let _=&#mapper;(#selected).flat_map(#run)}));
    }
    let run = fields.transform("run", |expr, _| {
        let Expr::Closure(closure) = expr else {
            return Err(syn::Error::new_spanned(expr, "run requires a closure"));
        };
        if closure.inputs.len() != 1 || closure.asyncness.is_some() {
            return Err(syn::Error::new_spanned(
                closure,
                "run requires one synchronous service parameter",
            ));
        }
        let pattern = closure
            .inputs
            .first()
            .ok_or_else(|| syn::Error::new_spanned(&closure, "run requires a service parameter"))?
            .clone();
        let Expr::Block(block) = *closure.body else {
            return Err(syn::Error::new_spanned(
                closure.body,
                "run body must be braced",
            ));
        };
        let context = Ident::new("__domain_context", Span::mixed_site());
        let (prefix, mut statements) = crate::context_expand::split_prefix(block.block.stmts);
        statements.insert(
            0,
            parse_quote!(#[allow(unused_variables)] let #pattern=#context.service();),
        );
        let callback = crate::context_expand::callback(statements);
        // The selection callback is reusable. Explicit prefix handle copies occur at
        // selection execution before the one-shot factory moves its evaluation locals.
        Ok(quote!(move |__service_handle,__service_mapper| {
            #[allow(unused_variables)] let #pattern=&__service_handle;
            #(#prefix)*
            #path::ServiceBody::from_body(__service_handle,__service_mapper,#callback)
        }))
    })?;
    fields.finish(quote!({let __service_mapper: ::std::sync::Arc<dyn Fn(_) -> _ + Send + Sync> = ::std::sync::Arc::new(#mapper);(#selected).flat_map(move |service|#run(service,::std::sync::Arc::clone(&__service_mapper)))}))
}
/// Declare scoped cache acquisition with all caller-supplied configuration and admission options.
pub(crate) fn cache(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let scope = fields.take("scope")?;
    let config = fields.take("config")?;
    let options = fields.take("options")?;
    let role = if fields.contains("load_shared") {
        "load_shared"
    } else {
        "load"
    };
    let inline = fields
        .fields
        .iter()
        .any(|field| field.name == role && field.inline);
    let load = fields.callback(role, 1)?;
    let method = if inline || role == "load_shared" {
        quote!(make_with_shared_key)
    } else {
        quote!(make_with_options)
    };
    let key = if inline || role == "load_shared" {
        quote!(::std::sync::Arc<K>)
    } else {
        quote!(&K)
    };
    fields.finish_inferred(
        quote!(K, A, E, R, F),
        &[
            ("scope", quote!(#path::Scope<#path::SharedValue<E>>)),
            ("config", quote!(#path::CacheConfig<K, E>)),
            ("options", quote!(#path::CacheOptions<A, E>)),
            ("load", quote!(F)),
            ("load_shared", quote!(F)),
        ],
        quote!(#path::Effect<#path::Cache<K, A, E>, #path::SharedValue<E>, R>),
        quote!(K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static,
            R: #path::HasClock + #path::HasTaskSpawner + Send + Sync + 'static,
            F: Fn(#key) -> #path::Effect<A, E, R> + Send + Sync + 'static),
        quote!(#path::Cache::#method(#scope,#config,#options,#load)),
    )
}
/// Declare a dependency-aware cache using its explicit policy and typed loader.
pub(crate) fn dependency_cache(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let scope = fields.take("scope")?;
    let policy = fields.take("policy")?;
    let inline = fields
        .fields
        .iter()
        .any(|field| field.name == "load" && field.inline);
    let load = fields.callback("load", 1)?;
    let method = if inline {
        quote!(make_with_shared_key)
    } else {
        quote!(make)
    };
    let key = if inline {
        quote!(::std::sync::Arc<K>)
    } else {
        quote!(&K)
    };
    fields.finish_inferred(
        quote!(K, D, A, E, R, F),
        &[("scope", quote!(#path::Scope<#path::SharedValue<#path::DependencyCacheError<E>>>)),
            ("policy", quote!(#path::DependencyCachePolicy<K, D, A, E>)), ("load", quote!(F))],
        quote!(#path::Effect<#path::DependencyCache<K, D, A, E>, #path::SharedValue<#path::DependencyCacheError<E>>, R>),
        quote!(K: Send + Sync + 'static, D: Send + Sync + 'static,
            A: Send + 'static, E: Send + 'static,
            R: #path::HasClock + #path::HasTaskSpawner + Send + Sync + 'static,
            F: Fn(#key) -> #path::Effect<(A, ::std::vec::Vec<D>), E, R> + Send + Sync + 'static),
        quote!(#path::DependencyCache::#method(#scope,#policy,#load)),
    )
}
/// Gather heterogeneous hydration bindings through the existing registry/duplicate checks.
pub(crate) fn hydration(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let registry = fields.take("registry")?;
    let limits = fields.take("limits")?;
    let (bindings, entries) = pair_bindings(&mut fields, "bindings", false)?;
    let (sources, schemas): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
    fields.finish(
        quote!((|| -> ::core::result::Result<_,#path::reactivity::HydrationError> {
        let (#((#sources,#schemas),)*)=#bindings;
        let mut set=#path::reactivity::HydrationSet::new(#registry,#limits);
        #(set.register(#sources,#schemas)?;)* Ok(set)
    })()),
    )
}
/// Execute one singleton body under the existing activation and lease protocol.
pub(crate) fn singleton(mut fields: Fields) -> syn::Result<TokenStream> {
    if fields.contains("address") {
        let path = crate::expand::effect_crate_path()?;
        let address = fields.take("address")?;
        return fields.finish(quote!(#path::cluster::ClusterSingleton::new(#address)));
    }
    let singleton = fields.take("singleton")?;
    let activation = fields.take("activation")?;
    let lease = fields.take("lease_until")?;
    let run = fields.callback("run", 1)?;
    fields.finish(quote!((#singleton).run(#activation,#lease,#run)))
}
/// Construct the existing cron schedule with every supported calendar and overlap policy explicit.
pub(crate) fn cluster_schedule(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let id = fields.take("id")?;
    let cron = fields.take("cron")?;
    let transitions = fields.take("transitions")?;
    let catch_up = fields.take("catch_up")?;
    let concurrent = fields.take("concurrent")?;
    let deadline = fields.take("deadline_after")?;
    fields.finish(quote!(#path::cluster::ClusterSchedule::new(#id,#cron,#transitions,#catch_up,#concurrent,#deadline)))
}
/// Construct a typed persisted queue without changing codec identities or durability authority.
pub(crate) fn persisted_queue(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let name = fields.take("name")?;
    let payload = fields.take("payload")?;
    let identity = fields.take("identity")?;
    let limits = fields.take("limits")?;
    let policy = fields.take("policy")?;
    fields.finish(
        quote!(#path::persistence::PersistedQueue::new(#name,#payload,#identity,#limits,#policy)),
    )
}
/// Wrap a queue or execute its existing recorded-result-before-acknowledgement worker boundary.
pub(crate) fn durable_queue(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let queue = fields.take("queue")?;
    if fields.contains("delivery") {
        let delivery = fields.take("delivery")?;
        let deferred = fields.take("deferred")?;
        let acknowledged = fields.take("acknowledged_at")?;
        let handler = fields.callback("handler", 1)?;
        return fields.finish(quote!((#queue).handle(#delivery,#deferred,#handler,#acknowledged)));
    }
    let output = quote!(#path::workflow::DurableQueue::new(#queue));
    let output = if fields.contains("environment") {
        let environment = fields.raw("environment")?;
        quote!((#output).require_physical_durability::<#environment>())
    } else {
        output
    };
    fields.finish(output)
}

/// Declare a unary operation, optionally pairing it with an inline or reusable typed handler.
pub(crate) fn rpc_operation(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    if fields.contains("operation") {
        let operation = fields.take("operation")?;
        let handler = fields.role("handler", 1, BodyCategory::EffectFactory)?;
        return fields.finish_inferred(
            quote!(Q, A, E, R, F),
            &[("operation", quote!(#path::rpc::RpcOperation<Q, A, E>)), ("handler", quote!(F))],
            quote!(#path::rpc::RpcHandler<Q, A, E, R>),
            quote!(Q: Send + 'static, A: Send + 'static, E: Send + 'static,
                R: Send + Sync + 'static, F: Fn(Q) -> #path::Effect<A, E, R> + Send + Sync + 'static),
            quote!(#path::rpc::RpcHandler::new(#operation, #handler)),
        );
    }
    let id = fields.take("id")?;
    let version = fields.take("version")?;
    let request = fields.take("request")?;
    let success = fields.take("success")?;
    let error = fields.take("error")?;
    let limits = fields.take("limits")?;
    let operation =
        quote!(#path::rpc::RpcOperation::unary(#id,#version,#request,#success,#error,#limits));
    let operation = if fields.contains("idempotency_key") {
        let key = fields.take("idempotency_key")?;
        quote!((#operation).with_idempotency_key(#key))
    } else {
        operation
    };
    let output = if fields.contains("handler") {
        let handler = fields.callback("handler", 1)?;
        quote!(#path::rpc::RpcHandler::new(#operation,#handler))
    } else {
        operation
    };
    let output = if fields.contains("project") {
        let project = fields.role("project", 1, BodyCategory::Native)?;
        quote!((#output).provide_some(#project))
    } else {
        output
    };
    fields.finish(output)
}
/// Declare streaming codecs and an optional request/context handler returning its typed producer Effect.
pub(crate) fn rpc_stream_operation(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    if fields.contains("operation") {
        let operation = fields.take("operation")?;
        let handler = fields.role("handler", 2, BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(Q, C, T, E, R, F),
            &[("operation", quote!(#path::rpc::RpcStreamOperation<Q, C, T, E>)), ("handler", quote!(F))],
            quote!(#path::rpc::RpcStreamHandler<Q, C, T, E, R>),
            quote!(Q: Send + 'static, C: Send + 'static, T: Send + 'static, E: Send + 'static,
                R: Send + Sync + 'static,
                F: Fn(Q, #path::rpc::RpcStreamContext) -> #path::Effect<#path::rpc::RpcStreamProducer<C, T, E, R>, E, R> + Send + Sync + 'static),
            quote!(#path::rpc::RpcStreamHandler::new(#operation, #handler)),
        );
    }
    let id = fields.take("id")?;
    let version = fields.take("version")?;
    let request = fields.take("request")?;
    let chunk = fields.take("chunk")?;
    let terminal = fields.take("terminal")?;
    let error = fields.take("error")?;
    let limits = fields.take("limits")?;
    let operation = quote!(#path::rpc::RpcStreamOperation::new(#id,#version,#request,#chunk,#terminal,#error,#limits));
    let output = if fields.contains("handler") {
        let handler = fields.callback("handler", 2)?;
        quote!(#path::rpc::RpcStreamHandler::new(#operation,#handler))
    } else {
        operation
    };
    let output = if fields.contains("project") {
        let project = fields.role("project", 1, BodyCategory::Native)?;
        quote!((#output).provide_some(#project))
    } else {
        output
    };
    fields.finish(output)
}
/// Evaluate a runtime handler list once at its written field position, retaining heterogeneous types.
fn handler_bindings(fields: &mut Fields, name: &str) -> syn::Result<(TokenStream, Vec<Ident>)> {
    let mut locals = Vec::new();
    let value = fields.transform(name, |expr, _| {
        let Expr::Array(array) = expr else {
            return Err(syn::Error::new_spanned(
                expr,
                "handlers must be a bracketed list",
            ));
        };
        if array.elems.is_empty() {
            return Err(syn::Error::new_spanned(
                array,
                "protocol requires at least one typed handler",
            ));
        }
        locals = (0..array.elems.len())
            .map(|index| Ident::new(&format!("__{name}_handler_{index}"), Span::mixed_site()))
            .collect();
        let entries = array.elems.into_iter().collect::<Vec<_>>();
        Ok(quote!((#(#entries,)*)))
    })?;
    Ok((value, locals))
}
/// Build the existing checked registry only after ordered handler/policy expression evaluation.
fn protocol_output(
    value: TokenStream,
    locals: Vec<Ident>,
    policy: &TokenStream,
    streaming: bool,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let mut entries = locals.iter();
    let first = entries
        .next()
        .ok_or_else(|| syn::Error::new(Span::call_site(), "protocol requires a handler"))?;
    let constructor = if streaming {
        quote!(#path::rpc::RpcStreamProtocol)
    } else {
        quote!(#path::rpc::RpcProtocol)
    };
    Ok(
        quote!((|| -> ::core::result::Result<_,#path::rpc::RpcRegistryError>{
        let (#(#locals,)*)=#value;
        let mut protocol=#constructor::from_handler(#policy,#first)?;
        #(protocol.register(#entries)?;)* Ok(protocol)
    })()),
    )
}
/// Build the retained unary registry or an explicit grouping of separate unary/streaming registries.
pub(crate) fn rpc_protocol(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let policy = fields.take("policy")?;
    if fields.contains("unary") || fields.contains("streaming") {
        let (unary, unary_locals) = handler_bindings(&mut fields, "unary")?;
        let (streaming, streaming_locals) = handler_bindings(&mut fields, "streaming")?;
        let unary = protocol_output(unary, unary_locals, &policy, false, &path)?;
        let streaming = protocol_output(streaming, streaming_locals, &policy, true, &path)?;
        return fields.finish(quote!((|| -> ::core::result::Result<_,#path::rpc::RpcRegistryError>{Ok(#path::rpc::RpcProtocolGroup{unary:#unary?,streaming:#streaming?})})()));
    }
    let (handlers, locals) = handler_bindings(&mut fields, "handlers")?;
    let output = protocol_output(handlers, locals, &policy, false, &path)?;
    fields.finish(output)
}
/// Build the retained streaming registry without erasing transport/session generic parameters.
pub(crate) fn rpc_stream_protocol(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let policy = fields.take("policy")?;
    let (handlers, locals) = handler_bindings(&mut fields, "handlers")?;
    let output = protocol_output(handlers, locals, &policy, true, &path)?;
    fields.finish(output)
}
/// Build callback middleware or the distinct scoped service-producing middleware description.
pub(crate) fn rpc_middleware(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    if fields.contains("acquire") {
        let acquire = fields.transform("acquire", |expr, inline| {
            if !inline {
                return Ok(quote!(#path::rpc::RpcServiceMiddleware::new(#expr)));
            }
            let Expr::Block(block) = expr else {
                return Err(syn::Error::new_spanned(expr, "acquire body must be braced"));
            };
            let callback = crate::context_expand::reusable_callback(block.block.stmts);
            Ok(quote!(#path::rpc::RpcServiceMiddleware::from_body(#callback)))
        })?;
        return fields.finish(quote!(#acquire));
    }
    let handler = fields.take("handler")?;
    let codec = fields.take("error_codec")?;
    let before = fields.callback("before", 1)?;
    let after = fields.callback("after", 1)?;
    fields.finish(quote!(#path::rpc::RpcServerMiddleware::wrap(#handler,#codec,#before,#after)))
}
/// Preserve before/chunk/terminal sequencing with inline or reused typed streaming callbacks.
pub(crate) fn rpc_stream_middleware(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    if fields.contains("middleware") {
        let middleware = fields.take("middleware")?;
        let connect = fields.take("connect")?;
        let handler = fields.take("handler")?;
        let policy = fields.take("policy")?;
        let accepted = fields.take("accepted")?;
        return fields
            .finish(quote!((#middleware).serve_stream(#connect,#handler,#policy,#accepted)));
    }
    let handler = fields.take("handler")?;
    let codec = fields.take("error_codec")?;
    let before = fields.callback("before", 1)?;
    let chunk = fields.callback("chunk", 1)?;
    let terminal = fields.callback("terminal", 1)?;
    fields.finish(
        quote!(#path::rpc::RpcStreamMiddleware::wrap(#handler,#codec,#before,#chunk,#terminal)),
    )
}
/// Retain low-level event descriptors and add one explicit aggregate-selector authority.
pub(crate) fn event(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let group = fields.take("group")?;
    let tag = fields.take("tag")?;
    let version = fields.take("version")?;
    let payload = fields.take("payload")?;
    let success = fields.take("success")?;
    let error = fields.take("error")?;
    let payload_identity = fields.take("payload_identity")?;
    let success_identity = fields.take("success_identity")?;
    let error_identity = fields.take("error_identity")?;
    let limits = fields.take("limits")?;
    let definition = quote!(#path::eventlog::EventDefinition::new(#path::eventlog::EventShape{group:#group,tag:#tag,version:#version},#path::eventlog::EventCodecs{payload:#payload,success:#success,error:#error},#path::eventlog::EventBinaryFormats{payload:#payload_identity,success:#success_identity,error:#error_identity,limits:#limits}));
    let output = if fields.contains("aggregate") {
        let aggregate = fields.take("aggregate")?;
        quote!(#path::eventlog::AggregateEvent::new(#definition,#aggregate))
    } else {
        definition
    };
    fields.finish(output)
}
/// Couple an event's typed definition, aggregate authority, commit policy and Effect handler.
pub(crate) fn handled_event(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let definition = fields.take("definition")?;
    let commit = fields.take("commit")?;
    let contextual = fields
        .fields
        .iter()
        .find(|field| field.name == "handler")
        .is_some_and(
            |field| matches!(&field.value,Expr::Closure(closure) if closure.inputs.len()==2),
        );
    let handler = fields.callback("handler", if contextual { 2 } else { 1 })?;
    if contextual {
        let bound = if fields.contains("aggregate") {
            let aggregate = fields.take("aggregate")?;
            quote!(#path::eventlog::AggregateEvent::new(#definition,#aggregate))
        } else {
            quote!(#definition)
        };
        return fields
            .finish(quote!(#path::eventlog::HandledEvent::with_context(#bound,#commit,#handler)));
    }
    let output = if fields.contains("aggregate") {
        let aggregate = fields.take("aggregate")?;
        quote!(#path::eventlog::HandledEvent::new(#definition,#aggregate,#commit,#handler))
    } else {
        quote!(#path::eventlog::HandledEvent::from_bound(#definition,#commit,#handler))
    };
    fields.finish(output)
}
/// Checked grouping accepts ordinary and handled definitions through their typed definition views.
pub(crate) fn event_group(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let group = fields.take("group")?;
    let (events, locals) = handler_bindings(&mut fields, "events")?;
    let mut entries = locals.iter();
    let first = entries
        .next()
        .ok_or_else(|| syn::Error::new(Span::call_site(), "event group requires a definition"))?;
    fields.finish(
        quote!((|| -> ::core::result::Result<_,#path::eventlog::EventGroupError>{
        let (#(#locals,)*)=#events;
        let mut group=#path::eventlog::EventGroup::from_definition(#group,&#first)?;
        #(group.register_definition(&#entries)?;)* Ok(group)
    })()),
    )
}
/// Retain raw folds or derive checked acceptance and typed payload/outcome decoding from cases.
pub(crate) fn projection(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let key = fields.take("key")?;
    let version = fields.take("version")?;
    let initial = fields.take("initial")?;
    if fields.contains("cases") {
        let (cases, entries) = pair_bindings(&mut fields, "cases", false)?;
        if entries.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "typed projection requires a case",
            ));
        }
        let (definitions, folds): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
        return fields.finish(
            quote!((|| -> ::core::result::Result<_,#path::eventlog::ProjectionCasesError>{
            let (#((#definitions,#folds),)*)=#cases;
            let mut cases=#path::eventlog::ProjectionCases::new();
            #(cases.register(&#definitions,#folds)?;)* cases.finish(#key,#version,#initial)
        })()),
        );
    }
    let accepted = fields.take("accepted")?;
    let fold = fields.role("fold", 2, BodyCategory::Native)?;
    fields.finish_inferred(
        quote!(S, FoldError, Initial, Fold),
        &[("key", quote!(#path::eventlog::ProjectionKey)),
            ("version", quote!(#path::eventlog::ProjectionVersion)),
            ("accepted", quote!(::std::collections::BTreeMap<(#path::eventlog::EventGroupId, #path::eventlog::EventTag), #path::eventlog::EventVersion>)),
            ("initial", quote!(Initial)), ("fold", quote!(Fold))],
        quote!(#path::eventlog::Projection<S, FoldError>),
        quote!(S: Send + Sync + 'static, FoldError: Send + 'static,
            Initial: Fn() -> S + Send + Sync + 'static,
            Fold: Fn(&S, &#path::eventlog::JournalEntry) -> ::core::result::Result<S, FoldError> + Send + Sync + 'static),
        quote!(#path::eventlog::Projection::new(#key,#version,#accepted,#initial,#fold)),
    )
}
/// Construct derived atoms with exactly one tracked program or retained evaluator callback.
pub(crate) fn reactive_derived(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let registry = fields.take("registry")?;
    let id = fields.take("id")?;
    let capacity = fields.take("capacity")?;
    let overflow = fields.take("overflow")?;
    let program_mode = fields.contains("program");
    let evaluate = match (fields.contains("program"), fields.contains("evaluate")) {
        (true, false) => {
            let program = fields.take("program")?;
            quote!(move |context| (#program).evaluate(context))
        }
        (false, true) => fields.transform("evaluate", |expr, inline| {
            if inline {
                crate::descriptor_expand::effect_evaluator(expr, 1, &path)
            } else {
                Ok(quote!(#expr))
            }
        })?,
        _ => {
            return Err(syn::Error::new(
                Span::call_site(),
                "specify exactly one of program or evaluate",
            ));
        }
    };
    fields.finish_inferred(
        if program_mode { quote!(A, E, R) } else { quote!(A, E, R, F) },
        &[("registry", quote!(#path::reactivity::AtomRegistry)),
            ("id", quote!(#path::reactivity::AtomId)),
            ("capacity", quote!(::core::num::NonZeroUsize)),
            ("overflow", quote!(#path::PubSubOverflow)), ("evaluate", quote!(F)),
            ("program", quote!(#path::reactivity::AtomProgram<A, E, R>))],
        quote!(::core::result::Result<#path::reactivity::Atom<A, E, R>, #path::reactivity::AtomRegistryError>),
        if program_mode { quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static) }
        else { quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
            F: Fn(#path::reactivity::AtomReadContext) -> #path::Effect<A, E, R> + Send + Sync + 'static) },
        quote!(#path::reactivity::Atom::new(#registry,#id,#capacity,#overflow,#evaluate)),
    )
}

/// Retain runtime pair expressions in ordered typed tuple locals, lowering only inline handler bodies.
fn pair_bindings(
    fields: &mut Fields,
    name: &str,
    handler_bodies: bool,
) -> syn::Result<(TokenStream, Vec<(Ident, Ident)>)> {
    let mut locals = Vec::new();
    let value = fields.transform(name, |expr, _| {
        let entries = pairs(expr)?.values;
        let mut output = Vec::new();
        for (index, pair) in entries.into_iter().enumerate() {
            let left = pair.left;
            let right = if handler_bodies && matches!(pair.right, Expr::Closure(_)) {
                callback(pair.right, 2)?
            } else {
                let right = pair.right;
                quote!(#right)
            };
            locals.push((
                Ident::new(&format!("__{name}_left_{index}"), Span::mixed_site()),
                Ident::new(&format!("__{name}_right_{index}"), Span::mixed_site()),
            ));
            output.push(quote!((#left,#right)));
        }
        Ok(quote!((#(#output,)*)))
    })?;
    Ok((value, locals))
}

/// Build a resource-aware typed entity definition with activation-owned state.
pub(crate) fn entity(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    if fields.contains("dispatcher") {
        let policy = fields.take("dispatcher")?;
        let entity_type = fields.take("entity_type")?;
        let handler = fields.take("handler")?;
        let accepts = fields.take("accepts")?;
        return fields.finish_inferred(
            quote!(Q, A, E, R, F),
            &[("dispatcher", quote!(#path::rpc::RpcEnvelopePolicy)),
                ("entity_type", quote!(#path::cluster::ClusterEntityType)),
                ("handler", quote!(#path::rpc::RpcHandler<Q, A, E, R>)), ("accepts", quote!(F))],
            quote!(::core::result::Result<#path::cluster::EntityDispatcher<R>, #path::cluster::EntityRegistrationError>),
            quote!(Q: Send + 'static, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
                F: Fn(&#path::cluster::ClusterAddress, &Q) -> bool + Send + Sync + 'static),
            quote!(#path::cluster::EntityDispatcher::from_addressed_handler(#policy, #entity_type, #handler, #accepts)),
        );
    }
    let entity_type = fields.take("entity_type")?;
    let resources = fields.take("resources")?;
    let (handlers, entries) = pair_bindings(&mut fields, "handlers", true)?;
    if entries.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "entity definition requires a typed handler",
        ));
    }
    let (operations, callbacks): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
    let accepts = if fields.contains("accepts") {
        Some(fields.take("accepts")?)
    } else {
        None
    };
    let mut registrations = Vec::new();
    for (index, (operation, callback)) in operations.iter().zip(callbacks.iter()).enumerate() {
        let make = quote!({let state=::std::sync::Arc::clone(&state);let callback=::std::sync::Arc::clone(&#callback);#path::rpc::RpcHandler::new(#operation.clone(),move |request|callback(::std::sync::Arc::clone(&state),request))});
        let accepts=accepts.as_ref().map(|accepts|quote!({let accepts=::std::sync::Arc::clone(&#accepts);move |address,request|accepts(address,request)}));
        let construct = match (index == 0, accepts) {
            (true, Some(accepts)) => {
                quote!(let mut dispatcher=#path::cluster::EntityDispatcher::from_addressed_handler(policy,__entity_type.clone(),#make,#accepts)?;)
            }
            (true, None) => {
                quote!(let mut dispatcher=#path::cluster::EntityDispatcher::from_handler(policy,__entity_type.clone(),#make)?;)
            }
            (false, Some(accepts)) => {
                quote!(dispatcher.register_addressed(__entity_type.clone(),#make,#accepts)?;)
            }
            (false, None) => quote!(dispatcher.register(__entity_type.clone(),#make)?;),
        };
        registrations.push(construct);
    }
    let accepts_share =
        accepts.map(|accepts| quote!(let #accepts=::std::sync::Arc::new(#accepts);));
    fields.finish(quote!({
            let (#((#operations,#callbacks),)*)=#handlers;
            #(let #callbacks=::std::sync::Arc::new(#callbacks);)*
            #accepts_share
            let __entity_type=#entity_type.clone();
            #path::cluster::EntityDefinition::new(#entity_type,#resources,move |state,policy|{#(#registrations)*Ok(dispatcher)})
        }))
}

/// Aggregate typed resource-aware definitions without acquiring state.
pub(crate) fn cluster(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let policy = fields.take("policy")?;
    let (entities, locals) = handler_bindings(&mut fields, "entities")?;
    fields.finish(
        quote!((|| -> ::core::result::Result<_,#path::cluster::ClusterDefinitionError>{
        let (#(#locals,)*)=#entities;
        let mut cluster=#path::cluster::ClusterDefinition::new(#policy);
        #(cluster.register(#locals)?;)* cluster.finish()
    })()),
    )
}

/// Focused parser contracts retain field order and reject malformed domain syntax.
#[cfg(test)]
mod tests {
    use super::*;
    /// Duplicate fields report the duplicate rather than silently replacing the first.
    #[test]
    fn duplicate_and_inline_diagnostics() {
        assert!(syn::parse_str::<Fields>("name: first(), name: second()").is_err());
        assert!(syn::parse_str::<Fields>("policy {} ").is_err());
        assert!(syn::parse_str::<Fields>("body {}, effect: helper()").is_ok());
    }
    /// Runtime initializer order follows the source even when the expander consumes fields differently.
    #[test]
    fn source_order_survives_consumption() -> Result<(), Box<dyn std::error::Error>> {
        let mut fields = syn::parse_str::<Fields>("second: side_b(), first: side_a()")?;
        let first = fields.take("first")?;
        let second = fields.take("second")?;
        let output = fields.finish(quote!((#first,#second)))?.to_string();
        let second_position = output.find("side_b").ok_or("missing second initializer")?;
        let first_position = output.find("side_a").ok_or("missing first initializer")?;
        assert!(second_position < first_position);
        Ok(())
    }
    /// Pair-list grammar retains nested macro and guarded fold expressions without interpreting them.
    #[test]
    fn pair_lists_and_unknown_fields() -> syn::Result<()> {
        let mut fields = syn::parse_str::<Fields>(
            "cases: [event! {tag: tag} => |state, value, outcome| {match value {v if v>0=>Ok(state+v),_=>outcome}}], extra: value",
        )?;
        let pair_list = pairs(fields.raw("cases")?)?;
        assert_eq!(pair_list.values.len(), 1);
        assert!(fields.finish(quote!(())).is_err());
        Ok(())
    }
}

/// Shared role vocabulary. Owners reject unused/incompatible roles in `finish`.
/// Metadata remains colon-only; adding a spelling does not grant it a body category.
pub(crate) fn inline_role(name: &str) -> bool {
    matches!(
        name,
        "body"
            | "run"
            | "load"
            | "acquire"
            | "handler"
            | "before"
            | "after"
            | "chunk"
            | "terminal"
            | "map"
            | "project"
            | "defect"
            | "missing"
            | "value"
            | "on_error"
            | "on_timeout"
            | "recover"
            | "finalizer"
            | "items"
            | "produce"
            | "release"
            | "build"
            | "combine"
            | "key_factory"
            | "step"
            | "modify"
            | "update"
            | "create"
            | "validate"
            | "dispose"
            | "predicate"
            | "initial"
            | "iterable"
            | "async_iterable"
            | "async_iterable_effect"
            | "read"
            | "pull"
            | "map_effect"
            | "map_error"
            | "decode"
            | "encode"
            | "shape"
            | "lazy"
            | "env"
            | "sync"
            | "future"
            | "operation"
            | "select"
            | "fold"
            | "evaluate"
            | "connect"
            | "accepts"
    )
}
/// The owning operation selects the body language; braces alone confer no runtime capability.
#[derive(Clone, Copy)]
pub(crate) enum BodyCategory {
    /// Native value, Result or Future factory; all nested `?` and awaits remain Rust.
    Native,
    /// Native async Result body; explicit handle prefixes run before its future.
    NativeFuture,
    /// Effect callback using the existing binder; owned invocation inputs are one-shot.
    Effect,
    /// Borrowed callback inputs are inspected now; only owned prefix data enters cleanup/work.
    EffectFactory,
    /// An owned pure transition is executed lazily through the existing sync-once constructor.
    SyncEffect,
}
/// Validate shared braced-role grammar and preserve typed/destructuring patterns.
/// Native factories run only when their constructor invokes them; Effect callbacks
/// return inert descriptions and never run a nested application interpreter.
pub(crate) fn role(
    expression: Expr,
    arity: usize,
    category: BodyCategory,
) -> syn::Result<TokenStream> {
    if matches!(category, BodyCategory::Effect) && arity > 0 {
        return callback(expression, arity);
    }
    let (inputs, statements) = if let Expr::Block(block) = expression {
        if arity != 0 {
            return Err(syn::Error::new_spanned(
                block,
                format!("role requires {arity} parameters and a braced body"),
            ));
        }
        (
            syn::punctuated::Punctuated::<syn::Pat, Token![,]>::new(),
            block.block.stmts,
        )
    } else if let Expr::Closure(closure) = expression {
        if closure.inputs.len() != arity
            || closure.asyncness.is_some()
            || !matches!(closure.output, syn::ReturnType::Default)
        {
            return Err(syn::Error::new_spanned(
                closure,
                format!("role requires {arity} parameters and a braced body"),
            ));
        }
        let Expr::Block(block) = *closure.body else {
            return Err(syn::Error::new_spanned(
                closure.body,
                "role body must be braced",
            ));
        };
        (closure.inputs, block.block.stmts)
    } else {
        return Err(syn::Error::new_spanned(
            expression,
            format!("role requires {arity} parameters and a braced body"),
        ));
    };
    match category {
        BodyCategory::Native => Ok(quote!(move |#inputs| {#(#statements)*})),
        BodyCategory::NativeFuture => {
            let (mut prefix, mut statements) = crate::context_expand::split_prefix(statements);
            // Invocation inputs must be owned by the future before any references
            // into them are formed. Only preparation of external captures belongs
            // outside the future of a reusable callback.
            /// Bound invocation names whose storage must live inside the future.
            struct Inputs {
                /// Names collected from every destructuring pattern.
                names: std::collections::HashSet<String>,
            }
            impl<'ast> syn::visit::Visit<'ast> for Inputs {
                /// Record bindings recursively, including nested tuple patterns.
                fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
                    self.names.insert(pattern.ident.to_string());
                    syn::visit::visit_pat_ident(self, pattern);
                }
            }
            let mut parameters = Inputs {
                names: Default::default(),
            };
            for input in &inputs {
                syn::visit::Visit::visit_pat(&mut parameters, input);
            }
            /// Detect input dependencies and opaque macro expressions conservatively.
            struct References<'a> {
                /// Callback bindings that cannot be borrowed outside the future.
                names: &'a std::collections::HashSet<String>,
                /// Whether this statement must stay inside the future.
                found: bool,
            }
            impl<'ast> syn::visit::Visit<'ast> for References<'_> {
                /// Opaque macro tokens can contain references to invocation inputs.
                fn visit_macro(&mut self, _: &'ast syn::Macro) {
                    self.found = true;
                }
                /// Detect direct input dependencies while visiting native expressions.
                fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
                    if expression.qself.is_none()
                        && expression
                            .path
                            .get_ident()
                            .is_some_and(|name| self.names.contains(&name.to_string()))
                    {
                        self.found = true;
                    }
                    syn::visit::visit_expr_path(self, expression);
                }
            }
            let boundary = prefix.iter().position(|statement| {
                let mut references = References {
                    names: &parameters.names,
                    found: false,
                };
                syn::visit::Visit::visit_stmt(&mut references, statement);
                references.found
            });
            if let Some(boundary) = boundary {
                let mut invocation = prefix.split_off(boundary);
                invocation.append(&mut statements);
                statements = invocation;
            }
            Ok(quote!(move |#inputs| { #(#prefix)* async move { #(#statements)* } }))
        }
        BodyCategory::Effect => {
            let body = crate::expand::expand(statements)?;
            Ok(quote!(move || #body))
        }
        BodyCategory::EffectFactory => {
            let (prefix, statements) = crate::context_expand::split_prefix(statements);
            let body = crate::expand::expand_once(statements)?;
            Ok(quote!(move |#inputs| {#(#prefix)* #body}))
        }
        BodyCategory::SyncEffect => {
            let path = crate::expand::effect_crate_path()?;
            Ok(quote!(move |#inputs| #path::Effect::sync_once(move || {#(#statements)*})))
        }
    }
}
