//! Descriptor alternatives are mutually exclusive.
use mandrill_effect::{Effect, span};
/// Both body alternatives cannot be supplied at once.
fn main() {
    let _ = span! {name:"conflict",body{1i64},effect:Effect::<i64,()>::sync(||2)};
}
