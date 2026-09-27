use mandrill_effect::{AsyncQueue, QueueCapacity};

fn main() {
    let _: AsyncQueue<()> = AsyncQueue::new(QueueCapacity::Bounded(0));
}
