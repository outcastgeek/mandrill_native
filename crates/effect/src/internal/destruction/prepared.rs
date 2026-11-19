//! Intrusive, dependency-ordered prepared destruction. A node's payload,
//! descendants, footer, footer descendants and physical owner retire in order.
//! All traversal storage is in prepared nodes; no user Drop runs under QUEUES.
use super::QUEUES;
use std::ptr::NonNull;
use std::thread::ThreadId;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Payload,
    Footer,
    Release,
}
pub(crate) struct DestructionNode {
    next: Option<NonNull<Self>>,
    root_next: Option<NonNull<Self>>,
    children: Option<NonNull<Self>>,
    last_child: Option<NonNull<Self>>,
    parent: Option<NonNull<Self>>,
    active: Option<NonNull<Self>>,
    releasing: bool,
    released_children: Option<NonNull<Self>>,
    last_released_child: Option<NonNull<Self>>,
    phase: Phase,
    thread: Option<ThreadId>,
    context: *mut (),
    destroy: unsafe fn(NonNull<Self>),
    footer: unsafe fn(NonNull<Self>),
    release: unsafe fn(NonNull<Self>),
}
impl DestructionNode {
    pub(crate) const fn new(
        destroy: unsafe fn(NonNull<Self>),
        footer: unsafe fn(NonNull<Self>),
        release: unsafe fn(NonNull<Self>),
    ) -> Self {
        Self {
            next: None,
            root_next: None,
            children: None,
            last_child: None,
            parent: None,
            active: None,
            releasing: false,
            released_children: None,
            last_released_child: None,
            phase: Phase::Payload,
            thread: None,
            context: std::ptr::null_mut(),
            destroy,
            footer,
            release,
        }
    }
    pub(crate) fn set_context(&mut self, context: *mut ()) {
        self.context = context;
    }
    pub(crate) unsafe fn context(node: NonNull<Self>) -> *mut () {
        // SAFETY: caller retains the initialized node; immutable callback context.
        unsafe { (*node.as_ptr()).context }
    }
}
unsafe fn append(
    head: *mut Option<NonNull<DestructionNode>>,
    tail: *mut Option<NonNull<DestructionNode>>,
    node: NonNull<DestructionNode>,
) {
    // SAFETY: caller holds QUEUES, owns the unlinked node and disjoint queue
    // fields. FIFO siblings preserve enqueue/field order; depth-first traversal
    // completes each sibling's whole descendant tree before the next sibling.
    unsafe {
        (*node.as_ptr()).next = None;
        if let Some(last) = *tail {
            (*last.as_ptr()).next = Some(node);
        } else {
            *head = Some(node);
        }
        *tail = Some(node);
    }
}
/// A releasing descendant may free its own node. Collect its reentrant children
/// on the still-live root, then put them BEFORE the parent's remaining siblings.
struct ReleaseChildren {
    root: NonNull<DestructionNode>,
    parent: NonNull<DestructionNode>,
}
impl Drop for ReleaseChildren {
    fn drop(&mut self) {
        let _lock = QUEUES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: root and parent remain retained through this release callback,
        // including unwind. No link to the potentially freed descendant is read.
        unsafe {
            let root = self.root.as_ptr();
            let parent = self.parent.as_ptr();
            if let Some(head) = (*root).released_children.take() {
                if let Some(tail) = (*root).last_released_child.take() {
                    (*tail.as_ptr()).next = (*parent).children;
                    if (*parent).children.is_none() {
                        (*parent).last_child = Some(tail);
                    }
                }
                (*parent).children = Some(head);
            }
            (*root).releasing = false;
        }
    }
}
struct Drain {
    root: NonNull<DestructionNode>,
    registered: bool,
}
enum Action {
    Payload(NonNull<DestructionNode>),
    Footer(NonNull<DestructionNode>),
    Release(NonNull<DestructionNode>, NonNull<DestructionNode>),
    Finish,
}
impl Drain {
    fn next(&self) -> Action {
        let _lock = QUEUES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: registered root retains every active ancestor. All links and
        // phase changes are serialized; phase advances BEFORE invoking user Drop.
        unsafe {
            let root = self.root.as_ptr();
            loop {
                let node = (*root).active.unwrap_or(self.root);
                let pointer = node.as_ptr();
                if (*pointer).phase == Phase::Payload {
                    (*pointer).phase = Phase::Footer;
                    return Action::Payload(node);
                }
                if let Some(child) = (*pointer).children {
                    (*pointer).children = (*child.as_ptr()).next;
                    if (*pointer).children.is_none() {
                        (*pointer).last_child = None;
                    }
                    (*child.as_ptr()).next = None;
                    (*child.as_ptr()).parent = Some(node);
                    (*root).active = Some(child);
                    continue;
                }
                if (*pointer).phase == Phase::Footer {
                    (*pointer).phase = Phase::Release;
                    return Action::Footer(node);
                }
                if let Some(parent) = (*pointer).parent {
                    (*root).active = Some(parent);
                    (*root).releasing = true;
                    return Action::Release(node, parent);
                }
                return Action::Finish;
            }
        }
    }
    fn drain(&mut self) {
        loop {
            match self.next() {
                Action::Payload(node) => {
                    // SAFETY: active node is uniquely queued and retained.
                    unsafe { ((*node.as_ptr()).destroy)(node) };
                }
                Action::Footer(node) => {
                    // SAFETY: payload and its complete descendant tree are retired;
                    // footer callback has its own once-only initialized state.
                    unsafe { ((*node.as_ptr()).footer)(node) };
                }
                Action::Release(node, parent) => {
                    let _children = ReleaseChildren {
                        root: self.root,
                        parent,
                    };
                    // SAFETY: callbacks are copied before releasing possibly all
                    // backing; no later access to node occurs, including on unwind.
                    unsafe { ((*node.as_ptr()).release)(node) };
                }
                Action::Finish => {
                    self.finish();
                    return;
                }
            }
        }
    }
    fn finish(&mut self) {
        {
            let mut queues = QUEUES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut current = queues.prepared;
            let mut previous: Option<NonNull<DestructionNode>> = None;
            // SAFETY: registered root is live until unlink; links under QUEUES.
            unsafe {
                while let Some(node) = current {
                    if node == self.root {
                        let next = (*node.as_ptr()).root_next;
                        if let Some(previous) = previous {
                            (*previous.as_ptr()).root_next = next;
                        } else {
                            queues.prepared = next;
                        }
                        break;
                    }
                    previous = current;
                    current = (*node.as_ptr()).root_next;
                }
            }
        }
        self.registered = false;
        // SAFETY: whole root tree is finished. Unlink before releasing backing,
        // as before; reentrant final-pool drops start a fresh registered drain.
        unsafe { ((*self.root.as_ptr()).release)(self.root) };
    }
}
impl Drop for Drain {
    fn drop(&mut self) {
        if self.registered {
            // Continue phase/descendant retirement on one unwind. A second user
            // destructor panic retains ordinary Rust double-panic semantics.
            self.drain();
        }
    }
}
/// Caller has installed its actual static owner in stable queued storage. Both
/// payload and footer callbacks clear their live flags before invoking Drop.
pub(crate) unsafe fn enqueue_prepared(node: NonNull<DestructionNode>) {
    let thread = std::thread::current().id();
    {
        let mut queues = QUEUES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut current = queues.prepared;
        // SAFETY: caller owns unlinked node; all queue writes under QUEUES.
        unsafe {
            (*node.as_ptr()).phase = Phase::Payload;
            (*node.as_ptr()).parent = None;
            (*node.as_ptr()).children = None;
            (*node.as_ptr()).last_child = None;
            while let Some(root) = current {
                let pointer = root.as_ptr();
                if (*pointer).thread == Some(thread) {
                    if (*pointer).releasing {
                        append(
                            std::ptr::addr_of_mut!((*pointer).released_children),
                            std::ptr::addr_of_mut!((*pointer).last_released_child),
                            node,
                        );
                    } else {
                        let active = (*pointer).active.unwrap_or(root).as_ptr();
                        append(
                            std::ptr::addr_of_mut!((*active).children),
                            std::ptr::addr_of_mut!((*active).last_child),
                            node,
                        );
                    }
                    return;
                }
                current = (*pointer).root_next;
            }
            (*node.as_ptr()).thread = Some(thread);
            (*node.as_ptr()).active = Some(node);
            (*node.as_ptr()).releasing = false;
            (*node.as_ptr()).released_children = None;
            (*node.as_ptr()).last_released_child = None;
            (*node.as_ptr()).root_next = queues.prepared;
        }
        queues.prepared = Some(node);
    }
    let mut drain = Drain {
        root: node,
        registered: true,
    };
    drain.drain();
}
