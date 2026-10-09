//! Tracing capabilities remain a compiler-visible requirement.
use mandrill_effect::{Effect, span};
/// Environment-free code cannot silently install a tracer or clock.
fn main() {
    let _: Effect<i64, (), ()> = span! {name:"missing",body{1i64}};
}
