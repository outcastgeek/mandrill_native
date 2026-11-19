//! Startup-owned PikeVM caches for one compiled checked pattern.
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SlotPool};
use regex_automata::nfa::thompson::{
    self, State,
    pikevm::{Cache, PikeVM},
};

/// Region record bytes and checked logical native cache entry counts.
/// Native cache arrays, temporary NFA storage, Arc headers and allocator
/// rounding are separate and their exact bytes are not established here.
#[derive(Clone, Copy, Debug)]
pub struct PreparedPatternCacheDemand {
    /// Region slot pool bookkeeping and inline Cache records.
    pub region_bytes: Bytes,
    /// Number of simultaneous cache records.
    pub caches: usize,
    /// Conservative epsilon-stack entries required by one `is_match` search.
    pub stack_entries_per_cache: usize,
    /// Initialized state-ID entries after reset, not retained Vec capacity.
    pub active_state_ids_per_cache: usize,
    /// Initialized capture-table entries after reset, not retained Vec capacity.
    pub capture_slots_per_cache: usize,
    /// Logical alternatives in the temporary startup NFA's root fanout.
    pub warmup_fanout: usize,
    /// Logical states in the temporary startup NFA.
    pub warmup_states: usize,
    /// Cache-owned native Vec buffers per cache (some may have zero capacity).
    pub native_vec_buffers_per_cache: usize,
    /// Separately allocated startup Arc control for the cache service.
    pub startup_arc_controls: usize,
}
pub(super) struct PreparedPatternCaches {
    pub pool: SlotPool<'static, Cache>,
}

pub(super) fn demand(
    vm: &PikeVM,
    caches: usize,
) -> Result<PreparedPatternCacheDemand, MemoryError> {
    let nfa = vm.get_nfa();
    if nfa.pattern_len() != 1 {
        return Err(MemoryError::UnsupportedRunContract);
    }
    let mut stack = 1usize;
    for state in nfa.states() {
        let additional = match state {
            State::Union { alternates } => alternates.len().saturating_sub(1),
            State::BinaryUnion { .. } | State::Capture { .. } => 1,
            State::Fail
            | State::Match { .. }
            | State::ByteRange { .. }
            | State::Sparse(_)
            | State::Dense(_)
            | State::Look { .. } => 0,
        };
        stack = stack
            .checked_add(additional)
            .ok_or(MemoryError::ArithmeticOverflow)?;
    }
    let states = nfa.states().len();
    let active = states
        .checked_mul(4)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    let slots = nfa.group_info().slot_len();
    let capture = states
        .checked_mul(slots)
        .and_then(|n| n.checked_add(slots.max(2)))
        .and_then(|n| n.checked_mul(2))
        .ok_or(MemoryError::ArithmeticOverflow)?;
    let fanout = stack
        .checked_add(1)
        .ok_or(MemoryError::ArithmeticOverflow)?
        .max(2);
    let warmup_states = fanout
        .checked_add(1)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    Ok(PreparedPatternCacheDemand {
        region_bytes: SlotPool::<Cache>::storage_bytes(caches)?,
        caches,
        stack_entries_per_cache: stack,
        active_state_ids_per_cache: active,
        capture_slots_per_cache: capture,
        warmup_fanout: fanout,
        warmup_states,
        native_vec_buffers_per_cache: 7,
        startup_arc_controls: 1,
    })
}

pub(super) fn prepare(
    memory: &PreparedMemory<'static>,
    region: RegionId,
    vm: &PikeVM,
    caches: usize,
) -> Result<PreparedPatternCaches, MemoryError> {
    let needed = demand(vm, caches)?.stack_entries_per_cache;
    let count = needed
        .checked_add(1)
        .ok_or(MemoryError::ArithmeticOverflow)?
        .max(2);
    // The builder and its temporary NFA use native startup backing. Its root is
    // checked below before any cache relies on the warm-up fanout.
    let mut builder = thompson::Builder::new();
    builder
        .start_pattern()
        .map_err(|_| MemoryError::InvalidLayout)?;
    let mut alternatives = Vec::new();
    alternatives
        .try_reserve_exact(count)
        .map_err(|_| MemoryError::BackingAllocationFailed)?;
    for _ in 0..count {
        alternatives.push(builder.add_fail().map_err(|_| MemoryError::InvalidLayout)?);
    }
    let start = builder
        .add_union(alternatives)
        .map_err(|_| MemoryError::InvalidLayout)?;
    builder
        .finish_pattern(start)
        .map_err(|_| MemoryError::InvalidLayout)?;
    let nfa = builder
        .build(start, start)
        .map_err(|_| MemoryError::InvalidLayout)?;
    if nfa.pattern_len() != 1 || nfa.has_empty() {
        return Err(MemoryError::UnsupportedRunContract);
    }
    let root = nfa.state(nfa.start_anchored());
    let valid = match root {
        State::Union { alternates } => {
            alternates.len() == count
                && alternates
                    .iter()
                    .all(|&id| matches!(nfa.state(id), State::Fail))
        }
        State::BinaryUnion { alt1, alt2 } => {
            count == 2
                && alt1 != alt2
                && matches!(nfa.state(*alt1), State::Fail)
                && matches!(nfa.state(*alt2), State::Fail)
        }
        State::Fail
        | State::Match { .. }
        | State::ByteRange { .. }
        | State::Sparse(_)
        | State::Dense(_)
        | State::Look { .. }
        | State::Capture { .. } => false,
    };
    if !valid {
        return Err(MemoryError::UnsupportedRunContract);
    }
    let warm = PikeVM::new_from_nfa(nfa).map_err(|_| MemoryError::InvalidLayout)?;
    let pool = memory.slots(region, caches, |_| {
        let mut cache = warm.create_cache();
        // This executes only during caller-requested startup preparation. The
        // verified all-Fail root pushes `needed` Explore frames at once.
        let _ = warm.is_match(&mut cache, "");
        cache.reset(vm);
        Ok(cache)
    })?;
    Ok(PreparedPatternCaches { pool })
}
