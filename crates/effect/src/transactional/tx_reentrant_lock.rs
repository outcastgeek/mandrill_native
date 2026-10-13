//! Scoped reentrant transaction ownership.

use super::{
    HasTransactionDomain, Transaction, TransactionDomain, TransactionDomainError, TransactionError,
    TransactionPolicy, TxRef, TxValue,
};
use crate::{Effect, ResourceValue, Scope};
use crate::{KeyDomain, StableKey};
use crate::{effect_scope_acquire, transaction};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by transactional tx reentrant lock operations.
pub struct TxExecutionId(Arc<[u8]>);

impl TxExecutionId {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl TxValue for TxExecutionId {}

#[derive(Clone)]
/// Typed lock state used by transactional tx reentrant lock.
struct LockState {
    owner: Option<TxExecutionId>,
    depth: usize,
}

impl TxValue for LockState {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// An ownership or domain failure in a transactional reentrant lock.
pub enum TxLockError {
    /// The supplied owner does not hold this lock.
    NotHeld,
    /// The supplied owner does not match the required or recorded value.
    WrongOwner,
    /// No further depth can be allocated without exceeding its range.
    DepthExhausted,
}

/// A lock whose ownership state is coordinated within a transaction domain.
pub struct TxReentrantLock {
    cell: TxRef<LockState>,
}

impl Clone for TxReentrantLock {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl TxReentrantLock {
    /// Construct a `TxReentrantLock` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `domain` (`&TransactionDomain`): Stable identity or provider of the operation’s transaction domain.
    ///
    /// # Returns
    /// Type: `Result<Self, TransactionDomainError>`.
    pub fn new(domain: &TransactionDomain) -> Result<Self, TransactionDomainError> {
        domain
            .reference(LockState {
                owner: None,
                depth: 0,
            })
            .map(|cell| Self { cell })
    }

    /// Ownership follows an application-supplied logical scoped execution ID,
    /// never a physical thread ID that may change across an await.
    #[must_use]
    /// # Parameters
    /// - `owner` (`TxExecutionId`): Worker or scope identity used to fence stale operations.
    ///
    /// # Returns
    /// Type: `Transaction<(), TxLockError>`.
    pub fn acquire(&self, owner: TxExecutionId) -> Transaction<(), TxLockError> {
        let cell = self.cell.clone();
        {
            let __dsl_transaction = self.cell.read();
            transaction! {
            let state = __dsl_transaction.clone()?; match &state.owner {
                        Some(current) if current != &owner => Transaction::retry(),
                        _ => match state.depth.checked_add(1) {
                            Some(depth) => cell.write(Arc::new(LockState {
                                owner: Some(owner.clone()),
                                depth,
                            })),
                            None => Transaction::fail_with(|| TxLockError::DepthExhausted),
                        },
                    }?
            }
        }
    }

    #[must_use]
    /// End this claim or lease and return its settlement status.
    ///
    /// # Parameters
    /// - `owner` (`TxExecutionId`): Worker or scope identity used to fence stale operations.
    ///
    /// # Returns
    /// Type: `Transaction<(), TxLockError>`.
    pub fn release(&self, owner: TxExecutionId) -> Transaction<(), TxLockError> {
        let cell = self.cell.clone();
        {
            let __dsl_transaction = self.cell.read();
            transaction! {
            let state = __dsl_transaction.clone()?; (|| { match &state.owner {
                        None => Transaction::fail_with(|| TxLockError::NotHeld),
                        Some(current) if current != &owner => {
                            Transaction::fail_with(|| TxLockError::WrongOwner)
                        }
                        Some(_) => {
                            let Some(depth) = state.depth.checked_sub(1) else {
                                return Transaction::fail_with(|| TxLockError::NotHeld);
                            };
                            let next_owner = if depth == 0 {
                                None
                            } else {
                                Some(owner.clone())
                            };
                            cell.write(Arc::new(LockState {
                                owner: next_owner,
                                depth,
                            }))
                        }
                    } })()?
            }
        }
    }

    #[must_use]
    /// Check whether the supplied owner currently holds the reentrant lock.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    ///
    /// # Parameters
    /// - `owner` (`TxExecutionId`): Worker or scope identity used to fence stale operations.
    ///
    /// # Returns
    /// Type: `Transaction<bool, E>`.
    pub fn held_by<E: Send + 'static>(&self, owner: TxExecutionId) -> Transaction<bool, E> {
        {
            let __dsl_transaction = self.cell.read();
            transaction! {
            let state = __dsl_transaction.clone()?; state.owner.as_ref() == Some(&owner)
            }
        }
    }

    /// Acquire the resource under a scope that owns its release.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasTransactionDomain + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `scope` (`&Scope<TransactionError<TxLockError>>`): Owner that joins child work and releases resources at close.
    /// - `owner` (`TxExecutionId`): Worker or scope identity used to fence stale operations.
    /// - `policy` (`TransactionPolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Effect<ResourceValue<()>, TransactionError<TxLockError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn acquire_scoped<R>(
        &self,
        scope: &Scope<TransactionError<TxLockError>>,
        owner: TxExecutionId,
        policy: TransactionPolicy,
    ) -> Effect<ResourceValue<()>, TransactionError<TxLockError>, R>
    where
        R: HasTransactionDomain + Send + Sync + 'static,
    {
        let lock = self.clone();
        let release_owner = owner.clone();
        effect_scope_acquire! {
        scope: scope, acquire: self.acquire(owner).atomically(policy), release: move |()| {
                    lock.release(release_owner.clone()).atomically(policy)
                },
        }
    }
}
