use effect::transaction;
use serde_json::json;
#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::transactional::{
    HasTransactionDomain, Transaction, TransactionDomain, TransactionError, TransactionPolicy,
    TxRef,
};
use effect::{Exit, JsonValue};
use futures_executor::block_on;
use std::io::Write;
use std::num::NonZeroUsize;
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

const MODEL_CELLS: usize = 2;
const DOMAIN_ERROR: &str = "insufficient balance";
const ABORT_ERROR: &str = "aborted after tentative write";

struct Environment(Arc<TransactionDomain>);

impl HasTransactionDomain for Environment {
    fn transaction_domain(&self) -> Arc<TransactionDomain> {
        Arc::clone(&self.0)
    }
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn configured_number(config: &JsonValue, path: &[&str]) -> Result<u64, String> {
    path.iter()
        .try_fold(config, |value, key| {
            value
                .get(*key)
                .ok_or_else(|| format!("missing campaign field {key}"))
        })?
        .as_u64()
        .ok_or_else(|| format!("campaign integer: {path:?}"))
}

fn run<A: Send + 'static>(
    operation: Transaction<A, &'static str>,
    environment: &Arc<Environment>,
    policy: TransactionPolicy,
) -> Exit<A, TransactionError<&'static str>> {
    block_on(operation.atomically(policy).run(Arc::clone(environment)))
}

fn snapshot(left: &TxRef<i64>, right: &TxRef<i64>) -> Transaction<(i64, i64), &'static str> {
    let right = right.clone();
    {
        let __dsl_transaction = left.read::<&'static str>();
        transaction! {
        let first = __dsl_transaction.clone()?; {
                let first = *first;
                { let __dsl_transaction = right
                    .read::<&'static str>(); transaction! {
        let second = __dsl_transaction.clone()?; (first, *second)
        } }
            }?
        }
    }
}

fn transfer(
    source: &TxRef<i64>,
    destination: &TxRef<i64>,
    amount: i64,
) -> Transaction<(), &'static str> {
    let destination = destination.clone();
    {
        let __dsl_transaction = transaction! {
        modify; reference: source, modify: move |balance| {
                    if *balance < amount {
                        Err(DOMAIN_ERROR)
                    } else {
                        Ok((*balance - amount, ()))
                    }
                },
        };
        transaction! {
        __dsl_transaction.clone()?; transaction! {
        modify; reference: destination, modify: move |balance| Ok((*balance + amount, ())),
        }?
        }
    }
}

fn clear_if_guard_positive(
    guard: &TxRef<i64>,
    target: &TxRef<i64>,
    first_read: Arc<Barrier>,
    label: &'static str,
    trace: Arc<Mutex<Vec<String>>>,
) -> Transaction<(), &'static str> {
    let target = target.clone();
    let entered = AtomicBool::new(false);
    {
        let __dsl_transaction = guard.read::<&'static str>();
        transaction! {
        let value = __dsl_transaction.clone()?; {
                let trace_guard = trace.lock();
                assert!(
                    trace_guard.is_ok(),
                    "trace lock: {:?}",
                    trace_guard.as_ref().err()
                );
                if let Ok(mut trace) = trace_guard {
                    trace.push(format!("{label}-read:{value}"));
                }
                if !entered.swap(true, Ordering::SeqCst) {
                    first_read.wait();
                }
                if *value > 0 {
                    target.write(Arc::new(0))
                } else {
                    Transaction::succeed_with(|| ())
                }
            }?
        }
    }
}

#[test]
fn overlapping_disjoint_writes_match_a_serial_order() -> Result<(), Box<dyn std::error::Error>> {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/feature_campaigns.json")
        });
    let config: JsonValue = std::fs::read_to_string(configured)?.parse()?;
    let rounds = configured_number(&config, &["campaigns", "transaction", "seeds"])?;
    let deadline = Duration::from_secs(configured_number(&config, &["budgets", "seconds"])?);
    let started = Instant::now();
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::MIN.saturating_add(MODEL_CELLS - 1),
    };
    let mut observed_serial_orders = [0_u64; MODEL_CELLS];
    let mut schedule_trace = String::new();

    for round in 0..rounds {
        let domain = Arc::new(TransactionDomain::new(
            NonZeroUsize::MIN.saturating_add(MODEL_CELLS - 1),
        ));
        let environment = Arc::new(Environment(Arc::clone(&domain)));
        let left = domain
            .reference(1_i64)
            .map_err(|error| format!("left cell: {error:?}"))?;
        let right = domain
            .reference(1_i64)
            .map_err(|error| format!("right cell: {error:?}"))?;
        let first_read = Arc::new(Barrier::new(MODEL_CELLS));
        let trace = Arc::new(Mutex::new(Vec::new()));
        let clear_left = clear_if_guard_positive(
            &right,
            &left,
            Arc::clone(&first_read),
            "left",
            Arc::clone(&trace),
        );
        let clear_right =
            clear_if_guard_positive(&left, &right, first_read, "right", Arc::clone(&trace));
        let (send, receive) = mpsc::channel();
        let first_env = Arc::clone(&environment);
        let first_send = send.clone();
        let first_trace = Arc::clone(&trace);
        let first = std::thread::spawn(move || -> Result<(), String> {
            let outcome = run(clear_left, &first_env, policy);
            let traced = first_trace
                .lock()
                .map(|mut trace| trace.push(String::from("left-settled")))
                .map_err(|error| format!("trace lock: {error}"));
            first_send
                .send(outcome)
                .map_err(|error| format!("left outcome: {error}"))?;
            traced
        });
        let second_trace = Arc::clone(&trace);
        let second = std::thread::spawn(move || -> Result<(), String> {
            let outcome = run(clear_right, &environment, policy);
            let traced = second_trace
                .lock()
                .map(|mut trace| trace.push(String::from("right-settled")))
                .map_err(|error| format!("trace lock: {error}"));
            send.send(outcome)
                .map_err(|error| format!("right outcome: {error}"))?;
            traced
        });
        for _ in 0..MODEL_CELLS {
            let remaining = deadline
                .checked_sub(started.elapsed())
                .ok_or("native transaction campaign exceeded its time budget")?;
            assert!(
                matches!(receive.recv_timeout(remaining), Ok(Exit::Success(()))),
                "overlapping transaction failed or stalled at round {round}"
            );
        }
        let first = first.join();
        let second = second.join();
        first.map_err(|panic| {
            format!(
                "first transaction thread: {:?}",
                effect::Defect::from_panic(panic)
            )
        })??;
        second.map_err(|panic| {
            format!(
                "second transaction thread: {:?}",
                effect::Defect::from_panic(panic)
            )
        })??;

        let environment = Arc::new(Environment(Arc::clone(&domain)));
        let Exit::Success(actual) = run(snapshot(&left, &right), &environment, policy) else {
            return Err(format!("committed snapshot failed at round {round}").into());
        };
        let mut first_then_second = (1_i64, 1_i64);
        if first_then_second.1 > 0 {
            first_then_second.0 = 0;
        }
        if first_then_second.0 > 0 {
            first_then_second.1 = 0;
        }
        let mut second_then_first = (1_i64, 1_i64);
        if second_then_first.0 > 0 {
            second_then_first.1 = 0;
        }
        if second_then_first.1 > 0 {
            second_then_first.0 = 0;
        }
        if actual == first_then_second {
            observed_serial_orders[0] += 1;
        } else if actual == second_then_first {
            observed_serial_orders[1] += 1;
        } else {
            return Err(format!(
                "write skew or partial commit at round {round}: {actual:?}; trace {:?}",
                trace
                    .lock()
                    .map_err(|error| format!("trace lock: {error}"))?
            )
            .into());
        }
        schedule_trace.push_str(&format!(
            "{round}: {} => {actual:?}\n",
            trace
                .lock()
                .map_err(|error| format!("trace lock: {error}"))?
                .join(", ")
        ));
    }
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or("missing workspace root")?;
    let artifact_relative = config
        .get("artifacts")
        .and_then(JsonValue::as_str)
        .ok_or("missing campaign artifact directory")?;
    let relative = std::path::Path::new(artifact_relative);
    assert!(
        relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    );
    let artifacts = workspace.join(relative);
    assert!(artifacts.starts_with(workspace.join("scratch/reports")));
    std::fs::create_dir_all(&artifacts)?;
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let trace_path = artifacts.join(format!("transaction-overlap-{profile}.trace"));
    std::fs::write(&trace_path, schedule_trace)?;
    writeln!(
        std::io::stderr().lock(),
        "{rounds} overlapping write-skew histories matched serial orders {observed_serial_orders:?}; trace {}",
        trace_path.display()
    )?;
    Ok(())
}

#[test]
fn configured_transaction_histories_match_independent_state_model()
-> Result<(), Box<dyn std::error::Error>> {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/feature_campaigns.json")
        });
    let config: JsonValue = std::fs::read_to_string(configured)?.parse()?;
    let seeds = configured_number(&config, &["campaigns", "transaction", "seeds"])?;
    let transitions = configured_number(&config, &["campaigns", "transaction", "transitions"])?;
    let step_budget = configured_number(&config, &["budgets", "steps"])?;
    let seconds = configured_number(&config, &["budgets", "seconds"])?;
    let initial_balance = i64::try_from(configured_number(
        &config,
        &["campaigns", "transaction", "initial_balance"],
    )?)?;
    let max_transfer = configured_number(&config, &["campaigns", "transaction", "max_transfer"])?;
    assert!(seeds >= 1_000 && transitions >= 100);
    assert!(initial_balance > 0 && max_transfer > 0);
    assert!(max_transfer <= i64::MAX as u64);
    assert!(
        initial_balance
            .checked_mul(MODEL_CELLS as i64)
            .and_then(|total| total.checked_add(i64::try_from(transitions).ok()?))
            .is_some()
    );
    assert!(
        seeds
            .checked_mul(transitions)
            .is_some_and(|steps| steps <= step_budget)
    );
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::MIN,
    };
    let started = Instant::now();
    let deadline = Duration::from_secs(seconds);
    let mut operations = [0_u64; 6];

    for seed in 0..seeds {
        let domain = Arc::new(TransactionDomain::new(
            NonZeroUsize::MIN.saturating_add(MODEL_CELLS - 1),
        ));
        let environment = Arc::new(Environment(Arc::clone(&domain)));
        let left = domain
            .reference(initial_balance)
            .map_err(|error| format!("left cell: {error:?}"))?;
        let right = domain
            .reference(initial_balance)
            .map_err(|error| format!("right cell: {error:?}"))?;
        let mut model = (initial_balance, initial_balance);
        let mut random = configured_number(&config, &["seed"])?
            .wrapping_add(seed)
            .max(1);

        for step in 0..transitions {
            let choice = (next(&mut random) % operations.len() as u64) as usize;
            *operations
                .get_mut(choice)
                .ok_or("operation outside configured set")? += 1;
            match choice {
                0 => {
                    assert!(matches!(
                        run(snapshot(&left, &right), &environment, policy),
                        Exit::Success(actual) if actual == model
                    ));
                }
                1 | 2 => {
                    let amount = i64::try_from(next(&mut random) % max_transfer + 1)?;
                    let (source, destination, available) = if choice == 1 {
                        (&left, &right, model.0)
                    } else {
                        (&right, &left, model.1)
                    };
                    let actual = run(transfer(source, destination, amount), &environment, policy);
                    if available < amount {
                        assert!(matches!(
                            actual,
                            Exit::Fail(TransactionError::Domain(DOMAIN_ERROR))
                        ));
                    } else {
                        assert!(matches!(actual, Exit::Success(())));
                        if choice == 1 {
                            model.0 -= amount;
                            model.1 += amount;
                        } else {
                            model.1 -= amount;
                            model.0 += amount;
                        }
                    }
                }
                3 => {
                    let aborted: Transaction<(), &'static str> = {
                        let __dsl_transaction = left.write(Arc::new(model.0 + 1));
                        transaction! {
                        __dsl_transaction.clone()?; Transaction::fail_with(|| ABORT_ERROR)?
                        }
                    };
                    assert!(matches!(
                        run(aborted, &environment, policy),
                        Exit::Fail(TransactionError::Domain(ABORT_ERROR))
                    ));
                }
                4 => {
                    let abandoned = {
                        let __dsl_transaction = left.write::<&'static str>(Arc::new(model.0 + 1));
                        transaction! {
                        __dsl_transaction.clone()?; Transaction::<(), &'static str>::retry()?
                        }
                    };
                    let alternative = {
                        let __dsl_transaction = right.read::<&'static str>();
                        transaction! {
                        __dsl_transaction.clone()?; ()
                        }
                    };
                    assert!(matches!(
                        run(abandoned.or_else(alternative), &environment, policy),
                        Exit::Success(())
                    ));
                }
                5 => {
                    let observed = left.clone();
                    let increment = {
                        let __dsl_transaction = transaction! {
                        modify; reference: left, modify: |balance| Ok::<_, &'static str>((*balance + 1, ())),
                        };
                        transaction! {
                        __dsl_transaction.clone()?; { let __dsl_transaction = observed.read::<&'static str>(); transaction! {
                        let value = __dsl_transaction.clone()?; *value
                        } }?
                        }
                    };
                    model.0 += 1;
                    assert!(matches!(
                        run(increment, &environment, policy),
                        Exit::Success(value) if value == model.0
                    ));
                }
                _ => return Err("operation outside configured set".into()),
            }
            assert!(
                matches!(
                    run(snapshot(&left, &right), &environment, policy),
                    Exit::Success(actual) if actual == model
                ),
                "model mismatch at seed {seed}, transition {step}, operation {choice}"
            );
        }
        assert!(
            started.elapsed() < deadline,
            "transaction campaign exceeded its time budget"
        );
    }
    assert!(operations.iter().all(|count| *count > 0));
    campaign_report::completed(
        "transaction",
        json!({
            "seeds": seeds, "transitions": transitions, "operations": operations,
        }),
    )?;
    writeln!(
        std::io::stderr().lock(),
        "transaction model completed {seeds} seeds × {transitions} transitions; operation counts {operations:?}"
    )?;
    Ok(())
}
