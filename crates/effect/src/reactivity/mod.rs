//! Scoped reactive state, sources, and integration bridges.
//!
//! A source provides a snapshot Effect and a stream of later changes. Atoms,
//! committed transactional state, persisted state, and RPC-backed queries can
//! share that consumer-facing contract while using different capabilities in
//! `R`. Invalidation tells consumers to resnapshot; it is not proof of a
//! durable write. A source must establish its own initial snapshot/subscription
//! boundary, and a lagging subscriber must resnapshot rather than infer history
//! from missed notifications.

mod async_result;
mod atom;
mod atom_cache;
mod atom_persistence;
/// Context-carrying tracked programs around the existing Atom evaluator.
mod atom_program;
mod atom_ref;
mod atom_registry;
mod atom_rpc;
mod hydration;
mod invalidation;
mod query;
mod source;

pub use async_result::{AsyncResult, AsyncResultPhase};
pub use atom::{Atom, AtomError};
pub use atom_cache::{AtomCache, AtomCacheError};
pub use atom_persistence::{
    PersistedAtom, PersistedAtomError, PersistedAtomLoad, PersistedAtomSave,
};
pub use atom_ref::{AtomDependencyError, AtomReadContext, AtomRef, AtomRefError};
pub use atom_registry::{AtomId, AtomKind, AtomRegistry, AtomRegistryError};
pub use atom_rpc::{
    RpcMiddlewareMutationAtom, RpcMiddlewareMutationError, RpcMiddlewareQueryAtom, RpcMutationAtom,
    RpcMutationError, RpcQueryAtom,
};
pub use hydration::{
    HydrationConflictPolicy, HydrationError, HydrationImportOutcome, HydrationRecordResult,
    HydrationSchema, HydrationSet,
};
pub use invalidation::{
    InvalidationBatch, InvalidationError, InvalidationKey, InvalidationRegistry,
    InvalidationSubscription,
};
pub use query::{ReactiveQuery, ReactiveQueryError};
pub use source::{EffectStreamSource, ReactiveStateSource};

pub use atom_program::AtomProgram;
#[doc(hidden)]
pub use atom_program::{AtomBindable, AtomProgramContext};
