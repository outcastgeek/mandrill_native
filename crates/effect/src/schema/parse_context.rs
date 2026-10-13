//! Fresh traversal state. Immutable preparation belongs to SchemaAst, while
//! counters and paths belong to one invocation, including suspended invocations.

use super::{
    Codec, IssueSegment, JsonValue, ParsePolicy, SchemaAst, SchemaIssue, SchemaIssueKind,
    SchemaResource,
};
use crate::Effect;
use crate::{effect, effect_try_sync};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
/// Bounds for one configured schema invocation, including its JSON representation.
///
/// Schema depth and JSON depth are independent: wrappers consume schema traversal
/// without necessarily adding JSON containers. All limits are caller supplied;
/// [`Codec::configured`] shares this policy with nested built-in compositions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchemaPolicy {
    /// Schema traversal, collection, byte and issue bounds, plus error and field policies.
    pub parse: ParsePolicy,
    /// Maximum JSON value depth; the root counts as one, independently of schema depth.
    pub json_depth: NonZeroUsize,
    /// Maximum Unicode scalar count in a JSON string or object property name; not UTF-8 bytes.
    pub string_length: NonZeroUsize,
    /// Maximum ASCII digit count in a JSON number token, including fractional and exponent digits.
    pub integer_digits: NonZeroUsize,
    /// Maximum cumulative pattern-search demand across all checked patterns in this invocation.
    pub pattern_steps: NonZeroUsize,
}
/// Issue-tree accounting shared by native field implementations within one invocation.
///
/// Counts both composite nodes and children. Nested compositions reuse the same
/// budget so already charged child issues are not counted twice. This hidden
/// public type is exposed for [`super::NativeFields`], not as a global allocator.
#[doc(hidden)]
#[derive(Clone)]
pub struct IssueBudget {
    maximum: usize,
    spent: Arc<Mutex<usize>>,
}
impl IssueBudget {
    pub(crate) fn new(maximum: NonZeroUsize) -> Self {
        Self {
            maximum: maximum.get(),
            spent: Arc::new(Mutex::new(0)),
        }
    }
    pub(crate) fn used(&self) -> usize {
        *self
            .spent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(crate) fn available(&self) -> Result<(), SchemaIssue> {
        if self.used() >= self.maximum {
            Err(Self::exhausted())
        } else {
            Ok(())
        }
    }
    /// Charge newly produced issue-tree nodes since `before`, excluding child work
    /// already charged to this shared budget. Reject overflow and budget exhaustion.
    pub(crate) fn record(&self, before: usize, issue: &SchemaIssue) -> Result<(), SchemaIssue> {
        let count = issue_count(issue);
        let mut spent = self
            .spent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let new = count.saturating_sub(spent.saturating_sub(before));
        *spent = spent.checked_add(new).ok_or_else(Self::exhausted)?;
        if *spent > self.maximum {
            Err(Self::exhausted())
        } else {
            Ok(())
        }
    }
    pub(crate) fn exhausted() -> SchemaIssue {
        SchemaIssue::new(SchemaIssueKind::LimitExceeded {
            resource: SchemaResource::Issues,
        })
    }
}
pub(crate) fn issue_count(issue: &SchemaIssue) -> usize {
    let mut count = 0usize;
    let mut pending = vec![issue];
    while let Some(issue) = pending.pop() {
        count = count.saturating_add(1);
        pending.extend(issue.children());
    }
    count
}
/// Owned traversal state for one configured decode, encode or validation.
///
/// Clones share node, pattern and issue counters; each child carries its own depth
/// and path. Keep this context with suspended effects and pass it to nested codecs.
/// Create a new root for each independent invocation, rather than reusing spent counters.
#[derive(Clone)]
pub struct ParseContext {
    policy: SchemaPolicy,
    nodes: Arc<Mutex<usize>>,
    pattern_steps: Arc<Mutex<usize>>,
    issues: IssueBudget,
    depth: usize,
    path: Vec<IssueSegment>,
}
impl ParseContext {
    /// Create fresh counters at schema depth one using the supplied policy.
    #[must_use]
    pub fn new(policy: SchemaPolicy) -> Self {
        Self {
            policy,
            nodes: Arc::new(Mutex::new(0)),
            pattern_steps: Arc::new(Mutex::new(0)),
            issues: IssueBudget::new(policy.parse.limits.issues),
            depth: 1,
            path: Vec::new(),
        }
    }
    /// Return the immutable limits and error policies governing this invocation.
    #[must_use]
    pub const fn policy(&self) -> SchemaPolicy {
        self.policy
    }
    /// Charge one schema visit and check the current schema depth.
    ///
    /// Returns [`SchemaIssueKind::ResourceDemand`] if the depth or cumulative node
    /// budget is exceeded. Returned issues are local to the current codec; composites
    /// add their field or index path once.
    pub fn visit(&self) -> Result<(), SchemaIssue> {
        if self.depth > self.policy.parse.limits.depth.get() {
            return Err(self.demand(
                SchemaResource::Depth,
                self.policy.parse.limits.depth.get(),
                self.depth,
            ));
        }
        let mut nodes = self
            .nodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *nodes = nodes.checked_add(1).ok_or_else(|| {
            self.demand(
                SchemaResource::Steps,
                self.policy.parse.limits.nodes.get(),
                usize::MAX,
            )
        })?;
        if *nodes > self.policy.parse.limits.nodes.get() {
            return Err(self.demand(
                SchemaResource::Steps,
                self.policy.parse.limits.nodes.get(),
                *nodes,
            ));
        }
        Ok(())
    }
    /// Create a child at the next schema depth with `segment` appended to its path.
    ///
    /// The child shares invocation counters. Depth arithmetic overflow fails here;
    /// the configured depth limit is checked when the child is visited.
    pub fn child(&self, segment: IssueSegment) -> Result<Self, SchemaIssue> {
        let depth = self.depth.checked_add(1).ok_or_else(|| {
            self.demand(
                SchemaResource::Depth,
                self.policy.parse.limits.depth.get(),
                usize::MAX,
            )
        })?;
        let mut path = self.path.clone();
        path.push(segment);
        Ok(Self {
            policy: self.policy,
            nodes: Arc::clone(&self.nodes),
            pattern_steps: Arc::clone(&self.pattern_steps),
            issues: self.issues.clone(),
            depth,
            path,
        })
    }
    fn demand(&self, resource: SchemaResource, limit: usize, observed: usize) -> SchemaIssue {
        SchemaIssue::new(SchemaIssueKind::ResourceDemand {
            resource,
            limit,
            observed,
        })
    }
    pub(crate) fn issue_budget(&self) -> IssueBudget {
        self.issues.clone()
    }
    pub(crate) fn descend(&self) -> Result<Self, SchemaIssue> {
        let mut child = self.clone();
        child.depth = self.depth.checked_add(1).ok_or_else(|| {
            self.demand(
                SchemaResource::Depth,
                self.policy.parse.limits.depth.get(),
                usize::MAX,
            )
        })?;
        Ok(child)
    }
    /// Charge bounded matcher demand to the shared invocation budget before running the search.
    pub(crate) fn charge_pattern(&self, demand: usize) -> Result<(), SchemaIssue> {
        let mut spent = self
            .pattern_steps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let observed = spent.checked_add(demand).ok_or_else(|| {
            self.demand(
                SchemaResource::PatternSteps,
                self.policy.pattern_steps.get(),
                usize::MAX,
            )
        })?;
        if observed > self.policy.pattern_steps.get() {
            return Err(self.demand(
                SchemaResource::PatternSteps,
                self.policy.pattern_steps.get(),
                observed,
            ));
        }
        *spent = observed;
        Ok(())
    }
    /// Apply configured JSON bounds when `value` is a [`JsonValue`].
    ///
    /// Other representation types are left to their explicit representation adapter;
    /// this method does not provide structural validation for arbitrary Rust values.
    pub fn check_representation<T: 'static>(&self, value: &T) -> Result<(), SchemaIssue> {
        if let Some(value) = (value as &dyn std::any::Any).downcast_ref::<JsonValue>() {
            self.check_json(value)?;
        }
        Ok(())
    }
    /// Check JSON depth, node count, collection width, Unicode length, numeric digits
    /// and encoded byte size without building an intermediate JSON string.
    ///
    /// Traversal failures retain field, index or property-name paths. These are
    /// representation checks; they do not consume the schema-visit counter or replace
    /// the codec's type and constraint validation.
    pub fn check_json(&self, value: &JsonValue) -> Result<(), SchemaIssue> {
        let mut pending = vec![(value, 1usize, Vec::<IssueSegment>::new())];
        let mut nodes = 0usize;
        while let Some((value, depth, path)) = pending.pop() {
            nodes = nodes.checked_add(1).ok_or_else(|| {
                self.demand(
                    SchemaResource::Steps,
                    self.policy.parse.limits.nodes.get(),
                    usize::MAX,
                )
            })?;
            let located = |resource, limit, observed| {
                let mut issue = self.demand(resource, limit, observed);
                for segment in path.iter().rev() {
                    issue = issue.at(segment.clone());
                }
                issue
            };
            if depth > self.policy.json_depth.get() {
                return Err(located(
                    SchemaResource::JsonDepth,
                    self.policy.json_depth.get(),
                    depth,
                ));
            }
            if nodes > self.policy.parse.limits.nodes.get() {
                return Err(located(
                    SchemaResource::Steps,
                    self.policy.parse.limits.nodes.get(),
                    nodes,
                ));
            }
            match value {
                JsonValue::String(text) => {
                    let count = text.chars().count();
                    if count > self.policy.string_length.get() {
                        return Err(located(
                            SchemaResource::StringLength,
                            self.policy.string_length.get(),
                            count,
                        ));
                    }
                }
                JsonValue::Number(number) => {
                    let count = number
                        .to_string()
                        .bytes()
                        .filter(u8::is_ascii_digit)
                        .count();
                    if count > self.policy.integer_digits.get() {
                        return Err(located(
                            SchemaResource::IntegerDigits,
                            self.policy.integer_digits.get(),
                            count,
                        ));
                    }
                }
                JsonValue::Array(items) => {
                    if items.len() > self.policy.parse.limits.collection.get() {
                        return Err(located(
                            SchemaResource::Width,
                            self.policy.parse.limits.collection.get(),
                            items.len(),
                        ));
                    }
                    for (index, value) in items.iter().enumerate().rev() {
                        let mut child = path.clone();
                        child.push(IssueSegment::Index(index));
                        pending.push((value, depth.saturating_add(1), child));
                    }
                }
                JsonValue::Object(fields) => {
                    if fields.len() > self.policy.parse.limits.collection.get() {
                        return Err(located(
                            SchemaResource::Width,
                            self.policy.parse.limits.collection.get(),
                            fields.len(),
                        ));
                    }
                    for (name, value) in fields.iter().rev() {
                        let count = name.chars().count();
                        if count > self.policy.string_length.get() {
                            let mut issue = self
                                .demand(
                                    SchemaResource::StringLength,
                                    self.policy.string_length.get(),
                                    count,
                                )
                                .at(IssueSegment::PropertyName(name.as_str().into()));
                            for segment in path.iter().rev() {
                                issue = issue.at(segment.clone());
                            }
                            return Err(issue);
                        }
                        let mut child = path.clone();
                        child.push(IssueSegment::Field(name.as_str().into()));
                        pending.push((value, depth.saturating_add(1), child));
                    }
                }
                JsonValue::Null | JsonValue::Bool(_) => {}
            }
        }
        // The checked writer computes bytes without allocating an intermediate JSON string.
        let mut writer = super::JsonWriter::new(super::JsonWriterPolicy {
            bytes: self.policy.parse.limits.bytes,
            depth: self.policy.json_depth,
        });
        writer.tree(value)?;
        Ok(())
    }
}
/// A reusable codec description with explicit limits for each directional call.
///
/// Each operation creates fresh [`ParseContext`] state. Decode uses `D`; encode
/// and owned validation use `E`. Cloning or configuring a codec does not select
/// an executor or clone its application payloads.
pub struct ConfiguredCodec<
    A: Send + 'static,
    Encoded: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
> {
    codec: Codec<A, Encoded, D, E>,
    policy: SchemaPolicy,
}
impl<A: Send + 'static, Encoded: Send + 'static, D: Send + Sync + 'static, E: Send + Sync + 'static>
    ConfiguredCodec<A, Encoded, D, E>
{
    /// Borrow the underlying immutable schema description; configuration does not rewrite it.
    #[must_use]
    pub fn ast(&self) -> &SchemaAst {
        self.codec.ast()
    }
    /// Create a lazy decode in `D` with fresh counters, consuming `input` on its first execution.
    pub fn decode_once(&self, input: Encoded) -> Effect<A, SchemaIssue, D> {
        self.codec
            .decode_in(input, Some(ParseContext::new(self.policy)))
    }
    /// Create a lazy encode in `E` with fresh counters, consuming the typed input on its first execution.
    pub fn encode_once(&self, input: A) -> Effect<Encoded, SchemaIssue, E> {
        self.codec
            .encode_in(input, Some(ParseContext::new(self.policy)))
    }
    /// Create lazy owned type validation in `E` with fresh counters.
    ///
    /// Returns the validated typed value. A codec without a usable validation path
    /// returns its schema error; validation is not automatically a direct-writer witness.
    pub fn validate_once(&self, input: A) -> Effect<A, SchemaIssue, E> {
        self.codec
            .validate_in(input, Some(ParseContext::new(self.policy)))
    }
}
impl<A: Send + 'static, D: Send + Sync + 'static, E: Send + Sync + 'static>
    ConfiguredCodec<A, JsonValue, D, E>
{
    /// Lazily parse JSON text and decode it in `D` with fresh state on every execution.
    ///
    /// Enforces `json_depth` while parsing, then configured representation bounds and
    /// schema constraints. Shared text can be reused; decoded values are created anew.
    pub fn parse(&self, text: impl Into<Arc<str>>) -> Effect<A, SchemaIssue, D> {
        let text = text.into();
        let policy = self.policy;
        let codec = self.codec.clone();
        (effect_try_sync! { body: move || {
            let context = ParseContext::new(policy);
            let mut parse = policy.parse;
            parse.limits.depth = policy.json_depth;
            let value = super::internal::parser::parse_json(&text, parse).map_err(|issue| {
                if matches!(
                    issue.kind(),
                    SchemaIssueKind::LimitExceeded {
                        resource: SchemaResource::Depth
                    }
                ) {
                    let mut located = SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                        resource: SchemaResource::JsonDepth,
                    });
                    for segment in issue.path().iter().rev() {
                        located = located.at(segment.clone());
                    }
                    located
                } else {
                    issue
                }
            })?;
            context.check_json(&value)?;
            Ok((value, context))
        }, on_error: std::convert::identity, })
        .flat_map(move |(value, context)| codec.decode_in(value, Some(context)))
    }
    /// Lazily check an owned JSON tree against configured representation bounds,
    /// then decode it in `D`. The returned effect consumes the input once.
    pub fn decode_json_once(&self, input: JsonValue) -> Effect<A, SchemaIssue, D> {
        let context = ParseContext::new(self.policy);
        let codec = self.codec.clone();
        {
            let __dsl_program = effect_try_sync! { once; body: move || {
                context.check_json(&input)?;
                Ok((input, context))
            }, on_error: std::convert::identity, };
            effect! { let codec = codec.clone(); let (input , context) = __dsl_program.clone()?; codec.decode_in(input, Some(context))? }
        }
    }
}
impl<A: Send + 'static, Encoded: Send + 'static, D: Send + Sync + 'static, E: Send + Sync + 'static>
    Codec<A, Encoded, D, E>
{
    /// Prepare this codec's AST under `policy.parse.limits` and return a configured description.
    ///
    /// Preparation is eager and may reject invalid or over-budget metadata. Directional
    /// callbacks remain lazy and receive fresh traversal state for each operation.
    pub fn configured(
        &self,
        policy: SchemaPolicy,
    ) -> Result<ConfiguredCodec<A, Encoded, D, E>, SchemaIssue> {
        self.ast().prepare(policy.parse.limits)?;
        Ok(ConfiguredCodec {
            codec: self.clone(),
            policy,
        })
    }
}

pub(crate) fn child(
    context: Option<&ParseContext>,
    segment: IssueSegment,
) -> Result<Option<ParseContext>, SchemaIssue> {
    context.map(|context| context.child(segment)).transpose()
}
pub(crate) fn descend(context: Option<&ParseContext>) -> Result<Option<ParseContext>, SchemaIssue> {
    context.map(ParseContext::descend).transpose()
}
pub(crate) fn policy(context: Option<&ParseContext>, fallback: ParsePolicy) -> ParsePolicy {
    context.map_or(fallback, |context| {
        let mut policy = context.policy().parse;
        policy.limits.depth = context.policy().json_depth;
        policy
    })
}
pub(crate) fn budget(context: Option<&ParseContext>, policy: ParsePolicy) -> IssueBudget {
    context.map_or_else(
        || IssueBudget::new(policy.limits.issues),
        ParseContext::issue_budget,
    )
}
