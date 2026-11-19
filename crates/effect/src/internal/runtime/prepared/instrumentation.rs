use crate::memory::{FramePool, MemoryError, SharedRuntimeFrame};
use crate::{PollScope, PreparedTraceContext};
use std::alloc::Layout;

/// Enter a prepared run's native polling context with a concrete stack guard.
/// Hook captures and any native allocations made by the implementation remain
/// part of the caller's working-set contract. A best-effort hook can isolate
/// its own failures before entering or releasing the guard.
pub trait PreparedPollHook: Send + Sync + 'static {
    type Guard<'a>: PollScope + 'a
    where
        Self: 'a;

    /// Enter for this transition, borrowing the operation's selected trace.
    fn enter<'a>(&'a self, trace: Option<&'a PreparedTraceContext>) -> Self::Guard<'a>;
}

type Enter = fn(&SharedRuntimeFrame, Option<&PreparedTraceContext>, &mut dyn FnMut());

/// Immutable shared hook in a caller-prepared typed frame.
pub struct PreparedPollInstrumentation {
    hook: SharedRuntimeFrame,
    enter: Enter,
}

impl PreparedPollInstrumentation {
    /// Exact inline hook storage for one startup frame. Captured pointees
    /// and the polling thread's stack guard have independent ownership.
    #[must_use]
    pub fn layout<H: PreparedPollHook>() -> Layout {
        Layout::new::<H>()
    }

    /// Install a concrete hook once. Refusal returns the original hook.
    pub fn prepare<H: PreparedPollHook>(
        frames: &FramePool<'static>,
        hook: H,
    ) -> Result<Self, (MemoryError, H)> {
        let frame = match frames.reserve::<H>() {
            Ok(frame) => frame,
            Err(error) => return Err((error, hook)),
        };
        Ok(Self {
            hook: frame.share(hook),
            enter: enter::<H>,
        })
    }

    pub(super) fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            hook: self.hook.try_clone()?,
            enter: self.enter,
        })
    }

    #[expect(clippy::expect_used, reason = "the private enter thunk invokes its callback exactly once")]
    pub(crate) fn with_scope<T>(
        &self,
        trace: Option<&PreparedTraceContext>,
        run: impl FnOnce() -> T,
    ) -> T {
        let mut run = Some(run);
        let mut result = None;
        (self.enter)(&self.hook, trace, &mut || {
            result = run.take().map(|run| run());
        });
        result.expect("prepared poll transition ran")
    }
}

fn enter<H: PreparedPollHook>(
    frame: &SharedRuntimeFrame,
    trace: Option<&PreparedTraceContext>,
    run: &mut dyn FnMut(),
) {
    // SAFETY: prepare pairs this thunk with a live immutable H in that frame.
    let hook = unsafe { frame.pointer().cast::<H>().as_ref() };
    let _guard = hook.enter(trace);
    run();
}
