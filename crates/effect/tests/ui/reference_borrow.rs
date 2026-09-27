use mandrill_effect::{AtomicRef, Effect};

fn main() {
    let reference = AtomicRef::new(String::new());
    let _: Effect<&'static str, ()> = reference.with(|value| value.as_str());
}
