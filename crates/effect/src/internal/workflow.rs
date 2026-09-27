use super::destruction::FlatDrop;
use super::runtime::{self, Context, Finish, Step};
use crate::LibraryDefect;
use crate::{Defect, Effect, Exit, ResourceBinding, ResourceValue};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

type Operation<A, E, R> = Box<dyn FnOnce(Arc<R>, Context, Finish<A, E>) -> Step + Send>;

/// Single-run typed construction used by `effect!`; contains no public runtime access.
pub struct Workflow<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    operation: FlatDrop<Operation<A, E, R>>,
    check_cancellation: bool,
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Workflow<A, E, R> {
    pub(crate) fn new(
        operation: impl FnOnce(Arc<R>, Context, Finish<A, E>) -> Step + Send + 'static,
    ) -> Self {
        Self {
            operation: FlatDrop::new(Box::new(operation)),
            check_cancellation: true,
        }
    }

    pub(crate) fn from_exit(outcome: Exit<A, E>) -> Self {
        let mut workflow = Self::new(move |_, _, finish| runtime::finish(finish, outcome));
        workflow.check_cancellation = false;
        workflow
    }

    pub(crate) fn execute(self, env: Arc<R>, context: Context, finish: Finish<A, E>) -> Step {
        runtime::more(move || {
            if self.check_cancellation && context.cancelled() {
                return runtime::finish(finish, Exit::Interrupted);
            }
            match self.operation.take() {
                Some(operation) => operation(env, context, finish),
                None => runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::WorkflowConsumed)),
                ),
            }
        })
    }
}

/// Move this run's value into its success channel without requiring Clone.
#[must_use]
pub fn success<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    value: A,
) -> Workflow<A, E, R> {
    Workflow::from_exit(Exit::Success(value))
}

/// Single-use lazy block for control-flow expansion.
pub fn defer<A, E, R>(
    factory: impl FnOnce() -> Workflow<A, E, R> + Send + 'static,
) -> Workflow<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Workflow::new(
        move |env, context, finish| match catch_unwind(AssertUnwindSafe(factory)) {
            Ok(workflow) => workflow.execute(env, context, finish),
            Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
        },
    )
}

/// Typed binding; the continuation is invoked once for this run and only on success.
#[must_use]
pub fn bind<P, B, E, R>(
    program: P,
    next: impl FnOnce(P::Value) -> Workflow<B, E, R> + Send + 'static,
) -> Workflow<B, E, R>
where
    P: Bindable<E, R>,
    B: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    program.bind_workflow(next)
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
    impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for super::Workflow<A, E, R>
    {
    }
}

/// Safe macro dispatch for ordinary effects and typed ownership declarations.
pub trait Bindable<E: Send + 'static, R: Send + Sync + 'static>: sealed::Sealed {
    type Value: Send + 'static;
    fn bind_workflow<B: Send + 'static>(
        self,
        next: impl FnOnce(Self::Value) -> Workflow<B, E, R> + Send + 'static,
    ) -> Workflow<B, E, R>;
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Bindable<E, R>
    for Effect<A, E, R>
{
    type Value = A;
    fn bind_workflow<B: Send + 'static>(
        self,
        next: impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static,
    ) -> Workflow<B, E, R> {
        bind_effect(self, next)
    }
}

impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Bindable<E, R>
    for ResourceBinding<T, E, R>
{
    type Value = ResourceValue<T>;
    fn bind_workflow<B: Send + 'static>(
        self,
        next: impl FnOnce(ResourceValue<T>) -> Workflow<B, E, R> + Send + 'static,
    ) -> Workflow<B, E, R> {
        self.bind_run(next)
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Bindable<E, R>
    for Workflow<A, E, R>
{
    type Value = A;
    fn bind_workflow<B: Send + 'static>(
        self,
        next: impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static,
    ) -> Workflow<B, E, R> {
        bind_operation(
            move |env, context, finish| self.execute(env, context, finish),
            next,
        )
    }
}

fn bind_effect<A, B, E, R>(
    program: Effect<A, E, R>,
    next: impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static,
) -> Workflow<B, E, R>
where
    A: Send + 'static,
    B: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    bind_operation(
        move |env, context, finish| program.execute(env, context, finish),
        next,
    )
}

fn bind_operation<A, B, E, R>(
    execute: impl FnOnce(Arc<R>, Context, Finish<A, E>) -> Step + Send + 'static,
    next: impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static,
) -> Workflow<B, E, R>
where
    A: Send + 'static,
    B: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Workflow::new(move |env, context, finish| {
        let next = FlatDrop::new(Box::new(next));
        execute(
            Arc::clone(&env),
            context.clone(),
            runtime::continuation(move |outcome| {
                runtime::more(move || match outcome.into_failure() {
                    Err(failure) => runtime::finish(finish, failure),
                    Ok(_) if context.cancelled() => runtime::finish(finish, Exit::Interrupted),
                    Ok(value) => {
                        let workflow =
                            catch_unwind(AssertUnwindSafe(|| next.take().map(|next| next(value))));
                        match workflow {
                            Ok(Some(workflow)) => workflow.execute(env, context, finish),
                            Ok(None) => runtime::finish(
                                finish,
                                Exit::Die(Defect::from(LibraryDefect::BindingConsumed)),
                            ),
                            Err(panic) => {
                                runtime::finish(finish, Exit::Die(Defect::from_panic(panic)))
                            }
                        }
                    }
                })
            }),
        )
    })
}

/// A fresh workflow factory is invoked lazily on every run.
pub fn reusable<A, E, R>(
    factory: impl Fn() -> Workflow<A, E, R> + Send + Sync + 'static,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Effect::new(
        move |env, context, finish| match catch_unwind(AssertUnwindSafe(&factory)) {
            Ok(workflow) => workflow.execute(env, context, finish),
            Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
        },
    )
}
