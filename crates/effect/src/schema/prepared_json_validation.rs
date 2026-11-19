//! Index-only scratch for the existing configured JSON representation check.
use super::{
    IssueSegment, JsonValue, JsonWriterPolicy, PreparedJsonWriterPool, SchemaIssue,
    SchemaIssueKind, SchemaPolicy, SchemaResource,
};
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
use std::sync::Arc;

/// Physical concurrent representation/issue traversals and per-check traversal depth.
#[derive(Clone, Copy, Debug)]
pub struct PreparedJsonValidationCapacity {
    /// Simultaneously admitted representation or issue-tree traversals.
    pub checks: usize,
    /// Maximum nested container or issue path frames per traversal.
    pub depth: usize,
}
/// Region scratch and separately classified pool Arc payload.
#[derive(Clone, Copy, Debug)]
pub struct PreparedJsonValidationDemand {
    /// Region scratch and pool metadata; writer backing remains separate.
    pub region_bytes: Bytes,
    /// Startup Arc used when selecting this pool for a context.
    pub startup_arc_controls: usize,
    /// Requested Arc payload bytes, excluding header and allocator overhead.
    pub startup_arc_payload_bytes: Bytes,
}
#[derive(Clone, Copy)]
struct Frame {
    next: usize,
}
struct Scratch {
    path: FixedVector<'static, Frame>,
}
struct Lease(SlotLease<'static, Scratch>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.path.clear();
    }
}
/// Fixed scratch and caller-owned prepared writer backing for JSON checks.
pub struct PreparedJsonValidationPool {
    scratch: SlotPool<'static, Scratch>,
    writers: Arc<PreparedJsonWriterPool>,
}

/// Admission for a borrowed JSON subtree limit check.
pub(crate) struct PreparedJsonLimitsReservation {
    scratch: Lease,
    writer: super::JsonWriter,
    limits: super::ParseLimits,
}

impl PreparedJsonLimitsReservation {
    pub(crate) fn check(self, value: super::PreparedJsonValue<'_>) -> Result<(), SchemaIssue> {
        PreparedJsonValidationPool::check_parse_with_scratch(
            value, &self.limits, self.scratch, self.writer,
        )
    }
}
pub(super) struct IssueScan {
    pub count: usize,
    pub resource: bool,
    pub physical: Option<SchemaIssueKind>,
}
pub(crate) fn physical_cause(issue: &SchemaIssue) -> Option<SchemaIssueKind> {
    if let SchemaIssueKind::Memory(error) = issue.kind() {
        return Some(SchemaIssueKind::Memory(*error));
    }
    if let SchemaIssueKind::LimitExceeded { resource } = issue.kind()
        && matches!(
            resource,
            SchemaResource::JsonDocuments
                | SchemaResource::WriterBoundaries
                | SchemaResource::ContextPaths
                | SchemaResource::RepresentationChecks
                | SchemaResource::IssueTraversals
        )
    {
        return Some(SchemaIssueKind::LimitExceeded {
            resource: *resource,
        });
    }
    None
}
impl PreparedJsonValidationCapacity {
    /// Region scratch payload; writer pool and startup Arc controls are separate.
    pub fn demand(self) -> Result<PreparedJsonValidationDemand, MemoryError> {
        Ok(PreparedJsonValidationDemand {
            region_bytes: self.storage_bytes()?,
            startup_arc_controls: 1,
            startup_arc_payload_bytes: Bytes::new(std::mem::size_of::<PreparedJsonValidationPool>()),
        })
    }
    /// Compute actual region scratch backing, excluding separately supplied writer storage.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let path = std::alloc::Layout::array::<Frame>(self.depth)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Scratch>::storage_bytes(self.checks)?
            .checked_add(Bytes::new(path.size()).checked_mul(self.checks)?)
    }
}
impl PreparedJsonValidationPool {
    pub(crate) fn reserve_parse_limits(
        &self,
        limits: super::ParseLimits,
    ) -> Result<PreparedJsonLimitsReservation, MemoryError> {
        let scratch = Lease(self.scratch.try_checkout()?);
        let writer = self.writers.admit_serde_counting(JsonWriterPolicy {
            bytes: limits.bytes,
            depth: limits.depth,
        })?;
        Ok(PreparedJsonLimitsReservation { scratch, writer, limits })
    }
    /// Prepare scratch; the supplied writer pool owns and reports its own backing.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedJsonValidationCapacity,
        writers: Arc<PreparedJsonWriterPool>,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.storage_bytes()?;
        let scratch = memory.slots(region, capacity.checks, |_| {
            Ok(Scratch {
                path: FixedVector::prepare(memory.allocator(region)?, capacity.depth)?,
            })
        })?;
        Ok(Self { scratch, writers })
    }
    pub(super) fn count_issue(&self, root: &SchemaIssue) -> Result<usize, SchemaIssue> {
        self.scan_issue(root).map(|scan| scan.count)
    }
    pub(super) fn scan_issue(&self, root: &SchemaIssue) -> Result<IssueScan, SchemaIssue> {
        let mut scratch = Lease(
            self.scratch
                .checkout()
                .map_err(|_| limit(SchemaResource::IssueTraversals))?,
        );
        let mut count = 0usize;
        let mut resource = false;
        let mut physical = None;
        let mut current = Some(root);
        loop {
            if let Some(issue) = current.take() {
                count = count.saturating_add(1);
                resource |= matches!(
                    issue.kind(),
                    SchemaIssueKind::LimitExceeded { .. }
                        | SchemaIssueKind::Memory(_)
                        | SchemaIssueKind::ResourceDemand { .. }
                        | SchemaIssueKind::PatternBudget
                );
                if physical.is_none() {
                    physical = physical_cause(issue);
                }
                if !issue.children().is_empty() && scratch.0.path.push(Frame { next: 0 }).is_err() {
                    return Err(limit(SchemaResource::Issues));
                }
            }
            loop {
                let Some((frame, ancestors)) = scratch.0.path.as_slice().split_last() else {
                    return Ok(IssueScan {
                        count,
                        resource,
                        physical,
                    });
                };
                let mut parent = root;
                for ancestor in ancestors {
                    let index = ancestor
                        .next
                        .checked_sub(1)
                        .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                    parent = parent
                        .children()
                        .get(index)
                        .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                }
                if let Some(child) = parent.children().get(frame.next) {
                    let frame = scratch
                        .0
                        .path
                        .as_mut_slice()
                        .last_mut()
                        .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                    frame.next = frame
                        .next
                        .checked_add(1)
                        .ok_or_else(|| limit(SchemaResource::Issues))?;
                    current = Some(child);
                    break;
                }
                scratch.0.path.pop();
            }
        }
    }
    pub(super) fn check(
        &self,
        root: &JsonValue,
        policy: SchemaPolicy,
        issues: Option<&super::PreparedIssueMemory>,
    ) -> Result<(), SchemaIssue> {
        self.check_view(View::Native(root), policy, issues)
    }
    pub(super) fn check_prepared(
        &self,
        root: super::PreparedJsonValue<'_>,
        policy: SchemaPolicy,
        issues: Option<&super::PreparedIssueMemory>,
    ) -> Result<(), SchemaIssue> {
        self.check_view(View::Managed(root), policy, issues)
    }
    /// Check the legacy parse byte, depth, node and collection limits on a
    /// managed subtree, using prepared scratch and serde-compatible byte counting.
    /// Other schema constraints and diagnostic paths are not applied here. A
    /// full physical traversal stack reports `Memory(NoSlot)` independently
    /// of the logical depth limit.
    pub fn check_parse_limits(
        &self,
        value: &super::SharedPreparedJsonValue,
        limits: &super::ParseLimits,
    ) -> Result<(), SchemaIssue> {
        let scratch = Lease(
            self.scratch
                .checkout()
                .map_err(|_| limit(SchemaResource::RepresentationChecks))?,
        );
        let writer = self
            .writers
            .admit_serde_counting(JsonWriterPolicy {
                bytes: limits.bytes,
                depth: limits.depth,
            })
            .map_err(|_| limit(SchemaResource::RepresentationChecks))?;
        Self::check_parse_with_scratch(value.view(), limits, scratch, writer)
    }
    pub(super) fn check_parse_with_writer(
        &self,
        value: super::PreparedJsonValue<'_>,
        limits: &super::ParseLimits,
        writer: super::JsonWriter,
    ) -> Result<(), SchemaIssue> {
        let scratch = Lease(
            self.scratch
                .checkout()
                .map_err(|_| limit(SchemaResource::RepresentationChecks))?,
        );
        Self::check_parse_with_scratch(value, limits, scratch, writer)
    }
    fn check_parse_with_scratch(
        value: super::PreparedJsonValue<'_>,
        limits: &super::ParseLimits,
        mut scratch: Lease,
        mut writer: super::JsonWriter,
    ) -> Result<(), SchemaIssue> {
        let root = View::Managed(value);
        let mut nodes = 0usize;
        let mut current = Some(root);
        loop {
            if let Some(value) = current.take() {
                let depth = scratch
                    .0
                    .path
                    .len()
                    .checked_add(1)
                    .ok_or_else(|| limit(SchemaResource::Depth))?;
                if depth > limits.depth.get() {
                    return Err(limit(SchemaResource::Depth));
                }
                nodes = nodes
                    .checked_add(1)
                    .ok_or_else(|| limit(SchemaResource::Steps))?;
                if nodes > limits.nodes.get() {
                    return Err(limit(SchemaResource::Steps));
                }
                if let Kind::Array(count) | Kind::Object(count) = value.kind()? {
                    if count > limits.collection.get() {
                        return Err(limit(SchemaResource::Width));
                    }
                    depth
                        .checked_add(1)
                        .ok_or_else(|| limit(SchemaResource::Depth))?;
                    if count > 0 && scratch.0.path.push(Frame { next: 0 }).is_err() {
                        return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                            MemoryError::NoSlot,
                        )));
                    }
                }
            }
            loop {
                let Some((frame, ancestors)) = scratch.0.path.as_slice().split_last() else {
                    root.write_to(&mut writer)?;
                    return writer.finish_sink();
                };
                let parent = locate_reverse(root, ancestors).ok_or_else(invalid)?;
                let count = match parent.kind()? {
                    Kind::Array(n) | Kind::Object(n) => n,
                    Kind::Null | Kind::Bool | Kind::Number | Kind::String(_) => {
                        return Err(invalid());
                    }
                };
                if let Some(index) = count.checked_sub(frame.next.saturating_add(1)) {
                    let child = parent.child(index)?.ok_or_else(invalid)?.1;
                    scratch
                        .0
                        .path
                        .as_mut_slice()
                        .last_mut()
                        .ok_or_else(invalid)?
                        .next += 1;
                    current = Some(child);
                    break;
                }
                scratch.0.path.pop();
            }
        }
    }
    fn check_view(
        &self,
        root: View<'_>,
        policy: SchemaPolicy,
        issues: Option<&super::PreparedIssueMemory>,
    ) -> Result<(), SchemaIssue> {
        let mut scratch = Lease(
            self.scratch
                .checkout()
                .map_err(|_| limit(SchemaResource::RepresentationChecks))?,
        );
        let mut writer = self
            .writers
            .admit_counting(JsonWriterPolicy {
                bytes: policy.parse.limits.bytes,
                depth: policy.json_depth,
            })
            .map_err(|_| limit(SchemaResource::RepresentationChecks))?;
        let mut nodes = 0usize;
        let mut current = Some(root);
        loop {
            if let Some(value) = current.take() {
                nodes = nodes.checked_add(1).ok_or_else(|| {
                    demand(
                        SchemaResource::Steps,
                        policy.parse.limits.nodes.get(),
                        usize::MAX,
                    )
                })?;
                let path = scratch.0.path.as_slice();
                let depth = path.len().saturating_add(1);
                if depth > policy.json_depth.get() {
                    return Err(located(
                        root,
                        path,
                        issues,
                        None,
                        demand(SchemaResource::JsonDepth, policy.json_depth.get(), depth),
                    ));
                }
                if nodes > policy.parse.limits.nodes.get() {
                    return Err(located(
                        root,
                        path,
                        issues,
                        None,
                        demand(
                            SchemaResource::Steps,
                            policy.parse.limits.nodes.get(),
                            nodes,
                        ),
                    ));
                }
                match value.kind()? {
                    Kind::String(text) => {
                        let count = text.chars().count();
                        if count > policy.string_length.get() {
                            return Err(located(
                                root,
                                path,
                                issues,
                                None,
                                demand(
                                    SchemaResource::StringLength,
                                    policy.string_length.get(),
                                    count,
                                ),
                            ));
                        }
                    }
                    Kind::Number => {
                        let count = value.digits()?;
                        if count > policy.integer_digits.get() {
                            return Err(located(
                                root,
                                path,
                                issues,
                                None,
                                demand(
                                    SchemaResource::IntegerDigits,
                                    policy.integer_digits.get(),
                                    count,
                                ),
                            ));
                        }
                    }
                    Kind::Array(count) | Kind::Object(count) => {
                        if count > policy.parse.limits.collection.get() {
                            return Err(located(
                                root,
                                path,
                                issues,
                                None,
                                demand(
                                    SchemaResource::Width,
                                    policy.parse.limits.collection.get(),
                                    count,
                                ),
                            ));
                        }
                        if matches!(value.kind()?, Kind::Object(_)) {
                            for index in (0..count).rev() {
                                let (name, _) = value.child(index)?.ok_or_else(invalid)?;
                                let name = name.ok_or_else(invalid)?;
                                let length = name.chars().count();
                                if length > policy.string_length.get() {
                                    let issue = demand(
                                        SchemaResource::StringLength,
                                        policy.string_length.get(),
                                        length,
                                    );
                                    return Err(located(root, path, issues, Some(name), issue));
                                }
                            }
                        }
                        if count > 0 && scratch.0.path.push(Frame { next: 0 }).is_err() {
                            return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                                MemoryError::NoSlot,
                            )));
                        }
                    }
                    Kind::Null | Kind::Bool => {}
                }
            }
            loop {
                let Some((frame, ancestors)) = scratch.0.path.as_slice().split_last() else {
                    root.write_to(&mut writer)?;
                    return writer.finish_sink();
                };
                let parent = locate(root, ancestors).ok_or_else(invalid)?;
                if let Some((_, child)) = parent.child(frame.next)? {
                    let frame = scratch
                        .0
                        .path
                        .as_mut_slice()
                        .last_mut()
                        .ok_or_else(invalid)?;
                    frame.next = frame
                        .next
                        .checked_add(1)
                        .ok_or_else(|| limit(SchemaResource::JsonDepth))?;
                    current = Some(child);
                    break;
                }
                scratch.0.path.pop();
            }
        }
    }
}
fn limit(resource: SchemaResource) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::LimitExceeded { resource })
}
fn demand(resource: SchemaResource, limit: usize, observed: usize) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::ResourceDemand {
        resource,
        limit,
        observed,
    })
}
fn invalid() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
}
#[derive(Clone, Copy)]
enum View<'a> {
    Native(&'a JsonValue),
    Managed(super::PreparedJsonValue<'a>),
}
enum Kind<'a> {
    Null,
    Bool,
    Number,
    String(&'a str),
    Array(usize),
    Object(usize),
}
impl<'a> View<'a> {
    fn kind(self) -> Result<Kind<'a>, SchemaIssue> {
        use super::PreparedJsonKind;
        Ok(match self {
            Self::Native(JsonValue::Null) => Kind::Null,
            Self::Native(JsonValue::Bool(_)) => Kind::Bool,
            Self::Native(JsonValue::Number(_)) => Kind::Number,
            Self::Native(JsonValue::String(value)) => Kind::String(value),
            Self::Native(JsonValue::Array(values)) => Kind::Array(values.len()),
            Self::Native(JsonValue::Object(values)) => Kind::Object(values.len()),
            Self::Managed(value) => match value.kind().ok_or_else(invalid)? {
                PreparedJsonKind::Null => Kind::Null,
                PreparedJsonKind::Bool(_) => Kind::Bool,
                PreparedJsonKind::Number => Kind::Number,
                PreparedJsonKind::String => Kind::String(value.as_str().ok_or_else(invalid)?),
                PreparedJsonKind::Array => Kind::Array(value.len()),
                PreparedJsonKind::Object => Kind::Object(value.len()),
            },
        })
    }
    fn child(self, index: usize) -> Result<Option<(Option<&'a str>, Self)>, SchemaIssue> {
        Ok(match self {
            Self::Native(JsonValue::Array(values)) => {
                values.get(index).map(|value| (None, Self::Native(value)))
            }
            Self::Native(JsonValue::Object(values)) => values
                .iter()
                .nth(index)
                .map(|(name, value)| (Some(name.as_str()), Self::Native(value))),
            Self::Managed(value) => {
                let mut child = value.first_child();
                for _ in 0..index {
                    child = match child {
                        Some(value) => value.next_sibling(),
                        None => break,
                    };
                }
                child.map(|value| (value.property_name(), Self::Managed(value)))
            }
            Self::Native(
                JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_),
            ) => None,
        })
    }
    fn digits(self) -> Result<usize, SchemaIssue> {
        struct Digits(usize);
        impl std::fmt::Write for Digits {
            fn write_str(&mut self, value: &str) -> std::fmt::Result {
                self.0 = self
                    .0
                    .saturating_add(value.bytes().filter(u8::is_ascii_digit).count());
                Ok(())
            }
        }
        let mut output = Digits(0);
        match self {
            Self::Native(JsonValue::Number(number)) => {
                let _ = std::fmt::write(&mut output, format_args!("{number}"));
            }
            Self::Managed(value) => {
                let token = value.number_token().ok_or_else(invalid)?;
                return super::managed_number::ManagedNumber::parse(token)
                    .map(super::managed_number::ManagedNumber::digits)
                    .ok_or_else(invalid);
            }
            Self::Native(
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::String(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_),
            ) => return Err(invalid()),
        }
        Ok(output.0)
    }
    fn write_to(self, writer: &mut super::JsonWriter) -> Result<(), SchemaIssue> {
        match self {
            Self::Native(value) => writer.tree(value),
            Self::Managed(value) => value.write_to(writer),
        }
    }
}
fn locate<'a>(root: View<'a>, path: &[Frame]) -> Option<View<'a>> {
    let mut value = root;
    for frame in path {
        value = value.child(frame.next.checked_sub(1)?).ok()??.1;
    }
    Some(value)
}
fn locate_reverse<'a>(root: View<'a>, path: &[Frame]) -> Option<View<'a>> {
    let mut value = root;
    for frame in path {
        let count = match value.kind().ok()? {
            Kind::Array(n) | Kind::Object(n) => n,
            Kind::Null | Kind::Bool | Kind::Number | Kind::String(_) => return None,
        };
        value = value.child(count.checked_sub(frame.next)?).ok()??.1;
    }
    Some(value)
}
fn located(
    root: View<'_>,
    path: &[Frame],
    issues: Option<&super::PreparedIssueMemory>,
    last: Option<&str>,
    issue: SchemaIssue,
) -> SchemaIssue {
    if path.is_empty() && last.is_none() {
        return issue;
    }
    let Some(issues) = issues else {
        return if physical_cause(&issue).is_some() {
            issue
        } else {
            SchemaIssue::new(SchemaIssueKind::Memory(MemoryError::UnsupportedRunContract))
        };
    };
    let segments = path
        .iter()
        .enumerate()
        .filter_map(|(depth, frame)| {
            let index = frame.next.checked_sub(1)?;
            let parent = locate(root, path.get(..depth)?)?;
            match parent.child(index) {
                Ok(Some((Some(name), _))) => Some(
                    super::schema_issue::format_diagnostic_text(
                        Some(issues),
                        format_args!("{name}"),
                    )
                    .map(IssueSegment::PreparedField),
                ),
                Ok(Some((None, _))) => Some(Ok(IssueSegment::Index(index))),
                Ok(None) | Err(_) => None,
            }
        })
        .chain(last.into_iter().map(|name| {
            super::schema_issue::format_diagnostic_text(Some(issues), format_args!("{name}"))
                .map(IssueSegment::PreparedPropertyName)
        }));
    issues.locate(issue, segments)
}
