//! Bounded record-issue accumulation using caller-prepared owners.
use super::{
    ParsePolicy, PreparedJsonArrayBuilder, PreparedJsonArrayPool, PreparedJsonValidationPool,
    SchemaErrorMode, SchemaIssue, SchemaIssueKind, SchemaResource,
};
use crate::memory::{Bytes, MemoryError};
/// Exclusive issue accumulator. Its array is admitted before field callbacks.
/// Existing issue kinds and paths retain their original backing; ordinary issue
/// cloning and path modification are not made prepared by this owner.
pub struct PreparedIssueAccumulator<'a> {
    issues: PreparedJsonArrayBuilder<SchemaIssue>,
    validation: &'a PreparedJsonValidationPool,
    policy: ParsePolicy,
    nodes: usize,
    stopped: bool,
    terminal: bool,
    fatal: Option<SchemaIssue>,
}

impl<'a> PreparedIssueAccumulator<'a> {
    /// Admit the physical array and validate capacity before field callbacks.
    pub fn admit(
        pool: &PreparedJsonArrayPool<SchemaIssue>,
        validation: &'a PreparedJsonValidationPool,
        policy: ParsePolicy,
    ) -> Result<Self, MemoryError> {
        let issues = pool.reserve_shared()?;
        let required = policy.limits.issues.get();
        if issues.capacity() < required {
            let requested = std::alloc::Layout::array::<SchemaIssue>(required)
                .map_err(|_| MemoryError::InvalidLayout)?
                .size();
            let capacity = std::alloc::Layout::array::<SchemaIssue>(issues.capacity())
                .map_err(|_| MemoryError::InvalidLayout)?
                .size();
            return Err(MemoryError::ValueTooLarge {
                requested: Bytes::new(requested),
                capacity: Bytes::new(capacity),
            });
        }
        Ok(Self {
            issues,
            validation,
            policy,
            nodes: 0,
            stopped: false,
            terminal: false,
            fatal: None,
        })
    }

    /// Whether field callbacks must stop.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }
    pub(crate) fn require_entries(&self, required: usize) -> Result<(), MemoryError> {
        let available = self.issues.capacity();
        if available < required {
            let requested = std::alloc::Layout::array::<SchemaIssue>(required)
                .map_err(|_| MemoryError::InvalidLayout)?
                .size();
            let capacity = std::alloc::Layout::array::<SchemaIssue>(available)
                .map_err(|_| MemoryError::InvalidLayout)?
                .size();
            return Err(MemoryError::ValueTooLarge {
                requested: Bytes::new(requested),
                capacity: Bytes::new(capacity),
            });
        }
        Ok(())
    }
    pub(crate) fn record_unknown(&mut self, issue: SchemaIssue) -> Result<(), SchemaIssue> {
        if !self.terminal {
            self.stopped = false;
        }
        self.record(issue)
    }
    pub(crate) fn begin_fields(&mut self) {
        if !self.terminal {
            self.stopped = false;
        }
    }
    pub(crate) fn unknown_stopped(&self) -> bool {
        self.terminal
    }

    /// Record an issue. After stopping, returns the unaccepted original issue.
    // The original owned issue can retain prepared children; boxing would allocate.
    #[allow(clippy::result_large_err)]
    pub fn record(&mut self, issue: SchemaIssue) -> Result<(), SchemaIssue> {
        if self.stopped {
            return Err(issue);
        }
        if let Some(kind) = super::prepared_json_validation::physical_cause(&issue) {
            self.issues.try_push(issue)?;
            self.fatal = Some(SchemaIssue::new(kind));
            self.stopped = true;
            self.terminal = true;
            return Ok(());
        }
        let scan = match self.validation.scan_issue(&issue) {
            Ok(scan) => scan,
            Err(fatal) => {
                // count_issue currently constructs only childless failures; the fatal
                // keeps its exact kind/path while the admitted array retains all inputs.
                self.issues.try_push(issue)?;
                self.fatal = Some(fatal);
                self.stopped = true;
                self.terminal = true;
                return Ok(());
            }
        };
        if let Some(kind) = scan.physical {
            self.issues.try_push(issue)?;
            self.fatal = Some(SchemaIssue::new(kind));
            self.stopped = true;
            self.terminal = true;
            return Ok(());
        }
        let nodes = self.nodes.saturating_add(scan.count);
        if nodes.saturating_add(usize::from(!self.issues.is_empty()))
            > self.policy.limits.issues.get()
        {
            self.issues.clear();
            // Capacity is at least the nonzero issue limit, checked at admission.
            self.issues
                .try_push(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: SchemaResource::Issues,
                }))?;
            self.nodes = 1;
            self.stopped = true;
            self.terminal = true;
            return Ok(());
        }
        let fatal = scan.resource;
        self.issues.try_push(issue)?;
        self.nodes = nodes;
        self.terminal = self.policy.errors == SchemaErrorMode::First || fatal;
        self.stopped = self.terminal || nodes >= self.policy.limits.issues.get();
        Ok(())
    }

    /// Return no issue, the single issue, or an aggregate retaining the array.
    #[must_use]
    pub fn finish(mut self) -> Option<SchemaIssue> {
        if let Some(fatal) = self.fatal.take() {
            return Some(fatal.with_prepared_children(self.issues.publish()));
        }
        match self.issues.len() {
            0 => None,
            1 => self.issues.remove_first(),
            _ => Some(
                SchemaIssue::new(SchemaIssueKind::Multiple)
                    .with_prepared_children(self.issues.publish()),
            ),
        }
    }
}
