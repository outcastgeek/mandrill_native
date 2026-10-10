//! Application-owned finite HTML rendering and optional native Maud integration.
//!
//! Web supplies ownership/security boundaries, not labels or a dashboard product.
//! Untrusted text uses the selected renderer's escaping operations. Native trusted
//! markup crosses an explicit finite Server HtmlContent boundary.

use effect::Effect;
use effect_server::{HtmlContent, Reply};
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
        Reply::render(self.html(model).map(Reply::html))
    }
    /// Native Maud owns escaping and markup construction. Conversion transfers its
    /// encoded finite String into Server's trusted markup, without a JSON intermediary.
    #[cfg(feature = "maud")]
    pub fn maud(render: impl Fn(A) -> Effect<maud::Markup, E, R> + Send + Sync + 'static) -> Self {
        Self::new(move |model| {
            render(model).map(|markup| HtmlContent::trusted(markup.into_string()))
        })
    }
}
