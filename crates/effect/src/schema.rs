//! Pure, owned effectful and domain-error codecs with explicit representation policies.
//!
//! [`Codec`] separates decode and encode environments; providers remain in those
//! environments. [`NativeRecordBuilder`] composes owned fields without a JSON
//! bridge or payload cloning. [`DomainCodec`] maps only schema failures into an
//! application error type. [`SchemaPolicy`] gives each configured invocation
//! fresh traversal and pattern budgets, independent of JSON nesting limits.
//! Independent type validators and direct writers are separate witnesses:
//! defaults never infer a validator from an encoder, and direct writing never
//! silently falls back to ordinary encoding.
//!
//! [`PureCodec`] keeps decoding, encoding, and encoded-side structure together.
//! [`SchemaAst`] describes a representation; it does not construct an unchecked
//! application value. Parsing uses explicit byte, depth, node, collection, and
//! issue limits in [`ParsePolicy`]. Decode and encode failures carry
//! [`SchemaIssue`] paths, so callers can report which field or element failed.
//! The `schema!` macro builds typed record and tagged-enum codecs, while
//! representation export reports unsupported or lossy structures explicitly.

use crate::{Effect, Exit};
pub use serde_json::Value as JsonValue;
use std::sync::Arc;

mod codec;
mod decimal;
mod domain_codec;
mod enum_builder;
mod exact_number;
pub(crate) mod internal;
mod json_codec;
mod json_constraints;
mod json_schema;
mod json_writer;
mod native_codec;
mod outcome_codec;
mod parse_context;
mod schema_ast;
mod schema_builder;
mod schema_issue;
mod variant_schema;

pub use codec::{Codec, EffectSchema, PureCodec};
pub use decimal::{Decimal, DecimalError};
pub use domain_codec::DomainCodec;
pub use enum_builder::TaggedEnumBuilder;
pub use exact_number::{JsonExactNumber, NumericConstraint, json_exact_number};
pub use json_codec::{
    json_array, json_bool, json_bytes, json_decimal, json_f64, json_i64, json_literal, json_null,
    json_nullable, json_stable_key, json_string, json_string_pattern, json_tuple2, json_u64,
    json_utc_millis, value_kind,
};
pub use json_constraints::{
    AnyOfValue, ArrayConstraints, CheckedPattern, LiteralEnumValue, MultiplePatternPolicy,
    ObjectConstraints, PatternPolicy, PatternProperty, PrefixElement, PrefixValue,
    StringConstraints, UnknownLiteralPolicy, json_all_of, json_any_of, json_array_constraints,
    json_literal_enum, json_object_constraints, json_one_of, json_prefix_rest,
    json_string_constraints,
};
pub use json_schema::{
    JsonSchemaDocument, RepresentationError, RepresentationLimits, RepresentationPrecision,
    export_json_schema,
};
pub use json_writer::{JsonWriter, JsonWriterPolicy};
pub use native_codec::{
    JsonRepresentation, NativeField, NativeFields, NativeNil, NativeRecordBuilder,
    ObjectRepresentation, SequenceRepresentation, native_array, native_map,
};
pub use outcome_codec::{
    StoredCleanup, StoredDefect, StoredFinalization, StoredOutcome, StoredPrimary,
    stored_outcome_codec,
};
pub use parse_context::{ConfiguredCodec, IssueBudget, ParseContext, SchemaPolicy};
pub use schema_ast::{
    PrimitiveShape, SchemaAst, SchemaProperty, SchemaReference, SchemaShape, StringLengthUnit,
    StringPattern,
};
pub use schema_builder::{RecordBuilder, RecordField, RecordFields, RecordNil};
pub use schema_issue::{
    DuplicateFields, IssueSegment, ParseLimits, ParsePolicy, SchemaErrorMode, SchemaIssue,
    SchemaIssueKind, SchemaResource, UnknownFields, ValueKind,
};
pub use variant_schema::{json_option, json_result};

/// Type alias for decoder in schema.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
type Decoder<A, E> = Arc<dyn Fn(&JsonValue) -> Result<A, E> + Send + Sync>;
/// A reusable description that validates and decodes input into an owned application value.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub struct Schema<A, E> {
    decoder: Decoder<A, E>,
}
impl<A, E> Clone for Schema<A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            decoder: Arc::clone(&self.decoder),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static> Schema<A, E> {
    #[must_use]
    /// Create a schema from a decoder that returns an owned typed value.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `decode` (`impl Fn(&JsonValue) -> Result<A, E> + Send + Sync + 'static`): Callback that validates and reconstructs a typed value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_decoder(
        decode: impl Fn(&JsonValue) -> Result<A, E> + Send + Sync + 'static,
    ) -> Self {
        Self {
            decoder: Arc::new(decode),
        }
    }
    #[must_use]
    /// Create a schema whose decoder reports absence as the supplied typed error.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `decode` (`impl Fn(&JsonValue) -> Option<A> + Send + Sync + 'static`): Callback that validates and reconstructs a typed value.
    /// - `on_invalid` (`impl Fn(&JsonValue) -> E + Send + Sync + 'static`): The on invalid supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_option_decoder(
        decode: impl Fn(&JsonValue) -> Option<A> + Send + Sync + 'static,
        on_invalid: impl Fn(&JsonValue) -> E + Send + Sync + 'static,
    ) -> Self {
        Self::from_decoder(move |value| decode(value).ok_or_else(|| on_invalid(value)))
    }
    /// Validate the encoded input and reconstruct its typed value.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `value` (`Arc<JsonValue>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Effect<A, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn decode<R: Send + Sync + 'static>(&self, value: Arc<JsonValue>) -> Effect<A, E, R> {
        let schema = self.clone();
        Effect::ready(move |_, _| match (schema.decoder)(&value) {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(error),
        })
    }
    /// Parse and validate the input under this schema or syntax policy.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `text` (`impl Into<Arc<str>>`): Input text parsed or emitted by this operation.
    /// - `on_syntax_error` (`impl Fn(serde_json::Error) -> E + Send + Sync + 'static`): The on syntax error supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<A, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn parse<R: Send + Sync + 'static>(
        &self,
        text: impl Into<Arc<str>>,
        on_syntax_error: impl Fn(serde_json::Error) -> E + Send + Sync + 'static,
    ) -> Effect<A, E, R> {
        let schema = self.clone();
        let text = text.into();
        Effect::ready(move |_, _| match serde_json::from_str::<JsonValue>(&text) {
            Ok(json) => match (schema.decoder)(&json) {
                Ok(value) => Exit::Success(value),
                Err(error) => Exit::Fail(error),
            },
            Err(error) => Exit::Fail(on_syntax_error(error)),
        })
    }
}
