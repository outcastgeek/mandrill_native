//! Environment-provided console input and output.

use crate::{Defect, Effect, Exit};
use std::io::{self, Write};
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
