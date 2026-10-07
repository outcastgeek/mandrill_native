use effect::eventlog::{
    AggregateId, AppendDecision, AppendReceipt, EventEnvelope, EventGroupId, EventId, EventJournal,
    EventJournalId, EventTag, EventVersion, JournalCursor, MemoryEventJournal,
    MemoryJournalCapacity,
};
use effect::transactional::{
    HasTransactionDomain, Transaction, TransactionDomain, TransactionDomainError, TransactionError,
    TransactionPolicy, TxSubscriptionRef, TxValue,
};
use effect::{
    DuplicateFields, JsonValue, ParseLimits, ParsePolicy, SchemaErrorMode, StringPattern,
    TaggedEnumBuilder, UnknownFields, json_i64, json_string_pattern,
};
use effect::{Effect, Exit, schema, transaction};
use effect::{KeyDomain, KeyNamespace, KeySegment, StableKey, UtcMillis};
use futures_executor::block_on;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

struct Environment(Arc<TransactionDomain>);

impl HasTransactionDomain for Environment {
    fn transaction_domain(&self) -> Arc<TransactionDomain> {
        Arc::clone(&self.0)
    }
}

fn run<A: Send + 'static, E: Send + 'static>(
    effect: &Effect<A, E, Environment>,
    environment: Arc<Environment>,
) -> Exit<A, E> {
    block_on(effect.run(environment))
}

#[test]
fn transaction_for_each_commits_all_steps_and_rolls_back_on_failure() {
    let domain = Arc::new(TransactionDomain::new(NonZeroUsize::new(1).unwrap()));
    let environment = Arc::new(Environment(Arc::clone(&domain)));
    let reference = domain.reference(0usize).unwrap();
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::new(1).unwrap(),
    };

    let item_ref = reference.clone();
    let successful = Transaction::<(), &'static str>::for_each(
        || 0..3,
        move |item| item_ref.update(move |value| Ok((value + item + 1, ()))),
    );
    assert!(matches!(
        run(&successful.atomically(policy), Arc::clone(&environment)),
        Exit::Success(())
    ));
    assert!(
        matches!(run(&reference.read::<&'static str>().atomically(policy), Arc::clone(&environment)), Exit::Success(value) if *value == 6)
    );

    let item_ref = reference.clone();
    let failing = Transaction::<(), &'static str>::for_each(
        || 0..3,
        move |item| {
            item_ref.update(move |value| {
                if item == 1 {
                    Err("stop")
                } else {
                    Ok((value + 1, ()))
                }
            })
        },
    );
    assert!(matches!(
        run(&failing.atomically(policy), Arc::clone(&environment)),
        Exit::Fail(TransactionError::Domain("stop"))
    ));
    assert!(
        matches!(run(&reference.read::<&'static str>().atomically(policy), environment), Exit::Success(value) if *value == 6)
    );
}

#[test]
fn transaction_cell_lives_with_its_last_reference_owner() {
    let capacity = NonZeroUsize::MIN;
    let domain = Arc::new(TransactionDomain::new(capacity));
    let environment = Arc::new(Environment(Arc::clone(&domain)));
    let reference = domain.reference(7usize).expect("first cell");
    let original_id = reference.id();
    let read = reference.read::<()>();
    drop(reference);

    assert!(matches!(
        domain.reference(8usize),
        Err(TransactionDomainError::Capacity)
    ));
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::MIN,
    };
    assert!(matches!(
        run(&read.atomically(policy), environment),
        Exit::Success(value) if *value == 7
    ));

    drop(read);
    let replacement = domain.reference(8usize).expect("released capacity");
    assert_ne!(replacement.id(), original_id);

    drop(replacement);
    let dynamic_domain = Arc::clone(&domain);
    let temporary: Transaction<Arc<usize>, ()> =
        Transaction::succeed_with(|| ()).flat_map(move |_| {
            dynamic_domain
                .reference(11usize)
                .expect("temporary cell")
                .read()
        });
    let environment = Arc::new(Environment(Arc::clone(&domain)));
    assert!(matches!(
        run(&temporary.atomically(policy), environment),
        Exit::Success(value) if *value == 11
    ));
    assert!(domain.reference(12usize).is_ok());
}

struct ReentrantDrop {
    domain: Arc<TransactionDomain>,
    entered: Arc<AtomicBool>,
}

impl TxValue for ReentrantDrop {}

impl Drop for ReentrantDrop {
    fn drop(&mut self) {
        self.entered
            .store(self.domain.reference(()).is_ok(), Ordering::SeqCst);
    }
}

#[test]
fn transaction_commit_drops_replaced_values_outside_domain_gate() {
    const COMMIT_DEADLINE: Duration = Duration::from_secs(5);
    let domain = Arc::new(TransactionDomain::new(
        NonZeroUsize::new(2).expect("two fixture cells"),
    ));
    let entered = Arc::new(AtomicBool::new(false));
    let reference = domain
        .reference(ReentrantDrop {
            domain: Arc::clone(&domain),
            entered: Arc::clone(&entered),
        })
        .expect("initial cell");
    let replacement = Arc::new(ReentrantDrop {
        domain: Arc::clone(&domain),
        entered: Arc::new(AtomicBool::new(false)),
    });
    let operation = reference
        .write::<()>(replacement)
        .atomically(TransactionPolicy {
            max_conflicts: NonZeroUsize::MIN,
        });
    let environment = Arc::new(Environment(Arc::clone(&domain)));
    let (send, receive) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = send.send(matches!(
            block_on(operation.run(environment)),
            Exit::Success(())
        ));
    });
    assert!(
        receive
            .recv_timeout(COMMIT_DEADLINE)
            .expect("commit must settle without deadlocking a reentrant destructor")
    );
    worker.join().expect("commit thread");
    assert!(entered.load(Ordering::SeqCst));
}

#[test]
fn schema_pattern_rejects_bad_encoded_and_decoded_values() {
    let codec = json_string_pattern(StringPattern::Prefix(Arc::from("evt-")), "event-id").unwrap();
    assert_eq!(
        codec
            .decode_ref(&JsonValue::String(String::from("evt-1")))
            .unwrap(),
        "evt-1"
    );
    assert!(
        codec
            .decode_ref(&JsonValue::String(String::from("bad")))
            .is_err()
    );
    assert!(codec.encode_ref(&String::from("bad")).is_err());
    let budget = NonZeroUsize::new(128).unwrap();
    let policy = ParsePolicy {
        limits: ParseLimits {
            bytes: budget,
            depth: budget,
            nodes: budget,
            collection: budget,
            issues: budget,
        },
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    };
    assert_eq!(codec.parse_str("\"evt-2\"", policy).unwrap(), "evt-2");
}

#[test]
fn transaction_macro_for_loop_shares_one_journal() {
    let domain = Arc::new(TransactionDomain::new(NonZeroUsize::new(1).unwrap()));
    let environment = Arc::new(Environment(Arc::clone(&domain)));
    let reference = domain.reference(0usize).unwrap();
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::new(1).unwrap(),
    };
    let operation: Transaction<usize, &'static str> = transaction! {
        let target = reference.clone();
        for _ in 0..3 {
            target.update(|value| Ok((value + 1, ())))?;
        }
        let total = reference.read()?;
        *total
    };
    assert!(matches!(
        run(&operation.atomically(policy), environment),
        Exit::Success(3)
    ));
}

#[test]
fn tagged_enum_codec_round_trips_and_rejects_unknown_tag() {
    enum Message {
        Number(i64),
    }
    let budget = NonZeroUsize::new(128).unwrap();
    let policy = ParsePolicy {
        limits: ParseLimits {
            bytes: budget,
            depth: budget,
            nodes: budget,
            collection: budget,
            issues: budget,
        },
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    };
    let codec = TaggedEnumBuilder::<Message>::new("kind", "payload")
        .variant(
            "number",
            json_i64(),
            |message| match message {
                Message::Number(value) => Some(value),
            },
            Message::Number,
        )
        .unwrap()
        .finish(policy)
        .unwrap();
    let encoded = codec.encode_ref(&Message::Number(7)).unwrap();
    assert!(matches!(codec.decode_ref(&encoded), Ok(Message::Number(7))));
    let mut wrong = encoded;
    wrong.as_object_mut().unwrap().insert(
        String::from("kind"),
        JsonValue::String(String::from("unknown")),
    );
    assert!(codec.decode_ref(&wrong).is_err());
}

#[test]
fn schema_macro_builds_typed_record_codec() {
    struct Record {
        count: i64,
    }
    let budget = NonZeroUsize::new(128).unwrap();
    let policy = ParsePolicy {
        limits: ParseLimits {
            bytes: budget,
            depth: budget,
            nodes: budget,
            collection: budget,
            issues: budget,
        },
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    };
    let codec = schema! { struct Record { count: json_i64() } policy: policy }.unwrap();
    let encoded = codec.encode_ref(&Record { count: 11 }).unwrap();
    assert_eq!(codec.decode_ref(&encoded).unwrap().count, 11);
}

#[test]
fn schema_macro_builds_tagged_enum_codec() {
    enum Message {
        Number(i64),
    }
    let budget = NonZeroUsize::new(128).unwrap();
    let policy = ParsePolicy {
        limits: ParseLimits {
            bytes: budget,
            depth: budget,
            nodes: budget,
            collection: budget,
            issues: budget,
        },
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    };
    let codec = schema! {
        enum Message { Number(json_i64()) => "number" }
        tag_field: "kind",
        content_field: "payload",
        policy: policy
    }
    .unwrap();
    let encoded = codec.encode_ref(&Message::Number(9)).unwrap();
    assert!(matches!(codec.decode_ref(&encoded), Ok(Message::Number(9))));
}

#[test]
fn transaction_commit_reaches_reactive_subscription_without_exposing_rollback() {
    let domain = Arc::new(TransactionDomain::new(NonZeroUsize::new(1).unwrap()));
    let environment = Arc::new(Environment(Arc::clone(&domain)));
    let source = TxSubscriptionRef::new(&domain, 1usize).unwrap();
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::new(1).unwrap(),
    };
    let (initial, subscription) = match run(&source.subscribe(), Arc::clone(&environment)) {
        Exit::Success(pair) => pair,
        _ => panic!("subscription failed"),
    };
    assert_eq!(*initial, 1);

    let rollback = source
        .set::<&'static str>(Arc::new(2))
        .flat_map(|_| Transaction::<(), &'static str>::fail_with(|| "abort"));
    assert!(matches!(
        run(&rollback.atomically(policy), Arc::clone(&environment)),
        Exit::Fail(TransactionError::Domain("abort"))
    ));
    assert!(
        matches!(run(&source.read_committed(), Arc::clone(&environment)), Exit::Success(value) if *value == 1)
    );

    assert!(matches!(
        run(
            &source.set::<&'static str>(Arc::new(3)).atomically(policy),
            Arc::clone(&environment)
        ),
        Exit::Success(())
    ));
    assert!(matches!(run(&subscription.next(), environment), Exit::Success(value) if *value == 3));
    assert!(subscription.dispose());
}

#[test]
fn memory_journal_fences_stale_writes_and_deduplicates_stable_event_ids() {
    struct Domain;
    impl KeyDomain for Domain {
        const TAG: &'static str = "journal-test";
    }
    let namespace = KeyNamespace::<Domain>::new("events").unwrap();
    let key = |value: &'static str| {
        StableKey::new(&namespace, [KeySegment::Text(Arc::from(value))]).unwrap()
    };
    let journal = MemoryEventJournal::new(MemoryJournalCapacity {
        streams: NonZeroUsize::new(1).unwrap(),
        entries_per_stream: NonZeroUsize::new(2).unwrap(),
        tombstones_per_stream: NonZeroUsize::new(1).unwrap(),
    });
    let event = EventEnvelope {
        journal: EventJournalId::from_stable(key("journal")),
        aggregate: AggregateId::from_stable(key("aggregate")),
        id: EventId::from_stable(key("event")),
        group: EventGroupId::new("group").unwrap(),
        tag: EventTag::new("created").unwrap(),
        version: EventVersion::new(NonZeroU32::new(1).unwrap()),
        payload: Arc::from(&b"payload"[..]),
        outcome: None,
        occurred_at: UtcMillis::new(0),
    };
    let execute = |value, cursor| block_on(journal.append(value, cursor).run(Arc::new(())));
    assert!(matches!(
        execute(event.clone(), JournalCursor::new(0)),
        Exit::Success(AppendReceipt::Decided(AppendDecision::Committed(_)))
    ));
    assert!(matches!(
        execute(event.clone(), JournalCursor::new(0)),
        Exit::Success(AppendReceipt::Decided(AppendDecision::Duplicate(_)))
    ));
    let mut different = event.clone();
    different.id = EventId::from_stable(key("other"));
    assert!(
        matches!(execute(different, JournalCursor::new(0)), Exit::Success(AppendReceipt::Decided(AppendDecision::StaleCursor { actual })) if actual.value() == 1)
    );
}

#[path = "support/drive.rs"]
mod correction_drive;
#[test]
fn cancelled_transactional_permit_and_lock_waits_preserve_interruption_and_ownership() {
    use effect::transactional::{TxExecutionId, TxReentrantLock, TxSemaphore};
    use effect::{Cancellation, HasCancellation};
    struct CancelEnv {
        domain: Arc<TransactionDomain>,
        cancel: Cancellation,
    }
    impl HasTransactionDomain for CancelEnv {
        fn transaction_domain(&self) -> Arc<TransactionDomain> {
            Arc::clone(&self.domain)
        }
    }
    impl HasCancellation for CancelEnv {
        fn cancellation(&self) -> &Cancellation {
            &self.cancel
        }
    }
    struct LockDomain;
    impl KeyDomain for LockDomain {
        const TAG: &'static str = "cancelled-lock-test";
    }
    let domain = Arc::new(TransactionDomain::new(NonZeroUsize::new(4).unwrap()));
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::MIN,
    };
    let semaphore = TxSemaphore::new(&domain, 0).unwrap();
    let env = Arc::new(CancelEnv {
        domain: Arc::clone(&domain),
        cancel: Cancellation::new(),
    });
    let entered = Arc::new(AtomicBool::new(false));
    let marker = Arc::clone(&entered);
    let acquire = semaphore.acquire::<()>(NonZeroUsize::MIN);
    let waiting = Transaction::succeed_with(move || marker.store(true, Ordering::SeqCst))
        .flat_map(move |_| acquire.clone())
        .atomically(policy);
    let mut future = Box::pin(waiting.run_cancellable(Arc::clone(&env)));
    correction_drive::until(future.as_mut(), || entered.load(Ordering::SeqCst));
    assert!(env.cancel.request());
    assert!(matches!(block_on(future), Exit::Interrupted));
    assert!(matches!(
        block_on(
            semaphore
                .release(NonZeroUsize::MIN)
                .atomically(policy)
                .run(Arc::clone(&env))
        ),
        Exit::Success(())
    ));
    assert!(matches!(
        block_on(semaphore.available::<()>().atomically(policy).run(env)),
        Exit::Success(1)
    ));
    let lock = TxReentrantLock::new(&domain).unwrap();
    let namespace = KeyNamespace::<LockDomain>::new("owners").unwrap();
    let owner = |name: &'static str| {
        TxExecutionId::from_stable(
            StableKey::new(&namespace, [KeySegment::Text(name.into())]).unwrap(),
        )
    };
    let first = owner("first");
    let second = owner("second");
    let env = Arc::new(CancelEnv {
        domain,
        cancel: Cancellation::new(),
    });
    assert!(matches!(
        block_on(
            lock.acquire(first.clone())
                .atomically(policy)
                .run(Arc::clone(&env))
        ),
        Exit::Success(())
    ));
    let entered = Arc::new(AtomicBool::new(false));
    let marker = Arc::clone(&entered);
    let acquire = lock.acquire(second.clone());
    let waiting = Transaction::succeed_with(move || marker.store(true, Ordering::SeqCst))
        .flat_map(move |_| acquire.clone())
        .atomically(policy);
    let mut future = Box::pin(waiting.run_cancellable(Arc::clone(&env)));
    correction_drive::until(future.as_mut(), || entered.load(Ordering::SeqCst));
    assert!(env.cancel.request());
    assert!(matches!(block_on(future), Exit::Interrupted));
    assert!(matches!(
        block_on(lock.release(first).atomically(policy).run(Arc::clone(&env))),
        Exit::Success(())
    ));
    assert!(matches!(
        block_on(lock.acquire(second).atomically(policy).run(env)),
        Exit::Success(())
    ));
}
