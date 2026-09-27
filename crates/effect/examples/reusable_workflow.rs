use effect::{Effect, Exit, effect};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, PartialEq, Eq)]
struct Document(String);

fn main() {
    let sequence = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&sequence);
    let program: Effect<Document, ()> = effect! {
        let witness = Arc::clone(&witness);
        let document = Effect::sync(move || {
            Document(witness.fetch_add(1, Ordering::SeqCst).to_string())
        })?;
        let suffix = Effect::sync(|| String::from("!"))?;
        Document(document.0 + &suffix)
    };
    assert_eq!(sequence.load(Ordering::SeqCst), 0);
    assert_eq!(
        futures_executor::block_on(program.run(Arc::new(()))),
        Exit::Success(Document(String::from("0!")))
    );
    assert_eq!(
        futures_executor::block_on(program.run(Arc::new(()))),
        Exit::Success(Document(String::from("1!")))
    );
}
