//! Native tag/restore storage adapters, consumed by the existing Step evaluator.
//! Concrete reservations are held before work; no allocation fallback at use.
use super::{Consume, Once};
use crate::Exit;
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::memory::{FramePool, FrameReservation, MemoryError, RuntimeFrame};
use std::alloc::Layout;

pub(crate) enum TaggedStep {
    System(Box<Step>),
    Reserved(RuntimeFrame),
}
impl TaggedStep {
    pub(crate) fn prepared(reservation: FrameReservation<'static, Step>, step: Step) -> Self {
        Self::Reserved(reservation.erase(step))
    }
    pub(crate) fn take(self) -> Step {
        match self {
            Self::System(step) => *step,
            Self::Reserved(mut frame) => {
                // SAFETY: only prepared() creates this variant, with exact Step;
                // the tag is data, never pinned or projected before moving out.
                let step = unsafe { frame.take::<Step>() };
                drop(frame);
                step
            }
        }
    }
}
struct Restore<A: Send + 'static, E: Send + 'static> {
    context: Context,
    invoke: FrameReservation<'static, Invoke<A, E>>,
    tag: FrameReservation<'static, Step>,
    // Retain terminal settlement ownership through all restored captures.
    callback: Finish<A, E>,
}
struct Invoke<A: Send + 'static, E: Send + 'static> {
    outcome: Exit<A, E>,
    callback: Finish<A, E>,
}
impl<A: Send + 'static, E: Send + 'static> Consume<(), Step> for Invoke<A, E> {
    fn consume(self, (): ()) -> Step {
        self.callback.call(self.outcome).unwrap_or(Step::Done)
    }
}
impl<A: Send + 'static, E: Send + 'static> Consume<Exit<A, E>, Step> for Restore<A, E> {
    fn consume(self, outcome: Exit<A, E>) -> Step {
        let next = Step::More(Once::prepared(
            self.invoke,
            Invoke {
                callback: self.callback,
                outcome,
            },
        ));
        runtime::instrument_step_reserved(next, self.context, self.tag)
    }
}
/// Actual metadata layouts consumed by the native entry/restore protocol. These
/// are compiler layouts, not callback execution or a bound on transitive native
/// allocations, shared context pointees or arbitrary nested callbacks.
#[derive(Clone, Copy, Debug)]
pub struct PreparedContextualDemand {
    pub entry_tag: Layout,
    pub restore: Layout,
    pub invoke: Layout,
    pub restore_tag: Layout,
}
impl PreparedContextualDemand {
    pub fn maximum_layout(self) -> Result<Layout, MemoryError> {
        let layouts = [self.entry_tag, self.restore, self.invoke, self.restore_tag];
        let size = layouts.iter().map(Layout::size).max().unwrap_or(0);
        let align = layouts.iter().map(Layout::align).max().unwrap_or(1);
        Layout::from_size_align(size, align)
            .map(|layout| layout.pad_to_align())
            .map_err(|_| MemoryError::InvalidLayout)
    }
}
pub(crate) struct InstrumentationReservation<A: Send + 'static, E: Send + 'static> {
    entry: FrameReservation<'static, Step>,
    restore: FrameReservation<'static, Restore<A, E>>,
    invoke: FrameReservation<'static, Invoke<A, E>>,
    tag: FrameReservation<'static, Step>,
}
impl<A: Send + 'static, E: Send + 'static> InstrumentationReservation<A, E> {
    pub(crate) fn layouts() -> PreparedContextualDemand {
        PreparedContextualDemand {
            entry_tag: Layout::new::<Step>(),
            restore: Layout::new::<Restore<A, E>>(),
            invoke: Layout::new::<Invoke<A, E>>(),
            restore_tag: Layout::new::<Step>(),
        }
    }
    pub(crate) fn reserve(pool: &FramePool<'static>) -> Result<Self, MemoryError> {
        let entry = pool.reserve::<Step>()?;
        let restore = pool.reserve::<Restore<A, E>>()?;
        let invoke = pool.reserve::<Invoke<A, E>>()?;
        let tag = pool.reserve::<Step>()?;
        Ok(Self {
            entry,
            restore,
            invoke,
            tag,
        })
    }
    pub(crate) fn finish(
        self,
        callback: Finish<A, E>,
        context: Context,
    ) -> (Finish<A, E>, FrameReservation<'static, Step>) {
        (
            Once::prepared(
                self.restore,
                Restore {
                    callback,
                    context,
                    invoke: self.invoke,
                    tag: self.tag,
                },
            ),
            self.entry,
        )
    }
}
