//! Native behavior contracts ported from contentmgmt2; see CORE_COVERAGE.md.
use effect::{Effect, Exit};
use std::sync::Arc;
fn run<A: Send + 'static, E: Send + std::fmt::Debug + 'static>(program: Effect<A, E, ()>) -> Result<A,E> {
    match futures_executor::block_on(program.run(Arc::new(()))) {
        Exit::Success(value) => Ok(value),
        Exit::Fail(error) => Err(error),
        other => panic!("unexpected effect exit: {}", match other { Exit::Die(_) => "defect", Exit::Interrupted => "interrupted", Exit::FinalizationFailed(_) => "finalization failed", _ => unreachable!() }),
    }
}
#[path = "core_contracts/definitions_temporal.rs"] mod definitions_temporal;
#[path = "core_contracts/content.rs"] mod content;
#[path = "core_contracts/workflows.rs"] mod workflows;
#[path = "core_contracts/relations.rs"] mod relations;
