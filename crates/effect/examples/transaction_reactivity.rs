use effect::Exit;
use effect::reactivity::ReactiveStateSource;
use effect::transactional::{
    HasTransactionDomain, TransactionDomain, TransactionPolicy, TxSubscriptionRef,
};
use futures_executor::block_on;
use std::num::NonZeroUsize;
use std::sync::Arc;

struct Environment {
    transactions: Arc<TransactionDomain>,
}

impl HasTransactionDomain for Environment {
    fn transaction_domain(&self) -> Arc<TransactionDomain> {
        Arc::clone(&self.transactions)
    }
}

fn main() {
    let transactions = Arc::new(TransactionDomain::new(NonZeroUsize::new(1).unwrap()));
    let environment = Arc::new(Environment {
        transactions: Arc::clone(&transactions),
    });
    let state = TxSubscriptionRef::new(&transactions, 0usize).unwrap();
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::new(1).unwrap(),
    };

    let (_, subscription) = match block_on(state.subscribe().run(Arc::clone(&environment))) {
        Exit::Success(pair) => pair,
        _ => panic!("unable to subscribe"),
    };
    let update = state.set::<()>(Arc::new(1)).atomically(policy);
    assert!(matches!(
        block_on(update.run(Arc::clone(&environment))),
        Exit::Success(())
    ));
    assert!(
        matches!(block_on(subscription.next().run(Arc::clone(&environment))), Exit::Success(value) if *value == 1)
    );

    // The same state can be exposed through the runtime-neutral reactive API.
    let source: &dyn ReactiveStateSource<
        Environment,
        Snapshot = Arc<usize>,
        ReadError = effect::transactional::TxSubscriptionError,
        StreamError = effect::transactional::TxSubscriptionError,
    > = &state;
    assert!(
        matches!(block_on(source.read().run(environment)), Exit::Success(value) if *value == 1)
    );
}
