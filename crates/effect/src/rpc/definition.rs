//! Operation identities, codecs, and compatibility versions.

use crate::{Effect, JsonValue, ParseLimits, PureCodec, SchemaIssue};
use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::Arc;

/// Type alias for idempotency key in rpc definition.
///
/// # Type parameters
/// - `Q`: Request type.
type IdempotencyKey<Q> = dyn Fn(&Q) -> Result<Arc<[u8]>, SchemaIssue> + Send + Sync;

/// Stable, application-owned identity. Renaming an operation is a protocol
/// migration; source locations and Rust type names are never wire identities.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(Arc<str>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// An invalid stable operation identity or version.
pub enum OperationIdError {
    /// No value is present.
    Empty,
    /// The character failed validation.
    InvalidCharacter,
}

impl OperationId {
    /// Construct a `OperationId` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `value` (`impl Into<Arc<str>>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, OperationIdError>`.
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, OperationIdError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(OperationIdError::Empty);
        }
        if value.chars().any(|character| character.is_control()) {
            return Err(OperationIdError::InvalidCharacter);
        }
        Ok(Self(value))
    }

    #[must_use]
    /// Borrow the textual representation without allocating a new string.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Application-selected compatibility revision. Zero is excluded by type,
/// while the exact revision is never chosen by the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RpcVersion(NonZeroU32);

impl RpcVersion {
    #[must_use]
    /// # Parameters
    /// - `value` (`NonZeroU32`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(value: NonZeroU32) -> Self {
        Self(value)
    }

    #[must_use]
    /// # Returns
    /// Type: `NonZeroU32`.
    pub const fn value(self) -> NonZeroU32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The unary or streaming shape declared by an RPC operation.
pub enum RpcOperationKind {
    /// The RPC operation produces one terminal reply.
    Unary,
    /// The RPC operation produces a sequence of acknowledged items.
    Stream,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The stable identity, version, and codecs of one typed RPC operation.
pub struct RpcOperationSpec {
    /// Stable identity of this record. Type: `OperationId`.
    pub id: OperationId,
    /// The declared compatibility version. Type: `RpcVersion`.
    pub version: RpcVersion,
    /// Typed discriminator for this state. Type: `RpcOperationKind`.
    pub kind: RpcOperationKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A duplicate or incompatible operation registration.
pub enum RpcRegistryError {
    /// The operation already exists or occurred twice.
    DuplicateOperation {
        /// Stable identity of this record. Type: `OperationId`.
        id: OperationId,
        /// The declared compatibility version. Type: `RpcVersion`.
        version: RpcVersion,
    },
    /// An incompatible operation kind is already recorded.
    OperationKindConflict {
        /// Stable identity of this record. Type: `OperationId`.
        id: OperationId,
        /// The declared compatibility version. Type: `RpcVersion`.
        version: RpcVersion,
    },
}

/// Registration validates identities before any call reaches dispatch. The
/// typed handler map built on this registry must keep its erased representation
/// private, so consumers never downcast request or response values.
#[derive(Clone)]
pub struct RpcRegistry {
    operations: BTreeMap<(OperationId, RpcVersion), RpcOperationKind>,
}

impl RpcRegistry {
    #[must_use]
    /// Construct a `RpcRegistry` from its required inputs and explicit policies.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new() -> Self {
        Self {
            operations: BTreeMap::new(),
        }
    }

    /// Register the supplied definition, rejecting an identity or compatibility conflict.
    ///
    /// # Parameters
    /// - `spec` (`&RpcOperationSpec`): The spec supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), RpcRegistryError>`.
    pub fn register(&mut self, spec: &RpcOperationSpec) -> Result<(), RpcRegistryError> {
        let key = (spec.id.clone(), spec.version);
        match self.operations.get(&key) {
            Some(existing) if *existing != spec.kind => {
                Err(RpcRegistryError::OperationKindConflict {
                    id: spec.id.clone(),
                    version: spec.version,
                })
            }
            Some(_) => Err(RpcRegistryError::DuplicateOperation {
                id: spec.id.clone(),
                version: spec.version,
            }),
            None => {
                self.operations.insert(key, spec.kind);
                Ok(())
            }
        }
    }

    #[must_use]
    /// Return the typed category of this value.
    ///
    /// # Parameters
    /// - `id` (`&OperationId`): Stable identity of the selected record or operation.
    /// - `version` (`RpcVersion`): Registered schema, event, or workflow compatibility version.
    ///
    /// # Returns
    /// Type: `Option<RpcOperationKind>`.
    /// Returns the value when present, or `None`.
    pub fn kind(&self, id: &OperationId, version: RpcVersion) -> Option<RpcOperationKind> {
        self.operations.get(&(id.clone(), version)).copied()
    }

    /// Inspect the operations registered in this protocol.
    ///
    /// # Returns
    /// Type: `impl Iterator<Item = RpcOperationSpec> + '_`.
    pub fn operations(&self) -> impl Iterator<Item = RpcOperationSpec> + '_ {
        self.operations
            .iter()
            .map(|((id, version), kind)| RpcOperationSpec {
                id: id.clone(),
                version: *version,
                kind: *kind,
            })
    }
}

impl Default for RpcRegistry {
    ///
    /// # Returns
    /// Type: `Self`.
    fn default() -> Self {
        Self::new()
    }
}

/// One unary operation links request, success, and domain-error schemas.
/// The same description can serve in-process and encoded transports.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub struct RpcOperation<Q, A, E> {
    spec: RpcOperationSpec,
    request: PureCodec<Q, JsonValue>,
    success: PureCodec<A, JsonValue>,
    domain_error: PureCodec<E, JsonValue>,
    limits: ParseLimits,
    idempotency_key: Option<Arc<IdempotencyKey<Q>>>,
}

impl<Q, A, E> Clone for RpcOperation<Q, A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            request: self.request.clone(),
            success: self.success.clone(),
            domain_error: self.domain_error.clone(),
            limits: self.limits,
            idempotency_key: self.idempotency_key.as_ref().map(Arc::clone),
        }
    }
}

impl<Q: Send + 'static, A: Send + 'static, E: Send + 'static> RpcOperation<Q, A, E> {
    #[must_use]
    /// Define a unary RPC operation with separate request and reply codecs.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `id` (`OperationId`): Stable identity of the selected record or operation.
    /// - `version` (`RpcVersion`): Registered schema, event, or workflow compatibility version.
    /// - `request` (`PureCodec<Q, JsonValue>`): The typed request to process.
    /// - `success` (`PureCodec<A, JsonValue>`): Successful application value or its codec.
    /// - `domain_error` (`PureCodec<E, JsonValue>`): The domain error supplied to this operation.
    /// - `limits` (`ParseLimits`): Explicit resource bounds enforced while processing this input.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn unary(
        id: OperationId,
        version: RpcVersion,
        request: PureCodec<Q, JsonValue>,
        success: PureCodec<A, JsonValue>,
        domain_error: PureCodec<E, JsonValue>,
        limits: ParseLimits,
    ) -> Self {
        Self {
            spec: RpcOperationSpec {
                id,
                version,
                kind: RpcOperationKind::Unary,
            },
            request,
            success,
            domain_error,
            limits,
            idempotency_key: None,
        }
    }

    #[must_use]
    /// Attach a stable request key extractor for safe duplicate recognition.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `key` (`impl Fn(&Q) -> Result<Arc<[u8]>, SchemaIssue> + Send + Sync + 'static`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn with_idempotency_key(
        mut self,
        key: impl Fn(&Q) -> Result<Arc<[u8]>, SchemaIssue> + Send + Sync + 'static,
    ) -> Self {
        self.idempotency_key = Some(Arc::new(key));
        self
    }

    #[must_use]
    /// Borrow the stable operation specification and its codecs.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `&RpcOperationSpec`.
    pub fn spec(&self) -> &RpcOperationSpec {
        &self.spec
    }

    #[must_use]
    /// Borrow the codec used to encode and decode requests.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `Q`: Request type.
    ///
    /// # Returns
    /// Type: `&PureCodec<Q, JsonValue>`.
    pub fn request_codec(&self) -> &PureCodec<Q, JsonValue> {
        &self.request
    }

    #[must_use]
    /// Borrow the codec used for successful RPC replies.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `&PureCodec<A, JsonValue>`.
    pub fn success_codec(&self) -> &PureCodec<A, JsonValue> {
        &self.success
    }

    #[must_use]
    /// Borrow the codec used for expected domain errors.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `&PureCodec<E, JsonValue>`.
    pub fn error_codec(&self) -> &PureCodec<E, JsonValue> {
        &self.domain_error
    }

    #[must_use]
    /// # Type parameters
    /// - `Q`: Request type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `ParseLimits`.
    pub const fn limits(&self) -> ParseLimits {
        self.limits
    }

    /// Derive the stable idempotency key for this request when one is configured.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `request` (`&Q`): The typed request to process.
    ///
    /// # Returns
    /// Type: `Result<Option<Arc<[u8]>>, SchemaIssue>`.
    pub fn idempotency_key(&self, request: &Q) -> Result<Option<Arc<[u8]>>, SchemaIssue> {
        self.idempotency_key
            .as_ref()
            .map(|key| key(request))
            .transpose()
    }

    /// Change only the typed domain-error representation. Request/success
    /// codecs, stable identity, version and original idempotency extractor
    /// remain intact through protocol/middleware wrapping.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `Next`: Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `domain_error` (`PureCodec<Next, JsonValue>`): The domain error supplied to this operation.
    ///
    /// # Returns
    /// Type: `RpcOperation<Q, A, Next>`.
    pub fn with_error_codec<Next: Send + 'static>(
        &self,
        domain_error: PureCodec<Next, JsonValue>,
    ) -> RpcOperation<Q, A, Next> {
        RpcOperation {
            spec: self.spec.clone(),
            request: self.request.clone(),
            success: self.success.clone(),
            domain_error,
            limits: self.limits,
            idempotency_key: self.idempotency_key.as_ref().map(Arc::clone),
        }
    }
}

/// The in-process pairing is checked by Rust's type system. No downcast or
/// encoded round-trip is required for a typed local invocation.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
pub struct RpcHandler<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    operation: RpcOperation<Q, A, E>,
    invoke: Arc<dyn Fn(Q) -> Effect<A, E, R> + Send + Sync>,
}

impl<Q, A, E, R> Clone for RpcHandler<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            operation: self.operation.clone(),
            invoke: Arc::clone(&self.invoke),
        }
    }
}

impl<Q, A, E, R> RpcHandler<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    /// Construct a `RpcHandler` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `operation` (`RpcOperation<Q, A, E>`): Typed operation definition or callback selected for execution.
    /// - `invoke` (`impl Fn(Q) -> Effect<A, E, R> + Send + Sync + 'static`): The invoke supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        operation: RpcOperation<Q, A, E>,
        invoke: impl Fn(Q) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            operation,
            invoke: Arc::new(invoke),
        }
    }

    #[must_use]
    /// Borrow the typed operation definition associated with this value.
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    ///
    /// # Returns
    /// Type: `&RpcOperation<Q, A, E>`.
    pub fn operation(&self) -> &RpcOperation<Q, A, E> {
        &self.operation
    }

    /// Project each invocation into this handler's R through existing Effect
    /// provision. Request ownership and operation identity remain unchanged;
    /// projection runs only when the returned Effect executes.
    pub fn provide_some<Outer: Send + Sync + 'static>(
        self,
        project: impl Fn(&Outer) -> R + Send + Sync + 'static,
    ) -> RpcHandler<Q, A, E, Outer> {
        let project = Arc::new(project);
        RpcHandler::new(self.operation, move |request| {
            let project = Arc::clone(&project);
            (self.invoke)(request).provide_some(move |outer| project(outer))
        })
    }

    /// Invoke the typed operation through its selected channel or handler.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `request` (`Q`): The typed request to process.
    ///
    /// # Returns
    /// Type: `Effect<A, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn call(&self, request: Q) -> Effect<A, E, R> {
        (self.invoke)(request)
    }
}
