//! Owned codec directions on the existing prepared graph description engine.
use super::{JsonWriter, ParseContext, PureCodec, SchemaAst, SchemaIssue, SchemaIssueKind};
use crate::__private::prepared_graph::Description;
use std::sync::Arc;

/// Owned codec directions represented as lazy prepared graph descriptions.
///
/// This selects prepared control flow. Payloads, callbacks and providers still
/// need their own applicable storage and allocation contracts. Witness flags
/// describe genuine independent directions; an encoder is never a substitute.
pub trait PreparedCodec<
    A: Send + 'static,
    Encoded: Send + 'static,
    DecodeEnv: Send + Sync + 'static,
    EncodeEnv: Send + Sync + 'static,
>: Send + Sync + 'static
{
    /// Expected error carried by every direction.
    type Error: Send + 'static;
    /// Existing codec metadata.
    fn ast(&self) -> &SchemaAst;
    /// Map an actual schema issue into the codec's expected error.
    fn map_schema_issue(&self, issue: SchemaIssue) -> Self::Error;
    /// Whether independent owned validation is supplied.
    fn has_validator(&self) -> bool {
        false
    }
    /// Whether an independent synchronous borrowed default validator exists.
    fn has_default_validator(&self) -> bool {
        false
    }
    /// Validate a borrowed default without encoding or consuming it.
    fn validate_default_ref(
        &self,
        _value: &A,
        _context: Option<&ParseContext>,
    ) -> Result<(), Self::Error> {
        Err(self.map_schema_issue(SchemaIssue::new(SchemaIssueKind::MissingValidator)))
    }
    /// Whether a genuine direct writer is supplied.
    fn has_writer(&self) -> bool {
        false
    }
    /// Decode one owned representation with the caller's optional context.
    fn decode(
        self: Arc<Self>,
        value: Encoded,
        context: Option<ParseContext>,
    ) -> impl Description<Value = A, Error = Self::Error, Env = DecodeEnv>;
    /// Encode one owned value with the caller's optional context.
    fn encode(
        self: Arc<Self>,
        value: A,
        context: Option<ParseContext>,
    ) -> impl Description<Value = Encoded, Error = Self::Error, Env = EncodeEnv>;
    /// Independently validate and return the same owned value, without replaying the encoder.
    fn validate(
        self: Arc<Self>,
        _value: A,
        _context: Option<ParseContext>,
    ) -> impl Description<Value = A, Error = Self::Error, Env = EncodeEnv> {
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: move || {
            let _owned = (_value, _context);
            Err::<A, _>(
                self.map_schema_issue(SchemaIssue::new(SchemaIssueKind::MissingValidator)),
            )
        }, }, }
    }
    /// Write one owned value into the supplied writer and return that owner.
    fn write(
        self: Arc<Self>,
        value: A,
        writer: JsonWriter,
    ) -> impl Description<Value = JsonWriter, Error = Self::Error, Env = EncodeEnv> {
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: move || {
            let _owned = (value, writer);
            Err::<JsonWriter, _>(self.map_schema_issue(SchemaIssue::new(SchemaIssueKind::MissingWriter)))
        }, }, }
    }
}

impl<A, Encoded, DecodeEnv, EncodeEnv> PreparedCodec<A, Encoded, DecodeEnv, EncodeEnv>
    for PureCodec<A, Encoded>
where
    A: Send + 'static,
    Encoded: Send + 'static,
    DecodeEnv: Send + Sync + 'static,
    EncodeEnv: Send + Sync + 'static,
{
    type Error = SchemaIssue;
    fn ast(&self) -> &SchemaAst {
        PureCodec::ast(self)
    }
    fn map_schema_issue(&self, issue: SchemaIssue) -> Self::Error {
        issue
    }
    fn has_validator(&self) -> bool {
        self.has_type_validator()
    }
    fn has_default_validator(&self) -> bool {
        self.has_type_validator()
    }
    fn validate_default_ref(
        &self,
        value: &A,
        context: Option<&ParseContext>,
    ) -> Result<(), Self::Error> {
        self.validate_in_ref(value, context)
    }
    fn has_writer(&self) -> bool {
        PureCodec::has_writer(self)
    }
    fn decode(
        self: Arc<Self>,
        value: Encoded,
        context: Option<ParseContext>,
    ) -> impl Description<Value = A, Error = Self::Error, Env = DecodeEnv> {
        crate::effect_prepare! { description; program: effect_try_sync! { once;
            body: move || self.decode_in_ref(&value, context.as_ref()),
        }, }
    }
    fn encode(
        self: Arc<Self>,
        value: A,
        context: Option<ParseContext>,
    ) -> impl Description<Value = Encoded, Error = Self::Error, Env = EncodeEnv> {
        crate::effect_prepare! { description; program: effect_try_sync! { once;
            body: move || self.encode_in_ref(&value, context.as_ref()),
        }, }
    }
    fn validate(
        self: Arc<Self>,
        value: A,
        context: Option<ParseContext>,
    ) -> impl Description<Value = A, Error = Self::Error, Env = EncodeEnv> {
        crate::effect_prepare! { description; program: effect_try_sync! { once;
            body: move || { self.validate_in_ref(&value, context.as_ref())?; Ok(value) },
        }, }
    }
    fn write(
        self: Arc<Self>,
        value: A,
        mut writer: JsonWriter,
    ) -> impl Description<Value = JsonWriter, Error = Self::Error, Env = EncodeEnv> {
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: move || {
            if !PureCodec::has_writer(&self) {
                return Err(SchemaIssue::new(SchemaIssueKind::MissingWriter));
            }
            self.write_into(&value, &mut writer)?;
            Ok(writer)
        }, }, }
    }
}

/// A pure codec configured with its prepared graph's expected-error family.
///
/// Mapping occurs inside each lazy leaf. This does not convert errors between
/// already constructed graphs or change the ordinary `PureCodec` methods.
pub struct PreparedPureCodec<A, Encoded, X, F> {
    codec: PureCodec<A, Encoded>,
    map: F,
    marker: std::marker::PhantomData<fn() -> X>,
}

impl<A: Send + 'static, Encoded: Send + 'static> PureCodec<A, Encoded> {
    /// Select a prepared root error family before graph construction.
    ///
    /// The mapper runs only for an actual schema issue in an invoked direction.
    /// Its storage and any allocations are part of the caller's callback contract.
    #[must_use]
    pub fn with_prepared_error_map<X, F>(self, map: F) -> PreparedPureCodec<A, Encoded, X, F>
    where
        X: Send + 'static,
        F: Fn(SchemaIssue) -> X + Send + Sync + 'static,
    {
        PreparedPureCodec {
            codec: self,
            map,
            marker: std::marker::PhantomData,
        }
    }
}

impl<A, Encoded, X, F, DecodeEnv, EncodeEnv> PreparedCodec<A, Encoded, DecodeEnv, EncodeEnv>
    for PreparedPureCodec<A, Encoded, X, F>
where
    A: Send + 'static,
    Encoded: Send + 'static,
    X: Send + 'static,
    F: Fn(SchemaIssue) -> X + Send + Sync + 'static,
    DecodeEnv: Send + Sync + 'static,
    EncodeEnv: Send + Sync + 'static,
{
    type Error = X;
    fn ast(&self) -> &SchemaAst {
        self.codec.ast()
    }
    fn map_schema_issue(&self, issue: SchemaIssue) -> X {
        (self.map)(issue)
    }
    fn has_validator(&self) -> bool {
        self.codec.has_type_validator()
    }
    fn has_default_validator(&self) -> bool {
        self.codec.has_type_validator()
    }
    fn validate_default_ref(
        &self,
        value: &A,
        context: Option<&ParseContext>,
    ) -> Result<(), Self::Error> {
        self.codec
            .validate_in_ref(value, context)
            .map_err(&self.map)
    }
    fn has_writer(&self) -> bool {
        self.codec.has_writer()
    }
    fn decode(
        self: Arc<Self>,
        value: Encoded,
        context: Option<ParseContext>,
    ) -> impl Description<Value = A, Error = X, Env = DecodeEnv> {
        crate::effect_prepare! { description; program: effect_try_sync! { once;
            body: move || self.codec.decode_in_ref(&value, context.as_ref()).map_err(&self.map),
        }, }
    }
    fn encode(
        self: Arc<Self>,
        value: A,
        context: Option<ParseContext>,
    ) -> impl Description<Value = Encoded, Error = X, Env = EncodeEnv> {
        crate::effect_prepare! { description; program: effect_try_sync! { once;
            body: move || self.codec.encode_in_ref(&value, context.as_ref()).map_err(&self.map),
        }, }
    }
    fn validate(
        self: Arc<Self>,
        value: A,
        context: Option<ParseContext>,
    ) -> impl Description<Value = A, Error = X, Env = EncodeEnv> {
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: move || {
            self.codec.validate_in_ref(&value, context.as_ref()).map_err(&self.map)?;
            Ok(value)
        }, }, }
    }
    fn write(
        self: Arc<Self>,
        value: A,
        mut writer: JsonWriter,
    ) -> impl Description<Value = JsonWriter, Error = X, Env = EncodeEnv> {
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: move || {
            if !self.codec.has_writer() {
                return Err((self.map)(SchemaIssue::new(SchemaIssueKind::MissingWriter)));
            }
            self.codec.write_into(&value, &mut writer).map_err(&self.map)?;
            Ok(writer)
        }, }, }
    }
}
