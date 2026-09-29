//! Procedural Rust block syntax for the executor-independent Effect kernel.
mod async_expand;
mod descriptor_expand;
mod durable_expand;
mod expand;
mod parse;
mod persistable_expand;
mod schema_enum_expand;
mod schema_expand;
mod transaction_expand;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Construct a lazy reusable Effect with macro-local `?` binding.
#[proc_macro]
pub fn effect(input: TokenStream) -> TokenStream {
    let statements = parse_macro_input!(input with syn::Block::parse_within);
    match expand::expand(statements) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Derive checked JSON record or tagged-enum codecs from typed field codecs
/// and an explicit parse policy.
#[proc_macro]
pub fn schema(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as schema_expand::SchemaInput);
    match schema_expand::expand(definition) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Build a tagged enum codec with explicit wire tags and envelope fields.
#[proc_macro]
pub fn schema_enum(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as schema_enum_expand::SchemaEnumInput);
    schema_enum_expand::expand(definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Pair stable request identity, success/error codecs, and record policy.
#[proc_macro]
pub fn persistable(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as persistable_expand::PersistableInput);
    match persistable_expand::expand(definition) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Compose pure transactional reads, writes, failures, retry and `for` loops
/// with `?`. An effectful loop reevaluates its iterable for each conflict
/// retry; clone an owned source explicitly only when its ownership requires it.
/// A bound step must be a Transaction; Effect and Future values do not typecheck.
#[proc_macro]
pub fn transaction(input: TokenStream) -> TokenStream {
    let statements = parse_macro_input!(input with syn::Block::parse_within);
    match transaction_expand::expand(statements) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[proc_macro]
pub fn event(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::event(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn event_group(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::event_group(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn projection(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::projection(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Compose durable operations, or define a typed workflow with `id`, codecs,
/// compatibility, limits and a durable `run` closure.
#[proc_macro]
pub fn workflow(input: TokenStream) -> TokenStream {
    let tokens: proc_macro2::TokenStream = input.into();
    let result = match syn::parse2::<descriptor_expand::NamedFields>(tokens.clone()) {
        Ok(fields) if fields.contains("id") || fields.contains("name") => {
            descriptor_expand::workflow(fields)
        }
        _ => syn::parse::Parser::parse2(syn::Block::parse_within, tokens)
            .and_then(durable_expand::expand),
    };
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Pair a named activity's input/result codecs and one-argument Effect body,
/// or run an existing description with a zero-argument Effect body. Ordinary
/// Effect operations are confined to that body, not workflow coordination.
#[proc_macro]
pub fn activity(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::activity(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn workflow_definition(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::workflow_definition(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn activity_definition(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::activity_definition(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn rpc_operation(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_operation(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn rpc_stream_operation(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_stream_operation(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn rpc_protocol(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_protocol(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn rpc_stream_protocol(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_stream_protocol(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn rpc_middleware(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_middleware(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn rpc_stream_middleware(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_stream_middleware(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn entity(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::entity(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct a typed writable source atom with an explicit publication policy.
#[proc_macro]
pub fn reactive_source(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::reactive_source(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct a derived atom; its evaluator body is lowered through `effect!`.
#[proc_macro]
pub fn reactive_derived(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::reactive_derived(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct a query whose typed dependency set drives scoped invalidation.
#[proc_macro]
pub fn reactive_query(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::reactive_query(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Pair an Effect snapshot reader with a typed Stream change source.
#[proc_macro]
pub fn reactive_bridge(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::reactive_bridge(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
