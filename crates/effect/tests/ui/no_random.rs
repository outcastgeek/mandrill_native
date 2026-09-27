use mandrill_effect::{Effect,Schedule,NonnegativeFactor,TestClock};
fn main() {
 let factor=NonnegativeFactor::new(1.0).unwrap();
 let _:Effect<(),(),TestClock>=Effect::sync(||()).repeat(Schedule::recurs(1).jitter(factor,factor).unwrap());
}
