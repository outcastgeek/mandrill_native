use mandrill_effect::{Effect, effect};
fn main() {
    let _: Effect<usize, String> = effect! {
        let value = Effect::<usize, usize>::fail(7)?;
        value
    };
}
