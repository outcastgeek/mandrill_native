//! Finite HTTP output caching, separate from origin-data retention.
//!
//! Each lookup executes the selected authorization and mutation/precondition gate
//! before consulting the core cache. Authorization/cookies/Set-Cookie/no-store/
//! private/Vary:* preserve conservative bypass. Streams and opaque replies cannot
//! acquire a finite witness through this constructor.

use crate::WebError;
use crate::{effect_die, effect_map, effect_map_error, effect_sync, effect_try_sync};
use effect::cache;
use effect::memory::{
    Buffer, BufferLease, BufferView, Bytes, MemoryError, PreparedMemory, RegionId, SlotLease,
    SlotPool,
};
use effect::{
    AdmissionCapacity, Cache, CacheAdmission, CacheCapacity, CacheConfig, CacheKey, CacheOptions,
    CacheTtl, Effect, HasClock, HasTaskSpawner, PreparedCacheCapacity, PreparedCacheRefusals,
    PreparedCacheStorage, ProviderAdmission, Scope, SharedValue, effect, service,
};
use effect_server::{reply, server};
use effect_server::{FiniteReply, HasRequest, Reply, Request, ResourceValidators};
use http::{HeaderName, HeaderValue, Method};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Trusted application partition selection borrowing the environment and request without credentials.
type OutputPartitioner<R> =
    Arc<dyn Fn(&R, &Request) -> Result<OutputPartition, WebError> + Send + Sync>;

/// Explicit finite output bounds/retention and variation policy.
#[derive(Clone, PartialEq, Eq)]
#[must_use]
pub struct FiniteOutputPolicy {
    /// Stable named policy namespace; separate policies never share cache identity.
    pub name: Arc<str>,
    /// Maximum encoded bytes in each checked finite response.
    pub response_bytes: NonZeroUsize,
    /// Maximum encoded bytes in one request cache key.
    pub key_bytes: NonZeroUsize,
    /// Maximum retained finite responses.
    pub entries: NonZeroUsize,
    /// Maximum concurrent finite renderers across cache misses and bypasses.
    pub maximum_active: NonZeroUsize,
    /// Positive elapsed retention; output responses always have finite retention.
    pub retain: Duration,
    /// Native request headers influencing representation/language selection.
    pub vary_headers: Arc<[HeaderName]>,
    /// Deliberately vary on the entire raw query string. Exact bytes preserve all
    /// caller query values without ambiguous normalization or credential inference.
    pub vary_query: bool,
}
impl FiniteOutputPolicy {
    /// Validate policy identity and finite duration before host startup. Authorization
    /// and Cookie are never admitted as an implicit private response cache partition.
    pub fn validate(self) -> Result<Self, WebError> {
        if self.name.is_empty()
            || self.retain.is_zero()
            || self
                .vary_headers
                .iter()
                .any(|name| name == http::header::AUTHORIZATION || name == http::header::COOKIE)
        {
            return Err(WebError::Configuration("finite output cache policy".into()));
        }
        let mut unique = std::collections::BTreeSet::new();
        if self
            .vary_headers
            .iter()
            .any(|name| !unique.insert(name.as_str()))
        {
            return Err(WebError::Configuration("duplicate output variation".into()));
        }
        Ok(self)
    }
}
/// Immutable explicit application partition. Public strings never confer auth
/// authority: lookup's gate remains mandatory before any cache operation.
#[derive(Clone, PartialEq, Eq)]
#[must_use]
pub struct OutputPartition {
    /// Caller-selected tenant/representation/culture/security policy namespace.
    pub namespace: Arc<str>,
    /// Typed application's stable public dimensions, excluding credential values.
    pub dimensions: Arc<[Arc<str>]>,
}
/// Complete lookup/render key, retaining request identity under finite bounds.
#[derive(Clone, PartialEq, Eq)]
#[must_use]
pub struct OutputKey {
    /// Validated policy namespace.
    pub policy: Arc<str>,
    /// Explicit application tenant/security/culture/representation partition.
    pub partition: OutputPartition,
    /// Exact request target path.
    pub path: Arc<str>,
    /// Exact raw query variation, explicitly selected by policy.
    pub query: Option<Arc<str>>,
    /// Exact repeated native header values in declared variation order.
    pub headers: Arc<[(HeaderName, Arc<[HeaderValue]>)]>,
}
/// Private gate-issued lookup authority. It cannot be manufactured by an endpoint
/// or stale request extension; authorization/preconditions are executed for each use.
struct AuthorizedKey {
    /// Selected key only after the application gate succeeded for this request.
    key: OutputKey,
    /// Conservative eligibility after selected authentication and native cookies.
    cacheable: bool,
}

enum AuthorizedOutput {
    Ordinary(AuthorizedKey),
    Prepared {
        input: PreparedOutputInput,
        identity: Option<BufferView<'static>>,
    },
}
/// Finite-only source. The callback returns already encoded bounded bytes, not a
/// Reply stream collected by Web. R provides the startup-selected render capabilities.
#[must_use]
pub struct FiniteSource<E: Send + 'static, R: Send + Sync + 'static> {
    /// Renderer uses complete explicit key dimensions and moves its response once.
    render: Arc<dyn Fn(Arc<OutputKey>) -> Effect<FiniteReply, E, R> + Send + Sync>,
    /// Only fixed checked metadata constructors may enable shared caching.
    cacheable: bool,
    /// Checked fixed response variation used to validate the policy key dimensions.
    vary: Arc<[HeaderName]>,
}

type OutputErrors<E> = Arc<dyn Fn(WebError) -> E + Send + Sync>;
type PreparedRender<E, R> = dyn Fn(PreparedOutputInput, RenderReservation, usize, OutputErrors<E>) -> Effect<FiniteReply, E, R>
    + Send
    + Sync;

struct RenderReservation {
    output: BufferLease<'static>,
    _renderer: SlotLease<'static, ()>,
}

/// A finite renderer whose fixed output slot is acquired before model loading or writing.
#[must_use]
pub struct PreparedFiniteSource<E: Send + 'static, R: Send + Sync + 'static> {
    render: Arc<PreparedRender<E, R>>,
    vary: Arc<[HeaderName]>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for PreparedFiniteSource<E, R> {
    fn clone(&self) -> Self {
        Self {
            render: self.render.clone(),
            vary: self.vary.clone(),
        }
    }
}

/// Checked destination retaining the actual preadmitted HTTP output backing.
pub struct PreparedOutputWriter<'a> {
    output: &'a mut Buffer<'static>,
    maximum: usize,
    failure: Option<WebError>,
}

impl PreparedOutputWriter<'_> {
    /// Append encoded bytes. The first refused append prevents publication.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), WebError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let result = self
            .output
            .len()
            .checked_add(bytes.len())
            .ok_or(WebError::Memory(MemoryError::ArithmeticOverflow))
            .and_then(|observed| {
                if observed > self.maximum {
                    return Err(WebError::ResponseLimit {
                        maximum: self.maximum,
                        observed,
                    });
                }
                self.output
                    .extend_from_slice(bytes)
                    .map_err(WebError::Memory)
            });
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }
}

/// Owned authorized input for a prepared finite renderer. Only the request
/// dimensions represented by the admitted cache identity are accessible.
pub struct PreparedOutputInput {
    policy: Arc<str>,
    partition: OutputPartition,
    vary: Arc<[HeaderName]>,
    request: Request,
}

impl PreparedOutputInput {
    /// Validated policy namespace.
    #[must_use]
    pub fn policy(&self) -> &str {
        &self.policy
    }

    /// Explicit authorized tenant/representation/culture dimensions.
    #[must_use]
    pub fn partition(&self) -> &OutputPartition {
        &self.partition
    }

    /// Exact request target path.
    #[must_use]
    pub fn path(&self) -> &str {
        self.request.uri().path()
    }

    /// Exact query, distinguishing an absent query from an empty one.
    #[must_use]
    pub fn query(&self) -> Option<&str> {
        self.request.uri().query()
    }

    /// Ordered values of a header declared by this cache's variation policy.
    pub fn header_values(
        &self,
        name: &HeaderName,
    ) -> Option<http::header::GetAll<'_, HeaderValue>> {
        self.vary
            .contains(name)
            .then(|| self.request.headers().get_all(name))
    }
}

struct PreparedOutputKey {
    identity: BufferView<'static>,
    input: Mutex<Option<PreparedOutputInput>>,
}

/// Cache metadata and typed key/reply cells prepared in the application's shared region.
pub struct PreparedOutputCacheStorage<E: Send + 'static> {
    cache: PreparedCacheStorage<PreparedOutputKey, FiniteReply, E>,
    capacity: PreparedCacheCapacity,
    keys: SlotPool<'static, Buffer<'static>>,
    renderers: SlotPool<'static, ()>,
}

impl<E: Send + 'static> PreparedOutputCacheStorage<E> {
    /// Calculate storage for the selected core cache records. The request's
    /// output bytes are supplied by the host pool and retain that pool owner.
    pub fn storage_bytes(
        capacity: PreparedCacheCapacity,
        key_backing_bytes: NonZeroUsize,
    ) -> Result<Bytes, MemoryError> {
        PreparedCacheStorage::<PreparedOutputKey, FiniteReply, E>::storage_bytes(capacity)?
            .checked_add(SlotPool::<Buffer<'static>>::storage_bytes(
                capacity.keys.get(),
            )?)?
            .checked_add(Bytes::new(key_backing_bytes.get()).checked_mul(capacity.keys.get())?)?
            .checked_add(SlotPool::<()>::storage_bytes(capacity.active.get())?)
    }

    /// Provision the cache records before traffic begins.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedCacheCapacity,
        key_backing_bytes: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            cache: PreparedCacheStorage::prepare(memory, region, capacity)?,
            capacity,
            keys: memory.buffers(
                region,
                capacity.keys.get(),
                Bytes::new(key_backing_bytes.get()),
            )?,
            renderers: memory.slots(region, capacity.active.get(), |_| Ok(()))?,
        })
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for FiniteSource<E, R> {
    /// Share only the inert callback; response bytes retain native shared ownership.
    fn clone(&self) -> Self {
        Self {
            render: self.render.clone(),
            cacheable: self.cacheable,
            vary: self.vary.clone(),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> FiniteSource<E, R> {
    fn render_admitted(
        &self,
        key: Arc<OutputKey>,
        renderers: ProviderAdmission,
        maximum: usize,
        errors: OutputErrors<E>,
    ) -> Effect<FiniteReply, E, R> {
        let render = self.render.clone();
        let on_capacity = errors.clone();
        crate::admission::independent(
            renderers,
            move || {
                let errors = errors.clone();
                effect! {
                    bind;
                    program: render(key.clone()),
                    body: move |reply| {
                        let errors = errors.clone();
                        effect_try_sync! {
                            once;
                            body: move || {
                                let observed = reply.bytes.len();
                                if observed > maximum {
                                    return Err(errors(WebError::ResponseLimit { maximum, observed }));
                                }
                                Ok(reply)
                            },
                            on_error: ::core::convert::identity,
                        }
                    },
                }
            },
            move |_| renderer_capacity_error(&on_capacity),
        )
    }

    /// Select a renderer whose terminal result is the Server finite witness. An
    /// opaque/native/stream Reply must not be converted via body collection here.
    pub fn new(
        render: impl Fn(Arc<OutputKey>) -> Effect<FiniteReply, E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            render: Arc::new(render),
            cacheable: false,
            vary: Arc::from([]),
        }
    }
    /// Safe finite text source with fixed native metadata. Dynamic native response
    /// callbacks use `new` and bypass shared caching; they can set cookies safely.
    pub fn text(
        headers: http::HeaderMap,
        render: impl Fn(Arc<OutputKey>) -> Effect<String, E, R> + Send + Sync + 'static,
    ) -> Result<Self, WebError> {
        let witness = FiniteReply {
            status: http::StatusCode::OK,
            headers: headers.clone(),
            bytes: bytes::Bytes::new(),
        };
        if !response_cacheable(&witness) {
            return Err(WebError::Configuration("finite text cache metadata".into()));
        }
        let mut vary = Vec::new();
        for value in headers.get_all(http::header::VARY) {
            let value = value
                .to_str()
                .map_err(|_| WebError::Configuration("finite text Vary".into()))?;
            for name in value.split(',') {
                vary.push(
                    HeaderName::from_bytes(name.trim().as_bytes())
                        .map_err(|_| WebError::Configuration("finite text Vary".into()))?,
                );
            }
        }
        Ok(Self {
            render: Arc::new(move |key| {
                let headers = headers.clone();
                effect_map! {
                    program: render(key), map: move |text| {
                    let mut reply = FiniteReply {
                        status: http::StatusCode::OK,
                        headers: headers.clone(),
                        bytes: bytes::Bytes::from(text),
                    };
                    reply.headers.insert(
                        http::header::CONTENT_TYPE,
                        HeaderValue::from_static("text/plain; charset=utf-8"),
                    );
                    reply
                }, }
            }),
            cacheable: true,
            vary: vary.into(),
        })
    }
    /// Safe finite encoded representation with fixed checked metadata. This source
    /// accepts already finite immutable bytes, never a collected native body/stream.
    /// Dynamic cookie/security metadata must use the bypass-only `new` constructor.
    pub fn encoded(
        media: HeaderValue,
        mut headers: http::HeaderMap,
        render: impl Fn(Arc<OutputKey>) -> Effect<bytes::Bytes, E, R> + Send + Sync + 'static,
    ) -> Result<Self, WebError> {
        headers.insert(http::header::CONTENT_TYPE, media);
        let witness = FiniteReply {
            status: http::StatusCode::OK,
            headers: headers.clone(),
            bytes: bytes::Bytes::new(),
        };
        if !response_cacheable(&witness) {
            return Err(WebError::Configuration(
                "finite encoded cache metadata".into(),
            ));
        }
        let mut vary = Vec::new();
        for value in headers.get_all(http::header::VARY) {
            let value = value
                .to_str()
                .map_err(|_| WebError::Configuration("finite encoded Vary".into()))?;
            for name in value.split(',') {
                vary.push(
                    HeaderName::from_bytes(name.trim().as_bytes())
                        .map_err(|_| WebError::Configuration("finite encoded Vary".into()))?,
                );
            }
        }
        Ok(Self {
            render: Arc::new(move |key| {
                let headers = headers.clone();
                effect_map! {
                    program: render(key), map: move |bytes| FiniteReply {
                    status: http::StatusCode::OK,
                    headers: headers.clone(),
                    bytes,
                }, }
            }),
            cacheable: true,
            vary: vary.into(),
        })
    }

    /// Select fixed response metadata and a checked writer over the output
    /// buffer already admitted for the request producing this response.
    pub fn prepared_encoded<A: Send + 'static>(
        media: HeaderValue,
        mut headers: http::HeaderMap,
        prepare_model: impl Fn(PreparedOutputInput) -> Effect<A, E, R> + Send + Sync + 'static,
        write: impl Fn(A, &mut PreparedOutputWriter<'_>) -> Result<(), E> + Send + Sync + 'static,
    ) -> Result<PreparedFiniteSource<E, R>, WebError> {
        headers.insert(http::header::CONTENT_TYPE, media);
        let witness = FiniteReply {
            status: http::StatusCode::OK,
            headers,
            bytes: bytes::Bytes::new(),
        };
        if !response_cacheable(&witness) {
            return Err(WebError::Configuration(
                "finite encoded cache metadata".into(),
            ));
        }
        let vary = fixed_vary(&witness.headers, "finite encoded Vary")?;
        let headers = Arc::new(witness.headers);
        let prepare_model = Arc::new(prepare_model);
        let write = Arc::new(write);
        Ok(PreparedFiniteSource {
            vary,
            render: Arc::new(move |key, output, maximum, errors| {
                let key = effect_sync! { once; body { key }, };
                let output = effect_sync! { once; body { output }, };
                let prepare_model = prepare_model.clone();
                let write = write.clone();
                let headers = headers.clone();
                effect! {
                    let output = output.clone();
                    let key = key.clone();
                    let prepare_model = prepare_model.clone();
                    let write = write.clone();
                    let headers = headers.clone();
                    let errors = errors.clone();
                    let output = output?;
                    let key = key?;
                    let model = prepare_model(key)?;
                    effect_try_sync! {
                        once;
                        body: move || {
                            let mut output = output;
                            let maximum = maximum.min(output.output.capacity().get());
                            let mut writer = PreparedOutputWriter { output: &mut output.output, maximum, failure: None };
                            write(model, &mut writer)?;
                            if let Some(error) = writer.failure {
                                return Err(errors(error));
                            }
                            let view = BufferView::new(output.output.share());
                            Ok(FiniteReply {
                                status: http::StatusCode::OK,
                                headers: headers.as_ref().clone(),
                                bytes: bytes::Bytes::from_owner(view),
                            })
                        },
                        on_error: ::core::convert::identity,
                    }?
                }
            }),
        })
    }
}

fn renderer_capacity_error<E>(errors: &OutputErrors<E>) -> E {
    errors(WebError::Configuration(
        "output renderer active capacity".into(),
    ))
}

fn fixed_vary(
    headers: &http::HeaderMap,
    error: &'static str,
) -> Result<Arc<[HeaderName]>, WebError> {
    let mut vary = Vec::new();
    for value in headers.get_all(http::header::VARY) {
        let value = value
            .to_str()
            .map_err(|_| WebError::Configuration(error.into()))?;
        for name in value.split(',') {
            vary.push(
                HeaderName::from_bytes(name.trim().as_bytes())
                    .map_err(|_| WebError::Configuration(error.into()))?,
            );
        }
    }
    Ok(vary.into())
}

impl<E: Send + 'static, R: Send + Sync + 'static> PreparedFiniteSource<E, R> {
    fn render_for(
        &self,
        input: PreparedOutputInput,
        renderers: &SlotPool<'static, ()>,
        maximum: usize,
        errors: OutputErrors<E>,
    ) -> Result<Effect<FiniteReply, E, R>, WebError> {
        let renderer = renderers.try_checkout().map_err(WebError::Memory)?;
        let output = input
            .request
            .take_prepared_output_buffer()
            .map_err(crate::application::web_server_error)?;
        Ok((self.render)(
            input,
            RenderReservation {
                output,
                _renderer: renderer,
            },
            maximum,
            errors,
        ))
    }
}

enum CacheBackend<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary {
        cache: Cache<OutputKey, FiniteReply, E>,
        source: FiniteSource<E, R>,
        renderers: ProviderAdmission,
    },
    Prepared {
        cache: Cache<PreparedOutputKey, FiniteReply, E>,
        source: PreparedFiniteSource<E, R>,
        keys: Arc<SlotPool<'static, Buffer<'static>>>,
        renderers: Arc<SlotPool<'static, ()>>,
        refusals: PreparedCacheRefusals<E>,
    },
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for CacheBackend<E, R> {
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary {
                cache,
                source,
                renderers,
            } => Self::Ordinary {
                cache: cache.clone(),
                source: source.clone(),
                renderers: renderers.clone(),
            },
            Self::Prepared {
                cache,
                source,
                keys,
                renderers,
                refusals,
            } => Self::Prepared {
                cache: cache.clone(),
                source: source.clone(),
                keys: keys.clone(),
                renderers: renderers.clone(),
                refusals: refusals.clone(),
            },
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> CacheBackend<E, R> {
    fn cacheable(&self) -> bool {
        match self {
            Self::Ordinary { source, .. } => source.cacheable,
            Self::Prepared { .. } => true,
        }
    }
}
/// Shared finite output owner over core Cache; no second HTTP cache map exists.
#[must_use]
pub struct OutputCache<E: Send + 'static, R: Send + Sync + 'static> {
    /// Core retained/shared finite producer owner.
    cache: CacheBackend<E, R>,
    /// Explicit policy retained for checked keys/responses.
    policy: FiniteOutputPolicy,
    /// Application-owned mandatory auth/precondition gate before lookup.
    gate: Arc<dyn Fn(Request) -> Effect<(), E, R> + Send + Sync>,
    /// Selected authentication state supplied by a trusted application capability.
    authenticated: Arc<dyn Fn(&R) -> bool + Send + Sync>,
    /// Explicit safe partition callback; no user credentials are stringified.
    partition: OutputPartitioner<R>,
    /// Typed web policy/finite bound error mapping.
    errors: Arc<dyn Fn(WebError) -> E + Send + Sync>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for OutputCache<E, R> {
    /// Share the same cache and selected gates, never active response payloads.
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            policy: self.policy.clone(),
            gate: self.gate.clone(),
            authenticated: self.authenticated.clone(),
            partition: self.partition.clone(),
            errors: self.errors.clone(),
        }
    }
}
impl<E: Send + 'static, R: HasClock + HasTaskSpawner + HasRequest + Send + Sync + 'static>
    OutputCache<E, R>
{
    /// Acquire the core finite owner in the host scope. Authentication/preconditions
    /// are caller-owned and always executed before a hit; failure caching is disabled.
    pub fn scoped(
        scope: Scope<SharedValue<E>>,
        policy: FiniteOutputPolicy,
        source: FiniteSource<E, R>,
        gate: impl Fn(Request) -> Effect<(), E, R> + Send + Sync + 'static,
        authenticated: impl Fn(&R) -> bool + Send + Sync + 'static,
        partition: impl Fn(&R, &Request) -> Result<OutputPartition, WebError> + Send + Sync + 'static,
        errors: impl Fn(WebError) -> E + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        let errors: OutputErrors<E> = Arc::new(errors);
        let valid = policy.validate().map_err({
            let errors = errors.clone();
            move |error| SharedValue::new(errors(error))
        });
        let source = Arc::new(source);
        let gate = Arc::new(gate);
        let authenticated = Arc::new(authenticated);
        let partition = Arc::new(partition);
        effect! {
        bind;
        program: effect::effect_from_result! { once; result: valid, },
        body: move |policy| {
                   if source
                       .vary
                       .iter()
                       .any(|name| !policy.vary_headers.contains(name))
                   {
                       return effect::effect_from_result! {
                           once;
                           result: Err(SharedValue::new(errors(
                                   WebError::Configuration("output Vary missing key dimension".into()),
                               ))),
                       };
                   }
                   let load_source = source.clone();
                   let maximum = policy.response_bytes.get();
                   let check_error = errors.clone();
                   let admission_error = errors.clone();
                   let maximum_active = policy.maximum_active;
                   let renderers = ProviderAdmission::new(AdmissionCapacity::Bounded(maximum_active));
                   let load_renderers = renderers.clone();
                   effect_map! {
                       program: cache! {
                       scope: scope.clone(),
                       config: CacheConfig {
                           equal: Arc::new(|left, right| left == right),
                           ttl: Some(policy.retain),
                           capacity: CacheCapacity::Bounded(policy.entries),
                           cache_failure: Arc::new(|_| false),
                       },
                       options: CacheOptions {
                           ttl: CacheTtl::Fixed(Some(policy.retain)),
                           admission: CacheAdmission::Bounded {
                               maximum: maximum_active,
                               capacity_error: Arc::new(move || renderer_capacity_error(&admission_error)),
                           },
                       },
                       load_shared: move |key| {
                           load_source.render_admitted(key, load_renderers.clone(), maximum, check_error.clone())
                       },
                   }, map: {
                       let source = source.clone();
                       let gate = gate.clone();
                       let authenticated = authenticated.clone();
                       let partition = partition.clone();
                       let errors = errors.clone();
                       move |cache| Self {
                           cache: CacheBackend::Ordinary {
                               cache, source: source.as_ref().clone(), renderers: renderers.clone(),
                           },
                           policy: policy.clone(),
                           gate: gate.clone(),
                           authenticated: authenticated.clone(),
                           partition: partition.clone(),
                           errors: errors.clone(),
                       }
                   }, }
               },
        }
    }

    /// Acquire a finite cache whose key backing and combined miss/bypass
    /// render slots come from the selected region. Responses retain the host's
    /// admitted output slot through cache hits and HTTP delivery.
    #[allow(clippy::too_many_arguments)] // The scoped gate and authentication/partition roles are independent.
    pub fn scoped_prepared(
        scope: Scope<SharedValue<E>>,
        policy: FiniteOutputPolicy,
        source: PreparedFiniteSource<E, R>,
        (storage, refusals): (PreparedOutputCacheStorage<E>, PreparedCacheRefusals<E>),
        gate: impl Fn(Request) -> Effect<(), E, R> + Send + Sync + 'static,
        authenticated: impl Fn(&R) -> bool + Send + Sync + 'static,
        partition: impl Fn(&R, &Request) -> Result<OutputPartition, WebError> + Send + Sync + 'static,
        errors: impl Fn(WebError) -> E + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        let errors: OutputErrors<E> = Arc::new(errors);
        let policy = policy.validate().and_then(|policy| {
            if source.vary.iter().any(|name| !policy.vary_headers.contains(name)) {
                return Err(WebError::Configuration("output Vary missing key dimension".into()));
            }
            if !matches!(storage.capacity.retention, CacheCapacity::Bounded(entries) if entries == policy.entries)
                || storage.capacity.active != policy.maximum_active
            {
                return Err(WebError::Configuration("output cache capacity differs from policy".into()));
            }
            Ok(policy)
        });
        let policy = match policy {
            Ok(policy) => policy,
            Err(error) => {
                return effect_try_sync! { body: move || Err(SharedValue::new(errors(error.clone()))), };
            }
        };
        let keys = Arc::new(storage.keys);
        let renderers = Arc::new(storage.renderers);
        let renderers_for_load = renderers.clone();
        let source_for_load = source.clone();
        let errors_for_load = errors.clone();
        let admission_error = errors.clone();
        let refusal_for_cache = refusals.clone();
        let maximum = policy.response_bytes.get();
        let cache = cache! {
            scope: scope,
            config: CacheConfig {
                equal: Arc::new(|left: &PreparedOutputKey, right: &PreparedOutputKey| left.identity.as_ref() == right.identity.as_ref()),
                ttl: Some(policy.retain), capacity: CacheCapacity::Bounded(policy.entries),
                cache_failure: Arc::new(|_| false),
            },
            options: CacheOptions {
                ttl: CacheTtl::Fixed(Some(policy.retain)),
                admission: CacheAdmission::Bounded {
                    maximum: policy.maximum_active,
                    capacity_error: Arc::new(move || admission_error(WebError::Memory(MemoryError::NoSlot))),
                },
            },
            memory: storage.cache,
            refusals: refusal_for_cache,
            load_shared: move |key: CacheKey<PreparedOutputKey>| {
                let input = key.input.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                let errors = errors_for_load.clone();
                match input.ok_or(WebError::Memory(MemoryError::MissingRuntimeStorage))
                    .and_then(|input| source_for_load.render_for(input, &renderers_for_load, maximum, errors.clone()))
                {
                    Ok(render) => render,
                    Err(error) => effect_try_sync! { body: move || Err(errors(error.clone())), },
                }
            },
        };
        let gate = Arc::new(gate);
        let authenticated = Arc::new(authenticated);
        let partition = Arc::new(partition);
        effect_map! {
            program: cache,
            map: move |cache| Self {
                cache: CacheBackend::Prepared {
                    cache, source: source.clone(), keys: keys.clone(), renderers: renderers.clone(), refusals: refusals.clone(),
                },
                policy: policy.clone(),
                gate: gate.clone(), authenticated: authenticated.clone(), partition: partition.clone(), errors: errors.clone(),
            },
        }
    }
    /// Authorize/evaluate mutation preconditions on every invocation, then consult
    /// finite cache only for eligible GET/HEAD unauthenticated cookie-free requests.
    pub fn reply(&self) -> Effect<Reply<SharedValue<E>, R>, SharedValue<E>, R> {
        let owner = self.clone();
        effect! {
            let owner = owner.clone();
            let request = server! { request; }?;
            effect_map_error! { program: (owner.gate)(request.clone()), map: SharedValue::new, }?;
            let key_owner = owner.clone();
            let request_for_key = request.clone();
            let selected = service! {
                env |env: &R| { key_owner.key(env, &request_for_key) },
            }?;
            let failure_owner = owner.clone();
            let selected = effect::effect_from_result! { once; result: selected, on_error: move |error| failure_owner.selection_error(error), }?;
            let finite = match (owner.cache.clone(), selected) {
                (CacheBackend::Ordinary { cache, .. }, AuthorizedOutput::Ordinary(AuthorizedKey { key, cacheable: true })) => {
                    let value = effect::cache! { cache: cache, get: key, }?;
                    match value.with(Clone::clone) {
                        Ok(reply) => reply,
                        Err(defect) => effect_die! { defect: defect, }?,
                    }
                }
                (CacheBackend::Ordinary { source, renderers, .. }, AuthorizedOutput::Ordinary(AuthorizedKey { key, cacheable: false })) => {
                    let render = source.render_admitted(
                        Arc::new(key), renderers, owner.policy.response_bytes.get(), owner.errors.clone(),
                    );
                    effect_map_error! { program: render, map: SharedValue::new, }?
                }
                (CacheBackend::Prepared { cache, refusals, .. }, AuthorizedOutput::Prepared { input, identity: Some(identity) }) => {
                    let key = effect_try_sync! {
                        once;
                        body: {
                            let cache = cache.clone();
                            move || {
                                cache.prepare_key(PreparedOutputKey { identity, input: Mutex::new(Some(input)) })
                                    .map_err(|(error, abandoned)| {
                                        drop(abandoned);
                                        refusals.memory(error)
                                    })
                            }
                        },
                        on_error: ::core::convert::identity,
                    }?;
                    let value = cache.get_prepared(key)?;
                    match value.with(Clone::clone) {
                        Ok(reply) => reply,
                        Err(defect) => effect_die! { defect: defect, }?,
                    }
                }
                (CacheBackend::Prepared { source, renderers, .. }, AuthorizedOutput::Prepared { input, identity: None }) => {
                    let reserved = source.render_for(input, &renderers, owner.policy.response_bytes.get(), owner.errors.clone());
                    let render = effect_try_sync! {
                        once;
                        body: move || reserved,
                        on_error: move |error| owner.selection_error(error),
                    }?;
                    effect_map_error! {
                        program: render,
                        map: SharedValue::new,
                    }?
                }
                _ => effect_die! { defect: effect::Defect::from(effect::LibraryDefect::PreparedConstructionUnbound), }?,
            };
            reply! { finite(finite) }
        }
    }

    fn selection_error(&self, error: WebError) -> SharedValue<E> {
        if let CacheBackend::Prepared { refusals, .. } = &self.cache
            && let WebError::Memory(
                reason @ (MemoryError::NoSlot
                | MemoryError::Busy
                | MemoryError::CapacityExceeded { .. }),
            ) = &error
        {
            return refusals.memory(*reason);
        }
        SharedValue::new((self.errors)(error))
    }

    /// Produce the authorized identity before lookup or renderer construction.
    fn key(&self, env: &R, request: &Request) -> Result<AuthorizedOutput, WebError> {
        let declared = request
            .with_extensions(|extensions| extensions.get::<RootOutputCache>().cloned())
            .ok_or_else(|| WebError::Configuration("output cache root selection".into()))?;
        if declared.definition.policy(&self.policy.name)? != self.policy {
            return Err(WebError::Configuration(
                "output policy differs from root selection".into(),
            ));
        }
        let partition = (self.partition)(env, request)?;
        if partition.namespace.is_empty() {
            return Err(WebError::Configuration("output partition namespace".into()));
        }
        let bytes = key_payload_bytes(&self.policy, &partition, request);
        if bytes.is_none_or(|bytes| bytes > self.policy.key_bytes.get()) {
            return Err(WebError::Configuration("output key byte bound".into()));
        }
        let cacheable = (self.policy.vary_query || request.uri().query().is_none())
            && self.cache.cacheable()
            && matches!(*request.method(), Method::GET | Method::HEAD)
            && !(self.authenticated)(env)
            && !request.headers().contains_key(http::header::AUTHORIZATION)
            && !request.headers().contains_key(http::header::COOKIE);
        match &self.cache {
            CacheBackend::Ordinary { .. } => {
                let headers = self
                    .policy
                    .vary_headers
                    .iter()
                    .map(|name| {
                        (
                            name.clone(),
                            request
                                .headers()
                                .get_all(name)
                                .iter()
                                .cloned()
                                .collect::<Vec<_>>()
                                .into(),
                        )
                    })
                    .collect::<Vec<_>>();
                Ok(AuthorizedOutput::Ordinary(AuthorizedKey {
                    key: OutputKey {
                        policy: self.policy.name.clone(),
                        partition,
                        path: Arc::from(request.uri().path()),
                        query: request.uri().query().map(Arc::from),
                        headers: headers.into(),
                    },
                    cacheable,
                }))
            }
            CacheBackend::Prepared { keys, .. } => {
                let identity = if cacheable {
                    let mut identity = keys.try_checkout().map_err(WebError::Memory)?;
                    identity.clear();
                    write_identity(&mut identity, &self.policy, &partition, request)
                        .map_err(WebError::Memory)?;
                    Some(BufferView::new(identity.share()))
                } else {
                    None
                };
                Ok(AuthorizedOutput::Prepared {
                    input: PreparedOutputInput {
                        policy: self.policy.name.clone(),
                        partition,
                        vary: self.policy.vary_headers.clone(),
                        request: request.clone(),
                    },
                    identity,
                })
            }
        }
    }
    /// Reuse Server's protocol-order validator handling on a finite reply. Callers
    /// must keep mutation preconditions in `gate` so hits cannot bypass them.
    pub fn conditional(
        reply: Reply<SharedValue<E>, R>,
        validators: ResourceValidators,
    ) -> Reply<SharedValue<E>, R> {
        reply! {
            validators(validators);
            source(reply);
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> OutputCache<E, R> {
    /// Evict finite responses and join fenced renderers after a source change.
    pub fn clear(&self) -> Effect<(), SharedValue<E>, R> {
        match &self.cache {
            CacheBackend::Ordinary { cache, .. } => effect::cache! { cache: cache, clear: (), },
            CacheBackend::Prepared { cache, .. } => effect::cache! { cache: cache, clear: (), },
        }
    }
}

fn key_payload_bytes(
    policy: &FiniteOutputPolicy,
    partition: &OutputPartition,
    request: &Request,
) -> Option<usize> {
    let mut bytes = policy
        .name
        .len()
        .checked_add(request.uri().path().len())?
        .checked_add(request.uri().query().map_or(0, str::len))?
        .checked_add(partition.namespace.len())?;
    for value in partition.dimensions.iter() {
        bytes = bytes.checked_add(value.len())?;
    }
    for name in policy.vary_headers.iter() {
        bytes = bytes.checked_add(name.as_str().len())?;
        for value in request.headers().get_all(name) {
            bytes = bytes.checked_add(value.as_bytes().len())?;
        }
    }
    Some(bytes)
}

fn write_identity(
    output: &mut Buffer<'static>,
    policy: &FiniteOutputPolicy,
    partition: &OutputPartition,
    request: &Request,
) -> Result<(), MemoryError> {
    fn length(output: &mut Buffer<'static>, count: usize) -> Result<(), MemoryError> {
        let count = u64::try_from(count).map_err(|_| MemoryError::ArithmeticOverflow)?;
        output.extend_from_slice(&count.to_be_bytes())
    }
    fn field(output: &mut Buffer<'static>, value: &[u8]) -> Result<(), MemoryError> {
        length(output, value.len())?;
        output.extend_from_slice(value)
    }
    field(output, policy.name.as_bytes())?;
    field(output, partition.namespace.as_bytes())?;
    length(output, partition.dimensions.len())?;
    for dimension in partition.dimensions.iter() {
        field(output, dimension.as_bytes())?;
    }
    field(output, request.uri().path().as_bytes())?;
    if let Some(query) = request.uri().query() {
        output.extend_from_slice(&[1])?;
        field(output, query.as_bytes())?;
    } else {
        output.extend_from_slice(&[0])?;
    }
    length(output, policy.vary_headers.len())?;
    for name in policy.vary_headers.iter() {
        field(output, name.as_str().as_bytes())?;
        let values = request.headers().get_all(name);
        length(output, values.iter().count())?;
        for value in values {
            field(output, value.as_bytes())?;
        }
    }
    Ok(())
}

/// Native conservative fixed-metadata eligibility. Dynamic native metadata uses
/// the advanced bypass source and never enters shared/retained cache ownership.
fn response_cacheable(reply: &FiniteReply) -> bool {
    reply.status == http::StatusCode::OK
        && !reply.headers.contains_key(http::header::SET_COOKIE)
        && !reply
            .headers
            .get_all(http::header::CACHE_CONTROL)
            .iter()
            .any(|value| {
                value.to_str().map_or(true, |text| {
                    text.split(',').any(|part| {
                        part.trim().split('=').next().is_some_and(|name| {
                            ["no-store", "private", "no-cache"]
                                .iter()
                                .any(|excluded| name.eq_ignore_ascii_case(excluded))
                        })
                    })
                })
            })
        && !reply
            .headers
            .get_all(http::header::VARY)
            .iter()
            .any(|value| {
                value
                    .to_str()
                    .map_or(true, |text| text.split(',').any(|part| part.trim() == "*"))
            })
}

/// Validated named finite-output policy definitions. This is immutable configuration
/// only: each selected source acquires the existing core cache through `OutputCache::scoped`.
#[derive(Clone)]
#[must_use]
pub struct OutputCacheDefinition {
    /// Explicit policy identities and finite units, shared without cache payloads.
    policies: Arc<[FiniteOutputPolicy]>,
}
impl OutputCacheDefinition {
    /// Validate nonempty named policies before traffic. No cache engine or native
    /// listener starts during configuration construction.
    pub fn new(policies: Vec<FiniteOutputPolicy>) -> Result<Self, WebError> {
        if policies.is_empty() {
            return Err(WebError::Configuration(
                "empty output cache policies".into(),
            ));
        }
        let mut checked = Vec::new();
        let mut names = std::collections::BTreeSet::new();
        for policy in policies {
            let policy = policy.validate()?;
            if !names.insert(policy.name.clone()) {
                return Err(WebError::Configuration(
                    "duplicate output cache policy".into(),
                ));
            }
            checked.push(policy);
        }
        Ok(Self {
            policies: checked.into(),
        })
    }
    /// Select a checked named policy for a finite source's scoped core acquisition.
    /// Unknown names fail; there is no implicit unnamed/private/stream fallback.
    pub fn policy(&self, name: &str) -> Result<FiniteOutputPolicy, WebError> {
        self.policies
            .iter()
            .find(|policy| policy.name.as_ref() == name)
            .cloned()
            .ok_or_else(|| WebError::Configuration("unknown output cache policy".into()))
    }
}
/// Explicit root output-cache selection. The safe configuration form is distinct
/// from an advanced application-owned ordered pipeline.
#[must_use]
pub enum OutputCacheSelection<E: Send + 'static, R: Send + Sync + 'static> {
    /// Validated finite-only named policies used by startup source acquisition.
    Definition(Result<OutputCacheDefinition, WebError>),
    /// Explicit advanced native/application policy stage preserving Server ownership.
    Pipeline(effect_server::Pipeline<E, R>),
}
/// Typed nesting for root definitions and advanced ordered policies.
pub trait IntoOutputCache<E: Send + 'static, R: Send + Sync + 'static> {
    /// Preserve the selected finite definition or advanced pipeline contract.
    fn into_output_cache(self) -> OutputCacheSelection<E, R>;
}
impl<E: Send + 'static, R: Send + Sync + 'static> IntoOutputCache<E, R> for OutputCacheDefinition {
    /// Checked named configuration remains inert.
    fn into_output_cache(self) -> OutputCacheSelection<E, R> {
        OutputCacheSelection::Definition(Ok(self))
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> IntoOutputCache<E, R>
    for Result<OutputCacheDefinition, WebError>
{
    /// Root startup retains configuration failures.
    fn into_output_cache(self) -> OutputCacheSelection<E, R> {
        OutputCacheSelection::Definition(self)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> IntoOutputCache<E, R>
    for effect_server::Pipeline<E, R>
{
    /// Advanced policies retain their explicit declaration/nesting order.
    fn into_output_cache(self) -> OutputCacheSelection<E, R> {
        OutputCacheSelection::Pipeline(self)
    }
}

/// Private root-issued policy selection. Native headers or endpoint metadata cannot
/// forge declaration authority; it grants no authentication or mutation permission.
#[derive(Clone)]
pub(crate) struct RootOutputCache {
    /// Validated immutable definitions selected by the same prepared Web host.
    pub(crate) definition: OutputCacheDefinition,
}
