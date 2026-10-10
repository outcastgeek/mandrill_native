//! Finite HTTP output caching, separate from origin-data retention.
//!
//! Each lookup executes the selected authorization and mutation/precondition gate
//! before consulting the core cache. Authorization/cookies/Set-Cookie/no-store/
//! private/Vary:* preserve conservative bypass. Streams and opaque replies cannot
//! acquire a finite witness through this constructor.

use crate::WebError;
use effect::{
    Cache, CacheAdmission, CacheCapacity, CacheConfig, CacheOptions, CacheTtl, Effect, HasClock,
    HasTaskSpawner, Scope, SharedValue,
};
use effect_server::{FiniteReply, HasRequest, Reply, Request, ResourceValidators};
use http::{HeaderName, HeaderValue, Method};
use std::num::NonZeroUsize;
use std::sync::Arc;
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
    /// Maximum independent finite renderers admitted by the core cache.
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
                render(key).map(move |text| {
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
                })
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
                render(key).map(move |bytes| FiniteReply {
                    status: http::StatusCode::OK,
                    headers: headers.clone(),
                    bytes,
                })
            }),
            cacheable: true,
            vary: vary.into(),
        })
    }
}
/// Shared finite output owner over core Cache; no second HTTP cache map exists.
#[must_use]
pub struct OutputCache<E: Send + 'static, R: Send + Sync + 'static> {
    /// Core retained/shared finite producer owner.
    cache: Cache<OutputKey, FiniteReply, E>,
    /// Explicit policy retained for checked keys/responses.
    policy: FiniteOutputPolicy,
    /// Independent bypass still invokes the same finite source.
    source: FiniteSource<E, R>,
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
            source: self.source.clone(),
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
        let errors = Arc::new(errors);
        let valid = policy.validate().map_err({
            let errors = errors.clone();
            move |error| SharedValue::new(errors(error))
        });
        let source = Arc::new(source);
        let gate = Arc::new(gate);
        let authenticated = Arc::new(authenticated);
        let partition = Arc::new(partition);
        Effect::from_result_once(valid).flat_map(move |policy| {
            if source
                .vary
                .iter()
                .any(|name| !policy.vary_headers.contains(name))
            {
                return Effect::from_result_once(Err(SharedValue::new(errors(
                    WebError::Configuration("output Vary missing key dimension".into()),
                ))));
            }
            let render = source.render.clone();
            let maximum = policy.response_bytes.get();
            let check_error = errors.clone();
            let admission_error = errors.clone();
            let maximum_active = policy.maximum_active;
            Cache::make_with_shared_key(
                scope.clone(),
                CacheConfig {
                    equal: Arc::new(|left, right| left == right),
                    ttl: Some(policy.retain),
                    capacity: CacheCapacity::Bounded(policy.entries),
                    cache_failure: Arc::new(|_| false),
                },
                CacheOptions {
                    ttl: CacheTtl::Fixed(Some(policy.retain)),
                    admission: CacheAdmission::Bounded {
                        maximum: maximum_active,
                        capacity_error: Arc::new(move || {
                            admission_error(WebError::Configuration(
                                "output renderer active capacity".into(),
                            ))
                        }),
                    },
                },
                move |key| {
                    let errors = check_error.clone();
                    render(key).flat_map(move |reply| {
                        if reply.bytes.len() > maximum {
                            Effect::from_result_once(Err(errors(WebError::ResponseLimit {
                                maximum,
                                observed: reply.bytes.len(),
                            })))
                        } else {
                            Effect::from_result_once(Ok(reply))
                        }
                    })
                },
            )
            .map({
                let source = source.clone();
                let gate = gate.clone();
                let authenticated = authenticated.clone();
                let partition = partition.clone();
                let errors = errors.clone();
                move |cache| Self {
                    cache,
                    policy: policy.clone(),
                    source: source.as_ref().clone(),
                    gate: gate.clone(),
                    authenticated: authenticated.clone(),
                    partition: partition.clone(),
                    errors: errors.clone(),
                }
            })
        })
    }
    /// Authorize/evaluate mutation preconditions on every invocation, then consult
    /// finite cache only for eligible GET/HEAD unauthenticated cookie-free requests.
    pub fn reply(&self) -> Effect<Reply<SharedValue<E>, R>, SharedValue<E>, R> {
        let owner = self.clone();
        Effect::service(|env: &R| env.request().clone()).flat_map(move |request| {
            let owner = owner.clone();
            let request_after_gate = request.clone();
            (owner.gate)(request)
                .map_error(SharedValue::new)
                .flat_map(move |()| {
                    let owner = owner.clone();
                    let request = request_after_gate.clone();
                    let key_owner = owner.clone();
                    Effect::service(move |env: &R| key_owner.key(env, &request)).flat_map({
                        let owner = owner.clone();
                        move |key| match key {
                            Err(error) => Effect::from_result_once(Err(SharedValue::new((owner
                                .errors)(
                                error
                            )))),
                            Ok(AuthorizedKey {
                                key,
                                cacheable: true,
                            }) => owner.cache.get(key).flat_map(|value| {
                                match value.with(Clone::clone) {
                                    Ok(reply) => {
                                        Effect::from_result_once(Ok(Reply::from_finite(reply)))
                                    }
                                    Err(defect) => Effect::die(defect),
                                }
                            }),
                            Ok(AuthorizedKey {
                                key,
                                cacheable: false,
                            }) => (owner.source.render)(Arc::new(key))
                                .map_error(SharedValue::new)
                                .flat_map({
                                    let owner = owner.clone();
                                    move |reply| {
                                        if reply.bytes.len() > owner.policy.response_bytes.get() {
                                            Effect::from_result_once(Err(SharedValue::new((owner
                                                .errors)(
                                                WebError::ResponseLimit {
                                                    maximum: owner.policy.response_bytes.get(),
                                                    observed: reply.bytes.len(),
                                                },
                                            ))))
                                        } else {
                                            Effect::from_result_once(Ok(Reply::from_finite(reply)))
                                        }
                                    }
                                }),
                        }
                    })
                })
        })
    }
    /// Produce a bounded exact key only after gate success; native authorization/
    /// cookie metadata and selected principal state impose conservative bypass.
    fn key(&self, env: &R, request: &Request) -> Result<AuthorizedKey, WebError> {
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
        // Bypassed renderers still receive the complete bounded target; omitting query
        // variation disables retention rather than skipping the key byte bound.
        let query = request.uri().query().map(Arc::from);
        let query_cacheable = self.policy.vary_query || query.is_none();
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
        let key = OutputKey {
            policy: self.policy.name.clone(),
            partition,
            path: Arc::from(request.uri().path()),
            query,
            headers: headers.into(),
        };
        let bytes = key
            .policy
            .len()
            .checked_add(key.path.len())
            .and_then(|n| n.checked_add(key.query.as_ref().map_or(0, |q| q.len())))
            .and_then(|n| n.checked_add(key.partition.namespace.len()))
            .and_then(|mut n| {
                for dimension in key.partition.dimensions.iter() {
                    n = n.checked_add(dimension.len())?;
                }
                for (name, values) in key.headers.iter() {
                    n = n.checked_add(name.as_str().len())?;
                    for value in values.iter() {
                        n = n.checked_add(value.as_bytes().len())?;
                    }
                }
                Some(n)
            });
        if bytes.is_none_or(|bytes| bytes > self.policy.key_bytes.get()) {
            return Err(WebError::Configuration("output key byte bound".into()));
        }
        let cacheable = query_cacheable
            && self.source.cacheable
            && matches!(*request.method(), Method::GET | Method::HEAD)
            && !(self.authenticated)(env)
            && !request.headers().contains_key(http::header::AUTHORIZATION)
            && !request.headers().contains_key(http::header::COOKIE);
        Ok(AuthorizedKey { key, cacheable })
    }
    /// Reuse Server's protocol-order validator handling on a finite reply. Callers
    /// must keep mutation preconditions in `gate` so hits cannot bypass them.
    pub fn conditional(
        reply: Reply<SharedValue<E>, R>,
        validators: ResourceValidators,
    ) -> Reply<SharedValue<E>, R> {
        effect_server::ReplyDraft::new()
            .validators(validators)
            .finish(reply)
    }
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
