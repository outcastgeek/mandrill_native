use std::fmt;

/// Exact bytes. Decimal and binary constructors use checked arithmetic.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Bytes(usize);

impl From<usize> for Bytes {
    fn from(value: usize) -> Self {
        Self::new(value)
    }
}

impl Bytes {
    #[must_use]
    pub const fn new(bytes: usize) -> Self {
        Self(bytes)
    }
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
    pub const fn kib(value: usize) -> Result<Self, MemoryError> {
        Self::new(value).checked_mul(1024)
    }
    pub const fn mib(value: usize) -> Result<Self, MemoryError> {
        Self::new(value).checked_mul(1024 * 1024)
    }
    pub const fn kb(value: usize) -> Result<Self, MemoryError> {
        Self::new(value).checked_mul(1000)
    }
    pub const fn mb(value: usize) -> Result<Self, MemoryError> {
        Self::new(value).checked_mul(1000 * 1000)
    }
    pub const fn checked_add(self, other: Self) -> Result<Self, MemoryError> {
        match self.0.checked_add(other.0) {
            Some(value) => Ok(Self(value)),
            None => Err(MemoryError::ArithmeticOverflow),
        }
    }
    pub const fn checked_mul(self, count: usize) -> Result<Self, MemoryError> {
        match self.0.checked_mul(count) {
            Some(value) => Ok(Self(value)),
            None => Err(MemoryError::ArithmeticOverflow),
        }
    }
}

/// Numeric identity within a memory plan; no dynamically allocated labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionId(pub u32);

/// Explicit execution policy. Neither policy grows backing storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryPolicy {
    /// Allocation requests are permitted only before sealing. Prepared owners
    /// remain usable afterwards; higher-level pools lease those existing owners.
    PreparedReuse,
    /// Execution may fallibly allocate and recycle within the fixed region.
    BoundedAllocation,
}

/// A partition's complete storage allowance, including its allocation headers
/// and padding. Payload demand must not assume this bookkeeping is free.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Partition {
    pub id: RegionId,
    pub capacity: Bytes,
}

/// Errors remain ordinary typed outcomes; no overload path aborts on OOM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryError {
    ArithmeticOverflow,
    OwnerLimit,
    NoSlot,
    CollectionReservationFailed,
    Busy,
    InvalidUtf8Boundary,
    InvalidRange,
    MissingRuntimeStorage,
    UnsupportedRunContract,
    ConstructionAlreadyBound,
    ConstructionUnbound,
    ConstructionDuplicateKey(u64),
    ConstructionMissingKey(u64),
    ConstructionTypeMismatch(u64),
    ConstructionAlreadyClaimed(u64),
    ControlPoolAlias,
    ValueTooLarge {
        requested: Bytes,
        capacity: Bytes,
    },
    CapacityExceeded {
        resource: usize,
        requested: usize,
        available: usize,
        control: bool,
    },
    InvalidLayout,
    DuplicateRegion(RegionId),
    UnknownRegion(RegionId),
    BudgetExceeded {
        required: Bytes,
        available: Bytes,
    },
    BackingAllocationFailed,
    BackingExceedsBudget {
        returned: Bytes,
        budget: Bytes,
    },
    UnsupportedAlignment {
        requested: usize,
        supported: usize,
    },
    Sealed,
    NotSealed,
    PreparedOnly,
    Closed,
    Exhausted {
        region: RegionId,
        requested: Bytes,
    },
}

impl fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "memory: {self:?}")
    }
}
impl std::error::Error for MemoryError {}

/// Startup layout accounting. All fields describe the allocator-visible block,
/// not malloc rounding, process RSS, native provider memory or thread stacks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryReport {
    /// Declared managed-region budget (not an admission quota).
    pub reserved: Bytes,
    /// Allocator-visible owned block, or exclusively borrowed region bytes.
    /// Includes control, padding and unassigned capacity; not RSS/page commitment.
    pub committed: Bytes,
    /// Compatibility name for committed. Breakdown fields sum to this total.
    pub backing: Bytes,
    pub control: Bytes,
    pub partition_storage: Bytes,
    pub padding: Bytes,
    pub unassigned: Bytes,
    pub alignment: usize,
}

/// A synchronized partition snapshot; rejected counts saturate rather than wrap.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemoryUsage {
    /// Declared partition allowance, including any charged shared metadata.
    pub reserved: Bytes,
    /// Usable fixed partition span, already backed at preparation.
    pub committed: Bytes,
    /// Subset of active_backing held at explicit retention/share boundaries.
    pub retained_backing: Bytes,
    pub high_water_retained: Bytes,
    /// Requested sizes of all live nonzero allocations, including idle pools.
    pub active_payload: Bytes,
    /// Full live free-list charges, including retained_backing (not additive).
    pub active_backing: Bytes,
    pub high_water_backing: Bytes,
    pub active_allocations: usize,
    pub rejected_allocations: usize,
}
