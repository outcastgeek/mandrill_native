//! Row programs decode owned native rows without an intermediate JSON representation.
//!
//! SQLx owns each returned row. Programs share that row's owner while reading cells;
//! strings/blobs are decoded into the caller's requested native owned Rust type,
//! rather than cloning the whole row or retaining a borrowed cell in a static Effect.

use crate::RowError;
use effect::{Codec, Effect, Exit, NativeContext, PureCodec};
use sqlx::{ColumnIndex, Decode, Row, Turso, TursoRow, Type, ValueRef};
use std::{future::Future, pin::Pin, sync::Arc};

/// One native row binding future, preserving the full structured outcome.
type RowFuture<'a, A> = Pin<Box<dyn Future<Output = Exit<A, RowError>> + Send + 'a>>;
/// Execution-local read consuming its captured native-cell codec/validator.
type PendingRead<A, R> =
    dyn FnOnce(Arc<TursoRow>, Arc<R>, NativeContext) -> RowFuture<'static, A> + Send;
/// Shared immutable row-program factory; output rows and payloads still move.
type RowProgram<A, R> = dyn Fn(RowContext<R>) -> RowFuture<'static, A> + Send + Sync;

/// Stable name or ordinal used in native-cell error paths.
#[derive(Clone, Debug)]
pub enum ColumnKey {
    /// Native SQL column name; missing names remain errors.
    Name(String),
    /// Zero-based SQLx ordinal.
    Ordinal(usize),
}
impl ColumnKey {
    /// Resolve native metadata while retaining the exact requested identity.
    fn index(&self, row: &TursoRow) -> Result<usize, RowError> {
        match self {
            Self::Name(name) => name.as_str().index(row),
            Self::Ordinal(index) => index.index(row),
        }
        .map_err(|_| RowError::MissingColumn(self.label()))
    }
    /// Produce a column path for an error without exposing its cell contents.
    fn label(&self) -> String {
        match self {
            Self::Name(name) => name.clone(),
            Self::Ordinal(index) => format!("[{index}]"),
        }
    }
}
impl From<&str> for ColumnKey {
    /// Retain a named column contract.
    fn from(name: &str) -> Self {
        Self::Name(name.to_owned())
    }
}
impl From<String> for ColumnKey {
    /// Move a dynamic column identity.
    fn from(name: String) -> Self {
        Self::Name(name)
    }
}
impl From<usize> for ColumnKey {
    /// Retain a zero-based native ordinal.
    fn from(index: usize) -> Self {
        Self::Ordinal(index)
    }
}

/// Native cell codec retaining Effect's existing structured validation and environment.
/// T is a SQLx Type/Decode representation, never a JSON DOM conversion.
#[must_use]
pub struct CellCodec<A: Send + 'static, T: Send + 'static, R: Send + Sync + 'static = ()> {
    /// Decode the one moved native Rust cell through the existing schema engine.
    codec: Codec<A, T, R, ()>,
}
impl<A: Send + 'static, T: Send + 'static, R: Send + Sync + 'static> Clone for CellCodec<A, T, R> {
    /// Share codec configuration while native cells and domain values still move once.
    fn clone(&self) -> Self {
        Self {
            codec: self.codec.clone(),
        }
    }
}
impl<A: Send + 'static, T: Send + 'static, R: Send + Sync + 'static> CellCodec<A, T, R> {
    /// Adapt an existing effectful native representation codec; validation may select services in R.
    pub fn new(codec: Codec<A, T, R, ()>) -> Self {
        Self { codec }
    }
    /// Adapt a pure native-cell codec to the selected R without adding an environment requirement.
    pub fn pure(codec: PureCodec<A, T>) -> Self {
        Self::new(codec.into_effect())
    }
    /// Encode one moved domain value to its native binding representation lazily.
    /// Bind this Effect explicitly before `param`/`value`/`set`; construction never
    /// invokes the codec or invents a SQLx Type implementation for a foreign type.
    pub fn encode_once(&self, value: A) -> Effect<T, effect::SchemaIssue, R> {
        self.codec.encode_once(value).provide_some(|_: &R| ())
    }
}

/// Pending one-cell native decode; ownership moves into its row-program binding.
#[must_use]
pub struct RowRead<A: Send + 'static, R: Send + Sync + 'static = ()> {
    /// One execution-local read, never a shared row-cell copy.
    read: Box<PendingRead<A, R>>,
}
/// Read a required named or ordinal cell through a native schema codec.
pub fn column<A, T, R>(key: impl Into<ColumnKey>, codec: CellCodec<A, T, R>) -> RowRead<A, R>
where
    A: Send + 'static,
    T: Send + 'static + Type<Turso>,
    for<'a> T: Decode<'a, Turso>,
    R: Send + Sync + 'static,
{
    let key = key.into();
    RowRead {
        read: Box::new(move |row, env, context| {
            Box::pin(async move {
                let index = match key.index(&row) {
                    Ok(index) => index,
                    Err(error) => return Exit::Fail(error),
                };
                match row.try_get_raw(index) {
                    Ok(value) if value.is_null() => {
                        return Exit::Fail(RowError::NullColumn(key.label()));
                    }
                    Err(source) => {
                        return Exit::Fail(RowError::Native {
                            column: key.label(),
                            source,
                        });
                    }
                    Ok(_) => {}
                }
                match row.try_get::<T, _>(index) {
                    Ok(value) => context
                        .run(codec.codec.decode_once(value), env)
                        .await
                        .map_error(|issue| RowError::Schema {
                            column: key.label(),
                            issue,
                        }),
                    Err(source) => Exit::Fail(RowError::Native {
                        column: key.label(),
                        source,
                    }),
                }
            })
        }),
    }
}
/// Read SQL NULL as None; missing rows/columns and native type errors still fail.
pub fn optional_column<A, T, R>(
    key: impl Into<ColumnKey>,
    codec: CellCodec<A, T, R>,
) -> RowRead<Option<A>, R>
where
    A: Send + 'static,
    T: Send + 'static + Type<Turso>,
    for<'a> T: Decode<'a, Turso>,
    R: Send + Sync + 'static,
{
    let key = key.into();
    RowRead {
        read: Box::new(move |row, env, context| {
            Box::pin(async move {
                let index = match key.index(&row) {
                    Ok(index) => index,
                    Err(error) => return Exit::Fail(error),
                };
                match row.try_get_raw(index) {
                    Ok(value) if value.is_null() => return Exit::Success(None),
                    Err(source) => {
                        return Exit::Fail(RowError::Native {
                            column: key.label(),
                            source,
                        });
                    }
                    Ok(_) => {}
                }
                match row.try_get::<T, _>(index) {
                    Ok(value) => match failure_as(
                        context
                            .run(codec.codec.decode_once(value), env)
                            .await
                            .map_error(|issue| RowError::Schema {
                                column: key.label(),
                                issue,
                            }),
                    ) {
                        Ok(value) => Exit::Success(Some(value)),
                        Err(exit) => exit,
                    },
                    Err(source) => Exit::Fail(RowError::Native {
                        column: key.label(),
                        source,
                    }),
                }
            })
        }),
    }
}
/// Decode a native Rust cell without a schema conversion; SQLx retains native type checks.
pub fn native_column<T, R>(key: impl Into<ColumnKey>) -> RowRead<T, R>
where
    T: Send + 'static + Type<Turso>,
    for<'a> T: Decode<'a, Turso>,
    R: Send + Sync + 'static,
{
    let key = key.into();
    RowRead {
        read: Box::new(move |row, _, _| {
            Box::pin(async move {
                let index = match key.index(&row) {
                    Ok(index) => index,
                    Err(error) => return Exit::Fail(error),
                };
                match row.try_get_raw(index) {
                    Ok(value) if value.is_null() => {
                        return Exit::Fail(RowError::NullColumn(key.label()));
                    }
                    Err(source) => {
                        return Exit::Fail(RowError::Native {
                            column: key.label(),
                            source,
                        });
                    }
                    Ok(_) => {}
                }
                match row.try_get::<T, _>(index) {
                    Ok(value) => Exit::Success(value),
                    Err(source) => Exit::Fail(RowError::Native {
                        column: key.label(),
                        source,
                    }),
                }
            })
        }),
    }
}

/// One live row-program invocation, carrying the original application environment/context.
pub struct RowContext<R> {
    /// Owned native row retained for this decoder only.
    row: Arc<TursoRow>,
    /// Application capabilities selected by the original Effect execution.
    env: Arc<R>,
    /// Existing interpreter context; no nested runtime is created.
    context: NativeContext,
    /// Core resource owner for effectful per-row validation bindings.
    scope: effect::Scope<RowError>,
}
/// Binding accepted within a row program: native reads or explicitly typed Effects.
pub trait RowStep<R: Send + Sync + 'static>: Send {
    /// Moved result of this read/validation step.
    type Output: Send + 'static;
    /// Evaluate while retaining the owned native row and original environment.
    fn evaluate(self, context: &RowContext<R>) -> RowFuture<'_, Self::Output>;
}
impl<A: Send + 'static, R: Send + Sync + 'static> RowStep<R> for RowRead<A, R> {
    /// Native decoded value; no Clone bound is introduced.
    type Output = A;
    /// Consume one pending native read under the current row owner.
    fn evaluate(self, context: &RowContext<R>) -> RowFuture<'_, A> {
        (self.read)(
            context.row.clone(),
            context.env.clone(),
            context.context.clone(),
        )
    }
}
impl<A: Send + 'static, R: Send + Sync + 'static> RowStep<R> for Effect<A, RowError, R> {
    /// Typed validation output.
    type Output = A;
    /// Reuse the existing Effect interpreter for effectful validation.
    fn evaluate(self, context: &RowContext<R>) -> RowFuture<'_, A> {
        Box::pin(context.context.run(self, context.env.clone()))
    }
}
impl<T: Send + Sync + 'static, R: Send + Sync + 'static> RowStep<R>
    for effect::ResourceBinding<T, RowError, R>
{
    /// Core resource access remains fenced to this row's validation scope.
    type Output = effect::ResourceValue<T>;
    /// Acquire through the existing sealed core binder and await finalizers when row decoding settles.
    fn evaluate(self, context: &RowContext<R>) -> RowFuture<'_, Self::Output> {
        use effect::__private::AsyncBindable;
        Box::pin(context.context.run(
            self.into_scoped_effect(context.scope.clone()),
            context.env.clone(),
        ))
    }
}
impl<R: Send + Sync + 'static> RowContext<R> {
    /// Bind one native read or Effect validation, retaining all failure channels.
    #[allow(
        clippy::result_large_err,
        reason = "The shared row lowering returns complete typed Exit values; boxing would change that ownership contract and allocate for failures"
    )]
    pub async fn bind<S: RowStep<R>, A: Send + 'static>(
        &self,
        step: S,
    ) -> Result<S::Output, Exit<A, RowError>> {
        if self.context.cancellation().is_requested() {
            return Err(Exit::Interrupted);
        }
        failure_as(step.evaluate(self).await)
    }
    /// Observe cooperative cancellation between row-program loop iterations.
    #[allow(
        clippy::result_large_err,
        reason = "Cancellation uses the same complete typed Exit contract as row binds without an additional failure allocation"
    )]
    pub async fn cooperate<A: Send + 'static>(&self) -> Result<(), Exit<A, RowError>> {
        if self.context.cancellation().is_requested() {
            Err(Exit::Interrupted)
        } else {
            Ok(())
        }
    }
}
/// Convert only success to a value; defects/interruption/finalization remain typed exits.
pub(crate) fn failure_as<A, B, E>(exit: Exit<A, E>) -> Result<A, Exit<B, E>> {
    match exit {
        Exit::Success(value) => Ok(value),
        Exit::Fail(error) => Err(Exit::Fail(error)),
        Exit::Die(defect) => Err(Exit::Die(defect)),
        Exit::Interrupted => Err(Exit::Interrupted),
        Exit::FinalizationFailed(details) => Err(Exit::FinalizationFailed(details)),
    }
}

/// Reusable row decoder description. Each invocation moves one native row into its own program.
#[must_use]
pub struct RowDecoder<A: Send + 'static, R: Send + Sync + 'static = ()> {
    /// Lazy program factory; shared handles may be explicitly retained by the caller.
    run: Arc<RowProgram<A, R>>,
}
impl<A: Send + 'static, R: Send + Sync + 'static> Clone for RowDecoder<A, R> {
    /// Share decoder configuration without copying rows or output payloads.
    fn clone(&self) -> Self {
        Self {
            run: self.run.clone(),
        }
    }
}
impl<A: Send + 'static, R: Send + Sync + 'static> RowDecoder<A, R> {
    /// Construct a D05 row-program carrier. The future is fresh for each row, with no borrowed escape.
    pub fn from_program<F>(factory: impl Fn(RowContext<R>) -> F + Send + Sync + 'static) -> Self
    where
        F: Future<Output = Result<A, Exit<A, RowError>>> + Send + 'static,
    {
        Self {
            run: Arc::new(move |context| {
                let pending = factory(context);
                Box::pin(async move {
                    match pending.await {
                        Ok(value) => Exit::Success(value),
                        Err(exit) => exit,
                    }
                })
            }),
        }
    }
    /// Adapt an extracted native projection returning typed row errors.
    pub fn pure(
        project: impl Fn(&TursoRow) -> Result<A, RowError> + Send + Sync + 'static,
    ) -> Self {
        let project = Arc::new(project);
        Self::from_program(move |context| {
            let project = project.clone();
            async move { project(&context.row).map_err(Exit::Fail) }
        })
    }
    /// Decode under the outer native bridge, preserving row failures and application capabilities.
    pub async fn decode(
        &self,
        row: TursoRow,
        env: Arc<R>,
        context: NativeContext,
    ) -> Exit<A, RowError> {
        let run = self.run.clone();
        let row = Arc::new(row);
        let program = effect::Scope::scoped(move |scope| {
            let run = run.clone();
            let row = row.clone();
            Effect::from_exit_future_with_context(move |env, context| {
                (run)(RowContext {
                    row: row.clone(),
                    env,
                    context,
                    scope: scope.clone(),
                })
            })
        });
        context.run(program, env).await
    }
}

/// Defer a native refinement, retaining a structured issue rather than coercing it into absence.
pub fn refine<A: Send + 'static, R: Send + Sync + 'static>(
    value: A,
    check: impl FnOnce(&A) -> Result<(), effect::SchemaIssue> + Send + 'static,
    column: impl Into<String>,
) -> RowRead<A, R> {
    let column = column.into();
    RowRead {
        read: Box::new(move |_, _, _| {
            Box::pin(async move {
                match check(&value) {
                    Ok(()) => Exit::Success(value),
                    Err(issue) => Exit::Fail(RowError::Schema { column, issue }),
                }
            })
        }),
    }
}

impl<T: Send + 'static, R: Send + Sync + 'static> CellCodec<T, T, R> {
    /// Keep a native SQLx value in its Rust representation, moving both codec directions without Clone.
    pub fn identity() -> Self {
        Self::new(Codec::new(
            |value| Effect::sync_once(move || value),
            |value| Effect::sync_once(move || value),
        ))
    }
}
impl<R: Send + Sync + 'static> CellCodec<bool, i64, R> {
    /// Decode only SQLite/Turso boolean-domain integers 0 and 1; arbitrary nonzero integers fail.
    /// This stricter domain codec coexists with SQLx's native permissive bool interoperability.
    pub fn boolean() -> Self {
        Self::new(Codec::new(
            |value| {
                Effect::from_result_once(match value {
                    0 => Ok(false),
                    1 => Ok(true),
                    _ => Err(effect::SchemaIssue::new(
                        effect::SchemaIssueKind::OutOfRange,
                    )),
                })
            },
            |value| Effect::sync_once(move || i64::from(value)),
        ))
    }
}
impl<R: Send + Sync + 'static> CellCodec<f64, f64, R> {
    /// Reject non-finite native reals in both directions without accepting a string/JSON detour.
    pub fn finite_real() -> Self {
        Self::new(Codec::new(
            |value: f64| {
                Effect::from_result_once(if value.is_finite() {
                    Ok(value)
                } else {
                    Err(effect::SchemaIssue::new(effect::SchemaIssueKind::NonFinite))
                })
            },
            |value: f64| {
                Effect::from_result_once(if value.is_finite() {
                    Ok(value)
                } else {
                    Err(effect::SchemaIssue::new(effect::SchemaIssueKind::NonFinite))
                })
            },
        ))
    }
}
impl<R: Send + Sync + 'static> CellCodec<effect::Decimal, String, R> {
    /// Decode invariant exact decimal text through the existing core Decimal representation.
    /// Grouping, exponent notation and non-finite text remain invalid; no numeric precision is lost.
    pub fn decimal_text() -> Self {
        Self::new(Codec::new(
            |value: String| {
                Effect::try_sync_once(
                    move || effect::Decimal::parse(&value),
                    |_| {
                        effect::SchemaIssue::new(effect::SchemaIssueKind::InvalidValue {
                            code: "sql.decimal_text".into(),
                        })
                    },
                )
            },
            |value: effect::Decimal| Effect::sync_once(move || value.as_str().to_owned()),
        ))
    }
}

/// Defer a typed consuming validator or projection within the live row owner.
/// An ordinary Result does not bind implicitly; this named adapter maps it explicitly to RowError.
pub fn validate<A: Send + 'static, B: Send + 'static, R: Send + Sync + 'static>(
    value: A,
    validator: impl FnOnce(A) -> Result<B, RowError> + Send + 'static,
) -> RowRead<B, R> {
    RowRead {
        read: Box::new(move |_, _, _| {
            Box::pin(async move {
                match validator(value) {
                    Ok(value) => Exit::Success(value),
                    Err(error) => Exit::Fail(error),
                }
            })
        }),
    }
}
/// Match native text ordinally against a nonempty unique domain choice list, moving its selected value.
/// Build the list per decoder invocation or supply explicit shared configuration/factories; no value Clone is required.
pub fn choice_text<A: Send + 'static, R: Send + Sync + 'static>(
    column: impl Into<String>,
    choices: Vec<(String, A)>,
    value: String,
) -> RowRead<A, R> {
    let column = column.into();
    RowRead {
        read: Box::new(move |_, _, _| {
            Box::pin(async move {
                let mut names = std::collections::HashSet::new();
                if choices.is_empty()
                    || choices.iter().any(|(name, _)| !names.insert(name.as_str()))
                {
                    return Exit::Fail(RowError::InvalidDecoder);
                }
                drop(names);
                match choices.into_iter().find(|(name, _)| name == &value) {
                    Some((_, value)) => Exit::Success(value),
                    None => {
                        Exit::Fail(RowError::Validation(format!("invalid choice for {column}")))
                    }
                }
            })
        }),
    }
}
