//! Checked scalar inputs and single-owner native request bodies.

use axum::body::Body;
use axum::extract::{FromRequest, Multipart};
use bytes::Bytes;
use effect::{Cancellation, Effect, HasCancellation, HasTaskSpawner};
use headers::{Header, HeaderMapExt};
use http::{Extensions, HeaderMap, HeaderValue, Method, Uri, Version};
use http_body_util::{BodyExt, Limited};
use std::collections::BTreeMap;
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// HTTP input/configuration failures; applications explicitly map these into E.
#[derive(Debug)]
pub enum ServerError {
    /// A required scalar or body was absent.
    Missing(String),
    /// A scalar field occurred more often than its declared cardinality.
    Duplicate(String),
    /// Native syntax, encoding or a refinement was rejected.
    Invalid(String),
    /// A caller-selected byte/count/text bound was exceeded.
    Limit(String),
    /// Another incompatible decoder already owns the native body.
    BodyClaimed,
    /// A compatible form cannot be reused under a different parse policy.
    FormPolicyConflict,
    /// The peer disconnected before the operation settled.
    Disconnected,
    /// Native connection transport failed with its retained defect diagnostics.
    Transport(effect::Defect),
    /// Native transport or file I/O failed, retaining its source.
    Io(std::io::Error),
    /// Native multipart decoding failed.
    Multipart(axum::extract::multipart::MultipartError),
    /// Native multipart extraction rejected the request.
    MultipartRejection(String),
    /// A compatible form shares its typed failed parse without cloning the error payload.
    SharedInput(Arc<ServerError>),
    /// A core codec retained its structured issue path.
    Schema(effect::SchemaIssue),
    /// Configuration descriptions were inconsistent before serving.
    Configuration(String),
}
impl std::fmt::Display for ServerError {
    /// Render safe category/context without request payloads or secret values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(name) => write!(f, "missing {name}"),
            Self::Duplicate(name) => write!(f, "duplicate {name}"),
            Self::Invalid(name) => write!(f, "invalid {name}"),
            Self::Limit(name) => write!(f, "limit exceeded: {name}"),
            Self::BodyClaimed => f.write_str("request body already claimed"),
            Self::FormPolicyConflict => f.write_str("incompatible form policy"),
            Self::Disconnected => f.write_str("peer disconnected"),
            Self::Transport(defect) => write!(f, "native transport: {}", defect.message()),
            Self::Io(error) => write!(f, "native I/O: {error}"),
            Self::Multipart(_) | Self::MultipartRejection(_) => {
                f.write_str("invalid multipart body")
            }
            Self::Schema(_) => f.write_str("body codec rejected input"),
            Self::SharedInput(error) => write!(f, "{error}"),
            Self::Configuration(name) => write!(f, "invalid server configuration: {name}"),
        }
    }
}
impl std::error::Error for ServerError {}

/// A positive total body bound, measured in encoded bytes before decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyLimit(NonZeroUsize);
impl BodyLimit {
    /// Select the maximum encoded body bytes; zero is not a valid resource bound.
    #[must_use]
    pub const fn new(bytes: NonZeroUsize) -> Self {
        Self(bytes)
    }
    /// Return the selected maximum bytes, never a hidden framework default.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self.0.get()
    }
}
/// Text bounds explicitly distinguish UTF-8 bytes and Unicode scalar values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextUnit {
    /// Encoded UTF-8 bytes.
    Bytes,
    /// Unicode scalar values; this is not a grapheme/display-width measure.
    Scalars,
}
/// Caller-selected scalar-field bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldBounds {
    /// Maximum repetitions of one field.
    pub count: NonZeroUsize,
    /// Maximum size of each value in `unit`.
    pub length: NonZeroUsize,
    /// Unit for `length`.
    pub unit: TextUnit,
}
impl FieldBounds {
    /// Check repetition and individual lengths before scalar decoding.
    fn check(self, name: &str, values: &[String]) -> Result<(), ServerError> {
        if values.len() > self.count.get() {
            return Err(ServerError::Limit(format!("{name} count")));
        }
        if values.iter().any(|value| match self.unit { TextUnit::Bytes => value.len(), TextUnit::Scalars => value.chars().count() } > self.length.get()) {
            return Err(ServerError::Limit(format!("{name} length")));
        }
        Ok(())
    }
}
/// Reversible scalar codec; the same encoder is used by typed reverse links.
pub struct Scalar<A> {
    /// Pure native-string decoder with typed input failures.
    decode: Arc<ScalarDecoder<A>>,
    /// Link/form encoding does not clone application values.
    encode: Arc<ScalarEncoder<A>>,
}
impl<A> Clone for Scalar<A> {
    /// Share immutable decoder/encoder descriptions, never A.
    fn clone(&self) -> Self {
        Self {
            decode: self.decode.clone(),
            encode: self.encode.clone(),
        }
    }
}
impl<A: 'static> Scalar<A> {
    /// Construct a reversible native scalar codec without invoking it.
    pub fn new(
        decode: impl Fn(&str) -> Result<A, ServerError> + Send + Sync + 'static,
        encode: impl Fn(&A) -> Result<String, ServerError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            decode: Arc::new(decode),
            encode: Arc::new(encode),
        }
    }
    /// Decode one value, retaining its native typed failure.
    pub fn decode(&self, value: &str) -> Result<A, ServerError> {
        (self.decode)(value)
    }
    /// Encode a borrowed value without payload copies.
    pub fn encode(&self, value: &A) -> Result<String, ServerError> {
        (self.encode)(value)
    }
    /// Refine decoded values and reverse-link inputs with the same predicate.
    pub fn refine(
        self,
        name: impl Into<Arc<str>>,
        predicate: impl Fn(&A) -> bool + Send + Sync + 'static,
    ) -> Self {
        let name = name.into();
        let predicate = Arc::new(predicate);
        let decoder = self.clone();
        let decode_predicate = predicate.clone();
        let decode_name = name.clone();
        Self::new(
            move |text| {
                let value = decoder.decode(text)?;
                if decode_predicate(&value) {
                    Ok(value)
                } else {
                    Err(ServerError::Invalid(decode_name.to_string()))
                }
            },
            move |value| {
                if predicate(value) {
                    self.encode(value)
                } else {
                    Err(ServerError::Invalid(name.to_string()))
                }
            },
        )
    }
}
/// Input field source; native structured headers use `Request::typed_header`.
#[derive(Clone, Copy)]
pub enum FieldSource {
    /// URL query pairs decoded with strict percent/UTF-8 validation.
    Query,
    /// Raw repeated header values, preserving native HeaderMap cardinality.
    Header,
    /// Parsed Cookie header values.
    Cookie,
    /// Compatible URL-encoded form snapshot.
    Form,
}
/// A typed scalar field with explicit source and resource bounds.
pub struct Field<A> {
    /// Field identity within its declared source.
    name: Arc<str>,
    /// Scalar source, never inferred from a hidden global request.
    source: FieldSource,
    /// Checked per-field resource policy.
    bounds: FieldBounds,
    /// Decoder and reverse encoder shared by descriptions.
    scalar: Scalar<A>,
}
impl<A: 'static> Field<A> {
    /// Describe a scalar input without reading a request.
    pub fn new(
        source: FieldSource,
        name: impl Into<Arc<str>>,
        bounds: FieldBounds,
        scalar: Scalar<A>,
    ) -> Self {
        Self {
            name: name.into(),
            source,
            bounds,
            scalar,
        }
    }
    /// Extract exactly one value; duplicates are errors rather than arbitrary selection.
    pub fn required(&self, request: &Request, form: Option<&FormData>) -> Result<A, ServerError> {
        self.optional(request, form)?
            .ok_or_else(|| ServerError::Missing(self.name.to_string()))
    }
    /// Extract zero or one value; malformed and duplicate values are never absence.
    pub fn optional(
        &self,
        request: &Request,
        form: Option<&FormData>,
    ) -> Result<Option<A>, ServerError> {
        let values = self.values(request, form)?;
        if values.len() > 1 {
            return Err(ServerError::Duplicate(self.name.to_string()));
        }
        values
            .first()
            .map(|value| self.scalar.decode(value))
            .transpose()
    }
    /// Decode all declared repetitions in source order.
    pub fn many(&self, request: &Request, form: Option<&FormData>) -> Result<Vec<A>, ServerError> {
        self.values(request, form)?
            .iter()
            .map(|value| self.scalar.decode(value))
            .collect()
    }
    /// Require a nonempty repeated field while retaining decode failures.
    pub fn nonempty(
        &self,
        request: &Request,
        form: Option<&FormData>,
    ) -> Result<Vec<A>, ServerError> {
        let values = self.many(request, form)?;
        if values.is_empty() {
            Err(ServerError::Missing(self.name.to_string()))
        } else {
            Ok(values)
        }
    }
    /// Apply a fresh default only for genuine absence.
    pub fn with_default(
        &self,
        request: &Request,
        form: Option<&FormData>,
        default: impl FnOnce() -> A,
    ) -> Result<A, ServerError> {
        let value = self.optional(request, form)?.unwrap_or_else(default);
        self.scalar.encode(&value)?;
        Ok(value)
    }
    /// Presence-only flags still reject duplicates and values other than empty/true/false.
    pub fn flag(&self, request: &Request, form: Option<&FormData>) -> Result<bool, ServerError> {
        let values = self.values(request, form)?;
        match values.as_slice() {
            [] => Ok(false),
            [value] if value.is_empty() || value == "true" => Ok(true),
            [value] if value == "false" => Ok(false),
            [_] => Err(ServerError::Invalid(self.name.to_string())),
            _ => Err(ServerError::Duplicate(self.name.to_string())),
        }
    }
    /// Read native source values and apply bounds before caller code runs.
    fn values(
        &self,
        request: &Request,
        form: Option<&FormData>,
    ) -> Result<Vec<String>, ServerError> {
        let values = match self.source {
            FieldSource::Query => parse_pairs(request.uri().query().unwrap_or(""))?
                .remove(self.name.as_ref())
                .unwrap_or_default(),
            FieldSource::Header => request
                .headers()
                .get_all(self.name.as_ref())
                .iter()
                .map(|value| {
                    value
                        .to_str()
                        .map(str::to_owned)
                        .map_err(|_| ServerError::Invalid(self.name.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?,
            FieldSource::Cookie => {
                let mut values = Vec::new();
                for header in request.headers().get_all(http::header::COOKIE) {
                    let text = header
                        .to_str()
                        .map_err(|_| ServerError::Invalid("cookie".into()))?;
                    for parsed in cookie::Cookie::split_parse(text) {
                        let parsed = parsed.map_err(|_| ServerError::Invalid("cookie".into()))?;
                        if parsed.name() == self.name.as_ref() {
                            values.push(parsed.value().to_owned());
                        }
                    }
                }
                values
            }
            FieldSource::Form => form
                .ok_or(ServerError::BodyClaimed)?
                .values
                .get(self.name.as_ref())
                .cloned()
                .unwrap_or_default(),
        };
        self.bounds.check(&self.name, &values)?;
        Ok(values)
    }
}
/// Shared compatible form snapshot; body bytes are not retained alongside it.
#[derive(Debug)]
pub struct FormData {
    /// Decoded fields preserving repeated-value order.
    pub values: BTreeMap<String, Vec<String>>,
}
/// Explicit total/field/count policy defining compatible form parse identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormPolicy {
    /// Maximum complete encoded form bytes.
    pub body: BodyLimit,
    /// Maximum distinct field names.
    pub fields: NonZeroUsize,
    /// Bounds for each repeated field value.
    pub values: FieldBounds,
}
/// Body claim state moves the native Body exactly once; compatible parses share a cell.
enum BodyState {
    /// An unread native body is available for one claim.
    Unread(Option<Body>),
    /// A native/bytes/text/JSON/multipart decoder owns it.
    Exclusive,
    /// Native routing temporarily owns the unread body; it returns at endpoint extraction.
    Routing,
    /// Same-policy form reads share one acknowledged parse.
    Form {
        policy: FormPolicy,
        result: Arc<tokio::sync::Mutex<ParsedForm>>,
    },
}
/// Compatible form owner serializes the one parse and retains its typed acknowledged result.
struct ParsedForm {
    /// Native body transfers into the first parse future only.
    body: Option<Body>,
    /// Completed parse result shared without copying form/error payloads.
    result: Option<Result<Arc<FormData>, Arc<ServerError>>>,
}
/// Native request metadata and checked body owner shared by one request only.
struct RequestState {
    /// Original parts remain intact; only service metadata is shared.
    parts: http::request::Parts,
    /// Explicit extension insertions support native auth/culture/telemetry stages.
    extensions: Arc<Mutex<Extensions>>,
    /// Short ownership transition lock; never held while awaiting native I/O.
    body: Arc<Mutex<BodyState>>,
    /// Body/disconnect cancellation source.
    cancellation: Cancellation,
}
/// Fresh request state; cloning shares metadata and its single body claim.
#[derive(Clone)]
pub struct Request {
    /// One request owner shared with selected application environment.
    state: Arc<RequestState>,
}
impl Request {
    /// Transfer an axum request into a fresh owner; no body or extension payload is cloned.
    pub fn from_native(request: http::Request<Body>, cancellation: Cancellation) -> Self {
        let (mut parts, body) = request.into_parts();
        if let Some(shared) = parts.extensions.remove::<RoutingSnapshot>() {
            let request = shared.0.with_parts(parts.clone());
            request
                .state
                .extensions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend(std::mem::take(&mut parts.extensions));
            let mut state = request
                .state
                .body
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if matches!(*state, BodyState::Routing) {
                *state = BodyState::Unread(Some(body));
            }
            drop(state);
            return request;
        }
        let extensions = std::mem::take(&mut parts.extensions);
        Self {
            state: Arc::new(RequestState {
                parts,
                extensions: Arc::new(Mutex::new(extensions)),
                body: Arc::new(Mutex::new(BodyState::Unread(Some(body)))),
                cancellation,
            }),
        }
    }
    /// Rebuild only immutable native metadata while sharing the exact body/extension owner.
    fn with_parts(&self, mut parts: http::request::Parts) -> Self {
        parts.extensions = Extensions::new();
        Self {
            state: Arc::new(RequestState {
                parts,
                extensions: self.state.extensions.clone(),
                body: self.state.body.clone(),
                cancellation: self.state.cancellation.clone(),
            }),
        }
    }
    /// Borrow the original method without cloning request parts.
    #[must_use]
    pub fn method(&self) -> &Method {
        &self.state.parts.method
    }
    /// Borrow the original URI.
    #[must_use]
    pub fn uri(&self) -> &Uri {
        &self.state.parts.uri
    }
    /// Borrow the original headers; structured parsing retains native syntax semantics.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.state.parts.headers
    }
    /// Read the native protocol selected for this connection.
    #[must_use]
    pub fn version(&self) -> Version {
        self.state.parts.version
    }
    /// Observe the request's cooperative cancellation source.
    #[must_use]
    pub fn cancellation(&self) -> &Cancellation {
        &self.state.cancellation
    }
    /// Borrow selected native extensions only while the metadata guard is live.
    pub fn with_extensions<A>(&self, read: impl FnOnce(&Extensions) -> A) -> A {
        read(
            &self
                .state
                .extensions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
    /// Install validated native stage output; this operation itself grants no authentication authority.
    pub fn insert_extension<T: Clone + Send + Sync + 'static>(&self, value: T) {
        self.state
            .extensions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(value);
    }
    /// Decode a native structured header, distinguishing absence from malformed syntax.
    pub fn typed_header<H: Header>(&self) -> Result<Option<H>, ServerError> {
        self.headers()
            .typed_try_get::<H>()
            .map_err(|_| ServerError::Invalid(H::name().to_string()))
    }
    /// Decode a singleton structured field, rejecting duplicate wire occurrences explicitly.
    pub fn singleton_header<H: Header>(&self) -> Result<Option<H>, ServerError> {
        if self.headers().get_all(H::name()).iter().count() > 1 {
            return Err(ServerError::Duplicate(H::name().to_string()));
        }
        self.typed_header::<H>()
    }
    /// Atomically claim the native body. The receiving future owns it through completion.
    fn claim(&self) -> Result<Body, ServerError> {
        let mut state = self
            .state
            .body
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut *state {
            BodyState::Unread(body) => {
                let body = body.take().ok_or(ServerError::BodyClaimed)?;
                *state = BodyState::Exclusive;
                Ok(body)
            }
            BodyState::Exclusive | BodyState::Routing | BodyState::Form { .. } => {
                Err(ServerError::BodyClaimed)
            }
        }
    }
    /// Read finite encoded bytes under the selected bound, with exclusive ownership.
    pub async fn read_bytes(&self, limit: BodyLimit) -> Result<Bytes, ServerError> {
        read_bounded(self.claim()?, limit).await
    }
    /// Decode strict UTF-8; replacement characters are not inserted for malformed bytes.
    pub async fn read_text(&self, limit: BodyLimit) -> Result<String, ServerError> {
        String::from_utf8(self.read_bytes(limit).await?.to_vec())
            .map_err(|_| ServerError::Invalid("UTF-8 body".into()))
    }
    /// Read strict bounded JSON directly through the core codec parser.
    pub async fn read_json<A: Send + 'static>(
        &self,
        limit: BodyLimit,
        policy: effect::ParsePolicy,
        codec: &effect::PureCodec<A, effect::JsonValue>,
    ) -> Result<A, ServerError> {
        codec
            .parse_utf8(&self.read_bytes(limit).await?, policy)
            .map_err(ServerError::Schema)
    }
    /// Share one compatible URL-encoded parse; incompatible and exclusive claims fail explicitly.
    pub async fn read_form(&self, policy: FormPolicy) -> Result<Arc<FormData>, ServerError> {
        let cell = {
            let mut state = self
                .state
                .body
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &mut *state {
                BodyState::Unread(body) => {
                    let body = body.take().ok_or(ServerError::BodyClaimed)?;
                    let result = Arc::new(tokio::sync::Mutex::new(ParsedForm {
                        body: Some(body),
                        result: None,
                    }));
                    *state = BodyState::Form {
                        policy,
                        result: result.clone(),
                    };
                    result
                }
                BodyState::Form {
                    policy: existing,
                    result,
                } if *existing == policy => result.clone(),
                BodyState::Form { .. } => return Err(ServerError::FormPolicyConflict),
                BodyState::Exclusive | BodyState::Routing => return Err(ServerError::BodyClaimed),
            }
        };
        let mut parsed = cell.lock().await;
        if let Some(result) = &parsed.result {
            return result.clone().map_err(ServerError::SharedInput);
        }
        let body = parsed.body.take().ok_or(ServerError::Disconnected)?;
        let result = async {
            let bytes = read_bounded(body, policy.body).await?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| ServerError::Invalid("form UTF-8".into()))?;
            let values = parse_pairs(text)?;
            if values.len() > policy.fields.get() {
                return Err(ServerError::Limit("form fields".into()));
            }
            for (name, values) in &values {
                policy.values.check(name, values)?;
            }
            Ok(Arc::new(FormData { values }))
        }
        .await
        .map_err(Arc::new);
        parsed.result = Some(result.clone());
        result.map_err(ServerError::SharedInput)
    }

    /// Move a bounded multipart parser to an owned callback; parts borrow it and cannot escape.
    pub async fn with_multipart<A, F>(
        &self,
        policy: MultipartPolicy,
        use_parts: impl FnOnce(MultipartReader) -> F,
    ) -> Result<A, ServerError>
    where
        F: Future<Output = Result<A, ServerError>>,
    {
        let mut request =
            http::Request::new(Body::new(Limited::new(self.claim()?, policy.body.bytes())));
        *request.headers_mut() = self.headers().clone();
        request
            .extensions_mut()
            .insert(axum::extract::DefaultBodyLimit::disable());
        let parser = Multipart::from_request(request, &())
            .await
            .map_err(|error| ServerError::MultipartRejection(error.to_string()))?;
        use_parts(MultipartReader {
            parser,
            policy,
            count: 0,
        })
        .await
    }
    /// Explicit native body escape; the callback owns the consumed body in this awaited future.
    pub async fn with_native_body<A, F>(
        &self,
        use_body: impl FnOnce(Body) -> F,
    ) -> Result<A, ServerError>
    where
        F: Future<Output = Result<A, ServerError>>,
    {
        use_body(self.claim()?).await
    }
}
/// Shared checked body/metadata bridge across native router ownership transfer.
#[derive(Clone)]
struct RoutingSnapshot(Request);
/// Mutable native metadata before actual axum matching, sharing one checked body owner.
/// Taking a body decoder here is explicit: compatible forms remain cached for endpoints;
/// other decoders claim the body and downstream incompatible reads fail honestly.
pub struct RoutingRequest {
    /// Native method/URI/header mutations determine subsequent router selection.
    parts: http::request::Parts,
    /// Original encoded body stays single-owned rather than buffered for routing.
    request: Request,
}
impl RoutingRequest {
    /// Create a pre-routing owner without reading or copying body bytes.
    pub(crate) fn from_native(native: http::Request<Body>, cancellation: Cancellation) -> Self {
        let request = Request::from_native(native, cancellation);
        let parts = request.state.parts.clone();
        Self { parts, request }
    }
    /// Borrow mutable native metadata. Method/URI changes affect the actual native router.
    /// Insert service metadata through `request().insert_extension` to share it with selected R.
    pub fn parts_mut(&mut self) -> &mut http::request::Parts {
        &mut self.parts
    }
    /// Borrow current native routing metadata without exposing a second body.
    pub fn parts(&self) -> &http::request::Parts {
        &self.parts
    }
    /// Share a metadata snapshot and the same checked body/extension owner for typed inputs.
    /// Metadata-only snapshots copy native head fields; encoded payload bytes are never cloned.
    pub fn request(&self) -> Request {
        self.request.with_parts(self.parts.clone())
    }
    /// Transfer unread bytes to native routing once; compatible parsed forms remain shared.
    pub(crate) fn into_native(mut self) -> http::Request<Body> {
        let request = self.request();
        let body = {
            let mut state = request
                .state
                .body
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &mut *state {
                BodyState::Unread(body) => {
                    let body = body.take().unwrap_or_else(Body::empty);
                    *state = BodyState::Routing;
                    body
                }
                BodyState::Exclusive | BodyState::Routing | BodyState::Form { .. } => Body::empty(),
            }
        };
        self.parts.extensions.extend(
            request
                .state
                .extensions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        );
        self.parts.extensions.insert(RoutingSnapshot(request));
        http::Request::from_parts(self.parts, body)
    }
}
/// Environments select fresh request state through R rather than a global locator.
pub trait HasRequest {
    /// Current request whose body/metadata owner survives through finalization.
    fn request(&self) -> &Request;
}
/// A convenient request environment sharing application services without cloning them.
pub struct RequestEnv<R> {
    /// Immutable application services and runtime capabilities.
    pub services: Arc<R>,
    /// Fresh metadata/body owner for this request.
    pub request: Request,
}
impl<R> HasRequest for RequestEnv<R> {
    /// Select the original request owner.
    fn request(&self) -> &Request {
        &self.request
    }
}
impl<R> HasCancellation for RequestEnv<R> {
    /// Request cancellation does not cancel unrelated application requests.
    fn cancellation(&self) -> &Cancellation {
        self.request.cancellation()
    }
}
impl<R: HasTaskSpawner> HasTaskSpawner for RequestEnv<R> {
    /// Preserve the application's selected task provider.
    type Tasks = R::Tasks;
    /// Delegate task execution to the selected application capability.
    fn task_spawner(&self) -> &Self::Tasks {
        self.services.task_spawner()
    }
}
/// Lazy typed request input, suitable for explicit environment/error projection.
pub struct RequestInput<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    /// Input execution remains a normal Effect in R.
    effect: Effect<A, E, R>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> RequestInput<A, E, R> {
    /// Describe a selected native reader without invoking it.
    #[must_use]
    pub fn new(effect: Effect<A, E, R>) -> Self {
        Self { effect }
    }
    /// Compose this input through the existing interpreter.
    pub fn effect(&self) -> Effect<A, E, R> {
        self.effect.clone()
    }
    /// Explicitly project a larger environment without losing the current interpreter context.
    pub fn provide_some<L: Send + Sync + 'static>(
        &self,
        project: impl Fn(&L) -> R + Send + Sync + 'static,
    ) -> RequestInput<A, E, L> {
        RequestInput::new(self.effect.clone().provide_some(project))
    }
}
/// Incremental multipart policy with separate body/part/header/field resource bounds.
#[derive(Clone, Copy)]
pub struct MultipartPolicy {
    /// Maximum complete encoded body bytes.
    pub body: BodyLimit,
    /// Maximum bytes in one part.
    pub part_bytes: NonZeroUsize,
    /// Maximum combined native part-header bytes.
    pub header_bytes: NonZeroUsize,
    /// Maximum number of parts.
    pub parts: NonZeroUsize,
    /// Maximum decoded field text bytes.
    pub field_bytes: NonZeroUsize,
}
/// Multipart owner; callbacks borrow native parts only while this parser remains alive.
pub struct MultipartReader {
    /// Native streaming parser, never collected into a complete upload buffer.
    parser: Multipart,
    /// Explicit distinct resource limits.
    policy: MultipartPolicy,
    /// Observed number of parts.
    count: usize,
}
/// Erase only the native parser borrow, preserving an awaited bounded part owner.
trait PartReader: Send {
    /// Borrow native disposition metadata while the owner is live.
    fn name(&self) -> Option<&str>;
    /// Borrow the untrusted native upload filename.
    fn file_name(&self) -> Option<&str>;
    /// Pull/count one encoded chunk through the same owner used for final draining.
    fn chunk(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Bytes>, ServerError>> + Send + '_>>;
}
/// One native field remains owned locally until its callback and remaining bytes settle.
struct NativePart<'a> {
    /// Authoritative streaming multipart field.
    field: axum::extract::multipart::Field<'a>,
    /// Explicit per-part encoded byte policy.
    policy: MultipartPolicy,
    /// Count shared by application reads and mandatory remaining-part drain.
    read: usize,
}
impl PartReader for NativePart<'_> {
    /// Forward borrowed native field identity.
    fn name(&self) -> Option<&str> {
        self.field.name()
    }
    /// Forward borrowed native filename without treating it as a storage path.
    fn file_name(&self) -> Option<&str> {
        self.field.file_name()
    }
    /// Keep each native chunk bounded even when the application skips a part.
    fn chunk(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Bytes>, ServerError>> + Send + '_>> {
        Box::pin(async move {
            let chunk = self.field.chunk().await.map_err(ServerError::Multipart)?;
            if let Some(chunk) = &chunk {
                self.read = self
                    .read
                    .checked_add(chunk.len())
                    .ok_or_else(|| ServerError::Limit("multipart part".into()))?;
                if self.read > self.policy.part_bytes.get() {
                    return Err(ServerError::Limit("multipart part".into()));
                }
            }
            Ok(chunk)
        })
    }
}
/// A nonescaping borrow of a native part; its owner also checks any unread remainder.
pub struct MultipartPart<'a> {
    /// Borrowed bounded native owner cannot be transferred out of the callback.
    reader: &'a mut dyn PartReader,
    /// Independent limit for finite decoded field text.
    field_bytes: NonZeroUsize,
}
impl MultipartReader {
    /// Await one part, then drain any unread remainder under the same byte bound.
    /// The higher-ranked callback cannot return a native part or a borrowed upload stream.
    pub async fn next<A>(
        &mut self,
        consume: impl for<'a> FnOnce(
            MultipartPart<'a>,
        ) -> Pin<
            Box<dyn Future<Output = Result<A, ServerError>> + Send + 'a>,
        >,
    ) -> Result<Option<A>, ServerError> {
        let Some(field) = self
            .parser
            .next_field()
            .await
            .map_err(ServerError::Multipart)?
        else {
            return Ok(None);
        };
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| ServerError::Limit("multipart count".into()))?;
        if self.count > self.policy.parts.get() {
            return Err(ServerError::Limit("multipart count".into()));
        }
        let header_bytes = field
            .headers()
            .iter()
            .try_fold(0usize, |total, (name, value)| {
                total
                    .checked_add(name.as_str().len())
                    .and_then(|total| total.checked_add(value.as_bytes().len()))
                    .ok_or_else(|| ServerError::Limit("multipart headers".into()))
            })?;
        if header_bytes > self.policy.header_bytes.get() {
            return Err(ServerError::Limit("multipart headers".into()));
        }
        let mut owner = NativePart {
            field,
            policy: self.policy,
            read: 0,
        };
        let output = consume(MultipartPart {
            reader: &mut owner,
            field_bytes: self.policy.field_bytes,
        })
        .await?;
        while owner.chunk().await?.is_some() {}
        Ok(Some(output))
    }
}
impl MultipartPart<'_> {
    /// Borrow the native field name without copying upload metadata.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.reader.name()
    }
    /// Borrow the untrusted client file name; callers decide storage naming.
    #[must_use]
    pub fn file_name(&self) -> Option<&str> {
        self.reader.file_name()
    }
    /// Pull one bounded chunk; awaiting consumer work preserves upload backpressure.
    pub async fn chunk(&mut self) -> Result<Option<Bytes>, ServerError> {
        self.reader.chunk().await
    }
    /// Decode a finite field with an independent text-byte limit and strict UTF-8.
    pub async fn text(&mut self) -> Result<String, ServerError> {
        let mut bytes = Vec::new();
        while let Some(chunk) = self.chunk().await? {
            if bytes.len().saturating_add(chunk.len()) > self.field_bytes.get() {
                return Err(ServerError::Limit("multipart field".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| ServerError::Invalid("multipart UTF-8".into()))
    }
}
/// Collect a finite body only when its selected decoder permits buffering.
async fn read_bounded(body: Body, limit: BodyLimit) -> Result<Bytes, ServerError> {
    Limited::new(body, limit.bytes())
        .collect()
        .await
        .map(|value| value.to_bytes())
        .map_err(|error| {
            if error.is::<http_body_util::LengthLimitError>() {
                ServerError::Limit("body bytes".into())
            } else {
                ServerError::Transport(effect::Defect::new(error.to_string()))
            }
        })
}
/// Reject malformed escapes/UTF-8 before URL form decoding, which otherwise replaces invalid text.
fn parse_pairs(text: &str) -> Result<BTreeMap<String, Vec<String>>, ServerError> {
    for part in text.split('&').flat_map(|pair| pair.splitn(2, '=')) {
        let raw = part.as_bytes();
        let mut index = 0;
        while let Some(byte) = raw.get(index) {
            if *byte == b'%' {
                if !raw
                    .get(index.saturating_add(1))
                    .is_some_and(u8::is_ascii_hexdigit)
                    || !raw
                        .get(index.saturating_add(2))
                        .is_some_and(u8::is_ascii_hexdigit)
                {
                    return Err(ServerError::Invalid("percent encoding".into()));
                }
                index = index.saturating_add(3);
            } else {
                index = index.saturating_add(1);
            }
        }
        percent_encoding::percent_decode_str(part)
            .decode_utf8()
            .map_err(|_| ServerError::Invalid("form UTF-8".into()))?;
    }
    let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(text.as_bytes()) {
        fields
            .entry(key.into_owned())
            .or_default()
            .push(value.into_owned());
    }
    Ok(fields)
}

impl<A, E, R> RequestInput<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
{
    /// Decode a selected input lazily after validating its native concrete media type.
    pub fn select(
        media: HeaderValue,
        read: impl Fn(Request) -> Effect<A, E, R> + Send + Sync + 'static,
        on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Self {
        let read = Arc::new(read);
        let on_error = Arc::new(on_error);
        let effect = Effect::service(|env: &R| env.request().clone()).flat_map(move |request| {
            let read = read.clone();
            let map = on_error.clone();
            let expected = media.to_str().ok().map(|value| {
                value
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            });
            let actual = request
                .singleton_header::<headers::ContentType>()
                .map(|value| {
                    value.map(|value| {
                        value
                            .to_string()
                            .split(';')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_ascii_lowercase()
                    })
                });
            match (expected, actual) {
                (Some(expected), Ok(Some(actual))) if expected == actual => read(request),
                (_, Err(error)) => Effect::from_result_once(Err(map(error))),
                _ => Effect::from_result_once(Err(map(ServerError::Invalid(
                    "unsupported content type".into(),
                )))),
            }
        });
        Self::new(effect)
    }
}
impl<E, R> RequestInput<Bytes, E, R>
where
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
{
    /// Read a finite native byte body under explicit encoded-byte/media policy.
    pub fn bytes(
        media: HeaderValue,
        limit: BodyLimit,
        on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Self {
        let map = Arc::new(on_error);
        let read_map = map.clone();
        Self::select(
            media,
            move |request| {
                let map = read_map.clone();
                Effect::try_future_once(
                    move |_| async move { request.read_bytes(limit).await },
                    move |error| map(error),
                )
            },
            move |error| map(error),
        )
    }
}
impl<E, R> RequestInput<String, E, R>
where
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
{
    /// Read strict UTF-8 body text; malformed bytes never become replacement characters.
    pub fn text(
        media: HeaderValue,
        limit: BodyLimit,
        on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Self {
        let map = Arc::new(on_error);
        let read_map = map.clone();
        Self::select(
            media,
            move |request| {
                let map = read_map.clone();
                Effect::try_future_once(
                    move |_| async move { request.read_text(limit).await },
                    move |error| map(error),
                )
            },
            move |error| map(error),
        )
    }
}
impl<A, E, R> RequestInput<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
{
    /// Decode bounded strict JSON with a native core codec. Missing/empty JSON is an error.
    pub fn json(
        codec: effect::PureCodec<A, effect::JsonValue>,
        limit: BodyLimit,
        policy: effect::ParsePolicy,
        on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Self {
        let map = Arc::new(on_error);
        let read_map = map.clone();
        Self::select(
            HeaderValue::from_static("application/json"),
            move |request| {
                let codec = codec.clone();
                let map = read_map.clone();
                Effect::try_future_once(
                    move |_| async move { request.read_json(limit, policy, &codec).await },
                    move |error| map(error),
                )
            },
            move |error| map(error),
        )
    }
    /// Explicitly permit genuinely absent/empty JSON with a fresh default; malformed JSON stays typed.
    pub fn json_with_empty(
        codec: effect::PureCodec<A, effect::JsonValue>,
        limit: BodyLimit,
        policy: effect::ParsePolicy,
        empty: impl Fn() -> A + Send + Sync + 'static,
        on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Self {
        let map = Arc::new(on_error);
        let empty = Arc::new(empty);
        Self::new(
            Effect::service(|env: &R| env.request().clone()).flat_map(move |request| {
                let codec = codec.clone();
                let map = map.clone();
                let empty = empty.clone();
                Effect::try_future_once(
                    move |_| async move {
                        let bytes = request.read_bytes(limit).await?;
                        if bytes.is_empty() {
                            Ok(empty())
                        } else {
                            let media = request
                                .singleton_header::<headers::ContentType>()?
                                .ok_or_else(|| ServerError::Missing("Content-Type".into()))?;
                            if !media.to_string().split(';').next().is_some_and(|base| {
                                base.trim().eq_ignore_ascii_case("application/json")
                            }) {
                                return Err(ServerError::Invalid(
                                    "unsupported content type".into(),
                                ));
                            }
                            codec
                                .parse_utf8(&bytes, policy)
                                .map_err(ServerError::Schema)
                        }
                    },
                    move |error| map(error),
                )
            }),
        )
    }
}
impl<E, R> RequestInput<Arc<FormData>, E, R>
where
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
{
    /// Share one compatible URL-encoded parse under explicit total/field bounds.
    pub fn form(
        policy: FormPolicy,
        on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Self {
        let map = Arc::new(on_error);
        let read_map = map.clone();
        Self::select(
            HeaderValue::from_static("application/x-www-form-urlencoded"),
            move |request| {
                let map = read_map.clone();
                Effect::try_future_once(
                    move |_| async move { request.read_form(policy).await },
                    move |error| map(error),
                )
            },
            move |error| map(error),
        )
    }
}

impl<R: effect::HasBlockingExecutor> effect::HasBlockingExecutor for RequestEnv<R> {
    /// Preserve the selected application's blocking provider, separate from its async/CPU provider.
    type Executor = R::Executor;
    /// Borrow the existing blocking provider without creating a worker pool.
    fn blocking_executor(&self) -> &Self::Executor {
        self.services.blocking_executor()
    }
}
impl<R: effect::HasCpuExecutor> effect::HasCpuExecutor for RequestEnv<R> {
    /// Preserve the selected CPU provider for mixed native HTTP and application CPU work.
    type Executor = R::Executor;
    /// Borrow the original CPU executor.
    fn cpu_executor(&self) -> &Self::Executor {
        self.services.cpu_executor()
    }
}
impl<R: effect::HasClock> effect::HasClock for RequestEnv<R> {
    /// Preserve the application's monotonic clock selection.
    type Clock = R::Clock;
    /// Borrow the selected core clock.
    fn clock(&self) -> &Self::Clock {
        self.services.clock()
    }
}
impl<R: effect::HasTracer> effect::HasTracer for RequestEnv<R> {
    /// Borrow the same Effect tracer used by host and business spans.
    fn tracer(&self) -> &effect::Tracer {
        self.services.tracer()
    }
}
impl<R: effect::HasLogger> effect::HasLogger for RequestEnv<R> {
    /// Borrow the original application logger without installing a global subscriber.
    fn logger(&self) -> &effect::Logger {
        self.services.logger()
    }
}
impl<R: crate::HasServerError> crate::HasServerError for RequestEnv<R> {
    /// Preserve the selected application's typed native error family.
    type Error = R::Error;
    /// Map native failures through the original selected services.
    fn server_error(&self, error: ServerError) -> Self::Error {
        self.services.server_error(error)
    }
}

/// Borrowed textual scalar decoder.
type ScalarDecoder<A> = dyn Fn(&str) -> Result<A, ServerError> + Send + Sync;

/// Borrowed application scalar encoder.
type ScalarEncoder<A> = dyn Fn(&A) -> Result<String, ServerError> + Send + Sync;
