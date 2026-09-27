use event_listener::Event;

/// Broadcast a persisted state change to every currently registered listener.
pub(crate) fn notify_all(event: &Event) {
    // Event::notify(n) ensures at least n registered listeners are notified.
    // usize::MAX covers every representable listener count: this is the
    // dependency's broadcast contract, not a chosen capacity or tuning value.
    // Late listeners inspect persisted state after registering their listener.
    // See event-listener's crate-level broadcast example and Event::notify docs.
    event.notify(usize::MAX);
}
