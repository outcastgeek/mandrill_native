//! Schema-derived binary payload encoding.

use crate::schema::{JsonValue, PureCodec, SchemaAst, SchemaIssue, SchemaResource};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;

mod internal;
use internal::{Cursor, Output};

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
        }
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
        let mut output = Output::new(self.limits);
        output.byte(SCHEMA_BINARY_FORMAT_VERSION)?;
        output.u64(self.identity.version.get())?;
        output.text(&self.identity.name)?;
        output.value(&self.ast, value, 1)?;
        Ok(output.bytes)
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
                crate::Effect::try_sync_once(
                    move || binary.decode_json(&bytes),
                    std::convert::identity,
                )
                .flat_map(move |value| decode.decode_once(value).map_error(BinaryError::from))
            },
            move |value| {
                let binary = encoder.clone();
                codec
                    .encode_once(value)
                    .map_error(BinaryError::from)
                    .flat_map(move |value| {
                        let binary = binary.clone();
                        crate::Effect::try_sync_once(
                            move || binary.encode_json(&value),
                            std::convert::identity,
                        )
                    })
            },
            move |value| validate.validate_once(value).map_error(BinaryError::from),
        ))
    }

    ///
    /// # Type parameters
    /// - `A`: Bounds: `Send + 'static`.
    ///
    /// # Parameters
    /// - `codec` (`&PureCodec<A, JsonValue>`): The codec supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), BinaryError>`.
    fn require_codec<A: Send + 'static>(
        &self,
        codec: &PureCodec<A, JsonValue>,
    ) -> Result<(), BinaryError> {
        if self.ast.identity() != codec.ast().identity() {
            return Err(BinaryError::CodecSchemaMismatch);
        }
        Ok(())
    }
}
