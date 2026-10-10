//! Finite V05 native service fixtures for checked inputs, HTTP policy and owned body settlement.

use crate::*;
use axum::body::Body;
use effect::{Cancellation, Effect, Exit, Scope};
use http_body_util::BodyExt;
use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

/// Explicit fixture bounds; byte/count limits are not hidden production defaults.
fn bounds() -> FieldBounds {
    FieldBounds {
        count: NonZeroUsize::new(2).expect("count"),
        length: NonZeroUsize::new(16).expect("length"),
        unit: TextUnit::Bytes,
    }
}
/// Construct a real native request with its own cancellation/body owner.
fn request(method: http::Method, uri: &str, headers: &[(&str, &str)], body: Body) -> Request {
    let mut native = http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        native = native.header(*name, *value);
    }
    Request::from_native(
        native.body(body).expect("fixture request"),
        Cancellation::new(),
    )
}
/// Typed fields reject duplicates, malformed percent escapes and incompatible body owners.
#[tokio::test]
async fn typed_input_and_native_escape_ownership() {
    let scalar = Scalar::new(
        |raw| {
            raw.parse::<u64>()
                .map_err(|_| ServerError::Invalid("number".into()))
        },
        |value: &u64| Ok(value.to_string()),
    );
    let field = Field::new(FieldSource::Query, "id", bounds(), scalar);
    let repeated = request(http::Method::GET, "/?id=7&id=8", &[], Body::empty());
    assert!(matches!(
        field.required(&repeated, None),
        Err(ServerError::Duplicate(_))
    ));
    let malformed = request(http::Method::GET, "/?id=%GG", &[], Body::empty());
    assert!(field.required(&malformed, None).is_err());
    let native = request(http::Method::POST, "/", &[], Body::from("coffee"));
    let moved = native
        .with_native_body(|body| async move {
            body.collect()
                .await
                .map(|bytes| bytes.to_bytes())
                .map_err(|error| ServerError::Transport(effect::Defect::new(error.to_string())))
        })
        .await
        .expect("native body");
    assert_eq!(moved, bytes::Bytes::from_static(b"coffee"));
    assert!(matches!(
        native
            .read_bytes(BodyLimit::new(NonZeroUsize::new(16).expect("limit")))
            .await,
        Err(ServerError::BodyClaimed)
    ));
    let limited = request(http::Method::POST, "/", &[], Body::from("coffee"));
    assert!(matches!(
        limited
            .read_bytes(BodyLimit::new(NonZeroUsize::new(2).expect("limit")))
            .await,
        Err(ServerError::Limit(_))
    ));
}
/// Negotiation exclusions and conditional HEAD avoid invoking lazy renderers.
#[tokio::test]
async fn negotiation_and_preconditions_before_rendering() {
    let calls = Arc::new(AtomicUsize::new(0));
    let rendered = calls.clone();
    let head = request(
        http::Method::HEAD,
        "/",
        &[("accept", "text/plain")],
        Body::empty(),
    );
    let env = Arc::new(RequestEnv {
        services: Arc::new(()),
        request: head.clone(),
    });
    let preparing = Scope::scoped(move |scope| {
        let rendered = rendered.clone();
        let choices:Representations<String,ServerError,RequestEnv<()>> = representations! {text(move |value:String| {let rendered=rendered.clone();Effect::sync_once(move || {rendered.fetch_add(1,Ordering::SeqCst);value})});}.expect("alternatives");
        Reply::negotiate(choices, String::from("latte")).prepare(
            head.clone(),
            scope,
            BodyPolicy {
                frames: NonZeroUsize::MIN,
                chunk_bytes: NonZeroUsize::new(64).expect("chunk"),
            },
            Arc::new(|error| error),
        )
    });
    let prepared = preparing.run(env).await;
    let Exit::Success(prepared) = prepared else {
        panic!("prepare HEAD");
    };
    assert_eq!(prepared.response.status(), http::StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let validators = ResourceValidators {
        exists: true,
        etag: Some("\"coffee\"".parse().expect("ETag")),
        last_modified: None,
    };
    let not_modified = request(
        http::Method::GET,
        "/",
        &[("if-none-match", "W/\"coffee\"")],
        Body::empty(),
    );
    assert_eq!(
        validators.evaluate(&not_modified).expect("precondition"),
        ConditionalOutcome::NotModified
    );
    let rejected = request(
        http::Method::PUT,
        "/",
        &[("if-match", "\"old\"")],
        Body::empty(),
    );
    assert_eq!(
        validators.evaluate(&rejected).expect("precondition"),
        ConditionalOutcome::PreconditionFailed
    );
    let get = request(
        http::Method::GET,
        "/",
        &[("accept", "text/plain;q=0, */*;q=1")],
        Body::empty(),
    );
    let env = Arc::new(RequestEnv {
        services: Arc::new(()),
        request: get.clone(),
    });
    let preparing = Scope::scoped(move |scope| {
        let excluded: Representations<String, ServerError, RequestEnv<()>> =
            representations! {text(|value:String|Effect::from_result_once(Ok(value)));}
                .expect("representation");
        Reply::negotiate(excluded, String::from("latte")).prepare(
            get.clone(),
            scope,
            BodyPolicy {
                frames: NonZeroUsize::MIN,
                chunk_bytes: NonZeroUsize::new(64).expect("chunk"),
            },
            Arc::new(|error| error),
        )
    });
    let prepared = preparing.run(env).await;
    assert!(
        matches!(prepared,Exit::Success(ref prepared) if prepared.response.status()==http::StatusCode::NOT_ACCEPTABLE)
    );
}
/// A writer cannot advance before pull; dropping its committed native body signals disconnect.
#[tokio::test]
async fn first_byte_backpressure_and_disconnect() {
    let request = request(http::Method::GET, "/", &[], Body::empty());
    let (mut body, writer, completion) = crate::body::channel::<ServerError, ()>(
        &request,
        BodyPolicy {
            frames: NonZeroUsize::MIN,
            chunk_bytes: NonZeroUsize::new(16).expect("chunk"),
        },
        Arc::new(|error| error),
    );
    completion.arm();
    let cleaned = Arc::new(AtomicUsize::new(0));
    let release = cleaned.clone();
    let settled = completion.clone();
    let owned = Scope::scoped(move |scope| {
        let release = release.clone();
        let writer = writer.clone();
        let settled = settled.clone();
        scope
            .add_release(move || {
                let release = release.clone();
                effect::Release::sync(move || {
                    release.fetch_add(1, Ordering::SeqCst);
                })
            })
            .flat_map(move |()| {
                let settled = settled.clone();
                writer
                    .write(bytes::Bytes::from_static(b"ready"))
                    .flat_map(move |()| {
                        let settled = settled.clone();
                        crate::endpoint::native_once(move |_, _| async move {
                            match settled.wait().await {
                                BodyOutcome::Completed => Exit::Success(()),
                                _ => Exit::Fail(ServerError::Disconnected),
                            }
                        })
                    })
            })
    });
    let writing = owned.run(Arc::new(()));
    tokio::pin!(writing);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut writing)
            .await
            .is_err()
    );
    let frame = body
        .frame()
        .await
        .expect("first frame")
        .expect("native frame");
    assert_eq!(
        frame.into_data().expect("data"),
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
}
/// SSE framing and resume cursor limits use the same explicit native field policy.
#[test]
fn sse_bounds_and_cursor() {
    let policy = SsePolicy {
        frame_bytes: NonZeroUsize::new(128).expect("frame"),
        cursor_bytes: NonZeroUsize::new(8).expect("cursor"),
    };
    let frame = SseRecord {
        event: Some("ready".into()),
        id: Some("42".into()),
        retry: None,
        data: "latte\nready".into(),
    }
    .frame(policy)
    .expect("SSE frame");
    assert_eq!(
        &frame[..],
        b"event: ready\nid: 42\ndata: latte\ndata: ready\n\n"
    );
    let resume = request(
        http::Method::GET,
        "/",
        &[("last-event-id", "42")],
        Body::empty(),
    );
    assert_eq!(
        sse_cursor(&resume, policy).expect("cursor"),
        Some("42".into())
    );
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
}
