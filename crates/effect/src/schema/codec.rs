//! Pure and effectful bidirectional codecs.

use super::{
    IssueSegment, JsonValue, ParsePolicy, Schema, SchemaAst, SchemaIssue, SchemaIssueKind,
    SchemaReference, SchemaShape,
};
use crate::Effect;
use std::sync::{Arc, Mutex, OnceLock};

/// Codec callbacks are shared trait objects: Rust dispatches each callback
/// through its local vtable, while `A`, `Encoded`, and the environments stay
/// statically typed. Only encoded values and schema metadata cross boundaries.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `Encoded`: Encoded representation type.
type Decoder<A, Encoded> = Arc<dyn Fn(&Encoded) -> Result<A, SchemaIssue> + Send + Sync>;
/// Type alias for encoder in schema codec.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `Encoded`: Encoded representation type.
type Encoder<A, Encoded> = Arc<dyn Fn(&A) -> Result<Encoded, SchemaIssue> + Send + Sync>;
/// Type alias for type validator in schema codec.
///
/// # Type parameters
/// - `A`: Application value type.
type TypeValidator<A> = Arc<dyn Fn(&A) -> Result<(), SchemaIssue> + Send + Sync>;
/// Type alias for effect decoder in schema codec.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `Encoded`: Encoded representation type.
/// - `Env`: Environment type supplying required capabilities.
type EffectDecoder<A, Encoded, Env> =
    Arc<dyn Fn(Encoded) -> Effect<A, SchemaIssue, Env> + Send + Sync>;
/// Type alias for effect encoder in schema codec.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `Encoded`: Encoded representation type.
/// - `Env`: Environment type supplying required capabilities.
type EffectEncoder<A, Encoded, Env> =
    Arc<dyn Fn(A) -> Effect<Encoded, SchemaIssue, Env> + Send + Sync>;
/// Type alias for once operation in schema codec.
///
/// # Type parameters
/// - `Input`: Input value type.
/// - `Output`: Output value type.
type OnceOperation<Input, Output> = dyn Fn(&Input) -> Result<Output, SchemaIssue> + Send + Sync;

/// Pure bidirectional conversion with independently owned results.
/// The callbacks inspect borrowed inputs; no payload Clone is implied.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `Encoded`: Encoded representation type.
pub struct PureCodec<A, Encoded> {
    ast: SchemaAst,
    decode: Decoder<A, Encoded>,
    encode: Encoder<A, Encoded>,
    validate_type: Option<TypeValidator<A>>,
}

impl<A, Encoded> Clone for PureCodec<A, Encoded> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            ast: self.ast.clone(),
            decode: Arc::clone(&self.decode),
            encode: Arc::clone(&self.encode),
            validate_type: self.validate_type.as_ref().map(Arc::clone),
        }
    }
}

impl<A: Send + 'static, Encoded: Send + 'static> PureCodec<A, Encoded> {
    #[must_use]
    /// Construct a `PureCodec` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `decode` (`impl Fn(&Encoded) -> Result<A, SchemaIssue> + Send + Sync + 'static`): Callback that validates and reconstructs a typed value.
    /// - `encode` (`impl Fn(&A) -> Result<Encoded, SchemaIssue> + Send + Sync + 'static`): The encode supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        decode: impl Fn(&Encoded) -> Result<A, SchemaIssue> + Send + Sync + 'static,
        encode: impl Fn(&A) -> Result<Encoded, SchemaIssue> + Send + Sync + 'static,
    ) -> Self {
        Self::with_ast(SchemaAst::opaque(), decode, encode)
    }

    /// Construct a codec with an explicit encoded-side description. The
    /// caller must keep that description consistent with both callbacks.
    ///
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `ast` (`SchemaAst`): Representation tree used for schema inspection or export.
    /// - `decode` (`impl Fn(&Encoded) -> Result<A, SchemaIssue> + Send + Sync + 'static`): Callback that validates and reconstructs a typed value.
    /// - `encode` (`impl Fn(&A) -> Result<Encoded, SchemaIssue> + Send + Sync + 'static`): The encode supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn with_ast(
        ast: SchemaAst,
        decode: impl Fn(&Encoded) -> Result<A, SchemaIssue> + Send + Sync + 'static,
        encode: impl Fn(&A) -> Result<Encoded, SchemaIssue> + Send + Sync + 'static,
    ) -> Self {
        Self {
            ast,
            decode: Arc::new(decode),
            encode: Arc::new(encode),
            validate_type: None,
        }
    }

    /// Provide a direct type-side check when constructing an encoded value
    /// solely for validation would be unnecessary. Encoding remains checked
    /// independently; this callback does not bypass the encoder.
    #[must_use]
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `validate` (`impl Fn(&A) -> Result<(), SchemaIssue> + Send + Sync + 'static`): The validate supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn with_type_validator(
        mut self,
        validate: impl Fn(&A) -> Result<(), SchemaIssue> + Send + Sync + 'static,
    ) -> Self {
        self.validate_type = Some(Arc::new(validate));
        self
    }

    /// Check application-side constraints separately from decoding. A custom
    /// validator does not assert that encoding succeeds; callers storing a
    /// value still use the checked encoder. Without a custom validator this
    /// method validates through encoding and discards that encoded value.
    ///
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`&A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<(), SchemaIssue>`.
    pub fn validate_ref(&self, value: &A) -> Result<(), SchemaIssue> {
        match &self.validate_type {
            Some(validate) => validate(value),
            None => self.encode_ref(value).map(|_| ()),
        }
    }

    /// Decode from a borrowed representation without taking its ownership.
    ///
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `encoded` (`&Encoded`): The encoded supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<A, SchemaIssue>`.
    pub fn decode_ref(&self, encoded: &Encoded) -> Result<A, SchemaIssue> {
        (self.decode)(encoded)
    }

    /// Encode a borrowed application value under this codec’s rules.
    ///
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`&A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Encoded, SchemaIssue>`.
    pub fn encode_ref(&self, value: &A) -> Result<Encoded, SchemaIssue> {
        (self.encode)(value)
    }

    #[must_use]
    /// Borrow the representation tree used for schema inspection or export.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `&SchemaAst`.
    pub fn ast(&self) -> &SchemaAst {
        &self.ast
    }

    /// Lift pure directions into independent Effect environments.
    #[must_use]
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Capabilities required while decoding. Additional bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Capabilities required while encoding. Additional bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Codec<A, Encoded, DecodeEnv, EncodeEnv>`.
    pub fn into_effect<DecodeEnv, EncodeEnv>(&self) -> Codec<A, Encoded, DecodeEnv, EncodeEnv>
    where
        DecodeEnv: Send + Sync + 'static,
        EncodeEnv: Send + Sync + 'static,
    {
        let decode = Arc::clone(&self.decode);
        let encode = Arc::clone(&self.encode);
        Codec::with_ast(
            self.ast.clone(),
            move |encoded| evaluate_once(encoded, Arc::clone(&decode)),
            move |value| evaluate_once(value, Arc::clone(&encode)),
        )
    }
}

impl<A: Send + 'static> PureCodec<A, JsonValue> {
    /// Parse JSON text under the declared limits, then decode its typed value.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `text` (`&str`): Input text parsed or emitted by this operation.
    /// - `policy` (`ParsePolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Result<A, SchemaIssue>`.
    pub fn parse_str(&self, text: &str, policy: ParsePolicy) -> Result<A, SchemaIssue> {
        self.ast.prepare(policy.limits)?;
        let encoded = super::internal::parser::parse_json(text, policy)?;
        self.decode_ref(&encoded)
    }

    /// Tie a recursive codec without an owning reference cycle. The body sees
    /// a typed self-reference, while AST references remain weak and exportable.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    /// - `shape` (`impl FnOnce(SchemaReference) -> SchemaShape`): Encoded-side structure required by this schema.
    /// - `build` (`impl FnOnce(Self) -> Result<Self, SchemaIssue>`): The build supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, SchemaIssue>`.
    pub fn recursive(
        name: impl Into<Arc<str>>,
        shape: impl FnOnce(SchemaReference) -> SchemaShape,
        build: impl FnOnce(Self) -> Result<Self, SchemaIssue>,
    ) -> Result<Self, SchemaIssue> {
        let ast = SchemaAst::recursive(name, shape)?;
        let cell = Arc::new(OnceLock::<Self>::new());
        let decode_cell = Arc::downgrade(&cell);
        let encode_cell = Arc::downgrade(&cell);
        let reference = Self::with_ast(
            ast.clone(),
            move |encoded| {
                decode_cell
                    .upgrade()
                    .and_then(|cell| cell.get().map(|codec| codec.decode_ref(encoded)))
                    .unwrap_or_else(|| Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)))
            },
            move |value| {
                encode_cell
                    .upgrade()
                    .and_then(|cell| cell.get().map(|codec| codec.encode_ref(value)))
                    .unwrap_or_else(|| Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)))
            },
        );
        let body = build(reference)?;
        cell.set(body)
            .map_err(|_| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let decode = Arc::clone(&cell);
        Ok(Self::with_ast(
            ast,
            move |encoded| {
                decode
                    .get()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?
                    .decode_ref(encoded)
            },
            move |value| {
                cell.get()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?
                    .encode_ref(value)
            },
        ))
    }

    /// A caller-named refinement validates both directions. A failed encode
    /// cannot silently emit a value the decoder would reject.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    /// - `predicate` (`impl Fn(&A) -> bool + Send + Sync + 'static`): The condition used to select or validate values.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn refine(
        self,
        name: impl Into<Arc<str>>,
        predicate: impl Fn(&A) -> bool + Send + Sync + 'static,
    ) -> Self {
        let name = name.into();
        let ast = self.ast().derive(SchemaShape::Checked {
            source: self.ast().clone(),
            name: Arc::clone(&name),
        });
        let decode = self.clone();
        let predicate = Arc::new(predicate);
        let decode_predicate = Arc::clone(&predicate);
        let decode_name = Arc::clone(&name);
        Self::with_ast(
            ast,
            move |encoded| {
                let value = decode.decode_ref(encoded)?;
                if decode_predicate(&value) {
                    Ok(value)
                } else {
                    Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                        code: Arc::clone(&decode_name),
                    }))
                }
            },
            move |value| {
                if predicate(value) {
                    self.encode_ref(value)
                } else {
                    Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                        code: Arc::clone(&name),
                    }))
                }
            },
        )
    }

    /// Map a codec to a different application type while preserving its encoded representation.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `decode` (`impl Fn(A) -> Result<B, SchemaIssue> + Send + Sync + 'static`): Callback that validates and reconstructs a typed value.
    /// - `encode` (`impl Fn(&B) -> Result<A, SchemaIssue> + Send + Sync + 'static`): The encode supplied to this operation.
    ///
    /// # Returns
    /// Type: `PureCodec<B, JsonValue>`.
    pub fn transform<B: Send + 'static>(
        self,
        decode: impl Fn(A) -> Result<B, SchemaIssue> + Send + Sync + 'static,
        encode: impl Fn(&B) -> Result<A, SchemaIssue> + Send + Sync + 'static,
    ) -> PureCodec<B, JsonValue> {
        let ast = self
            .ast()
            .derive(SchemaShape::EncodedView(self.ast().clone()));
        let input = self.clone();
        PureCodec::with_ast(
            ast,
            move |encoded| decode(input.decode_ref(encoded)?),
            move |value| self.encode_ref(&encode(value)?),
        )
    }

    /// Preserve the existing decoder surface. An encoder is already present in
    /// this PureCodec; adapting an old one-way Schema in reverse is not inferred.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `issue` (`impl Fn(SchemaIssue) -> E + Send + Sync + 'static`): The issue supplied to this operation.
    ///
    /// # Returns
    /// Type: `Schema<A, E>`.
    pub fn to_schema<E: Send + 'static>(
        &self,
        issue: impl Fn(SchemaIssue) -> E + Send + Sync + 'static,
    ) -> Schema<A, E> {
        let codec = self.clone();
        Schema::from_decoder(move |encoded| codec.decode_ref(encoded).map_err(&issue))
    }
}

///
/// # Type parameters
/// - `Input`: Bounds: `Send + 'static`.
/// - `Output`: Bounds: `Send + 'static`.
/// - `Env`: Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `input` (`Input`): Owned input decoded or consumed by this operation.
/// - `operation` (`Arc<OnceOperation<Input, Output>>`): Typed operation definition or callback selected for execution.
///
/// # Returns
/// Type: `Effect<Output, SchemaIssue, Env>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn evaluate_once<Input: Send + 'static, Output: Send + 'static, Env: Send + Sync + 'static>(
    input: Input,
    operation: Arc<OnceOperation<Input, Output>>,
) -> Effect<Output, SchemaIssue, Env> {
    let input = Mutex::new(Some(input));
    Effect::try_sync(
        move || {
            let value = input
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InputConsumed))?;
            operation(&value)
        },
        std::convert::identity,
    )
}

/// Effectful codec with independent decode and encode environments.
/// Directions own their inputs; effectful transformations may suspend safely.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `Encoded`: Encoded representation type. Bound or type: `Send + 'static`.
/// - `DecodeEnv`: Decoding environment type. Bound or type: `Send + Sync + 'static`.
/// - `EncodeEnv`: Encoding environment type. Bound or type: `Send + Sync + 'static`.
pub struct Codec<
    A: Send + 'static,
    Encoded: Send + 'static,
    DecodeEnv: Send + Sync + 'static,
    EncodeEnv: Send + Sync + 'static,
> {
    ast: SchemaAst,
    decode: EffectDecoder<A, Encoded, DecodeEnv>,
    encode: EffectEncoder<A, Encoded, EncodeEnv>,
}

impl<
    A: Send + 'static,
    Encoded: Send + 'static,
    DecodeEnv: Send + Sync + 'static,
    EncodeEnv: Send + Sync + 'static,
> Clone for Codec<A, Encoded, DecodeEnv, EncodeEnv>
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            ast: self.ast.clone(),
            decode: Arc::clone(&self.decode),
            encode: Arc::clone(&self.encode),
        }
    }
}

impl<
    A: Send + 'static,
    Encoded: Send + 'static,
    DecodeEnv: Send + Sync + 'static,
    EncodeEnv: Send + Sync + 'static,
> Codec<A, Encoded, DecodeEnv, EncodeEnv>
{
    #[must_use]
    /// Construct a `Codec` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `decode` (`impl Fn(Encoded) -> Effect<A, SchemaIssue, DecodeEnv> + Send + Sync + 'static`): Callback that validates and reconstructs a typed value.
    /// - `encode` (`impl Fn(A) -> Effect<Encoded, SchemaIssue, EncodeEnv> + Send + Sync + 'static`): The encode supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        decode: impl Fn(Encoded) -> Effect<A, SchemaIssue, DecodeEnv> + Send + Sync + 'static,
        encode: impl Fn(A) -> Effect<Encoded, SchemaIssue, EncodeEnv> + Send + Sync + 'static,
    ) -> Self {
        Self::with_ast(SchemaAst::opaque(), decode, encode)
    }

    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `ast` (`SchemaAst`): Representation tree used for schema inspection or export.
    /// - `decode` (`impl Fn(Encoded) -> Effect<A, SchemaIssue, DecodeEnv> + Send + Sync + 'static`): Callback that validates and reconstructs a typed value.
    /// - `encode` (`impl Fn(A) -> Effect<Encoded, SchemaIssue, EncodeEnv> + Send + Sync + 'static`): The encode supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn with_ast(
        ast: SchemaAst,
        decode: impl Fn(Encoded) -> Effect<A, SchemaIssue, DecodeEnv> + Send + Sync + 'static,
        encode: impl Fn(A) -> Effect<Encoded, SchemaIssue, EncodeEnv> + Send + Sync + 'static,
    ) -> Self {
        Self {
            ast,
            decode: Arc::new(decode),
            encode: Arc::new(encode),
        }
    }

    /// Each evaluation receives a fresh owned input without requiring Clone.
    ///
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `input` (`impl Fn() -> Encoded + Send + Sync + 'static`): The input value or factory consumed by this operation.
    ///
    /// # Returns
    /// Type: `Effect<A, SchemaIssue, DecodeEnv>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn decode_with(
        &self,
        input: impl Fn() -> Encoded + Send + Sync + 'static,
    ) -> Effect<A, SchemaIssue, DecodeEnv> {
        let decode = Arc::clone(&self.decode);
        Effect::defer(move || decode(input()))
    }

    /// Supply an effectful encoder while retaining this schema’s decode contract.
    ///
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `input` (`impl Fn() -> A + Send + Sync + 'static`): The input value or factory consumed by this operation.
    ///
    /// # Returns
    /// Type: `Effect<Encoded, SchemaIssue, EncodeEnv>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn encode_with(
        &self,
        input: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<Encoded, SchemaIssue, EncodeEnv> {
        let encode = Arc::clone(&self.encode);
        Effect::defer(move || encode(input()))
    }

    #[must_use]
    /// Borrow the representation tree used for schema inspection or export.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `&SchemaAst`.
    pub fn ast(&self) -> &SchemaAst {
        &self.ast
    }

    /// Adapt only decoding to a larger environment. The original encode
    /// environment remains unchanged and visible in the returned type.
    #[must_use]
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `Outer`: Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `project` (`impl Fn(&Outer) -> DecodeEnv + Send + Sync + 'static`): The project supplied to this operation.
    ///
    /// # Returns
    /// Type: `Codec<A, Encoded, Outer, EncodeEnv>`.
    pub fn provide_decode<Outer: Send + Sync + 'static>(
        &self,
        project: impl Fn(&Outer) -> DecodeEnv + Send + Sync + 'static,
    ) -> Codec<A, Encoded, Outer, EncodeEnv> {
        let decode = Arc::clone(&self.decode);
        let encode = Arc::clone(&self.encode);
        let project = Arc::new(project);
        Codec::with_ast(
            self.ast.clone(),
            move |input| {
                let project = Arc::clone(&project);
                decode(input).provide_some(move |outer| project(outer))
            },
            move |input| encode(input),
        )
    }

    /// Adapt only encoding without changing decoding's requirements.
    #[must_use]
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `Outer`: Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `project` (`impl Fn(&Outer) -> EncodeEnv + Send + Sync + 'static`): The project supplied to this operation.
    ///
    /// # Returns
    /// Type: `Codec<A, Encoded, DecodeEnv, Outer>`.
    pub fn provide_encode<Outer: Send + Sync + 'static>(
        &self,
        project: impl Fn(&Outer) -> EncodeEnv + Send + Sync + 'static,
    ) -> Codec<A, Encoded, DecodeEnv, Outer> {
        let decode = Arc::clone(&self.decode);
        let encode = Arc::clone(&self.encode);
        let project = Arc::new(project);
        Codec::with_ast(
            self.ast.clone(),
            move |input| decode(input),
            move |input| {
                let project = Arc::clone(&project);
                encode(input).provide_some(move |outer| project(outer))
            },
        )
    }

    /// Compose effectful conversion in both directions. Each callback owns
    /// its value, so suspension does not borrow a temporary or require Clone.
    /// Decode and encode keep their independently selected environments.
    #[must_use]
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `after_decode` (`impl Fn(A) -> Effect<B, SchemaIssue, DecodeEnv> + Send + Sync + 'static`): The after decode supplied to this operation.
    /// - `before_encode` (`impl Fn(B) -> Effect<A, SchemaIssue, EncodeEnv> + Send + Sync + 'static`): The before encode supplied to this operation.
    ///
    /// # Returns
    /// Type: `Codec<B, Encoded, DecodeEnv, EncodeEnv>`.
    pub fn transform_effectful<B: Send + 'static>(
        self,
        after_decode: impl Fn(A) -> Effect<B, SchemaIssue, DecodeEnv> + Send + Sync + 'static,
        before_encode: impl Fn(B) -> Effect<A, SchemaIssue, EncodeEnv> + Send + Sync + 'static,
    ) -> Codec<B, Encoded, DecodeEnv, EncodeEnv> {
        let after_decode = Arc::new(after_decode);
        let before_encode = Arc::new(before_encode);
        let ast = self.ast.derive(SchemaShape::EncodedView(self.ast.clone()));
        let decode = self.decode;
        let encode = self.encode;
        Codec::with_ast(
            ast,
            move |encoded| {
                let after_decode = Arc::clone(&after_decode);
                decode(encoded).flat_map(move |value| after_decode(value))
            },
            move |value| {
                let encode = Arc::clone(&encode);
                before_encode(value).flat_map(move |value| encode(value))
            },
        )
    }

    /// A refinement is a transformation whose application-side type stays A.
    #[must_use]
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    /// - `after_decode` (`impl Fn(A) -> Effect<A, SchemaIssue, DecodeEnv> + Send + Sync + 'static`): The after decode supplied to this operation.
    /// - `before_encode` (`impl Fn(A) -> Effect<A, SchemaIssue, EncodeEnv> + Send + Sync + 'static`): The before encode supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn refine_effectful(
        self,
        name: impl Into<Arc<str>>,
        after_decode: impl Fn(A) -> Effect<A, SchemaIssue, DecodeEnv> + Send + Sync + 'static,
        before_encode: impl Fn(A) -> Effect<A, SchemaIssue, EncodeEnv> + Send + Sync + 'static,
    ) -> Self {
        let mut refined = self.transform_effectful(after_decode, before_encode);
        refined.ast = refined.ast.derive(SchemaShape::Checked {
            source: refined.ast.clone(),
            name: name.into(),
        });
        refined
    }

    /// A single owned input can be evaluated once. Further runs fail as data
    /// with InputConsumed rather than silently duplicating the original input.
    ///
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `input` (`Encoded`): The input value or factory consumed by this operation.
    ///
    /// # Returns
    /// Type: `Effect<A, SchemaIssue, DecodeEnv>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn decode_once(&self, input: Encoded) -> Effect<A, SchemaIssue, DecodeEnv> {
        let decode = Arc::clone(&self.decode);
        let input = Mutex::new(Some(input));
        Effect::defer(move || {
            match input
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                Some(value) => decode(value),
                None => Effect::fail_with(|| SchemaIssue::new(SchemaIssueKind::InputConsumed)),
            }
        })
    }

    /// Consume one owned value during a single encoding run.
    ///
    /// # Type parameters
    /// - `Encoded`: Encoded representation type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `input` (`A`): The input value or factory consumed by this operation.
    ///
    /// # Returns
    /// Type: `Effect<Encoded, SchemaIssue, EncodeEnv>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn encode_once(&self, input: A) -> Effect<Encoded, SchemaIssue, EncodeEnv> {
        let encode = Arc::clone(&self.encode);
        let input = Mutex::new(Some(input));
        Effect::defer(move || {
            match input
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                Some(value) => encode(value),
                None => Effect::fail_with(|| SchemaIssue::new(SchemaIssueKind::InputConsumed)),
            }
        })
    }
}

/// An Effect-returning compatibility view for general JSON codecs. It retains
/// the decode environment; no blocking conversion to the old pure Schema occurs.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `DecodeEnv`: Decoding environment type. Bound or type: `Send + Sync + 'static`.
pub struct EffectSchema<A: Send + 'static, DecodeEnv: Send + Sync + 'static> {
    ast: SchemaAst,
    decode: EffectDecoder<A, JsonValue, DecodeEnv>,
}

impl<A: Send + 'static, DecodeEnv: Send + Sync + 'static> Clone for EffectSchema<A, DecodeEnv> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            ast: self.ast.clone(),
            decode: Arc::clone(&self.decode),
        }
    }
}

impl<A: Send + 'static, DecodeEnv: Send + Sync + 'static> EffectSchema<A, DecodeEnv> {
    #[must_use]
    /// Borrow the representation tree used for schema inspection or export.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `&SchemaAst`.
    pub fn ast(&self) -> &SchemaAst {
        &self.ast
    }

    /// Consume one encoded value during a single decoding run.
    ///
    /// # Type parameters
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `input` (`JsonValue`): The input value or factory consumed by this operation.
    ///
    /// # Returns
    /// Type: `Effect<A, SchemaIssue, DecodeEnv>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn decode_once(&self, input: JsonValue) -> Effect<A, SchemaIssue, DecodeEnv> {
        let decode = Arc::clone(&self.decode);
        let input = Mutex::new(Some(input));
        Effect::defer(move || {
            match input
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                Some(value) => decode(value),
                None => Effect::fail_with(|| SchemaIssue::new(SchemaIssueKind::InputConsumed)),
            }
        })
    }

    /// JSON parsing repeats from immutable text; decoding retains DecodeEnv.
    /// The caller supplies the byte and structural budgets explicitly.
    ///
    /// # Type parameters
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `text` (`impl Into<Arc<str>>`): Input text parsed or emitted by this operation.
    /// - `policy` (`ParsePolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Effect<A, SchemaIssue, DecodeEnv>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn parse(
        &self,
        text: impl Into<Arc<str>>,
        policy: ParsePolicy,
    ) -> Effect<A, SchemaIssue, DecodeEnv> {
        let text = text.into();
        let decode = Arc::clone(&self.decode);
        let ast = self.ast.clone();
        Effect::try_sync(
            move || {
                ast.prepare(policy.limits)?;
                super::internal::parser::parse_json(&text, policy)
            },
            std::convert::identity,
        )
        .flat_map(move |json| decode(json))
    }
}

impl<A: Send + 'static, DecodeEnv: Send + Sync + 'static, EncodeEnv: Send + Sync + 'static>
    Codec<A, JsonValue, DecodeEnv, EncodeEnv>
{
    /// Supply a typed field default through DecodeEnv only when the encoded
    /// object omits that field. A present null is never treated as absence.
    /// The field codec must be the one used by the record declaration; the
    /// constructor checks their shared schema identity before returning.
    /// Chained defaults run in reverse adapter order, once each when absent.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `F`: Bounds: `Send + 'static`.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    /// - `field_codec` (`PureCodec<F, JsonValue>`): The field codec supplied to this operation.
    /// - `default` (`impl Fn() -> Effect<F, SchemaIssue, DecodeEnv> + Send + Sync + 'static`): The default supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, SchemaIssue>`.
    pub fn with_effectful_field_default<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        field_codec: PureCodec<F, JsonValue>,
        default: impl Fn() -> Effect<F, SchemaIssue, DecodeEnv> + Send + Sync + 'static,
    ) -> Result<Self, SchemaIssue> {
        let name = name.into();
        let SchemaShape::Object {
            properties,
            unknown_fields,
        } = self.ast.shape()
        else {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        };
        let mut properties = properties.clone();
        let Some(property) = properties
            .iter_mut()
            .find(|property| property.name() == name.as_ref())
        else {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        };
        if !property.required()
            || property.pure_default().is_some()
            || !property.schema().shares_shape(field_codec.ast())
        {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        *property = property.with_required(false);
        let ast = self.ast.derive(SchemaShape::Object {
            properties,
            unknown_fields: *unknown_fields,
        });
        let decode = self.decode;
        let encode = self.encode;
        let field_codec = Arc::new(field_codec);
        let default = Arc::new(default);
        Ok(Self::with_ast(
            ast,
            move |input| {
                let JsonValue::Object(object) = input else {
                    return decode(input);
                };
                if object.contains_key(name.as_ref()) {
                    return decode(JsonValue::Object(object));
                }
                let pending = Mutex::new(Some(object));
                let decode = Arc::clone(&decode);
                let field_codec = Arc::clone(&field_codec);
                let field_name = Arc::clone(&name);
                default()
                    .map_error({
                        let field_name = Arc::clone(&field_name);
                        move |issue| issue.at(IssueSegment::Field(Arc::clone(&field_name)))
                    })
                    .flat_map(move |value| {
                        let object = pending
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        let Some(mut object) = object else {
                            return Effect::fail_with(|| {
                                SchemaIssue::new(SchemaIssueKind::InputConsumed)
                            });
                        };
                        let encoded = field_codec.encode_ref(&value).map_err(|issue| {
                            issue.at(IssueSegment::Field(Arc::clone(&field_name)))
                        });
                        match encoded {
                            Ok(encoded) => {
                                object.insert(field_name.to_string(), encoded);
                                decode(JsonValue::Object(object))
                            }
                            Err(issue) => Effect::from_result_once(Err(issue)),
                        }
                    })
            },
            move |value| encode(value),
        ))
    }

    #[must_use]
    /// Lift this pure codec into an Effect-compatible schema.
    ///
    /// # Type parameters
    /// - `DecodeEnv`: Decoding environment type. Bound: `Send + Sync + 'static`.
    /// - `EncodeEnv`: Encoding environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `EffectSchema<A, DecodeEnv>`.
    pub fn as_effect_schema(&self) -> EffectSchema<A, DecodeEnv> {
        EffectSchema {
            ast: self.ast.clone(),
            decode: Arc::clone(&self.decode),
        }
    }
}
