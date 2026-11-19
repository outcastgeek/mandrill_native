//! Private binary tags, widths, and parser mechanics.

use super::{BinaryError, BinaryLimits};
use crate::schema::{
    IssueSegment, JsonValue, PrimitiveShape, SchemaAst, SchemaIssue, SchemaIssueKind,
    SchemaResource, SchemaShape, StringLengthUnit, UnknownFields, ValueKind, value_kind,
};
use std::sync::Arc;

#[derive(Clone, Copy)]
enum View<'a> {
    Native(&'a JsonValue),
    Managed(crate::schema::PreparedJsonValue<'a>),
}
enum ViewKind<'a> {
    Null,
    Bool(bool),
    Number(&'a str),
    String(&'a str),
    Array(usize),
    Object(usize),
}
enum Children<'a> {
    Array(std::slice::Iter<'a, JsonValue>),
    Object(serde_json::map::Iter<'a>),
    Managed(Option<crate::schema::PreparedJsonValue<'a>>),
    Empty,
}
/// Caller-selected binary traversal concurrency and physical frame count.
#[derive(Clone, Copy, Debug)]
pub struct PreparedBinaryTraversalCapacity {
    /// Concurrent admitted traversal stacks.
    pub traversals: usize,
    /// Actual control frames in each stack.
    pub frames_per_traversal: usize,
}
/// Startup-backed call-scoped binary traversal frame cells.
pub struct PreparedBinaryTraversalPool {
    cells: crate::memory::SlotPool<
        'static,
        crate::memory::FixedVector<'static, std::mem::MaybeUninit<ControlFrame<'static>>>,
    >,
}
enum ControlFrame<'a> {
    DecodeShape {
        shape: Arc<SchemaShape>,
        depth: usize,
        charge: bool,
    },
    DecodeAny {
        depth: usize,
        charge: bool,
    },
    DecodeSequence {
        shape: Arc<SchemaShape>,
        parent: DecodeNode,
        index: usize,
        count: usize,
        depth: usize,
        tuple: bool,
    },
    DecodeObject {
        shape: Arc<SchemaShape>,
        parent: DecodeNode,
        index: usize,
        depth: usize,
    },
    DecodeAnyCollection {
        parent: DecodeNode,
        index: usize,
        count: usize,
        depth: usize,
        object: bool,
    },
    DecodeAppend {
        parent: DecodeNode,
        name: Option<DecodeName<'a>>,
        native_name: Option<String>,
        duplicate: bool,
        resume: DecodeResume,
    },
    DecodeCheck {
        shape: Arc<SchemaShape>,
    },
    DecodeLiteral {
        shape: Arc<SchemaShape>,
    },
    DecodePath(OwnedPath),
    EncodeShape {
        shape: Arc<SchemaShape>,
        value: View<'a>,
        depth: usize,
        charge: bool,
    },
    EncodeSequence {
        shape: Arc<SchemaShape>,
        values: Children<'a>,
        index: usize,
        depth: usize,
        tuple: bool,
    },
    EncodeObject {
        shape: Arc<SchemaShape>,
        value: View<'a>,
        index: usize,
        depth: usize,
    },
    EncodeAny {
        value: View<'a>,
        depth: usize,
        charge: bool,
    },
    EncodeAnyChildren {
        values: Children<'a>,
        depth: usize,
        object: bool,
    },
    EncodePath(OwnedPath),
    EncodeUnionTry {
        shape: Arc<SchemaShape>,
        value: View<'a>,
        depth: usize,
        next: usize,
        failures: UnionFailures,
    },
    EncodeUnionWait {
        shape: Arc<SchemaShape>,
        value: View<'a>,
        depth: usize,
        next: usize,
        failures: UnionFailures,
        start: usize,
    },
    Compare {
        value: View<'a>,
        literal: &'a JsonValue,
    },
    CompareArray {
        values: Children<'a>,
        literals: std::slice::Iter<'a, JsonValue>,
    },
    CompareObject {
        value: View<'a>,
        literals: serde_json::map::Iter<'a>,
    },
    CompareNativeObject {
        values: serde_json::map::Iter<'a>,
        literals: serde_json::map::Iter<'a>,
    },
}
enum OwnedPath {
    Index(usize),
    Field(Arc<str>),
}
enum DecodeResume {
    Sequence {
        shape: Arc<SchemaShape>,
        index: usize,
        count: usize,
        depth: usize,
        tuple: bool,
    },
    Object {
        shape: Arc<SchemaShape>,
        index: usize,
        depth: usize,
    },
    Any {
        index: usize,
        count: usize,
        depth: usize,
        object: bool,
    },
}
enum DecodeName<'a> {
    Input(&'a str),
    Schema(Arc<str>),
}
enum UnionFailures {
    Native(Vec<BinaryError>),
    Prepared(crate::schema::PreparedJsonArrayBuilder<BinaryError>),
    Bank(super::bank::BankBuilder),
}
type Cell = crate::memory::FixedVector<'static, std::mem::MaybeUninit<ControlFrame<'static>>>;
impl PreparedBinaryTraversalCapacity {
    /// Actual fixed region frame cells and pool metadata, excluding allocator overhead.
    pub fn storage_bytes(self) -> Result<crate::memory::Bytes, crate::memory::MemoryError> {
        let layout = std::alloc::Layout::array::<std::mem::MaybeUninit<ControlFrame<'static>>>(
            self.frames_per_traversal,
        )
        .map_err(|_| crate::memory::MemoryError::InvalidLayout)?;
        crate::memory::SlotPool::<Cell>::storage_bytes(self.traversals)?
            .checked_add(crate::memory::Bytes::new(layout.size()).checked_mul(self.traversals)?)
    }
}
impl PreparedBinaryTraversalPool {
    /// Provision actual uninitialized cells for each caller-selected traversal.
    pub fn prepare(
        memory: &crate::memory::PreparedMemory<'static>,
        region: crate::memory::RegionId,
        capacity: PreparedBinaryTraversalCapacity,
    ) -> Result<Self, crate::memory::MemoryError> {
        let _ = capacity.storage_bytes()?;
        let cells = memory.slots(region, capacity.traversals, |_| {
            let mut cells = crate::memory::FixedVector::prepare(
                memory.allocator(region)?,
                capacity.frames_per_traversal,
            )?;
            for _ in 0..capacity.frames_per_traversal {
                cells
                    .push(std::mem::MaybeUninit::uninit())
                    .map_err(|_| crate::memory::MemoryError::NoSlot)?;
            }
            Ok(cells)
        })?;
        Ok(Self { cells })
    }
    pub(super) fn reserve(&self) -> Result<ReservedTraversal, crate::memory::MemoryError> {
        Ok(ReservedTraversal(self.cells.checkout()?))
    }
    fn checkout<'input>(&self) -> Result<FrameStack<'input>, crate::memory::MemoryError> {
        Ok(self.reserve()?.into_stack())
    }
}
pub(super) struct ReservedTraversal(crate::memory::SlotLease<'static, Cell>);
impl ReservedTraversal {
    fn into_stack<'input>(self) -> FrameStack<'input> {
        FrameStack {
            backing: StackBacking::Prepared(self.0),
            initialized: 0,
            scope: std::marker::PhantomData,
        }
    }
}
enum StackBacking {
    Native(Vec<std::mem::MaybeUninit<ControlFrame<'static>>>),
    Prepared(crate::memory::SlotLease<'static, Cell>),
}
struct FrameStack<'input> {
    backing: StackBacking,
    initialized: usize,
    scope: std::marker::PhantomData<&'input ()>,
}
impl<'input> FrameStack<'input> {
    fn native() -> Self {
        Self {
            backing: StackBacking::Native(Vec::new()),
            initialized: 0,
            scope: std::marker::PhantomData,
        }
    }
    fn cell_for_push(
        &mut self,
    ) -> Result<&mut std::mem::MaybeUninit<ControlFrame<'static>>, BinaryError> {
        match &mut self.backing {
            StackBacking::Native(cells) => {
                if cells.len() == self.initialized {
                    cells.push(std::mem::MaybeUninit::uninit());
                }
                cells
                    .get_mut(self.initialized)
                    .ok_or(BinaryError::Memory(crate::memory::MemoryError::NoSlot))
            }
            StackBacking::Prepared(cells) => cells
                .as_mut_slice()
                .get_mut(self.initialized)
                .ok_or(BinaryError::Memory(crate::memory::MemoryError::NoSlot)),
        }
    }
    // SAFETY: callers must clear all frames of `frame`'s lifetime before it ends;
    // only an uninitialized cell is projected, with identical lifetime-only layout.
    unsafe fn push_scoped<'frame>(
        &mut self,
        frame: ControlFrame<'frame>,
    ) -> Result<(), BinaryError> {
        let cell = self.cell_for_push()?;
        let destination = (cell as *mut std::mem::MaybeUninit<ControlFrame<'static>>)
            .cast::<std::mem::MaybeUninit<ControlFrame<'frame>>>();
        unsafe {
            (*destination).write(frame);
        }
        self.initialized += 1;
        Ok(())
    }
    // SAFETY: the top frame was written for `frame`, and is removed before its
    // scope ends. The pre-decrement prevents duplicate drops even on unwind.
    unsafe fn pop_scoped<'frame>(&mut self) -> Option<ControlFrame<'frame>> {
        self.initialized = self.initialized.checked_sub(1)?;
        let cell = match &mut self.backing {
            StackBacking::Native(cells) => cells.get_mut(self.initialized)?,
            StackBacking::Prepared(cells) => cells.as_mut_slice().get_mut(self.initialized)?,
        };
        let source = (cell as *mut std::mem::MaybeUninit<ControlFrame<'static>>)
            .cast::<std::mem::MaybeUninit<ControlFrame<'frame>>>();
        Some(unsafe { (*source).assume_init_read() })
    }
    // SAFETY: all frames above base belong to the same live scope. A guard
    // resumes retirement if one destructor unwinds, before this lease returns.
    unsafe fn drain_scoped<'frame>(
        &mut self,
        base: usize,
        scope: std::marker::PhantomData<&'frame ()>,
    ) {
        while self.initialized > base {
            let Some(frame): Option<ControlFrame<'frame>> = (unsafe { self.pop_scoped() }) else {
                break;
            };
            let guard =
                std::mem::DropGuard::new((&mut *self, base, scope), |(stack, base, scope)| {
                    // SAFETY: unwinding remains within the same frame lifetime.
                    unsafe { stack.drain_scoped(base, scope) };
                });
            drop(frame);
            let _ = std::mem::DropGuard::dismiss(guard);
        }
    }
    fn push(&mut self, frame: ControlFrame<'input>) -> Result<(), BinaryError> {
        // SAFETY: FrameStack drops every outer frame before its input scope ends.
        unsafe { self.push_scoped(frame) }
    }
    fn pop(&mut self) -> Option<ControlFrame<'input>> {
        // SAFETY: only outer frames are accessed without a live exclusive suffix.
        unsafe { self.pop_scoped() }
    }
    fn suffix<'short>(&'short mut self) -> StackSuffix<'short, 'input> {
        let base = self.initialized;
        StackSuffix { stack: self, base }
    }
}
impl<'input> Drop for FrameStack<'input> {
    fn drop(&mut self) {
        // SAFETY: no suffix can remain borrowed when the outer stack drops.
        unsafe { self.drain_scoped(0, std::marker::PhantomData::<&'input ()>) };
    }
}
struct StackSuffix<'short, 'input> {
    stack: &'short mut FrameStack<'input>,
    base: usize,
}
impl<'short> StackSuffix<'short, '_> {
    fn len(&self) -> usize {
        self.stack.initialized - self.base
    }
    fn push(&mut self, frame: ControlFrame<'short>) -> Result<(), BinaryError> {
        // SAFETY: the suffix exclusively borrows its parent and clears its frames on Drop.
        unsafe { self.stack.push_scoped(frame) }
    }
    fn pop(&mut self) -> Option<ControlFrame<'short>> {
        if self.stack.initialized == self.base {
            return None;
        }
        // SAFETY: this suffix owns every frame above its saved base.
        unsafe { self.stack.pop_scoped() }
    }
}
impl<'short, 'input> Drop for StackSuffix<'short, 'input> {
    fn drop(&mut self) {
        // SAFETY: every cell above base was initialized for this suffix.
        unsafe {
            self.stack
                .drain_scoped(self.base, std::marker::PhantomData::<&'short ()>)
        };
    }
}

impl<'a> Iterator for Children<'a> {
    type Item = (Option<&'a str>, View<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Array(items) => items.next().map(|value| (None, View::Native(value))),
            Self::Object(items) => items
                .next()
                .map(|(name, value)| (Some(name.as_str()), View::Native(value))),
            Self::Managed(current) => {
                let value = current.take()?;
                *current = value.next_sibling();
                Some((value.property_name(), View::Managed(value)))
            }
            Self::Empty => None,
        }
    }
}
impl<'a> View<'a> {
    fn children(self) -> Children<'a> {
        match self {
            Self::Native(JsonValue::Array(items)) => Children::Array(items.iter()),
            Self::Native(JsonValue::Object(items)) => Children::Object(items.iter()),
            Self::Managed(value) => Children::Managed(value.first_child()),
            Self::Native(_) => Children::Empty,
        }
    }
    fn kind(self) -> Result<ViewKind<'a>, BinaryError> {
        use crate::schema::PreparedJsonKind as K;
        Ok(match self {
            Self::Native(JsonValue::Null) => ViewKind::Null,
            Self::Native(JsonValue::Bool(value)) => ViewKind::Bool(*value),
            Self::Native(JsonValue::Number(value)) => ViewKind::Number(value.as_str()),
            Self::Native(JsonValue::String(value)) => ViewKind::String(value),
            Self::Native(JsonValue::Array(value)) => ViewKind::Array(value.len()),
            Self::Native(JsonValue::Object(value)) => ViewKind::Object(value.len()),
            Self::Managed(value) => match value
                .kind()
                .ok_or_else(|| issue(SchemaIssueKind::InvalidDefinition))?
            {
                K::Null => ViewKind::Null,
                K::Bool(v) => ViewKind::Bool(v),
                K::Number => ViewKind::Number(
                    value
                        .number_token()
                        .ok_or_else(|| issue(SchemaIssueKind::InvalidDefinition))?,
                ),
                K::String => ViewKind::String(
                    value
                        .as_str()
                        .ok_or_else(|| issue(SchemaIssueKind::InvalidDefinition))?,
                ),
                K::Array => ViewKind::Array(value.len()),
                K::Object => ViewKind::Object(value.len()),
            },
        })
    }
    fn get(self, name: &str) -> Option<Self> {
        match self {
            Self::Native(JsonValue::Object(v)) => v.get(name).map(Self::Native),
            Self::Managed(v) => v.get(name).map(Self::Managed),
            Self::Native(_) => None,
        }
    }
    fn as_i64(self) -> Option<i64> {
        match self {
            Self::Native(value) => value.as_i64(),
            Self::Managed(value) => {
                crate::schema::managed_number::ManagedNumber::parse(value.number_token()?)?.as_i64()
            }
        }
    }
    fn as_u64(self) -> Option<u64> {
        match self {
            Self::Native(value) => value.as_u64(),
            Self::Managed(value) => {
                crate::schema::managed_number::ManagedNumber::parse(value.number_token()?)?.as_u64()
            }
        }
    }
    fn as_str(self) -> Option<&'a str> {
        if let ViewKind::String(value) = self.kind().ok()? {
            Some(value)
        } else {
            None
        }
    }
    fn actual(self) -> Result<ValueKind, BinaryError> {
        match self {
            Self::Native(value) => Ok(value_kind(value)),
            Self::Managed(value) => {
                crate::schema::prepared_view_kind(value).map_err(BinaryError::Schema)
            }
        }
    }
}
fn view_mismatch(expected: ValueKind, actual: View<'_>) -> BinaryError {
    match actual.actual() {
        Ok(actual) => issue(SchemaIssueKind::TypeMismatch { expected, actual }),
        Err(error) => error,
    }
}
fn equals_literal_scoped<'a>(
    value: View<'a>,
    literal: &'a JsonValue,
    stack: &mut StackSuffix<'a, '_>,
) -> Result<bool, BinaryError> {
    let initial = stack.len();
    stack.push(ControlFrame::Compare { value, literal })?;
    while stack.len() > initial {
        let Some(frame) = stack.pop() else {
            return Err(issue(SchemaIssueKind::InvalidDefinition));
        };
        match frame {
            ControlFrame::Compare { value, literal } => match (value.kind()?, literal) {
                (ViewKind::Null, JsonValue::Null) => {}
                (ViewKind::Bool(a), JsonValue::Bool(b)) if a == *b => {}
                (ViewKind::String(a), JsonValue::String(b)) if a == b => {}
                (ViewKind::Number(a), JsonValue::Number(b)) => {
                    let equal = match value {
                        View::Native(_) => a == b.as_str(),
                        View::Managed(_) => normalized_equals(a, b.as_str())?,
                    };
                    if !equal {
                        while stack.len() > initial {
                            drop(stack.pop());
                        }
                        return Ok(false);
                    }
                }
                (ViewKind::Array(count), JsonValue::Array(items)) if count == items.len() => {
                    stack.push(ControlFrame::CompareArray {
                        values: value.children(),
                        literals: items.iter(),
                    })?;
                }
                (ViewKind::Object(count), JsonValue::Object(items)) if count == items.len() => {
                    match value {
                        View::Native(JsonValue::Object(values)) => {
                            stack.push(ControlFrame::CompareNativeObject {
                                values: values.iter(),
                                literals: items.iter(),
                            })?
                        }
                        View::Managed(_) => stack.push(ControlFrame::CompareObject {
                            value,
                            literals: items.iter(),
                        })?,
                        View::Native(_) => return Err(issue(SchemaIssueKind::InvalidDefinition)),
                    }
                }
                _ => {
                    while stack.len() > initial {
                        drop(stack.pop());
                    }
                    return Ok(false);
                }
            },
            ControlFrame::CompareArray {
                mut values,
                mut literals,
            } => match (values.next(), literals.next()) {
                (None, None) => {}
                (Some((_, value)), Some(literal)) => {
                    stack.push(ControlFrame::CompareArray { values, literals })?;
                    stack.push(ControlFrame::Compare { value, literal })?;
                }
                _ => return Err(issue(SchemaIssueKind::InvalidDefinition)),
            },
            ControlFrame::CompareObject {
                value,
                mut literals,
            } => {
                if let Some((name, literal)) = literals.next() {
                    let Some(child) = value.get(name) else {
                        while stack.len() > initial {
                            drop(stack.pop());
                        }
                        return Ok(false);
                    };
                    stack.push(ControlFrame::CompareObject { value, literals })?;
                    stack.push(ControlFrame::Compare {
                        value: child,
                        literal,
                    })?;
                }
            }
            ControlFrame::CompareNativeObject {
                mut values,
                mut literals,
            } => match (values.next(), literals.next()) {
                (None, None) => {}
                (Some((left_name, value)), Some((right_name, literal)))
                    if left_name == right_name =>
                {
                    stack.push(ControlFrame::CompareNativeObject { values, literals })?;
                    stack.push(ControlFrame::Compare {
                        value: View::Native(value),
                        literal,
                    })?;
                }
                (Some(_), Some(_)) => {
                    while stack.len() > initial {
                        drop(stack.pop());
                    }
                    return Ok(false);
                }
                _ => return Err(issue(SchemaIssueKind::InvalidDefinition)),
            },
            ControlFrame::DecodeShape { .. }
            | ControlFrame::DecodeAny { .. }
            | ControlFrame::DecodeSequence { .. }
            | ControlFrame::DecodeObject { .. }
            | ControlFrame::DecodeAnyCollection { .. }
            | ControlFrame::DecodeAppend { .. }
            | ControlFrame::DecodeCheck { .. }
            | ControlFrame::DecodeLiteral { .. }
            | ControlFrame::DecodePath(_)
            | ControlFrame::EncodeShape { .. }
            | ControlFrame::EncodeSequence { .. }
            | ControlFrame::EncodeObject { .. }
            | ControlFrame::EncodeAny { .. }
            | ControlFrame::EncodeAnyChildren { .. }
            | ControlFrame::EncodePath(_)
            | ControlFrame::EncodeUnionTry { .. }
            | ControlFrame::EncodeUnionWait { .. } => {
                return Err(issue(SchemaIssueKind::InvalidDefinition));
            }
        }
    }
    Ok(true)
}

fn normalized_equals(token: &str, expected: &str) -> Result<bool, BinaryError> {
    let number = crate::schema::managed_number::ManagedNumber::parse(token)
        .ok_or(BinaryError::InvalidNumber)?;
    let (a, b, c, d) = number.pieces();
    Ok(a.bytes()
        .chain(b.bytes())
        .chain(c.bytes())
        .chain(d.bytes())
        .eq(expected.bytes()))
}

// Format-1 discriminants. They are wire definitions, not tunable policy.
const ABSENT: u8 = 0;
const PRESENT: u8 = 1;
const FALSE: u8 = 0;
const TRUE: u8 = 1;
const SIGNED: u8 = 0;
const UNSIGNED: u8 = 1;
const FLOAT: u8 = 2;
const ANY_NULL: u8 = 0;
const ANY_FALSE: u8 = 1;
const ANY_TRUE: u8 = 2;
const ANY_SIGNED: u8 = 3;
const ANY_UNSIGNED: u8 = 4;
const ANY_FLOAT: u8 = 5;
const ANY_STRING: u8 = 6;
const ANY_ARRAY: u8 = 7;
const ANY_OBJECT: u8 = 8;

///
/// # Parameters
/// - `kind` (`SchemaIssueKind`): The kind supplied to this operation.
///
/// # Returns
/// Type: `BinaryError`.
/// Returns `BinaryError`.
fn issue(kind: SchemaIssueKind) -> BinaryError {
    BinaryError::Schema(SchemaIssue::new(kind))
}

///
/// # Parameters
/// - `error` (`BinaryError`): Expected or provider error preserved in the returned outcome.
/// - `segment` (`IssueSegment`): The segment supplied to this operation.
///
/// # Returns
/// Type: `BinaryError`.
/// Returns `BinaryError`.
#[derive(Clone, Copy)]
enum Diagnostics<'a> {
    Native,
    Prepared(Option<&'a crate::schema::PreparedIssueMemory>),
}
enum BinaryPath<'a> {
    Index(usize),
    Field(&'a str),
}
fn at(error: BinaryError, path: BinaryPath<'_>, diagnostics: Diagnostics<'_>) -> BinaryError {
    let BinaryError::Schema(issue) = error else {
        return error;
    };
    let segment = match (diagnostics, path) {
        (_, BinaryPath::Index(index)) => IssueSegment::Index(index),
        (Diagnostics::Native, BinaryPath::Field(name)) => IssueSegment::Field(Arc::from(name)),
        (Diagnostics::Prepared(memory), BinaryPath::Field(name)) => {
            match crate::schema::format_diagnostic_text(memory, format_args!("{name}")) {
                Ok(name) => IssueSegment::PreparedField(name),
                Err(error) => {
                    return if crate::schema::physical_cause(&issue).is_some() {
                        BinaryError::Schema(issue)
                    } else {
                        backing_error(error)
                    };
                }
            }
        }
    };
    match diagnostics {
        Diagnostics::Native => BinaryError::Schema(issue.at(segment)),
        Diagnostics::Prepared(memory) => {
            backing_error(crate::schema::selected_issue_at(memory, issue, segment))
        }
    }
}
fn unknown_field(name: &str, diagnostics: Diagnostics<'_>) -> BinaryError {
    match diagnostics {
        Diagnostics::Native => issue(SchemaIssueKind::UnknownField(Arc::from(name))),
        Diagnostics::Prepared(memory) => {
            backing_error(crate::schema::prepared_unknown_field(None, memory, name))
        }
    }
}

///
/// # Parameters
/// - `depth` (`usize`): Nesting depth used to enforce a parser or schema bound.
///
/// # Returns
/// Type: `Result<usize, BinaryError>`.
fn child_depth(depth: usize) -> Result<usize, BinaryError> {
    depth
        .checked_add(1)
        .ok_or(BinaryError::LimitExceeded(SchemaResource::Depth))
}

///
/// # Parameters
/// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
/// - `expected` (`ValueKind`): Value or revision required for the proposed state transition.
///
/// # Returns
/// Type: `Result<(u8, [u8; 8]), BinaryError>`.
fn number_parts(value: View<'_>, expected: ValueKind) -> Result<(u8, [u8; 8]), BinaryError> {
    let ViewKind::Number(token) = value.kind()? else {
        return Err(view_mismatch(expected, value));
    };
    if let Some(signed) = value.as_i64() {
        Ok((SIGNED, signed.to_be_bytes()))
    } else if let Some(unsigned) = value.as_u64() {
        Ok((UNSIGNED, unsigned.to_be_bytes()))
    } else if let Some(float) = match value {
        View::Native(v) => v.as_f64(),
        View::Managed(_) => {
            crate::schema::managed_number::ManagedNumber::parse(token).and_then(|n| n.as_f64())
        }
    }
    .filter(|value| value.is_finite())
    {
        // Pinned zmij 1.0.23 bounds finite-f64 formatting to 24 bytes.
        let mut storage = [0u8; 24];
        let used = {
            let mut remaining = storage.as_mut_slice();
            serde_json::to_writer(&mut remaining, &float)
                .map_err(|_| BinaryError::InvalidNumber)?;
            24 - remaining.len()
        };
        let expected = std::str::from_utf8(storage.get(..used).ok_or(BinaryError::InvalidNumber)?)
            .map_err(|_| BinaryError::InvalidNumber)?;
        let equal = match value {
            View::Native(_) => token == expected,
            View::Managed(_) => normalized_equals(token, expected)?,
        };
        if !equal {
            return Err(BinaryError::InvalidNumber);
        }
        Ok((FLOAT, float.to_be_bytes()))
    } else {
        Err(issue(SchemaIssueKind::NonFinite))
    }
}

#[derive(Clone, Copy)]
/// Explicit resource bounds for encoding schema binary internal.
struct Budget {
    limits: BinaryLimits,
    nodes: usize,
    steps: usize,
}

impl Budget {
    ///
    /// # Parameters
    /// - `depth` (`usize`): Nesting depth used to enforce a parser or schema bound.
    ///
    /// # Returns
    /// Type: `Result<(), BinaryError>`.
    fn value(&mut self, depth: usize) -> Result<(), BinaryError> {
        if depth > self.limits.depth.get() {
            return Err(BinaryError::LimitExceeded(SchemaResource::Depth));
        }
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or(BinaryError::LengthOverflow)?;
        if self.nodes > self.limits.nodes.get() {
            return Err(BinaryError::LimitExceeded(SchemaResource::Steps));
        }
        Ok(())
    }

    ///
    /// # Returns
    /// Type: `Result<(), BinaryError>`.
    fn step(&mut self) -> Result<(), BinaryError> {
        self.steps = self
            .steps
            .checked_add(1)
            .ok_or(BinaryError::LengthOverflow)?;
        if self.steps > self.limits.steps.get() {
            return Err(BinaryError::LimitExceeded(SchemaResource::Steps));
        }
        Ok(())
    }

    ///
    /// # Parameters
    /// - `count` (`usize`): Caller-selected number of iterations, values, or permits.
    ///
    /// # Returns
    /// Type: `Result<(), BinaryError>`.
    fn collection(&self, count: usize) -> Result<(), BinaryError> {
        if count > self.limits.collection.get() {
            Err(BinaryError::LimitExceeded(SchemaResource::Width))
        } else {
            Ok(())
        }
    }
}

/// Typed output used by encoding schema binary internal.
enum Destination<'a, 'b> {
    Native(&'a mut Vec<u8>),
    Prepared(&'a mut crate::memory::Buffer<'b>),
}
pub(super) struct Output<'a, 'b> {
    bytes: Destination<'a, 'b>,
    budget: Budget,
    initial: usize,
    errors: Option<&'a crate::schema::PreparedJsonArrayPool<BinaryError>>,
    bank: Option<&'a super::bank::Bank>,
    diagnostics: Diagnostics<'a>,
    traversal: Option<&'a PreparedBinaryTraversalPool>,
    committed: bool,
}

impl<'a, 'b> Output<'a, 'b> {
    /// # Parameters
    /// - `limits` (`BinaryLimits`): Explicit resource bounds enforced while processing this input.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(super) fn new(limits: BinaryLimits, bytes: &'a mut Vec<u8>) -> Self {
        Self {
            bytes: Destination::Native(bytes),
            budget: Budget {
                limits,
                nodes: 0,
                steps: 0,
            },
            initial: 0,
            errors: None,
            bank: None,
            diagnostics: Diagnostics::Native,
            traversal: None,
            committed: false,
        }
    }
    pub(super) fn prepared(
        limits: BinaryLimits,
        bytes: &'a mut crate::memory::Buffer<'b>,
        errors: &'a crate::schema::PreparedJsonArrayPool<BinaryError>,
        issues: Option<&'a crate::schema::PreparedIssueMemory>,
        traversal: Option<&'a PreparedBinaryTraversalPool>,
    ) -> Self {
        let initial = bytes.len();
        Self {
            bytes: Destination::Prepared(bytes),
            budget: Budget {
                limits,
                nodes: 0,
                steps: 0,
            },
            initial,
            errors: Some(errors),
            bank: None,
            diagnostics: Diagnostics::Prepared(issues),
            traversal,
            committed: false,
        }
    }
    pub(super) fn with_bank(mut self, bank: &'a super::bank::Bank) -> Self {
        self.bank = Some(bank);
        self
    }
    fn len(&self) -> usize {
        match &self.bytes {
            Destination::Native(bytes) => bytes.len(),
            Destination::Prepared(bytes) => bytes.len(),
        }
    }
    fn truncate(&mut self, len: usize) {
        match &mut self.bytes {
            Destination::Native(bytes) => bytes.truncate(len),
            Destination::Prepared(bytes) => bytes.truncate(len),
        }
    }
    pub(super) fn commit(&mut self) {
        self.committed = true;
    }

    ///
    /// # Parameters
    /// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
    ///
    /// # Returns
    /// Type: `Result<(), BinaryError>`.
    fn raw(&mut self, bytes: &[u8]) -> Result<(), BinaryError> {
        let next = self
            .len()
            .saturating_sub(self.initial)
            .checked_add(bytes.len())
            .ok_or(BinaryError::LengthOverflow)?;
        if next > self.budget.limits.bytes.get() {
            return Err(BinaryError::LimitExceeded(SchemaResource::Bytes));
        }
        match &mut self.bytes {
            Destination::Native(output) => output.extend_from_slice(bytes),
            Destination::Prepared(output) => output
                .extend_from_slice(bytes)
                .map_err(BinaryError::Memory)?,
        }
        Ok(())
    }

    /// # Parameters
    /// - `byte` (`u8`): The byte supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), BinaryError>`.
    pub(super) fn byte(&mut self, byte: u8) -> Result<(), BinaryError> {
        self.raw(&[byte])
    }

    /// # Parameters
    /// - `value` (`u64`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<(), BinaryError>`.
    pub(super) fn u64(&mut self, value: u64) -> Result<(), BinaryError> {
        self.raw(&value.to_be_bytes())
    }

    ///
    /// # Parameters
    /// - `length` (`usize`): The length supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), BinaryError>`.
    fn length(&mut self, length: usize) -> Result<(), BinaryError> {
        self.u64(u64::try_from(length).map_err(|_| BinaryError::LengthOverflow)?)
    }

    /// # Parameters
    /// - `text` (`&str`): Input text parsed or emitted by this operation.
    ///
    /// # Returns
    /// Type: `Result<(), BinaryError>`.
    pub(super) fn text(&mut self, text: &str) -> Result<(), BinaryError> {
        self.length(text.len())?;
        self.raw(text.as_bytes())
    }

    /// # Parameters
    /// - `ast` (`&SchemaAst`): Representation tree used for schema inspection or export.
    /// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
    /// - `depth` (`usize`): Nesting depth used to enforce a parser or schema bound.
    ///
    /// # Returns
    /// Type: `Result<(), BinaryError>`.
    pub(super) fn value(
        &mut self,
        ast: &SchemaAst,
        value: &JsonValue,
        depth: usize,
    ) -> Result<(), BinaryError> {
        self.value_view(ast, View::Native(value), depth)
    }
    pub(super) fn prepared_value(
        &mut self,
        ast: &SchemaAst,
        value: crate::schema::PreparedJsonValue<'_>,
        depth: usize,
    ) -> Result<(), BinaryError> {
        self.value_view(ast, View::Managed(value), depth)
    }
    pub(super) fn prepared_value_reserved(
        &mut self,
        ast: &SchemaAst,
        value: crate::schema::PreparedJsonValue<'_>,
        depth: usize,
        reserved: ReservedTraversal,
    ) -> Result<(), BinaryError> {
        let mut stack = reserved.into_stack();
        self.value_view_with_stack(ast, View::Managed(value), depth, &mut stack)
    }
    fn value_view<'input>(
        &mut self,
        ast: &SchemaAst,
        value: View<'input>,
        depth: usize,
    ) -> Result<(), BinaryError> {
        let mut stack = match self.diagnostics {
            Diagnostics::Native => FrameStack::native(),
            Diagnostics::Prepared(_) => self
                .traversal
                .ok_or(BinaryError::Memory(
                    crate::memory::MemoryError::UnsupportedRunContract,
                ))?
                .checkout()
                .map_err(BinaryError::Memory)?,
        };
        self.value_view_with_stack(ast, value, depth, &mut stack)
    }
    fn value_view_with_stack<'input>(
        &mut self,
        ast: &SchemaAst,
        value: View<'input>,
        depth: usize,
        stack: &mut FrameStack<'input>,
    ) -> Result<(), BinaryError> {
        stack.push(ControlFrame::EncodeShape {
            shape: ast.shape_owner(),
            value,
            depth,
            charge: true,
        })?;
        let mut failure = None;
        while let Some(frame) = stack.pop() {
            if let Some(mut error) = failure.take() {
                match frame {
                    ControlFrame::EncodePath(path) => {
                        error = match path {
                            OwnedPath::Index(index) => {
                                at(error, BinaryPath::Index(index), self.diagnostics)
                            }
                            OwnedPath::Field(name) => {
                                at(error, BinaryPath::Field(&name), self.diagnostics)
                            }
                        };
                        failure = Some(error);
                    }
                    ControlFrame::EncodeUnionWait {
                        shape,
                        value,
                        depth,
                        next,
                        mut failures,
                        start,
                    } => {
                        if matches!(error, BinaryError::LimitExceeded(_))
                            || ((self.errors.is_some() || self.bank.is_some())
                                && matches!(error, BinaryError::Memory(_)))
                        {
                            failure = Some(error);
                            continue;
                        }
                        self.truncate(start);
                        let retained = match &mut failures {
                            UnionFailures::Native(values) => {
                                values.push(error);
                                Ok(())
                            }
                            UnionFailures::Prepared(values) => {
                                values.try_push(error).map_err(|_| {
                                    BinaryError::Memory(crate::memory::MemoryError::NoSlot)
                                })
                            }
                            UnionFailures::Bank(values) => values.try_push(error).map_err(|_| {
                                BinaryError::Memory(crate::memory::MemoryError::NoSlot)
                            }),
                        };
                        if let Err(error) = retained {
                            failure = Some(error);
                            continue;
                        }
                        let SchemaShape::Union(values) = shape.as_ref() else {
                            return Err(issue(SchemaIssueKind::InvalidDefinition));
                        };
                        let count = values.len();
                        if next == count {
                            failure = Some(match failures {
                                UnionFailures::Native(values) => {
                                    BinaryError::NoUnionVariant(values)
                                }
                                UnionFailures::Prepared(values) => {
                                    BinaryError::PreparedNoUnionVariant(values.publish())
                                }
                                UnionFailures::Bank(values) => match values.publish() {
                                    Ok(values) => BinaryError::PreparedNoUnionVariant(values),
                                    Err(error) => BinaryError::Memory(error),
                                },
                            });
                        } else if let Err(error) = stack.push(ControlFrame::EncodeUnionTry {
                            shape,
                            value,
                            depth,
                            next,
                            failures,
                        }) {
                            failure = Some(error);
                        }
                    }
                    ControlFrame::DecodeShape { .. }
                    | ControlFrame::DecodeAny { .. }
                    | ControlFrame::DecodeSequence { .. }
                    | ControlFrame::DecodeObject { .. }
                    | ControlFrame::DecodeAnyCollection { .. }
                    | ControlFrame::DecodeAppend { .. }
                    | ControlFrame::DecodeCheck { .. }
                    | ControlFrame::DecodeLiteral { .. }
                    | ControlFrame::DecodePath(_)
                    | ControlFrame::EncodeShape { .. }
                    | ControlFrame::EncodeSequence { .. }
                    | ControlFrame::EncodeObject { .. }
                    | ControlFrame::EncodeAny { .. }
                    | ControlFrame::EncodeAnyChildren { .. }
                    | ControlFrame::EncodeUnionTry { .. }
                    | ControlFrame::Compare { .. }
                    | ControlFrame::CompareArray { .. }
                    | ControlFrame::CompareObject { .. }
                    | ControlFrame::CompareNativeObject { .. } => failure = Some(error),
                }
                continue;
            }
            let result = (|| -> Result<(), BinaryError> {
                match frame {
                    ControlFrame::EncodeShape {
                        shape,
                        value,
                        depth,
                        charge,
                    } => {
                        if charge {
                            self.budget.value(depth)?;
                        }
                        self.budget.step()?;
                        match shape.as_ref() {
                            SchemaShape::Opaque => Err(BinaryError::UnsupportedOpaque),
                            SchemaShape::Primitive(PrimitiveShape::Any) => {
                                stack.push(ControlFrame::EncodeAny {
                                    value,
                                    depth,
                                    charge: false,
                                })
                            }
                            SchemaShape::Primitive(primitive) => {
                                self.primitive(*primitive, value, depth)
                            }
                            SchemaShape::Literal(literal) => {
                                let equal = {
                                    let mut suffix = stack.suffix();
                                    equals_literal_scoped(value, literal.as_ref(), &mut suffix)?
                                };
                                if !equal {
                                    return Err(issue(SchemaIssueKind::LiteralMismatch));
                                }
                                stack.push(ControlFrame::EncodeAny {
                                    value,
                                    depth,
                                    charge: false,
                                })
                            }
                            SchemaShape::Sequence { maximum, .. } => {
                                let ViewKind::Array(count) = value.kind()? else {
                                    return Err(view_mismatch(ValueKind::Sequence, value));
                                };
                                self.budget.collection(count)?;
                                if maximum.is_some_and(|limit| count > limit.get()) {
                                    return Err(BinaryError::LimitExceeded(SchemaResource::Width));
                                }
                                self.length(count)?;
                                let depth = child_depth(depth)?;
                                stack.push(ControlFrame::EncodeSequence {
                                    shape,
                                    values: value.children(),
                                    index: 0,
                                    depth,
                                    tuple: false,
                                })
                            }
                            SchemaShape::Tuple(parts) => {
                                let ViewKind::Array(count) = value.kind()? else {
                                    return Err(view_mismatch(ValueKind::Sequence, value));
                                };
                                self.budget.collection(parts.len())?;
                                if count != parts.len() {
                                    return Err(issue(SchemaIssueKind::LengthMismatch {
                                        expected: parts.len(),
                                        actual: count,
                                    }));
                                }
                                let depth = child_depth(depth)?;
                                stack.push(ControlFrame::EncodeSequence {
                                    shape,
                                    values: value.children(),
                                    index: 0,
                                    depth,
                                    tuple: true,
                                })
                            }
                            SchemaShape::Object {
                                properties,
                                unknown_fields,
                            } => {
                                let ViewKind::Object(count) = value.kind()? else {
                                    return Err(view_mismatch(ValueKind::Record, value));
                                };
                                self.budget.collection(count)?;
                                self.budget.collection(properties.len())?;
                                if *unknown_fields == UnknownFields::Reject {
                                    for (name, _) in value.children() {
                                        let name = name.ok_or_else(|| {
                                            issue(SchemaIssueKind::InvalidDefinition)
                                        })?;
                                        if !properties.iter().any(|p| p.name() == name) {
                                            return Err(unknown_field(name, self.diagnostics));
                                        }
                                    }
                                }
                                let depth = child_depth(depth)?;
                                stack.push(ControlFrame::EncodeObject {
                                    shape,
                                    value,
                                    index: 0,
                                    depth,
                                })
                            }
                            SchemaShape::Union(variants) => {
                                self.budget.collection(variants.len())?;
                                let failures = if let Some(bank) = self.bank {
                                    let failures = bank.checkout().map_err(BinaryError::Memory)?;
                                    if failures.capacity() < variants.len() {
                                        let requested = std::alloc::Layout::array::<BinaryError>(
                                            variants.len(),
                                        )
                                        .map_err(|_| {
                                            BinaryError::Memory(
                                                crate::memory::MemoryError::InvalidLayout,
                                            )
                                        })?
                                        .size();
                                        let capacity = std::alloc::Layout::array::<BinaryError>(
                                            failures.capacity(),
                                        )
                                        .map_err(|_| {
                                            BinaryError::Memory(
                                                crate::memory::MemoryError::InvalidLayout,
                                            )
                                        })?
                                        .size();
                                        return Err(BinaryError::Memory(
                                            crate::memory::MemoryError::ValueTooLarge {
                                                requested: crate::memory::Bytes::new(requested),
                                                capacity: crate::memory::Bytes::new(capacity),
                                            },
                                        ));
                                    }
                                    UnionFailures::Bank(failures)
                                } else if let Some(pool) = self.errors {
                                    let failures =
                                        pool.reserve_shared().map_err(BinaryError::Memory)?;
                                    if failures.capacity() < variants.len() {
                                        let requested = std::alloc::Layout::array::<BinaryError>(
                                            variants.len(),
                                        )
                                        .map_err(|_| {
                                            BinaryError::Memory(
                                                crate::memory::MemoryError::InvalidLayout,
                                            )
                                        })?
                                        .size();
                                        let capacity = std::alloc::Layout::array::<BinaryError>(
                                            failures.capacity(),
                                        )
                                        .map_err(|_| {
                                            BinaryError::Memory(
                                                crate::memory::MemoryError::InvalidLayout,
                                            )
                                        })?
                                        .size();
                                        return Err(BinaryError::Memory(
                                            crate::memory::MemoryError::ValueTooLarge {
                                                requested: crate::memory::Bytes::new(requested),
                                                capacity: crate::memory::Bytes::new(capacity),
                                            },
                                        ));
                                    }
                                    UnionFailures::Prepared(failures)
                                } else {
                                    UnionFailures::Native(Vec::new())
                                };
                                if variants.is_empty() {
                                    return Err(match failures {
                                        UnionFailures::Native(values) => {
                                            BinaryError::NoUnionVariant(values)
                                        }
                                        UnionFailures::Prepared(values) => {
                                            BinaryError::PreparedNoUnionVariant(values.publish())
                                        }
                                        UnionFailures::Bank(values) => match values.publish() {
                                            Ok(values) => {
                                                BinaryError::PreparedNoUnionVariant(values)
                                            }
                                            Err(error) => BinaryError::Memory(error),
                                        },
                                    });
                                }
                                stack.push(ControlFrame::EncodeUnionTry {
                                    shape,
                                    value,
                                    depth,
                                    next: 0,
                                    failures,
                                })
                            }
                            SchemaShape::Reference(reference) => {
                                let target = reference
                                    .shape_owner()
                                    .ok_or(BinaryError::UnresolvedReference)?;
                                stack.push(ControlFrame::EncodeShape {
                                    shape: target,
                                    value,
                                    depth,
                                    charge: false,
                                })
                            }
                            SchemaShape::Checked { source, .. }
                            | SchemaShape::EncodedView(source) => {
                                stack.push(ControlFrame::EncodeShape {
                                    shape: source.shape_owner(),
                                    value,
                                    depth,
                                    charge: false,
                                })
                            }
                            SchemaShape::IntegerRange {
                                source,
                                minimum,
                                maximum,
                            } => {
                                let number = value
                                    .as_i64()
                                    .ok_or_else(|| view_mismatch(ValueKind::Integer, value))?;
                                if number < *minimum || number > *maximum {
                                    return Err(issue(SchemaIssueKind::OutOfRange));
                                }
                                stack.push(ControlFrame::EncodeShape {
                                    shape: source.shape_owner(),
                                    value,
                                    depth,
                                    charge: false,
                                })
                            }
                            SchemaShape::UnsignedRange {
                                source,
                                minimum,
                                maximum,
                            } => {
                                let number = value
                                    .as_u64()
                                    .ok_or_else(|| view_mismatch(ValueKind::Integer, value))?;
                                if number < *minimum || number > *maximum {
                                    return Err(issue(SchemaIssueKind::OutOfRange));
                                }
                                stack.push(ControlFrame::EncodeShape {
                                    shape: source.shape_owner(),
                                    value,
                                    depth,
                                    charge: false,
                                })
                            }
                            SchemaShape::StringLength {
                                source,
                                minimum,
                                maximum,
                                unit,
                            } => {
                                check_view_string_length(value, *minimum, *maximum, *unit)?;
                                stack.push(ControlFrame::EncodeShape {
                                    shape: source.shape_owner(),
                                    value,
                                    depth,
                                    charge: false,
                                })
                            }
                            SchemaShape::StringPattern { source, pattern } => {
                                let text = value
                                    .as_str()
                                    .ok_or_else(|| view_mismatch(ValueKind::String, value))?;
                                if !pattern.matches(text) {
                                    return Err(issue(SchemaIssueKind::PatternMismatch));
                                }
                                stack.push(ControlFrame::EncodeShape {
                                    shape: source.shape_owner(),
                                    value,
                                    depth,
                                    charge: false,
                                })
                            }
                        }
                    }
                    ControlFrame::EncodeSequence {
                        shape,
                        mut values,
                        index,
                        depth,
                        tuple,
                    } => {
                        if let Some((_, value)) = values.next() {
                            let child = if let SchemaShape::Sequence { element, .. } =
                                shape.as_ref()
                                && !tuple
                            {
                                element.shape_owner()
                            } else if let SchemaShape::Tuple(parts) = shape.as_ref()
                                && tuple
                            {
                                parts
                                    .get(index)
                                    .ok_or_else(|| issue(SchemaIssueKind::InvalidDefinition))?
                                    .shape_owner()
                            } else {
                                return Err(issue(SchemaIssueKind::InvalidDefinition));
                            };
                            stack.push(ControlFrame::EncodeSequence {
                                shape,
                                values,
                                index: index + 1,
                                depth,
                                tuple,
                            })?;
                            stack.push(ControlFrame::EncodePath(OwnedPath::Index(index)))?;
                            stack.push(ControlFrame::EncodeShape {
                                shape: child,
                                value,
                                depth,
                                charge: true,
                            })?;
                        }
                        Ok(())
                    }
                    ControlFrame::EncodeObject {
                        shape,
                        value,
                        index,
                        depth,
                    } => {
                        let SchemaShape::Object { properties, .. } = shape.as_ref() else {
                            return Err(issue(SchemaIssueKind::InvalidDefinition));
                        };
                        if let Some(property) = properties.get(index) {
                            let name = property.shared_name();
                            let child = property.schema().shape_owner();
                            let found = value.get(property.name());
                            let required = property.required() && property.pure_default().is_none();
                            stack.push(ControlFrame::EncodeObject {
                                shape,
                                value,
                                index: index + 1,
                                depth,
                            })?;
                            match found {
                                Some(value) => {
                                    self.byte(PRESENT)?;
                                    stack.push(ControlFrame::EncodePath(OwnedPath::Field(name)))?;
                                    stack.push(ControlFrame::EncodeShape {
                                        shape: child,
                                        value,
                                        depth,
                                        charge: true,
                                    })?;
                                }
                                None if required => {
                                    return Err(issue(SchemaIssueKind::MissingField(name)));
                                }
                                None => self.byte(ABSENT)?,
                            }
                        }
                        Ok(())
                    }
                    ControlFrame::EncodeAny {
                        value,
                        depth,
                        charge,
                    } => {
                        if charge {
                            self.budget.value(depth)?;
                        }
                        self.budget.step()?;
                        match value.kind()? {
                            ViewKind::Null => self.byte(ANY_NULL),
                            ViewKind::Bool(false) => self.byte(ANY_FALSE),
                            ViewKind::Bool(true) => self.byte(ANY_TRUE),
                            ViewKind::Number(_) => {
                                let (kind, bytes) = number_parts(value, ValueKind::Number)?;
                                let tag = match kind {
                                    SIGNED => ANY_SIGNED,
                                    UNSIGNED => ANY_UNSIGNED,
                                    FLOAT => ANY_FLOAT,
                                    _ => return Err(BinaryError::InvalidTag(kind)),
                                };
                                self.byte(tag)?;
                                self.raw(&bytes)
                            }
                            ViewKind::String(text) => {
                                self.byte(ANY_STRING)?;
                                self.text(text)
                            }
                            ViewKind::Array(count) | ViewKind::Object(count) => {
                                self.budget.collection(count)?;
                                let object = matches!(value.kind()?, ViewKind::Object(_));
                                self.byte(if object { ANY_OBJECT } else { ANY_ARRAY })?;
                                self.length(count)?;
                                stack.push(ControlFrame::EncodeAnyChildren {
                                    values: value.children(),
                                    depth: child_depth(depth)?,
                                    object,
                                })
                            }
                        }
                    }
                    ControlFrame::EncodeAnyChildren {
                        mut values,
                        depth,
                        object,
                    } => {
                        if let Some((name, value)) = values.next() {
                            if object {
                                self.text(
                                    name.ok_or_else(|| issue(SchemaIssueKind::InvalidDefinition))?,
                                )?;
                            }
                            stack.push(ControlFrame::EncodeAnyChildren {
                                values,
                                depth,
                                object,
                            })?;
                            stack.push(ControlFrame::EncodeAny {
                                value,
                                depth,
                                charge: true,
                            })?;
                        }
                        Ok(())
                    }
                    ControlFrame::EncodeUnionTry {
                        shape,
                        value,
                        depth,
                        next,
                        failures,
                    } => {
                        let SchemaShape::Union(variants) = shape.as_ref() else {
                            return Err(issue(SchemaIssueKind::InvalidDefinition));
                        };
                        let variant = variants
                            .get(next)
                            .ok_or_else(|| issue(SchemaIssueKind::InvalidDefinition))?
                            .shape_owner();
                        let start = self.len();
                        stack.push(ControlFrame::EncodeUnionWait {
                            shape,
                            value,
                            depth,
                            next: next + 1,
                            failures,
                            start,
                        })?;
                        self.length(next)?;
                        stack.push(ControlFrame::EncodeShape {
                            shape: variant,
                            value,
                            depth,
                            charge: false,
                        })
                    }
                    ControlFrame::EncodePath(_) | ControlFrame::EncodeUnionWait { .. } => Ok(()),
                    ControlFrame::DecodeShape { .. }
                    | ControlFrame::DecodeAny { .. }
                    | ControlFrame::DecodeSequence { .. }
                    | ControlFrame::DecodeObject { .. }
                    | ControlFrame::DecodeAnyCollection { .. }
                    | ControlFrame::DecodeAppend { .. }
                    | ControlFrame::DecodeCheck { .. }
                    | ControlFrame::DecodeLiteral { .. }
                    | ControlFrame::DecodePath(_)
                    | ControlFrame::Compare { .. }
                    | ControlFrame::CompareArray { .. }
                    | ControlFrame::CompareObject { .. }
                    | ControlFrame::CompareNativeObject { .. } => {
                        Err(issue(SchemaIssueKind::InvalidDefinition))
                    }
                }
            })();
            if let Err(error) = result {
                failure = Some(error);
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn primitive(
        &mut self,
        shape: PrimitiveShape,
        value: View<'_>,
        _depth: usize,
    ) -> Result<(), BinaryError> {
        match shape {
            PrimitiveShape::Any => Err(issue(SchemaIssueKind::InvalidDefinition)),
            PrimitiveShape::Null if matches!(value.kind()?, ViewKind::Null) => Ok(()),
            PrimitiveShape::Null => Err(view_mismatch(ValueKind::Null, value)),
            PrimitiveShape::Boolean => {
                if let ViewKind::Bool(boolean) = value.kind()? {
                    self.byte(u8::from(boolean))
                } else {
                    Err(view_mismatch(ValueKind::Boolean, value))
                }
            }
            PrimitiveShape::String => {
                if let ViewKind::String(text) = value.kind()? {
                    self.text(text)
                } else {
                    Err(view_mismatch(ValueKind::String, value))
                }
            }
            PrimitiveShape::Integer => {
                let (tag, bytes) = number_parts(value, ValueKind::Integer)?;
                if tag == FLOAT {
                    return Err(view_mismatch(ValueKind::Integer, value));
                }
                self.byte(tag)?;
                self.raw(&bytes)
            }
            PrimitiveShape::Number => {
                let (tag, bytes) = number_parts(value, ValueKind::Number)?;
                self.byte(tag)?;
                self.raw(&bytes)
            }
            PrimitiveShape::Binary => {
                let ViewKind::Array(count) = value.kind()? else {
                    return Err(view_mismatch(ValueKind::Binary, value));
                };
                self.budget.collection(count)?;
                self.length(count)?;
                for (index, (_, item)) in value.children().enumerate() {
                    let byte = item
                        .as_u64()
                        .and_then(|number| u8::try_from(number).ok())
                        .ok_or_else(|| {
                            at(
                                issue(SchemaIssueKind::OutOfRange),
                                BinaryPath::Index(index),
                                self.diagnostics,
                            )
                        })?;
                    self.byte(byte)?;
                }
                Ok(())
            }
        }
    }
}
impl Drop for Output<'_, '_> {
    fn drop(&mut self) {
        if !self.committed && matches!(self.bytes, Destination::Prepared(_)) {
            self.truncate(self.initial);
        }
    }
}

/// Typed cursor used by encoding schema binary internal.
///
/// # Type parameters
/// - `'a`: Lifetime parameter.
pub(super) struct Cursor<'a> {
    input: &'a [u8],
    pub(super) offset: usize,
    budget: Budget,
}
pub(super) enum DecodeNode {
    Native(JsonValue),
    Managed(usize),
}
impl DecodeNode {
    pub(super) fn managed_root(self) -> Result<usize, BinaryError> {
        match self {
            Self::Managed(root) => Ok(root),
            Self::Native(_) => Err(issue(SchemaIssueKind::InvalidDefinition)),
        }
    }
}
pub(super) enum DecodeBackend<'a> {
    Native,
    Managed {
        owner: &'a mut crate::schema::internal::parser::PreparedDocumentAssembly,
        issues: Option<&'a crate::schema::PreparedIssueMemory>,
    },
}
pub(super) fn backing_error(error: SchemaIssue) -> BinaryError {
    if let SchemaIssueKind::Memory(value) = error.kind() {
        return BinaryError::Memory(*value);
    }
    if let SchemaIssueKind::LimitExceeded { resource } = error.kind() {
        return BinaryError::LimitExceeded(*resource);
    }
    BinaryError::Schema(error)
}
impl DecodeBackend<'_> {
    fn diagnostics(&self) -> Diagnostics<'_> {
        match self {
            Self::Native => Diagnostics::Native,
            Self::Managed { issues, .. } => Diagnostics::Prepared(*issues),
        }
    }
    fn view<'a>(&'a self, value: &'a DecodeNode) -> Result<View<'a>, BinaryError> {
        match (self, value) {
            (Self::Native, DecodeNode::Native(value)) => Ok(View::Native(value)),
            (Self::Managed { owner, .. }, DecodeNode::Managed(index)) => {
                owner.view(*index).map(View::Managed).map_err(backing_error)
            }
            (Self::Native, DecodeNode::Managed(_))
            | (Self::Managed { .. }, DecodeNode::Native(_)) => {
                Err(issue(SchemaIssueKind::InvalidDefinition))
            }
        }
    }
    fn make(
        &mut self,
        kind: crate::schema::internal::parser::AssemblyKind<'_>,
        depth: usize,
    ) -> Result<DecodeNode, BinaryError> {
        use crate::schema::internal::parser::AssemblyKind as K;
        match self {
            Self::Managed { owner, .. } => owner
                .node(kind, depth)
                .map(DecodeNode::Managed)
                .map_err(backing_error),
            Self::Native => Ok(DecodeNode::Native(match kind {
                K::Null => JsonValue::Null,
                K::Bool(value) => JsonValue::Bool(value),
                K::String(value) => JsonValue::String(value.to_owned()),
                K::Array => JsonValue::Array(Vec::new()),
                K::Object => JsonValue::Object(serde_json::Map::new()),
                K::Number(text) => {
                    JsonValue::Number(text.parse().map_err(|_| BinaryError::InvalidNumber)?)
                }
            })),
        }
    }
    fn append(
        &mut self,
        parent: &mut DecodeNode,
        child: DecodeNode,
        name: Option<&str>,
    ) -> Result<(), BinaryError> {
        match (self, parent, child, name) {
            (
                Self::Native,
                DecodeNode::Native(JsonValue::Array(values)),
                DecodeNode::Native(child),
                None,
            ) => {
                values.push(child);
                Ok(())
            }
            (
                Self::Native,
                DecodeNode::Native(JsonValue::Object(values)),
                DecodeNode::Native(child),
                Some(name),
            ) => {
                values.insert(name.to_owned(), child);
                Ok(())
            }
            (
                Self::Managed { owner, .. },
                DecodeNode::Managed(parent),
                DecodeNode::Managed(child),
                name,
            ) => owner.append(*parent, child, name).map_err(backing_error),
            _ => Err(issue(SchemaIssueKind::InvalidDefinition)),
        }
    }
    fn append_owned_name(
        &mut self,
        parent: &mut DecodeNode,
        child: DecodeNode,
        name: String,
    ) -> Result<(), BinaryError> {
        match (self, parent, child) {
            (
                Self::Native,
                DecodeNode::Native(JsonValue::Object(values)),
                DecodeNode::Native(child),
            ) => {
                values.insert(name, child);
                Ok(())
            }
            _ => Err(issue(SchemaIssueKind::InvalidDefinition)),
        }
    }
}

impl<'a> Cursor<'a> {
    /// # Type parameters
    /// - `'a`: Lifetime parameter.
    ///
    /// # Parameters
    /// - `input` (`&'a [u8]`): The input value or factory consumed by this operation.
    /// - `limits` (`BinaryLimits`): Explicit resource bounds enforced while processing this input.
    ///
    /// # Returns
    /// Type: `Result<Self, BinaryError>`.
    pub(super) fn new(input: &'a [u8], limits: BinaryLimits) -> Result<Self, BinaryError> {
        if input.len() > limits.bytes.get() {
            return Err(BinaryError::LimitExceeded(SchemaResource::Bytes));
        }
        Ok(Self {
            input,
            offset: 0,
            budget: Budget {
                limits,
                nodes: 0,
                steps: 0,
            },
        })
    }

    ///
    /// # Parameters
    /// - `count` (`usize`): Caller-selected number of iterations, values, or permits.
    ///
    /// # Returns
    /// Type: `Result<&'a [u8], BinaryError>`.
    fn raw(&mut self, count: usize) -> Result<&'a [u8], BinaryError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(BinaryError::LengthOverflow)?;
        let bytes = self
            .input
            .get(self.offset..end)
            .ok_or(BinaryError::Truncated {
                offset: self.offset,
                needed: count,
            })?;
        self.offset = end;
        Ok(bytes)
    }

    /// # Type parameters
    /// - `'a`: Lifetime parameter.
    ///
    /// # Returns
    /// Type: `Result<u8, BinaryError>`.
    pub(super) fn byte(&mut self) -> Result<u8, BinaryError> {
        let offset = self.offset;
        self.raw(1)?
            .first()
            .copied()
            .ok_or(BinaryError::Truncated { offset, needed: 1 })
    }

    /// # Type parameters
    /// - `'a`: Lifetime parameter.
    ///
    /// # Returns
    /// Type: `Result<u64, BinaryError>`.
    pub(super) fn u64(&mut self) -> Result<u64, BinaryError> {
        let mut bytes = [0; size_of::<u64>()];
        let length = bytes.len();
        bytes.copy_from_slice(self.raw(length)?);
        Ok(u64::from_be_bytes(bytes))
    }

    ///
    /// # Returns
    /// Type: `Result<usize, BinaryError>`.
    fn length(&mut self) -> Result<usize, BinaryError> {
        usize::try_from(self.u64()?).map_err(|_| BinaryError::LengthOverflow)
    }

    /// # Type parameters
    /// - `'a`: Lifetime parameter.
    ///
    /// # Returns
    /// Type: `Result<&'a str, BinaryError>`.
    pub(super) fn text(&mut self) -> Result<&'a str, BinaryError> {
        let count = self.length()?;
        std::str::from_utf8(self.raw(count)?).map_err(|_| BinaryError::InvalidUtf8)
    }

    /// # Type parameters
    /// - `'a`: Lifetime parameter.
    ///
    /// # Parameters
    /// - `ast` (`&SchemaAst`): Representation tree used for schema inspection or export.
    /// - `depth` (`usize`): Nesting depth used to enforce a parser or schema bound.
    ///
    /// # Returns
    /// Type: `Result<JsonValue, BinaryError>`.
    pub(super) fn value(
        &mut self,
        ast: &SchemaAst,
        depth: usize,
    ) -> Result<JsonValue, BinaryError> {
        match self.value_node(ast, depth, &mut DecodeBackend::Native, None)? {
            DecodeNode::Native(value) => Ok(value),
            DecodeNode::Managed(_) => Err(issue(SchemaIssueKind::InvalidDefinition)),
        }
    }
    pub(super) fn value_node(
        &mut self,
        ast: &SchemaAst,
        depth: usize,
        backend: &mut DecodeBackend,
        traversal: Option<&PreparedBinaryTraversalPool>,
    ) -> Result<DecodeNode, BinaryError> {
        use crate::schema::internal::parser::AssemblyKind as K;
        let mut stack = if matches!(backend, DecodeBackend::Native) {
            FrameStack::native()
        } else {
            traversal
                .ok_or(BinaryError::Memory(
                    crate::memory::MemoryError::UnsupportedRunContract,
                ))?
                .checkout()
                .map_err(BinaryError::Memory)?
        };
        stack.push(ControlFrame::DecodeShape {
            shape: ast.shape_owner(),
            depth,
            charge: true,
        })?;
        let mut value: Option<DecodeNode> = None;
        let mut failure: Option<BinaryError> = None;
        while let Some(frame) = stack.pop() {
            if let Some(error) = failure.take() {
                failure = Some(if let ControlFrame::DecodePath(path) = frame {
                    match path {
                        OwnedPath::Index(index) => {
                            at(error, BinaryPath::Index(index), backend.diagnostics())
                        }
                        OwnedPath::Field(name) => {
                            at(error, BinaryPath::Field(&name), backend.diagnostics())
                        }
                    }
                } else {
                    error
                });
                continue;
            }
            let result = (|| -> Result<(), BinaryError> {
                match frame {
                    ControlFrame::DecodeShape {
                        shape,
                        depth,
                        charge,
                    } => {
                        if charge {
                            self.budget.value(depth)?;
                        }
                        self.budget.step()?;
                        match shape.as_ref() {
                            SchemaShape::Opaque => return Err(BinaryError::UnsupportedOpaque),
                            SchemaShape::Primitive(PrimitiveShape::Any) => {
                                stack.push(ControlFrame::DecodeAny {
                                    depth,
                                    charge: false,
                                })?
                            }
                            SchemaShape::Primitive(primitive) => {
                                value = Some(self.primitive(*primitive, depth, backend)?)
                            }
                            SchemaShape::Literal(_) => {
                                stack.push(ControlFrame::DecodeLiteral { shape })?;
                                stack.push(ControlFrame::DecodeAny {
                                    depth,
                                    charge: false,
                                })?;
                            }
                            SchemaShape::Sequence { maximum, .. } => {
                                let count = self.length()?;
                                self.budget.collection(count)?;
                                if maximum.is_some_and(|limit| count > limit.get()) {
                                    return Err(BinaryError::LimitExceeded(SchemaResource::Width));
                                }
                                let parent = backend.make(K::Array, depth)?;
                                let depth = child_depth(depth)?;
                                stack.push(ControlFrame::DecodeSequence {
                                    shape,
                                    parent,
                                    index: 0,
                                    count,
                                    depth,
                                    tuple: false,
                                })?;
                            }
                            SchemaShape::Tuple(parts) => {
                                let count = parts.len();
                                self.budget.collection(count)?;
                                let parent = backend.make(K::Array, depth)?;
                                let depth = child_depth(depth)?;
                                stack.push(ControlFrame::DecodeSequence {
                                    shape,
                                    parent,
                                    index: 0,
                                    count,
                                    depth,
                                    tuple: true,
                                })?;
                            }
                            SchemaShape::Object { properties, .. } => {
                                self.budget.collection(properties.len())?;
                                let parent = backend.make(K::Object, depth)?;
                                let depth = child_depth(depth)?;
                                stack.push(ControlFrame::DecodeObject {
                                    shape,
                                    parent,
                                    index: 0,
                                    depth,
                                })?;
                            }
                            SchemaShape::Union(variants) => {
                                self.budget.collection(variants.len())?;
                                let index = self.u64()?;
                                let n = usize::try_from(index)
                                    .map_err(|_| BinaryError::InvalidUnionIndex(index))?;
                                let target = variants
                                    .get(n)
                                    .ok_or(BinaryError::InvalidUnionIndex(index))?
                                    .shape_owner();
                                stack.push(ControlFrame::DecodeShape {
                                    shape: target,
                                    depth,
                                    charge: false,
                                })?;
                            }
                            SchemaShape::Reference(reference) => {
                                let target = reference
                                    .shape_owner()
                                    .ok_or(BinaryError::UnresolvedReference)?;
                                stack.push(ControlFrame::DecodeShape {
                                    shape: target,
                                    depth,
                                    charge: false,
                                })?;
                            }
                            SchemaShape::Checked { source, .. }
                            | SchemaShape::EncodedView(source) => {
                                let target = source.shape_owner();
                                stack.push(ControlFrame::DecodeShape {
                                    shape: target,
                                    depth,
                                    charge: false,
                                })?;
                            }
                            SchemaShape::IntegerRange { source, .. }
                            | SchemaShape::UnsignedRange { source, .. }
                            | SchemaShape::StringLength { source, .. }
                            | SchemaShape::StringPattern { source, .. } => {
                                let target = source.shape_owner();
                                stack.push(ControlFrame::DecodeCheck { shape })?;
                                stack.push(ControlFrame::DecodeShape {
                                    shape: target,
                                    depth,
                                    charge: false,
                                })?;
                            }
                        }
                    }
                    ControlFrame::DecodeSequence {
                        shape,
                        parent,
                        index,
                        count,
                        depth,
                        tuple,
                    } => {
                        if index == count {
                            value = Some(parent);
                        } else {
                            let target = if let SchemaShape::Sequence { element, .. } =
                                shape.as_ref()
                                && !tuple
                            {
                                element.shape_owner()
                            } else if let SchemaShape::Tuple(parts) = shape.as_ref()
                                && tuple
                            {
                                parts
                                    .get(index)
                                    .ok_or_else(|| issue(SchemaIssueKind::InvalidDefinition))?
                                    .shape_owner()
                            } else {
                                return Err(issue(SchemaIssueKind::InvalidDefinition));
                            };
                            stack.push(ControlFrame::DecodeAppend {
                                parent,
                                name: None,
                                native_name: None,
                                duplicate: false,
                                resume: DecodeResume::Sequence {
                                    shape,
                                    index: index + 1,
                                    count,
                                    depth,
                                    tuple,
                                },
                            })?;
                            stack.push(ControlFrame::DecodePath(OwnedPath::Index(index)))?;
                            stack.push(ControlFrame::DecodeShape {
                                shape: target,
                                depth,
                                charge: true,
                            })?;
                        }
                    }
                    ControlFrame::DecodeObject {
                        shape,
                        parent,
                        index,
                        depth,
                    } => {
                        let SchemaShape::Object { properties, .. } = shape.as_ref() else {
                            return Err(issue(SchemaIssueKind::InvalidDefinition));
                        };
                        if let Some(property) = properties.get(index) {
                            match self.byte()? {
                                ABSENT
                                    if property.required() && property.pure_default().is_none() =>
                                {
                                    return Err(issue(SchemaIssueKind::MissingField(
                                        property.shared_name(),
                                    )));
                                }
                                ABSENT => stack.push(ControlFrame::DecodeObject {
                                    shape,
                                    parent,
                                    index: index + 1,
                                    depth,
                                })?,
                                PRESENT => {
                                    let name = property.shared_name();
                                    let target = property.schema().shape_owner();
                                    stack.push(ControlFrame::DecodeAppend {
                                        parent,
                                        name: Some(DecodeName::Schema(Arc::clone(&name))),
                                        native_name: None,
                                        duplicate: false,
                                        resume: DecodeResume::Object {
                                            shape,
                                            index: index + 1,
                                            depth,
                                        },
                                    })?;
                                    stack.push(ControlFrame::DecodePath(OwnedPath::Field(name)))?;
                                    stack.push(ControlFrame::DecodeShape {
                                        shape: target,
                                        depth,
                                        charge: true,
                                    })?;
                                }
                                tag => return Err(BinaryError::InvalidTag(tag)),
                            }
                        } else {
                            value = Some(parent);
                        }
                    }
                    ControlFrame::DecodeAny { depth, charge } => {
                        if charge {
                            self.budget.value(depth)?;
                        }
                        self.budget.step()?;
                        match self.byte()? {
                            ANY_NULL => value = Some(backend.make(K::Null, depth)?),
                            ANY_FALSE => value = Some(backend.make(K::Bool(false), depth)?),
                            ANY_TRUE => value = Some(backend.make(K::Bool(true), depth)?),
                            ANY_SIGNED => {
                                value = Some(self.number_payload(SIGNED, true, depth, backend)?)
                            }
                            ANY_UNSIGNED => {
                                value = Some(self.number_payload(UNSIGNED, true, depth, backend)?)
                            }
                            ANY_FLOAT => {
                                value = Some(self.number_payload(FLOAT, true, depth, backend)?)
                            }
                            ANY_STRING => {
                                value = Some(backend.make(K::String(self.text()?), depth)?)
                            }
                            tag @ (ANY_ARRAY | ANY_OBJECT) => {
                                let count = self.length()?;
                                self.budget.collection(count)?;
                                let object = tag == ANY_OBJECT;
                                let parent = backend
                                    .make(if object { K::Object } else { K::Array }, depth)?;
                                let depth = child_depth(depth)?;
                                stack.push(ControlFrame::DecodeAnyCollection {
                                    parent,
                                    index: 0,
                                    count,
                                    depth,
                                    object,
                                })?;
                            }
                            tag => return Err(BinaryError::InvalidTag(tag)),
                        }
                    }
                    ControlFrame::DecodeAnyCollection {
                        parent,
                        index,
                        count,
                        depth,
                        object,
                    } => {
                        if index == count {
                            value = Some(parent);
                        } else {
                            let name = if object { Some(self.text()?) } else { None };
                            let native_name = if matches!(backend, DecodeBackend::Native) {
                                name.map(str::to_owned)
                            } else {
                                None
                            };
                            stack.push(ControlFrame::DecodeAppend {
                                parent,
                                name: name.map(DecodeName::Input),
                                native_name,
                                duplicate: object,
                                resume: DecodeResume::Any {
                                    index: index + 1,
                                    count,
                                    depth,
                                    object,
                                },
                            })?;
                            stack.push(ControlFrame::DecodeAny {
                                depth,
                                charge: true,
                            })?;
                        }
                    }
                    ControlFrame::DecodeAppend {
                        mut parent,
                        name,
                        native_name,
                        duplicate,
                        resume,
                    } => {
                        let child = value
                            .take()
                            .ok_or_else(|| issue(SchemaIssueKind::InvalidDefinition))?;
                        let name = name.as_ref().map(|name| match name {
                            DecodeName::Input(name) => *name,
                            DecodeName::Schema(name) => name.as_ref(),
                        });
                        if duplicate
                            && let Some(name) = name
                            && backend.view(&parent)?.get(name).is_some()
                        {
                            return Err(match backend.diagnostics() {
                                Diagnostics::Native => BinaryError::DuplicateKey(
                                    native_name.unwrap_or_else(|| name.to_owned()),
                                ),
                                Diagnostics::Prepared(memory) => {
                                    match crate::schema::format_diagnostic_text(
                                        memory,
                                        format_args!("{name}"),
                                    ) {
                                        Ok(name) => BinaryError::PreparedDuplicateKey(name),
                                        Err(issue) => backing_error(issue),
                                    }
                                }
                            });
                        }
                        if let Some(name) = native_name {
                            backend.append_owned_name(&mut parent, child, name)?;
                        } else {
                            backend.append(&mut parent, child, name)?;
                        }
                        stack.push(match resume {
                            DecodeResume::Sequence {
                                shape,
                                index,
                                count,
                                depth,
                                tuple,
                            } => ControlFrame::DecodeSequence {
                                shape,
                                parent,
                                index,
                                count,
                                depth,
                                tuple,
                            },
                            DecodeResume::Object {
                                shape,
                                index,
                                depth,
                            } => ControlFrame::DecodeObject {
                                shape,
                                parent,
                                index,
                                depth,
                            },
                            DecodeResume::Any {
                                index,
                                count,
                                depth,
                                object,
                            } => ControlFrame::DecodeAnyCollection {
                                parent,
                                index,
                                count,
                                depth,
                                object,
                            },
                        })?;
                    }
                    ControlFrame::DecodeCheck { shape } => {
                        let current = value
                            .as_ref()
                            .ok_or_else(|| issue(SchemaIssueKind::InvalidDefinition))?;
                        let view = backend.view(current)?;
                        match shape.as_ref() {
                            SchemaShape::IntegerRange {
                                minimum, maximum, ..
                            } => {
                                let n = view
                                    .as_i64()
                                    .ok_or_else(|| view_mismatch(ValueKind::Integer, view))?;
                                if n < *minimum || n > *maximum {
                                    return Err(issue(SchemaIssueKind::OutOfRange));
                                }
                            }
                            SchemaShape::UnsignedRange {
                                minimum, maximum, ..
                            } => {
                                let n = view
                                    .as_u64()
                                    .ok_or_else(|| view_mismatch(ValueKind::Integer, view))?;
                                if n < *minimum || n > *maximum {
                                    return Err(issue(SchemaIssueKind::OutOfRange));
                                }
                            }
                            SchemaShape::StringLength {
                                minimum,
                                maximum,
                                unit,
                                ..
                            } => check_view_string_length(view, *minimum, *maximum, *unit)?,
                            SchemaShape::StringPattern { pattern, .. } => {
                                let text = view
                                    .as_str()
                                    .ok_or_else(|| view_mismatch(ValueKind::String, view))?;
                                if !pattern.matches(text) {
                                    return Err(issue(SchemaIssueKind::PatternMismatch));
                                }
                            }
                            SchemaShape::Opaque
                            | SchemaShape::Primitive(_)
                            | SchemaShape::Literal(_)
                            | SchemaShape::Sequence { .. }
                            | SchemaShape::Tuple(_)
                            | SchemaShape::Object { .. }
                            | SchemaShape::Union(_)
                            | SchemaShape::Reference(_)
                            | SchemaShape::Checked { .. }
                            | SchemaShape::EncodedView(_) => {
                                return Err(issue(SchemaIssueKind::InvalidDefinition));
                            }
                        }
                    }
                    ControlFrame::DecodeLiteral { shape } => {
                        let SchemaShape::Literal(literal) = shape.as_ref() else {
                            return Err(issue(SchemaIssueKind::InvalidDefinition));
                        };
                        let current = value
                            .as_ref()
                            .ok_or_else(|| issue(SchemaIssueKind::InvalidDefinition))?;
                        let equal = {
                            let mut suffix = stack.suffix();
                            equals_literal_scoped(backend.view(current)?, literal, &mut suffix)?
                        };
                        if !equal {
                            return Err(issue(SchemaIssueKind::LiteralMismatch));
                        }
                    }
                    ControlFrame::DecodePath(_) => {}
                    ControlFrame::EncodeShape { .. }
                    | ControlFrame::EncodeSequence { .. }
                    | ControlFrame::EncodeObject { .. }
                    | ControlFrame::EncodeAny { .. }
                    | ControlFrame::EncodeAnyChildren { .. }
                    | ControlFrame::EncodePath(_)
                    | ControlFrame::EncodeUnionTry { .. }
                    | ControlFrame::EncodeUnionWait { .. }
                    | ControlFrame::Compare { .. }
                    | ControlFrame::CompareArray { .. }
                    | ControlFrame::CompareObject { .. }
                    | ControlFrame::CompareNativeObject { .. } => {
                        return Err(issue(SchemaIssueKind::InvalidDefinition));
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                failure = Some(error);
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        value.ok_or_else(|| issue(SchemaIssueKind::InvalidDefinition))
    }

    ///
    /// # Parameters
    /// - `shape` (`PrimitiveShape`): Encoded-side structure required by this schema.
    /// - `depth` (`usize`): Nesting depth used to enforce a parser or schema bound.
    ///
    /// # Returns
    /// Type: `Result<JsonValue, BinaryError>`.
    fn primitive(
        &mut self,
        shape: PrimitiveShape,
        depth: usize,
        backend: &mut DecodeBackend,
    ) -> Result<DecodeNode, BinaryError> {
        use crate::schema::internal::parser::AssemblyKind as K;
        match shape {
            PrimitiveShape::Any => Err(issue(SchemaIssueKind::InvalidDefinition)),
            PrimitiveShape::Null => backend.make(K::Null, depth),
            PrimitiveShape::Boolean => match self.byte()? {
                FALSE => backend.make(K::Bool(false), depth),
                TRUE => backend.make(K::Bool(true), depth),
                tag => Err(BinaryError::InvalidTag(tag)),
            },
            PrimitiveShape::String => backend.make(K::String(self.text()?), depth),
            PrimitiveShape::Integer => self.number(false, depth, backend),
            PrimitiveShape::Number => self.number(true, depth, backend),
            PrimitiveShape::Binary => {
                let count = self.length()?;
                self.budget.collection(count)?;
                let bytes = self.raw(count)?;
                if matches!(backend, DecodeBackend::Native) {
                    return Ok(DecodeNode::Native(JsonValue::Array(
                        bytes
                            .iter()
                            .map(|byte| {
                                JsonValue::Number(serde_json::Number::from(u64::from(*byte)))
                            })
                            .collect(),
                    )));
                }
                let mut array = backend.make(K::Array, depth)?;
                for byte in bytes {
                    let child = child_depth(depth)?;
                    let mut buffer = std::fmt::NumBuffer::new();
                    let value = backend.make(K::Number(byte.format_into(&mut buffer)), child)?;
                    backend.append(&mut array, value, None)?;
                }
                Ok(array)
            }
        }
    }
    fn number(
        &mut self,
        allow_float: bool,
        depth: usize,
        backend: &mut DecodeBackend,
    ) -> Result<DecodeNode, BinaryError> {
        let tag = self.byte()?;
        self.number_payload(tag, allow_float, depth, backend)
    }
    fn number_payload(
        &mut self,
        tag: u8,
        allow_float: bool,
        depth: usize,
        backend: &mut DecodeBackend,
    ) -> Result<DecodeNode, BinaryError> {
        if tag > FLOAT || (tag == FLOAT && !allow_float) {
            return Err(BinaryError::InvalidTag(tag));
        }
        let mut bytes = [0; size_of::<u64>()];
        let length = bytes.len();
        bytes.copy_from_slice(self.raw(length)?);
        if matches!(backend, DecodeBackend::Native) {
            let number = match tag {
                SIGNED => serde_json::Number::from(i64::from_be_bytes(bytes)),
                UNSIGNED => serde_json::Number::from(u64::from_be_bytes(bytes)),
                FLOAT if allow_float => serde_json::Number::from_f64(f64::from_be_bytes(bytes))
                    .ok_or(BinaryError::InvalidNumber)?,
                _ => return Err(BinaryError::InvalidTag(tag)),
            };
            return Ok(DecodeNode::Native(JsonValue::Number(number)));
        }
        let mut signed_buffer = std::fmt::NumBuffer::new();
        let mut unsigned_buffer = std::fmt::NumBuffer::new();
        let mut float_buffer = [0u8; 24];
        let text = match tag {
            SIGNED => i64::from_be_bytes(bytes).format_into(&mut signed_buffer),
            UNSIGNED => u64::from_be_bytes(bytes).format_into(&mut unsigned_buffer),
            FLOAT if allow_float => {
                let float = f64::from_be_bytes(bytes);
                if !float.is_finite() {
                    return Err(BinaryError::InvalidNumber);
                }
                let mut remaining = float_buffer.as_mut_slice();
                serde_json::to_writer(&mut remaining, &float)
                    .map_err(|_| BinaryError::InvalidNumber)?;
                let used = 24 - remaining.len();
                std::str::from_utf8(float_buffer.get(..used).ok_or(BinaryError::InvalidNumber)?)
                    .map_err(|_| BinaryError::InvalidNumber)?
            }
            _ => return Err(BinaryError::InvalidTag(tag)),
        };
        backend.make(
            crate::schema::internal::parser::AssemblyKind::Number(text),
            depth,
        )
    }
}

///
/// # Parameters
/// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
/// - `minimum` (`usize`): Lower bound enforced by this policy.
/// - `maximum` (`usize`): Upper bound enforced by this policy.
/// - `unit` (`StringLengthUnit`): The unit supplied to this operation.
///
/// # Returns
/// Type: `Result<(), BinaryError>`.
fn check_view_string_length(
    value: View<'_>,
    minimum: usize,
    maximum: usize,
    unit: StringLengthUnit,
) -> Result<(), BinaryError> {
    let text = value
        .as_str()
        .ok_or_else(|| view_mismatch(ValueKind::String, value))?;
    let length = match unit {
        StringLengthUnit::Utf8Bytes => text.len(),
        StringLengthUnit::UnicodeScalars => text.chars().count(),
        StringLengthUnit::Graphemes => return Err(BinaryError::UnsupportedGraphemeLength),
    };
    if length < minimum || length > maximum {
        Err(issue(SchemaIssueKind::OutOfRange))
    } else {
        Ok(())
    }
}
