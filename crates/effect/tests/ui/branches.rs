use mandrill_effect::{Effect, effect};
struct Message(String);
fn main() {
    let _program: Effect<Message, String> = effect! {
        let value = Effect::sync(|| String::from("owned"))?;
        let size = match value { text if text.len() > 2 => Effect::succeed(text.len())?, text => text.len() };
        if size > 0 { let name = Effect::sync(|| String::from("yes"))?; Message(name) }
        else { Message(String::from("no")) }
    };
}
