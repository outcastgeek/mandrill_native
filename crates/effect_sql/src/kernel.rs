//! Native async SQL kernel shared by Effect and native Identity stores.
//!
//! No method creates or invokes an Effect runtime. Tokio is the selected local
//! SQLx provider's reactor; every operation has an exclusive connection owner.

use crate::{SqlError, SqlStatement};
use crate::prepared_rows::{NativeRows, PreparedSqlRowLanes, ReservedNativeRows, RowLaneLease};
use effect::memory::{
    Admission, AdmissionBundle, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId,
    SharedSlot, SlotPool,
};
use futures_util::{FutureExt, StreamExt, TryStreamExt, stream::BoxStream};
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
type OrdinaryCursorItems =
    BoxStream<'static, Result<either::Either<TursoQueryResult, TursoRow>, sqlx::Error>>;
enum CursorItems {
    Ordinary(OrdinaryCursorItems),
    Prepared(
        BoxStream<
            'static,
            Result<either::Either<TursoQueryResult, TursoRow>, sqlx::PreparedTursoCursorError>,
        >,
    ),
}

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

#[derive(Clone)]
struct ConnectionPool {
    provider: TursoPool,
    pending_admission: Option<Arc<Admission<'static, 1>>>,
}

impl ConnectionPool {
    fn reserve_pending(&self) -> Result<Option<AdmissionBundle<'static, 1>>, MemoryError> {
        self.pending_admission
            .as_deref()
            .map(|admission| admission.reserve([1], [0]))
            .transpose()
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    async fn acquire(&self) -> Result<sqlx::pool::PoolConnection<Turso>, SqlError> {
        self.acquire_reserved(self.reserve_pending().map_err(SqlError::Memory)?)
            .await
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    async fn acquire_reserved(
        &self,
        _pending: Option<AdmissionBundle<'static, 1>>,
    ) -> Result<sqlx::pool::PoolConnection<Turso>, SqlError> {
        self.provider.acquire().await.map_err(Into::into)
    }
}

/// Authoritative local shared SQLx pool. Native stores use operation-owned transactions.
#[derive(Clone)]
#[must_use]
pub struct SqlKernel {
    /// Shared native pool and optional pending-checkout admission.
    pool: ConnectionPool,
    /// Private configuration identity retained by maintenance grants.
    identity: Arc<()>,
    session_pool: Option<Arc<PreparedSqlSessionPool>>,
    rows_pool: Option<NativeRows>,
    prepared_panic_diagnostics: Option<Arc<effect::PreparedPanicPool>>,
}
impl SqlKernel {
    /// Select a pool. Replicas may perform reads but authoritative operations reject their topology.
    pub fn new(pool: TursoPool) -> Self {
        Self {
            pool: ConnectionPool { provider: pool, pending_admission: None },
            identity: Arc::new(()),
            session_pool: None,
            rows_pool: None,
            prepared_panic_diagnostics: None,
        }
    }
    /// Select startup-prepared session controls for subsequent session acquisitions.
    pub fn with_session_pool(mut self, pool: Arc<PreparedSqlSessionPool>) -> Self {
        self.session_pool = Some(pool);
        self
    }
    /// Select retained prepared row backing for incremental cursors.
    pub fn with_rows_pool(mut self, pool: Arc<sqlx::PreparedTursoRowsPool>) -> Self {
        self.rows_pool = Some(NativeRows::Shared(pool));
        self
    }

    /// Select exclusive preprepared driver registries for incremental cursors.
    pub fn with_row_lanes(mut self, lanes: Arc<PreparedSqlRowLanes>) -> Self {
        self.rows_pool = Some(NativeRows::Lanes(lanes));
        self
    }

    /// Select a separate Slots admission for pending native pool checkouts.
    /// One work slot stays held while SQLx acquires the connection and runs its hooks.
    pub fn with_pending_checkout_admission(
        mut self,
        admission: Arc<Admission<'static, 1>>,
    ) -> Result<Self, MemoryError> {
        crate::prepared_rows::validate_slot_admission(&admission)?;
        self.pool.pending_admission = Some(admission);
        Ok(self)
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub(crate) async fn acquire_connection(&self) -> Result<sqlx::pool::PoolConnection<Turso>, SqlError> {
        self.pool.acquire().await
    }
    pub(crate) fn with_prepared_panic_diagnostics(
        mut self,
        pool: Arc<effect::PreparedPanicPool>,
    ) -> Self {
        self.prepared_panic_diagnostics = Some(pool);
        self
    }
    pub(crate) fn prepared_panic_diagnostics(&self) -> Option<&effect::PreparedPanicPool> {
        self.prepared_panic_diagnostics.as_deref()
    }
    /// Reserve backing from the shared driver pool without starting SQL work.
    /// Exclusive lanes retain their permits inside Effect SQL cursor paths.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub fn reserve_prepared_rows(&self) -> Result<sqlx::PreparedTursoCursorReservation, SqlError> {
        match &self.rows_pool {
            Some(NativeRows::Shared(pool)) => pool.reserve_cursor().map_err(SqlError::PreparedRow),
            _ => Err(SqlError::Memory(MemoryError::UnsupportedRunContract)),
        }
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub(crate) fn reserve_optional_rows(
        &self,
    ) -> Result<Option<ReservedNativeRows>, SqlError> {
        self.rows_pool.as_ref().map(NativeRows::reserve).transpose()
    }
    /// Borrow the exact SQLx pool for checked macros and independently owned native store calls.
    #[must_use]
    pub fn pool(&self) -> &TursoPool {
        &self.pool.provider
    }
    /// Check selected configuration identity without publishing forgeable connection authority.
    #[must_use]
    pub fn same_configuration(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
    /// Require embedded local write authority, rejecting read-only and synchronized configurations.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub fn require_authoritative(&self) -> Result<(), SqlError> {
        let options = self.pool().connect_options();
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
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn session(&self) -> Result<NativeSession, SqlError> {
        if self.pool.pending_admission.is_some() {
            let connection = self.pool.acquire().await?;
            return match self.reserve_optional_session().map_err(SqlError::Memory)? {
                Some(reservation) => Ok(reservation.open_acquired(connection).await),
                None => Ok(self.session_from_acquired(connection)),
            };
        }
        if let Some(reservation) = self.reserve_optional_session().map_err(SqlError::Memory)? {
            return reservation.open().await;
        }
        let connection = self.pool.acquire().await?;
        Ok(self.session_from_acquired(connection))
    }
    pub(crate) fn session_from_acquired(
        &self,
        connection: sqlx::pool::PoolConnection<Turso>,
    ) -> NativeSession {
        let mut session = NativeSession::new(ConnectionOwner::Pooled(connection));
        session.rows_pool = self.rows_pool.clone();
        session.prepared_panic_diagnostics = self.prepared_panic_diagnostics.clone();
        session
    }
    /// Claim the selected physical session control without opening a database connection.
    pub fn reserve_prepared_session(&self) -> Result<PreparedSqlSessionReservation, MemoryError> {
        self.reserve_optional_session()?
            .ok_or(MemoryError::UnsupportedRunContract)
    }
    pub(crate) fn reserve_optional_session(
        &self,
    ) -> Result<Option<PreparedSqlSessionReservation>, MemoryError> {
        self.session_pool
            .as_ref()
            .map(|pool| {
                Ok(PreparedSqlSessionReservation {
                    checkout: pool.checkout()?,
                    pool: self.pool.clone(),
                    rows_pool: self.rows_pool.clone(),
                    cleanup: pool.cleanup.clone(),
                    prepared_panic_diagnostics: self.prepared_panic_diagnostics.clone(),
                })
            })
            .transpose()
    }
    pub(crate) fn reserve_optional_callback_session(
        &self,
    ) -> Result<Option<PreparedSqlCallbackSessionReservation>, MemoryError> {
        self.reserve_optional_session()?
            .map(|session| {
                // No session handle has been published from this checkout.
                let callback = SessionLoan::try_new(
                    &session.checkout.state,
                    &session.checkout.signals,
                )?;
                Ok(PreparedSqlCallbackSessionReservation { callback, session })
            })
            .transpose()
    }
    pub(crate) fn reserve_optional_cleanup(
        &self,
    ) -> Result<Option<effect::CleanupFailures<SqlError>>, MemoryError> {
        self.session_pool
            .as_ref()
            .and_then(|pool| pool.cleanup.as_ref())
            .map(|cleanup| cleanup.reserve_failures())
            .transpose()
    }
    pub(crate) fn prepared_session_configured(&self) -> bool {
        self.session_pool.is_some()
    }

    pub(crate) fn early_checkout_configured(&self) -> bool {
        self.prepared_session_configured() || self.pool.pending_admission.is_some()
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    async fn open_operation_lease(&self) -> Result<NativeOperationLease, SqlError> {
        let reservation = self.reserve_optional_session().map_err(SqlError::Memory)?;
        let pending = self.pool.reserve_pending().map_err(SqlError::Memory)?;
        self.open_reserved_operation_lease(reservation, pending).await
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    async fn open_reserved_operation_lease(
        &self,
        reservation: Option<PreparedSqlSessionReservation>,
        pending: Option<AdmissionBundle<'static, 1>>,
    ) -> Result<NativeOperationLease, SqlError> {
        let mut connection = self.pool.acquire_reserved(pending).await?;
        // An abandoned callback can leave native state unsettled.
        connection.close_on_drop();
        if let Some(reservation) = reservation {
            let session = reservation.open_acquired(connection).await;
            let state = session.state.clone().lock_owned().await;
            Ok(NativeOperationLease::Prepared { state, session })
        } else {
            Ok(NativeOperationLease::Ordinary(connection))
        }
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
            let mut lease = self.open_operation_lease().await?;
            let connection = lease.connection()?;
            let outcome =
                std::panic::AssertUnwindSafe(async { operation.run_boxed(connection).await })
                    .catch_unwind()
                    .await;
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(panic) => Err(SqlError::NativeInvocation(Box::new(effect::Exit::Die(
                    effect::Defect::from_panic(panic),
                )))),
            };
            join_cleanup(outcome, lease.finish().await.map_or(Ok(()), Err))
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
            let lease = self.open_operation_lease().await?;
            run_transaction_operation(lease, operation).await
        })
    }

    /// Admit the configured session control and pending checkout before producing
    /// arguments for a native transaction. Opening the native connection still occurs
    /// in `run` and is owned by SQLx.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub fn reserve_transaction_operation(&self) -> Result<NativeTransactionReservation<'_>, SqlError> {
        self.require_authoritative()?;
        Ok(NativeTransactionReservation {
            kernel: self,
            session: self.reserve_optional_session().map_err(SqlError::Memory)?,
            pending: self.pool.reserve_pending().map_err(SqlError::Memory)?,
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

/// One transaction's admitted session and checkout controls.
#[must_use = "dropping the reservation returns its session and checkout controls"]
pub struct NativeTransactionReservation<'a> {
    kernel: &'a SqlKernel,
    session: Option<PreparedSqlSessionReservation>,
    pending: Option<AdmissionBundle<'static, 1>>,
}
impl NativeTransactionReservation<'_> {
    /// Run the operation under the same BEGIN IMMEDIATE and settlement path as the ordinary kernel.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn run<A: Send, O: NativeOperation<A>>(self, operation: O) -> Result<A, SqlError> {
        let lease = self
            .kernel
            .open_reserved_operation_lease(self.session, self.pending)
            .await?;
        run_transaction_operation(lease, operation).await
    }
}

#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
async fn run_transaction_operation<A: Send, O: NativeOperation<A>>(
    mut lease: NativeOperationLease,
    operation: O,
) -> Result<A, SqlError> {
    let connection = lease.connection()?;
    if let Err(error) = begin_native(connection, true).await {
        return join_cleanup(Err(error), lease.finish().await.map_or(Ok(()), Err));
    }
    let result =
        std::panic::AssertUnwindSafe(async { operation.run_boxed(connection).await })
            .catch_unwind()
            .await;
    let outcome = match result {
        Ok(Ok(value)) => match commit_native(connection, true).await {
            Ok(()) => Ok(value),
            Err(primary) => match rollback_native(connection).await {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(SqlError::Cleanup {
                    primary: Some(Box::new(primary)),
                    cleanup: Box::new(cleanup),
                }),
            },
        },
        Ok(Err(primary)) => match rollback_native(connection).await {
            Ok(()) => Err(primary),
            Err(cleanup) => Err(SqlError::Cleanup {
                primary: Some(Box::new(primary)),
                cleanup: Box::new(cleanup),
            }),
        },
        Err(panic) => {
            let cleanup = rollback_native(connection).await;
            Err(SqlError::NativeInvocation(Box::new(
                crate::client::append_cleanup(
                    effect::Exit::Die(effect::Defect::from_panic(panic)),
                    cleanup,
                ),
            )))
        }
    };
    let close = lease.finish().await.map_or(Ok(()), Err);
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
}

/// A lifetime-bound native callback borrows the actual pooled connection.
enum NativeOperationLease {
    Ordinary(sqlx::pool::PoolConnection<Turso>),
    Prepared {
        // On abandonment the guard must retire before the final session loan.
        state: OwnedMutexGuard<SessionState>,
        session: NativeSession,
    },
}
impl NativeOperationLease {
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    fn connection(&mut self) -> Result<&mut TursoConnection, SqlError> {
        match self {
            Self::Ordinary(connection) => Ok(connection),
            Self::Prepared { state, session } => state.connection(session.frame, session.identity),
        }
    }

    async fn finish(self) -> Option<SqlError> {
        match self {
            Self::Ordinary(connection) => finish_pool(connection).await.err(),
            Self::Prepared { mut state, session } => {
                let owner = state.owner.take();
                drop(state);
                let failure = match owner {
                    Some(ConnectionOwner::Pooled(connection)) => finish_pool(connection).await.err(),
                    Some(owner @ ConnectionOwner::Transferred(_)) => owner.close().await.err(),
                    None => Some(SqlError::InactiveSession),
                };
                drop(session);
                failure
            }
        }
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
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    async fn close(self) -> Result<(), SqlError> {
        self.close_fault().await.map_err(CloseFault::into_sql)
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    async fn close_fault(self) -> Result<(), CloseFault> {
        self.close_fault_with(&mut None).await
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    async fn close_fault_with(
        self,
        diagnostic: &mut Option<effect::PreparedPanicReservation>,
    ) -> Result<(), CloseFault> {
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
            Ok(result) => result.map_err(CloseFault::Sql),
            Err(panic) => Err(CloseFault::Defect(capture_panic(diagnostic, panic))),
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
    frames: FrameStorage,
    generation: u64,
    /// Transaction leases require awaited close even after confirmed settlement.
    retired: bool,
}
impl SessionState {
    /// Validate the current frame while holding the connection's exclusive guard.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    fn connection(
        &mut self,
        frame: usize,
        identity: u64,
    ) -> Result<&mut TursoConnection, SqlError> {
        if self.phase != Phase::Active
            || self.depth != frame
            || !self.frames.matches(frame, identity)
        {
            return Err(SqlError::InactiveSession);
        }
        self.owner
            .as_mut()
            .map(ConnectionOwner::connection)
            .ok_or(SqlError::InactiveSession)
    }
}

// A frame generation is meaningful only within the owning SessionState.
enum FrameStorage {
    Ordinary(Vec<u64>),
    Prepared(FixedVector<'static, u64>),
}
impl FrameStorage {
    fn matches(&self, frame: usize, identity: u64) -> bool {
        let frames = match self {
            Self::Ordinary(frames) => frames.as_slice(),
            Self::Prepared(frames) => frames.as_slice(),
        };
        frames.get(frame) == Some(&identity)
    }

    fn preflight(&mut self, generation: u64) -> Result<u64, MemoryError> {
        match self {
            Self::Ordinary(frames) => {
                frames.try_reserve(1).map_err(|_| MemoryError::CollectionReservationFailed)?;
            }
            Self::Prepared(frames) if frames.len() == frames.capacity() => {
                return Err(MemoryError::NoSlot);
            }
            Self::Prepared(_) => {}
        }
        generation.checked_add(1).ok_or(MemoryError::OwnerLimit)
    }

    fn push(&mut self, generation: u64) -> Result<(), MemoryError> {
        match self {
            Self::Ordinary(frames) => {
                frames.push(generation);
                Ok(())
            }
            Self::Prepared(frames) => frames.push(generation).map_err(|_| MemoryError::NoSlot),
        }
    }

    fn pop(&mut self) {
        match self {
            Self::Ordinary(frames) => {
                frames.pop();
            }
            Self::Prepared(frames) => {
                frames.pop();
            }
        }
    }
    fn clear(&mut self) {
        match self {
            Self::Ordinary(frames) => frames.clear(),
            Self::Prepared(frames) => frames.clear(),
        }
    }
}

/// Explicit concurrent session controls and frame authorities per session.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSqlSessionCapacity {
    /// Simultaneous checked-out sessions.
    pub sessions: usize,
    /// Frame authorities per session, including the root session frame.
    pub frames_per_session: usize,
}
/// Actual region demand and separately allocated startup Arc payloads.
pub struct PreparedSqlSessionDemand {
    /// Region registry, controls and fixed frame vectors, excluding rounding/overhead.
    pub region_bytes: Bytes,
    /// Startup state Arc controls and their payload layout (Arc headers excluded).
    pub state_controls: usize,
    /// Startup state Arc payload layout.
    pub state_payload: std::alloc::Layout,
    /// Startup cancellation/busy Arc controls and their payload layout.
    pub busy_controls: usize,
    /// Startup busy Arc payload layout.
    pub busy_payload: std::alloc::Layout,
}
struct SessionSignals {
    busy: AtomicBool,
    pending_savepoint: AtomicBool,
    loans: std::sync::Mutex<SessionLoans>,
}
pub(crate) struct CloseFailures {
    pub(crate) rollback: Option<CloseFault>,
    pub(crate) settle: Option<CloseFault>,
    pub(crate) close: Option<CloseFault>,
}
pub(crate) enum CloseFault {
    Sql(SqlError),
    Defect(effect::Defect),
}

/// Separately admitted diagnostics for one root transaction's native work.
pub(crate) struct TransactionPanicReservations {
    pub(crate) cursor: Option<effect::PreparedPanicReservation>,
    pub(crate) begin: Option<effect::PreparedPanicReservation>,
    pub(crate) transition: Option<effect::PreparedPanicReservation>,
    pub(crate) close_rollback: Option<effect::PreparedPanicReservation>,
    pub(crate) close_settle: Option<effect::PreparedPanicReservation>,
    pub(crate) close_dispose: Option<effect::PreparedPanicReservation>,
}

pub(crate) fn capture_panic(
    diagnostic: &mut Option<effect::PreparedPanicReservation>,
    payload: Box<dyn std::any::Any + Send>,
) -> effect::Defect {
    match diagnostic.take() {
        Some(reservation) => reservation.capture(payload),
        None => effect::Defect::from_panic(payload),
    }
}
impl CloseFault {
    fn into_sql(self) -> SqlError {
        match self {
            Self::Sql(error) => error,
            Self::Defect(defect) => SqlError::NativeInvocation(Box::new(effect::Exit::Die(defect))),
        }
    }
    pub(crate) fn into_exit<A>(self) -> effect::Exit<A, SqlError> {
        match self {
            Self::Sql(error) => error.into_exit(),
            Self::Defect(defect) => effect::Exit::Die(defect),
        }
    }
}
impl From<SqlError> for CloseFault {
    fn from(error: SqlError) -> Self {
        Self::Sql(error)
    }
}
pub(crate) enum CommitFault {
    Sql(SqlError),
    Defect {
        defect: effect::Defect,
        durable: bool,
    },
}
impl From<SqlError> for CommitFault {
    fn from(error: SqlError) -> Self {
        Self::Sql(error)
    }
}
impl CommitFault {
    pub(crate) fn into_exit<A>(self) -> effect::Exit<A, SqlError> {
        match self {
            Self::Sql(error) => error.into_exit(),
            Self::Defect {
                defect,
                durable: false,
            } => effect::Exit::Die(defect),
            Self::Defect {
                defect,
                durable: true,
            } => effect::Exit::FinalizationFailed(effect::Finalization {
                primary: effect::PrimaryOutcome::Defected(defect),
                first: effect::CleanupFailure::ReleaseFailure(SqlError::CommitOutcomeUnknown(
                    sqlx::Error::Protocol("native commit panicked before acknowledgment".into()),
                )),
                rest: Default::default(),
            }),
        }
    }
    fn into_sql(self) -> SqlError {
        match self {
            Self::Sql(error) => error,
            fault @ Self::Defect { .. } => SqlError::NativeInvocation(Box::new(fault.into_exit())),
        }
    }
}
struct SessionLoans {
    count: usize,
    quota: Option<AdmissionBundle<'static, 1>>,
}
#[derive(Clone)]
enum CursorSignals {
    Ordinary(Arc<AtomicBool>),
    Prepared(Arc<SessionSignals>),
}
impl CursorSignals {
    fn busy(&self) -> &AtomicBool {
        match self {
            Self::Ordinary(value) => value,
            Self::Prepared(value) => &value.busy,
        }
    }
}
struct SessionControl {
    state: Arc<Mutex<SessionState>>,
    signals: Arc<SessionSignals>,
}
type SessionRegistry = std::sync::Mutex<FixedVector<'static, SessionControl>>;
/// Region-backed registry of startup native session controls.
pub struct PreparedSqlSessionPool {
    registry: SharedSlot<'static, SessionRegistry>,
    admission: Option<Admission<'static, 1>>,
    cleanup: Option<Arc<effect::PreparedCleanupConfig<SqlError>>>,
}
impl PreparedSqlSessionCapacity {
    /// Report actual storage; caller's outer Arc and allocator overhead are separate.
    pub fn demand(self) -> Result<PreparedSqlSessionDemand, MemoryError> {
        let controls = std::alloc::Layout::array::<SessionControl>(self.sessions)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let frames = std::alloc::Layout::array::<u64>(self.frames_per_session)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let region_bytes = SlotPool::<SessionRegistry>::storage_bytes(1)?
            .checked_add(Bytes::new(controls.size()))?
            .checked_add(Bytes::new(frames.size()).checked_mul(self.sessions)?)?;
        Ok(PreparedSqlSessionDemand {
            region_bytes,
            state_controls: self.sessions,
            state_payload: std::alloc::Layout::new::<Mutex<SessionState>>(),
            busy_controls: self.sessions,
            busy_payload: std::alloc::Layout::new::<SessionSignals>(),
        })
    }
}
struct SessionCheckout {
    state: Arc<Mutex<SessionState>>,
    signals: Arc<SessionSignals>,
    identity: u64,
    quota: Option<AdmissionBundle<'static, 1>>,
}
/// Actual prepared session control tied to its selected SQLx pool. `open`
/// acquires a native connection; SQLx and VM memory remain outside this bound.
#[must_use = "the reserved session control is released if dropped unopened"]
pub struct PreparedSqlSessionReservation {
    checkout: SessionCheckout,
    pool: ConnectionPool,
    rows_pool: Option<NativeRows>,
    cleanup: Option<Arc<effect::PreparedCleanupConfig<SqlError>>>,
    prepared_panic_diagnostics: Option<Arc<effect::PreparedPanicPool>>,
}
impl PreparedSqlSessionReservation {
    /// Acquire a native connection from the retained pool and publish its session.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn open(self) -> Result<NativeSession, SqlError> {
        let connection = self.pool.acquire().await?;
        Ok(self.open_acquired(connection).await)
    }
    pub(crate) async fn open_acquired(
        self,
        connection: sqlx::pool::PoolConnection<Turso>,
    ) -> NativeSession {
        let Self {
            checkout,
            rows_pool,
            cleanup,
            prepared_panic_diagnostics,
            ..
        } = self;
        let mut session = checkout.publish(ConnectionOwner::Pooled(connection)).await;
        session.rows_pool = rows_pool;
        session.cleanup = cleanup;
        session.prepared_panic_diagnostics = prepared_panic_diagnostics;
        session
    }
}
/// The callback's loan is acquired before transferring a one-use input.
pub(crate) struct PreparedSqlCallbackSessionReservation {
    callback: SessionLoan,
    session: PreparedSqlSessionReservation,
}
impl PreparedSqlCallbackSessionReservation {
    pub(crate) async fn open_acquired(
        self,
        connection: sqlx::pool::PoolConnection<Turso>,
    ) -> (NativeSession, SessionLoan) {
        let Self { callback, session } = self;
        (session.open_acquired(connection).await, callback)
    }
}
// Each loan owns the actual state Arc. Borrowed state guards require a live
// NativeSession; owned cursor guards are dropped before their cursor loan.
pub(crate) struct SessionLoan {
    state: Option<Arc<Mutex<SessionState>>>,
    signals: Arc<SessionSignals>,
}
/// Holds the prepared session alive until the reserved transition is redeemed.
pub(crate) struct SavepointReservation {
    loan: Option<SessionLoan>,
    frame: usize,
    identity: u64,
}
impl SavepointReservation {
    fn matches(&self, session: &NativeSession, state: &SessionState) -> bool {
        self.frame == session.frame
            && state.frames.matches(self.frame, self.identity)
            && self
                .loan
                .as_ref()
                .and_then(|loan| loan.state.as_ref())
                .is_some_and(|reserved| Arc::ptr_eq(reserved, &session.state))
    }

    fn clear_pending(&self) {
        if let Some(loan) = &self.loan {
            loan.signals.pending_savepoint.store(false, Ordering::Release);
        }
    }

    fn redeem(mut self) -> Option<SessionLoan> {
        self.clear_pending();
        self.loan.take()
    }
}
impl Drop for SavepointReservation {
    fn drop(&mut self) {
        self.clear_pending();
    }
}
impl SessionLoan {
    #[expect(
        clippy::unreachable,
        reason = "a live session loan retains its state until Drop"
    )]
    fn try_clone_prepared(&self) -> Result<Self, MemoryError> {
        let state = self
            .state
            .as_ref()
            .unwrap_or_else(|| unreachable!("live session loan"));
        Self::try_new(state, &self.signals)
    }
    fn try_new(
        state: &Arc<Mutex<SessionState>>,
        signals: &Arc<SessionSignals>,
    ) -> Result<Self, MemoryError> {
        let mut loans = crate::try_prepared_lock(&signals.loans)?;
        let state = state.clone();
        let signals = signals.clone();
        // The Arc strong-owner limit is reached before this loan count can overflow.
        loans.count += 1;
        drop(loans);
        Ok(Self {
            state: Some(state),
            signals,
        })
    }
    fn new(state: Arc<Mutex<SessionState>>, signals: Arc<SessionSignals>) -> Self {
        // Each loan already owns an Arc; Arc's strong-owner limit is reached first.
        signals
            .loans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .count += 1;
        Self {
            state: Some(state),
            signals,
        }
    }
}
impl Clone for SessionLoan {
    #[expect(
        clippy::unreachable,
        reason = "a live session loan retains its state until Drop"
    )]
    fn clone(&self) -> Self {
        Self::new(
            self.state
                .as_ref()
                .cloned()
                .unwrap_or_else(|| unreachable!("live session loan")),
            self.signals.clone(),
        )
    }
}
impl Drop for SessionLoan {
    #[expect(
        clippy::unreachable,
        reason = "a final prepared session loan cannot coexist with a state guard"
    )]
    fn drop(&mut self) {
        // Declared first so unwind drops the physical state before quota.
        let quota;
        let Some(state_arc) = self.state.take() else {
            return;
        };
        {
            let mut loans = self
                .signals
                .loans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if loans.count != 1 {
                // Another loan owns state, so this cannot destroy its contents.
                drop(state_arc);
                loans.count -= 1;
                return;
            }
            loans.count = 0;
            quota = loans.quota.take();
        }
        let owner = {
            let Ok(mut state) = state_arc.try_lock() else {
                unreachable!("final prepared session loan cannot coexist with a state guard");
            };
            state.phase = Phase::Closed;
            state.depth = 0;
            state.frames.clear();
            state.owner.take()
        };
        // Keep state alive through actual native owner destruction.
        drop(owner);
        drop(state_arc);
        drop(quota);
    }
}
impl SessionCheckout {
    async fn publish(mut self, owner: ConnectionOwner) -> NativeSession {
        {
            let mut state = self.state.lock().await;
            state.owner = Some(owner);
            state.phase = Phase::Active;
        }
        // This checkout is exclusive and has not published a loan yet. The
        // previous final loan retired any quota before checkout could succeed.
        self.signals
            .loans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .quota = self.quota.take();
        let loan = SessionLoan::new(self.state.clone(), self.signals.clone());
        NativeSession {
            state: self.state,
            frame: 0,
            identity: self.identity,
            cursor_busy: CursorSignals::Prepared(self.signals),
            rows_pool: None,
            cleanup: None,
            prepared_panic_diagnostics: None,
            loan: Some(loan),
        }
    }
}
impl PreparedSqlSessionPool {
    /// Prepare region backing and the existing native Arc controls once at startup.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedSqlSessionCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand()?;
        let mut controls = FixedVector::prepare(memory.allocator(region)?, capacity.sessions)?;
        for _ in 0..capacity.sessions {
            let frames =
                FixedVector::prepare(memory.allocator(region)?, capacity.frames_per_session)?;
            let control = SessionControl {
                state: Arc::new(Mutex::new(SessionState {
                    owner: None,
                    phase: Phase::Closed,
                    depth: 0,
                    frames: FrameStorage::Prepared(frames),
                    generation: 0,
                    retired: false,
                })),
                signals: Arc::new(SessionSignals {
                    busy: AtomicBool::new(false),
                    pending_savepoint: AtomicBool::new(false),
                    loans: std::sync::Mutex::new(SessionLoans {
                        count: 0,
                        quota: None,
                    }),
                }),
            };
            if controls.push(control).is_err() {
                return Err(MemoryError::NoSlot);
            }
        }
        let mut controls = Some(controls);
        let pool = memory.slots(region, 1, |_| {
            Ok(std::sync::Mutex::new(
                controls.take().ok_or(MemoryError::NoSlot)?,
            ))
        })?;
        Ok(Self {
            registry: pool.checkout()?.share(),
            admission: None,
            cleanup: None,
        })
    }
    /// Couple each checked-out session control to one work slot in `admission`.
    /// Native connections and their allocations are separate from this charge.
    pub fn with_admission(mut self, admission: Admission<'static, 1>) -> Result<Self, MemoryError> {
        crate::prepared_rows::validate_slot_admission(&admission)?;
        self.admission = Some(admission);
        Ok(self)
    }
    /// Select prepared cleanup backing for root and nested transaction and atomic SQL operations.
    #[must_use]
    pub fn with_cleanup(mut self, cleanup: Arc<effect::PreparedCleanupConfig<SqlError>>) -> Self {
        self.cleanup = Some(cleanup);
        self
    }
    fn checkout(&self) -> Result<SessionCheckout, MemoryError> {
        let (mut checkout, old_owner) = {
            let mut registry = crate::try_prepared_lock(&self.registry)?;
            let mut found = None;
            let mut exhausted = false;
            for control in registry.as_mut_slice() {
                let Some(unique) = Arc::get_mut(&mut control.state) else {
                    continue;
                };
                let state = unique.get_mut();
                let FrameStorage::Prepared(frames) = &mut state.frames else {
                    continue;
                };
                if frames.capacity() == 0 {
                    continue;
                }
                let Some(generation) = state.generation.checked_add(1) else {
                    exhausted = true;
                    continue;
                };
                frames.clear();
                if frames.push(generation).is_err() {
                    continue;
                }
                state.generation = generation;
                let old_owner = state.owner.take();
                state.phase = Phase::Closed;
                state.depth = 0;
                state.retired = false;
                control.signals.busy.store(false, Ordering::Release);
                control
                    .signals
                    .loans
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .count = 0;
                found = Some((
                    SessionCheckout {
                        state: control.state.clone(),
                        signals: control.signals.clone(),
                        identity: generation,
                        quota: None,
                    },
                    old_owner,
                ));
                break;
            }
            found.ok_or(if exhausted {
                MemoryError::OwnerLimit
            } else {
                MemoryError::NoSlot
            })?
        };
        // An abandoned former session may still have a native owner. Retire it
        // after excluding old aliases and after releasing the registry gate.
        drop(old_owner);
        if let Some(admission) = &self.admission {
            checkout.quota = Some(admission.reserve([1], [0])?);
        }
        Ok(checkout)
    }
}

/// Shared handle to one exclusive session. Sharing a handle does not duplicate transaction authority.
#[derive(Clone)]
#[must_use]
pub struct NativeSession {
    /// Locked state retained by in-flight native cursors.
    state: Arc<Mutex<SessionState>>,
    /// Frame this handle is authorized to access.
    frame: usize,
    /// Session-local generation of the frame, invalidated when it settles.
    identity: u64,
    /// Fail reentrant cursor callbacks before waiting for the guard they themselves retain.
    /// Actual frame authority is still validated only after exclusive lock acquisition.
    cursor_busy: CursorSignals,
    rows_pool: Option<NativeRows>,
    cleanup: Option<Arc<effect::PreparedCleanupConfig<SqlError>>>,
    prepared_panic_diagnostics: Option<Arc<effect::PreparedPanicPool>>,
    /// Last field retires prepared authority after the handle's other owners.
    loan: Option<SessionLoan>,
}
impl NativeSession {
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    fn check_pending(&self) -> Result<(), SqlError> {
        if self.loan.as_ref().is_some_and(|loan| {
            loan.signals.pending_savepoint.load(Ordering::Acquire)
        }) {
            Err(SqlError::Memory(MemoryError::Busy))
        } else {
            Ok(())
        }
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) fn reserve_savepoint(&self) -> Result<Option<SavepointReservation>, SqlError> {
        if self.loan.is_none() {
            return Ok(None);
        }
        self.check_cursor()?;
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| SqlError::Memory(MemoryError::Busy))?;
        self.reserve_savepoint_locked(&mut state)
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn reserve_savepoint_wait(
        &self,
    ) -> Result<Option<SavepointReservation>, SqlError> {
        if self.loan.is_none() {
            return Ok(None);
        }
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        self.reserve_savepoint_locked(&mut state)
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    fn reserve_savepoint_locked(
        &self,
        state: &mut SessionState,
    ) -> Result<Option<SavepointReservation>, SqlError> {
        let Some(loan) = &self.loan else {
            return Ok(None);
        };
        state.connection(self.frame, self.identity)?;
        if self.frame == 0 {
            return Err(SqlError::InactiveSession);
        }
        state
            .frames
            .preflight(state.generation)
            .map_err(SqlError::Memory)?;
        if loan.signals.pending_savepoint.swap(true, Ordering::AcqRel) {
            return Err(SqlError::Memory(MemoryError::Busy));
        }
        match loan.try_clone_prepared() {
            Ok(loan) => Ok(Some(SavepointReservation {
                loan: Some(loan),
                frame: self.frame,
                identity: self.identity,
            })),
            Err(error) => {
                loan.signals.pending_savepoint.store(false, Ordering::Release);
                Err(SqlError::Memory(error))
            }
        }
    }
    /// A cursor callback cannot wait for its own retained exclusive guard.
    /// The flag only rejects busy work; it never grants access. Frame/lifecycle
    /// checks still happen together with the native mutable borrow under the lock.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    fn check_cursor(&self) -> Result<(), SqlError> {
        if self.cursor_busy.busy().load(Ordering::Acquire) {
            Err(SqlError::InactiveSession)
        } else {
            Ok(())
        }
    }
    /// Establish private state for a transferred/acquired owner with no active transaction.
    fn new(owner: ConnectionOwner) -> Self {
        Self {
            state: Arc::new(Mutex::new(SessionState {
                owner: Some(owner),
                phase: Phase::Active,
                depth: 0,
                frames: FrameStorage::Ordinary(vec![0]),
                generation: 0,
                retired: false,
            })),
            frame: 0,
            identity: 0,
            cursor_busy: CursorSignals::Ordinary(Arc::new(AtomicBool::new(false))),
            rows_pool: None,
            cleanup: None,
            prepared_panic_diagnostics: None,
            loan: None,
        }
    }
    /// Transfer an already owned SQLx connection, preserving the driver's native/sync adoption behavior.
    pub fn adopt(connection: TursoConnection) -> Self {
        Self::new(ConnectionOwner::Transferred(connection))
    }
    /// Run a lifetime-bound callback under exclusive access. Borrowed values cannot outlive the callback.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn with_native<A: Send, F>(&self, body: F) -> Result<A, SqlError>
    where
        F: for<'a> FnOnce(&'a mut TursoConnection) -> NativeFuture<'a, A> + Send,
    {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        state.connection(self.frame, self.identity)?;
        self.check_pending()?;
        state
            .owner
            .as_mut()
            .ok_or(SqlError::InactiveSession)?
            .retire();
        state.retired = true;
        let result = std::panic::AssertUnwindSafe(async {
            body(state.connection(self.frame, self.identity)?).await
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
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn execute(&self, statement: SqlStatement) -> Result<TursoQueryResult, SqlError> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        state.connection(self.frame, self.identity)?;
        self.check_pending()?;
        let (text, arguments, _statement_retention) = statement.into_retained_parts();
        sqlx::query_with(AssertSqlSafe(text), arguments)
            .execute(state.connection(self.frame, self.identity)?)
            .await
            .map_err(Into::into)
    }
    /// Buffer native owned rows. Decode failure remains separate from empty result sets.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn query(&self, statement: SqlStatement) -> Result<Vec<TursoRow>, SqlError> {
        self.read_query_rows(statement, |rows: &mut Vec<TursoRow>, row| {
            rows.try_reserve(1).map_err(|_| MemoryError::CollectionReservationFailed)?;
            rows.push(row);
            Ok(())
        }).await
    }

    /// Stage rows for post-fetch decoding. Each block can retire during decoding.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub(crate) async fn query_staged(
        &self,
        statement: SqlStatement,
    ) -> Result<Vec<Vec<TursoRow>>, SqlError> {
        const ROWS_PER_BLOCK: usize = 64;
        self.read_query_rows(statement, |blocks: &mut Vec<Vec<TursoRow>>, row| {
            if let Some(block) = blocks.last_mut()
                && block.len() < ROWS_PER_BLOCK
            {
                block.try_reserve(1).map_err(|_| MemoryError::CollectionReservationFailed)?;
                block.push(row);
            } else {
                blocks.try_reserve(1).map_err(|_| MemoryError::CollectionReservationFailed)?;
                let mut block = Vec::new();
                block.try_reserve(1).map_err(|_| MemoryError::CollectionReservationFailed)?;
                block.push(row);
                blocks.push(block);
            }
            Ok(())
        }).await
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    async fn read_query_rows<B: Default + Send>(
        &self,
        statement: SqlStatement,
        mut retain: impl FnMut(&mut B, TursoRow) -> Result<(), MemoryError> + Send,
    ) -> Result<B, SqlError> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        state.connection(self.frame, self.identity)?;
        self.check_pending()?;
        let (text, arguments, _statement_retention) = statement.into_retained_parts();
        let mut source = sqlx::query_with(AssertSqlSafe(text), arguments)
            .fetch(state.connection(self.frame, self.identity)?);
        let mut rows = Ok(B::default());
        while let Some(row) = source.try_next().await? {
            if let Ok(buffer) = &mut rows
                && let Err(error) = retain(buffer, row)
            {
                // Drop staged rows, then drain so a later native error stays primary.
                rows = Err(error);
            }
        }
        rows.map_err(SqlError::Memory)
    }
    /// Fetch exactly the first optional row without buffering subsequent rows.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn first(&self, statement: SqlStatement) -> Result<Option<TursoRow>, SqlError> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        state.connection(self.frame, self.identity)?;
        self.check_pending()?;
        let (text, arguments, _statement_retention) = statement.into_retained_parts();
        sqlx::query_with(AssertSqlSafe(text), arguments)
            .fetch_optional(state.connection(self.frame, self.identity)?)
            .await
            .map_err(Into::into)
    }
    /// Retain this session's exclusive lease through provider cursor consumption.
    /// Unbound multi-command SQL completes as a native batch before yielding its first item.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn cursor(&self, statement: SqlStatement) -> Result<NativeCursor, SqlError> {
        self.check_cursor()?;
        let loan;
        let mut state = self.state.clone().lock_owned().await;
        state.connection(self.frame, self.identity)?;
        self.check_pending()?;
        loan = self.try_loan().map_err(SqlError::Memory)?;
        let (text, arguments, _statement_retention) = statement.into_retained_parts();
        let items = state
            .connection(self.frame, self.identity)?
            .owned_cursor(
                AssertSqlSafe(text).into_sql_str(),
                if arguments.len() == 0 {
                    None
                } else {
                    Some(arguments)
                },
            )
            .await?;
        self.cursor_busy.busy().store(true, Ordering::Release);
        Ok(NativeCursor {
            items: CursorItems::Ordinary(items),
            _row_lane: None,
            _statement_retention,
            _state: state,
            cursor_busy: self.cursor_busy.clone(),
            _loan: loan,
        })
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub(crate) fn reserve_optional_rows(
        &self,
    ) -> Result<Option<ReservedNativeRows>, SqlError> {
        self.rows_pool.as_ref().map(NativeRows::reserve).transpose()
    }
    pub(crate) fn reserve_optional_cleanup(
        &self,
    ) -> Result<Option<effect::CleanupFailures<SqlError>>, MemoryError> {
        self.cleanup
            .as_ref()
            .map(|cleanup| cleanup.reserve_failures())
            .transpose()
    }
    pub(crate) fn prepared_panic_diagnostics(&self) -> Option<&effect::PreparedPanicPool> {
        self.prepared_panic_diagnostics.as_deref()
    }

    pub(crate) fn is_prepared(&self) -> bool {
        self.loan.is_some()
    }

    fn try_loan(&self) -> Result<Option<SessionLoan>, MemoryError> {
        self.loan.as_ref().map(SessionLoan::try_clone_prepared).transpose()
    }

    pub(crate) fn try_clone_prepared(&self) -> Result<Self, MemoryError> {
        let loan = self.try_loan()?;
        Ok(self.with_loan(loan))
    }

    fn with_loan(&self, loan: Option<SessionLoan>) -> Self {
        Self {
            state: self.state.clone(),
            frame: self.frame,
            identity: self.identity,
            cursor_busy: self.cursor_busy.clone(),
            rows_pool: self.rows_pool.clone(),
            cleanup: self.cleanup.clone(),
            prepared_panic_diagnostics: self.prepared_panic_diagnostics.clone(),
            loan,
        }
    }

    pub(crate) fn into_savepoint_body(mut self, nested: &Self) -> Self {
        // The owning runner derived nested from this session. Transfer its
        // existing parent loan with the newly admitted child's authority.
        self.frame = nested.frame;
        self.identity = nested.identity;
        self
    }

    pub(crate) fn transaction_body(&self, callback: SessionLoan) -> Self {
        self.with_loan(Some(callback))
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub(crate) async fn incremental_cursor_reserved(
        &self,
        statement: SqlStatement,
        reservation: Option<ReservedNativeRows>,
    ) -> Result<BorrowedNativeCursor<'_>, SqlError> {
        self.check_cursor()?;
        if self.rows_pool.is_some() && reservation.is_none() {
            return Err(SqlError::Memory(MemoryError::UnsupportedRunContract));
        }
        let mut state = self.state.clone().lock_owned().await;
        state.connection(self.frame, self.identity)?;
        self.check_pending()?;
        let cursor = self.open_incremental_cursor(statement, reservation, state).await?;
        Ok(BorrowedNativeCursor { cursor, _session: std::marker::PhantomData })
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    async fn open_incremental_cursor(
        &self,
        statement: SqlStatement,
        reservation: Option<ReservedNativeRows>,
        mut state: OwnedMutexGuard<SessionState>,
    ) -> Result<NativeCursor, SqlError> {
        let (text, arguments, _statement_retention) = statement.into_retained_parts();
        let arguments = if arguments.len() == 0 {
            None
        } else {
            Some(arguments)
        };
        let connection = state.connection(self.frame, self.identity)?;
        let (items, row_lane) = match reservation {
            Some(ReservedNativeRows { permit, lane }) => {
                // The provider future is retired before `lane` on refusal or cancellation.
                let items = connection
                    .owned_incremental_cursor_reserved(
                        AssertSqlSafe(text).into_sql_str(),
                        arguments,
                        permit,
                    )
                    .await?;
                (CursorItems::Prepared(items), lane)
            }
            None => {
                let items = connection
                    .owned_incremental_cursor(AssertSqlSafe(text).into_sql_str(), arguments)
                    .await?;
                (CursorItems::Ordinary(items), None)
            }
        };
        self.cursor_busy.busy().store(true, Ordering::Release);
        Ok(NativeCursor {
            items,
            _row_lane: row_lane,
            _statement_retention,
            _state: state,
            cursor_busy: self.cursor_busy.clone(),
            _loan: None,
        })
    }

    /// Open an exclusive cursor that yields before fetching the next row.
    /// Argument-free multi-command statements execute sequentially as consumed,
    /// so a later error may follow earlier results. No transaction is added;
    /// native statement and row backing remains separately allocated.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn incremental_cursor(
        &self,
        statement: SqlStatement,
    ) -> Result<NativeCursor, SqlError> {
        self.check_cursor()?;
        let loan;
        let mut state = self.state.clone().lock_owned().await;
        state.connection(self.frame, self.identity)?;
        self.check_pending()?;
        let reservation = self.reserve_optional_rows()?;
        loan = self.try_loan().map_err(SqlError::Memory)?;
        let mut cursor = self.open_incremental_cursor(statement, reservation, state).await?;
        cursor._loan = loan;
        Ok(cursor)
    }
    /// Start an operation-owned transaction, using BEGIN IMMEDIATE for local write coordination.
    /// State and provider acknowledgment remain under one lock; no cancellation select splits them.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn begin(&self) -> Result<Self, SqlError> {
        self.begin_direct().await.map_err(CloseFault::into_sql)
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn begin_direct(&self) -> Result<Self, CloseFault> {
        self.begin_direct_with_panic(&mut None).await
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn begin_direct_with_panic(
        &self,
        diagnostic: &mut Option<effect::PreparedPanicReservation>,
    ) -> Result<Self, CloseFault> {
        self.check_cursor()?;
        let loan;
        let mut state = self.state.lock().await;
        state.connection(self.frame, self.identity)?;
        self.check_pending()?;
        if self.frame != 0 {
            return Err(SqlError::InactiveSession.into());
        }
        let owner = state.owner.as_mut().ok_or(SqlError::InactiveSession)?;
        if owner.connection().options().sync_options().is_some() {
            return Err(SqlError::ReplicaNotAuthoritative.into());
        }
        let generation = state.generation;
        let next = state
            .frames
            .preflight(generation)
            .map_err(SqlError::Memory)?;
        loan = self.try_loan().map_err(SqlError::Memory)?;
        state
            .owner
            .as_mut()
            .ok_or(SqlError::InactiveSession)?
            .retire();
        state.retired = true;
        state.phase = Phase::Finishing;
        let result = begin_fault_with(
            state
                .owner
                .as_mut()
                .ok_or(SqlError::InactiveSession)?
                .connection(),
            true,
            diagnostic,
        )
        .await;
        match result {
            Ok(()) => {
                state.depth = 1;
                if let Err(error) = state.frames.push(next) {
                    state.phase = Phase::Uncertain;
                    return Err(SqlError::Memory(error).into());
                }
                state.generation = next;
                state.phase = Phase::Active;
                Ok(Self {
                    state: self.state.clone(),
                    frame: 1,
                    identity: next,
                    cursor_busy: self.cursor_busy.clone(),
                    rows_pool: self.rows_pool.clone(),
                    cleanup: self.cleanup.clone(),
                    prepared_panic_diagnostics: self.prepared_panic_diagnostics.clone(),
                    loan,
                })
            }
            Err(error) => {
                state.phase = Phase::Uncertain;
                Err(error)
            }
        }
    }
    /// Begin a nested SQLx savepoint on the same owner. Parent handles are fenced until it resolves.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn begin_savepoint(&self) -> Result<Self, SqlError> {
        self.begin_savepoint_direct()
            .await
            .map_err(CloseFault::into_sql)
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn begin_savepoint_direct(&self) -> Result<Self, CloseFault> {
        self.begin_savepoint_with_admission(None).await
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn begin_savepoint_with_admission(
        &self,
        reservation: Option<SavepointReservation>,
    ) -> Result<Self, CloseFault> {
        self.begin_savepoint_with_panic(reservation, &mut None).await
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn begin_savepoint_with_panic(
        &self,
        reservation: Option<SavepointReservation>,
        diagnostic: &mut Option<effect::PreparedPanicReservation>,
    ) -> Result<Self, CloseFault> {
        self.check_cursor()?;
        // A failed native transition retires the reserved loan after the state guard.
        let reserved_loan;
        let mut state = self.state.lock().await;
        state.connection(self.frame, self.identity)?;
        match &reservation {
            Some(reservation) if !reservation.matches(self, &state) => {
                return Err(SqlError::InactiveSession.into());
            }
            Some(_) => {}
            None => self.check_pending()?,
        }
        if self.frame == 0 {
            return Err(SqlError::InactiveSession.into());
        }
        let generation = state.generation;
        let next = state
            .frames
            .preflight(generation)
            .map_err(SqlError::Memory)?;
        reserved_loan = match reservation {
            Some(reservation) => reservation.redeem(),
            None => self.try_loan().map_err(SqlError::Memory)?,
        };
        state.phase = Phase::Finishing;
        let result = begin_fault_with(
            state
                .owner
                .as_mut()
                .ok_or(SqlError::InactiveSession)?
                .connection(),
            false,
            diagnostic,
        )
        .await;
        match result {
            Ok(()) => {
                state.depth += 1;
                if let Err(error) = state.frames.push(next) {
                    state.phase = Phase::Uncertain;
                    return Err(SqlError::Memory(error).into());
                }
                state.generation = next;
                state.phase = Phase::Active;
                Ok(Self {
                    state: self.state.clone(),
                    frame: self.frame + 1,
                    identity: next,
                    cursor_busy: self.cursor_busy.clone(),
                    rows_pool: self.rows_pool.clone(),
                    cleanup: self.cleanup.clone(),
                    prepared_panic_diagnostics: self.prepared_panic_diagnostics.clone(),
                    loan: reserved_loan,
                })
            }
            Err(error) => {
                state.phase = Phase::Uncertain;
                Err(error)
            }
        }
    }
    /// Await commit/savepoint release. A failed acknowledgment fences the entire owner as uncertain.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn commit(&self) -> Result<(), SqlError> {
        self.commit_direct().await.map_err(CommitFault::into_sql)
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn commit_direct(&self) -> Result<(), CommitFault> {
        self.commit_direct_with_panic(&mut None).await
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn commit_direct_with_panic(
        &self,
        diagnostic: &mut Option<effect::PreparedPanicReservation>,
    ) -> Result<(), CommitFault> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        state.connection(self.frame, self.identity)?;
        if self.frame == 0 {
            return Err(SqlError::InactiveSession.into());
        }
        state.phase = Phase::Finishing;
        let result = commit_fault_with(
            state
                .owner
                .as_mut()
                .ok_or(SqlError::InactiveSession)?
                .connection(),
            self.frame == 1,
            diagnostic,
        )
        .await;
        match result {
            Ok(()) => {
                state.depth -= 1;
                state.frames.pop();
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
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn rollback(&self) -> Result<(), SqlError> {
        self.rollback_direct().await.map_err(CloseFault::into_sql)
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn rollback_direct(&self) -> Result<(), CloseFault> {
        self.rollback_direct_with_panic(&mut None).await
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn rollback_direct_with_panic(
        &self,
        diagnostic: &mut Option<effect::PreparedPanicReservation>,
    ) -> Result<(), CloseFault> {
        self.check_cursor()?;
        let mut state = self.state.lock().await;
        if state.depth != self.frame
            || self.frame == 0
            || state.phase == Phase::Closed
            || !state.frames.matches(self.frame, self.identity)
        {
            return Err(SqlError::InactiveSession.into());
        }
        state.phase = Phase::Finishing;
        let connection = state
            .owner
            .as_mut()
            .ok_or(SqlError::InactiveSession)?
            .connection();
        let result = rollback_fault_with(connection, diagnostic).await;
        // SQLx rollback-to-savepoint does not RELEASE; explicitly release its native frame.
        let result = if result.is_ok() && self.frame > 1 {
            use std::io::Write as _;
            const PREFIX: &str = "RELEASE SAVEPOINT _sqlx_savepoint_";
            let mut bytes = [0u8; PREFIX.len() + usize::BITS as usize];
            let length = {
                let mut cursor = std::io::Cursor::new(&mut bytes[..]);
                write!(&mut cursor, "{PREFIX}{}", self.frame - 1)
                    .ok()
                    .and_then(|()| usize::try_from(cursor.position()).ok())
            };
            match length
                .and_then(|length| bytes.get(..length))
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
            {
                Some(sql) => match std::panic::AssertUnwindSafe(async {
                    connection.native_execute(sql, ()).await
                })
                .catch_unwind()
                .await
                {
                    Ok(result) => result
                        .map(|_| ())
                        .map_err(|error| CloseFault::Sql(SqlError::Provider(error))),
                    Err(panic) => Err(CloseFault::Defect(capture_panic(diagnostic, panic))),
                },
                None => Err(CloseFault::Sql(SqlError::Memory(
                    MemoryError::InvalidLayout,
                ))),
            }
        } else {
            result
        };
        match result {
            Ok(()) => {
                state.depth -= 1;
                state.frames.pop();
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
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn close(&self) -> Result<(), SqlError> {
        let failures = self.close_failures().await?;
        let cleanup = match (
            failures.rollback.map(CloseFault::into_sql),
            failures.settle.map(CloseFault::into_sql),
        ) {
            (Some(primary), Some(cleanup)) => Some(SqlError::Cleanup {
                primary: Some(Box::new(primary)),
                cleanup: Box::new(cleanup),
            }),
            (Some(error), None) | (None, Some(error)) => Some(error),
            (None, None) => None,
        };
        match (cleanup, failures.close.map(CloseFault::into_sql)) {
            (Some(primary), Some(cleanup)) => Err(SqlError::Cleanup {
                primary: Some(Box::new(primary)),
                cleanup: Box::new(cleanup),
            }),
            (Some(error), None) | (None, Some(error)) => Err(error),
            (None, None) => Ok(()),
        }
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn close_failures(&self) -> Result<CloseFailures, SqlError> {
        self.close_failures_with_panic(&mut None, &mut None, &mut None).await
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) async fn close_failures_with_panic(
        &self,
        rollback_diagnostic: &mut Option<effect::PreparedPanicReservation>,
        settle_diagnostic: &mut Option<effect::PreparedPanicReservation>,
        close_diagnostic: &mut Option<effect::PreparedPanicReservation>,
    ) -> Result<CloseFailures, SqlError> {
        let mut state = self.state.lock().await;
        if self.frame != 0 {
            return Err(SqlError::InactiveSession);
        }
        if state.phase == Phase::Closed {
            return Ok(CloseFailures {
                rollback: None,
                settle: None,
                close: None,
            });
        }
        if !state.frames.matches(0, self.identity) {
            return Err(SqlError::InactiveSession);
        }
        let uncertain = state.phase == Phase::Uncertain || state.phase == Phase::Finishing;
        let mut owner = state.owner.take().ok_or(SqlError::InactiveSession)?;
        state.phase = Phase::Closed;
        state.frames.clear();
        let mut rollback = None;
        while state.depth > 0 {
            match rollback_fault_with(owner.connection(), rollback_diagnostic).await {
                Ok(()) => state.depth -= 1,
                Err(error) => {
                    rollback = Some(error);
                    break;
                }
            }
        }
        let settle = settle_fault_with(owner.connection(), settle_diagnostic).await.err();
        if uncertain || rollback.is_some() || settle.is_some() {
            owner.retire();
        }
        // A resolved ordinary pooled lease returns to its native pool. Transferred
        // owners and any uncertain/transaction lease are explicitly closed.
        let close = match owner {
            ConnectionOwner::Pooled(connection)
                if !uncertain && !state.retired && rollback.is_none() && settle.is_none() =>
            {
                drop(connection);
                Ok(())
            }
            owner @ (ConnectionOwner::Pooled(_) | ConnectionOwner::Transferred(_)) => {
                owner.close_fault_with(close_diagnostic).await
            }
        };
        Ok(CloseFailures {
            rollback,
            settle,
            close: close.err(),
        })
    }
}

/// An incremental SQLx cursor owning the exclusive session guard until disposal.
#[must_use]
pub struct NativeCursor {
    /// Native iterator is dropped before its exclusive guard is released.
    items: CursorItems,
    /// Exclusive lane returns only after the provider iterator has retired.
    _row_lane: Option<RowLaneLease>,
    /// Prepared source retires after the native iterator and before the session guard.
    _statement_retention: crate::statement::StatementRetention,
    /// Retained connection authority; borrowing prevents a pool reuse race.
    _state: OwnedMutexGuard<SessionState>,
    /// Shared rejection flag; no connection/frame authority is derived from it.
    cursor_busy: CursorSignals,
    /// The guard above must retire before the final prepared loan.
    _loan: Option<SessionLoan>,
}
impl Drop for NativeCursor {
    /// Clear busy rejection while the guard still prevents any native access.
    /// Field order then drops statement/rows before releasing the session guard.
    fn drop(&mut self) {
        self.cursor_busy.busy().store(false, Ordering::Release);
    }
}
impl NativeCursor {
    /// Pull one native result item; batches preserve row/result ordering.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn next_item(
        &mut self,
    ) -> Result<Option<either::Either<TursoQueryResult, TursoRow>>, SqlError> {
        match &mut self.items {
            CursorItems::Ordinary(items) => {
                items.next().await.transpose().map_err(SqlError::Provider)
            }
            CursorItems::Prepared(items) => {
                items.next().await.transpose().map_err(|error| match error {
                    sqlx::PreparedTursoCursorError::Native(error) => SqlError::Provider(error),
                    sqlx::PreparedTursoCursorError::Memory(error) => SqlError::PreparedRow(error),
                })
            }
        }
    }
    /// Pull one row while retaining result metadata internally; no row buffering occurs.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
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
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub async fn close(self) -> Result<(), SqlError> {
        drop(self);
        Ok(())
    }
}

/// The atomic cursor borrows the existing session loan until its native guard retires.
pub(crate) struct BorrowedNativeCursor<'a> {
    cursor: NativeCursor,
    _session: std::marker::PhantomData<&'a NativeSession>,
}
impl BorrowedNativeCursor<'_> {
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub(crate) async fn next_item(
        &mut self,
    ) -> Result<Option<either::Either<TursoQueryResult, TursoRow>>, SqlError> {
        self.cursor.next_item().await
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub(crate) async fn next_row(&mut self) -> Result<Option<TursoRow>, SqlError> {
        self.cursor.next_row().await
    }
}

/// Start through SQLx's existing manager while retaining native defects in an invocation-local carrier.
#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
async fn begin_native(connection: &mut TursoConnection, immediate: bool) -> Result<(), SqlError> {
    begin_fault(connection, immediate)
        .await
        .map_err(CloseFault::into_sql)
}
#[expect(
    clippy::result_large_err,
    reason = "retains owned SQL diagnostics without an extra heap allocation"
)]
async fn begin_fault(connection: &mut TursoConnection, immediate: bool) -> Result<(), CloseFault> {
    begin_fault_with(connection, immediate, &mut None).await
}
#[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
async fn begin_fault_with(
    connection: &mut TursoConnection,
    immediate: bool,
    diagnostic: &mut Option<effect::PreparedPanicReservation>,
) -> Result<(), CloseFault> {
    match std::panic::AssertUnwindSafe(sqlx::TursoTransactionManager::begin_owned(
        connection, immediate,
    ))
    .catch_unwind()
    .await
    {
        Ok(result) => result.map_err(|error| CloseFault::Sql(SqlError::Provider(error))),
        Err(panic) => Err(CloseFault::Defect(capture_panic(diagnostic, panic))),
    }
}
/// Await commit and its bookkeeping together; acknowledgment failure stays uncertain.
#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
async fn commit_native(connection: &mut TursoConnection, durable: bool) -> Result<(), SqlError> {
    commit_fault(connection, durable)
        .await
        .map_err(CommitFault::into_sql)
}
#[expect(
    clippy::result_large_err,
    reason = "retains owned SQL diagnostics without an extra heap allocation"
)]
async fn commit_fault(connection: &mut TursoConnection, durable: bool) -> Result<(), CommitFault> {
    commit_fault_with(connection, durable, &mut None).await
}
#[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
async fn commit_fault_with(
    connection: &mut TursoConnection,
    durable: bool,
    diagnostic: &mut Option<effect::PreparedPanicReservation>,
) -> Result<(), CommitFault> {
    match std::panic::AssertUnwindSafe(sqlx::TursoTransactionManager::commit_owned(connection))
        .catch_unwind()
        .await
    {
        Ok(result) => result.map_err(|error| {
            CommitFault::Sql(if durable {
                SqlError::CommitOutcomeUnknown(error)
            } else {
                SqlError::Provider(error)
            })
        }),
        Err(panic) => Err(CommitFault::Defect {
            defect: capture_panic(diagnostic, panic),
            durable,
        }),
    }
}
/// Await rollback through the existing manager; a dropped future is never an acknowledgment.
#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
async fn rollback_native(connection: &mut TursoConnection) -> Result<(), SqlError> {
    rollback_fault(connection)
        .await
        .map_err(CloseFault::into_sql)
}
#[expect(
    clippy::result_large_err,
    reason = "retains owned SQL diagnostics without an extra heap allocation"
)]
async fn rollback_fault(connection: &mut TursoConnection) -> Result<(), CloseFault> {
    rollback_fault_with(connection, &mut None).await
}
#[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
async fn rollback_fault_with(
    connection: &mut TursoConnection,
    diagnostic: &mut Option<effect::PreparedPanicReservation>,
) -> Result<(), CloseFault> {
    match std::panic::AssertUnwindSafe(sqlx::TursoTransactionManager::rollback_owned(connection))
        .catch_unwind()
        .await
    {
        Ok(result) => result.map_err(|error| CloseFault::Sql(SqlError::Provider(error))),
        Err(panic) => Err(CloseFault::Defect(capture_panic(diagnostic, panic))),
    }
}
/// Join native cleanup without replacing primary failure or claiming retry safety.
#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
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
#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
async fn settle_native(connection: &mut TursoConnection) -> Result<(), SqlError> {
    settle_fault(connection).await.map_err(CloseFault::into_sql)
}
#[expect(
    clippy::result_large_err,
    reason = "retains owned SQL diagnostics without an extra heap allocation"
)]
async fn settle_fault(connection: &mut TursoConnection) -> Result<(), CloseFault> {
    settle_fault_with(connection, &mut None).await
}
#[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
async fn settle_fault_with(
    connection: &mut TursoConnection,
    diagnostic: &mut Option<effect::PreparedPanicReservation>,
) -> Result<(), CloseFault> {
    match std::panic::AssertUnwindSafe(connection.settle_owned())
        .catch_unwind()
        .await
    {
        Ok(result) => result.map_err(|error| CloseFault::Sql(SqlError::Provider(error))),
        Err(panic) => Err(CloseFault::Defect(capture_panic(diagnostic, panic))),
    }
}
/// Resolve native state, then await disposal of an independently owned pooled lease.
#[expect(
    clippy::result_large_err,
    reason = "retains the owned SQL error representation without an extra heap allocation"
)]
async fn finish_pool(mut connection: sqlx::pool::PoolConnection<Turso>) -> Result<(), SqlError> {
    let settlement = settle_native(&mut connection).await;
    let close = ConnectionOwner::Pooled(connection).close().await;
    join_cleanup(settlement, close)
}
