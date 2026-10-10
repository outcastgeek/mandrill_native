//! Isolated metrics-only native exporter listener under the existing Server host owner.
//!
//! Exporter exposition comes from the application's supported native Tower service,
//! not a Web text-format serializer. A separate loopback listener/router owns the
//! scrape surface; forging Host on the application listener does not reach it.

use effect::{Effect, HasCancellation, HasTaskSpawner};
use effect_server::{
    HasTokio, HttpProtocols, RequestBinding, Router, Server, ServerError, ServerHost, ServerPolicy,
};
use std::convert::Infallible;
use std::net::SocketAddr;

/// Explicit management exposure, validated against the native listener address.
#[derive(Clone, Copy)]
#[must_use]
pub struct LoopbackExposure(SocketAddr);
impl LoopbackExposure {
    /// Require actual loopback binding; port zero is allowed only when the caller
    /// deliberately requests OS allocation and observes ServerHost.address.
    pub fn new(address: SocketAddr) -> Result<Self, ServerError> {
        if address.ip().is_loopback() {
            Ok(Self(address))
        } else {
            Err(ServerError::Configuration(
                "metrics listener must be loopback".into(),
            ))
        }
    }
    /// Borrow the actual selected native bind address, not a Host-header filter.
    #[must_use]
    pub const fn address(self) -> SocketAddr {
        self.0
    }
}
/// Run a separate native metrics exporter listener in the existing Effect scope.
/// Only the supplied scrape path is mounted; application registration descriptions
/// cannot be passed to this function. Exporter provider lifetime stays caller-owned.
pub fn with_host<A, E, R, S>(
    policy: ServerPolicy<E, R>,
    exposure: LoopbackExposure,
    protocols: HttpProtocols,
    scrape_path: &str,
    exporter: S,
    use_host: impl Fn(ServerHost) -> Effect<A, E, R> + Send + Sync + 'static,
) -> Result<Effect<A, E, R>, ServerError>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static,
    S: tower::Service<
            http::Request<axum::body::Body>,
            Response = http::Response<axum::body::Body>,
            Error = Infallible,
        > + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    // Reuse Server's route validation rather than introducing a management matcher.
    let _validated = effect_server::Route::literal(scrape_path)?;
    let path = scrape_path.to_owned();
    let routes = Router::new().native(move |router| {
        Ok(router.route(&path, axum::routing::get_service(exporter.clone())))
    });
    let environment = RequestBinding::new(|_| {
        Effect::from_future_with_env(|environment, _| async move { environment })
    });
    Ok(Server::new(policy, environment, exposure.address(), protocols, routes).with_host(use_host))
}

/// Host the supported native Prometheus registry/encoder on a separate loopback
/// listener. The registry belongs to the caller's native OTel Prometheus reader or
/// native Prometheus collectors; Web installs no global registry/provider/exporter.
/// Encoding uses the official library and a caller-bounded native byte sink.
#[cfg(feature = "prometheus")]
pub fn with_prometheus_host<A, E, R>(
    policy: ServerPolicy<E, R>,
    exposure: LoopbackExposure,
    protocols: HttpProtocols,
    scrape_path: &str,
    registry: prometheus::Registry,
    maximum_bytes: std::num::NonZeroUsize,
    errors: impl Fn(prometheus::Error) -> E + Send + Sync + 'static,
    use_host: impl Fn(ServerHost) -> Effect<A, E, R> + Send + Sync + 'static,
) -> Result<Effect<A, E, R>, ServerError>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static,
{
    use prometheus::Encoder;
    let errors = std::sync::Arc::new(errors);
    let render = Effect::try_sync(
        move || {
            let encoder = prometheus::TextEncoder::new();
            let families = registry.gather();
            let mut output = BoundedMetrics {
                maximum: maximum_bytes.get(),
                bytes: Vec::new(),
            };
            encoder.encode(&families, &mut output)?;
            let media = http::HeaderValue::from_str(encoder.format_type())
                .map_err(|error| prometheus::Error::Msg(error.to_string()))?;
            Ok::<_, prometheus::Error>((bytes::Bytes::from(output.bytes), media))
        },
        move |error| errors(error),
    )
    .map(|(bytes, media)| {
        effect_server::ReplyDraft::new()
            .header(
                http::header::CACHE_CONTROL,
                http::HeaderValue::from_static("no-store"),
            )
            .finish(effect_server::Reply::bytes(bytes, media))
    });
    let routes = Router::new().registered(
        effect_server::HttpMethodSet::new(vec![http::Method::GET])?,
        scrape_path,
        effect_server::Registered::new(
            effect_server::Endpoint::from_effect(render),
            effect_server::Registration::default(),
        ),
    )?;
    let environment = RequestBinding::new(|_| {
        Effect::from_future_with_env(|environment, _| async move { environment })
    });
    Ok(Server::new(policy, environment, exposure.address(), protocols, routes).with_host(use_host))
}
/// Finite native encoding sink. It enforces caller policy while the official
/// Prometheus encoder supplies every wire-format rule; Web implements no serializer.
#[cfg(feature = "prometheus")]
struct BoundedMetrics {
    /// Explicit maximum encoded scrape response bytes.
    maximum: usize,
    /// Finite bytes supplied exclusively by the native encoder.
    bytes: Vec<u8>,
}
#[cfg(feature = "prometheus")]
impl std::io::Write for BoundedMetrics {
    /// Reject a write before allocation when the caller's finite byte bound is exceeded.
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|length| length > self.maximum)
        {
            return Err(std::io::Error::other("metrics encoded byte bound"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    /// The finite in-memory sink has no independent flush or provider lifecycle.
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
