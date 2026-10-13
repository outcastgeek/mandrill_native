//! SQLx-compatible history, database write coordination and deliberate dirty recovery.
//!
//! Transactional steps use operation-owned BEGIN IMMEDIATE across history revalidation
//! and schema/history changes. Nontransactional steps require a caller-selected
//! exclusive-maintenance provider; a filesystem lock is never distributed authority.

use crate::effect_future;
use crate::{Identifier, MigrationPhase, SqlError, SqlKernel};
use effect::{Defect, Effect, Exit};
use futures_util::FutureExt;
use sqlx::{AssertSqlSafe, Row, SqlSafeStr, TursoConnection};
use std::{borrow::Cow, sync::Arc, time::Instant};

/// SQLx's established journal identity, reused rather than creating parallel history.
pub const SQLX_JOURNAL: &str = "_sqlx_migrations";
/// Versioned extension metadata: dirty direction/checksum only, never a second applied history.
pub const DIRTY_METADATA: &str = "_effect_sql_migration_state_v1";

/// Explicit reversibility declaration; irreversible migrations fail down preflight.
#[derive(Clone)]
pub enum Rollback {
    /// Native SQLx down migration with the same stable version as its up step.
    Reversible(sqlx::migrate::Migration),
    /// Schema rollback is deliberately unavailable and cannot be guessed.
    Irreversible,
}
/// One static catalog entry preserving SQLx checksums and native SQL execution.
#[derive(Clone)]
#[must_use]
pub struct MigrationStep {
    /// Native SQLx migration; checksum is SQLx SHA-384 over its native SQL.
    pub up: sqlx::migrate::Migration,
    /// Explicit down contract required by the constructor.
    pub rollback: Rollback,
}
impl MigrationStep {
    /// Declare a native migration and its explicit reversibility without performing I/O.
    pub fn new(up: sqlx::migrate::Migration, rollback: Rollback) -> Self {
        Self { up, rollback }
    }
    /// Construct a transactional literal step with SQLx's own checksum calculation.
    pub fn transactional(
        version: i64,
        name: &'static str,
        up: &'static str,
        down: Rollback,
    ) -> Self {
        Self::new(
            sqlx::migrate::Migration::new(
                version,
                Cow::Borrowed(name),
                sqlx::migrate::MigrationType::Simple,
                up.into_sql_str(),
                false,
            ),
            down,
        )
    }
}
/// Validated immutable catalog with one selected SQLx journal and ordered unique identities.
#[derive(Clone)]
#[must_use]
pub struct MigrationCatalog {
    /// Stable journal name from the SQLx schema owner, never source-position generated.
    journal: String,
    /// Quoted provider journal syntax.
    quoted: String,
    /// Static metadata shared without copying execution payloads.
    steps: Arc<[MigrationStep]>,
}
impl MigrationCatalog {
    /// Validate strict ordering, unique IDs, checksums and explicit reversible version agreement.
    /// Use the existing journal for a schema; a custom journal must be an explicit schema contract.
    pub fn new(journal: impl Into<String>, steps: Vec<MigrationStep>) -> Result<Self, SqlError> {
        let journal = journal.into();
        let quoted = Identifier::new(&journal)?.quoted().to_owned();
        let mut previous = None;
        for step in &steps {
            if previous.is_some_and(|version| version >= step.up.version)
                || !step.up.migration_type.is_up_migration()
                || step.up.description.is_empty()
                || step.up.sql.as_str().trim().is_empty()
                || step.up.checksum.as_ref()
                    != sqlx::migrate::Migration::new(
                        step.up.version,
                        step.up.description.clone(),
                        step.up.migration_type,
                        step.up.sql.clone(),
                        step.up.no_tx,
                    )
                    .checksum
                    .as_ref()
            {
                return Err(migration_failure(
                    MigrationPhase::Validate,
                    Some(step.up.version),
                    SqlError::Configuration("invalid ordered SQLx migration catalog".into()),
                ));
            }
            if let Rollback::Reversible(down) = &step.rollback
                && (!down.migration_type.is_down_migration()
                    || down.version != step.up.version
                    || down.sql.as_str().trim().is_empty()
                    || down.checksum.as_ref()
                        != sqlx::migrate::Migration::new(
                            down.version,
                            down.description.clone(),
                            down.migration_type,
                            down.sql.clone(),
                            down.no_tx,
                        )
                        .checksum
                        .as_ref())
            {
                return Err(migration_failure(
                    MigrationPhase::Validate,
                    Some(step.up.version),
                    SqlError::Configuration("invalid reversible migration".into()),
                ));
            }
            previous = Some(step.up.version);
        }
        Ok(Self {
            journal,
            quoted,
            steps: steps.into(),
        })
    }
    /// Borrow the exact SQLx journal selected for this schema.
    #[must_use]
    pub fn journal(&self) -> &str {
        &self.journal
    }
    /// Borrow immutable native metadata for static checked/provider interoperation.
    #[must_use]
    pub fn steps(&self) -> &[MigrationStep] {
        &self.steps
    }
}

/// Captured acknowledged SQLx history; only the migrator constructs its down plan.
#[derive(Clone, PartialEq, Eq)]
struct HistoryEntry {
    /// Stable native SQLx version.
    version: i64,
    /// SQLx-native immutable checksum.
    checksum: Vec<u8>,
}
/// Opaque preflighted down plan, fenced to both its catalog and exact captured history.
#[must_use]
pub struct DownPlan {
    /// Catalog used to establish reversibility.
    catalog: MigrationCatalog,
    /// Full history snapshot rechecked under database write authority.
    history: Vec<HistoryEntry>,
    /// Number of reversible tail entries selected by the caller.
    count: usize,
    /// Private selected database configuration; an identical history in another database grants no authority.
    kernel: SqlKernel,
}
impl DownPlan {
    /// Number of preflighted down steps; no schema writes have happened yet.
    #[must_use]
    pub fn count(&self) -> usize {
        self.count
    }
    /// Stable versions in actual rollback order, without exposing construction authority.
    #[must_use = "the returned iterator describes rollback order and performs no schema work"]
    pub fn versions(&self) -> impl Iterator<Item = i64> + '_ {
        self.history
            .iter()
            .rev()
            .take(self.count)
            .map(|entry| entry.version)
    }
}
/// Direction of an unresolved step; protocol metadata is stable across process restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationDirection {
    /// Up SQL may have run.
    Up,
    /// Down SQL may have run.
    Down,
}
/// Dirty step resolved from the selected catalog; its constructor is private.
#[must_use]
pub struct DirtyMigration {
    /// Stable SQLx migration identity.
    version: i64,
    /// Verified catalog name, not persisted application secrets.
    name: String,
    /// Catalog checksum copied as immutable metadata only.
    checksum: Vec<u8>,
    /// Which schema program needs assessment.
    direction: MigrationDirection,
    /// Existing native SQLx success=false history supplied this unresolved marker.
    native_dirty: bool,
}
impl DirtyMigration {
    /// Stable version requiring deliberate recovery.
    #[must_use]
    pub fn version(&self) -> i64 {
        self.version
    }
    /// Catalog name useful to an operator's selected schema assessment.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Expected native SQLx checksum for the declared step.
    #[must_use]
    pub fn checksum(&self) -> &[u8] {
        &self.checksum
    }
    /// Whether the unresolved schema program was an up or down step.
    #[must_use]
    pub fn direction(&self) -> MigrationDirection {
        self.direction
    }
}
/// Application's assessed actual schema result; never inferred from a timeout or dropped future.
pub enum RecoveryDecision {
    /// The declared schema program took effect.
    StepApplied,
    /// The declared schema program did not take effect.
    StepNotApplied,
}

/// Retained application/provider maintenance guard. Acquisition must exclude every schema writer.
/// The contract is deployment-specific and does not claim a replicated database lease.
pub trait MaintenanceGuard: Send {
    /// Await release of selected maintenance ownership; shutdown failures remain typed.
    fn release(self: Box<Self>) -> crate::NativeFuture<'static, ()>;
}
/// Caller-selected authority for schema statements that cannot run within a database transaction.
pub trait MaintenanceProvider: Send + Sync {
    /// Acquire real exclusive maintenance for the supplied authoritative configuration.
    /// A mere process mutex or filesystem lock cannot satisfy this across replicas.
    fn acquire(&self, kernel: SqlKernel)
    -> crate::NativeFuture<'static, Box<dyn MaintenanceGuard>>;
}

/// Inert migration service bound to one actual local kernel and immutable SQLx catalog.
#[derive(Clone)]
#[must_use]
pub struct SqlMigrator {
    /// Same native pool/authority selected for ordinary operations.
    kernel: SqlKernel,
    /// Validated immutable schema metadata.
    catalog: MigrationCatalog,
    /// Optional real external exclusive-maintenance authority.
    maintenance: Option<Arc<dyn MaintenanceProvider>>,
}
impl SqlMigrator {
    /// Select authoritative migration configuration without opening a connection.
    pub fn new(kernel: SqlKernel, catalog: MigrationCatalog) -> Result<Self, SqlError> {
        kernel.require_authoritative()?;
        Ok(Self {
            kernel,
            catalog,
            maintenance: None,
        })
    }
    /// Select an explicit maintenance provider for catalog steps declared nontransactional.
    pub fn with_maintenance(mut self, maintenance: Arc<dyn MaintenanceProvider>) -> Self {
        self.maintenance = Some(maintenance);
        self
    }
    /// Ensure compatible SQLx journal/metadata under database write authority.
    /// Native Identity startup calls this directly without invoking Effect.
    pub async fn initialize(&self) -> Result<(), SqlError> {
        let catalog = self.catalog.clone();
        self.kernel
            .transaction(move |connection| {
                Box::pin(async move { ensure(connection, &catalog).await })
            })
            .await
            .map_err(annotate_terminal)
    }
    /// Preflight unapplied versions under a consistent authoritative history snapshot.
    pub async fn plan_up(&self) -> Result<Vec<i64>, SqlError> {
        let catalog = self.catalog.clone();
        self.kernel
            .transaction(move |connection| {
                Box::pin(async move {
                    ensure(connection, &catalog).await?;
                    reject_dirty(connection, &catalog).await?;
                    let history = history(connection, &catalog).await?;
                    validate_history(&catalog, &history)?;
                    Ok(catalog
                        .steps
                        .iter()
                        .skip(history.len())
                        .map(|step| step.up.version)
                        .collect())
                })
            })
            .await
            .map_err(annotate_terminal)
    }
    /// Capture exactly count reversible applied tail steps without changing user schema.
    pub async fn plan_down(&self, count: usize) -> Result<DownPlan, SqlError> {
        let catalog = self.catalog.clone();
        let kernel = self.kernel.clone();
        self.kernel
            .transaction(move |connection| {
                Box::pin(async move {
                    ensure(connection, &catalog).await?;
                    reject_dirty(connection, &catalog).await?;
                    let history = history(connection, &catalog).await?;
                    validate_history(&catalog, &history)?;
                    if count > history.len() {
                        return Err(migration_failure(
                            MigrationPhase::History,
                            None,
                            SqlError::Configuration("down count exceeds applied history".into()),
                        ));
                    }
                    for step in catalog.steps.iter().take(history.len()).rev().take(count) {
                        if matches!(step.rollback, Rollback::Irreversible) {
                            return Err(migration_failure(
                                MigrationPhase::Validate,
                                Some(step.up.version),
                                SqlError::Configuration(
                                    "irreversible migration in down plan".into(),
                                ),
                            ));
                        }
                    }
                    Ok(DownPlan {
                        catalog,
                        history,
                        count,
                        kernel,
                    })
                })
            })
            .await
            .map_err(annotate_terminal)
    }
    /// Apply all pending transactional steps atomically, revalidating history under BEGIN IMMEDIATE.
    /// Nontransactional steps use a durable dirty marker and the selected maintenance owner.
    pub async fn up(&self) -> Result<usize, SqlError> {
        if self.catalog.steps.iter().any(|step| step.up.no_tx) {
            return self.up_with_maintenance().await;
        }
        let catalog = self.catalog.clone();
        self.kernel
            .transaction(move |connection| {
                Box::pin(async move {
                    ensure(connection, &catalog).await?;
                    reject_dirty(connection, &catalog).await?;
                    let applied = history(connection, &catalog).await?;
                    validate_history(&catalog, &applied)?;
                    for step in catalog.steps.iter().skip(applied.len()) {
                        apply_transactional(connection, &catalog, step, MigrationDirection::Up)
                            .await?;
                    }
                    Ok(catalog.steps.len() - applied.len())
                })
            })
            .await
            .map_err(annotate_terminal)
    }
    /// Apply only an opaque preflighted plan after exact locked history/catalog revalidation.
    pub async fn down(&self, plan: DownPlan) -> Result<usize, SqlError> {
        if !self.kernel.same_configuration(&plan.kernel)
            || !same_catalog(&self.catalog, &plan.catalog)
        {
            return Err(migration_failure(
                MigrationPhase::History,
                None,
                SqlError::Configuration("down plan belongs to another catalog".into()),
            ));
        }
        if plan
            .catalog
            .steps
            .iter()
            .take(plan.history.len())
            .rev()
            .take(plan.count)
            .any(|step| matches!(&step.rollback, Rollback::Reversible(down) if down.no_tx))
        {
            return self.down_with_maintenance(plan).await;
        }
        self.kernel
            .transaction(move |connection| {
                Box::pin(async move {
                    ensure(connection, &plan.catalog).await?;
                    reject_dirty(connection, &plan.catalog).await?;
                    let current = history(connection, &plan.catalog).await?;
                    validate_history(&plan.catalog, &current)?;
                    if current != plan.history {
                        return Err(stale_plan());
                    }
                    for step in plan
                        .catalog
                        .steps
                        .iter()
                        .take(current.len())
                        .rev()
                        .take(plan.count)
                    {
                        apply_transactional(
                            connection,
                            &plan.catalog,
                            step,
                            MigrationDirection::Down,
                        )
                        .await?;
                    }
                    Ok(plan.count)
                })
            })
            .await
            .map_err(annotate_terminal)
    }
    /// Inspect dirty schema state without guessing whether a migration program took effect.
    pub async fn inspect_dirty(&self) -> Result<Option<DirtyMigration>, SqlError> {
        let catalog = self.catalog.clone();
        self.kernel
            .transaction(move |connection| {
                Box::pin(async move {
                    ensure(connection, &catalog).await?;
                    dirty(connection, &catalog).await
                })
            })
            .await
            .map_err(annotate_terminal)
    }
    /// Assess actual schema under database write authority, then reconcile the same SQLx history.
    /// The callback has a lifetime-bound native connection, not another ambient SQL environment.
    pub async fn recover<F>(&self, assess: F) -> Result<(), SqlError>
    where
        F: for<'a> FnOnce(
                &'a DirtyMigration,
                &'a mut TursoConnection,
            ) -> crate::NativeFuture<'a, RecoveryDecision>
            + Send
            + 'static,
    {
        let catalog = self.catalog.clone();
        self.kernel
            .transaction(move |connection| {
                Box::pin(async move {
                    ensure(connection, &catalog).await?;
                    let dirty = dirty(connection, &catalog).await?.ok_or_else(|| {
                        migration_failure(
                            MigrationPhase::Recovery,
                            None,
                            SqlError::Configuration("no dirty migration".into()),
                        )
                    })?;
                    adopt_native_dirty(connection, &catalog, &dirty).await?;
                    let current = recovery_history(connection, &catalog).await?;
                    validate_history(&catalog, &current)?;
                    let index = catalog
                        .steps
                        .iter()
                        .position(|step| step.up.version == dirty.version)
                        .ok_or_else(stale_plan)?;
                    if current.len() != index && current.len() != index + 1 {
                        return Err(migration_failure(
                            MigrationPhase::Recovery,
                            Some(dirty.version),
                            SqlError::Configuration("dirty step is not history boundary".into()),
                        ));
                    }
                    let started = Instant::now();
                    let decision = assess(&dirty, connection).await?;
                    let journaled = matches!(
                        (&dirty.direction, decision),
                        (MigrationDirection::Up, RecoveryDecision::StepApplied)
                            | (MigrationDirection::Down, RecoveryDecision::StepNotApplied)
                    );
                    if journaled && current.len() == index {
                        insert_history(
                            connection,
                            &catalog,
                            catalog.steps.get(index).ok_or_else(stale_plan)?,
                            elapsed_ns(started)?,
                        )
                        .await?;
                    }
                    if !journaled {
                        delete_recovery_history(connection, &catalog, dirty.version).await?;
                    }
                    clear_dirty(connection, &catalog).await
                })
            })
            .await
            .map_err(annotate_terminal)
    }
    /// Lift initialization lazily, restoring complete native invocation failures.
    pub fn initialize_effect<R: Send + Sync + 'static>(&self) -> Effect<(), SqlError, R> {
        let this = self.clone();
        effect_future! {
            exit_with_context; body: move |_, context| {
            let this = this.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                native_exit(this.initialize().await)
            }
        }, }
    }
    /// Preflight pending versions lazily in the caller's environment without a nested runtime.
    pub fn plan_up_effect<R: Send + Sync + 'static>(&self) -> Effect<Vec<i64>, SqlError, R> {
        let this = self.clone();
        effect_future! {
            exit_with_context; body: move |_, context| {
            let this = this.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                native_exit(this.plan_up().await)
            }
        }, }
    }
    /// Capture a reversible plan lazily, preserving native failure and cleanup channels.
    pub fn plan_down_effect<R: Send + Sync + 'static>(
        &self,
        count: usize,
    ) -> Effect<DownPlan, SqlError, R> {
        let this = self.clone();
        effect_future! {
            exit_with_context; body: move |_, context| {
            let this = this.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                native_exit(this.plan_down(count).await)
            }
        }, }
    }
    /// Lift native up lazily into any selected Effect environment without a nested runtime.
    /// Once started, migration settlement is awaited; a dropped future is not acknowledgment.
    pub fn up_effect<R: Send + Sync + 'static>(&self) -> Effect<usize, SqlError, R> {
        let this = self.clone();
        effect_future! {
            exit_with_context; body: move |_, context| {
            let this = this.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                native_exit(this.up().await)
            }
        }, }
    }
    /// Apply one transferred opaque plan; subsequent executions report consumption.
    pub fn down_effect<R: Send + Sync + 'static>(
        &self,
        plan: DownPlan,
    ) -> Effect<usize, SqlError, R> {
        let this = self.clone();
        let plan = Arc::new(std::sync::Mutex::new(Some(plan)));
        effect_future! {
            exit_with_context; body: move |_, context| {
            let this = this.clone();
            let plan = plan.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let plan = plan
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                match plan {
                    Some(plan) => native_exit(this.down(plan).await),
                    None => Exit::Fail(SqlError::Consumed),
                }
            }
        }, }
    }
    /// Lift dirty inspection while preserving the caller's typed environment and full exit.
    pub fn inspect_dirty_effect<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<Option<DirtyMigration>, SqlError, R> {
        let this = self.clone();
        effect_future! {
            exit_with_context; body: move |_, context| {
            let this = this.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                native_exit(this.inspect_dirty().await)
            }
        }, }
    }
    /// Effectful recovery evaluates the selected assessment through the current interpreter context.
    pub fn recover_effect<R: Send + Sync + 'static>(
        &self,
        assess: impl Fn(DirtyMigration, crate::SqlTransaction) -> Effect<RecoveryDecision, SqlError, R>
        + Send
        + Sync
        + 'static,
    ) -> Effect<(), SqlError, R> {
        let this = self.clone();
        let assess = Arc::new(assess);
        effect_future! {
            exit_with_context; body: move |env, context| {
            let this = this.clone();
            let assess = assess.clone();
            async move {
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                let session = match this.kernel.session().await {
                    Ok(session) => session,
                    Err(error) => return error.into_exit(),
                };
                let tx = match session.begin().await {
                    Ok(tx) => tx,
                    Err(error) => {
                        return crate::client::append_cleanup(
                            error.into_exit(),
                            session.close().await,
                        );
                    }
                };
                let outcome = std::panic::AssertUnwindSafe(async {
                    let catalog = this.catalog.clone();
                    let dirty = match tx
                        .with_native(move |connection| {
                            Box::pin(async move {
                                ensure(connection, &catalog).await?;
                                let dirty =
                                    dirty(connection, &catalog).await?.ok_or_else(stale_plan)?;
                                adopt_native_dirty(connection, &catalog, &dirty).await?;
                                let current = recovery_history(connection, &catalog).await?;
                                validate_history(&catalog, &current)?;
                                let index = catalog
                                    .steps
                                    .iter()
                                    .position(|step| step.up.version == dirty.version)
                                    .ok_or_else(stale_plan)?;
                                if current.len() != index && current.len() != index + 1 {
                                    return Err(stale_plan());
                                }
                                Ok(dirty)
                            })
                        })
                        .await
                    {
                        Ok(dirty) => dirty,
                        Err(error) => return error.into_exit(),
                    };
                    let version = dirty.version;
                    let direction = dirty.direction;
                    let decision = match crate::row::failure_as::<_, (), _>(
                        context
                            .run(
                                assess(dirty, crate::SqlTransaction::from_native(tx.clone())),
                                env,
                            )
                            .await,
                    ) {
                        Ok(decision) => decision,
                        Err(exit) => return exit,
                    };
                    let catalog = this.catalog.clone();
                    let repair = tx
                        .with_native(move |connection| {
                            Box::pin(async move {
                                repair_recovery(connection, &catalog, version, direction, decision)
                                    .await
                            })
                        })
                        .await;
                    match repair {
                        Ok(()) => match tx.commit().await {
                            Ok(()) => Exit::Success(()),
                            Err(error) => error.into_exit(),
                        },
                        Err(error) => error.into_exit(),
                    }
                })
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                crate::client::append_cleanup(outcome, session.close().await)
            }
        }, }
    }
    /// Acquire external maintenance once and retain it across nontransactional schema execution.
    async fn up_with_maintenance(&self) -> Result<usize, SqlError> {
        let provider = self
            .maintenance
            .as_ref()
            .ok_or(SqlError::MaintenanceRequired)?;
        let guard = provider.acquire(self.kernel.clone()).await?;
        let result = std::panic::AssertUnwindSafe(async {
            let pending = self.plan_up().await?;
            let count = pending.len();
            for version in pending {
                let step = self
                    .catalog
                    .steps
                    .iter()
                    .find(|step| step.up.version == version)
                    .ok_or_else(stale_plan)?;
                self.apply_maintained(step, MigrationDirection::Up, None)
                    .await?;
            }
            Ok(count)
        })
        .catch_unwind()
        .await;
        join_native_cleanup(caught_native(result), release_maintenance(guard).await)
    }
    /// Keep the same explicit maintenance guard through exact down-plan revalidation and all steps.
    async fn down_with_maintenance(&self, plan: DownPlan) -> Result<usize, SqlError> {
        let provider = self
            .maintenance
            .as_ref()
            .ok_or(SqlError::MaintenanceRequired)?;
        let guard = provider.acquire(self.kernel.clone()).await?;
        let result = std::panic::AssertUnwindSafe(async {
            let mut expected = plan.history;
            for step in plan
                .catalog
                .steps
                .iter()
                .take(expected.len())
                .rev()
                .take(plan.count)
            {
                self.apply_maintained(step, MigrationDirection::Down, Some(expected.clone()))
                    .await?;
                let _closed_version = expected.pop();
            }
            Ok(plan.count)
        })
        .catch_unwind()
        .await;
        join_native_cleanup(caught_native(result), release_maintenance(guard).await)
    }
    /// Persist dirty state before nontransactional I/O, or keep transactional steps inside one lock.
    async fn apply_maintained(
        &self,
        step: &MigrationStep,
        direction: MigrationDirection,
        expected: Option<Vec<HistoryEntry>>,
    ) -> Result<(), SqlError> {
        let native = match direction {
            MigrationDirection::Up => &step.up,
            MigrationDirection::Down => match &step.rollback {
                Rollback::Reversible(down) => down,
                Rollback::Irreversible => return Err(stale_plan()),
            },
        };
        let no_tx = native.no_tx;
        let sql = native.sql.clone();
        let catalog = self.catalog.clone();
        let step = step.clone();
        if !no_tx {
            return self
                .kernel
                .transaction(move |connection| {
                    Box::pin(async move {
                        ensure(connection, &catalog).await?;
                        reject_dirty(connection, &catalog).await?;
                        let current = history(connection, &catalog).await?;
                        validate_history(&catalog, &current)?;
                        if expected.is_some_and(|expected| expected != current) {
                            return Err(stale_plan());
                        }
                        validate_boundary(&catalog, &current, step.up.version, direction)?;
                        apply_transactional(connection, &catalog, &step, direction).await
                    })
                })
                .await
                .map_err(annotate_terminal);
        }
        let prepare_catalog = catalog.clone();
        let prepare_step = step.clone();
        self.kernel
            .transaction(move |connection| {
                Box::pin(async move {
                    ensure(connection, &prepare_catalog).await?;
                    reject_dirty(connection, &prepare_catalog).await?;
                    let current = history(connection, &prepare_catalog).await?;
                    validate_history(&prepare_catalog, &current)?;
                    if expected.is_some_and(|expected| expected != current) {
                        return Err(stale_plan());
                    }
                    validate_boundary(
                        &prepare_catalog,
                        &current,
                        prepare_step.up.version,
                        direction,
                    )?;
                    mark_dirty(connection, &prepare_catalog, &prepare_step, direction).await
                })
            })
            .await
            .map_err(annotate_terminal)?;
        let started = Instant::now();
        let version = step.up.version;
        self.kernel
            .with_connection(move |connection| {
                Box::pin(async move {
                    connection
                        .native_execute_batch(sql.as_str())
                        .await
                        .map_err(|error| {
                            migration_failure(MigrationPhase::Apply, Some(version), error.into())
                        })
                })
            })
            .await?;
        self.kernel
            .transaction(move |connection| {
                Box::pin(async move {
                    let dirty = dirty(connection, &catalog).await?.ok_or_else(stale_plan)?;
                    if dirty.version != step.up.version || dirty.direction != direction {
                        return Err(stale_plan());
                    }
                    publish_step(connection, &catalog, &step, direction, elapsed_ns(started)?)
                        .await?;
                    clear_dirty(connection, &catalog).await
                })
            })
            .await
            .map_err(annotate_terminal)
    }
}

/// Bootstrap SQLx-compatible history and only the versioned dirty extension metadata.
async fn ensure(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
) -> Result<(), SqlError> {
    connection.native_execute_batch(format!("CREATE TABLE IF NOT EXISTS {} (version BIGINT PRIMARY KEY, description TEXT NOT NULL, installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, success BOOLEAN NOT NULL, checksum BLOB NOT NULL, execution_time BIGINT NOT NULL); CREATE TABLE IF NOT EXISTS {DIRTY_METADATA} (journal TEXT PRIMARY KEY, version BIGINT NOT NULL, direction TEXT NOT NULL CHECK(direction IN ('up','down')), checksum BLOB NOT NULL);", catalog.quoted)).await.map_err(|error| migration_failure(MigrationPhase::History, None, error.into()))?;
    Ok(())
}
/// Read acknowledged SQLx history, rejecting native SQLx dirty rows rather than treating them applied.
async fn history(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
) -> Result<Vec<HistoryEntry>, SqlError> {
    let rows = sqlx::query(AssertSqlSafe(format!(
        "SELECT version,checksum,success FROM {} ORDER BY version",
        catalog.quoted
    )))
    .fetch_all(connection)
    .await
    .map_err(|error| migration_failure(MigrationPhase::History, None, error.into()))?;
    rows.into_iter()
        .map(|row| {
            let version: i64 = row.try_get("version")?;
            if !row.try_get::<bool, _>("success")? {
                return Err(migration_failure(
                    MigrationPhase::Recovery,
                    Some(version),
                    SqlError::Configuration("SQLx dirty migration requires assessment".into()),
                ));
            }
            Ok(HistoryEntry {
                version,
                checksum: row.try_get("checksum")?,
            })
        })
        .collect()
}
/// Enforce exact catalog-prefix history rather than ignoring missing or modified migrations.
fn validate_history(catalog: &MigrationCatalog, current: &[HistoryEntry]) -> Result<(), SqlError> {
    if current.len() > catalog.steps.len() {
        return Err(stale_plan());
    }
    for (step, current) in catalog.steps.iter().zip(current) {
        if step.up.version != current.version || step.up.checksum.as_ref() != current.checksum {
            return Err(migration_failure(
                MigrationPhase::History,
                Some(current.version),
                SqlError::Configuration("migration history/checksum mismatch".into()),
            ));
        }
    }
    Ok(())
}
/// Require that a maintained step is still the exact next up or last applied down boundary.
fn validate_boundary(
    catalog: &MigrationCatalog,
    current: &[HistoryEntry],
    version: i64,
    direction: MigrationDirection,
) -> Result<(), SqlError> {
    let actual = match direction {
        MigrationDirection::Up => catalog.steps.get(current.len()).map(|step| step.up.version),
        MigrationDirection::Down => current.last().map(|step| step.version),
    };
    if actual != Some(version) {
        return Err(stale_plan());
    }
    Ok(())
}
/// Resolve a versioned dirty marker against the current immutable catalog/checksum.
async fn dirty(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
) -> Result<Option<DirtyMigration>, SqlError> {
    let marker = sqlx::query(AssertSqlSafe(format!(
        "SELECT version,direction,checksum FROM {DIRTY_METADATA} WHERE journal=?"
    )))
    .bind(&catalog.journal)
    .fetch_optional(&mut *connection)
    .await?;
    let (version, checksum, direction, native_dirty) = if let Some(row) = marker {
        let direction = match row.try_get::<String, _>("direction")?.as_str() {
            "up" => MigrationDirection::Up,
            "down" => MigrationDirection::Down,
            _ => return Err(stale_plan()),
        };
        (
            row.try_get::<i64, _>("version")?,
            row.try_get::<Vec<u8>, _>("checksum")?,
            direction,
            false,
        )
    } else {
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT version,checksum FROM {} WHERE success=FALSE ORDER BY version",
            catalog.quoted
        )))
        .fetch_all(connection)
        .await
        .map_err(|error| migration_failure(MigrationPhase::History, None, error.into()))?;
        if rows.len() > 1 {
            return Err(migration_failure(
                MigrationPhase::Recovery,
                None,
                SqlError::Configuration("multiple native dirty migrations".into()),
            ));
        }
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        (
            row.try_get::<i64, _>("version")?,
            row.try_get::<Vec<u8>, _>("checksum")?,
            MigrationDirection::Up,
            true,
        )
    };
    let step = catalog
        .steps
        .iter()
        .find(|step| step.up.version == version)
        .ok_or_else(stale_plan)?;
    if checksum != step.up.checksum.as_ref() {
        return Err(stale_plan());
    }
    Ok(Some(DirtyMigration {
        version,
        name: step.up.description.to_string(),
        checksum,
        direction,
        native_dirty,
    }))
}
/// Retain clean native history during deliberate recovery; false rows are assessed separately.
async fn recovery_history(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
) -> Result<Vec<HistoryEntry>, SqlError> {
    let rows = sqlx::query(AssertSqlSafe(format!(
        "SELECT version,checksum FROM {} WHERE success=TRUE ORDER BY version",
        catalog.quoted
    )))
    .fetch_all(connection)
    .await
    .map_err(|error| migration_failure(MigrationPhase::History, None, error.into()))?;
    rows.into_iter()
        .map(|row| {
            Ok(HistoryEntry {
                version: row.try_get("version")?,
                checksum: row.try_get("checksum")?,
            })
        })
        .collect()
}
/// Extend native dirty metadata with a direction marker inside the same write transaction.
async fn adopt_native_dirty(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
    dirty: &DirtyMigration,
) -> Result<(), SqlError> {
    if dirty.native_dirty {
        let step = catalog
            .steps
            .iter()
            .find(|step| step.up.version == dirty.version)
            .ok_or_else(stale_plan)?;
        mark_dirty(connection, catalog, step, dirty.direction).await?;
    }
    Ok(())
}
/// Reject unresolved schema state before any new history plan or mutation.
async fn reject_dirty(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
) -> Result<(), SqlError> {
    if let Some(dirty) = dirty(connection, catalog).await? {
        return Err(migration_failure(
            MigrationPhase::Recovery,
            Some(dirty.version),
            SqlError::Configuration("dirty schema assessment required".into()),
        ));
    }
    Ok(())
}
/// Mark a schema boundary in compatible extension metadata; nontransactional callers commit this first.
async fn mark_dirty(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
    step: &MigrationStep,
    direction: MigrationDirection,
) -> Result<(), SqlError> {
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO {DIRTY_METADATA}(journal,version,direction,checksum) VALUES (?,?,?,?)"
    )))
    .bind(&catalog.journal)
    .bind(step.up.version)
    .bind(match direction {
        MigrationDirection::Up => "up",
        MigrationDirection::Down => "down",
    })
    .bind(step.up.checksum.as_ref())
    .execute(connection)
    .await?;
    Ok(())
}
/// Clear only one acknowledged marker; absence is a recovery failure rather than guessed success.
async fn clear_dirty(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
) -> Result<(), SqlError> {
    let result = sqlx::query(AssertSqlSafe(format!(
        "DELETE FROM {DIRTY_METADATA} WHERE journal=?"
    )))
    .bind(&catalog.journal)
    .execute(connection)
    .await?;
    if result.rows_affected() != 1 {
        return Err(stale_plan());
    }
    Ok(())
}
/// Apply a declared step, journal it and clear its marker under the same database write owner.
async fn apply_transactional(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
    step: &MigrationStep,
    direction: MigrationDirection,
) -> Result<(), SqlError> {
    let started = Instant::now();
    mark_dirty(connection, catalog, step, direction).await?;
    let sql = match direction {
        MigrationDirection::Up => &step.up.sql,
        MigrationDirection::Down => match &step.rollback {
            Rollback::Reversible(down) => &down.sql,
            Rollback::Irreversible => return Err(stale_plan()),
        },
    };
    connection
        .native_execute_batch(sql.as_str())
        .await
        .map_err(|error| {
            migration_failure(MigrationPhase::Apply, Some(step.up.version), error.into())
        })?;
    publish_step(connection, catalog, step, direction, elapsed_ns(started)?).await?;
    clear_dirty(connection, catalog).await
}
/// Reconcile the existing SQLx journal according to the declared migration direction.
async fn publish_step(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
    step: &MigrationStep,
    direction: MigrationDirection,
    execution_ns: i64,
) -> Result<(), SqlError> {
    match direction {
        MigrationDirection::Up => insert_history(connection, catalog, step, execution_ns).await,
        MigrationDirection::Down => delete_history(connection, catalog, step.up.version).await,
    }
}
/// Insert SQLx-native checksum/history fields; execution_time uses native nanosecond units; recovery entries record assessment/publication duration, never an invented original step duration.
async fn insert_history(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
    step: &MigrationStep,
    execution_ns: i64,
) -> Result<(), SqlError> {
    let result = sqlx::query(AssertSqlSafe(format!("INSERT INTO {}(version,description,success,checksum,execution_time) VALUES (?,?,TRUE,?,?) ON CONFLICT(version) DO UPDATE SET success=TRUE,execution_time=excluded.execution_time WHERE checksum=excluded.checksum AND success=FALSE", catalog.quoted))).bind(step.up.version).bind(step.up.description.as_ref()).bind(step.up.checksum.as_ref()).bind(execution_ns).execute(connection).await.map_err(|error| migration_failure(MigrationPhase::Journal, Some(step.up.version), error.into()))?;
    if result.rows_affected() != 1 {
        return Err(stale_plan());
    }
    Ok(())
}
/// Delete exactly the selected down boundary from the same SQLx journal.
async fn delete_history(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
    version: i64,
) -> Result<(), SqlError> {
    let result = sqlx::query(AssertSqlSafe(format!(
        "DELETE FROM {} WHERE version=?",
        catalog.quoted
    )))
    .bind(version)
    .execute(connection)
    .await
    .map_err(|error| migration_failure(MigrationPhase::Journal, Some(version), error.into()))?;
    if result.rows_affected() != 1 {
        return Err(stale_plan());
    }
    Ok(())
}
/// Repair only the assessed dirty boundary while the caller retains the database write transaction.
async fn repair_recovery(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
    version: i64,
    direction: MigrationDirection,
    decision: RecoveryDecision,
) -> Result<(), SqlError> {
    let started = Instant::now();
    let unresolved = dirty(connection, catalog).await?.ok_or_else(stale_plan)?;
    if unresolved.version != version || unresolved.direction != direction {
        return Err(stale_plan());
    }
    let current = recovery_history(connection, catalog).await?;
    validate_history(catalog, &current)?;
    let index = catalog
        .steps
        .iter()
        .position(|step| step.up.version == version)
        .ok_or_else(stale_plan)?;
    if current.len() != index && current.len() != index + 1 {
        return Err(stale_plan());
    }
    let journaled = matches!(
        (direction, decision),
        (MigrationDirection::Up, RecoveryDecision::StepApplied)
            | (MigrationDirection::Down, RecoveryDecision::StepNotApplied)
    );
    if journaled && current.len() == index {
        insert_history(
            connection,
            catalog,
            catalog.steps.get(index).ok_or_else(stale_plan)?,
            elapsed_ns(started)?,
        )
        .await?;
    }
    if !journaled {
        delete_recovery_history(connection, catalog, version).await?;
    }
    clear_dirty(connection, catalog).await
}
/// Convert provider-owned elapsed time to SQLx's signed nanosecond journal units without truncation.
fn elapsed_ns(started: Instant) -> Result<i64, SqlError> {
    i64::try_from(started.elapsed().as_nanos()).map_err(|_| {
        SqlError::Configuration(
            "migration execution duration exceeds SQLx nanosecond representation".into(),
        )
    })
}
/// Delete assessed nonapplied history, accepting absence deliberately only in the recovery path.
async fn delete_recovery_history(
    connection: &mut TursoConnection,
    catalog: &MigrationCatalog,
    version: i64,
) -> Result<(), SqlError> {
    sqlx::query(AssertSqlSafe(format!(
        "DELETE FROM {} WHERE version=?",
        catalog.quoted
    )))
    .bind(version)
    .execute(connection)
    .await?;
    Ok(())
}
/// Compare exact immutable schema contracts before allowing a down-plan to cross configurations.
fn same_catalog(left: &MigrationCatalog, right: &MigrationCatalog) -> bool {
    left.journal == right.journal
        && left.steps.len() == right.steps.len()
        && left
            .steps
            .iter()
            .zip(right.steps.iter())
            .all(|(left, right)| {
                same_native_step(&left.up, &right.up)
                    && match (&left.rollback, &right.rollback) {
                        (Rollback::Irreversible, Rollback::Irreversible) => true,
                        (Rollback::Reversible(left), Rollback::Reversible(right)) => {
                            same_native_step(left, right)
                        }
                        (Rollback::Irreversible, Rollback::Reversible(_))
                        | (Rollback::Reversible(_), Rollback::Irreversible) => false,
                    }
            })
}
/// Compare the whole immutable native contract; SQLx's own equality compares only version/type.
fn same_native_step(left: &sqlx::migrate::Migration, right: &sqlx::migrate::Migration) -> bool {
    left.version == right.version
        && left.description == right.description
        && left.migration_type == right.migration_type
        && left.sql.as_str() == right.sql.as_str()
        && left.checksum == right.checksum
        && left.no_tx == right.no_tx
}
/// History mismatch is a typed migration phase, not an automatic retry instruction.
fn stale_plan() -> SqlError {
    migration_failure(
        MigrationPhase::History,
        None,
        SqlError::Configuration("migration history changed".into()),
    )
}
/// Add a migration boundary without discarding its underlying typed provider/uncertainty category.
fn migration_failure(phase: MigrationPhase, version: Option<i64>, source: SqlError) -> SqlError {
    let uncertain = source.outcome_uncertain();
    SqlError::Migration {
        phase,
        version,
        uncertain,
        source: Box::new(source),
    }
}
/// Commit uncertainty and cleanup remain separate from ordinary schema application errors.
fn annotate_terminal(error: SqlError) -> SqlError {
    match error {
        SqlError::Migration { .. } => error,
        SqlError::CommitOutcomeUnknown(_) => migration_failure(MigrationPhase::Commit, None, error),
        SqlError::Cleanup { .. } => migration_failure(MigrationPhase::Cleanup, None, error),
        error @ (SqlError::Configuration(_)
        | SqlError::Identifier(_)
        | SqlError::Binding(_)
        | SqlError::Provider(_)
        | SqlError::RowAbsent
        | SqlError::Decode(_)
        | SqlError::InactiveSession
        | SqlError::MaintenanceRequired
        | SqlError::ReplicaNotAuthoritative
        | SqlError::NativeInvocation(_)
        | SqlError::Consumed) => {
            let phase = if error.outcome_uncertain() {
                MigrationPhase::Commit
            } else {
                MigrationPhase::Lock
            };
            migration_failure(phase, None, error)
        }
    }
}
/// Preserve primary and maintenance release failures in observation order.
fn join_native_cleanup<A>(
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

/// Restore every native failure channel at an outer Effect boundary.
fn native_exit<A>(result: Result<A, SqlError>) -> Exit<A, SqlError> {
    match result {
        Ok(value) => Exit::Success(value),
        Err(error) => error.into_exit(),
    }
}
/// Retain invocation-local defects while still reaching maintenance release.
fn caught_native<A>(
    result: Result<Result<A, SqlError>, Box<dyn std::any::Any + Send>>,
) -> Result<A, SqlError> {
    match result {
        Ok(result) => result,
        Err(panic) => Err(SqlError::NativeInvocation(Box::new(Exit::Die(
            Defect::from_panic(panic),
        )))),
    }
}
/// Catch both release-factory and release-future panics as ordered cleanup defects.
async fn release_maintenance(guard: Box<dyn MaintenanceGuard>) -> Result<(), SqlError> {
    caught_native(
        std::panic::AssertUnwindSafe(async move { guard.release().await })
            .catch_unwind()
            .await,
    )
}
