use mandrill_effect::Effect;
use std::num::NonZeroUsize;
fn main() {
    let _ = Effect::<usize, (), ()>::all_concurrent(NonZeroUsize::MIN, Vec::new());
}
