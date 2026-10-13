//! Typed reversible routes and ordered registration descriptions.

use crate::registered;
use crate::{Endpoint, Pipeline, Registered, Registration, Request, Scalar, ServerError};
use effect::Effect;
use http::Method;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use std::collections::BTreeMap;
use std::sync::Arc;

/// RFC 3986 path-segment escaping includes delimiters and percent itself.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');
/// Captured native path values, percent-decoded exactly once with strict UTF-8.
pub type Captures = BTreeMap<String, String>;
/// Reversible typed path description. The same codecs govern handler values and reverse links.
pub struct Route<A: Send + 'static> {
    /// Finite native registrations including optional/default alternatives.
    pub(crate) patterns: Vec<String>,
    /// Typed decoder reads only the route's native captured values.
    decode: Arc<CaptureDecoder<A>>,
    /// Encoder borrows A without cloning values.
    encode: Arc<LinkEncoder<A>>,
    /// Registration-time metadata remains independent of requests.
    pub(crate) metadata: Registration,
}
impl<A: Send + 'static> Clone for Route<A> {
    /// Share codecs and immutable registration descriptions, never captured values.
    fn clone(&self) -> Self {
        Self {
            patterns: self.patterns.clone(),
            decode: self.decode.clone(),
            encode: self.encode.clone(),
            metadata: self.metadata.clone(),
        }
    }
}
impl Route<()> {
    /// Construct a literal native path; dynamic malformed paths return typed configuration failures.
    pub fn literal(path: &str) -> Result<Self, ServerError> {
        validate_literal(path)?;
        let encoded = path.to_owned();
        Ok(Self {
            patterns: vec![path.to_owned()],
            decode: Arc::new(|_| Ok(())),
            encode: Arc::new(move |()| Ok(encoded.clone())),
            metadata: Registration::default(),
        })
    }
}
impl<A: Send + 'static> Route<A> {
    /// Append a literal segment to every optional/default native registration.
    pub fn segment(mut self, path: &str) -> Result<Self, ServerError> {
        validate_literal(path)?;
        let segment = path.to_owned();
        for pattern in &mut self.patterns {
            *pattern = join(pattern, path);
        }
        let encode = self.encode.clone();
        self.encode = Arc::new(move |value| encode(value).map(|prefix| join(&prefix, &segment)));
        Ok(self)
    }
    /// Add a required typed capture. Multiple captures form nested tuples with static type relationships.
    pub fn capture<B: Send + 'static>(
        self,
        name: &str,
        scalar: Scalar<B>,
    ) -> Result<Route<(A, B)>, ServerError> {
        self.capture_with(name, scalar, None)
    }
    /// Apply an explicit pure native-string constraint before decoding; reverse links use the same check.
    pub fn capture_constrained<B: Send + 'static>(
        self,
        name: &str,
        scalar: Scalar<B>,
        constraint: impl Fn(&str) -> bool + Send + Sync + 'static,
    ) -> Result<Route<(A, B)>, ServerError> {
        self.capture_with(name, scalar, Some(Arc::new(constraint)))
    }
    /// Build a checked native capture without embedding an alternate route matcher.
    fn capture_with<B: Send + 'static>(
        self,
        name: &str,
        scalar: Scalar<B>,
        constraint: Option<Arc<CaptureConstraint>>,
    ) -> Result<Route<(A, B)>, ServerError> {
        validate_capture(name, &self.patterns)?;
        let name = name.to_owned();
        let decode_name = name.clone();
        let encode_name = name.clone();
        let decoder = self.decode.clone();
        let encoder = self.encode.clone();
        let decode_scalar = scalar.clone();
        let decode_constraint = constraint.clone();
        Ok(Route {
            patterns: self
                .patterns
                .iter()
                .map(|prefix| join(prefix, &format!("{{{name}}}")))
                .collect(),
            metadata: self.metadata,
            decode: Arc::new(move |captures| {
                let raw = captures
                    .get(&decode_name)
                    .ok_or_else(|| ServerError::Missing(decode_name.clone()))?;
                if decode_constraint.as_ref().is_some_and(|check| !check(raw)) {
                    return Err(ServerError::Invalid(decode_name.clone()));
                }
                Ok((decoder(captures)?, decode_scalar.decode(raw)?))
            }),
            encode: Arc::new(move |(prefix, value)| {
                let value = scalar.encode(value)?;
                if constraint.as_ref().is_some_and(|check| !check(&value)) {
                    return Err(ServerError::Invalid(encode_name.clone()));
                }
                Ok(join(
                    &encoder(prefix)?,
                    &utf8_percent_encode(&value, SEGMENT).to_string(),
                ))
            }),
        })
    }
    /// Add an optional final capture with a finite native registration for absence.
    pub fn optional_capture<B: Send + 'static>(
        self,
        name: &str,
        scalar: Scalar<B>,
    ) -> Result<Route<(A, Option<B>)>, ServerError> {
        validate_capture(name, &self.patterns)?;
        let name = name.to_owned();
        let decoder = self.decode.clone();
        let encoder = self.encode.clone();
        let decode_scalar = scalar.clone();
        let decode_name = name.clone();
        let mut patterns = self.patterns.clone();
        patterns.extend(
            self.patterns
                .iter()
                .map(|prefix| join(prefix, &format!("{{{name}}}"))),
        );
        Ok(Route {
            patterns,
            metadata: self.metadata,
            decode: Arc::new(move |captures| {
                Ok((
                    decoder(captures)?,
                    captures
                        .get(&decode_name)
                        .map(|raw| decode_scalar.decode(raw))
                        .transpose()?,
                ))
            }),
            encode: Arc::new(move |(prefix, value)| {
                let prefix = encoder(prefix)?;
                match value {
                    Some(value) => Ok(join(
                        &prefix,
                        &utf8_percent_encode(&scalar.encode(value)?, SEGMENT).to_string(),
                    )),
                    None => Ok(prefix),
                }
            }),
        })
    }
    /// Add a default capture; defaults are freshly constructed and checked through the same scalar encoder.
    pub fn default_capture<B: Send + 'static>(
        self,
        name: &str,
        scalar: Scalar<B>,
        default: impl Fn() -> B + Send + Sync + 'static,
    ) -> Result<Route<(A, B)>, ServerError> {
        validate_capture(name, &self.patterns)?;
        let name = name.to_owned();
        let decoder = self.decode.clone();
        let encoder = self.encode.clone();
        let decode_scalar = scalar.clone();
        let decode_name = name.clone();
        let mut patterns = self.patterns.clone();
        patterns.extend(
            self.patterns
                .iter()
                .map(|prefix| join(prefix, &format!("{{{name}}}"))),
        );
        Ok(Route {
            patterns,
            metadata: self.metadata,
            decode: Arc::new(move |captures| {
                let value = match captures.get(&decode_name) {
                    Some(raw) => decode_scalar.decode(raw)?,
                    None => default(),
                };
                decode_scalar.encode(&value)?;
                Ok((decoder(captures)?, value))
            }),
            encode: Arc::new(move |(prefix, value)| {
                Ok(join(
                    &encoder(prefix)?,
                    &utf8_percent_encode(&scalar.encode(value)?, SEGMENT).to_string(),
                ))
            }),
        })
    }
    /// Add a native terminal catch-all capture, preserving slash-separated link segments.
    pub fn catch_all(self, name: &str) -> Result<Route<(A, String)>, ServerError> {
        validate_capture(name, &self.patterns)?;
        let name = name.to_owned();
        let decoder = self.decode.clone();
        let encoder = self.encode.clone();
        let decode_name = name.clone();
        Ok(Route {
            patterns: self
                .patterns
                .iter()
                .map(|prefix| join(prefix, &format!("{{*{name}}}")))
                .collect(),
            metadata: self.metadata,
            decode: Arc::new(move |captures| {
                Ok((
                    decoder(captures)?,
                    captures
                        .get(&decode_name)
                        .cloned()
                        .ok_or_else(|| ServerError::Missing(decode_name.clone()))?,
                ))
            }),
            encode: Arc::new(move |(prefix, tail)| {
                Ok(join(
                    &encoder(prefix)?,
                    &tail
                        .split('/')
                        .map(|segment| utf8_percent_encode(segment, SEGMENT).to_string())
                        .collect::<Vec<_>>()
                        .join("/"),
                ))
            }),
        })
    }
    /// Adapt typed capture shape using explicit reversible borrowed projection.
    pub fn project<B: Send + 'static>(
        self,
        decode: impl Fn(A) -> Result<B, ServerError> + Send + Sync + 'static,
        encode: impl Fn(&B) -> Result<String, ServerError> + Send + Sync + 'static,
    ) -> Route<B> {
        let decoder = self.decode;
        Route {
            patterns: self.patterns,
            metadata: self.metadata,
            decode: Arc::new(move |captures| decode(decoder(captures)?)),
            encode: Arc::new(encode),
        }
    }
    /// Assign registration/link identity before serving.
    pub fn name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.metadata.name = Some(name.into());
        self
    }
    /// Assign registration tags before serving.
    #[must_use]
    pub fn tags(mut self, tags: Vec<Arc<str>>) -> Self {
        self.metadata.tags = tags;
        self
    }
    /// Apply native metadata conventions before mounting.
    pub fn configure(mut self, configure: impl FnOnce(&mut Registration)) -> Self {
        configure(&mut self.metadata);
        self
    }
    /// Generate a link from the same typed capture shape and scalar codecs as the handler.
    pub fn path(&self, parameters: &A) -> Result<String, ServerError> {
        (self.encode)(parameters)
    }
    /// Decode native captures into the statically required handler value.
    pub(crate) fn decode(&self, captures: &Captures) -> Result<A, ServerError> {
        (self.decode)(captures)
    }
}
/// Require native path syntax without user captures in literal operations.
fn validate_literal(path: &str) -> Result<(), ServerError> {
    if !path.starts_with('/') || path.contains(['{', '}', '?', '#']) {
        Err(ServerError::Configuration("literal route path".into()))
    } else {
        Ok(())
    }
}
/// Reject duplicate or malformed capture names before native registration.
fn validate_capture(name: &str, patterns: &[String]) -> Result<(), ServerError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        || patterns
            .iter()
            .any(|pattern| pattern.contains(&format!("{{{name}}}")) || pattern.contains("{*"))
    {
        Err(ServerError::Configuration("capture name/order".into()))
    } else {
        Ok(())
    }
}
/// Compose native path fragments without changing encoded capture values.
fn join(prefix: &str, child: &str) -> String {
    format!(
        "{}/{}",
        prefix.trim_end_matches('/'),
        child.trim_start_matches('/')
    )
}
/// A nonempty native method set, preserving extension-token spelling/case.
#[derive(Clone)]
pub struct HttpMethodSet(Vec<Method>);
impl HttpMethodSet {
    /// Validate a nonempty unique set of native method tokens.
    pub fn new(methods: Vec<Method>) -> Result<Self, ServerError> {
        if methods.is_empty()
            || methods
                .iter()
                .enumerate()
                .any(|(index, method)| methods.iter().take(index).any(|other| other == method))
        {
            Err(ServerError::Configuration("nonempty unique methods".into()))
        } else {
            Ok(Self(methods))
        }
    }
    /// Borrow native methods without string normalization.
    #[must_use]
    pub fn methods(&self) -> &[Method] {
        &self.0
    }
}
/// Inert route registration with a type-checked handler factory.
pub(crate) struct RouteEntry<E: Send + 'static, R: Send + Sync + 'static> {
    /// Native method matching is authoritative after registration.
    pub(crate) methods: HttpMethodSet,
    /// Native route patterns including explicit optional alternatives.
    pub(crate) patterns: Vec<String>,
    /// Typed captures become a completed endpoint before request execution.
    pub(crate) endpoint: Arc<EndpointSelector<E, R>>,
    /// Registration metadata is installed before environment resolution.
    pub(crate) metadata: Registration,
}
/// Ordered router description. Native axum routing owns 404/405/HEAD behavior.
pub struct Router<E: Send + 'static, R: Send + Sync + 'static> {
    /// Ordered typed/native registrations.
    pub(crate) entries: Vec<RouteEntry<E, R>>,
    /// Native configuration hooks preserve Tower services and application layers.
    pub(crate) native: Vec<Arc<RouterConfigurator>>,
    /// Native nested groups retain their actual child-router configuration scope.
    pub(crate) groups: Vec<RouteGroup<E, R>>,
    /// Effect fallback remains scoped and distinct from native method-not-allowed handling.
    pub(crate) fallback: Option<Endpoint<E, R>>,
    /// Outer pipeline applies to children in declaration/nesting order.
    pub(crate) pipeline: Pipeline<E, R>,
    /// Scoped expected-error policy propagated only into these entries.
    pub(crate) on_error: Option<Arc<RouteErrorRenderer<E, R>>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Default for Router<E, R> {
    /// Empty router preserves native fallback semantics.
    fn default() -> Self {
        Self::new()
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Router<E, R> {
    /// Construct an inert ordered router.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            native: Vec::new(),
            groups: Vec::new(),
            fallback: None,
            pipeline: Pipeline::new(),
            on_error: None,
        }
    }
    /// Register a typed route and handler whose input shape must match its codecs.
    pub fn methods<A: Send + 'static>(
        mut self,
        methods: HttpMethodSet,
        route: Route<A>,
        handler: impl Fn(A) -> Endpoint<E, R> + Send + Sync + 'static,
    ) -> Self {
        let patterns = route.patterns.clone();
        let metadata = route.metadata.clone();
        self.entries.push(RouteEntry {
            methods,
            patterns,
            endpoint: Arc::new(move |captures| route.decode(&captures).map(&handler)),
            metadata,
        });
        self
    }
    /// Register a constant-path endpoint with separate native metadata.
    pub fn registered(
        mut self,
        methods: HttpMethodSet,
        path: &str,
        registration: Registered<E, R>,
    ) -> Result<Self, ServerError> {
        validate_literal(path)?;
        let endpoint = registration.endpoint;
        self.entries.push(RouteEntry {
            methods,
            patterns: vec![path.to_owned()],
            endpoint: Arc::new(move |_| Ok(endpoint.clone())),
            metadata: registration.metadata,
        });
        Ok(self)
    }
    /// Merge a reusable ordered route module without evaluating request programs.
    #[must_use]
    pub fn include(mut self, mut routes: Self) -> Self {
        let pipeline = routes.pipeline.clone();
        let policy = routes.on_error.clone();
        for entry in &mut routes.entries {
            let handler = entry.endpoint.clone();
            let pipeline = pipeline.clone();
            let policy = policy.clone();
            entry.endpoint = Arc::new(move |captures| {
                let mut endpoint = handler(captures)?.pipe_through(pipeline.clone());
                if endpoint.on_error.is_none() {
                    endpoint.on_error = policy.clone();
                }
                Ok(endpoint)
            });
        }
        for group in &mut routes.groups {
            group.child.pipeline = routes.pipeline.clone().append(group.child.pipeline.clone());
            if group.child.on_error.is_none() {
                group.child.on_error = routes.on_error.clone();
            }
        }
        self.entries.append(&mut routes.entries);
        self.native.append(&mut routes.native);
        self.groups.append(&mut routes.groups);
        if let Some(endpoint) = routes.fallback {
            self.fallback = Some(endpoint.pipe_through(routes.pipeline));
        }
        self
    }
    /// Mount a child native router under its scoped prefix rather than flattening its hooks globally.
    pub fn group(mut self, prefix: &str, child: Self) -> Result<Self, ServerError> {
        validate_literal(prefix)?;
        self.groups.push(RouteGroup {
            prefix: prefix.to_owned(),
            child: Box::new(child),
        });
        Ok(self)
    }
    /// Configure the actual child native router before nesting under the group prefix.
    pub fn group_configured(
        self,
        prefix: &str,
        configure: impl Fn(axum::Router) -> Result<axum::Router, ServerError> + Send + Sync + 'static,
        child: Self,
    ) -> Result<Self, ServerError> {
        self.group(prefix, child.native(configure))
    }
    /// Apply a native axum/Tower service/layer transformation before serving.
    pub fn native(
        mut self,
        configure: impl Fn(axum::Router) -> Result<axum::Router, ServerError> + Send + Sync + 'static,
    ) -> Self {
        self.native.push(Arc::new(configure));
        self
    }
    /// Select a typed fallback without replacing native 405 behavior.
    #[must_use]
    pub fn fallback(mut self, endpoint: Endpoint<E, R>) -> Self {
        self.fallback = Some(endpoint);
        self
    }
    /// Scope a middleware pipeline to all entries currently in this router.
    #[must_use]
    pub fn pipe_through(mut self, pipeline: Pipeline<E, R>) -> Self {
        self.pipeline = pipeline;
        self
    }
    /// Scope expected-error rendering; a renderer failure does not reenter itself.
    pub fn on_error(
        mut self,
        on_error: impl Fn(E) -> Effect<crate::Reply<E, R>, E, R> + Send + Sync + 'static,
    ) -> Self {
        self.on_error = Some(Arc::new(on_error));
        self
    }
}
/// Native nested registration scope; configuration applies only to this actual child router.
pub(crate) struct RouteGroup<E: Send + 'static, R: Send + Sync + 'static> {
    /// Checked absolute native mount prefix.
    pub(crate) prefix: String,
    /// Owned child registration/policy description.
    pub(crate) child: Box<Router<E, R>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RouteEntry<E, R> {
    /// Share registration factories/configuration without cloning capture/application values.
    fn clone(&self) -> Self {
        Self {
            methods: self.methods.clone(),
            patterns: self.patterns.clone(),
            endpoint: self.endpoint.clone(),
            metadata: self.metadata.clone(),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for Router<E, R> {
    /// A fresh host mounts shared immutable registration descriptions; no request bodies are copied.
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            native: self.native.clone(),
            groups: self
                .groups
                .iter()
                .map(|group| RouteGroup {
                    prefix: group.prefix.clone(),
                    child: Box::new(group.child.as_ref().clone()),
                })
                .collect(),
            fallback: self.fallback.clone(),
            pipeline: self.pipeline.clone(),
            on_error: self.on_error.clone(),
        }
    }
}
/// Reusable route-set assembly; descriptions are ordered, distinct from execution.
pub type Routes<E, R> = Router<E, R>;
/// Request-local native captured path values, installed before selected environment resolution.
#[derive(Clone)]
pub(crate) struct NativeCaptures(pub(crate) Captures);
/// Read one decoded path capture from the original native registration.
pub fn path_capture(request: &Request, name: &str) -> Result<String, ServerError> {
    request
        .with_extensions(|extensions| {
            extensions
                .get::<NativeCaptures>()
                .and_then(|captures| captures.0.get(name))
                .cloned()
        })
        .ok_or_else(|| ServerError::Missing(name.to_owned()))
}

impl Route<()> {
    /// Decode the first capture directly as B; later captures append tuple components.
    pub fn capture_first<B: Send + 'static>(
        self,
        name: &str,
        scalar: Scalar<B>,
    ) -> Result<Route<B>, ServerError> {
        self.first_capture(name, scalar, None)
    }
    /// First constrained capture retains typed reverse-link validation.
    pub fn constrained_first<B: Send + 'static>(
        self,
        name: &str,
        scalar: Scalar<B>,
        constraint: impl Fn(&str) -> bool + Send + Sync + 'static,
    ) -> Result<Route<B>, ServerError> {
        self.first_capture(name, scalar, Some(Arc::new(constraint)))
    }
    /// Build first-capture codecs without materializing or cloning a tuple payload.
    fn first_capture<B: Send + 'static>(
        self,
        name: &str,
        scalar: Scalar<B>,
        constraint: Option<Arc<CaptureConstraint>>,
    ) -> Result<Route<B>, ServerError> {
        validate_capture(name, &self.patterns)?;
        let name = name.to_owned();
        let decode_name = name.clone();
        let encoder = self.encode;
        let decode_scalar = scalar.clone();
        let decode_constraint = constraint.clone();
        Ok(Route {
            patterns: self
                .patterns
                .iter()
                .map(|prefix| join(prefix, &format!("{{{name}}}")))
                .collect(),
            metadata: self.metadata,
            decode: Arc::new(move |captures| {
                let raw = captures
                    .get(&decode_name)
                    .ok_or_else(|| ServerError::Missing(decode_name.clone()))?;
                if decode_constraint.as_ref().is_some_and(|check| !check(raw)) {
                    return Err(ServerError::Invalid(decode_name.clone()));
                }
                decode_scalar.decode(raw)
            }),
            encode: Arc::new(move |value| {
                let text = scalar.encode(value)?;
                if constraint.as_ref().is_some_and(|check| !check(&text)) {
                    return Err(ServerError::Invalid(name.clone()));
                }
                Ok(join(
                    &encoder(&())?,
                    &utf8_percent_encode(&text, SEGMENT).to_string(),
                ))
            }),
        })
    }
    /// Decode the first optional capture as `Option<B>`, preserving absent versus malformed values.
    pub fn optional_first<B: Send + 'static>(
        self,
        name: &str,
        scalar: Scalar<B>,
    ) -> Result<Route<Option<B>>, ServerError> {
        validate_capture(name, &self.patterns)?;
        let name = name.to_owned();
        let encoder = self.encode;
        let decode_scalar = scalar.clone();
        let decode_name = name.clone();
        let mut patterns = self.patterns.clone();
        patterns.extend(
            self.patterns
                .iter()
                .map(|prefix| join(prefix, &format!("{{{name}}}"))),
        );
        Ok(Route {
            patterns,
            metadata: self.metadata,
            decode: Arc::new(move |captures| {
                captures
                    .get(&decode_name)
                    .map(|raw| decode_scalar.decode(raw))
                    .transpose()
            }),
            encode: Arc::new(move |value| match value {
                Some(value) => Ok(join(
                    &encoder(&())?,
                    &utf8_percent_encode(&scalar.encode(value)?, SEGMENT).to_string(),
                )),
                None => encoder(&()),
            }),
        })
    }
    /// First default capture constructs a fresh fallback and checks its reverse encoding.
    pub fn default_first<B: Send + 'static>(
        self,
        name: &str,
        scalar: Scalar<B>,
        default: impl Fn() -> B + Send + Sync + 'static,
    ) -> Result<Route<B>, ServerError> {
        let optional = self.optional_first(name, scalar.clone())?;
        let decoder = optional.decode;
        let encoder_prefix = self_literal_prefix(&optional.patterns);
        let encode_scalar = scalar.clone();
        Ok(Route {
            patterns: optional.patterns,
            metadata: optional.metadata,
            decode: Arc::new(move |captures| {
                let value = decoder(captures)?.unwrap_or_else(&default);
                scalar.encode(&value)?;
                Ok(value)
            }),
            encode: Arc::new(move |value| {
                Ok(join(
                    &encoder_prefix,
                    &utf8_percent_encode(&encode_scalar.encode(value)?, SEGMENT).to_string(),
                ))
            }),
        })
    }
    /// First native catch-all returns its decoded string directly.
    pub fn catch_all_first(self, name: &str) -> Result<Route<String>, ServerError> {
        validate_capture(name, &self.patterns)?;
        let name = name.to_owned();
        let encoder = self.encode;
        Ok(Route {
            patterns: self
                .patterns
                .iter()
                .map(|prefix| join(prefix, &format!("{{*{name}}}")))
                .collect(),
            metadata: self.metadata,
            decode: Arc::new(move |captures| {
                captures
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| ServerError::Missing(name.clone()))
            }),
            encode: Arc::new(move |tail| {
                Ok(join(
                    &encoder(&())?,
                    &tail
                        .split('/')
                        .map(|segment| utf8_percent_encode(segment, SEGMENT).to_string())
                        .collect::<Vec<_>>()
                        .join("/"),
                ))
            }),
        })
    }
}
/// Recover the original literal prefix from the absence registration of a first default capture.
fn self_literal_prefix(patterns: &[String]) -> String {
    patterns.first().cloned().unwrap_or_default()
}
/// Typed mounting dispatch used by router macros; wrong handler shapes fail Rust type checking.
pub trait RouteMount<E: Send + 'static, R: Send + Sync + 'static, H> {
    /// Consume one registration description without running request work.
    fn mount(
        self,
        router: Router<E, R>,
        methods: HttpMethodSet,
        handler: H,
    ) -> Result<Router<E, R>, ServerError>;
}
impl<A, E, R, H> RouteMount<E, R, H> for Route<A>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    H: Fn(A) -> Endpoint<E, R> + Send + Sync + 'static,
{
    /// Type-check the capture-to-handler relationship before mounting native routes.
    fn mount(
        self,
        router: Router<E, R>,
        methods: HttpMethodSet,
        handler: H,
    ) -> Result<Router<E, R>, ServerError> {
        Ok(router.methods(methods, self, handler))
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> RouteMount<E, R, Endpoint<E, R>> for &str {
    /// Constant paths accept completed endpoints without inventing captured arguments.
    fn mount(
        self,
        router: Router<E, R>,
        methods: HttpMethodSet,
        handler: Endpoint<E, R>,
    ) -> Result<Router<E, R>, ServerError> {
        router.registered(methods, self, registered! { endpoint: handler, })
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> RouteMount<E, R, Registered<E, R>> for &str {
    /// Constant-path registration metadata is installed before per-request execution.
    fn mount(
        self,
        router: Router<E, R>,
        methods: HttpMethodSet,
        handler: Registered<E, R>,
    ) -> Result<Router<E, R>, ServerError> {
        router.registered(methods, self, handler)
    }
}
/// Resolve typed mounting through ordinary Rust traits; no method-name interpreter exists.
pub fn mount_route<E, R, H>(
    router: Router<E, R>,
    methods: HttpMethodSet,
    route: impl RouteMount<E, R, H>,
    handler: H,
) -> Result<Router<E, R>, ServerError>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    route.mount(router, methods, handler)
}
/// Convert extracted route modules and checked construction results into ordered router assembly.
pub trait IntoRouter<E: Send + 'static, R: Send + Sync + 'static> {
    /// Retain typed configuration errors rather than unwrapping descriptions.
    fn into_router(self) -> Result<Router<E, R>, ServerError>;
}
impl<E: Send + 'static, R: Send + Sync + 'static> IntoRouter<E, R> for Router<E, R> {
    /// An already checked router is inert and ready to assemble.
    fn into_router(self) -> Result<Router<E, R>, ServerError> {
        Ok(self)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> IntoRouter<E, R>
    for Result<Router<E, R>, ServerError>
{
    /// Preserve an extracted module's actual configuration failure.
    fn into_router(self) -> Result<Router<E, R>, ServerError> {
        self
    }
}

/// Typed capture decoder retaining malformed-input errors.
type CaptureDecoder<A> = dyn Fn(&Captures) -> Result<A, ServerError> + Send + Sync;

/// Reverse-link encoder borrowing the route value.
type LinkEncoder<A> = dyn Fn(&A) -> Result<String, ServerError> + Send + Sync;

/// Native capture constraint before typed decoding.
type CaptureConstraint = dyn Fn(&str) -> bool + Send + Sync;

/// Endpoint selection from owned native captures.
type EndpointSelector<E, R> = dyn Fn(Captures) -> Result<Endpoint<E, R>, ServerError> + Send + Sync;

/// Native router configuration during description assembly.
type RouterConfigurator = dyn Fn(axum::Router) -> Result<axum::Router, ServerError> + Send + Sync;

/// Scoped route error renderer.
type RouteErrorRenderer<E, R> = dyn Fn(E) -> Effect<crate::Reply<E, R>, E, R> + Send + Sync;
