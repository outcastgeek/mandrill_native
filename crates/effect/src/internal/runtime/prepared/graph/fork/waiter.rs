//! Bounded physical observer registrations. The task gate serializes links;
//! pre-held self anchors retain queued nodes, never a late clone/Event/Box.
//! This is notification metadata, not a scheduler or computation interpreter.
use super::*;
use crate::memory::{RetainedFrameValue, SharedRuntimeFrame};
use std::alloc::Layout;
use std::cell::UnsafeCell;
use std::ptr::NonNull;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Idle,
    Linked,
    Queued,
}
struct Links {
    next: Option<NonNull<Node>>,
    mode: Mode,
    live: bool,
    waker: Option<Waker>,
    anchor: Option<SharedRuntimeFrame>,
}
struct Node {
    links: UnsafeCell<Links>,
}
// SAFETY: only the owning Task mutex permits link/waker access. Linked/queued
// nodes own pre-admitted anchors; observers never mutate through shared payloads.
unsafe impl Send for Node {}
unsafe impl Sync for Node {}
pub(super) struct Waiters {
    head: Option<NonNull<Node>>,
}
// SAFETY: mutex-protected list; all members retain their own stable frame.
unsafe impl Send for Waiters {}
impl Waiters {
    pub(super) fn new() -> Self {
        Self { head: None }
    }
}
pub(super) struct Waiter {
    _record: SharedRuntimeFrame,
    spare: Option<SharedRuntimeFrame>,
    node: NonNull<Node>,
}
// SAFETY: access is serialized by the retained task gate, including unlink.
unsafe impl Send for Waiter {}
impl Waiter {
    pub(super) fn layout<V: Send + 'static, E: Send + 'static>() -> Layout {
        Layout::new::<RetainedFrameValue<Node, PreparedRunContext<V, E>>>()
    }
    pub(super) fn prepare<V: Send + 'static, E: Send + 'static>(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self, MemoryError> {
        let record = frames
            .reserve::<RetainedFrameValue<Node, PreparedRunContext<V, E>>>()?
            .share_parts(
                Node {
                    links: UnsafeCell::new(Links {
                        next: None,
                        mode: Mode::Idle,
                        live: true,
                        waker: None,
                        anchor: None,
                    }),
                },
                context.try_clone()?,
            );
        let spare = Some(record.try_clone()?);
        // SAFETY: exact retained Node/actual-context frame installed above.
        let node = unsafe {
            RetainedFrameValue::<Node, PreparedRunContext<V, E>>::value_pointer(
                record.pointer().cast(),
            )
        };
        Ok(Self {
            _record: record,
            spare,
            node,
        })
    }
    pub(super) fn listen<A: Send, E: Send, D: ProviderWorkingSet>(
        &mut self,
        task: &Task<A, E, D>,
        cx: &Context<'_>,
    ) -> Result<bool, Defect> {
        let waiter = cx.waker().clone();
        let (ready, old) = task.update(|state| {
            if matches!(state.phase, Phase::Running | Phase::Consumed) {
                return (true, Some(waiter));
            }
            // SAFETY: owning task gate; the observer retains this node.
            let links = unsafe { &mut *self.node.as_ref().links.get() };
            if links.mode == Mode::Idle {
                if links.anchor.is_none() {
                    links.anchor = self.spare.take();
                }
                links.next = state.waiters.head;
                state.waiters.head = Some(self.node);
                links.mode = Mode::Linked;
            }
            (false, links.waker.replace(waiter))
        })?;
        drop(old);
        Ok(ready)
    }
    pub(super) fn unlink<A: Send, E: Send, D: ProviderWorkingSet>(&mut self, task: &Task<A, E, D>) {
        let removed = task.update(|state| {
            // SAFETY: gate serializes list and queued ownership transitions.
            let pointer = unsafe { self.node.as_ref().links.get() };
            unsafe {
                (*pointer).live = false;
            }
            let mode = unsafe { (*pointer).mode };
            if mode == Mode::Queued {
                return (None, None);
            }
            if mode == Mode::Linked {
                let mut current = state.waiters.head;
                let mut previous: Option<NonNull<Node>> = None;
                while let Some(node) = current {
                    let next = unsafe { (*node.as_ref().links.get()).next };
                    if node == self.node {
                        if let Some(previous) = previous {
                            unsafe {
                                (*previous.as_ref().links.get()).next = next;
                            }
                        } else {
                            state.waiters.head = next;
                        }
                        break;
                    }
                    previous = current;
                    current = next;
                }
            }
            // No aggregate mutable borrow spans list traversal (which may read
            // this same node through its registered raw link).
            let links = unsafe { &mut *pointer };
            links.mode = Mode::Idle;
            links.next = None;
            (links.waker.take(), links.anchor.take())
        });
        if let Ok((waker, anchor)) = removed {
            drop(waker);
            drop(anchor);
        }
    }
}
pub(super) struct Notifications<'a, A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet> {
    task: &'a Task<A, E, D>,
    next: Option<NonNull<Node>>,
    cleanup: Option<Waker>,
}
impl<'a, A: Send, E: Send, D: ProviderWorkingSet> Notifications<'a, A, E, D> {
    pub(super) fn take(task: &'a Task<A, E, D>) -> Result<Self, Defect> {
        let (head, cleanup) = task.update(|state| {
            let head = state.waiters.head.take();
            let mut current = head;
            while let Some(node) = current {
                // SAFETY: every linked node has an anchor; owning task gate.
                let links = unsafe { &mut *node.as_ref().links.get() };
                links.mode = Mode::Queued;
                current = links.next;
            }
            (head, state.cleanup_waiter.take())
        })?;
        Ok(Self {
            task,
            next: head,
            cleanup,
        })
    }
    pub(super) fn run(&mut self) {
        while let Some(node) = self.next {
            let taken = self.task.update(|_| {
                // SAFETY: queued anchor retains this node even after observer
                // Drop. Advance before ANY native waker operation can unwind.
                let links = unsafe { &mut *node.as_ref().links.get() };
                self.next = links.next;
                links.next = None;
                links.mode = Mode::Idle;
                (
                    links.waker.take(),
                    if links.live {
                        None
                    } else {
                        links.anchor.take()
                    },
                )
            });
            if let Ok((wake, anchor)) = taken {
                // Live observers keep their pre-held anchor for a future wait;
                // detached nodes keep this local guard through callback return.
                let _anchor = anchor;
                if let Some(wake) = wake {
                    wake.wake();
                }
            } else {
                self.next = None;
            }
        }
        if let Some(wake) = self.cleanup.take() {
            wake.wake();
        }
    }
}
impl<A: Send, E: Send, D: ProviderWorkingSet> Drop for Notifications<'_, A, E, D> {
    fn drop(&mut self) {
        self.run();
    }
}
