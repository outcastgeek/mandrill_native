use super::tasks::{Environment, Marker, Payload};
use fx::{effect_for_each_concurrent, effect_future, effect_race, effect_sync};
use fx::{Effect, Exit};
use std::cell::Cell;
use std::num::NonZeroUsize;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
struct Length(Cell<usize>); // Neither Clone nor Sync.
struct Input(String); // Shared immutable input, without Clone.

pub async fn parallel_cases() {
    let env = Arc::new(Environment::default());
    let descriptions: Arc<[_]> = vec![
        {
            let __dsl_program: Effect<Payload, Payload, Environment> =
                effect_sync! { body: || Payload(Cell::new(Marker::Value)), };
            __dsl_program
        },
        effect_sync! { body: || Payload(Cell::new(Marker::Value)), },
    ]
    .into();
    let count = descriptions.len();
    let all = Effect::all_concurrent_shared(NonZeroUsize::MIN, descriptions);
    for _ in 0..2 {
        // Repeat runs create fresh payloads while sharing descriptions.
        assert_eq!(
            all.run(Arc::clone(&env)).await,
            Exit::Success(
                (0..count)
                    .map(|_| Payload(Cell::new(Marker::Value)))
                    .collect()
            )
        );
    }
    // NonZeroUsize::MIN is explicitly one worker; each of the two runs submits one.
    assert_eq!(env.tasks.submitted(), 2);
    env.tasks.finish();
    let inputs: Arc<[Input]> =
        vec![Input(String::from("first")), Input(String::from("second"))].into();
    let addresses: Vec<_> = inputs
        .iter()
        .map(|input| std::ptr::from_ref(input) as usize)
        .collect();
    let expected: Vec<_> = inputs.iter().map(|input| input.0.len()).collect();
    let shared = Arc::clone(&inputs);
    let values: Effect<Vec<Length>, Payload, Environment> = effect_for_each_concurrent! { shared; concurrency: NonZeroUsize::MIN, values: shared, body: move |input| {
        assert!(addresses.contains(&(std::ptr::from_ref(input) as usize)));
        let length = input.0.len();
        effect_sync! { body: move || Length(Cell::new(length)), }
    }, };
    assert_eq!(
        values.run(Arc::clone(&env)).await,
        Exit::Success(
            expected
                .iter()
                .map(|length| Length(Cell::new(*length)))
                .collect()
        )
    );
    let left = {
        let __dsl_program: Effect<Payload, Payload, Environment> =
            effect_sync! { body: || Payload(Cell::new(Marker::Value)), };
        __dsl_program
    };
    let right = effect_future! { body: |token| async move {
        token.cancelled().await;
        Payload(Cell::new(Marker::Value))
    }, };
    assert_eq!(
        effect_race! { program: left, right: right, }
            .run(Arc::clone(&env))
            .await,
        Exit::Success(Payload(Cell::new(Marker::Value)))
    );
    env.tasks.finish();
}
