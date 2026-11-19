//! Prepared STM payload refusal preserves the committed value and skips updates.

use effect::memory::{MemoryError, RegionId};
use effect::transactional::{
    HasTransactionDomain, PreparedTransactionCapacity, PreparedTxValuePool, TransactionDomain,
    TransactionError, TransactionPolicy,
};
use effect::{Exit, memory, transaction};
use futures_executor::block_on;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const WORK: RegionId = RegionId::named("work");
const ONE: NonZeroUsize = NonZeroUsize::MIN;
const TWO: NonZeroUsize = ONE.saturating_add(1);

struct Environment(Arc<TransactionDomain>);

impl HasTransactionDomain for Environment {
    fn transaction_domain(&self) -> Arc<TransactionDomain> {
        Arc::clone(&self.0)
    }
}

#[test]
fn retained_value_refuses_old_frame_reuse_before_the_update_callback() -> Result<(), String> {
    let prepared = (memory! {
        total: 128 KiB, mode: prepared, metadata: work, partition work: 128 KiB,
    })
    .and_then(|plan| plan.prepare())
    .map_err(|error| format!("{error:?}"))?;
    let domain = (transaction! {
        prepare_domain;
        memory: &prepared, region: WORK,
        capacity: PreparedTransactionCapacity {
            cells: ONE, reads: ONE, writes: ONE, journals: ONE, retry_waiters: ONE,
        },
    })
    .map_err(|error| format!("{error:?}"))?;
    let values = PreparedTxValuePool::<u32>::prepare(&prepared, WORK, TWO)
        .map_err(|error| format!("{error:?}"))?;
    let reference = domain
        .reference_prepared(0, values)
        .map_err(|error| format!("{error:?}"))?;
    let environment = Arc::new(Environment(Arc::new(domain)));
    let count = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&count);
    let policy = TransactionPolicy { max_conflicts: ONE };
    let read = reference.read::<()>().atomically(policy);
    let update = reference
        .update(move |value| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok::<_, ()>((*value + 1, ()))
        })
        .map_err(|error| format!("{error:?}"))?
        .atomically(policy);
    let _sealed = prepared.seal();

    let Exit::Success(retained) = block_on(read.run(Arc::clone(&environment))) else {
        return Err("initial selected read failed".into());
    };
    assert!(matches!(
        block_on(update.run(Arc::clone(&environment))),
        Exit::Success(())
    ));
    assert!(matches!(
        block_on(update.run(Arc::clone(&environment))),
        Exit::Fail(TransactionError::Memory(MemoryError::NoSlot))
    ));
    assert_eq!(*retained, 0);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    let Exit::Success(current) = block_on(read.run(Arc::clone(&environment))) else {
        return Err("selected read after refusal failed".into());
    };
    assert_eq!(*current, 1);
    drop(retained);
    assert!(matches!(
        block_on(update.run(environment)),
        Exit::Success(())
    ));
    assert_eq!(*current, 1);
    assert_eq!(count.load(Ordering::Relaxed), 2);
    Ok(())
}
