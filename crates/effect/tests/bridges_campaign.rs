use serde_json::json;
#[path = "support/campaign_report.rs"]
mod campaign_report;
#[path = "fixtures/shared/cluster_workflow_takeover.rs"]
pub mod cluster_workflow_takeover;
#[path = "fixtures/capability_consumer/src/host.rs"]
pub mod host;
#[path = "fixtures/shared/provider_program.rs"]
pub mod provider_program;

use effect::eventlog::{HasEventJournal, MemoryEventJournal, MemoryJournalCapacity};
use effect::persistence::{HasPersistedQueueStore, MemoryPersistedQueueStore, QueueStoreCapacity};
use effect::transactional::{HasTransactionDomain, TransactionDomain, TxSubscriptionRef};
use effect::workflow::{HasWorkflowEngine, MemoryWorkflowEngine};
use effect::{Exit, HasCpuExecutor, HasTaskSpawner, JsonValue};
use futures_executor::block_on;
use host::WorkerPool;
use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::Arc;

pub fn drive<F: Future>(future: F) -> F::Output {
    block_on(future)
}

struct Environment {
    journal: Arc<MemoryEventJournal>,
    queue: Arc<MemoryPersistedQueueStore>,
    workflow: Arc<MemoryWorkflowEngine>,
    transactions: Arc<TransactionDomain>,
    tasks: Arc<WorkerPool>,
    cpu: Arc<WorkerPool>,
}

impl HasEventJournal for Environment {
    type Journal = MemoryEventJournal;
    fn event_journal(&self) -> Arc<Self::Journal> {
        Arc::clone(&self.journal)
    }
}
impl HasPersistedQueueStore for Environment {
    type Store = MemoryPersistedQueueStore;
    fn persisted_queue_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.queue)
    }
}
impl HasWorkflowEngine for Environment {
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> {
        Arc::clone(&self.workflow)
    }
}
impl HasTransactionDomain for Environment {
    fn transaction_domain(&self) -> Arc<TransactionDomain> {
        Arc::clone(&self.transactions)
    }
}
impl HasTaskSpawner for Environment {
    type Tasks = WorkerPool;
    fn task_spawner(&self) -> &Self::Tasks {
        &self.tasks
    }
}
impl HasCpuExecutor for Environment {
    type Executor = WorkerPool;
    fn cpu_executor(&self) -> &Self::Executor {
        &self.cpu
    }
}

fn number(config: &JsonValue, path: &[&str]) -> Result<u64, String> {
    path.iter()
        .try_fold(config, |value, field| {
            value
                .get(*field)
                .ok_or_else(|| format!("missing campaign field: {field}"))
        })?
        .as_u64()
        .ok_or_else(|| format!("campaign integer: {}", path.join(".")))
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn environment(tasks: &Arc<WorkerPool>, cpu: &Arc<WorkerPool>) -> Arc<Environment> {
    Arc::new(Environment {
        journal: Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
            streams: NonZeroUsize::MIN,
            entries_per_stream: NonZeroUsize::MIN,
            tombstones_per_stream: NonZeroUsize::MIN,
        })),
        queue: Arc::new(MemoryPersistedQueueStore::new(QueueStoreCapacity::Bounded(
            NonZeroUsize::MIN,
        ))),
        workflow: Arc::new(MemoryWorkflowEngine::new()),
        transactions: Arc::new(TransactionDomain::new(NonZeroUsize::MIN.saturating_add(1))),
        tasks: Arc::clone(tasks),
        cpu: Arc::clone(cpu),
    })
}

#[test]
fn configured_bridge_histories_preserve_authority_after_faults() -> Result<(), String> {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::fs::read_to_string)
        .transpose()
        .map_err(|error| format!("campaign configuration: {error}"))?
        .unwrap_or_else(|| include_str!("feature_campaigns.json").to_owned());
    let config: JsonValue =
        serde_json::from_str(&configured).map_err(|error| format!("campaign JSON: {error}"))?;
    let seeds = number(&config, &["campaigns", "bridges", "seeds"])?;
    let transitions = number(&config, &["campaigns", "bridges", "transitions"])?;
    let workers = |name| -> Result<NonZeroUsize, String> {
        NonZeroUsize::new(
            usize::try_from(number(&config, &["campaigns", "bridges", name])?)
                .map_err(|error| format!("worker policy fits usize: {error}"))?,
        )
        .ok_or_else(|| format!("positive worker policy: {name}"))
    };
    let tasks = Arc::new(WorkerPool::new(
        workers("task_workers")?,
        workers("queued_jobs")?,
    )?);
    let cpu = Arc::new(WorkerPool::new(
        workers("cpu_workers")?,
        workers("queued_jobs")?,
    )?);
    let mut counts = [0_u64; 5];

    for seed in 0..seeds {
        let mut generator = number(&config, &["seed"])?.wrapping_add(seed).max(1);
        let mut order = [0_usize, 1, 2, 3, 4];
        for step in 0..transitions {
            if step % order.len() as u64 == 0 {
                for index in (1..order.len()).rev() {
                    let selection = usize::try_from(next(&mut generator) % (index as u64 + 1))
                        .map_err(|error| format!("selection fits usize: {error}"))?;
                    order.swap(index, selection);
                }
            }
            let case_index = usize::try_from(step % order.len() as u64)
                .map_err(|error| format!("case index fits usize: {error}"))?;
            let case = *order
                .get(case_index)
                .ok_or("case index outside the bridge order")?;
            let env = environment(&tasks, &cpu);
            let preserved =
                match case {
                    0 => {
                        let aborted = TxSubscriptionRef::new(&env.transactions, 41_i64)
                            .map_err(|error| format!("transactional abort source: {error:?}"))?;
                        let abort_preserved = matches!(
                            drive(
                                provider_program::transactional_reactive::<Environment>(aborted)
                                    .run(Arc::clone(&env)),
                            ),
                            Exit::Success(true)
                        );
                        let committed =
                            TxSubscriptionRef::new(&env.transactions, 41_i64).map_err(|error| {
                                format!("transactional notification source: {error:?}")
                            })?;
                        abort_preserved
                            && matches!(drive(
                        provider_program::notification_loss_after_commit::<Environment>(committed)
                            .run(env),
                    ), Exit::Success(true))
                    }
                    1 => drive(provider_program::rpc_event_notification_loss(env)),
                    2 => drive(provider_program::queue_workflow_race(env)),
                    3 => drive(provider_program::replica_observer_restart(env)),
                    4 => drive(cluster_workflow_takeover::run(env)),
                    other => return Err(format!("unexpected bridge case: {other}")),
                };
            assert!(
                preserved,
                "bridge INT0{} failed at seed {seed}, step {step}",
                case + 1
            );
            *counts.get_mut(case).ok_or("missing bridge counter")? += 1;
        }
    }
    tasks.wait_idle();
    cpu.wait_idle();
    tasks.assert_healthy();
    cpu.assert_healthy();
    assert!(counts.iter().all(|count| *count > 0));
    assert_eq!(counts.iter().sum::<u64>(), seeds * transitions);
    let [int01, int02, int03, int04, int05] = counts;
    campaign_report::completed(
        "bridges",
        json!({
            "seeds": seeds,
            "transitions": transitions,
            "operations": counts,
            "bridges": {
                "INT01": int01, "INT02": int02, "INT03": int03,
                "INT04": int04, "INT05": int05,
            },
        }),
    )?;
    Ok(())
}
