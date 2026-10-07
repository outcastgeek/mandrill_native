//! Procedural Rust block syntax for the executor-independent Effect kernel.
//!
//! Applications normally import these macros through `effect`, which re-exports
//! them beside the types they construct. Macro invocations are checked and
//! expanded at compile time. `effect!` binds Effect steps with Rust's `?` and
//! preserves ordinary Rust control flow; the schema, RPC, workflow, event,
//! reactive, and transactional macros construct typed definitions in their
//! respective domains. Expansion does not create a runtime or choose a
//! provider. Consult the `effect` crate's `guide` and feature modules for the
//! resulting contracts and examples.
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
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn effect(input: TokenStream) -> TokenStream {
    let statements = parse_macro_input!(input with syn::Block::parse_within);
    match expand::expand(statements) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Build typed pure JSON records/enums or owned native effectful records with explicit policies.
///
/// A record form supplies `struct Type { field: codec, ... }` and `policy:`.
/// Native records additionally supply both `representation:` and `unknown_fields:`;
/// all field codecs share Encoded, DecodeEnv and EncodeEnv. Generated constructors
/// move field values; generated destructuring moves the application value without
/// requiring Clone.
///
/// Native field attributes support `#[schema(optional)]` for Option-valued fields
/// and `#[schema(default_with = factory)]` for an absent-field effect factory. They
/// are mutually exclusive. A default uses an independent borrowed contextual
/// validator; present null goes through the field decoder. Pure record fields keep
/// the existing explicit optional/default form. Missing policies, mixed native
/// representations or incompatible environments are compile-time errors.
///
/// Expansion preserves dependency aliases and never creates a runtime or selects
/// providers. See the re-exporting effect crate's native record example and
/// NativeRecordBuilder contract for construction and error behavior.
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
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn schema_enum(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as schema_enum_expand::SchemaEnumInput);
    schema_enum_expand::expand(definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Pair stable request identity, success/error codecs, and record policy.
#[proc_macro]
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
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
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn transaction(input: TokenStream) -> TokenStream {
    let statements = parse_macro_input!(input with syn::Block::parse_within);
    match transaction_expand::expand(statements) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[proc_macro]
/// Expand a typed event declaration into its identity and codec definition.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn event(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::event(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
/// Expand a stable event-group declaration into checked Rust items.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn event_group(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::event_group(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
/// Expand a typed event projection definition and its fold contract.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn projection(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::projection(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Compose durable operations, or define a typed workflow with `id`, codecs,
/// compatibility, limits and a durable `run` closure.
#[proc_macro]
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
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
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn activity(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::activity(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
/// Expand a workflow identity, version, and codec definition.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn workflow_definition(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::workflow_definition(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
/// Expand a named activity identity and typed codec definition.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn activity_definition(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::activity_definition(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
/// Expand a unary RPC operation with typed request and reply codecs.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn rpc_operation(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_operation(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
/// Expand a streaming RPC operation with typed item and terminal codecs.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn rpc_stream_operation(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_stream_operation(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
/// Expand a protocol with its first required typed handler.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn rpc_protocol(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_protocol(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
/// Expand a streaming protocol with its first required typed handler.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn rpc_stream_protocol(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_stream_protocol(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
/// Expand a typed wrapper around unary RPC client or server handling.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn rpc_middleware(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_middleware(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
/// Expand a typed wrapper around streaming RPC handling.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn rpc_stream_middleware(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_stream_middleware(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
/// Expand an addressed entity dispatcher with checked typed handlers.
///
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn entity(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::entity(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct a typed writable source atom with an explicit publication policy.
#[proc_macro]
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn reactive_source(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::reactive_source(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct a derived atom; its evaluator body is lowered through `effect!`.
#[proc_macro]
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn reactive_derived(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::reactive_derived(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct a query whose typed dependency set drives scoped invalidation.
#[proc_macro]
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn reactive_query(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::reactive_query(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Pair an Effect snapshot reader with a typed Stream change source.
#[proc_macro]
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn reactive_bridge(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::reactive_bridge(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
