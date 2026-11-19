//! Environment-provided console input and output.

use crate::memory::{
    Bytes, MemoryError, PreparedMemory, RegionId, SlotPool, Text, TextFormatError,
};
use crate::{Defect, Effect, Exit};
use std::alloc::Layout;
use std::fmt;
use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The output channel selected for a console write.
pub enum ConsoleChannel {
    /// Write to the application’s ordinary output channel.
    Output,
    /// The value is an expected application error.
    Error,
}
/// Supplies console operations without choosing a process-global console.
pub trait Console: Send + Sync + 'static {
    /// # Parameters
    /// - `channel` (`ConsoleChannel`): The channel supplied to this operation.
    /// - `message` (`&str`): Text or payload transferred to the selected consumer.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn write(&self, channel: ConsoleChannel, message: &str) -> Result<(), Defect>;
}
/// Selects the console capability in an application environment.
pub trait HasConsole {
    /// Console capability selected by this environment.
    type Console: Console + ?Sized;
    /// # Returns
    /// Type: `&Self::Console`.
    fn console(&self) -> &Self::Console;
}
/// The process console adapter used when an application chooses standard streams.
pub struct StandardConsole;
impl Console for StandardConsole {
    ///
    /// # Parameters
    /// - `channel` (`ConsoleChannel`): The channel supplied to this operation.
    /// - `message` (`&str`): Text or payload transferred to the selected consumer.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn write(&self, channel: ConsoleChannel, message: &str) -> Result<(), Defect> {
        // writeln's terminator is the platform writer's line contract, not policy.
        let result = match channel {
            ConsoleChannel::Output => writeln!(io::stdout().lock(), "{message}"),
            ConsoleChannel::Error => writeln!(io::stderr().lock(), "{message}"),
        };
        result.map_err(Defect::from_io)
    }
}

/// Failure to stage or deliver one prepared console message.
#[derive(Debug)]
pub enum PreparedConsoleError {
    /// Physical checkout or bounded formatting failed before delivery.
    Memory(MemoryError),
    /// The supplied formatter refused before delivery.
    Formatter(fmt::Error),
    /// The environment's console reported an output defect.
    Output(Defect),
}

/// Reusable regional UTF-8 buffers for console messages.
pub struct PreparedConsole {
    messages: SlotPool<'static, Text<'static>>,
}

impl PreparedConsole {
    /// Region bytes for the message slots and text buffers before allocator overhead.
    pub fn storage_bytes(slots: NonZeroUsize, bytes: Bytes) -> Result<Bytes, MemoryError> {
        let layout = Layout::array::<u8>(bytes.get()).map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Text<'static>>::storage_bytes(slots.get())?
            .checked_add(Bytes::new(layout.size()).checked_mul(slots.get())?)
    }

    /// Prepare the actual formatting bytes before the region is sealed.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        slots: NonZeroUsize,
        bytes: Bytes,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(slots, bytes)?;
        Ok(Self {
            messages: memory.texts(region, slots.get(), bytes)?,
        })
    }

    /// Retain the same formatting slots without increasing capacity.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            messages: self.messages.try_clone()?,
        })
    }

    /// Deliver a complete prepared message to the selected console channel.
    pub fn write(
        &self,
        console: &(impl Console + ?Sized),
        channel: ConsoleChannel,
        message: &str,
    ) -> Result<(), PreparedConsoleError> {
        self.write_fmt(console, channel, format_args!("{message}"))
    }

    /// Reserve text before entering the formatter and deliver only on success.
    /// The console borrows the real bytes through the write. Formatter and
    /// console implementations own any work or allocations they perform.
    pub fn write_fmt(
        &self,
        console: &(impl Console + ?Sized),
        channel: ConsoleChannel,
        arguments: fmt::Arguments<'_>,
    ) -> Result<(), PreparedConsoleError> {
        let mut text = self
            .messages
            .try_checkout()
            .map_err(PreparedConsoleError::Memory)?;
        text.clear();
        text.try_write_fmt(arguments).map_err(|error| match error {
            TextFormatError::Memory(error) => PreparedConsoleError::Memory(error),
            TextFormatError::Formatter(error) => PreparedConsoleError::Formatter(error),
        })?;
        console
            .write(channel, text.as_str())
            .map_err(PreparedConsoleError::Output)
    }
}

impl<R: HasConsole + Send + Sync + 'static, E: Send + 'static> Effect<(), E, R> {
    /// Write to the console capability selected by the environment.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `HasConsole + Send + Sync + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Parameters
    /// - `channel` (`ConsoleChannel`): The channel supplied to this operation.
    /// - `message` (`impl Into<Arc<str>>`): Text or payload transferred to the selected consumer.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn console_write(channel: ConsoleChannel, message: impl Into<Arc<str>>) -> Self {
        let message = message.into();
        Self::ready(move |env, _| match env.console().write(channel, &message) {
            Ok(()) => Exit::Success(()),
            Err(defect) => Exit::Die(defect),
        })
    }
}
