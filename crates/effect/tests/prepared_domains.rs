//! Selected domain APIs exercise retained capacity and recovery after refusal.

use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use effect::persistence::{
    CompareExchange, PreparedMemoryKeyValueStore, PreparedStoreCapacity, PreparedStoreError,
    PreparedStoreRead,
};
use effect::{
    CalendarTransitionPolicy, Cron, CronSearchBudget, DayMatch, DefectKind, DiagnosticOverflow,
    Exit, FixedOffsetZone, GapPolicy, KeyDomain, KeyNamespace, KeySegment, KeySegmentRef,
    OverlapPolicy, PreparedChunk, PreparedChunkPool, PreparedClockCapacity, PreparedClockError,
    PreparedCronError, PreparedDiagnosticCapacity, PreparedDiagnosticError, PreparedDiagnostics,
    PreparedFoldPool, PreparedKeyError, PreparedKeyPool, PreparedTestClock, StableKey, UtcMillis,
    memory, sink, stream,
};
use futures_executor::block_on;
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

const WORK: RegionId = RegionId::named("work");
const ONE: NonZeroUsize = NonZeroUsize::MIN;
const TWO: NonZeroUsize = ONE.saturating_add(1);

fn memory() -> Result<PreparedMemory<'static>, MemoryError> {
    memory! { total: 128 KiB, mode: prepared, metadata: work, partition work: 128 KiB, }?.prepare()
}

#[test]
fn retained_stream_chunk_refuses_before_pull_and_fold_reuses_slots() -> Result<(), MemoryError> {
    let prepared = memory()?;
    let chunks = PreparedChunkPool::<u32>::prepare(&prepared, WORK, ONE, TWO)?;
    let accumulator = PreparedFoldPool::<u32>::prepare(&prepared, WORK, 1)?;
    let _sealed = prepared.seal();
    let read = Arc::new(AtomicUsize::new(0));
    let count = read.clone();
    let iterable = [1, 2, 4, 8].into_iter().inspect(move |_| {
        count.fetch_add(1, Ordering::Relaxed);
    });
    let mut stream = stream! {
        iterable_prepared; iterable: iterable, memory: chunks,
        on_capacity: std::convert::identity,
    };
    let Exit::Success(PreparedChunk::Values(retained)) = block_on(stream.pull()) else {
        panic!("first chunk was not produced");
    };
    assert_eq!(retained.as_slice(), &[1, 2]);
    assert_eq!(read.load(Ordering::Relaxed), 2);
    assert!(matches!(
        block_on(stream.pull()),
        Exit::Fail(MemoryError::NoSlot)
    ));
    assert_eq!(read.load(Ordering::Relaxed), 2);
    drop(retained);
    let fold = sink! {
        fold_prepared; stream: &mut stream, memory: &accumulator,
        initial: || 10, step: |sum, item| { *sum += item; Ok(()) },
        on_capacity: std::convert::identity,
    };
    let Exit::Success(output) = block_on(fold) else {
        panic!("fold failed")
    };
    assert_eq!(*output.value(), 22);
    assert_eq!(read.load(Ordering::Relaxed), 4);
    let skipped = sink! {
        fold_prepared; stream: &mut stream, memory: &accumulator,
        initial: || panic!("exhausted output must not start fold"),
        step: |_, _| Ok(()), on_capacity: std::convert::identity,
    };
    assert!(matches!(block_on(skipped), Exit::Fail(MemoryError::NoSlot)));
    drop(output);
    Ok(())
}

#[test]
fn prepared_store_replacement_refusal_preserves_revision_and_reader()
-> Result<(), PreparedStoreError> {
    let prepared = memory()?;
    let store = PreparedMemoryKeyValueStore::prepare(
        &prepared,
        WORK,
        PreparedStoreCapacity {
            entries: ONE,
            key_slots: ONE,
            key_bytes: Bytes::new(8),
            value_slots: TWO,
            value_bytes: Bytes::new(8),
            batches: ONE,
            keys_per_batch: ONE,
        },
    )?;
    let _sealed = prepared.seal();
    let first = store.put(b"order", b"open", None)?;
    let PreparedStoreRead::Found(retained) = store.get(b"order", UtcMillis::new(0))? else {
        panic!("stored value missing");
    };
    assert_eq!(retained.revision, first);
    let second = store.put(b"order", b"paid", None)?;
    assert_eq!(retained.value.bytes(), b"open");
    assert!(
        matches!(store.compare_exchange(b"order", Some(first), Some((b"conflict", None)))?,
        CompareExchange::Conflict { actual: Some(actual) } if actual == second)
    );
    assert_eq!(
        store.put(b"order", b"done", None),
        Err(PreparedStoreError::Memory(MemoryError::NoSlot))
    );
    let PreparedStoreRead::Found(active) = store.get(b"order", UtcMillis::new(0))? else {
        panic!("refusal removed live value");
    };
    assert_eq!(active.revision, second);
    assert_eq!(active.value.bytes(), b"paid");
    drop(active);
    drop(retained);
    let third = store.put(b"order", b"done", None)?;
    assert_ne!(third, second);
    Ok(())
}

#[test]
fn prepared_clock_unlinks_waiters_and_awaits_last_tick() -> Result<(), PreparedClockError> {
    let prepared = memory()?;
    let clock = PreparedTestClock::prepare(
        &prepared,
        WORK,
        Duration::ZERO,
        PreparedClockCapacity {
            timers: ONE,
            advances: ONE,
        },
    )?;
    let _sealed = prepared.seal();
    let second = Duration::from_secs(1);
    let mut context = Context::from_waker(Waker::noop());
    let abandoned = clock.sleep(second)?;
    assert_eq!(clock.pending(), 1);
    assert!(matches!(
        clock.sleep(second),
        Err(PreparedClockError::Memory(MemoryError::NoSlot))
    ));
    drop(abandoned);
    assert_eq!(clock.pending(), 0);
    let mut wait = clock.sleep(second)?;
    let mut advance = clock.advance(second * 2)?;
    assert!(Pin::new(&mut advance).poll(&mut context).is_pending());
    let Poll::Ready(Ok(tick)) = Pin::new(&mut wait).poll(&mut context) else {
        panic!("due timer did not settle");
    };
    assert_eq!(clock.now(), second);
    assert!(Pin::new(&mut advance).poll(&mut context).is_pending());
    let next_wait = clock.sleep(second)?;
    drop(advance);
    assert!(matches!(
        clock.advance(second),
        Err(PreparedClockError::Memory(MemoryError::NoSlot))
    ));
    drop(tick);
    let mut fresh_advance = clock.advance(second)?;
    assert!(Pin::new(&mut fresh_advance).poll(&mut context).is_pending());
    let next_tick = block_on(next_wait)?;
    drop(next_tick);
    block_on(fresh_advance)?;
    assert_eq!(clock.now(), second * 2);
    Ok(())
}

struct Menu;
impl KeyDomain for Menu {
    const TAG: &'static str = "menu";
}

#[test]
fn prepared_stable_key_keeps_wire_identity_while_retained() -> Result<(), Box<dyn std::error::Error>>
{
    let prepared = memory()?;
    let pool = PreparedKeyPool::prepare(&prepared, WORK, ONE, Bytes::new(128))?;
    let _sealed = prepared.seal();
    let namespace = KeyNamespace::<Menu>::new("lunch")?;
    let raw = [
        KeySegmentRef::Text("café"),
        KeySegmentRef::Binary(&[0, 255]),
        KeySegmentRef::Unsigned(7),
    ];
    let original = StableKey::new(
        &namespace,
        [
            KeySegment::Text("café".into()),
            KeySegment::Binary(Arc::from([0, 255])),
            KeySegment::Unsigned(7),
        ],
    )?;
    let key = pool.encode(&namespace, &raw)?;
    assert_eq!(key.as_bytes(), original.as_bytes());
    let retained = key.try_clone()?;
    drop(key);
    assert!(matches!(
        pool.encode(&namespace, &raw),
        Err(PreparedKeyError::Memory(MemoryError::NoSlot))
    ));
    assert_eq!(retained.as_bytes(), original.as_bytes());
    drop(retained);
    assert_eq!(
        pool.encode(&namespace, &raw)?.as_bytes(),
        original.as_bytes()
    );
    Ok(())
}

#[test]
fn prepared_cron_page_matches_civil_resolution_and_refuses_reuse()
-> Result<(), Box<dyn std::error::Error>> {
    let prepared = memory()?;
    let pages = PreparedChunkPool::prepare(&prepared, WORK, ONE, TWO)?;
    let _sealed = prepared.seal();
    let cron = Cron::parse("0 * * * * *", DayMatch::Both)?;
    let zone = FixedOffsetZone::east_seconds(0)?;
    let transitions = CalendarTransitionPolicy {
        gap: GapPolicy::Reject,
        overlap: OverlapPolicy::Both,
    };
    let budget = CronSearchBudget {
        days: ONE.saturating_add(3),
        candidates: ONE.saturating_add(4_000),
    };
    let after = UtcMillis::new(0);
    let through = UtcMillis::new(120_000);
    let ordinary = cron.occurrences_between(after, through, &zone, transitions, budget)?;
    let page =
        cron.occurrences_between_prepared(after, through, &zone, transitions, budget, &pages)?;
    assert_eq!(page.as_slice(), ordinary.as_slice());
    assert_eq!(page.len(), 2);
    assert!(matches!(
        cron.occurrences_between_prepared(after, through, &zone, transitions, budget, &pages),
        Err(PreparedCronError::Memory(MemoryError::NoSlot))
    ));
    drop(page);
    Ok(())
}

#[test]
fn prepared_diagnostic_pages_hold_messages_and_reject_before_formatting()
-> Result<(), PreparedDiagnosticError> {
    let prepared = memory()?;
    let diagnostics = PreparedDiagnostics::prepare(
        &prepared,
        WORK,
        PreparedDiagnosticCapacity {
            entries: ONE,
            messages: TWO,
            bytes_per_message: Bytes::new(16),
            snapshots: ONE,
        },
        DiagnosticOverflow::DiscardNewest,
    )?;
    let _sealed = prepared.seal();
    assert!(diagnostics.try_record(DefectKind::Custom, "first")?);
    struct InvalidFormatter;
    impl std::fmt::Display for InvalidFormatter {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("full ring must reject before formatting")
        }
    }
    assert!(
        !diagnostics.try_record_format(DefectKind::Custom, format_args!("{InvalidFormatter}"))?
    );
    assert_eq!(diagnostics.discarded(), 1);
    let page = diagnostics.try_drain()?;
    assert!(diagnostics.try_record(DefectKind::Custom, "second")?);
    assert!(matches!(diagnostics.try_drain(), Err(MemoryError::NoSlot)));
    let [first] = page.entries() else {
        panic!("wrong first page");
    };
    assert_eq!(first.message(), "first");
    drop(page);
    let second = diagnostics.try_drain()?;
    let [entry] = second.entries() else {
        panic!("refused drain lost its event");
    };
    assert_eq!(entry.message(), "second");
    Ok(())
}
