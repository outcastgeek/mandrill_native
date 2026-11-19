//! Managed object routing with caller-owned entries and route indexes.
use super::prepared_alternatives::{Budget, error_array, memory};
use super::{
    CheckedPattern, IssueSegment, JsonWriter, MultiplePatternPolicy, ParseContext, ParsePolicy,
    PreparedIssueMemory, PreparedJsonArray, PreparedJsonArrayPool, PreparedJsonKind,
    PreparedJsonParserPool, PreparedJsonString, PreparedJsonValidationPool, PureCodec, SchemaAst,
    SchemaIssue, SchemaIssueKind, SchemaResource, SharedPreparedJsonValue, ValueKind,
};
use std::{collections::BTreeMap, sync::Arc};

fn invalid() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
}
/// One pattern route in a managed object definition.
pub struct PreparedPatternProperty<A> {
    /// Pattern with an explicitly prepared cache.
    pub pattern: CheckedPattern,
    /// Value codec selected by the pattern.
    pub codec: PureCodec<A, SharedPreparedJsonValue>,
}
/// Managed counterpart to object constraints. Definition metadata is startup-owned.
pub struct PreparedObjectConstraints<A> {
    /// Genuine name decoder and independent typed name validator.
    pub names: PureCodec<PreparedJsonString, SharedPreparedJsonValue>,
    /// Startup declared routes.
    pub declared: BTreeMap<String, PureCodec<A, SharedPreparedJsonValue>>,
    /// Pattern routes in declaration order.
    pub patterns: Vec<PreparedPatternProperty<A>>,
    /// Fallback when neither a declared nor a pattern route matches.
    pub additional: Option<PureCodec<A, SharedPreparedJsonValue>>,
    /// Inclusive minimum property count.
    pub minimum: usize,
    /// Inclusive maximum property count.
    pub maximum: usize,
    /// Multiple-pattern handling.
    pub multiple_patterns: MultiplePatternPolicy,
}
/// A retained original key and its first decoded value.
pub struct PreparedObjectEntry<A: Send + 'static> {
    name: PreparedJsonString,
    value: A,
}
impl<A: Send + 'static> PreparedObjectEntry<A> {
    pub(crate) fn new(name: PreparedJsonString, value: A) -> Self {
        Self { name, value }
    }
    pub(crate) fn into_parts(self) -> (PreparedJsonString, A) {
        (self.name, self.value)
    }
    /// Borrow the retained original property name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    /// Borrow the decoded property value.
    #[must_use]
    pub fn value(&self) -> &A {
        &self.value
    }
}
/// An immutable actual prepared array of managed object entries.
pub struct PreparedObjectValue<A: Send + 'static> {
    entries: PreparedJsonArray<PreparedObjectEntry<A>>,
}
impl<A: Send + 'static> PreparedObjectValue<A> {
    pub(crate) fn take_named(&mut self, name: &str) -> Option<A> {
        let index = self
            .entries
            .as_slice()
            .iter()
            .position(|entry| entry.name() == name)?;
        self.entries
            .remove_at(index)
            .map(|entry| entry.into_parts().1)
    }
    pub(crate) fn remove_first(&mut self) -> Option<PreparedObjectEntry<A>> {
        self.entries.remove_first()
    }

    pub(crate) fn from_entries(entries: PreparedJsonArray<PreparedObjectEntry<A>>) -> Self {
        Self { entries }
    }
    pub(crate) fn into_entries(self) -> PreparedJsonArray<PreparedObjectEntry<A>> {
        self.entries
    }
    /// Reserve an entry array before consuming the iterator; duplicate names replace the prior value.
    /// On refusal the iterator and any partial entries are retired normally.
    pub fn try_from_iter(
        pool: &PreparedJsonArrayPool<PreparedObjectEntry<A>>,
        entries: impl IntoIterator<Item = (PreparedJsonString, A)>,
    ) -> Result<Self, crate::memory::MemoryError> {
        let mut values = pool.reserve()?;
        for (name, value) in entries {
            if let Some(existing) = values
                .as_mut_slice()
                .iter_mut()
                .find(|entry| entry.name() == name.as_str())
            {
                existing.value = value;
            } else {
                values
                    .push(PreparedObjectEntry { name, value })
                    .map_err(|_| crate::memory::MemoryError::NoSlot)?;
            }
        }
        Ok(Self { entries: values })
    }
    /// Borrow entries in retained input order.
    #[must_use]
    pub fn entries(&self) -> &[PreparedObjectEntry<A>] {
        self.entries.as_slice()
    }
}
enum Routes {
    Empty,
    Allocated(PreparedJsonArray<usize>),
}
enum NameMode<'a> {
    Decode,
    Typed(Option<&'a JsonWriter>),
}
impl Routes {
    fn as_slice(&self) -> &[usize] {
        match self {
            Self::Empty => &[],
            Self::Allocated(values) => values.as_slice(),
        }
    }
    fn push(&mut self, value: usize) -> Result<(), usize> {
        match self {
            Self::Empty => Err(value),
            Self::Allocated(values) => values.push(value),
        }
    }
}
struct Diagnostics {
    count: Arc<str>,
    range: Arc<str>,
    multiple: Arc<str>,
    reject: Arc<str>,
}
impl Diagnostics {
    fn count(&self) -> SchemaIssue {
        SchemaIssue::new(SchemaIssueKind::Constraint {
            name: Arc::clone(&self.count),
            operand: Arc::clone(&self.range),
        })
    }
    fn multiple(&self) -> SchemaIssue {
        SchemaIssue::new(SchemaIssueKind::Constraint {
            name: Arc::clone(&self.multiple),
            operand: Arc::clone(&self.reject),
        })
    }
}
fn segment(
    name: &str,
    property: bool,
    context: Option<&ParseContext>,
    issues: &PreparedIssueMemory,
) -> Result<IssueSegment, SchemaIssue> {
    let memory = context
        .and_then(ParseContext::issue_memory)
        .or(Some(issues));
    let text = super::schema_issue::format_diagnostic_text(memory, format_args!("{name}"))?;
    Ok(if property {
        IssueSegment::PreparedPropertyName(text)
    } else {
        IssueSegment::PreparedField(text)
    })
}
fn prefix(
    context: Option<&ParseContext>,
    issues: &PreparedIssueMemory,
    issue: SchemaIssue,
    segment: IssueSegment,
    writer: Option<&JsonWriter>,
) -> SchemaIssue {
    if let Some(writer) = writer {
        writer.issue_at(issue, segment)
    } else {
        super::parse_context::selected_issue_at(
            context
                .and_then(ParseContext::issue_memory)
                .or(Some(issues)),
            issue,
            segment,
        )
    }
}
impl<A: Send + 'static> PreparedObjectConstraints<A> {
    fn max_routes(&self) -> Result<usize, SchemaIssue> {
        if self.declared.is_empty() {
            Ok(self
                .patterns
                .len()
                .max(usize::from(self.additional.is_some())))
        } else {
            self.patterns
                .len()
                .checked_add(1)
                .ok_or_else(|| memory(crate::memory::MemoryError::ArithmeticOverflow))
        }
    }
    fn select(
        &self,
        name: &PreparedJsonString,
        context: Option<&ParseContext>,
        issues: &PreparedIssueMemory,
        mut routes: Routes,
        mode: NameMode<'_>,
        diagnostics: &Diagnostics,
    ) -> Result<Routes, SchemaIssue> {
        let name_path = segment(name.as_str(), true, context, issues)?;
        let child_path = name_path.try_clone_prepared().map_err(memory)?;
        let child = super::parse_context::child(context, child_path)?;
        if let NameMode::Typed(writer) = mode {
            self.names
                .validate_in_ref(name, child.as_ref())
                .map_err(|issue| prefix(context, issues, issue, name_path, writer))?;
        } else {
            let encoded = name.retain_value().map_err(memory)?;
            self.names
                .decode_in_ref(&encoded, child.as_ref())
                .map_err(|issue| prefix(context, issues, issue, name_path, None))?;
        }
        if self.declared.contains_key(name.as_str()) {
            routes
                .push(0)
                .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?;
        }
        let mut patterns = 0usize;
        for (index, pattern) in self.patterns.iter().enumerate() {
            if pattern.pattern.matches_in(name.as_str(), context)? {
                patterns = patterns
                    .checked_add(1)
                    .ok_or_else(|| memory(crate::memory::MemoryError::ArithmeticOverflow))?;
                routes
                    .push(
                        index.checked_add(1).ok_or_else(|| {
                            memory(crate::memory::MemoryError::ArithmeticOverflow)
                        })?,
                    )
                    .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?;
            }
        }
        if patterns > 1 && matches!(self.multiple_patterns, MultiplePatternPolicy::Reject) {
            return Err(diagnostics.multiple());
        }
        if routes.as_slice().is_empty() {
            if self.additional.is_some() {
                routes
                    .push(usize::MAX)
                    .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?;
            } else {
                return Err(super::prepared_unknown_field(
                    context,
                    Some(issues),
                    name.as_str(),
                ));
            }
        }
        Ok(routes)
    }
    fn route(
        &self,
        name: &str,
        route: usize,
    ) -> Result<&PureCodec<A, SharedPreparedJsonValue>, SchemaIssue> {
        if route == 0 {
            self.declared.get(name).ok_or_else(invalid)
        } else if route == usize::MAX {
            self.additional.as_ref().ok_or_else(invalid)
        } else {
            self.patterns
                .get(route - 1)
                .map(|entry| &entry.codec)
                .ok_or_else(invalid)
        }
    }
}
fn count<A: Send + 'static>(
    length: usize,
    constraints: &PreparedObjectConstraints<A>,
    policy: ParsePolicy,
    diagnostics: &Diagnostics,
) -> Result<(), SchemaIssue> {
    if length > policy.limits.collection.get() {
        return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
            resource: SchemaResource::Width,
        }));
    }
    if length < constraints.minimum || length > constraints.maximum {
        return Err(diagnostics.count());
    }
    Ok(())
}
fn retain_issue(
    issue: SchemaIssue,
    policy: ParsePolicy,
    budget: &mut Budget,
    before: usize,
    errors: &mut Option<super::PreparedJsonArrayBuilder<SchemaIssue>>,
    validation: &PreparedJsonValidationPool,
) -> Result<(), SchemaIssue> {
    if policy.errors == super::SchemaErrorMode::First {
        return Err(issue);
    }
    if super::prepared_alternatives::root_resource(&issue) {
        return Err(issue);
    }
    let scan = validation.scan_issue(&issue)?;
    if scan.resource {
        return Err(issue);
    }
    budget.record(before, scan.count)?;
    errors
        .as_mut()
        .ok_or_else(invalid)?
        .try_push(issue)
        .map_err(|_| memory(crate::memory::MemoryError::NoSlot))
}
fn finish_issues(
    errors: Option<super::PreparedJsonArrayBuilder<SchemaIssue>>,
    budget: &mut Budget,
    initial: usize,
    validation: &PreparedJsonValidationPool,
) -> Result<(), SchemaIssue> {
    if let Some(mut errors) = errors
        && !errors.is_empty()
    {
        let issue = if errors.len() == 1 {
            errors.remove_first().ok_or_else(invalid)?
        } else {
            SchemaIssue::new(SchemaIssueKind::Multiple).with_prepared_children(errors.publish())
        };
        let count = validation.scan_issue(&issue)?.count;
        budget.record(initial, count)?;
        return Err(issue);
    }
    Ok(())
}
fn routes<A: Send + 'static>(
    constraints: &PreparedObjectConstraints<A>,
    pool: &PreparedJsonArrayPool<usize>,
) -> Result<Routes, SchemaIssue> {
    let required = constraints.max_routes()?;
    if required == 0 {
        return Ok(Routes::Empty);
    }
    let routes = pool.reserve().map_err(memory)?;
    if routes.capacity() < required {
        return Err(memory(crate::memory::MemoryError::NoSlot));
    }
    Ok(Routes::Allocated(routes))
}
/// Managed object constraints with caller-prepared entry, route and document backing.
pub fn json_prepared_object_constraints<A: Send + 'static>(
    constraints: PreparedObjectConstraints<A>,
    policy: ParsePolicy,
    entries: Arc<PreparedJsonArrayPool<PreparedObjectEntry<A>>>,
    route_pool: Arc<PreparedJsonArrayPool<usize>>,
    output: Arc<PreparedJsonParserPool>,
    validation: Arc<PreparedJsonValidationPool>,
    issues: Arc<PreparedIssueMemory>,
) -> Result<PureCodec<PreparedObjectValue<A>, SharedPreparedJsonValue>, SchemaIssue> {
    if constraints.minimum > constraints.maximum
        || constraints.maximum > policy.limits.collection.get()
    {
        return Err(invalid());
    }
    let _ = constraints.max_routes()?;
    if constraints
        .patterns
        .iter()
        .any(|entry| !entry.pattern.has_prepared_cache())
    {
        return Err(memory(crate::memory::MemoryError::UnsupportedRunContract));
    }
    let diagnostics = Arc::new(Diagnostics {
        count: Arc::from("propertyCount"),
        range: Arc::from(format!("{}..={}", constraints.minimum, constraints.maximum)),
        multiple: Arc::from("multiplePatterns"),
        reject: Arc::from("reject"),
    });
    let constraints = Arc::new(constraints);
    let has_validator = constraints.names.has_type_validator()
        && constraints
            .declared
            .values()
            .all(PureCodec::has_type_validator)
        && constraints
            .patterns
            .iter()
            .all(|p| p.codec.has_type_validator())
        && constraints
            .additional
            .as_ref()
            .is_none_or(PureCodec::has_type_validator);
    let has_writer = has_validator
        && constraints.declared.values().all(PureCodec::has_writer)
        && constraints.patterns.iter().all(|p| p.codec.has_writer())
        && constraints
            .additional
            .as_ref()
            .is_none_or(PureCodec::has_writer);
    let decode = Arc::clone(&constraints);
    let validate = Arc::clone(&constraints);
    let write = Arc::clone(&constraints);
    let dd = Arc::clone(&diagnostics);
    let vd = Arc::clone(&diagnostics);
    let wd = Arc::clone(&diagnostics);
    let di = Arc::clone(&issues);
    let vi = Arc::clone(&issues);
    let wi = Arc::clone(&issues);
    let dv = Arc::clone(&validation);
    let dr = Arc::clone(&route_pool);
    let vr = Arc::clone(&route_pool);
    let wr = Arc::clone(&route_pool);
    let name_output = Arc::clone(&output);
    let writer_output = Arc::clone(&output);
    let result = PureCodec::with_context_ast(
        SchemaAst::opaque(),
        move |input: &SharedPreparedJsonValue, context| {
            let active = super::parse_context::policy(context, policy);
            dv.check_parse_limits(input, &active.limits)?;
            let root = input.view();
            if root.kind() != Some(PreparedJsonKind::Object) {
                return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                    expected: ValueKind::Record,
                    actual: super::prepared_view_kind(root)?,
                }));
            }
            // Decode's prior check_limits has already performed the width check.
            if root.len() < decode.minimum || root.len() > decode.maximum {
                return Err(dd.count());
            }
            let mut values = entries.reserve().map_err(memory)?;
            if values.capacity() < root.len() {
                return Err(memory(crate::memory::MemoryError::NoSlot));
            }
            let error_capacity = root
                .len()
                .saturating_mul(decode.max_routes()?.max(1))
                .min(active.limits.issues.get());
            let mut errors =
                if active.errors == super::SchemaErrorMode::First || error_capacity == 0 {
                    None
                } else {
                    Some(error_array(&di, error_capacity)?)
                };
            let mut budget = Budget::new(context, active);
            let initial = budget.used();
            let mut cursor = root.first_child();
            while let Some(item) = cursor {
                if active.errors != super::SchemaErrorMode::First {
                    budget.available()?;
                }
                let before = budget.used();
                let name = item.property_name().ok_or_else(invalid)?;
                let slot = routes(&decode, &dr)?;
                let name = name_output.prepare_name(name, active)?;
                let selected_routes =
                    match decode.select(&name, context, &di, slot, NameMode::Decode, &dd) {
                        Ok(routes) => routes,
                        Err(issue) => {
                            retain_issue(issue, active, &mut budget, before, &mut errors, &dv)?;
                            cursor = item.next_sibling();
                            continue;
                        }
                    };
                let encoded = input
                    .try_get(name.as_str())
                    .map_err(memory)?
                    .ok_or_else(invalid)?;
                let mut selected = None;
                let mut failed = false;
                for route in selected_routes.as_slice() {
                    if active.errors != super::SchemaErrorMode::First {
                        budget.available()?;
                    }
                    let before = budget.used();
                    let path = segment(name.as_str(), false, context, &di)?;
                    let child = super::parse_context::child(
                        context,
                        path.try_clone_prepared().map_err(memory)?,
                    )?;
                    match decode
                        .route(name.as_str(), *route)?
                        .decode_in_ref(&encoded, child.as_ref())
                    {
                        Ok(value) => {
                            if selected.is_none() {
                                selected = Some(value);
                            }
                        }
                        Err(issue) => {
                            failed = true;
                            let issue = prefix(context, &di, issue, path, None);
                            retain_issue(issue, active, &mut budget, before, &mut errors, &dv)?;
                        }
                    }
                }
                if !failed {
                    values
                        .push(PreparedObjectEntry {
                            name,
                            value: selected.ok_or_else(invalid)?,
                        })
                        .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?;
                }
                cursor = item.next_sibling();
            }
            finish_issues(errors, &mut budget, initial, &dv)?;
            Ok(PreparedObjectValue { entries: values })
        },
        move |values: &PreparedObjectValue<A>, context| {
            let active = super::parse_context::policy(context, policy);
            if values.entries().len() < constraints.minimum
                || values.entries().len() > constraints.maximum
            {
                return Err(diagnostics.count());
            }
            let mut object = output.admit_document(active)?;
            let root = object.node(super::internal::parser::AssemblyKind::Object, 1)?;
            for entry in values.entries() {
                let slot = routes(&constraints, &route_pool)?;
                let selected_routes = constraints.select(
                    &entry.name,
                    context,
                    &issues,
                    slot,
                    NameMode::Decode,
                    &diagnostics,
                )?;
                let mut merge = output.admit_document(active)?;
                let mut merged = None;
                for route in selected_routes.as_slice() {
                    let path = segment(entry.name(), false, context, &issues)?;
                    let child = super::parse_context::child(
                        context,
                        path.try_clone_prepared().map_err(memory)?,
                    )?;
                    let next = constraints
                        .route(entry.name(), *route)?
                        .encode_in_ref(&entry.value, child.as_ref())
                        .map_err(|issue| prefix(context, &issues, issue, path, None))?;
                    if let Some(root) = merged {
                        merge.merge_value(root, next.view(), Some(&issues))?;
                    } else {
                        merged = Some(merge.merge_root(next.view(), Some(&issues))?);
                    }
                }
                let value = merge.finish_merge_for_writer(merged.ok_or_else(invalid)?)?;
                object.merge_object_insert(root, entry.name(), value.view(), Some(&issues))?;
            }
            object.finish_merge(root, &validation)
        },
    );
    let result = if has_validator {
        result.with_context_type_validator(move |values: &PreparedObjectValue<A>, context| {
            let active = super::parse_context::policy(context, policy);
            count(values.entries().len(), &validate, active, &vd)?;
            for entry in values.entries() {
                let slot = routes(&validate, &vr)?;
                let selected_routes =
                    validate.select(&entry.name, context, &vi, slot, NameMode::Typed(None), &vd)?;
                for route in selected_routes.as_slice() {
                    let path = segment(entry.name(), false, context, &vi)?;
                    let child = super::parse_context::child(
                        context,
                        path.try_clone_prepared().map_err(memory)?,
                    )?;
                    validate
                        .route(entry.name(), *route)?
                        .validate_in_ref(&entry.value, child.as_ref())
                        .map_err(|issue| prefix(context, &vi, issue, path, None))?;
                }
            }
            Ok(())
        })
    } else {
        result
    };
    Ok(if has_writer {
        result.with_writer(move |values, writer| {
            count(values.entries().len(), &write, policy, &wd)?;
            writer.begin_object()?;
            for entry in values.entries() {
                let slot = routes(&write, &wr)?;
                let selected_routes = write.select(
                    &entry.name,
                    None,
                    &wi,
                    slot,
                    NameMode::Typed(Some(writer)),
                    &wd,
                )?;
                let mut merge = writer_output.admit_document(policy)?;
                let mut merged = None;
                for route in selected_routes.as_slice() {
                    let next = writer_output
                        .encode_with_writer_policy(
                            write.route(entry.name(), *route)?,
                            &entry.value,
                            policy,
                            writer.policy(),
                        )
                        .map_err(|error| match error {
                            super::internal::parser::PreparedWriteStageError::Admission(issue)
                            | super::internal::parser::PreparedWriteStageError::Write(issue)
                            | super::internal::parser::PreparedWriteStageError::Parse(issue) => {
                                issue
                            }
                        })?;
                    if let Some(root) = merged {
                        merge.merge_value(root, next.view(), Some(&wi))?;
                    } else {
                        merged = Some(merge.merge_root(next.view(), Some(&wi))?);
                    }
                }
                let value = merge.finish_merge_for_writer(merged.ok_or_else(invalid)?)?;
                writer.property(entry.name())?;
                value.view().write_to(writer)?;
            }
            writer.end_object()
        })
    } else {
        result
    })
}
