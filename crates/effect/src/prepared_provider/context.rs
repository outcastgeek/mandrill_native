//! Worker-local provider context and concrete synchronous jobs.
use super::PreparedCancellation;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Installed exactly once on each fixed worker, outside all provider locks.
/// The guard never crosses threads. Entry panic is reported by startup before
/// submission becomes available; guard-drop panic is retained by owner join.
/// Clone/entry/guard storage belongs to the named native startup envelope.
pub trait ProviderWorkerContext: Clone + Send + 'static {
    type Guard<'a>
    where
        Self: 'a;
    fn enter(&self) -> Self::Guard<'_>;
}
#[derive(Clone, Copy)]
pub struct PlainWorker;
impl ProviderWorkerContext for PlainWorker {
    type Guard<'a> = ();
    fn enter(&self) {}
}

/// A concrete CPU/blocking job. The closure is constructed only after physical
/// reservation, then executed on a fixed worker. No erased Box/Arc action or
/// extra completion allocation is introduced. Cancellation stays cooperative;
/// even a callback ignoring a request retains its reservation until return.
pub struct SynchronousJob<C> {
    callback: Option<C>,
    cancellation: Option<PreparedCancellation>,
}
impl<C> SynchronousJob<C> {
    pub fn new(callback: C, cancellation: PreparedCancellation) -> Self {
        Self {
            callback: Some(callback),
            cancellation: Some(cancellation),
        }
    }
}
// No field is structurally pinned or projected as Pin; a callback is consumed
// by value on the first poll, just as an FnOnce job is on its worker.
impl<C> Unpin for SynchronousJob<C> {}
impl<C, T> Future for SynchronousJob<C>
where
    C: FnOnce(PreparedCancellation) -> T,
{
    type Output = T;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<T> {
        let this = self.get_mut();
        match (this.callback.take(), this.cancellation.take()) {
            (Some(callback), Some(cancellation)) => Poll::Ready(callback(cancellation)),
            // Future permits Pending after completion; never execute a consumed
            // FnOnce twice or manufacture a cancellation capability.
            (None, None) | (Some(_), None) | (None, Some(_)) => Poll::Pending,
        }
    }
}
