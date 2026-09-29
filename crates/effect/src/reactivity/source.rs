//! Effect and stream sources for reactive values.

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
    /// Snapshot value read from this reactive source.
    type Snapshot: Send + 'static;
    /// Typed error from reading the reactive source.
    type ReadError: Send + 'static;
    /// Typed error from observing source changes.
    type StreamError: Send + 'static;

    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Effect<Self::Snapshot, Self::ReadError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn read(&self) -> Effect<Self::Snapshot, Self::ReadError, R>;
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Stream<Self::Snapshot, Self::StreamError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
    fn changes(&self) -> Stream<Self::Snapshot, Self::StreamError, R>;
}

/// A reactive source assembled from a snapshot Effect and a change Stream.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `ReadError`: Error returned while reading the source.
/// - `StreamError`: Error returned by the streaming provider.
/// - `R`: Environment type.
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
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Construct a `EffectStreamSource` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `ReadError`: Error returned while reading the source.
    /// - `StreamError`: Error returned by the streaming provider.
    /// - `A`: Success or application-value type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `read` (`impl Fn() -> Effect<A, ReadError, R> + Send + Sync + 'static`): The read supplied to this operation.
    /// - `changes` (`impl Fn() -> Stream<A, StreamError, R> + Send + Sync + 'static`): The changes supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Type alias for snapshot in reactivity source.
    type Snapshot = A;
    /// Type alias for read error in reactivity source.
    type ReadError = ReadError;
    /// Type alias for stream error in reactivity source.
    type StreamError = StreamError;

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Effect<A, ReadError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn read(&self) -> Effect<A, ReadError, R> {
        (self.read)()
    }
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Stream<A, StreamError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
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
    /// Type alias for snapshot in reactivity source.
    type Snapshot = Arc<AsyncResult<A, AtomError<E>>>;
    /// Type alias for read error in reactivity source.
    type ReadError = AtomError<E>;
    /// Type alias for stream error in reactivity source.
    type StreamError = PubSubError;

    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Effect<Self::Snapshot, Self::ReadError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn read(&self) -> Effect<Self::Snapshot, Self::ReadError, R> {
        Atom::read(self)
    }
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Stream<Self::Snapshot, Self::StreamError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
    fn changes(&self) -> Stream<Self::Snapshot, Self::StreamError, R> {
        Atom::stream(self)
    }
}

impl<A, R> ReactiveStateSource<R> for AtomRef<A>
where
    A: Send + Sync + 'static,
    R: Send + Sync + 'static,
{
    /// Type alias for snapshot in reactivity source.
    type Snapshot = Arc<A>;
    /// Type alias for read error in reactivity source.
    type ReadError = AtomRefError;
    /// Type alias for stream error in reactivity source.
    type StreamError = PubSubError;

    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Effect<Self::Snapshot, Self::ReadError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn read(&self) -> Effect<Self::Snapshot, Self::ReadError, R> {
        AtomRef::read(self)
    }
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Stream<Self::Snapshot, Self::StreamError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
    fn changes(&self) -> Stream<Self::Snapshot, Self::StreamError, R> {
        AtomRef::stream(self)
    }
}

impl<A, R> ReactiveStateSource<R> for TxSubscriptionRef<A>
where
    A: TxValue,
    R: Send + Sync + 'static,
{
    /// Type alias for snapshot in reactivity source.
    type Snapshot = Arc<A>;
    /// Type alias for read error in reactivity source.
    type ReadError = TxSubscriptionError;
    /// Type alias for stream error in reactivity source.
    type StreamError = TxSubscriptionError;

    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Effect<Self::Snapshot, Self::ReadError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn read(&self) -> Effect<Self::Snapshot, Self::ReadError, R> {
        self.read_committed()
    }

    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Stream<Self::Snapshot, Self::StreamError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
    fn changes(&self) -> Stream<Self::Snapshot, Self::StreamError, R> {
        self.stream()
    }
}
