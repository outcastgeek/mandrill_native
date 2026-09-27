use mandrill_effect::{AtomicRef, Deferred, Effect, Exit, Latch};
use std::cell::Cell;

struct Data(Cell<usize>); // No Clone or Sync implementation.

fn main() {
    let reference = AtomicRef::new(Data(Cell::new(0)));
    let _: Effect<usize, Data> = reference.with(|value| value.0.get());
    let _: Effect<Data, Data> = reference.modify(|value| {
        (Data(Cell::new(value.0.get())), Data(Cell::new(1)))
    });
    let _: Effect<(), Data> = reference.modify_in_place(|value| value.0.set(2));
    let _: Effect<AtomicRef<Data>, Data> = AtomicRef::make_with(|| Data(Cell::new(0)));
    let deferred: Deferred<Data, Data> = Deferred::new();
    let _: Effect<bool, Data> = deferred.complete(Exit::Fail(Data(Cell::new(3))));
    let _: Effect<Data, Data> = deferred.await_value();
    let _: Effect<mandrill_effect::SharedExit<Data, Data>, Data> = deferred.await_shared();
    let latch = Latch::new();
    let _: Effect<bool, Data> = latch.open();
    let _: Effect<(), Data> = latch.await_open();
}
