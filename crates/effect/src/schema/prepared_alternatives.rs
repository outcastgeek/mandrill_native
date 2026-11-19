//! Managed one-of and any-of probes using admitted owners.
use super::{
    IssueBudget, IssueSegment, JsonWriter, ParseContext, ParsePolicy, PreparedIssueMemory,
    PreparedJsonArray, PreparedJsonArrayPool, PreparedJsonValidationPool, PureCodec, SchemaAst,
    SchemaIssue, SchemaIssueKind, SchemaShape, SharedPreparedJsonValue,
};
use std::sync::Arc;

fn invalid() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
}
pub(super) fn root_resource(issue: &SchemaIssue) -> bool {
    matches!(
        issue.kind(),
        SchemaIssueKind::LimitExceeded { .. }
            | SchemaIssueKind::Memory(_)
            | SchemaIssueKind::ResourceDemand { .. }
            | SchemaIssueKind::PatternBudget
    )
}
pub(super) fn memory(error: crate::memory::MemoryError) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::Memory(error))
}
fn prefix(
    context: Option<&ParseContext>,
    service: &PreparedIssueMemory,
    issue: SchemaIssue,
    index: usize,
) -> SchemaIssue {
    let selected = context
        .and_then(ParseContext::issue_memory)
        .or(Some(service));
    super::parse_context::selected_issue_at(selected, issue, IssueSegment::Alternative(index))
}
pub(super) enum Budget {
    Shared(IssueBudget),
    Local { spent: usize, maximum: usize },
}
impl Budget {
    pub(super) fn new(context: Option<&ParseContext>, policy: ParsePolicy) -> Self {
        match context {
            Some(context) => Self::Shared(context.issue_budget()),
            None => Self::Local {
                spent: 0,
                maximum: policy.limits.issues.get(),
            },
        }
    }
    pub(super) fn used(&self) -> usize {
        match self {
            Self::Shared(budget) => budget.used(),
            Self::Local { spent, .. } => *spent,
        }
    }
    pub(super) fn available(&self) -> Result<(), SchemaIssue> {
        match self {
            Self::Shared(budget) => budget.available(),
            Self::Local { spent, maximum } if spent >= maximum => Err(IssueBudget::exhausted()),
            Self::Local { .. } => Ok(()),
        }
    }
    pub(super) fn record(&mut self, before: usize, count: usize) -> Result<(), SchemaIssue> {
        match self {
            Self::Shared(budget) => budget.record_precounted(before, count),
            Self::Local { spent, maximum } => {
                super::parse_context::record_count(spent, *maximum, before, count)
            }
        }
    }
}
pub(super) fn error_array(
    service: &PreparedIssueMemory,
    count: usize,
) -> Result<super::PreparedJsonArrayBuilder<SchemaIssue>, SchemaIssue> {
    let builder = service.child_pool().reserve_shared().map_err(memory)?;
    if builder.capacity() < count {
        return Err(memory(crate::memory::MemoryError::NoSlot));
    }
    Ok(builder)
}
fn decode_all_of<A: Send + 'static>(
    codecs: &[PureCodec<A, SharedPreparedJsonValue>],
    select_decode: usize,
    value: &SharedPreparedJsonValue,
    policy: ParsePolicy,
    context: Option<&ParseContext>,
    validation: &PreparedJsonValidationPool,
    service: &PreparedIssueMemory,
) -> Result<A, SchemaIssue> {
    let policy = super::parse_context::policy(context, policy);
    validation.check_parse_limits(value, &policy.limits)?;
    let mut errors = if policy.errors == super::SchemaErrorMode::First {
        None
    } else {
        Some(error_array(
            service,
            codecs.len().min(policy.limits.issues.get()),
        )?)
    };
    let mut budget = Budget::new(context, policy);
    let initial = budget.used();
    let mut selected = None;
    for (index, codec) in codecs.iter().enumerate() {
        if policy.errors != super::SchemaErrorMode::First {
            budget.available()?;
        }
        let before = budget.used();
        let child = super::parse_context::child(context, IssueSegment::Alternative(index))?;
        match codec.decode_in_ref(value, child.as_ref()) {
            Ok(value) => {
                if index == select_decode {
                    selected = Some(value);
                }
            }
            Err(issue) => {
                if policy.errors == super::SchemaErrorMode::First {
                    return Err(prefix(context, service, issue, index));
                }
                let issue = prefix(context, service, issue, index);
                if root_resource(&issue) {
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
                    .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?;
            }
        }
    }
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
    selected.ok_or_else(invalid)
}
fn decoded_matches<A: Send + 'static>(
    codecs: &[PureCodec<A, SharedPreparedJsonValue>],
    value: &SharedPreparedJsonValue,
    policy: ParsePolicy,
    context: Option<&ParseContext>,
    arrays: &PreparedJsonArrayPool<(usize, A)>,
    validation: &PreparedJsonValidationPool,
    service: &PreparedIssueMemory,
) -> Result<PreparedJsonArray<(usize, A)>, SchemaIssue> {
    let policy = super::parse_context::policy(context, policy);
    validation.check_parse_limits(value, &policy.limits)?;
    let mut matches = arrays.reserve().map_err(memory)?;
    if matches.capacity() < codecs.len() {
        return Err(memory(crate::memory::MemoryError::NoSlot));
    }
    let errors = error_array(service, codecs.len().min(policy.limits.issues.get()))?;
    let _ = decoded_matches_admitted(
        codecs,
        value,
        policy,
        context,
        validation,
        service,
        (&mut matches, errors),
    )?;
    Ok(matches)
}
fn decoded_matches_admitted<A: Send + 'static>(
    codecs: &[PureCodec<A, SharedPreparedJsonValue>],
    value: &SharedPreparedJsonValue,
    policy: ParsePolicy,
    context: Option<&ParseContext>,
    validation: &PreparedJsonValidationPool,
    service: &PreparedIssueMemory,
    admitted: (
        &mut PreparedJsonArray<(usize, A)>,
        super::PreparedJsonArrayBuilder<SchemaIssue>,
    ),
) -> Result<usize, SchemaIssue> {
    let policy = super::parse_context::policy(context, policy);
    let (matches, mut errors) = admitted;
    let mut budget = Budget::new(context, policy);
    let initial = budget.used();
    for (index, codec) in codecs.iter().enumerate() {
        budget.available()?;
        let before = budget.used();
        let child = super::parse_context::child(context, IssueSegment::Alternative(index))?;
        match codec.decode_in_ref(value, child.as_ref()) {
            Ok(value) => matches
                .push((index, value))
                .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?,
            Err(issue) => {
                let issue = prefix(context, service, issue, index);
                if root_resource(&issue) {
                    return Err(issue);
                }
                let scan = validation.scan_issue(&issue)?;
                if scan.resource {
                    return Err(issue);
                }
                budget.record(before, scan.count)?;
                errors
                    .try_push(issue)
                    .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?;
            }
        }
    }
    if matches.as_slice().is_empty() {
        let issue = SchemaIssue::new(SchemaIssueKind::NoAlternative)
            .with_prepared_children(errors.publish());
        let count = validation.scan_issue(&issue)?.count;
        budget.record(initial, count)?;
        Err(issue)
    } else {
        Ok(matches.as_slice().len())
    }
}
fn typed_matches<A: Send + 'static>(
    codecs: &[PureCodec<A, SharedPreparedJsonValue>],
    value: &A,
    policy: ParsePolicy,
    context: Option<&ParseContext>,
    validation: &PreparedJsonValidationPool,
    service: &PreparedIssueMemory,
    writer: Option<&JsonWriter>,
) -> Result<super::schema_issue::PreparedMatchIndices, SchemaIssue> {
    let policy = super::parse_context::policy(context, policy);
    let mut matches = service
        .reserve_match_indices(codecs.len())
        .map_err(memory)?;
    let mut errors = error_array(service, codecs.len().min(policy.limits.issues.get()))?;
    let mut budget = Budget::new(context, policy);
    let initial = budget.used();
    for (index, codec) in codecs.iter().enumerate() {
        budget.available()?;
        let before = budget.used();
        let child = super::parse_context::child(context, IssueSegment::Alternative(index))?;
        match codec.validate_in_ref(value, child.as_ref()) {
            Ok(()) => matches.push(index).map_err(memory)?,
            Err(issue) => {
                let issue = if let Some(writer) = writer {
                    writer.issue_at(issue, IssueSegment::Alternative(index))
                } else {
                    prefix(context, service, issue, index)
                };
                if root_resource(&issue) {
                    return Err(issue);
                }
                let scan = validation.scan_issue(&issue)?;
                if scan.resource {
                    return Err(issue);
                }
                budget.record(before, scan.count)?;
                errors
                    .try_push(issue)
                    .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?;
            }
        }
    }
    if matches.as_slice().is_empty() {
        let issue = SchemaIssue::new(SchemaIssueKind::NoAlternative)
            .with_prepared_children(errors.publish());
        let count = validation.scan_issue(&issue)?.count;
        budget.record(initial, count)?;
        Err(issue)
    } else {
        Ok(matches)
    }
}
/// Retained original managed document and all actual decoded branch results.
pub struct PreparedAnyOfValue<A: Send + 'static> {
    input: SharedPreparedJsonValue,
    matches: PreparedJsonArray<(usize, A)>,
}
impl<A: Send + 'static> PreparedAnyOfValue<A> {
    /// Borrow the retained original document view.
    #[must_use]
    pub fn input(&self) -> &SharedPreparedJsonValue {
        &self.input
    }
    /// Borrow successful results in declaration order.
    #[must_use]
    pub fn matches(&self) -> &[(usize, A)] {
        self.matches.as_slice()
    }
}
/// Managed any-of with physically admitted match values and diagnostic arrays.
pub fn json_prepared_any_of<A: Send + 'static>(
    codecs: Vec<PureCodec<A, SharedPreparedJsonValue>>,
    policy: ParsePolicy,
    arrays: Arc<PreparedJsonArrayPool<(usize, A)>>,
    validation: Arc<PreparedJsonValidationPool>,
    issues: Arc<PreparedIssueMemory>,
) -> Result<PureCodec<PreparedAnyOfValue<A>, SharedPreparedJsonValue>, SchemaIssue> {
    if codecs.is_empty() || codecs.len() > policy.limits.collection.get() {
        return Err(invalid());
    }
    let ast = SchemaAst::new(SchemaShape::Union(
        codecs.iter().map(|c| c.ast().clone()).collect(),
    ));
    let codecs: Arc<[PureCodec<A, SharedPreparedJsonValue>]> = codecs.into();
    let has_validator = codecs.iter().all(PureCodec::has_type_validator);
    let decode = Arc::clone(&codecs);
    let validate = Arc::clone(&codecs);
    let write = Arc::clone(&codecs);
    let dv = Arc::clone(&validation);
    let vi = Arc::clone(&issues);
    let wi = Arc::clone(&issues);
    let vv = Arc::clone(&validation);
    let wv = Arc::clone(&validation);
    let result = PureCodec::with_context_ast(
        ast,
        move |input: &SharedPreparedJsonValue, context| {
            let owned_input = input.try_clone().map_err(memory)?;
            let matches = decoded_matches(&decode, input, policy, context, &arrays, &dv, &issues)?;
            Ok(PreparedAnyOfValue {
                input: owned_input,
                matches,
            })
        },
        move |value: &PreparedAnyOfValue<A>, context| {
            validation.check_parse_limits(
                &value.input,
                &super::parse_context::policy(context, policy).limits,
            )?;
            value.input.try_clone().map_err(memory)
        },
    );
    let result = if has_validator {
        result.with_context_type_validator(move |value, context| {
            validate_any(value, &validate, policy, context, &vv, &vi, None)
        })
    } else {
        result
    };
    Ok(if has_validator {
        result.with_writer(move |value, writer| {
            validate_any(value, &write, policy, None, &wv, &wi, Some(writer))?;
            value.input.view().write_to(writer)
        })
    } else {
        result
    })
}
fn validate_any<A: Send + 'static>(
    value: &PreparedAnyOfValue<A>,
    codecs: &[PureCodec<A, SharedPreparedJsonValue>],
    policy: ParsePolicy,
    context: Option<&ParseContext>,
    validation: &PreparedJsonValidationPool,
    issues: &PreparedIssueMemory,
    writer: Option<&JsonWriter>,
) -> Result<(), SchemaIssue> {
    validation.check_parse_limits(
        &value.input,
        &super::parse_context::policy(context, policy).limits,
    )?;
    if value.matches.as_slice().is_empty() {
        return Err(invalid());
    }
    for (index, value) in value.matches.as_slice() {
        let child = super::parse_context::child(context, IssueSegment::Alternative(*index))?;
        codecs
            .get(*index)
            .ok_or_else(invalid)?
            .validate_in_ref(value, child.as_ref())
            .map_err(|issue| {
                if let Some(writer) = writer {
                    writer.issue_at(issue, IssueSegment::Alternative(*index))
                } else {
                    prefix(context, issues, issue, *index)
                }
            })?;
    }
    Ok(())
}
/// Managed one-of retaining all branch values until probing completes.
pub fn json_prepared_one_of<A: Send + 'static>(
    codecs: Vec<PureCodec<A, SharedPreparedJsonValue>>,
    select_encode: impl Fn(&A) -> usize + Send + Sync + 'static,
    policy: ParsePolicy,
    arrays: Arc<PreparedJsonArrayPool<(usize, A)>>,
    validation: Arc<PreparedJsonValidationPool>,
    issues: Arc<PreparedIssueMemory>,
) -> Result<PureCodec<A, SharedPreparedJsonValue>, SchemaIssue> {
    if codecs.is_empty() || codecs.len() > policy.limits.collection.get() {
        return Err(invalid());
    }
    let ast = SchemaAst::new(SchemaShape::Checked {
        source: SchemaAst::new(SchemaShape::Union(
            codecs.iter().map(|c| c.ast().clone()).collect(),
        )),
        name: "oneOf".into(),
    });
    let codecs: Arc<[PureCodec<A, SharedPreparedJsonValue>]> = codecs.into();
    let has_validator = codecs.iter().all(PureCodec::has_type_validator);
    let has_writer = has_validator && codecs.iter().all(PureCodec::has_writer);
    let decode = Arc::clone(&codecs);
    let validate = Arc::clone(&codecs);
    let write = Arc::clone(&codecs);
    let select = Arc::new(select_encode);
    let vs = Arc::clone(&select);
    let ws = Arc::clone(&select);
    let da = Arc::clone(&arrays);
    let dv = Arc::clone(&validation);
    let di = Arc::clone(&issues);
    let ev = Arc::clone(&validation);
    let ei = Arc::clone(&issues);
    let vi = Arc::clone(&issues);
    let wi = Arc::clone(&issues);
    let wv = Arc::clone(&validation);
    let result = PureCodec::with_context_ast(
        ast,
        move |input: &SharedPreparedJsonValue, context| {
            let mut indices = di.reserve_match_indices(decode.len()).map_err(memory)?;
            let mut matches = decoded_matches(&decode, input, policy, context, &da, &dv, &di)?;
            if matches.as_slice().len() != 1 {
                for (index, _) in matches.as_slice() {
                    indices.push(*index).map_err(memory)?;
                }
                return Err(indices.into_issue());
            }
            matches
                .remove_first()
                .map(|(_, value)| value)
                .ok_or_else(invalid)
        },
        move |value: &A, context| {
            let mut indices = ei.reserve_match_indices(codecs.len()).map_err(memory)?;
            // The branch encoder may invoke callbacks, so reserve probing storage before it runs.
            let mut probe = arrays.reserve().map_err(memory)?;
            if probe.capacity() < codecs.len() {
                return Err(memory(crate::memory::MemoryError::NoSlot));
            }
            let active = super::parse_context::policy(context, policy);
            let errors = error_array(&ei, codecs.len().min(active.limits.issues.get()))?;
            let selected = select(value);
            let child = super::parse_context::child(context, IssueSegment::Alternative(selected))?;
            let encoded = codecs
                .get(selected)
                .ok_or_else(invalid)?
                .encode_in_ref(value, child.as_ref())?;
            ev.check_parse_limits(
                &encoded,
                &super::parse_context::policy(context, policy).limits,
            )?;
            let matches = decoded_matches_admitted(
                &codecs,
                &encoded,
                policy,
                context,
                &ev,
                &ei,
                (&mut probe, errors),
            )?;
            if matches != 1 {
                for (index, _) in probe.as_slice() {
                    indices.push(*index).map_err(memory)?;
                }
                return Err(indices.into_issue());
            }
            Ok(encoded)
        },
    );
    let result = if has_validator {
        result.with_context_type_validator(move |value, context| {
            let matches = typed_matches(&validate, value, policy, context, &validation, &vi, None)?;
            if matches.as_slice().len() != 1 {
                return Err(matches.into_issue());
            }
            if matches.as_slice().first().copied() != Some(vs(value)) {
                return Err(invalid());
            }
            Ok(())
        })
    } else {
        result
    };
    Ok(if has_writer {
        result.with_writer(move |value, writer| {
            let matches = typed_matches(&write, value, policy, None, &wv, &wi, Some(writer))?;
            if matches.as_slice().len() != 1 {
                return Err(matches.into_issue());
            }
            let selected = ws(value);
            if matches.as_slice().first().copied() != Some(selected) {
                return Err(invalid());
            }
            write
                .get(selected)
                .ok_or_else(invalid)?
                .write_into(value, writer)
        })
    } else {
        result
    })
}

/// Managed all-of preserving branch order and retaining one admitted merge document.
pub fn json_prepared_all_of<A: Send + 'static>(
    codecs: Vec<PureCodec<A, SharedPreparedJsonValue>>,
    select_decode: usize,
    policy: ParsePolicy,
    output: Arc<super::PreparedJsonParserPool>,
    validation: Arc<PreparedJsonValidationPool>,
    issues: Arc<PreparedIssueMemory>,
) -> Result<PureCodec<A, SharedPreparedJsonValue>, SchemaIssue> {
    if codecs.get(select_decode).is_none() || codecs.len() > policy.limits.collection.get() {
        return Err(invalid());
    }
    let ast = SchemaAst::new(SchemaShape::Checked {
        source: SchemaAst::opaque(),
        name: "allOf".into(),
    });
    let codecs: Arc<[PureCodec<A, SharedPreparedJsonValue>]> = codecs.into();
    let decode = Arc::clone(&codecs);
    let validate = Arc::clone(&codecs);
    let write = Arc::clone(&codecs);
    let has_validator = codecs.iter().all(PureCodec::has_type_validator);
    let has_writer = codecs.iter().all(PureCodec::has_writer);
    let decode_validation = Arc::clone(&validation);
    let decode_issues = Arc::clone(&issues);
    let validate_issues = Arc::clone(&issues);
    let writer_issues = Arc::clone(&issues);
    let writer_output = Arc::clone(&output);
    let result = PureCodec::with_context_ast(
        ast,
        move |input: &SharedPreparedJsonValue, context| {
            decode_all_of(
                &decode,
                select_decode,
                input,
                policy,
                context,
                &decode_validation,
                &decode_issues,
            )
        },
        move |value: &A, context| {
            let active = super::parse_context::policy(context, policy);
            let mut merge = output.admit_document(active)?;
            let mut root = None;
            for (index, codec) in codecs.iter().enumerate() {
                let child = super::parse_context::child(context, IssueSegment::Alternative(index))?;
                let next = codec
                    .encode_in_ref(value, child.as_ref())
                    .map_err(|issue| prefix(context, &issues, issue, index))?;
                validation.check_parse_limits(&next, &active.limits)?;
                if let Some(root) = root {
                    merge.merge_value(root, next.view(), Some(&issues))?;
                } else {
                    root = Some(merge.merge_root(next.view(), Some(&issues))?);
                }
            }
            merge.finish_merge(root.ok_or_else(invalid)?, &validation)
        },
    );
    let result = if has_validator {
        result.with_context_type_validator(move |value, context| {
            for (index, codec) in validate.iter().enumerate() {
                let child = super::parse_context::child(context, IssueSegment::Alternative(index))?;
                codec
                    .validate_in_ref(value, child.as_ref())
                    .map_err(|issue| prefix(context, &validate_issues, issue, index))?;
            }
            Ok(())
        })
    } else {
        result
    };
    Ok(if has_writer {
        result.with_writer(move |value, writer| {
            let mut merge = writer_output.admit_document(policy)?;
            let mut root = None;
            for (index, codec) in write.iter().enumerate() {
                let next = writer_output
                    .encode_with_writer_policy(codec, value, policy, writer.policy())
                    .map_err(|error| match error {
                        super::internal::parser::PreparedWriteStageError::Write(issue) => {
                            writer.issue_at(issue, IssueSegment::Alternative(index))
                        }
                        super::internal::parser::PreparedWriteStageError::Admission(issue)
                        | super::internal::parser::PreparedWriteStageError::Parse(issue) => issue,
                    })?;
                if let Some(root) = root {
                    merge.merge_value(root, next.view(), Some(&writer_issues))?;
                } else {
                    root = Some(merge.merge_root(next.view(), Some(&writer_issues))?);
                }
            }
            let merged = merge.finish_merge_for_writer(root.ok_or_else(invalid)?)?;
            // The ordinary direct writer applies its own output policy to the merged value.
            merged.view().write_to(writer)
        })
    } else {
        result
    })
}
