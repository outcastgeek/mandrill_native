use mandrill_effect::{AsyncQueue, Effect, QueueCapacity, QueueError};
use std::cell::Cell;

struct Data(Cell<usize>);
fn main() {
    let queue = AsyncQueue::new(QueueCapacity::Unbounded);
    let _: Effect<(), QueueError> = queue.offer_cloned(Data(Cell::new(0)));
}
