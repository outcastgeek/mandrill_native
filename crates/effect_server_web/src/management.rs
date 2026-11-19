//! Isolated metrics-only native exporter listener under the existing Server host owner.
//!
//! Exporter exposition comes from the application's supported native Tower service,
//! not a Web text-format serializer. A separate loopback listener/router owns the
//! scrape surface; forging Host on the application listener does not reach it.

use crate::effect_future;
use effect::{Effect, HasCancellation, HasTaskSpawner};
use effect_server::{
    HasTokio, HttpProtocols, RequestBinding, ServerError, ServerHost, ServerPolicy,
};
use effect_server::{route, router, server};
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
    let _validated = route! { literal: scrape_path }?;
    let path = scrape_path.to_owned();
    let routes = router! {
        native: move |router| {
        Ok(router.route(&path, axum::routing::get_service(exporter.clone())))
    } };
    let environment = RequestBinding::new(|_| {
        effect_future! { with_env; body: |environment, _| async move { environment }, }
    });
    Ok(server! {
        policy: policy,
        environment: environment,
        listen: exposure.address(),
        protocols: protocols,
        routes: routes,
    }?
    .with_host(use_host))
}

#[cfg(feature = "prometheus")]
use effect::memory::{
    Buffer, BufferLease, BufferView, Bytes, MemoryError, PreparedMemory, RegionId, SlotPool,
};
#[cfg(feature = "prometheus")]
use std::sync::Arc;

/// Independent response owners for the isolated operational metrics listener.
#[cfg(feature = "prometheus")]
pub struct PreparedMetricsOutput<E> {
    buffers: SlotPool<'static, Buffer<'static>>,
    errors: Arc<dyn Fn(MemoryError) -> E + Send + Sync>,
}

#[cfg(feature = "prometheus")]
impl<E> PreparedMetricsOutput<E> {
    /// Checked encoded-output bytes and physical pool metadata.
    pub fn storage_bytes(scrapes: usize, output_bytes: Bytes) -> Result<Bytes, MemoryError> {
        SlotPool::<Buffer<'static>>::storage_bytes(scrapes)?
            .checked_add(output_bytes.checked_mul(scrapes)?)
    }

    /// Prepare actual response owners before any metrics gather runs.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        scrapes: usize,
        output_bytes: Bytes,
        errors: impl Fn(MemoryError) -> E + Send + Sync + 'static,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(scrapes, output_bytes)?;
        Ok(Self {
            buffers: memory.buffers(region, scrapes, output_bytes)?,
            errors: Arc::new(errors),
        })
    }
}

/// Host the supported native Prometheus registry/encoder on a separate loopback
/// listener. The registry belongs to the caller's native OTel Prometheus reader or
/// native Prometheus collectors; Web installs no global registry/provider/exporter.
/// Encoding uses the official library and a caller-bounded native byte sink.
/// `exporter` pairs the caller-owned registry with its maximum encoded response size.
#[cfg(feature = "prometheus")]
pub fn with_prometheus_host<A, E, R>(
    policy: ServerPolicy<E, R>,
    exposure: LoopbackExposure,
    protocols: HttpProtocols,
    scrape_path: &str,
    exporter: (prometheus::Registry, std::num::NonZeroUsize),
    errors: impl Fn(prometheus::Error) -> E + Send + Sync + 'static,
    use_host: impl Fn(ServerHost) -> Effect<A, E, R> + Send + Sync + 'static,
) -> Result<Effect<A, E, R>, ServerError>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static,
{
    let (registry, maximum_bytes) = exporter;
    prometheus_host(
        policy,
        exposure,
        protocols,
        scrape_path,
        (registry, maximum_bytes, None),
        errors,
        use_host,
    )
}

/// Host the same native exporter with its encoded response leased from an
/// independent prepared operational region. Native registry gather is unchanged.
#[cfg(feature = "prometheus")]
pub fn with_prepared_prometheus_host<A, E, R>(
    policy: ServerPolicy<E, R>,
    exposure: LoopbackExposure,
    protocols: HttpProtocols,
    scrape_path: &str,
    exporter: (
        prometheus::Registry,
        std::num::NonZeroUsize,
        Arc<PreparedMetricsOutput<E>>,
    ),
    errors: impl Fn(prometheus::Error) -> E + Send + Sync + 'static,
    use_host: impl Fn(ServerHost) -> Effect<A, E, R> + Send + Sync + 'static,
) -> Result<Effect<A, E, R>, ServerError>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static,
{
    let (registry, maximum_bytes, output) = exporter;
    prometheus_host(
        policy,
        exposure,
        protocols,
        scrape_path,
        (registry, maximum_bytes, Some(output)),
        errors,
        use_host,
    )
}

#[cfg(feature = "prometheus")]
fn prometheus_host<A, E, R>(
    policy: ServerPolicy<E, R>,
    exposure: LoopbackExposure,
    protocols: HttpProtocols,
    scrape_path: &str,
    exporter: (
        prometheus::Registry,
        std::num::NonZeroUsize,
        Option<Arc<PreparedMetricsOutput<E>>>,
    ),
    errors: impl Fn(prometheus::Error) -> E + Send + Sync + 'static,
    use_host: impl Fn(ServerHost) -> Effect<A, E, R> + Send + Sync + 'static,
) -> Result<Effect<A, E, R>, ServerError>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static,
{
    use crate::effect_try_sync;
    use effect_server::{endpoint, registered, reply};
    use prometheus::Encoder;
    let (registry, maximum_bytes, storage) = exporter;
    let render = effect_try_sync! {
        body: move || {
            let bytes = match &storage {
                Some(storage) => {
                    let mut output = storage.buffers.try_checkout().map_err(|error| (storage.errors)(error))?;
                    output.clear();
                    MetricsBytes::Prepared(output)
                }
                None => MetricsBytes::Ordinary(Vec::new()),
            };
            let mut output = BoundedMetrics { maximum: maximum_bytes.get(), bytes, failure: None };
            let encoder = prometheus::TextEncoder::new();
            let families = registry.gather();
            if let Err(error) = encoder.encode(&families, &mut output) {
                let failure = output.failure;
                drop(output);
                if let Some((storage, failure)) = storage.as_ref().zip(failure) {
                    return Err((storage.errors)(failure));
                }
                return Err(errors(error));
            }
            let media = http::HeaderValue::from_str(encoder.format_type())
                .map_err(|error| errors(prometheus::Error::Msg(error.to_string())))?;
            let source = match output.bytes {
                MetricsBytes::Prepared(buffer) => reply! { lease(BufferView::new(buffer.share()), media) },
                MetricsBytes::Ordinary(bytes) => reply! { bytes(bytes::Bytes::from(bytes), media) },
            };
            Ok(reply! {
                header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store")),
                source(source),
            })
        },
        on_error: ::core::convert::identity,
    };
    let routes = router! {
        methods(effect_server::HttpMethodSet::new(vec![http::Method::GET])?, scrape_path, registered! {
            endpoint(endpoint! { effect(render) }),
        }),
    }?;
    let environment = RequestBinding::new(|_| {
        effect_future! { with_env; body: |environment, _| async move { environment }, }
    });
    Ok(server! {
        policy: policy,
        environment: environment,
        listen: exposure.address(),
        protocols: protocols,
        routes: routes,
    }?
    .with_host(use_host))
}
#[cfg(feature = "prometheus")]
enum MetricsBytes {
    Ordinary(Vec<u8>),
    Prepared(BufferLease<'static>),
}

/// Finite native encoding sink. The official encoder supplies every wire rule.
#[cfg(feature = "prometheus")]
struct BoundedMetrics {
    maximum: usize,
    bytes: MetricsBytes,
    failure: Option<MemoryError>,
}
#[cfg(feature = "prometheus")]
impl std::io::Write for BoundedMetrics {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let used = match &self.bytes {
            MetricsBytes::Ordinary(bytes) => bytes.len(),
            MetricsBytes::Prepared(buffer) => buffer.len(),
        };
        if used
            .checked_add(bytes.len())
            .is_none_or(|length| length > self.maximum)
        {
            return Err(std::io::Error::other("metrics encoded byte bound"));
        }
        match &mut self.bytes {
            MetricsBytes::Ordinary(output) => output.extend_from_slice(bytes),
            MetricsBytes::Prepared(output) => {
                if let Err(error) = output.extend_from_slice(bytes) {
                    self.failure = Some(error);
                    return Err(std::io::ErrorKind::WriteZero.into());
                }
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
