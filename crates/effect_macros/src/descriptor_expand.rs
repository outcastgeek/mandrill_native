//! Expansion of typed feature descriptors and registrations.

use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, Ident, Token};

/// Typed field used by descriptor expand.
struct Field {
    name: Ident,
    _colon: Option<Token![:]>,
    value: Expr,
}

impl Parse for Field {
    ///
    /// # Parameters
    /// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
    ///
    /// # Returns
    /// Type: `syn::Result<Self>`.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        let colon = if input.peek(Token![:]) {
            Some(input.parse()?)
        } else if name == "body" || name == "run" {
            None
        } else {
            return Err(input.error("expected `:` after descriptor field"));
        };
        Ok(Self {
            name,
            _colon: colon,
            value: input.parse()?,
        })
    }
}

/// Typed named fields used by descriptor expand.
pub(crate) struct NamedFields {
    values: BTreeMap<String, Expr>,
}

impl Parse for NamedFields {
    ///
    /// # Parameters
    /// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
    ///
    /// # Returns
    /// Type: `syn::Result<Self>`.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let fields = Punctuated::<Field, Token![,]>::parse_terminated(input)?;
        let mut values = BTreeMap::new();
        for field in fields {
            let name = field.name.to_string();
            if values.insert(name, field.value).is_some() {
                return Err(syn::Error::new(
                    field.name.span(),
                    "duplicate descriptor field",
                ));
            }
        }
        Ok(Self { values })
    }
}

impl NamedFields {
    /// # Parameters
    /// - `name` (`&str`): Stable name used to register or locate this definition.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    ///
    /// # Parameters
    /// - `name` (`&str`): Stable name used to register or locate this definition.
    ///
    /// # Returns
    /// Type: `syn::Result<Expr>`.
    fn take(&mut self, name: &str) -> syn::Result<Expr> {
        self.values.remove(name).ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("missing descriptor field: {name}"),
            )
        })
    }

    ///
    /// # Parameters
    /// - `first` (`&str`): First value or branch in the ordered composition.
    /// - `second` (`&str`): The second supplied to this operation.
    ///
    /// # Returns
    /// Type: `syn::Result<Expr>`.
    fn take_one_of(&mut self, first: &str, second: &str) -> syn::Result<Expr> {
        match (self.values.remove(first), self.values.remove(second)) {
            (Some(value), None) | (None, Some(value)) => Ok(value),
            (Some(_), Some(_)) => Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("specify either `{first}` or `{second}`, not both"),
            )),
            (None, None) => Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("missing descriptor field: `{first}` or `{second}`"),
            )),
        }
    }

    ///
    /// # Returns
    /// Type: `syn::Result<()>`.
    fn finish(self) -> syn::Result<()> {
        if let Some((name, _)) = self.values.into_iter().next() {
            Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("unknown descriptor field: {name}"),
            ))
        } else {
            Ok(())
        }
    }

    ///
    /// # Parameters
    /// - `name` (`&str`): Stable name used to register or locate this definition.
    ///
    /// # Returns
    /// Type: `Option<Expr>`.
    /// Returns the value when present, or `None`.
    fn optional(&mut self, name: &str) -> Option<Expr> {
        self.values.remove(name)
    }
}

///
/// # Parameters
/// - `expression` (`Expr`): The expression supplied to this operation.
/// - `parameters` (`usize`): The parameters supplied to this operation.
/// - `path` (`&TokenStream`): Location of the field, file, or configuration value being processed.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
fn effect_evaluator(
    expression: Expr,
    parameters: usize,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let Expr::Closure(closure) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "expected a closure with an Effect body",
        ));
    };
    if closure.asyncness.is_some()
        || closure.inputs.len() != parameters
        || !matches!(&closure.output, syn::ReturnType::Default)
    {
        return Err(syn::Error::new_spanned(
            closure,
            "expected a synchronous closure with the required parameters",
        ));
    }
    let inputs = closure.inputs;
    let body = *closure.body;
    if let Expr::Block(block) = body {
        let statements = block.block.stmts;
        return Ok(quote!(move |#inputs| #path::effect! { #(#statements)* }));
    }
    if let Expr::Macro(ref invocation) = body
        && invocation
            .mac
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "effect")
    {
        let tokens = &invocation.mac.tokens;
        return Ok(quote!(move |#inputs| #path::effect! { #tokens }));
    }
    Err(syn::Error::new_spanned(
        body,
        "expected a braced body or effect! block",
    ))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn reactive_source(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let registry = fields.take("registry")?;
    let id = fields.take("id")?;
    let initial = fields.take("initial")?;
    let capacity = fields.take("capacity")?;
    let overflow = fields.take("overflow")?;
    fields.finish()?;
    Ok(quote!(#path::reactivity::AtomRef::new(
        #registry, #id, #initial, #capacity, #overflow,
    )))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn reactive_derived(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let registry = fields.take("registry")?;
    let id = fields.take("id")?;
    let capacity = fields.take("capacity")?;
    let overflow = fields.take("overflow")?;
    let evaluate = effect_evaluator(fields.take("evaluate")?, 1, &path)?;
    fields.finish()?;
    Ok(quote!(#path::reactivity::Atom::new(
        #registry, #id, #capacity, #overflow, #evaluate,
    )))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn reactive_query(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let registry = fields.take("registry")?;
    let dependencies = fields.take("dependencies")?;
    let evaluate = effect_evaluator(fields.take("evaluate")?, 0, &path)?;
    fields.finish()?;
    Ok(quote!(#path::reactivity::ReactiveQuery::new(
        #registry, #dependencies, #evaluate,
    )))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn reactive_bridge(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let read = effect_evaluator(fields.take("read")?, 0, &path)?;
    let changes = fields.take("changes")?;
    fields.finish()?;
    Ok(quote!(#path::reactivity::EffectStreamSource::new(
        #read, #changes,
    )))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn rpc_operation(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let id = fields.take("id")?;
    let version = fields.take("version")?;
    let request = fields.take("request")?;
    let success = fields.take("success")?;
    let error = fields.take("error")?;
    let limits = fields.take("limits")?;
    let idempotency = fields.optional("idempotency_key");
    fields.finish()?;
    let operation = quote!(#path::rpc::RpcOperation::unary(
        #id, #version, #request, #success, #error, #limits,
    ));
    Ok(match idempotency {
        Some(key) => quote!((#operation).with_idempotency_key(#key)),
        None => operation,
    })
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn rpc_stream_operation(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let id = fields.take("id")?;
    let version = fields.take("version")?;
    let request = fields.take("request")?;
    let chunk = fields.take("chunk")?;
    let terminal = fields.take("terminal")?;
    let error = fields.take("error")?;
    let limits = fields.take("limits")?;
    fields.finish()?;
    Ok(quote!(#path::rpc::RpcStreamOperation::new(
        #id, #version, #request, #chunk, #terminal, #error, #limits,
    )))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn rpc_protocol(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let policy = fields.take("policy")?;
    let handlers = fields.take("handlers")?;
    fields.finish()?;
    let Expr::Array(handlers) = handlers else {
        return Err(syn::Error::new_spanned(
            handlers,
            "handlers must be a bracketed list of typed handlers",
        ));
    };
    if handlers.elems.is_empty() {
        return Err(syn::Error::new_spanned(
            handlers,
            "protocol requires at least one typed handler",
        ));
    }
    let mut handlers = handlers.elems.into_iter().collect::<Vec<_>>();
    let first = handlers.remove(0);
    Ok(
        quote!((|| -> ::core::result::Result<_, #path::rpc::RpcRegistryError> {
        let mut protocol = #path::rpc::RpcProtocol::from_handler(#policy, #first)?;
        #(protocol.register(#handlers)?;)*
        ::core::result::Result::Ok(protocol)
    })()),
    )
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn rpc_stream_protocol(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let policy = fields.take("policy")?;
    let handlers = fields.take("handlers")?;
    fields.finish()?;
    let Expr::Array(handlers) = handlers else {
        return Err(syn::Error::new_spanned(
            handlers,
            "handlers must be a bracketed list of typed streaming handlers",
        ));
    };
    if handlers.elems.is_empty() {
        return Err(syn::Error::new_spanned(
            handlers,
            "stream protocol requires at least one typed handler",
        ));
    }
    let mut handlers = handlers.elems.into_iter().collect::<Vec<_>>();
    let first = handlers.remove(0);
    Ok(
        quote!((|| -> ::core::result::Result<_, #path::rpc::RpcRegistryError> {
        let mut protocol = #path::rpc::RpcStreamProtocol::from_handler(#policy, #first)?;
        #(protocol.register(#handlers)?;)*
        ::core::result::Result::Ok(protocol)
    })()),
    )
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn rpc_middleware(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let handler = fields.take("handler")?;
    let error_codec = fields.take("error_codec")?;
    let before = fields.take("before")?;
    let after = fields.take("after")?;
    fields.finish()?;
    Ok(quote!(#path::rpc::RpcServerMiddleware::wrap(
        #handler, #error_codec, #before, #after,
    )))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn rpc_stream_middleware(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let handler = fields.take("handler")?;
    let error_codec = fields.take("error_codec")?;
    let before = fields.take("before")?;
    let chunk = fields.take("chunk")?;
    let terminal = fields.take("terminal")?;
    fields.finish()?;
    Ok(quote!(#path::rpc::RpcStreamMiddleware::wrap(
        #handler, #error_codec, #before, #chunk, #terminal,
    )))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn entity(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let dispatcher = fields.optional("dispatcher");
    let policy = fields.optional("policy");
    let entity_type = fields.take("entity_type")?;
    let handler = fields.take("handler")?;
    let accepts = fields.optional("accepts");
    fields.finish()?;
    Ok(match (dispatcher, policy, accepts) {
        (Some(dispatcher), None, Some(accepts)) => quote!((#dispatcher).register_addressed(
            #entity_type, #handler, #accepts,
        )),
        (Some(dispatcher), None, None) => {
            quote!((#dispatcher).register(#entity_type, #handler))
        }
        (None, Some(policy), Some(accepts)) => quote!(
            #path::cluster::EntityDispatcher::from_addressed_handler(
                #policy, #entity_type, #handler, #accepts,
            )
        ),
        (None, Some(policy), None) => quote!(
            #path::cluster::EntityDispatcher::from_handler(
                #policy, #entity_type, #handler,
            )
        ),
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "entity requires exactly one of policy or dispatcher",
            ));
        }
    })
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn event(mut fields: NamedFields) -> syn::Result<TokenStream> {
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
    fields.finish()?;
    Ok(quote!(#path::eventlog::EventDefinition::new(
        #path::eventlog::EventShape { group: #group, tag: #tag, version: #version },
        #path::eventlog::EventCodecs {
            payload: #payload, success: #success, error: #error,
        },
        #path::eventlog::EventBinaryFormats {
            payload: #payload_identity,
            success: #success_identity,
            error: #error_identity,
            limits: #limits,
        },
    )))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn event_group(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let group = fields.take("group")?;
    let events = fields.take("events")?;
    fields.finish()?;
    let Expr::Array(events) = events else {
        return Err(syn::Error::new_spanned(
            events,
            "events must be a bracketed list of typed event definitions",
        ));
    };
    let mut events = events.elems.into_iter();
    let Some(first) = events.next() else {
        return Err(syn::Error::new_spanned(
            group,
            "event group requires at least one typed event",
        ));
    };
    Ok(
        quote!((|| -> ::core::result::Result<_, #path::eventlog::EventGroupError> {
        let mut group = #path::eventlog::EventGroup::from_event(#group, &#first)?;
        #(group.register(&#events)?;)*
        ::core::result::Result::<_, #path::eventlog::EventGroupError>::Ok(group)
    })()),
    )
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn projection(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let key = fields.take("key")?;
    let version = fields.take("version")?;
    let accepted = fields.take("accepted")?;
    let initial = fields.take("initial")?;
    let fold = fields.take("fold")?;
    fields.finish()?;
    Ok(quote!(#path::eventlog::Projection::new(
        #key, #version, #accepted, #initial, #fold,
    )))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn workflow_definition(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let id = fields.take("id")?;
    let version = fields.take("version")?;
    let input = fields.take("input")?;
    let success = fields.take("success")?;
    let error = fields.take("error")?;
    let compatibility = fields.take("compatibility")?;
    let limits = fields.take("limits")?;
    fields.finish()?;
    Ok(quote!(#path::workflow::WorkflowDefinition {
        id: #id, version: #version, input: #input, success: #success,
        error: #error, compatibility: #compatibility, limits: #limits,
    }))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn workflow(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let id = fields.take_one_of("id", "name")?;
    let version = fields.take("version")?;
    let input = fields.take_one_of("input", "payload")?;
    let success = fields.take("success")?;
    let error = fields.take("error")?;
    let compatibility = fields.take("compatibility")?;
    let limits = fields.take("limits")?;
    let run = durable_evaluator(fields.take("run")?)?;
    fields.finish()?;
    Ok(quote!(#path::workflow::TypedWorkflowDefinition::new(
        #path::workflow::WorkflowDefinition {
            id: #id, version: #version, input: #input, success: #success,
            error: #error, compatibility: #compatibility, limits: #limits,
        },
        #run,
    )))
}

///
/// # Parameters
/// - `expression` (`Expr`): The expression supplied to this operation.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
fn durable_evaluator(expression: Expr) -> syn::Result<TokenStream> {
    let Expr::Closure(closure) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "expected a one-argument closure with a durable workflow body",
        ));
    };
    if closure.asyncness.is_some()
        || closure.inputs.len() != 1
        || !matches!(&closure.output, syn::ReturnType::Default)
    {
        return Err(syn::Error::new_spanned(
            closure,
            "expected a synchronous one-argument workflow closure",
        ));
    }
    let inputs = closure.inputs;
    let expression = *closure.body;
    let body = if let Expr::Block(block) = expression {
        crate::durable_expand::expand(block.block.stmts)?
    } else if let Expr::Macro(ref invocation) = expression
        && invocation
            .mac
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "workflow")
    {
        let statements =
            syn::parse::Parser::parse2(syn::Block::parse_within, invocation.mac.tokens.clone())?;
        crate::durable_expand::expand(statements)?
    } else {
        return Err(syn::Error::new_spanned(
            expression,
            "expected a braced body or workflow! block",
        ));
    };
    Ok(quote!(move |#inputs| #body))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn activity_definition(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let id = fields.take("id")?;
    let success = fields.take("success")?;
    let error = fields.take("error")?;
    let limits = fields.take("limits")?;
    fields.finish()?;
    Ok(quote!(#path::workflow::WorkflowActivity {
        id: #id, success: #success, error: #error, limits: #limits,
    }))
}

/// # Parameters
/// - `fields` (`NamedFields`): Typed fields composing this record or schema.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn activity(mut fields: NamedFields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    if let Some(description) = fields.optional("description") {
        let run = fields.take("run")?;
        let attempt = fields.take("attempt")?;
        let body = effect_evaluator(fields.take("body")?, 0, &path)?;
        fields.finish()?;
        return Ok(quote!(#path::workflow::durable_activity(
            #description, #run, #attempt, #body,
        )));
    }
    let id = fields.take("name")?;
    let input = fields.take("input")?;
    let success = fields.take("success")?;
    let error = fields.take("error")?;
    let limits = fields.take("limits")?;
    let body = effect_evaluator(fields.take("body")?, 1, &path)?;
    fields.finish()?;
    Ok(quote!(#path::workflow::TypedWorkflowActivity::new(
        #path::workflow::WorkflowActivity {
            id: #id, success: #success, error: #error, limits: #limits,
        },
        #input, #body,
    )))
}
