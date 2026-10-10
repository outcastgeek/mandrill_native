//! Deferred native bodies executed under a scope owned by the enclosing domain.

use super::macro_future::MacroContext;
use super::runtime::{self, Step};
use crate::{Defect, Effect, Exit, Scope};
use futures_util::FutureExt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

/// Factory type sharing only a reusable description, never evaluation-local values.
type Run<A, E, R> = dyn Fn(Scope<E>) -> Effect<A, E, R> + Send + Sync;

/// Internal typed body adapter; its external owner is responsible for protected scope closure.
/// It introduces no public runtime access and accepts the existing sealed macro bindings.
#[doc(hidden)]
pub struct ScopedBody<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    /// Lazy factory using the caller's already established resource owner.
    run: Arc<Run<A, E, R>>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone for ScopedBody<A, E, R> {
    /// Share the description without copying payloads or failures.
    fn clone(&self) -> Self {
        Self {
            run: Arc::clone(&self.run),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> ScopedBody<A, E, R> {
    /// Lift a reusable Effect into the existing owner's execution boundary.
    #[must_use]
    pub fn from_effect(effect: Effect<A, E, R>) -> Self {
        Self {
            run: Arc::new(move |_| effect.clone()),
        }
    }
    /// Describe a native body, reconstructing its future and locals on each execution.
    pub fn from_body<F>(factory: impl Fn(MacroContext<E, R>) -> F + Send + Sync + 'static) -> Self
    where
        F: Future<Output = Result<A, Exit<A, E>>> + Send + 'static,
    {
        let factory = Arc::new(factory);
        Self {
            run: Arc::new(move |scope| {
                let factory = Arc::clone(&factory);
                Effect::new(move |env, context, finish| {
                    let bridge = MacroContext::with_scope(env, context, scope.clone());
                    let factory = Arc::clone(&factory);
                    Step::Composed(Box::pin(async move {
                        let result = AssertUnwindSafe(async { factory(bridge).await })
                            .catch_unwind()
                            .await;
                        runtime::finish(
                            finish,
                            match result {
                                Ok(Ok(value)) => Exit::Success(value),
                                Ok(Err(failure)) => failure,
                                Err(panic) => Exit::Die(Defect::from_panic(panic)),
                            },
                        )
                    }))
                })
            }),
        }
    }
    /// Execute under the supplied scope without closing or replacing its resource owner.
    pub fn evaluate(&self, scope: Scope<E>) -> Effect<A, E, R> {
        let run = Arc::clone(&self.run);
        Effect::defer(move || run(scope.clone()))
    }
}
