//! Atomic transactions and committed observations.
//!
//! [`Transaction`] describes reads and writes against one optimistic journal.
//! `atomically` validates its read set, commits all writes together, and
//! publishes changes only after commit. Expected failure abandons the journal;
//! conflict retry follows the caller's policy. A transactional `retry` waits
//! for an observed cell to change. This is process-local synchronization, not
//! a persistent database or a substitute for a workflow engine.

mod internal;
mod transaction;
mod tx_chunk;
mod tx_deferred;
mod tx_hash_map;
mod tx_hash_set;
mod tx_priority_queue;
mod tx_pub_sub;
mod tx_queue;
mod tx_reentrant_lock;
mod tx_ref;
mod tx_semaphore;
mod tx_subscription_ref;
mod tx_value;

pub(crate) use transaction::TxStep;
pub use transaction::{Transaction, TransactionError, TransactionPolicy};
pub use tx_chunk::TxChunk;
pub use tx_deferred::TxDeferred;
pub use tx_hash_map::TxHashMap;
pub use tx_hash_set::TxHashSet;
pub use tx_priority_queue::TxPriorityQueue;
pub use tx_pub_sub::{
    TxPubSub, TxPubSubError, TxPubSubOverflow, TxPubSubSubscription, TxPublishReceipt,
};
pub use tx_queue::TxQueue;
pub use tx_reentrant_lock::{TxExecutionId, TxLockError, TxReentrantLock};
pub use tx_ref::{HasTransactionDomain, TransactionDomain, TransactionDomainError, TxRef};
pub use tx_semaphore::{TxSemaphore, TxSemaphoreError};
pub use tx_subscription_ref::{TxSubscription, TxSubscriptionError, TxSubscriptionRef};
pub use tx_value::TxValue;
