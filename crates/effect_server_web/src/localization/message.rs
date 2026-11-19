//! Typed named message arguments and checked composite templates.

use super::LocalizationError;
use std::fmt;
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
    /// Write a typed argument to a checked destination.
    /// Custom formatters can override this to avoid their ordinary String result.
    fn write(
        &self,
        culture: &super::CultureId,
        append: &mut dyn FnMut(&str) -> Result<(), LocalizationError>,
    ) -> Result<(), LocalizationError> {
        append(&self.format(culture)?)
    }
}
impl ArgumentValue for String {
    /// Preserve owned user text; HTML escaping remains the renderer's responsibility.
    fn format(&self, _: &super::CultureId) -> Result<String, LocalizationError> {
        Ok(self.clone())
    }
    fn write(
        &self,
        _: &super::CultureId,
        append: &mut dyn FnMut(&str) -> Result<(), LocalizationError>,
    ) -> Result<(), LocalizationError> {
        append(self)
    }
}
impl ArgumentValue for Arc<str> {
    /// Encode immutable text for the finite message output without a payload clone bound.
    fn format(&self, _: &super::CultureId) -> Result<String, LocalizationError> {
        Ok(self.to_string())
    }
    fn write(
        &self,
        _: &super::CultureId,
        append: &mut dyn FnMut(&str) -> Result<(), LocalizationError>,
    ) -> Result<(), LocalizationError> {
        append(self)
    }
}
impl ArgumentValue for &'static str {
    /// Static application text is already owned for the message lifetime.
    fn format(&self, _: &super::CultureId) -> Result<String, LocalizationError> {
        Ok((*self).to_owned())
    }
    fn write(
        &self,
        _: &super::CultureId,
        append: &mut dyn FnMut(&str) -> Result<(), LocalizationError>,
    ) -> Result<(), LocalizationError> {
        append(self)
    }
}
/// Implement explicit numeric conversion, without locale claims. Applications can
/// provide a wrapper formatter when grouping/currency/localized digits matter.
macro_rules! integer_arguments { ($($ty:ty),* $(,)?) => { $(impl ArgumentValue for $ty {
    /// Decimal integer text; culture-specific formatting requires an application wrapper.
    fn format(&self, _: &super::CultureId) -> Result<String, LocalizationError> { Ok(self.to_string()) }
    fn write(&self, _: &super::CultureId, append: &mut dyn FnMut(&str) -> Result<(), LocalizationError>) -> Result<(), LocalizationError> {
        append(self.format_into(&mut std::fmt::NumBuffer::new()))
    }
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
    fn write(
        &self,
        _: &super::CultureId,
        append: &mut dyn FnMut(&str) -> Result<(), LocalizationError>,
    ) -> Result<(), LocalizationError> {
        if !self.is_finite() {
            return Err(LocalizationError::Template("nonfinite numeric argument".into()));
        }
        let mut writer = ArgumentFormat { append, failure: None };
        if fmt::write(&mut writer, format_args!("{self}")).is_err() {
            return Err(writer.failure.unwrap_or_else(|| LocalizationError::Template("numeric argument formatting".into())));
        }
        Ok(())
    }
})* }; }
floating_arguments!(f32, f64);

struct ArgumentFormat<'a> {
    append: &'a mut dyn FnMut(&str) -> Result<(), LocalizationError>,
    failure: Option<LocalizationError>,
}
impl fmt::Write for ArgumentFormat<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.failure.is_some() {
            return Err(fmt::Error);
        }
        (self.append)(text).map_err(|error| {
            self.failure = Some(error);
            fmt::Error
        })
    }
}
impl ArgumentValue for bool {
    /// Boolean protocol text is explicit; localized human labels use a message contract.
    fn format(&self, _: &super::CultureId) -> Result<String, LocalizationError> {
        Ok(self.to_string())
    }
    fn write(
        &self,
        _: &super::CultureId,
        append: &mut dyn FnMut(&str) -> Result<(), LocalizationError>,
    ) -> Result<(), LocalizationError> {
        append(if *self { "true" } else { "false" })
    }
}
/// A typed argument list supporting checked named access. Tuple implementations
/// preserve each field type and move arguments only once into a bound message.
pub trait ArgumentList: Send + 'static {
    /// Field identities carried by this typed value.
    fn names(&self) -> Vec<&str>;
    /// Match the ordered fields against a declared message contract.
    fn matches_names(&self, expected: &[&str]) -> bool {
        self.names() == expected
    }
    /// Format one requested field, rejecting unknown identities.
    fn value(&self, field: &str, culture: &super::CultureId) -> Result<String, LocalizationError>;
    /// Send the selected field to a checked writer.
    fn write_value(
        &self,
        field: &str,
        culture: &super::CultureId,
        append: &mut dyn FnMut(&str) -> Result<(), LocalizationError>,
    ) -> Result<(), LocalizationError> {
        append(&self.value(field, culture)?)
    }
}
impl ArgumentList for () {
    /// A text-only message has no fields.
    fn names(&self) -> Vec<&str> {
        Vec::new()
    }
    fn matches_names(&self, expected: &[&str]) -> bool {
        expected.is_empty()
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
        fn matches_names(&self, expected: &[&str]) -> bool {
            [$(self.$index.0),+].as_slice() == expected
        }
        /// Explicitly match a checked contract field and use its typed formatter.
        fn value(&self, field: &str, culture: &super::CultureId) -> Result<String, LocalizationError> {
            $(if field == self.$index.0 { return self.$index.1.format(culture); })+
            Err(LocalizationError::Template(field.into()))
        }
        fn write_value(
            &self,
            field: &str,
            culture: &super::CultureId,
            append: &mut dyn FnMut(&str) -> Result<(), LocalizationError>,
        ) -> Result<(), LocalizationError> {
            $(if field == self.$index.0 { return self.$index.1.write(culture, append); })+
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
        if !arguments.matches_names(M::FIELDS) {
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

/// Visit validated literals and fields while borrowing all source text.
pub(super) fn visit_template(
    source: &str,
    fields: &[&str],
    mut visit: impl FnMut(bool, &str) -> Result<(), LocalizationError>,
) -> Result<(), LocalizationError> {
    let mut remainder = source;
    while let Some(index) = remainder.find(['{', '}']) {
        let (literal, brace) = remainder.split_at(index);
        if !literal.is_empty() {
            visit(false, literal)?;
        }
        if let Some(tail) = brace.strip_prefix("{{") {
            visit(false, "{")?;
            remainder = tail;
        } else if let Some(tail) = brace.strip_prefix("}}") {
            visit(false, "}")?;
            remainder = tail;
        } else if let Some(field) = brace.strip_prefix('{') {
            let matching = field.split_once('}');
            let name = matching.map_or(field, |(name, _)| name);
            if !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err(LocalizationError::Template("field syntax".into()));
            }
            let Some((field, tail)) = matching else {
                return Err(LocalizationError::Template("field contract".into()));
            };
            if field.is_empty() || !fields.contains(&field) {
                return Err(LocalizationError::Template("field contract".into()));
            }
            visit(true, field)?;
            remainder = tail;
        } else {
            return Err(LocalizationError::Template(
                "unmatched closing brace".into(),
            ));
        }
    }
    if !remainder.is_empty() {
        visit(false, remainder)?;
    }
    Ok(())
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
        let mut literal = String::new();
        let mut parts = Vec::new();
        visit_template(text, fields, |field, text| {
            if field {
                if !literal.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut literal).into()));
                }
                parts.push(Part::Field(text.into()));
            } else {
                literal.push_str(text);
            }
            Ok(())
        })?;
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
        self.write(arguments, culture, maximum_bytes, |text| {
            output.push_str(text);
            Ok(())
        })?;
        Ok(output)
    }

    pub(super) fn write(
        &self,
        arguments: &impl ArgumentList,
        culture: &super::CultureId,
        maximum_bytes: usize,
        emit: impl FnMut(&str) -> Result<(), LocalizationError>,
    ) -> Result<(), LocalizationError> {
        write_bounded(maximum_bytes, emit, |append| {
            for part in self.parts.iter() {
                match part {
                    Part::Text(text) => append(text)?,
                    Part::Field(field) => arguments.write_value(field, culture, append)?,
                }
            }
            Ok(())
        })
    }
}

type Append<'a> = dyn FnMut(&str) -> Result<(), LocalizationError> + 'a;

pub(super) fn write_bounded(
    maximum_bytes: usize,
    mut emit: impl FnMut(&str) -> Result<(), LocalizationError>,
    write: impl FnOnce(&mut Append<'_>) -> Result<(), LocalizationError>,
) -> Result<(), LocalizationError> {
    let mut length = 0_usize;
    let mut failure: Option<LocalizationError> = None;
    let mut append = |text: &str| {
        if let Some(error) = &failure {
            return Err(error.clone());
        }
        let result = length
            .checked_add(text.len())
            .filter(|length| *length <= maximum_bytes)
            .ok_or_else(|| LocalizationError::Limit("formatted message bytes".into()))
            .and_then(|next| {
                emit(text)?;
                length = next;
                Ok(())
            });
        if let Err(error) = &result {
            failure = Some(error.clone());
        }
        result
    };
    write(&mut append)?;
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
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
    fn matches_names(&self, expected: &[&str]) -> bool {
        expected
            .split_first()
            .is_some_and(|(first, tail)| *first == self.field.0 && self.tail.matches_names(tail))
    }
    /// Invoke the explicitly typed formatter for this exact contract field.
    fn value(&self, field: &str, culture: &super::CultureId) -> Result<String, LocalizationError> {
        if field == self.field.0 {
            self.field.1.format(culture)
        } else {
            self.tail.value(field, culture)
        }
    }
    fn write_value(
        &self,
        field: &str,
        culture: &super::CultureId,
        append: &mut dyn FnMut(&str) -> Result<(), LocalizationError>,
    ) -> Result<(), LocalizationError> {
        if field == self.field.0 {
            self.field.1.write(culture, append)
        } else {
            self.tail.write_value(field, culture, append)
        }
    }
}
