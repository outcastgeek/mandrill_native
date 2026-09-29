use crate::{Effect, JsonValue, ParseLimits, PureCodec, SchemaIssue};
use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::Arc;

type IdempotencyKey<Q> = dyn Fn(&Q) -> Result<Arc<[u8]>, SchemaIssue> + Send + Sync;

/// Stable, application-owned identity. Renaming an operation is a protocol
/// migration; source locations and Rust type names are never wire identities.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(Arc<str>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationIdError {
    Empty,
    InvalidCharacter,
}

impl OperationId {
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
    pub const fn new(value: NonZeroU32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> NonZeroU32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcOperationKind {
    Unary,
    Stream,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcOperationSpec {
    pub id: OperationId,
    pub version: RpcVersion,
    pub kind: RpcOperationKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpcRegistryError {
    DuplicateOperation {
        id: OperationId,
        version: RpcVersion,
    },
    OperationKindConflict {
        id: OperationId,
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
    pub fn new() -> Self {
        Self {
            operations: BTreeMap::new(),
        }
    }

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
    pub fn kind(&self, id: &OperationId, version: RpcVersion) -> Option<RpcOperationKind> {
        self.operations.get(&(id.clone(), version)).copied()
    }

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
    fn default() -> Self {
        Self::new()
    }
}

/// One unary operation links request, success, and domain-error schemas.
/// The same description can serve in-process and encoded transports.
pub struct RpcOperation<Q, A, E> {
    spec: RpcOperationSpec,
    request: PureCodec<Q, JsonValue>,
    success: PureCodec<A, JsonValue>,
    domain_error: PureCodec<E, JsonValue>,
    limits: ParseLimits,
    idempotency_key: Option<Arc<IdempotencyKey<Q>>>,
}

impl<Q, A, E> Clone for RpcOperation<Q, A, E> {
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
    pub fn with_idempotency_key(
        mut self,
        key: impl Fn(&Q) -> Result<Arc<[u8]>, SchemaIssue> + Send + Sync + 'static,
    ) -> Self {
        self.idempotency_key = Some(Arc::new(key));
        self
    }

    #[must_use]
    pub fn spec(&self) -> &RpcOperationSpec {
        &self.spec
    }

    #[must_use]
    pub fn request_codec(&self) -> &PureCodec<Q, JsonValue> {
        &self.request
    }

    #[must_use]
    pub fn success_codec(&self) -> &PureCodec<A, JsonValue> {
        &self.success
    }

    #[must_use]
    pub fn error_codec(&self) -> &PureCodec<E, JsonValue> {
        &self.domain_error
    }

    #[must_use]
    pub const fn limits(&self) -> ParseLimits {
        self.limits
    }

    pub fn idempotency_key(&self, request: &Q) -> Result<Option<Arc<[u8]>>, SchemaIssue> {
        self.idempotency_key
            .as_ref()
            .map(|key| key(request))
            .transpose()
    }

    /// Change only the typed domain-error representation. Request/success
    /// codecs, stable identity, version and original idempotency extractor
    /// remain intact through protocol/middleware wrapping.
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
    pub fn operation(&self) -> &RpcOperation<Q, A, E> {
        &self.operation
    }

    pub fn call(&self, request: Q) -> Effect<A, E, R> {
        (self.invoke)(request)
    }
}
