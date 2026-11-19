//! Owned sequence representations and their physical working-set contracts.
use super::internal::parser::PreparedArrayAssembly;
use super::{
    ParsePolicy, PreparedJsonArray, PreparedJsonKind, PreparedJsonParserPool,
    PreparedJsonValidationPool, SchemaIssue, SchemaIssueKind, SharedPreparedJsonValue, ValueKind,
};
use std::sync::Arc;
/// An owned cursor retaining actual representation backing for its whole lifetime.
pub trait PreparedSequenceCursor<Encoded>: Send + 'static {
    /// Exact number of values not yet returned.
    fn remaining(&self) -> usize;
    /// Move one value out; failure preserves the actual typed cause.
    fn take_next(&mut self) -> Result<Option<Encoded>, SchemaIssue>;
}
impl<Encoded: Send + 'static> PreparedSequenceCursor<Encoded> for PreparedJsonArray<Encoded> {
    fn remaining(&self) -> usize {
        self.as_slice().len()
    }
    fn take_next(&mut self) -> Result<Option<Encoded>, SchemaIssue> {
        Ok(self.remove_first())
    }
}
/// A prepared sequence representation. Cursor/write owners must retain actual
/// prepared backing or an explicitly bounded and separately accounted envelope;
/// implementations must not silently fall back to native Vec collection.
pub trait PreparedSequenceRepresentation<Encoded: Send + 'static>: Send + Sync + 'static {
    /// Actual owned input traversal state.
    type Cursor: PreparedSequenceCursor<Encoded>;
    /// Actual owned output construction state.
    type WriteAdmission: Send + 'static;
    /// Consume the representation and create an owned cursor.
    fn read(&self, input: Encoded, policy: ParsePolicy) -> Result<Self::Cursor, SchemaIssue>;
    /// Admit output backing for the known element count before callbacks.
    fn admit_write(
        &self,
        count: usize,
        policy: ParsePolicy,
    ) -> Result<Self::WriteAdmission, SchemaIssue>;
    /// Verify the next output placement before the child encoder runs.
    fn preflight(&self, writer: &Self::WriteAdmission) -> Result<(), SchemaIssue>;
    /// Consume one encoded child into the retained output.
    fn push(&self, writer: &mut Self::WriteAdmission, value: Encoded) -> Result<(), SchemaIssue>;
    /// Publish the actual completed representation owner.
    fn finish(&self, writer: Self::WriteAdmission) -> Result<Encoded, SchemaIssue>;
}
/// Managed JSON sequence representation over existing parser and validation pools.
pub struct PreparedJsonSequenceRepresentation {
    output: Arc<PreparedJsonParserPool>,
    validation: Arc<PreparedJsonValidationPool>,
}
impl PreparedJsonSequenceRepresentation {
    /// Select actual managed output and validation backing.
    #[must_use]
    pub fn new(
        output: Arc<PreparedJsonParserPool>,
        validation: Arc<PreparedJsonValidationPool>,
    ) -> Self {
        Self { output, validation }
    }
}
/// Owned sibling cursor retaining complete managed document backing.
pub struct PreparedJsonSequenceCursor {
    next: Option<SharedPreparedJsonValue>,
    remaining: usize,
}
impl PreparedSequenceCursor<SharedPreparedJsonValue> for PreparedJsonSequenceCursor {
    fn remaining(&self) -> usize {
        self.remaining
    }
    fn take_next(&mut self) -> Result<Option<SharedPreparedJsonValue>, SchemaIssue> {
        let Some(value) = self.next.take() else {
            return if self.remaining == 0 {
                Ok(None)
            } else {
                Err(invalid())
            };
        };
        let next = value
            .try_next_sibling()
            .map_err(super::prepared_alternatives::memory)?;
        self.remaining = self.remaining.checked_sub(1).ok_or_else(invalid)?;
        if self.remaining == 0 && next.is_some() {
            return Err(invalid());
        }
        self.next = next;
        Ok(Some(value))
    }
}
/// Opaque actual managed array document and normalized counting-writer owner.
pub struct PreparedJsonSequenceWriteAdmission {
    assembly: PreparedArrayAssembly,
    remaining: usize,
}
fn invalid() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
}
impl PreparedSequenceRepresentation<SharedPreparedJsonValue>
    for PreparedJsonSequenceRepresentation
{
    type Cursor = PreparedJsonSequenceCursor;
    type WriteAdmission = PreparedJsonSequenceWriteAdmission;
    fn read(
        &self,
        input: SharedPreparedJsonValue,
        policy: ParsePolicy,
    ) -> Result<Self::Cursor, SchemaIssue> {
        self.validation.check_parse_limits(&input, &policy.limits)?;
        if input.view().kind() != Some(PreparedJsonKind::Array) {
            return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                expected: ValueKind::Sequence,
                actual: super::prepared_view_kind(input.view())?,
            }));
        }
        let remaining = input.view().len();
        let next = input
            .try_first_child()
            .map_err(super::prepared_alternatives::memory)?;
        if (remaining == 0) != next.is_none() {
            return Err(invalid());
        }
        Ok(PreparedJsonSequenceCursor { next, remaining })
    }
    fn admit_write(
        &self,
        count: usize,
        policy: ParsePolicy,
    ) -> Result<Self::WriteAdmission, SchemaIssue> {
        let assembly = self.output.admit_array(policy)?;
        assembly.preflight_count(count)?;
        Ok(PreparedJsonSequenceWriteAdmission {
            assembly,
            remaining: count,
        })
    }
    fn preflight(&self, writer: &Self::WriteAdmission) -> Result<(), SchemaIssue> {
        if writer.remaining == 0 {
            return Err(invalid());
        }
        writer.assembly.preflight()
    }
    fn push(
        &self,
        writer: &mut Self::WriteAdmission,
        value: SharedPreparedJsonValue,
    ) -> Result<(), SchemaIssue> {
        self.preflight(writer)?;
        writer.assembly.push_value(value)?;
        writer.remaining -= 1;
        Ok(())
    }
    fn finish(&self, writer: Self::WriteAdmission) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        if writer.remaining != 0 {
            return Err(invalid());
        }
        writer.assembly.finish()
    }
}
