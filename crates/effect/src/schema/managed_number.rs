//! Borrowed JSON-number grammar and serde_json arbitrary-precision spelling.
/// A validated token whose exponent uses serde_json's lowercase/sign spelling on output.
#[derive(Clone, Copy)]
pub(crate) struct ManagedNumber<'a> {
    token: &'a str,
    exponent: Option<usize>,
}
impl<'a> ManagedNumber<'a> {
    pub fn parse(token: &'a str) -> Option<Self> {
        let b = token.as_bytes();
        let mut i = 0usize;
        if b.get(i) == Some(&b'-') {
            i += 1;
        }
        match b.get(i) {
            Some(b'0') => {
                i += 1;
                if b.get(i).is_some_and(u8::is_ascii_digit) {
                    return None;
                }
            }
            Some(b'1'..=b'9') => {
                i += 1;
                while b.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
            }
            _ => return None,
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            let start = i;
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if i == start {
                return None;
            }
        }
        let exponent = if matches!(b.get(i), Some(b'e' | b'E')) {
            let start = i;
            i += 1;
            if matches!(b.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            let digits = i;
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if i == digits {
                return None;
            }
            Some(start)
        } else {
            None
        };
        (i == b.len()).then_some(Self { token, exponent })
    }
    pub fn pieces(self) -> (&'a str, &'static str, &'static str, &'a str) {
        if let Some(index) = self.exponent {
            let prefix = self.token.get(..index).unwrap_or("");
            let suffix = self.token.get(index + 1..).unwrap_or("");
            let sign = if suffix.starts_with(['+', '-']) {
                ""
            } else {
                "+"
            };
            (prefix, "e", sign, suffix)
        } else {
            (self.token, "", "", "")
        }
    }
    pub fn is_normalized(self) -> bool {
        let (prefix, e, sign, suffix) = self.pieces();
        self.token.len() == prefix.len() + e.len() + sign.len() + suffix.len()
            && (e.is_empty() || self.token.as_bytes().get(prefix.len()) == Some(&b'e'))
    }
    pub fn digits(self) -> usize {
        self.token.bytes().filter(u8::is_ascii_digit).count()
    }
    pub fn as_i64(self) -> Option<i64> {
        self.token.parse().ok()
    }
    pub fn as_u64(self) -> Option<u64> {
        self.token.parse().ok()
    }
    pub fn as_f64(self) -> Option<f64> {
        self.token.parse::<f64>().ok().filter(|n| n.is_finite())
    }
}

#[derive(Clone, Copy)]
pub(super) struct DecimalParts<'a> {
    pub negative: bool,
    integer: usize,
    leading: usize,
    pub significant: usize,
    pub trailing: usize,
    pub fraction: usize,
    coefficient: &'a [u8],
    exponent: Exponent<'a>,
}
#[derive(Clone, Copy)]
struct Exponent<'a> {
    negative: bool,
    digits: &'a [u8],
}
#[derive(Clone, Copy)]
struct SignedMagnitude {
    negative: bool,
    magnitude: Option<usize>,
}
impl SignedMagnitude {
    fn zero() -> Self {
        Self {
            negative: false,
            magnitude: Some(0),
        }
    }
    fn compare_to_offset(self, negative: bool, offset: usize) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        if self.negative != negative {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let magnitude = self
            .magnitude
            .map_or(Ordering::Greater, |value| value.cmp(&offset));
        if self.negative {
            magnitude.reverse()
        } else {
            magnitude
        }
    }
}
impl<'a> Exponent<'a> {
    fn from_text(text: &'a str) -> Self {
        let (negative, digits) = if let Some(rest) = text.strip_prefix('-') {
            (true, rest)
        } else if let Some(rest) = text.strip_prefix('+') {
            (false, rest)
        } else {
            (false, text)
        };
        let digits = digits.as_bytes();
        let first = digits
            .iter()
            .position(|b| *b != b'0')
            .unwrap_or(digits.len());
        let digits = digits.get(first..).unwrap_or(&[]);
        Self {
            negative: negative && !digits.is_empty(),
            digits,
        }
    }
    fn magnitude_cmp(self, other: Self) -> std::cmp::Ordering {
        self.digits
            .len()
            .cmp(&other.digits.len())
            .then_with(|| self.digits.cmp(other.digits))
    }
}
impl<'a> ManagedNumber<'a> {
    pub(super) fn decimal(self) -> Result<DecimalParts<'a>, crate::memory::MemoryError> {
        let (coefficient, exponent) = match self.exponent {
            Some(index) => (
                self.token
                    .get(..index)
                    .ok_or(crate::memory::MemoryError::InvalidRange)?,
                self.token
                    .get(index + 1..)
                    .ok_or(crate::memory::MemoryError::InvalidRange)?,
            ),
            None => (self.token, "0"),
        };
        let negative = coefficient.starts_with('-');
        let unsigned = coefficient.trim_prefix('-');
        let integer = unsigned.find('.').unwrap_or(unsigned.len());
        let mut leading = 0usize;
        let mut total = 0usize;
        let mut nonzero = false;
        for byte in unsigned.bytes().filter(u8::is_ascii_digit) {
            total = total
                .checked_add(1)
                .ok_or(crate::memory::MemoryError::ArithmeticOverflow)?;
            if !nonzero && byte == b'0' {
                leading = leading
                    .checked_add(1)
                    .ok_or(crate::memory::MemoryError::ArithmeticOverflow)?;
            } else {
                nonzero = true;
            }
        }
        let trailing = if nonzero {
            unsigned
                .bytes()
                .rev()
                .filter(u8::is_ascii_digit)
                .take_while(|byte| *byte == b'0')
                .count()
        } else {
            0
        };
        Ok(DecimalParts {
            negative: negative && nonzero,
            integer,
            leading,
            significant: total - leading,
            trailing,
            fraction: total - integer,
            coefficient: unsigned.as_bytes(),
            exponent: Exponent::from_text(exponent),
        })
    }
    /// Exact mathematical comparison without expanding exponent or coefficient.
    pub fn compare(self, other: Self) -> Result<std::cmp::Ordering, crate::memory::MemoryError> {
        use std::cmp::Ordering;
        let a = self.decimal()?;
        let b = other.decimal()?;
        if a.significant == 0 || b.significant == 0 {
            return Ok(match (a.significant == 0, b.significant == 0) {
                (true, true) => Ordering::Equal,
                (true, false) => {
                    if b.negative {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    }
                }
                (false, true) => {
                    if a.negative {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                }
                (false, false) => Ordering::Equal,
            });
        }
        if a.negative != b.negative {
            return Ok(if a.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            });
        }
        let left = b
            .integer
            .checked_add(a.leading)
            .ok_or(crate::memory::MemoryError::ArithmeticOverflow)?;
        let right = a
            .integer
            .checked_add(b.leading)
            .ok_or(crate::memory::MemoryError::ArithmeticOverflow)?;
        let (negative, offset) = if left >= right {
            (false, left - right)
        } else {
            (true, right - left)
        };
        let magnitude =
            exponent_difference(a.exponent, b.exponent).compare_to_offset(negative, offset);
        let ordering = if magnitude == Ordering::Equal {
            let length = a.significant.max(b.significant);
            a.coefficient
                .iter()
                .copied()
                .filter(u8::is_ascii_digit)
                .skip(a.leading)
                .chain(std::iter::repeat(b'0'))
                .take(length)
                .cmp(
                    b.coefficient
                        .iter()
                        .copied()
                        .filter(u8::is_ascii_digit)
                        .skip(b.leading)
                        .chain(std::iter::repeat(b'0'))
                        .take(length),
                )
        } else {
            magnitude
        };
        Ok(if a.negative {
            ordering.reverse()
        } else {
            ordering
        })
    }
    pub(super) fn scale_at_least(
        self,
        other: Self,
        minimum: usize,
    ) -> Result<bool, crate::memory::MemoryError> {
        let a = self.decimal()?;
        let b = other.decimal()?;
        let left = a
            .fraction
            .checked_add(b.trailing)
            .and_then(|n| n.checked_add(minimum))
            .ok_or(crate::memory::MemoryError::ArithmeticOverflow)?;
        let right = a
            .trailing
            .checked_add(b.fraction)
            .ok_or(crate::memory::MemoryError::ArithmeticOverflow)?;
        let (negative, offset) = if left >= right {
            (false, left - right)
        } else {
            (true, right - left)
        };
        Ok(
            exponent_difference(a.exponent, b.exponent).compare_to_offset(negative, offset)
                != std::cmp::Ordering::Less,
        )
    }
}
impl DecimalParts<'_> {
    pub(super) fn normalized_digits(&self) -> impl Iterator<Item = u8> + '_ {
        self.coefficient
            .iter()
            .copied()
            .filter(u8::is_ascii_digit)
            .skip(self.leading)
            .take(self.significant.saturating_sub(self.trailing))
    }
}
fn exponent_difference(a: Exponent<'_>, b: Exponent<'_>) -> SignedMagnitude {
    use std::cmp::Ordering;
    let same_sign = a.negative == b.negative;
    let (left, right, negative, subtract) = if same_sign {
        match a.magnitude_cmp(b) {
            Ordering::Equal => return SignedMagnitude::zero(),
            Ordering::Greater => (a.digits, b.digits, a.negative, true),
            Ordering::Less => (b.digits, a.digits, !a.negative, true),
        }
    } else {
        (a.digits, b.digits, a.negative, false)
    };
    let mut left = left.iter().rev();
    let mut right = right.iter().rev();
    let mut carry = 0i16;
    let mut factor = Some(1usize);
    let mut value = Some(0usize);
    loop {
        let l = left.next().map(|b| i16::from(*b - b'0'));
        let r = right.next().map(|b| i16::from(*b - b'0'));
        if l.is_none() && r.is_none() && carry == 0 {
            break;
        }
        let mut digit = if subtract {
            l.unwrap_or(0) - r.unwrap_or(0) - carry
        } else {
            l.unwrap_or(0) + r.unwrap_or(0) + carry
        };
        if subtract {
            if digit < 0 {
                digit += 10;
                carry = 1
            } else {
                carry = 0
            }
        } else {
            carry = digit / 10;
            digit %= 10;
        }
        if digit != 0 {
            value = value.and_then(|v| {
                factor
                    .and_then(|f| f.checked_mul(digit as usize))
                    .and_then(|n| v.checked_add(n))
            });
        }
        factor = factor.and_then(|f| f.checked_mul(10));
    }
    if value == Some(0) {
        SignedMagnitude::zero()
    } else {
        SignedMagnitude {
            negative,
            magnitude: value,
        }
    }
}
