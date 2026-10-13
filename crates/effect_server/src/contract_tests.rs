//! Finite V05 native service fixtures for checked inputs, HTTP policy and owned body settlement.

use crate::*;
use axum::body::Body;
use effect::{Cancellation, Exit};
use effect::{
    effect, effect_finalizer, effect_resource, effect_scope, effect_sync, effect_try_sync,
};
use http_body_util::BodyExt;
use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::pin;

/// Explicit fixture bounds; byte/count limits are not hidden production defaults.
fn bounds() -> FieldBounds {
    FieldBounds {
        count: NonZeroUsize::MIN.saturating_add(1),
        length: NonZeroUsize::MIN.saturating_add(15),
        unit: TextUnit::Bytes,
    }
}
/// Construct a real native request with its own cancellation/body owner.
fn request(
    method: http::Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Body,
) -> Result<Request, http::Error> {
    let mut native = http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        native = native.header(*name, *value);
    }
    Ok(Request::from_native(
        native.body(body)?,
        Cancellation::new(),
    ))
}
/// Typed fields reject duplicates, malformed percent escapes and incompatible body owners.
#[tokio::test]
async fn typed_input_and_native_escape_ownership() -> Result<(), Box<dyn std::error::Error>> {
    let scalar = Scalar::new(
        |raw| {
            raw.parse::<u64>()
                .map_err(|_| ServerError::Invalid("number".into()))
        },
        |value: &u64| Ok(value.to_string()),
    );
    let field = Field::new(FieldSource::Query, "id", bounds(), scalar);
    let repeated = request(http::Method::GET, "/?id=7&id=8", &[], Body::empty())?;
    assert!(matches!(
        field.required(&repeated, None),
        Err(ServerError::Duplicate(_))
    ));
    let malformed = request(http::Method::GET, "/?id=%GG", &[], Body::empty())?;
    assert!(field.required(&malformed, None).is_err());
    let native = request(http::Method::POST, "/", &[], Body::from("coffee"))?;
    let moved = native
        .with_native_body(|body| async move {
            body.collect()
                .await
                .map(|bytes| bytes.to_bytes())
                .map_err(|error| ServerError::Transport(effect::Defect::new(error.to_string())))
        })
        .await?;
    assert_eq!(moved, bytes::Bytes::from_static(b"coffee"));
    assert!(matches!(
        native
            .read_bytes(BodyLimit::new(NonZeroUsize::MIN.saturating_add(15)))
            .await,
        Err(ServerError::BodyClaimed)
    ));
    let limited = request(http::Method::POST, "/", &[], Body::from("coffee"))?;
    assert!(matches!(
        limited
            .read_bytes(BodyLimit::new(NonZeroUsize::MIN.saturating_add(1)))
            .await,
        Err(ServerError::Limit(_))
    ));
    Ok(())
}
/// Negotiation exclusions and conditional HEAD avoid invoking lazy renderers.
#[tokio::test]
async fn negotiation_and_preconditions_before_rendering() -> Result<(), Box<dyn std::error::Error>>
{
    let calls = Arc::new(AtomicUsize::new(0));
    let rendered = calls.clone();
    let head = request(
        http::Method::HEAD,
        "/",
        &[("accept", "text/plain")],
        Body::empty(),
    )?;
    let env = Arc::new(RequestEnv {
        services: Arc::new(()),
        request: head.clone(),
    });
    let preparing = effect_scope! {
    body: move |scope| {
            let rendered = rendered.clone();
            let choices: Representations<String, ServerError, RequestEnv<()>> = match representations! {text(move |value:String| {let rendered=rendered.clone();effect_sync! { once; body: move || {rendered.fetch_add(1,Ordering::SeqCst);value}, }});}
            {
                Ok(choices) => choices,
                Err(error) => return effect_try_sync! { once; body: { let __dsl_result = Err(error); move || __dsl_result }, on_error: ::core::convert::identity, },
            };
            reply! { negotiate(choices, String::from("latte")); }.prepare(
                head.clone(),
                scope,
                BodyPolicy {
                    frames: NonZeroUsize::MIN,
                    chunk_bytes: NonZeroUsize::MIN.saturating_add(63),
                },
                Arc::new(|error| error),
            )
        },
    };
    let prepared = preparing.run(env).await;
    let Exit::Success(prepared) = prepared else {
        return Err("HEAD preparation must succeed".into());
    };
    assert_eq!(prepared.response.status(), http::StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let validators = ResourceValidators {
        exists: true,
        etag: Some("\"coffee\"".parse()?),
        last_modified: None,
    };
    let not_modified = request(
        http::Method::GET,
        "/",
        &[("if-none-match", "W/\"coffee\"")],
        Body::empty(),
    )?;
    assert_eq!(
        validators.evaluate(&not_modified)?,
        ConditionalOutcome::NotModified
    );
    let rejected = request(
        http::Method::PUT,
        "/",
        &[("if-match", "\"old\"")],
        Body::empty(),
    )?;
    assert_eq!(
        validators.evaluate(&rejected)?,
        ConditionalOutcome::PreconditionFailed
    );
    let get = request(
        http::Method::GET,
        "/",
        &[("accept", "text/plain;q=0, */*;q=1")],
        Body::empty(),
    )?;
    let env = Arc::new(RequestEnv {
        services: Arc::new(()),
        request: get.clone(),
    });
    let preparing = effect_scope! {
    body: move |scope| {
            let excluded: Representations<String, ServerError, RequestEnv<()>> = match representations! {text(|value:String|effect_try_sync! { once; body: { let __dsl_result = Ok(value); move || __dsl_result }, on_error: ::core::convert::identity, });}
            {
                Ok(excluded) => excluded,
                Err(error) => return effect_try_sync! { once; body: { let __dsl_result = Err(error); move || __dsl_result }, on_error: ::core::convert::identity, },
            };
            reply! { negotiate(excluded, String::from("latte")); }.prepare(
                get.clone(),
                scope,
                BodyPolicy {
                    frames: NonZeroUsize::MIN,
                    chunk_bytes: NonZeroUsize::MIN.saturating_add(63),
                },
                Arc::new(|error| error),
            )
        },
    };
    let prepared = preparing.run(env).await;
    assert!(
        matches!(prepared,Exit::Success(ref prepared) if prepared.response.status()==http::StatusCode::NOT_ACCEPTABLE)
    );
    Ok(())
}
/// A writer cannot advance before pull; dropping its committed native body signals disconnect.
#[tokio::test]
async fn first_byte_backpressure_and_disconnect() -> Result<(), Box<dyn std::error::Error>> {
    let request = request(http::Method::GET, "/", &[], Body::empty())?;
    let (mut body, writer, completion) = crate::body::channel::<ServerError, ()>(
        &request,
        BodyPolicy {
            frames: NonZeroUsize::MIN,
            chunk_bytes: NonZeroUsize::MIN.saturating_add(15),
        },
        Arc::new(|error| error),
    );
    completion.arm();
    let cleaned = Arc::new(AtomicUsize::new(0));
    let release = cleaned.clone();
    let settled = completion.clone();
    let owned = effect_scope! {
    body: move |scope| {
            let release = release.clone();
            let writer = writer.clone();
            let settled = settled.clone();
            effect! {
     bind;
     program: effect_finalizer! { release; scope: scope, release: move || {
                    let release = release.clone();
                    effect_resource! { release_sync; body: move || {
                        release.fetch_add(1, Ordering::SeqCst);
                    }, }
                }, },
     body: move |()| {
                    let settled = settled.clone();
                    effect! {
     bind;
     program: writer
                        .write(bytes::Bytes::from_static(b"ready")),
     body: move |()| {
                            let settled = settled.clone();
                            crate::endpoint::native_once(move |_, _| async move {
                                match settled.wait().await {
                                    BodyOutcome::Completed => Exit::Success(()),
                                    BodyOutcome::Open
                                    | BodyOutcome::Disconnected
                                    | BodyOutcome::Failed => Exit::Fail(ServerError::Disconnected),
                                }
                            })
                        },
     }
                },
     }
        },
    };
    let writing = owned.run(Arc::new(()));
    pin!(writing);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut writing)
            .await
            .is_err()
    );
    let frame = body.frame().await.ok_or("missing first frame")??;
    assert_eq!(
        frame.into_data().map_err(|_| "expected a data frame")?,
        bytes::Bytes::from_static(b"ready")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut writing)
            .await
            .is_err()
    );
    assert_eq!(cleaned.load(Ordering::SeqCst), 0);
    assert_eq!(completion.outcome(), BodyOutcome::Open);
    drop(body);
    assert!(matches!(
        writing.await,
        Exit::Fail(ServerError::Disconnected)
    ));
    assert_eq!(completion.wait().await, BodyOutcome::Disconnected);
    assert!(request.cancellation().is_requested());
    assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    Ok(())
}
/// SSE framing and resume cursor limits use the same explicit native field policy.
#[test]
fn sse_bounds_and_cursor() -> Result<(), Box<dyn std::error::Error>> {
    let policy = SsePolicy {
        frame_bytes: NonZeroUsize::MIN.saturating_add(127),
        cursor_bytes: NonZeroUsize::MIN.saturating_add(7),
    };
    let frame = SseRecord {
        event: Some("ready".into()),
        id: Some("42".into()),
        retry: None,
        data: "latte\nready".into(),
    }
    .frame(policy)?;
    assert_eq!(
        &frame[..],
        b"event: ready\nid: 42\ndata: latte\ndata: ready\n\n"
    );
    let resume = request(
        http::Method::GET,
        "/",
        &[("last-event-id", "42")],
        Body::empty(),
    )?;
    assert_eq!(sse_cursor(&resume, policy)?, Some("42".into()));
    assert!(
        SseRecord {
            event: None,
            id: Some("bad\ncursor".into()),
            retry: None,
            data: String::new()
        }
        .frame(policy)
        .is_err()
    );
    Ok(())
}
