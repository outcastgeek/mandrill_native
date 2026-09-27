use crate::{Defect, Effect, Exit};
use std::io::{self, Write};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleChannel {
    Output,
    Error,
}
pub trait Console: Send + Sync + 'static {
    fn write(&self, channel: ConsoleChannel, message: &str) -> Result<(), Defect>;
}
pub trait HasConsole {
    type Console: Console + ?Sized;
    fn console(&self) -> &Self::Console;
}
pub struct StandardConsole;
impl Console for StandardConsole {
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
    pub fn console_write(channel: ConsoleChannel, message: impl Into<Arc<str>>) -> Self {
        let message = message.into();
        Self::ready(move |env, _| match env.console().write(channel, &message) {
            Ok(()) => Exit::Success(()),
            Err(defect) => Exit::Die(defect),
        })
    }
}
