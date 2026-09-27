use mandrill_effect::{AtomicRef, Effect};
use std::cell::Cell;

struct Data(Cell<usize>);

fn main() {
    let reference = AtomicRef::new(Data(Cell::new(0)));
    let _: Effect<Data, ()> = reference.get();
}
