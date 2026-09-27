use mandrill_effect::{AsyncQueue, Effect, QueueCapacity, QueueError, QueueShutdown};
use std::cell::Cell;

struct Data(Cell<usize>); // Neither Clone nor Sync.
const RENDEZVOUS: QueueCapacity = QueueCapacity::from_limit(Some(0));

fn main() {
    let queue = AsyncQueue::new(RENDEZVOUS);
    let _: Effect<(), QueueError> = queue.offer(Data(Cell::new(0)));
    let _: Effect<(), QueueError> = queue.offer_with(|| Data(Cell::new(1)));
    let _: Effect<Data, QueueError> = queue.take();
    let _: Effect<usize, QueueError> = queue.size();
    let _: Effect<(), QueueError> = queue.shutdown(QueueShutdown::Drain);
    let _: Effect<AsyncQueue<Data>, QueueError> = AsyncQueue::make(QueueCapacity::Unbounded);
    let _: &dyn std::error::Error = &QueueError::Shutdown;
}
