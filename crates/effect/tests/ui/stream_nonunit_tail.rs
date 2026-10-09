//! Stream bodies complete with unit rather than a success payload.
use mandrill_effect::{Stream, stream};
/// Values are emitted through yield and cannot be hidden in the completion tail.
fn main() {
    let _: Stream<i64, (), ()> = stream! {yield 1;2i64};
}
