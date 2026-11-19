# Generated graph storage — source implementation

Generated scheduling, canonical scoped children, join/race and actual result
retirement now follow [the G27 scheduling contract](../../SCHEDULING.md).
Source/check handoff is not review acceptance or FULL-13 runtime certification.

`effect_prepare! { graph; program: effect! { ... }, contract: ... }` lowers computation tokens before ordinary `Effect` callback erasure. It emits inferred sealed descriptors, not a second public computation language or interpreter. Ordinary Effect/Layer types, Clone and constructors are unchanged.

`effect_prepare! { description; program: effect_try_sync! { once; ... }, }`
lowers an existing Effect DSL expression to one typed prepared description.
It does not prepare storage, admit a run, or invoke the callback. The resulting
description can be composed into the same prepared graph.

Prepared graphs accept the existing `effect! { while_loop; ... }` factory form
with an explicit `on_capacity: |memory_error| ...` mapper into the graph's E.
The guard runs before each body factory, and each body must produce a prepared
description with value `()`. Initial physical admission returns `MemoryError`
before either factory runs; later rearm refusal invokes the mapper. A panic in
the guard, body factory or mapper remains a defect. The ordinary while-loop
form and its fields are unchanged.

```rust,ignore
let plan = effect_prepare! { graph;
    program: effect! { while_loop;
        guard: move || has_more_work(),
        body: move || effect_try_sync! { body: move || do_one_work_item(), },
        on_capacity: |error| ApplicationError::Capacity(error),
    },
    contract: contract,
};
let minimum = plan.demand(concurrent_runs)?;
let actual = plan.demand_with_layout(concurrent_runs, frames.layout())?;
let graph = effect_prepare! { prepare;
    plan: plan, memory: memory, region: region,
    capacity: concurrent_runs, frames: frames,
}?;
```

Each concurrently admitted loop owns a finite lane: the body has its own
checked frames and description storage, while control frames and the actual
iteration-origin group are separate. A new iteration begins only after the
preceding continuation has returned. A body may save a bare provider-backed
value into enclosing state and still return `()`: success does not prove that
value has retired. Generic output owners, native origin snapshots, projected
environments and escapable context controls therefore use the actual enclosing
pool and sink; their origins survive until that owner retires. Only a proven
duplicate of the loop's already-anchored base environment can use the
recyclable iteration group. Nested loops retain the appropriate persistent
parent authority and have distinct lanes. Finite persistent headroom can be
exhausted by retained owners; rearm reports the actual refusal through the
mapper rather than reclaiming them early. `demand` reports the minimum
enclosing layout and one iteration's checked origin headroom;
`demand_with_layout` accounts the private lane bytes for the supplied actual
layout. Affine
construction sites such as imports remain one-shot: a later rearm reports its
actual `ConstructionAlreadyClaimed` through `on_capacity`, even if unused
frame cells remain. This does not impose an iteration-count limit. Runtime
validation of this source remains deferred.

A graph can instead use `effect_prepare! { graph; input |value| { ... }, contract: ..., cleanup: ..., panic_diagnostics: ... }` for a reusable recipe with one late owned input. `input` and `program` are mutually exclusive; the cleanup and diagnostic fields are optional configuration. With an already-owned `Arc<R>`, `admit_owned_input` reserves the invocation and owned output driver before accepting the input, and `publish_input` moves the input before exposing that driver. It does not bound allocations inside the input or its callbacks.

The current slice covers owned success, native sync/try-sync and concrete native Future/try-Future leaves, typed computation/resource bind, deferred blocks, branch descriptors, prepared environment projection, service environment reads and lexical resource acquisition/borrow/release. Startup stores the reusable description factory in a counted slot. Admission recursively reserves actual typed operation, incoming-outcome resume, continuation and terminal cells plus context/environment owners before calling that factory. A bind producer's concrete returned descriptor type supplies the complete next tree's layout; no opaque Arc is introspected and no post-prefix frame reservation is attempted. Branch admission owns both declared branch shapes; only the selected description runs. Each sealed description also has a typed startup Storage owner and byte demand, provisioned recursively once and supplied to admission. Primitive storage is unit; lexical resources provision actual reusable canonical Scope rows, guarded typed value/factory/body-outcome records and protected cleanup admission. All execution returns existing Step/Once transitions to the same drive evaluator and intrusive destruction mechanism.

Pure success follows Workflow::from_exit (no native-operation reset or cancellation precheck). Native leaves begin/check their operation; bind entry checks cancellation and its producer runs behind a separately reserved More boundary. Outcome and user capture fields precede terminal/context guards in destruction order. Compiler layouts account for owned input outcomes and callback captures rather than only pointers. This is not a proof that arbitrary native callback pointees allocate nothing.

Environment projection reserves its complete typed R record, publication owner and subtree views before invoking the projector; no System Arc<R> is created for the projected environment. The record contains user R before the actual root/node context footer, retaining admission through queued R destruction. Shared views transfer their last owner to a unique cell without publishing free, then use the existing intrusive destruction lane. Ordinary external Arc<R> remains a caller-supplied native dependency, not an arena accounting claim. Layer build/scoped/merge/provide/provide-to now use this counted environment seam, not a System Arc<O> wrapper.

The receipt example constructs menu and quantity inputs with Layer build/merge, then a default build with an explicit two-permit construction shape registers quantity and menu services in the enclosing scope, and provides their checked views to the pricing body. Borrowed service access returns its explicit Result as a value, preserving E=!, then the existing host result mapping handles it. Scope cleanup precedes output. It retains the pricing output owner through native receipt rendering/delivery and derives frame provisioning from graph demand. Its enlarged 512 KiB configuration is engineering headroom, not measured fit. It is checked/linted, not executed or measured.

PreparedScopeClose consumes the SAME ScopeLink::begin_close/DrainGuard authority as ordinary close. Before body entry it holds capacity+1 concrete More/resume/control records, the terminal, concurrent-close Future working set and waiter. Registered settlement guards survive actual cleanup completion through the retained resume frame. The waiter registers/checks under the SAME Inner phase gate; it neither uses Event::listen nor obtains late owners. Waker clone/drop/wake remains outside that gate (external executor Waker allocation behavior is not certified). Notification drains resume on a single unwind, including existing Event wake failure. Replaced cleanup outcomes are destroyed outside the phase gate. Readiness is canonical Scope status, not acknowledgment of a detached native job.

Lexical resource admission installs an EMPTY registered finalizer before acquisition: no unacquired value means no release invocation. The concrete release capture is installed before acquisition; raw successful T is installed into its guarded cell before body cancellation/publication. The owned body outcome is retained for a borrowed PrimaryOutcomeRef through protected cleanup, restored before canonical scope settlement, and combined with cleanup before output. Escaped checked views become ResourceScopeEnded after invalidation. Consuming access does not perform a late observer clone; arbitrary additional dynamic sharing still needs explicit checked ownership. Tail macro values and continuation/explicit-error mapper callback inference preserve ordinary syntax. syn 3 match guards reside in Pat::Guard and are preserved by quoting the complete pattern.

Layer admission owns one canonical scope, derives the finite finalizer maximum recursively, acquires canonical entry PROMISES and complete guarded scoped-finalizer records and reserves both sequential build trees, all callbacks/owned intermediate inputs, environment publishers/views, protected cleanup and the shared lexical Closing body/output handoff before the root factory. Raw success installs T, then commits its finalizer under the same Scope phase gate. A competing closer cannot consume a still-empty release capture: refused late commit awaits its pre-held retained cleanup callback before reporting ordinary ScopeRegistrationClosed (not a capacity-to-Die mapping). Pending promises retain the actual row. Build failure routes through the same close, so acquired earlier services roll back and unacquired later records do not invoke release. Dependent provide keeps its first environment through dependent cleanup; merged resources release LIFO. Scoped release receives Succeeded, matching ordinary acquire_scoped semantics rather than inventing a borrowed body primary. Merge's user combine has its own pre-held More boundary/cancellation check; all Layer terminals retain the actual context node after outcome/finish fields.

The accepted resource final-output footer finding is corrected: its own Terminal<N,V> transfers the existing actual-node context into BOTH normal and put-error terminals, with matching physical demand. A root output ROW alone is not that context NODE. Adjacent earlier failure terminals still have an actual same-node guard in FinishBody, ReleaseFinish or canonical Scope Resume.Done. This correction does not claim the broader escaped-output attachment work is done.

Sealed descriptions now explicitly declare enclosing scope demand and propagate actual scope/protected admission through bind, defer, both branches, projection, lexical A/N/C and provided programs. Build no longer assumes zero; it reserves D's scoped working set before its callback. An ADDITIVE finite construction contract separates P from the executed D: `layer!` with `construction:` supplies a linear permit shape and a two-input build callback; `try_build` is explicitly native/fallible and has a pre-held E failure continuation. Reusable construction recipes contain no invocation permit; each admission creates fresh P. `effect_scope_acquire! { construction; }` is the one-permit plan marker (tuples compose it).

A ScopePermit owns the actual retained PreparedScopeEntry/vacancy. Permit-target binding moves it infallibly; an explicit `scope:` expression is evaluated once in written field order and uses checked retained `same_slot`, returning TargetMismatch with BOTH permit and recipe. No key/global epoch, clone after callback, bank checkout/replenishment, implicit `?`, mismatch Die or scope substitution exists. `unbind` explicitly returns the same permit/recipe. Unused bound descriptions keep their captures/target until dropped, never acquire/release. At execution the descriptor's OWN permit supplies the promise; no second independently chosen admission promise is compared later. The execution vector reuses Layer scoped acquisition/retained refusal-cleanup and canonical gate. `effect_finalizer` uses the same permit path and returns unit; cleanup remains lazy. Explicit host `reserve_permit` is a fallible pre-business companion. Native checked constructor prefixes retain native Rust control/Result handling while existing description constructors lower before erasure.

This is not transparent infallible support for unlimited arbitrary native construction/late target choice. Selected dynamic target imports/providers still need their explicit prebusiness admission contract and full integration; no required target family is permanently excluded. Output origins now follow the [physical ownership contract](../../OUTPUT_OWNERS.md): every implemented graph/Layer node reserves a retained actual environment/node footer before factories, root recipe ownership survives borrowed admission, and mapped/Future outputs keep the complete chain through last-consumer destruction. Checked `with2`/consuming `access2` borrow both actual resource views (aliases lock once); scoped invalidation remains unchanged. Provider business-result transfer requires its actual finally-settled entry/D and a pre-reserved origin cell, not a detached T or count. Checked dynamic prepared-run import and foreign construction now follow [the construction contract](../../CONSTRUCTION.md): real typed vacancies and complete outer frames precede native target construction and all projector/build/business callbacks; imported admitted Step graphs retain their foreign origins through unchanged A/E handoff. This is a checked prepared alternative, not retrospective accounting of an ordinary erased Effect. Provider/fork/join grammar and settlement wiring now follow SCHEDULING.md. Loop-frame recycling and all13 wiring retain their separate requirements. Ordinary APIs remain unchanged. Configured prepared resource, scope, scoped layer and race cleanup use the preadmitted cleanup accumulator. It retains the primary outcome, ordered secondary failures and the selected overflow summary. The prepared while-loop form and its finite rearm contract are described above.

## Native Future leaves

Within `effect_prepare! { graph; ... }`, existing `effect_future!` and
`effect_try_future!` constructors lower before Effect erasure. Reusable and
`once` bodies become per-invocation one-shot native factories; `with_env` and
`exit_with_context` retain their native Future/Result/Exit distinctions. The
native body is not recursively interpreted or rewritten. Concrete compiler
layouts cover the factory operation, pinned pending Future, terminal and native
context control record. All four frames, both environment views, actual node
owners and the cancellation listener are acquired by recursive graph admission
before either graph or native factory invocation. Failed admission rolls them
back; no pending Box or ordinary Effect fallback exists under either memory
policy.

The native callback receives a **prepared companion**, not an ordinary
System-backed `Cancellation` or `NativeContext`: `FutureContext<E>` owns one
pre-admitted notification registration, offers `is_requested`, `request` and
`cancelled(&mut self)`, and moves into the native Future without allocation.
It is intentionally not Clone and does not promise an unlimited number of
simultaneous waits. The observation retains the prepared cancellation/owner-loss
distinction. Environment/context modes receive `Environment<R>` rather than
allocating `Arc<R>` for a projected environment; `with` provides synchronous
checked borrowing and `try_clone` retains its actual backing. Ordinary macros,
callback types and APIs outside prepared graph lowering are unchanged.

Running Futures remain pinned in the existing `Pending` storage and execute
through the same `Step`/`Once`/drive evaluator. Pending and ready polls acknowledge
the existing clock frontier. Terminal publication uses the same actual node's
cancellation/fiber/protected flags. Future destruction precedes environment and
node-owner destruction, including unpolled abandonment. Native cancellation
remains cooperative, not an assertion of native transport/job settlement.

These leaves do **not** make arbitrary native Future pointees, executor wakers,
callback allocations or escaped outputs region-accounted. General provider,
scoped fork/join/race and accepted-job/result drain are described in
SCHEDULING.md, including their pre-reserved source-origin branches. Macro
consumer/runtime, pinning and cancellation validation remain deferred; library
checks are not substitutes for those later checks.

Selected prepared panic diagnostics use preadmitted backing and its configured
message policy. Unconfigured native panic conversion can still allocate ordinary
System diagnostics. Nonempty trace/instrumentation accounting remains separate;
provider settlement/output attachment wiring is described in SCHEDULING.md.
Runtime validation remains deferred. No footprint-fit, zero-allocation, 512 MiB
or latency qualification follows from this slice.


## Finally-settled provider result ownership

`effect_prepare! { provider_result; result: ... }` is a result-only graph adapter
for an already finally-settled unique PreparedOwnedProviderResult. Recursive
preflight reserves its operation and actual provider-entry/D transfer cell before
factories. Sealed dispatch calls a crate-private handoff into the existing typed
Finish/Step continuation: Value stays A, including non-Clone A, while D moves to
the originating root's owner chain. The same live invocation holds its output
through drive/abandonment; the temporary transfer consumer is not stored in its
own root output. Public provider views still expose only borrowed access. This adapter is
result-only; submission, waiting and provider-backed fork/join use SCHEDULING.md,
not a second execution grammar. See [output origins](../../OUTPUT_OWNERS.md) for exact authority/rollback proof.

Prepared descriptions support the existing zero-argument `.either()` and
`effect_map_error!` forms. The latter currently preserves the same error type;
it maps expected errors in primary, first cleanup, then remaining cleanup
order. Defects, interruption and finalization structure remain intact. For
example, a schema operation can be decorated before observing its result:

```rust,ignore
let description = effect_prepare! { description;
    program: effect_map_error! {
        program: schema_operation,
        map: move |issue| issue.at(field.clone()),
    }.either(),
};
```

These descriptions still require graph admission before execution. The
prepared cleanup buffer remains the owner of its existing entries.

The `description` form can optionally take both `capture_errors` and
`classify_error`. The former is a boolean; the latter classifies an expected
error into either a captured value or the original error. When capture is
false, an expected failure stays a failure and the classifier is not called.
When true, only a successful classification becomes `Success(Err(value))`;
a rejected classification stays `Fail`. Defects, interruption and
finalization outcomes pass through unchanged. This is separate from ordinary
`.either()`, which continues to capture every expected failure.
