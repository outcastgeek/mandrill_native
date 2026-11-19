//! Independent in-memory execution and replay.

use super::{
    CancellationOutcome, ChildWait, CompensationState, DeadlineClear, DeadlineWait, DeadlineWake,
    EncodedWorkflowOutcome, RecordedActivity, RecordedChild, RecordedCompensation,
    RecordedDeadline, RecordedRace, RecordedSignal, SignalWait, WorkflowActivityId,
    WorkflowCompensationId, WorkflowDefinitionId, WorkflowEngine, WorkflowEngineError,
    WorkflowHandle, WorkflowHead, WorkflowRaceId, WorkflowRegistration, WorkflowRunId,
    WorkflowSignalId, WorkflowSnapshot, WorkflowStatus, WorkflowVersion, WorkflowWait,
    WorkflowWakeReason,
};
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use crate::{Defect, Effect, Exit, UtcMillis, effect_sync};
use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, MutexGuard};

mod prepared;
use prepared::{History, PreparedWorkflowHistories, PreparedWorkflowStorage, Table};
pub use prepared::{
    PreparedWorkflowCapacity, PreparedWorkflowHistoryCursor, PreparedWorkflowHistoryError,
    PreparedWorkflowHistoryPage, WorkflowHistoryRecord,
};

type DefinitionKey = (WorkflowDefinitionId, WorkflowVersion);
type Definitions = Table<DefinitionKey, Arc<[u8]>>;
type Runs = Table<WorkflowRunId, Run>;

struct Run {
    execution: u64,
    handle: WorkflowHandle,
    compatibility: Arc<[u8]>,
    encoded_input: Arc<[u8]>,
    status: WorkflowStatus,
    activities: History<RecordedActivity>,
    children: History<RecordedChild>,
    signals: History<RecordedSignal>,
    compensations: History<RecordedCompensation>,
    races: History<RecordedRace>,
    deadline: Option<RecordedDeadline>,
    waiting_on: Option<WorkflowWait>,
    wake_reason: Option<WorkflowWakeReason>,
    history_revision: u64,
    retained_bytes: Bytes,
}

impl Run {
    fn from_snapshot(snapshot: WorkflowSnapshot) -> Self {
        Self {
            execution: snapshot.execution,
            handle: snapshot.handle,
            compatibility: snapshot.compatibility,
            encoded_input: snapshot.encoded_input,
            status: snapshot.status,
            activities: History::Ordinary(snapshot.activities),
            children: History::Ordinary(snapshot.children),
            signals: History::Ordinary(snapshot.signals),
            compensations: History::Ordinary(snapshot.compensations),
            races: History::Ordinary(snapshot.races),
            deadline: snapshot.deadline,
            waiting_on: snapshot.waiting_on,
            wake_reason: snapshot.wake_reason,
            history_revision: 0,
            retained_bytes: Bytes::new(0),
        }
    }

    fn head(&self) -> WorkflowHead {
        WorkflowHead {
            execution: self.execution,
            handle: self.handle.clone(),
            compatibility: Arc::clone(&self.compatibility),
            encoded_input: Arc::clone(&self.encoded_input),
            status: self.status.clone(),
            deadline: self.deadline.clone(),
            waiting_on: self.waiting_on.clone(),
            wake_reason: self.wake_reason.clone(),
        }
    }

    fn checkpoint(&self) -> super::WorkflowCheckpoint {
        super::WorkflowCheckpoint {
            status: self.status.clone(),
            execution: self.execution,
        }
    }

    fn snapshot(&self) -> WorkflowSnapshot {
        let head = self.head();
        WorkflowSnapshot {
            execution: head.execution,
            handle: head.handle,
            compatibility: head.compatibility,
            encoded_input: head.encoded_input,
            status: head.status,
            activities: self.activities.to_vec(),
            children: self.children.to_vec(),
            signals: self.signals.to_vec(),
            compensations: self.compensations.to_vec(),
            races: self.races.to_vec(),
            deadline: head.deadline,
            waiting_on: head.waiting_on,
            wake_reason: head.wake_reason,
        }
    }

    fn prepare_history(
        &self,
        capacity: Option<PreparedWorkflowCapacity>,
        identities: &[usize],
        payloads: &[usize],
    ) -> Result<(Bytes, u64), MemoryError> {
        let Some(capacity) = capacity else {
            return Ok((self.retained_bytes, self.history_revision));
        };
        for &length in identities {
            check_bytes(Bytes::new(length), capacity.bytes_per_identity)?;
        }
        let bytes = payload_bytes(capacity, self.retained_bytes, payloads)?;
        let revision = self
            .history_revision
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok((bytes, revision))
    }

    fn publish_history(&mut self, (bytes, revision): (Bytes, u64)) {
        self.retained_bytes = bytes;
        self.history_revision = revision;
    }
}

fn check_bytes(requested: Bytes, capacity: Bytes) -> Result<(), MemoryError> {
    if requested > capacity {
        return Err(MemoryError::ValueTooLarge {
            requested,
            capacity,
        });
    }
    Ok(())
}

fn payload_bytes(
    capacity: PreparedWorkflowCapacity,
    retained: Bytes,
    payloads: &[usize],
) -> Result<Bytes, MemoryError> {
    let mut total = retained;
    for &length in payloads {
        let bytes = Bytes::new(length);
        check_bytes(bytes, capacity.bytes_per_payload)?;
        total = total.checked_add(bytes)?;
    }
    check_bytes(total, capacity.bytes_per_run)?;
    Ok(total)
}

/// Typed state used by workflow memory workflow engine.
#[derive(Default)]
struct State {
    definitions: Definitions,
    runs: Runs,
    shutdown: bool,
    prepared: Option<PreparedWorkflowStorage>,
}

impl State {
    fn capacity(&self) -> Option<PreparedWorkflowCapacity> {
        self.prepared.as_ref().map(|storage| storage.capacity)
    }
    ///
    /// # Returns
    /// Type: `Result<(), WorkflowEngineError>`.
    fn ensure_live(&self) -> Result<(), WorkflowEngineError> {
        if self.shutdown {
            Err(WorkflowEngineError::EngineShutdown)
        } else {
            Ok(())
        }
    }
}

///
/// # Parameters
/// - `state` (`&Arc<Mutex<State>>`): Current or replacement state owned by this operation.
///
/// # Returns
/// Type: `Result<MutexGuard<'_, State>, WorkflowEngineError>`.
fn lock_live(state: &Arc<Mutex<State>>) -> Result<MutexGuard<'_, State>, WorkflowEngineError> {
    let guard = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.ensure_live()?;
    Ok(guard)
}

///
/// # Parameters
/// - `snapshot` (`&WorkflowSnapshot`): The snapshot supplied to this operation.
///
/// # Returns
/// Type: `Result<(), WorkflowEngineError>`.
fn validate_records(snapshot: &WorkflowSnapshot) -> Result<(), WorkflowEngineError> {
    if matches!(
        snapshot.status,
        WorkflowStatus::Completed(_) | WorkflowStatus::Cancelled
    ) && (snapshot.deadline.is_some() || snapshot.waiting_on.is_some())
    {
        return Err(WorkflowEngineError::StateConflict);
    }
    if matches!(&snapshot.waiting_on,
        Some(WorkflowWait::Signal { version, .. }) if *version != snapshot.handle.version)
    {
        return Err(WorkflowEngineError::DefinitionMismatch);
    }
    if snapshot.waiting_on.is_some() && snapshot.status != WorkflowStatus::Suspended {
        return Err(WorkflowEngineError::StateConflict);
    }
    if matches!(&snapshot.waiting_on,
        Some(WorkflowWait::Deadline(deadline)) if snapshot.deadline.as_ref() != Some(deadline))
    {
        return Err(WorkflowEngineError::StateConflict);
    }
    if snapshot.status == WorkflowStatus::Suspended && snapshot.wake_reason.is_some() {
        return Err(WorkflowEngineError::StateConflict);
    }
    if let Some(WorkflowWait::Child(child)) = &snapshot.waiting_on
        && !snapshot
            .children
            .iter()
            .any(|record| record.handle.run == *child)
    {
        return Err(WorkflowEngineError::ChildConflict);
    }
    let mut activity_attempts = BTreeMap::new();
    for record in &snapshot.activities {
        if let Some(previous) = activity_attempts.insert(record.identity.clone(), record.attempt)
            && previous >= record.attempt
        {
            return Err(WorkflowEngineError::ActivityAttemptConflict);
        }
    }
    let mut signals = BTreeSet::new();
    for record in &snapshot.signals {
        if record.version != snapshot.handle.version {
            return Err(WorkflowEngineError::DefinitionMismatch);
        }
        if !signals.insert(record.identity.clone()) {
            return Err(WorkflowEngineError::SignalConflict);
        }
    }
    let mut compensation_ids = BTreeSet::new();
    let mut compensation_orders = BTreeSet::new();
    for record in &snapshot.compensations {
        if !compensation_ids.insert(record.identity.clone())
            || !compensation_orders.insert(record.order)
        {
            return Err(WorkflowEngineError::CompensationConflict);
        }
    }
    let mut races = BTreeSet::new();
    for record in &snapshot.races {
        if !races.insert(record.identity.clone()) {
            return Err(WorkflowEngineError::RaceConflict);
        }
        if !snapshot.activities.iter().any(|activity| {
            activity.identity == record.winner && activity.attempt == record.attempt
        }) {
            return Err(WorkflowEngineError::RaceWinnerMissing);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The behavior chosen when a workflow engine stops admitting work.
pub enum WorkflowShutdownPolicy {
    /// Retain the failed record for a later recovery attempt.
    PreserveForRecovery,
    /// Request interruption even when work is currently active.
    ForceInterrupt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The running and settled work accounted for during engine shutdown.
pub struct WorkflowShutdownReport {
    /// Whether shutdown had already begun before this request. Type: `bool`.
    pub already_shutdown: bool,
    /// Count of runs abandoned during engine shutdown. Type: `usize`.
    pub abandoned: usize,
    /// Count of runs interrupted during engine shutdown. Type: `usize`.
    pub interrupted: usize,
}

/// An explicit compatibility mapping for changing a recorded workflow definition.
pub struct WorkflowMigration {
    /// Previously registered workflow definition. Type: `WorkflowRegistration`.
    pub from: WorkflowRegistration,
    /// Replacement workflow definition used by the migration. Type: `WorkflowRegistration`.
    pub to: WorkflowRegistration,
    transform: Arc<
        dyn Fn(WorkflowSnapshot) -> Result<WorkflowSnapshot, WorkflowEngineError> + Send + Sync,
    >,
}

impl WorkflowMigration {
    #[must_use]
    /// Construct a `WorkflowMigration` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `from` (`WorkflowRegistration`): The from supplied to this operation.
    /// - `to` (`WorkflowRegistration`): The to supplied to this operation.
    /// - `transform` (`impl Fn(WorkflowSnapshot) -> Result<WorkflowSnapshot, WorkflowEngineError> + Send + Sync + 'static`): Callback applied to the selected value or error.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        from: WorkflowRegistration,
        to: WorkflowRegistration,
        transform: impl Fn(WorkflowSnapshot) -> Result<WorkflowSnapshot, WorkflowEngineError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            from,
            to,
            transform: Arc::new(transform),
        }
    }
}

/// The rule used to validate and resume recorded workflow histories.
pub enum WorkflowReplayPolicy {
    /// Reject input that violates the declared representation.
    Strict,
    /// Apply an explicitly registered workflow compatibility migration. Contains `Vec<WorkflowMigration>`.
    Migrate(Vec<WorkflowMigration>),
}

/// Reconstructible in-process implementation of the WorkflowEngine contract.
/// Its records can be copied to a different provider, but this type itself
/// makes no claim of surviving process loss.
pub struct MemoryWorkflowEngine<P = OrdinaryWorkflowStorage> {
    state: Arc<Mutex<State>>,
    policy: PhantomData<P>,
}

/// The selected backend retains the ordinary lifecycle and prepared memory errors.
pub type PreparedMemoryWorkflowEngine = MemoryWorkflowEngine<PreparedWorkflowPolicy>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparedWorkflowError {
    Engine(WorkflowEngineError),
    Memory(MemoryError),
}

impl From<WorkflowEngineError> for PreparedWorkflowError {
    fn from(error: WorkflowEngineError) -> Self {
        Self::Engine(error)
    }
}

impl From<MemoryError> for PreparedWorkflowError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

#[doc(hidden)]
pub struct OrdinaryWorkflowStorage;

#[doc(hidden)]
pub struct PreparedWorkflowPolicy;

#[doc(hidden)]
pub trait WorkflowStoragePolicy: Send + Sync + 'static {
    type Error: Send + 'static;

    fn publish<A>(outcome: Result<A, PreparedWorkflowError>) -> Exit<A, Self::Error>;
}

impl WorkflowStoragePolicy for OrdinaryWorkflowStorage {
    type Error = WorkflowEngineError;

    fn publish<A>(outcome: Result<A, PreparedWorkflowError>) -> Exit<A, Self::Error> {
        match outcome {
            Ok(value) => Exit::Success(value),
            Err(PreparedWorkflowError::Engine(error)) => Exit::Fail(error),
            // Only the prepared state can produce this failure.
            Err(PreparedWorkflowError::Memory(_)) => Exit::Die(Defect::new(
                "ordinary workflow state returned a prepared storage refusal",
            )),
        }
    }
}

impl WorkflowStoragePolicy for PreparedWorkflowPolicy {
    type Error = PreparedWorkflowError;

    fn publish<A>(outcome: Result<A, PreparedWorkflowError>) -> Exit<A, Self::Error> {
        match outcome {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(error),
        }
    }
}

impl Default for MemoryWorkflowEngine {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            policy: PhantomData,
        }
    }
}

impl MemoryWorkflowEngine {
    #[must_use]
    /// Construct a `MemoryWorkflowEngine` from its required inputs and explicit policies.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepare the shared engine's physical run, history, and replay storage.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedWorkflowCapacity,
    ) -> Result<PreparedMemoryWorkflowEngine, MemoryError> {
        capacity.storage_bytes()?;
        Ok(MemoryWorkflowEngine {
            state: Arc::new(Mutex::new(State {
                definitions: Table::prepare(memory, region, capacity.definitions)?,
                runs: Table::prepare(memory, region, capacity.runs)?,
                shutdown: false,
                prepared: Some(PreparedWorkflowStorage::prepare(memory, region, capacity)?),
            })),
            policy: PhantomData,
        })
    }

    /// Reconstruct state from the supplied recorded snapshot.
    ///
    /// # Parameters
    /// - `definitions` (`impl IntoIterator<Item = WorkflowRegistration>`): The definitions supplied to this operation.
    /// - `snapshots` (`impl IntoIterator<Item = WorkflowSnapshot>`): Recorded states used to reconstruct workflow or projection state.
    ///
    /// # Returns
    /// Type: `Result<Self, WorkflowEngineError>`.
    pub fn restore(
        definitions: impl IntoIterator<Item = WorkflowRegistration>,
        snapshots: impl IntoIterator<Item = WorkflowSnapshot>,
    ) -> Result<Self, WorkflowEngineError> {
        Self::restore_with_policy(definitions, snapshots, WorkflowReplayPolicy::Strict)
    }

    /// Recover retained records against an explicit wall-clock observation.
    /// Deadlines due at this instant become ready during reconstruction.
    ///
    /// # Parameters
    /// - `definitions` (`impl IntoIterator<Item = WorkflowRegistration>`): The definitions supplied to this operation.
    /// - `snapshots` (`impl IntoIterator<Item = WorkflowSnapshot>`): Recorded states used to reconstruct workflow or projection state.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Result<Self, WorkflowEngineError>`.
    pub fn restore_at(
        definitions: impl IntoIterator<Item = WorkflowRegistration>,
        snapshots: impl IntoIterator<Item = WorkflowSnapshot>,
        now: UtcMillis,
    ) -> Result<Self, WorkflowEngineError> {
        Self::restore_with_policy_at(definitions, snapshots, WorkflowReplayPolicy::Strict, now)
    }

    /// Rebuild a memory engine from snapshots under an explicit replay policy.
    ///
    /// # Parameters
    /// - `definitions` (`impl IntoIterator<Item = WorkflowRegistration>`): The definitions supplied to this operation.
    /// - `snapshots` (`impl IntoIterator<Item = WorkflowSnapshot>`): Recorded states used to reconstruct workflow or projection state.
    /// - `policy` (`WorkflowReplayPolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Result<Self, WorkflowEngineError>`.
    pub fn restore_with_policy(
        definitions: impl IntoIterator<Item = WorkflowRegistration>,
        snapshots: impl IntoIterator<Item = WorkflowSnapshot>,
        policy: WorkflowReplayPolicy,
    ) -> Result<Self, WorkflowEngineError> {
        Self::restore_internal(definitions, snapshots, policy, None)
    }

    /// Rebuild snapshots as of the supplied instant under the replay policy.
    ///
    /// # Parameters
    /// - `definitions` (`impl IntoIterator<Item = WorkflowRegistration>`): The definitions supplied to this operation.
    /// - `snapshots` (`impl IntoIterator<Item = WorkflowSnapshot>`): Recorded states used to reconstruct workflow or projection state.
    /// - `policy` (`WorkflowReplayPolicy`): The caller-supplied behavior policy.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Result<Self, WorkflowEngineError>`.
    pub fn restore_with_policy_at(
        definitions: impl IntoIterator<Item = WorkflowRegistration>,
        snapshots: impl IntoIterator<Item = WorkflowSnapshot>,
        policy: WorkflowReplayPolicy,
        now: UtcMillis,
    ) -> Result<Self, WorkflowEngineError> {
        Self::restore_internal(definitions, snapshots, policy, Some(now))
    }

    ///
    /// # Parameters
    /// - `definitions` (`impl IntoIterator<Item = WorkflowRegistration>`): The definitions supplied to this operation.
    /// - `snapshots` (`impl IntoIterator<Item = WorkflowSnapshot>`): Recorded states used to reconstruct workflow or projection state.
    /// - `policy` (`WorkflowReplayPolicy`): The caller-supplied behavior policy.
    /// - `now` (`Option<UtcMillis>`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Result<Self, WorkflowEngineError>`.
    fn restore_internal(
        definitions: impl IntoIterator<Item = WorkflowRegistration>,
        snapshots: impl IntoIterator<Item = WorkflowSnapshot>,
        policy: WorkflowReplayPolicy,
        now: Option<UtcMillis>,
    ) -> Result<Self, WorkflowEngineError> {
        let mut registered = BTreeMap::new();
        for definition in definitions {
            let key = (definition.id, definition.version);
            if registered.insert(key, definition.compatibility).is_some() {
                return Err(WorkflowEngineError::DefinitionConflict);
            }
        }
        let migrations = match policy {
            WorkflowReplayPolicy::Strict => Vec::new(),
            WorkflowReplayPolicy::Migrate(migrations) => migrations,
        };
        let mut migration_sources = BTreeSet::new();
        for migration in &migrations {
            if !migration_sources.insert((
                migration.from.id.clone(),
                migration.from.version,
                Arc::clone(&migration.from.compatibility),
            )) {
                return Err(WorkflowEngineError::MigrationConflict);
            }
            if registered.get(&(migration.to.id.clone(), migration.to.version))
                != Some(&migration.to.compatibility)
            {
                return Err(WorkflowEngineError::DefinitionMismatch);
            }
        }
        let mut runs = BTreeMap::new();
        for mut snapshot in snapshots {
            let key = (snapshot.handle.definition.clone(), snapshot.handle.version);
            if registered.get(&key) != Some(&snapshot.compatibility) {
                let migration = migrations
                    .iter()
                    .find(|migration| {
                        migration.from.id == snapshot.handle.definition
                            && migration.from.version == snapshot.handle.version
                            && migration.from.compatibility == snapshot.compatibility
                    })
                    .ok_or(WorkflowEngineError::DefinitionMismatch)?;
                let original_run = snapshot.handle.run.clone();
                snapshot = (migration.transform)(snapshot)?;
                if snapshot.handle.run != original_run
                    || snapshot.handle.definition != migration.to.id
                    || snapshot.handle.version != migration.to.version
                    || snapshot.compatibility != migration.to.compatibility
                {
                    return Err(WorkflowEngineError::MigrationConflict);
                }
            }
            validate_records(&snapshot)?;
            // Running and abandoned are activation-local observations. On
            // reconstruction they are eligible for replay from records.
            if matches!(
                snapshot.status,
                WorkflowStatus::Running | WorkflowStatus::Abandoned
            ) {
                snapshot.status = WorkflowStatus::Ready;
            }
            if runs
                .insert(snapshot.handle.run.clone(), Run::from_snapshot(snapshot))
                .is_some()
            {
                return Err(WorkflowEngineError::RunConflict);
            }
        }
        let mut child_owners = BTreeMap::new();
        for (parent, snapshot) in &runs {
            for child in &snapshot.children {
                if child.handle.run == *parent
                    || child_owners
                        .insert(child.handle.run.clone(), parent)
                        .is_some()
                {
                    return Err(WorkflowEngineError::ChildConflict);
                }
                if let Some(started) = runs.get(&child.handle.run)
                    && (started.handle != child.handle
                        || started.encoded_input != child.encoded_input)
                {
                    return Err(WorkflowEngineError::ChildConflict);
                }
            }
        }
        for run in runs.keys() {
            let mut visited = BTreeSet::new();
            let mut current = run;
            while let Some(parent) = child_owners.get(current) {
                if !visited.insert(current) {
                    return Err(WorkflowEngineError::ChildConflict);
                }
                current = *parent;
            }
        }
        let ready: Vec<_> = runs
            .iter()
            .filter_map(|(run, snapshot)| {
                if snapshot.status != WorkflowStatus::Suspended {
                    return None;
                }
                let reason = match &snapshot.waiting_on {
                    Some(WorkflowWait::Signal { identity, version })
                        if snapshot.signals.iter().any(|signal| {
                            signal.identity == *identity && signal.version == *version
                        }) =>
                    {
                        Some(WorkflowWakeReason::Signal(identity.clone()))
                    }
                    Some(WorkflowWait::Child(child)) => match runs.get(child) {
                        None => Some(WorkflowWakeReason::Child(child.clone())),
                        Some(child_snapshot)
                            if matches!(
                                child_snapshot.status,
                                WorkflowStatus::Completed(_)
                                    | WorkflowStatus::Cancelled
                                    | WorkflowStatus::Abandoned
                            ) =>
                        {
                            Some(WorkflowWakeReason::Child(child.clone()))
                        }
                        Some(_) => None,
                    },
                    Some(WorkflowWait::Deadline(due)) if now.is_some_and(|now| due.at <= now) => {
                        Some(WorkflowWakeReason::Deadline(due.clone()))
                    }
                    _ => None,
                };
                reason.map(|reason| (run.clone(), reason))
            })
            .collect();
        for (run, reason) in ready {
            if let Some(snapshot) = runs.get_mut(&run) {
                snapshot.status = WorkflowStatus::Ready;
                snapshot.waiting_on = None;
                snapshot.wake_reason = Some(reason);
            }
        }
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                definitions: Table::Ordinary(registered),
                runs: Table::Ordinary(runs),
                shutdown: false,
                prepared: None,
            })),
            policy: PhantomData,
        })
    }
}

impl<P> MemoryWorkflowEngine<P> {
    #[must_use]
    /// Capture every registered run’s current replayable state.
    ///
    /// # Returns
    /// Type: `Vec<WorkflowSnapshot>`.
    pub fn snapshot_all(&self) -> Vec<WorkflowSnapshot> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .runs
            .values()
            .map(Run::snapshot)
            .collect()
    }

    #[must_use]
    /// Borrow the workflow definitions registered with this engine.
    ///
    /// # Returns
    /// Type: `Vec<WorkflowRegistration>`.
    pub fn definitions(&self) -> Vec<WorkflowRegistration> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .definitions
            .iter()
            .map(|((id, version), compatibility)| WorkflowRegistration {
                id: id.clone(),
                version: *version,
                compatibility: Arc::clone(compatibility),
            })
            .collect()
    }

    /// Stop this process-local engine atomically. Recovery keeps retained
    /// records, while forced interruption is an explicit unsafe policy for
    /// running external work whose final outcome may be uncertain.
    ///
    /// # Parameters
    /// - `policy` (`WorkflowShutdownPolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `WorkflowShutdownReport`.
    pub fn shutdown(&self, policy: WorkflowShutdownPolicy) -> WorkflowShutdownReport {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.shutdown {
            return WorkflowShutdownReport {
                already_shutdown: true,
                abandoned: 0,
                interrupted: 0,
            };
        }
        state.shutdown = true;
        let mut abandoned = 0usize;
        let mut interrupted = 0usize;
        for snapshot in state.runs.values_mut() {
            match policy {
                WorkflowShutdownPolicy::PreserveForRecovery => {
                    if snapshot.status == WorkflowStatus::Running {
                        snapshot.status = WorkflowStatus::Abandoned;
                        abandoned += 1;
                    }
                }
                WorkflowShutdownPolicy::ForceInterrupt => {
                    if !matches!(
                        snapshot.status,
                        WorkflowStatus::Completed(_) | WorkflowStatus::Cancelled
                    ) {
                        snapshot.status = WorkflowStatus::Cancelled;
                        snapshot.deadline = None;
                        snapshot.waiting_on = None;
                        interrupted += 1;
                    }
                }
            }
        }
        WorkflowShutdownReport {
            already_shutdown: false,
            abandoned,
            interrupted,
        }
    }
}

///
/// # Parameters
/// - `runs` (`&BTreeMap<WorkflowRunId, WorkflowSnapshot>`): The runs supplied to this operation.
/// - `root` (`&WorkflowRunId`): The root supplied to this operation.
///
/// # Returns
/// Type: `Result<Vec<WorkflowRunId>, WorkflowEngineError>`.
fn descendants(
    state: &State,
    root: &WorkflowRunId,
) -> Result<History<WorkflowRunId>, PreparedWorkflowError> {
    if let Some(storage) = &state.prepared {
        return storage
            .collect_descendants(&state.runs, root)
            .map(History::Prepared);
    }
    let runs = &state.runs;
    let mut pending = runs
        .get(root)
        .ok_or(WorkflowEngineError::MissingRun)?
        .children
        .iter()
        .map(|child| child.handle.run.clone())
        .collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    while let Some(run) = pending.pop() {
        if run == *root || !seen.insert(run.clone()) {
            return Err(WorkflowEngineError::ChildConflict.into());
        }
        if let Some(snapshot) = runs.get(&run) {
            pending.extend(
                snapshot
                    .children
                    .iter()
                    .map(|child| child.handle.run.clone()),
            );
        }
        result.push(run);
    }
    Ok(History::Ordinary(result))
}

///
/// # Parameters
/// - `runs` (`&mut BTreeMap<WorkflowRunId, WorkflowSnapshot>`): The runs supplied to this operation.
/// - `child` (`&WorkflowRunId`): Child workflow or scope-owned task being observed.
///
/// # Returns
/// Type: `()`.
fn wake_parent(runs: &mut Runs, child: &WorkflowRunId) {
    for snapshot in runs.values_mut() {
        if snapshot.status == WorkflowStatus::Suspended
            && matches!(&snapshot.waiting_on,
                Some(WorkflowWait::Child(waiting)) if waiting == child)
        {
            snapshot.status = WorkflowStatus::Ready;
            snapshot.waiting_on = None;
            snapshot.wake_reason = Some(WorkflowWakeReason::Child(child.clone()));
        }
    }
}

impl<P: WorkflowStoragePolicy> MemoryWorkflowEngine<P> {
    fn storage_effect<A: Send + 'static>(
        work: impl Fn() -> Result<A, PreparedWorkflowError> + Send + Sync + 'static,
    ) -> Effect<A, P::Error> {
        Effect::ready(move |_, _| P::publish(work()))
    }

    fn start_projected<O: Send + 'static>(
        &self,
        handle: WorkflowHandle,
        encoded_input: Arc<[u8]>,
        project: fn(&Run) -> O,
    ) -> Effect<O, P::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let compatibility = state
                .definitions
                .get(&(handle.definition.clone(), handle.version))
                .ok_or(WorkflowEngineError::DefinitionMismatch)?;
            if let Some(existing) = state.runs.get(&handle.run) {
                if existing.handle != handle
                    || existing.compatibility != *compatibility
                    || existing.encoded_input != encoded_input
                {
                    return Err(WorkflowEngineError::RunConflict.into());
                }
                return Ok(project(existing));
            }
            let compatibility = Arc::clone(compatibility);
            let (histories, retained_bytes) = if let Some(storage) = &state.prepared {
                let capacity = storage.capacity;
                check_bytes(
                    Bytes::new(handle.run.as_bytes().len()),
                    capacity.bytes_per_identity,
                )?;
                check_bytes(
                    Bytes::new(handle.definition.as_str().len()),
                    capacity.bytes_per_identity,
                )?;
                let retained = payload_bytes(capacity, Bytes::new(0), &[encoded_input.len()])?;
                (storage.histories()?, retained)
            } else {
                (
                    PreparedWorkflowHistories {
                        activities: History::ordinary(),
                        children: History::ordinary(),
                        signals: History::ordinary(),
                        compensations: History::ordinary(),
                        races: History::ordinary(),
                    },
                    Bytes::new(0),
                )
            };
            let snapshot = Run {
                execution: 0,
                handle: handle.clone(),
                compatibility,
                encoded_input: Arc::clone(&encoded_input),
                status: WorkflowStatus::Ready,
                activities: histories.activities,
                children: histories.children,
                signals: histories.signals,
                compensations: histories.compensations,
                races: histories.races,
                deadline: None,
                waiting_on: None,
                wake_reason: None,
                history_revision: 0,
                retained_bytes,
            };
            let output = project(&snapshot);
            state.runs.try_insert(handle.run.clone(), snapshot)?;
            Ok(output)
        })
    }

    fn transition_projected<O: Send + 'static>(
        &self,
        run: WorkflowRunId,
        expected: super::WorkflowCheckpoint,
        next: WorkflowStatus,
        project: fn(&Run) -> O,
    ) -> Effect<O, P::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if snapshot.checkpoint() != expected {
                return Err(WorkflowEngineError::StateConflict.into());
            }
            if matches!(
                snapshot.status,
                WorkflowStatus::Completed(_)
                    | WorkflowStatus::Cancelled
                    | WorkflowStatus::Abandoned
            ) {
                return Err(WorkflowEngineError::TerminalRun.into());
            }
            if !matches!(
                (&snapshot.status, &next),
                (WorkflowStatus::Ready, WorkflowStatus::Running)
                    | (WorkflowStatus::Running, WorkflowStatus::Ready)
                    | (WorkflowStatus::Running, WorkflowStatus::Suspended)
                    | (WorkflowStatus::Suspended, WorkflowStatus::Ready)
            ) {
                return Err(WorkflowEngineError::InvalidTransition.into());
            }
            if next == WorkflowStatus::Running {
                snapshot.execution = snapshot
                    .execution
                    .checked_add(1)
                    .ok_or(WorkflowEngineError::ExecutionExhausted)?;
            }
            snapshot.status = next.clone();
            if next == WorkflowStatus::Running {
                snapshot.wake_reason = None;
            } else if next == WorkflowStatus::Ready {
                snapshot.waiting_on = None;
                if expected.status == WorkflowStatus::Suspended {
                    snapshot.wake_reason = Some(WorkflowWakeReason::Manual);
                }
            }
            Ok(project(snapshot))
        })
    }

    fn complete_projected<O: Send + 'static>(
        &self,
        run: WorkflowRunId,
        outcome: EncodedWorkflowOutcome,
        project: fn(&Run) -> O,
    ) -> Effect<O, P::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let prior = state
                .runs
                .get(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            match &prior.status {
                WorkflowStatus::Completed(existing) if existing == &outcome => {
                    return Ok(project(prior));
                }
                WorkflowStatus::Completed(_)
                | WorkflowStatus::Cancelled
                | WorkflowStatus::Abandoned => {
                    return Err(WorkflowEngineError::TerminalRun.into());
                }
                WorkflowStatus::Ready | WorkflowStatus::Running | WorkflowStatus::Suspended => {}
            }
            let child_runs = descendants(&state, &run)?;
            if child_runs.iter().any(|child| {
                !matches!(
                    state.runs.get(child).map(|snapshot| &snapshot.status),
                    Some(WorkflowStatus::Completed(_)) | Some(WorkflowStatus::Cancelled)
                )
            }) {
                return Err(WorkflowEngineError::StateConflict.into());
            }
            let capacity = state.capacity();
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if let Some(capacity) = capacity {
                snapshot.retained_bytes =
                    payload_bytes(capacity, snapshot.retained_bytes, &[outcome.payload.len()])?;
            }
            snapshot.status = WorkflowStatus::Completed(outcome.clone());
            snapshot.deadline = None;
            snapshot.waiting_on = None;
            snapshot.wake_reason = None;
            let completed = project(snapshot);
            wake_parent(&mut state.runs, &run);
            Ok(completed)
        })
    }
}

impl<P: WorkflowStoragePolicy> WorkflowEngine for MemoryWorkflowEngine<P> {
    /// Type alias for error in workflow memory workflow engine.
    type Error = P::Error;

    ///
    /// # Parameters
    /// - `definition` (`WorkflowRegistration`): The definition supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn register(&self, definition: WorkflowRegistration) -> Effect<(), Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let key = (definition.id.clone(), definition.version);
            match state.definitions.get(&key) {
                Some(existing) if existing.as_ref() == definition.compatibility.as_ref() => {
                    return Ok(());
                }
                Some(_) => return Err(WorkflowEngineError::DefinitionConflict.into()),
                None => {}
            }
            if let Some(capacity) = state.capacity() {
                check_bytes(
                    Bytes::new(definition.id.as_str().len()),
                    capacity.bytes_per_identity,
                )?;
                check_bytes(
                    Bytes::new(definition.compatibility.len()),
                    capacity.bytes_per_payload,
                )?;
            }
            state
                .definitions
                .try_insert(key, Arc::clone(&definition.compatibility))?;
            Ok(())
        })
    }

    ///
    /// # Parameters
    /// - `handle` (`WorkflowHandle`): Owned or borrowed capability handle kept live through this operation.
    /// - `encoded_input` (`Arc<[u8]>`): The encoded input supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<WorkflowSnapshot, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn start(
        &self,
        handle: WorkflowHandle,
        encoded_input: Arc<[u8]>,
    ) -> Effect<WorkflowSnapshot, Self::Error> {
        self.start_projected(handle, encoded_input, Run::snapshot)
    }

    fn start_head(
        &self,
        handle: WorkflowHandle,
        encoded_input: Arc<[u8]>,
    ) -> Effect<WorkflowHead, Self::Error> {
        self.start_projected(handle, encoded_input, Run::head)
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    ///
    /// # Returns
    /// Type: `Effect<Option<WorkflowSnapshot>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect(&self, run: WorkflowRunId) -> Effect<Option<WorkflowSnapshot>, Self::Error> {
        let state = Arc::clone(&self.state);
        effect_sync! { body: move || {
            state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .runs
                .get(&run)
                .map(Run::snapshot)
        }, }
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `expected` (`WorkflowStatus`): Value or revision required for the proposed state transition.
    /// - `next` (`WorkflowStatus`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Effect<WorkflowSnapshot, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect_head(&self, run: WorkflowRunId) -> Effect<Option<WorkflowHead>, Self::Error> {
        let state = Arc::clone(&self.state);
        effect_sync! { body: move || {
            state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                .runs.get(&run).map(Run::head)
        }, }
    }

    fn transition(
        &self,
        run: WorkflowRunId,
        expected: super::WorkflowCheckpoint,
        next: WorkflowStatus,
    ) -> Effect<WorkflowSnapshot, Self::Error> {
        self.transition_projected(run, expected, next, Run::snapshot)
    }

    fn transition_head(
        &self,
        run: WorkflowRunId,
        expected: super::WorkflowCheckpoint,
        next: WorkflowStatus,
    ) -> Effect<WorkflowHead, Self::Error> {
        self.transition_projected(run, expected, next, Run::head)
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `activity` (`WorkflowActivityId`): The activity supplied to this operation.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    /// - `encoded_input` (`Arc<[u8]>`): The encoded input supplied to this operation.
    /// - `outcome` (`EncodedWorkflowOutcome`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Effect<RecordedActivity, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn record_activity(
        &self,
        run: WorkflowRunId,
        activity: WorkflowActivityId,
        attempt: NonZeroU32,
        encoded_input: Arc<[u8]>,
        outcome: EncodedWorkflowOutcome,
    ) -> Effect<RecordedActivity, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let capacity = state.capacity();
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            let mut latest_attempt = None;
            for existing in snapshot
                .activities
                .iter()
                .rev()
                .filter(|record| record.identity == activity)
            {
                latest_attempt.get_or_insert(existing.attempt);
                if existing.attempt == attempt {
                    if existing.encoded_input != encoded_input || existing.outcome != outcome {
                        return Err(WorkflowEngineError::ActivityConflict.into());
                    }
                    return Ok(existing.clone());
                }
            }
            if latest_attempt.is_some_and(|latest| latest > attempt) {
                return Err(WorkflowEngineError::ActivityAttemptConflict.into());
            }
            if matches!(
                snapshot.status,
                WorkflowStatus::Completed(_)
                    | WorkflowStatus::Cancelled
                    | WorkflowStatus::Abandoned
            ) {
                return Err(WorkflowEngineError::TerminalRun.into());
            }
            let record = RecordedActivity {
                identity: activity.clone(),
                attempt,
                encoded_input: Arc::clone(&encoded_input),
                outcome: outcome.clone(),
            };
            let charge = snapshot.prepare_history(
                capacity,
                &[activity.as_bytes().len()],
                &[encoded_input.len(), outcome.payload.len()],
            )?;
            snapshot.activities.try_push(record.clone())?;
            snapshot.publish_history(charge);
            Ok(record)
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `activity` (`WorkflowActivityId`): The activity supplied to this operation.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    ///
    /// # Returns
    /// Type: `Effect<Option<RecordedActivity>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect_activity(
        &self,
        run: WorkflowRunId,
        activity: WorkflowActivityId,
        attempt: NonZeroU32,
    ) -> Effect<Option<RecordedActivity>, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let snapshot = state
                .runs
                .get(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            Ok(snapshot
                .activities
                .iter()
                .find(|record| record.identity == activity && record.attempt == attempt)
                .cloned())
        })
    }

    ///
    /// # Parameters
    /// - `parent` (`WorkflowRunId`): Parent scope, span, or workflow identity.
    /// - `child` (`RecordedChild`): Child workflow or scope-owned task being observed.
    ///
    /// # Returns
    /// Type: `Effect<RecordedChild, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn link_child(
        &self,
        parent: WorkflowRunId,
        child: RecordedChild,
    ) -> Effect<RecordedChild, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            if child.handle.run == parent {
                return Err(WorkflowEngineError::ChildConflict.into());
            }
            if let Some(started) = state.runs.get(&child.handle.run)
                && (started.handle != child.handle || started.encoded_input != child.encoded_input)
            {
                return Err(WorkflowEngineError::ChildConflict.into());
            }
            let snapshot = state
                .runs
                .get(&parent)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if let Some(existing) = snapshot
                .children
                .iter()
                .find(|existing| existing.handle.run == child.handle.run)
            {
                return if existing == &child {
                    Ok(existing.clone())
                } else {
                    Err(WorkflowEngineError::ChildConflict.into())
                };
            }
            if matches!(
                snapshot.status,
                WorkflowStatus::Completed(_)
                    | WorkflowStatus::Cancelled
                    | WorkflowStatus::Abandoned
            ) {
                return Err(WorkflowEngineError::TerminalRun.into());
            }
            if state.runs.iter().any(|(run, snapshot)| {
                run != &parent
                    && snapshot
                        .children
                        .iter()
                        .any(|existing| existing.handle.run == child.handle.run)
            }) {
                return Err(WorkflowEngineError::ChildConflict.into());
            }
            let mut ancestor = parent.clone();
            // Every parent visited along a valid chain belongs to the run index.
            let mut remaining = state.runs.len();
            loop {
                if ancestor == child.handle.run || remaining == 0 {
                    return Err(WorkflowEngineError::ChildConflict.into());
                }
                remaining -= 1;
                let next = state.runs.iter().find_map(|(run, snapshot)| {
                    snapshot
                        .children
                        .iter()
                        .any(|linked| linked.handle.run == ancestor)
                        .then(|| run.clone())
                });
                match next {
                    Some(next) => ancestor = next,
                    None => break,
                }
            }
            let capacity = state.capacity();
            let snapshot = state
                .runs
                .get_mut(&parent)
                .ok_or(WorkflowEngineError::MissingRun)?;
            let charge = snapshot.prepare_history(
                capacity,
                &[
                    child.handle.run.as_bytes().len(),
                    child.handle.definition.as_str().len(),
                ],
                &[child.encoded_input.len()],
            )?;
            snapshot.children.try_push(child.clone())?;
            snapshot.publish_history(charge);
            Ok(child.clone())
        })
    }

    ///
    /// # Parameters
    /// - `parent` (`WorkflowRunId`): Parent scope, span, or workflow identity.
    /// - `child` (`WorkflowRunId`): Child workflow or scope-owned task being observed.
    ///
    /// # Returns
    /// Type: `Effect<ChildWait, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn wait_child(
        &self,
        parent: WorkflowRunId,
        child: WorkflowRunId,
    ) -> Effect<ChildWait, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let parent_snapshot = state
                .runs
                .get(&parent)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if !parent_snapshot
                .children
                .iter()
                .any(|record| record.handle.run == child)
            {
                return Err(WorkflowEngineError::ChildConflict.into());
            }
            if matches!(
                parent_snapshot.status,
                WorkflowStatus::Completed(_)
                    | WorkflowStatus::Cancelled
                    | WorkflowStatus::Abandoned
            ) {
                return Err(WorkflowEngineError::TerminalRun.into());
            }
            let child_status = state
                .runs
                .get(&child)
                .map(|snapshot| snapshot.status.clone());
            let parent_snapshot = state
                .runs
                .get_mut(&parent)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if let Some(status) = child_status
                && matches!(
                    status,
                    WorkflowStatus::Completed(_)
                        | WorkflowStatus::Cancelled
                        | WorkflowStatus::Abandoned
                )
            {
                parent_snapshot.waiting_on = None;
                parent_snapshot.wake_reason = Some(WorkflowWakeReason::Child(child.clone()));
                if parent_snapshot.status == WorkflowStatus::Suspended {
                    parent_snapshot.status = WorkflowStatus::Ready;
                }
                return Ok(ChildWait::Terminal(status));
            }
            parent_snapshot.status = WorkflowStatus::Suspended;
            parent_snapshot.waiting_on = Some(WorkflowWait::Child(child.clone()));
            parent_snapshot.wake_reason = None;
            Ok(ChildWait::Waiting)
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `deadline` (`RecordedDeadline`): The deadline for the pending operation.
    ///
    /// # Returns
    /// Type: `Effect<WorkflowSnapshot, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn set_deadline(
        &self,
        run: WorkflowRunId,
        deadline: RecordedDeadline,
    ) -> Effect<WorkflowSnapshot, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let capacity = state.capacity();
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if matches!(
                snapshot.status,
                WorkflowStatus::Completed(_)
                    | WorkflowStatus::Cancelled
                    | WorkflowStatus::Abandoned
            ) {
                return Err(WorkflowEngineError::TerminalRun.into());
            }
            if let Some(capacity) = capacity {
                check_bytes(
                    Bytes::new(deadline.identity.as_bytes().len()),
                    capacity.bytes_per_identity,
                )?;
            }
            if snapshot.status == WorkflowStatus::Suspended
                && matches!(&snapshot.waiting_on, Some(WorkflowWait::Deadline(_)))
            {
                snapshot.waiting_on = Some(WorkflowWait::Deadline(deadline.clone()));
            }
            snapshot.deadline = Some(deadline.clone());
            Ok(snapshot.snapshot())
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `expected` (`RecordedDeadline`): Value or revision required for the proposed state transition.
    ///
    /// # Returns
    /// Type: `Effect<DeadlineClear, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn clear_deadline(
        &self,
        run: WorkflowRunId,
        expected: RecordedDeadline,
    ) -> Effect<DeadlineClear, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if matches!(
                snapshot.status,
                WorkflowStatus::Completed(_)
                    | WorkflowStatus::Cancelled
                    | WorkflowStatus::Abandoned
            ) {
                return Err(WorkflowEngineError::TerminalRun.into());
            }
            if snapshot.deadline.as_ref() != Some(&expected) {
                return Ok(DeadlineClear::Unchanged(snapshot.snapshot()));
            }
            snapshot.deadline = None;
            if matches!(&snapshot.waiting_on,
                Some(WorkflowWait::Deadline(waiting)) if waiting == &expected)
            {
                snapshot.waiting_on = None;
                snapshot.status = WorkflowStatus::Ready;
                snapshot.wake_reason = Some(WorkflowWakeReason::Manual);
            }
            Ok(DeadlineClear::Cleared(snapshot.snapshot()))
        })
    }

    ///
    /// # Parameters
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<Vec<WorkflowRunId>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn wake_due(&self, now: UtcMillis) -> Effect<Vec<WorkflowRunId>, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let mut woken = Vec::new();
            for (run, snapshot) in state.runs.iter_mut() {
                if snapshot.status == WorkflowStatus::Suspended
                    && let Some(WorkflowWait::Deadline(due)) = &snapshot.waiting_on
                    && snapshot.deadline.as_ref() == Some(due)
                    && due.at <= now
                {
                    let due = due.clone();
                    snapshot.status = WorkflowStatus::Ready;
                    snapshot.waiting_on = None;
                    snapshot.wake_reason = Some(WorkflowWakeReason::Deadline(due));
                    woken.push(run.clone());
                }
            }
            Ok(woken)
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `expected` (`RecordedDeadline`): Value or revision required for the proposed state transition.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<DeadlineWake, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn wake_deadline(
        &self,
        run: WorkflowRunId,
        expected: RecordedDeadline,
        now: UtcMillis,
    ) -> Effect<DeadlineWake, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if snapshot.status == WorkflowStatus::Suspended
                && snapshot.deadline.as_ref() == Some(&expected)
                && matches!(&snapshot.waiting_on,
                Some(WorkflowWait::Deadline(due)) if *due == expected)
                && expected.at <= now
            {
                snapshot.status = WorkflowStatus::Ready;
                snapshot.waiting_on = None;
                snapshot.wake_reason = Some(WorkflowWakeReason::Deadline(expected.clone()));
                Ok(DeadlineWake::Woken(snapshot.snapshot()))
            } else {
                Ok(DeadlineWake::Unchanged(snapshot.snapshot()))
            }
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `deadline` (`RecordedDeadline`): The deadline for the pending operation.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<DeadlineWait, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn wait_deadline(
        &self,
        run: WorkflowRunId,
        deadline: RecordedDeadline,
        now: UtcMillis,
    ) -> Effect<DeadlineWait, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let capacity = state.capacity();
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if matches!(
                snapshot.status,
                WorkflowStatus::Completed(_)
                    | WorkflowStatus::Cancelled
                    | WorkflowStatus::Abandoned
            ) {
                return Err(WorkflowEngineError::TerminalRun.into());
            }
            if let Some(capacity) = capacity {
                check_bytes(
                    Bytes::new(deadline.identity.as_bytes().len()),
                    capacity.bytes_per_identity,
                )?;
            }
            snapshot.deadline = Some(deadline.clone());
            if deadline.at <= now {
                if snapshot.status == WorkflowStatus::Suspended {
                    snapshot.status = WorkflowStatus::Ready;
                }
                snapshot.waiting_on = None;
                snapshot.wake_reason = Some(WorkflowWakeReason::Deadline(deadline.clone()));
                Ok(DeadlineWait::Due)
            } else {
                snapshot.status = WorkflowStatus::Suspended;
                snapshot.waiting_on = Some(WorkflowWait::Deadline(deadline.clone()));
                snapshot.wake_reason = None;
                Ok(DeadlineWait::Waiting)
            }
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `signal` (`RecordedSignal`): Named value delivered to a waiting workflow.
    ///
    /// # Returns
    /// Type: `Effect<RecordedSignal, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn deliver_signal(
        &self,
        run: WorkflowRunId,
        signal: RecordedSignal,
    ) -> Effect<RecordedSignal, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let capacity = state.capacity();
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if snapshot.handle.version != signal.version {
                return Err(WorkflowEngineError::DefinitionMismatch.into());
            }
            if let Some(existing) = snapshot
                .signals
                .iter()
                .find(|entry| entry.identity == signal.identity)
            {
                if existing.payload != signal.payload {
                    return Err(WorkflowEngineError::SignalConflict.into());
                }
                return Ok(existing.clone());
            }
            if matches!(
                snapshot.status,
                WorkflowStatus::Completed(_)
                    | WorkflowStatus::Cancelled
                    | WorkflowStatus::Abandoned
            ) {
                return Err(WorkflowEngineError::TerminalRun.into());
            }
            let charge = snapshot.prepare_history(
                capacity,
                &[signal.identity.as_bytes().len()],
                &[signal.payload.len()],
            )?;
            snapshot.signals.try_push(signal.clone())?;
            snapshot.publish_history(charge);
            if snapshot.status == WorkflowStatus::Suspended
                && matches!(&snapshot.waiting_on,
                Some(WorkflowWait::Signal { identity, version })
                    if identity == &signal.identity && *version == signal.version)
            {
                snapshot.status = WorkflowStatus::Ready;
                snapshot.waiting_on = None;
                snapshot.wake_reason = Some(WorkflowWakeReason::Signal(signal.identity.clone()));
            }
            Ok(signal.clone())
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `signal` (`WorkflowSignalId`): Named value delivered to a waiting workflow.
    /// - `version` (`WorkflowVersion`): Registered schema, event, or workflow compatibility version.
    ///
    /// # Returns
    /// Type: `Effect<Option<RecordedSignal>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect_signal(
        &self,
        run: WorkflowRunId,
        signal: WorkflowSignalId,
        version: WorkflowVersion,
    ) -> Effect<Option<RecordedSignal>, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let snapshot = state
                .runs
                .get(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if snapshot.handle.version != version {
                return Err(WorkflowEngineError::DefinitionMismatch.into());
            }
            Ok(snapshot
                .signals
                .iter()
                .find(|entry| entry.identity == signal)
                .cloned())
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `signal` (`WorkflowSignalId`): Named value delivered to a waiting workflow.
    /// - `version` (`WorkflowVersion`): Registered schema, event, or workflow compatibility version.
    ///
    /// # Returns
    /// Type: `Effect<SignalWait, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn wait_signal(
        &self,
        run: WorkflowRunId,
        signal: WorkflowSignalId,
        version: WorkflowVersion,
    ) -> Effect<SignalWait, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let capacity = state.capacity();
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if snapshot.handle.version != version {
                return Err(WorkflowEngineError::DefinitionMismatch.into());
            }
            if let Some(recorded) = snapshot
                .signals
                .iter()
                .find(|entry| entry.identity == signal)
                .cloned()
            {
                if snapshot.status == WorkflowStatus::Suspended
                    && matches!(&snapshot.waiting_on,
                    Some(WorkflowWait::Signal { identity, version: waiting })
                        if identity == &signal && *waiting == version)
                {
                    snapshot.status = WorkflowStatus::Ready;
                    snapshot.waiting_on = None;
                    snapshot.wake_reason = Some(WorkflowWakeReason::Signal(signal.clone()));
                }
                return Ok(SignalWait::Ready(recorded));
            }
            if matches!(
                snapshot.status,
                WorkflowStatus::Completed(_)
                    | WorkflowStatus::Cancelled
                    | WorkflowStatus::Abandoned
            ) {
                return Err(WorkflowEngineError::TerminalRun.into());
            }
            if let Some(capacity) = capacity {
                check_bytes(
                    Bytes::new(signal.as_bytes().len()),
                    capacity.bytes_per_identity,
                )?;
            }
            snapshot.status = WorkflowStatus::Suspended;
            snapshot.waiting_on = Some(WorkflowWait::Signal {
                identity: signal.clone(),
                version,
            });
            snapshot.wake_reason = None;
            Ok(SignalWait::Waiting)
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `identity` (`WorkflowCompensationId`): Stable identity used to deduplicate or find this record.
    /// - `order` (`NonZeroU32`): The order supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<RecordedCompensation, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn register_compensation(
        &self,
        run: WorkflowRunId,
        identity: WorkflowCompensationId,
        order: NonZeroU32,
    ) -> Effect<RecordedCompensation, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let capacity = state.capacity();
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if let Some(existing) = snapshot
                .compensations
                .iter()
                .find(|entry| entry.identity == identity)
            {
                if existing.order != order {
                    return Err(WorkflowEngineError::CompensationConflict.into());
                }
                return Ok(existing.clone());
            }
            if matches!(
                snapshot.status,
                WorkflowStatus::Completed(_)
                    | WorkflowStatus::Cancelled
                    | WorkflowStatus::Abandoned
            ) {
                return Err(WorkflowEngineError::TerminalRun.into());
            }
            if snapshot
                .compensations
                .iter()
                .any(|entry| entry.order == order)
            {
                return Err(WorkflowEngineError::CompensationConflict.into());
            }
            let entry = RecordedCompensation {
                identity: identity.clone(),
                order,
                state: CompensationState::Pending,
            };
            let charge = snapshot.prepare_history(capacity, &[identity.as_bytes().len()], &[])?;
            snapshot.compensations.try_push(entry.clone())?;
            snapshot.publish_history(charge);
            Ok(entry)
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `identity` (`WorkflowCompensationId`): Stable identity used to deduplicate or find this record.
    /// - `outcome` (`EncodedWorkflowOutcome`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Effect<RecordedCompensation, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn finish_compensation(
        &self,
        run: WorkflowRunId,
        identity: WorkflowCompensationId,
        outcome: EncodedWorkflowOutcome,
    ) -> Effect<RecordedCompensation, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let capacity = state.capacity();
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            let index = snapshot
                .compensations
                .iter()
                .position(|entry| entry.identity == identity)
                .ok_or(WorkflowEngineError::CompensationConflict)?;
            let entry = snapshot
                .compensations
                .get(index)
                .ok_or(WorkflowEngineError::CompensationConflict)?;
            match &entry.state {
                CompensationState::Completed(existing) if existing == &outcome => {
                    return Ok(entry.clone());
                }
                CompensationState::Completed(_) => {
                    return Err(WorkflowEngineError::CompensationConflict.into());
                }
                CompensationState::Pending => {}
            }
            let charge = snapshot.prepare_history(capacity, &[], &[outcome.payload.len()])?;
            let entry = snapshot
                .compensations
                .get_mut(index)
                .ok_or(WorkflowEngineError::CompensationConflict)?;
            entry.state = CompensationState::Completed(outcome.clone());
            let output = entry.clone();
            snapshot.publish_history(charge);
            Ok(output)
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    ///
    /// # Returns
    /// Type: `Effect<Vec<RecordedCompensation>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn pending_compensations(
        &self,
        run: WorkflowRunId,
    ) -> Effect<Vec<RecordedCompensation>, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let snapshot = state
                .runs
                .get(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            let mut entries: Vec<_> = snapshot
                .compensations
                .iter()
                .filter(|entry| entry.state == CompensationState::Pending)
                .cloned()
                .collect();
            entries.sort_by_key(|entry| std::cmp::Reverse(entry.order));
            Ok(entries)
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `race` (`WorkflowRaceId`): Stable identity or definition of the durable race.
    ///
    /// # Returns
    /// Type: `Effect<Option<RecordedRace>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect_race(
        &self,
        run: WorkflowRunId,
        race: WorkflowRaceId,
    ) -> Effect<Option<RecordedRace>, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let snapshot = state
                .runs
                .get(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            Ok(snapshot
                .races
                .iter()
                .find(|entry| entry.identity == race)
                .cloned())
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `race` (`RecordedRace`): Stable identity or definition of the durable race.
    ///
    /// # Returns
    /// Type: `Effect<RecordedRace, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn record_race(
        &self,
        run: WorkflowRunId,
        race: RecordedRace,
    ) -> Effect<RecordedRace, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let capacity = state.capacity();
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            if let Some(existing) = snapshot
                .races
                .iter()
                .find(|entry| entry.identity == race.identity)
            {
                return if existing == &race {
                    Ok(existing.clone())
                } else {
                    Err(WorkflowEngineError::RaceConflict.into())
                };
            }
            if matches!(
                snapshot.status,
                WorkflowStatus::Completed(_)
                    | WorkflowStatus::Cancelled
                    | WorkflowStatus::Abandoned
            ) {
                return Err(WorkflowEngineError::TerminalRun.into());
            }
            if !snapshot
                .activities
                .iter()
                .any(|entry| entry.identity == race.winner && entry.attempt == race.attempt)
            {
                return Err(WorkflowEngineError::RaceWinnerMissing.into());
            }
            let charge = snapshot.prepare_history(
                capacity,
                &[race.identity.as_bytes().len(), race.winner.as_bytes().len()],
                &[],
            )?;
            snapshot.races.try_push(race.clone())?;
            snapshot.publish_history(charge);
            Ok(race.clone())
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `outcome` (`EncodedWorkflowOutcome`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Effect<WorkflowSnapshot, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn complete(
        &self,
        run: WorkflowRunId,
        outcome: EncodedWorkflowOutcome,
    ) -> Effect<WorkflowSnapshot, Self::Error> {
        self.complete_projected(run, outcome, Run::snapshot)
    }

    fn complete_head(
        &self,
        run: WorkflowRunId,
        outcome: EncodedWorkflowOutcome,
    ) -> Effect<WorkflowHead, Self::Error> {
        self.complete_projected(run, outcome, Run::head)
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    ///
    /// # Returns
    /// Type: `Effect<CancellationOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn cancel(&self, run: WorkflowRunId) -> Effect<CancellationOutcome, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let child_runs = descendants(&state, &run)?;
            if child_runs.iter().any(|child| {
                matches!(
                    state.runs.get(child).map(|snapshot| &snapshot.status),
                    Some(WorkflowStatus::Running)
                )
            }) {
                return Ok(CancellationOutcome::Active);
            }
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            let outcome = match &snapshot.status {
                WorkflowStatus::Completed(_) => CancellationOutcome::AlreadyCompleted,
                WorkflowStatus::Cancelled => CancellationOutcome::AlreadyCancelled,
                WorkflowStatus::Running => CancellationOutcome::Active,
                WorkflowStatus::Ready | WorkflowStatus::Suspended | WorkflowStatus::Abandoned => {
                    snapshot.status = WorkflowStatus::Cancelled;
                    snapshot.deadline = None;
                    snapshot.waiting_on = None;
                    snapshot.wake_reason = None;
                    CancellationOutcome::Cancelled
                }
            };
            if matches!(outcome, CancellationOutcome::Cancelled) {
                for child in child_runs.iter() {
                    if let Some(snapshot) = state.runs.get_mut(child)
                        && !matches!(snapshot.status, WorkflowStatus::Completed(_))
                    {
                        snapshot.status = WorkflowStatus::Cancelled;
                        snapshot.deadline = None;
                        snapshot.waiting_on = None;
                        snapshot.wake_reason = None;
                    }
                }
            }
            if matches!(outcome, CancellationOutcome::Cancelled) {
                wake_parent(&mut state.runs, &run);
            }
            Ok(outcome)
        })
    }

    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    ///
    /// # Returns
    /// Type: `Effect<CancellationOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn force_interrupt(&self, run: WorkflowRunId) -> Effect<CancellationOutcome, Self::Error> {
        let state = Arc::clone(&self.state);
        Self::storage_effect(move || {
            let mut state = lock_live(&state)?;
            let child_runs = descendants(&state, &run)?;
            let snapshot = state
                .runs
                .get_mut(&run)
                .ok_or(WorkflowEngineError::MissingRun)?;
            let outcome = match &snapshot.status {
                WorkflowStatus::Completed(_) => CancellationOutcome::AlreadyCompleted,
                WorkflowStatus::Cancelled => CancellationOutcome::AlreadyCancelled,
                WorkflowStatus::Ready
                | WorkflowStatus::Running
                | WorkflowStatus::Suspended
                | WorkflowStatus::Abandoned => {
                    snapshot.status = WorkflowStatus::Cancelled;
                    snapshot.deadline = None;
                    snapshot.waiting_on = None;
                    snapshot.wake_reason = None;
                    CancellationOutcome::Cancelled
                }
            };
            if matches!(outcome, CancellationOutcome::Cancelled) {
                for child in child_runs.iter() {
                    if let Some(snapshot) = state.runs.get_mut(child)
                        && !matches!(snapshot.status, WorkflowStatus::Completed(_))
                    {
                        snapshot.status = WorkflowStatus::Cancelled;
                        snapshot.deadline = None;
                        snapshot.waiting_on = None;
                        snapshot.wake_reason = None;
                    }
                }
            }
            if matches!(outcome, CancellationOutcome::Cancelled) {
                wake_parent(&mut state.runs, &run);
            }
            Ok(outcome)
        })
    }
}
