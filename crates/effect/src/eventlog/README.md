# EventLog

An `EventJournal` is the authority for appended events. A stable `EventId` fences duplicate deliveries, and a `JournalCursor` compare-and-set fences stale stream writers. `MemoryEventJournal` implements those contracts in process memory with caller-selected capacities. A persistent provider must preserve the same atomic identity and cursor decisions across crashes.

For memory compaction, obtain the journal's `retention_authority()`, register required consumers and a replay policy there, then request a permit from that authority. The journal rejects a permit from another authority with `WrongRetentionAuthority` before changing retained history.

`Projection::advance` reads the journal after its stored checkpoint, checks every event group, tag and version, folds outside the store lock, then atomically publishes the new state and checkpoint through `AtomicProjectionStore`. A fold failure writes neither state nor checkpoint. The projection store is a derived view; replaying the journal rebuilds it.

`EventObservation::reconcile` maps committed entries to reactive invalidation keys. It advances the returned cursor only after notification succeeds. On notification failure, `EventObservationError::Notification` carries `replay_after`, so a consumer can re-read from the prior cursor. A notification is never evidence of an append or a replacement for the journal.

The [bridge test](../../tests/eventlog_bridge.rs) executes append → projection → invalidation and injects fold and notification failures. Run it in an optimized native binary with:

```sh
cargo test -p effect --release --test eventlog_bridge
```

The external [provider-neutral consumer](../../tests/fixtures/shared/provider_program.rs) builds `event!` and `projection!` definitions in `event_cpu` and the replica catch-up path, then executes them with journal capabilities supplied through `R`.

The [configured EventLog campaign](../../tests/eventlog_campaign.rs) compares 1,000 seeded 100-step histories against independent journal and projection-count/checkpoint models in debug and native release. It covers append/cursor/read/compaction decisions, successful projection advancement and explicit truncated-history rejection. Replica, protection and recovery histories require separate acceptance evidence.
