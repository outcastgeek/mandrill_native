//! Application-owned finite HTML rendering and optional native Maud integration.
//!
//! Web supplies ownership/security boundaries, not labels or a dashboard product.
//! Untrusted text uses the selected renderer's escaping operations. Native trusted
//! markup crosses an explicit finite Server HtmlContent boundary.

use crate::effect_map;
use effect::Effect;
use effect::memory::{Buffer, BufferLease, BufferView, MemoryError};
use effect_server::native::http::HeaderValue;
use effect_server::reply;
use effect_server::{HtmlContent, Reply, Request, ServerError};
use std::fmt;
use std::sync::Arc;

/// Selected finite HTML renderer; model values move once within request execution.
#[must_use]
pub struct Renderer<A, E: Send + 'static, R: Send + Sync + 'static> {
    /// Caller-owned lazy callback and selected error/environment relationship.
    render: Arc<dyn Fn(A) -> Effect<HtmlContent, E, R> + Send + Sync>,
}
impl<A, E: Send + 'static, R: Send + Sync + 'static> Clone for Renderer<A, E, R> {
    /// Share only immutable renderer configuration, without requiring A Clone.
    fn clone(&self) -> Self {
        Self {
            render: self.render.clone(),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Renderer<A, E, R> {
    /// Select a finite renderer without invoking it or acquiring a native runtime.
    pub fn new(render: impl Fn(A) -> Effect<HtmlContent, E, R> + Send + Sync + 'static) -> Self {
        Self {
            render: Arc::new(render),
        }
    }
    /// Move one execution-owned model into the selected renderer, retaining typed failures.
    pub fn html(&self, model: A) -> Effect<HtmlContent, E, R> {
        (self.render)(model)
    }
    /// Compose the existing lazy native reply boundary; no body collection occurs.
    pub fn reply(&self, model: A) -> Reply<E, R> {
        reply! {
            render(effect_map! {
                program: self.html(model),
                map: |html| reply! { html(html) },
            }),
        }
    }
    /// Native Maud owns escaping and markup construction. Conversion transfers its
    /// encoded finite String into Server's trusted markup, without a JSON intermediary.
    #[cfg(feature = "maud")]
    pub fn maud(render: impl Fn(A) -> Effect<maud::Markup, E, R> + Send + Sync + 'static) -> Self {
        Self::new(move |model| {
            effect_map! { program: render(model), map: |markup| HtmlContent::trusted(markup.into_string()), }
        })
    }
}

/// A fixed HTML destination reserved for the selected request.
pub struct PreparedHtmlWriter<'a> {
    output: &'a mut Buffer<'static>,
    failure: Option<WriteFailure>,
}

#[derive(Clone, Copy)]
enum WriteFailure {
    Memory(MemoryError),
    Format,
}
impl WriteFailure {
    fn into_error(self) -> ServerError {
        match self {
            Self::Memory(error) => ServerError::Memory(error),
            Self::Format => ServerError::Invalid("HTML text formatting".into()),
        }
    }
}

impl PreparedHtmlWriter<'_> {
    /// Format a typed message in its pinned language and escape it directly into the admitted HTML.
    pub fn write_message<C, M>(
        &mut self,
        catalog: &crate::localization::PinnedCatalog<C>,
        message: &crate::localization::BoundMessage<M>,
    ) -> Result<(), crate::localization::LocalizationError>
    where
        C: Send + Sync + 'static,
        M: crate::localization::MessageContract<Catalogue = C>,
    {
        use crate::localization::LocalizationError;
        let start = self.output.len();
        let mut writer = std::mem::DropGuard::new(self, |writer| writer.output.truncate(start));
        catalog.write_text(message, |text| {
            writer.write_text(text).map_err(|_| match writer.failure {
                Some(WriteFailure::Memory(memory)) => LocalizationError::Memory(memory),
                Some(WriteFailure::Format) | None => {
                    LocalizationError::Definition("HTML message formatting".into())
                }
            })
        })?;
        let _ = std::mem::DropGuard::dismiss(writer);
        Ok(())
    }

    /// Write escaped HTML text with the same five escapes as `HtmlContent::text`.
    pub fn write_text(&mut self, text: &str) -> Result<(), ServerError> {
        let mut start = 0;
        for (index, character) in text.char_indices() {
            let escaped = match character {
                '&' => "&amp;",
                '<' => "&lt;",
                '>' => "&gt;",
                '"' => "&quot;",
                '\'' => "&#39;",
                _ => continue,
            };
            self.write_trusted(&text[start..index])?;
            self.write_trusted(escaped)?;
            start = index + character.len_utf8();
        }
        self.write_trusted(&text[start..])
    }

    /// Format escaped text without an intermediate String.
    pub fn write_text_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<(), ServerError> {
        if let Some(failure) = self.failure {
            return Err(failure.into_error());
        }
        if fmt::write(&mut EscapedFormat(self), args).is_err() {
            return Err(self
                .failure
                .get_or_insert(WriteFailure::Format)
                .into_error());
        }
        Ok(())
    }

    /// Write explicit trusted markup without re-escaping it.
    pub fn write_trusted(&mut self, markup: &str) -> Result<(), ServerError> {
        if let Some(failure) = self.failure {
            return Err(failure.into_error());
        }
        self.output
            .extend_from_slice(markup.as_bytes())
            .map_err(|error| {
                self.failure = Some(WriteFailure::Memory(error));
                ServerError::Memory(error)
            })
    }
}

struct EscapedFormat<'writer, 'output>(&'writer mut PreparedHtmlWriter<'output>);
impl fmt::Write for EscapedFormat<'_, '_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0.write_text(text).map_err(|_| fmt::Error)
    }
}

/// Render into the selected request's preadmitted output buffer and transfer it to HTTP.
/// The finite reply witness retains the same backing when selected for caching.
pub fn prepared_html<A, E: Send + 'static, R: Send + Sync + 'static>(
    request: &Request,
    model: A,
    render: impl FnOnce(A, &mut PreparedHtmlWriter<'_>) -> Result<(), E>,
    on_error: impl Fn(ServerError) -> E,
) -> Result<Reply<E, R>, E> {
    let output = request.take_prepared_output_buffer().map_err(&on_error)?;
    prepared_html_into(output, model, render, on_error)
}

/// Clear an already selected output lease, render into it and transfer its filled buffer to HTTP.
pub fn prepared_html_into<A, E: Send + 'static, R: Send + Sync + 'static>(
    mut output: BufferLease<'static>,
    model: A,
    render: impl FnOnce(A, &mut PreparedHtmlWriter<'_>) -> Result<(), E>,
    on_error: impl Fn(ServerError) -> E,
) -> Result<Reply<E, R>, E> {
    output.clear();
    let mut writer = PreparedHtmlWriter {
        output: &mut output,
        failure: None,
    };
    render(model, &mut writer)?;
    if let Some(failure) = writer.failure {
        return Err(on_error(failure.into_error()));
    }
    let view = BufferView::new(output.share());
    Ok(reply! { lease(view, HeaderValue::from_static("text/html; charset=utf-8")); })
}
