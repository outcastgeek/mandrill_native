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
pub struct BinaryLimits {
    pub bytes: NonZeroUsize,
    pub depth: NonZeroUsize,
    pub nodes: NonZeroUsize,
    pub collection: NonZeroUsize,
    /// Includes schema redirects and failed union alternatives.
    pub steps: NonZeroUsize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryIdentity {
    name: Arc<str>,
    version: NonZeroU64,
}

impl BinaryIdentity {
    pub fn new(name: impl Into<Arc<str>>, version: NonZeroU64) -> Result<Self, BinaryError> {
        let name = name.into();
        if name.is_empty() {
            return Err(BinaryError::EmptySchemaName);
        }
        Ok(Self { name, version })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn version(&self) -> NonZeroU64 {
        self.version
    }
}

#[derive(Debug)]
pub enum BinaryError {
    EmptySchemaName,
    UnsupportedFormat(u8),
    SchemaNameMismatch,
    SchemaVersionMismatch { expected: u64, actual: u64 },
    CodecSchemaMismatch,
    UnsupportedOpaque,
    UnsupportedGraphemeLength,
    UnresolvedReference,
    InvalidTag(u8),
    InvalidUnionIndex(u64),
    InvalidUtf8,
    InvalidNumber,
    DuplicateKey(String),
    Truncated { offset: usize, needed: usize },
    TrailingBytes { offset: usize },
    LengthOverflow,
    LimitExceeded(SchemaResource),
    NoUnionVariant(Vec<Self>),
    Schema(SchemaIssue),
}

impl std::fmt::Display for BinaryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for BinaryError {}

impl From<SchemaIssue> for BinaryError {
    fn from(issue: SchemaIssue) -> Self {
        Self::Schema(issue)
    }
}

/// An owned single-value binary codec. The caller supplies every resource
/// limit; no process-global cache, executor or provider is consulted.
pub struct SchemaBinary {
    ast: SchemaAst,
    identity: BinaryIdentity,
    limits: BinaryLimits,
}

impl SchemaBinary {
    #[must_use]
    pub fn new(ast: SchemaAst, identity: BinaryIdentity, limits: BinaryLimits) -> Self {
        Self {
            ast,
            identity,
            limits,
        }
    }

    #[must_use]
    pub fn identity(&self) -> &BinaryIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn limits(&self) -> BinaryLimits {
        self.limits
    }

    /// Encode the schema's JSON-facing representation. `Checked` metadata
    /// cannot run application validators; a typed codec must do that first.
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

    pub fn encode_typed<A: Send + 'static>(
        &self,
        codec: &PureCodec<A, JsonValue>,
        value: &A,
    ) -> Result<Vec<u8>, BinaryError> {
        self.require_codec(codec)?;
        let encoded = codec.encode_ref(value)?;
        self.encode_json(&encoded)
    }

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
