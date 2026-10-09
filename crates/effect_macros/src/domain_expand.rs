//! Ordered field parsing and construction for the cross-cutting domain DSLs.

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Token};

/// A field retains its source position and distinguishes lowering from typed callback reuse.
struct Field {
    /// User-visible field name and diagnostic span.
    name: Ident,
    /// Original expression; pair lists retain their own grammar until the owning expander consumes them.
    value: Expr,
    /// Colon-free body syntax opts into lowering instead of passing an already typed expression.
    inline: bool,
    /// Lowered runtime initializer, emitted in source order after validation.
    binding: Option<TokenStream>,
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
            if inline
                && !matches!(
                    name.to_string().as_str(),
                    "body"
                        | "run"
                        | "load"
                        | "acquire"
                        | "handler"
                        | "before"
                        | "after"
                        | "chunk"
                        | "terminal"
                )
            {
                return Err(input.error("expected `:` after descriptor field"));
            }
            let value = if name == "bindings" || name == "cases" {
                let content;
                syn::bracketed!(content in input);
                let pairs = content.parse::<Pairs>()?;
                let tokens = pairs.tokens();
                Expr::Verbatim(quote!([#tokens]))
            } else if name == "handlers" && input.peek(syn::token::Bracket) {
                let content;
                syn::bracketed!(content in input);
                let tokens: TokenStream = content.parse()?;
                syn::parse2::<Expr>(quote!([#tokens]))
                    .unwrap_or_else(|_| Expr::Verbatim(quote!([#tokens])))
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
        field.binding = Some(quote!(let #local = #value;));
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
    /// Validate unconsumed fields and emit initializer bindings in the original source order.
    pub(crate) fn finish(self, output: TokenStream) -> syn::Result<TokenStream> {
        if let Some(field) = self.fields.iter().find(|field| !field.taken) {
            return Err(syn::Error::new(
                field.name.span(),
                format!("unknown or conflicting descriptor field: {}", field.name),
            ));
        }
        let bindings = self.fields.into_iter().filter_map(|field| field.binding);
        Ok(quote!({ #(#bindings)* #output }))
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
            syn::bracketed!(content in input);
            Ok(Self(content.parse()?))
        }
    }
    syn::parse2::<Bracketed>(quote!(#expression)).map(|value| value.0)
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
    let name = fields.take("name")?;
    let body = effect_body(&mut fields)?;
    let output = quote!((#body).with_span(#name));
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
    if fields.contains("project") {
        let project = fields.take("project")?;
        let body = effect_body(&mut fields)?;
        return fields.finish(quote!((#body).provide_some(#project)));
    }
    let selected = match (fields.contains("select"), fields.contains("handle")) {
        (true, false) => {
            let select = fields.take("select")?;
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
        fields.take("on_error")?
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
            syn::parse_quote!(#[allow(unused_variables)] let #pattern=#context.service();),
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
    let inline = fields
        .fields
        .iter()
        .any(|field| field.name == "load" && field.inline);
    let load = fields.callback("load", 1)?;
    let method = if inline {
        quote!(make_with_shared_key)
    } else {
        quote!(make_with_options)
    };
    fields.finish(quote!(#path::Cache::#method(#scope,#config,#options,#load)))
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
    fields.finish(quote!(#path::DependencyCache::#method(#scope,#policy,#load)))
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
    fields.finish(output)
}
/// Declare streaming codecs and an optional request/context handler returning its typed producer Effect.
pub(crate) fn rpc_stream_operation(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
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
    let fold = fields.take("fold")?;
    fields.finish(quote!(#path::eventlog::Projection::new(#key,#version,#accepted,#initial,#fold)))
}
/// Construct derived atoms with exactly one tracked program or retained evaluator callback.
pub(crate) fn reactive_derived(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let registry = fields.take("registry")?;
    let id = fields.take("id")?;
    let capacity = fields.take("capacity")?;
    let overflow = fields.take("overflow")?;
    let evaluate = match (fields.contains("program"), fields.contains("evaluate")) {
        (true, false) => {
            let program = fields.take("program")?;
            quote!(move |context| (#program).evaluate(context))
        }
        (false, true) => fields.transform("evaluate", |expr, _| {
            crate::descriptor_expand::effect_evaluator(expr, 1, &path)
        })?,
        _ => {
            return Err(syn::Error::new(
                Span::call_site(),
                "specify exactly one of program or evaluate",
            ));
        }
    };
    fields.finish(quote!(#path::reactivity::Atom::new(#registry,#id,#capacity,#overflow,#evaluate)))
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

/// Build legacy registrations or a resource-aware typed entity definition with live-state handlers.
pub(crate) fn entity(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let entity_type = fields.take("entity_type")?;
    if fields.contains("resources") {
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
        return fields.finish(quote!({
            let (#((#operations,#callbacks),)*)=#handlers;
            #(let #callbacks=::std::sync::Arc::new(#callbacks);)*
            #accepts_share
            let __entity_type=#entity_type.clone();
            #path::cluster::EntityDefinition::new(#entity_type,#resources,move |state,policy|{#(#registrations)*Ok(dispatcher)})
        }));
    }
    let handler = fields.take("handler")?;
    let accepts = if fields.contains("accepts") {
        Some(fields.take("accepts")?)
    } else {
        None
    };
    let output = match (
        fields.contains("dispatcher"),
        fields.contains("policy"),
        accepts,
    ) {
        (true, false, Some(accepts)) => {
            let dispatcher = fields.take("dispatcher")?;
            quote!((#dispatcher).register_addressed(#entity_type,#handler,#accepts))
        }
        (true, false, None) => {
            let dispatcher = fields.take("dispatcher")?;
            quote!((#dispatcher).register(#entity_type,#handler))
        }
        (false, true, Some(accepts)) => {
            let policy = fields.take("policy")?;
            quote!(#path::cluster::EntityDispatcher::from_addressed_handler(#policy,#entity_type,#handler,#accepts))
        }
        (false, true, None) => {
            let policy = fields.take("policy")?;
            quote!(#path::cluster::EntityDispatcher::from_handler(#policy,#entity_type,#handler))
        }
        _ => {
            return Err(syn::Error::new(
                Span::call_site(),
                "legacy entity requires exactly one of policy or dispatcher",
            ));
        }
    };
    fields.finish(output)
}
/// Aggregate resource-aware definitions and explicit legacy registrations without acquiring state.
pub(crate) fn cluster(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let policy = fields.take("policy")?;
    let (entities, locals) = handler_bindings(&mut fields, "entities")?;
    fields.finish(
        quote!((|| -> ::core::result::Result<_,#path::cluster::ClusterDefinitionError>{
        let (#(#locals,)*)=#entities;
        let mut cluster=#path::cluster::ClusterDefinition::new(#policy);
        #(#path::cluster::ClusterEntry::register_in(#locals,&mut cluster)?;)* cluster.finish()
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
    fn source_order_survives_consumption() {
        let mut fields =
            syn::parse_str::<Fields>("second: side_b(), first: side_a()").expect("fields");
        let first = fields.take("first").expect("first");
        let second = fields.take("second").expect("second");
        let output = fields
            .finish(quote!((#first,#second)))
            .expect("complete")
            .to_string();
        assert!(output.find("side_b").expect("b") < output.find("side_a").expect("a"));
    }
    /// Pair-list grammar retains nested macro and guarded fold expressions without interpreting them.
    #[test]
    fn pair_lists_and_unknown_fields() {
        let mut fields=syn::parse_str::<Fields>("cases: [event! {tag: tag} => |state, value, outcome| {match value {v if v>0=>Ok(state+v),_=>outcome}}], extra: value").expect("pairs");
        let pair_list = pairs(fields.raw("cases").expect("cases")).expect("pair grammar");
        assert_eq!(pair_list.values.len(), 1);
        assert!(fields.finish(quote!(())).is_err());
    }
}
