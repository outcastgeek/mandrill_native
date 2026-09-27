use super::runtime::{self, Context, Step};
use super::workflow::Workflow;
use crate::scope::Registered;
use crate::{Defect, Effect, Exit, LibraryDefect, ResourceBinding, ResourceValue, Scope};
use futures_util::FutureExt;
use std::future::{Future, poll_fn};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::task::Poll;

/// Generated-code bridge. Contains run-derived state, never provider settings.
#[doc(hidden)]
pub struct MacroContext<E: Send + 'static, R: Send + Sync + 'static> {
    env: Arc<R>,
    context: Context,
    scope: Scope<E>,
}
mod sealed {
    pub trait Sealed {}
    impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for crate::Effect<A, E, R>
    {
    }
    impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for crate::ResourceBinding<T, E, R>
    {
    }
}
#[doc(hidden)]
pub trait AsyncBindable<E: Send + 'static, R: Send + Sync + 'static>: sealed::Sealed {
    type Value: Send + 'static;
    fn into_scoped_effect(self, scope: Scope<E>) -> Effect<Self::Value, E, R>;
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> AsyncBindable<E, R>
    for Effect<A, E, R>
{
    type Value = A;
    fn into_scoped_effect(self, _: Scope<E>) -> Self {
        self
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> AsyncBindable<E, R>
    for ResourceBinding<T, E, R>
{
    type Value = ResourceValue<T>;
    fn into_scoped_effect(self, scope: Scope<E>) -> Effect<Self::Value, E, R> {
        self.acquire_scoped(scope)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> MacroContext<E, R> {
    pub async fn bind<P: AsyncBindable<E, R>, A: Send + 'static>(
        &self,
        program: P,
    ) -> Result<P::Value, Exit<A, E>> {
        runtime::evaluate_composed(
            program.into_scoped_effect(self.scope.clone()),
            Arc::clone(&self.env),
            &self.context,
        )
        .await
        .into_failure()
    }
    #[must_use]
    pub fn checkpoint(&self) -> usize {
        self.scope.checkpoint()
    }
    pub async fn release<A: Send + 'static>(&self, checkpoint: usize) -> Result<(), Exit<A, E>> {
        let releases = self.scope.take_since(checkpoint)?;
        if releases.is_empty() {
            return Ok(());
        }
        let context = self.context.protected();
        evaluate(
            release_work(releases, Exit::Success(())),
            Arc::clone(&self.env),
            &context,
        )
        .await
        .into_failure()
    }
    pub async fn cooperate<A: Send + 'static>(&self) -> Result<(), Exit<A, E>> {
        enum Turn {
            Yield,
            Resume,
        }
        let mut turn = Turn::Yield;
        poll_fn(|cx| {
            if self.context.cancelled() {
                return Poll::Ready(Err(Exit::Interrupted));
            }
            match turn {
                Turn::Yield => {
                    turn = Turn::Resume;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                Turn::Resume => Poll::Ready(Ok(())),
            }
        })
        .await
    }
}
fn release_work<E: Send + 'static, R: Send + Sync + 'static>(
    mut releases: Vec<Registered<E>>,
    cleanup: Exit<(), E>,
) -> Workflow<(), E, R> {
    Workflow::new(move |env, context, finish| match releases.pop() {
        Some(release) => release.execute(runtime::continuation(move |result| {
            release_work(releases, super::outcome::append_cleanup(cleanup, result))
                .execute(env, context, finish)
        })),
        None => runtime::finish(finish, cleanup),
    })
}
async fn evaluate<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    workflow: Workflow<A, E, R>,
    env: Arc<R>,
    context: &Context,
) -> Exit<A, E> {
    let candidate = Mutex::new(Some(workflow));
    let program = Effect::new(move |env, context, finish| {
        match candidate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            Some(workflow) => workflow.execute(env, context, finish),
            None => runtime::finish(
                finish,
                Exit::Die(Defect::from(LibraryDefect::WorkflowConsumed)),
            ),
        }
    });
    runtime::evaluate_composed(program, env, context).await
}
#[doc(hidden)]
pub fn async_workflow<A, E, R, F>(
    factory: impl FnOnce(MacroContext<E, R>) -> F + Send + 'static,
) -> Workflow<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Future<Output = Exit<A, E>> + Send + 'static,
{
    Workflow::new(move |env, context, finish| {
        let scope = Scope::new();
        let macro_context = MacroContext {
            env: Arc::clone(&env),
            context: context.clone(),
            scope: scope.clone(),
        };
        let body = Workflow::new(move |_, _, finish| {
            Step::Composed(Box::pin(async move {
                let result = AssertUnwindSafe(async { factory(macro_context).await })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, result)
            }))
        });
        scope.execute_workflow(body, env, context, finish)
    })
}
