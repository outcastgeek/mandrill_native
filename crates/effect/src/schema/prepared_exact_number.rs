//! Managed exact decimal values and bounded divisibility arithmetic.
use super::managed_number::ManagedNumber;
use super::{
    NumericConstraint, PreparedJsonKind, PreparedNumericScratchPool, PureCodec, SchemaAst,
    SchemaIssue, SchemaIssueKind, SchemaResource, SchemaShape, SharedPreparedJsonValue, ValueKind,
};
use std::{cmp::Ordering, num::NonZeroUsize, sync::Arc};

/// A numeric subtree retaining its complete managed document.
pub struct PreparedJsonExactNumber {
    owner: SharedPreparedJsonValue,
}
impl PreparedJsonExactNumber {
    /// Borrow the original validated token.
    #[must_use]
    #[allow(clippy::expect_used)] // Private construction admits only immutable numeric subtrees.
    pub fn raw_token(&self) -> &str {
        self.owner
            .view()
            .number_token()
            .expect("prepared number invariant")
    }
    /// Retain another owner of the complete document.
    pub fn try_clone(&self) -> Result<Self, crate::memory::MemoryError> {
        Ok(Self {
            owner: self.owner.try_clone()?,
        })
    }
    /// Compare exact values without expanding exponents.
    pub fn compare(&self, other: &Self) -> Result<Ordering, crate::memory::MemoryError> {
        let a = ManagedNumber::parse(self.raw_token())
            .ok_or(crate::memory::MemoryError::InvalidRange)?;
        let b = ManagedNumber::parse(other.raw_token())
            .ok_or(crate::memory::MemoryError::InvalidRange)?;
        a.compare(b)
    }
}
#[derive(Clone, Copy)]
enum Op {
    Minimum,
    Maximum,
    ExclusiveMinimum,
    ExclusiveMaximum,
    MultipleOf,
    Integer,
}
struct Predicate {
    op: Op,
    token: Option<Arc<str>>,
    name: Arc<str>,
    operand: Arc<str>,
    divisor: Option<Divisor>,
}
struct Divisor {
    digits: Arc<[u8]>,
    twos: usize,
    fives: usize,
}
fn invalid() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
}
fn memory(error: crate::memory::MemoryError) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::Memory(error))
}
fn number(token: &str) -> Result<ManagedNumber<'_>, SchemaIssue> {
    ManagedNumber::parse(token).ok_or_else(invalid)
}
fn count_digits(token: &str, limit: NonZeroUsize) -> Result<(), SchemaIssue> {
    if token.bytes().filter(u8::is_ascii_digit).count() > limit.get() {
        Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
            resource: SchemaResource::IntegerDigits,
        }))
    } else {
        Ok(())
    }
}
fn prepare_predicates(
    constraints: Vec<NumericConstraint>,
) -> Result<Arc<[Predicate]>, SchemaIssue> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(constraints.len())
        .map_err(|_| memory(crate::memory::MemoryError::BackingAllocationFailed))?;
    for constraint in constraints {
        let (op, name, bound) = match constraint {
            NumericConstraint::Minimum(v) => (Op::Minimum, "minimum", Some(v)),
            NumericConstraint::Maximum(v) => (Op::Maximum, "maximum", Some(v)),
            NumericConstraint::ExclusiveMinimum(v) => {
                (Op::ExclusiveMinimum, "exclusiveMinimum", Some(v))
            }
            NumericConstraint::ExclusiveMaximum(v) => {
                (Op::ExclusiveMaximum, "exclusiveMaximum", Some(v))
            }
            NumericConstraint::MultipleOf(v) => (Op::MultipleOf, "multipleOf", Some(v)),
            NumericConstraint::Integer => (Op::Integer, "integer", None),
        };
        let token = bound.as_ref().map(|v| Arc::<str>::from(v.as_str()));
        let operand = token
            .as_ref()
            .map_or_else(|| Arc::<str>::from("integer"), Arc::clone);
        let divisor = if matches!(op, Op::MultipleOf) {
            let token = token.as_deref().ok_or_else(invalid)?;
            let parsed = number(token)?;
            let decimal = parsed.decimal().map_err(memory)?;
            if decimal.negative || decimal.significant == 0 {
                return Err(invalid());
            }
            let digits: Vec<u8> = decimal.normalized_digits().collect();
            let mut ascii = Vec::new();
            ascii
                .try_reserve_exact(digits.len())
                .map_err(|_| memory(crate::memory::MemoryError::BackingAllocationFailed))?;
            ascii.extend(digits.iter().copied());
            let mut integer = num_bigint::BigUint::parse_bytes(&ascii, 10).ok_or_else(invalid)?;
            let mut twos = 0usize;
            let mut fives = 0usize;
            while &integer % 2u8 == num_bigint::BigUint::from(0u8) {
                integer /= 2u8;
                twos = twos
                    .checked_add(1)
                    .ok_or_else(|| memory(crate::memory::MemoryError::ArithmeticOverflow))?;
            }
            while &integer % 5u8 == num_bigint::BigUint::from(0u8) {
                integer /= 5u8;
                fives = fives
                    .checked_add(1)
                    .ok_or_else(|| memory(crate::memory::MemoryError::ArithmeticOverflow))?;
            }
            Some(Divisor {
                digits: digits
                    .into_iter()
                    .map(|v| v - b'0')
                    .collect::<Vec<_>>()
                    .into(),
                twos,
                fives,
            })
        } else {
            None
        };
        output.push(Predicate {
            op,
            token,
            name: Arc::from(name),
            operand,
            divisor,
        });
    }
    Ok(output.into())
}
fn trim(values: &mut crate::memory::FixedVector<'static, u8>) {
    while values.len() > 1 && values.as_slice().first() == Some(&0) {
        values.remove_first();
    }
}
fn divide(values: &mut crate::memory::FixedVector<'static, u8>, divisor: u8) {
    let mut carry = 0u16;
    for digit in values.as_mut_slice() {
        let value = carry * 10 + u16::from(*digit);
        *digit = (value / u16::from(divisor)) as u8;
        carry = value % u16::from(divisor);
    }
    trim(values);
}
fn greater_or_equal(left: &[u8], right: &[u8]) -> bool {
    left.len() > right.len() || (left.len() == right.len() && left >= right)
}
fn subtract(left: &mut [u8], right: &[u8]) -> Result<(), SchemaIssue> {
    let mut borrow = 0i16;
    let mut right = right.iter().rev();
    for digit in left.iter_mut().rev() {
        let mut value = i16::from(*digit) - i16::from(*right.next().unwrap_or(&0)) - borrow;
        if value < 0 {
            value += 10;
            borrow = 1
        } else {
            borrow = 0
        }
        *digit = u8::try_from(value).map_err(|_| invalid())?;
    }
    if borrow != 0 || right.next().is_some() {
        return Err(invalid());
    }
    Ok(())
}
fn multiple(
    number: ManagedNumber<'_>,
    bound: ManagedNumber<'_>,
    divisor: &Divisor,
    scratch: &PreparedNumericScratchPool,
) -> Result<bool, SchemaIssue> {
    let digits = number.decimal().map_err(memory)?;
    if digits.significant == 0 {
        return Ok(true);
    }
    if !number.scale_at_least(bound, 0).map_err(memory)? {
        return Ok(false);
    }
    if divisor.digits.as_ref() == [1] {
        return Ok(true);
    }
    let mut low = 0usize;
    let mut high = divisor.twos.max(divisor.fives);
    while low < high {
        let mid = low + (high - low) / 2 + 1;
        if number.scale_at_least(bound, mid).map_err(memory)? {
            low = mid
        } else {
            high = mid - 1;
        }
    }
    let mut slot = scratch.checkout().map_err(memory)?;
    let needed = divisor
        .digits
        .len()
        .checked_add(1)
        .ok_or_else(|| memory(crate::memory::MemoryError::ArithmeticOverflow))?;
    if slot.divisor.capacity() < needed || slot.remainder.capacity() < needed {
        return Err(memory(crate::memory::MemoryError::ValueTooLarge {
            requested: crate::memory::Bytes::new(needed),
            capacity: crate::memory::Bytes::new(
                slot.divisor.capacity().min(slot.remainder.capacity()),
            ),
        }));
    }
    for digit in divisor.digits.iter().copied() {
        slot.divisor
            .push(digit)
            .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?;
    }
    for _ in 0..low.min(divisor.twos) {
        divide(&mut slot.divisor, 2);
    }
    for _ in 0..low.min(divisor.fives) {
        divide(&mut slot.divisor, 5);
    }
    if slot.divisor.as_slice() == [1] {
        return Ok(true);
    }
    for digit in digits.normalized_digits() {
        slot.remainder
            .push(digit - b'0')
            .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?;
        trim(&mut slot.remainder);
        // Before the next digit remainder < divisor, so remainder*10+digit
        // is less than ten divisors and needs at most nine subtractions.
        while greater_or_equal(slot.remainder.as_slice(), slot.divisor.as_slice()) {
            let super::prepared_numeric_scratch::Scratch { divisor, remainder } = &mut *slot;
            subtract(remainder.as_mut_slice(), divisor.as_slice())?;
            trim(remainder);
        }
    }
    Ok(slot.remainder.as_slice().iter().all(|digit| *digit == 0))
}
fn check(
    token: &str,
    constraints: &[Predicate],
    scratch: &PreparedNumericScratchPool,
) -> Result<(), SchemaIssue> {
    let value = number(token)?;
    for predicate in constraints {
        let valid = match predicate.op {
            Op::Integer => {
                let parts = value.decimal().map_err(memory)?;
                parts.significant == 0 || value.scale_at_least(number("1")?, 0).map_err(memory)?
            }
            Op::MultipleOf => multiple(
                value,
                number(predicate.token.as_deref().ok_or_else(invalid)?)?,
                predicate.divisor.as_ref().ok_or_else(invalid)?,
                scratch,
            )?,
            Op::Minimum | Op::Maximum | Op::ExclusiveMinimum | Op::ExclusiveMaximum => {
                let order = value
                    .compare(number(predicate.token.as_deref().ok_or_else(invalid)?)?)
                    .map_err(memory)?;
                match predicate.op {
                    Op::Minimum => order != Ordering::Less,
                    Op::Maximum => order != Ordering::Greater,
                    Op::ExclusiveMinimum => order == Ordering::Greater,
                    Op::ExclusiveMaximum => order == Ordering::Less,
                    Op::MultipleOf | Op::Integer => return Err(invalid()),
                }
            }
        };
        if !valid {
            return Err(SchemaIssue::new(SchemaIssueKind::Constraint {
                name: Arc::clone(&predicate.name),
                operand: Arc::clone(&predicate.operand),
            }));
        }
    }
    Ok(())
}
/// Construct a managed exact-number codec with explicit divisibility workspaces.
/// Predicate metadata is prepared at definition; every value retains its actual document.
pub fn json_prepared_exact_number(
    constraints: Vec<NumericConstraint>,
    digits_limit: NonZeroUsize,
    scratch: Arc<PreparedNumericScratchPool>,
) -> Result<PureCodec<PreparedJsonExactNumber, SharedPreparedJsonValue>, SchemaIssue> {
    let predicates = prepare_predicates(constraints)?;
    let decode = Arc::clone(&predicates);
    let validation = Arc::clone(&predicates);
    let writing = Arc::clone(&predicates);
    let decode_scratch = Arc::clone(&scratch);
    let validation_scratch = Arc::clone(&scratch);
    let writer_scratch = Arc::clone(&scratch);
    Ok(PureCodec::with_context_ast(
        SchemaAst::new(SchemaShape::Checked {
            source: SchemaAst::primitive(super::PrimitiveShape::Number),
            name: Arc::from("exact_json_number"),
        }),
        move |encoded: &SharedPreparedJsonValue, context| {
            if encoded.view().kind() != Some(PreparedJsonKind::Number) {
                return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                    expected: ValueKind::Number,
                    actual: super::json_codec::prepared_value_kind(encoded)?,
                }));
            }
            let token = encoded.view().number_token().ok_or_else(invalid)?;
            count_digits(
                token,
                context.map_or(digits_limit, |c| c.policy().integer_digits),
            )?;
            check(token, &decode, &decode_scratch)?;
            Ok(PreparedJsonExactNumber {
                owner: encoded.try_clone().map_err(memory)?,
            })
        },
        move |value: &PreparedJsonExactNumber, context| {
            check(value.raw_token(), &predicates, &scratch)?;
            count_digits(
                value.raw_token(),
                context.map_or(digits_limit, |c| c.policy().integer_digits),
            )?;
            value.owner.try_clone().map_err(memory)
        },
    )
    .with_context_type_validator(move |value: &PreparedJsonExactNumber, context| {
        count_digits(
            value.raw_token(),
            context.map_or(digits_limit, |c| c.policy().integer_digits),
        )?;
        check(value.raw_token(), &validation, &validation_scratch)
    })
    .with_writer(move |value, writer| {
        count_digits(value.raw_token(), digits_limit)?;
        check(value.raw_token(), &writing, &writer_scratch)?;
        value.owner.view().write_to(writer)
    })
    // Validated live slices each fit isize::MAX, so the two-length comparison
    // sums fit usize and the private token projections cannot fail.
    .with_equivalence(|a, b| a.compare(b).is_ok_and(|order| order == Ordering::Equal)))
}
