//! Cancellation state and notifications for Effect execution.

use crate::internal::destruction::FlatDrop;
use crate::internal::notification::notify_all;
use crate::memory::prepared_notification;
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool,
    WeakSlot,
};
use event_listener::Event;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::{future::Future, ops::Deref};

#[derive(Default)]
struct State {
    requested: AtomicBool,
    changed: Event,
    observers: Option<prepared_notification::Event>,
    children: Mutex<Children>,
    parent: Mutex<Option<Parent>>,
    // Only the invocation that first marks this state may link it.
    work: Mutex<Option<Cancellation>>,
}

type PreparedChildren = SlotLease<'static, FixedVector<'static, Link>>;

#[derive(Default)]
struct Children {
    ordinary: Vec<Link>,
    prepared: Option<PreparedChildren>,
}

enum Child {
    Ordinary(Weak<State>),
    Prepared(WeakSlot<'static, State>),
}

struct Link {
    address: usize,
    child: Child,
}

impl Link {
    fn upgrade(&self) -> Option<Cancellation> {
        match &self.child {
            Child::Ordinary(child) => child.upgrade().map(|state| Cancellation {
                state: Node::Ordinary(state),
            }),
            Child::Prepared(child) => child.upgrade().map(|state| Cancellation {
                state: Node::Prepared(Some(state)),
            }),
        }
    }
}

enum Parent {
    Ordinary(FlatDrop<Cancellation>),
    // Selected children attach to one startup anchor, which retains its own
    // parent through the flattened ordinary destruction path.
    Anchor(Cancellation),
}

impl Parent {
    fn node(&self) -> Option<&Cancellation> {
        match self {
            Self::Ordinary(parent) => parent.as_ref(),
            Self::Anchor(anchor) => Some(anchor),
        }
    }
}

enum Node {
    Ordinary(Arc<State>),
    Prepared(Option<SharedSlot<'static, State>>),
}
impl Clone for Node {
    #[allow(clippy::expect_used)] // The live owner exists; Clone shares Arc's representable reference bound.
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(state) => Self::Ordinary(state.clone()),
            Self::Prepared(state) => Self::Prepared(Some(
                state
                    .as_ref()
                    .expect("live cancellation node")
                    .try_clone()
                    .expect("cancellation owner limit"),
            )),
        }
    }
}
impl Deref for Node {
    type Target = State;
    #[allow(clippy::expect_used)] // The owner is present until its own destructor runs.
    fn deref(&self) -> &State {
        match self {
            Self::Ordinary(state) => state,
            Self::Prepared(state) => state.as_ref().expect("live cancellation node"),
        }
    }
}
impl Drop for Node {
    fn drop(&mut self) {
        if let Self::Prepared(state) = self
            && let Some(state) = state.take().and_then(SharedSlot::into_last)
        {
            state.unlink();
            state.requested.store(false, Ordering::Relaxed);
        }
    }
}

/// Invocation-owned intrusive lists. Every linked state has won its only
/// false-to-true transition; the flag never resets, so no other propagation
/// invocation may use its work link. Pending is LIFO and changed is FIFO.
struct Propagation {
    pending: Option<Cancellation>,
    changed: Option<Cancellation>,
    changed_tail: Option<Cancellation>,
}
impl Propagation {
    fn new(root: Cancellation) -> Self {
        Self {
            pending: Some(root),
            changed: None,
            changed_tail: None,
        }
    }
    fn push_pending(&mut self, state: Cancellation) {
        let next = self.pending.take();
        let displaced = {
            let mut link = state
                .state
                .work
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::replace(&mut *link, next)
        };
        drop(displaced);
        self.pending = Some(state);
    }
    fn pop_pending(&mut self) -> Option<Cancellation> {
        let state = self.pending.take()?;
        let next = state
            .state
            .work
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        self.pending = next;
        Some(state)
    }
    fn push_changed(&mut self, state: Cancellation) {
        if let Some(tail) = self.changed_tail.take() {
            let next = state.clone();
            let displaced = {
                let mut link = tail
                    .state
                    .work
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                link.replace(next)
            };
            drop(displaced);
            drop(tail);
        } else {
            self.changed = Some(state.clone());
        }
        self.changed_tail = Some(state);
    }
    fn pop_changed(&mut self) -> Option<Cancellation> {
        let state = self.changed.take()?;
        let next = state
            .state
            .work
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if next.is_none() {
            drop(self.changed_tail.take());
        }
        self.changed = next;
        Some(state)
    }
    fn drain(&mut self) {
        while let Some(state) = self.pop_pending() {
            drop(state);
        }
        while let Some(state) = self.pop_changed() {
            drop(state);
        }
        drop(self.changed_tail.take());
    }
}
impl Drop for Propagation {
    fn drop(&mut self) {
        // Continue detaching links even if destroying one owner unwinds.
        let mut remaining = std::mem::DropGuard::new(self, |work| work.drain());
        remaining.drain();
        let _ = std::mem::DropGuard::dismiss(remaining);
    }
}

impl State {
    fn new(
        parent: Option<Parent>,
        prepared: Option<PreparedChildren>,
        observers: Option<prepared_notification::Event>,
    ) -> Self {
        Self {
            requested: AtomicBool::new(false),
            changed: Event::new(),
            observers,
            children: Mutex::new(Children {
                ordinary: Vec::new(),
                prepared,
            }),
            parent: Mutex::new(parent),
            work: Mutex::new(None),
        }
    }

    fn address(&self) -> usize {
        std::ptr::from_ref(self).addr()
    }

    fn unlink(&self) {
        let parent = self
            .parent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(parent) = parent.as_ref().and_then(Parent::node) {
            let address = self.address();
            let mut children = parent
                .state
                .children
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            children.ordinary.retain(|link| link.address != address);
            if let Some(prepared) = &mut children.prepared {
                prepared.retain(|link| link.address != address);
            }
        }
    }

    fn notify(&self) {
        let _wake_ordinary = std::mem::DropGuard::new(&self.changed, notify_all);
        if let Some(observers) = &self.observers {
            observers.notify_all();
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.unlink();
    }
}

/// Executor-independent, persistent cancellation request with broadcast notification.
#[derive(Clone)]
#[derive(crate::EffectEnvironment)]
#[effect_environment(cancellation = self)]
pub struct Cancellation {
    state: Node,
}
impl Default for Cancellation {
    fn default() -> Self {
        Self {
            state: Node::Ordinary(Arc::new(State::default())),
        }
    }
}

impl Cancellation {
    #[must_use]
    /// Construct a `Cancellation` from its required inputs and explicit policies.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new() -> Self {
        Self::default()
    }

    /// A derived request source. Parent requests reach this child and its
    /// descendants; requesting the child does not request its parent/siblings.
    /// Parent registration stays alive until the child is dropped. Teardown
    /// unregisters the weak child link and destroys ancestors iteratively.
    #[must_use]
    /// # Returns
    /// Type: `Self`.
    pub fn child(&self) -> Self {
        self.ordinary_child(None)
    }

    fn ordinary_child(&self, prepared: Option<PreparedChildren>) -> Self {
        let state = Arc::new(State::new(
            Some(Parent::Ordinary(FlatDrop::new(self.clone()))),
            prepared,
            None,
        ));
        let requested = {
            let mut children = self
                .state
                .children
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.is_requested() {
                true
            } else {
                children.ordinary.push(Link {
                    address: state.address(),
                    child: Child::Ordinary(Arc::downgrade(&state)),
                });
                false
            }
        };
        let child = Self {
            state: Node::Ordinary(state),
        };
        if requested {
            let _ = child.request();
        }
        child
    }

    /// Returns true only for the first request. Every registered waiter wakes.
    #[must_use]
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub fn request(&self) -> bool {
        let first = !self.state.requested.swap(true, Ordering::AcqRel);
        if !first {
            return false;
        }
        let mut work = Propagation::new(self.clone());
        // Iterative propagation keeps deep child trees stack-safe. Weak links
        // are detached before notifications/descendant requests, and no user
        // wake callback runs while a registration gate is held.
        while let Some(node) = work.pop_pending() {
            let Children { ordinary, prepared } = std::mem::take(
                &mut *node
                    .state
                    .children
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            let mut prepared =
                prepared.map(|links| std::mem::DropGuard::new(links, |mut links| links.clear()));
            let selected = prepared
                .as_mut()
                .into_iter()
                .flat_map(|links| links.drain());
            for child in ordinary
                .into_iter()
                .chain(selected)
                .filter_map(|link| link.upgrade())
            {
                if !child.state.requested.swap(true, Ordering::AcqRel) {
                    work.push_pending(child);
                }
            }
            work.push_changed(node);
        }
        // Record the entire linked request before invoking any wake callback.
        while let Some(node) = work.pop_changed() {
            node.state.notify();
        }
        true
    }

    /// # Returns
    /// Type: `()`.
    pub(crate) fn unlink(&self) {
        self.state.unlink();
    }

    #[must_use]
    /// Check whether cancellation has already been requested.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub fn is_requested(&self) -> bool {
        self.state.requested.load(Ordering::Acquire)
    }

    /// Registration precedes the flag check, so an intervening request cannot be lost.
    /// The listener lives inline in the returned future; the event's native
    /// state and the future owner's storage remain separately accounted.
    ///
    /// # Returns
    /// Type: `()`.
    pub async fn cancelled(&self) {
        match self.try_cancelled() {
            Ok(wait) => wait.await,
            // Arbitrary ordinary callbacks retain the infallible API.
            // Selected callers preadmit through try_cancelled.
            Err(_) => self.wait_ordinary().await,
        }
    }

    pub(crate) fn try_cancelled(&self) -> Result<impl Future<Output = ()> + '_, MemoryError> {
        let observer = if self.is_requested() {
            None
        } else {
            self.state
                .observers
                .as_ref()
                .map(prepared_notification::Event::listen)
                .transpose()?
        };
        Ok(async move {
            if let Some(observer) = observer {
                self.wait_prepared(&observer).await;
            } else if !self.is_requested() {
                self.wait_ordinary().await;
            }
        })
    }

    pub(crate) fn reserve_wait_owned(&self) -> Result<OwnedCancellationWait, MemoryError> {
        let wait = if self.is_requested() {
            CancellationWait::Ready
        } else if let Some(event) = &self.state.observers {
            CancellationWait::Prepared(event.listen()?)
        } else {
            CancellationWait::Ordinary(self.state.changed.listen())
        };
        Ok(OwnedCancellationWait {
            cancellation: self.clone(),
            wait,
        })
    }

    async fn wait_prepared(&self, observer: &prepared_notification::Registration) {
        loop {
            observer.reset();
            if self.is_requested() {
                return;
            }
            std::future::poll_fn(|context| observer.poll(context)).await;
        }
    }

    async fn wait_ordinary(&self) {
        loop {
            event_listener::listener!(self.state.changed => listener);
            if self.is_requested() {
                return;
            }
            listener.await;
        }
    }
}

pub(crate) struct OwnedCancellationWait {
    // Recycle a borrowed listener before the last handle can recycle its node.
    wait: CancellationWait,
    cancellation: Cancellation,
}

enum CancellationWait {
    Ready,
    Ordinary(event_listener::EventListener),
    Prepared(prepared_notification::Registration),
}

impl OwnedCancellationWait {
    pub(crate) async fn wait(self) {
        match self.wait {
            CancellationWait::Ready => {}
            CancellationWait::Ordinary(listener) => {
                if !self.cancellation.is_requested() {
                    listener.await;
                }
            }
            CancellationWait::Prepared(observer) => {
                self.cancellation.wait_prepared(&observer).await
            }
        }
    }
}

/// One startup anchor, physical child states and per-child observers.
pub(crate) struct PreparedCancellationBank {
    children: SlotPool<'static, State>,
    links: Option<SlotPool<'static, FixedVector<'static, Link>>>,
}
impl PreparedCancellationBank {
    pub(crate) fn detached_storage_bytes(
        nodes: usize,
        listeners: usize,
    ) -> Result<Bytes, MemoryError> {
        SlotPool::<State>::storage_bytes(nodes)?.checked_add(
            prepared_notification::Event::storage_bytes(listeners)?.checked_mul(nodes)?,
        )
    }

    pub(crate) fn storage_bytes(nodes: usize, listeners: usize) -> Result<Bytes, MemoryError> {
        let links =
            std::alloc::Layout::array::<Link>(nodes).map_err(|_| MemoryError::InvalidLayout)?;
        Self::detached_storage_bytes(nodes, listeners)?
            .checked_add(SlotPool::<FixedVector<'static, Link>>::storage_bytes(1)?)?
            .checked_add(Bytes::new(links.size()))
    }

    pub(crate) fn prepare_detached(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        nodes: usize,
        listeners: usize,
    ) -> Result<Self, MemoryError> {
        Self::detached_storage_bytes(nodes, listeners)?;
        Ok(Self {
            children: memory.slots(region, nodes, |_| {
                let observers = prepared_notification::Event::prepare(memory, region, listeners)?;
                Ok(State::new(None, None, Some(observers)))
            })?,
            links: None,
        })
    }

    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        nodes: usize,
        listeners: usize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(nodes, listeners)?;
        let mut bank = Self::prepare_detached(memory, region, nodes, listeners)?;
        bank.links = Some(memory.slots(region, 1, |_| {
            FixedVector::prepare(memory.allocator(region)?, nodes)
        })?);
        Ok(bank)
    }

    pub(crate) fn bind(&self, parent: &Cancellation) -> Result<LoaderCancellation, MemoryError> {
        let children = self.children.try_clone()?;
        let links = self
            .links
            .as_ref()
            .ok_or(MemoryError::UnsupportedRunContract)?
            .try_checkout()?;
        Ok(LoaderCancellation {
            bank: Some(children),
            root: parent.ordinary_child(Some(links)),
        })
    }

    pub(crate) fn reserve_detached(&self) -> Result<Cancellation, MemoryError> {
        Ok(Cancellation {
            state: Node::Prepared(Some(self.children.try_checkout()?.share())),
        })
    }

    pub(crate) fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            children: self.children.try_clone()?,
            links: self.links.as_ref().map(SlotPool::try_clone).transpose()?,
        })
    }
}

pub(crate) struct LoaderCancellation {
    bank: Option<SlotPool<'static, State>>,
    root: Cancellation,
}
impl LoaderCancellation {
    pub(crate) fn ordinary(parent: &Cancellation) -> Self {
        Self {
            bank: None,
            root: parent.child(),
        }
    }

    pub(crate) fn child(&self) -> Result<Cancellation, MemoryError> {
        let Some(bank) = &self.bank else {
            return Ok(self.root.child());
        };
        let state = bank.try_checkout()?.share();
        let link = Link {
            address: state.address(),
            child: Child::Prepared(state.downgrade()?),
        };
        let child = Cancellation {
            state: Node::Prepared(Some(state)),
        };
        *child
            .state
            .parent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(Parent::Anchor(self.root.clone()));
        let requested = {
            let mut children = self
                .root
                .state
                .children
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.root.is_requested() {
                true
            } else {
                let links = children
                    .prepared
                    .as_mut()
                    .ok_or(MemoryError::ConstructionUnbound)?;
                links.push(link).map_err(|_| MemoryError::NoSlot)?;
                false
            }
        };
        if requested {
            let _ = child.request();
        }
        Ok(child)
    }

    pub(crate) fn request(&self) -> bool {
        self.root.request()
    }

    pub(crate) fn unlink(&self) {
        self.root.unlink();
    }
}
