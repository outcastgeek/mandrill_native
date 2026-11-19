//! Checked scalar inputs and single-owner native request bodies.

use crate::{effect_from_result, effect_provide_some, effect_try_future};
use axum::body::Body;
use axum::extract::{FromRequest, Multipart};
use bytes::Bytes;
use effect::{Cancellation, Effect};
use effect::memory::{BufferLease, BufferView, MemoryError};
use effect::{effect, service};
use headers::{Header, HeaderMapExt};
use http::{Extensions, HeaderMap, HeaderValue, Method, Uri, Version};
use http_body_util::{BodyExt, Limited};
use std::collections::BTreeMap;
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

mod prepared_form;
use prepared_form::PreparedForm;

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
    /// A selected native response ingestion limit was exceeded.
    OutputLimit(crate::OutputLimit),
    /// Another incompatible decoder already owns the native body.
    BodyClaimed,
    /// A compatible form cannot be reused under a different parse policy.
    FormPolicyConflict,
    /// The peer disconnected before the operation settled.
    Disconnected,
    /// Prepared endpoint/lifecycle admission was refused.
    Memory(effect::memory::MemoryError),
    /// The selected prepared native provider refused admission.
    Provider(effect::prepared_provider::ProviderRefusal),
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
            Self::OutputLimit(limit) => write!(f, "output limit exceeded: {limit:?}"),
            Self::BodyClaimed => f.write_str("request body already claimed"),
            Self::FormPolicyConflict => f.write_str("incompatible form policy"),
            Self::Disconnected => f.write_str("peer disconnected"),
            Self::Memory(error) => write!(f, "prepared hosting: {error}"),
            Self::Provider(error) => write!(f, "prepared provider refused: {error:?}"),
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
            FieldSource::Query => {
                decode_text_pairs(request.uri().query().unwrap_or("").as_bytes(), false)?
                    .remove(self.name.as_ref())
                    .unwrap_or_default()
            }
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
impl FormPolicy {
    /// Worst-case physical backing for prepared decoded bytes and their sorted field records.
    pub fn prepared_storage_bytes(self) -> Result<effect::memory::Bytes, MemoryError> {
        prepared_form::storage_bytes(self)
    }
}
/// Explicit byte/count bounds for one byte-preserving form field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteFieldBounds {
    /// Maximum repetitions of one field.
    pub count: NonZeroUsize,
    /// Maximum decoded byte length of each repeated value.
    pub bytes: NonZeroUsize,
}
impl ByteFieldBounds {
    /// Check repetition and byte lengths without interpreting the field name.
    fn check(self, values: &[Vec<u8>]) -> Result<(), ServerError> {
        if values.len() > self.count.get() {
            return Err(ServerError::Limit("form field count".into()));
        }
        if values.iter().any(|value| value.len() > self.bytes.get()) {
            return Err(ServerError::Limit("form field bytes".into()));
        }
        Ok(())
    }
}
/// Explicit total/field/count policy for byte-preserving form reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteFormPolicy {
    /// Maximum complete encoded form bytes.
    pub body: BodyLimit,
    /// Maximum distinct field names.
    pub fields: NonZeroUsize,
    /// Bounds for each repeated field value.
    pub values: ByteFieldBounds,
}
/// Decoded URL-encoded pairs preserving key and value bytes.
pub type BytePairs = BTreeMap<Vec<u8>, Vec<Vec<u8>>>;
/// Byte-preserving decoded fields and any syntax diagnostic from the same input.
#[derive(Debug)]
pub struct BytePairsOutcome {
    /// Valid decoded fields, preserving repeated-value order.
    pub values: BytePairs,
    /// Syntax diagnostic, if any segment was rejected.
    pub diagnostic: Option<ServerError>,
}
/// Shared byte-preserving form snapshot; body bytes are not retained alongside it.
#[derive(Debug)]
pub struct ByteFormData {
    /// Decoded fields preserving repeated-value order.
    pub values: BytePairs,
}
/// Compatibility identity for one acknowledged form representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormReadPolicy {
    /// UTF-8 text representation with its own bounds.
    Text {
        policy: FormPolicy,
        reject_raw_semicolons: bool,
    },
    /// Byte-preserving representation with byte/count bounds.
    Bytes {
        policy: ByteFormPolicy,
        reject_raw_semicolons: bool,
    },
    /// Byte-preserving fields with retained syntax diagnostics.
    PartialBytes {
        policy: ByteFormPolicy,
        segment_limit: Option<NonZeroUsize>,
    },
    /// Multipart fields with shared memory accounting.
    AccountedMultipartFields {
        policy: MultipartPolicy,
        fields: NonZeroUsize,
        values: ByteFieldBounds,
        file_memory: usize,
    },
    /// Multipart field collection with explicit streaming and field bounds.
    MultipartFields {
        policy: MultipartPolicy,
        fields: NonZeroUsize,
        values: ByteFieldBounds,
    },
}
/// One parsed form representation kept in the shared claim cell.
enum ParsedFormData {
    /// UTF-8 text fields.
    Text(Arc<FormData>),
    /// UTF-8 fields in admitted byte backing, with optional public FormData projection.
    PreparedText {
        form: PreparedForm,
        materialized: Option<Arc<FormData>>,
    },
    /// Byte-preserving fields.
    Bytes(Arc<ByteFormData>),
    /// Byte fields and a retained syntax diagnostic.
    PartialBytes(Arc<BytePairsOutcome>),
    /// Successfully collected multipart fields.
    MultipartFields(Arc<ByteFormData>),
}
enum TextFormRead {
    Ordinary(Arc<FormData>),
    Prepared(PreparedForm),
}
/// Body claim state moves the native Body exactly once; compatible parses share a cell.
enum BodyState {
    /// An unread native body is available for one claim.
    Unread(Option<Body>),
    /// A native/bytes/text/JSON/multipart decoder owns it.
    Exclusive,
    /// Native routing temporarily owns the unread body; it returns at endpoint extraction.
    Routing,
    /// Same-representation form reads share one acknowledged parse.
    Form {
        policy: FormReadPolicy,
        result: Option<Arc<tokio::sync::Mutex<ParsedForm>>>,
    },
}
/// Compatible form owner serializes the one parse and retains its typed acknowledged result.
struct ParsedForm {
    /// Native body transfers into the first parse future only.
    body: Option<Body>,
    /// Completed parse result shared without copying form/error payloads.
    result: Option<Result<ParsedFormData, Arc<ServerError>>>,
}
/// Shared body and extension storage for a prepared base request.
pub(crate) struct RequestBase {
    extensions: Mutex<Extensions>,
    body: Mutex<BodyState>,
    form: tokio::sync::Mutex<ParsedForm>,
    endpoint_bytes: Mutex<Option<crate::prepared_endpoint::EndpointByteLeases>>,
}
#[derive(Clone)]
enum RequestExtensions {
    Ordinary(Arc<Mutex<Extensions>>),
    Prepared(crate::prepared_request_storage::BaseLoan),
}
impl RequestExtensions {
    fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, Extensions>> {
        match self { Self::Ordinary(value) => value.lock(), Self::Prepared(value) => value.extensions.lock() }
    }
}
#[derive(Clone)]
enum RequestBody {
    Ordinary(Arc<Mutex<BodyState>>),
    Prepared(crate::prepared_request_storage::BaseLoan),
}
impl RequestBody {
    fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, BodyState>> {
        match self { Self::Ordinary(value) => value.lock(), Self::Prepared(value) => value.body.lock() }
    }

    fn try_lock(&self) -> std::sync::TryLockResult<std::sync::MutexGuard<'_, BodyState>> {
        match self {
            Self::Ordinary(value) => value.try_lock(),
            Self::Prepared(value) => value.body.try_lock(),
        }
    }
}
enum FormCell<'a> {
    Ordinary(Arc<tokio::sync::Mutex<ParsedForm>>),
    Prepared(&'a tokio::sync::Mutex<ParsedForm>),
}
impl std::ops::Deref for FormCell<'_> {
    type Target = tokio::sync::Mutex<ParsedForm>;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ordinary(cell) => cell,
            Self::Prepared(cell) => cell,
        }
    }
}
impl RequestBody {
    fn form_cell<'a>(
        &'a self,
        result: &Option<Arc<tokio::sync::Mutex<ParsedForm>>>,
    ) -> Option<FormCell<'a>> {
        match (self, result) {
            (_, Some(cell)) => Some(FormCell::Ordinary(cell.clone())),
            (Self::Prepared(base), None) => Some(FormCell::Prepared(&base.form)),
            (Self::Ordinary(_), None) => None,
        }
    }
}

/// Native request metadata and checked body owner shared by one request only.
pub(crate) struct RequestState {
    parts: http::request::Parts,
    extensions: RequestExtensions,
    body: RequestBody,
    cancellation: Cancellation,
    endpoint_bytes: Mutex<Option<crate::prepared_endpoint::EndpointByteLeases>>,
}
#[derive(Clone)]
enum RequestOwner {
    Ordinary(Arc<RequestState>),
    Prepared { snapshot: crate::prepared_request_storage::SnapshotLoan, pool: Arc<crate::PreparedRequestStoragePool> },
}
impl std::ops::Deref for RequestOwner {
    type Target = RequestState;
    fn deref(&self) -> &RequestState { match self { Self::Ordinary(value) => value, Self::Prepared { snapshot, .. } => snapshot } }
}
/// Fresh request state; cloning shares metadata and its single body claim.
#[derive(Clone)]
pub struct Request {
    state: RequestOwner,
}
impl Request {
    fn from_reserved_native(
        request: http::Request<Body>,
        cancellation: Cancellation,
        reservation: crate::prepared_request_storage::PreparedRequestStorageReservation,
    ) -> Result<Self, effect::memory::MemoryError> {
        if request.extensions().get::<RoutingSnapshot>().is_some() {
            return Err(effect::memory::MemoryError::UnsupportedRunContract);
        }
        let (mut parts, body) = request.into_parts();
        let extensions = std::mem::take(&mut parts.extensions);
        let base = reservation.base.publish(RequestBase {
            extensions: Mutex::new(extensions),
            body: Mutex::new(BodyState::Unread(Some(body))),
            form: tokio::sync::Mutex::new(ParsedForm { body: None, result: None }),
            endpoint_bytes: Mutex::new(None),
        }).map_err(|_| effect::memory::MemoryError::ConstructionUnbound)?;
        let state = RequestState {
            parts,
            extensions: RequestExtensions::Prepared(base.clone()),
            body: RequestBody::Prepared(base),
            cancellation,
            endpoint_bytes: Mutex::new(None),
        };
        let snapshot = reservation.snapshot.publish(state).map_err(|_| effect::memory::MemoryError::ConstructionUnbound)?;
        Ok(Self { state: RequestOwner::Prepared { snapshot, pool: reservation.pool } })
    }
    /// Construct from native input using finite pre-reserved request and snapshot storage.
    /// Native HTTP containers and the supplied cancellation retain their own backing.
    /// An existing prepared routing snapshot retains its original pool and
    /// shares its original body and extensions; a new request uses `pool`.
    /// On refusal, this consumes and drops the unadmitted native request and
    /// cancellation after rolling back physical reservations, before body reads.
    pub fn try_from_native_in(
        request: http::Request<Body>,
        cancellation: Cancellation,
        pool: &Arc<crate::PreparedRequestStoragePool>,
    ) -> Result<Self, effect::memory::MemoryError> {
        if let Some(shared) = request.extensions().get::<RoutingSnapshot>() {
            let original = match &shared.0.state {
                RequestOwner::Prepared { pool, .. } => Arc::clone(pool),
                RequestOwner::Ordinary(_) => return Err(effect::memory::MemoryError::UnsupportedRunContract),
            };
            let vacancy = original.reserve_snapshot()?;
            let (mut parts, body) = request.into_parts();
            let shared = parts.extensions.remove::<RoutingSnapshot>().ok_or(effect::memory::MemoryError::ConstructionUnbound)?;
            let extensions = std::mem::take(&mut parts.extensions);
            let request = shared.0.publish_parts(parts, vacancy, original)?;
            request.state.extensions.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(extensions);
            let mut state = request.state.body.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if matches!(*state, BodyState::Routing) { *state = BodyState::Unread(Some(body)); }
            drop(state);
            return Ok(request);
        }
        let reservation = pool.reserve_request()?;
        Self::from_reserved_native(request, cancellation, reservation)
    }
    fn publish_parts(&self, mut parts: http::request::Parts, vacancy: crate::prepared_request_storage::SnapshotVacancy, pool: Arc<crate::PreparedRequestStoragePool>) -> Result<Self, effect::memory::MemoryError> {
        parts.extensions = Extensions::new();
        let state = RequestState { parts, extensions: self.state.extensions.clone(), body: self.state.body.clone(), cancellation: self.state.cancellation.clone(), endpoint_bytes: Mutex::new(None) };
        let snapshot = vacancy.publish(state).map_err(|_| effect::memory::MemoryError::ConstructionUnbound)?;
        Ok(Self { state: RequestOwner::Prepared { snapshot, pool } })
    }
    fn try_with_parts(&self, parts: &http::request::Parts) -> Result<Self, effect::memory::MemoryError> {
        let RequestOwner::Prepared { pool, .. } = &self.state else { return Err(effect::memory::MemoryError::UnsupportedRunContract); };
        let vacancy = pool.reserve_snapshot()?;
        self.publish_parts(parts.clone(), vacancy, Arc::clone(pool))
    }
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
            state: RequestOwner::Ordinary(Arc::new(RequestState {
                parts,
                extensions: RequestExtensions::Ordinary(Arc::new(Mutex::new(extensions))),
                body: RequestBody::Ordinary(Arc::new(Mutex::new(BodyState::Unread(Some(body))))),
                cancellation,
                endpoint_bytes: Mutex::new(None),
            })),
        }
    }
    /// Rebuild only immutable native metadata while sharing the exact body/extension owner.
    fn with_parts(&self, mut parts: http::request::Parts) -> Self {
        parts.extensions = Extensions::new();
        Self {
            state: RequestOwner::Ordinary(Arc::new(RequestState {
                parts,
                extensions: self.state.extensions.clone(),
                body: self.state.body.clone(),
                cancellation: self.state.cancellation.clone(),
                endpoint_bytes: Mutex::new(None),
            })),
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
    /// Install a request extension only if its type has no existing value.
    /// A losing publisher gets its original value back after the lock is released.
    pub fn try_insert_extension<T: Clone + Send + Sync + 'static>(
        &self,
        value: T,
    ) -> Result<(), T> {
        let mut extensions = self.state.extensions.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if extensions.get::<T>().is_some() {
            return Err(value);
        }
        let replaced = extensions.insert(value);
        drop(extensions);
        drop(replaced);
        Ok(())
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
        match self.read_admitted_body(self.claim()?, limit).await? {
            AdmittedBody::Ordinary(bytes) => Ok(bytes),
            AdmittedBody::Prepared(view) => Ok(Bytes::from_owner(view)),
        }
    }
    pub(crate) fn install_endpoint_bytes(&self, leases: crate::prepared_endpoint::EndpointByteLeases) {
        *self.state.endpoint_bytes.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(leases);
    }
    pub(crate) fn install_shared_endpoint_bytes(
        &self,
        leases: crate::prepared_endpoint::EndpointByteLeases,
    ) -> Result<(), MemoryError> {
        let RequestBody::Prepared(base) = &self.state.body else {
            return Err(MemoryError::UnsupportedRunContract);
        };
        *base.endpoint_bytes.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(leases);
        Ok(())
    }
    fn take_endpoint_bytes(
        &self,
        select: impl Fn(&mut crate::prepared_endpoint::EndpointByteLeases) -> &mut Option<crate::prepared_endpoint::AdmittedByteLease>,
    ) -> Result<Option<BufferLease<'static>>, ServerError> {
        {
            let mut leases = self.state.endpoint_bytes.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(lease) = leases.as_mut().and_then(|leases| select(leases).as_mut()) {
                return lease.0.take().map(Some).ok_or(ServerError::BodyClaimed);
            }
        }
        if let RequestBody::Prepared(base) = &self.state.body {
            let mut leases = base.endpoint_bytes.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(lease) = leases.as_mut().and_then(|leases| select(leases).as_mut()) {
                return lease.0.take().map(Some).ok_or(ServerError::BodyClaimed);
            }
        }
        Ok(None)
    }
    /// Read the native body once into its preadmitted physical buffer.
    /// The returned view holds the actual backing independently of this request.
    pub async fn read_prepared_bytes(&self) -> Result<BufferView<'static>, ServerError> {
        self.read_prepared_bytes_with_optional_limit(None).await
    }

    /// Read into the admitted body slot with the caller's additional logical limit.
    pub async fn read_prepared_bytes_limited(
        &self,
        limit: BodyLimit,
    ) -> Result<BufferView<'static>, ServerError> {
        self.read_prepared_bytes_with_optional_limit(Some(limit)).await
    }

    async fn read_prepared_bytes_with_optional_limit(
        &self,
        limit: Option<BodyLimit>,
    ) -> Result<BufferView<'static>, ServerError> {
        let buffer = self.take_endpoint_bytes(|leases| &mut leases.body)?
            .ok_or(ServerError::Memory(MemoryError::UnsupportedRunContract))?;
        let limit = limit.map_or_else(|| buffer.capacity().get(), |limit| limit.bytes());
        read_prepared_bounded(self.claim()?, buffer, limit).await
    }
    async fn read_admitted_body(&self, body: Body, limit: BodyLimit) -> Result<AdmittedBody, ServerError> {
        match self.take_endpoint_bytes(|leases| &mut leases.body)? {
            Some(buffer) => {
                let limit = limit.bytes().min(buffer.capacity().get());
                read_prepared_bounded(body, buffer, limit).await.map(AdmittedBody::Prepared)
            }
            None => read_bounded(body, limit).await.map(AdmittedBody::Ordinary),
        }
    }
    /// Take the selected fixed byte destination for explicit decoding into it.
    pub fn take_prepared_decoded_buffer(&self) -> Result<BufferLease<'static>, ServerError> {
        self.take_endpoint_bytes(|leases| &mut leases.decoded)?
            .ok_or(ServerError::Memory(MemoryError::UnsupportedRunContract))
    }
    /// Take the selected fixed output destination, for writing and transfer to a reply or stream.
    ///
    /// ```
    /// use effect::memory::BufferView;
    /// use effect_server::{Reply, Request, ServerError, native::http::HeaderValue, reply};
    ///
    /// fn response<E: Send + 'static, R: Send + Sync + 'static>(
    ///     request: &Request,
    ///     payload: &[u8],
    ///     media: HeaderValue,
    /// ) -> Result<Reply<E, R>, ServerError> {
    ///     let mut out = request.take_prepared_output_buffer()?;
    ///     out.extend_from_slice(payload).map_err(ServerError::Memory)?;
    ///     let view = BufferView::new(out.share());
    ///     Ok(reply! { lease(view, media); })
    /// }
    /// ```
    pub fn take_prepared_output_buffer(&self) -> Result<BufferLease<'static>, ServerError> {
        self.take_endpoint_bytes(|leases| &mut leases.output)?
            .ok_or(ServerError::Memory(MemoryError::UnsupportedRunContract))
    }
    /// Take the selected generated-header slot, independently of the body output.
    pub fn take_prepared_response_header_buffer(&self) -> Result<Option<BufferLease<'static>>, ServerError> {
        self.take_endpoint_bytes(|leases| &mut leases.response_headers)
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
        self.read_form_with(policy, false).await
    }

    /// Share one compatible URL-encoded parse that rejects an unencoded semicolon.
    ///
    /// A raw `;` anywhere in an encoded field is refused before decoding; a
    /// percent-encoded `%3B` remains a value byte. Bounds, sharing and typed errors
    /// are otherwise the same as [`Self::read_form`]. A snapshot is shared only with
    /// reads that select the same separator behavior and policy.
    pub async fn read_form_rejecting_raw_semicolons(
        &self,
        policy: FormPolicy,
    ) -> Result<Arc<FormData>, ServerError> {
        self.read_form_with(policy, true).await
    }

    /// Share one compatible byte-preserving URL-encoded parse.
    ///
    /// A raw `;` is an ordinary value byte in this read. Bounds are the supplied
    /// byte/count policy, and a snapshot is shared only with reads selecting the same
    /// representation, separator behavior and policy.
    pub async fn read_byte_form(
        &self,
        policy: ByteFormPolicy,
    ) -> Result<Arc<ByteFormData>, ServerError> {
        self.read_byte_form_with(policy, false).await
    }

    /// Share one byte-preserving URL-encoded parse that rejects an unencoded semicolon.
    ///
    /// A raw `;` anywhere in an encoded field is refused before decoding; a
    /// percent-encoded `%3B` remains a value byte. Bounds, sharing and typed errors
    /// are otherwise the same as [`Self::read_byte_form`].
    pub async fn read_byte_form_rejecting_raw_semicolons(
        &self,
        policy: ByteFormPolicy,
    ) -> Result<Arc<ByteFormData>, ServerError> {
        self.read_byte_form_with(policy, true).await
    }

    /// Read one shared byte form while retaining valid fields and syntax diagnostics.
    ///
    /// This representation has its own compatibility identity. Transport and limit
    /// errors remain terminal errors rather than syntax diagnostics.
    pub async fn read_partial_byte_form(
        &self,
        policy: ByteFormPolicy,
    ) -> Result<Arc<BytePairsOutcome>, ServerError> {
        self.read_partial_byte_form_with_segment_limit(policy, None)
            .await
    }

    /// Read a partial byte form with an optional encoded-segment limit.
    pub async fn read_partial_byte_form_with_segment_limit(
        &self,
        policy: ByteFormPolicy,
        segment_limit: Option<NonZeroUsize>,
    ) -> Result<Arc<BytePairsOutcome>, ServerError> {
        let cell = self.claim_form(FormReadPolicy::PartialBytes {
            policy,
            segment_limit,
        })?;
        let mut parsed = cell.lock().await;
        if let Some(result) = &parsed.result {
            return shared_partial_bytes(result);
        }
        let body = parsed.body.take().ok_or(ServerError::Disconnected)?;
        parsed.result = Some(Err(Arc::new(ServerError::Disconnected)));
        let result: Result<Arc<BytePairsOutcome>, Arc<ServerError>> = async {
            let bytes = self.read_admitted_body(body, policy.body).await?;
            let bytes = bytes.as_ref();
            let outcome = if segment_limit
                .is_some_and(|limit| bytes.split(|byte| *byte == b'&').count() > limit.get())
            {
                BytePairsOutcome {
                    values: BTreeMap::new(),
                    diagnostic: Some(ServerError::Limit("form segments".into())),
                }
            } else {
                decode_url_encoded_pairs_partial(bytes)
            };
            if outcome.values.len() > policy.fields.get() {
                return Err(ServerError::Limit("form fields".into()));
            }
            for values in outcome.values.values() {
                policy.values.check(values)?;
            }
            Ok(Arc::new(outcome))
        }
        .await
        .map_err(Arc::new);
        let result = result.map(ParsedFormData::PartialBytes);
        parsed.result = Some(result);
        shared_partial_bytes(parsed.result.as_ref().ok_or(ServerError::Disconnected)?)
    }

    /// Inspect an already completed partial byte-form parse without claiming a body.
    ///
    /// Returns `None` when this policy has no completed parse, including while a
    /// parse is in progress. A completed transport or limit failure remains an error.
    #[must_use]
    pub fn peek_partial_byte_form(
        &self,
        policy: ByteFormPolicy,
    ) -> Option<Result<Arc<BytePairsOutcome>, ServerError>> {
        self.peek_partial_byte_form_with_segment_limit(policy, None)
    }

    /// Snapshot already acknowledged body fields without claiming or reparsing the body.
    /// This does not include query fields or enforce a new parsing policy.
    #[must_use]
    pub fn peek_completed_form_fields(&self) -> Option<ByteFormData> {
        let cell = {
            let state = self
                .state
                .body
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &*state {
                BodyState::Form { result, .. } => self.state.body.form_cell(result)?,
                BodyState::Unread(_) | BodyState::Exclusive | BodyState::Routing => return None,
            }
        };
        let parsed = cell.try_lock().ok()?;
        let values = match parsed.result.as_ref()?.as_ref().ok()? {
            ParsedFormData::Text(data) => data
                .values
                .iter()
                .map(|(key, values)| {
                    (
                        key.as_bytes().to_vec(),
                        values.iter().map(|v| v.as_bytes().to_vec()).collect(),
                    )
                })
                .collect(),
            ParsedFormData::PreparedText { form, .. } => {
                let mut values = BytePairs::new();
                for (name, value) in form.pairs() {
                    if let Some(repeated) = values.get_mut(name.as_bytes()) {
                        repeated.push(value.as_bytes().to_vec());
                    } else {
                        values.insert(name.as_bytes().to_vec(), vec![value.as_bytes().to_vec()]);
                    }
                }
                values
            }
            ParsedFormData::Bytes(data) | ParsedFormData::MultipartFields(data) => {
                data.values.clone()
            }
            ParsedFormData::PartialBytes(data) => data.values.clone(),
        };
        Some(ByteFormData { values })
    }

    /// Borrow acknowledged body fields in their stored order without reparsing
    /// or copying. An absent or failed parse yields `false`. A busy owner yields
    /// a memory refusal. The visitor must not access the form it is borrowing.
    pub fn visit_completed_form_fields(
        &self,
        mut visit: impl FnMut(&[u8], &[u8]) -> Result<(), ServerError>,
    ) -> Result<bool, ServerError> {
        use effect::memory::MemoryError;
        use std::sync::TryLockError;
        let cell = {
            let state = match self.state.body.try_lock() {
                Ok(state) => state,
                Err(TryLockError::Poisoned(error)) => error.into_inner(),
                Err(TryLockError::WouldBlock) => return Err(ServerError::Memory(MemoryError::Busy)),
            };
            match &*state {
                BodyState::Form { result, .. } => self.state.body.form_cell(result),
                BodyState::Unread(_) | BodyState::Exclusive | BodyState::Routing => None,
            }
        };
        let Some(cell) = cell else { return Ok(false) };
        let parsed = cell.try_lock().map_err(|_| ServerError::Memory(MemoryError::Busy))?;
        let Some(Ok(form)) = parsed.result.as_ref() else { return Ok(false) };
        let mut visit_bytes = |pairs: &BytePairs| {
            pairs.iter().try_for_each(|(name, values)| {
                values.iter().try_for_each(|value| visit(name, value))
            })
        };
        match form {
            ParsedFormData::Text(data) => data.values.iter().try_for_each(|(name, values)| {
                values.iter().try_for_each(|value| visit(name.as_bytes(), value.as_bytes()))
            }),
            ParsedFormData::PreparedText { form, .. } => form.pairs().try_for_each(|(name, value)| {
                visit(name.as_bytes(), value.as_bytes())
            }),
            ParsedFormData::Bytes(data) | ParsedFormData::MultipartFields(data) => visit_bytes(&data.values),
            ParsedFormData::PartialBytes(data) => visit_bytes(&data.values),
        }?;
        Ok(true)
    }

    /// Inspect a completed partial byte form with the selected segment limit.
    #[must_use]
    pub fn peek_partial_byte_form_with_segment_limit(
        &self,
        policy: ByteFormPolicy,
        segment_limit: Option<NonZeroUsize>,
    ) -> Option<Result<Arc<BytePairsOutcome>, ServerError>> {
        let cell = {
            let state = self
                .state
                .body
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &*state {
                BodyState::Form {
                    policy: existing,
                    result,
                } if *existing
                    == (FormReadPolicy::PartialBytes {
                        policy,
                        segment_limit,
                    }) =>
                {
                    self.state.body.form_cell(result)?
                }
                BodyState::Unread(_)
                | BodyState::Exclusive
                | BodyState::Routing
                | BodyState::Form { .. } => return None,
            }
        };
        let parsed = cell.try_lock().ok()?;
        parsed.result.as_ref().map(shared_partial_bytes)
    }

    /// Collect multipart non-file fields into the shared form owner.
    ///
    /// File parts are drained under the streaming policy, not published or stored.
    /// No fields are published when parsing or a selected bound fails. Query values
    /// are not included. This representation is incompatible with other form reads.
    pub async fn read_multipart_fields(
        &self,
        policy: MultipartPolicy,
        fields: NonZeroUsize,
        values: ByteFieldBounds,
    ) -> Result<Arc<ByteFormData>, ServerError> {
        let cell = self.claim_form(FormReadPolicy::MultipartFields {
            policy,
            fields,
            values,
        })?;
        let mut parsed = cell.lock().await;
        if let Some(result) = &parsed.result {
            return shared_multipart_fields(result);
        }
        let body = parsed.body.take().ok_or(ServerError::Disconnected)?;
        parsed.result = Some(Err(Arc::new(ServerError::Disconnected)));
        let result = collect_multipart_fields(body, self.headers(), policy, fields, values)
            .await
            .map(Arc::new)
            .map_err(Arc::new)
            .map(ParsedFormData::MultipartFields);
        parsed.result = Some(result);
        shared_multipart_fields(parsed.result.as_ref().ok_or(ServerError::Disconnected)?)
    }

    /// Collect multipart fields with an additional shared memory allowance.
    ///
    /// File content is drained, not retained; its memory allowance is accounted
    /// separately from the caller's encoded-body and part bounds.
    pub async fn read_accounted_multipart_fields(
        &self,
        policy: MultipartPolicy,
        fields: NonZeroUsize,
        values: ByteFieldBounds,
        file_memory: usize,
    ) -> Result<Arc<ByteFormData>, ServerError> {
        let cell = self.claim_form(FormReadPolicy::AccountedMultipartFields {
            policy,
            fields,
            values,
            file_memory,
        })?;
        let mut parsed = cell.lock().await;
        if let Some(result) = &parsed.result {
            return shared_multipart_fields(result);
        }
        let body = parsed.body.take().ok_or(ServerError::Disconnected)?;
        parsed.result = Some(Err(Arc::new(ServerError::Disconnected)));
        let result = collect_accounted_multipart_fields(
            body,
            self.headers(),
            policy,
            fields,
            values,
            file_memory,
        )
        .await
        .map(Arc::new)
        .map_err(Arc::new)
        .map(ParsedFormData::MultipartFields);
        parsed.result = Some(result);
        shared_multipart_fields(parsed.result.as_ref().ok_or(ServerError::Disconnected)?)
    }

    /// Consume an accounted multipart body into a caller-admitted field destination.
    /// Files are drained under the same accounting as the owned field collector.
    pub async fn read_accounted_multipart_into<T: MultipartFieldDestination>(
        &self,
        policy: MultipartPolicy,
        field_bytes: NonZeroUsize,
        file_memory: usize,
        destination: &mut T,
    ) -> Result<(), ServerError> {
        collect_accounted_multipart_into(
            self.claim()?,
            self.headers(),
            policy,
            field_bytes,
            file_memory,
            destination,
        )
        .await
    }

    /// Inspect a completed multipart field collection without claiming the body.
    /// Returns `None` for a different policy, no completed collection, or one in progress.
    #[must_use]
    pub fn peek_multipart_fields(
        &self,
        policy: MultipartPolicy,
        fields: NonZeroUsize,
        values: ByteFieldBounds,
    ) -> Option<Result<Arc<ByteFormData>, ServerError>> {
        let cell = {
            let state = self
                .state
                .body
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &*state {
                BodyState::Form {
                    policy: existing,
                    result,
                } if *existing
                    == (FormReadPolicy::MultipartFields {
                        policy,
                        fields,
                        values,
                    }) =>
                {
                    self.state.body.form_cell(result)?
                }
                BodyState::Unread(_)
                | BodyState::Exclusive
                | BodyState::Routing
                | BodyState::Form { .. } => return None,
            }
        };
        let parsed = cell.try_lock().ok()?;
        parsed.result.as_ref().map(shared_multipart_fields)
    }

    fn claim_form(&self, policy: FormReadPolicy) -> Result<FormCell<'_>, ServerError> {
        let mut state = self
            .state
            .body
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut *state {
            BodyState::Unread(body) => {
                if let RequestBody::Prepared(base) = &self.state.body {
                    let mut parsed = base
                        .form
                        .try_lock()
                        .map_err(|_| ServerError::Memory(MemoryError::Busy))?;
                    parsed.body = Some(body.take().ok_or(ServerError::BodyClaimed)?);
                    *state = BodyState::Form {
                        policy,
                        result: None,
                    };
                    Ok(FormCell::Prepared(&base.form))
                } else {
                    let body = body.take().ok_or(ServerError::BodyClaimed)?;
                    let result = Arc::new(tokio::sync::Mutex::new(ParsedForm {
                        body: Some(body),
                        result: None,
                    }));
                    *state = BodyState::Form {
                        policy,
                        result: Some(result.clone()),
                    };
                    Ok(FormCell::Ordinary(result))
                }
            }
            BodyState::Form {
                policy: existing,
                result,
            } if *existing == policy => self
                .state
                .body
                .form_cell(result)
                .ok_or(ServerError::Memory(MemoryError::ConstructionUnbound)),
            BodyState::Form { .. } => Err(ServerError::FormPolicyConflict),
            BodyState::Exclusive | BodyState::Routing => Err(ServerError::BodyClaimed),
        }
    }

    /// Validate the complete compatible form, then borrow a single decoded value.
    /// Repetitions are rejected before the reader runs. With a selected decoded
    /// slot, this shares a prepared snapshot with later compatible form readers.
    pub async fn with_form_value<A>(
        &self,
        policy: FormPolicy,
        name: &str,
        read: impl FnOnce(Option<&str>) -> Result<A, ServerError>,
    ) -> Result<A, ServerError> {
        let cell = self.claim_form(FormReadPolicy::Text {
            policy,
            reject_raw_semicolons: false,
        })?;
        let snapshot = {
            let mut parsed = cell.lock().await;
            self.parse_text_form(&mut parsed, policy, false, true)
                .await?;
            match parsed.result.as_ref().ok_or(ServerError::Disconnected)? {
                Ok(ParsedFormData::Text(form)) => TextFormRead::Ordinary(form.clone()),
                Ok(ParsedFormData::PreparedText { form, .. }) => {
                    TextFormRead::Prepared(form.try_clone()?)
                }
                Ok(
                    ParsedFormData::Bytes(_)
                    | ParsedFormData::PartialBytes(_)
                    | ParsedFormData::MultipartFields(_),
                ) => return Err(ServerError::FormPolicyConflict),
                Err(error) => return Err(ServerError::SharedInput(error.clone())),
            }
        };
        match snapshot {
            TextFormRead::Ordinary(form) => {
                match form.values.get(name).map(Vec::as_slice).unwrap_or(&[]) {
                    [] => read(None),
                    [value] => read(Some(value)),
                    _ => Err(ServerError::Duplicate(name.to_owned())),
                }
            }
            TextFormRead::Prepared(form) => {
                let mut selected = form
                    .pairs()
                    .filter_map(|(field, value)| (field == name).then_some(value));
                let value = selected.next();
                if selected.next().is_some() {
                    return Err(ServerError::Duplicate(name.to_owned()));
                }
                read(value)
            }
        }
    }

    async fn read_form_with(
        &self,
        policy: FormPolicy,
        reject_raw_semicolons: bool,
    ) -> Result<Arc<FormData>, ServerError> {
        let cell = self.claim_form(FormReadPolicy::Text {
            policy,
            reject_raw_semicolons,
        })?;
        let mut parsed = cell.lock().await;
        self.parse_text_form(&mut parsed, policy, reject_raw_semicolons, false)
            .await?;
        shared_text(parsed.result.as_mut().ok_or(ServerError::Disconnected)?)
    }

    async fn parse_text_form(
        &self,
        parsed: &mut ParsedForm,
        policy: FormPolicy,
        reject_raw_semicolons: bool,
        prefer_prepared: bool,
    ) -> Result<(), ServerError> {
        if parsed.result.is_some() {
            return Ok(());
        }
        if parsed.body.is_none() {
            return Err(ServerError::Disconnected);
        }
        let output = if prefer_prepared {
            self.take_endpoint_bytes(|leases| &mut leases.decoded)?
        } else {
            None
        };
        let body = parsed.body.take().ok_or(ServerError::Disconnected)?;
        let result = async {
            let bytes = self.read_admitted_body(body, policy.body).await?;
            if let Some(output) = output {
                let form =
                    PreparedForm::parse(bytes.as_ref(), output, policy, reject_raw_semicolons)?;
                return Ok(ParsedFormData::PreparedText {
                    form,
                    materialized: None,
                });
            }
            let text = std::str::from_utf8(bytes.as_ref())
                .map_err(|_| ServerError::Invalid("form UTF-8".into()))?;
            let values = decode_text_pairs(text.as_bytes(), reject_raw_semicolons)?;
            if values.len() > policy.fields.get() {
                return Err(ServerError::Limit("form fields".into()));
            }
            for (name, values) in &values {
                policy.values.check(name, values)?;
            }
            Ok(ParsedFormData::Text(Arc::new(FormData { values })))
        }
        .await
        .map_err(Arc::new);
        parsed.result = Some(result);
        Ok(())
    }

    async fn read_byte_form_with(
        &self,
        policy: ByteFormPolicy,
        reject_raw_semicolons: bool,
    ) -> Result<Arc<ByteFormData>, ServerError> {
        let cell = self.claim_form(FormReadPolicy::Bytes {
            policy,
            reject_raw_semicolons,
        })?;
        let mut parsed = cell.lock().await;
        if let Some(result) = &parsed.result {
            return shared_bytes(result);
        }
        let body = parsed.body.take().ok_or(ServerError::Disconnected)?;
        let result: Result<Arc<ByteFormData>, Arc<ServerError>> = async {
            let bytes = self.read_admitted_body(body, policy.body).await?;
            let values = decode_url_encoded_pairs(bytes.as_ref(), reject_raw_semicolons)?;
            if values.len() > policy.fields.get() {
                return Err(ServerError::Limit("form fields".into()));
            }
            for values in values.values() {
                policy.values.check(values)?;
            }
            Ok(Arc::new(ByteFormData { values }))
        }
        .await
        .map_err(Arc::new);
        let result = result.map(ParsedFormData::Bytes);
        parsed.result = Some(result);
        shared_bytes(parsed.result.as_ref().ok_or(ServerError::Disconnected)?)
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
    initial_snapshot: Option<crate::prepared_request_storage::SnapshotVacancy>,
}
/// An admitted metadata snapshot for a checked routing stage.
pub struct RoutingSnapshotReservation(pub(crate) Option<crate::prepared_request_storage::SnapshotVacancy>);
impl RoutingSnapshotReservation {
    /// Publish the stage's current metadata, including edits made since admission.
    pub fn publish(self, routing: &RoutingRequest) -> Result<Request, MemoryError> {
        match self.0 {
            Some(vacancy) => routing.request_with_vacancy(vacancy),
            None => Ok(routing.request()),
        }
    }
}
impl RoutingRequest {
    pub(crate) fn from_reserved_native(native: http::Request<Body>, cancellation: Cancellation, reservation: crate::prepared_request_storage::PreparedRequestStorageReservation, initial_snapshot: crate::prepared_request_storage::SnapshotVacancy) -> Result<Self, effect::memory::MemoryError> {
        let request = Request::from_reserved_native(native, cancellation, reservation)?;
        let parts = request.state.parts.clone();
        Ok(Self { parts, request, initial_snapshot: Some(initial_snapshot) })
    }
    pub(crate) fn take_initial_request(&mut self) -> Result<Request, effect::memory::MemoryError> {
        let vacancy = self.initial_snapshot.take().ok_or(effect::memory::MemoryError::ConstructionUnbound)?;
        self.request_with_vacancy(vacancy)
    }
    /// Create a pre-routing owner using caller-selected finite request storage.
    /// On refusal, the unadmitted native request and cancellation are consumed
    /// and physical reservations are returned before the error is returned.
    pub fn try_from_native_in(native: http::Request<Body>, cancellation: Cancellation, pool: &Arc<crate::PreparedRequestStoragePool>) -> Result<Self, effect::memory::MemoryError> {
        let request = Request::try_from_native_in(native, cancellation, pool)?;
        let parts = request.state.parts.clone();
        Ok(Self { parts, request, initial_snapshot: None })
    }
    pub(crate) fn reserve_snapshot(&self) -> Result<crate::prepared_request_storage::SnapshotVacancy, effect::memory::MemoryError> {
        match &self.request.state {
            RequestOwner::Prepared { pool, .. } => pool.reserve_snapshot(),
            RequestOwner::Ordinary(_) => Err(effect::memory::MemoryError::UnsupportedRunContract),
        }
    }
    pub(crate) fn request_with_vacancy(&self, vacancy: crate::prepared_request_storage::SnapshotVacancy) -> Result<Request, effect::memory::MemoryError> {
        let RequestOwner::Prepared { pool, .. } = &self.request.state else { return Err(effect::memory::MemoryError::UnsupportedRunContract); };
        self.request.publish_parts(self.parts.clone(), vacancy, Arc::clone(pool))
    }
    /// Retain a fresh metadata snapshot with the same actual body and extensions.
    pub fn try_request(&self) -> Result<Request, effect::memory::MemoryError> {
        self.request.try_with_parts(&self.parts)
    }
    /// Create a pre-routing owner without reading or copying body bytes.
    pub(crate) fn from_native(native: http::Request<Body>, cancellation: Cancellation) -> Self {
        let request = Request::from_native(native, cancellation);
        let parts = request.state.parts.clone();
        Self { parts, request, initial_snapshot: None }
    }
    pub(crate) fn cancellation(&self) -> &Cancellation { self.request.cancellation() }
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
    pub(crate) fn into_native(self) -> http::Request<Body> {
        let request = self.request();
        self.into_native_with_request(request)
    }
    #[allow(clippy::result_large_err)] // Preserve the complete routing owner on physical refusal.
    pub(crate) fn into_native_with_vacancy(self, vacancy: crate::prepared_request_storage::SnapshotVacancy) -> Result<http::Request<Body>, (effect::memory::MemoryError, Self)> {
        match self.request_with_vacancy(vacancy) {
            Ok(request) => Ok(self.into_native_with_request(request)),
            Err(error) => Err((error, self)),
        }
    }
    fn into_native_with_request(mut self, request: Request) -> http::Request<Body> {
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
#[derive(crate::EffectEnvironment)]
#[effect_environment(
    request = request,
    cancellation = self.request.cancellation(),
    tokio(get = crate::HasTokio::tokio(self.services.as_ref()), requires(R: crate::HasTokio)),
    task_spawner(
        provider = <R as effect::HasTaskSpawner>::Tasks,
        get = effect::HasTaskSpawner::task_spawner(self.services.as_ref()),
        requires(R: effect::HasTaskSpawner)
    ),
    http_transport(
        provider = <R as effect::HasHttpTransport>::Transport,
        get = effect::HasHttpTransport::http_transport(self.services.as_ref()),
        requires(R: effect::HasHttpTransport)
    ),
    blocking_executor(
        provider = <R as effect::HasBlockingExecutor>::Executor,
        get = effect::HasBlockingExecutor::blocking_executor(self.services.as_ref()),
        requires(R: effect::HasBlockingExecutor)
    ),
    cpu_executor(
        provider = <R as effect::HasCpuExecutor>::Executor,
        get = effect::HasCpuExecutor::cpu_executor(self.services.as_ref()),
        requires(R: effect::HasCpuExecutor)
    ),
    clock(
        provider = <R as effect::HasClock>::Clock,
        get = effect::HasClock::clock(self.services.as_ref()),
        requires(R: effect::HasClock)
    ),
    tracer(
        get = effect::HasTracer::tracer(self.services.as_ref()),
        requires(R: effect::HasTracer)
    ),
    logger(
        get = effect::HasLogger::logger(self.services.as_ref()),
        requires(R: effect::HasLogger)
    ),
    server_error(
        error = <R as crate::HasServerError>::Error,
        map = |error| crate::HasServerError::server_error(self.services.as_ref(), error),
        requires(R: crate::HasServerError)
    )
)]
pub struct RequestEnv<R> {
    /// Immutable application services and runtime capabilities.
    pub services: Arc<R>,
    /// Fresh metadata/body owner for this request.
    pub request: Request,
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
        RequestInput::new(effect_provide_some! { program: self.effect.clone(), project: project, })
    }
}
/// Incremental multipart policy with separate body/part/header/field resource bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
async fn collect_accounted_multipart_fields(
    body: Body,
    headers: &HeaderMap,
    policy: MultipartPolicy,
    fields: NonZeroUsize,
    bounds: ByteFieldBounds,
    file_memory: usize,
) -> Result<ByteFormData, ServerError> {
    let mut destination = OwnedMultipartFields {
        fields,
        bounds,
        values: BTreeMap::new(),
        pending_name: Vec::new(),
        pending_value: Vec::new(),
    };
    collect_accounted_multipart_into(
        body, headers, policy, bounds.bytes, file_memory, &mut destination,
    ).await?;
    Ok(ByteFormData { values: destination.values })
}

/// A bounded destination for one accounted multipart field at a time.
pub trait MultipartFieldDestination: Send {
    /// Begin a non-file field with a nonempty name.
    fn begin_field(&mut self, name: &[u8]) -> Result<(), ServerError>;
    /// Append raw bytes of the current field.
    fn append_field(&mut self, bytes: &[u8]) -> Result<(), ServerError>;
    /// Complete the current field and check aggregate limits.
    fn finish_field(&mut self) -> Result<(), ServerError>;
}

struct OwnedMultipartFields {
    fields: NonZeroUsize,
    bounds: ByteFieldBounds,
    values: BytePairs,
    pending_name: Vec<u8>,
    pending_value: Vec<u8>,
}

impl MultipartFieldDestination for OwnedMultipartFields {
    fn begin_field(&mut self, name: &[u8]) -> Result<(), ServerError> {
        self.pending_name = name.to_vec();
        Ok(())
    }

    fn append_field(&mut self, bytes: &[u8]) -> Result<(), ServerError> {
        self.pending_value.extend_from_slice(bytes);
        Ok(())
    }

    fn finish_field(&mut self) -> Result<(), ServerError> {
        let entries = self.values.entry(std::mem::take(&mut self.pending_name)).or_default();
        entries.push(std::mem::take(&mut self.pending_value));
        self.bounds.check(entries)?;
        if self.values.len() > self.fields.get() {
            return Err(ServerError::Limit("form fields".into()));
        }
        Ok(())
    }
}

async fn collect_accounted_multipart_into<T: MultipartFieldDestination>(
    body: Body,
    headers: &HeaderMap,
    policy: MultipartPolicy,
    field_bytes: NonZeroUsize,
    file_memory: usize,
    destination: &mut T,
) -> Result<(), ServerError> {
    let mut request = http::Request::new(Body::new(Limited::new(body, policy.body.bytes())));
    *request.headers_mut() = headers.clone();
    request
        .extensions_mut()
        .insert(axum::extract::DefaultBodyLimit::disable());
    let mut parser = Multipart::from_request(request, &())
        .await
        .map_err(|error| ServerError::MultipartRejection(error.to_string()))?;
    let mut remaining = file_memory.saturating_add(10 * 1024 * 1024) as u128;
    let mut file_remaining = file_memory as u128;
    let mut headers_remaining = 10_000usize;
    let mut part_count = 0usize;
    while let Some(mut part) = parser.next_field().await.map_err(ServerError::Multipart)? {
        part_count += 1;
        if part_count > policy.parts.get() {
            return Err(ServerError::Limit("multipart count".into()));
        }
        let header_count = part.headers().values().count();
        if header_count > headers_remaining {
            return Err(ServerError::Limit("multipart headers".into()));
        }
        let mut header_bytes = 0usize;
        let mut header_memory = 400u128;
        for name in part.headers().keys() {
            header_memory += name.as_str().len() as u128 + 200;
            for value in part.headers().get_all(name).iter() {
                header_bytes = header_bytes
                    .saturating_add(name.as_str().len())
                    .saturating_add(value.as_bytes().len());
                header_memory += value.as_bytes().len() as u128;
            }
        }
        if header_bytes > policy.header_bytes.get() {
            return Err(ServerError::Limit("multipart headers".into()));
        }
        if header_memory > remaining {
            return Err(ServerError::Limit("multipart memory".into()));
        }
        let name = part.name().unwrap_or("").as_bytes();
        let named = !name.is_empty();
        let file = part.file_name().is_some_and(|name| !name.is_empty());
        if named && file {
            headers_remaining -= header_count;
        }
        if named {
            let charge = name.len() as u128 + 200 + if file { header_memory + 300 } else { 0 };
            remaining = remaining
                .checked_sub(charge)
                .ok_or_else(|| ServerError::Limit("multipart memory".into()))?;
        }
        if named && !file {
            destination.begin_field(name)?;
        }
        let mut length = 0usize;
        while let Some(chunk) = part.chunk().await.map_err(ServerError::Multipart)? {
            length = length
                .checked_add(chunk.len())
                .ok_or_else(|| ServerError::Limit("multipart part".into()))?;
            if length > policy.part_bytes.get() {
                return Err(ServerError::Limit("multipart part".into()));
            }
            if named && !file {
                if length > policy.field_bytes.get() || length > field_bytes.get() {
                    return Err(ServerError::Limit("multipart field".into()));
                }
                remaining = remaining
                    .checked_sub(chunk.len() as u128)
                    .ok_or_else(|| ServerError::Limit("multipart memory".into()))?;
                destination.append_field(&chunk)?;
            }
        }
        if named && file && (length as u128) <= file_remaining {
            file_remaining -= length as u128;
            remaining = remaining.saturating_sub(length as u128);
        } else if named && !file {
            destination.finish_field()?;
        }
    }
    Ok(())
}

async fn collect_multipart_fields(
    body: Body,
    headers: &HeaderMap,
    policy: MultipartPolicy,
    fields: NonZeroUsize,
    bounds: ByteFieldBounds,
) -> Result<ByteFormData, ServerError> {
    let mut request = http::Request::new(Body::new(Limited::new(body, policy.body.bytes())));
    *request.headers_mut() = headers.clone();
    request
        .extensions_mut()
        .insert(axum::extract::DefaultBodyLimit::disable());
    let parser = Multipart::from_request(request, &())
        .await
        .map_err(|error| ServerError::MultipartRejection(error.to_string()))?;
    let mut reader = MultipartReader {
        parser,
        policy,
        count: 0,
    };
    let mut collected: BytePairs = BTreeMap::new();
    while let Some(part) = reader
        .next(|mut part| {
            Box::pin(async move {
                let name = part.name().unwrap_or("").as_bytes().to_vec();
                if name.is_empty() || part.file_name().is_some_and(|name| !name.is_empty()) {
                    return Ok(None);
                }
                let mut value = Vec::new();
                while let Some(chunk) = part.chunk().await? {
                    if chunk.len() > policy.field_bytes.get().saturating_sub(value.len())
                        || chunk.len() > bounds.bytes.get().saturating_sub(value.len())
                    {
                        return Err(ServerError::Limit("multipart field".into()));
                    }
                    value.extend_from_slice(&chunk);
                }
                Ok(Some((name, value)))
            })
        })
        .await?
    {
        if let Some((name, value)) = part {
            let entries = collected.entry(name).or_default();
            entries.push(value);
            bounds.check(entries)?;
            if collected.len() > fields.get() {
                return Err(ServerError::Limit("form fields".into()));
            }
        }
    }
    Ok(ByteFormData { values: collected })
}

enum AdmittedBody {
    Ordinary(Bytes),
    Prepared(BufferView<'static>),
}
impl AsRef<[u8]> for AdmittedBody {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Ordinary(bytes) => bytes.as_ref(),
            Self::Prepared(view) => view.as_ref(),
        }
    }
}

async fn read_prepared_bounded(
    mut body: Body,
    mut buffer: BufferLease<'static>,
    limit: usize,
) -> Result<BufferView<'static>, ServerError> {
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| ServerError::Transport(effect::Defect::new(error.to_string())))?;
        if let Some(bytes) = frame.data_ref() {
            if bytes.len() > limit.saturating_sub(buffer.len()) {
                return Err(ServerError::Limit("body bytes".into()));
            }
            buffer.extend_from_slice(bytes).map_err(ServerError::Memory)?;
        }
    }
    Ok(BufferView::new(buffer.share()))
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
/// Decode URL-encoded pairs while preserving key and value bytes.
///
/// A raw semicolon is refused before decoding when requested; a percent-encoded
/// semicolon stays data. A malformed percent escape is an error, while valid
/// non-text decoded bytes are retained. `+` decodes to a space, keys and values are
/// not trimmed, and repeated values keep wire order.
pub fn decode_url_encoded_pairs(
    encoded: &[u8],
    reject_raw_semicolons: bool,
) -> Result<BytePairs, ServerError> {
    let mut fields: BytePairs = BTreeMap::new();
    visit_url_encoded_component_pairs(encoded, reject_raw_semicolons, |raw_key, raw_value| {
        fields
            .entry(decode_url_encoded_component(raw_key)?)
            .or_default()
            .push(decode_url_encoded_component(raw_value)?);
        Ok(())
    })?;
    Ok(fields)
}

/// Decode valid URL-encoded byte pairs while retaining a syntax diagnostic.
///
/// Empty segments are ignored; empty keys and values are retained. Segments with
/// raw semicolons or malformed escapes are skipped, and later segments are still
/// decoded. Escaped semicolons remain data. A raw semicolon replaces any earlier
/// diagnostic; an invalid escape is recorded only when no diagnostic exists.
/// This pure operation applies no body or field limits and does not claim a request.
#[must_use]
pub fn decode_url_encoded_pairs_partial(encoded: &[u8]) -> BytePairsOutcome {
    let mut outcome = BytePairsOutcome {
        values: BTreeMap::new(),
        diagnostic: None,
    };
    for segment in encoded.split(|byte| *byte == b'&') {
        if segment.contains(&b';') {
            outcome.diagnostic = Some(ServerError::Invalid("form separator".into()));
            continue;
        }
        if segment.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = split_form_segment(segment);
        let pair = decode_url_encoded_component(raw_key)
            .and_then(|key| decode_url_encoded_component(raw_value).map(|value| (key, value)));
        match pair {
            Ok((key, value)) => outcome.values.entry(key).or_default().push(value),
            Err(error) => {
                if outcome.diagnostic.is_none() {
                    outcome.diagnostic = Some(error);
                }
            }
        }
    }
    outcome
}

fn visit_url_encoded_component_pairs(
    encoded: &[u8],
    reject_raw_semicolons: bool,
    mut visit: impl FnMut(&[u8], &[u8]) -> Result<(), ServerError>,
) -> Result<(), ServerError> {
    for segment in encoded.split(|byte| *byte == b'&') {
        if reject_raw_semicolons && segment.contains(&b';') {
            return Err(ServerError::Invalid("form separator".into()));
        }
        if segment.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = split_form_segment(segment);
        visit(raw_key, raw_value)?;
    }
    Ok(())
}

fn split_form_segment(segment: &[u8]) -> (&[u8], &[u8]) {
    match segment.iter().position(|byte| *byte == b'=') {
        Some(index) => {
            // position is inside this same byte slice; the tail contains '='.
            let (key, tail) = segment.split_at(index);
            let (_, value) = tail.split_at(1);
            (key, value)
        }
        None => (segment, &[]),
    }
}

fn decode_url_encoded_component(raw: &[u8]) -> Result<Vec<u8>, ServerError> {
    let mut decoded = Vec::with_capacity(raw.len());
    visit_decoded_component(raw, |byte| {
        decoded.push(byte);
        Ok(())
    })?;
    Ok(decoded)
}

/// Append one byte-valued URL-encoded component to prepared output.
/// A malformed escape or a capacity failure restores the original prefix.
pub fn append_decoded_url_encoded_component(
    raw: &[u8],
    output: &mut effect::memory::Buffer<'_>,
) -> Result<(), ServerError> {
    let start = output.len();
    let mut output = std::mem::DropGuard::new(output, move |output| output.truncate(start));
    visit_decoded_component(raw, |byte| {
        output.extend_from_slice(&[byte]).map_err(ServerError::Memory)
    })?;
    let _ = std::mem::DropGuard::dismiss(output);
    Ok(())
}

fn visit_decoded_component(
    raw: &[u8],
    mut emit: impl FnMut(u8) -> Result<(), ServerError>,
) -> Result<(), ServerError> {
    let mut index = 0_usize;
    while let Some(byte) = raw.get(index) {
        match byte {
            b'+' => {
                emit(b' ')?;
                index += 1;
            }
            b'%' => {
                let high = raw
                    .get(index + 1)
                    .copied()
                    .and_then(hex_digit)
                    .ok_or_else(|| ServerError::Invalid("percent encoding".into()))?;
                let low = raw
                    .get(index + 2)
                    .copied()
                    .and_then(hex_digit)
                    .ok_or_else(|| ServerError::Invalid("percent encoding".into()))?;
                emit(high * 16 + low)?;
                index += 3;
            }
            other => {
                emit(*other)?;
                index += 1;
            }
        }
    }
    Ok(())
}

fn decode_text_pairs(
    encoded: &[u8],
    reject_raw_semicolons: bool,
) -> Result<BTreeMap<String, Vec<String>>, ServerError> {
    let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
    visit_url_encoded_component_pairs(encoded, reject_raw_semicolons, |raw_key, raw_value| {
        let key = decode_text_component(raw_key)?;
        let value = decode_text_component(raw_value)?;
        fields.entry(key).or_default().push(value);
        Ok(())
    })?;
    Ok(fields)
}

fn decode_text_component(raw: &[u8]) -> Result<String, ServerError> {
    String::from_utf8(decode_url_encoded_component(raw)?)
        .map_err(|_| ServerError::Invalid("form UTF-8".into()))
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn shared_text(
    result: &mut Result<ParsedFormData, Arc<ServerError>>,
) -> Result<Arc<FormData>, ServerError> {
    match result {
        Ok(ParsedFormData::Text(data)) => Ok(data.clone()),
        Ok(ParsedFormData::PreparedText { form, materialized }) => Ok(materialized
            .get_or_insert_with(|| {
                let mut values = BTreeMap::<String, Vec<String>>::new();
                for (name, value) in form.pairs() {
                    if let Some(repeated) = values.get_mut(name) {
                        repeated.push(value.to_owned());
                    } else {
                        values.insert(name.to_owned(), vec![value.to_owned()]);
                    }
                }
                Arc::new(FormData { values })
            })
            .clone()),
        Ok(
            ParsedFormData::Bytes(_)
            | ParsedFormData::PartialBytes(_)
            | ParsedFormData::MultipartFields(_),
        ) => Err(ServerError::FormPolicyConflict),
        Err(error) => Err(ServerError::SharedInput(error.clone())),
    }
}

fn shared_bytes(
    result: &Result<ParsedFormData, Arc<ServerError>>,
) -> Result<Arc<ByteFormData>, ServerError> {
    match result {
        Ok(ParsedFormData::Bytes(data)) => Ok(data.clone()),
        Ok(
            ParsedFormData::Text(_)
            | ParsedFormData::PreparedText { .. }
            | ParsedFormData::PartialBytes(_)
            | ParsedFormData::MultipartFields(_),
        ) => Err(ServerError::FormPolicyConflict),
        Err(error) => Err(ServerError::SharedInput(error.clone())),
    }
}

fn shared_partial_bytes(
    result: &Result<ParsedFormData, Arc<ServerError>>,
) -> Result<Arc<BytePairsOutcome>, ServerError> {
    match result {
        Ok(ParsedFormData::PartialBytes(data)) => Ok(data.clone()),
        Ok(
            ParsedFormData::Text(_)
            | ParsedFormData::PreparedText { .. }
            | ParsedFormData::Bytes(_)
            | ParsedFormData::MultipartFields(_),
        ) => Err(ServerError::FormPolicyConflict),
        Err(error) => Err(ServerError::SharedInput(error.clone())),
    }
}

fn shared_multipart_fields(
    result: &Result<ParsedFormData, Arc<ServerError>>,
) -> Result<Arc<ByteFormData>, ServerError> {
    match result {
        Ok(ParsedFormData::MultipartFields(data)) => Ok(data.clone()),
        Ok(_) => Err(ServerError::FormPolicyConflict),
        Err(error) => Err(ServerError::SharedInput(error.clone())),
    }
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
        let effect = effect! {
        bind;
        program: service! { env: |env: &R| env.request().clone(), },
        body: move |request| {
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
                       (_, Err(error)) => effect_from_result! { once; result: Err(map(error)), },
                       _ => effect_from_result! {
                           once; result: Err(map(ServerError::Invalid("unsupported content type".into()))),
                       },
                   }
               },
        };
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
                effect_try_future! { once; body: move |_| async move { request.read_bytes(limit).await }, on_error: move |error| map(error), }
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
                effect_try_future! { once; body: move |_| async move { request.read_text(limit).await }, on_error: move |error| map(error), }
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
                effect_try_future! { once; body: move |_| async move { request.read_json(limit, policy, &codec).await }, on_error: move |error| map(error), }
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
        Self::new(effect! {
        bind;
        program: service! { env: |env: &R| env.request().clone(), },
        body: move |request| {
                       let codec = codec.clone();
                       let map = map.clone();
                       let empty = empty.clone();
                       effect_try_future! {
                           once; body: move |_| async move {
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
                           }, on_error: move |error| map(error), }
                   },
        })
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
                effect_try_future! { once; body: move |_| async move { request.read_form(policy).await }, on_error: move |error| map(error), }
            },
            move |error| map(error),
        )
    }
}

/// Borrowed textual scalar decoder.
type ScalarDecoder<A> = dyn Fn(&str) -> Result<A, ServerError> + Send + Sync;

/// Borrowed application scalar encoder.
type ScalarEncoder<A> = dyn Fn(&A) -> Result<String, ServerError> + Send + Sync;
