//! Finite V05 real transport fixtures reuse the runnable café's provider and DSL assembly.
//! HTTP/2 needs the test-only `hyper/client` feature; no second server or Effect runtime exists.

#![cfg(any(feature = "http1", feature = "http2"))]

use effect::{effect, effect_map_error, effect_sync, effect_try_future, effect_try_sync};
use effect_server::{router, server};
#[path = "../examples/serve_cafe_orders.rs"]
pub mod cafe;
use effect::Exit;
#[cfg(feature = "http1")]
use effect_server::RequestCompletion;
use effect_server::{BodyPolicy, HostPolicy, HttpProtocols};
#[cfg(feature = "http1")]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{num::NonZeroUsize, sync::Arc, time::Duration};

/// Caller-owned fixture policies bound these short responses and the test's graceful shutdown.
fn policies() -> (BodyPolicy, HostPolicy) {
    (
        BodyPolicy {
            frames: NonZeroUsize::MIN,
            chunk_bytes: NonZeroUsize::MIN.saturating_add(127),
        },
        HostPolicy {
            drain: Duration::from_secs(2),
        },
    )
}
/// Each fixture owns one native runtime and SDK originals, reusing the café's existing providers.
/// All scheduled/blocking work drains before those caller-owned native providers shut down.
fn with_cafe<F: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>>(
    test: impl FnOnce(Arc<cafe::Cafe>) -> F,
) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?,
    );
    let traces = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let metrics = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
    let logs = opentelemetry_sdk::logs::SdkLoggerProvider::builder().build();
    let providers = effect_otel::BorrowedProviders::new(&traces, &metrics, &logs);
    // Request/load spans can overlap during cleanup; these fixtures register no metrics.
    let capacity = cafe::cafe_telemetry::TelemetryCapacity {
        active_spans: NonZeroUsize::MIN.saturating_add(7),
        metric_descriptors: NonZeroUsize::MIN,
        series_per_descriptor: NonZeroUsize::MIN,
    };
    let root = cafe::environment(
        runtime.clone(),
        providers,
        cafe::order_policy(NonZeroUsize::MIN.saturating_add(127)),
        capacity,
    )?;
    let outcome = runtime.block_on(test(root.clone()));
    runtime.block_on(cafe::drain_providers(&root));
    let logs_closed = logs.shutdown();
    let metrics_closed = metrics.shutdown();
    let traces_closed = traces.shutdown();
    outcome?;
    logs_closed?;
    metrics_closed?;
    traces_closed?;
    Ok(())
}
/// Actual HTTP/1 requests cover authorization/error scope, HEAD/ETag, native 404/405 and wrappers.
#[cfg(feature = "http1")]
#[test]
fn http1_policy_and_native_completion() -> Result<(), Box<dyn std::error::Error>> {
    with_cafe(|root| async move {
        let (body, host) = policies();
        let mut server = cafe::describe(HttpProtocols::Http1, body, host)?;
        let completed = Arc::new(AtomicUsize::new(0));
        let observe = completed.clone();
        server.policy = server.policy.clone().on_exit(
            move |_, _, completion: &RequestCompletion<cafe::CafeError>| {
                assert!(completion.committed);
                assert_eq!(completion.body, effect_server::BodyOutcome::Completed);
                let observe = observe.clone();
                effect_sync! { once; body: move || {
                    observe.fetch_add(1, Ordering::SeqCst);
                }, }
            },
        );
        server = server! { source: server, after_response: |_, mut response| {
            response.headers_mut().insert("x-outer", http::HeaderValue::from_static("owned"));
            effect_try_sync! { once; body: move || Ok(response), on_error: ::core::convert::identity, }
        }, };
        server = server! { source: server, before_routing: |mut routing, _| {
            if routing.parts().headers.contains_key("x-demo-get") {
                routing.parts_mut().method = http::Method::GET;
            }
            effect_try_sync! { once; body: move || Ok(routing), on_error: ::core::convert::identity, }
        }, };
        let execution=server.with_host(|host|effect_try_future! { once; body: move |_|async move {
        let cases=[
            ("GET /orders/42","", "401"),
            ("GET /orders/42","Authorization: Bearer cafe-demo-customer-7\r\nAccept: text/html\r\n","200"),
            ("HEAD /orders/42","Authorization: Bearer cafe-demo-customer-7\r\n","200"),
            ("GET /orders/42","Authorization: Bearer cafe-demo-customer-7\r\nIf-None-Match: W/\"order-42\"\r\n","304"),
            ("GET /missing","","404"),
            ("POST /health","","405"),
            ("POST /health","x-demo-get: yes\r\n","200"),
        ];
        for (line,fields,status) in cases {let received=cafe::exchange(host.address,&format!("{line} HTTP/1.1\r\nHost: localhost\r\n{fields}Connection: close\r\n\r\n")).await?;assert!(received.starts_with(&format!("HTTP/1.1 {status}")),"{received}");assert!(received.to_ascii_lowercase().contains("x-outer: owned"));if line.starts_with("HEAD") {assert!(received.ends_with("\r\n\r\n"));}}
        Ok::<_,effect_server::ServerError>(())
    }, on_error: cafe::CafeError::Http, });
        let exit = execution.run(root).await;
        assert!(
            matches!(&exit, Exit::Success(())),
            "retained host exit: {exit:?}"
        );
        assert_eq!(completed.load(Ordering::SeqCst), 7);
        Ok(())
    })
}
/// The official native HTTP/2 client exercises the selected HTTP/2-only serving stack.
#[cfg(feature = "http2")]
#[test]
fn http2_request_and_drain() -> Result<(), Box<dyn std::error::Error>> {
    with_cafe(|root| async move {
        let (body, host) = policies();
        let server = cafe::describe(HttpProtocols::Http2, body, host)?;
        let execution = server.with_host(|host| {
            effect_try_future! { once; body: move |_| async move {
                let socket = tokio::net::TcpStream::connect(host.address)
                    .await
                    .map_err(effect_server::ServerError::Io)?;
                let (mut client, connection) = hyper::client::conn::http2::handshake(
                    hyper_util::rt::TokioExecutor::new(),
                    hyper_util::rt::TokioIo::new(socket),
                )
                .await
                .map_err(|error| {
                    effect_server::ServerError::Transport(effect::Defect::new(
                        error.to_string(),
                    ))
                })?;
                let driver = tokio::spawn(connection);
                let request = http::Request::builder()
                    .uri(format!("http://{}/health", host.address))
                    .body(http_body_util::Empty::<bytes::Bytes>::new())
                    .map_err(|error| effect_server::ServerError::Invalid(error.to_string()))?;
                let response = client.send_request(request).await.map_err(|error| {
                    effect_server::ServerError::Transport(effect::Defect::new(
                        error.to_string(),
                    ))
                })?;
                assert_eq!(response.version(), http::Version::HTTP_2);
                assert_eq!(response.status(), http::StatusCode::OK);
                let bytes = http_body_util::BodyExt::collect(response.into_body())
                    .await
                    .map_err(|error| {
                        effect_server::ServerError::Transport(effect::Defect::new(
                            error.to_string(),
                        ))
                    })?
                    .to_bytes();
                assert_eq!(&bytes[..], b"ready");
                drop(client);
                let _ = driver.await.map_err(|error| {
                    effect_server::ServerError::Transport(effect::Defect::new(
                        error.to_string(),
                    ))
                })?;
                Ok::<_, effect_server::ServerError>(())
            }, on_error: cafe::CafeError::Http, }
        });
        let exit = execution.run(root).await;
        assert!(
            matches!(&exit, Exit::Success(())),
            "retained host exit: {exit:?}"
        );
        Ok(())
    })
}

/// A native HTTP/1 upgrade retains the core duplex session through message and peer-close settlement.
/// This fixture does not claim support for an ingress's HTTP/2 extended-CONNECT configuration.
#[cfg(all(feature = "http1", feature = "websocket"))]
#[test]
fn websocket_native_session_lifecycle() -> Result<(), Box<dyn std::error::Error>> {
    with_cafe(|root| async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (body, host) = policies();
        let mut server = cafe::describe(HttpProtocols::Http1, body, host)?;
        let limits = effect::WebSocketLimits {
            max_message_bytes: NonZeroUsize::MIN.saturating_add(63),
            receive_buffer_bytes: NonZeroUsize::MIN.saturating_add(63),
            close_timeout: Duration::from_secs(1),
        };
        let options = effect_server::WebSocketOptions {
            read_buffer_bytes: NonZeroUsize::MIN.saturating_add(63),
            write_buffer_bytes: NonZeroUsize::MIN.saturating_add(63),
            max_write_buffer_bytes: NonZeroUsize::MIN.saturating_add(127),
            max_frame_bytes: NonZeroUsize::MIN.saturating_add(63),
            limits,
            protocols: Vec::new(),
            accept_unmasked_frames: false,
        };
        let policy = effect_server::WebSocketPolicy::new(
            options,
            |_| effect_sync! { body: || (), },
            |error| {
                cafe::CafeError::Http(effect_server::ServerError::Invalid(format!("{error:?}")))
            },
            cafe::CafeError::Http,
        )?;
        server.routes = router! { source: server.routes, websocket("/socket", policy, |socket|effect! {
            let socket=socket.clone();
            let receiving=effect_try_sync! { once; body: { let __dsl_result = socket.with(|connection|connection.receive::<cafe::Cafe>()).map_err(|error|cafe::CafeError::Http(effect_server::ServerError::Invalid(format!("live owner: {error:?}")))); move || __dsl_result }, on_error: ::core::convert::identity, }?;
            let received=effect_map_error! { program: receiving, map: |error|cafe::CafeError::Http(effect_server::ServerError::Invalid(format!("{error:?}"))), }?;
            if let effect::WebSocketReceive::Message(message)=received {
                let sending=effect_try_sync! { once; body: { let __dsl_result = socket.with(|connection|connection.send::<cafe::Cafe>(message)).map_err(|error|cafe::CafeError::Http(effect_server::ServerError::Invalid(format!("live owner: {error:?}")))); move || __dsl_result }, on_error: ::core::convert::identity, }?;
                effect_map_error! { program: sending, map: |error|cafe::CafeError::Http(effect_server::ServerError::Invalid(format!("{error:?}"))), }?;
            }
            let receiving=effect_try_sync! { once; body: { let __dsl_result = socket.with(|connection|connection.receive::<cafe::Cafe>()).map_err(|error|cafe::CafeError::Http(effect_server::ServerError::Invalid(format!("live owner: {error:?}")))); move || __dsl_result }, on_error: ::core::convert::identity, }?;
            let closed=effect_map_error! { program: receiving, map: |error|cafe::CafeError::Http(effect_server::ServerError::Invalid(format!("{error:?}"))), }?;
            assert!(matches!(closed,effect::WebSocketReceive::Closed(_)));
        }); }?;
        let execution=server.with_host(|host|effect_try_future! { once; body: move |_|async move {
        let mut socket=tokio::net::TcpStream::connect(host.address).await.map_err(effect_server::ServerError::Io)?;
        socket.write_all(b"GET /socket HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").await.map_err(effect_server::ServerError::Io)?;
        let mut headers=Vec::new();while !headers.ends_with(b"\r\n\r\n") {headers.push(socket.read_u8().await.map_err(effect_server::ServerError::Io)?);}assert!(headers.starts_with(b"HTTP/1.1 101"),"native upgrade response: {}",String::from_utf8_lossy(&headers));
        socket.write_all(&[0x81,0x85,1,2,3,4,b'r'^1,b'e'^2,b'a'^3,b'd'^4,b'y'^1]).await.map_err(effect_server::ServerError::Io)?;
        let mut frame=[0;7];socket.read_exact(&mut frame).await.map_err(effect_server::ServerError::Io)?;assert_eq!(&frame[..],b"\x81\x05ready");
        socket.write_all(&[0x88,0x82,1,2,3,4,0x03^1,0xe8^2]).await.map_err(effect_server::ServerError::Io)?;
        let mut closed=Vec::new();socket.read_to_end(&mut closed).await.map_err(effect_server::ServerError::Io)?;assert!(closed.starts_with(b"\x88"));Ok::<_,effect_server::ServerError>(())
    }, on_error: cafe::CafeError::Http, });
        let exit = execution.run(root).await;
        assert!(
            matches!(&exit, Exit::Success(())),
            "retained host exit: {exit:?}"
        );
        Ok(())
    })
}
