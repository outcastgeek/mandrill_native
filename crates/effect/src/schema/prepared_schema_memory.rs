//! Optional same-region composition of the actual prepared schema pools.
use super::{
    PreparedJsonParserCapacity, PreparedJsonParserPool, PreparedJsonValidationCapacity,
    PreparedJsonValidationPool, PreparedJsonWriterCapacity, PreparedJsonWriterPool,
    PreparedParseContextCapacity, PreparedParseContextPool,
};
use crate::memory::{Bytes, FramePool, MemoryError, PreparedMemory, RegionId};
use std::sync::Arc;

/// Explicit capacities for each independently prepared schema owner.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSchemaMemoryCapacity {
    /// Direct and counting writer records.
    pub writers: PreparedJsonWriterCapacity,
    /// Representation and issue traversal scratch.
    pub validation: PreparedJsonValidationCapacity,
    /// Managed JSON document arenas.
    pub parser: PreparedJsonParserCapacity,
    /// Invocation root and immutable path records.
    pub contexts: PreparedParseContextCapacity,
}
/// Physical region payload and separately classified startup Arc controls.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSchemaMemoryDemand {
    /// Sum of the four pools' actual region payloads, excluding allocator overhead.
    pub region_bytes: Bytes,
    /// Four pool Arcs plus the context pool's private startup controls.
    pub startup_arc_controls: usize,
    /// Requested Arc payload bytes; headers, alignment and allocator rounding are separate.
    pub startup_arc_payload_bytes: Bytes,
}
/// A narrow owner of the same four pools, sharing the one actual writer pool.
pub struct PreparedSchemaMemory {
    writers: Arc<PreparedJsonWriterPool>,
    validation: Arc<PreparedJsonValidationPool>,
    parser: Arc<PreparedJsonParserPool>,
    contexts: Arc<PreparedParseContextPool>,
}
impl PreparedSchemaMemoryCapacity {
    /// Compute pool backing once; caller-supplied frame pool and outer Arc are separate.
    pub fn demand(self) -> Result<PreparedSchemaMemoryDemand, MemoryError> {
        let context = self.contexts.demand()?;
        let region_bytes = self
            .writers
            .storage_bytes()?
            .checked_add(self.validation.storage_bytes()?)?
            .checked_add(self.parser.storage_bytes()?)?
            .checked_add(context.region_bytes)?;
        let startup_arc_controls = context
            .startup_arc_controls
            .checked_add(4)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let startup_arc_payload_bytes = context
            .startup_arc_payload_bytes
            .checked_add(Bytes::new(std::mem::size_of::<PreparedJsonWriterPool>()))?
            .checked_add(Bytes::new(std::mem::size_of::<PreparedJsonValidationPool>()))?
            .checked_add(Bytes::new(std::mem::size_of::<PreparedJsonParserPool>()))?
            .checked_add(Bytes::new(std::mem::size_of::<PreparedParseContextPool>()))?;
        Ok(PreparedSchemaMemoryDemand {
            region_bytes,
            startup_arc_controls,
            startup_arc_payload_bytes,
        })
    }
}
impl PreparedSchemaMemory {
    /// Prepare four pools in one selected region; no runtime or provider is selected.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedSchemaMemoryCapacity,
        frames: FramePool<'static>,
    ) -> Result<Self, MemoryError> {
        Self::prepare_inner(memory, region, capacity, frames, None)
    }
    /// Prepare the same four pools and select independently prepared issue backing.
    pub fn prepare_with_issues(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedSchemaMemoryCapacity,
        frames: FramePool<'static>,
        issues: Arc<super::PreparedIssueMemory>,
    ) -> Result<Self, MemoryError> {
        Self::prepare_inner(memory, region, capacity, frames, Some(issues))
    }
    fn prepare_inner(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedSchemaMemoryCapacity,
        frames: FramePool<'static>,
        issues: Option<Arc<super::PreparedIssueMemory>>,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand()?;
        let mut writers =
            PreparedJsonWriterPool::prepare(memory, region, capacity.writers, frames)?;
        if let Some(issues) = &issues {
            writers = writers.with_issue_memory(Arc::clone(issues));
        }
        let writers = Arc::new(writers);
        let validation = Arc::new(PreparedJsonValidationPool::prepare(
            memory,
            region,
            capacity.validation,
            Arc::clone(&writers),
        )?);
        let parser = Arc::new(PreparedJsonParserPool::prepare(
            memory,
            region,
            capacity.parser,
            Arc::clone(&writers),
        )?);
        let mut contexts = PreparedParseContextPool::prepare(memory, region, capacity.contexts)?
            .with_json_validation(Arc::clone(&validation));
        if let Some(issues) = issues {
            contexts = contexts.with_issue_memory(issues);
        }
        let contexts = Arc::new(contexts);
        Ok(Self {
            writers,
            validation,
            parser,
            contexts,
        })
    }
    /// Borrow the single writer pool.
    #[must_use]
    pub fn writers(&self) -> &PreparedJsonWriterPool {
        &self.writers
    }
    /// Retain the single writer pool.
    #[must_use]
    pub fn shared_writers(&self) -> Arc<PreparedJsonWriterPool> {
        Arc::clone(&self.writers)
    }
    /// Borrow representation and issue scratch.
    #[must_use]
    pub fn validation(&self) -> &PreparedJsonValidationPool {
        &self.validation
    }
    /// Retain representation and issue scratch.
    #[must_use]
    pub fn shared_validation(&self) -> Arc<PreparedJsonValidationPool> {
        Arc::clone(&self.validation)
    }
    /// Borrow the managed parser pool.
    #[must_use]
    pub fn parser(&self) -> &PreparedJsonParserPool {
        &self.parser
    }
    /// Retain the managed parser pool.
    #[must_use]
    pub fn shared_parser(&self) -> Arc<PreparedJsonParserPool> {
        Arc::clone(&self.parser)
    }
    /// Borrow the context pool selected with this validation pool.
    #[must_use]
    pub fn contexts(&self) -> &PreparedParseContextPool {
        &self.contexts
    }
    /// Retain the context pool selected with this validation pool.
    #[must_use]
    pub fn shared_contexts(&self) -> Arc<PreparedParseContextPool> {
        Arc::clone(&self.contexts)
    }
}
