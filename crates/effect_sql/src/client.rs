//! Lazy Effect SQL adapters over the shared native async kernel.

use crate::{
    NativeSession, PreparedSqlRows, PreparedSqlRowsBuilder, PreparedSqlRowsPool, RowDecoder,
    SqlError, SqlKernel, SqlStartupError, SqlStatement,
};
use crate::{PreparedSqlBatch, PreparedSqlBatchItem, PreparedSqlRowLanes};
use crate::prepared_rows::ReservedNativeRows;
use crate::{
    effect_fail, effect_from_result, effect_future, effect_provide_some, effect_resource, effect_sync,
    row, sql,
};
use effect::__private::prepared_graph::{Description, Environment, FutureContext};
use effect::memory::{MemoryError, MemoryPolicy};
use effect::{
    CleanupFailure, Defect, Effect, Exit, Finalization, NativeContext, PrimaryOutcome, effect,
    service,
};
use effect::{sink, stream};
use futures_util::FutureExt;
use sqlx::{Decode, Row, Turso, TursoPool, Type};
use std::{
    borrow::Borrow,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
};

trait SqlCancellation {
    fn is_requested(&self) -> bool;
}
impl SqlCancellation for NativeContext {
    fn is_requested(&self) -> bool {
        self.cancellation().is_requested()
    }
}
impl SqlCancellation for effect::__private::prepared_graph::FutureContext<SqlError> {
    fn is_requested(&self) -> bool {
        effect::__private::prepared_graph::FutureContext::is_requested(self)
    }
}
impl<T: SqlCancellation + ?Sized> SqlCancellation for &T {
    fn is_requested(&self) -> bool { (**self).is_requested() }
}
impl<T: SqlCancellation + ?Sized> SqlCancellation for &mut T {
    fn is_requested(&self) -> bool { (**self).is_requested() }
}

/// SQL capability selected by an application's sole environment type R.
pub trait HasSqlClient {
    /// Retain the selected pool/service handle, without opening a connection during projection.
    fn sql_client(&self) -> &SqlClient;
}

/// Type-level distinction for a fresh factory, inferred by statement-taking methods.
#[doc(hidden)]
pub struct FreshStatement;
/// Type-level distinction for a transferred statement or construction Result.
#[doc(hidden)]
pub struct OwnedStatement;

/// Borrowed SQL cell representation copied into preadmitted byte buffers.
#[derive(Clone, Copy, Debug)]
pub enum PreparedSqlBytesKind {
    /// Decode UTF-8 text and retain its bytes.
    Text,
    /// Decode binary data.
    Blob,
}

/// Actual outer and cursor future/completion frame layouts for a prepared
/// atomic SQL operation. Each pending/completion pair can be live simultaneously.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSqlFrameDemand {
    /// Outer contextual future frame.
    pub outer_pending: std::alloc::Layout,
    /// Outer completion transition frame.
    pub outer_completion: std::alloc::Layout,
    /// Nested cursor future frame.
    pub cursor_pending: std::alloc::Layout,
    /// Nested cursor completion transition frame.
    pub cursor_completion: std::alloc::Layout,
    /// Nested interpreter outcome frame.
    pub cursor_output: std::alloc::Layout,
    /// Nested interpreter terminal store frame.
    pub cursor_store: std::alloc::Layout,
    /// Nested operation entry frame.
    pub cursor_entry: std::alloc::Layout,
    /// Nested context entry and restore frame layouts.
    pub cursor_instrumentation: effect::memory::PreparedContextualDemand,
}

impl PreparedSqlFrameDemand {
    fn new(
        (outer_pending, outer_completion): (std::alloc::Layout, std::alloc::Layout),
        cursor: effect::PreparedContextualFutureLayouts,
    ) -> Self {
        Self {
            outer_pending,
            outer_completion,
            cursor_pending: cursor.pending,
            cursor_completion: cursor.completion,
            cursor_output: cursor.output,
            cursor_store: cursor.store,
            cursor_entry: cursor.entry,
            cursor_instrumentation: cursor.instrumentation,
        }
    }

    /// Maximum simultaneously live work frames for this operation.
    #[must_use]
    pub const fn work_frames(&self) -> usize {
        11
    }

    /// Smallest uniform frame layout that accommodates each reported frame.
    /// Transitive pointees and native provider allocations are separate.
    pub fn maximum_layout(&self) -> Result<std::alloc::Layout, effect::memory::MemoryError> {
        let context = self.cursor_instrumentation;
        let layouts = [
            self.outer_pending,
            self.outer_completion,
            self.cursor_pending,
            self.cursor_completion,
            self.cursor_output,
            self.cursor_store,
            self.cursor_entry,
            context.entry_tag,
            context.restore,
            context.invoke,
            context.restore_tag,
        ];
        let size = layouts.iter().map(std::alloc::Layout::size).max().unwrap_or(0);
        let align = layouts.iter().map(std::alloc::Layout::align).max().unwrap_or(1);
        std::alloc::Layout::from_size_align(size, align)
            .map(|layout| layout.pad_to_align())
            .map_err(|_| effect::memory::MemoryError::InvalidLayout)
    }
}

/// Atomic batch Effect and its exact outer and cursor frame requirements.
pub type PreparedSqlBatchEffect<A, R> = (
    Effect<PreparedSqlRows<PreparedSqlBatchItem<A>>, SqlError, R>,
    PreparedSqlFrameDemand,
);

fn contextual_sql<A, R, F, P>(
    factory: F,
    prepared: bool,
) -> (Effect<A, SqlError, R>, (std::alloc::Layout, std::alloc::Layout))
where
    A: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(Arc<R>, NativeContext) -> P + Send + Sync + 'static,
    P: Future<Output = Exit<A, SqlError>> + Send + 'static,
{
    let layouts = Effect::<A, SqlError, R>::prepared_exit_future_with_context_layouts(&factory);
    let program = if prepared {
        effect_future! { exit_with_context; body: factory, on_capacity: SqlError::Memory, }
    } else {
        effect_future! { exit_with_context; body: factory, }
    };
    (program, layouts)
}
enum AtomicBatchInput {
    Ordinary(Mutex<Option<PreparedSqlBatch>>),
    Selected(Arc<Mutex<Option<PreparedSqlBatch>>>),
}
impl AtomicBatchInput {
    fn new(batch: PreparedSqlBatch, selected: bool) -> Self {
        if selected { Self::Selected(Arc::new(Mutex::new(Some(batch)))) }
        else { Self::Ordinary(Mutex::new(Some(batch))) }
    }
    fn get(&self) -> &Mutex<Option<PreparedSqlBatch>> {
        match self { Self::Ordinary(input) => input, Self::Selected(input) => input }
    }
    fn selected(&self) -> Option<Arc<Mutex<Option<PreparedSqlBatch>>>> {
        match self { Self::Selected(input) => Some(Arc::clone(input)), Self::Ordinary(_) => None }
    }
}

/// Inert statement input: a fresh factory is reusable; an owned statement/Result transfers once.
/// This boundary never clones bindings or application payloads. Its first validation
/// failure remains the original SQL error, and later attempts report consumption.
pub trait StatementSource<Kind>: Send + 'static {
    /// Prepare a lazy factory without executing SQL or evaluating a caller factory.
    #[must_use = "the returned factory is inert until a SQL operation invokes it"]
    fn into_factory(self) -> impl Fn() -> Result<SqlStatement, SqlError> + Send + Sync + 'static;
}
impl<F> StatementSource<FreshStatement> for F
where
    F: Fn() -> Result<SqlStatement, SqlError> + Send + Sync + 'static,
{
    /// Retain a reusable factory; it supplies a fresh native argument owner per execution.
    fn into_factory(self) -> impl Fn() -> Result<SqlStatement, SqlError> + Send + Sync + 'static {
        self
    }
}
impl StatementSource<OwnedStatement> for SqlStatement {
    /// Retain one moved statement behind checked one-use ownership.
    fn into_factory(self) -> impl Fn() -> Result<SqlStatement, SqlError> + Send + Sync + 'static {
        one_statement(self)
    }
}
impl StatementSource<OwnedStatement> for Result<SqlStatement, SqlError> {
    /// Retain either one moved statement or its original inert validation failure.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    fn into_factory(self) -> impl Fn() -> Result<SqlStatement, SqlError> + Send + Sync + 'static {
        let input = Mutex::new(Some(self));
        move || {
            input
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .ok_or(SqlError::Consumed)?
        }
    }
}
impl StatementSource<OwnedStatement> for crate::PreparedSqlStatement {
    fn into_factory(self) -> impl Fn() -> Result<SqlStatement, SqlError> + Send + Sync + 'static {
        prepared_statement_factory(Ok(self))
    }
}
impl StatementSource<OwnedStatement>
    for Result<crate::PreparedSqlStatement, crate::PreparedSqlStatementError>
{
    fn into_factory(self) -> impl Fn() -> Result<SqlStatement, SqlError> + Send + Sync + 'static {
        prepared_statement_factory(self)
    }
}
#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
fn prepared_statement_factory(
    input: Result<crate::PreparedSqlStatement, crate::PreparedSqlStatementError>,
) -> impl Fn() -> Result<SqlStatement, SqlError> + Send + Sync + 'static {
    let input = Mutex::new(Some(input));
    move || {
        let mut pending = crate::try_prepared_lock(&input).map_err(SqlError::Memory)?;
        let charge = match pending.as_ref() {
            Some(Ok(statement)) => statement.reserve_materialization()?,
            _ => None,
        };
        let input = pending.take();
        drop(pending);
        match input {
            Some(Ok(statement)) => {
                let mut native = statement.materialize_reserved(charge)?;
                native.retention.prepared_source = Some(statement);
                Ok(native)
            }
            Some(Err(crate::PreparedSqlStatementError::Memory(error))) => {
                Err(SqlError::Memory(error))
            }
            Some(Err(error)) => Err(SqlError::PreparedStatement(error)),
            None => Err(SqlError::Consumed),
        }
    }
}

/// Immutable SQL service configuration; each run acquires a fresh native execution owner.
#[derive(Clone, crate::EffectEnvironment)]
#[effect_environment(sql_client = self)]
#[must_use]
pub struct SqlClient {
    /// Shared native kernel; cloned clients share configuration, not leases.
    kernel: SqlKernel,
}
impl SqlClient {
    /// Select a caller-owned local SQLx pool. Construction is inert and never creates a runtime.
    pub fn new(pool: TursoPool) -> Self {
        Self {
            kernel: SqlKernel::new(pool),
        }
    }
    /// Validate the SQLx/Turso native domain before acquiring a pool.
    /// `None` selects ordinary provider-managed allocation. Either strict
    /// memory policy requires a verified native VM byte envelope.
    #[expect(clippy::result_large_err, reason = "preserves the owned SQL startup error")]
    pub fn validate_native_memory(policy: Option<MemoryPolicy>) -> Result<(), SqlStartupError> {
        match policy {
            None => Ok(()),
            Some(policy) => Err(SqlStartupError::MissingNativeMemoryEnvelope(policy)),
        }
    }

    /// Validate native memory selection before invoking the caller's pool factory.
    /// The factory runs once on successful validation. The returned client owns
    /// that pool through the ordinary SQL kernel.
    #[expect(clippy::result_large_err, reason = "preserves the owned SQL startup error")]
    pub async fn open<F>(
        native_memory: Option<MemoryPolicy>,
        acquire_pool: impl FnOnce() -> F,
    ) -> Result<Self, SqlStartupError>
    where
        F: Future<Output = Result<TursoPool, SqlError>>,
    {
        Self::validate_native_memory(native_memory)?;
        Ok(Self::new(acquire_pool().await?))
    }

    /// Select an existing pool for prepared graph SQL cursor and settlement
    /// diagnostics. Root transactions preclaim six slots and nested savepoints
    /// preclaim three, separately from the graph's own panic demand.
    pub fn with_prepared_panic_diagnostics(mut self, pool: Arc<effect::PreparedPanicPool>) -> Self {
        self.kernel = self.kernel.with_prepared_panic_diagnostics(pool);
        self
    }

    fn reserve_transaction_panics(
        &self,
    ) -> Result<Option<crate::kernel::TransactionPanicReservations>, effect::memory::MemoryError> {
        let Some(pool) = self.kernel.prepared_panic_diagnostics() else {
            return Ok(None);
        };
        let [cursor, begin, transition, close_rollback, close_settle, close_dispose] =
            pool.try_reserve_group()?;
        Ok(Some(crate::kernel::TransactionPanicReservations {
            cursor,
            begin,
            transition,
            close_rollback,
            close_settle,
            close_dispose,
        }))
    }
    /// Select prepared retained rows for incremental cursors.
    pub fn with_rows_pool(mut self, pool: Arc<sqlx::PreparedTursoRowsPool>) -> Self {
        self.kernel = self.kernel.with_rows_pool(pool);
        self
    }
    /// Select exclusive preprepared driver registries for incremental cursors.
    pub fn with_row_lanes(mut self, lanes: Arc<PreparedSqlRowLanes>) -> Self {
        self.kernel = self.kernel.with_row_lanes(lanes);
        self
    }
    /// Select the prepared controls used by new native sessions.
    pub fn with_session_pool(mut self, pool: Arc<crate::PreparedSqlSessionPool>) -> Self {
        self.kernel = self.kernel.with_session_pool(pool);
        self
    }
    /// Select a shared, nonblocking work-slot limit for pending SQLx acquisitions.
    pub fn with_pending_checkout_admission(
        mut self,
        admission: Arc<effect::memory::Admission<'static, 1>>,
    ) -> Result<Self, effect::memory::MemoryError> {
        self.kernel = self.kernel.with_pending_checkout_admission(admission)?;
        Ok(self)
    }
    /// Retain the shared native kernel for Identity store trait calls and checked SQLx use.
    #[must_use]
    pub fn kernel(&self) -> &SqlKernel {
        &self.kernel
    }
    /// Execute with an explicit transaction and a preclaimed fixed output.
    /// Native driver/parser/row and returned-value pointee allocations remain separate.
    pub fn query_into_rows_atomic<M, A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
        rows: Arc<PreparedSqlRowsPool<A>>,
    ) -> Effect<PreparedSqlRows<A>, SqlError, R> {
        self.query_into_rows_atomic_selected(statement, decoder, rows)
    }

    /// Build a one-use prepared graph description for an atomic decoded query.
    /// The actual environment is borrowed only during each synchronous decoder
    /// projection; native provider and decoded pointee allocations remain separate.
    pub fn query_into_rows_atomic_prepared<M, A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
        rows: Arc<PreparedSqlRowsPool<A>>,
    ) -> impl effect::__private::prepared_graph::Description<
        Value = PreparedSqlRows<A>,
        Error = SqlError,
        Env = R,
    > {
        let client = self.clone();
        let statement = statement.into_factory();
        effect::effect_prepare! { description; program: effect::effect_future! { exit_with_context; body: move |environment: effect::__private::prepared_graph::Environment<R>, mut context: effect::__private::prepared_graph::FutureContext<SqlError>| async move {
                    if context.is_requested() { return Exit::Interrupted; }
                    if !decoder.has_borrowed_decoder_with_env() { return Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder)); }
                    let connection = match acquire_transaction_connection(&client.kernel, &mut context).await { Ok(value) => value, Err(exit) => return exit };
                    let first = match client.kernel.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() };
                    let panics = match client.reserve_transaction_panics() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let builder = match rows.reserve() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let reservation = match client.kernel.reserve_optional_session() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let cleanup = match client.kernel.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let statement = match statement() { Ok(statement) => statement, Err(error) => return error.into_exit() };
                    let body = PreparedTransactionQuery {
                        statement, destinations: PreparedRowsDestination { rows: builder, decoder, environment }, first,
                    };
                    run_prepared_transaction(client.kernel, reservation, connection, cleanup, panics, context, body).await
                } } }
    }

    /// Construct the atomic query with outer and cursor futures in selected
    /// run frames, returning their exact layouts.
    pub fn query_into_rows_atomic_with_frames<M, A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
        rows: Arc<PreparedSqlRowsPool<A>>,
    ) -> (
        Effect<PreparedSqlRows<A>, SqlError, R>,
        PreparedSqlFrameDemand,
    ) {
        let client = self.clone();
        let statement = Arc::new(statement.into_factory());
        let cursor_layouts = query_cursor_layouts(query_cursor::<R, RowsDestination<A, R>>);
        let factory = move |env: Arc<R>, context: NativeContext| {
            let client = client.clone();
            let statement = statement.clone();
            let decoder = decoder.clone();
            let rows = rows.clone();
            async move {
                match AssertUnwindSafe(async move {
                    if context.cancellation().is_requested() { return Exit::Interrupted; }
                    if !decoder.has_borrowed_decoder_with_env() { return Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder)); }
                    let connection = match acquire_transaction_connection(&client.kernel, &context).await { Ok(value) => value, Err(exit) => return exit };
                    let first = match client.kernel.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() };
                    let cursor_frames = match reserve_query_cursor(&context, query_cursor::<R, RowsDestination<A, R>>) { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let builder = match rows.reserve() { Ok(builder) => builder, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let reservation = match client.kernel.reserve_optional_callback_session() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let cleanup = match client.kernel.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let statement = match statement() { Ok(statement) => statement, Err(error) => return error.into_exit() };
                    run_transaction_async(client.kernel, reservation, Some(connection), cleanup, env, context, move |tx, env, _| async move {
                        let input = QueryCursorInput { session: tx.session, statement, destinations: RowsDestination { rows: builder, decoder }, first };
                        cursor_frames.run(input, env, query_cursor::<R, _>).await
                    }).await
                }).catch_unwind().await {
                    Ok(outcome) => outcome,
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                }
            }
        };
        let (program, outer_layouts) = contextual_sql(factory, true);
        (program, PreparedSqlFrameDemand::new(outer_layouts, cursor_layouts))
    }

    fn query_into_rows_atomic_selected<M, A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
        rows: Arc<PreparedSqlRowsPool<A>>,
    ) -> Effect<PreparedSqlRows<A>, SqlError, R> {
        let client = self.clone();
        let statement = Arc::new(statement.into_factory());
        let factory = move |env: Arc<R>, context: NativeContext| {
        let client = client.clone();
        let statement = statement.clone();
        let decoder = decoder.clone();
        let rows = rows.clone();
        async move {
            match AssertUnwindSafe(async move {
                if context.cancellation().is_requested() { return Exit::Interrupted; }
                if !decoder.has_borrowed_decoder_with_env() { return Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder)); }
                let connection = match acquire_prepared_connection(&client.kernel, &context).await { Ok(value) => value, Err(exit) => return exit };
                let builder = match rows.reserve() { Ok(builder) => builder, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let reservation = match client.kernel.reserve_optional_callback_session() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let first = match client.kernel.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() };
                let cleanup = match client.kernel.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let statement = match statement() { Ok(statement) => statement, Err(error) => return error.into_exit() };
                run_transaction(client.kernel, reservation, connection, cleanup, env, context, move |tx| session_query_rows_once(tx.session, statement, builder, decoder, first)).await
            }).catch_unwind().await {
                Ok(outcome) => outcome,
                Err(panic) => Exit::Die(Defect::from_panic(panic)),
            }
        }
    };
    contextual_sql(factory, false).0
}
/// Execute atomically and retain one prepared buffer per decoded text/blob row.
/// All destinations are claimed before statement construction or transaction begin.
pub fn query_bytes_into_rows_atomic<M, R: Send + Sync + 'static>(
    &self,
    statement: impl StatementSource<M>,
    column: crate::ColumnKey,
    kind: PreparedSqlBytesKind,
    rows: Arc<PreparedSqlRowsPool<effect::memory::BufferLease<'static>>>,
    buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
) -> Effect<PreparedSqlRows<effect::memory::BufferLease<'static>>, SqlError, R> {
    self.query_bytes_into_rows_atomic_selected(statement, column, kind, rows, buffers)
}

    /// Build a one-use prepared graph description for an atomic text/blob
    /// query. The graph owns its execution frames; SQLx, native VM and caller
    /// pointee allocations remain separate from that graph backing.
    pub fn query_bytes_into_rows_atomic_prepared<M, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
        column: crate::ColumnKey,
        kind: PreparedSqlBytesKind,
        rows: Arc<PreparedSqlRowsPool<effect::memory::BufferLease<'static>>>,
        buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
    ) -> impl effect::__private::prepared_graph::Description<
        Value = PreparedSqlRows<effect::memory::BufferLease<'static>>,
        Error = SqlError,
        Env = R,
    > {
        let client = self.clone();
        let statement = statement.into_factory();
        effect::effect_prepare! { description; program: effect::effect_future! { exit_with_context; body: move |_environment: effect::__private::prepared_graph::Environment<R>, mut context: effect::__private::prepared_graph::FutureContext<SqlError>| async move {
                    if context.is_requested() { return Exit::Interrupted; }
                    let connection = match acquire_transaction_connection(&client.kernel, &mut context).await { Ok(value) => value, Err(exit) => return exit };
                    let first = match client.kernel.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() };
                    let panics = match client.reserve_transaction_panics() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let destinations = match PreparedBytesDestinations::reserve(&rows, &buffers) { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let reservation = match client.kernel.reserve_optional_session() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let cleanup = match client.kernel.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let statement = match statement() { Ok(statement) => statement, Err(error) => return error.into_exit() };
                    let body = PreparedTransactionQuery {
                        statement, destinations: BytesDestination { destinations, column, kind }, first,
                    };
                    run_prepared_transaction(client.kernel, reservation, connection, cleanup, panics, context, body).await
                } } }
    }

/// Construct the atomic text/blob query with both outer and cursor future
/// frames selected explicitly. Native provider and interpreter allocations
/// beyond those frames remain separate.
pub fn query_bytes_into_rows_atomic_with_frames<M, R: Send + Sync + 'static>(
    &self,
    statement: impl StatementSource<M>,
    column: crate::ColumnKey,
    kind: PreparedSqlBytesKind,
    rows: Arc<PreparedSqlRowsPool<effect::memory::BufferLease<'static>>>,
    buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
) -> (
    Effect<PreparedSqlRows<effect::memory::BufferLease<'static>>, SqlError, R>,
    PreparedSqlFrameDemand,
) {
    let client = self.clone();
    let statement = Arc::new(statement.into_factory());
    let column = Arc::new(column);
    let cursor_layouts = query_cursor_layouts(query_cursor::<R, BytesDestination>);
    let factory = move |env: Arc<R>, context: NativeContext| {
        let client = client.clone();
        let statement = statement.clone();
        let column = column.clone();
        let rows = rows.clone();
        let buffers = buffers.clone();
        async move {
            match AssertUnwindSafe(async move {
                if context.cancellation().is_requested() { return Exit::Interrupted; }
                let connection = match acquire_transaction_connection(&client.kernel, &context).await { Ok(value) => value, Err(exit) => return exit };
                let first = match client.kernel.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() };
                let cursor_frames = match reserve_query_cursor(&context, query_cursor::<R, BytesDestination>) { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let destinations = match PreparedBytesDestinations::reserve(&rows, &buffers) { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let reservation = match client.kernel.reserve_optional_callback_session() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let cleanup = match client.kernel.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let statement = match statement() { Ok(statement) => statement, Err(error) => return error.into_exit() };
                run_transaction_async(client.kernel, reservation, Some(connection), cleanup, env, context, move |tx, env, _| async move {
                    let input = QueryCursorInput { session: tx.session, statement, destinations: BytesDestination { destinations, column, kind }, first };
                    cursor_frames.run(input, env, query_cursor::<R, _>).await
                }).await
            }).catch_unwind().await {
                Ok(outcome) => outcome,
                Err(panic) => Exit::Die(Defect::from_panic(panic)),
            }
        }
    };
    let (program, outer_layouts) = contextual_sql(factory, true);
    (program, PreparedSqlFrameDemand::new(outer_layouts, cursor_layouts))
}

fn query_bytes_into_rows_atomic_selected<M, R: Send + Sync + 'static>(
    &self,
    statement: impl StatementSource<M>,
    column: crate::ColumnKey,
    kind: PreparedSqlBytesKind,
    rows: Arc<PreparedSqlRowsPool<effect::memory::BufferLease<'static>>>,
    buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
) -> Effect<PreparedSqlRows<effect::memory::BufferLease<'static>>, SqlError, R> {
    let client = self.clone();
    let statement = Arc::new(statement.into_factory());
    let column = Arc::new(column);
    let factory = move |env: Arc<R>, context: NativeContext| {
        let client = client.clone();
        let statement = statement.clone();
        let column = column.clone();
        let rows = rows.clone();
        let buffers = buffers.clone();
        async move {
            match AssertUnwindSafe(async move {
                if context.cancellation().is_requested() { return Exit::Interrupted; }
                let connection = match acquire_prepared_connection(&client.kernel, &context).await { Ok(value) => value, Err(exit) => return exit };
                let destinations = match PreparedBytesDestinations::reserve(&rows, &buffers) { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let reservation = match client.kernel.reserve_optional_callback_session() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let first = match client.kernel.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() };
                let cleanup = match client.kernel.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let statement = match statement() { Ok(statement) => statement, Err(error) => return error.into_exit() };
                run_transaction(client.kernel, reservation, connection, cleanup, env, context, move |tx| session_query_bytes_once(tx.session, statement, destinations, column, kind, first)).await
            }).catch_unwind().await {
                Ok(outcome) => outcome,
                Err(panic) => Exit::Die(Defect::from_panic(panic)),
            }
        }
    };
    contextual_sql(factory, false).0
}
/// Run an owned batch in one explicit transaction. The output capacity
/// counts both decoded rows and statement-result metadata items.
pub fn batch_into_rows_atomic<A: Send + 'static, R: Send + Sync + 'static>(
    &self,
    batch: PreparedSqlBatch,
    decoder: RowDecoder<A, R>,
    rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<A>>>,
) -> Effect<PreparedSqlRows<PreparedSqlBatchItem<A>>, SqlError, R> {
    self.batch_into_rows_atomic_selected(batch, decoder, rows)
}

/// Build a one-use prepared graph description for an owned decoded batch.
/// Publish the batch through a prepared graph input after admission.
#[must_use]
pub fn batch_into_rows_atomic_prepared<A: Send + 'static, R: Send + Sync + 'static>(
    &self,
    mut batch: PreparedSqlBatch,
    decoder: RowDecoder<A, R>,
    rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<A>>>,
) -> impl effect::__private::prepared_graph::Description<
    Value = PreparedSqlRows<PreparedSqlBatchItem<A>>,
    Error = SqlError,
    Env = R,
> + use<A, R> {
    let client = self.clone();
    effect::effect_prepare! { description; program: effect::effect_future! { exit_with_context; body: move |environment: effect::__private::prepared_graph::Environment<R>, mut context: effect::__private::prepared_graph::FutureContext<SqlError>| async move {
                if context.is_requested() { return Exit::Interrupted; }
                if !decoder.has_borrowed_decoder_with_env() { return Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder)); }
                let connection = match acquire_transaction_connection(&client.kernel, &mut context).await { Ok(value) => value, Err(exit) => return exit };
                let first = if batch.is_empty() { None } else { match client.kernel.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() } };
                let panics = match client.reserve_transaction_panics() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let builder = match rows.reserve() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let reservation = match client.kernel.reserve_optional_session() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let cleanup = match client.kernel.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                if batch.len() > builder.capacity() { return Exit::Fail(SqlError::RowLimit { limit: builder.capacity() }); }
                if let Err(error) = batch.materialize_all() { return error.into_exit(); }
                let destination = PreparedBatchRowsDestination { rows: builder, decoder, environment };
                let body = PreparedTransactionBatch { batch, destination, first };
                run_prepared_transaction(client.kernel, reservation, connection, cleanup, panics, context, body).await
            } } }
}

/// Construct the atomic batch with its outer future and completion in
/// selected run frames, returning their exact layouts.
pub fn batch_into_rows_atomic_with_frames<A: Send + 'static, R: Send + Sync + 'static>(
    &self,
    batch: PreparedSqlBatch,
    decoder: RowDecoder<A, R>,
    rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<A>>>,
) -> PreparedSqlBatchEffect<A, R> {
    let client = self.clone();
    let batch = Arc::new(Mutex::new(Some(batch)));
    let cursor_layouts = batch_cursor_layouts(batch_cursor::<R, BatchRowsDestination<A, R>>);
    let factory = move |env: Arc<R>, context: NativeContext| {
        let client = client.clone();
        let decoder = decoder.clone();
        let batch = batch.clone();
        let rows = rows.clone();
        async move {
            if context.cancellation().is_requested() { return Exit::Interrupted; }
            if !decoder.has_borrowed_decoder_with_env() {
                return Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder));
            }
            if let Err(error) = batch_needs_row(&batch) { return error.into_exit(); }
            let connection = match acquire_transaction_connection(&client.kernel, &context).await {
                Ok(connection) => connection,
                Err(exit) => return exit,
            };
            let first = match reserve_batch_first_row(&batch, &client.kernel) {
                Ok(first) => first,
                Err(error) => return error.into_exit(),
            };
            #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
            let input = (|| {
                let frames = reserve_batch_cursor(&context, batch_cursor::<R, BatchRowsDestination<A, R>>).map_err(SqlError::Memory)?;
                let builder = rows.reserve().map_err(SqlError::Memory)?;
                let reservation = client.kernel.reserve_optional_callback_session().map_err(SqlError::Memory)?;
                let cleanup = client.kernel.reserve_optional_cleanup().map_err(SqlError::Memory)?;
                let (batch, builder) = take_batch_input(&batch, builder)?;
                Ok::<_, SqlError>((frames, batch, builder, reservation, cleanup))
            })();
            let (frames, batch, builder, reservation, cleanup) = match input {
                Ok(input) => input,
                Err(error) => return error.into_exit(),
            };
            run_transaction_async(client.kernel, reservation, Some(connection), cleanup, env, context, move |tx, env, _| async move {
                let input = BatchCursorInput { session: tx.session, batch, destination: BatchRowsDestination { rows: builder, decoder }, first };
                frames.run(input, env, batch_cursor::<R, _>).await
            }).await
        }
    };
    let (program, outer_layouts) = contextual_sql(factory, true);
    (program, PreparedSqlFrameDemand::new(outer_layouts, cursor_layouts))
}

fn batch_into_rows_atomic_selected<A: Send + 'static, R: Send + Sync + 'static>(
    &self,
    batch: PreparedSqlBatch,
    decoder: RowDecoder<A, R>,
    rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<A>>>,
) -> Effect<PreparedSqlRows<PreparedSqlBatchItem<A>>, SqlError, R> {
    let client = self.clone();
    let batch = AtomicBatchInput::new(batch, client.kernel.early_checkout_configured());
    let factory = move |env: Arc<R>, context: NativeContext| {
            let prepared = if context.cancellation().is_requested() { Err(Exit::Interrupted) }
                else if !decoder.has_borrowed_decoder_with_env() { Err(Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder))) }
                else if batch.selected().is_some() { batch_needs_row(batch.get()).map(|_| None).map_err(Exit::Fail) }
                else { prepare_batch_input_with_session(batch.get(), &rows, &client.kernel).map(Some).map_err(Exit::Fail) };
            let client = client.clone();
            let decoder = decoder.clone();
            let selected = batch.selected();
            let rows = rows.clone();
            async move {
                let prepared = match prepared { Ok(value) => value, Err(exit) => return exit };
                let (connection, prepared) = if let Some(batch) = selected {
                    let connection = match acquire_prepared_connection(&client.kernel, &context).await { Ok(value) => value, Err(exit) => return exit };
                    let prepared = match prepare_batch_input_with_session(&batch, &rows, &client.kernel) { Ok(value) => value, Err(error) => return error.into_exit() };
                    (connection, prepared)
                } else {
                    let Some(prepared) = prepared else { return Exit::Fail(SqlError::Memory(effect::memory::MemoryError::InvalidLayout)); };
                    (None, prepared)
                };
                let (batch, builder, reservation, first, cleanup) = prepared;
                run_transaction(client.kernel, reservation, connection, cleanup, env, context, move |tx| session_batch_rows_once(tx.session, batch, builder, decoder, first)).await
            }
        };
        contextual_sql(factory, false).0
    }
    /// Atomically execute a batch with preadmitted text/blob row buffers.
    /// Result metadata consumes an output position and releases its unused buffer.
    pub fn batch_bytes_into_rows_atomic<R: Send + Sync + 'static>(
        &self, batch: PreparedSqlBatch, column: crate::ColumnKey, kind: PreparedSqlBytesKind,
        rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>>,
        buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
    ) -> Effect<PreparedSqlRows<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>, SqlError, R> {
        self.batch_bytes_into_rows_atomic_selected(batch, column, kind, rows, buffers)
    }

    /// Build a one-use prepared graph description for an owned text/blob batch.
    /// Publish the batch through a prepared graph input after admission.
    #[must_use]
    pub fn batch_bytes_into_rows_atomic_prepared<R: Send + Sync + 'static>(
        &self,
        mut batch: PreparedSqlBatch,
        column: crate::ColumnKey,
        kind: PreparedSqlBytesKind,
        rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>>,
        buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
    ) -> impl effect::__private::prepared_graph::Description<
        Value = PreparedSqlRows<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>,
        Error = SqlError,
        Env = R,
    > + use<R> {
        let client = self.clone();
        effect::effect_prepare! { description; program: effect::effect_future! { exit_with_context; body: move |_environment: effect::__private::prepared_graph::Environment<R>, mut context: effect::__private::prepared_graph::FutureContext<SqlError>| async move {
                    if context.is_requested() { return Exit::Interrupted; }
                    let connection = match acquire_transaction_connection(&client.kernel, &mut context).await { Ok(value) => value, Err(exit) => return exit };
                    let first = if batch.is_empty() { None } else { match client.kernel.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() } };
                    let panics = match client.reserve_transaction_panics() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let destination = match BatchBytesDestination::reserve(&rows, &buffers, column, kind) { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let reservation = match client.kernel.reserve_optional_session() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let cleanup = match client.kernel.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    if batch.len() > destination.rows.capacity() { return Exit::Fail(SqlError::RowLimit { limit: destination.rows.capacity() }); }
                    if let Err(error) = batch.materialize_all() { return error.into_exit(); }
                    let body = PreparedTransactionBatch { batch, destination, first };
                    run_prepared_transaction(client.kernel, reservation, connection, cleanup, panics, context, body).await
                } } }
    }

    /// Construct the atomic text/blob batch with its outer future and
    /// completion in selected run frames, returning their exact layouts.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub fn batch_bytes_into_rows_atomic_with_frames<R: Send + Sync + 'static>(
        &self, batch: PreparedSqlBatch, column: crate::ColumnKey, kind: PreparedSqlBytesKind,
        rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>>,
        buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
    ) -> PreparedSqlBatchEffect<effect::memory::BufferLease<'static>, R> {
        let client = self.clone();
        let batch = Arc::new(Mutex::new(Some(batch)));
        let column = Arc::new(column);
        let cursor_layouts = batch_cursor_layouts(batch_cursor::<R, BatchBytesDestination>);
        let factory = move |env: Arc<R>, context: NativeContext| {
            let client = client.clone();
            let batch = batch.clone();
            let rows = rows.clone();
            let buffers = buffers.clone();
            let column = column.clone();
            async move {
                if context.cancellation().is_requested() { return Exit::Interrupted; }
                if let Err(error) = batch_needs_row(&batch) { return error.into_exit(); }
                let connection = match acquire_transaction_connection(&client.kernel, &context).await {
                    Ok(connection) => connection,
                    Err(exit) => return exit,
                };
                let first = match reserve_batch_first_row(&batch, &client.kernel) {
                    Ok(first) => first,
                    Err(error) => return Exit::Fail(error),
                };
                let input = (|| {
                    let frames = reserve_batch_cursor(&context, batch_cursor::<R, BatchBytesDestination>).map_err(SqlError::Memory)?;
                    let destination = BatchBytesDestination::reserve(&rows, &buffers, column, kind).map_err(SqlError::Memory)?;
                    let reservation = client.kernel.reserve_optional_callback_session().map_err(SqlError::Memory)?;
                    let cleanup = client.kernel.reserve_optional_cleanup().map_err(SqlError::Memory)?;
                    let (batch, destination) = take_bytes_batch(&batch, destination)?;
                    Ok((frames, batch, destination, reservation, cleanup))
                })();
                let (frames, batch, destination, reservation, cleanup) = match input {
                    Ok(input) => input,
                    Err(error) => return Exit::Fail(error),
                };
                run_transaction_async(client.kernel, reservation, Some(connection), cleanup, env, context, move |tx, env, _| async move {
                    let input = BatchCursorInput { session: tx.session, batch, destination, first };
                    frames.run(input, env, batch_cursor::<R, _>).await
                }).await
            }
        };
        let (program, outer_layouts) = contextual_sql(factory, true);
        (program, PreparedSqlFrameDemand::new(outer_layouts, cursor_layouts))
    }

    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    fn batch_bytes_into_rows_atomic_selected<R: Send + Sync + 'static>(
        &self, batch: PreparedSqlBatch, column: crate::ColumnKey, kind: PreparedSqlBytesKind,
        rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>>,
        buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
    ) -> Effect<PreparedSqlRows<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>, SqlError, R> {
        let client = self.clone();
        let batch = AtomicBatchInput::new(batch, client.kernel.early_checkout_configured());
        let column = Arc::new(column);
        let factory = move |env: Arc<R>, context: NativeContext| {
            let prepared = if context.cancellation().is_requested() { Err(Exit::Interrupted) } else if batch.selected().is_some() {
                batch_needs_row(batch.get()).map(|_| None).map_err(Exit::Fail)
            } else {
                (|| {
                    let destination = BatchBytesDestination::reserve(&rows, &buffers, column.clone(), kind).map_err(SqlError::Memory)?;
                    let reservation = client.kernel.reserve_optional_callback_session().map_err(SqlError::Memory)?;
                    let first = if batch_needs_row(batch.get())? { client.kernel.reserve_optional_rows()? } else { None };
                    let cleanup = client.kernel.reserve_optional_cleanup().map_err(SqlError::Memory)?;
                    let (batch, destination) = take_bytes_batch(batch.get(), destination)?;
                    Ok(Some((batch, destination, reservation, first, cleanup)))
                })().map_err(Exit::Fail)
            };
            let client = client.clone();
            let selected = batch.selected();
            let rows = rows.clone();
            let buffers = buffers.clone();
            let column = column.clone();
            async move {
                let prepared = match prepared { Ok(value) => value, Err(exit) => return exit };
                let (connection, prepared) = if let Some(batch) = selected {
                    let connection = match acquire_prepared_connection(&client.kernel, &context).await { Ok(value) => value, Err(exit) => return exit };
                    let prepared = (|| {
                        let destination = BatchBytesDestination::reserve(&rows, &buffers, column, kind).map_err(SqlError::Memory)?;
                        let reservation = client.kernel.reserve_optional_callback_session().map_err(SqlError::Memory)?;
                        let first = if batch_needs_row(&batch)? { client.kernel.reserve_optional_rows()? } else { None };
                        let cleanup = client.kernel.reserve_optional_cleanup().map_err(SqlError::Memory)?;
                        let (batch, destination) = take_bytes_batch(&batch, destination)?;
                        Ok((batch, destination, reservation, first, cleanup))
                    })();
                    let prepared = match prepared { Ok(value) => value, Err(error) => return Exit::Fail(error) };
                    (connection, prepared)
                } else {
                    let Some(prepared) = prepared else { return Exit::Fail(SqlError::Memory(effect::memory::MemoryError::InvalidLayout)); };
                    (None, prepared)
                };
                let (batch, destination, reservation, first, cleanup) = prepared;
                run_transaction(client.kernel, reservation, connection, cleanup, env, context, move |tx| session_batch_destination_once(tx.session, batch, destination, first)).await
            }
        };
        contextual_sql(factory, false).0
    }
    /// Execute a fresh statement factory per run, or transfer one owned statement/Result once.
    pub fn execute<M, R>(&self, statement: impl StatementSource<M>) -> Effect<u64, SqlError, R>
    where
        R: Send + Sync + 'static,
    {
        let kernel = self.kernel.clone();
        let statement = Arc::new(statement.into_factory());
        effect_future! {
            exit_with_context; body: move |_, context| {
            let kernel = kernel.clone();
            let statement = statement.clone();
            async move {
                with_session(&kernel, &context, move |session| async move {
                    session
                        .execute(statement()?)
                        .await
                        .map(|result| result.rows_affected())
                })
                .await
            }
        }, }
    }
    /// Buffer decoded rows, retaining native decode errors and ordered lease cleanup.
    pub fn query<M, A, R>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
    ) -> Effect<Vec<A>, SqlError, R>
    where
        A: Send + 'static,
        R: Send + Sync + 'static,
    {
        let kernel = self.kernel.clone();
        let statement = Arc::new(statement.into_factory());
        effect_future! {
            exit_with_context; body: move |env: Arc<R>, context| {
            let kernel = kernel.clone();
            let statement = statement.clone();
            let decoder = decoder.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let session = match kernel.session().await {
                    Ok(session) => session,
                    Err(error) => return error.into_exit(),
                };
                let work = AssertUnwindSafe(async {
                    if context.cancellation().is_requested() {
                        return Exit::Interrupted;
                    }
                    run_buffered_query(&session, statement(), &decoder, env, context).await
                })
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                append_session_close(work, &session).await
            }
        }, }
    }
    /// Decode only the first optional row; a present invalid row is never translated to None.
    pub fn first<M, A, R>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
    ) -> Effect<Option<A>, SqlError, R>
    where
        A: Send + 'static,
        R: Send + Sync + 'static,
    {
        let kernel = self.kernel.clone();
        let statement = Arc::new(statement.into_factory());
        effect_future! {
            exit_with_context; body: move |env, context| {
            let kernel = kernel.clone();
            let statement = statement.clone();
            let decoder = decoder.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let session = match kernel.session().await {
                    Ok(session) => session,
                    Err(error) => return error.into_exit(),
                };
                let work = AssertUnwindSafe(async {
                    if context.cancellation().is_requested() {
                        return Exit::Interrupted;
                    }
                    let statement = match statement() {
                        Ok(statement) => statement,
                        Err(error) => return error.into_exit(),
                    };
                    match session.first(statement).await {
                        Ok(None) if context.cancellation().is_requested() => Exit::Interrupted,
                        Ok(None) => Exit::Success(None),
                        Ok(Some(row)) => match decoder
                                .decode(row, env, context.clone())
                                .await
                                .map_error(SqlError::Decode)
                                .into_failure::<Option<A>>() {
                            Ok(value) => Exit::Success(Some(value)),
                            Err(exit) => exit,
                        },
                        Err(error) => error.into_exit(),
                    }
                })
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                append_session_close(work, &session).await
            }
        }, }
    }
    /// Read the first row of an explicitly selected read-only statement through
    /// a prepared cursor and synchronous borrowed projection. The cursor is
    /// retired at the first row. Use `first` for the native fetch-optional contract.
    pub fn first_prepared_read<M, A, R>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
    ) -> Effect<Option<A>, SqlError, R>
    where
        A: Send + 'static,
        R: Send + Sync + 'static,
    {
        let kernel = self.kernel.clone();
        let statement = Arc::new(statement.into_factory());
        effect_future! {
            exit_with_context; body: move |env: Arc<R>, context| {
                let kernel = kernel.clone();
                let statement = statement.clone();
                let decoder = decoder.clone();
                async move {
                    if context.cancellation().is_requested() {
                        return Exit::Interrupted;
                    }
                    if !decoder.has_borrowed_decoder_with_env() {
                        return Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder));
                    }
                    let first = match kernel.reserve_optional_rows() {
                        Ok(Some(reservation)) => Some(reservation),
                        Ok(None) => return Exit::Fail(SqlError::Memory(MemoryError::UnsupportedRunContract)),
                        Err(error) => return error.into_exit(),
                    };
                    let cleanup = match kernel.reserve_optional_cleanup() {
                        Ok(Some(cleanup)) => Some(cleanup),
                        Ok(None) => return Exit::Fail(SqlError::Memory(MemoryError::UnsupportedRunContract)),
                        Err(error) => return Exit::Fail(SqlError::Memory(error)),
                    };
                    let reservation = match kernel.reserve_optional_session() {
                        Ok(Some(reservation)) => reservation,
                        Ok(None) => return Exit::Fail(SqlError::Memory(MemoryError::UnsupportedRunContract)),
                        Err(error) => return Exit::Fail(SqlError::Memory(error)),
                    };
                    let session = match reservation.open().await {
                        Ok(session) => session,
                        Err(error) => return error.into_exit(),
                    };
                    let work = AssertUnwindSafe(async {
                        if context.cancellation().is_requested() {
                            return Exit::Interrupted;
                        }
                        let result = async {
                            let mut cursor = session.incremental_cursor_reserved(statement()?, first).await?;
                            match cursor.next_row().await? {
                                Some(row) => decoder.decode_ref_with_env(&row, &env)
                                    .map(Some)
                                    .map_err(SqlError::Decode),
                                None => Ok(None),
                            }
                        }.await;
                        match result {
                            Ok(_) if context.cancellation().is_requested() => Exit::Interrupted,
                            Ok(value) => Exit::Success(value),
                            Err(error) => error.into_exit(),
                        }
                    })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    finish_transaction_cleanup(work, None, cleanup, &session, None).await
                }
            },
        }
    }

    /// Read a required scalar from ordinal zero through SQLx native Type/Decode.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub fn scalar<M, T, R>(&self, statement: impl StatementSource<M>) -> Effect<T, SqlError, R>
    where
        T: Send + 'static + Type<Turso>,
        for<'a> T: Decode<'a, Turso>,
        R: Send + Sync + 'static,
    {
        effect! {
            bind;
            program: sql! {
                first;
                client: self,
                statement: statement,
                decoder: row! {
                    project: |row| {
                        row.try_get(0).map_err(|source| crate::RowError::Native {
                            column: "[0]".into(),
                            source,
                        })
                    },
                },
            },
            body: |value| match value {
                Some(value) => effect_sync! { once; body: move || value, },
                None => effect_fail! { error: SqlError::RowAbsent, },
            },
        }
    }
    /// Execute an ordered batch of bound statements on one lease; it is not implicitly transactional.
    /// Each statement retains its own row/result sequence, including provider-supported multi-results.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub fn batch<R>(
        &self,
        statements: impl Fn() -> Result<Vec<SqlStatement>, SqlError> + Send + Sync + 'static,
    ) -> Effect<Vec<BatchResult>, SqlError, R>
    where
        R: Send + Sync + 'static,
    {
        let kernel = self.kernel.clone();
        let statements = Arc::new(statements);
        effect_future! {
            exit_with_context; body: move |env, context| {
            let kernel = kernel.clone();
            let statements = statements.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let session = match kernel.session().await {
                    Ok(session) => session,
                    Err(error) => return error.into_exit(),
                };
                let program = session_batch(session.clone(), move || statements());
                let outcome = AssertUnwindSafe(effect::effect! { run_in_context; context: context, program: program, environment: env, })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                append_session_close(outcome, &session).await
            }
        }, }
    }
    /// Incrementally fold rows under a live private cursor; accumulator and row payloads move.
    pub fn fold_rows<M, A, S, R>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
        initial: impl Fn() -> S + Send + Sync + 'static,
        step: impl Fn(S, A) -> Result<S, SqlError> + Send + Sync + 'static,
    ) -> Effect<S, SqlError, R>
    where
        A: Send + 'static,
        S: Send + 'static,
        R: Send + Sync + 'static,
    {
        sql! {
            fold_effect;
            client: self,
            statement: statement,
            decoder: decoder,
            initial: initial,
            step: move |state, value| effect_from_result! { once; result: step(state, value), },
        }
    }
    /// Fold decoded rows with Effects through the existing core Stream/Sink scope machinery.
    /// The cursor connection remains private; callbacks may open independent sessions through R.
    pub fn fold_rows_effect<M, A, S, R>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
        initial: impl Fn() -> S + Send + Sync + 'static,
        step: impl Fn(S, A) -> Effect<S, SqlError, R> + Send + Sync + 'static,
    ) -> Effect<S, SqlError, R>
    where
        A: Send + 'static,
        S: Send + 'static,
        R: Send + Sync + 'static,
    {
        let kernel = self.kernel.clone();
        let statement = Arc::new(statement.into_factory());
        let acquire = effect_future! {
            exit_with_context; body: move |_, context| {
            let kernel = kernel.clone();
            let statement = statement.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let session = match kernel.session().await {
                    Ok(session) => session,
                    Err(error) => return error.into_exit(),
                };
                if context.cancellation().is_requested() {
                    return append_session_close(Exit::Interrupted, &session).await;
                }
                let result = AssertUnwindSafe(async { session.cursor(statement()?).await })
                    .catch_unwind()
                    .await;
                match result {
                    Ok(Ok(cursor)) => Exit::Success(CursorOwner {
                        cursor: Arc::new(tokio::sync::Mutex::new(Some(cursor))),
                        session,
                        close_session: true,
                    }),
                    Ok(Err(error)) => append_session_close(error.into_exit(), &session).await,
                    Err(panic) => {
                        append_session_close(Exit::Die(Defect::from_panic(panic)), &session).await
                    }
                }
            }
        }, };
        fold_from_cursor(acquire, decoder, initial, step)
    }

    /// Run a scoped transaction Effect on one exclusive native owner, preserving complete outcomes.
    /// Begin/commit/rollback and state publication are awaited without a cancellation race.
    pub fn transaction<A, R>(
        &self,
        body: impl Fn(SqlTransaction) -> Effect<A, SqlError, R> + Send + Sync + 'static,
    ) -> Effect<A, SqlError, R>
    where
        A: Send + 'static,
        R: Send + Sync + 'static,
    {
        let kernel = self.kernel.clone();
        let body = Arc::new(body);
        effect_future! {
            exit_with_context; body: move |env, context| {
            let kernel = kernel.clone();
            let body = body.clone();
            async move {
                let connection = match acquire_prepared_connection(&kernel, &context).await { Ok(value) => value, Err(exit) => return exit };
                let reservation = if connection.is_some() {
                    match kernel.reserve_optional_callback_session() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) }
                } else { None };
                let cleanup = match kernel.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                run_transaction(kernel, reservation, connection, cleanup, env, context, move |transaction| body(transaction)).await
            }
        }, }
    }
    /// Transfer one transaction callback and its non-Clone captures into this execution.
    /// Reuse reports consumption before acquiring a connection; reusable transaction
    /// descriptions use `transaction` with a fresh body factory.
    pub fn transaction_once<A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        body: impl FnOnce(SqlTransaction) -> Effect<A, SqlError, R> + Send + 'static,
    ) -> Effect<A, SqlError, R> {
        let client = self.clone();
        let body = Arc::new(Mutex::new(Some(body)));
        effect_future! {
            exit_with_context; body: move |env, context| {
            let client = client.clone();
            let body = body.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                if client.kernel.early_checkout_configured() {
                    let input = match crate::try_prepared_lock(&body) { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    if input.is_none() { return Exit::Fail(SqlError::Consumed); }
                }
                let connection = match acquire_prepared_connection(&client.kernel, &context).await { Ok(value) => value, Err(exit) => return exit };
                let reservation = if connection.is_some() {
                    match client.kernel.reserve_optional_callback_session() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) }
                } else { None };
                let cleanup = match client.kernel.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let body = if connection.is_some() {
                    match crate::try_prepared_lock(&body) { Ok(mut value) => value.take(), Err(error) => return Exit::Fail(SqlError::Memory(error)) }
                } else {
                    body.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
                };
                let Some(body) = body else {
                    return Exit::Fail(SqlError::Consumed);
                };
                run_transaction(client.kernel, reservation, connection, cleanup, env, context, body).await
            }
        }, }
    }
    /// Scope a transferred SQLx/native connection. Borrowed native transactions use SqlKernel::with_borrowed.
    /// Repeated runs require a fresh owner factory; transferred owners are finalized exactly once.
    pub fn with_owned_session<A, R>(
        &self,
        acquire: impl Fn() -> Result<sqlx::TursoConnection, SqlError> + Send + Sync + 'static,
        body: impl Fn(SqlSession) -> Effect<A, SqlError, R> + Send + Sync + 'static,
    ) -> Effect<A, SqlError, R>
    where
        A: Send + 'static,
        R: Send + Sync + 'static,
    {
        let acquire = Arc::new(acquire);
        let body = Arc::new(body);
        effect_future! {
            exit_with_context; body: move |env, context| {
            let acquire = acquire.clone();
            let body = body.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let session = match acquire() {
                    Ok(connection) => NativeSession::adopt(connection),
                    Err(error) => return error.into_exit(),
                };
                let outcome = match catch_unwind(AssertUnwindSafe(|| {
                    body(SqlSession {
                        session: session.clone(),
                    })
                })) {
                    Ok(program) => effect::effect! { run_in_context; context: context, program: program, environment: env, }.await,
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                };
                append_session_close(outcome, &session).await
            }
        }, }
    }
}

/// Buffered native batch items preserve native row/result order and do not claim atomicity.
pub struct BatchResult {
    /// Each native result set ends with its affected-count metadata.
    pub items: Vec<either::Either<sqlx::TursoQueryResult, sqlx::TursoRow>>,
}
/// Scoped cursor resource retained by core Stream until success, failure or interruption cleanup.
struct CursorOwner {
    /// Cursor ownership is shared only to access it through a resource binding.
    cursor: Arc<tokio::sync::Mutex<Option<crate::NativeCursor>>>,
    /// Session closed after cursor disposal.
    session: NativeSession,
    /// An independent cursor lease closes here; transaction cursors retain their enclosing owner.
    close_session: bool,
}

/// Scoped ordinary SQL session. Retained handles cannot reopen the released owner.
#[derive(Clone)]
#[must_use]
pub struct SqlSession {
    /// Private native state checked together with exclusive access.
    session: NativeSession,
}
/// Transaction-only handle; construction is private and parent frames are fenced during savepoints.
#[derive(Clone)]
#[must_use]
pub struct SqlTransaction {
    /// Same native owner as the enclosing scope.
    session: NativeSession,
}
impl SqlSession {
    /// Execute a fresh native statement through this scoped owner.
    pub fn execute<M, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
    ) -> Effect<u64, SqlError, R> {
        session_execute(self.session.clone(), statement)
    }
}
impl SqlTransaction {
    /// Internal construction for a migration assessment already owning a native transaction.
    pub(crate) fn from_native(session: NativeSession) -> Self {
        Self { session }
    }
    /// Execute on this transaction frame, without reacquiring the pool.
    pub fn execute<M, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
    ) -> Effect<u64, SqlError, R> {
        session_execute(self.session.clone(), statement)
    }
    /// Move an execution-local statement once into this transaction; use execute(factory) for replay.
    pub fn execute_once<R: Send + Sync + 'static>(
        &self,
        statement: SqlStatement,
    ) -> Effect<u64, SqlError, R> {
        sql! { execute; client: self, statement: one_statement(statement), }
    }
    /// Query decoded rows on the current frame with the original environment for validations.
    pub fn query<M, A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
    ) -> Effect<Vec<A>, SqlError, R> {
        session_query(self.session.clone(), statement, decoder)
    }
    /// Execute on a nested savepoint with an independently admitted output.
    pub fn query_into_rows_atomic<M, A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
        rows: Arc<PreparedSqlRowsPool<A>>,
    ) -> Effect<PreparedSqlRows<A>, SqlError, R> {
        self.query_into_rows_atomic_selected(statement, decoder, rows)
    }

    /// Transfer an existing transaction handle into a one-use prepared savepoint query.
    /// Native provider and decoded pointee allocations remain separate.
    pub fn query_into_rows_atomic_prepared<M, A: Send + 'static, R: Send + Sync + 'static>(
        self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
        rows: Arc<PreparedSqlRowsPool<A>>,
    ) -> impl Description<Value = PreparedSqlRows<A>, Error = SqlError, Env = R> {
        let session = self.session;
        let statement = statement.into_factory();
        effect::effect_prepare! { description; program: effect::effect_future! {
            exit_with_context;
            body: move |environment: Environment<R>, context: FutureContext<SqlError>| async move {
                if context.is_requested() { return Exit::Interrupted; }
                if !decoder.has_borrowed_decoder_with_env() {
                    return Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder));
                }
                run_prepared_savepoint_query(session, statement, move || {
                    let rows = rows.reserve()?;
                    Ok(PreparedRowsDestination { rows, decoder, environment })
                }, context).await
            }
        } }
    }

    /// Construct the nested query with outer and cursor futures in selected
    /// run frames, returning their exact layouts.
    pub fn query_into_rows_atomic_with_frames<M, A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
        rows: Arc<PreparedSqlRowsPool<A>>,
    ) -> (
        Effect<PreparedSqlRows<A>, SqlError, R>,
        PreparedSqlFrameDemand,
    ) {
        let transaction = self.clone();
        let statement = Arc::new(statement.into_factory());
        let cursor_layouts = query_cursor_layouts(query_cursor::<R, RowsDestination<A, R>>);
        let factory = move |env: Arc<R>, context: NativeContext| {
            let selected_session = if transaction.session.is_prepared() {
                (!context.cancellation().is_requested() && decoder.has_borrowed_decoder_with_env()).then(|| transaction.session.try_clone_prepared())
            } else { Some(Ok(transaction.session.clone())) };
            let statement = statement.clone();
            let decoder = decoder.clone();
            let rows = rows.clone();
            async move {
                match AssertUnwindSafe(async move {
                    if context.cancellation().is_requested() { return Exit::Interrupted; }
                    if !decoder.has_borrowed_decoder_with_env() { return Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder)); }
                    let session = match selected_session { Some(Ok(session)) => session, Some(Err(error)) => return Exit::Fail(SqlError::Memory(error)), None => return Exit::Interrupted };
                    let first = match session.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() };
                    let cursor_frames = match reserve_query_cursor(&context, query_cursor::<R, RowsDestination<A, R>>) { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let builder = match rows.reserve() { Ok(builder) => builder, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let cleanup = match session.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                    let reservation = match session.reserve_savepoint() { Ok(value) => value, Err(error) => return error.into_exit() };
                    let statement = match statement() { Ok(statement) => statement, Err(error) => return error.into_exit() };
                    run_savepoint_async(session, reservation, cleanup, env, context, move |tx, env, _| async move {
                        let input = QueryCursorInput { session: tx.session, statement, destinations: RowsDestination { rows: builder, decoder }, first };
                        cursor_frames.run(input, env, query_cursor::<R, _>).await
                    }).await
                }).catch_unwind().await {
                    Ok(outcome) => outcome,
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                }
            }
        };
        let (program, outer_layouts) = contextual_sql(factory, true);
        (program, PreparedSqlFrameDemand::new(outer_layouts, cursor_layouts))
    }

    fn query_into_rows_atomic_selected<M, A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
        rows: Arc<PreparedSqlRowsPool<A>>,
    ) -> Effect<PreparedSqlRows<A>, SqlError, R> {
        let transaction = self.clone();
        let statement = Arc::new(statement.into_factory());
        let factory = move |env: Arc<R>, context: NativeContext| {
        let selected_session = if transaction.session.is_prepared() {
            (!context.cancellation().is_requested() && decoder.has_borrowed_decoder_with_env())
                .then(|| transaction.session.try_clone_prepared())
        } else {
            Some(Ok(transaction.session.clone()))
        };
        let statement = statement.clone();
        let decoder = decoder.clone();
        let rows = rows.clone();
        async move {
            match AssertUnwindSafe(async move {
                if context.cancellation().is_requested() { return Exit::Interrupted; }
                if !decoder.has_borrowed_decoder_with_env() { return Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder)); }
                let session = match selected_session {
                    Some(Ok(session)) => session,
                    Some(Err(error)) => return Exit::Fail(SqlError::Memory(error)),
                    None => return Exit::Interrupted,
                };
                let builder = match rows.reserve() { Ok(builder) => builder, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let first = match session.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() };
                let cleanup = match session.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let reservation = match session.reserve_savepoint() { Ok(value) => value, Err(error) => return error.into_exit() };
                let statement = match statement() { Ok(statement) => statement, Err(error) => return error.into_exit() };
                run_savepoint(session, reservation, cleanup, env, context, move |tx| session_query_rows_once(tx.session, statement, builder, decoder, first)).await
            }).catch_unwind().await {
                Ok(outcome) => outcome,
                Err(panic) => Exit::Die(Defect::from_panic(panic)),
            }
        }
    };
    contextual_sql(factory, false).0
}
/// Execute on a savepoint with all text/blob destinations preadmitted.
pub fn query_bytes_into_rows_atomic<M, R: Send + Sync + 'static>(
    &self,
    statement: impl StatementSource<M>,
    column: crate::ColumnKey,
    kind: PreparedSqlBytesKind,
    rows: Arc<PreparedSqlRowsPool<effect::memory::BufferLease<'static>>>,
    buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
) -> Effect<PreparedSqlRows<effect::memory::BufferLease<'static>>, SqlError, R> {
    self.query_bytes_into_rows_atomic_selected(statement, column, kind, rows, buffers)
}

/// Transfer an existing transaction handle into a one-use prepared savepoint
/// query with preadmitted text/blob destinations.
pub fn query_bytes_into_rows_atomic_prepared<M, R: Send + Sync + 'static>(
    self,
    statement: impl StatementSource<M>,
    column: crate::ColumnKey,
    kind: PreparedSqlBytesKind,
    rows: Arc<PreparedSqlRowsPool<effect::memory::BufferLease<'static>>>,
    buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
) -> impl Description<
    Value = PreparedSqlRows<effect::memory::BufferLease<'static>>,
    Error = SqlError,
    Env = R,
> {
    let session = self.session;
    let statement = statement.into_factory();
    effect::effect_prepare! { description; program: effect::effect_future! {
        exit_with_context;
        body: move |_environment: Environment<R>, context: FutureContext<SqlError>| async move {
            if context.is_requested() { return Exit::Interrupted; }
            run_prepared_savepoint_query(session, statement, move || {
                let destinations = PreparedBytesDestinations::reserve(&rows, &buffers)?;
                Ok(BytesDestination { destinations, column, kind })
            }, context).await
        }
    } }
}

/// Construct the nested text/blob query with outer and cursor futures in
/// selected run frames, returning their exact layouts.
pub fn query_bytes_into_rows_atomic_with_frames<M, R: Send + Sync + 'static>(
    &self,
    statement: impl StatementSource<M>,
    column: crate::ColumnKey,
    kind: PreparedSqlBytesKind,
    rows: Arc<PreparedSqlRowsPool<effect::memory::BufferLease<'static>>>,
    buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
) -> (
    Effect<PreparedSqlRows<effect::memory::BufferLease<'static>>, SqlError, R>,
    PreparedSqlFrameDemand,
) {
    let transaction = self.clone();
    let statement = Arc::new(statement.into_factory());
    let column = Arc::new(column);
    let cursor_layouts = query_cursor_layouts(query_cursor::<R, BytesDestination>);
    let factory = move |env: Arc<R>, context: NativeContext| {
        let selected_session = if transaction.session.is_prepared() {
                (!context.cancellation().is_requested()).then(|| transaction.session.try_clone_prepared())
            } else { Some(Ok(transaction.session.clone())) };
        let statement = statement.clone();
        let column = column.clone();
        let rows = rows.clone();
        let buffers = buffers.clone();
        async move {
            match AssertUnwindSafe(async move {
                if context.cancellation().is_requested() { return Exit::Interrupted; }
                let session = match selected_session { Some(Ok(session)) => session, Some(Err(error)) => return Exit::Fail(SqlError::Memory(error)), None => return Exit::Interrupted };
                let first = match session.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() };
                    let cursor_frames = match reserve_query_cursor(&context, query_cursor::<R, BytesDestination>) { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let destinations = match PreparedBytesDestinations::reserve(&rows, &buffers) { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let cleanup = match session.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let reservation = match session.reserve_savepoint() { Ok(value) => value, Err(error) => return error.into_exit() };
                let statement = match statement() { Ok(statement) => statement, Err(error) => return error.into_exit() };
                run_savepoint_async(session, reservation, cleanup, env, context, move |tx, env, _| async move {
                    let input = QueryCursorInput { session: tx.session, statement, destinations: BytesDestination { destinations, column, kind }, first };
                    cursor_frames.run(input, env, query_cursor::<R, _>).await
                }).await
            }).catch_unwind().await {
                Ok(outcome) => outcome,
                Err(panic) => Exit::Die(Defect::from_panic(panic)),
            }
        }
    };
    let (program, outer_layouts) = contextual_sql(factory, true);
    (program, PreparedSqlFrameDemand::new(outer_layouts, cursor_layouts))
}

fn query_bytes_into_rows_atomic_selected<M, R: Send + Sync + 'static>(
    &self,
    statement: impl StatementSource<M>,
    column: crate::ColumnKey,
    kind: PreparedSqlBytesKind,
    rows: Arc<PreparedSqlRowsPool<effect::memory::BufferLease<'static>>>,
    buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
) -> Effect<PreparedSqlRows<effect::memory::BufferLease<'static>>, SqlError, R> {
    let transaction = self.clone();
    let statement = Arc::new(statement.into_factory());
    let column = Arc::new(column);
    let factory = move |env: Arc<R>, context: NativeContext| {
        let selected_session = if transaction.session.is_prepared() {
            (!context.cancellation().is_requested()).then(|| transaction.session.try_clone_prepared())
        } else {
            Some(Ok(transaction.session.clone()))
        };
        let statement = statement.clone();
        let column = column.clone();
        let rows = rows.clone();
        let buffers = buffers.clone();
        async move {
            match AssertUnwindSafe(async move {
                if context.cancellation().is_requested() { return Exit::Interrupted; }
                let session = match selected_session {
                    Some(Ok(session)) => session,
                    Some(Err(error)) => return Exit::Fail(SqlError::Memory(error)),
                    None => return Exit::Interrupted,
                };
                let destinations = match PreparedBytesDestinations::reserve(&rows, &buffers) { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let first = match session.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() };
                let cleanup = match session.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let reservation = match session.reserve_savepoint() { Ok(value) => value, Err(error) => return error.into_exit() };
                let statement = match statement() { Ok(statement) => statement, Err(error) => return error.into_exit() };
                run_savepoint(session, reservation, cleanup, env, context, move |tx| session_query_bytes_once(tx.session, statement, destinations, column, kind, first)).await
            }).catch_unwind().await {
                Ok(outcome) => outcome,
                Err(panic) => Exit::Die(Defect::from_panic(panic)),
            }
        }
    };
    contextual_sql(factory, false).0
}
/// Run an owned batch on a nested savepoint; rows and result metadata both
/// consume output item capacity.
pub fn batch_into_rows_atomic<A: Send + 'static, R: Send + Sync + 'static>(
    &self,
    batch: PreparedSqlBatch,
    decoder: RowDecoder<A, R>,
    rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<A>>>,
) -> Effect<PreparedSqlRows<PreparedSqlBatchItem<A>>, SqlError, R> {
        self.batch_into_rows_atomic_selected(batch, decoder, rows)
}

/// Transfer this transaction handle and a batch into a one-use prepared
/// savepoint description. Use late graph input to transfer them after admission.
#[must_use]
pub fn batch_into_rows_atomic_prepared<A: Send + 'static, R: Send + Sync + 'static>(
    self,
    batch: PreparedSqlBatch,
    decoder: RowDecoder<A, R>,
    rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<A>>>,
) -> impl Description<Value = PreparedSqlRows<PreparedSqlBatchItem<A>>, Error = SqlError, Env = R> {
    let session = self.session;
    effect::effect_prepare! { description; program: effect::effect_future! {
        exit_with_context;
        body: move |environment: Environment<R>, context: FutureContext<SqlError>| async move {
            if context.is_requested() { return Exit::Interrupted; }
            if !decoder.has_borrowed_decoder_with_env() {
                return Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder));
            }
            run_prepared_savepoint_batch(session, batch, move || {
                let rows = rows.reserve()?;
                Ok(PreparedBatchRowsDestination { rows, decoder, environment })
            }, context).await
        }
    } }
}

/// Construct the nested atomic batch with its outer future and completion
/// in selected run frames, returning their exact layouts.
pub fn batch_into_rows_atomic_with_frames<A: Send + 'static, R: Send + Sync + 'static>(
    &self,
    batch: PreparedSqlBatch,
    decoder: RowDecoder<A, R>,
    rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<A>>>,
) -> PreparedSqlBatchEffect<A, R> {
        let transaction = self.clone();
        let batch = Mutex::new(Some(batch));
        let cursor_layouts = batch_cursor_layouts(batch_cursor::<R, BatchRowsDestination<A, R>>);
        let factory = move |env: Arc<R>, context: NativeContext| {
            #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
            let prepared = if context.cancellation().is_requested() {
                Err(Exit::Interrupted)
            } else if !decoder.has_borrowed_decoder_with_env() {
                Err(Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder)))
            } else {
                (|| {
                    let session = if transaction.session.is_prepared() {
                        batch_needs_row(&batch)?;
                        Some(transaction.session.try_clone_prepared().map_err(SqlError::Memory)?)
                    } else {
                        None
                    };
                    let active = session.as_ref().unwrap_or(&transaction.session);
                    let first = if batch_needs_row(&batch)? { active.reserve_optional_rows()? } else { None };
                    let frames = reserve_batch_cursor(&context, batch_cursor::<R, BatchRowsDestination<A, R>>).map_err(SqlError::Memory)?;
                    let builder = rows.reserve().map_err(SqlError::Memory)?;
                    let cleanup = active.reserve_optional_cleanup().map_err(SqlError::Memory)?;
                    let reservation = active.reserve_savepoint()?;
                    let (batch, builder) = take_batch_input(&batch, builder)?;
                    Ok((session, frames, reservation, batch, builder, first, cleanup))
                })().map_err(Exit::Fail)
            };
            let ordinary_session = (!transaction.session.is_prepared()).then(|| transaction.session.clone());
            let decoder = decoder.clone();
            async move {
                let (session, frames, reservation, batch, builder, first, cleanup) = match prepared { Ok(value) => value, Err(exit) => return exit };
                let Some(session) = session.or(ordinary_session) else { return Exit::Fail(SqlError::Memory(effect::memory::MemoryError::InvalidLayout)); };
                run_savepoint_async(session, reservation, cleanup, env, context, move |tx, env, _| async move {
                    let input = BatchCursorInput { session: tx.session, batch, destination: BatchRowsDestination { rows: builder, decoder }, first };
                    frames.run(input, env, batch_cursor::<R, _>).await
                }).await
            }
        };
        let (program, outer_layouts) = contextual_sql(factory, true);
        (program, PreparedSqlFrameDemand::new(outer_layouts, cursor_layouts))
}

fn batch_into_rows_atomic_selected<A: Send + 'static, R: Send + Sync + 'static>(
    &self,
    batch: PreparedSqlBatch,
    decoder: RowDecoder<A, R>,
        rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<A>>>,
    ) -> Effect<PreparedSqlRows<PreparedSqlBatchItem<A>>, SqlError, R> {
    let transaction = self.clone();
    let batch = Mutex::new(Some(batch));
    let factory = move |env: Arc<R>, context: NativeContext| {
            #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
            let prepared = if context.cancellation().is_requested() {
                Err(Exit::Interrupted)
            } else if !decoder.has_borrowed_decoder_with_env() {
                Err(Exit::Fail(SqlError::Decode(crate::RowError::MissingBorrowedDecoder)))
            } else {
                (|| {
                    let session = if transaction.session.is_prepared() {
                        batch_needs_row(&batch)?;
                        Some(transaction.session.try_clone_prepared().map_err(SqlError::Memory)?)
                    } else {
                        None
                    };
                    let active = session.as_ref().unwrap_or(&transaction.session);
                    let reservation = active.reserve_savepoint()?;
                    let input = prepare_batch_input(&batch, &rows, active)?;
                    Ok((session, reservation, input))
                })().map_err(Exit::Fail)
            };
            let ordinary_session = (!transaction.session.is_prepared()).then(|| transaction.session.clone());
            let decoder = decoder.clone();
            async move {
                let (session, reservation, (batch, builder, first, cleanup)) = match prepared { Ok(value) => value, Err(exit) => return exit };
                let Some(session) = session.or(ordinary_session) else { return Exit::Fail(SqlError::Memory(effect::memory::MemoryError::InvalidLayout)); };
                run_savepoint(session, reservation, cleanup, env, context, move |tx| session_batch_rows_once(tx.session, batch, builder, decoder, first)).await
            }
        };
        contextual_sql(factory, false).0
    }
    /// Execute a batch on a savepoint with preadmitted text/blob row buffers.
    pub fn batch_bytes_into_rows_atomic<R: Send + Sync + 'static>(
        &self, batch: PreparedSqlBatch, column: crate::ColumnKey, kind: PreparedSqlBytesKind,
        rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>>,
        buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
    ) -> Effect<PreparedSqlRows<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>, SqlError, R> {
        self.batch_bytes_into_rows_atomic_selected(batch, column, kind, rows, buffers)
    }

    /// Transfer this transaction handle and a batch into a one-use prepared
    /// savepoint description with preadmitted text/blob destinations.
    #[must_use]
    pub fn batch_bytes_into_rows_atomic_prepared<R: Send + Sync + 'static>(
        self,
        batch: PreparedSqlBatch,
        column: crate::ColumnKey,
        kind: PreparedSqlBytesKind,
        rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>>,
        buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
    ) -> impl Description<
        Value = PreparedSqlRows<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>,
        Error = SqlError,
        Env = R,
    > {
        let session = self.session;
        effect::effect_prepare! { description; program: effect::effect_future! {
            exit_with_context;
            body: move |_environment: Environment<R>, context: FutureContext<SqlError>| async move {
                if context.is_requested() { return Exit::Interrupted; }
                run_prepared_savepoint_batch(session, batch, move || {
                    BatchBytesDestination::reserve(&rows, &buffers, column, kind)
                }, context).await
            }
        } }
    }

    /// Construct the nested text/blob batch with its outer future and
    /// completion in selected run frames, returning their exact layouts.
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    pub fn batch_bytes_into_rows_atomic_with_frames<R: Send + Sync + 'static>(
        &self, batch: PreparedSqlBatch, column: crate::ColumnKey, kind: PreparedSqlBytesKind,
        rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>>,
        buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
    ) -> PreparedSqlBatchEffect<effect::memory::BufferLease<'static>, R> {
        let transaction = self.clone();
        let batch = Mutex::new(Some(batch));
        let column = Arc::new(column);
        let cursor_layouts = batch_cursor_layouts(batch_cursor::<R, BatchBytesDestination>);
        let factory = move |env: Arc<R>, context: NativeContext| {
            let prepared = if context.cancellation().is_requested() { Err(Exit::Interrupted) } else {
                (|| {
                    if transaction.session.is_prepared() { batch_needs_row(&batch)?; }
                    let session = if transaction.session.is_prepared() { Some(transaction.session.try_clone_prepared().map_err(SqlError::Memory)?) } else { None };
                    let active = session.as_ref().unwrap_or(&transaction.session);
                    let first = if batch_needs_row(&batch)? { active.reserve_optional_rows()? } else { None };
                    let frames = reserve_batch_cursor(&context, batch_cursor::<R, BatchBytesDestination>).map_err(SqlError::Memory)?;
                    let reservation = active.reserve_savepoint()?;
                    let destination = BatchBytesDestination::reserve(&rows, &buffers, column.clone(), kind).map_err(SqlError::Memory)?;
                    let cleanup = active.reserve_optional_cleanup().map_err(SqlError::Memory)?;
                    let (batch, destination) = take_bytes_batch(&batch, destination)?;
                    Ok((session, frames, reservation, batch, destination, first, cleanup))
                })().map_err(Exit::Fail)
            };
            let ordinary_session = (!transaction.session.is_prepared()).then(|| transaction.session.clone());
            async move {
                let (session, frames, reservation, batch, destination, first, cleanup) = match prepared { Ok(value) => value, Err(exit) => return exit };
                let Some(session) = session.or(ordinary_session) else { return Exit::Fail(SqlError::Memory(effect::memory::MemoryError::InvalidLayout)); };
                run_savepoint_async(session, reservation, cleanup, env, context, move |tx, env, _| async move {
                    let input = BatchCursorInput { session: tx.session, batch, destination, first };
                    frames.run(input, env, batch_cursor::<R, _>).await
                }).await
            }
        };
        let (program, outer_layouts) = contextual_sql(factory, true);
        (program, PreparedSqlFrameDemand::new(outer_layouts, cursor_layouts))
    }

    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    fn batch_bytes_into_rows_atomic_selected<R: Send + Sync + 'static>(
        &self, batch: PreparedSqlBatch, column: crate::ColumnKey, kind: PreparedSqlBytesKind,
        rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>>,
        buffers: Arc<effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>>,
    ) -> Effect<PreparedSqlRows<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>, SqlError, R> {
        let transaction = self.clone();
        let batch = Mutex::new(Some(batch));
        let column = Arc::new(column);
        let factory = move |env: Arc<R>, context: NativeContext| {
            let prepared = if context.cancellation().is_requested() { Err(Exit::Interrupted) } else {
                (|| {
                    let session = if transaction.session.is_prepared() {
                        batch_needs_row(&batch)?;
                        Some(transaction.session.try_clone_prepared().map_err(SqlError::Memory)?)
                    } else {
                        None
                    };
                    let active = session.as_ref().unwrap_or(&transaction.session);
                    let reservation = active.reserve_savepoint()?;
                    let destination = BatchBytesDestination::reserve(&rows, &buffers, column.clone(), kind).map_err(SqlError::Memory)?;
                    let first = if batch_needs_row(&batch)? { active.reserve_optional_rows()? } else { None };
                    let cleanup = active.reserve_optional_cleanup().map_err(SqlError::Memory)?;
                    let (batch, destination) = take_bytes_batch(&batch, destination)?;
                    Ok((session, reservation, batch, destination, first, cleanup))
                })().map_err(Exit::Fail)
            };
            let ordinary_session = (!transaction.session.is_prepared()).then(|| transaction.session.clone());
            async move {
                let (session, reservation, batch, destination, first, cleanup) = match prepared { Ok(value) => value, Err(exit) => return exit };
                let Some(session) = session.or(ordinary_session) else { return Exit::Fail(SqlError::Memory(effect::memory::MemoryError::InvalidLayout)); };
                run_savepoint(session, reservation, cleanup, env, context, move |tx| session_batch_destination_once(tx.session, batch, destination, first)).await
            }
        };
        contextual_sql(factory, false).0
    }
    /// Run a nested savepoint and join rollback/release on any body failure.
    pub fn savepoint<A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        body: impl Fn(SqlTransaction) -> Effect<A, SqlError, R> + Send + Sync + 'static,
    ) -> Effect<A, SqlError, R> {
        let session = self.session.clone();
        let body = Arc::new(body);
        effect_future! {
            exit_with_context; body: move |env, context| {
            let session = session.try_clone_prepared();
            let body = body.clone();
            async move {
                if context.cancellation().is_requested() { return Exit::Interrupted; }
                let session = match session {
                    Ok(session) => session,
                    Err(error) => return Exit::Fail(SqlError::Memory(error)),
                };
                let reservation = if session.is_prepared() {
                    match session.reserve_savepoint_wait().await { Ok(value) => value, Err(error) => return error.into_exit() }
                } else { None };
                if reservation.is_some() && context.cancellation().is_requested() { return Exit::Interrupted; }
                let cleanup = match session.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                run_savepoint(session, reservation, cleanup, env, context, move |tx| body(tx)).await
            }
        }, }
    }
    /// Transfer one savepoint callback and its captures without a hidden Clone requirement.
    /// Its one-use ownership delegates the same checked frame and settlement path.
    pub fn savepoint_once<A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        body: impl FnOnce(SqlTransaction) -> Effect<A, SqlError, R> + Send + 'static,
    ) -> Effect<A, SqlError, R> {
        let session = self.session.clone();
        let body = Arc::new(Mutex::new(Some(body)));
        effect_future! {
            exit_with_context; body: move |env, context| {
            let session = if session.is_prepared() {
                if context.cancellation().is_requested() {
                    Err(Exit::Interrupted)
                } else {
                    match crate::try_prepared_lock(&body) {
                        Ok(input) if input.is_none() => Err(Exit::Fail(SqlError::Consumed)),
                        Ok(_) => session.try_clone_prepared()
                            .map_err(|error| Exit::Fail(SqlError::Memory(error))),
                        Err(error) => Err(Exit::Fail(SqlError::Memory(error))),
                    }
                }
            } else {
                Ok(session.clone())
            };
            let body = body.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let session = match session { Ok(session) => session, Err(exit) => return exit };
                let reservation = if session.is_prepared() {
                    match session.reserve_savepoint_wait().await { Ok(value) => value, Err(error) => return error.into_exit() }
                } else { None };
                if reservation.is_some() && context.cancellation().is_requested() { return Exit::Interrupted; }
                let cleanup = match session.reserve_optional_cleanup() { Ok(value) => value, Err(error) => return Exit::Fail(SqlError::Memory(error)) };
                let body = if session.is_prepared() {
                    match crate::try_prepared_lock(&body) { Ok(mut value) => value.take(), Err(error) => return Exit::Fail(SqlError::Memory(error)) }
                } else {
                    body.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
                };
                let Some(body) = body else {
                    return Exit::Fail(SqlError::Consumed);
                };
                run_savepoint(session, reservation, cleanup, env, context, body).await
            }
        }, }
    }
    /// Borrow the native frame only within a checked async callback; no unrestricted handle escapes.
    pub fn native<A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        body: impl for<'a> Fn(&'a mut sqlx::TursoConnection) -> crate::NativeFuture<'a, A>
        + Send
        + Sync
        + 'static,
    ) -> Effect<A, SqlError, R> {
        let session = self.session.clone();
        let body = Arc::new(body);
        effect_future! {
            exit_with_context; body: move |_, context| {
            let session = session.try_clone_prepared();
            let body = body.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let session = match session {
                    Ok(session) => session,
                    Err(error) => return Exit::Fail(SqlError::Memory(error)),
                };
                match sql! { session_native; session: session, body: |connection| body(connection), }.await {
                    Ok(value) => Exit::Success(value),
                    Err(error) => error.into_exit(),
                }
            }
        }, }
    }
}

/// Environment-selected SQL operations used directly inside sql_with!/effect! bodies.
pub struct Sql;
impl Sql {
    /// Execute a factory or transferred statement/Result in the selected client's independent session.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub fn execute<M, R: HasSqlClient + Send + Sync + 'static>(
        statement: impl StatementSource<M>,
    ) -> Effect<u64, SqlError, R> {
        let statement = Arc::new(statement.into_factory());
        effect! {
            let statement = statement.clone();
            let client = service! { env |env: &R| { env.sql_client().clone() }, }?;
            sql! { execute; client: client, statement: move || statement(), }?
        }
    }
    /// Decode a fresh or transferred statement using the selected client and original R.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub fn query<M, A: Send + 'static, R: HasSqlClient + Send + Sync + 'static>(
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
    ) -> Effect<Vec<A>, SqlError, R> {
        let statement = Arc::new(statement.into_factory());
        effect! {
            let statement = statement.clone();
            let decoder = decoder.clone();
            let client = service! { env |env: &R| { env.sql_client().clone() }, }?;
            sql! { query; client: client, statement: move || statement(), decoder: decoder, }?
        }
    }
    /// Decode one optional row from a factory or owned input; only true absence becomes None.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub fn first<M, A: Send + 'static, R: HasSqlClient + Send + Sync + 'static>(
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
    ) -> Effect<Option<A>, SqlError, R> {
        let statement = Arc::new(statement.into_factory());
        effect! {
            let statement = statement.clone();
            let decoder = decoder.clone();
            let client = service! { env |env: &R| { env.sql_client().clone() }, }?;
            sql! { first; client: client, statement: move || statement(), decoder: decoder, }?
        }
    }
}
/// Project an extracted SQL Effect into R lazily, preserving every typed failure channel.
pub fn sql_with<
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    S: std::borrow::Borrow<SqlClient>,
>(
    select: impl Fn(&R) -> S + Send + Sync + 'static,
    program: Effect<A, E, SqlClient>,
) -> Effect<A, E, R> {
    effect_provide_some! { program: program, project: move |env| select(env).borrow().clone(), }
}

/// Join one native operation with session cleanup; panics become defects before the lease closes.
async fn with_session<A: Send, F: Future<Output = Result<A, SqlError>> + Send>(
    kernel: &SqlKernel,
    context: &NativeContext,
    body: impl FnOnce(NativeSession) -> F + Send,
) -> Exit<A, SqlError> {
    if context.cancellation().is_requested() {
        return Exit::Interrupted;
    }
    let session = match kernel.session().await {
        Ok(session) => session,
        Err(error) => return error.into_exit(),
    };
    let result = AssertUnwindSafe(async { body(session.clone()).await })
        .catch_unwind()
        .await;
    let outcome = match result {
        Ok(Ok(value)) if !context.cancellation().is_requested() => Exit::Success(value),
        Ok(Ok(_)) => Exit::Interrupted,
        Ok(Err(error)) => error.into_exit(),
        Err(panic) => Exit::Die(Defect::from_panic(panic)),
    };
    append_session_close(outcome, &session).await
}
/// Append an ordered cleanup failure while retaining the original error/defect/interruption.
pub(crate) fn append_cleanup<A>(
    mut outcome: Exit<A, SqlError>,
    cleanup: Result<(), SqlError>,
) -> Exit<A, SqlError> {
    let Err(error) = cleanup else {
        return outcome;
    };
    match error.into_exit::<()>() {
        Exit::Fail(error) => append_cleanup_failure(outcome, CleanupFailure::ReleaseFailure(error)),
        Exit::Die(defect) => append_cleanup_failure(outcome, CleanupFailure::ReleaseDefect(defect)),
        Exit::Interrupted => append_cleanup_failure(outcome, CleanupFailure::ReleaseInterrupted),
        Exit::Success(()) => outcome,
        Exit::FinalizationFailed(details) => {
            match details.primary {
                PrimaryOutcome::Succeeded => {}
                PrimaryOutcome::Failed(error) => {
                    outcome =
                        append_cleanup_failure(outcome, CleanupFailure::ReleaseFailure(error));
                }
                PrimaryOutcome::Defected(defect) => {
                    outcome =
                        append_cleanup_failure(outcome, CleanupFailure::ReleaseDefect(defect));
                }
                PrimaryOutcome::WasInterrupted => {
                    outcome = append_cleanup_failure(outcome, CleanupFailure::ReleaseInterrupted);
                }
            }
            outcome = append_cleanup_failure(outcome, details.first);
            for failure in details.rest {
                outcome = append_cleanup_failure(outcome, failure);
            }
            outcome
        }
    }
}
fn append_cleanup_failure<A>(
    outcome: Exit<A, SqlError>,
    failure: CleanupFailure<SqlError>,
) -> Exit<A, SqlError> {
    match outcome {
        Exit::FinalizationFailed(mut details) => {
            details.rest.push(failure);
            Exit::FinalizationFailed(details)
        }
        Exit::Success(_) => Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Succeeded,
            first: failure,
            rest: Default::default(),
        }),
        Exit::Fail(error) => Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(error),
            first: failure,
            rest: Default::default(),
        }),
        Exit::Die(defect) => Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Defected(defect),
            first: failure,
            rest: Default::default(),
        }),
        Exit::Interrupted => Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::WasInterrupted,
            first: failure,
            rest: Default::default(),
        }),
    }
}
async fn append_session_close<A>(
    mut outcome: Exit<A, SqlError>,
    session: &NativeSession,
) -> Exit<A, SqlError> {
    match session.close_failures().await {
        Err(error) => append_cleanup(outcome, Err(error)),
        Ok(failures) => {
            for error in [failures.rollback, failures.settle, failures.close]
                .into_iter()
                .flatten()
            {
                outcome = match error {
                    crate::kernel::CloseFault::Sql(error) => append_cleanup(outcome, Err(error)),
                    crate::kernel::CloseFault::Defect(defect) => {
                        append_cleanup_failure(outcome, CleanupFailure::ReleaseDefect(defect))
                    }
                };
            }
            outcome
        }
    }
}
fn append_close_fault<A>(outcome: Exit<A, SqlError>, fault: crate::kernel::CloseFault) -> Exit<A, SqlError> {
    match fault {
        crate::kernel::CloseFault::Sql(error) => append_cleanup(outcome, Err(error)),
        crate::kernel::CloseFault::Defect(defect) => append_cleanup_failure(outcome, CleanupFailure::ReleaseDefect(defect)),
    }
}
/// Build a replay-safe factory around one explicitly transferred execution payload.
#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
fn one_statement(
    statement: SqlStatement,
) -> impl Fn() -> Result<SqlStatement, SqlError> + Send + Sync + 'static {
    let statement = Mutex::new(Some(statement));
    move || {
        statement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or(SqlError::Consumed)
    }
}
async fn run_savepoint<A: Send + 'static, R: Send + Sync + 'static>(
    session: NativeSession,
    reservation: Option<crate::kernel::SavepointReservation>,
    cleanup: Option<effect::CleanupFailures<SqlError>>,
    env: Arc<R>,
    context: NativeContext,
    body: impl FnOnce(SqlTransaction) -> Effect<A, SqlError, R>,
) -> Exit<A, SqlError> {
    run_savepoint_async(session, reservation, cleanup, env, context, move |tx, env, context| async move {
        match catch_unwind(AssertUnwindSafe(|| body(tx))) {
            Ok(program) => effect::effect! { run_in_context; context: context, program: program, environment: env, }.await,
            Err(panic) => Exit::Die(Defect::from_panic(panic)),
        }
    }).await
}

async fn run_savepoint_async<A, R, F>(
    session: NativeSession,
    reservation: Option<crate::kernel::SavepointReservation>,
    cleanup: Option<effect::CleanupFailures<SqlError>>,
    env: Arc<R>,
    context: NativeContext,
    body: impl FnOnce(SqlTransaction, Arc<R>, NativeContext) -> F,
) -> Exit<A, SqlError>
where
    A: Send + 'static,
    R: Send + Sync + 'static,
    F: Future<Output = Exit<A, SqlError>>,
{
    if context.cancellation().is_requested() {
        return Exit::Interrupted;
    }
    let nested = match session.begin_savepoint_with_admission(reservation).await {
        Ok(nested) => nested,
        Err(error) => return error.into_exit(),
    };
    let outcome = match catch_unwind(AssertUnwindSafe(|| {
        let session = if session.is_prepared() {
            session.into_savepoint_body(&nested)
        } else {
            nested.clone()
        };
        body(SqlTransaction { session }, env, context)
    })) {
        Ok(future) => future.await,
        Err(panic) => Exit::Die(Defect::from_panic(panic)),
    };
    finish_savepoint(&nested, outcome, cleanup, &mut None).await
}

trait PreparedTransactionBody: Send + 'static {
    type Output: Send + 'static;
    fn run<'a>(
        self,
        session: &'a NativeSession,
        cancellation: FutureContext<SqlError>,
        diagnostic: Option<effect::PreparedPanicReservation>,
    ) -> impl Future<Output = Exit<Self::Output, SqlError>> + Send + 'a;
}

#[derive(Default)]
struct SavepointPanics {
    begin_or_body: Option<effect::PreparedPanicReservation>,
    cursor: Option<effect::PreparedPanicReservation>,
    transition: Option<effect::PreparedPanicReservation>,
}

fn reserve_savepoint_panics(
    session: &NativeSession,
) -> Result<SavepointPanics, effect::memory::MemoryError> {
    let Some(pool) = session.prepared_panic_diagnostics() else {
        return Ok(SavepointPanics::default());
    };
    let [begin_or_body, cursor, transition] = pool.try_reserve_group()?;
    Ok(SavepointPanics { begin_or_body, cursor, transition })
}

async fn run_prepared_savepoint<B: PreparedTransactionBody>(
    session: NativeSession,
    reservation: Option<crate::kernel::SavepointReservation>,
    cleanup: Option<effect::CleanupFailures<SqlError>>,
    panics: SavepointPanics,
    cancellation: FutureContext<SqlError>,
    body: B,
) -> Exit<B::Output, SqlError> {
    if cancellation.is_requested() {
        return Exit::Interrupted;
    }
    let SavepointPanics { mut begin_or_body, cursor, mut transition } = panics;
    let nested = match session.begin_savepoint_with_panic(reservation, &mut begin_or_body).await {
        Ok(nested) => nested,
        Err(error) => return error.into_exit(),
    };
    let result = AssertUnwindSafe(async { body.run(&nested, cancellation, cursor).await })
        .catch_unwind()
        .await;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(panic) => Exit::Die(crate::kernel::capture_panic(&mut begin_or_body, panic)),
    };
    drop(begin_or_body);
    finish_savepoint(&nested, outcome, cleanup, &mut transition).await
}

async fn finish_savepoint<A>(
    nested: &NativeSession,
    outcome: Exit<A, SqlError>,
    cleanup: Option<effect::CleanupFailures<SqlError>>,
    diagnostic: &mut Option<effect::PreparedPanicReservation>,
) -> Exit<A, SqlError> {
    match outcome {
        Exit::Success(value) => match nested.commit_direct_with_panic(diagnostic).await {
            Ok(()) => Exit::Success(value),
            Err(error) => error.into_exit(),
        },
        failure @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            let rollback = nested.rollback_direct_with_panic(diagnostic).await;
            if let Some(rest) = cleanup {
                let mut accumulator = effect::PreparedCleanupAccumulator::from_reserved(failure, rest);
                if let Err(fault) = rollback { accumulator.append_cleanup(fault.into_exit()); }
                accumulator.finish()
            } else {
                match rollback {
                    Ok(()) => failure,
                    Err(fault) => append_close_fault(failure, fault),
                }
            }
        }
    }
}

struct PreparedTransactionQuery<D> {
    statement: SqlStatement,
    destinations: D,
    first: Option<ReservedNativeRows>,
}

impl<D: QueryDestination<()>> PreparedTransactionBody for PreparedTransactionQuery<D> {
    type Output = D::Output;
    fn run<'a>(
        self,
        session: &'a NativeSession,
        cancellation: FutureContext<SqlError>,
        diagnostic: Option<effect::PreparedPanicReservation>,
    ) -> impl Future<Output = Exit<D::Output, SqlError>> + Send + 'a {
        let Self { statement, destinations, first } = self;
        query_cursor_shared(QueryCursorInput { session, statement, destinations, first }, &(), cancellation, diagnostic)
    }
}

async fn run_prepared_savepoint_query<D: QueryDestination<()>>(
    session: NativeSession,
    statement: impl FnOnce() -> Result<SqlStatement, SqlError> + Send,
    destinations: impl FnOnce() -> Result<D, effect::memory::MemoryError> + Send,
    context: FutureContext<SqlError>,
) -> Exit<D::Output, SqlError> {
    let first = match session.reserve_optional_rows() {
        Ok(reservation) => reservation,
        Err(error) => return error.into_exit(),
    };
    let panics = match reserve_savepoint_panics(&session) {
        Ok(panics) => panics,
        Err(error) => return Exit::Fail(SqlError::Memory(error)),
    };
    let destinations = match destinations() {
        Ok(destination) => destination,
        Err(error) => return Exit::Fail(SqlError::Memory(error)),
    };
    let cleanup = match session.reserve_optional_cleanup() {
        Ok(cleanup) => cleanup,
        Err(error) => return Exit::Fail(SqlError::Memory(error)),
    };
    let reservation = match session.reserve_savepoint() {
        Ok(reservation) => reservation,
        Err(error) => return error.into_exit(),
    };
    let statement = match statement() {
        Ok(statement) => statement,
        Err(error) => return error.into_exit(),
    };
    let body = PreparedTransactionQuery { statement, destinations, first };
    run_prepared_savepoint(session, reservation, cleanup, panics, context, body).await
}

struct PreparedTransactionBatch<D> {
    batch: PreparedSqlBatch,
    destination: D,
    first: Option<ReservedNativeRows>,
}

impl<D: BatchDestination<()>> PreparedTransactionBody for PreparedTransactionBatch<D> {
    type Output = D::Output;
    fn run<'a>(
        self,
        session: &'a NativeSession,
        cancellation: FutureContext<SqlError>,
        diagnostic: Option<effect::PreparedPanicReservation>,
    ) -> impl Future<Output = Exit<D::Output, SqlError>> + Send + 'a {
        let Self { batch, destination, first } = self;
        batch_cursor_shared(BatchCursorInput { session, batch, destination, first }, &(), cancellation, diagnostic)
    }
}

async fn run_prepared_savepoint_batch<D: BatchDestination<()>>(
    session: NativeSession,
    mut batch: PreparedSqlBatch,
    destination: impl FnOnce() -> Result<D, effect::memory::MemoryError> + Send,
    context: FutureContext<SqlError>,
) -> Exit<D::Output, SqlError> {
    let first = if batch.is_empty() {
        None
    } else {
        match session.reserve_optional_rows() {
            Ok(reservation) => reservation,
            Err(error) => return error.into_exit(),
        }
    };
    let panics = match reserve_savepoint_panics(&session) {
        Ok(panics) => panics,
        Err(error) => return Exit::Fail(SqlError::Memory(error)),
    };
    let destination = match destination() {
        Ok(destination) => destination,
        Err(error) => return Exit::Fail(SqlError::Memory(error)),
    };
    let cleanup = match session.reserve_optional_cleanup() {
        Ok(cleanup) => cleanup,
        Err(error) => return Exit::Fail(SqlError::Memory(error)),
    };
    if batch.len() > destination.capacity() {
        return Exit::Fail(SqlError::RowLimit { limit: destination.capacity() });
    }
    let reservation = match session.reserve_savepoint() {
        Ok(reservation) => reservation,
        Err(error) => return error.into_exit(),
    };
    if let Err(error) = batch.materialize_all() {
        return error.into_exit();
    }
    let body = PreparedTransactionBatch { batch, destination, first };
    run_prepared_savepoint(session, reservation, cleanup, panics, context, body).await
}

#[expect(clippy::result_large_err, reason = "retains the original SQL exit channels without heap allocation")]
async fn acquire_prepared_connection<A, C: SqlCancellation + Send>(
    kernel: &SqlKernel,
    context: C,
) -> Result<Option<sqlx::pool::PoolConnection<Turso>>, Exit<A, SqlError>> {
    if !kernel.early_checkout_configured() {
        return Ok(None);
    }
    acquire_transaction_connection(kernel, context).await.map(Some)
}

#[expect(clippy::result_large_err, reason = "retains the original SQL exit channels without heap allocation")]
async fn acquire_transaction_connection<A, C: SqlCancellation + Send>(
    kernel: &SqlKernel,
    context: C,
) -> Result<sqlx::pool::PoolConnection<Turso>, Exit<A, SqlError>> {
    if let Err(error) = kernel.require_authoritative() {
        return Err(error.into_exit());
    }
    if context.is_requested() {
        return Err(Exit::Interrupted);
    }
    let connection = kernel.acquire_connection().await.map_err(SqlError::into_exit)?;
    if context.is_requested() {
        drop(connection);
        return Err(Exit::Interrupted);
    }
    Ok(connection)
}

async fn run_transaction<A: Send + 'static, R: Send + Sync + 'static>(
    kernel: SqlKernel,
    reservation: Option<crate::kernel::PreparedSqlCallbackSessionReservation>,
    connection: Option<sqlx::pool::PoolConnection<Turso>>,
    cleanup: Option<effect::CleanupFailures<SqlError>>,
    env: Arc<R>,
    context: NativeContext,
    body: impl FnOnce(SqlTransaction) -> Effect<A, SqlError, R>,
) -> Exit<A, SqlError> {
    run_transaction_async(kernel, reservation, connection, cleanup, env, context, move |tx, env, context| async move {
        match catch_unwind(AssertUnwindSafe(|| body(tx))) {
            Ok(program) => effect::effect! { run_in_context; context: context, program: program, environment: env, }.await,
            Err(panic) => Exit::Die(Defect::from_panic(panic)),
        }
    }).await
}

async fn run_transaction_async<A, R, F>(
    kernel: SqlKernel,
    reservation: Option<crate::kernel::PreparedSqlCallbackSessionReservation>,
    connection: Option<sqlx::pool::PoolConnection<Turso>>,
    cleanup: Option<effect::CleanupFailures<SqlError>>,
    env: Arc<R>,
    context: NativeContext,
    body: impl FnOnce(SqlTransaction, Arc<R>, NativeContext) -> F,
) -> Exit<A, SqlError>
where
    A: Send + 'static,
    R: Send + Sync + 'static,
    F: Future<Output = Exit<A, SqlError>>,
{
    if let Err(error) = kernel.require_authoritative() {
        return error.into_exit();
    }
    if context.cancellation().is_requested() {
        return Exit::Interrupted;
    }
    let (session, callback) = match (reservation, connection) {
        (Some(reservation), Some(connection)) => {
            let (session, callback) = reservation.open_acquired(connection).await;
            (session, Some(callback))
        }
        (None, connection) => match open_transaction_session(&kernel, None, connection).await {
            Ok(session) => (session, None),
            Err(error) => return error.into_exit(),
        },
        (Some(_), None) => {
            return Exit::Fail(SqlError::Memory(effect::memory::MemoryError::InvalidLayout));
        }
    };
    let tx = match session.begin_direct().await {
        Ok(tx) => tx,
        Err(error) => return finish_transaction_cleanup(error.into_exit(), None, cleanup, &session, None).await,
    };
    let outcome = match catch_unwind(AssertUnwindSafe(|| {
        let session = match callback {
            Some(callback) => tx.transaction_body(callback),
            None => tx.clone(),
        };
        body(SqlTransaction { session }, env, context)
    })) {
        Ok(future) => future.await,
        Err(panic) => Exit::Die(Defect::from_panic(panic)),
    };
    finish_transaction(&session, &tx, outcome, cleanup, None).await
}

#[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
async fn open_transaction_session(
    kernel: &SqlKernel,
    reservation: Option<crate::PreparedSqlSessionReservation>,
    connection: Option<sqlx::pool::PoolConnection<Turso>>,
) -> Result<NativeSession, SqlError> {
    match (reservation, connection) {
        (Some(reservation), Some(connection)) => Ok(reservation.open_acquired(connection).await),
        (Some(reservation), None) => reservation.open().await,
        (None, None) => kernel.session().await,
        (None, Some(connection)) if !kernel.prepared_session_configured() => {
            Ok(kernel.session_from_acquired(connection))
        }
        (None, Some(_)) => Err(SqlError::Memory(effect::memory::MemoryError::InvalidLayout)),
    }
}

async fn run_prepared_transaction<B: PreparedTransactionBody>(
    kernel: SqlKernel,
    reservation: Option<crate::PreparedSqlSessionReservation>,
    connection: sqlx::pool::PoolConnection<Turso>,
    cleanup: Option<effect::CleanupFailures<SqlError>>,
    mut panics: Option<crate::kernel::TransactionPanicReservations>,
    cancellation: FutureContext<SqlError>,
    body: B,
) -> Exit<B::Output, SqlError> {
    if let Err(error) = kernel.require_authoritative() {
        return error.into_exit();
    }
    if cancellation.is_requested() {
        return Exit::Interrupted;
    }
    let session = match open_transaction_session(&kernel, reservation, Some(connection)).await {
        Ok(session) => session,
        Err(error) => return error.into_exit(),
    };
    let mut begin_or_body = panics.as_mut().and_then(|owners| owners.begin.take());
    let cursor = panics.as_mut().and_then(|owners| owners.cursor.take());
    let tx = match session.begin_direct_with_panic(&mut begin_or_body).await {
        Ok(tx) => tx,
        Err(error) => return finish_transaction_cleanup(error.into_exit(), None, cleanup, &session, panics).await,
    };
    let result = AssertUnwindSafe(async { body.run(&tx, cancellation, cursor).await })
        .catch_unwind()
        .await;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(panic) => Exit::Die(crate::kernel::capture_panic(&mut begin_or_body, panic)),
    };
    drop(begin_or_body);
    finish_transaction(&session, &tx, outcome, cleanup, panics).await
}

async fn finish_transaction<A>(
    session: &NativeSession,
    tx: &NativeSession,
    outcome: Exit<A, SqlError>,
    cleanup: Option<effect::CleanupFailures<SqlError>>,
    mut panics: Option<crate::kernel::TransactionPanicReservations>,
) -> Exit<A, SqlError> {
    let (outcome, rollback) = match outcome {
        Exit::Success(value) => match tx.commit_direct_with_panic(&mut panics.as_mut().and_then(|p| p.transition.take())).await {
            Ok(()) => (Exit::Success(value), None),
            Err(error) => (error.into_exit(), None),
        },
        failure @ (Exit::Fail(_)
        | Exit::Die(_)
        | Exit::Interrupted
        | Exit::FinalizationFailed(_)) => (failure, tx.rollback_direct_with_panic(&mut panics.as_mut().and_then(|p| p.transition.take())).await.err()),
    };
    finish_transaction_cleanup(outcome, rollback, cleanup, session, panics).await
}

async fn finish_transaction_cleanup<A>(
    mut outcome: Exit<A, SqlError>,
    rollback: Option<crate::kernel::CloseFault>,
    cleanup: Option<effect::CleanupFailures<SqlError>>,
    session: &NativeSession,
    mut panics: Option<crate::kernel::TransactionPanicReservations>,
) -> Exit<A, SqlError> {
    if let Some(rest) = cleanup {
        let mut accumulator = effect::PreparedCleanupAccumulator::from_reserved(outcome, rest);
        if let Some(fault) = rollback {
            accumulator.append_cleanup(fault.into_exit());
        }
        match close_with_panics(session, &mut panics).await {
            Err(error) => accumulator.append_cleanup(error.into_exit()),
            Ok(failures) => {
                for fault in [failures.rollback, failures.settle, failures.close].into_iter().flatten() {
                    accumulator.append_cleanup(fault.into_exit());
                }
            }
        }
        accumulator.finish()
    } else {
        if let Some(fault) = rollback {
            outcome = append_close_fault(outcome, fault);
        }
        match close_with_panics(session, &mut panics).await {
            Err(error) => append_cleanup(outcome, Err(error)),
            Ok(failures) => [failures.rollback, failures.settle, failures.close].into_iter().flatten().fold(outcome, append_close_fault),
        }
    }
}

#[expect(
    clippy::result_large_err,
    reason = "retains owned SQL diagnostics without an extra heap allocation"
)]
async fn close_with_panics(
    session: &NativeSession,
    panics: &mut Option<crate::kernel::TransactionPanicReservations>,
) -> Result<crate::kernel::CloseFailures, SqlError> {
    if let Some(p) = panics.as_mut() {
        session.close_failures_with_panic(&mut p.close_rollback, &mut p.close_settle, &mut p.close_dispose).await
    } else {
        session.close_failures().await
    }
}

#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
fn prepare_batch_input<A: Send + 'static>(
    batch: &Mutex<Option<PreparedSqlBatch>>,
    rows: &PreparedSqlRowsPool<PreparedSqlBatchItem<A>>,
    session: &NativeSession,
) -> Result<BatchRowsAdmission<A>, SqlError> {
    let builder = rows.reserve().map_err(SqlError::Memory)?;
    let first = if batch_needs_row(batch)? { session.reserve_optional_rows()? } else { None };
    let cleanup = session.reserve_optional_cleanup().map_err(SqlError::Memory)?;
    let (batch, builder) = take_batch_input(batch, builder)?;
    Ok((batch, builder, first, cleanup))
}

#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
fn reserve_batch_first_row(
    batch: &Mutex<Option<PreparedSqlBatch>>,
    kernel: &SqlKernel,
) -> Result<Option<ReservedNativeRows>, SqlError> {
    if batch_needs_row(batch)? {
        kernel.reserve_optional_rows()
    } else {
        Ok(None)
    }
}

#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
fn prepare_batch_input_with_session<A: Send + 'static>(
    batch: &Mutex<Option<PreparedSqlBatch>>,
    rows: &PreparedSqlRowsPool<PreparedSqlBatchItem<A>>,
    kernel: &SqlKernel,
) -> Result<BatchAdmission<A>, SqlError> {
    let builder = rows.reserve().map_err(SqlError::Memory)?;
    let reservation = kernel
        .reserve_optional_callback_session()
        .map_err(SqlError::Memory)?;
    let first = reserve_batch_first_row(batch, kernel)?;
    let cleanup = kernel.reserve_optional_cleanup().map_err(SqlError::Memory)?;
    let (batch, builder) = take_batch_input(batch, builder)?;
    Ok((batch, builder, reservation, first, cleanup))
}

type BatchRowsAdmission<A> = (
    PreparedSqlBatch,
    PreparedSqlRowsBuilder<PreparedSqlBatchItem<A>>,
    Option<ReservedNativeRows>,
    Option<effect::CleanupFailures<SqlError>>,
);

type BatchAdmission<A> = (
    PreparedSqlBatch,
    PreparedSqlRowsBuilder<PreparedSqlBatchItem<A>>,
    Option<crate::kernel::PreparedSqlCallbackSessionReservation>,
    Option<ReservedNativeRows>,
    Option<effect::CleanupFailures<SqlError>>,
);

#[expect(clippy::result_large_err, reason = "retains the owned SQL error representation without an extra heap allocation")]
fn batch_needs_row(batch: &Mutex<Option<PreparedSqlBatch>>) -> Result<bool, SqlError> {
    let input = crate::try_prepared_lock(batch).map_err(SqlError::Memory)?;
    input.as_ref().map(|batch| !batch.is_empty()).ok_or(SqlError::Consumed)
}

#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
fn take_batch_input<A: Send + 'static>(
    batch: &Mutex<Option<PreparedSqlBatch>>,
    builder: PreparedSqlRowsBuilder<PreparedSqlBatchItem<A>>,
) -> Result<
    (
        PreparedSqlBatch,
        PreparedSqlRowsBuilder<PreparedSqlBatchItem<A>>,
    ),
    SqlError,
> {
    let mut input = crate::try_prepared_lock(batch).map_err(SqlError::Memory)?;
    let Some(batch) = input.as_mut() else {
        return Err(SqlError::Consumed);
    };
    if batch.len() > builder.capacity() {
        return Err(SqlError::RowLimit {
            limit: builder.capacity(),
        });
    }
    batch.reserve_materializations()?;
    let Some(mut batch) = input.take() else {
        return Err(SqlError::Consumed);
    };
    drop(input);
    batch.materialize_reserved_all()?;
    Ok((batch, builder))
}

fn session_batch_rows_once<A: Send + 'static, R: Send + Sync + 'static>(
    session: NativeSession,
    batch: PreparedSqlBatch,
    builder: PreparedSqlRowsBuilder<PreparedSqlBatchItem<A>>,
    decoder: RowDecoder<A, R>,
    first: Option<ReservedNativeRows>,
) -> Effect<PreparedSqlRows<PreparedSqlBatchItem<A>>, SqlError, R> {
    session_batch_destination_once(session, batch, BatchRowsDestination { rows: builder, decoder }, first)
}

trait BatchDestination<R>: Send + 'static {
    type Output: Send + 'static;
    type Pending;
    fn capacity(&self) -> usize;
    fn remaining(&self) -> usize;
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    fn decode(&mut self, item: either::Either<sqlx::TursoQueryResult, sqlx::TursoRow>, env: &R) -> Result<Self::Pending, CursorDecodeError>;
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    fn commit(&mut self, index: usize, value: Self::Pending) -> Result<(), SqlError>;
    fn finish(self) -> Self::Output;
}
struct BatchRowsDestination<A: Send + 'static, R: Send + Sync + 'static> {
    rows: PreparedSqlRowsBuilder<PreparedSqlBatchItem<A>>,
    decoder: RowDecoder<A, R>,
}
impl<A: Send + 'static, R: Send + Sync + 'static> BatchDestination<R> for BatchRowsDestination<A, R> {
    type Output = PreparedSqlRows<PreparedSqlBatchItem<A>>;
    type Pending = either::Either<sqlx::TursoQueryResult, A>;
    fn capacity(&self) -> usize { self.rows.capacity() }
    fn remaining(&self) -> usize { self.rows.remaining() }
    fn decode(&mut self, item: either::Either<sqlx::TursoQueryResult, sqlx::TursoRow>, env: &R) -> Result<Self::Pending, CursorDecodeError> {
        Ok(match item {
            either::Either::Left(metadata) => either::Either::Left(metadata),
            either::Either::Right(row) => either::Either::Right(self.decoder.decode_ref_with_env(&row, env).map_err(SqlError::Decode)?),
        })
    }
    fn commit(&mut self, index: usize, value: Self::Pending) -> Result<(), SqlError> {
        self.rows.try_push(PreparedSqlBatchItem { statement_index: index, value }).map_err(|value| { drop(value); SqlError::RowLimit { limit: self.rows.capacity() } })
    }
    fn finish(self) -> Self::Output { self.rows.finish() }
}

struct PreparedBatchRowsDestination<A: Send + 'static, R: Send + Sync + 'static> {
    rows: PreparedSqlRowsBuilder<PreparedSqlBatchItem<A>>,
    decoder: RowDecoder<A, R>,
    environment: effect::__private::prepared_graph::Environment<R>,
}
impl<A: Send + 'static, R: Send + Sync + 'static> BatchDestination<()> for PreparedBatchRowsDestination<A, R> {
    type Output = PreparedSqlRows<PreparedSqlBatchItem<A>>;
    type Pending = either::Either<sqlx::TursoQueryResult, A>;
    fn capacity(&self) -> usize { self.rows.capacity() }
    fn remaining(&self) -> usize { self.rows.remaining() }
    #[expect(clippy::result_large_err, reason = "retains owned row diagnostics without an extra heap allocation")]
    fn decode(&mut self, item: either::Either<sqlx::TursoQueryResult, sqlx::TursoRow>, _: &()) -> Result<Self::Pending, CursorDecodeError> {
        Ok(match item {
            either::Either::Left(metadata) => either::Either::Left(metadata),
            either::Either::Right(row) => either::Either::Right(
                self.environment.with(|env| self.decoder.decode_ref_with_env(&row, env))
                    .map_err(CursorDecodeError::Defect)?
                    .map_err(SqlError::Decode)?,
            ),
        })
    }
    fn commit(&mut self, index: usize, value: Self::Pending) -> Result<(), SqlError> {
        self.rows.try_push(PreparedSqlBatchItem { statement_index: index, value }).map_err(|value| { drop(value); SqlError::RowLimit { limit: self.rows.capacity() } })
    }
    fn finish(self) -> Self::Output { self.rows.finish() }
}
fn session_batch_destination_once<R: Send + Sync + 'static, D: BatchDestination<R>>(
    session: NativeSession, batch: PreparedSqlBatch, destination: D,
    first: Option<ReservedNativeRows>,
) -> Effect<D::Output, SqlError, R> {
    let prepared = session.is_prepared();
    let inputs = Mutex::new(Some(BatchCursorInput { session, batch, destination, first }));
    effect_future! {
        exit_with_context; body: move |env: Arc<R>, context| {
            let input = if prepared {
                crate::try_prepared_lock(&inputs).map(|mut input| input.take())
            } else {
                Ok(inputs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take())
            };
            async move {
                match input {
                    Ok(Some(input)) => batch_cursor(input, env, context).await,
                    Ok(None) => Exit::Fail(SqlError::Consumed),
                    Err(error) => Exit::Fail(SqlError::Memory(error)),
                }
            }
        },
    }
}

struct BatchCursorInput<D, S = NativeSession> {
    session: S,
    batch: PreparedSqlBatch,
    destination: D,
    first: Option<ReservedNativeRows>,
}

type CursorFrameReservation<I, R, F, A> = Result<
    effect::PreparedContextualFutureReservation<I, R, F, A, SqlError>,
    effect::memory::MemoryError,
>;

async fn batch_cursor<R: Send + Sync + 'static, D: BatchDestination<R>>(
    input: BatchCursorInput<D>, env: Arc<R>, context: NativeContext,
) -> Exit<D::Output, SqlError> {
    batch_cursor_shared(input, env.as_ref(), &context, None).await
}

async fn batch_cursor_shared<R, D, C, S>(
    input: BatchCursorInput<D, S>, env: &R, context: C,
    panic: Option<effect::PreparedPanicReservation>,
) -> Exit<D::Output, SqlError>
where
    R: Send + Sync + 'static,
    D: BatchDestination<R>,
    C: SqlCancellation + Send,
    S: Borrow<NativeSession> + Send,
{
    let BatchCursorInput { session, mut batch, mut destination, mut first } = input;
    match AssertUnwindSafe(async move {
                    let session = session.borrow();
                    let mut index = 0usize;
                    while !batch.is_empty() {
                        if context.is_requested() { return Exit::Interrupted; }
                        if destination.remaining() < batch.len() { return Exit::Fail(SqlError::RowLimit { limit: destination.capacity() }); }
                        let reservation = if index == 0 { first.take() } else { match session.reserve_optional_rows() { Ok(value) => value, Err(error) => return error.into_exit() } };
                        let Some(native) = batch.take_first() else { return Exit::Fail(SqlError::Memory(effect::memory::MemoryError::InvalidLayout)); };
                        let mut cursor = match session.incremental_cursor_reserved(native, reservation).await { Ok(cursor) => cursor, Err(error) => return error.into_exit() };
                        let outcome = loop {
                            if context.is_requested() { break Exit::Interrupted; }
                            let item = match cursor.next_item().await { Ok(item) => item, Err(error) => break error.into_exit() };
                            if context.is_requested() { break Exit::Interrupted; }
                            let Some(item) = item else { break Exit::Success(()); };
                            if destination.remaining() == 0 { break Exit::Fail(SqlError::RowLimit { limit: destination.capacity() }); }
                            let value = match destination.decode(item, env) {
                                Ok(value) => value,
                                Err(CursorDecodeError::Sql(error)) => break Exit::Fail(error),
                                Err(CursorDecodeError::Defect(defect)) => break Exit::Die(defect),
                            };
                            if context.is_requested() { break Exit::Interrupted; }
                            if let Err(error) = destination.commit(index, value) { break Exit::Fail(error); }
                        };
                        drop(cursor);
                        match outcome {
                            Exit::Success(()) => {},
                            Exit::Fail(error) => return Exit::Fail(error),
                            Exit::Die(defect) => return Exit::Die(defect),
                            Exit::Interrupted => return Exit::Interrupted,
                            Exit::FinalizationFailed(details) => return Exit::FinalizationFailed(details),
                        }
                        index = match index.checked_add(1) { Some(index) => index, None => return Exit::Fail(SqlError::Memory(effect::memory::MemoryError::ArithmeticOverflow)) };
                    }
                    Exit::Success(destination.finish())
    }).catch_unwind().await {
        Ok(outcome) => outcome,
        Err(payload) => Exit::Die(match panic {
            Some(reservation) => reservation.capture(payload),
            None => Defect::from_panic(payload),
        }),
    }
}

fn batch_cursor_layouts<R, D, F>(
    _: fn(BatchCursorInput<D>, Arc<R>, NativeContext) -> F,
) -> effect::PreparedContextualFutureLayouts
where
    R: Send + Sync + 'static,
    D: BatchDestination<R>,
    F: Future<Output = Exit<D::Output, SqlError>> + Send + 'static,
{
    effect::PreparedContextualFutureReservation::<BatchCursorInput<D>, R, F, D::Output, SqlError>::layouts()
}

fn reserve_batch_cursor<R, D, F>(
    context: &NativeContext,
    _: fn(BatchCursorInput<D>, Arc<R>, NativeContext) -> F,
) -> CursorFrameReservation<BatchCursorInput<D>, R, F, D::Output>
where
    R: Send + Sync + 'static,
    D: BatchDestination<R>,
    F: Future<Output = Exit<D::Output, SqlError>> + Send + 'static,
{
    context.reserve_prepared_exit_future()
}

struct PreparedBytesDestinations {
    rows: PreparedSqlRowsBuilder<effect::memory::BufferLease<'static>>,
    used: usize,
}
struct BatchBytesDestination<K = Arc<crate::ColumnKey>> {
    rows: PreparedSqlRowsBuilder<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>,
    used: usize,
    column: K,
    kind: PreparedSqlBytesKind,
}
#[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
fn take_bytes_batch(
    batch: &Mutex<Option<PreparedSqlBatch>>,
    destination: BatchBytesDestination,
) -> Result<(PreparedSqlBatch, BatchBytesDestination), SqlError> {
    let mut input = crate::try_prepared_lock(batch).map_err(SqlError::Memory)?;
    let current = input.as_mut().ok_or(SqlError::Consumed)?;
    if current.len() > destination.rows.capacity() {
        return Err(SqlError::RowLimit { limit: destination.rows.capacity() });
    }
    current.reserve_materializations()?;
    let mut batch = input.take().ok_or(SqlError::Consumed)?;
    drop(input);
    batch.materialize_reserved_all()?;
    Ok((batch, destination))
}
impl<K: Borrow<crate::ColumnKey> + Send + 'static> BatchBytesDestination<K> {
    fn reserve(
        pool: &PreparedSqlRowsPool<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>,
        buffers: &effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>,
        column: K, kind: PreparedSqlBytesKind,
    ) -> Result<Self, effect::memory::MemoryError> {
        let mut rows = pool.reserve()?;
        while rows.remaining() != 0 {
            let mut buffer = buffers.try_checkout()?;
            buffer.clear();
            if let Err(item) = rows.try_push(PreparedSqlBatchItem { statement_index: 0, value: either::Either::Right(buffer) }) {
                drop(item);
                return Err(effect::memory::MemoryError::InvalidLayout);
            }
        }
        Ok(Self { rows, used: 0, column, kind })
    }
}
impl<R, K: Borrow<crate::ColumnKey> + Send + 'static> BatchDestination<R> for BatchBytesDestination<K> {
    type Output = PreparedSqlRows<PreparedSqlBatchItem<effect::memory::BufferLease<'static>>>;
    type Pending = Option<sqlx::TursoQueryResult>;
    fn capacity(&self) -> usize { self.rows.capacity() }
    fn remaining(&self) -> usize { self.rows.capacity() - self.used }
    fn decode(&mut self, item: either::Either<sqlx::TursoQueryResult, sqlx::TursoRow>, _: &R) -> Result<Self::Pending, CursorDecodeError> {
        let limit = self.rows.capacity();
        let output = self.rows.get_mut(self.used).ok_or(SqlError::RowLimit { limit })?;
        match item {
            either::Either::Left(metadata) => Ok(Some(metadata)),
            either::Either::Right(row) => {
                let either::Either::Right(buffer) = &mut output.value else { return Err(SqlError::Memory(effect::memory::MemoryError::InvalidLayout).into()); };
                let bytes = borrowed_cell(&row, self.column.borrow(), self.kind)?;
                buffer.extend_from_slice(bytes).map_err(SqlError::Memory)?;
                Ok(None)
            }
        }
    }
    fn commit(&mut self, index: usize, value: Self::Pending) -> Result<(), SqlError> {
        let limit = self.rows.capacity();
        let output = self.rows.get_mut(self.used).ok_or(SqlError::RowLimit { limit })?;
        if let Some(metadata) = value {
            let old = std::mem::replace(output, PreparedSqlBatchItem { statement_index: index, value: either::Either::Left(metadata) });
            drop(old);
        } else {
            output.statement_index = index;
        }
        self.used += 1;
        Ok(())
    }
    fn finish(mut self) -> Self::Output {
        while self.rows.len() > self.used { drop(self.rows.pop()); }
        self.rows.finish()
    }
}
impl PreparedBytesDestinations {
    fn reserve(
        pool: &PreparedSqlRowsPool<effect::memory::BufferLease<'static>>,
        buffers: &effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>,
    ) -> Result<Self, effect::memory::MemoryError> {
        let mut rows = pool.reserve()?;
        while rows.remaining() != 0 {
            let mut buffer = buffers.try_checkout()?;
            buffer.clear();
            if let Err(buffer) = rows.try_push(buffer) {
                drop(buffer);
                return Err(effect::memory::MemoryError::InvalidLayout);
            }
        }
        Ok(Self { rows, used: 0 })
    }
    fn finish(mut self) -> PreparedSqlRows<effect::memory::BufferLease<'static>> {
        while self.rows.len() > self.used {
            drop(self.rows.pop());
        }
        self.rows.finish()
    }
}

trait QueryDestination<R>: Send + 'static {
    type Output: Send + 'static;
    type Pending;
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    fn decode(&mut self, row: &sqlx::TursoRow, env: &R) -> Result<Self::Pending, CursorDecodeError>;
    #[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
    fn commit(&mut self, value: Self::Pending) -> Result<(), SqlError>;
    fn finish(self) -> Self::Output;
}
enum CursorDecodeError {
    Sql(SqlError),
    Defect(Defect),
}
impl From<SqlError> for CursorDecodeError {
    fn from(error: SqlError) -> Self { Self::Sql(error) }
}
struct BytesDestination<K = Arc<crate::ColumnKey>> {
    destinations: PreparedBytesDestinations,
    column: K,
    kind: PreparedSqlBytesKind,
}
impl<R, K: Borrow<crate::ColumnKey> + Send + 'static> QueryDestination<R> for BytesDestination<K> {
    type Output = PreparedSqlRows<effect::memory::BufferLease<'static>>;
    type Pending = ();
    fn decode(&mut self, row: &sqlx::TursoRow, _: &R) -> Result<(), CursorDecodeError> {
        let limit = self.destinations.rows.capacity();
        let buffer = self.destinations.rows.get_mut(self.destinations.used).ok_or(SqlError::RowLimit { limit })?;
        let bytes = borrowed_cell(row, self.column.borrow(), self.kind)?;
        buffer.extend_from_slice(bytes).map_err(SqlError::Memory)?;
        Ok(())
    }
    fn commit(&mut self, (): ()) -> Result<(), SqlError> { self.destinations.used += 1; Ok(()) }
    fn finish(self) -> Self::Output { self.destinations.finish() }
}
#[expect(clippy::result_large_err, reason = "retains owned SQL diagnostics without an extra heap allocation")]
fn borrowed_cell<'a>(row: &'a sqlx::TursoRow, column: &crate::ColumnKey, kind: PreparedSqlBytesKind) -> Result<&'a [u8], SqlError> {
        match (column, kind) {
            (crate::ColumnKey::Name(name), PreparedSqlBytesKind::Text) => row.try_get::<&str, _>(name.as_str()).map(str::as_bytes),
            (crate::ColumnKey::Ordinal(index), PreparedSqlBytesKind::Text) => row.try_get::<&str, _>(*index).map(str::as_bytes),
            (crate::ColumnKey::Name(name), PreparedSqlBytesKind::Blob) => row.try_get::<&[u8], _>(name.as_str()),
            (crate::ColumnKey::Ordinal(index), PreparedSqlBytesKind::Blob) => row.try_get::<&[u8], _>(*index),
        }.map_err(SqlError::Provider)
}
struct RowsDestination<A: Send + 'static, R: Send + Sync + 'static> {
    rows: PreparedSqlRowsBuilder<A>,
    decoder: RowDecoder<A, R>,
}
impl<A: Send + 'static, R: Send + Sync + 'static> QueryDestination<R> for RowsDestination<A, R> {
    type Output = PreparedSqlRows<A>;
    type Pending = A;
    fn decode(&mut self, row: &sqlx::TursoRow, env: &R) -> Result<A, CursorDecodeError> {
        if self.rows.remaining() == 0 { return Err(SqlError::RowLimit { limit: self.rows.capacity() }.into()); }
        self.decoder.decode_ref_with_env(row, env).map_err(SqlError::Decode).map_err(Into::into)
    }
    fn commit(&mut self, value: A) -> Result<(), SqlError> {
        self.rows.try_push(value).map_err(|value| { drop(value); SqlError::RowLimit { limit: self.rows.capacity() } })
    }
    fn finish(self) -> Self::Output { self.rows.finish() }
}

struct PreparedRowsDestination<A: Send + 'static, R: Send + Sync + 'static> {
    rows: PreparedSqlRowsBuilder<A>,
    decoder: RowDecoder<A, R>,
    environment: effect::__private::prepared_graph::Environment<R>,
}
impl<A: Send + 'static, R: Send + Sync + 'static> QueryDestination<()> for PreparedRowsDestination<A, R> {
    type Output = PreparedSqlRows<A>;
    type Pending = A;
    #[expect(clippy::result_large_err, reason = "retains owned row diagnostics without an extra heap allocation")]
    fn decode(&mut self, row: &sqlx::TursoRow, _: &()) -> Result<A, CursorDecodeError> {
        if self.rows.remaining() == 0 { return Err(SqlError::RowLimit { limit: self.rows.capacity() }.into()); }
        self.environment.with(|env| self.decoder.decode_ref_with_env(row, env))
            .map_err(CursorDecodeError::Defect)?
            .map_err(SqlError::Decode)
            .map_err(Into::into)
    }
    fn commit(&mut self, value: A) -> Result<(), SqlError> {
        self.rows.try_push(value).map_err(|value| { drop(value); SqlError::RowLimit { limit: self.rows.capacity() } })
    }
    fn finish(self) -> Self::Output { self.rows.finish() }
}

fn session_query_bytes_once<R: Send + Sync + 'static>(
    session: NativeSession,
    statement: SqlStatement,
    destinations: PreparedBytesDestinations,
    column: Arc<crate::ColumnKey>,
    kind: PreparedSqlBytesKind,
    first: Option<ReservedNativeRows>,
) -> Effect<PreparedSqlRows<effect::memory::BufferLease<'static>>, SqlError, R> {
    session_query_destination_once(session, statement, BytesDestination { destinations, column, kind }, first)
}

fn session_query_destination_once<R: Send + Sync + 'static, D: QueryDestination<R>>(
    session: NativeSession, statement: SqlStatement, destinations: D,
    first: Option<ReservedNativeRows>,
) -> Effect<D::Output, SqlError, R> {
    let prepared = session.is_prepared();
    let inputs = Mutex::new(Some(QueryCursorInput { session, statement, destinations, first }));
    effect_future! {
        exit_with_context; body: move |env: Arc<R>, context| {
            let input = if prepared {
                crate::try_prepared_lock(&inputs).map(|mut input| input.take())
            } else {
                Ok(inputs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take())
            };
            async move {
                match input {
                    Ok(Some(input)) => query_cursor(input, env, context).await,
                    Ok(None) => Exit::Fail(SqlError::Consumed),
                    Err(error) => Exit::Fail(SqlError::Memory(error)),
                }
            }
        },
    }
}

struct QueryCursorInput<D, S = NativeSession> {
    session: S,
    statement: SqlStatement,
    destinations: D,
    first: Option<ReservedNativeRows>,
}

async fn query_cursor<R: Send + Sync + 'static, D: QueryDestination<R>>(
    input: QueryCursorInput<D>, env: Arc<R>, context: NativeContext,
) -> Exit<D::Output, SqlError> {
    query_cursor_shared(input, env.as_ref(), &context, None).await
}

async fn query_cursor_shared<R, D, C, S>(
    input: QueryCursorInput<D, S>, env: &R, context: C,
    panic: Option<effect::PreparedPanicReservation>,
) -> Exit<D::Output, SqlError>
where
    R: Send + Sync + 'static,
    D: QueryDestination<R>,
    C: SqlCancellation + Send,
    S: Borrow<NativeSession> + Send,
{
    let QueryCursorInput { session, statement, mut destinations, first } = input;
    match AssertUnwindSafe(async move {
        if context.is_requested() { return Exit::Interrupted; }
        let mut cursor = match session.borrow().incremental_cursor_reserved(statement, first).await {
            Ok(cursor) => cursor,
            Err(error) => return error.into_exit(),
        };
        let outcome = loop {
            if context.is_requested() { break Exit::Interrupted; }
            let row = match cursor.next_row().await { Ok(row) => row, Err(error) => break error.into_exit() };
            if context.is_requested() { break Exit::Interrupted; }
            let Some(row) = row else { break Exit::Success(()); };
            let value = match destinations.decode(&row, env) {
                Ok(value) => value,
                Err(CursorDecodeError::Sql(error)) => break Exit::Fail(error),
                Err(CursorDecodeError::Defect(defect)) => break Exit::Die(defect),
            };
            if context.is_requested() { break Exit::Interrupted; }
            if let Err(error) = destinations.commit(value) { break Exit::Fail(error); }
        };
        drop(cursor);
        match outcome {
            Exit::Success(()) => Exit::Success(destinations.finish()),
            Exit::Fail(error) => Exit::Fail(error),
            Exit::Die(defect) => Exit::Die(defect),
            Exit::Interrupted => Exit::Interrupted,
            Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
        }
    }).catch_unwind().await {
        Ok(outcome) => outcome,
        Err(payload) => Exit::Die(match panic {
            Some(reservation) => reservation.capture(payload),
            None => Defect::from_panic(payload),
        }),
    }
}

fn query_cursor_layouts<R, D, F>(
    _: fn(QueryCursorInput<D>, Arc<R>, NativeContext) -> F,
) -> effect::PreparedContextualFutureLayouts
where
    R: Send + Sync + 'static,
    D: QueryDestination<R>,
    F: Future<Output = Exit<D::Output, SqlError>> + Send + 'static,
{
    effect::PreparedContextualFutureReservation::<QueryCursorInput<D>, R, F, D::Output, SqlError>::layouts()
}

fn reserve_query_cursor<R, D, F>(
    context: &NativeContext,
    _: fn(QueryCursorInput<D>, Arc<R>, NativeContext) -> F,
) -> CursorFrameReservation<QueryCursorInput<D>, R, F, D::Output>
where
    R: Send + Sync + 'static,
    D: QueryDestination<R>,
    F: Future<Output = Exit<D::Output, SqlError>> + Send + 'static,
{
    context.reserve_prepared_exit_future()
}

/// Consume the admitted output on the already-selected transaction frame.
fn session_query_rows_once<A: Send + 'static, R: Send + Sync + 'static>(
    session: NativeSession,
    statement: SqlStatement,
    builder: PreparedSqlRowsBuilder<A>,
    decoder: RowDecoder<A, R>,
    first: Option<ReservedNativeRows>,
) -> Effect<PreparedSqlRows<A>, SqlError, R> {
    session_query_destination_once(session, statement, RowsDestination { rows: builder, decoder }, first)
}

/// Execute one fresh statement on a scoped frame with cooperative cancellation at boundaries.
fn session_execute<M, R: Send + Sync + 'static>(
    session: NativeSession,
    statement: impl StatementSource<M>,
) -> Effect<u64, SqlError, R> {
    let statement = Arc::new(statement.into_factory());
    effect_future! {
        exit_with_context; body: move |_, context| {
        let session = session.try_clone_prepared();
        let statement = statement.clone();
        async move {
            if context.cancellation().is_requested() {
                return Exit::Interrupted;
            }
            let session = match session {
                Ok(session) => session,
                Err(error) => return Exit::Fail(SqlError::Memory(error)),
            };
            match statement() {
                Ok(statement) => match session.execute(statement).await {
                    Ok(result) if !context.cancellation().is_requested() => {
                        Exit::Success(result.rows_affected())
                    }
                    Ok(_) => Exit::Interrupted,
                    Err(error) => error.into_exit(),
                },
                Err(error) => error.into_exit(),
            }
        }
    }, }
}
/// Decode rows on a transaction/session frame without opening another lease.
fn session_query<M, A: Send + 'static, R: Send + Sync + 'static>(
    session: NativeSession,
    statement: impl StatementSource<M>,
    decoder: RowDecoder<A, R>,
) -> Effect<Vec<A>, SqlError, R> {
    let statement = Arc::new(statement.into_factory());
    effect_future! {
        exit_with_context; body: move |env: Arc<R>, context| {
        let session = session.try_clone_prepared();
        let statement = statement.clone();
        let decoder = decoder.clone();
        async move {
            if context.cancellation().is_requested() {
                return Exit::Interrupted;
            }
            let session = match session {
                Ok(session) => session,
                Err(error) => return Exit::Fail(SqlError::Memory(error)),
            };
            run_buffered_query(&session, statement(), &decoder, env, context).await
        }
    }, }
}
async fn run_buffered_query<A: Send + 'static, R: Send + Sync + 'static>(
    session: &NativeSession,
    statement: Result<SqlStatement, SqlError>,
    decoder: &RowDecoder<A, R>,
    env: Arc<R>,
    context: NativeContext,
) -> Exit<Vec<A>, SqlError> {
    let statement = match statement {
        Ok(statement) => statement,
        Err(error) => return error.into_exit(),
    };
    let blocks = match session.query_staged(statement).await {
        Ok(blocks) => blocks,
        Err(error) => return error.into_exit(),
    };
    if context.cancellation().is_requested() {
        return Exit::Interrupted;
    }
    let mut values = Vec::new();
    for block in blocks {
        for row in block {
            if context.cancellation().is_requested() {
                return Exit::Interrupted;
            }
            if values.try_reserve(1).is_err() {
                return Exit::Fail(SqlError::Memory(effect::memory::MemoryError::CollectionReservationFailed));
            }
            let outcome = decoder.decode(row, env.clone(), context.clone()).await.map_error(SqlError::Decode);
            match outcome.into_failure() {
                Ok(value) => values.push(value),
                Err(exit) => return exit,
            }
        }
    }
    Exit::Success(values)
}

/// Share the core Stream/Sink adapter between independent sessions and transaction frames.
fn fold_from_cursor<A, S, R>(
    acquire: Effect<CursorOwner, SqlError, R>,
    decoder: RowDecoder<A, R>,
    initial: impl Fn() -> S + Send + Sync + 'static,
    step: impl Fn(S, A) -> Effect<S, SqlError, R> + Send + Sync + 'static,
) -> Effect<S, SqlError, R>
where
    A: Send + 'static,
    S: Send + 'static,
    R: Send + Sync + 'static,
{
    let source = stream! {
        pull: acquire,
        read: move |resource| {
            let decoder = decoder.clone();
            effect_future! {
                exit_with_context; body: move |env, context| {
                let owner = resource.with(|owner| owner.cursor.clone());
                let decoder = decoder.clone();
                async move {
                    let owner = match owner {
                        Ok(owner) => owner,
                        Err(defect) => return Exit::Die(defect),
                    };
                    if context.cancellation().is_requested() {
                        return Exit::Interrupted;
                    }
                    let mut owner = owner.lock().await;
                    let cursor = match owner.as_mut() {
                        Some(cursor) => cursor,
                        None => return Exit::Fail(SqlError::InactiveSession),
                    };
                    match cursor.next_row().await {
                        Ok(Some(row)) => match decoder
                                .decode(row, env, context)
                                .await
                                .map_error(SqlError::Decode)
                                .into_failure() {
                            Ok(value) => Exit::Success(effect::Chunk::Values(vec![value])),
                            Err(exit) => exit,
                        },
                        Ok(None) => Exit::Success(effect::Chunk::End),
                        Err(error) => error.into_exit(),
                    }
                }
            }, }
        },
        release: |owner| {
            effect_resource! {
                release_exit_future;
                body: move |_| async move {
                let cursor = owner.cursor.lock().await.take();
                let cursor_result = match cursor {
                    Some(cursor) => cursor.close().await,
                    None => Ok(()),
                };
                let close = if owner.close_session {
                    owner.session.close().await
                } else {
                    Ok(())
                };
                append_cleanup(
                    match cursor_result {
                        Ok(()) => Exit::Success(()),
                        Err(error) => error.into_exit(),
                    },
                    close,
                )
                },
            }
        },
    };
    sink! {
        fold_effect;
        initial: initial,
        step: step,
    }
    .run(source)
}

/// Lower shared scoped operations to the same native session helpers for ordinary and transaction handles.
macro_rules! scoped_operations {
    ($handle:ty) => {
        impl $handle {
            /// Decode only one optional native row under this frame; NULL/type failures remain errors.
            pub fn first<M, A: Send + 'static, R: Send + Sync + 'static>(&self, statement: impl StatementSource<M>, decoder: RowDecoder<A, R>) -> Effect<Option<A>, SqlError, R> { session_first(self.session.clone(), statement, decoder) }
            /// Decode a required native scalar from ordinal zero; absent rows remain RowAbsent.
            #[expect(clippy::result_large_err, reason = "retains the owned SQL error representation without an extra heap allocation")]
            pub fn scalar<M, T, R>(&self, statement: impl StatementSource<M>) -> Effect<T, SqlError, R>
            where T: Send + 'static + Type<Turso>, for<'a> T: Decode<'a, Turso>, R: Send + Sync + 'static {
                effect! {
 bind;
 program: sql! {
    first;
    client: self,
    statement: statement,
    decoder: row! {
        project: |row| row.try_get(0).map_err(|source| crate::RowError::Native { column: "[0]".into(), source }),
    },
 },
 body: |value| match value {
    Some(value) => effect_sync! { once; body: move || value, },
    None => effect_fail! { error: SqlError::RowAbsent, },
 },
 }
            }
            /// Incrementally fold native rows without buffering; the enclosing scope retains the lease.
            pub fn fold_rows<M, A, S, R>(&self, statement: impl StatementSource<M>, decoder: RowDecoder<A, R>, initial: impl Fn() -> S + Send + Sync + 'static, step: impl Fn(S, A) -> Result<S, SqlError> + Send + Sync + 'static) -> Effect<S, SqlError, R>
            where A: Send + 'static, S: Send + 'static, R: Send + Sync + 'static {
                sql! {
                    fold_effect;
                    client: self,
                    statement: statement,
                    decoder: decoder,
                    initial: initial,
                    step: move |state, row| effect_from_result! { once; result: step(state, row), },
                }
            }
            /// Fold rows through core Stream/Sink and the selected R; row callbacks cannot reopen this live cursor frame.
            pub fn fold_rows_effect<M, A, S, R>(&self, statement: impl StatementSource<M>, decoder: RowDecoder<A, R>, initial: impl Fn() -> S + Send + Sync + 'static, step: impl Fn(S, A) -> Effect<S, SqlError, R> + Send + Sync + 'static) -> Effect<S, SqlError, R>
            where A: Send + 'static, S: Send + 'static, R: Send + Sync + 'static { session_fold(self.session.clone(), statement, decoder, initial, step) }
            /// Run ordered native statement/result sets on this frame; atomicity comes only from its enclosing transaction.
            pub fn batch<R: Send + Sync + 'static>(&self, statements: impl Fn() -> Result<Vec<SqlStatement>, SqlError> + Send + Sync + 'static) -> Effect<Vec<BatchResult>, SqlError, R> { session_batch(self.session.clone(), statements) }
        }
    };
}
scoped_operations!(SqlSession);
scoped_operations!(SqlTransaction);

impl SqlSession {
    /// Buffer typed native rows within this scoped lease without reacquiring the pool.
    pub fn query<M, A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
    ) -> Effect<Vec<A>, SqlError, R> {
        session_query(self.session.clone(), statement, decoder)
    }
    /// Move an execution-local statement once; a fresh factory is required for reusable descriptions.
    pub fn execute_once<R: Send + Sync + 'static>(
        &self,
        statement: SqlStatement,
    ) -> Effect<u64, SqlError, R> {
        sql! { execute; client: self, statement: one_statement(statement), }
    }
    /// Execute a checked SQLx/native callback under a lifetime-bound exclusive connection borrow.
    pub fn native<A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        body: impl for<'a> Fn(&'a mut sqlx::TursoConnection) -> crate::NativeFuture<'a, A>
        + Send
        + Sync
        + 'static,
    ) -> Effect<A, SqlError, R> {
        sql! {
            native;
            client: SqlTransaction { session: self.session.clone() },
            body: body,
        }
    }
}
impl SqlClient {
    /// Run an Effect under one fresh ordinary session and await its close on every complete exit.
    pub fn with_session<A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        body: impl Fn(SqlSession) -> Effect<A, SqlError, R> + Send + Sync + 'static,
    ) -> Effect<A, SqlError, R> {
        let kernel = self.kernel.clone();
        let body = Arc::new(body);
        effect_future! {
            exit_with_context; body: move |env, context| {
            let kernel = kernel.clone();
            let body = body.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let session = match kernel.session().await {
                    Ok(session) => session,
                    Err(error) => return error.into_exit(),
                };
                let outcome = match catch_unwind(AssertUnwindSafe(|| {
                    body(SqlSession {
                        session: session.clone(),
                    })
                })) {
                    Ok(program) => effect::effect! { run_in_context; context: context, program: program, environment: env, }.await,
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                };
                append_session_close(outcome, &session).await
            }
        }, }
    }
    /// Transfer a freshly acquired async native SQLx owner into a scoped Effect session.
    /// The factory may adopt a native database/connection through SQLx's real ownership APIs.
    pub fn with_native_session<A, R, F>(
        &self,
        acquire: impl Fn() -> F + Send + Sync + 'static,
        body: impl Fn(SqlSession) -> Effect<A, SqlError, R> + Send + Sync + 'static,
    ) -> Effect<A, SqlError, R>
    where
        A: Send + 'static,
        R: Send + Sync + 'static,
        F: Future<Output = Result<sqlx::TursoConnection, SqlError>> + Send + 'static,
    {
        let acquire = Arc::new(acquire);
        let body = Arc::new(body);
        effect_future! {
            exit_with_context; body: move |env, context| {
            let acquire = acquire.clone();
            let body = body.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let session = match acquire().await {
                    Ok(connection) => NativeSession::adopt(connection),
                    Err(error) => return error.into_exit(),
                };
                let outcome = match catch_unwind(AssertUnwindSafe(|| {
                    body(SqlSession {
                        session: session.clone(),
                    })
                })) {
                    Ok(program) => effect::effect! { run_in_context; context: context, program: program, environment: env, }.await,
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                };
                append_session_close(outcome, &session).await
            }
        }, }
    }
}
impl Sql {
    /// Read a required scalar from a factory or transferred input through the selected capability.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub fn scalar<M, T, R>(statement: impl StatementSource<M>) -> Effect<T, SqlError, R>
    where
        T: Send + 'static + Type<Turso>,
        for<'a> T: Decode<'a, Turso>,
        R: HasSqlClient + Send + Sync + 'static,
    {
        let statement = Arc::new(statement.into_factory());
        effect! {
        bind;
        program: service! { env: |env: &R| env.sql_client().clone(), },
        body: move |client| {
                   let statement = statement.clone();
                   sql! { scalar; client: client, statement: move || statement(), }
               },
        }
    }
}

/// Read one optional native row on a scoped frame while preserving original validation context.
fn session_first<M, A: Send + 'static, R: Send + Sync + 'static>(
    session: NativeSession,
    statement: impl StatementSource<M>,
    decoder: RowDecoder<A, R>,
) -> Effect<Option<A>, SqlError, R> {
    let statement = Arc::new(statement.into_factory());
    effect_future! {
        exit_with_context; body: move |env, context| {
        let session = session.try_clone_prepared();
        let statement = statement.clone();
        let decoder = decoder.clone();
        async move {
            if context.cancellation().is_requested() {
                return Exit::Interrupted;
            }
            let session = match session {
                Ok(session) => session,
                Err(error) => return Exit::Fail(SqlError::Memory(error)),
            };
            let statement = match statement() {
                Ok(statement) => statement,
                Err(error) => return error.into_exit(),
            };
            match session.first(statement).await {
                Ok(None) if context.cancellation().is_requested() => Exit::Interrupted,
                Ok(None) => Exit::Success(None),
                Ok(Some(row)) => match decoder
                        .decode(row, env, context)
                        .await
                        .map_error(SqlError::Decode)
                        .into_failure() {
                    Ok(value) => Exit::Success(Some(value)),
                    Err(exit) => exit,
                },
                Err(error) => error.into_exit(),
            }
        }
    }, }
}
/// Acquire only a cursor for a retained frame; core Stream cleanup releases it before parent settlement.
fn session_fold<M, A, S, R>(
    session: NativeSession,
    statement: impl StatementSource<M>,
    decoder: RowDecoder<A, R>,
    initial: impl Fn() -> S + Send + Sync + 'static,
    step: impl Fn(S, A) -> Effect<S, SqlError, R> + Send + Sync + 'static,
) -> Effect<S, SqlError, R>
where
    A: Send + 'static,
    S: Send + 'static,
    R: Send + Sync + 'static,
{
    let statement = Arc::new(statement.into_factory());
    let acquire = effect_future! {
        exit_with_context; body: move |_, context| {
        let session = session.try_clone_prepared();
        let statement = statement.clone();
        async move {
            if context.cancellation().is_requested() {
                return Exit::Interrupted;
            }
            let session = match session {
                Ok(session) => session,
                Err(error) => return Exit::Fail(SqlError::Memory(error)),
            };
            let statement = match statement() {
                Ok(statement) => statement,
                Err(error) => return error.into_exit(),
            };
            match session.cursor(statement).await {
                Ok(cursor) => Exit::Success(CursorOwner {
                    cursor: Arc::new(tokio::sync::Mutex::new(Some(cursor))),
                    session,
                    close_session: false,
                }),
                Err(error) => error.into_exit(),
            }
        }
    }, };
    fold_from_cursor(acquire, decoder, initial, step)
}
/// Execute ordered batch items without another lease and with cancellation between native result steps.
fn session_batch<R: Send + Sync + 'static>(
    session: NativeSession,
    statements: impl Fn() -> Result<Vec<SqlStatement>, SqlError> + Send + Sync + 'static,
) -> Effect<Vec<BatchResult>, SqlError, R> {
    let statements = Arc::new(statements);
    effect_future! {
        exit_with_context; body: move |_, context| {
        let session = session.try_clone_prepared();
        let statements = statements.clone();
        async move {
            if context.cancellation().is_requested() {
                return Exit::Interrupted;
            }
            let session = match session {
                Ok(session) => session,
                Err(error) => return Exit::Fail(SqlError::Memory(error)),
            };
            let statements = match statements() {
                Ok(statements) => statements,
                Err(error) => return error.into_exit(),
            };
            let mut results = Vec::new();
            if results.try_reserve_exact(statements.len()).is_err() {
                return Exit::Fail(SqlError::Memory(effect::memory::MemoryError::CollectionReservationFailed));
            }
            for statement in statements {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let mut cursor = match session.cursor(statement).await {
                    Ok(cursor) => cursor,
                    Err(error) => return error.into_exit(),
                };
                let mut items = Ok(Vec::new());
                loop {
                    if context.cancellation().is_requested() {
                        let cleanup = cursor.close().await;
                        return append_cleanup(Exit::Interrupted, cleanup);
                    }
                    match cursor.next_item().await {
                        Ok(Some(item)) => {
                            if let Ok(buffer) = &mut items {
                                if buffer.try_reserve(1).is_ok() {
                                    buffer.push(item);
                                } else {
                                    items = Err(effect::memory::MemoryError::CollectionReservationFailed);
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            return append_cleanup(error.into_exit(), cursor.close().await);
                        }
                    }
                }
                if let Err(error) = cursor.close().await {
                    return error.into_exit();
                }
                match items {
                    Ok(items) => results.push(BatchResult { items }),
                    Err(error) => return Exit::Fail(SqlError::Memory(error)),
                }
            }
            Exit::Success(results)
        }
    }, }
}
