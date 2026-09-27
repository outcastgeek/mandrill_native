use mandrill_effect::{Effect, effect};

fn main() {
    let _program: Effect<usize, ()> = effect! {
        let Some(value) = Some(7) else { return 0; };
        value
    };
}
