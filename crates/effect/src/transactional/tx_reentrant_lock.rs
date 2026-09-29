use super::{
    HasTransactionDomain, Transaction, TransactionDomain, TransactionDomainError, TransactionError,
    TransactionPolicy, TxRef, TxValue,
};
use crate::{Effect, Release, ResourceValue, Scope};
use crate::{KeyDomain, StableKey};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TxExecutionId(Arc<[u8]>);

impl TxExecutionId {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl TxValue for TxExecutionId {}

#[derive(Clone)]
struct LockState {
    owner: Option<TxExecutionId>,
    depth: usize,
}

impl TxValue for LockState {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxLockError {
    NotHeld,
    WrongOwner,
    DepthExhausted,
}

pub struct TxReentrantLock {
    cell: TxRef<LockState>,
}

impl Clone for TxReentrantLock {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl TxReentrantLock {
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
    pub fn acquire(&self, owner: TxExecutionId) -> Transaction<(), TxLockError> {
        let cell = self.cell.clone();
        self.cell.read().flat_map(move |state| match &state.owner {
            Some(current) if current != &owner => Transaction::retry(),
            _ => match state.depth.checked_add(1) {
                Some(depth) => cell.write(Arc::new(LockState {
                    owner: Some(owner.clone()),
                    depth,
                })),
                None => Transaction::fail_with(|| TxLockError::DepthExhausted),
            },
        })
    }

    #[must_use]
    pub fn release(&self, owner: TxExecutionId) -> Transaction<(), TxLockError> {
        let cell = self.cell.clone();
        self.cell.read().flat_map(move |state| match &state.owner {
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
        })
    }

    #[must_use]
    pub fn held_by<E: Send + 'static>(&self, owner: TxExecutionId) -> Transaction<bool, E> {
        self.cell
            .read()
            .map(move |state| state.owner.as_ref() == Some(&owner))
    }

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
        scope.acquire(self.acquire(owner).atomically(policy), move |()| {
            Release::from_effect(lock.release(release_owner.clone()).atomically(policy))
        })
    }
}
