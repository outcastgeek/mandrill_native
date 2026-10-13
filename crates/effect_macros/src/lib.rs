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

use quote::quote;
mod async_expand;
mod callback_expand;
mod codec_expand;
mod descriptor_expand;
mod durable_expand;
mod expand;
mod ownership_expand;
mod parse;
mod persistable_expand;
mod restricted_expand;
mod schema_enum_expand;
mod schema_expand;
mod transaction_expand;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Construct a lazy reusable Effect with macro-local `?` binding.
///
/// `bind; program: source, body: factory,` composes an existing Effect-returning
/// factory without an additional body scope. `body |value| { ... }` is the inline
/// spelling of that same factory: ordinary Rust returns an Effect description,
/// and nested DSL macros construct its steps. The factory runs only after source
/// success and remains reusable; captured owned slots retain their typed reuse
/// errors. No payload is cloned or wrapped by this form.
#[proc_macro]
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn effect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as parse::EffectInput);
    let expanded = match input {
        parse::EffectInput::Statements(statements) => expand::expand(statements),
        parse::EffectInput::Continuation(fields) => callback_expand::continuation(fields),
        parse::EffectInput::WhileLoop(fields) => callback_expand::while_loop(fields),
    };
    match expanded {
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
/// Named `map_error;` maps expected journal errors. `modify;` receives a borrowed snapshot
/// and returns Result<(replacement, result), E>; `update;` returns Result<replacement,E>.
/// Both delegate TxRef::update in the same journal. No native Future or Effect is admitted.
#[proc_macro]
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn transaction(input: TokenStream) -> TokenStream {
    let tokens: proc_macro2::TokenStream = input.into();
    let result = if restricted_expand::has_mode(&tokens, &["map_error", "modify", "update"]) {
        syn::parse2(tokens).and_then(restricted_expand::transaction)
    } else {
        syn::parse::Parser::parse2(syn::Block::parse_within, tokens)
            .and_then(transaction_expand::expand)
    };
    result.unwrap_or_else(syn::Error::into_compile_error).into()
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
/// Raw `fold |state, record| { Result }` is pure; typed case callbacks keep their
/// existing borrowed payload/outcome inputs. No Effect projection is constructed.
#[proc_macro]
pub fn projection(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as descriptor_expand::NamedFields);
    descriptor_expand::projection(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Compose durable operations, or define a typed workflow with `id`, codecs,
/// compatibility, limits and a durable `run` closure.
/// Named map_error/for_each modes preserve Durable coordination. Retry uses explicit
/// run/branch/policy; compensate uses existing WorkflowCompensation run/on_failure
/// with its named activity body. Ordinary Effects remain confined to activity boundaries.
/// `result; value: result,` consumes one owned native Result as a durable success
/// or typed failure. It accepts values rather than Effects or Futures; construction
/// does not execute work, and repeated execution retains the native consumed-input defect.
#[proc_macro]
/// # Parameters
/// - `input` (`TokenStream`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
pub fn workflow(input: TokenStream) -> TokenStream {
    let tokens: proc_macro2::TokenStream = input.into();
    let result = if restricted_expand::has_mode(
        &tokens,
        &["map_error", "for_each", "retry", "compensate", "result"],
    ) {
        syn::parse2(tokens).and_then(restricted_expand::workflow)
    } else {
        match syn::parse2::<descriptor_expand::NamedFields>(tokens.clone()) {
            Ok(fields) if fields.contains("id") || fields.contains("name") => {
                descriptor_expand::workflow(fields)
            }
            _ => syn::parse::Parser::parse2(syn::Block::parse_within, tokens)
                .and_then(durable_expand::expand),
        }
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
/// Optional pure `project |outer| { Inner }` projects each handler Effect through
/// existing provision while retaining request ownership and operation identity.
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
/// Optional pure project transforms opening and every producer pull under the outer
/// environment; the existing RpcStreamContext and transport ownership remain intact.
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

/// Declare an inert resource-aware entity with typed activation-scoped handlers.
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
/// `env |outer|` selects a pure value; `operation |receiver|` returns a provider Effect.
/// Native `sync`/`future` receiver roles return `Result`/`Future<Result>` and may map native
/// errors with on_error. `acquire`/`release` construct the existing ServiceResource.
/// Borrowed receivers cannot escape; capture owned data before returning deferred work.
#[proc_macro]
pub fn service(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::service(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
/// Create a cache through its existing scoped Effect boundary. Required `scope`, `config`
/// and `options` retain caller-chosen equality, TTL, admission and retention. An inline
/// `load |key| { ... }` composes Effects; `load: callback` reuses an owned-key loader.
/// `load_shared: callback` retains the native Arc-key callback without cloning its key.
/// Lookup and invalidation return existing Effects under the same environment capabilities.
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
/// `map_error; program: tracked, map |error| { E2 }` maps a standalone AtomProgram
/// before registration. It retains the evaluation read context and never retypes a
/// registered atom or introduces a second publication/dependency mechanism.
#[proc_macro]
pub fn atom(input: TokenStream) -> TokenStream {
    let tokens: proc_macro2::TokenStream = input.into();
    let result = if restricted_expand::has_mode(&tokens, &["map_error"]) {
        syn::parse2(tokens).and_then(restricted_expand::atom)
    } else {
        syn::parse::Parser::parse2(syn::Block::parse_within, tokens).and_then(|statements| {
            crate::expand::effect_crate_path().map(|path| {
                context_expand::expand(
                    statements,
                    quote!(#path::reactivity::AtomProgram::from_body),
                )
            })
        })
    };
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
/// `acquire |context| { ... }` exposes a borrowed EntityPlanContext inside the
/// activation future while binding through its original externally owned scope.
/// `acquire: helper` accepts the same typed context-to-Future factory.
#[proc_macro]
pub fn entity_resource(input: TokenStream) -> TokenStream {
    let tokens: proc_macro2::TokenStream = input.into();
    let result = match syn::parse2::<callback_expand::Input>(tokens.clone()) {
        Ok(input) if input.fields.contains("acquire") => restricted_expand::entity_resource(input),
        _ => syn::parse::Parser::parse2(syn::Block::parse_within, tokens).and_then(|statements| {
            crate::expand::effect_crate_path().map(|path| {
                context_expand::expand(
                    statements,
                    quote!(#path::cluster::EntityResourcePlan::from_body),
                )
            })
        }),
    };
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
/// Named iterable/async_iterable/async_iterable_effect and pull/read/release forms
/// construct scoped sources. An extracted `read: callback` retains the native pull factory;
/// inline reads construct fresh work for the next pull, sharing the checked resource handle.
/// map_effect/map_error/project transform an explicit stream.
/// Native async factories return owned iterators; release is mandatory. Mapped opening,
/// pull and cleanup errors stay owned by the existing scope; no second stream engine.
#[proc_macro]
pub fn stream(input: TokenStream) -> TokenStream {
    let tokens: proc_macro2::TokenStream = input.into();
    let result = match syn::parse2::<callback_expand::Input>(tokens.clone()) {
        Ok(input) => ownership_expand::demand("stream", input),
        Err(_) => syn::parse::Parser::parse2(syn::Block::parse_within, tokens)
            .and_then(stream_expand::expand),
    };
    result.unwrap_or_else(syn::Error::into_compile_error).into()
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
/// entity definitions acquire only at activation. Activation returns an Effect using the supplied
/// owner/scope and the existing dispatcher, preserving typed acquisition/registration
/// failures and accepted-address checks. This creates no cluster runner or store.
#[proc_macro]
pub fn cluster(input: TokenStream) -> TokenStream {
    let fields = parse_macro_input!(input as domain_expand::Fields);
    domain_expand::cluster(fields)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Native Identity capability, workflow and route DSL lowering.
mod extension_identity;
/// OTel convenience lowering over the existing Effect span/context path.
mod extension_otel;
/// HTTP description and owned endpoint/body DSL lowering.
mod extension_server;
/// SQL statement and selected session DSL lowering; no SQL runs at expansion time.
mod extension_sql;
/// Common hygienic lowering support for optional native extension descriptions.
mod extension_support;
/// Web policy, localization, cache and application DSL lowering.
mod extension_web;

/// Compose typed SQL syntax with `text`, `identifier`, `fragment`, `where` and `param` operations. Native parameters remain bound values, distinct from trusted syntax.
#[proc_macro]
pub fn sql(input: TokenStream) -> TokenStream {
    extension_sql::expand("sql", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe a nonempty insert with `table:` and repeated `value(column, value)` entries. Construction validates shape; execution owns and binds each value.
#[proc_macro]
pub fn sql_insert(input: TokenStream) -> TokenStream {
    extension_sql::expand("sql_insert", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe nonempty `set(column, value)` assignments for `table:` and one guarded `where(predicate)`. Duplicate guards and empty assignments are rejected during expansion.
#[proc_macro]
pub fn sql_update(input: TokenStream) -> TokenStream {
    extension_sql::expand("sql_update", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe named or ordinal `column` reads, optional cells, refinements and scoped Effect validation. The row remains owned by its native query scope and cannot escape borrowed access.
#[proc_macro]
pub fn row(input: TokenStream) -> TokenStream {
    extension_sql::expand("row", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Select a SQL service using `select:`, an explicit handle or projection, and `on_error:`. Inline `run |sql| { ... }` and extracted `run: helper` compose Effects without selecting a second runtime.
#[proc_macro]
pub fn sql_with(input: TokenStream) -> TokenStream {
    extension_sql::expand("sql_with", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe an inert host with explicit `policy:`, `environment:`, `listen:`, `protocols:` and `routes:` plus native builder, application, middleware and WebSocket hooks. Serving starts only through the returned description’s run/use-host Effect.
#[proc_macro]
pub fn server(input: TokenStream) -> TokenStream {
    extension_server::expand("server", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe a request Effect ending in a typed reply. Inline body, extracted Effect, preconditions, pipeline and scoped error handling execute per request; metadata and body ownership remain distinct.
#[proc_macro]
pub fn endpoint(input: TokenStream) -> TokenStream {
    extension_server::expand("endpoint", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Attach registration-time name, tags, summary, description, metadata/filter/configure, accepts/produces/hosts and authorization policy to an endpoint. Registration does not consume a request or execute endpoint Effects.
#[proc_macro]
pub fn registered(input: TokenStream) -> TokenStream {
    extension_server::expand("registered", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Build a typed reply with status, headers, cookies, trailers, validators and one terminal body. Text, HTML, JSON/direct codec, problem, bytes, stream/SSE, negotiation, native, redirect, file, no-content and empty forms preserve body lifetime and commitment.
#[proc_macro]
pub fn reply(input: TokenStream) -> TokenStream {
    extension_server::expand("reply", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe checked media/language alternatives using text, HTML, JSON/direct codec, stream or general variants. Negotiation selects a renderer lazily before producing its body.
#[proc_macro]
pub fn representations(input: TokenStream) -> TokenStream {
    extension_server::expand("representations", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe literals/prefix, name/tags/configure and typed capture, constrained, optional, default or catch-all segments. Link generation uses the same codecs and escaping as request matching.
#[proc_macro]
pub fn route(input: TokenStream) -> TokenStream {
    extension_server::expand("route", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Assemble method routes, WebSocket routes, groups/configured groups, native services and fallback with scoped errors and middleware. Dynamic method and route descriptions retain typed validation.
#[proc_macro]
pub fn router(input: TokenStream) -> TokenStream {
    extension_server::expand("router", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Compose reusable ordered route sets using includes, conditional or iterated sets and extracted modules. Route registration stays separate from per-request Effect execution.
#[proc_macro]
pub fn routes(input: TokenStream) -> TokenStream {
    extension_server::expand("routes", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Compose ordered before/after/guard/on-exit/respond/set-header stages with inline or extracted Effect callbacks. Short-circuiting retains ownership of the awaited continuation and request cleanup.
#[proc_macro]
pub fn pipeline(input: TokenStream) -> TokenStream {
    extension_server::expand("pipeline", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct a server body program with the shared stream lowering, scoped Effect/resource binds, incremental writes/framing/flush and delegation. Re-exported as `effect_server::stream!`; response completion owns finalization.
#[proc_macro]
pub fn server_stream(input: TokenStream) -> TokenStream {
    extension_server::expand("server_stream", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Select typed accounts, sessions, keys, roles, permissions, challenges, queries, audit, consent, delivery, notifications, jobs, personal-data and native query capabilities. Selecting handles performs no persistence and grants no authority by itself.
#[proc_macro]
pub fn identity_store(input: TokenStream) -> TokenStream {
    extension_identity::expand("identity_store", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe native Identity storage, typed errors, password/authenticator/external/passkey/email-confirmation policies, clock, components, admission and browser mode. Construction is inert; selected persistence has no implicit fallback.
#[proc_macro]
pub fn identity(input: TokenStream) -> TokenStream {
    extension_identity::expand("identity", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe prefixed registration/login/recovery/authenticator/passkey/external/management/logout/refresh/antiforgery groups and custom GET/POST routes. Handlers and renderers compose through typed Effects and selected grants.
#[proc_macro]
pub fn identity_endpoints(input: TokenStream) -> TokenStream {
    extension_identity::expand("identity_endpoints", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Assemble an inert application with root/native errors, startup, localization, environment, native hooks, security, observation, availability, output cache, protocols/listen and routes. The selected host owns initialization and cleanup.
#[proc_macro]
pub fn web(input: TokenStream) -> TokenStream {
    extension_web::expand("web", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Compose explicit browser, CSP/nonce, trusted-forwarding, CORS/header/CSRF/authentication/authorization, compression/static-file, body-bound, method-override and HSTS policies. Nonces are bound per response and forwarding requires selected trust.
#[proc_macro]
pub fn security(input: TokenStream) -> TokenStream {
    extension_web::expand("security", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe typed CSP directives and append operations with incompatible-source diagnostics. Per-response nonce sources remain distinct from static policy configuration.
#[proc_macro]
pub fn csp(input: TokenStream) -> TokenStream {
    extension_web::expand("csp", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Acquire/project a scoped core cache with explicit loader, keys, retention, admission and failure-caching policy. Inline `loader |key| { ... }` and extracted loaders use the existing cache engine.
#[proc_macro]
pub fn read_through(input: TokenStream) -> TokenStream {
    extension_web::expand("read_through", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Acquire/project the core dependency cache with explicit key and dependency capture/selection policies. Active-load invalidation delegates to the existing engine and does not create a second cache runtime.
#[proc_macro]
pub fn dependent_read_through(input: TokenStream) -> TokenStream {
    extension_web::expand("dependent_read_through", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe service/error policy, readiness/health/liveness/process-liveness probes, deadlines, parallelism, traffic readiness and an optional dashboard. Probe outcomes remain classified rather than being reduced to booleans.
#[proc_macro]
pub fn availability(input: TokenStream) -> TokenStream {
    extension_web::expand("availability", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Describe typed named `field(name, value)` arguments checked against the selected `message:` contract. Construction does not use raw string interpolation or erase argument types.
#[proc_macro]
pub fn message_arguments(input: TokenStream) -> TokenStream {
    extension_web::expand("message_arguments", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Select catalogs, cultures, request selection, missing/fallback policy and scoped reload configuration. Catalog generations and reload failures remain explicit; selection performs no hidden global lookup.
#[proc_macro]
pub fn localization(input: TokenStream) -> TokenStream {
    extension_web::expand("localization", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Run a named Effect through the existing span/context path with optional typed attributes and links. Inline `body { ... }` or extracted `effect:` supports lazy tags, events, count, log, status and link operations; construction exports nothing.
#[proc_macro]
pub fn otel(input: TokenStream) -> TokenStream {
    extension_otel::expand("otel", input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Delay `body { value }` with the reusable sync factory. `once;` consumes owned captures without Clone. Ordinary Rust statements and Result boundaries are unchanged; panics remain defects.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_sync(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::leaf("effect_sync", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Delay a native Result body and map its expected error with optional `on_error |problem| { E }`. `once;` consumes owned input once. Native `?` propagates Result errors and never binds an Effect; panics remain defects.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_try_sync(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::leaf("effect_try_sync", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Delay `body |cancel| { future }` with the native Future owner. Use an explicit `async move` tail for awaits. `once;` owns captures once; `with_env;` receives `Arc<R>`, Cancellation on every run. R selects capabilities.
/// `exit_with_context;` receives `Arc<R>` and NativeContext and returns a future of the complete typed Exit.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_future(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::leaf("effect_future", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct `Die` from `defect: expression`, evaluated once without reclassifying it as an expected error.
#[proc_macro]
pub fn effect_die(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::defect(definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Delay a `Future<Result<A, P>>` factory with explicit Cancellation and optional P-to-E mapping. Native awaits/Result `?` stay inside its async block. `once;` owns captures once; `with_env;` receives `Arc<R>`, Cancellation. Cancellation remains the existing Future-owner protocol.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_try_future(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::leaf("effect_try_future", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Submit `body |cancel| { value }` through R's CPU provider, joining accepted work on cancellation. Reusable by default; `once;` moves captures. `with_context;` and `once_with_context;` additionally receive the existing TraceContext.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_cpu(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::leaf("effect_cpu", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Submit a native Result body through R's CPU provider, with optional `on_error` mapping. Supports reusable, `once;`, and reusable `with_context;` contracts. Accepted work is joined; panics stay defects.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_try_cpu(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::leaf("effect_try_cpu", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Submit `body |cancel| { value }` through R's separate blocking provider. Reusable, `once;`, `with_context;` and `once_with_context;` retain cancellation/join and trace context. No executor is constructed.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_blocking(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::leaf("effect_blocking", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Submit a native Result body through R's blocking provider, with optional `on_error`. Reusable, `once;` and reusable `with_context;` preserve native errors, defects and accepted-work settlement.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_try_blocking(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::leaf("effect_try_blocking", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Transform the owned success of `program` with pure `map |value| { B }`; execution is delayed and all failure/cleanup channels retain their owners.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_map(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_map", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Transform only expected errors of `program` with pure `map |error| { E2 }`. Defects and interruption remain distinct, including retained cleanup outcomes.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_map_error(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_map_error", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Project borrowed outer R with `project |outer| { Inner }` for `program`. Existing provision preserves trace/runtime context; inputs do not define an alternate environment.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_provide_some(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_provide_some", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Explicitly convert a defect from `program` using `defect |defect| { E }`. Expected errors and interruption retain their existing channels; no implicit panic-to-domain conversion.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_sandbox(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_sandbox", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Map absence lazily using `missing { E }`. `once; value: option` consumes non-Clone payloads; reusable `value { option }` creates fresh input on each execution. Colon values retain the existing constructor's Clone bound.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_from_option(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_from_option", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Apply explicit `duration: Duration` and lazy `on_timeout { E }` to `program` or an Effect `body`. The existing timeout owns cancellation and join; construction runs no body.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_timeout(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_timeout", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Recover expected errors with `recover |error| { Effect-bound body }` or an extracted typed factory. Each owned error feeds one fresh one-shot Effect; defects/interruption are not caught.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_recover(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_recover", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Traverse `values` through the existing owned collection contract, or supply reusable `produce { iterable }` for fresh inputs. `body |item|` binds Effects; producer traversal returns unit and rebuilds on execution.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_for_each(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_for_each", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Traverse with mandatory `concurrency: NonZeroUsize` and borrowed item callback. `shared;` accepts the existing Arc slice variant. Capture owned data explicitly before a delayed body; no payload Clone is added.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_for_each_concurrent(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_for_each_concurrent", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Provide the explicit `environment: Arc<R>` to `program` or an Effect `body`, retaining the existing context. The returned Effect requires unit environment.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_provided(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_provided", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Race `right` against `program` or an Effect `body` with the existing loser cancellation/join and retained cleanup rules.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_race(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_race", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Mask interruption for `program` or an Effect `body` with the existing operator. Braces construct a reusable description and do not execute a nested run.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_uninterruptible(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_uninterruptible", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Attach `finalizer: effect` or inline `finalizer { Effect body }` to `program`/`body`. Cleanup runs only under the existing interruption boundary and retains cleanup failures.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_on_interrupt(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_on_interrupt", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Assemble ordered inert `items: Vec<Effect>` or `items { item!(effect); items_from!(effects); }` with ordinary Rust control flow. Local markers are consumed here; effects execute only when the resulting description runs.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_all(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_all", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Assemble ordered inert items using the same owner-local markers as effect_all, then run with mandatory `concurrency: NonZeroUsize`. Existing admission/cancellation/ordered cleanup remain authoritative.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_all_concurrent(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_all_concurrent", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Retry `program` or reusable Effect `body` with an explicit typed `schedule`. R supplies the clock and schedule sampling capability; one-shot payloads are not made replayable.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_retry(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_retry", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Repeat successful execution of `program` or reusable Effect `body` with an explicit typed `schedule`. R supplies clock/sampling; failures/interruption and cleanup retain existing semantics.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_repeat(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    callback_expand::operator("effect_repeat", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Acquire through ResourceBinding, pass checked `ResourceValue<T>` to Effect `body |handle|`, and consume T in cleanup. One-input `release` binds Effects; two-input `release |resource, primary|` synchronously inspects borrowed PrimaryOutcomeRef and returns `Effect<(), E, R>` or Release. Colon release helpers use the factory boundary; the primary borrow cannot escape.
/// `ensuring; program: effect, release: callback` attaches a primary-aware Effect cleanup through the native ensuring operator.
/// `release_exit_future; body: callback` constructs a one-shot native Release whose future returns the complete Exit.
/// `release_sync; body: callback` defers a one-shot synchronous native disposal without an Effect wrapper.
/// `release_try_sync`, `release_future`, and `release_try_future` retain the native one-shot release boundaries.
/// `release_effect; program: effect` adapts an existing cleanup description into a Release.
/// `ensuring_release; program: effect, release: callback` retains a primary-aware native Release factory.
/// `use; binding: resource, body: callback` consumes an existing ResourceBinding without reacquisition.
/// `binding; acquire: effect, release: callback` returns the inert ResourceBinding for an existing scoped binder.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_resource(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::ownership("effect_resource", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Use a checked resource handle with Drop-backed cleanup by default, or an explicit release role with the same factory/Effect distinction as effect_resource. ResourceBinding owns protected release; no AsyncDrop protocol is invented.
/// `binding; acquire: effect` returns its inert Drop-backed binding without acquiring or opening a scope.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_using(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::ownership("effect_using", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Create a fresh owner per run with Effect `body |scope|`. Its existing close joins ordered finalizers and retains primary/cleanup failures; construction registers nothing.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_scope(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::ownership("effect_scope", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Acquire into explicit `scope`, returning checked `ResourceValue<T>`. Resource-only `release |resource|` binds Effects; colon release factories return Effect/Release. Existing scope owns close and late acquisition.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_scope_acquire(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::ownership("effect_scope_acquire", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Register `program` or Effect `body` in explicit `scope`. `release; release { Release }` registers a fresh factory. The scope retains the registration environment and owns protected close/late-registration behavior.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_finalizer(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::ownership("effect_finalizer", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct a Layer with Effect `build |scope|`. `scoped;` uses acquire/release ResourceBinding roles; `merge;` requires left/right and pure two-input combine; `provide;` composes typed input/output Layers. Acquisition remains deferred to its owner.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn layer(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::ownership("layer", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Provide the explicit `layer` to `program` or Effect `body`. Existing Layer ownership retains services until application cleanup and keeps typed input/output environments.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_with_layer(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::ownership("effect_with_layer", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Acquire explicit `permits: NonZeroUsize` from `semaphore`, run `program`/Effect `body`, and release the owned permit through ResourceBinding. Optional on_error maps admission and release. `partitioned;` requires key or reusable key_factory; no default capacity is inferred.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_permit(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::ownership("effect_permit", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Unfold explicit `state` through pure `step |state, input, elapsed, sample|`. Inputs borrow State/I and receive Duration/`Option<UnitInterval>`; output is the existing transition Result. State and sampling policy retain current semantics.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn schedule(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::state("schedule", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Require modify/update/in_place/synchronized/synchronized_pure mode and explicit reference. Pure roles use existing Ref transitions; synchronized roles use modify_effect and mandatory on_error admission mapping. Owned snapshots may feed delayed work; publication is not replaced by read-then-set.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_ref(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::state("effect_ref", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Create a scoped pool with explicit capacity/idle_ttl, Effect create, borrowed validate and owned dispose roles. Borrowed validation is inspected before returning owned delayed work. `exists_idle;` supplies pool/predicate. Existing admission, TTL and protected disposal remain authoritative.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn pool(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::state("pool", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Load explicit cache/key through zero-input Effect load with optional partition. The existing weak index owns key lifetime; expansion captures no implicit strong key. Extracted loaders keep the same reusable factory contract.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn weak_cache(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::state("weak_cache", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct the existing scoped grouped resolver with Effect `run |group, requests|`. Return ordered `Vec<Exit<A, E>>` with one outcome per request; whole-batch E remains separate. R supplies sharing; no timed batching policy is added.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn request_resolver(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::state("request_resolver", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Require for_each/fold/fold_effect mode. Effect body consumes owned items; folds require native initial factory and two-input step. Pure fold executes via delayed sync-once step over the existing fold engine; construction opens no stream.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn sink(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::demand("sink", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Consume explicit stream with Effect `body |item|`. Stream scope owns opening, pulls and release on success/failure/interruption; the callback consumes each item once.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_stream_for_each(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::demand("effect_stream_for_each", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Use explicit socket acquire/limits or connection options with Effect `body |connection|` and optional on_error. The checked connection handle retains existing native transport/cleanup ownership. No WebSocket runtime or policy defaults are added.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_socket(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    ownership_expand::demand("effect_socket", definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Construct paired directions with pure/effectful/contextual modes. Pure inputs borrow and return Result; effectful inputs own and bind Effects under separate decode/encode environments. Contextual mode requires ast and forwards optional ParseContext. Transform modes use source; recursive modes require name/shape/build; lazy returns a standard LazyLock description. No validator/writer witness is inferred.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn codec(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    codec_expand::codec(definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Parse text using explicit ConfiguredCodec in `codec`, preserving its directional decode environment and configured schema/JSON/pattern/issue budgets. Optional on_error maps SchemaIssue; parsing remains lazy.
/// Inline roles are braced; `role: helper` reuses the same typed boundary.
#[proc_macro]
pub fn effect_schema_parse(input: TokenStream) -> TokenStream {
    let definition = parse_macro_input!(input as callback_expand::Input);
    codec_expand::parse(definition)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
