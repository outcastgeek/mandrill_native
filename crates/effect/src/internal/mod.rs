//! Private execution and synchronization mechanics.

pub(crate) mod destruction;
pub(crate) mod notification;
pub(crate) mod outcome;
pub(crate) mod runtime;
pub(crate) mod waiting;
pub(crate) mod workflow;

pub(crate) mod gate;

pub(crate) mod macro_future;

/// Native body adapter retaining an externally owned resource scope.
pub(crate) mod scoped_body;
