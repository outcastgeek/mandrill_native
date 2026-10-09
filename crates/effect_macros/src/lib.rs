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

/// Declare event identity, pure codecs and explicit binary formats.
/// Optional `aggregate` associates one typed selector with that definition; handled
/// execution uses the same authority and EventLog Effect boundary. Stored formats are unchanged.
#[proc_macro]
pub fn event(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::event(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct a checked group from ordinary, aggregate-bound or handled definitions.
/// Each typed view retains its original identity/codec authority, duplicate checks and
/// typed handler access; grouping installs no untyped callback dictionary.
#[proc_macro]
pub fn event_group(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::event_group(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct a raw pure-fold projection or checked `cases: [definition => fold, ...]`.
/// Cases derive accepted identities and typed payload/outcome decoding together. Decode,
/// version and fold failures remain distinct; `advance` is the existing Effect boundary
/// and checkpoints do not advance on failure. This adds no effectful-fold engine.
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

/// Declare a unary operation with explicit identity, version, codecs and limits.
/// Optional `handler |request| { ... }` composes nested Effects and returns RpcHandler;
/// `handler: callback` reuses a typed function. Descriptor-only forms remain supported.
/// Calling the handler returns an Effect under the original application environment.
#[proc_macro]
pub fn rpc_operation(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_operation(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Declare streaming request/chunk/terminal/error codecs with explicit limits.
/// Optional `handler |request, context| { ... }` returns the existing producer Effect;
/// typed callbacks reuse the same contract. No unary result or shared transport is inferred.
#[proc_macro]
pub fn rpc_stream_operation(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_stream_operation(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Build a checked unary registry from `policy` and `handlers`.
/// Grouped form uses separate `unary` and `streaming` handler lists under the explicit
/// envelope policy. Each registry retains its own typed handle/serve APIs and validation.
#[proc_macro]
pub fn rpc_protocol(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_protocol(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Build checked streaming registrations from explicit `policy` and `handlers`.
/// Handler construction and serving retain chunk/terminal, backend and session types;
/// an empty literal list is rejected during expansion and dynamic conflicts at construction.
#[proc_macro]
pub fn rpc_stream_protocol(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_stream_protocol(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Compose unary callbacks or acquire a typed service through `acquire { ... }`.
/// Service-producing middleware applies an explicit connector and handler under the
/// original R, retaining its acquisition resources through handler or complete stream serving
/// execution. `acquire: effect` reuses a typed acquisition. Callback forms retain their
/// explicit error codec and before/after ordering; inline callback syntax lowers Effects.
#[proc_macro]
pub fn rpc_middleware(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_middleware(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Wrap streaming handlers with explicit error codecs and before/chunk/terminal callbacks.
/// Colon-free callback bodies compose Effects; typed callbacks pass through unchanged.
/// Service-producing middleware from `rpc_middleware!` applies to full streaming serving
/// through its typed `serve_stream` boundary, retaining acknowledgement and cleanup.
#[proc_macro]
pub fn rpc_stream_middleware(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::rpc_stream_middleware(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Declare a checked single dispatcher registration, or an inert resource-aware entity.
/// Resource form supplies `entity_type`, `resources` and `handlers: [operation =>
/// |state, request| { ... }, ...]`, plus optional address acceptance. `cluster!` supplies
/// the registration policy; activation acquires into its owner scope and publishes only
/// a fenced dispatcher. State is shared within that activation and never copied into records.
#[proc_macro]
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

/// Construct a derived atom with exactly one `evaluate` callback or tracked `program`.
/// AtomProgram retains its active read context through helpers, branches and ordinary
/// Effect lifting. Each evaluation reconstructs observations; refresh/subscription stay
/// the existing Effect and scoped stream boundaries.
#[proc_macro]
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

/// Typed domain-body lowering shared by reactive and scoped resource programs.
mod context_expand;

/// Ordered descriptor parsing for new domain computation surfaces.
mod domain_expand;

/// Create a lazy named Effect span. `name` is required; use exactly one of `body { ... }`
/// or `effect: typed_effect`. Optional `annotations` surrounds creation and body execution,
/// preserving inherited metadata, nested parentage and the existing cleanup/outcome contract.
#[proc_macro]
pub fn span(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::span(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
/// Select once per Effect evaluation with `select` or an owned `handle`, then execute
/// `run |service| { ... }` under the original R. `on_error` maps typed ServiceOperation
/// and ServiceResource failures only. `project` with `body`/`effect` explicitly changes
/// the environment view using typed provision, retaining the outer runtime context.
#[proc_macro]
pub fn service(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::service(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
/// Create a cache through its existing scoped Effect boundary. Required `scope`, `config`
/// and `options` retain caller-chosen equality, TTL, admission and retention. An inline
/// `load |key| { ... }` composes Effects; `load: callback` reuses a typed loader. Lookup
/// and invalidation return existing Effects under the same environment capabilities.
#[proc_macro]
pub fn cache(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::cache(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
/// Create a scoped dependency cache Effect with explicit `scope`, `policy` and typed
/// `load` body/callback returning `(value, dependencies)`. The existing policy determines
/// dependency bounds, admission, TTL and bounded retention; no new retention mode is inferred.
#[proc_macro]
pub fn dependency_cache(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::dependency_cache(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
/// Build a reusable AtomProgram whose binds accept tracked helpers and ordinary Effects.
/// `AtomProgram::read`/`read_derived` carry the active context through nested helpers and
/// branches. `reactive_derived! { program: ... }` supplies a fresh evaluation context;
/// refreshing the resulting Atom is the ordinary Effect execution boundary.
#[proc_macro]
pub fn atom(input: TokenStream) -> TokenStream {
    let statements = parse_macro_input!(input with syn::Block::parse_within);
    let result = crate::expand::effect_crate_path().map(|path| {
        context_expand::expand(
            statements,
            quote::quote!(#path::reactivity::AtomProgram::from_body),
        )
    });
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}
/// Build checked heterogeneous hydration bindings with `registry`, `limits` and
/// `bindings: [source => schema, ...]`. Returns the existing construction Result;
/// ownership and duplicate validation remain authoritative. `export_all`/`import_all`
/// execute as Effects and retain the original encoded formats.
#[proc_macro]
pub fn hydration(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::hydration(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
/// Build a reusable EntityResourcePlan. Effects, ResourceBindings, fenced EntityResources
/// and nested plans bind under the activation's supplied owner and scope. Construction
/// acquires nothing; acquiring the plan returns an Effect and resources survive its
/// lexical body until the activation owner closes the scope. Explicit error mappings
/// must cover both acquisition and registered finalizers.
#[proc_macro]
pub fn entity_resource(input: TokenStream) -> TokenStream {
    let statements = parse_macro_input!(input with syn::Block::parse_within);
    let result = crate::expand::effect_crate_path().map(|path| {
        context_expand::expand(
            statements,
            quote::quote!(#path::cluster::EntityResourcePlan::from_body),
        )
    });
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}
/// Run a singleton through its existing lazy Effect boundary, with explicit singleton,
/// activation and lease_until inputs. Inline `run |owner| { ... }` accepts nested Effects;
/// `run: callback` reuses a typed body. Ownership, clock and activation failures remain
/// in SingletonError, and construction starts no renewal or scheduling worker.
#[proc_macro]
pub fn singleton(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::singleton(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
/// Describe the existing cron schedule with explicit `id`, `cron`, `transitions`,
/// `catch_up`, `concurrent` and `deadline_after`. Planning occurrences is separate from
/// Effect orchestration; this macro starts no background worker or interval scheduler.
#[proc_macro]
pub fn cluster_schedule(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::cluster_schedule(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
/// Define PersistedQueue with explicit `name`, pure `payload` codec, binary `identity`,
/// `limits` and message `policy`. The compiler infers/checks its identity domain. Offer,
/// claim and settlement use the existing typed Effects and the store selected by R.
#[proc_macro]
pub fn persisted_queue(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::persisted_queue(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
/// Wrap `queue` as DurableQueue; optional `environment: R` checks the existing physical
/// durability marker rather than installing a store. Worker form supplies `delivery`,
/// `deferred`, `acknowledged_at` and a handler body/callback, returning the existing
/// handle Effect with result-recording-before-acknowledgement and fencing preserved.
#[proc_macro]
pub fn durable_queue(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::durable_queue(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Stable pull-stream yield lowering over the existing typed Effect scope.
mod stream_expand;

/// Construct a lazy Stream using `yield value;`, Effect/resource binds, native branches,
/// guards and loops. `yield_from!(stream);` is a stream-local marker consumed here and
/// has no standalone export. Effects inside a yield run before emission; resource binds
/// remain owned across yields until the stream scope closes, including early consumption
/// failure. `open`, `pull` and consume operations remain typed Effects under the same R.
#[proc_macro]
pub fn stream(input: TokenStream) -> TokenStream {
    let statements = parse_macro_input!(input with syn::Block::parse_within);
    stream_expand::expand(statements)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Associate a definition, aggregate selector, explicit commit policy and Effect handler.
/// `handler |payload| { ... }` retains the domain Result inside Effect; contextual
/// `handler |context, payload| { ... }` exposes existing invocation metadata. Reusable
/// callbacks remain typed. Execution requires explicit journal/event/version/time/payload
/// inputs and delegates to EventLog, preserving journal, handler and expected-domain failures.
#[proc_macro]
pub fn handled_event(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::handled_event(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Build a checked catalog with explicit `policy` and `entities: [...]`. Resource-aware
/// entity definitions acquire only at activation; legacy entries use explicit
/// `(entity_type, dispatcher)` pairs. Activation returns an Effect using the supplied
/// owner/scope and the existing dispatcher, preserving typed acquisition/registration
/// failures and accepted-address checks. This creates no cluster runner or store.
#[proc_macro]
pub fn cluster(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::cluster(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
