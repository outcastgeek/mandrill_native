use mandrill_effect::{Effect, Release, ResourceBinding, effect};
use std::sync::Arc;

struct Native(String);

fn main() {
    let program: Effect<String, String> = effect! {
        let handle = ResourceBinding::new(
            Effect::sync(|| Native(String::from("resource"))),
            |native, _reason| Release::try_future(move |_token| async move {
                drop(native);
                Ok(())
            }),
        )?;
        let contents = handle.access(|native| native.0.clone())?;
        contents
    };
    let (_driver, _observer) = program.owned_run(Arc::new(()));
}
