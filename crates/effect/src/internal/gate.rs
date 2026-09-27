use super::notification::notify_all;
use super::waiting::{Position, Waiting};
use crate::Cancellation;
use event_listener::Event;
use futures_util::future::{Either, select};
use std::sync::{Arc, Mutex};
#[derive(Clone, Copy)]
pub(crate) enum GateError {
    Closed,
    Interrupted,
}
#[derive(Clone, Copy)]
enum Status {
    Queued,
    Granted,
    Closed,
}
struct Waiter {
    status: Mutex<Status>,
    changed: Event,
}
struct State {
    closed: bool,
    busy: bool,
    waiting: Waiting<Arc<Waiter>>,
}
struct Inner {
    state: Mutex<State>,
}
#[derive(Clone)]
pub(crate) struct Gate(Arc<Inner>);
pub(crate) struct Permit {
    gate: Gate,
}
struct Registration {
    gate: Gate,
    waiter: Option<Arc<Waiter>>,
    position: Position,
}
impl Gate {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Inner {
            state: Mutex::new(State {
                closed: false,
                busy: false,
                waiting: Waiting::default(),
            }),
        }))
    }
    pub(crate) async fn enter(&self, cancellation: Cancellation) -> Result<Permit, GateError> {
        if cancellation.is_requested() {
            return Err(GateError::Interrupted);
        }
        let waiter = Arc::new(Waiter {
            status: Mutex::new(Status::Queued),
            changed: Event::new(),
        });
        let position = {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed {
                return Err(GateError::Closed);
            }
            let position = state.waiting.push_back(Arc::clone(&waiter));
            if !state.busy {
                state.busy = true;
                let _ = state.waiting.pop_front();
                *waiter
                    .status
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Status::Granted;
            }
            position
        };
        let mut registration = Registration {
            gate: self.clone(),
            waiter: Some(Arc::clone(&waiter)),
            position,
        };
        loop {
            let changed = waiter.changed.listen();
            let status = *waiter
                .status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match status {
                Status::Granted if cancellation.is_requested() => {
                    return Err(GateError::Interrupted);
                }
                Status::Granted => {
                    registration.waiter = None;
                    return Ok(Permit { gate: self.clone() });
                }
                Status::Closed => return Err(GateError::Closed),
                Status::Queued => {}
            }
            match select(Box::pin(changed), Box::pin(cancellation.cancelled())).await {
                Either::Left(((), _)) => {}
                Either::Right(((), _)) => return Err(GateError::Interrupted),
            }
        }
    }
    fn release(&self) {
        let next = {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let next = state.waiting.pop_front();
            if let Some(waiter) = &next {
                *waiter
                    .status
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Status::Granted;
            } else {
                state.busy = false;
            }
            next
        };
        if let Some(next) = next {
            notify_all(&next.changed);
        }
    }
    pub(crate) fn close(&self) {
        let waiting = {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;

            std::mem::take(&mut state.waiting)
        };
        let mut waiting = waiting;
        while let Some(waiter) = waiting.pop_front() {
            *waiter
                .status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Status::Closed;
            notify_all(&waiter.changed);
        }
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.gate.release();
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(waiter) = self.waiter.take() {
            let granted = {
                let mut state = self
                    .gate
                    .0
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let _ = state
                    .waiting
                    .remove_where(self.position, |candidate| Arc::ptr_eq(candidate, &waiter));
                matches!(
                    *waiter
                        .status
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                    Status::Granted
                )
            };
            if granted {
                self.gate.release();
            }
        }
    }
}
