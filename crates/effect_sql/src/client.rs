//! Lazy Effect SQL adapters over the shared native async kernel.

use crate::{NativeSession, RowDecoder, SqlError, SqlKernel, SqlStatement};
use effect::{CleanupFailure, Defect, Effect, Exit, Finalization, NativeContext, PrimaryOutcome};
use futures_util::FutureExt;
use sqlx::{Decode, Row, Turso, TursoPool, Type};
use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
};

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

/// Immutable SQL service configuration; each run acquires a fresh native execution owner.
#[derive(Clone)]
#[must_use]
pub struct SqlClient {
    /// Shared native kernel; cloned clients share configuration, not leases.
    kernel: SqlKernel,
}
impl HasSqlClient for SqlClient {
    /// Select this explicit SQL environment.
    fn sql_client(&self) -> &SqlClient {
        self
    }
}
impl SqlClient {
    /// Select a caller-owned local SQLx pool. Construction is inert and never creates a runtime.
    pub fn new(pool: TursoPool) -> Self {
        Self {
            kernel: SqlKernel::new(pool),
        }
    }
    /// Retain the shared native kernel for Identity store trait calls and checked SQLx use.
    #[must_use]
    pub fn kernel(&self) -> &SqlKernel {
        &self.kernel
    }
    /// Execute a fresh statement factory per run, or transfer one owned statement/Result once.
    pub fn execute<M, R>(&self, statement: impl StatementSource<M>) -> Effect<u64, SqlError, R>
    where
        R: Send + Sync + 'static,
    {
        let kernel = self.kernel.clone();
        let statement = Arc::new(statement.into_factory());
        Effect::from_exit_future_with_context(move |_, context| {
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
        })
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
        Effect::from_exit_future_with_context(move |env: Arc<R>, context| {
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
                    let rows = match statement().and_then_result_query(&session).await {
                        Ok(rows) => rows,
                        Err(error) => return error.into_exit(),
                    };
                    if context.cancellation().is_requested() {
                        return Exit::Interrupted;
                    }
                    let mut values = Vec::with_capacity(rows.len());
                    for row in rows {
                        match crate::row::failure_as(
                            decoder
                                .decode(row, env.clone(), context.clone())
                                .await
                                .map_error(SqlError::Decode),
                        ) {
                            Ok(value) => values.push(value),
                            Err(exit) => return exit,
                        }
                    }
                    Exit::Success(values)
                })
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                append_cleanup(work, session.close().await)
            }
        })
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
        Effect::from_exit_future_with_context(move |env, context| {
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
                        Ok(Some(row)) => match crate::row::failure_as::<_, Option<A>, _>(
                            decoder
                                .decode(row, env, context.clone())
                                .await
                                .map_error(SqlError::Decode),
                        ) {
                            Ok(value) => Exit::Success(Some(value)),
                            Err(exit) => exit,
                        },
                        Err(error) => error.into_exit(),
                    }
                })
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                append_cleanup(work, session.close().await)
            }
        })
    }
    /// Read a required scalar from ordinal zero through SQLx native Type/Decode.
    pub fn scalar<M, T, R>(&self, statement: impl StatementSource<M>) -> Effect<T, SqlError, R>
    where
        T: Send + 'static + Type<Turso>,
        for<'a> T: Decode<'a, Turso>,
        R: Send + Sync + 'static,
    {
        self.first(
            statement,
            RowDecoder::pure(|row| {
                row.try_get(0).map_err(|source| crate::RowError::Native {
                    column: "[0]".into(),
                    source,
                })
            }),
        )
        .flat_map(|value| match value {
            Some(value) => once_value(value),
            None => Effect::fail_with(|| SqlError::RowAbsent),
        })
    }
    /// Execute an ordered batch of bound statements on one lease; it is not implicitly transactional.
    /// Each statement retains its own row/result sequence, including provider-supported multi-results.
    pub fn batch<R>(
        &self,
        statements: impl Fn() -> Result<Vec<SqlStatement>, SqlError> + Send + Sync + 'static,
    ) -> Effect<Vec<BatchResult>, SqlError, R>
    where
        R: Send + Sync + 'static,
    {
        let kernel = self.kernel.clone();
        let statements = Arc::new(statements);
        Effect::from_exit_future_with_context(move |env, context| {
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
                let outcome = AssertUnwindSafe(context.run(program, env))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                append_cleanup(outcome, session.close().await)
            }
        })
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
        self.fold_rows_effect(statement, decoder, initial, move |state, value| {
            once_result(step(state, value))
        })
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
        let acquire = Effect::from_exit_future_with_context(move |_, context| {
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
                    return append_cleanup(Exit::Interrupted, session.close().await);
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
                    Ok(Err(error)) => append_cleanup(error.into_exit(), session.close().await),
                    Err(panic) => {
                        append_cleanup(Exit::Die(Defect::from_panic(panic)), session.close().await)
                    }
                }
            }
        });
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
        Effect::from_exit_future_with_context(move |env, context| {
            let kernel = kernel.clone();
            let body = body.clone();
            async move {
                if let Err(error) = kernel.require_authoritative() {
                    return error.into_exit();
                }
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let session = match kernel.session().await {
                    Ok(session) => session,
                    Err(error) => return error.into_exit(),
                };
                let tx = match session.begin().await {
                    Ok(tx) => tx,
                    Err(error) => return append_cleanup(error.into_exit(), session.close().await),
                };
                let outcome = match catch_unwind(AssertUnwindSafe(|| {
                    body(SqlTransaction {
                        session: tx.clone(),
                    })
                })) {
                    Ok(program) => context.run(program, env).await,
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                };
                let outcome = match outcome {
                    Exit::Success(value) => match tx.commit().await {
                        Ok(()) => Exit::Success(value),
                        Err(error) => error.into_exit(),
                    },
                    failure @ (Exit::Fail(_)
                    | Exit::Die(_)
                    | Exit::Interrupted
                    | Exit::FinalizationFailed(_)) => append_cleanup(failure, tx.rollback().await),
                };
                append_cleanup(outcome, session.close().await)
            }
        })
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
        Effect::from_exit_future_with_context(move |env, context| {
            let client = client.clone();
            let body = body.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let body = body
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                let Some(body) = body else {
                    return Exit::Fail(SqlError::Consumed);
                };
                let body = Mutex::new(Some(body));
                context
                    .run(
                        client.transaction(move |tx| {
                            match body
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .take()
                            {
                                Some(body) => body(tx),
                                None => Effect::fail_with(|| SqlError::Consumed),
                            }
                        }),
                        env,
                    )
                    .await
            }
        })
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
        Effect::from_exit_future_with_context(move |env, context| {
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
                    Ok(program) => context.run(program, env).await,
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                };
                append_cleanup(outcome, session.close().await)
            }
        })
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
        self.execute(one_statement(statement))
    }
    /// Query decoded rows on the current frame with the original environment for validations.
    pub fn query<M, A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
    ) -> Effect<Vec<A>, SqlError, R> {
        session_query(self.session.clone(), statement, decoder)
    }
    /// Run a nested savepoint and join rollback/release on any body failure.
    pub fn savepoint<A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        body: impl Fn(SqlTransaction) -> Effect<A, SqlError, R> + Send + Sync + 'static,
    ) -> Effect<A, SqlError, R> {
        let session = self.session.clone();
        let body = Arc::new(body);
        Effect::from_exit_future_with_context(move |env, context| {
            let session = session.clone();
            let body = body.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let nested = match session.begin_savepoint().await {
                    Ok(nested) => nested,
                    Err(error) => return error.into_exit(),
                };
                let outcome = match catch_unwind(AssertUnwindSafe(|| {
                    body(SqlTransaction {
                        session: nested.clone(),
                    })
                })) {
                    Ok(program) => context.run(program, env).await,
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                };
                match outcome {
                    Exit::Success(value) => match nested.commit().await {
                        Ok(()) => Exit::Success(value),
                        Err(error) => error.into_exit(),
                    },
                    failure @ (Exit::Fail(_)
                    | Exit::Die(_)
                    | Exit::Interrupted
                    | Exit::FinalizationFailed(_)) => {
                        append_cleanup(failure, nested.rollback().await)
                    }
                }
            }
        })
    }
    /// Transfer one savepoint callback and its captures without a hidden Clone requirement.
    /// Its one-use ownership delegates the same checked frame and settlement path.
    pub fn savepoint_once<A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        body: impl FnOnce(SqlTransaction) -> Effect<A, SqlError, R> + Send + 'static,
    ) -> Effect<A, SqlError, R> {
        let transaction = self.clone();
        let body = Arc::new(Mutex::new(Some(body)));
        Effect::from_exit_future_with_context(move |env, context| {
            let transaction = transaction.clone();
            let body = body.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let body = body
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                let Some(body) = body else {
                    return Exit::Fail(SqlError::Consumed);
                };
                let body = Mutex::new(Some(body));
                context
                    .run(
                        transaction.savepoint(move |tx| {
                            match body
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .take()
                            {
                                Some(body) => body(tx),
                                None => Effect::fail_with(|| SqlError::Consumed),
                            }
                        }),
                        env,
                    )
                    .await
            }
        })
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
        Effect::from_exit_future_with_context(move |_, context| {
            let session = session.clone();
            let body = body.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                match session.with_native(|connection| body(connection)).await {
                    Ok(value) => Exit::Success(value),
                    Err(error) => error.into_exit(),
                }
            }
        })
    }
}

/// Environment-selected SQL operations used directly inside sql_with!/effect! bodies.
pub struct Sql;
impl Sql {
    /// Execute a factory or transferred statement/Result in the selected client's independent session.
    pub fn execute<M, R: HasSqlClient + Send + Sync + 'static>(
        statement: impl StatementSource<M>,
    ) -> Effect<u64, SqlError, R> {
        let statement = Arc::new(statement.into_factory());
        Effect::service(|env: &R| env.sql_client().clone()).flat_map(move |client| {
            let statement = statement.clone();
            client.execute(move || statement())
        })
    }
    /// Decode a fresh or transferred statement using the selected client and original R.
    pub fn query<M, A: Send + 'static, R: HasSqlClient + Send + Sync + 'static>(
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
    ) -> Effect<Vec<A>, SqlError, R> {
        let statement = Arc::new(statement.into_factory());
        Effect::service(|env: &R| env.sql_client().clone()).flat_map(move |client| {
            let statement = statement.clone();
            client.query(move || statement(), decoder.clone())
        })
    }
    /// Decode one optional row from a factory or owned input; only true absence becomes None.
    pub fn first<M, A: Send + 'static, R: HasSqlClient + Send + Sync + 'static>(
        statement: impl StatementSource<M>,
        decoder: RowDecoder<A, R>,
    ) -> Effect<Option<A>, SqlError, R> {
        let statement = Arc::new(statement.into_factory());
        Effect::service(|env: &R| env.sql_client().clone()).flat_map(move |client| {
            let statement = statement.clone();
            client.first(move || statement(), decoder.clone())
        })
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
    program.provide_some(move |env| select(env).borrow().clone())
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
    append_cleanup(outcome, session.close().await)
}
/// Append an ordered cleanup failure while retaining the original error/defect/interruption.
pub(crate) fn append_cleanup<A>(
    mut outcome: Exit<A, SqlError>,
    cleanup: Result<(), SqlError>,
) -> Exit<A, SqlError> {
    let Err(error) = cleanup else {
        return outcome;
    };
    let failures = match error.into_exit::<()>() {
        Exit::Fail(error) => vec![CleanupFailure::ReleaseFailure(error)],
        Exit::Die(defect) => vec![CleanupFailure::ReleaseDefect(defect)],
        Exit::Interrupted => vec![CleanupFailure::ReleaseInterrupted],
        Exit::Success(()) => return outcome,
        Exit::FinalizationFailed(details) => {
            let mut failures = Vec::new();
            match details.primary {
                PrimaryOutcome::Succeeded => {}
                PrimaryOutcome::Failed(error) => {
                    failures.push(CleanupFailure::ReleaseFailure(error))
                }
                PrimaryOutcome::Defected(defect) => {
                    failures.push(CleanupFailure::ReleaseDefect(defect))
                }
                PrimaryOutcome::WasInterrupted => failures.push(CleanupFailure::ReleaseInterrupted),
            }
            failures.push(details.first);
            failures.extend(details.rest);
            failures
        }
    };
    for failure in failures {
        outcome = match outcome {
            Exit::FinalizationFailed(mut details) => {
                details.rest.push(failure);
                Exit::FinalizationFailed(details)
            }
            Exit::Success(_) => Exit::FinalizationFailed(Finalization {
                primary: PrimaryOutcome::Succeeded,
                first: failure,
                rest: Vec::new(),
            }),
            Exit::Fail(error) => Exit::FinalizationFailed(Finalization {
                primary: PrimaryOutcome::Failed(error),
                first: failure,
                rest: Vec::new(),
            }),
            Exit::Die(defect) => Exit::FinalizationFailed(Finalization {
                primary: PrimaryOutcome::Defected(defect),
                first: failure,
                rest: Vec::new(),
            }),
            Exit::Interrupted => Exit::FinalizationFailed(Finalization {
                primary: PrimaryOutcome::WasInterrupted,
                first: failure,
                rest: Vec::new(),
            }),
        };
    }
    outcome
}
/// Build a replay-safe factory around one explicitly transferred execution payload.
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
/// Lift one moved value into core's existing single-use workflow rather than cloning it.
fn once_value<A: Send + 'static, R: Send + Sync + 'static>(value: A) -> Effect<A, SqlError, R> {
    Effect::sync_once(move || value)
}
/// Lift an explicit Result without interpreting arbitrary Result ? as an Effect bind.
fn once_result<A: Send + 'static, R: Send + Sync + 'static>(
    result: Result<A, SqlError>,
) -> Effect<A, SqlError, R> {
    Effect::from_result_once(result)
}
/// Execute one fresh statement on a scoped frame with cooperative cancellation at boundaries.
fn session_execute<M, R: Send + Sync + 'static>(
    session: NativeSession,
    statement: impl StatementSource<M>,
) -> Effect<u64, SqlError, R> {
    let statement = Arc::new(statement.into_factory());
    Effect::from_exit_future_with_context(move |_, context| {
        let session = session.clone();
        let statement = statement.clone();
        async move {
            if context.cancellation().is_requested() {
                return Exit::Interrupted;
            }
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
    })
}
/// Decode rows on a transaction/session frame without opening another lease.
fn session_query<M, A: Send + 'static, R: Send + Sync + 'static>(
    session: NativeSession,
    statement: impl StatementSource<M>,
    decoder: RowDecoder<A, R>,
) -> Effect<Vec<A>, SqlError, R> {
    let statement = Arc::new(statement.into_factory());
    Effect::from_exit_future_with_context(move |env: Arc<R>, context| {
        let session = session.clone();
        let statement = statement.clone();
        let decoder = decoder.clone();
        async move {
            if context.cancellation().is_requested() {
                return Exit::Interrupted;
            }
            let statement = match statement() {
                Ok(statement) => statement,
                Err(error) => return error.into_exit(),
            };
            let rows = match session.query(statement).await {
                Ok(rows) => rows,
                Err(error) => return error.into_exit(),
            };
            if context.cancellation().is_requested() {
                return Exit::Interrupted;
            }
            let mut values = Vec::with_capacity(rows.len());
            for row in rows {
                match crate::row::failure_as(
                    decoder
                        .decode(row, env.clone(), context.clone())
                        .await
                        .map_error(SqlError::Decode),
                ) {
                    Ok(value) => values.push(value),
                    Err(exit) => return exit,
                }
            }
            Exit::Success(values)
        }
    })
}
/// A private convenience retaining the original statement error before query I/O.
trait StatementQuery {
    /// Await native query only when inert statement validation succeeded.
    async fn and_then_result_query(
        self,
        session: &NativeSession,
    ) -> Result<Vec<sqlx::TursoRow>, SqlError>;
}
impl StatementQuery for Result<SqlStatement, SqlError> {
    /// Preserve construction failure precedence.
    async fn and_then_result_query(
        self,
        session: &NativeSession,
    ) -> Result<Vec<sqlx::TursoRow>, SqlError> {
        session.query(self?).await
    }
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
    let source = effect::Stream::from_pull(
        acquire,
        move |resource| {
            let decoder = decoder.clone();
            Effect::from_exit_future_with_context(move |env, context| {
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
                        Ok(Some(row)) => match crate::row::failure_as(
                            decoder
                                .decode(row, env, context)
                                .await
                                .map_error(SqlError::Decode),
                        ) {
                            Ok(value) => Exit::Success(effect::Chunk::Values(vec![value])),
                            Err(exit) => exit,
                        },
                        Ok(None) => Exit::Success(effect::Chunk::End),
                        Err(error) => error.into_exit(),
                    }
                }
            })
        },
        |owner| {
            effect::Release::from_exit_future(move |_| async move {
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
            })
        },
    );
    effect::Sink::fold(initial, step).run(source)
}

/// Lower shared scoped operations to the same native session helpers for ordinary and transaction handles.
macro_rules! scoped_operations {
    ($handle:ty) => {
        impl $handle {
            /// Decode only one optional native row under this frame; NULL/type failures remain errors.
            pub fn first<M, A: Send + 'static, R: Send + Sync + 'static>(&self, statement: impl StatementSource<M>, decoder: RowDecoder<A, R>) -> Effect<Option<A>, SqlError, R> { session_first(self.session.clone(), statement, decoder) }
            /// Decode a required native scalar from ordinal zero; absent rows remain RowAbsent.
            pub fn scalar<M, T, R>(&self, statement: impl StatementSource<M>) -> Effect<T, SqlError, R>
            where T: Send + 'static + Type<Turso>, for<'a> T: Decode<'a, Turso>, R: Send + Sync + 'static {
                self.first(statement, RowDecoder::pure(|row| row.try_get(0).map_err(|source| crate::RowError::Native { column: "[0]".into(), source }))).flat_map(|value| match value { Some(value) => once_value(value), None => Effect::fail_with(|| SqlError::RowAbsent) })
            }
            /// Incrementally fold native rows without buffering; the enclosing scope retains the lease.
            pub fn fold_rows<M, A, S, R>(&self, statement: impl StatementSource<M>, decoder: RowDecoder<A, R>, initial: impl Fn() -> S + Send + Sync + 'static, step: impl Fn(S, A) -> Result<S, SqlError> + Send + Sync + 'static) -> Effect<S, SqlError, R>
            where A: Send + 'static, S: Send + 'static, R: Send + Sync + 'static { self.fold_rows_effect(statement, decoder, initial, move |state, row| once_result(step(state, row))) }
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
        self.execute(one_statement(statement))
    }
    /// Execute a checked SQLx/native callback under a lifetime-bound exclusive connection borrow.
    pub fn native<A: Send + 'static, R: Send + Sync + 'static>(
        &self,
        body: impl for<'a> Fn(&'a mut sqlx::TursoConnection) -> crate::NativeFuture<'a, A>
        + Send
        + Sync
        + 'static,
    ) -> Effect<A, SqlError, R> {
        SqlTransaction {
            session: self.session.clone(),
        }
        .native(body)
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
        Effect::from_exit_future_with_context(move |env, context| {
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
                    Ok(program) => context.run(program, env).await,
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                };
                append_cleanup(outcome, session.close().await)
            }
        })
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
        Effect::from_exit_future_with_context(move |env, context| {
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
                    Ok(program) => context.run(program, env).await,
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                };
                append_cleanup(outcome, session.close().await)
            }
        })
    }
}
impl Sql {
    /// Read a required scalar from a factory or transferred input through the selected capability.
    pub fn scalar<M, T, R>(statement: impl StatementSource<M>) -> Effect<T, SqlError, R>
    where
        T: Send + 'static + Type<Turso>,
        for<'a> T: Decode<'a, Turso>,
        R: HasSqlClient + Send + Sync + 'static,
    {
        let statement = Arc::new(statement.into_factory());
        Effect::service(|env: &R| env.sql_client().clone()).flat_map(move |client| {
            let statement = statement.clone();
            client.scalar(move || statement())
        })
    }
}

/// Read one optional native row on a scoped frame while preserving original validation context.
fn session_first<M, A: Send + 'static, R: Send + Sync + 'static>(
    session: NativeSession,
    statement: impl StatementSource<M>,
    decoder: RowDecoder<A, R>,
) -> Effect<Option<A>, SqlError, R> {
    let statement = Arc::new(statement.into_factory());
    Effect::from_exit_future_with_context(move |env, context| {
        let session = session.clone();
        let statement = statement.clone();
        let decoder = decoder.clone();
        async move {
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
                Ok(Some(row)) => match crate::row::failure_as(
                    decoder
                        .decode(row, env, context)
                        .await
                        .map_error(SqlError::Decode),
                ) {
                    Ok(value) => Exit::Success(Some(value)),
                    Err(exit) => exit,
                },
                Err(error) => error.into_exit(),
            }
        }
    })
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
    let acquire = Effect::from_exit_future_with_context(move |_, context| {
        let session = session.clone();
        let statement = statement.clone();
        async move {
            if context.cancellation().is_requested() {
                return Exit::Interrupted;
            }
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
    });
    fold_from_cursor(acquire, decoder, initial, step)
}
/// Execute ordered batch items without another lease and with cancellation between native result steps.
fn session_batch<R: Send + Sync + 'static>(
    session: NativeSession,
    statements: impl Fn() -> Result<Vec<SqlStatement>, SqlError> + Send + Sync + 'static,
) -> Effect<Vec<BatchResult>, SqlError, R> {
    let statements = Arc::new(statements);
    Effect::from_exit_future_with_context(move |_, context| {
        let session = session.clone();
        let statements = statements.clone();
        async move {
            if context.cancellation().is_requested() {
                return Exit::Interrupted;
            }
            let statements = match statements() {
                Ok(statements) => statements,
                Err(error) => return error.into_exit(),
            };
            let mut results = Vec::new();
            for statement in statements {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let mut cursor = match session.cursor(statement).await {
                    Ok(cursor) => cursor,
                    Err(error) => return error.into_exit(),
                };
                let mut items = Vec::new();
                loop {
                    if context.cancellation().is_requested() {
                        let cleanup = cursor.close().await;
                        return append_cleanup(Exit::Interrupted, cleanup);
                    }
                    match cursor.next_item().await {
                        Ok(Some(item)) => items.push(item),
                        Ok(None) => break,
                        Err(error) => {
                            return append_cleanup(error.into_exit(), cursor.close().await);
                        }
                    }
                }
                if let Err(error) = cursor.close().await {
                    return error.into_exit();
                }
                results.push(BatchResult { items });
            }
            Exit::Success(results)
        }
    })
}
