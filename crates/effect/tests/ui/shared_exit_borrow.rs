use mandrill_effect::SharedExit;
fn escape(outcome: &SharedExit<String, ()>) -> &String {
    outcome.with(|exit| match exit {
        mandrill_effect::Exit::Success(value) => value,
        _ => panic!("fixture expected a value"),
    }).unwrap()
}
fn main() {}
