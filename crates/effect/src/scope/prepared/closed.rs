//! Pre-held close observation on the SAME Inner phase gate. No Event::listen,
//! Box, new waiter allocation or late fallible owner clone occurs during polling.
use super::{Inner, Phase, PreparedScope, ScopeCloseStatus, ScopeLink};
use crate::PreparedRunContext;
use crate::memory::{FramePool, MemoryError, SharedRuntimeFrame};
use std::alloc::Layout;
use std::cell::UnsafeCell;
use std::future::Future;
use std::pin::Pin;
use std::ptr::NonNull;
use std::task::{Context, Poll, Waker};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Idle,
    Linked,
    Queued,
}
struct Links {
    next: Option<NonNull<Node>>,
    mode: Mode,
    waker: Option<Waker>,
    anchor: Option<SharedRuntimeFrame>,
}
struct Node {
    links: UnsafeCell<Links>,
}
// SAFETY: links are accessed only under the owning Inner state gate. The typed
// frame is Send+Sync and its pre-held self anchor retains every registered/queued
// node. No shared reference exposes link mutation to a caller.
unsafe impl Send for Node {}
unsafe impl Sync for Node {}
pub(in crate::scope) struct Waiters {
    head: Option<NonNull<Node>>,
}
// SAFETY: protected by Inner.state; all listed nodes retain their own Send+Sync
// frame anchors until unlink or notification outside the phase gate completes.
unsafe impl Send for Waiters {}
impl Waiters {
    pub(in crate::scope) fn new() -> Self {
        Self { head: None }
    }
}
struct Record<V: Send + 'static, E: Send + 'static> {
    node: Node,
    scope: ScopeLink<E>,
    // Waker/payload and retained scope destruct BEFORE actual root/node owner.
    _context: PreparedRunContext<V, E>,
}
fn inner<V: Send + 'static, E: Send + 'static>(record: &SharedRuntimeFrame) -> &Inner<E> {
    // SAFETY: reservation installs exactly this Record and matching accessor.
    unsafe {
        record
            .pointer()
            .cast::<Record<V, E>>()
            .as_ref()
            .scope
            .inner()
    }
}
/// One owned, physically reserved close observation. Readiness/Drop does not
/// acknowledge a native job; it observes canonical Scope phase/status only.
pub struct PreparedCloseWait<E: Send + 'static> {
    record: SharedRuntimeFrame,
    spare: Option<SharedRuntimeFrame>,
    node: NonNull<Node>,
    inner: for<'a> fn(&'a SharedRuntimeFrame) -> &'a Inner<E>,
    result: Option<ScopeCloseStatus>,
}
// SAFETY: the raw node belongs to this retained Send+Sync frame; every access is
// serialized by its retained scope phase gate, including unlink/notification.
unsafe impl<E: Send + 'static> Send for PreparedCloseWait<E> {}
impl<E: Send + 'static> Unpin for PreparedCloseWait<E> {}
impl<E: Send + 'static> PreparedCloseWait<E> {
    #[must_use]
    pub fn layout<V: Send + 'static>() -> Layout {
        Layout::new::<Record<V, E>>()
    }
    pub(super) fn prepare<V: Send + 'static>(
        scope: ScopeLink<E>,
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self, MemoryError> {
        let reserved = frames.reserve::<Record<V, E>>()?;
        let record = reserved.share(Record {
            node: Node {
                links: UnsafeCell::new(Links {
                    next: None,
                    mode: Mode::Idle,
                    waker: None,
                    anchor: None,
                }),
            },
            scope,
            _context: context.try_clone()?,
        });
        let spare = record.try_clone()?;
        // SAFETY: exact initialized Record layout. This shared origin never
        // creates an aggregate mutable reborrow over queued/link metadata.
        let node = unsafe {
            NonNull::new_unchecked(std::ptr::addr_of_mut!(
                (*record.pointer().cast::<Record<V, E>>().as_ptr()).node
            ))
        };
        Ok(Self {
            record,
            spare: Some(spare),
            node,
            inner: inner::<V, E>,
            result: None,
        })
    }
    fn unlink(&mut self) {
        let inner = (self.inner)(&self.record);
        let (waker, anchor) = {
            let mut state = inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // SAFETY: this record retains node; the phase gate serializes links.
            let mode = unsafe { (*self.node.as_ref().links.get()).mode };
            if mode == Mode::Linked {
                let mut current = state.prepared_waiters.head;
                let mut previous: Option<NonNull<Node>> = None;
                while let Some(node) = current {
                    // SAFETY: every linked node has its registry anchor.
                    let next = unsafe { (*node.as_ref().links.get()).next };
                    if node == self.node {
                        if let Some(previous) = previous {
                            unsafe {
                                (*previous.as_ref().links.get()).next = next;
                            }
                        } else {
                            state.prepared_waiters.head = next;
                        }
                        break;
                    }
                    previous = current;
                    current = next;
                }
                let links = unsafe { &mut *self.node.as_ref().links.get() };
                links.mode = Mode::Idle;
                links.next = None;
                (links.waker.take(), links.anchor.take())
            } else {
                // A detached notification owns queued links/anchor until pop.
                // Removing its Waker does not free a still-queued record.
                let links = unsafe { &mut *self.node.as_ref().links.get() };
                (links.waker.take(), None)
            }
        };
        drop(waker);
        drop(anchor);
    }
}
impl<E: Send + 'static> Future for PreparedCloseWait<E> {
    type Output = ScopeCloseStatus;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(result) = this.result {
            return Poll::Ready(result);
        }
        // Executor cloning can allocate/panic externally; it is not a blanket
        // library zero-allocation guarantee and NEVER occurs under a scope gate.
        let next = cx.waker().clone();
        let inner = (this.inner)(&this.record);
        let (ready, old) = {
            let mut state = inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // SAFETY: owned node and same phase gate as registration/settlement.
            let links = unsafe { &mut *this.node.as_ref().links.get() };
            if links.mode == Mode::Idle && this.spare.is_some() {
                links.anchor = this.spare.take();
                links.next = state.prepared_waiters.head;
                links.mode = Mode::Linked;
                state.prepared_waiters.head = Some(this.node);
            }
            // Arm before checking under the SAME settlement gate. Idle without
            // a spare can only mean a Closed-phase notifier already popped us;
            // that single-use observation must never re-link without an anchor.
            let old = links.waker.replace(next);
            ((state.phase == Phase::Closed).then_some(state.status), old)
        };
        drop(old);
        if let Some(result) = ready {
            this.unlink();
            this.result = Some(result);
            Poll::Ready(result)
        } else {
            Poll::Pending
        }
    }
}
impl<E: Send + 'static> Drop for PreparedCloseWait<E> {
    fn drop(&mut self) {
        self.unlink();
    }
}
impl<E: Send + 'static> PreparedScope<E> {
    /// Acquire actual waiter/frame/scope/root ownership before business work.
    /// Polling never allocates a System Event listener or acquires a new owner.
    pub fn reserve_closed<V: Send + 'static>(
        &self,
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<PreparedCloseWait<E>, MemoryError> {
        PreparedCloseWait::prepare(
            ScopeLink::Prepared(self.inner.try_clone()?),
            frames,
            context,
        )
    }
}

struct Notify<'a, E: Send + 'static> {
    inner: &'a Inner<E>,
    head: Option<NonNull<Node>>,
}
impl<E: Send + 'static> Notify<'_, E> {
    fn run(&mut self) {
        while let Some(node) = self.head {
            let (waker, anchor) = {
                let _state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                // SAFETY: queued nodes each retain their anchor until this pop;
                // phase gate serializes unregister and this ownership transfer.
                let links = unsafe { &mut *node.as_ref().links.get() };
                self.head = links.next.take();
                links.mode = Mode::Idle;
                (links.waker.take(), links.anchor.take())
            };
            let _anchor = anchor;
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}
impl<E: Send + 'static> Drop for Notify<'_, E> {
    fn drop(&mut self) {
        self.run();
    }
}
pub(in crate::scope) fn notify<E: Send + 'static>(inner: &Inner<E>) {
    let head = {
        let mut state = inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let head = state.prepared_waiters.head.take();
        let mut current = head;
        while let Some(node) = current {
            // SAFETY: phase gate and registry anchors protect the entire list.
            let links = unsafe { &mut *node.as_ref().links.get() };
            links.mode = Mode::Queued;
            current = links.next;
        }
        head
    };
    let mut notify = Notify { inner, head };
    notify.run();
}
/// Run prepared notifications even if an existing external Event wake unwinds.
pub(in crate::scope) struct NotifyOnDrop<'a, E: Send + 'static> {
    pub(in crate::scope) inner: &'a Inner<E>,
}
impl<E: Send + 'static> Drop for NotifyOnDrop<'_, E> {
    fn drop(&mut self) {
        notify(self.inner);
    }
}
