# Prepared output origins (W05-16 source contract)

Output ownership is independent of execution completion. A root row by itself
is not an originating context node, projected environment, resource view or
provider working-set owner.

## Physical preflight and final consumer

`PreparedRunContext::reserve_output_owner<T>` obtains an exact
`PreparedOutputOwnerReservation<T, A, E>` from the supplied frame pool, plus
already-counted ownership of its actual originating node and root. Its `layout`
is the compiler layout of T, the intrusive predecessor owner and node footer.
Reserve it in the pre-business admission hook, not after constructing expensive
native output. `attach` transfers the actual T into that cell; it does not clone
T, reserve additional owners, grow a collection, allocate System metadata or
replace T with a numeric claim. Reservation also pre-acquires a counted output consumer. A closed output
refuses that reservation; no root can close between successful reservation and
handoff. An unused reservation rolls back. `attach_retained` returns an immutable
`PreparedOutputOwnerView` of the installed cell with that pre-held consumer; it
never returns bare T. `attach` discards only this extra observer, leaving T in the
origin chain for the remaining output consumers. Attachment marks this concrete frame retained
without charging its payload twice.

The root holds an intrusive chain of these cells. Origin cells split their
payload from their predecessor/node footer into two explicit destruction
phases, not just ordinary struct field declaration order. Every generated computation
and Layer admission includes a cell for its actual selected environment and
originating context node, before any description/native factory runs. Recursive
shapes include these cells' layouts and counts; scoped admission forwards the
actual scope/protected context rather than inventing another target. Both root
families retain their startup recipe owner. A concrete native root also retains
its selected external Arc<R>. Projected environment records retain their own
region allocation and node footer, not a newly allocated Arc<R>.

`OutputLease` counts every shared `PreparedOutput`, mapped `LeasedRunResult`,
original mapped Future and its pre-held Ready-transfer owner. Mapping never sets
the consumer count to zero while a mapped output remains alive. A ready Future
and its produced value can be dropped in either order; both retain the same
origin chain until their final consumer. Safe `map` callbacks borrow the original Exit/T; they cannot send the original
owned value elsewhere and return unit. The original root Exit remains in its
root record through safe mapping. Owned transformations are only available via
`unsafe map_owned_unchecked`, whose explicit contract forbids detaching original
values/pointees from the resulting consumer except with independent actual
origin ownership. The receipt's sole owned-mapping caller moves the receipt
only into its retained delivery Future and settles synchronous write/flush there.
The last consumer takes the root result and origin chain out of their mutexes
and initiates value retirement before origins. This explicitly breaks deliberate
root -> origin -> node -> root links. Publication, `R` destruction, mapping and
native EOF cannot independently recycle those cells. Pending reservations and returned retained views count as actual output
consumers, so final-output Drop cannot overtake their handoff or borrowed access.
No user value, native budget or waker destructor executes under the chain gate.
A retained view is an external consumer, not a value to insert into its own
originating output; intentionally storing same-output consumers inside their
own result can form an ownership cycle and retain capacity, never unsafe reuse.

Prepared destruction is now iterative depth-first postorder with FIFO siblings:
payload -> its complete prepared descendant tree -> footer -> footer descendant
tree -> physical owner release. Children queued by physical release are placed
before remaining siblings using root-owned intrusive scratch links; no releasing
node is accessed after its owner may free it. Projected/shared environment
records also split R from the actual source-node footer, including cross-root
retention. Thus queued R or nested PreparedExecutionLease payloads cannot be
overtaken by predecessor/recipe/node retirement. Output Drop inside an already
active drain enqueues payload descendants before origin siblings. Phase/live
flags advance before user Drop; the same traversal continues on a single unwind.
There is no recursive graph descent, new allocation, user Drop under locks, or
change to ordinary second-panic/abort semantics. System FlatDrop policy is not
changed by this prepared-lane dependency fix.

This is deliberately conservative: known subtree environments/nodes (including
reserved branch shapes) remain held through the output even when some branches
do not run. Capacity is finite and described by demand, not an unbounded output
registry. `PreparedReuse` checks out prepared cells; `BoundedAllocation` uses the
same cells without asserting that the selected native working set is startup
allocation-free. Native root demand now includes six cells: original
start/pending/completion/store plus recipe and environment retention.

## Borrowed resource/run integration

`PreparedGraphResourceValue::with2` synchronously borrows two actual counted
views, including aliases of one backing. Its consuming `access2` is a concrete
existing Ready leaf: both views and the callback are part of the leaf layout;
there is no late observer clone. `Environment::with_pair` takes an alias's
publication lock once (checked Any borrowing, no cross-type raw cast), and
orders distinct records by stable owner identity. These borrowed references
cannot escape the callback. The callback must not await, recursively borrow or
retire either borrowed record. N-way native code must arrange its complete
borrow set before entering callbacks, rather than nesting acquisitions that
reacquire an already-held publication lock.

Canonical resource cleanup still invalidates checked views before releasing T.
Retaining output capacity is NOT permission to keep a scoped resource usable
after close: late accesses continue to report ResourceScopeEnded. Release,
protected finalization and error ordering are not replaced by Drop. Multiple
independently retained resource views already retain their complete shared
publication record; pair access preserves both records during the borrow.

Both `PreparedGraph::admit(&self, ...)` and `PreparedRun::admit(&self, ...)`
borrow the reusable runner only during admission. The invocation owns its
recipe, frames, environment and node/output handles; running/dropping an
invocation or retaining/mapping its output does not borrow the runner. Dropping
the reusable runner cannot invalidate those records. Likewise, `with`, `with2`
and result-observation callbacks borrow only for their synchronous call. The
ordinary Effect run/owned-run APIs are unchanged and are not silently certified
as prepared by this companion.

## Provider output settlement transfer

Provider publication (`PreparedReceipt::settled`) remains a result-notification
boundary; it is not relabeled final native execution retirement. Each prepared
cancellation/control cell now has a separate final-settlement flag. The worker
sets it after future destruction, result publication, execution-owner retirement
and occupied queue/running-count retirement, BEFORE notifying the final waiter.
The worker retains its entry/D through notification and then retires its entry
owner under the existing diagnostic contracts. The flag does not assert that
entry/D destruction has already completed. Its acquire/release handoff cannot be
manufactured by consumer Drop or cancellation.

`prepared_provider::PreparedOutput::try_take` is **nonblocking** and recoverable.
It requires that final flag and unique entry ownership (no receipt, worker or
other result aliases; the pool has no weak-upgrade path). It does NOT extract T:
`PreparedOwnedProviderResult` is an exclusive retained view with its settlement
still in the SAME entry. Borrowing uses the entry's existing result gate.
`transfer` moves that actual entry/D owner into the originating run's
**pre-reserved** cell and returns `PreparedOutputOwnerView` carrying its already
admitted output consumer. Its `result().settlement()` only borrows the native
settlement. Dropping the original output concurrently cannot free this view's
entry, result or D. No safe public transfer returns bare ProviderSettlement<T>.

The provider's final EntryOwner uses the same intrusive postorder lane. Its
payload phase retires the future, result and waiter, including descendants;
its footer phase retires D and D's descendants; its release phase returns the
entry and its independently pre-held actual cancellation/control slot. An
abandoned exclusive view has this same teardown; no moved settlement on the
stack can enqueue T and release D prematurely by ordinary field order. The
entry/node/queued-owner/control layouts are included in startup storage bytes;
retirement allocates no metadata. Graph business A/E and all Exit channels are
unchanged; public provider observers expose only consumer-coupled borrowing.

### Private unchanged-type engine handoff

The external counted view is not used as an internal business-value substitute.
`effect_prepare! { provider_result; result: finally_settled_unique_result }`
constructs a sealed **result-only** description in the existing prepared graph
computation/bind grammar. Its Value is exactly the provider's A (including
non-Clone A); its Error/Env remain the enclosing E/R. It neither submits nor
waits for provider work. The input must already be an affine
PreparedOwnedProviderResult obtained by the final-settlement/uniqueness check.
A provider A which is itself Result/Exit remains that same A; the adapter does
not silently flatten or discard a business value's channels.

The real call path is macro -> ProviderResultDescription::reserve/dispatch ->
prepared Once/Step operation -> crate-private PreparedOwnedProviderResult::handoff
-> the existing Finish<A,E> -> the same graph bind/root continuation. Preflight
includes the concrete operation, actual environment/node origin and typed
provider-entry/D origin cell plus its output consumer, before graph factories.
The result can be produced by a preceding typed graph operation/bind; native
prefix construction does not defer the known output-cell reservation.

Only that sealed operation calls handoff. Its continuation and origin reservation
come from the same invocation; external users cannot construct Finish or the
operation fields. PreparedInvocation::run owns its OutputLease for the complete
drive (including suspension, cancellation and unwind), and drops an abandoned
Step before that output owner. Handoff first installs the actual entry/D under
the pre-admitted temporary consumer, THEN moves the original A once out of its
exclusive finally-settled entry and calls Finish with Exit::Success(A). A raw
provider panic follows the existing native graph Die conversion boundary, not a
new diagnostic policy. Cancellation before handoff extracts no value and uses
normal phased entry retirement.

The temporary extra consumer spans the private continuation call, then is dropped
while the live invocation still owns its output consumer. Only the entry/D and
originating node are installed into the origin chain: no same-root consumer/view
is stored inside its originating output or origin value. Successor Steps and
ultimately the unchanged root A/E remain protected by the invocation/output's
existing consumers until last destruction. This is not a public bare extraction
or unrestricted owned callback; full provider scheduling/fork/join integration
remains W05-14.

### Factory rollback owns D before user construction

PreparedSubmission::submit installs working_set in the uniquely owned,
unpublished entry BEFORE invoking the future factory. The state is empty after
previous retirement/at startup; no old D is dropped under its metadata gate.
A stable D pointer is obtained while holding that gate, then borrowed only for
the factory call with the lock released. No worker or receipt has access to the
unpublished entry, and the entry cannot move or retire during the call.

If the factory unwinds inside an active prepared drain, partially constructed
prepared capture destructors queue before submission rollback. D is no longer
a stack local: last EntryOwner queues the entry's existing phased retirement,
whose budget footer follows the queued partial-capture trees. If no drain was
active, those capture drops complete synchronously first. The factory's raw
panic still unwinds unchanged; no catch, sink/loss or allocation fallback is
introduced. Actual native/transitive allocation obligations remain D's contract.

This API does not add a spin loop, a second waiter, a timeout or a new
notification/diagnostic saturation policy. A refusal from `try_take` preserves
the view for a later correctly scheduled attempt; scheduler/fork/join integration
continues under W05-14. Native connection/transport settlement remains W08 work.

## Explicit accounting boundary

Compiler layouts account inline records, not arbitrary pointees. An external
Arc<R>, factory capture, native Future, custom L or returned native value still
needs its declared actual allocation/working-set contract. A native reserve hook
must attach any actual originating working-set/backing owner on which escaped
output depends; passing only a byte count does not fulfill that contract.
Provider transfer retains the actual D under the existing unsafe
ProviderWorkingSet obligations. Cloned/sliced payloads that leave an output
observer must themselves carry their originating backing/budget (for example
counted region views), or obey their explicitly named native ownership contract;
a borrowed callback cannot make arbitrary Clone implementations region-aware.
No pointer-sized representation is reported as full transitive accounting.

G26 dynamic imports and G27 provider/fork/join source wiring use this ownership
contract; SCHEDULING.md details the pre-reserved immutable origin-prefix branch
and D-before-source phased footer needed when an accepted job/result outlives
its awaiting root. The branch moves the old actual origin head, never captures
its new anchor/result, and stores no permanent self-output consumer. Arbitrary
native adapters, error-vector bounds and panic/trace allocation retain their
separate requirements. Existing panic conversion/Drop diagnostic behavior is preserved;
this source does not invent a sink, loss policy or W05-18 completion. Tests,
concurrency/pinning/drop/allocation/runtime validation remain deferred until the
full production milestone; library checks and this source proof are not runtime
certification.
