use super::{
    IssueSegment, JsonValue, ParsePolicy, Schema, SchemaAst, SchemaIssue, SchemaIssueKind,
    SchemaReference, SchemaShape,
};
use crate::Effect;
use std::sync::{Arc, Mutex, OnceLock};

type Decoder<A, Encoded> = Arc<dyn Fn(&Encoded) -> Result<A, SchemaIssue> + Send + Sync>;
type Encoder<A, Encoded> = Arc<dyn Fn(&A) -> Result<Encoded, SchemaIssue> + Send + Sync>;
type TypeValidator<A> = Arc<dyn Fn(&A) -> Result<(), SchemaIssue> + Send + Sync>;
type EffectDecoder<A, Encoded, Env> =
    Arc<dyn Fn(Encoded) -> Effect<A, SchemaIssue, Env> + Send + Sync>;
type EffectEncoder<A, Encoded, Env> =
    Arc<dyn Fn(A) -> Effect<Encoded, SchemaIssue, Env> + Send + Sync>;
type OnceOperation<Input, Output> = dyn Fn(&Input) -> Result<Output, SchemaIssue> + Send + Sync;

/// Pure bidirectional conversion with independently owned results.
/// The callbacks inspect borrowed inputs; no payload Clone is implied.
pub struct PureCodec<A, Encoded> {
    ast: SchemaAst,
    decode: Decoder<A, Encoded>,
    encode: Encoder<A, Encoded>,
    validate_type: Option<TypeValidator<A>>,
}

impl<A, Encoded> Clone for PureCodec<A, Encoded> {
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
    pub fn new(
        decode: impl Fn(&Encoded) -> Result<A, SchemaIssue> + Send + Sync + 'static,
        encode: impl Fn(&A) -> Result<Encoded, SchemaIssue> + Send + Sync + 'static,
    ) -> Self {
        Self::with_ast(SchemaAst::opaque(), decode, encode)
    }

    /// Construct a codec with an explicit encoded-side description. The
    /// caller must keep that description consistent with both callbacks.
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
    pub fn validate_ref(&self, value: &A) -> Result<(), SchemaIssue> {
        match &self.validate_type {
            Some(validate) => validate(value),
            None => self.encode_ref(value).map(|_| ()),
        }
    }

    pub fn decode_ref(&self, encoded: &Encoded) -> Result<A, SchemaIssue> {
        (self.decode)(encoded)
    }

    pub fn encode_ref(&self, value: &A) -> Result<Encoded, SchemaIssue> {
        (self.encode)(value)
    }

    #[must_use]
    pub fn ast(&self) -> &SchemaAst {
        &self.ast
    }

    /// Lift pure directions into independent Effect environments.
    #[must_use]
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
    pub fn parse_str(&self, text: &str, policy: ParsePolicy) -> Result<A, SchemaIssue> {
        self.ast.prepare(policy.limits)?;
        let encoded = super::internal::parser::parse_json(text, policy)?;
        self.decode_ref(&encoded)
    }

    /// Tie a recursive codec without an owning reference cycle. The body sees
    /// a typed self-reference, while AST references remain weak and exportable.
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
    pub fn to_schema<E: Send + 'static>(
        &self,
        issue: impl Fn(SchemaIssue) -> E + Send + Sync + 'static,
    ) -> Schema<A, E> {
        let codec = self.clone();
        Schema::from_decoder(move |encoded| codec.decode_ref(encoded).map_err(&issue))
    }
}

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
    pub fn new(
        decode: impl Fn(Encoded) -> Effect<A, SchemaIssue, DecodeEnv> + Send + Sync + 'static,
        encode: impl Fn(A) -> Effect<Encoded, SchemaIssue, EncodeEnv> + Send + Sync + 'static,
    ) -> Self {
        Self::with_ast(SchemaAst::opaque(), decode, encode)
    }

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
    pub fn decode_with(
        &self,
        input: impl Fn() -> Encoded + Send + Sync + 'static,
    ) -> Effect<A, SchemaIssue, DecodeEnv> {
        let decode = Arc::clone(&self.decode);
        Effect::defer(move || decode(input()))
    }

    pub fn encode_with(
        &self,
        input: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<Encoded, SchemaIssue, EncodeEnv> {
        let encode = Arc::clone(&self.encode);
        Effect::defer(move || encode(input()))
    }

    #[must_use]
    pub fn ast(&self) -> &SchemaAst {
        &self.ast
    }

    /// Adapt only decoding to a larger environment. The original encode
    /// environment remains unchanged and visible in the returned type.
    #[must_use]
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
pub struct EffectSchema<A: Send + 'static, DecodeEnv: Send + Sync + 'static> {
    ast: SchemaAst,
    decode: EffectDecoder<A, JsonValue, DecodeEnv>,
}

impl<A: Send + 'static, DecodeEnv: Send + Sync + 'static> Clone for EffectSchema<A, DecodeEnv> {
    fn clone(&self) -> Self {
        Self {
            ast: self.ast.clone(),
            decode: Arc::clone(&self.decode),
        }
    }
}

impl<A: Send + 'static, DecodeEnv: Send + Sync + 'static> EffectSchema<A, DecodeEnv> {
    #[must_use]
    pub fn ast(&self) -> &SchemaAst {
        &self.ast
    }

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
    pub fn as_effect_schema(&self) -> EffectSchema<A, DecodeEnv> {
        EffectSchema {
            ast: self.ast.clone(),
            decode: Arc::clone(&self.decode),
        }
    }
}
