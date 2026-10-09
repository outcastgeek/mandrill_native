//! Expansion of typed feature descriptors and registrations.

use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, Ident, Token};

/// Descriptor syntax retains source spans and explicitly requested body lowering.
struct Field {
    /// Field name carrying its original diagnostic span.
    name: Ident,
    /// Presence distinguishes typed callback reuse from inline lowering.
    _colon: Option<Token![:]>,
    /// Original typed expression, never executed during expansion.
    value: Expr,
    /// Colon-free syntax requests domain-body lowering.
    inline: bool,
}

impl Parse for Field {
    /// Parse one field at its original span, retaining colon-free inline lowering and opaque typed pair-list syntax.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        let colon = if input.peek(Token![:]) {
            Some(input.parse()?)
        } else if matches!(
            name.to_string().as_str(),
            "body"
                | "run"
                | "handler"
                | "load"
                | "acquire"
                | "before"
                | "after"
                | "chunk"
                | "terminal"
        ) {
            None
        } else {
            return Err(input.error("expected `:` after descriptor field"));
        };
        let value = if name == "cases" || name == "bindings" {
            let content;
            syn::bracketed!(content in input);
            let tokens: proc_macro2::TokenStream = content.parse()?;
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
        Ok(Self {
            name,
            inline: colon.is_none(),
            _colon: colon,
            value,
        })
    }
}

/// Retained descriptor fields with source ordering for richer domain construction.
pub(crate) struct NamedFields {
    /// Field lookup supports mandatory/conflicting-field validation without runtime work.
    values: BTreeMap<String, Expr>,
    /// Original field order and explicit lowering markers.
    order: Vec<(Ident, bool)>,
}

impl Parse for NamedFields {
    /// Reject duplicate names before any runtime evaluation, recording the original field order and inline-body markers.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let fields = Punctuated::<Field, Token![,]>::parse_terminated(input)?;
        let mut values = BTreeMap::new();
        let mut order = Vec::new();
        for field in fields {
            let name = field.name.to_string();
            order.push((field.name.clone(), field.inline));
            if values.insert(name, field.value).is_some() {
                return Err(syn::Error::new(
                    field.name.span(),
                    "duplicate descriptor field",
                ));
            }
        }
        Ok(Self { values, order })
    }
}

impl NamedFields {
    /// Preserve written order when a richer descriptor uses the shared domain parser.
    fn into_ordered(mut self) -> syn::Result<crate::domain_expand::Fields> {
        let mut tokens = proc_macro2::TokenStream::new();
        for (name, inline) in self.order {
            let value = self.values.remove(&name.to_string()).ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "descriptor field was already consumed",
                )
            })?;
            let colon = (!inline).then(|| quote!(:));
            tokens.extend(quote!(#name #colon #value,));
        }
        syn::parse2(tokens)
    }

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

/// Lower retained inline callback syntax into a fresh Effect per invocation, or pass a reusable typed function/expression.
/// Owned request captures use the one-shot path; provider selection remains in R.
pub(crate) fn effect_evaluator(
    expression: Expr,
    parameters: usize,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let Expr::Closure(closure) = expression else {
        return Ok(quote!(#expression));
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
        let (prefix, statements) = crate::context_expand::split_prefix(block.block.stmts);
        let body = crate::expand::expand_once(statements)?;
        let adapter = Ident::new(
            &format!("defer_callback{parameters}"),
            proc_macro2::Span::mixed_site(),
        );
        return Ok(quote!(#path::__private::#adapter(move |#inputs| {#(#prefix)* #body})));
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
    Ok(quote!(move |#inputs| #body))
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

/// Delegate tracked-program/evaluator construction through ordered fields, requiring exactly one evaluator authority.
pub(crate) fn reactive_derived(fields: NamedFields) -> syn::Result<TokenStream> {
    crate::domain_expand::reactive_derived(fields.into_ordered()?)
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

/// Retain descriptor-only declarations and add typed inline/reusable unary handlers while preserving runtime expression order.
pub(crate) fn rpc_operation(fields: NamedFields) -> syn::Result<TokenStream> {
    crate::domain_expand::rpc_operation(fields.into_ordered()?)
}

/// Keep chunk/terminal typing and optionally attach a request/context handler through ordered construction.
pub(crate) fn rpc_stream_operation(fields: NamedFields) -> syn::Result<TokenStream> {
    crate::domain_expand::rpc_stream_operation(fields.into_ordered()?)
}

/// Construct checked unary registrations or explicit unary/streaming grouping without erasing serving generics.
pub(crate) fn rpc_protocol(fields: NamedFields) -> syn::Result<TokenStream> {
    crate::domain_expand::rpc_protocol(fields.into_ordered()?)
}

/// Preserve the checked streaming registry and evaluate descriptor inputs once in their written order.
pub(crate) fn rpc_stream_protocol(fields: NamedFields) -> syn::Result<TokenStream> {
    crate::domain_expand::rpc_stream_protocol(fields.into_ordered()?)
}

/// Distinguish callback wrappers from actual scoped service acquisition; the latter retains resources through typed application.
pub(crate) fn rpc_middleware(fields: NamedFields) -> syn::Result<TokenStream> {
    crate::domain_expand::rpc_middleware(fields.into_ordered()?)
}

/// Lower requested streaming callbacks without changing chunk acknowledgement or terminal transformation ordering.
pub(crate) fn rpc_stream_middleware(fields: NamedFields) -> syn::Result<TokenStream> {
    crate::domain_expand::rpc_stream_middleware(fields.into_ordered()?)
}

/// Retain single registrations and add inert resource-aware definitions with typed live-state handlers.
pub(crate) fn entity(fields: NamedFields) -> syn::Result<TokenStream> {
    crate::domain_expand::entity(fields.into_ordered()?)
}

/// Retain codecs/binary identities and optionally bind the single typed aggregate-selector authority.
pub(crate) fn event(fields: NamedFields) -> syn::Result<TokenStream> {
    crate::domain_expand::event(fields.into_ordered()?)
}

/// Group typed definition views without erasing the original handled definitions or their behavior.
pub(crate) fn event_group(fields: NamedFields) -> syn::Result<TokenStream> {
    crate::domain_expand::event_group(fields.into_ordered()?)
}

/// Retain raw folds or build typed definition/case dispatch with one acceptance and decoding authority.
pub(crate) fn projection(fields: NamedFields) -> syn::Result<TokenStream> {
    crate::domain_expand::projection(fields.into_ordered()?)
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

/// Lower a workflow callback at the restricted durable boundary, preserving owned per-evaluation input and reusable helper expressions.
fn durable_evaluator(expression: Expr) -> syn::Result<TokenStream> {
    let Expr::Closure(closure) = expression else {
        return Ok(quote!(#expression));
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
        let (prefix, statements) = crate::context_expand::split_prefix(block.block.stmts);
        let body = crate::durable_expand::expand_once(statements)?;
        quote!({#(#prefix)* #body})
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
        let (prefix, statements) = crate::context_expand::split_prefix(statements);
        let body = crate::durable_expand::expand_once(statements)?;
        quote!({#(#prefix)* #body})
    } else {
        quote!(#expression)
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
