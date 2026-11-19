//! Reusable diagnostic text and retained FIFO drain pages.

use super::DiagnosticOverflow;
use crate::internal::waiting::Waiting;
use crate::memory::{
    Bytes, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool, Text, TextFormatError,
};
use crate::stream::{PreparedChunkPool, PreparedChunkValues};
use crate::{Defect, DefectKind};
use std::any::Any;
use std::fmt;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

/// Actual queued events, message storage and independently retained drains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedDiagnosticCapacity {
    /// Maximum events in the live queue.
    pub entries: NonZeroUsize,
    /// Simultaneous queued, formatting and drained message owners.
    pub messages: NonZeroUsize,
    /// Maximum UTF-8 message bytes in one owner.
    pub bytes_per_message: Bytes,
    /// Simultaneously retained drain pages, each able to hold the live queue.
    pub snapshots: NonZeroUsize,
}

impl PreparedDiagnosticCapacity {
    /// Regional demand excluding allocator headers and alignment padding.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let message = std::alloc::Layout::array::<u8>(self.bytes_per_message.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        Waiting::<PreparedDiagnostic>::payload_bytes(self.entries.get())?
            .checked_add(SlotPool::<Text<'static>>::storage_bytes(
                self.messages.get(),
            )?)?
            .checked_add(Bytes::new(message.size()).checked_mul(self.messages.get())?)?
            .checked_add(PreparedChunkPool::<PreparedDiagnostic>::storage_bytes(
                self.snapshots,
                self.entries,
            )?)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The diagnostic could not be formatted or stored in its selected backing.
pub enum PreparedDiagnosticError {
    Memory(MemoryError),
    Formatter(fmt::Error),
}

impl From<MemoryError> for PreparedDiagnosticError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<TextFormatError> for PreparedDiagnosticError {
    fn from(error: TextFormatError) -> Self {
        match error {
            TextFormatError::Memory(error) => Self::Memory(error),
            TextFormatError::Formatter(error) => Self::Formatter(error),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct PanicMetadata {
    truncated: bool,
    original_length: Option<usize>,
}

/// One diagnostic retaining its actual reusable text slot.
pub struct PreparedDiagnostic {
    kind: DefectKind,
    message: SlotLease<'static, Text<'static>>,
    panic: PanicMetadata,
}

impl PreparedDiagnostic {
    #[must_use]
    pub const fn kind(&self) -> DefectKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        self.message.as_str()
    }

    #[must_use]
    pub const fn panic_message_truncated(&self) -> bool {
        self.panic.truncated
    }

    #[must_use]
    pub const fn panic_original_length(&self) -> Option<usize> {
        self.panic.original_length
    }
}

/// A drained FIFO page that continues to own every retained message.
pub struct PreparedDiagnosticSnapshot {
    entries: PreparedChunkValues<PreparedDiagnostic>,
    discarded: usize,
}

impl PreparedDiagnosticSnapshot {
    #[must_use]
    pub fn entries(&self) -> &[PreparedDiagnostic] {
        self.entries.as_slice()
    }

    /// Total diagnostics discarded as of this drain.
    #[must_use]
    pub const fn discarded(&self) -> usize {
        self.discarded
    }
}

struct State {
    entries: Waiting<PreparedDiagnostic>,
    discarded: usize,
}

impl State {
    fn discard(&mut self) {
        self.discarded = self.discarded.saturating_add(1);
    }
}

struct Core {
    state: Mutex<State>,
    messages: SlotPool<'static, Text<'static>>,
    snapshots: PreparedChunkPool<PreparedDiagnostic>,
    limit: NonZeroUsize,
    overflow: DiagnosticOverflow,
}

/// A prepared diagnostic sink with the existing discard policies. Exhaustion
/// increments its loss counter and never tries to report another diagnostic.
#[derive(Clone)]
pub struct PreparedDiagnostics {
    core: Arc<Core>,
}

impl PreparedDiagnostics {
    /// Provision the queue, UTF-8 owners and drain pages before recording.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedDiagnosticCapacity,
        overflow: DiagnosticOverflow,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes()?;
        let entries = Waiting::prepare(memory, region, capacity.entries.get())?;
        let messages = memory.slots(region, capacity.messages.get(), |_| {
            memory
                .buffer(region, capacity.bytes_per_message)
                .map(Text::new)
        })?;
        let snapshots =
            PreparedChunkPool::prepare(memory, region, capacity.snapshots, capacity.entries)?;
        Ok(Self {
            core: Arc::new(Core {
                state: Mutex::new(State {
                    entries,
                    discarded: 0,
                }),
                messages,
                snapshots,
                limit: capacity.entries,
                overflow,
            }),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.core
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Record borrowed text. `Ok(false)` is an intentional discard-newest decision.
    pub fn try_record(
        &self,
        kind: DefectKind,
        message: &str,
    ) -> Result<bool, PreparedDiagnosticError> {
        self.record_with(kind, PanicMetadata::default(), |text| {
            text.try_push_str(message).map_err(Into::into)
        })
    }

    /// Admit a message slot before invoking any formatting implementation.
    /// Oversize text and formatting failures count as one discarded diagnostic.
    pub fn try_record_format(
        &self,
        kind: DefectKind,
        message: fmt::Arguments<'_>,
    ) -> Result<bool, PreparedDiagnosticError> {
        self.record_with(kind, PanicMetadata::default(), |text| {
            text.try_write_fmt(message).map_err(Into::into)
        })
    }

    /// Copy a typed defect into the selected text owner without cloning its native text.
    pub fn record_defect(&self, defect: &Defect) -> Result<bool, PreparedDiagnosticError> {
        let panic = PanicMetadata {
            truncated: defect.panic_message_truncated(),
            original_length: defect.panic_original_length(),
        };
        self.record_with(defect.kind(), panic, |text| {
            text.try_push_str(defect.message()).map_err(Into::into)
        })
    }

    fn record_with(
        &self,
        kind: DefectKind,
        panic: PanicMetadata,
        write: impl FnOnce(&mut Text<'static>) -> Result<(), PreparedDiagnosticError>,
    ) -> Result<bool, PreparedDiagnosticError> {
        if self.core.overflow == DiagnosticOverflow::DiscardNewest {
            let mut state = self.state();
            if state.entries.len() == self.core.limit.get() {
                state.discard();
                return Ok(false);
            }
        }
        let incomplete = std::mem::DropGuard::new((), |()| self.state().discard());
        let mut message = self.core.messages.try_checkout()?;
        message.clear();
        write(&mut message)?;
        std::mem::DropGuard::dismiss(incomplete);
        let event = PreparedDiagnostic {
            kind,
            message,
            panic,
        };
        self.publish(event)
    }

    fn publish(&self, event: PreparedDiagnostic) -> Result<bool, PreparedDiagnosticError> {
        let mut state = self.state();
        let retired = if state.entries.len() == self.core.limit.get() {
            state.discard();
            match self.core.overflow {
                DiagnosticOverflow::DiscardOldest => state.entries.pop_front(),
                DiagnosticOverflow::DiscardNewest => return Ok(false),
            }
        } else {
            None
        };
        // The queue is preallocated, and this operation either used an empty
        // slot or retired exactly one owner before inserting the new event.
        let inserted = state.entries.try_push_back(event);
        if inserted.is_err() {
            state.discard();
        }
        drop(state);
        drop(retired);
        inserted.map(|_| true).map_err(|(error, _)| error.into())
    }

    /// Reserve a complete drain page before removing any queued event.
    pub fn try_drain(&self) -> Result<PreparedDiagnosticSnapshot, MemoryError> {
        let mut entries = self.core.snapshots.try_checkout()?;
        let mut state = self.state();
        while let Some(event) = state.entries.pop_front() {
            entries.push_reserved(event);
        }
        Ok(PreparedDiagnosticSnapshot {
            entries,
            discarded: state.discarded,
        })
    }

    #[must_use]
    pub fn discarded(&self) -> usize {
        self.state().discarded
    }

    fn record_panic(&self, payload: &(dyn Any + Send)) {
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied());
        let panic = PanicMetadata {
            truncated: false,
            original_length: message.map(str::len),
        };
        let _ = self.record_with(DefectKind::CallbackPanic, panic, |text| {
            text.try_push_str(message.unwrap_or("User callback panicked with a non-string payload"))
                .map_err(Into::into)
        });
    }

    pub(super) fn isolate<T>(&self, callback: impl FnOnce() -> Result<T, Defect>) -> Option<T> {
        match catch_unwind(AssertUnwindSafe(callback)) {
            Ok(Ok(value)) => Some(value),
            Ok(Err(defect)) => {
                let _ = self.record_defect(&defect);
                None
            }
            Err(panic) => {
                self.record_panic(panic.as_ref());
                None
            }
        }
    }
}
