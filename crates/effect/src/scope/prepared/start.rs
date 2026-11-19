//! One common ownership handoff for dedicated and reusable prepared scopes.
use super::{PreparedScope, PreparedScopeOwner};
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::scope::{Scope, link::ScopeLink};
use crate::{Effect, Exit};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

type AdmittedScope<E> = (
    PreparedScopeOwner<E>,
    PreparedScope<E>,
    ScopeLink<E>,
    crate::CleanupFailures<E>,
    crate::CleanupFailures<E>,
    Option<crate::PreparedPanicReservation>,
);

pub(super) fn start<A, E, R>(
    admitted: AdmittedScope<E>,
    body: impl FnOnce(PreparedScope<E>) -> Result<Effect<A, E, R>, Exit<A, E>>,
    environment: Arc<R>,
    context: Context,
    finish: Finish<A, E>,
) -> Step
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let (mut owner, handle, closing, cleanup_buffer, close_buffer, panic) = admitted;
    let body_context = context.clone();
    let after_body = runtime::continuation(move |result| {
        let result = runtime::publish(result, &context);
        let join_context = context.clone();
        closing.join_close_prepared(
            result,
            runtime::continuation(move |result| {
                let cleanup = Scope::<E>::take_outcome_inner(&owner.scope.inner, false);
                owner.settled = true;
                drop(owner);
                let mut accumulator = crate::PreparedCleanupAccumulator::from_reserved(
                    runtime::publish(result, &join_context),
                    cleanup_buffer,
                );
                accumulator.append_cleanup(cleanup);
                runtime::finish(finish, accumulator.finish())
            }),
            close_buffer,
        )
    });
    match catch_unwind(AssertUnwindSafe(|| body(handle))) {
        Ok(Ok(body)) => body.execute(environment, body_context, after_body),
        Ok(Err(outcome)) => runtime::finish(after_body, outcome),
        Err(payload) => runtime::finish(
            after_body,
            Exit::Die(crate::internal::runtime::panic_defect(panic, payload)),
        ),
    }
}
