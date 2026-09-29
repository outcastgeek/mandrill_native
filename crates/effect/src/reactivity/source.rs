use super::{AsyncResult, Atom, AtomError, AtomRef, AtomRefError};
use crate::transactional::{TxSubscriptionError, TxSubscriptionRef, TxValue};
use crate::{Effect, PubSubError, Stream};
use std::sync::Arc;

/// Server adapters can observe a state source without knowing whether it is
/// an atom, persistence-backed view, RPC query or external capability.
/// A source that promises an initial snapshot must establish its subscription
/// boundary inside `changes`; a separate `read` followed by `changes` is not
/// an atomic observation protocol.
pub trait ReactiveStateSource<R: Send + Sync + 'static> {
    type Snapshot: Send + 'static;
    type ReadError: Send + 'static;
    type StreamError: Send + 'static;

    fn read(&self) -> Effect<Self::Snapshot, Self::ReadError, R>;
    fn changes(&self) -> Stream<Self::Snapshot, Self::StreamError, R>;
}

pub struct EffectStreamSource<A, ReadError, StreamError, R>
where
    A: Send + 'static,
    ReadError: Send + 'static,
    StreamError: Send + 'static,
    R: Send + Sync + 'static,
{
    read: Arc<dyn Fn() -> Effect<A, ReadError, R> + Send + Sync>,
    changes: Arc<dyn Fn() -> Stream<A, StreamError, R> + Send + Sync>,
}

impl<A, ReadError, StreamError, R> Clone for EffectStreamSource<A, ReadError, StreamError, R>
where
    A: Send + 'static,
    ReadError: Send + 'static,
    StreamError: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            read: Arc::clone(&self.read),
            changes: Arc::clone(&self.changes),
        }
    }
}

impl<A, ReadError, StreamError, R> EffectStreamSource<A, ReadError, StreamError, R>
where
    A: Send + 'static,
    ReadError: Send + 'static,
    StreamError: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    pub fn new(
        read: impl Fn() -> Effect<A, ReadError, R> + Send + Sync + 'static,
        changes: impl Fn() -> Stream<A, StreamError, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Arc::new(read),
            changes: Arc::new(changes),
        }
    }
}

impl<A, ReadError, StreamError, R> ReactiveStateSource<R>
    for EffectStreamSource<A, ReadError, StreamError, R>
where
    A: Send + 'static,
    ReadError: Send + 'static,
    StreamError: Send + 'static,
    R: Send + Sync + 'static,
{
    type Snapshot = A;
    type ReadError = ReadError;
    type StreamError = StreamError;

    fn read(&self) -> Effect<A, ReadError, R> {
        (self.read)()
    }
    fn changes(&self) -> Stream<A, StreamError, R> {
        (self.changes)()
    }
}

impl<A, E, R> ReactiveStateSource<R> for Atom<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    type Snapshot = Arc<AsyncResult<A, AtomError<E>>>;
    type ReadError = AtomError<E>;
    type StreamError = PubSubError;

    fn read(&self) -> Effect<Self::Snapshot, Self::ReadError, R> {
        Atom::read(self)
    }
    fn changes(&self) -> Stream<Self::Snapshot, Self::StreamError, R> {
        Atom::stream(self)
    }
}

impl<A, R> ReactiveStateSource<R> for AtomRef<A>
where
    A: Send + Sync + 'static,
    R: Send + Sync + 'static,
{
    type Snapshot = Arc<A>;
    type ReadError = AtomRefError;
    type StreamError = PubSubError;

    fn read(&self) -> Effect<Self::Snapshot, Self::ReadError, R> {
        AtomRef::read(self)
    }
    fn changes(&self) -> Stream<Self::Snapshot, Self::StreamError, R> {
        AtomRef::stream(self)
    }
}

impl<A, R> ReactiveStateSource<R> for TxSubscriptionRef<A>
where
    A: TxValue,
    R: Send + Sync + 'static,
{
    type Snapshot = Arc<A>;
    type ReadError = TxSubscriptionError;
    type StreamError = TxSubscriptionError;

    fn read(&self) -> Effect<Self::Snapshot, Self::ReadError, R> {
        self.read_committed()
    }

    fn changes(&self) -> Stream<Self::Snapshot, Self::StreamError, R> {
        self.stream()
    }
}
