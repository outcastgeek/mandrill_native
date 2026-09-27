use crate::{Effect, Exit};
pub use serde_json::Value as JsonValue;
use std::sync::Arc;

type Decoder<A, E> = Arc<dyn Fn(&JsonValue) -> Result<A, E> + Send + Sync>;
pub struct Schema<A, E> {
    decoder: Decoder<A, E>,
}
impl<A, E> Clone for Schema<A, E> {
    fn clone(&self) -> Self {
        Self {
            decoder: Arc::clone(&self.decoder),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static> Schema<A, E> {
    #[must_use]
    pub fn from_decoder(
        decode: impl Fn(&JsonValue) -> Result<A, E> + Send + Sync + 'static,
    ) -> Self {
        Self {
            decoder: Arc::new(decode),
        }
    }
    #[must_use]
    pub fn from_option_decoder(
        decode: impl Fn(&JsonValue) -> Option<A> + Send + Sync + 'static,
        on_invalid: impl Fn(&JsonValue) -> E + Send + Sync + 'static,
    ) -> Self {
        Self::from_decoder(move |value| decode(value).ok_or_else(|| on_invalid(value)))
    }
    pub fn decode<R: Send + Sync + 'static>(&self, value: Arc<JsonValue>) -> Effect<A, E, R> {
        let schema = self.clone();
        Effect::ready(move |_, _| match (schema.decoder)(&value) {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(error),
        })
    }
    pub fn parse<R: Send + Sync + 'static>(
        &self,
        text: impl Into<Arc<str>>,
        on_syntax_error: impl Fn(serde_json::Error) -> E + Send + Sync + 'static,
    ) -> Effect<A, E, R> {
        let schema = self.clone();
        let text = text.into();
        Effect::ready(move |_, _| match serde_json::from_str::<JsonValue>(&text) {
            Ok(json) => match (schema.decoder)(&json) {
                Ok(value) => Exit::Success(value),
                Err(error) => Exit::Fail(error),
            },
            Err(error) => Exit::Fail(on_syntax_error(error)),
        })
    }
}
