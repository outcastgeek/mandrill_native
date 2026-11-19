//! Borrowed equality between a managed subtree and native schema metadata.
use super::{JsonValue, PreparedJsonKind, PreparedJsonValue, SchemaIssue, SchemaIssueKind};
fn invalid() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
}
/// Compare without copying either representation or changing object ordering.
pub(crate) fn equals(
    value: PreparedJsonValue<'_>,
    literal: &JsonValue,
) -> Result<bool, SchemaIssue> {
    Ok(match (value.kind().ok_or_else(invalid)?, literal) {
        (PreparedJsonKind::Null, JsonValue::Null) => true,
        (PreparedJsonKind::Bool(a), JsonValue::Bool(b)) => a == *b,
        (PreparedJsonKind::String, JsonValue::String(b)) => {
            value.as_str().ok_or_else(invalid)? == b
        }
        (PreparedJsonKind::Number, JsonValue::Number(b)) => {
            let number = super::managed_number::ManagedNumber::parse(
                value.number_token().ok_or_else(invalid)?,
            )
            .ok_or_else(invalid)?;
            let (a, b1, c, d) = number.pieces();
            a.bytes()
                .chain(b1.bytes())
                .chain(c.bytes())
                .chain(d.bytes())
                .eq(b.as_str().bytes())
        }
        (PreparedJsonKind::Array, JsonValue::Array(items)) if value.len() == items.len() => {
            let mut child = value.first_child();
            let mut same = true;
            for item in items {
                let current = child.ok_or_else(invalid)?;
                if !equals(current, item)? {
                    same = false;
                    break;
                }
                child = current.next_sibling();
            }
            same
        }
        (PreparedJsonKind::Object, JsonValue::Object(items)) if value.len() == items.len() => {
            let mut same = true;
            for (name, item) in items {
                let Some(child) = value.get(name) else {
                    same = false;
                    break;
                };
                if !equals(child, item)? {
                    same = false;
                    break;
                }
            }
            same
        }
        _ => false,
    })
}

/// Numeric-aware equality for managed encoded values, independent of lexical
/// literal equality. Recursive traversal retains both borrowed document owners.
pub(crate) fn numeric_structural_equal(
    left: PreparedJsonValue<'_>,
    right: PreparedJsonValue<'_>,
) -> Result<bool, SchemaIssue> {
    let left_kind = left.kind().ok_or_else(invalid)?;
    let right_kind = right.kind().ok_or_else(invalid)?;
    Ok(match (left_kind, right_kind) {
        (PreparedJsonKind::Null, PreparedJsonKind::Null) => true,
        (PreparedJsonKind::Bool(a), PreparedJsonKind::Bool(b)) => a == b,
        (PreparedJsonKind::String, PreparedJsonKind::String) => {
            left.as_str().ok_or_else(invalid)? == right.as_str().ok_or_else(invalid)?
        }
        (PreparedJsonKind::Number, PreparedJsonKind::Number) => {
            let a = super::managed_number::ManagedNumber::parse(
                left.number_token().ok_or_else(invalid)?,
            )
            .ok_or_else(invalid)?;
            let b = super::managed_number::ManagedNumber::parse(
                right.number_token().ok_or_else(invalid)?,
            )
            .ok_or_else(invalid)?;
            a.compare(b)
                .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?
                == std::cmp::Ordering::Equal
        }
        (PreparedJsonKind::Array, PreparedJsonKind::Array) if left.len() == right.len() => {
            let mut a = left.first_child();
            let mut b = right.first_child();
            let mut same = true;
            while let Some(item) = a {
                let other = b.ok_or_else(invalid)?;
                if !numeric_structural_equal(item, other)? {
                    same = false;
                    break;
                }
                a = item.next_sibling();
                b = other.next_sibling();
            }
            same
        }
        (PreparedJsonKind::Object, PreparedJsonKind::Object) if left.len() == right.len() => {
            let mut child = left.first_child();
            let mut same = true;
            while let Some(item) = child {
                let name = item.property_name().ok_or_else(invalid)?;
                let Some(other) = right.get(name) else {
                    same = false;
                    break;
                };
                if !numeric_structural_equal(item, other)? {
                    same = false;
                    break;
                }
                child = item.next_sibling();
            }
            same
        }
        _ => false,
    })
}
