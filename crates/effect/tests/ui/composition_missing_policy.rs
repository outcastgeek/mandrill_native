//! Scoped cache admission has no generated policy defaults.
use mandrill_effect::{Effect, Scope, cache};
/// Missing options receive a descriptor diagnostic before type checking.
fn main() {
    let _ = cache! {scope:Scope::<()>::new(),config:(),load |key|{Effect::<(),()>::sync(||())?}};
}
