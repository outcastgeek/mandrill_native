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
mod prepared_codec;
mod prepared_effect_array;
mod prepared_effect_sequence;
mod prepared_sequence;
pub use prepared_sequence::{PreparedSequenceCursor, PreparedSequenceRepresentation, PreparedJsonSequenceRepresentation, PreparedJsonSequenceCursor, PreparedJsonSequenceWriteAdmission};
pub use prepared_effect_sequence::{PreparedSequenceCodec, PreparedSequenceCodecResources, PreparedSequenceOperationCapacity, PreparedSequenceOperationPool};
mod prepared_effect_map;
mod prepared_effect_record;
pub use prepared_effect_record::{Required as PreparedRecordRequired, Optional as PreparedRecordOptional, Defaulted as PreparedRecordDefaulted, ChildDefaultValidator as PreparedRecordChildDefaultValidator, ContextDefaultValidator as PreparedRecordContextDefaultValidator, PreparedOptionalRecord, PreparedDefaultRecord, PreparedContextDefaultRecord, JsonRecordBacking, RecordBacking, PreparedGenericRecordBacking, PreparedGenericRecordResources, PreparedEffectRecordWithPolicy, PreparedEffectRecordAdded, PreparedEffectRecordBuilt, PreparedEffectRecordBuilder, PreparedEffectRecordField, PreparedEffectRecordFields, PreparedEffectRecordNil, PreparedEffectRecordCodec, PreparedEffectRecordResources, PreparedRecordOperationCapacity, PreparedRecordOperationPool};
mod prepared_effect_object;
mod prepared_object;
pub use prepared_effect_object::{PreparedObjectCodec, PreparedObjectCodecResources, PreparedObjectOperationCapacity, PreparedObjectOperationPool};
pub use prepared_object::{PreparedObjectCursor, PreparedObjectRepresentation, PreparedRecordRepresentation, PreparedJsonObjectCursor, PreparedJsonObjectRepresentation, PreparedJsonObjectWriteAdmission};
pub use prepared_effect_map::{PreparedMapOperationCapacity, PreparedMapOperationPool, PreparedJsonMapCodec, PreparedJsonMapCodecResources};
pub use prepared_effect_array::{PreparedArrayOperationCapacity, PreparedArrayOperationPool, PreparedJsonArrayCodec, PreparedJsonArrayCodecResources};
mod decimal;
mod domain_codec;
mod enum_builder;
mod exact_number;
pub(crate) mod internal;
mod json_codec;
#[doc(hidden)]
pub use json_codec::prepared_view_kind;
mod json_constraints;
mod json_schema;
mod json_writer;
pub(crate) mod managed_number;
pub(crate) mod managed_literal;
mod prepared_json_writer;
mod prepared_json_array;
mod prepared_object_constraints;
pub use prepared_object_constraints::{json_prepared_object_constraints, PreparedObjectConstraints, PreparedObjectEntry, PreparedObjectValue, PreparedPatternProperty};
mod prepared_alternatives;
pub use prepared_alternatives::{json_prepared_any_of, json_prepared_one_of, json_prepared_all_of, PreparedAnyOfValue};
mod prepared_issue_accumulator;
mod prepared_exact_number;
mod prepared_numeric_scratch;
pub use prepared_exact_number::{json_prepared_exact_number, PreparedJsonExactNumber};
pub use prepared_numeric_scratch::{PreparedNumericScratchCapacity, PreparedNumericScratchDemand, PreparedNumericScratchPool};
mod prepared_pattern_cache;
mod prepared_prefix;
pub use prepared_prefix::{json_prepared_prefix_rest, PreparedPrefixElement, PreparedPrefixValue};
pub use prepared_pattern_cache::PreparedPatternCacheDemand;
mod prepared_enum;
pub use prepared_enum::PreparedTaggedEnumBuilder;
mod prepared_record;
pub use prepared_record::{PreparedRecordBuilder, PreparedRecordField, PreparedRecordFields, PreparedRecordNil, PreparedRecordIssues};
#[doc(hidden)]
pub use internal::parser::PreparedObjectAssembly;
pub use prepared_issue_accumulator::PreparedIssueAccumulator;
pub use prepared_json_array::{PreparedJsonArray, PreparedJsonArrayBuilder, PreparedJsonArrayCapacity, PreparedJsonArrayDemand, PreparedJsonArrayPool, SharedPreparedJsonArray};
mod prepared_parse_context;
mod prepared_schema_memory;
pub use prepared_schema_memory::{PreparedSchemaMemory, PreparedSchemaMemoryCapacity, PreparedSchemaMemoryDemand};
mod prepared_json_validation;
pub use internal::parser::{PreparedJsonDocument, PreparedJsonKind, PreparedJsonParserCapacity, PreparedJsonParserDemand, PreparedJsonParserPool, PreparedJsonValue, SharedPreparedJsonValue};
pub use prepared_json_validation::{PreparedJsonValidationCapacity, PreparedJsonValidationDemand, PreparedJsonValidationPool};
pub use prepared_parse_context::{PreparedParseContextCapacity, PreparedParseContextDemand, PreparedParseContextPool};
pub use prepared_json_writer::{PreparedJsonOutput, PreparedJsonWriterCapacity, PreparedJsonWriterPool, PreparedJsonWriterSinkReservation};
mod native_codec;
mod outcome_codec;
mod parse_context;
mod schema_ast;
mod schema_builder;
mod schema_issue;
pub(crate) use schema_issue::format_diagnostic_text;
pub(crate) use prepared_json_validation::physical_cause;
pub(crate) use prepared_json_validation::PreparedJsonLimitsReservation;
pub(crate) use parse_context::selected_issue_at;
mod variant_schema;

pub use codec::{Codec, EffectSchema, PureCodec};
pub use prepared_codec::{PreparedCodec, PreparedPureCodec};
pub use decimal::{Decimal, DecimalError};
pub use domain_codec::DomainCodec;
pub use enum_builder::TaggedEnumBuilder;
pub use exact_number::{JsonExactNumber, NumericConstraint, json_exact_number};
pub use json_codec::{
    json_array, json_bool, json_bytes, json_decimal, json_f64, json_i64, json_literal, json_null,
    json_nullable, json_stable_key, json_string, json_prepared_string, json_prepared_literal, json_prepared_bytes, json_prepared_nullable, json_prepared_array, json_prepared_tuple2, json_prepared_null, json_prepared_bool, json_prepared_i64, json_prepared_u64, json_prepared_f64, PreparedJsonString, json_string_pattern, json_tuple2, json_u64,
    json_utc_millis, value_kind,
};
pub use json_constraints::{
    AnyOfValue, ArrayConstraints, CheckedPattern, LiteralEnumValue, PreparedLiteralEnumValue, MultiplePatternPolicy,
    ObjectConstraints, PatternPolicy, PatternProperty, PrefixElement, PrefixValue,
    StringConstraints, UnknownLiteralPolicy, json_all_of, json_any_of, json_array_constraints, json_prepared_array_constraints,
    json_literal_enum, json_prepared_literal_enum, json_object_constraints, json_one_of, json_prefix_rest,
    json_string_constraints, json_prepared_string_constraints,
};
pub use json_schema::{
    JsonSchemaDocument, RepresentationError, RepresentationLimits, RepresentationPrecision,
    export_json_schema,
};
pub use json_writer::{JsonWriter, JsonWriterPolicy, JsonWriterSink};
pub use native_codec::{
    JsonRepresentation, NativeField, NativeFields, NativeNil, NativeRecordBuilder,
    ObjectRepresentation, SequenceRepresentation, native_array, native_map,
};
pub use outcome_codec::{
    StoredCleanup, StoredDefect, StoredFinalization, StoredOutcome, StoredPrimary,
    stored_outcome_codec,
};
pub use parse_context::{ConfiguredCodec, IssueBudget, ParseContext, SchemaPolicy, prepared_issue_at};
pub use schema_ast::{
    PrimitiveShape, SchemaAst, SchemaProperty, SchemaReference, SchemaShape, StringLengthUnit,
    StringPattern,
};
pub use schema_builder::{RecordBuilder, RecordField, RecordFields, RecordNil};
pub use schema_issue::{
    DuplicateFields, IssueSegment, ParseLimits, ParsePolicy, PreparedDiagnosticText, PreparedDiagnosticTextBuilder, PreparedDiagnosticTextError, PreparedIssueMemory, PreparedIssueMemoryCapacity, PreparedIssueMemoryDemand, SchemaErrorMode, SchemaIssue, prepared_unknown_field,
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
