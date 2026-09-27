use mandrill_effect::{AsyncQueue, QueueCapacity};

fn main() {
    let _: AsyncQueue<()> = AsyncQueue::new(QueueCapacity::from_limit(Some(-1)));
}
