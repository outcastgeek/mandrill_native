//! Schema errors mapped explicitly into a domain channel; Effect's other
//! outcome channels are preserved by map_error.
use super::{Codec, SchemaAst, SchemaIssue};
use crate::Effect;
use std::sync::Arc;
type Direction<A, B, E, R> = Arc<dyn Fn(A) -> Effect<B, E, R> + Send + Sync>;
/// An owned codec with an application-defined typed error channel.
///
/// Decode runs in `D`; encode and validation run in `R`. Each directional call
/// moves its input into a lazy one-shot effect. Cloning shares callbacks and AST
/// metadata, without requiring `A`, `Encoded` or `E` to be cloneable.
pub struct DomainCodec<
    A: Send + 'static,
    Encoded: Send + 'static,
    E: Send + 'static,
    D: Send + Sync + 'static,
    R: Send + Sync + 'static,
> {
    ast: SchemaAst,
    decode: Direction<Encoded, A, E, D>,
    encode: Direction<A, Encoded, E, R>,
    validate: Direction<A, A, E, R>,
}
impl<
    A: Send + 'static,
    Encoded: Send + 'static,
    E: Send + 'static,
    D: Send + Sync + 'static,
    R: Send + Sync + 'static,
> Clone for DomainCodec<A, Encoded, E, D, R>
{
    fn clone(&self) -> Self {
        Self {
            ast: self.ast.clone(),
            decode: Arc::clone(&self.decode),
            encode: Arc::clone(&self.encode),
            validate: Arc::clone(&self.validate),
        }
    }
}
impl<
    A: Send + 'static,
    Encoded: Send + 'static,
    E: Send + 'static,
    D: Send + Sync + 'static,
    R: Send + Sync + 'static,
> DomainCodec<A, Encoded, E, D, R>
{
    /// Describe three owned directions and their metadata without executing them.
    ///
    /// `validate` must return the validated typed value. The caller is responsible for
    /// making the supplied AST and callbacks agree; metadata alone enforces no constraint.
    #[must_use]
    pub fn new(
        ast: SchemaAst,
        decode: impl Fn(Encoded) -> Effect<A, E, D> + Send + Sync + 'static,
        encode: impl Fn(A) -> Effect<Encoded, E, R> + Send + Sync + 'static,
        validate: impl Fn(A) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            ast,
            decode: Arc::new(decode),
            encode: Arc::new(encode),
            validate: Arc::new(validate),
        }
    }
    /// Map schema failures into E while preserving defects, interruption and cleanup outcomes.
    ///
    /// Decode uses `decode_issue`; both encode and type validation use `encode_issue`.
    /// This mapping does not acquire capabilities from the opposite direction.
    ///
    /// ```
    /// use effect::{DomainCodec, Effect, JsonValue, json_i64};
    /// let codec = DomainCodec::from_schema(
    ///     json_i64().into_effect::<(), ()>(),
    ///     |issue| format!("invalid input: {issue:?}"),
    ///     |issue| format!("invalid output: {issue:?}"),
    /// );
    /// let decode: Effect<i64, String> = codec.decode_once(JsonValue::from(3));
    /// // Execute decode with the application's selected environment and driver.
    /// let _ = decode;
    /// ```
    #[must_use]
    pub fn from_schema(
        codec: Codec<A, Encoded, D, R>,
        decode_issue: impl Fn(SchemaIssue) -> E + Send + Sync + 'static,
        encode_issue: impl Fn(SchemaIssue) -> E + Send + Sync + 'static,
    ) -> Self {
        let decode = codec.clone();
        let validate = codec.clone();
        let encode_issue = Arc::new(encode_issue);
        let validation_issue = Arc::clone(&encode_issue);
        let decode_issue = Arc::new(decode_issue);
        Self {
            ast: codec.ast().clone(),
            decode: Arc::new(move |input| {
                decode.decode_once(input).map_error({
                    let issue = Arc::clone(&decode_issue);
                    move |error| issue(error)
                })
            }),
            encode: Arc::new(move |input| {
                codec.encode_once(input).map_error({
                    let issue = Arc::clone(&encode_issue);
                    move |error| issue(error)
                })
            }),
            validate: Arc::new(move |input| {
                validate.validate_once(input).map_error({
                    let issue = Arc::clone(&validation_issue);
                    move |error| issue(error)
                })
            }),
        }
    }
    /// Borrow the immutable description used for introspection.
    #[must_use]
    pub fn ast(&self) -> &SchemaAst {
        &self.ast
    }
    /// Lazily consume one encoded value and decode it in `D`; the input is not replayable.
    pub fn decode_once(&self, input: Encoded) -> Effect<A, E, D> {
        let decode = Arc::clone(&self.decode);
        Effect::from_result_once(Ok(input)).flat_map(move |input| decode(input))
    }
    /// Lazily consume one application value and encode it in `R`; the input is not cloned.
    pub fn encode_once(&self, input: A) -> Effect<Encoded, E, R> {
        let encode = Arc::clone(&self.encode);
        Effect::from_result_once(Ok(input)).flat_map(move |input| encode(input))
    }
    /// Lazily consume and validate one typed value in `R`, returning it on success.
    pub fn validate_once(&self, input: A) -> Effect<A, E, R> {
        let validate = Arc::clone(&self.validate);
        Effect::from_result_once(Ok(input)).flat_map(move |input| validate(input))
    }
}

impl<
    A: Send + 'static,
    Encoded: Send + 'static,
    E: Send + 'static,
    D: Send + Sync + 'static,
    R: Send + Sync + 'static,
> DomainCodec<A, Encoded, E, D, R>
{
    /// Lift all directions to `Option` without calling a payload callback for `None`.
    ///
    /// Absence is represented by `Option<Encoded>` and remains distinct from an encoded null.
    #[must_use]
    pub fn optional(self) -> DomainCodec<Option<A>, Option<Encoded>, E, D, R> {
        let decode = self.clone();
        let validate = self.clone();
        DomainCodec::new(
            SchemaAst::opaque(),
            move |input| match input {
                Some(value) => decode.decode_once(value).map(Some),
                None => Effect::from_result_once(Ok(None)),
            },
            move |input| match input {
                Some(value) => self.encode_once(value).map(Some),
                None => Effect::from_result_once(Ok(None)),
            },
            move |input| match input {
                Some(value) => validate.validate_once(value).map(Some),
                None => Effect::from_result_once(Ok(None)),
            },
        )
    }
    /// Represent `None` using the caller's null predicate and null factory.
    ///
    /// Decode calls `is_null` before the payload decoder. Encode calls `null` only for
    /// `None`; validation of `None` requires no encoding or decoding replay.
    pub fn nullable(
        self,
        is_null: impl Fn(&Encoded) -> bool + Send + Sync + 'static,
        null: impl Fn() -> Encoded + Send + Sync + 'static,
    ) -> DomainCodec<Option<A>, Encoded, E, D, R> {
        let decode = self.clone();
        let validate = self.clone();
        DomainCodec::new(
            SchemaAst::opaque(),
            move |input| {
                if is_null(&input) {
                    Effect::from_result_once(Ok(None))
                } else {
                    decode.decode_once(input).map(Some)
                }
            },
            move |input| match input {
                Some(value) => self.encode_once(value),
                None => Effect::from_result_once(Ok(null())),
            },
            move |input| match input {
                Some(value) => validate.validate_once(value).map(Some),
                None => Effect::from_result_once(Ok(None)),
            },
        )
    }
    /// Compose an owned sequence through an explicit [`super::SequenceRepresentation`].
    ///
    /// Checks `limits.collection` before encoding or validating elements; the adapter
    /// checks its representation bounds. Element effects execute sequentially and
    /// stop on failure. Representation failures use the direction's issue mapping.
    pub fn sequence<Repr: super::SequenceRepresentation<Encoded>>(
        self,
        representation: Repr,
        limits: super::ParseLimits,
        decode_issue: impl Fn(SchemaIssue) -> E + Send + Sync + 'static,
        encode_issue: impl Fn(SchemaIssue) -> E + Send + Sync + 'static,
    ) -> DomainCodec<Vec<A>, Encoded, E, D, R> {
        let repr = Arc::new(representation);
        let read = Arc::clone(&repr);
        let decode = self.clone();
        let validate = self.clone();
        let decode_issue = Arc::new(decode_issue);
        let encode_issue = Arc::new(encode_issue);
        let validation_issue = Arc::clone(&encode_issue);
        DomainCodec::new(
            SchemaAst::new(super::SchemaShape::Sequence {
                element: self.ast.clone(),
                maximum: Some(limits.collection),
            }),
            move |input| {
                let read = Arc::clone(&read);
                let issue = Arc::clone(&decode_issue);
                let decode = decode.clone();
                Effect::try_sync_once(
                    move || read.read(input, limits).map_err(|error| issue(error)),
                    std::convert::identity,
                )
                .flat_map(move |items| {
                    Effect::all(
                        items
                            .into_iter()
                            .map(|item| decode.decode_once(item))
                            .collect(),
                    )
                })
            },
            move |items: Vec<A>| {
                if items.len() > limits.collection.get() {
                    return Effect::from_result_once(Err(encode_issue(super::SchemaIssue::new(
                        super::SchemaIssueKind::LimitExceeded {
                            resource: super::SchemaResource::Width,
                        },
                    ))));
                }
                let repr = Arc::clone(&repr);
                let issue = Arc::clone(&encode_issue);
                Effect::all(
                    items
                        .into_iter()
                        .map(|item| self.encode_once(item))
                        .collect(),
                )
                .flat_map(move |items| {
                    let repr = Arc::clone(&repr);
                    let issue = Arc::clone(&issue);
                    Effect::try_sync_once(
                        move || repr.write(items, limits).map_err(|error| issue(error)),
                        std::convert::identity,
                    )
                })
            },
            move |items: Vec<A>| {
                if items.len() > limits.collection.get() {
                    return Effect::from_result_once(Err(validation_issue(
                        super::SchemaIssue::new(super::SchemaIssueKind::LimitExceeded {
                            resource: super::SchemaResource::Width,
                        }),
                    )));
                }
                Effect::all(
                    items
                        .into_iter()
                        .map(|item| validate.validate_once(item))
                        .collect(),
                )
            },
        )
    }
    /// Compose string-keyed owned values through an explicit [`super::ObjectRepresentation`].
    ///
    /// Decode rejects duplicate names and excessive width before element effects.
    /// Encode and validation check width and stop on the first failing element.
    /// Representation failures are mapped separately from application payload errors.
    pub fn map<Repr: super::ObjectRepresentation<Encoded>>(
        self,
        representation: Repr,
        limits: super::ParseLimits,
        decode_issue: impl Fn(SchemaIssue) -> E + Send + Sync + 'static,
        encode_issue: impl Fn(SchemaIssue) -> E + Send + Sync + 'static,
    ) -> DomainCodec<std::collections::BTreeMap<String, A>, Encoded, E, D, R> {
        let repr = Arc::new(representation);
        let read = Arc::clone(&repr);
        let decode = self.clone();
        let validate = self.clone();
        let decode_issue = Arc::new(decode_issue);
        let encode_issue = Arc::new(encode_issue);
        let validation_issue = Arc::clone(&encode_issue);
        DomainCodec::new(
            SchemaAst::opaque(),
            move |input| {
                let read = Arc::clone(&read);
                let issue = Arc::clone(&decode_issue);
                let decode = decode.clone();
                Effect::try_sync_once(
                    move || {
                        let fields = read.read(input, limits).map_err(|error| issue(error))?;
                        let mut seen = std::collections::BTreeSet::new();
                        if fields.len() > limits.collection.get() {
                            return Err(issue(SchemaIssue::new(
                                super::SchemaIssueKind::LimitExceeded {
                                    resource: super::SchemaResource::Width,
                                },
                            )));
                        }
                        for (name, _) in &fields {
                            if !seen.insert(name.clone()) {
                                return Err(issue(SchemaIssue::new(
                                    super::SchemaIssueKind::DuplicateField(name.as_str().into()),
                                )));
                            }
                        }
                        Ok(fields)
                    },
                    std::convert::identity,
                )
                .flat_map(move |fields| {
                    Effect::all(
                        fields
                            .into_iter()
                            .map(|(name, item)| {
                                decode
                                    .decode_once(item)
                                    .zip(Effect::from_result_once(Ok(name)))
                                    .map(|(value, name)| (name, value))
                            })
                            .collect(),
                    )
                    .map(|fields| fields.into_iter().collect())
                })
            },
            move |items: std::collections::BTreeMap<String, A>| {
                if items.len() > limits.collection.get() {
                    return Effect::from_result_once(Err(encode_issue(SchemaIssue::new(
                        super::SchemaIssueKind::LimitExceeded {
                            resource: super::SchemaResource::Width,
                        },
                    ))));
                }
                let repr = Arc::clone(&repr);
                let issue = Arc::clone(&encode_issue);
                Effect::all(
                    items
                        .into_iter()
                        .map(|(name, item)| {
                            self.encode_once(item)
                                .zip(Effect::from_result_once(Ok(name)))
                                .map(|(value, name)| (name, value))
                        })
                        .collect(),
                )
                .flat_map(move |fields| {
                    let repr = Arc::clone(&repr);
                    let issue = Arc::clone(&issue);
                    Effect::try_sync_once(
                        move || repr.write(fields, limits).map_err(|error| issue(error)),
                        std::convert::identity,
                    )
                })
            },
            move |items: std::collections::BTreeMap<String, A>| {
                if items.len() > limits.collection.get() {
                    return Effect::from_result_once(Err(validation_issue(SchemaIssue::new(
                        super::SchemaIssueKind::LimitExceeded {
                            resource: super::SchemaResource::Width,
                        },
                    ))));
                }
                Effect::all(
                    items
                        .into_iter()
                        .map(|(name, item)| {
                            validate
                                .validate_once(item)
                                .zip(Effect::from_result_once(Ok(name)))
                                .map(|(value, name)| (name, value))
                        })
                        .collect(),
                )
                .map(|fields| fields.into_iter().collect())
            },
        )
    }
}
