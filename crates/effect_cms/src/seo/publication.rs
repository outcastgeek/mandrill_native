//! The private publication stage.
//!
//! A prepared value and its optional validator are one value that the publisher
//! consumes. Consuming the stage is what guarantees that a value is validated once
//! and that the validated value, not a copy or a substitute, is the published value.
//! A stage cannot be reused or forged because no stage value can be produced outside
//! this owner's preparation path and no reference form is exposed.
//!
//! The validator runs only when one was supplied. Its expected error is returned
//! unchanged; a value that failed validation is not published.

use std::sync::Arc;

/// One prepared value with its optional validator.
pub(super) struct Prepared<Value, Error> {
    /// The value to validate and publish.
    pub(super) value: Value,
    /// The validator, when one was supplied.
    pub(super) validator:
        Option<Arc<dyn Fn(&mut Value) -> Result<(), Error> + Send + Sync + 'static>>,
}

/// Validate one prepared value and publish it.
///
/// The validator mutates the published value in place. The exact validator error is
/// returned on failure, and the value is not published.
pub(super) fn publish<Value, Error>(prepared: Prepared<Value, Error>) -> Result<Value, Error> {
    let Prepared { mut value, validator } = prepared;
    if let Some(validator) = validator {
        validator(&mut value)?;
    }
    Ok(value)
}
