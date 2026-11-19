//! Typed cross-root ownership, erased only after reserving the concrete capture.
//! Notification and terminal reclamation use the existing intrusive frame drain.
use super::context::{Domain, Id, ListenerId};
use super::{PreparedRunContext, Root};
use crate::memory::{
    Bytes, FramePool, MemoryError, PreparedMemory, RegionId, RuntimeFrame, SharedSlot,
};
use std::alloc::Layout;

pub(super) struct View<'a> {
    pub domain: &'a Domain,
    pub id: Id,
}
pub(super) struct Remote {
    owner: RuntimeFrame,
    view: for<'a> fn(&'a RuntimeFrame) -> View<'a>,
    attach: fn(&mut RuntimeFrame, Id),
    retire: fn(&mut RuntimeFrame),
}
impl Remote {
    pub(super) fn view(&self) -> View<'_> {
        (self.view)(&self.owner)
    }
    pub(super) fn attach(&mut self, id: Id) {
        (self.attach)(&mut self.owner, id);
    }
    pub(super) fn retire(&mut self) {
        (self.retire)(&mut self.owner);
    }
}
// SAFETY: only prepare constructs Remote, with a Sync Capture. It is immutable
// while a node reference exists; attach precedes publication, retire follows the
// last reference, and its RuntimeFrame exclusively owns capture destruction.
unsafe impl Sync for Remote {}

pub(super) struct Dispatch {
    owner: RuntimeFrame,
    invoke: fn(RuntimeFrame),
}
impl Dispatch {
    pub(super) fn fire(self) {
        (self.invoke)(self.owner);
    }
}
struct Notify<A: Send + 'static, E: Send + 'static> {
    armed: bool,
    root: SharedSlot<'static, Root<A, E>>,
}
impl<A: Send + 'static, E: Send + 'static> Drop for Notify<A, E> {
    fn drop(&mut self) {
        if self.armed {
            self.root.contexts.notify();
        }
    }
}
fn fire<A: Send + 'static, E: Send + 'static>(owner: RuntimeFrame) {
    // SAFETY: this function pointer is installed only alongside the exact Notify
    // reservation below. The unique, never-pinned capture has no live projection.
    unsafe {
        (*owner.pointer().cast::<Notify<A, E>>().as_ptr()).armed = true;
    }
    // Drop enqueues, not recursively calls another domain. Notify's root owner
    // remains inside its cell until callback/destructor and unlink complete.
    drop(owner);
}
struct Settlement<A: Send + 'static, E: Send + 'static> {
    id: Option<Id>,
    armed: bool,
    root: SharedSlot<'static, Root<A, E>>,
}
impl<A: Send + 'static, E: Send + 'static> Drop for Settlement<A, E> {
    fn drop(&mut self) {
        if self.armed
            && let Some(id) = self.id
        {
            self.root.contexts.finish_releasing(id);
        }
    }
}
struct Capture<TA: Send + 'static, TE: Send + 'static, SA: Send + 'static, SE: Send + 'static> {
    listener: Option<ListenerId>,
    // Source data/context destruction precedes terminal target-node ownership.
    source: PreparedRunContext<SA, SE>,
    settlement: Settlement<TA, TE>,
}
impl<TA: Send + 'static, TE: Send + 'static, SA: Send + 'static, SE: Send + 'static> Drop
    for Capture<TA, TE, SA, SE>
{
    fn drop(&mut self) {
        if let Some(listener) = self.listener.take() {
            self.source.root.contexts.unlink(listener);
        }
    }
}
fn view<TA: Send + 'static, TE: Send + 'static, SA: Send + 'static, SE: Send + 'static>(
    owner: &RuntimeFrame,
) -> View<'_> {
    // SAFETY: exact typed reservation/vtable pairing, immutable initialized
    // capture, retained by the parent's strong node reference.
    let capture = unsafe { owner.pointer().cast::<Capture<TA, TE, SA, SE>>().as_ref() };
    View {
        domain: &capture.source.root.contexts,
        id: capture.source.id,
    }
}
fn attach<TA: Send + 'static, TE: Send + 'static, SA: Send + 'static, SE: Send + 'static>(
    owner: &mut RuntimeFrame,
    id: Id,
) {
    // SAFETY: exact never-pinned type, before publishing any node/view.
    unsafe {
        (*owner.pointer().cast::<Capture<TA, TE, SA, SE>>().as_ptr())
            .settlement
            .id = Some(id);
    }
}
fn retire<TA: Send + 'static, TE: Send + 'static, SA: Send + 'static, SE: Send + 'static>(
    owner: &mut RuntimeFrame,
) {
    // SAFETY: exact type, after all node/view references have ended. Terminal
    // settlement stays in this SAME capture through source payload destruction.
    unsafe {
        (*owner.pointer().cast::<Capture<TA, TE, SA, SE>>().as_ptr())
            .settlement
            .armed = true;
    }
}
/// Separate physical source/notification lanes: a completed notification cannot
/// let another source link bypass the declared bridge-owner quota.
pub(super) struct Storage {
    sources: FramePool<'static>,
    notifications: FramePool<'static>,
}
impl Storage {
    pub(super) fn payload<A: Send + 'static, E: Send + 'static>(
        count: usize,
    ) -> Result<Bytes, MemoryError> {
        FramePool::storage_bytes(count, Layout::new::<Capture<A, E, A, E>>())?.checked_add(
            FramePool::storage_bytes(count, Layout::new::<Notify<A, E>>())?,
        )
    }
    pub(super) fn prepare<A: Send + 'static, E: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            sources: memory.frames(region, count, Layout::new::<Capture<A, E, A, E>>())?,
            notifications: memory.frames(region, count, Layout::new::<Notify<A, E>>())?,
        })
    }
    pub(super) fn in_use(&self) -> bool {
        self.sources.active() != 0 || self.notifications.active() != 0
    }
}
pub(super) fn prepare<
    TA: Send + 'static,
    TE: Send + 'static,
    SA: Send + 'static,
    SE: Send + 'static,
>(
    target: &PreparedRunContext<TA, TE>,
    source: &PreparedRunContext<SA, SE>,
    cancellation: bool,
) -> Result<Remote, MemoryError> {
    // Every concrete capture/callback/owner is obtained before registration or
    // any native factory dispatch; all refusal paths retain rollback ownership.
    let capture = target
        .root
        .bridges
        .sources
        .reserve::<Capture<TA, TE, SA, SE>>()?;
    let notify = if cancellation {
        Some(
            target
                .root
                .bridges
                .notifications
                .reserve::<Notify<TA, TE>>()?,
        )
    } else {
        None
    };
    let source = source.try_clone()?;
    let settlement = Settlement {
        id: None,
        armed: false,
        root: target.root.try_clone()?,
    };
    let notification = match notify {
        Some(reservation) => Some(Dispatch {
            owner: reservation.erase(Notify {
                armed: false,
                root: target.root.try_clone()?,
            }),
            invoke: fire::<TA, TE>,
        }),
        None => None,
    };
    let mut value = Capture {
        listener: None,
        source,
        settlement,
    };
    if let Some(notification) = notification {
        let listener = value.source.root.contexts.listener(value.source.id)?;
        value.listener = Some(listener);
        // Store the unregister owner BEFORE binding can observe a raced request
        // and dispatch a callback; unwind therefore cannot leave registration.
        value
            .source
            .root
            .contexts
            .bind_bridge(listener, notification);
    }
    Ok(Remote {
        owner: capture.erase(value),
        view: view::<TA, TE, SA, SE>,
        attach: attach::<TA, TE, SA, SE>,
        retire: retire::<TA, TE, SA, SE>,
    })
}
