//! Exact decimal tokens. Base ten is the JSON number radix; exponents are
//! arbitrary integers so comparison never expands a potentially huge power.
use super::{
    JsonValue, PrimitiveShape, PureCodec, SchemaAst, SchemaIssue, SchemaIssueKind, SchemaResource,
    SchemaShape,
};
use num_bigint::{BigInt, BigUint, Sign};
use std::cmp::Ordering;
use std::num::NonZeroUsize;
use std::sync::Arc;
/// A JSON decimal token with exact numeric comparison and arbitrary-size exponent.
///
/// Stores a normalized token and decimal coefficient/exponent, never an `f64`.
/// Numeric equality treats equivalent spellings and signed zero as equal;
/// [`Self::as_str`] returns the stored JSON token, not an expanded integer.
#[derive(Clone, Debug)]
pub struct JsonExactNumber {
    text: Arc<str>,
    negative: bool,
    digits: String,
    exponent: BigInt,
}
impl JsonExactNumber {
    /// Parse a JSON number after checking its ASCII digit count.
    ///
    /// `digits_limit` counts coefficient, fractional and exponent digits. Invalid
    /// syntax yields `OutOfRange`; excess digits yield `LimitExceeded(IntegerDigits)`.
    /// Huge exponents remain compact rather than allocating their expanded power.
    pub fn parse(text: &str, digits_limit: NonZeroUsize) -> Result<Self, SchemaIssue> {
        if text.bytes().filter(u8::is_ascii_digit).count() > digits_limit.get() {
            return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: SchemaResource::IntegerDigits,
            }));
        }
        let number: serde_json::Number = text
            .parse()
            .map_err(|_| SchemaIssue::new(SchemaIssueKind::OutOfRange))?;
        let text = number.to_string();
        let unsigned = text.strip_prefix('-').unwrap_or(&text);
        let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
        let exponent: BigInt = exponent
            .parse()
            .map_err(|_| SchemaIssue::new(SchemaIssueKind::OutOfRange))?;
        let fraction = mantissa
            .split_once('.')
            .map_or(0, |(_, fraction)| fraction.len());
        let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
        let digits = digits.trim_start_matches('0');
        if digits.is_empty() {
            return Ok(Self {
                text: text.into(),
                negative: false,
                digits: "0".into(),
                exponent: BigInt::from(0),
            });
        }
        let trimmed = digits.trim_end_matches('0');
        let exponent =
            exponent - BigInt::from(fraction) + BigInt::from(digits.len() - trimmed.len());
        let negative = text.starts_with('-');
        let digits = trimmed.to_owned();
        Ok(Self {
            text: text.into(),
            negative,
            digits,
            exponent,
        })
    }
    /// Borrow the normalized JSON token retained at construction.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
    /// Compare exact mathematical values without expanding powers of ten or rounding through floating point.
    #[must_use]
    pub fn compare(&self, other: &Self) -> Ordering {
        let sign = |value: &Self| {
            if value.digits == "0" {
                0
            } else if value.negative {
                -1
            } else {
                1
            }
        };
        let ordering = sign(self).cmp(&sign(other));
        if ordering != Ordering::Equal || self.digits == "0" {
            return ordering;
        }
        let magnitude = (&self.exponent + BigInt::from(self.digits.len()))
            .cmp(&(&other.exponent + BigInt::from(other.digits.len())));
        let ordering = if magnitude == Ordering::Equal {
            let len = self.digits.len().max(other.digits.len());
            self.digits
                .bytes()
                .chain(std::iter::repeat(b'0'))
                .take(len)
                .cmp(
                    other
                        .digits
                        .bytes()
                        .chain(std::iter::repeat(b'0'))
                        .take(len),
                )
        } else {
            magnitude
        };
        if self.negative {
            ordering.reverse()
        } else {
            ordering
        }
    }
    /// Whether the exact value has no fractional part, including zero with any exponent.
    #[must_use]
    pub fn is_integer(&self) -> bool {
        self.digits == "0" || self.exponent.sign() != Sign::Minus
    }
    /// Materialize an integral value only when its expanded decimal width fits `maximum_digits`.
    ///
    /// Fractions or an exponent outside the supported power representation yield
    /// `OutOfRange`; oversized expansion yields `LimitExceeded(IntegerDigits)`.
    pub fn to_big_integer(&self, maximum_digits: NonZeroUsize) -> Result<BigInt, SchemaIssue> {
        if !self.is_integer() {
            return Err(SchemaIssue::new(SchemaIssueKind::OutOfRange));
        }
        if self.digits == "0" {
            return Ok(BigInt::from(0));
        }
        let required = BigInt::from(self.digits.len()) + &self.exponent;
        if required > BigInt::from(maximum_digits.get()) {
            return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: SchemaResource::IntegerDigits,
            }));
        }
        let exponent = u32::try_from(&self.exponent)
            .map_err(|_| SchemaIssue::new(SchemaIssueKind::OutOfRange))?;
        let coefficient: BigInt = self
            .digits
            .parse()
            .map_err(|_| SchemaIssue::new(SchemaIssueKind::OutOfRange))?;
        let value = coefficient * BigInt::from(10).pow(exponent);
        Ok(if self.negative { -value } else { value })
    }
    /// Check exact decimal divisibility by a strictly positive divisor.
    ///
    /// Zero or negative divisors are invalid definitions. Uses modular arithmetic
    /// where possible so a large positive exponent need not be expanded.
    pub fn multiple_of(&self, divisor: &Self) -> Result<bool, SchemaIssue> {
        if divisor.digits == "0" || divisor.negative {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        if self.digits == "0" {
            return Ok(true);
        }
        let numerator: BigUint = self
            .digits
            .parse()
            .map_err(|_| SchemaIssue::new(SchemaIssueKind::OutOfRange))?;
        let denominator: BigUint = divisor
            .digits
            .parse()
            .map_err(|_| SchemaIssue::new(SchemaIssueKind::OutOfRange))?;
        let scale = &self.exponent - &divisor.exponent;
        if scale.sign() != Sign::Minus {
            let scale = scale
                .to_biguint()
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))?;
            Ok(
                (numerator * BigUint::from(10u8).modpow(&scale, &denominator)) % denominator
                    == BigUint::from(0u8),
            )
        } else if BigInt::from(divisor.digits.len()) - &scale > BigInt::from(self.digits.len()) {
            Ok(false)
        } else {
            let exponent =
                u32::try_from(-scale).map_err(|_| SchemaIssue::new(SchemaIssueKind::OutOfRange))?;
            Ok(numerator % (denominator * BigUint::from(10u8).pow(exponent)) == BigUint::from(0u8))
        }
    }
}
impl PartialEq for JsonExactNumber {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other) == Ordering::Equal
    }
}
impl Eq for JsonExactNumber {}
/// Exact numeric predicates applied by [`json_exact_number`] in declaration order.
#[derive(Clone, Debug)]
pub enum NumericConstraint {
    /// Inclusive lower bound, compared without floating-point conversion.
    Minimum(JsonExactNumber),
    /// Inclusive upper bound.
    Maximum(JsonExactNumber),
    /// Strict lower bound.
    ExclusiveMinimum(JsonExactNumber),
    /// Strict upper bound.
    ExclusiveMaximum(JsonExactNumber),
    /// Exact divisibility by a strictly positive decimal value.
    MultipleOf(JsonExactNumber),
    /// Require an exact integral value, independent of its token spelling.
    Integer,
}
fn check(value: &JsonExactNumber, constraints: &[NumericConstraint]) -> Result<(), SchemaIssue> {
    for constraint in constraints {
        let (valid, name, operand) = match constraint {
            NumericConstraint::Minimum(bound) => (
                value.compare(bound) != Ordering::Less,
                "minimum",
                bound.as_str(),
            ),
            NumericConstraint::Maximum(bound) => (
                value.compare(bound) != Ordering::Greater,
                "maximum",
                bound.as_str(),
            ),
            NumericConstraint::ExclusiveMinimum(bound) => (
                value.compare(bound) == Ordering::Greater,
                "exclusiveMinimum",
                bound.as_str(),
            ),
            NumericConstraint::ExclusiveMaximum(bound) => (
                value.compare(bound) == Ordering::Less,
                "exclusiveMaximum",
                bound.as_str(),
            ),
            NumericConstraint::MultipleOf(bound) => {
                (value.multiple_of(bound)?, "multipleOf", bound.as_str())
            }
            NumericConstraint::Integer => (value.is_integer(), "integer", "integer"),
        };
        if !valid {
            return Err(SchemaIssue::new(SchemaIssueKind::Constraint {
                name: name.into(),
                operand: operand.into(),
            }));
        }
    }
    Ok(())
}
/// Build a number codec with independent type validation, direct writing and exact equality.
///
/// Rejects nonpositive `MultipleOf` operands eagerly. The digit bound applies to
/// decode and encode; configured invocations use their active numeric bound.
/// Predicate failures carry `Constraint` details. Its checked AST exports a safe
/// approximation rather than claiming exact floating-point bounds.
///
/// ```
/// use effect::{JsonExactNumber, NumericConstraint, json_exact_number};
/// use std::num::NonZeroUsize;
/// let digits = NonZeroUsize::new(32).unwrap();
/// let codec = json_exact_number(vec![NumericConstraint::Integer], digits)?;
/// let value = JsonExactNumber::parse("9007199254740993", digits)?;
/// codec.validate_ref(&value)?;
/// assert_eq!(codec.encode_ref(&value)?.to_string(), "9007199254740993");
/// # Ok::<(), effect::SchemaIssue>(())
/// ```
pub fn json_exact_number(
    constraints: Vec<NumericConstraint>,
    digits_limit: NonZeroUsize,
) -> Result<PureCodec<JsonExactNumber, JsonValue>, SchemaIssue> {
    for constraint in &constraints {
        if let NumericConstraint::MultipleOf(divisor) = constraint
            && (divisor.negative || divisor.digits == "0")
        {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
    }
    let constraints: Arc<[NumericConstraint]> = constraints.into();
    let decode = Arc::clone(&constraints);
    let validate = Arc::clone(&constraints);
    let writer_constraints = Arc::clone(&constraints);
    // Checked means export is safely approximate rather than claiming f64 bounds.
    Ok(PureCodec::with_context_ast(
        SchemaAst::new(SchemaShape::Checked {
            source: SchemaAst::primitive(PrimitiveShape::Number),
            name: Arc::from("exact_json_number"),
        }),
        move |value, context| {
            let digits_limit =
                context.map_or(digits_limit, |context| context.policy().integer_digits);
            let JsonValue::Number(number) = value else {
                return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                    expected: super::ValueKind::Number,
                    actual: super::value_kind(value),
                }));
            };
            let value = JsonExactNumber::parse(&number.to_string(), digits_limit)?;
            check(&value, &decode)?;
            Ok(value)
        },
        move |value: &JsonExactNumber, context| {
            let digits_limit =
                context.map_or(digits_limit, |context| context.policy().integer_digits);
            check(value, &constraints)?;
            let parsed = JsonExactNumber::parse(value.as_str(), digits_limit)?;
            parsed
                .as_str()
                .parse()
                .map(JsonValue::Number)
                .map_err(|_| SchemaIssue::new(SchemaIssueKind::OutOfRange))
        },
    )
    .with_context_type_validator(move |value, context| {
        let digits_limit = context.map_or(digits_limit, |context| context.policy().integer_digits);
        JsonExactNumber::parse(value.as_str(), digits_limit)?;
        check(value, &validate)
    })
    .with_equivalence(|left, right| left.compare(right) == Ordering::Equal)
    .with_writer(move |value, writer| {
        JsonExactNumber::parse(value.as_str(), digits_limit)?;
        check(value, &writer_constraints)?;
        writer.number(value.as_str())
    }))
}
