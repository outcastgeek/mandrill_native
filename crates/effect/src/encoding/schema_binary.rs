//! Schema-derived binary payload encoding.

use crate::schema::{JsonValue, PureCodec, SchemaAst, SchemaIssue, SchemaResource};
use crate::{effect, effect_map_error, effect_try_sync};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;

mod bank;
mod internal;
pub use bank::{PreparedBinaryUnionCapacity, PreparedBinaryUnionPool};
use internal::{Cursor, DecodeBackend, Output};
pub use internal::{PreparedBinaryTraversalCapacity, PreparedBinaryTraversalPool};

/// Format 1 is independent of the frame envelope and application revision.
/// It begins with a format octet, a big-endian schema version and a length-
/// prefixed UTF-8 schema name. The remaining bytes follow the encoded AST:
/// fixed-width big-endian numbers, u64 length prefixes, positional object
/// presence octets and a u64 union alternative index in declaration order.
/// `Any` alone uses the tagged JSON-value subformat. This is this crate's
/// format; it makes no claim of upstream SchemaBinary wire compatibility.
pub const SCHEMA_BINARY_FORMAT_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit resource bounds for encoding schema binary.
pub struct BinaryLimits {
    /// The owned encoded bytes. Type: `NonZeroUsize`.
    pub bytes: NonZeroUsize,
    /// Maximum permitted nesting depth. Type: `NonZeroUsize`.
    pub depth: NonZeroUsize,
    /// Maximum permitted schema or parse nodes. Type: `NonZeroUsize`.
    pub nodes: NonZeroUsize,
    /// Maximum number of items in one collection. Type: `NonZeroUsize`.
    pub collection: NonZeroUsize,
    /// Includes schema redirects and failed union alternatives.
    pub steps: NonZeroUsize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Stable format identity and version used to interpret encoded binary data.
pub struct BinaryIdentity {
    name: Arc<str>,
    version: NonZeroU64,
}

impl BinaryIdentity {
    /// Construct a `BinaryIdentity` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    /// - `version` (`NonZeroU64`): Registered schema, event, or workflow compatibility version.
    ///
    /// # Returns
    /// Type: `Result<Self, BinaryError>`.
    pub fn new(name: impl Into<Arc<str>>, version: NonZeroU64) -> Result<Self, BinaryError> {
        let name = name.into();
        if name.is_empty() {
            return Err(BinaryError::EmptySchemaName);
        }
        Ok(Self { name, version })
    }

    #[must_use]
    /// Borrow the stable name used to identify this definition.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    /// # Returns
    /// Type: `NonZeroU64`.
    pub const fn version(&self) -> NonZeroU64 {
        self.version
    }
}

#[derive(Debug)]
/// A typed identity, size, or decoding failure for binary representation.
pub enum BinaryError {
    /// The required schema name is empty.
    EmptySchemaName,
    /// The format is not supported. Carries `u8`.
    UnsupportedFormat(u8),
    /// The schema name differs from the required or recorded value.
    SchemaNameMismatch,
    /// The encoded schema version differs from the expected version.
    SchemaVersionMismatch {
        /// Version required by the decoder. Type: `u64`.
        expected: u64,
        /// Version found in the payload. Type: `u64`.
        actual: u64,
    },
    /// The codec schema differs from the required or recorded value.
    CodecSchemaMismatch,
    /// The opaque is not supported.
    UnsupportedOpaque,
    /// The grapheme length is not supported.
    UnsupportedGraphemeLength,
    /// A schema reference names no available definition.
    UnresolvedReference,
    /// The tag failed validation. Carries `u8`.
    InvalidTag(u8),
    /// The union index failed validation. Carries `u64`.
    InvalidUnionIndex(u64),
    /// The utf8 failed validation.
    InvalidUtf8,
    /// The number failed validation.
    InvalidNumber,
    /// The key already exists or occurred twice. Carries `String`.
    DuplicateKey(String),
    /// The duplicate key retained in prepared text backing.
    PreparedDuplicateKey(crate::schema::PreparedDiagnosticText),
    /// The binary payload ends before the next value is complete.
    Truncated {
        /// Byte position of the incomplete value. Type: `usize`.
        offset: usize,
        /// Additional bytes required to complete it. Type: `usize`.
        needed: usize,
    },
    /// Decoded data is followed by unexpected bytes.
    TrailingBytes {
        /// Position where trailing bytes begin. Type: `usize`.
        offset: usize,
    },
    /// Computing length exceeded its supported range.
    LengthOverflow,
    /// The configured resource limit was exceeded. Retains the `SchemaResource` detail.
    LimitExceeded(SchemaResource),
    /// No union variant is available. Carries `Vec<Self>`.
    NoUnionVariant(Vec<Self>),
    /// No union alternative matched; failures retain their prepared array owner.
    PreparedNoUnionVariant(crate::schema::SharedPreparedJsonArray<Self>),
    /// Prepared backing admission or output failed.
    Memory(crate::memory::MemoryError),
    /// The value failed schema validation. Retains the `SchemaIssue` detail.
    Schema(SchemaIssue),
}

impl std::fmt::Display for BinaryError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for BinaryError {}
impl BinaryError {
    /// Borrow a duplicate key regardless of native or prepared storage.
    #[must_use]
    pub fn duplicate_key(&self) -> Option<&str> {
        if let Self::DuplicateKey(key) = self {
            Some(key)
        } else if let Self::PreparedDuplicateKey(key) = self {
            Some(key.as_str())
        } else {
            None
        }
    }
}

impl From<SchemaIssue> for BinaryError {
    ///
    /// # Parameters
    /// - `issue` (`SchemaIssue`): The issue supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    fn from(issue: SchemaIssue) -> Self {
        Self::Schema(issue)
    }
}

/// An owned single-value binary codec. The caller supplies every resource
/// limit; no process-global cache, executor or provider is consulted.
#[derive(Clone)]
pub struct SchemaBinary {
    ast: SchemaAst,
    identity: BinaryIdentity,
    limits: BinaryLimits,
    issues: Option<Arc<crate::schema::PreparedIssueMemory>>,
    traversal: Option<Arc<PreparedBinaryTraversalPool>>,
}

impl SchemaBinary {
    #[must_use]
    /// Construct a `SchemaBinary` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `ast` (`SchemaAst`): Representation tree used for schema inspection or export.
    /// - `identity` (`BinaryIdentity`): Stable identity used to deduplicate or find this record.
    /// - `limits` (`BinaryLimits`): Explicit resource bounds enforced while processing this input.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(ast: SchemaAst, identity: BinaryIdentity, limits: BinaryLimits) -> Self {
        Self {
            ast,
            identity,
            limits,
            issues: None,
            traversal: None,
        }
    }
    /// Select prepared diagnostic ownership for prepared binary operations.
    #[must_use]
    pub fn with_issue_memory(mut self, issues: Arc<crate::schema::PreparedIssueMemory>) -> Self {
        self.issues = Some(issues);
        self
    }
    /// Select fixed call-scoped traversal backing for prepared binary operations.
    #[must_use]
    pub fn with_traversal_pool(mut self, pool: Arc<PreparedBinaryTraversalPool>) -> Self {
        self.traversal = Some(pool);
        self
    }

    #[must_use]
    /// Borrow the stable binary format identity used by this codec.
    ///
    /// # Returns
    /// Type: `&BinaryIdentity`.
    pub fn identity(&self) -> &BinaryIdentity {
        &self.identity
    }

    #[must_use]
    /// # Returns
    /// Type: `BinaryLimits`.
    pub const fn limits(&self) -> BinaryLimits {
        self.limits
    }

    /// Encode the schema's JSON-facing representation. `Checked` metadata
    /// cannot run application validators; a typed codec must do that first.
    ///
    /// # Parameters
    /// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Vec<u8>, BinaryError>`.
    pub fn encode_json(&self, value: &JsonValue) -> Result<Vec<u8>, BinaryError> {
        let mut bytes = Vec::new();
        let mut output = Output::new(self.limits, &mut bytes);
        output.byte(SCHEMA_BINARY_FORMAT_VERSION)?;
        output.u64(self.identity.version.get())?;
        output.text(&self.identity.name)?;
        output.value(&self.ast, value, 1)?;
        drop(output);
        Ok(bytes)
    }

    /// Append one format-1 value to prepared backing, restoring its initial
    /// prefix on failure or unwind. The logical byte limit applies to the append.
    pub fn encode_json_into(
        &self,
        value: &JsonValue,
        buffer: &mut crate::memory::Buffer<'_>,
        errors: &crate::schema::PreparedJsonArrayPool<BinaryError>,
    ) -> Result<(), BinaryError> {
        let mut output = Output::prepared(
            self.limits,
            buffer,
            errors,
            self.issues.as_deref(),
            self.traversal.as_deref(),
        );
        output.byte(SCHEMA_BINARY_FORMAT_VERSION)?;
        output.u64(self.identity.version.get())?;
        output.text(&self.identity.name)?;
        output.value(&self.ast, value, 1)?;
        output.commit();
        Ok(())
    }

    /// Append a managed JSON subtree using the same format and rollback rules.
    /// Object traversal follows the managed document's retained input order.
    pub fn encode_prepared_into(
        &self,
        value: &crate::schema::SharedPreparedJsonValue,
        buffer: &mut crate::memory::Buffer<'_>,
        errors: &crate::schema::PreparedJsonArrayPool<BinaryError>,
    ) -> Result<(), BinaryError> {
        let mut output = Output::prepared(
            self.limits,
            buffer,
            errors,
            self.issues.as_deref(),
            self.traversal.as_deref(),
        );
        output.byte(SCHEMA_BINARY_FORMAT_VERSION)?;
        output.u64(self.identity.version.get())?;
        output.text(&self.identity.name)?;
        output.prepared_value(&self.ast, value.view(), 1)?;
        output.commit();
        Ok(())
    }

    /// Decode into independently owned JSON values. No view into `input` or
    /// a retained arena outlives the call.
    ///
    /// # Parameters
    /// - `input` (`&[u8]`): The input value or factory consumed by this operation.
    ///
    /// # Returns
    /// Type: `Result<JsonValue, BinaryError>`.
    pub fn decode_json(&self, input: &[u8]) -> Result<JsonValue, BinaryError> {
        let mut cursor = Cursor::new(input, self.limits)?;
        let version = cursor.byte()?;
        if version != SCHEMA_BINARY_FORMAT_VERSION {
            return Err(BinaryError::UnsupportedFormat(version));
        }
        let version = cursor.u64()?;
        if version != self.identity.version.get() {
            return Err(BinaryError::SchemaVersionMismatch {
                expected: self.identity.version.get(),
                actual: version,
            });
        }
        if cursor.text()?.as_bytes() != self.identity.name.as_bytes() {
            return Err(BinaryError::SchemaNameMismatch);
        }
        let value = cursor.value(&self.ast, 1)?;
        if cursor.offset != input.len() {
            return Err(BinaryError::TrailingBytes {
                offset: cursor.offset,
            });
        }
        Ok(value)
    }

    /// Decode into an admitted managed document and count its normalized JSON bytes.
    /// The input and traversal retain their existing binary limits; the selected
    /// parse policy independently bounds the managed representation. This path
    /// requires a selected prepared traversal pool.
    pub fn decode_prepared(
        &self,
        input: &[u8],
        output: &crate::schema::PreparedJsonParserPool,
        policy: crate::schema::ParsePolicy,
    ) -> Result<crate::schema::SharedPreparedJsonValue, BinaryError> {
        let mut owner = output
            .admit_document(policy)
            .map_err(internal::backing_error)?;
        let issues = output.issue_memory().or(self.issues.as_deref());
        let mut backend = DecodeBackend::Managed {
            owner: &mut owner,
            issues,
        };
        let mut cursor = Cursor::new(input, self.limits)?;
        let version = cursor.byte()?;
        if version != SCHEMA_BINARY_FORMAT_VERSION {
            return Err(BinaryError::UnsupportedFormat(version));
        }
        let version = cursor.u64()?;
        if version != self.identity.version.get() {
            return Err(BinaryError::SchemaVersionMismatch {
                expected: self.identity.version.get(),
                actual: version,
            });
        }
        if cursor.text()?.as_bytes() != self.identity.name.as_bytes() {
            return Err(BinaryError::SchemaNameMismatch);
        }
        let value = cursor.value_node(&self.ast, 1, &mut backend, self.traversal.as_deref())?;
        if cursor.offset != input.len() {
            return Err(BinaryError::TrailingBytes {
                offset: cursor.offset,
            });
        }
        let root = value.managed_root()?;
        owner.finish(root).map_err(internal::backing_error)
    }

    /// Encode a typed value with the declared binary identity and resource bounds.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    ///
    /// # Parameters
    /// - `codec` (`&PureCodec<A, JsonValue>`): The codec used to convert and validate values.
    /// - `value` (`&A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Vec<u8>, BinaryError>`.
    pub fn encode_typed<A: Send + 'static>(
        &self,
        codec: &PureCodec<A, JsonValue>,
        value: &A,
    ) -> Result<Vec<u8>, BinaryError> {
        self.require_codec(codec)?;
        let encoded = codec.encode_ref(value)?;
        self.encode_json(&encoded)
    }

    /// Check codec AST identity, decode bounded binary bytes, then validate through the typed JSON decoder; schema and format failures remain distinct BinaryError variants.
    pub fn decode_typed<A: Send + 'static>(
        &self,
        codec: &PureCodec<A, JsonValue>,
        input: &[u8],
    ) -> Result<A, BinaryError> {
        self.require_codec(codec)?;
        codec
            .decode_ref(&self.decode_json(input)?)
            .map_err(Into::into)
    }

    /// Encode through a managed codec after reserving the actual traversal cell.
    /// The supplied context is forwarded to the genuine codec encoder once.
    pub fn encode_prepared_typed_into<A: Send + 'static>(
        &self,
        codec: &PureCodec<A, crate::schema::SharedPreparedJsonValue>,
        value: &A,
        context: Option<&crate::schema::ParseContext>,
        buffer: &mut crate::memory::Buffer<'_>,
        errors: &crate::schema::PreparedJsonArrayPool<BinaryError>,
    ) -> Result<(), BinaryError> {
        self.require_codec(codec)?;
        let reservation = self
            .traversal
            .as_deref()
            .ok_or(BinaryError::Memory(
                crate::memory::MemoryError::UnsupportedRunContract,
            ))?
            .reserve()
            .map_err(BinaryError::Memory)?;
        // The reservation contains no borrowed frames. The stack is initialized
        // only after `encoded` exists, so it retires before that owner on unwind.
        let encoded = codec.encode_in_ref(value, context)?;
        let mut output = Output::prepared(
            self.limits,
            buffer,
            errors,
            self.issues.as_deref(),
            self.traversal.as_deref(),
        );
        output.byte(SCHEMA_BINARY_FORMAT_VERSION)?;
        output.u64(self.identity.version.get())?;
        output.text(&self.identity.name)?;
        output.prepared_value_reserved(&self.ast, encoded.view(), 1, reservation)?;
        output.commit();
        Ok(())
    }

    /// Decode an admitted managed document through the genuine typed decoder.
    /// The parse policy and the optional codec context remain independent.
    pub fn decode_prepared_typed<A: Send + 'static>(
        &self,
        codec: &PureCodec<A, crate::schema::SharedPreparedJsonValue>,
        input: &[u8],
        output: &crate::schema::PreparedJsonParserPool,
        policy: crate::schema::ParsePolicy,
        context: Option<&crate::schema::ParseContext>,
    ) -> Result<A, BinaryError> {
        self.require_codec(codec)?;
        let encoded = self.decode_prepared(input, output, policy)?;
        codec.decode_in_ref(&encoded, context).map_err(Into::into)
    }

    /// Adapt this same binary format to owned effectful directions with separate `D` and `E`.
    ///
    /// The codec must share this binary codec's AST identity or construction returns
    /// CodecSchemaMismatch. Decode parses bytes then runs the schema in `D`; encode
    /// runs the schema in `E` then emits binary bytes under existing limits. Schema
    /// errors map to BinaryError; defects, interruption and cleanup remain unchanged.
    /// Owned validation uses the schema's independent validation direction. Format 1
    /// retains its lossless i64/u64/f64 restriction on numbers; this adapter adds no
    /// new wire format or physical persistence guarantee.
    pub fn effect_codec<A: Send + 'static, D: Send + Sync + 'static, E: Send + Sync + 'static>(
        &self,
        codec: crate::Codec<A, JsonValue, D, E>,
    ) -> Result<crate::DomainCodec<A, Vec<u8>, BinaryError, D, E>, BinaryError> {
        if self.ast.identity() != codec.ast().identity() {
            return Err(BinaryError::CodecSchemaMismatch);
        }
        let binary = self.clone();
        let encoder = self.clone();
        let decode = codec.clone();
        let validate = codec.clone();
        Ok(crate::DomainCodec::new(
            codec.ast().clone(),
            move |bytes: Vec<u8>| {
                let binary = binary.clone();
                let decode = decode.clone();
                let parsed = effect_try_sync! {
                    once;
                    body: move || binary.decode_json(&bytes),
                    on_error: std::convert::identity,
                };
                effect! {
                    let decode = decode.clone();
                    let value = parsed.clone()?;
                    effect_map_error! {
                        program: decode.decode_once(value),
                        map: BinaryError::from,
                    }?
                }
            },
            move |value| {
                let binary = encoder.clone();
                let encoded = effect_map_error! {
                    program: codec.encode_once(value),
                    map: BinaryError::from,
                };
                effect! {
                    let binary = binary.clone();
                    let value = encoded.clone()?;
                    effect_try_sync! {
                        once;
                        body: move || binary.encode_json(&value),
                        on_error: std::convert::identity,
                    }?
                }
            },
            move |value| effect_map_error! { program: validate.validate_once(value), map: BinaryError::from, },
        ))
    }

    ///
    /// # Type parameters
    /// - `A`: Bounds: `Send + 'static`.
    ///
    /// # Parameters
    /// - `codec` (`&PureCodec<A, Encoded>`): The codec supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), BinaryError>`.
    fn require_codec<A: Send + 'static, Encoded: Send + 'static>(
        &self,
        codec: &PureCodec<A, Encoded>,
    ) -> Result<(), BinaryError> {
        if self.ast.identity() != codec.ast().identity() {
            return Err(BinaryError::CodecSchemaMismatch);
        }
        Ok(())
    }
}

/// Actual storage selected for prepared binary directions.
pub struct PreparedBinaryCodecResources {
    /// Physical output byte cells.
    pub bytes: Arc<super::PreparedBytePool>,
    /// Actual union error arrays and retained frames.
    pub errors: Arc<crate::PreparedJsonArrayPool<BinaryError>>,
    /// Finite union builder bank controls.
    pub banks: Arc<PreparedBinaryUnionPool>,
    /// Managed decode document and counting writer pool.
    pub output: Arc<crate::PreparedJsonParserPool>,
    /// Explicit managed decode policy.
    pub policy: crate::ParsePolicy,
}
/// Owned prepared binary directions over one managed-value codec.
/// F selects the expected root error family before graph construction.
pub struct PreparedBinaryCodec<
    C,
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    F = fn(BinaryError) -> BinaryError,
> {
    binary: SchemaBinary,
    codec: Arc<C>,
    resources: PreparedBinaryCodecResources,
    map: F,
    marker: std::marker::PhantomData<fn() -> A>,
    environments: std::marker::PhantomData<fn() -> (D, E)>,
}
impl<C, A, D, E> PreparedBinaryCodec<C, A, D, E>
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    C: crate::PreparedCodec<A, crate::SharedPreparedJsonValue, D, E, Error = BinaryError>,
{
    /// Select physical storage and preserve the BinaryError root family.
    pub fn new(
        binary: SchemaBinary,
        codec: Arc<C>,
        bytes: Arc<super::PreparedBytePool>,
        errors: Arc<crate::PreparedJsonArrayPool<BinaryError>>,
        banks: Arc<PreparedBinaryUnionPool>,
        output: Arc<crate::PreparedJsonParserPool>,
        policy: crate::ParsePolicy,
    ) -> Result<Self, BinaryError> {
        Self::new_mapped(
            binary,
            codec,
            PreparedBinaryCodecResources {
                bytes,
                errors,
                banks,
                output,
                policy,
            },
            std::convert::identity::<BinaryError>,
        )
    }
}
impl<C, A, D, E, F> PreparedBinaryCodec<C, A, D, E, F>
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    /// Select one root error family without converting an existing graph.
    /// AST mismatch is returned directly before any description is built.
    pub fn new_mapped<X>(
        binary: SchemaBinary,
        codec: Arc<C>,
        resources: PreparedBinaryCodecResources,
        map: F,
    ) -> Result<Self, BinaryError>
    where
        X: Send + 'static,
        F: Fn(BinaryError) -> X + Send + Sync + 'static,
        C: crate::PreparedCodec<A, crate::SharedPreparedJsonValue, D, E, Error = X>,
    {
        if binary.ast.identity() != codec.ast().identity() {
            return Err(BinaryError::CodecSchemaMismatch);
        }
        Ok(Self {
            binary,
            codec,
            resources,
            map,
            marker: std::marker::PhantomData,
            environments: std::marker::PhantomData,
        })
    }
}
impl<C, A, D, E, F, X> crate::PreparedCodec<A, super::PreparedBytes, D, E>
    for PreparedBinaryCodec<C, A, D, E, F>
where
    A: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    X: Send + 'static,
    F: Fn(BinaryError) -> X + Send + Sync + 'static,
    C: crate::PreparedCodec<A, crate::SharedPreparedJsonValue, D, E, Error = X>,
{
    type Error = X;
    fn ast(&self) -> &SchemaAst {
        &self.binary.ast
    }
    fn map_schema_issue(&self, issue: SchemaIssue) -> X {
        (self.map)(BinaryError::Schema(issue))
    }
    fn has_validator(&self) -> bool {
        self.codec.has_validator()
    }
    fn has_default_validator(&self) -> bool {
        self.codec.has_default_validator()
    }
    fn validate_default_ref(
        &self,
        value: &A,
        context: Option<&crate::ParseContext>,
    ) -> Result<(), X> {
        self.codec.validate_default_ref(value, context)
    }
    fn decode(
        self: Arc<Self>,
        bytes: super::PreparedBytes,
        context: Option<crate::ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<Value = A, Error = X, Env = D> {
        let codec = Arc::clone(&self.codec);
        crate::effect_prepare! { description; program: effect! {
            let value = effect_try_sync! { once; body: move || self.binary.decode_prepared(bytes.as_slice(),&self.resources.output,self.resources.policy).map_err(&self.map), }?;
            let result = <C as crate::PreparedCodec<A,crate::SharedPreparedJsonValue,D,E>>::decode(codec,value,context)?;
            result
        }, }
    }
    fn encode(
        self: Arc<Self>,
        value: A,
        context: Option<crate::ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<
        Value = super::PreparedBytes,
        Error = X,
        Env = E,
    > {
        let prepare = Arc::clone(&self);
        let codec = Arc::clone(&self.codec);
        crate::effect_prepare! { description; program: effect! {
            let (mut bytes,traversal,bank) = effect_try_sync! { once; body: move || {
                (|| -> Result<_,BinaryError> {
                    let bytes=prepare.resources.bytes.checkout().map_err(BinaryError::Memory)?;
                    let traversal=prepare.binary.traversal.as_deref().ok_or(BinaryError::Memory(crate::memory::MemoryError::UnsupportedRunContract))?.reserve().map_err(BinaryError::Memory)?;
                    let bank=prepare.resources.banks.admit(&prepare.resources.errors).map_err(BinaryError::Memory)?;
                    Ok((bytes,traversal,bank))
                })().map_err(&prepare.map)
            }, }?;
            let encoded = <C as crate::PreparedCodec<A,crate::SharedPreparedJsonValue,D,E>>::encode(codec,value,context)?;
            let bytes = effect_try_sync! { once; body: move || {
                (|| -> Result<_,BinaryError> {
                    let buffer=bytes.buffer_mut().ok_or(BinaryError::Memory(crate::memory::MemoryError::Closed))?;
                    let mut output=Output::prepared(self.binary.limits,buffer,&self.resources.errors,self.binary.issues.as_deref(),self.binary.traversal.as_deref()).with_bank(&bank);
                    output.byte(SCHEMA_BINARY_FORMAT_VERSION)?;
                    output.u64(self.binary.identity.version.get())?;
                    output.text(&self.binary.identity.name)?;
                    output.prepared_value_reserved(&self.binary.ast,encoded.view(),1,traversal)?;
                    output.commit();
                    drop(output);
                    Ok(bytes)
                })().map_err(&self.map)
            }, }?;
            bytes
        }, }
    }
    fn validate(
        self: Arc<Self>,
        value: A,
        context: Option<crate::ParseContext>,
    ) -> impl crate::__private::prepared_graph::Description<Value = A, Error = X, Env = E> {
        <C as crate::PreparedCodec<A, crate::SharedPreparedJsonValue, D, E>>::validate(
            Arc::clone(&self.codec),
            value,
            context,
        )
    }
}
