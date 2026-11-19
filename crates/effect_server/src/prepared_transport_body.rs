//! Data ownership seam for prepared transport. Header/trailer and connection
//! settlement ownership are separate and are not established by this adapter.
use crate::{Reply, host::NativeOutput};
use axum::body::{Body, Bytes};
use bytes::Buf;
use effect::{LeasedRunResult, memory::MemoryError};
use http_body::{Body as HttpBody, Frame, SizeHint};
use std::{
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

/// Prepared graph ownership or the selected host's native request reservation.
pub(crate) enum TransportOrigin<E: Send + 'static, R: Send + Sync + 'static> {
    Graph {
        origin: LeasedRunResult<(), Reply<E, R>, E>,
        queue: Option<NativeOutput<E, R>>,
    },
    Native(NativeOutput<E, R>),
}

impl<E: Send + 'static, R: Send + Sync + 'static> TransportOrigin<E, R> {
    fn try_retain(&self) -> Result<Self, MemoryError> {
        match self {
            Self::Graph { origin, queue } => Ok(Self::Graph {
                origin: origin.try_retain()?,
                queue: queue.clone(),
            }),
            Self::Native(owner) => Ok(Self::Native(owner.clone())),
        }
    }
}

/// Original data is dropped before its prepared origin guard.
pub(crate) struct RetainedData<E: Send + 'static, R: Send + Sync + 'static> {
    bytes: Bytes,
    _origin: TransportOrigin<E, R>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Buf for RetainedData<E, R> {
    fn remaining(&self) -> usize {
        self.bytes.remaining()
    }
    fn chunk(&self) -> &[u8] {
        self.bytes.chunk()
    }
    fn advance(&mut self, count: usize) {
        self.bytes.advance(count);
    }
}

#[derive(Debug)]
pub(crate) enum PreparedBodyError {
    Body(axum::Error),
    Retain(MemoryError),
}
impl fmt::Display for PreparedBodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Body(error) => write!(f, "response body: {error}"),
            Self::Retain(error) => write!(f, "response origin retention: {error}"),
        }
    }
}
impl std::error::Error for PreparedBodyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Body(error) => Some(error),
            Self::Retain(error) => Some(error),
        }
    }
}

/// Outer data adapter. A yielded data buffer independently retains the origin.
pub(crate) struct PreparedTransportBody<E: Send + 'static, R: Send + Sync + 'static> {
    body: Body,
    origin: TransportOrigin<E, R>,
    failed: bool,
}
// Body is Unpin and the origin is an indirect lease, not a pinned payload.
impl<E: Send + 'static, R: Send + Sync + 'static> Unpin for PreparedTransportBody<E, R> {}
impl<E: Send + 'static, R: Send + Sync + 'static> PreparedTransportBody<E, R> {
    pub(crate) fn new(body: Body, origin: TransportOrigin<E, R>) -> Self {
        Self {
            body,
            origin,
            failed: false,
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> HttpBody for PreparedTransportBody<E, R> {
    type Data = RetainedData<E, R>;
    type Error = PreparedBodyError;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if this.failed {
            return Poll::Ready(None);
        }
        let guard = match this.origin.try_retain() {
            Ok(guard) => guard,
            Err(error) => {
                this.failed = true;
                return Poll::Ready(Some(Err(PreparedBodyError::Retain(error))));
            }
        };
        match Pin::new(&mut this.body).poll_frame(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(Err(error))) => Poll::Ready(Some(Err(PreparedBodyError::Body(error)))),
            Poll::Ready(Some(Ok(frame))) => {
                if frame.data_ref().is_some_and(Bytes::is_empty) {
                    // Empty DATA carries no bytes. Poll the actual body again on a
                    // later turn so EOF, trailers, errors and completion observers
                    // still follow its own contract without an unbounded ready loop.
                    drop(frame);
                    drop(guard);
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Poll::Ready(Some(Ok(frame.map_data(|bytes| RetainedData {
                    bytes,
                    _origin: guard,
                }))))
            }
        }
    }
    fn is_end_stream(&self) -> bool {
        self.failed || self.body.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

/// Outermost response body, selected only after native response transforms.
pub(crate) enum TransportBody<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(Body),
    Prepared(PreparedTransportBody<E, R>),
}
pub(crate) enum TransportData<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(Bytes),
    Prepared(RetainedData<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> Buf for TransportData<E, R> {
    fn remaining(&self) -> usize {
        match self {
            Self::Ordinary(data) => data.remaining(),
            Self::Prepared(data) => data.remaining(),
        }
    }
    fn chunk(&self) -> &[u8] {
        match self {
            Self::Ordinary(data) => data.chunk(),
            Self::Prepared(data) => data.chunk(),
        }
    }
    fn advance(&mut self, count: usize) {
        match self {
            Self::Ordinary(data) => data.advance(count),
            Self::Prepared(data) => data.advance(count),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Unpin for TransportBody<E, R> {}
impl<E: Send + 'static, R: Send + Sync + 'static> HttpBody for TransportBody<E, R> {
    type Data = TransportData<E, R>;
    type Error = PreparedBodyError;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            Self::Ordinary(body) => Pin::new(body).poll_frame(cx).map(|frame| {
                frame.map(|result| {
                    result
                        .map(|frame| frame.map_data(TransportData::Ordinary))
                        .map_err(PreparedBodyError::Body)
                })
            }),
            Self::Prepared(body) => Pin::new(body).poll_frame(cx).map(|frame| {
                frame.map(|result| result.map(|frame| frame.map_data(TransportData::Prepared)))
            }),
        }
    }
    fn is_end_stream(&self) -> bool {
        match self {
            Self::Ordinary(body) => body.is_end_stream(),
            Self::Prepared(body) => body.is_end_stream(),
        }
    }
    fn size_hint(&self) -> SizeHint {
        match self {
            Self::Ordinary(body) => body.size_hint(),
            Self::Prepared(body) => body.size_hint(),
        }
    }
}
