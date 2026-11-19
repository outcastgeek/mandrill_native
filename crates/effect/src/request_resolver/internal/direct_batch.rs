use super::super::{HasRequestScope, PreparedBatchRunner, RequestResolverError};
use super::prepared_batch::{CompletionStorage, PreparedValueBatch, clone_with_diagnostics};
use super::{
    BatchEntries, CapacityMapper, ResolverState, failed, finish_failed_batch, settle_batch,
    share_prepared_request,
};
use crate::fiber::prepared_fork_task_layout;
use crate::internal::runtime::{self, Context, PreparedNestedReservation};
use crate::memory::{Bytes, FramePool, MemoryError, PreparedMemory, RegionId};
use crate::{Defect, Exit, HasTaskSpawner, RequestGroup, SharedValue};
use futures_util::FutureExt;
use std::alloc::Layout;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Weak};

type Evaluation<Q, A, E> = PreparedNestedReservation<super::PreparedRequestBatch<Q, A, E>, E>;

pub(super) struct PreparedInvocation<Q: Send + 'static, A: Send + 'static, E: Send + 'static> {
    values: PreparedValueBatch<Q, A, E>,
    evaluation: Evaluation<Q, A, E>,
}

impl<Q: Send + 'static, A: Send + 'static, E: Send + 'static> PreparedInvocation<Q, A, E> {
    fn frames(count: usize) -> Result<(usize, Layout), MemoryError> {
        let (output, store) = Evaluation::<Q, A, E>::layouts();
        let layout = Layout::from_size_align(
            output.size().max(store.size()),
            output.align().max(store.align()),
        )
        .map_err(|_| MemoryError::InvalidLayout)?;
        Ok((
            count
                .checked_mul(2)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            layout,
        ))
    }

    pub(super) fn storage_bytes(count: usize) -> Result<Bytes, MemoryError> {
        let (count, layout) = Self::frames(count)?;
        FramePool::storage_bytes(count, layout)
    }

    pub(super) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
    ) -> Result<FramePool<'static>, MemoryError> {
        let (count, layout) = Self::frames(count)?;
        memory.frames(region, count, layout)
    }

    pub(super) fn reserve(
        pool: &FramePool<'static>,
        values: PreparedValueBatch<Q, A, E>,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            values,
            evaluation: Evaluation::reserve(pool)?,
        })
    }
}

pub(super) struct BatchTracking<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
{
    pub(super) entries: BatchEntries<A, E>,
    pub(super) owner: Weak<ResolverState<Q, A, E, R>>,
}

impl<Q, A, E, R> Drop for BatchTracking<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
{
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            for entry in self.entries.entries() {
                owner.forget(entry);
            }
        }
    }
}

pub(super) struct PreparedBatchTask<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
{
    pub(super) group: RequestGroup,
    pub(super) invocation: PreparedInvocation<Q, A, E>,
    pub(super) tracking: BatchTracking<Q, A, E, R>,
    pub(super) run: Arc<PreparedBatchRunner<Q, A, E, R>>,
    pub(super) capacity: Arc<CapacityMapper<E>>,
    pub(super) env: Arc<R>,
    pub(super) context: Context,
}

impl<Q, A, E, R> PreparedBatchTask<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
{
    pub(super) fn layout() -> Layout {
        prepared_fork_task_layout(Self::run)
    }

    pub(super) async fn run(self) -> Exit<(), RequestResolverError<E>> {
        let Self {
            group,
            invocation,
            tracking,
            run,
            capacity,
            env,
            context,
        } = self;
        if context.cancelled() {
            return Exit::Interrupted;
        }
        let entries = tracking.entries.entries();
        let capacity_failure = |error| finish_failed_batch(entries, Exit::Fail(capacity(error)));
        let work = async {
            let PreparedInvocation { values, evaluation } = invocation;
            let PreparedValueBatch {
                writer,
                expected,
                first_copy,
            } = values;
            let program = run(group, writer);
            let outcome = runtime::evaluate_context_reserved(&context, evaluation, |finish| {
                program.execute(env, context.clone(), finish)
            })
            .await;
            match outcome.into_failure() {
                Ok(mut outcomes) => {
                    if !expected.accepts(&outcomes) {
                        return capacity_failure(MemoryError::InvalidRange);
                    }
                    let count = match outcomes.written() {
                        Ok(count) => count,
                        Err(error) => return capacity_failure(error),
                    };
                    if count != entries.len() {
                        return settle_batch(entries, count, std::iter::empty());
                    }
                    if !outcomes.verify_outcomes() {
                        return capacity_failure(MemoryError::MissingRuntimeStorage);
                    }
                    for (offset, entry) in entries.iter().enumerate() {
                        let (Some(result), Some(completion)) = (
                            outcomes.take_outcome(offset),
                            expected.take_completion(offset),
                        ) else {
                            return capacity_failure(MemoryError::MissingRuntimeStorage);
                        };
                        entry.complete(share_prepared_request(result, completion));
                    }
                    Exit::Success(())
                }
                Err(failure) => {
                    if !expected.verify_completions() {
                        return capacity_failure(MemoryError::MissingRuntimeStorage);
                    }
                    let Some(completion) = expected.take_completion(0) else {
                        return capacity_failure(MemoryError::MissingRuntimeStorage);
                    };
                    let (failure, value) =
                        completion.share::<()>(failure, RequestResolverError::BatchFailed);
                    drop(value);
                    let mut first_copy = Some(first_copy);
                    for (offset, entry) in entries.iter().enumerate() {
                        let mut copy = if offset == 0 {
                            first_copy.take()
                        } else {
                            expected
                                .take_completion(offset)
                                .map(CompletionStorage::into_diagnostics)
                        };
                        let observed = clone_with_diagnostics(&failure, &mut copy);
                        entry.complete(failed::<_, SharedValue<A>, _>(observed));
                    }
                    failure
                }
            }
        };
        let outcome = AssertUnwindSafe(work)
            .catch_unwind()
            .await
            .unwrap_or_else(|panic| {
                let defect = Defect::from_panic(panic);
                finish_failed_batch(entries, Exit::Die(defect))
            });
        drop(tracking);
        outcome
    }
}
