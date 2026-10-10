//! Typed named message arguments and checked composite templates.

use super::LocalizationError;
use std::marker::PhantomData;
use std::sync::Arc;

/// Application message contract with catalogue identity and argument type retained
/// at compile time. `FIELDS` names form the resource validation contract.
pub trait MessageContract: Send + Sync + 'static {
    /// Marker identifying the catalogue family, preventing cross-catalogue lookup.
    type Catalogue: Send + Sync + 'static;
    /// Owned typed named argument tuple. It is not converted through JSON/Debug.
    type Arguments: ArgumentList;
    /// Stable catalogue identity used to validate resource/message association.
    const CATALOGUE: &'static str;
    /// Stable flattened message identity.
    const KEY: &'static str;
    /// Exact named fields, in declared argument order.
    const FIELDS: &'static [&'static str];
}
/// Supported typed argument conversions. Implementations may use a selected
/// application formatter; generic Debug/stringification is never a fallback.
pub trait ArgumentValue: Send + 'static {
    /// Produce finite display text for the selected formatting culture.
    fn format(&self, culture: &super::CultureId) -> Result<String, LocalizationError>;
}
impl ArgumentValue for String {
    /// Preserve owned user text; HTML escaping remains the renderer's responsibility.
    fn format(&self, _: &super::CultureId) -> Result<String, LocalizationError> {
        Ok(self.clone())
    }
}
impl ArgumentValue for Arc<str> {
    /// Encode immutable text for the finite message output without a payload clone bound.
    fn format(&self, _: &super::CultureId) -> Result<String, LocalizationError> {
        Ok(self.to_string())
    }
}
impl ArgumentValue for &'static str {
    /// Static application text is already owned for the message lifetime.
    fn format(&self, _: &super::CultureId) -> Result<String, LocalizationError> {
        Ok((*self).to_owned())
    }
}
/// Implement explicit numeric conversion, without locale claims. Applications can
/// provide a wrapper formatter when grouping/currency/localized digits matter.
macro_rules! integer_arguments { ($($ty:ty),* $(,)?) => { $(impl ArgumentValue for $ty {
    /// Decimal integer text; culture-specific formatting requires an application wrapper.
    fn format(&self, _: &super::CultureId) -> Result<String, LocalizationError> { Ok(self.to_string()) }
})* }; }
integer_arguments!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);
/// Explicit finite floating-point formatting; nonfinite numeric values are rejected
/// rather than silently becoming a human-readable message or a JSON-like sentinel.
macro_rules! floating_arguments { ($($ty:ty),* $(,)?) => { $(impl ArgumentValue for $ty {
    /// Native finite decimal representation; localized numeric wrappers remain caller owned.
    fn format(&self, _: &super::CultureId) -> Result<String, LocalizationError> {
        if !self.is_finite() { return Err(LocalizationError::Template("nonfinite numeric argument".into())); }
        Ok(self.to_string())
    }
})* }; }
floating_arguments!(f32, f64);
impl ArgumentValue for bool {
    /// Boolean protocol text is explicit; localized human labels use a message contract.
    fn format(&self, _: &super::CultureId) -> Result<String, LocalizationError> {
        Ok(self.to_string())
    }
}
/// A typed argument list supporting checked named access. Tuple implementations
/// preserve each field type and move arguments only once into a bound message.
pub trait ArgumentList: Send + 'static {
    /// Field identities carried by this typed value.
    fn names(&self) -> Vec<&str>;
    /// Format one requested field, rejecting unknown identities.
    fn value(&self, field: &str, culture: &super::CultureId) -> Result<String, LocalizationError>;
}
impl ArgumentList for () {
    /// A text-only message has no fields.
    fn names(&self) -> Vec<&str> {
        Vec::new()
    }
    /// Empty argument contracts never accept an interpolation field.
    fn value(&self, field: &str, _: &super::CultureId) -> Result<String, LocalizationError> {
        Err(LocalizationError::Template(field.into()))
    }
}
/// Generate typed tuple support without erasing field types or cloning values.
macro_rules! argument_tuple { ($(($ty:ident, $index:tt)),+) => {
    impl<$($ty: ArgumentValue),+> ArgumentList for ($((&'static str, $ty),)+) {
        /// Return field metadata only; field payloads stay owned by this tuple.
        fn names(&self) -> Vec<&str> { vec![$(self.$index.0),+] }
        /// Explicitly match a checked contract field and use its typed formatter.
        fn value(&self, field: &str, culture: &super::CultureId) -> Result<String, LocalizationError> {
            $(if field == self.$index.0 { return self.$index.1.format(culture); })+
            Err(LocalizationError::Template(field.into()))
        }
    }
}; }
argument_tuple!((A, 0));
argument_tuple!((A, 0), (B, 1));
argument_tuple!((A, 0), (B, 1), (C, 2));
argument_tuple!((A, 0), (B, 1), (C, 2), (D, 3));
argument_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (F, 4));
argument_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (F, 4), (G, 5));
/// A reusable message identity, not a raw string template or runtime interpolation fallback.
#[must_use]
pub struct Message<M: MessageContract>(PhantomData<fn() -> M>);
impl<M: MessageContract> Copy for Message<M> {}
impl<M: MessageContract> Clone for Message<M> {
    /// Message metadata contains no payload.
    fn clone(&self) -> Self {
        *self
    }
}
impl<M: MessageContract> Default for Message<M> {
    /// Construct the application's static contract witness.
    fn default() -> Self {
        Self::new()
    }
}
impl<M: MessageContract> Message<M> {
    /// Construct an inert identity preserving catalogue and argument type relationships.
    pub const fn new() -> Self {
        Self(PhantomData)
    }
    /// Move exactly the contract's argument type and check names/order without
    /// stringifying unknown values. Duplicate fields are never silently overwritten.
    pub fn bind(self, arguments: M::Arguments) -> Result<BoundMessage<M>, LocalizationError> {
        if arguments.names() != M::FIELDS {
            return Err(LocalizationError::Template(M::KEY.into()));
        }
        Ok(BoundMessage {
            arguments,
            marker: PhantomData,
        })
    }
}
/// Owned typed arguments; formatting borrows them under the request-pinned generation.
#[must_use]
pub struct BoundMessage<M: MessageContract> {
    /// Original typed values, never stored in a JSON/Debug intermediary.
    pub(crate) arguments: M::Arguments,
    /// Catalogue/message relation retained without constructing M.
    marker: PhantomData<fn() -> M>,
}
/// Static template operation; literals and named placeholders remain distinct.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    /// Literal source with escaped braces resolved during validation.
    Text(Arc<str>),
    /// A named contract field; no positional/raw-string interpolation fallback.
    Field(Arc<str>),
}
/// Validated immutable composite template.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub struct Template {
    /// Prepared operations shared by catalogue generations.
    parts: Arc<[Part]>,
}
impl Template {
    /// Parse `{name}` with doubled brace escapes. Every field must occur in the
    /// provided contract; missing/repeated argument uses remain contract-valid.
    pub fn parse(text: &str, fields: &[&str]) -> Result<Self, LocalizationError> {
        let mut chars = text.chars().peekable();
        let mut literal = String::new();
        let mut parts = Vec::new();
        while let Some(character) = chars.next() {
            match character {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    literal.push('}');
                }
                '{' => {
                    if !literal.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut literal).into()));
                    }
                    let mut field = String::new();
                    let mut closed = false;
                    for character in chars.by_ref() {
                        if character == '}' {
                            closed = true;
                            break;
                        }
                        if !(character.is_ascii_alphanumeric() || character == '_') {
                            return Err(LocalizationError::Template("field syntax".into()));
                        }
                        field.push(character);
                    }
                    if !closed || field.is_empty() || !fields.contains(&field.as_str()) {
                        return Err(LocalizationError::Template("field contract".into()));
                    }
                    parts.push(Part::Field(field.into()));
                }
                '}' => {
                    return Err(LocalizationError::Template(
                        "unmatched closing brace".into(),
                    ));
                }
                character => literal.push(character),
            }
        }
        if !literal.is_empty() {
            parts.push(Part::Text(literal.into()));
        }
        Ok(Self {
            parts: parts.into(),
        })
    }
    /// Render only the already checked typed argument list under explicit culture
    /// and a caller-supplied byte bound. No raw template execution fallback exists.
    pub fn render(
        &self,
        arguments: &impl ArgumentList,
        culture: &super::CultureId,
        maximum_bytes: usize,
    ) -> Result<String, LocalizationError> {
        let mut output = String::new();
        for part in self.parts.iter() {
            let text = match part {
                Part::Text(text) => text.to_string(),
                Part::Field(field) => arguments.value(field, culture)?,
            };
            if output
                .len()
                .checked_add(text.len())
                .is_none_or(|length| length > maximum_bytes)
            {
                return Err(LocalizationError::Limit("formatted message bytes".into()));
            }
            output.push_str(&text);
        }
        Ok(output)
    }
}

/// Recursive typed named argument list, supporting any application-defined arity
/// without an arbitrary tuple-width limit. D26 moves fields into this ownership chain.
#[must_use]
pub struct NamedArgument<V: ArgumentValue, Tail: ArgumentList> {
    /// Checked field metadata and its original typed value.
    field: (&'static str, V),
    /// Remaining typed named values, preserving declaration order.
    tail: Tail,
}
impl<V: ArgumentValue, Tail: ArgumentList> NamedArgument<V, Tail> {
    /// Construct one owned field node; Message::bind validates the complete contract.
    pub fn new(field: (&'static str, V), tail: Tail) -> Self {
        Self { field, tail }
    }
}
impl<V: ArgumentValue, Tail: ArgumentList> ArgumentList for NamedArgument<V, Tail> {
    /// Inspect only field metadata; payloads remain in their original owners.
    fn names(&self) -> Vec<&str> {
        let mut names = vec![self.field.0];
        names.extend(self.tail.names());
        names
    }
    /// Invoke the explicitly typed formatter for this exact contract field.
    fn value(&self, field: &str, culture: &super::CultureId) -> Result<String, LocalizationError> {
        if field == self.field.0 {
            self.field.1.format(culture)
        } else {
            self.tail.value(field, culture)
        }
    }
}
