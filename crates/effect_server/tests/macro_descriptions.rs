//! Extracted native writer typing and reuse of existing endpoint/pipeline descriptions.

use effect::effect_sync;
use effect_server::{BodyProgram, Endpoint, HttpMethodSet, Pipeline, Reply, Router, ServerError};
use effect_server::{endpoint, pipeline, registered, reply, router, stream};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
/// A writer callback infers its native handle without starting a producer.
fn extracted_writer_and_existing_descriptors_remain_inert() {
    let _: BodyProgram<ServerError, ()> = stream! {
        body: |writer| {
            let writer = writer.clone();
            writer.write(effect_server::native::bytes::Bytes::from_static(b"ready"))
        },
    };
    let visits = AtomicUsize::new(0);
    let endpoint: Endpoint<ServerError, ()> = endpoint! {
        effect(effect_sync! {
            body: || reply! {
                empty(),
            },
        }),
    };
    let pipeline: Pipeline<ServerError, ()> = pipeline! {};
    let pipeline = pipeline! {
        source: { visits.fetch_add(1, Ordering::SeqCst); pipeline },
    };
    let _: Endpoint<ServerError, ()> = endpoint! {
        source: { visits.fetch_add(1, Ordering::SeqCst); endpoint },
        pipe_through(pipeline),
    };
    assert_eq!(visits.load(Ordering::SeqCst), 2);
}

#[test]
/// Route assembly borrows cloned methods and evaluates each reply source once.
fn route_collections_keep_reusable_methods_and_native_reply_sources() -> Result<(), ServerError> {
    let methods = HttpMethodSet::new(vec![effect_server::native::http::Method::GET])?;
    let mut routes: Router<ServerError, ()> = router! {}?;
    for path in ["/first", "/second"] {
        routes = router! {
            source: routes,
            methods(methods.clone(), path, registered! {
                endpoint(endpoint! {
                    body: |_context| async {
                        Ok(reply! { empty() })
                    },
                }),
            }),
        }?;
    }
    let visits = AtomicUsize::new(0);
    let reply: Reply<ServerError, ()> = reply! {
        source({ visits.fetch_add(1, Ordering::SeqCst); reply! { empty() } }),
    };
    let finite = reply.into_finite(std::num::NonZeroUsize::MIN)?;
    let _: Reply<ServerError, ()> = reply! {
        finite({ visits.fetch_add(1, Ordering::SeqCst); finite }),
    };
    assert_eq!(visits.load(Ordering::SeqCst), 2);
    Ok(())
}
