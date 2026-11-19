//! System-or-reserved owners at the existing trampoline's storage boundary.
//! Prepared constructors consume checked physical reservations; none maps a
//! capacity error into Die or falls back to system storage.
use super::destruction::FlatDrop;
use super::runtime::Step;
use crate::memory::{FrameReservation, RuntimeFrame};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

mod native_context;
mod prepared_future;
pub use native_context::PreparedContextualDemand;
pub(crate) use native_context::{InstrumentationReservation, TaggedStep};
pub(crate) use prepared_future::{FutureReservation, prepare_future};

pub(crate) trait Consume<A, B> {
    fn consume(self, input: A) -> B;
}

/// Reserved-only one-shot handoff used by generated native adapters. Its
/// storage and typed callback are paired at construction; there is no System
/// or heap fallback.
#[doc(hidden)]
pub struct ReservedOnce<A: 'static, B: 'static> {
    frame: RuntimeFrame,
    call: unsafe fn(RuntimeFrame, A) -> B,
}
struct ReservedHandoff<T, A, B> {
    value: T,
    call: fn(T, A) -> B,
}
impl<T, A, B> Consume<A, B> for ReservedHandoff<T, A, B> {
    fn consume(self, input: A) -> B {
        (self.call)(self.value, input)
    }
}
/// Uninitialized concrete backing for one reserved-only handoff.
#[doc(hidden)]
pub struct ReservedOnceReservation<T: Send + 'static, A: 'static, B: 'static> {
    frame: FrameReservation<'static, ReservedHandoff<T, A, B>>,
}
impl<T: Send + 'static, A: 'static, B: 'static> ReservedOnceReservation<T, A, B> {
    pub fn initialize(self, value: T, call: fn(T, A) -> B) -> ReservedOnce<A, B> {
        ReservedOnce::from_prepared(self.frame, ReservedHandoff { value, call })
    }
}
impl<A: 'static, B: 'static> ReservedOnce<A, B> {
    #[must_use]
    pub fn layout<T: Send + 'static>() -> std::alloc::Layout {
        std::alloc::Layout::new::<ReservedHandoff<T, A, B>>()
    }
    pub fn reserve<T: Send + 'static>(
        frames: &crate::memory::FramePool<'static>,
    ) -> Result<ReservedOnceReservation<T, A, B>, crate::memory::MemoryError> {
        Ok(ReservedOnceReservation {
            frame: frames.reserve()?,
        })
    }
    fn from_prepared<T: Consume<A, B> + Send + 'static>(
        reservation: FrameReservation<'static, T>,
        value: T,
    ) -> Self {
        unsafe fn invoke<A, B, T: Consume<A, B>>(mut frame: RuntimeFrame, input: A) -> B {
            // SAFETY: the constructor pairs this exact never-pinned T with the frame.
            let value = unsafe { frame.take::<T>() };
            let output = value.consume(input);
            drop(frame);
            output
        }
        Self {
            frame: reservation.erase(value),
            call: invoke::<A, B, T>,
        }
    }
    pub fn call(self, input: A) -> B {
        // SAFETY: the constructor pairs the callback with its exact frame.
        unsafe { (self.call)(self.frame, input) }
    }
}
impl<A, B, F: FnOnce(A) -> B> Consume<A, B> for F {
    fn consume(self, input: A) -> B {
        self(input)
    }
}

/// Typed enclosing payload proves its guard fits the same callback allocation.
/// Initially both fields are owned here; call_with leaves guard initialized in
/// that physical frame until the terminal owner completes or is abandoned.
pub(crate) struct Settlement<T, G> {
    pub(crate) operation: T,
    pub(crate) guard: G,
}

type InputBuilder<'a, A> = &'a mut dyn FnMut(Option<RuntimeFrame>) -> Option<A>;
type RetainedCall<A, B> = for<'a> unsafe fn(RuntimeFrame, InputBuilder<'a, A>) -> Option<B>;

struct Retained<F> {
    callback: F,
    frame: Option<RuntimeFrame>,
}
impl<F> Retained<F> {
    fn invoke<A, B>(self, input: A) -> B
    where
        F: FnOnce(A) -> B,
    {
        let output = (self.callback)(input);
        drop(self.frame);
        output
    }
}

pub(crate) enum Once<A: 'static, B: 'static> {
    System(FlatDrop<Box<dyn FnOnce(A) -> B + Send>>),
    Reserved {
        frame: RuntimeFrame,
        call: unsafe fn(RuntimeFrame, A) -> B,
        retained_call: RetainedCall<A, B>,
    },
}
impl<A: 'static, B: 'static> Once<A, B> {
    pub(crate) fn system(callback: impl FnOnce(A) -> B + Send + 'static) -> Self {
        Self::System(FlatDrop::new(Box::new(callback)))
    }
    pub(crate) fn prepared<T: Consume<A, B> + Send + 'static>(
        reservation: FrameReservation<'static, T>,
        value: T,
    ) -> Self {
        unsafe fn invoke_retained<A, B, T: Consume<A, B>>(
            mut frame: RuntimeFrame,
            build: InputBuilder<'_, A>,
        ) -> Option<B> {
            // SAFETY: same paired, never-pinned callback T. The builder is called
            // synchronously once through a borrowed trait object (no heap box).
            // It transfers this empty physical frame into the terminal owner.
            let value = unsafe { frame.take::<T>() };
            let input = build(Some(frame))?;
            Some(value.consume(input))
        }
        let ReservedOnce { frame, call } = ReservedOnce::from_prepared(reservation, value);
        Self::Reserved {
            frame,
            call,
            retained_call: invoke_retained::<A, B, T>,
        }
    }
    pub(crate) fn prepared_settlement<T: Consume<A, B> + Send + 'static, G: Send + 'static>(
        reservation: FrameReservation<'static, Settlement<T, G>>,
        value: Settlement<T, G>,
    ) -> Self {
        unsafe fn invoke<A, B, T: Consume<A, B>, G>(mut frame: RuntimeFrame, input: A) -> B {
            // SAFETY: paired constructor's exact, never-pinned enclosing type.
            let Settlement { operation, guard } = unsafe { frame.take::<Settlement<T, G>>() };
            let output = operation.consume(input);
            drop(guard);
            drop(frame);
            output
        }
        unsafe fn retained<A, B, T: Consume<A, B>, G: Send + 'static>(
            mut frame: RuntimeFrame,
            build: InputBuilder<'_, A>,
        ) -> Option<B> {
            // SAFETY: exact enclosing type, moved once before any pin exists.
            let Settlement { operation, guard } = unsafe { frame.take::<Settlement<T, G>>() };
            // SAFETY: G is a field of the reserved, unpacked Settlement<T,G>:
            // its size/alignment fit that original reservation. The old payload
            // is now on the stack and marked absent. G is installed before the
            // builder receives ownership; no borrow crosses frame movement.
            unsafe { frame.replace_unpinned(guard) };
            let input = build(Some(frame))?;
            Some(operation.consume(input))
        }
        Self::Reserved {
            frame: reservation.erase(value),
            call: invoke::<A, B, T, G>,
            retained_call: retained::<A, B, T, G>,
        }
    }
    pub(crate) fn retained_system(
        callback: impl FnOnce(A) -> B + Send + 'static,
        frame: Option<RuntimeFrame>,
    ) -> Self {
        // One existing continuation owner, not a second allocation. Struct field
        // order drops callback captures before its physical reservation on abandon.
        let retained = Retained { callback, frame };
        Self::system(move |input| retained.invoke(input))
    }
    pub(crate) fn call_with(self, build: impl FnOnce(Option<RuntimeFrame>) -> A) -> Option<B> {
        let mut build = Some(build);
        let mut adapter = |frame| build.take().map(|build| build(frame));
        match self {
            Self::System(callback) => {
                let input = adapter(None)?;
                callback.take().map(|callback| callback(input))
            }
            Self::Reserved {
                frame,
                retained_call,
                ..
            } => {
                // SAFETY: constructor paired frame T and the retained call shim.
                unsafe { retained_call(frame, &mut adapter) }
            }
        }
    }
    pub(crate) fn call(self, input: A) -> Option<B> {
        match self {
            Self::System(callback) => callback.take().map(|callback| callback(input)),
            Self::Reserved { frame, call, .. } => {
                // SAFETY: call function and frame type are paired by prepared.
                Some(unsafe { call(frame, input) })
            }
        }
    }
}

pub(crate) enum Pending {
    System(Pin<Box<dyn Future<Output = Step> + Send>>),
    Reserved {
        frame: RuntimeFrame,
        poll: unsafe fn(&mut RuntimeFrame, &mut Context<'_>) -> Poll<Step>,
    },
}
impl Pending {
    pub(crate) fn prepared<F: Future<Output = Step> + Send + 'static>(
        reservation: FrameReservation<'static, F>,
        future: F,
    ) -> Self {
        unsafe fn poll<F: Future<Output = Step>>(
            frame: &mut RuntimeFrame,
            context: &mut Context<'_>,
        ) -> Poll<Step> {
            // SAFETY: paired concrete F was initialized at its final stable
            // address. Only poll gets mutable pinned access; Drop is in place.
            unsafe { Pin::new_unchecked(frame.pointer().cast::<F>().as_mut()).poll(context) }
        }
        Self::Reserved {
            frame: reservation.erase(future),
            poll: poll::<F>,
        }
    }
    pub(crate) fn poll(&mut self, context: &mut Context<'_>) -> Poll<Step> {
        match self {
            Self::System(future) => future.as_mut().poll(context),
            Self::Reserved { frame, poll } => {
                // SAFETY: initialized frame and poll function were paired above.
                unsafe { poll(frame, context) }
            }
        }
    }
}
