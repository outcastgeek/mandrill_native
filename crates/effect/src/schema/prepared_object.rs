//! Owned object representations with actual selected backing.
use super::internal::parser::PreparedObjectAssembly;
use super::{
    ParsePolicy, PreparedJsonKind, PreparedJsonParserPool, PreparedJsonString,
    PreparedJsonValidationPool, PreparedObjectValue, SchemaIssue, SchemaIssueKind,
    SharedPreparedJsonValue, ValueKind,
};
use std::sync::Arc;
/// An owned cursor with an exact remaining field count.
pub trait PreparedObjectCursor<Encoded>: Send + 'static {
    /// Exact fields not yet returned.
    fn remaining(&self) -> usize;
    /// Move one original name and value out of the actual owner.
    fn take_next(&mut self) -> Result<Option<(PreparedJsonString, Encoded)>, SchemaIssue>;
}
impl<Encoded: Send + 'static> PreparedObjectCursor<Encoded> for PreparedObjectValue<Encoded> {
    fn remaining(&self) -> usize {
        self.entries().len()
    }
    fn take_next(&mut self) -> Result<Option<(PreparedJsonString, Encoded)>, SchemaIssue> {
        Ok(self
            .remove_first()
            .map(super::PreparedObjectEntry::into_parts))
    }
}
/// An object representation retaining actual prepared or explicitly bounded backing.
/// Implementations must not silently delegate to the ordinary Vec-returning representation.
pub trait PreparedObjectRepresentation<Encoded: Send + 'static>: Send + Sync + 'static {
    /// Actual owned field cursor.
    type Cursor: PreparedObjectCursor<Encoded>;
    /// Actual owned output construction state.
    type WriteAdmission: Send + 'static;
    /// Consume the input and retain its field backing.
    fn read(&self, input: Encoded, policy: ParsePolicy) -> Result<Self::Cursor, SchemaIssue>;
    /// Consume borrowed original names synchronously and admit actual output backing.
    fn admit_write<'a>(
        &self,
        names: impl Iterator<Item = &'a str>,
        policy: ParsePolicy,
    ) -> Result<Self::WriteAdmission, SchemaIssue>;
    /// Verify one placement before its child encoder runs.
    fn preflight(&self, writer: &Self::WriteAdmission, name: &str) -> Result<(), SchemaIssue>;
    /// Move one encoded child into its actual output owner.
    fn push(
        &self,
        writer: &mut Self::WriteAdmission,
        name: &str,
        value: Encoded,
    ) -> Result<(), SchemaIssue>;
    /// Publish the completed owned representation.
    fn finish(&self, writer: Self::WriteAdmission) -> Result<Encoded, SchemaIssue>;
}
/// Record output admission with guaranteed names and a finite field maximum.
///
/// The returned owner must be the actual representation backing. Conditional
/// names are preflighted only when emitted; the maximum is an upper bound and
/// does not require every possible field to be present. Providers must account
/// for their actual backing or a separately bounded native envelope.
pub trait PreparedRecordRepresentation<Encoded: Send + 'static>:
    PreparedObjectRepresentation<Encoded>
{
    /// Admit output before record callbacks, checking guaranteed names only.
    fn admit_record_write<'a>(
        &self,
        guaranteed: impl Iterator<Item = &'a str>,
        maximum_fields: usize,
        policy: ParsePolicy,
    ) -> Result<Self::WriteAdmission, SchemaIssue>;
    /// Publish an output containing at most the admitted maximum fields.
    fn finish_record(&self, writer: Self::WriteAdmission) -> Result<Encoded, SchemaIssue>;
}
/// Managed JSON object representation with separately selected name documents.
pub struct PreparedJsonObjectRepresentation {
    names: Arc<PreparedJsonParserPool>,
    output: Arc<PreparedJsonParserPool>,
    validation: Arc<PreparedJsonValidationPool>,
}
impl PreparedJsonObjectRepresentation {
    /// Select the actual name, output and validation backing.
    #[must_use]
    pub fn new(
        names: Arc<PreparedJsonParserPool>,
        output: Arc<PreparedJsonParserPool>,
        validation: Arc<PreparedJsonValidationPool>,
    ) -> Self {
        Self {
            names,
            output,
            validation,
        }
    }
}
/// Cursor over retained managed sibling owners.
pub struct PreparedJsonObjectCursor {
    next: Option<SharedPreparedJsonValue>,
    remaining: usize,
    names: Arc<PreparedJsonParserPool>,
    policy: ParsePolicy,
}
fn invalid() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
}
impl PreparedObjectCursor<SharedPreparedJsonValue> for PreparedJsonObjectCursor {
    fn remaining(&self) -> usize {
        self.remaining
    }
    fn take_next(
        &mut self,
    ) -> Result<Option<(PreparedJsonString, SharedPreparedJsonValue)>, SchemaIssue> {
        let Some(value) = self.next.take() else {
            return if self.remaining == 0 {
                Ok(None)
            } else {
                Err(invalid())
            };
        };
        let name = PreparedJsonString::copy_from(
            value.view().property_name().ok_or_else(invalid)?,
            &self.names,
            self.policy,
        )?;
        let next = value
            .try_next_sibling()
            .map_err(super::prepared_alternatives::memory)?;
        self.remaining = self.remaining.checked_sub(1).ok_or_else(invalid)?;
        if self.remaining == 0 && next.is_some() {
            return Err(invalid());
        }
        self.next = next;
        Ok(Some((name, value)))
    }
}
/// Opaque actual managed object and counting-writer owner.
pub struct PreparedJsonObjectWriteAdmission {
    assembly: PreparedObjectAssembly,
    remaining: usize,
    exact: bool,
}
impl PreparedObjectRepresentation<SharedPreparedJsonValue> for PreparedJsonObjectRepresentation {
    type Cursor = PreparedJsonObjectCursor;
    type WriteAdmission = PreparedJsonObjectWriteAdmission;
    fn read(
        &self,
        input: SharedPreparedJsonValue,
        policy: ParsePolicy,
    ) -> Result<Self::Cursor, SchemaIssue> {
        self.validation.check_parse_limits(&input, &policy.limits)?;
        if input.view().kind() != Some(PreparedJsonKind::Object) {
            return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                expected: ValueKind::Record,
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
        Ok(PreparedJsonObjectCursor {
            next,
            remaining,
            names: Arc::clone(&self.names),
            policy,
        })
    }
    fn admit_write<'a>(
        &self,
        names: impl Iterator<Item = &'a str>,
        policy: ParsePolicy,
    ) -> Result<Self::WriteAdmission, SchemaIssue> {
        let assembly = self.output.admit_object(policy)?;
        let remaining = assembly.preflight_names(names)?;
        Ok(PreparedJsonObjectWriteAdmission {
            assembly,
            remaining,
            exact: true,
        })
    }
    fn preflight(&self, writer: &Self::WriteAdmission, name: &str) -> Result<(), SchemaIssue> {
        if writer.remaining == 0 {
            return Err(invalid());
        }
        writer.assembly.preflight(name)
    }
    fn push(
        &self,
        writer: &mut Self::WriteAdmission,
        name: &str,
        value: SharedPreparedJsonValue,
    ) -> Result<(), SchemaIssue> {
        self.preflight(writer, name)?;
        writer.assembly.insert_value(name, value)?;
        writer.remaining -= 1;
        Ok(())
    }
    fn finish(&self, writer: Self::WriteAdmission) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        if !writer.exact || writer.remaining != 0 {
            return Err(invalid());
        }
        writer.assembly.finish()
    }
}
impl PreparedRecordRepresentation<SharedPreparedJsonValue> for PreparedJsonObjectRepresentation {
    fn admit_record_write<'a>(
        &self,
        guaranteed: impl Iterator<Item = &'a str>,
        maximum_fields: usize,
        policy: ParsePolicy,
    ) -> Result<Self::WriteAdmission, SchemaIssue> {
        let assembly = self.output.admit_object(policy)?;
        let guaranteed_count = assembly.preflight_names(guaranteed)?;
        if guaranteed_count > maximum_fields {
            return Err(invalid());
        }
        Ok(PreparedJsonObjectWriteAdmission {
            assembly,
            remaining: maximum_fields,
            exact: false,
        })
    }
    fn finish_record(
        &self,
        writer: Self::WriteAdmission,
    ) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        if writer.exact {
            return Err(invalid());
        }
        writer.assembly.finish()
    }
}
