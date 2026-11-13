//! Commands: action availability/support and consumer-bound command execution.
//!
//! Commands owns workflow W8:
//!
//! - the submitted action value model and the action-label key table;
//! - the action label resolver over the caller's translation port;
//! - the consumer-supplied executor bindings keyed by resource kind and action,
//!   their ordered collection and live resolution;
//! - the mutable command-input identity, the observed mutation result, the service
//!   option surface, action availability/support and the execution sequence
//!   validate/mutate input → support and executor selection from the updated input →
//!   invocation → fill missing identity from the updated input → validate result.
//!
//! # Consumer-owned data access
//!
//! Every command executor is a consumer-supplied capability stored in
//! [`ResourceCommandBindings`]; there is no executor by service name, no optional
//! capability, no setter and no runtime registry. The CMS owns no CRUD
//! implementation, database, SQL statement, schema, migration, pool or backend
//! selection. The label translator and the input/result validators are likewise
//! caller-supplied callbacks.
//!
//! # Effects and record payloads
//!
//! Action availability and support read the caller-supplied definition registry and
//! the label port without I/O; availability is a lazy [`effect::Effect`]. Command
//! execution sequences the supplied callbacks with `effect!`, so an executor is
//! invoked only when the description runs. Consumer callbacks keep their own
//! expected error family and environment; CMS request/action failures stay typed in
//! [`CommandError`]. [`CommandInput`] preserves the query owner's admitted list and
//! detail records and the supplied form bytes, [`CommandInputHandle`] shares one
//! mutable input through the callbacks, and [`MutationResult`] preserves the redirect
//! route/language and the shared redirect-parameter map contents.
//!
//! # Branches preserved
//!
//! - Actions are projected in declaration order, and `translate` is appended once
//!   when the definition declares widgets.
//! - `translate` is supported exactly when widgets are declared; every other
//!   action is supported exactly when the definition lists it.
//! - An unknown kind yields no supported action and an availability error carrying
//!   the registry's own lookup error.
//! - Execution rejects an absent input, a refused action and a missing executor,
//!   and fills an empty result kind/action from the input before result
//!   validation. An absent executor result/payload is unrepresentable because a
//!   successful executor returns the typed [`MutationResult`] itself.
//! - Private availability/support stage factories and their one-shot replay/forgery
//!   bookkeeping adapt interface visibility and nil branches. Here ownership makes
//!   a forged or replayed stage unrepresentable, so there is no runtime stage
//!   registry to reproduce.
//!
//! # HTML
//!
//! This owner produces data for form/rendering owners and no HTML.

mod actions;
mod bindings;
mod service;

pub use self::actions::{
    ActionLabelKeys, ActionLabelResolver, SubmitActionVM, TranslateActionLabelKeyFunc,
    action_label_key, new_action_label_resolver,
};
pub use self::bindings::{
    ResourceCommandBinder, ResourceCommandBindings, ResourceCommandExecutor,
    new_resource_command_bindings,
};
pub use self::service::{
    ActionResolver, CommandError, CommandInput, CommandInputHandle, CommandInputValidator,
    CommandService, ERR_INVALID_REQUEST, ERR_UNSUPPORTED_ACTION, Executor, InputBytes,
    MutationResult, MutationResultValidator, Options, new_service,
};
