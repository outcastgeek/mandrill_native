# Application-selected memory

This additive module supplies fixed backing storage; it does not change ordinary `Effect<A,E,R>` execution or install a global allocator. Production integration across the family is still in progress. Tests and runtime measurements are deferred until the full implementation is in place, as required by the current plan.

## Authority and policies

`MemoryPlan` borrows partition descriptors; `MemoryConfiguration<N>` owns an inert descriptor array. Both explicitly prepare owned or borrowed backing. Preparation permits allocation. `seal` consumes preparation authority and returns `Memory`; existing preparation allocator aliases also observe the seal. They may still deallocate, but cannot make new allocations.

`PreparedReuse` rejects new allocation requests after sealing. Prepare `SlotPool`, buffer/text pools, fixed vectors/indexes and admission metadata first, then execute using existing leases. `BoundedAllocation` instead exposes an execution allocator that can fallibly allocate/recycle inside the same backing. No path grows backing or falls back to another allocator. Allocator-backed `Vec` growth is available only when the selected facade permits it; `FixedVector` never exposes growth beyond its selected bound.

`HasMemory` delegates the application capability from R. `HasMemoryPool<P>` delegates a typed prepared pool selected by an application-owned marker value; no global pool lookup occurs. Borrowed storage remains lifetime-bound to the original exclusive slice, including leases that outlive the environment value. Under the existing Effect runtime, returned values must be `'static`; borrowed pools remain directly usable within their Rust storage lifetime rather than forging that lifetime for Effect output.

## Budget accounting

A single fallible backing request holds the shared control header, caller-selected allocator value, partition descriptors and payload ranges. No secondary Arc/Vec/String metadata allocation is hidden in preparation. Handle values themselves occupy caller-owned stack/embedding storage; external containers or interpreter frames holding those values are separate domains until their adapters are integrated. `prepare_in` also permits application-supplied backing policies; any pre-existing external resources held by that allocator remain a separate caller domain.

`total` is a maximum allocator-visible block size, not payload capacity or RSS. Partition allowances include their leading/trailing alignment loss and allocation headers. Without an explicit metadata partition, shared control/descriptors require additional bytes within `total`. Selecting `metadata(region)` charges shared control and (for borrowed storage) initial alignment loss to that partition's allowance; its remaining storage stays usable. The `memory!` configuration form exposes this as `metadata: partition_name`. Unassigned bytes remain reported, not fallback capacity.

### Byte categories and boundaries

All byte counters use exact `usize` bytes, not pages or RSS. The two policies
use the same accounting; sealing and closing do not release any live owner.

| Counter | Meaning and lifetime |
|---|---|
| `MemoryReport::reserved` | Selected `total` managed-region budget, not held admission quota. |
| `MemoryReport::committed` | Complete allocator-visible owned block, or the first `total` exclusively borrowed bytes (including initial alignment loss). Preparation backs the entire region at once, so committed equals reserved on every successful preparation. This does **not** mean OS physical-page commitment or page touching. |
| `MemoryReport::backing` | Compatibility name for committed bytes. Control + usable partition storage + padding + unassigned equals this total. Do not add these breakdowns to committed again. |
| `MemoryUsage::reserved` | Selected partition allowance, including any shared metadata charged to it. |
| `MemoryUsage::committed` | Usable fixed partition span after shared metadata/alignment loss. These spans sum to report partition storage, **not** to complete region commitment. |
| `active_payload` | Requested layout sizes of all nonzero live partition allocations, including idle prepared pool storage and its bookkeeping. Not initialized application length or checkout demand. |
| `active_backing` | Full charges removed from the partition free list: payload + allocation header + alignment/rounding + unsplittable tail. Incremented on allocation and removed exactly once on deallocation. |
| `retained_backing` | **Subset** of active backing held through the explicit ownership boundaries below. Not additional commitment or a byte count per slice. Active backing minus retained backing is the live, non-retention-marked charge. |
| `high_water_backing`, `high_water_retained` | Maximum simultaneously live full charges in the respective category since successful region preparation. Not cumulative allocation volume. |

`active_allocations` excludes zero-size requests. `rejected_allocations` saturates
rather than wrapping and records partition allocation refusals (capacity, policy,
closure and alignment); allocator-handle acquisition errors before an allocation
request are not allocation refusals. Zero-size requests consume no block/header
and add no active or retained charge. The sorted free list coalesces returned
ranges. All partition counters and retained allocation bits are serialized by the
region metadata lock; multi-allocation ownership transitions can be observed in
progress, but each allocation charge is counted at most once.

**Retention is an ownership contract, not pointer-escape detection:**

- `Allocation::retain(self)` explicitly transfers the unique capability into the
  retained category until its actual deallocation. Moving an ordinary allocation
  or cloning a `Memory`, allocator or pool handle does not infer an escape.
- `SlotLease::share` marks its entire slab allocation retained, including pool
  control, records and all inline T storage. Multiple shared slots/views mark the
  same slab only once. The mark ends when the final retention-marked slot returns,
  even if the pool remains alive for reuse. An unrelated ordinary checkout can
  coexist in that slab; attribution is at indivisible allocation granularity,
  not a sum of visible slot sizes.
- Pools from `PreparedMemory::buffers`, `texts` and `frames` additionally track
  each shared slot's concrete separately owned buffer/text/frame allocation.
  Immutable `BufferView` slices/clones retain the complete buffer allocation,
  never just their visible range. Final-view return clears that slot's mark;
  backing remains active for reuse. Private typed tracking shims access only
  allocation metadata, never user callbacks or destructors.
- Shared runtime frames keep the mark through last-view transfer to unique
  destruction ownership. Runtime destruction-queue transfer also explicitly
  marks a previously unique frame. The final queued lease returns only after
  payload destruction/unlinking outside destruction-registry locks. No pointer
  clone is treated as proof of payload escape.

Generic `slots<T>` accounts for its actual inline slab, **not** arbitrary T-owned
pointees. For example, `slots` containing a user-built Buffer is not the concrete
`buffers` factory and cannot discover its separate backing. Use the concrete
factories for their tracking, or an explicit retained `Allocation` capability
inside a caller-owned representation. Arbitrary output graphs, user/dependency
allocations and later domain/provider/transport adapters are not inferred or
implemented by these counters. Likewise, a unique checked-out slot or direct
pinned frame remains active rather than becoming retained merely because it
might be returned from a function. This is not a whole-program liveness oracle.

Partial preparation/factory failure and unwind drop initialized values and their
allocations before returning the slab charge; no retained mark is published
before an actual retention boundary. Failed checkout/share-clone leaves no new
mark or charge. Slot reference-count saturation refuses without incrementing the
reference; successful share is infallible and uses existing metadata. Returning a
slot clears its marks under the pool lock (lock order: pool then region), without
dropping T. Whole-pool destruction still runs T destruction outside those locks,
retaining allocation ownership throughout unwind. Closing, sealing and dropping
an environment do not shorten any allocation lifetime. Last-region-owner drop
releases the owned block once, or ends the exclusive borrowed backing lifetime.

These are allocator-visible accounting guarantees, not measurements of native
allocator bookkeeping, rounded malloc blocks, provider allocations, stacks or
kernel memory. Those remain explicit integration/validation work. A custom
allocator returning more visible bytes than the budget is rejected and released;
no successfully prepared report is published on startup failure.

## Ownership and prepared storage

`Memory` and `RegionAllocator` share through `try_clone`; the custom shared owner supports up to `isize::MAX` simultaneous references. Saturation returns `MemoryError::OwnerLimit` before acquiring payload storage. This is a fallible implementation choice respecting the repository's existing no-abort policy, not a prohibition on Clone/Weak facilities elsewhere in the family.

`Allocation` owns initialized zeroed bytes and a matching allocator reference. `SlotPool<T>` initializes actual T values at startup with ordered fallible factories. Partial initialization and unwinding drop only initialized elements, retaining backing through their destruction. Pool control, free-list records, generations and values occupy one charged allocation. Checkout constructs no payload and performs no heap allocation. The unique `SlotLease` returns its value to the pool on drop; values persist between uses, so callers clear/reset their own state explicitly.

`SlotLease::share` transfers ownership into immutable `SharedSlot` views. `BufferView` supports checked slicing while retaining the entire shared backing owner, not just the visible byte range. The slot returns only after the last view drops. Shared views do not expose mutation. Generation-bearing IDs are local to one pool and never authorize access; generation exhaustion retires a record rather than wrapping, with retired count exposed. Closure rejects new checkout without invalidating live leases. Typed owners preserve Send/Sync constraints and permit local-only values where the runtime contract allows them.

`Buffer` and `Text` append directly to fixed byte ownership. Overflow is explicit, never successful truncation. Text preserves UTF-8 boundaries. `memory_write!` borrows its destination and writes `fmt::Arguments` directly, without an intermediate String. User formatting implementations can still allocate; this writer cannot prove otherwise. `FixedIndex` provides finite linear lookup/storage without rehashing; storage inside arbitrary keys/values remains the caller's domain.

`Text::try_write_fmt` preserves the memory error separately from an error
returned by the formatter. It restores the previous visible text if formatting
fails or unwinds. `PreparedConsole` borrows a checked-out text buffer through
the selected `Console` write and returns capacity errors before calling the
console. The formatter and console backend retain responsibility for their own
allocations and side effects.

## Admission and cleanup/control

`Admission<N>` prepares one finite coordinator for a complete vector of resource units. Admission checks every work/control component under one lock and commits only when the entire vector fits. No partial acquisition waits on another resource. Contention may produce typed Busy refusal. Rollback/return uses the reserved coordinator and never loses quota on lock contention.

Work quota and cleanup/control quota are separate. Work cannot consume emergency capacity. `AdmissionBundle` owns the complete reservation and can split/transfer subsets to children or retained output without reacquisition. `release_work` returns settled work while retaining cleanup quota. Closing ordinary admission still permits the explicit reserved-control path. Quotas describe admission, not another physical memory region: adapters must connect actual leases/task/finalizer/transport demand to the same vector. Dropping a bundle returns counters; it does not invent acknowledgment of asynchronous cleanup.

`AdmissionUsage` exposes capacity and availability vectors in each declared
`CapacityUnit`, plus per-component `high_water_work` and `high_water_control` of
**simultaneously held** quota. Bytes-unit quotas are reservation promises, not
additional physical commitment; do not add them to region byte reports. Peaks
change only after a complete successful vector commit. Refusal of any component
changes no availability or peak, even when earlier components fit. Split transfers
already-held quota; `release_work`/drop restore it exactly once and never decrease
historical peaks. Work and control peaks are independent; zero demand adds none.

`rejected_work` counts refused ordinary `reserve` calls (including refusal caused
by control shortage); `rejected_control` counts refused `reserve_control` calls.
Each failed call increments exactly one saturating statistic, whether due to
capacity, Closed, Busy or owner-count saturation; split refusal is not a new
admission request. A control-only call remains independent after work closure or
saturation. Availability, closure and peak arrays are one locked snapshot;
rejection totals are separate atomic samples and can advance concurrently with
a snapshot. The counter lock can refuse Busy, while shared-owner acquisition can
briefly spin on pool metadata; no partial quota is held while waiting. No labels,
queues, heap owners or diagnostics are allocated by reporting or refusal.

## Interpreter frame storage

The additive [prepared frame seam](FRAMES.md) implements checked physical frame reservations, private initialized pinned owners, separate work/cleanup pools and an explicit-capacity prepared future leaf in the existing evaluator. It retains the actual borrowed lifetime for direct frames and never erases borrowed backing into static callbacks. The remaining interpreter/context/scope/generated-descriptor integration is not presented as complete.

The additive [prepared scope owner](../scope/README.md) shares the ordinary scope phase/close engine while provisioning finite finalizer batches, suffix destinations and typed factory/callback storage. This does not yet replace ordinary scope listeners, all resource acquisition or the complete generated run descriptor.

## Macros and application example

The [macro expansion reference](../../../effect_macros/README.md) lists the
supported memory helpers and their typed constructor equivalents.

The actual exports are `memory!`, `memory_lease!`, `memory_vec!`, `memory_write!` and `effect_drop_guard!`. They use the existing renamed-dependency resolver and DSL construction trust inventory. They do not disable constructor enforcement or hide a global allocator.

`memory!` accepts explicit `total`, `mode: prepared` or `mode: bounded`, named `partition` allowances, optional `slots` allowance arithmetic (`count` × `each`), optional alignment and metadata-partition selection. Quantities accept exact bytes or explicit B/kB/KB/MB/KiB/MiB/GiB literals; dynamic typed byte expressions use checked ordinary APIs. Slot allowance syntax does not itself construct a typed pool: preparation chooses its value type and pays the actual metadata/layout costs. Names map deterministically to numeric RegionIds; collisions are rejected as duplicate IDs. The capability form delegates HasMemory; a handwritten trait implementation remains equivalent.

`memory_lease!` resolves the typed pool through R at execution and maps capacity errors explicitly. Optional `environment: Type` supplies the ordinary Rust type-inference boundary when associated output types are ambiguous. Pool selectors and error mappers evaluate once in source order at description creation. Pool checkout remains lazy. This form uses the ordinary Effect kernel and accounts for the selected pool lease. The enclosing interpreter and providers follow their own execution contracts.

`memory_vec!` reserves before ordered initialization and retains an allocator-aware vector. Its empty form reserves capacity without initializing values. The initialized form preserves application failures via `InitializationError<E>`. `effect_drop_guard!` produces a concrete synchronous guard with cleanup returning unit; bind it until a safe ownership handoff. Async cleanup remains in awaited resource/scope machinery. Nothing here substitutes a destructor for SQL rollback, telemetry flush or provider drain.

See [bounded_receipt.rs](../../examples/bounded_receipt.rs) for a cafe receipt application composing the five macros, typed environment, prepared text pool, admission/control reservation and output ownership. Its normal interpreter/executor/stdout remain separate memory domains. It is not the integrated fixed-budget web example or whole-process strict guarantee. The example has been source-checked, not executed; all runtime validation is deferred.

### Cleanup failure collection migration

`Finalization::rest` is `CleanupFailures<E>`. Existing ordinary Vec values can
be supplied with `.into()`; the collection supports ordered iteration, `push`,
`extend`, `len`, and borrowed slice access. `Clone` always creates an ordinary
Global Vec-backed copy and may invoke `E::clone`; it is not a prepared operation.

`PreparedCleanupPool` prepares the actual finite selected-region buffers at
startup. Each reservation also needs a caller-provided pre-reserved frame; its
payload retires contained failures before its footer returns the buffer after
prepared descendants. Owned iteration moves each yielded failure into its destination; the destination must retain any actual source origins independently. Buffer reuse waits for the remaining contents and their destruction descendants, not values already transferred. The caller selects the number and capacity of buffers;
the configured count is retained secondary entries and one additional entry is reserved for `LibraryDefect::CleanupOverflow` with an omitted
count and saturation indicator. Nested overflow summaries are combined into the final marker; retained real causes remain in their original relative order, while the marker summarizes omissions across the sequence. The prepared execution call sites and complete
admission demand are still being integrated; this API alone does not certify a
bounded cleanup path.

A generated graph reports `demand.cleanup_sites` per concurrent run. Prepare
`PreparedCleanupPool::<E>::prepare(&prepared, region, sites * concurrency,
retained_entries)` and an independent `prepared.frames(region, sites *
concurrency, PreparedCleanupPool::<E>::frame_layout())`, then pass
`PreparedCleanupConfig::new(pool, frames)` as `cleanup:` to `effect_prepare!`
or a prepared `endpoint!`, or use `.with_cleanup(config)` on the plan. A scope
pool can accept `cleanup:` in `effect_scope! { prepare_pool; ... }` or use
`.with_cleanup(config)`. A cleanup-capable graph without configuration is
refused before its business factory; `retained_entries` is caller-selected and
one additional marker slot is reserved. Existing `Vec<CleanupFailure<E>>`
struct literal values require `.into()` for `Finalization::rest`.

`PreparedScopeDemand::cleanup_accumulators` accounts for two buffers per
scoped owner: finalizer accumulation and terminal merge. Each concurrently
admitted direct scope close, prepared release registration, or prepared resource
adds one separately reserved accumulator; resource demand reports the latter.
Prepared graph demand composes its own concrete close, resource, scoped-layer
and race site counts, rather than adding standalone scope demand again.

`memory!` also accepts explicit `schema; memory: ..., region: ..., capacity: ...,
frames: ...`, `schema_array; memory: ..., region: ..., capacity: ..., frames: ...`, `numeric; memory: ..., region: ..., capacity: ...`, `config_store; memory: ..., region: ..., capacity: ...`, and
`config; path: ..., codec: ...` forms. They construct the existing prepared
schema-pool companion, numeric workspace pool, managed configuration store, and inert configuration
description respectively. `config` optionally accepts an explicit `default`
factory. They select no capacities, runtime, or source policy on the caller's
behalf; each expression is evaluated once in written field order.
