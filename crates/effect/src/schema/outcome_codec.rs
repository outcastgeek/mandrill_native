use super::{
    IssueSegment, JsonValue, ParseLimits, PureCodec, SchemaAst, SchemaIssue, SchemaIssueKind,
    ValueKind, value_kind,
};
use crate::{CleanupFailure, Defect, DefectKind, Exit, Finalization, PrimaryOutcome};
use serde::Deserialize;
use serde_json::Map;
use std::sync::Arc;

/// Diagnostic data saved from a defect. It is not a live panic payload or a
/// reconstructible provider exception, even when its kind is preserved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredDefect {
    kind: DefectKind,
    message: Arc<str>,
}

impl StoredDefect {
    #[must_use]
    pub fn from_defect(defect: &Defect) -> Self {
        Self {
            kind: defect.kind(),
            message: Arc::from(defect.message()),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> DefectKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StoredPrimary<E> {
    Succeeded,
    Failed(E),
    Defected(StoredDefect),
    WasInterrupted,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StoredCleanup<E> {
    Failed(E),
    Defected(StoredDefect),
    Interrupted,
}

#[derive(Debug, PartialEq, Eq)]
pub struct StoredFinalization<E> {
    pub primary: StoredPrimary<E>,
    pub first: StoredCleanup<E>,
    pub rest: Vec<StoredCleanup<E>>,
}

/// Serializable *shape* of a full terminal Exit. The A/E fields are still
/// typed application data; an OutcomeCodec supplies their actual encodings.
#[derive(Debug, PartialEq, Eq)]
pub enum StoredOutcome<A, E> {
    Success(A),
    Failure(E),
    Defect(StoredDefect),
    Interrupted,
    FinalizationFailed(StoredFinalization<E>),
}

impl<A, E> StoredOutcome<A, E> {
    #[must_use]
    pub fn from_exit(outcome: Exit<A, E>) -> Self {
        match outcome {
            Exit::Success(value) => Self::Success(value),
            Exit::Fail(error) => Self::Failure(error),
            Exit::Die(defect) => Self::Defect(StoredDefect::from_defect(&defect)),
            Exit::Interrupted => Self::Interrupted,
            Exit::FinalizationFailed(Finalization {
                primary,
                first,
                rest,
            }) => Self::FinalizationFailed(StoredFinalization {
                primary: match primary {
                    PrimaryOutcome::Succeeded => StoredPrimary::Succeeded,
                    PrimaryOutcome::Failed(error) => StoredPrimary::Failed(error),
                    PrimaryOutcome::Defected(defect) => {
                        StoredPrimary::Defected(StoredDefect::from_defect(&defect))
                    }
                    PrimaryOutcome::WasInterrupted => StoredPrimary::WasInterrupted,
                },
                first: StoredCleanup::from_failure(first),
                rest: rest.into_iter().map(StoredCleanup::from_failure).collect(),
            }),
        }
    }
}

impl<E> StoredCleanup<E> {
    fn from_failure(failure: CleanupFailure<E>) -> Self {
        match failure {
            CleanupFailure::ReleaseFailure(error) => Self::Failed(error),
            CleanupFailure::ReleaseDefect(defect) => {
                Self::Defected(StoredDefect::from_defect(&defect))
            }
            CleanupFailure::ReleaseInterrupted => Self::Interrupted,
        }
    }
}

/// Version 1 wire contract: every outcome is an object with `version` and
/// `kind`. Success/failure use `value`/`error`; defects use a nested
/// `{ kind, message }` diagnostic; finalization uses ordered `primary`,
/// `first`, and `rest`. Unknown tags, versions, and fields are rejected.
/// The protocol stores diagnostics, never a live panic or provider object.
mod wire {
    pub(super) const VERSION: u64 = 1;
    pub(super) const VERSION_FIELD: &str = "version";
    pub(super) const KIND: &str = "kind";
    pub(super) const VALUE: &str = "value";
    pub(super) const ERROR: &str = "error";
    pub(super) const DEFECT: &str = "defect";
    pub(super) const MESSAGE: &str = "message";
    pub(super) const PRIMARY: &str = "primary";
    pub(super) const FIRST: &str = "first";
    pub(super) const REST: &str = "rest";
    pub(super) const SUCCESS: &str = "success";
    pub(super) const FAILURE: &str = "failure";
    pub(super) const DEFECTED: &str = "defected";
    pub(super) const INTERRUPTED: &str = "interrupted";
    pub(super) const FINALIZATION_FAILED: &str = "finalization_failed";
    pub(super) const SUCCEEDED: &str = "succeeded";
    pub(super) const FAILED: &str = "failed";
}

fn wrong_type(expected: ValueKind, actual: &JsonValue) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::TypeMismatch {
        expected,
        actual: value_kind(actual),
    })
}

fn object(value: &JsonValue) -> Result<&Map<String, JsonValue>, SchemaIssue> {
    value
        .as_object()
        .ok_or_else(|| wrong_type(ValueKind::Record, value))
}

fn field<'a>(
    object: &'a Map<String, JsonValue>,
    name: &'static str,
) -> Result<&'a JsonValue, SchemaIssue> {
    object.get(name).ok_or_else(|| {
        SchemaIssue::new(SchemaIssueKind::MissingField(Arc::from(name)))
            .at(IssueSegment::Field(Arc::from(name)))
    })
}

fn exact_fields(object: &Map<String, JsonValue>, allowed: &[&str]) -> Result<(), SchemaIssue> {
    for name in object.keys() {
        if !allowed.contains(&name.as_str()) {
            return Err(
                SchemaIssue::new(SchemaIssueKind::UnknownField(Arc::from(name.as_str())))
                    .at(IssueSegment::Field(Arc::from(name.as_str()))),
            );
        }
    }
    Ok(())
}

fn tag(object: &Map<String, JsonValue>) -> Result<&str, SchemaIssue> {
    let value = field(object, wire::KIND)?;
    value.as_str().ok_or_else(|| {
        wrong_type(ValueKind::String, value).at(IssueSegment::Field(Arc::from(wire::KIND)))
    })
}

fn encoded_object(
    tag: &'static str,
    fields: impl IntoIterator<Item = (&'static str, JsonValue)>,
) -> JsonValue {
    let mut object = Map::new();
    object.insert(wire::KIND.into(), JsonValue::String(tag.into()));
    for (name, value) in fields {
        object.insert(name.into(), value);
    }
    JsonValue::Object(object)
}

fn defect_to_json(defect: &StoredDefect) -> Result<JsonValue, SchemaIssue> {
    let kind = serde_json::to_value(defect.kind)
        .map_err(|_| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
    Ok(JsonValue::Object(Map::from_iter([
        (wire::KIND.into(), kind),
        (
            wire::MESSAGE.into(),
            JsonValue::String(defect.message.to_string()),
        ),
    ])))
}

fn defect_from_json(value: &JsonValue) -> Result<StoredDefect, SchemaIssue> {
    let object = object(value)?;
    exact_fields(object, &[wire::KIND, wire::MESSAGE])?;
    let kind = DefectKind::deserialize(field(object, wire::KIND)?).map_err(|_| {
        SchemaIssue::new(SchemaIssueKind::InvalidTag).at(IssueSegment::Field(Arc::from(wire::KIND)))
    })?;
    let message = field(object, wire::MESSAGE)?;
    let message = message.as_str().ok_or_else(|| {
        wrong_type(ValueKind::String, message).at(IssueSegment::Field(Arc::from(wire::MESSAGE)))
    })?;
    Ok(StoredDefect {
        kind,
        message: Arc::from(message),
    })
}

fn error_to_json<E: Send + 'static>(
    value: &E,
    codec: &PureCodec<E, JsonValue>,
) -> Result<JsonValue, SchemaIssue> {
    codec
        .encode_ref(value)
        .map_err(|issue| issue.at(IssueSegment::Field(Arc::from(wire::ERROR))))
}

fn error_from_json<E: Send + 'static>(
    object: &Map<String, JsonValue>,
    codec: &PureCodec<E, JsonValue>,
) -> Result<E, SchemaIssue> {
    codec
        .decode_ref(field(object, wire::ERROR)?)
        .map_err(|issue| issue.at(IssueSegment::Field(Arc::from(wire::ERROR))))
}

fn primary_to_json<E: Send + 'static>(
    value: &StoredPrimary<E>,
    error: &PureCodec<E, JsonValue>,
) -> Result<JsonValue, SchemaIssue> {
    Ok(match value {
        StoredPrimary::Succeeded => encoded_object(wire::SUCCEEDED, []),
        StoredPrimary::Failed(value) => {
            encoded_object(wire::FAILED, [(wire::ERROR, error_to_json(value, error)?)])
        }
        StoredPrimary::Defected(value) => {
            encoded_object(wire::DEFECTED, [(wire::DEFECT, defect_to_json(value)?)])
        }
        StoredPrimary::WasInterrupted => encoded_object(wire::INTERRUPTED, []),
    })
}

fn primary_from_json<E: Send + 'static>(
    value: &JsonValue,
    error: &PureCodec<E, JsonValue>,
) -> Result<StoredPrimary<E>, SchemaIssue> {
    let object = object(value)?;
    Ok(match tag(object)? {
        wire::SUCCEEDED => {
            exact_fields(object, &[wire::KIND])?;
            StoredPrimary::Succeeded
        }
        wire::FAILED => {
            exact_fields(object, &[wire::KIND, wire::ERROR])?;
            StoredPrimary::Failed(error_from_json(object, error)?)
        }
        wire::DEFECTED => {
            exact_fields(object, &[wire::KIND, wire::DEFECT])?;
            StoredPrimary::Defected(defect_from_json(field(object, wire::DEFECT)?)?)
        }
        wire::INTERRUPTED => {
            exact_fields(object, &[wire::KIND])?;
            StoredPrimary::WasInterrupted
        }
        _ => return Err(SchemaIssue::new(SchemaIssueKind::InvalidTag)),
    })
}

fn cleanup_to_json<E: Send + 'static>(
    value: &StoredCleanup<E>,
    error: &PureCodec<E, JsonValue>,
) -> Result<JsonValue, SchemaIssue> {
    Ok(match value {
        StoredCleanup::Failed(value) => {
            encoded_object(wire::FAILED, [(wire::ERROR, error_to_json(value, error)?)])
        }
        StoredCleanup::Defected(value) => {
            encoded_object(wire::DEFECTED, [(wire::DEFECT, defect_to_json(value)?)])
        }
        StoredCleanup::Interrupted => encoded_object(wire::INTERRUPTED, []),
    })
}

fn cleanup_from_json<E: Send + 'static>(
    value: &JsonValue,
    error: &PureCodec<E, JsonValue>,
) -> Result<StoredCleanup<E>, SchemaIssue> {
    let object = object(value)?;
    Ok(match tag(object)? {
        wire::FAILED => {
            exact_fields(object, &[wire::KIND, wire::ERROR])?;
            StoredCleanup::Failed(error_from_json(object, error)?)
        }
        wire::DEFECTED => {
            exact_fields(object, &[wire::KIND, wire::DEFECT])?;
            StoredCleanup::Defected(defect_from_json(field(object, wire::DEFECT)?)?)
        }
        wire::INTERRUPTED => {
            exact_fields(object, &[wire::KIND])?;
            StoredCleanup::Interrupted
        }
        _ => return Err(SchemaIssue::new(SchemaIssueKind::InvalidTag)),
    })
}

/// Build a borrowed-input codec for complete, versioned Exit data. A and E
/// are encoded through their own typed codecs, without blanket Clone bounds.
#[must_use]
pub fn stored_outcome_codec<A: Send + 'static, E: Send + 'static>(
    success: PureCodec<A, JsonValue>,
    error: PureCodec<E, JsonValue>,
    limits: ParseLimits,
) -> PureCodec<StoredOutcome<A, E>, JsonValue> {
    let decode_success = success.clone();
    let decode_error = error.clone();
    PureCodec::with_ast(
        SchemaAst::opaque(),
        move |encoded| {
            super::internal::parser::check_limits(encoded, &limits)?;
            let object = object(encoded)?;
            let version = field(object, wire::VERSION_FIELD)?;
            let actual = version.as_u64();
            if actual != Some(wire::VERSION) {
                return Err(SchemaIssue::new(SchemaIssueKind::VersionMismatch {
                    expected: wire::VERSION,
                    actual,
                }));
            }
            match tag(object)? {
                wire::SUCCESS => {
                    exact_fields(object, &[wire::VERSION_FIELD, wire::KIND, wire::VALUE])?;
                    let value = decode_success
                        .decode_ref(field(object, wire::VALUE)?)
                        .map_err(|issue| issue.at(IssueSegment::Field(Arc::from(wire::VALUE))))?;
                    Ok(StoredOutcome::Success(value))
                }
                wire::FAILURE => {
                    exact_fields(object, &[wire::VERSION_FIELD, wire::KIND, wire::ERROR])?;
                    Ok(StoredOutcome::Failure(error_from_json(
                        object,
                        &decode_error,
                    )?))
                }
                wire::DEFECTED => {
                    exact_fields(object, &[wire::VERSION_FIELD, wire::KIND, wire::DEFECT])?;
                    Ok(StoredOutcome::Defect(defect_from_json(field(
                        object,
                        wire::DEFECT,
                    )?)?))
                }
                wire::INTERRUPTED => {
                    exact_fields(object, &[wire::VERSION_FIELD, wire::KIND])?;
                    Ok(StoredOutcome::Interrupted)
                }
                wire::FINALIZATION_FAILED => {
                    exact_fields(
                        object,
                        &[
                            wire::VERSION_FIELD,
                            wire::KIND,
                            wire::PRIMARY,
                            wire::FIRST,
                            wire::REST,
                        ],
                    )?;
                    let primary = primary_from_json(field(object, wire::PRIMARY)?, &decode_error)
                        .map_err(|issue| {
                        issue.at(IssueSegment::Field(Arc::from(wire::PRIMARY)))
                    })?;
                    let first = cleanup_from_json(field(object, wire::FIRST)?, &decode_error)
                        .map_err(|issue| issue.at(IssueSegment::Field(Arc::from(wire::FIRST))))?;
                    let rest = field(object, wire::REST)?;
                    let JsonValue::Array(rest) = rest else {
                        return Err(wrong_type(ValueKind::Sequence, rest)
                            .at(IssueSegment::Field(Arc::from(wire::REST))));
                    };
                    let rest = rest
                        .iter()
                        .enumerate()
                        .map(|(index, value)| {
                            cleanup_from_json(value, &decode_error).map_err(|issue| {
                                issue
                                    .at(IssueSegment::Index(index))
                                    .at(IssueSegment::Field(Arc::from(wire::REST)))
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(StoredOutcome::FinalizationFailed(StoredFinalization {
                        primary,
                        first,
                        rest,
                    }))
                }
                _ => Err(SchemaIssue::new(SchemaIssueKind::InvalidTag)),
            }
        },
        move |outcome| {
            let mut encoded = match outcome {
                StoredOutcome::Success(value) => encoded_object(
                    wire::SUCCESS,
                    [(
                        wire::VALUE,
                        success.encode_ref(value).map_err(|issue| {
                            issue.at(IssueSegment::Field(Arc::from(wire::VALUE)))
                        })?,
                    )],
                ),
                StoredOutcome::Failure(value) => encoded_object(
                    wire::FAILURE,
                    [(wire::ERROR, error_to_json(value, &error)?)],
                ),
                StoredOutcome::Defect(value) => {
                    encoded_object(wire::DEFECTED, [(wire::DEFECT, defect_to_json(value)?)])
                }
                StoredOutcome::Interrupted => encoded_object(wire::INTERRUPTED, []),
                StoredOutcome::FinalizationFailed(details) => {
                    let primary = primary_to_json(&details.primary, &error)
                        .map_err(|issue| issue.at(IssueSegment::Field(Arc::from(wire::PRIMARY))))?;
                    let first = cleanup_to_json(&details.first, &error)
                        .map_err(|issue| issue.at(IssueSegment::Field(Arc::from(wire::FIRST))))?;
                    let rest = details
                        .rest
                        .iter()
                        .enumerate()
                        .map(|(index, value)| {
                            cleanup_to_json(value, &error).map_err(|issue| {
                                issue
                                    .at(IssueSegment::Index(index))
                                    .at(IssueSegment::Field(Arc::from(wire::REST)))
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    encoded_object(
                        wire::FINALIZATION_FAILED,
                        [
                            (wire::PRIMARY, primary),
                            (wire::FIRST, first),
                            (wire::REST, JsonValue::Array(rest)),
                        ],
                    )
                }
            };
            if let JsonValue::Object(ref mut object) = encoded {
                object.insert(
                    wire::VERSION_FIELD.into(),
                    JsonValue::Number(wire::VERSION.into()),
                );
            }
            super::internal::parser::check_limits(&encoded, &limits)?;
            Ok(encoded)
        },
    )
}
