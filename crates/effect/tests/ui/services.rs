use mandrill_effect::{Effect,Schema,Stream,Schedule,GrowthFactor,UnitInterval,LibraryDefect,SharedValue};
use std::cell::Cell;
struct Owned(Cell<usize>);
const GROWTH: Result<GrowthFactor,LibraryDefect>=GrowthFactor::new(2.0);
const SAMPLE: Result<UnitInterval,LibraryDefect>=UnitInterval::new(0.5);
fn main() {
 let schema=Schema::from_decoder(|_| Ok::<_,Owned>(Owned(Cell::new(1))));
 let _:Effect<Owned,Owned,()>=schema.parse("{}",|_|Owned(Cell::new(0)));
 let _:Effect<Vec<Owned>,Owned,()>=Stream::from_iterable(|| vec![Owned(Cell::new(1))].into_iter()).run_collect(None);
 let shared=SharedValue::new(Owned(Cell::new(1))); let _:usize=shared.with(|value|value.0.get()).unwrap();
 let _=(GROWTH,SAMPLE,Schedule::<Owned,usize>::recurs(2));
}
