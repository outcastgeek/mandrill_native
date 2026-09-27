use mandrill_effect::Effect;
fn main() { let _ = Effect::<(), ()>::new(|_, _, _| panic!("inaccessible")); }
