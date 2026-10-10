//! Native async SQL kernel shared by Effect and native Identity stores.
//!
//! No method creates or invokes an Effect runtime. Tokio is the selected local
//! SQLx provider's reactor; every operation has an exclusive connection owner.

use crate::{SqlError, SqlStatement};
use futures_util::{FutureExt, StreamExt, stream::BoxStream};
use sqlx::{
    Arguments, AssertSqlSafe, Connection, SqlSafeStr, Turso, TursoConnection, TursoPool,
    TursoQueryResult, TursoRow,
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Exact native result sequence retained by an owned cursor.
type CursorItems =
    BoxStream<'static, Result<either::Either<TursoQueryResult, TursoRow>, sqlx::Error>>;

/// Native callback borrowing one exclusive connection; its output cannot borrow the lease.
pub type NativeFuture<'a, A> = Pin<Box<dyn Future<Output = Result<A, SqlError>> + Send + 'a>>;

/// A scoped native operation that may retain borrowed credentials or payload slices.
///
/// The future may borrow both the connection and the operation's inputs. `Self: 'a`
/// limits that borrow to the caller's real input lifetime; no `'static` capture,
/// payload copy, blanket Clone or nested Effect runtime is required. Implementors
/// own their SQLx statements while the kernel owns transaction settlement.
pub trait NativeOperation<A: Send>: Send + Sized {
    /// Consume this invocation under one exclusive connection borrow.
    fn run<'a>(
        self,
        connection: &'a mut TursoConnection,
    ) -> impl Future<Output = Result<A, SqlError>> + Send + 'a
    where
        Self: 'a,
        A: 'a;

    /// Erase the operation future at the lease lifetime before entering settlement.
    /// Existing implementations retain their scoped future and borrowed inputs.
    fn run_boxed<'a>(self, connection: &'a mut TursoConnection) -> NativeFuture<'a, A>
    where
        Self: 'a,
        A: 'a,
    {
        Box::pin(self.run(connection))
    }
}

/// Adapt the convenient owned-capture closure form to the same scoped operation path.
struct ClosureOperation<F>(F);
impl<A: Send, F> NativeOperation<A> for ClosureOperation<F>
where
    F: for<'a> FnOnce(&'a mut TursoConnection) -> NativeFuture<'a, A> + Send,
{
    /// Invoke the closure exactly once; the kernel still retains the cleanup owner.
    fn run<'a>(
        self,
        connection: &'a mut TursoConnection,
    ) -> impl Future<Output = Result<A, SqlError>> + Send + 'a
    where
        Self: 'a,
        A: 'a,
    {
        (self.0)(connection)
    }
}

/// Authoritative local shared SQLx pool. Native stores use operation-owned transactions.
#[derive(Clone)]
#[must_use]
pub struct SqlKernel {
    /// Shared native pool; no additional concurrency gate is created.
    pool: TursoPool,
    /// Private configuration identity retained by maintenance grants.
    identity: Arc<()>,
}
impl SqlKernel {
    /// Select a pool. Replicas may perform reads but authoritative operations reject their topology.
    pub fn new(pool: TursoPool) -> Self {
        Self {
            pool,
            identity: Arc::new(()),
        }
    }
    /// Borrow the exact SQLx pool for checked macros and independently owned native store calls.
    #[must_use]
    pub fn pool(&self) -> &TursoPool {
        &self.pool
    }
    /// Check selected configuration identity without publishing forgeable connection authority.
    #[must_use]
    pub fn same_configuration(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
    /// Require embedded local write authority, rejecting read-only and synchronized configurations.
    pub fn require_authoritative(&self) -> Result<(), SqlError> {
        let options = self.pool.connect_options();
        if options.sync_options().is_some() {
            return Err(SqlError::ReplicaNotAuthoritative);
        }
        if options.is_read_only() {
            return Err(SqlError::Configuration(
                "read-only pool cannot own writes".into(),
            ));
        }
        Ok(())
    }
    /// Acquire one fresh exclusive operation owner through native pool admission.
    pub async fn session(&self) -> Result<NativeSession, SqlError> {
        let connection = self.pool.acquire().await?;
        Ok(NativeSession::new(ConnectionOwner::Pooled(connection)))
    }
    /// Execute a native callback on an independent pool lease, retaining static SQLx interoperability.
    pub fn with_connection<'op, A: Send + 'op, F>(&'op self, body: F) -> NativeFuture<'op, A>
    where
        F: for<'a> FnOnce(&'a mut TursoConnection) -> NativeFuture<'a, A> + Send + 'op,
    {
        self.connection_operation(ClosureOperation(body))
    }
    /// Execute borrowed-input native work on one fresh lease without copying its inputs.
    pub fn connection_operation<'op, A: Send + 'op, O: NativeOperation<A> + 'op>(
        &'op self,
        operation: O,
    ) -> NativeFuture<'op, A> {
        Box::pin(async move {
            let mut connection = self.pool.acquire().await?;
            connection.close_on_drop();
            let outcome =
                std::panic::AssertUnwindSafe(async { operation.run_boxed(&mut connection).await })
                    .catch_unwind()
                    .await;
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(panic) => Err(SqlError::NativeInvocation(Box::new(effect::Exit::Die(
                    effect::Defect::from_panic(panic),
                )))),
            };
            join_cleanup(outcome, finish_pool(connection).await)
        })
    }
    /// Execute one operation-owned BEGIN IMMEDIATE transaction and await commit or rollback.
    ///
    /// Cancellation by dropping this native future retires its lease. Commit errors
    /// are uncertain and never retried. The body borrows SQLx directly and may use
    /// checked query macros; it never enrolls in an ambient application transaction.
    /// The boxed Send future fixes this callback boundary's lifetime independently
    /// of callers lifting a whole operation into an Effect.
    pub fn transaction<'k, A: Send + 'k, F>(&'k self, body: F) -> NativeFuture<'k, A>
    where
        F: for<'a> FnOnce(&'a mut TursoConnection) -> NativeFuture<'a, A> + Send + 'k,
    {
        self.transaction_operation(ClosureOperation(body))
    }
    /// Execute one operation-owned BEGIN IMMEDIATE transaction for borrowed native inputs.
    /// This is the same native settlement path used by the closure API. The kernel
    /// retains exclusive ownership through commit/rollback and acknowledged close;
    /// transaction leases are explicitly closed even after confirmed settlement,
    /// and unresolved acknowledgment is never retried as success.
    pub fn transaction_operation<'op, A: Send + 'op, O: NativeOperation<A> + 'op>(
        &'op self,
        operation: O,
    ) -> NativeFuture<'op, A> {
        Box::pin(async move {
            self.require_authoritative()?;
            let mut connection = self.pool.acquire().await?;
            // SQLx has no clear-close-on-drop operation. Transaction leases are
            // deliberately retired even after success so any abandoned transition
            // can never return unresolved state to another pool borrower.
            connection.close_on_drop();
            if let Err(error) = begin_native(&mut connection, true).await {
                return join_cleanup(Err(error), finish_pool(connection).await);
            }
            let result =
                std::panic::AssertUnwindSafe(async { operation.run_boxed(&mut connection).await })
                    .catch_unwind()
                    .await;
            let outcome = match result {
                Ok(Ok(value)) => match commit_native(&mut connection, true).await {
                    Ok(()) => Ok(value),
                    Err(primary) => match rollback_native(&mut connection).await {
                        Ok(()) => Err(primary),
                        Err(cleanup) => Err(SqlError::Cleanup {
                            primary: Some(Box::new(primary)),
                            cleanup: Box::new(cleanup),
                        }),
                    },
                },
                Ok(Err(primary)) => match rollback_native(&mut connection).await {
                    Ok(()) => Err(primary),
                    Err(cleanup) => Err(SqlError::Cleanup {
                        primary: Some(Box::new(primary)),
                        cleanup: Box::new(cleanup),
                    }),
                },
                Err(panic) => {
                    let cleanup = rollback_native(&mut connection).await;
                    Err(SqlError::NativeInvocation(Box::new(
                        crate::client::append_cleanup(
                            effect::Exit::Die(effect::Defect::from_panic(panic)),
                            cleanup,
                        ),
                    )))
                }
            };
            let close = finish_pool(connection).await;
            match (outcome, close) {
                (Ok(value), Ok(())) => Ok(value),
                (Err(error), Ok(())) => Err(error),
                (Ok(_), Err(cleanup)) => Err(SqlError::Cleanup {
                    primary: None,
                    cleanup: Box::new(cleanup),
                }),
                (Err(primary), Err(cleanup)) => Err(SqlError::Cleanup {
                    primary: Some(Box::new(primary)),
                    cleanup: Box::new(cleanup),
                }),
            }
        })
    }

    /// Borrow an existing SQLx owner for a lifetime-bound native callback without disposing it.
    /// The Rust mutable borrow excludes simultaneous use and prevents escaping native transactions.
    pub fn with_borrowed<'op, A: Send + 'op, F>(
        connection: &'op mut TursoConnection,
        body: F,
    ) -> NativeFuture<'op, A>
    where
        F: for<'a> FnOnce(&'a mut TursoConnection) -> NativeFuture<'a, A> + Send + 'op,
    {
        Self::borrowed_operation(connection, ClosureOperation(body))
    }
    /// Borrow an existing native owner for input-scoped work; its owner remains responsible for close.
    pub fn borrowed_operation<'op, A: Send + 'op, O: NativeOperation<A> + 'op>(
        connection: &'op mut TursoConnection,
        operation: O,
    ) -> NativeFuture<'op, A> {
        Box::pin(async move {
            match std::panic::AssertUnwindSafe(async { operation.run_boxed(connection).await })
                .catch_unwind()
                .await
            {
                Ok(result) => result,
                Err(panic) => Err(SqlError::NativeInvocation(Box::new(effect::Exit::Die(
                    effect::Defect::from_panic(panic),
                )))),
            }
        })
    }
}

/// One retained connection owner. Only transferred owners are closed by this kernel.
enum ConnectionOwner {
    /// An exclusive SQLx pool checkout.
    Pooled(sqlx::pool::PoolConnection<Turso>),
    /// A transferred native SQLx connection, including retained sync owners.
    Transferred(TursoConnection),
}
impl ConnectionOwner {
    /// Borrow native access while the session state guard excludes all other calls.
    fn connection(&mut self) -> &mut TursoConnection {
        match self {
            Self::Pooled(connection) => connection,
            Self::Transferred(connection) => connection,
        }
    }
    /// Ensure an abandoned transaction transition cannot recycle the pooled connection.
    fn retire(&mut self) {
        if let Self::Pooled(connection) = self {
            connection.close_on_drop();
        }
    }
    /// Await native close after transaction state has resolved; transferred sync retention ends here.
    async fn close(self) -> Result<(), SqlError> {
        match std::panic::AssertUnwindSafe(async move {
            match self {
                Self::Pooled(connection) => connection.close().await.map_err(SqlError::Provider),
                Self::Transferred(connection) => {
                    connection.close().await.map_err(SqlError::Provider)
                }
            }
        })
        .catch_unwind()
        .await
        {
            Ok(result) => result,
            Err(panic) => Err(SqlError::NativeInvocation(Box::new(effect::Exit::Die(
                effect::Defect::from_panic(panic),
            )))),
        }
    }
}

/// Private transaction frame state checked under the same lock as native access.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Lease acquired with no active transaction.
    Active,
    /// A native transaction/savepoint transition is in progress.
    Finishing,
    /// An acknowledged terminal close makes retained handles unusable.
    Closed,
    /// Provider acknowledgment cannot establish the durable result.
    Uncertain,
}
/// Connection and its state form one lock-protected authority; handles never check then reacquire.
struct SessionState {
    /// Connection owned until ordered cleanup takes it.
    owner: Option<ConnectionOwner>,
    /// Current lifecycle state; publication occurs while access remains exclusive.
    phase: Phase,
    /// Current native transaction depth; parent handles cannot execute inside child frames.
    depth: usize,
    /// Unforgeable frame identities; a reused depth never revives an old handle.
    frames: Vec<Arc<FrameIdentity>>,
    /// Transaction leases require awaited close even after confirmed settlement.
    retired: bool,
}
impl SessionState {
    /// Validate the current frame while holding the connection's exclusive guard.
    fn connection(
        &mut self,
        frame: usize,
        identity: &Arc<FrameIdentity>,
    ) -> Result<&mut TursoConnection, SqlError> {
        if self.phase != Phase::Active
            || self.depth != frame
            || !self
                .frames
                .get(frame)
                .is_some_and(|active| Arc::ptr_eq(active, identity))
        {
            return Err(SqlError::InactiveSession);
        }
        self.owner
            .as_mut()
            .map(ConnectionOwner::connection)
            .ok_or(SqlError::InactiveSession)
    }
}

/// Private allocation identity for one active frame; public callers cannot construct it.
struct FrameIdentity;

/// Shared handle to one exclusive session. Sharing a handle does not duplicate transaction authority.
#[derive(Clone)]
#[must_use]
pub struct NativeSession {
    /// Locked state retained by in-flight native cursors.
    state: Arc<Mutex<SessionState>>,
    /// Frame this handle is authorized to access.
    frame: usize,
    /// Exact active frame allocation, invalidated when that frame settles.
    identity: Arc<FrameIdentity>,
    /// Fail reentrant cursor callbacks before waiting for the guard they themselves retain.
    /// Actual frame authority is still validated only after exclusive lock acquisition.
    cursor_busy: Arc<AtomicBool>,
}
impl NativeSession {
    /// A cursor callback cannot wait for its own retained exclusive guard.
    /// The flag only rejects busy work; it never grants access. Frame/lifecycle
    /// checks still happen together with the native mutable borrow under the lock.
    fn check_cursor(&self) -> Result<(), SqlError> {
        if self.cursor_busy.load(Ordering::Acquire) {
            Err(SqlError::InactiveSession)
        } else {
            Ok(())
        }
    }
    /// Establish private state for a transferred/acquired owner with no active transaction.
    fn new(owner: ConnectionOwner) -> Self {
        let identity = Arc::new(FrameIdentity);
        Self {
            state: Arc::new(Mutex::new(SessionState {
                owner: Some(owner),
                phase: Phase::Active,
                depth: 0,
                frames: vec![identity.clone()],
                retired: false,
            })),
            frame: 0,
            identity,
            cursor_busy: Arc::new(AtomicBool::new(false)),
        }
    }
    /// Transfer an already owned SQLx connection, preserving the driver's native/sync adoption behavior.
    pub fn adopt(connection: TursoConnection) -> Self {
        Self::new(ConnectionOwner::Transferred(connection))
    }
    /// Run a lifetime-bound callback under exclusive access. Borrowed values cannot outlive the callback.
    pub async fn with_native<A: Send, F>(&self, body: F) -> Result<A, SqlError>
    where
        F: for<'a> FnOnce(&'a mut TursoConnection) -> NativeFuture<'a, A> + Send,
    {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        state.connection(self.frame, &self.identity)?;
        state
            .owner
            .as_mut()
            .ok_or(SqlError::InactiveSession)?
            .retire();
        state.retired = true;
        let result = std::panic::AssertUnwindSafe(async {
            body(state.connection(self.frame, &self.identity)?).await
        })
        .catch_unwind()
        .await;
        match result {
            Ok(result) => result,
            Err(panic) => {
                state.phase = Phase::Uncertain;
                Err(SqlError::NativeInvocation(Box::new(effect::Exit::Die(
                    effect::Defect::from_panic(panic),
                ))))
            }
        }
    }
    /// Move one statement and its bindings into SQLx execution, retaining affected-row metadata.
    pub async fn execute(&self, statement: SqlStatement) -> Result<TursoQueryResult, SqlError> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        let (text, arguments) = statement.into_parts();
        sqlx::query_with(AssertSqlSafe(text), arguments)
            .execute(state.connection(self.frame, &self.identity)?)
            .await
            .map_err(Into::into)
    }
    /// Buffer native owned rows. Decode failure remains separate from empty result sets.
    pub async fn query(&self, statement: SqlStatement) -> Result<Vec<TursoRow>, SqlError> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        let (text, arguments) = statement.into_parts();
        sqlx::query_with(AssertSqlSafe(text), arguments)
            .fetch_all(state.connection(self.frame, &self.identity)?)
            .await
            .map_err(Into::into)
    }
    /// Fetch exactly the first optional row without buffering subsequent rows.
    pub async fn first(&self, statement: SqlStatement) -> Result<Option<TursoRow>, SqlError> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        let (text, arguments) = statement.into_parts();
        sqlx::query_with(AssertSqlSafe(text), arguments)
            .fetch_optional(state.connection(self.frame, &self.identity)?)
            .await
            .map_err(Into::into)
    }
    /// Retain this session's exclusive lease throughout incremental native row consumption.
    pub async fn cursor(&self, statement: SqlStatement) -> Result<NativeCursor, SqlError> {
        self.check_cursor()?;
        let mut state = self.state.clone().lock_owned().await;
        let (text, arguments) = statement.into_parts();
        let items = state
            .connection(self.frame, &self.identity)?
            .owned_cursor(
                AssertSqlSafe(text).into_sql_str(),
                if arguments.len() == 0 {
                    None
                } else {
                    Some(arguments)
                },
            )
            .await?;
        self.cursor_busy.store(true, Ordering::Release);
        Ok(NativeCursor {
            items,
            _state: state,
            cursor_busy: self.cursor_busy.clone(),
        })
    }
    /// Start an operation-owned transaction, using BEGIN IMMEDIATE for local write coordination.
    /// State and provider acknowledgment remain under one lock; no cancellation select splits them.
    pub async fn begin(&self) -> Result<Self, SqlError> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        state.connection(self.frame, &self.identity)?;
        if self.frame != 0 {
            return Err(SqlError::InactiveSession);
        }
        let owner = state.owner.as_mut().ok_or(SqlError::InactiveSession)?;
        if owner.connection().options().sync_options().is_some() {
            return Err(SqlError::ReplicaNotAuthoritative);
        }
        owner.retire();
        state.retired = true;
        state.phase = Phase::Finishing;
        let result = begin_native(
            state
                .owner
                .as_mut()
                .ok_or(SqlError::InactiveSession)?
                .connection(),
            true,
        )
        .await;
        match result {
            Ok(()) => {
                state.depth = 1;
                let identity = Arc::new(FrameIdentity);
                state.frames.push(identity.clone());
                state.phase = Phase::Active;
                Ok(Self {
                    state: self.state.clone(),
                    frame: 1,
                    identity,
                    cursor_busy: self.cursor_busy.clone(),
                })
            }
            Err(error) => {
                state.phase = Phase::Uncertain;
                Err(error)
            }
        }
    }
    /// Begin a nested SQLx savepoint on the same owner. Parent handles are fenced until it resolves.
    pub async fn begin_savepoint(&self) -> Result<Self, SqlError> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        state.connection(self.frame, &self.identity)?;
        if self.frame == 0 {
            return Err(SqlError::InactiveSession);
        }
        state.phase = Phase::Finishing;
        let result = begin_native(
            state
                .owner
                .as_mut()
                .ok_or(SqlError::InactiveSession)?
                .connection(),
            false,
        )
        .await;
        match result {
            Ok(()) => {
                state.depth += 1;
                let identity = Arc::new(FrameIdentity);
                state.frames.push(identity.clone());
                state.phase = Phase::Active;
                Ok(Self {
                    state: self.state.clone(),
                    frame: self.frame + 1,
                    identity,
                    cursor_busy: self.cursor_busy.clone(),
                })
            }
            Err(error) => {
                state.phase = Phase::Uncertain;
                Err(error)
            }
        }
    }
    /// Await commit/savepoint release. A failed acknowledgment fences the entire owner as uncertain.
    pub async fn commit(&self) -> Result<(), SqlError> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        state.connection(self.frame, &self.identity)?;
        if self.frame == 0 {
            return Err(SqlError::InactiveSession);
        }
        state.phase = Phase::Finishing;
        let result = commit_native(
            state
                .owner
                .as_mut()
                .ok_or(SqlError::InactiveSession)?
                .connection(),
            self.frame == 1,
        )
        .await;
        match result {
            Ok(()) => {
                state.depth -= 1;
                let _closed_frame = state.frames.pop();
                state.phase = Phase::Active;
                Ok(())
            }
            Err(error) => {
                state.phase = Phase::Uncertain;
                Err(error)
            }
        }
    }
    /// Await rollback and release a nested savepoint before restoring the parent's frame.
    pub async fn rollback(&self) -> Result<(), SqlError> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        if state.depth != self.frame
            || self.frame == 0
            || state.phase == Phase::Closed
            || !state
                .frames
                .get(self.frame)
                .is_some_and(|active| Arc::ptr_eq(active, &self.identity))
        {
            return Err(SqlError::InactiveSession);
        }
        state.phase = Phase::Finishing;
        let connection = state
            .owner
            .as_mut()
            .ok_or(SqlError::InactiveSession)?
            .connection();
        let result = rollback_native(connection).await;
        // SQLx rollback-to-savepoint does not RELEASE; explicitly release its native frame.
        let result = if result.is_ok() && self.frame > 1 {
            connection
                .native_execute(
                    format!("RELEASE SAVEPOINT _sqlx_savepoint_{}", self.frame - 1),
                    (),
                )
                .await
                .map(|_| ())
                .map_err(SqlError::Provider)
        } else {
            result
        };
        match result {
            Ok(()) => {
                state.depth -= 1;
                let _closed_frame = state.frames.pop();
                state.phase = Phase::Active;
                Ok(())
            }
            Err(error) => {
                state.phase = Phase::Uncertain;
                Err(error)
            }
        }
    }
    /// Join exclusive access, resolve all active frames, then release/close ownership once.
    /// Closed handles stay fenced. Unknown commit never becomes a definitive rollback claim.
    pub async fn close(&self) -> Result<(), SqlError> {
        let mut state = self.state.lock().await;
        if self.frame != 0 {
            return Err(SqlError::InactiveSession);
        }
        if state.phase == Phase::Closed {
            return Ok(());
        }
        if !state
            .frames
            .first()
            .is_some_and(|active| Arc::ptr_eq(active, &self.identity))
        {
            return Err(SqlError::InactiveSession);
        }
        let uncertain = state.phase == Phase::Uncertain || state.phase == Phase::Finishing;
        let mut owner = state.owner.take().ok_or(SqlError::InactiveSession)?;
        state.phase = Phase::Closed;
        state.frames.clear();
        let mut cleanup = None;
        while state.depth > 0 {
            match rollback_native(owner.connection()).await {
                Ok(()) => state.depth -= 1,
                Err(error) => {
                    cleanup = Some(error);
                    break;
                }
            }
        }
        if let Err(error) = settle_native(owner.connection()).await {
            cleanup = Some(match cleanup {
                Some(primary) => SqlError::Cleanup {
                    primary: Some(Box::new(primary)),
                    cleanup: Box::new(error),
                },
                None => error,
            });
        }
        if uncertain || cleanup.is_some() {
            owner.retire();
        }
        // A resolved ordinary pooled lease returns to its native pool. Transferred
        // owners and any uncertain/transaction lease are explicitly closed.
        let close = match owner {
            ConnectionOwner::Pooled(connection)
                if !uncertain && !state.retired && cleanup.is_none() =>
            {
                drop(connection);
                Ok(())
            }
            owner @ (ConnectionOwner::Pooled(_) | ConnectionOwner::Transferred(_)) => {
                owner.close().await
            }
        };
        match (cleanup, close) {
            (Some(primary), Err(cleanup)) => Err(SqlError::Cleanup {
                primary: Some(Box::new(primary)),
                cleanup: Box::new(cleanup),
            }),
            (Some(error), Ok(())) | (None, Err(error)) => Err(error),
            (None, Ok(())) => Ok(()),
        }
    }
}

/// An incremental SQLx cursor owning the exclusive session guard until disposal.
#[must_use]
pub struct NativeCursor {
    /// Native iterator is dropped before its exclusive guard is released.
    items: CursorItems,
    /// Retained connection authority; borrowing prevents a pool reuse race.
    _state: OwnedMutexGuard<SessionState>,
    /// Shared rejection flag; no connection/frame authority is derived from it.
    cursor_busy: Arc<AtomicBool>,
}
impl Drop for NativeCursor {
    /// Clear busy rejection while the guard still prevents any native access.
    /// Field order then drops statement/rows before releasing the session guard.
    fn drop(&mut self) {
        self.cursor_busy.store(false, Ordering::Release);
    }
}
impl NativeCursor {
    /// Pull one native result item; batches preserve row/result ordering.
    pub async fn next_item(
        &mut self,
    ) -> Result<Option<either::Either<TursoQueryResult, TursoRow>>, SqlError> {
        self.items.next().await.transpose().map_err(Into::into)
    }
    /// Pull one row while retaining result metadata internally; no row buffering occurs.
    pub async fn next_row(&mut self) -> Result<Option<TursoRow>, SqlError> {
        loop {
            match self.next_item().await? {
                Some(either::Either::Right(row)) => return Ok(Some(row)),
                Some(either::Either::Left(_)) => {}
                None => return Ok(None),
            }
        }
    }
    /// Dispose native statement state before releasing the connection guard.
    pub async fn close(self) -> Result<(), SqlError> {
        drop(self);
        Ok(())
    }
}

/// Start through SQLx's existing manager while retaining native defects in an invocation-local carrier.
async fn begin_native(connection: &mut TursoConnection, immediate: bool) -> Result<(), SqlError> {
    match std::panic::AssertUnwindSafe(sqlx::TursoTransactionManager::begin_owned(
        connection, immediate,
    ))
    .catch_unwind()
    .await
    {
        Ok(result) => result.map_err(SqlError::Provider),
        Err(panic) => Err(SqlError::NativeInvocation(Box::new(effect::Exit::Die(
            effect::Defect::from_panic(panic),
        )))),
    }
}
/// Await commit and its bookkeeping together; acknowledgment failure stays uncertain.
async fn commit_native(connection: &mut TursoConnection, durable: bool) -> Result<(), SqlError> {
    match std::panic::AssertUnwindSafe(sqlx::TursoTransactionManager::commit_owned(connection))
        .catch_unwind()
        .await
    {
        Ok(result) => result.map_err(|error| {
            if durable {
                SqlError::CommitOutcomeUnknown(error)
            } else {
                SqlError::Provider(error)
            }
        }),
        Err(panic) => {
            let defect = effect::Defect::from_panic(panic);
            let outcome = if durable {
                effect::Exit::FinalizationFailed(effect::Finalization {
                    primary: effect::PrimaryOutcome::Defected(defect),
                    first: effect::CleanupFailure::ReleaseFailure(SqlError::CommitOutcomeUnknown(
                        sqlx::Error::Protocol(
                            "native commit panicked before acknowledgment".into(),
                        ),
                    )),
                    rest: Vec::new(),
                })
            } else {
                effect::Exit::Die(defect)
            };
            Err(SqlError::NativeInvocation(Box::new(outcome)))
        }
    }
}
/// Await rollback through the existing manager; a dropped future is never an acknowledgment.
async fn rollback_native(connection: &mut TursoConnection) -> Result<(), SqlError> {
    match std::panic::AssertUnwindSafe(sqlx::TursoTransactionManager::rollback_owned(connection))
        .catch_unwind()
        .await
    {
        Ok(result) => result.map_err(SqlError::Provider),
        Err(panic) => Err(SqlError::NativeInvocation(Box::new(effect::Exit::Die(
            effect::Defect::from_panic(panic),
        )))),
    }
}
/// Join native cleanup without replacing primary failure or claiming retry safety.
fn join_cleanup<A>(
    result: Result<A, SqlError>,
    cleanup: Result<(), SqlError>,
) -> Result<A, SqlError> {
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup)) => Err(SqlError::Cleanup {
            primary: None,
            cleanup: Box::new(cleanup),
        }),
        (Err(primary), Err(cleanup)) => Err(SqlError::Cleanup {
            primary: Some(Box::new(primary)),
            cleanup: Box::new(cleanup),
        }),
    }
}

/// Await pending SQLx rollback and any transferred native transaction before lease disposal.
async fn settle_native(connection: &mut TursoConnection) -> Result<(), SqlError> {
    match std::panic::AssertUnwindSafe(connection.settle_owned())
        .catch_unwind()
        .await
    {
        Ok(result) => result.map_err(SqlError::Provider),
        Err(panic) => Err(SqlError::NativeInvocation(Box::new(effect::Exit::Die(
            effect::Defect::from_panic(panic),
        )))),
    }
}
/// Resolve native state, then await disposal of an independently owned pooled lease.
async fn finish_pool(mut connection: sqlx::pool::PoolConnection<Turso>) -> Result<(), SqlError> {
    let settlement = settle_native(&mut connection).await;
    let close = ConnectionOwner::Pooled(connection).close().await;
    join_cleanup(settlement, close)
}
