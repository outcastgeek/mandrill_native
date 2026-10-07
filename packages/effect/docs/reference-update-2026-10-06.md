# Reference update correction status: 2026-10-06

The previous completion claim was incorrect. The earlier verification snapshot predates the corrections below and cannot establish the corrected behavior. The corrected source and focused fixtures now have the finite verification evidence below. The authorized window ends at 08:02:36 UTC; this report does not extend it.

The approved U1–U7/F1–F8/T1–T10/Z1–Z5 plan remains unchanged. Supported reference snapshots are F# `070304dc87c039d23283c297db089de847cca199` and TypeScript `7064da92f2716f3e8d57e31695ad7d50c7ca2023` (package `4.0.1`), as pinned in the unchanged delta register. Work is limited to `packages/effect`; pinned reference trees and unrelated Rust/native-build changes are preserved.

## Corrected behavior

- Eligible records and variants expose real child-checked JSON writer evidence. Variant writing emits discriminator and selected payload without constructing an ObjectMap/tree. `encodeBytesWith` takes explicit writer limits and chooses a checked fallback for ineligible children; historical APIs remain available. Defaults, transforms and optional omission have explicit single-evaluation fixtures.
- String, exact-number, boolean and generic writers count JSON value depth consistently. Writer failures retain resource/digit details. Numeric equality storage derives from formatting bounds. UUID v4 validates version and variant; exact export rejects format-only validation, while conservative export annotates UUID and over-approximates v4 semantics. Mixed inclusive/exclusive contradictions are rejected at construction and dynamic AST validation.
- `JsonSurface.Limits` separates input bytes/value bytes/JSON depth, schema nodes/edges/depth, issues, integer digits and output writer limits. A selector returning these limits exposes bounded UTF-8 parse methods and a `Parsed(A)` owner. Writer-only selectors retain the DOM adapter and carry no input-document byte claim. Static metadata counts are computed at comptime, including only non-null tagged-union payload children; existing dynamic Definitions preparation remains the dynamic/reference-resolution path. Per-invocation traversal and branch state remain local.
- Prefix/rest and declared/pattern/additional object child failures retain owned field/index paths and constraint detail. Issue clone/deinit and the v1 issue envelope include runtime details and related issues; prior v1 envelopes without detail remain readable. The released-source fixture frees its parsed document before cloning and serializing the issue.
- `DomainCodec.Directed` and mappings/optional composition retain independent pure decode and encode witnesses and direct validation for pure encoding. Native optional values accept effectful children. Native record fixtures use distinct direction environments on the shared trampoline. Primitive JSON codecs carry explicit immutable shape pointers; refinements/transforms preserve child shape plus named executable checks, `asCodec` and domain issue mappings retain metadata, and exact export rejects checks with unknown portable semantics. Conservative export retains the underlying shape. Nullable null writing counts value depth.
- Cache replacement copies are prepared before sampling settlement time. Equality and clocks execute outside the mutex, revision changes retry the sample, and access/expiry are assigned during atomic insertion. Existing gated invalidation/observation proofs remain, with a focused key-retention clock-advance fixture.
- Normal RPC EOF stops intake and drains accepted per-correlation queues. `end_of_input` lets receive drivers stop. Abnormal close/scope cleanup releases unread buffers. A public StreamClient fixture exercises failed decoding, its cancellation finalizer, another correlation's successful chunk, acknowledgement and terminal event; a separate registry fixture covers EOF draining.
- Workflow resume carries an expected checkpoint revision through the provider and cluster bridge. Memory compares it under the storage mutex. Explicit failed-run retry also supplies the observed terminal bytes; both must match before removing the terminal checkpoint and queuing the run. Plain resume does not replay completed runs. Poll captures revisions while locked; activation fences still reject late owners/completion. Recorded activities retain their existing typed replay contract. The staged consumer encodes and observes an actual typed failure, rejects mismatched terminal bytes, resets that checkpoint, and replays its recorded activity under a new activation without executing the handler again.
- The staged independent consumer uses its SafeAllocator in its real Effect/provider environments and checks zero after owners are released. An imported owning payload has public marker/clone/deinit evidence; a private marker negative rejects for the intended reason. Caller control of allocator, I/O and engines is preserved. Zig 0.17 arenas permit concurrent allocation with a thread-safe backing allocator; reset/deinit wait for all users to join.

`Number.toInteger(T, maximum_digits)` supports arbitrary-width Zig integers with explicit decimal digit bounds; the focused fixture uses i512. A separate dynamic big-integer API is not required for that conversion contract.

## Existing applicable dispositions

The original typed trampoline, cancellation and joined scopes, executor capability checks, pool lease shutdown, transactional fresh journals/interruptible waits, prior span restoration, Cron strict-after handling, and cluster activation/message fencing remain in place. Existing proofs are reused where unchanged. The new source does not add absent JS/F# host APIs, SQL/sharding providers, reconnect/liveness timers, regex providers, or global allocation/runtime state.

Zig 0.17 manifests/build modes/reflection/enum migrations, external staged consumer, complete-body codegen comparisons, leak discrimination and inclusive HTTP body limits remain part of the existing finite acceptance. No earlier successful command will be presented as evidence for changed source without the affected integration rerun.

## Verification ledger

Evidence recorded at 07:48 UTC on Zig 0.17.0, native macOS/aarch64. Production corrections preceded verification. Repairs after compiler failures were followed by affected reruns; unchanged passing targets were reused.

| §9.6 acceptance | Actual result |
| --- | --- |
| `zig build fmt` | Passed after final Zig source/fixture edits. |
| `zig build test -Doptimize=debug` | Initial aggregate had only reference-update compile failures; all other targets and the intentional leak harness passed. The repaired reference-update proof passed all 28 cases. After metadata corrections, affected reference-update (28), facilities (35), and new-compositions (6) proofs passed again in debug. The bounded tagged-union static-child repair then passed the same 28-case proof again; no second full debug campaign was run. |
| `zig build test -Doptimize=safe` | All 22 build steps passed, including 28 reference-update cases and clean/leaking discrimination. After metadata corrections, the same three affected proof targets passed again in safe. The final bounded tagged-union repair received the affected 28-case safe rerun. |
| `zig build compile-fail` | All 28 negative consumers rejected for their intended reason; repeated only after changing the schema witness surface. |
| `zig build codegen` | All nine complete optimized-body comparisons passed, including pure boolean writer, tagged variant, direct validation and core map/bind/recovery; repeated after changing PureCodec metadata. |
| `zig build consumer -Doptimize=debug` | Staged independent public-module consumer passed after the typed reset/replay case was added. |
| `zig build consumer -Doptimize=safe` | Same consumer passed with actual SafeAllocator environments and zero outstanding allocations after teardown. |
| `zig build example` | Passed after final schema source changes. |
| `zig build docs` | Passed after final public source changes. |
| Patch hygiene | `git diff --check -- packages/effect` passed. |

The evidence includes the existing injected engines/barriers, cancellation/join, resource failure sweeps, HTTP inclusive body limit, binary wire tags/field order/invalid tags, and leak-harness discrimination. It also includes targeted new object allocation failures, recursive concurrent traversal, cache retention settlement time, blocked producer cancellation, owned issue source release, public StreamClient isolation, and typed checkpoint reset/replay. It is bounded native evidence, not exhaustive upstream suite or cross-provider/platform certification.

## U1–U7 delivery map

| Workstream | Delivered source and public evidence |
| --- | --- |
| U1 | `schema/{codec,native_codec,domain_codec,sequence_codec,schema_issue,issue_codec,json_surface}.zig`, typed native/record builders and immutable preparation; native direction/ownership/recursive/issue fixtures and independent consumer. |
| U2 | `schema/{json_number_codec,json_constraints,json_alternatives,json_writer,json_value,json_schema,variant_schema}.zig`; numeric/Unicode/alternatives/prefix/object/export/direct writer cases and bounded variant parse/encode. |
| U3 | `dependency_cache.zig` and replacement settlement in `cache.zig`; observation/capacity/dependency invalidation and retain-time fixtures, existing pool/cache lifecycle evidence. |
| U4 | Existing shared interpreter, scopes, primitives, streams, schedule/Cron and trace adapters retained, with lazy async iterator Effect acquisition and lifecycle corrections; runtime-binding/concurrency/facility/lifecycle proofs. |
| U5 | `reactivity/invalidation_registry.zig`, local dependency graph and RPC session/StreamClient; generations/reentry, normal EOF draining and per-call failed decode fixtures. |
| U6 | Workflow revision/terminal-byte storage reset, cluster resume bridge and authorization-before-bind; typed failed retry/replay consumer, stale activation and replica authorization fixtures, existing transactional/cluster compositions. |
| U7 | Public root exports, checked ownership/capability negatives, imported owner fixture, actual SafeAllocator consumer, current contracts/README and migration/examples below. The plan and pinned reference trees are unchanged. |

## Fixed reference dispositions

| Row | Disposition, source and relevant evidence |
| --- | --- |
| F1 | Implemented in `dependency_cache.zig` and `cache.zig`; dependency selection/invalidation/admission and settlement fixtures in reference-update, plus existing lifecycle tests. |
| F2 | Implemented in native/domain/sequence codecs and builders; distinct direction environments, optional/default composition and public staged consumer. |
| F3 | Implemented in codec validation, independent pure witnesses, JSON writer and record/variant paths; callback-once/default/optional and writer/representation fixtures, negative witnesses, complete-body codegen. |
| F4 | Implemented in schema issue/issue wire, parser and JSON surface; explicit limits, owned nested paths, released input and wire round-trip fixtures. |
| F5 | Implemented in exact numbers, constraints, alternatives and schema export; exact decimal/i512/UUID/Unicode/uniqueness/prefix/object/pattern/export cases. |
| F6 | Already satisfied by existing typed interpreter/resource contracts; lifecycle/concurrency/facility proofs exercise cleanup, cancellation, late acquisitions and joined children. |
| F7 | Existing lazy Effect acquisition through `Stream.fromPull` and owned async iterator disposal retained; facilities covers native acquisition failure and cancellation/disposal. No second stream interpreter. |
| F8 | Prior span restoration retained in lexical trace adapters; facility/mixed/lifecycle proofs cover state isolation and 10,000 spans. .NET diagnostic displays are not applicable. |
| T1 | Existing runtime/scopes/layers use typed callbacks, protected finalization and joined startup losers; concurrency/lifecycle/runtime-binding evidence reused. JS AsyncLocalStorage/panic recovery absent. |
| T2 | Cache settlement/replacement corrected; existing interrupt/coalescing/pool shutdown ownership retained and verified by lifecycle/facility/reference-update cases. |
| T3 | Existing primitive/PubSub/semaphore transitions retained; added reentrant queue cleanup and cancelled blocked producer cases alongside existing concurrency/facility cases. JS floating capacities absent. |
| T4 | Existing typed stream/sink finalization and late acquisition behavior retained; owning mapper/disposal, async cancellation and acquisition-failure cases. Absent Channel/zipLatest/throttle operations are not added. |
| T5 | Immutable static shape preparation, child metadata/check retention and local recursive traversal implemented; concurrent/reused schema, discriminator and export fixtures. No JS JIT/inheritance adapter. |
| T6 | Registry/subscription generations and local dependency state retained; fixed invalidation watch-key selection; reentrant/fresh watch generations, hydration and composition tests. |
| T7 | EOF drains accepted per-correlation frames; actual StreamClient failed decode cancels only that call and releases owned frames. Existing bounded envelope framing remains; reconnect/liveness timers and newline-host adapters absent. |
| T8 | Provider/cluster resume uses atomic checkpoint revision and optional terminal-byte comparison; typed failed-run reset/replay and stale activation fixtures plus existing cluster fencing cases. SQL/sharding providers absent. |
| T9 | Replica authorization precedes binding; restricted synchronous fresh journals and interruptible transactional waits retained; authorization fixture and staged transactional consumer. |
| T10 | Existing composite config causes, schedule exits, tracing restoration and strict-after Cron behavior retained; facility/consumer evidence reused. No JS Intl or absent logger-batching/HTTP tracing adapter. |

## Zig 0.17 dispositions

Z1 manifests, lowercase optimization modes, tracked Run arguments and staged child-package consumer are implemented. Z2 paired field/type/attribute, function, pointer and enum reflection is migrated without changing wire order/tags. Z3 imported public ownership evidence is exercised and private evidence rejects explicitly. Z4 removed conversions use checked enum membership before `@fromBackingInt`, including the StreamClient-instantiated defect wire. Z5 diagnostics and complete-body LLVM comparisons pass without dropping proof bodies.

For §9.3, static tables use paired metadata directly. The reactive dependency graph pins its actual list-element borrow with `lockPointers`/`unlockPointers`; separately allocated runtime frames and copied preparation frames do not borrow list storage. SafeAllocator is supplied through real consumer R values. No identified caller scratch path required BufferFirstAllocator, and no new timed-wait API was needed. New sizing uses checked arithmetic; no new nonzero-denominator ceiling division needed `@divCeil`. The existing facility URL formatting uses the selected allocator's `print`.

For §9.4, HTTP accepts exactly the configured body bound and rejects excess with the released Reader. There is no existing ambiguous `readAlloc` to rename, sentinel-target JSON parsing, directory enumeration or std RwLock replacement. Explicit JSON value limits apply to actual DOM/UTF-8 representations. Provider capabilities and join-before-destroy remain intact; synchronization/platform fixes are inherited from the release without claiming Evented/Windows certification. Finite numbers reject nonfinite values and uniqueness uses deliberate numeric equality. No backend/linker limitation was encountered on the supported native paths.



## Public-module examples and migration

All complete fixtures import only `effect` through `src/root.zig`. Find native/domain composition in [reference-update native record](../proofs/reference-update.zig#L134), [independent pure directions](../proofs/reference-update.zig#L739), and [distinct native environments](../proofs/reference-update.zig#L1267). The [bounded variant writer/DOM/UTF-8 fixture](../proofs/reference-update.zig#L582) covers payload, void/null and discriminator-only AST cases. [Dependency-cache usage](../proofs/reference-update.zig#L222) supplies its own scope, allocator, I/O and executor and releases every owner. The [independent consumer](../tests/capabilities/consumer/src/main.zig#L841) demonstrates imported ownership, rich UTF-8 exact-number parsing, direct record/string writing and domain optional validation. Its [workflow example](../tests/capabilities/consumer/src/main.zig#L710) shows actual typed failure reset and activity replay. [The external port example](../examples/port.zig) demonstrates the core injected runtime.

Workflow provider migration changes the resume argument. Previously a provider hook received only `ExecutionId`:

```zig
// Before: no observed checkpoint revision was carried to storage.
var resume = engine.vtable.resume_execution(engine.userdata, &env, execution);
```

Now that hook receives `WorkflowTypes.ResumeRequest`. For ordinary suspended/running resumes, use public `WorkflowEngine.resumeExecution` to poll then submit the observed revision, or `resumeChecked` with a caller request selector. For an explicit failed-run retry, keep the owning poll result alive through the request:

```zig
// After: observed is a successful poll of a completed failed run.
const checkpoint = observed.success.completed;
var resume = engine.vtable.resume_execution(engine.userdata, &env, .{
    .execution = execution,
    .expected_revision = checkpoint.revision,
    .expected_terminal_reply = checkpoint.items,
});
defer resume.deinit();
var result = fx.run(&env, resume);
defer result.deinit();
// .recorded resets this checkpoint; .stale_revision changes nothing.
```

Custom providers must compare the revision and supplied terminal bytes under the same storage lock/transaction as the reset. Plain resume of a completed run returns `already_recorded`; it never resets or replays it. Cluster resume commands now carry this request as well.

Normal EOF handling also changes. Previously using close at EOF discarded accepted unread stream frames and the receive driver reported disconnection:

```zig
// Before: normal EOF used destructive registry close.
registry.close();
```

The receive drivers now stop intake and return a successful terminal delivery:

```zig
// After: retain accepted frames for their existing correlation consumers.
registry.stopIntake();
// In a driver loop after running RpcReceiveStreamOne/RpcReceiveAnyOne:
if (delivery == .success and delivery.success == .end_of_input) break;
```

Drain correlation queues through their normal consumers, then release owners. Abnormal failure/scope teardown still uses close to release unread frames. Variant void cases retain the existing explicit null payload wire convention; optional AST payloads are only structural metadata and do not change that convention.


## Later scoped correction: standard JSON and API comments

This is a new user-authorized correction after the earlier finite acceptance;
the former implementation window is closed. The parser already delegated to
`std.json.Scanner`/`parseFromSlice`. The writer/exporter now delegate escaping,
scalar formatting and container/field punctuation to `std.json.Stringify` through
one internal unbuffered checked sink. Effect retains exact decimal semantics,
caller bounds, schema traversal and typed error/ownership conversion. Each std
emitter handles a shallow local clause/container so its fixed debug grammar stack
does not become a hidden application depth limit.

Public comments for the reference-update schema/native/domain/sequence, issue/AST,
cache/dependency, Effect, workflow, RPC and root additions now describe actual
ownership, units/bounds, direction requirements, callbacks and failure channels.
The existing reference proof adds control/Unicode/key escaping, exact token,
inclusive output, owned failure detail, and above-256 DOM/AST depth evidence.
Verification of this scoped correction is complete on Zig 0.17:

- `zig build fmt docs --summary all`: 5/5 steps passed after the final comments.
- The existing reference-update proof: all 29 tests passed in Debug and Safe.
- `zig build consumer -Doptimize=debug --summary all` and the Safe equivalent:
  both independent consumer runs passed, including the fixture's zero-live-owner
  SafeAllocator checks.
- `zig build codegen --summary all`: all nine existing comparisons passed,
  including the pure boolean writer comparison.
- `git diff --check -- packages/effect`: passed.

Initial compile failures in this correction (a local name collision, private
callback access and an unsupported fixture escape) were repaired before those
passing runs. Earlier passing evidence is reused only where these changes do not
affect it; the historical acceptance campaign was not restarted.
