//! Pure and effectful bidirectional codecs.

use super::{
    JsonValue, JsonWriter, JsonWriterPolicy, ParseContext, ParsePolicy, Schema, SchemaAst,
    SchemaIssue, SchemaIssueKind, SchemaReference, SchemaShape,
};
use crate::Effect;
use crate::{codec, effect, effect_provide_some, effect_try_sync};
use std::sync::{Arc, Mutex, OnceLock};

/// Codec callbacks are shared trait objects: Rust dispatches each callback
/// through its local vtable, while `A`, `Encoded`, and the environments stay
/// statically typed. Only encoded values and schema metadata cross boundaries.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `Encoded`: Encoded representation type.
type Decoder<A, Encoded> =
    Arc<dyn Fn(&Encoded, Option<&ParseContext>) -> Result<A, SchemaIssue> + Send + Sync>;
/// Type alias for encoder in schema codec.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `Encoded`: Encoded representation type.
type Encoder<A, Encoded> =
    Arc<dyn Fn(&A, Option<&ParseContext>) -> Result<Encoded, SchemaIssue> + Send + Sync>;
/// Type alias for type validator in schema codec.
///
/// # Type parameters
/// - `A`: Application value type.
type DirectWriter<A> = Arc<dyn Fn(&A, &mut JsonWriter) -> Result<(), SchemaIssue> + Send + Sync>;
type Equivalence<A> = Arc<dyn Fn(&A, &A) -> bool + Send + Sync>;
type TypeValidator<A> =
    Arc<dyn Fn(&A, Option<&ParseContext>) -> Result<(), SchemaIssue> + Send + Sync>;
/// Type alias for effect decoder in schema codec.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `Encoded`: Encoded representation type.
/// - `Env`: Environment type supplying required capabilities.
type EffectDecoder<A, Encoded, Env> =
    Arc<dyn Fn(Encoded, Option<ParseContext>) -> Effect<A, SchemaIssue, Env> + Send + Sync>;
/// Type alias for effect encoder in schema codec.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `Encoded`: Encoded representation type.
/// - `Env`: Environment type supplying required capabilities.
type EffectEncoder<A, Encoded, Env> =
    Arc<dyn Fn(A, Option<ParseContext>) -> Effect<Encoded, SchemaIssue, Env> + Send + Sync>;
/// Borrowed bidirectional conversion with owned results and immutable schema metadata.
///
/// Clones share callbacks; payloads need no Clone bound. Independent borrowed
/// validation, direct JSON writing and semantic equality are separate optional
/// witnesses. Ordinary [`Self::validate_ref`] may encode as a fallback; defaults
/// and witness-dependent compositions require genuine independent validation.
pub struct PureCodec<A, Encoded> {
    ast: SchemaAst,
    decode: Decoder<A, Encoded>,
    encode: Encoder<A, Encoded>,
    validate_type: Option<TypeValidator<A>>,
    writer: Option<DirectWriter<A>>,
    equivalent: Option<Equivalence<A>>,
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
            writer: self.writer.as_ref().map(Arc::clone),
            equivalent: self.equivalent.as_ref().map(Arc::clone),
        }
    }
}

impl<A: Send + 'static, Encoded: Send + 'static> PureCodec<A, Encoded> {
    /// Construct borrowed decode/encode callbacks with opaque metadata and no inferred validation or writer witnesses.
    #[must_use]
    pub fn new(
        decode: impl Fn(&Encoded) -> Result<A, SchemaIssue> + Send + Sync + 'static,
        encode: impl Fn(&A) -> Result<Encoded, SchemaIssue> + Send + Sync + 'static,
    ) -> Self {
        Self::with_ast(SchemaAst::opaque(), decode, encode)
    }

    /// Construct borrowed directions with explicit metadata.
    ///
    /// The caller must keep the AST consistent with both callbacks. This does not
    /// synthesize an independent validator or direct JSON writer from the metadata.
    pub fn with_ast(
        ast: SchemaAst,
        decode: impl Fn(&Encoded) -> Result<A, SchemaIssue> + Send + Sync + 'static,
        encode: impl Fn(&A) -> Result<Encoded, SchemaIssue> + Send + Sync + 'static,
    ) -> Self {
        Self {
            ast,
            decode: Arc::new(move |value, _| decode(value)),
            encode: Arc::new(move |value, _| encode(value)),
            validate_type: None,
            writer: None,
            equivalent: None,
        }
    }

    pub(crate) fn with_context_ast(
        ast: SchemaAst,
        decode: impl Fn(&Encoded, Option<&ParseContext>) -> Result<A, SchemaIssue>
        + Send
        + Sync
        + 'static,
        encode: impl Fn(&A, Option<&ParseContext>) -> Result<Encoded, SchemaIssue>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            ast,
            decode: Arc::new(decode),
            encode: Arc::new(encode),
            validate_type: None,
            writer: None,
            equivalent: None,
        }
    }

    /// Store macro-generated directions in their existing native callback slots.
    /// Each direction retains its borrowed inputs and original shared allocation;
    /// this boundary creates no validator, writer or equivalence witness.
    #[doc(hidden)]
    pub fn from_shared_context(
        ast: SchemaAst,
        decode: Decoder<A, Encoded>,
        encode: Encoder<A, Encoded>,
    ) -> Self {
        Self {
            ast,
            decode,
            encode,
            validate_type: None,
            writer: None,
            equivalent: None,
        }
    }
    pub(crate) fn with_context_type_validator(
        mut self,
        validate: impl Fn(&A, Option<&ParseContext>) -> Result<(), SchemaIssue> + Send + Sync + 'static,
    ) -> Self {
        self.validate_type = Some(Arc::new(validate));
        self
    }
    pub(crate) fn decode_in_ref(
        &self,
        value: &Encoded,
        context: Option<&ParseContext>,
    ) -> Result<A, SchemaIssue> {
        if let Some(context) = context {
            context.visit()?;
        }
        (self.decode)(value, context)
    }
    pub(crate) fn encode_in_ref(
        &self,
        value: &A,
        context: Option<&ParseContext>,
    ) -> Result<Encoded, SchemaIssue> {
        if let Some(context) = context {
            context.visit()?;
        }
        (self.encode)(value, context)
    }
    pub(crate) fn validate_in_ref(
        &self,
        value: &A,
        context: Option<&ParseContext>,
    ) -> Result<(), SchemaIssue> {
        if let Some(context) = context {
            context.visit()?;
        }
        self.validate_type
            .as_ref()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::MissingValidator))?(
            value, context
        )
    }
    /// Supply semantic equality for type-side uniqueness without encoding values.
    ///
    /// The callback must define stable equality appropriate to this codec's values;
    /// cloning the codec shares the witness, not the payloads.
    #[must_use]
    pub fn with_equivalence(
        mut self,
        equivalent: impl Fn(&A, &A) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.equivalent = Some(Arc::new(equivalent));
        self
    }
    pub(crate) fn equivalence(&self) -> Option<Equivalence<A>> {
        self.equivalent.as_ref().map(Arc::clone)
    }
    /// Install an independent borrowed type check that does not construct an encoded value.
    ///
    /// The callback must enforce the application's type constraints. It does not
    /// assert that encoding succeeds, and does not supply a direct-writer witness.
    #[must_use]
    pub fn with_type_validator(
        mut self,
        validate: impl Fn(&A) -> Result<(), SchemaIssue> + Send + Sync + 'static,
    ) -> Self {
        self.validate_type = Some(Arc::new(move |value, _| validate(value)));
        self
    }

    /// Whether a genuine independent borrowed validator is installed; encode fallback does not count.
    #[must_use]
    pub fn has_type_validator(&self) -> bool {
        self.validate_type.is_some()
    }
    /// Check a borrowed default using only its independent validator.
    ///
    /// Returns `MissingValidator` when absent, rather than encoding or decoding the
    /// value. This allows defaults to retain ownership and avoid encode capabilities.
    pub fn validate_default_ref(&self, value: &A) -> Result<(), SchemaIssue> {
        match &self.validate_type {
            Some(validate) => validate(value, None),
            None => Err(SchemaIssue::new(SchemaIssueKind::MissingValidator)),
        }
    }

    /// Check application constraints using the borrowed validator when supplied.
    ///
    /// Without that witness, encodes the value and discards the representation. Use
    /// [`Self::validate_default_ref`] when representation replay is forbidden.
    pub fn validate_ref(&self, value: &A) -> Result<(), SchemaIssue> {
        match &self.validate_type {
            Some(validate) => validate(value, None),
            None => self.encode_ref(value).map(|_| ()),
        }
    }

    /// Inspect a borrowed encoded input and return an owned application value or a typed schema issue.
    pub fn decode_ref(&self, encoded: &Encoded) -> Result<A, SchemaIssue> {
        (self.decode)(encoded, None)
    }

    /// Inspect a borrowed application value and return an independently owned representation, applying the encoder's checks.
    pub fn encode_ref(&self, value: &A) -> Result<Encoded, SchemaIssue> {
        (self.encode)(value, None)
    }

    /// Borrow immutable encoded-side metadata for inspection and preparation.
    #[must_use]
    pub fn ast(&self) -> &SchemaAst {
        &self.ast
    }

    /// Lift borrowed directions to owned lazy effects with separate decode and encode environments.
    ///
    /// Inputs move into one execution without requiring Clone. Existing independent
    /// validation and writer witnesses propagate; contextual built-in callbacks retain
    /// configured traversal state. No runtime capability is chosen by this adapter.
    #[must_use]
    pub fn into_effect<DecodeEnv, EncodeEnv>(&self) -> Codec<A, Encoded, DecodeEnv, EncodeEnv>
    where
        DecodeEnv: Send + Sync + 'static,
        EncodeEnv: Send + Sync + 'static,
    {
        let decode = Arc::clone(&self.decode);
        let encode = Arc::clone(&self.encode);
        let validator = self.validate_type.as_ref().map(Arc::clone);
        let mut lifted = codec! { contextual; ast: self.ast.clone(), decode: move |value, context| {
            let decode = Arc::clone(&decode);
            effect_try_sync! { once; body: move || decode(&value, context.as_ref()), on_error: std::convert::identity, }
        }, encode: move |value, context| {
            let encode = Arc::clone(&encode);
            effect_try_sync! { once; body: move || encode(&value, context.as_ref()), on_error: std::convert::identity, }
        }, };
        lifted.pure_validate = validator.as_ref().map(Arc::clone);
        if let Some(writer) = self.writer.as_ref().map(Arc::clone) {
            lifted.writer = Some(Arc::new(move |value, mut output| {
                let writer = Arc::clone(&writer);
                effect_try_sync! { once; body: move || {
                    writer(&value, &mut output)?;
                    Ok(output)
                }, on_error: std::convert::identity, }
            }));
        }
        match validator {
            Some(validate) => lifted.with_context_validator(move |value, context| {
                let validate = Arc::clone(&validate);
                effect_try_sync! { once; body: move || {
                    validate(&value, context.as_ref())?;
                    Ok(value)
                }, on_error: std::convert::identity, }
            }),
            None => lifted,
        }
    }
}

impl<A: Send + 'static> PureCodec<A, JsonValue> {
    /// Install an independent direct JSON writer for one borrowed value.
    ///
    /// The callback must validate its own constraints, append exactly one value to the
    /// supplied writer, and leave caller-owned containers open. It must not replay the
    /// ordinary encoder; [`Self::write_into`] checks the value boundary and poisons on error.
    #[must_use]
    pub fn with_writer(
        mut self,
        writer: impl Fn(&A, &mut JsonWriter) -> Result<(), SchemaIssue> + Send + Sync + 'static,
    ) -> Self {
        self.writer = Some(Arc::new(writer));
        self
    }
    /// Whether a direct JSON writer witness is installed; an encoder alone is insufficient.
    #[must_use]
    pub fn has_writer(&self) -> bool {
        self.writer.is_some()
    }
    /// Append one borrowed value to an existing writer using its genuine witness.
    ///
    /// Returns `UnsupportedRepresentation` when unavailable. Enforces one value in
    /// the existing root/container and poisons on callback or boundary failure; the
    /// caller's writer policy and containers remain authoritative.
    pub fn write_into(&self, value: &A, writer: &mut JsonWriter) -> Result<(), SchemaIssue> {
        let Some(write) = &self.writer else {
            return Err(SchemaIssue::new(
                SchemaIssueKind::UnsupportedRepresentation {
                    kind: "direct_json_writer".into(),
                },
            ));
        };
        let boundary = writer.boundary()?;
        let result = write(value, writer).and_then(|()| writer.complete_value(&boundary));
        if result.is_err() {
            writer.poison();
        }
        result
    }
    /// Write one complete JSON document under `policy`, returning owned UTF-8 bytes.
    ///
    /// Requires a direct-writer witness and never falls back to ordinary encoding.
    /// Invalid structure or a resource error returns no partial document.
    pub fn write(&self, value: &A, policy: JsonWriterPolicy) -> Result<Vec<u8>, SchemaIssue> {
        if !self.has_writer() {
            return Err(SchemaIssue::new(
                SchemaIssueKind::UnsupportedRepresentation {
                    kind: "direct_json_writer".into(),
                },
            ));
        }
        let mut writer = JsonWriter::new(policy);
        self.write_into(value, &mut writer)?;
        writer.finish()
    }
    /// Prepare metadata, strictly parse UTF-8 JSON under `policy`, then decode it.
    ///
    /// Invalid UTF-8, syntax, structural bounds or schema constraints return a typed
    /// issue; bytes are not lossily repaired.
    pub fn parse_utf8(&self, bytes: &[u8], policy: ParsePolicy) -> Result<A, SchemaIssue> {
        self.ast.prepare(policy.limits)?;
        self.decode_ref(&super::internal::parser::parse_json_bytes(bytes, policy)?)
    }

    /// Prepare metadata, parse JSON under the supplied limits and field policy, then decode the resulting tree.
    pub fn parse_str(&self, text: &str, policy: ParsePolicy) -> Result<A, SchemaIssue> {
        self.ast.prepare(policy.limits)?;
        let encoded = super::internal::parser::parse_json(text, policy)?;
        self.decode_ref(&encoded)
    }

    /// Tie a recursive codec using weak self-references without an ownership cycle.
    ///
    /// `shape` describes recursive metadata; `build` constructs the directional body
    /// from a typed reference. Invalid metadata or construction fails eagerly. Nested
    /// built-in calls retain the active context; independent validation and writing
    /// are usable only when the body actually provides them.
    pub fn recursive(
        name: impl Into<Arc<str>>,
        shape: impl FnOnce(SchemaReference) -> SchemaShape,
        build: impl FnOnce(Self) -> Result<Self, SchemaIssue>,
    ) -> Result<Self, SchemaIssue> {
        let ast = SchemaAst::recursive(name, shape)?;
        let cell = Arc::new(OnceLock::<Self>::new());
        let decode_cell = Arc::downgrade(&cell);
        let encode_cell = Arc::downgrade(&cell);
        let writer_cell = Arc::downgrade(&cell);
        let validate_cell = Arc::downgrade(&cell);
        let mut reference = Self::with_context_ast(
            ast.clone(),
            move |encoded, context| {
                decode_cell
                    .upgrade()
                    .and_then(|cell| {
                        cell.get()
                            .map(|codec| codec.decode_in_ref(encoded, context))
                    })
                    .unwrap_or_else(|| Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)))
            },
            move |value, context| {
                encode_cell
                    .upgrade()
                    .and_then(|cell| cell.get().map(|codec| codec.encode_in_ref(value, context)))
                    .unwrap_or_else(|| Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)))
            },
        );
        reference.writer = Some(Arc::new(move |value, writer| {
            writer_cell
                .upgrade()
                .and_then(|cell| {
                    cell.get().map(|codec| match &codec.writer {
                        Some(write) => write(value, writer),
                        None => Err(SchemaIssue::new(SchemaIssueKind::MissingValidator)),
                    })
                })
                .unwrap_or_else(|| Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)))
        }));
        reference.validate_type = Some(Arc::new(move |value, context| {
            validate_cell
                .upgrade()
                .and_then(|cell| {
                    cell.get()
                        .map(|codec| codec.validate_in_ref(value, context))
                })
                .unwrap_or_else(|| Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)))
        }));
        let body = build(reference)?;
        let has_writer = body.writer.is_some();
        let has_validator = body.validate_type.is_some();
        cell.set(body)
            .map_err(|_| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let decode = Arc::clone(&cell);
        let writer_cell = Arc::clone(&cell);
        let validate_cell = Arc::clone(&cell);
        let mut recursive = Self::with_context_ast(
            ast,
            move |encoded, context| {
                decode
                    .get()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?
                    .decode_in_ref(encoded, context)
            },
            move |value, context| {
                cell.get()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?
                    .encode_in_ref(value, context)
            },
        );
        if has_writer {
            recursive.writer = Some(Arc::new(move |value, writer| {
                writer_cell
                    .get()
                    .and_then(|codec| codec.writer.as_ref())
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?(
                    value, writer,
                )
            }));
        }
        if has_validator {
            recursive.validate_type = Some(Arc::new(move |value, context| {
                validate_cell
                    .get()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?
                    .validate_in_ref(value, context)
            }));
        }
        Ok(recursive)
    }

    /// Apply a named predicate after decode and before encode or direct writing.
    ///
    /// Failure returns `InvalidValue` with `name`. The refined codec retains an
    /// independent validator or writer only when the source has that witness; the
    /// predicate does not fabricate missing type validation.
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
        let writer_source = self.clone();
        let validate_source = self.validate_type.as_ref().map(Arc::clone);
        let has_writer = self.has_writer();
        let predicate = Arc::new(predicate);
        let decode_predicate = Arc::clone(&predicate);
        let writer_predicate = Arc::clone(&predicate);
        let validate_predicate = Arc::clone(&predicate);
        let writer_name = Arc::clone(&name);
        let validate_name = Arc::clone(&name);
        let decode_name = Arc::clone(&name);
        let refined = Self::with_context_ast(
            ast,
            move |encoded, context| {
                let value = decode.decode_in_ref(encoded, context)?;
                if decode_predicate(&value) {
                    Ok(value)
                } else {
                    Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                        code: Arc::clone(&decode_name),
                    }))
                }
            },
            move |value, context| {
                if predicate(value) {
                    self.encode_in_ref(value, context)
                } else {
                    Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                        code: Arc::clone(&name),
                    }))
                }
            },
        );
        let refined = if let Some(validate_source) = validate_source {
            refined.with_context_type_validator(move |value, context| {
                validate_source(value, context)?;
                if validate_predicate(value) {
                    Ok(())
                } else {
                    Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                        code: Arc::clone(&validate_name),
                    }))
                }
            })
        } else {
            refined
        };
        if has_writer {
            refined.with_writer(move |value, writer| {
                if !writer_predicate(value) {
                    return Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                        code: Arc::clone(&writer_name),
                    }));
                }
                writer_source.write_into(value, writer)
            })
        } else {
            refined
        }
    }

    /// Map owned decoded values to `B` and project borrowed `B` values back to `A`.
    ///
    /// The encoded representation and contextual traversal are retained. Independent
    /// validation and writing propagate only from genuine source witnesses and apply
    /// the projection without ordinary representation replay.
    pub fn transform<B: Send + 'static>(
        self,
        decode: impl Fn(A) -> Result<B, SchemaIssue> + Send + Sync + 'static,
        encode: impl Fn(&B) -> Result<A, SchemaIssue> + Send + Sync + 'static,
    ) -> PureCodec<B, JsonValue> {
        let ast = self
            .ast()
            .derive(SchemaShape::EncodedView(self.ast().clone()));
        let input = self.clone();
        let writer_source = self.clone();
        let validation_source = self.clone();
        let has_validator = self.validate_type.is_some();
        let has_writer = self.has_writer();
        let encode = Arc::new(encode);
        let validation_encode = Arc::clone(&encode);
        let writer_encode = Arc::clone(&encode);
        let transformed = PureCodec::with_context_ast(
            ast,
            move |encoded, context| decode(input.decode_in_ref(encoded, context)?),
            move |value, context| self.encode_in_ref(&encode(value)?, context),
        );
        let transformed = if has_validator {
            transformed.with_context_type_validator(move |value, context| {
                validation_source.validate_in_ref(&validation_encode(value)?, context)
            })
        } else {
            transformed
        };
        if has_writer {
            transformed.with_writer(move |value, writer| {
                writer_source.write_into(&writer_encode(value)?, writer)
            })
        } else {
            transformed
        }
    }

    /// Expose the pure JSON decoder through the legacy one-way Schema error mapping; no reverse encoder or effectful blocking bridge is inferred.
    pub fn to_schema<E: Send + 'static>(
        &self,
        issue: impl Fn(SchemaIssue) -> E + Send + Sync + 'static,
    ) -> Schema<A, E> {
        let codec = self.clone();
        Schema::from_decoder(move |encoded| codec.decode_ref(encoded).map_err(&issue))
    }
}

type EffectValidator<A, R> =
    Arc<dyn Fn(A, Option<ParseContext>) -> Effect<A, SchemaIssue, R> + Send + Sync>;
type EffectWriter<A, R> =
    Arc<dyn Fn(A, JsonWriter) -> Effect<JsonWriter, SchemaIssue, R> + Send + Sync>;

/// Owned effectful directions with independently selected decode and encode environments.
///
/// Decode uses `DecodeEnv`; encode, owned validation and direct writing use
/// `EncodeEnv`. Callbacks may suspend while retaining owned values and invocation
/// context. Clones share descriptions, not payloads. One-shot inputs are consumed
/// once; factory methods provide reusable executions. Validation and writing are
/// optional capabilities, never inferred from an encoded AST alone.
pub struct Codec<
    A: Send + 'static,
    Encoded: Send + 'static,
    DecodeEnv: Send + Sync + 'static,
    EncodeEnv: Send + Sync + 'static,
> {
    ast: SchemaAst,
    decode: EffectDecoder<A, Encoded, DecodeEnv>,
    encode: EffectEncoder<A, Encoded, EncodeEnv>,
    validate: Option<EffectValidator<A, EncodeEnv>>,
    pure_validate: Option<TypeValidator<A>>,
    writer: Option<EffectWriter<A, EncodeEnv>>,
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
            validate: self.validate.as_ref().map(Arc::clone),
            pure_validate: self.pure_validate.as_ref().map(Arc::clone),
            writer: self.writer.as_ref().map(Arc::clone),
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
    /// Describe owned effectful decode and encode callbacks with opaque metadata; callbacks remain lazy and no witnesses are inferred.
    #[must_use]
    pub fn new(
        decode: impl Fn(Encoded) -> Effect<A, SchemaIssue, DecodeEnv> + Send + Sync + 'static,
        encode: impl Fn(A) -> Effect<Encoded, SchemaIssue, EncodeEnv> + Send + Sync + 'static,
    ) -> Self {
        Self::with_ast(SchemaAst::opaque(), decode, encode)
    }

    /// Describe owned directions with explicit AST metadata; the caller must make the metadata and callbacks agree.
    pub fn with_ast(
        ast: SchemaAst,
        decode: impl Fn(Encoded) -> Effect<A, SchemaIssue, DecodeEnv> + Send + Sync + 'static,
        encode: impl Fn(A) -> Effect<Encoded, SchemaIssue, EncodeEnv> + Send + Sync + 'static,
    ) -> Self {
        Self::with_context(
            ast,
            move |value, _| decode(value),
            move |value, _| encode(value),
        )
    }

    /// Construct directional callbacks that explicitly receive invocation state.
    ///
    /// Configured calls provide an owned [`ParseContext`]; unconfigured calls supply
    /// None. Custom composites must forward the context to nested work so suspension
    /// does not reset counters or paths. The caller keeps metadata and checks consistent.
    pub fn with_context(
        ast: SchemaAst,
        decode: impl Fn(Encoded, Option<ParseContext>) -> Effect<A, SchemaIssue, DecodeEnv>
        + Send
        + Sync
        + 'static,
        encode: impl Fn(A, Option<ParseContext>) -> Effect<Encoded, SchemaIssue, EncodeEnv>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self::from_shared_context(ast, Arc::new(decode), Arc::new(encode))
    }

    /// Store macro-generated owned directions in the existing native callback slots.
    /// ParseContext and both environment types remain explicit; already shared
    /// callbacks are retained without another allocation or inferred witnesses.
    #[doc(hidden)]
    pub fn from_shared_context(
        ast: SchemaAst,
        decode: EffectDecoder<A, Encoded, DecodeEnv>,
        encode: EffectEncoder<A, Encoded, EncodeEnv>,
    ) -> Self {
        Self {
            ast,
            decode,
            encode,
            validate: None,
            pure_validate: None,
            writer: None,
        }
    }
    /// Tie owned effectful directions through weak self-references, retaining context across suspension.
    ///
    /// `shape` supplies recursive metadata and `build` constructs the body. Values need
    /// no Clone bound. Optional validation/writer capabilities propagate only from the
    /// body's genuine witnesses; invalid preparation or body construction fails eagerly.
    pub fn recursive(
        name: impl Into<Arc<str>>,
        shape: impl FnOnce(SchemaReference) -> SchemaShape,
        build: impl FnOnce(Self) -> Result<Self, SchemaIssue>,
    ) -> Result<Self, SchemaIssue> {
        let ast = SchemaAst::recursive(name, shape)?;
        let cell = Arc::new(OnceLock::<Self>::new());
        let decode = Arc::downgrade(&cell);
        let encode = Arc::downgrade(&cell);
        let validate = Arc::downgrade(&cell);
        let writer = Arc::downgrade(&cell);
        let mut reference = Self::with_context(
            ast.clone(),
            move |value, context| match decode
                .upgrade()
                .and_then(|cell| cell.get().map(|body| (body.decode)(value, context)))
            {
                Some(effect) => effect,
                None => Effect::fail_with(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition)),
            },
            move |value, context| match encode
                .upgrade()
                .and_then(|cell| cell.get().map(|body| (body.encode)(value, context)))
            {
                Some(effect) => effect,
                None => Effect::fail_with(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition)),
            },
        );
        reference.validate = Some(Arc::new(move |value, context| {
            match validate.upgrade().and_then(|cell| {
                cell.get()
                    .and_then(|body| body.validate.as_ref())
                    .map(|validate| validate(value, context))
            }) {
                Some(effect) => effect,
                None => Effect::fail_with(|| SchemaIssue::new(SchemaIssueKind::MissingValidator)),
            }
        }));
        reference.writer = Some(Arc::new(move |value, output| {
            match writer.upgrade().and_then(|cell| {
                cell.get()
                    .and_then(|body| body.writer.as_ref())
                    .map(|write| write(value, output))
            }) {
                Some(effect) => effect,
                None => Effect::fail_with(|| {
                    SchemaIssue::new(SchemaIssueKind::UnsupportedRepresentation {
                        kind: "direct_json_writer".into(),
                    })
                }),
            }
        }));
        let body = build(reference)?;
        let has_validator = body.validate.is_some();
        let has_writer = body.writer.is_some();
        let pure_validate = body.pure_validate.clone();
        cell.set(body)
            .map_err(|_| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let decode = Arc::clone(&cell);
        let encode = Arc::clone(&cell);
        let validate = Arc::clone(&cell);
        let writer = Arc::clone(&cell);
        let mut result = Self::with_context(
            ast,
            move |value, context| match decode.get() {
                Some(body) => (body.decode)(value, context),
                None => Effect::fail_with(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition)),
            },
            move |value, context| match encode.get() {
                Some(body) => (body.encode)(value, context),
                None => Effect::fail_with(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition)),
            },
        );
        if has_validator {
            result.validate = Some(Arc::new(move |value, context| {
                match validate.get().and_then(|body| body.validate.as_ref()) {
                    Some(validate) => validate(value, context),
                    None => {
                        Effect::fail_with(|| SchemaIssue::new(SchemaIssueKind::MissingValidator))
                    }
                }
            }));
        }
        if has_writer {
            result.writer = Some(Arc::new(move |value, output| {
                match writer.get().and_then(|body| body.writer.as_ref()) {
                    Some(write) => write(value, output),
                    None => {
                        Effect::fail_with(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))
                    }
                }
            }));
        }
        result.pure_validate = pure_validate;
        Ok(result)
    }

    /// Install a lazy owned writer in `EncodeEnv`.
    ///
    /// The callback must check its constraints and append exactly one value to the same
    /// writer, preserving policy and caller containers. It may suspend; checked boundary
    /// completion rejects writer replacement or an incorrect number of values.
    #[must_use]
    pub fn with_writer(
        mut self,
        write: impl Fn(A, JsonWriter) -> Effect<JsonWriter, SchemaIssue, EncodeEnv>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.writer = Some(Arc::new(write));
        self
    }
    /// Whether the owned direct-writing direction has an explicit witness.
    #[must_use]
    pub fn has_writer(&self) -> bool {
        self.writer.is_some()
    }
    /// Lazily consume `value` and append exactly one JSON value in `EncodeEnv`.
    ///
    /// Requires a genuine writer witness. It returns the same owned writer, preserving
    /// its policy and caller containers even across suspension. Replacing the writer,
    /// closing caller containers, or producing zero/two values is a schema failure.
    /// The returned effect owns a one-shot value/writer pair.
    pub fn write_into_once(
        &self,
        value: A,
        mut writer: JsonWriter,
    ) -> Effect<JsonWriter, SchemaIssue, EncodeEnv> {
        match self.writer.as_ref().map(Arc::clone) {
            Some(write) => {
                let boundary = match writer.boundary() {
                    Ok(boundary) => Arc::new(boundary),
                    Err(issue) => {
                        return effect_try_sync! { once; body: { let __dsl_result = Err(issue); move || __dsl_result }, on_error: ::core::convert::identity, };
                    }
                };
                let input = effect_try_sync! {
                    once;
                    body: move || Ok((value, writer)),
                    on_error: std::convert::identity,
                };
                effect! {
                    let input = input.clone();
                    let write = Arc::clone(&write);
                    let boundary = Arc::clone(&boundary);
                    let (value, writer) = input?;
                    let mut writer = write(value, writer)?;
                    effect_try_sync! {
                        once;
                        body: move || writer.complete_value(&boundary).map(|()| writer),
                        on_error: std::convert::identity,
                    }?
                }
            }
            None => effect_try_sync! { body: { let __dsl_error = || {
                SchemaIssue::new(SchemaIssueKind::UnsupportedRepresentation {
                    kind: "direct_json_writer".into(),
                })
            }; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, },
        }
    }
    /// Lazily write a complete document in `EncodeEnv` using the supplied output bounds.
    ///
    /// Returns owned bytes only after checked completion; no ordinary-encoder fallback
    /// exists. The input is consumed once without requiring Clone.
    pub fn write_once(
        &self,
        value: A,
        policy: JsonWriterPolicy,
    ) -> Effect<Vec<u8>, SchemaIssue, EncodeEnv> {
        let write = self.write_into_once(value, JsonWriter::new(policy));
        effect! {
            let writer = write.clone()?;
            effect_try_sync! {
                once;
                body: move || writer.finish(),
                on_error: std::convert::identity,
            }?
        }
    }

    /// Add immutable annotation metadata while preserving directional witnesses.
    pub fn annotate(
        mut self,
        name: impl Into<Arc<str>>,
        value: impl Into<Arc<str>>,
    ) -> Result<Self, SchemaIssue> {
        self.ast = self.ast.annotate(name, value)?;
        Ok(self)
    }

    /// Install lazy owned type validation in `EncodeEnv`, returning the validated input.
    ///
    /// This does not create the borrowed witness needed for defaults decoded in a
    /// different environment. Use context-aware validation for custom nested traversal.
    #[must_use]
    pub fn with_validator(
        mut self,
        validate: impl Fn(A) -> Effect<A, SchemaIssue, EncodeEnv> + Send + Sync + 'static,
    ) -> Self {
        self.validate = Some(Arc::new(move |value, _| validate(value)));
        self
    }

    /// Check a borrowed default without representation replay or encode-environment access.
    ///
    /// Requires the independent pure witness, as propagated by [`PureCodec::into_effect`];
    /// a standalone owned validator is insufficient and yields `MissingValidator`.
    pub fn validate_default_ref(&self, value: &A) -> Result<(), SchemaIssue> {
        self.validate_default_in_ref(value, None)
    }
    /// Validate a borrowed default with the independent witness and optional active context.
    ///
    /// A supplied context charges one schema visit before validation. Returns
    /// `MissingValidator` when no pure witness exists; never invokes an encoder or
    /// acquires encode-environment capabilities. Used by generated native defaults.
    pub fn validate_default_in_ref(
        &self,
        value: &A,
        context: Option<&ParseContext>,
    ) -> Result<(), SchemaIssue> {
        if let Some(context) = context {
            context.visit()?;
        }
        match &self.pure_validate {
            Some(validate) => validate(value, context),
            None => Err(SchemaIssue::new(SchemaIssueKind::MissingValidator)),
        }
    }

    /// Install owned validation that explicitly retains the optional invocation context.
    ///
    /// Nested checks must reuse this state rather than creating fresh counters. Returns
    /// the owned validated value in `EncodeEnv`; it supplies no borrowed default witness.
    #[must_use]
    pub fn with_context_validator(
        mut self,
        validate: impl Fn(A, Option<ParseContext>) -> Effect<A, SchemaIssue, EncodeEnv>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.validate = Some(Arc::new(validate));
        self
    }

    /// Lazily validate one owned value in `EncodeEnv`, returning it on success.
    ///
    /// Returns `MissingValidator` when unavailable. The value is consumed once;
    /// validation does not fall back to encoding.
    pub fn validate_once(&self, value: A) -> Effect<A, SchemaIssue, EncodeEnv> {
        self.validate_in(value, None)
    }

    pub(crate) fn validate_in(
        &self,
        value: A,
        context: Option<ParseContext>,
    ) -> Effect<A, SchemaIssue, EncodeEnv> {
        let validate = self.validate.as_ref().map(Arc::clone);
        let input = effect_try_sync! { once; body: move || {
            if let Some(context) = &context {
                context.visit()?;
            }
            Ok((value, context))
        }, on_error: std::convert::identity, };
        effect! {
            let validate = validate.as_ref().map(Arc::clone);
            let (value, context) = input.clone()?;
            match validate {
                Some(validate) => validate(value, context)?,
                None => effect_try_sync! {
                    body: || Err(SchemaIssue::new(SchemaIssueKind::MissingValidator)),
                    on_error: std::convert::identity,
                }?,
            }
        }
    }

    /// Decode one owned input while forwarding a caller's optional traversal context.
    /// Custom contextual codec directions use this existing boundary to share paths
    /// and cumulative budgets through nested work and suspension. The input is consumed
    /// once; None selects an unconfigured invocation and grants no purity/writer witness.
    pub fn decode_in(
        &self,
        value: Encoded,
        context: Option<ParseContext>,
    ) -> Effect<A, SchemaIssue, DecodeEnv> {
        let decode = Arc::clone(&self.decode);
        let input = effect_try_sync! { once; body: move || {
            if let Some(context) = &context {
                context.visit()?;
                context.check_representation(&value)?;
            }
            Ok((value, context))
        }, on_error: std::convert::identity, };
        effect! {
            let decode = Arc::clone(&decode);
            let (value, context) = input.clone()?;
            decode(value, context)?
        }
    }

    /// Encode one owned value under the caller's optional traversal context.
    /// The same context and directional EncodeEnv cross nested codec callbacks;
    /// visits and cumulative budgets retain their existing checks. No payload copy,
    /// context reset or validator/writer witness is introduced by this adapter.
    pub fn encode_in(
        &self,
        value: A,
        context: Option<ParseContext>,
    ) -> Effect<Encoded, SchemaIssue, EncodeEnv> {
        let encode = Arc::clone(&self.encode);
        let input = effect_try_sync! { once; body: move || {
            if let Some(context) = &context {
                context.visit()?;
            }
            Ok((value, context))
        }, on_error: std::convert::identity, };
        effect! {
            let encode = Arc::clone(&encode);
            let (value, context) = input.clone()?;
            let checked = context.clone();
            let value = encode(value, context)?;
            effect_try_sync! { once; body: move || {
                        if let Some(context) = checked {
                            context.check_representation(&value)?;
                        }
                        Ok(value)
                    }, on_error: std::convert::identity, }?
        }
    }

    /// Create a reusable lazy decode in `DecodeEnv` from a fresh owned input factory.
    ///
    /// The factory runs for each execution; no stored input or decoded payload is cloned.
    pub fn decode_with(
        &self,
        input: impl Fn() -> Encoded + Send + Sync + 'static,
    ) -> Effect<A, SchemaIssue, DecodeEnv> {
        let codec = self.clone();
        {
            let __dsl_factory = move || codec.decode_once(input());
            effect! { __dsl_factory()? }
        }
    }

    /// Create a reusable lazy encode in `EncodeEnv`; the factory supplies a fresh owned application value per execution.
    pub fn encode_with(
        &self,
        input: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<Encoded, SchemaIssue, EncodeEnv> {
        let encode = Arc::clone(&self.encode);
        {
            let __dsl_factory = move || encode(input(), None);
            effect! { __dsl_factory()? }
        }
    }

    /// Borrow the immutable encoded-side AST without acquiring directional capabilities.
    #[must_use]
    pub fn ast(&self) -> &SchemaAst {
        &self.ast
    }

    /// Project an outer environment into DecodeEnv while preserving the encode, validation and writer directions.
    #[must_use]
    pub fn provide_decode<Outer: Send + Sync + 'static>(
        &self,
        project: impl Fn(&Outer) -> DecodeEnv + Send + Sync + 'static,
    ) -> Codec<A, Encoded, Outer, EncodeEnv> {
        let decode = Arc::clone(&self.decode);
        let encode = Arc::clone(&self.encode);
        let project = Arc::new(project);
        let mut codec = codec! { contextual; ast: self.ast.clone(), decode: move |input, context| {
            let project = Arc::clone(&project);
            effect_provide_some! { program: decode(input, context), project: move |outer| project(outer), }
        }, encode: move |input, context| encode(input, context), };
        codec.validate = self.validate.as_ref().map(Arc::clone);
        codec.writer = self.writer.as_ref().map(Arc::clone);
        codec.pure_validate = self.pure_validate.as_ref().map(Arc::clone);
        codec
    }

    /// Project an outer environment into EncodeEnv for encode, owned validation and writing; decode remains independent.
    #[must_use]
    pub fn provide_encode<Outer: Send + Sync + 'static>(
        &self,
        project: impl Fn(&Outer) -> EncodeEnv + Send + Sync + 'static,
    ) -> Codec<A, Encoded, DecodeEnv, Outer> {
        let decode = Arc::clone(&self.decode);
        let encode = Arc::clone(&self.encode);
        let project = Arc::new(project);
        let validation_project = Arc::clone(&project);
        let mut codec = codec! { contextual; ast: self.ast.clone(), decode: move |input, context| decode(input, context), encode: move |input, context| {
            let project = Arc::clone(&project);
            effect_provide_some! { program: encode(input, context), project: move |outer| project(outer), }
        }, };
        if let Some(validate) = self.validate.as_ref().map(Arc::clone) {
            let validation_project = Arc::clone(&validation_project);
            codec.validate = Some(Arc::new(move |value, context| {
                let project = Arc::clone(&validation_project);
                effect_provide_some! { program: validate(value, context), project: move |outer| project(outer), }
            }));
        }
        codec.pure_validate = self.pure_validate.as_ref().map(Arc::clone);
        if let Some(write) = self.writer.as_ref().map(Arc::clone) {
            let project = Arc::clone(&validation_project);
            codec.writer = Some(Arc::new(move |value, writer| {
                let project = Arc::clone(&project);
                effect_provide_some! { program: write(value, writer), project: move |outer| project(outer), }
            }));
        }
        codec
    }

    /// Compose owned conversion effects after decode and before encode, retaining each environment.
    ///
    /// Both callbacks may suspend. A source writer propagates through `before_encode`;
    /// the transformed type has no inferred independent validator or borrowed default
    /// witness. Supply explicit validation when the transformed contract needs it.
    #[must_use]
    pub fn transform_effectful<B: Send + 'static>(
        self,
        after_decode: impl Fn(A) -> Effect<B, SchemaIssue, DecodeEnv> + Send + Sync + 'static,
        before_encode: impl Fn(B) -> Effect<A, SchemaIssue, EncodeEnv> + Send + Sync + 'static,
    ) -> Codec<B, Encoded, DecodeEnv, EncodeEnv> {
        let after_decode = Arc::new(after_decode);
        let before_encode = Arc::new(before_encode);
        let writer = self.writer;
        let writer_before = Arc::clone(&before_encode);
        let ast = self.ast.derive(SchemaShape::EncodedView(self.ast.clone()));
        let decode = self.decode;
        let encode = self.encode;
        let mut transformed = Codec::with_context(
            ast,
            move |encoded, context| {
                let after_decode = Arc::clone(&after_decode);
                let decoded = decode(encoded, context);
                effect! {
                    let after_decode = Arc::clone(&after_decode);
                    let value = decoded.clone()?;
                    after_decode(value)?
                }
            },
            move |value, context| {
                let encode = Arc::clone(&encode);
                let input = before_encode(value);
                effect! {
                    let encode = Arc::clone(&encode);
                    let context = context.clone();
                    let value = input.clone()?;
                    encode(value, context)?
                }
            },
        );
        if let Some(write) = writer {
            transformed.writer = Some(Arc::new(move |value, writer| {
                let write = Arc::clone(&write);
                let input = writer_before(value);
                let output = effect_try_sync! {
                    once;
                    body: move || Ok(writer),
                    on_error: std::convert::identity,
                };
                effect! {
                    let write = Arc::clone(&write);
                    let output = output.clone();
                    let value = input.clone()?;
                    let writer = output?;
                    write(value, writer)?
                }
            }));
        }
        transformed
    }

    /// Keep application type `A` while adding named effectful checks in both directions.
    ///
    /// Decode and encode checks use their respective environments. If the source has
    /// owned validation, validation also applies `before_encode`; the refinement does
    /// not invent a borrowed default witness or ordinary-encoder fallback.
    #[must_use]
    pub fn refine_effectful(
        self,
        name: impl Into<Arc<str>>,
        after_decode: impl Fn(A) -> Effect<A, SchemaIssue, DecodeEnv> + Send + Sync + 'static,
        before_encode: impl Fn(A) -> Effect<A, SchemaIssue, EncodeEnv> + Send + Sync + 'static,
    ) -> Self {
        let validate = self.clone();
        let before_encode = Arc::new(before_encode);
        let validation_check = Arc::clone(&before_encode);
        let mut refined = codec! { transform; source: self, decode: after_decode, encode: move |value| before_encode(value), };
        if validate.validate.is_some() {
            refined = refined.with_context_validator(move |value, context| {
                let validate = validate.clone();
                let input = validation_check(value);
                effect! {
                    let validate = validate.clone();
                    let context = context.clone();
                    let value = input.clone()?;
                    validate.validate_in(value, context)?
                }
            });
        }
        refined.ast = refined.ast.derive(SchemaShape::Checked {
            source: refined.ast.clone(),
            name: name.into(),
        });
        refined
    }

    /// Lazily consume one encoded input in `DecodeEnv`, without cloning it.
    ///
    /// The returned effect is single-use. Further execution reaches the consumed
    /// synchronous-callback defect in [`crate::Exit::Die`]; use [`Self::decode_with`]
    /// to supply a new input for each run.
    pub fn decode_once(&self, input: Encoded) -> Effect<A, SchemaIssue, DecodeEnv> {
        self.decode_in(input, None)
    }

    /// Lazily consume one application value in `EncodeEnv`, without cloning it.
    ///
    /// The returned effect is single-use; repeated execution yields a consumed-input
    /// defect. Use [`Self::encode_with`] for reusable value factories.
    pub fn encode_once(&self, input: A) -> Effect<Encoded, SchemaIssue, EncodeEnv> {
        self.encode_in(input, None)
    }
}

/// One-way compatibility view of an effectful JSON codec.
///
/// Preserves the decode environment. Owned decode is single-use; parsing shared
/// text can run repeatedly. It does not convert effectful callbacks into a blocking
/// pure schema or supply an encoder.
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
    /// Borrow the underlying JSON decode metadata.
    #[must_use]
    pub fn ast(&self) -> &SchemaAst {
        &self.ast
    }

    /// Lazily decode one owned JSON value in `DecodeEnv`.
    ///
    /// This compatibility view reports `SchemaIssueKind::InputConsumed` on further
    /// executions of the same effect, rather than copying the retained input.
    pub fn decode_once(&self, input: JsonValue) -> Effect<A, SchemaIssue, DecodeEnv> {
        let decode = Arc::clone(&self.decode);
        let input = Mutex::new(Some(input));
        {
            let __dsl_factory = move || match input
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                Some(value) => decode(value, None),
                None => {
                    effect_try_sync! { body: { let __dsl_error = || SchemaIssue::new(SchemaIssueKind::InputConsumed); move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }
                }
            };
            effect! { __dsl_factory()? }
        }
    }

    /// Lazily parse shared immutable JSON text and decode in DecodeEnv on each execution.
    ///
    /// Prepares metadata and enforces the supplied parse policy before decoding. Use
    /// Codec::configured for independent schema/JSON depth and cumulative configured
    /// pattern budgets; this compatibility surface carries only ParsePolicy.
    pub fn parse(
        &self,
        text: impl Into<Arc<str>>,
        policy: ParsePolicy,
    ) -> Effect<A, SchemaIssue, DecodeEnv> {
        let text = text.into();
        let decode = Arc::clone(&self.decode);
        let ast = self.ast.clone();
        let parsed = effect_try_sync! { body: move || {
            ast.prepare(policy.limits)?;
            super::internal::parser::parse_json(&text, policy)
        }, on_error: std::convert::identity, };
        effect! {
            let decode = Arc::clone(&decode);
            let json = parsed.clone()?;
            decode(json, None)?
        }
    }
}

impl<A: Send + 'static, DecodeEnv: Send + Sync + 'static, EncodeEnv: Send + Sync + 'static>
    Codec<A, JsonValue, DecodeEnv, EncodeEnv>
{
    /// Expose a one-way effectful JSON decode view, preserving DecodeEnv and immutable metadata.
    #[must_use]
    pub fn as_effect_schema(&self) -> EffectSchema<A, DecodeEnv> {
        EffectSchema {
            ast: self.ast.clone(),
            decode: Arc::clone(&self.decode),
        }
    }
}
