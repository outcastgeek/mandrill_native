# Generated scheduling and scoped children (W05-14 / G27)

This is production source for the generated Future/provider/fork/join family,
not runtime certification or completion of FULL-13. Ordinary APIs are unchanged.
The same computation grammar lowers before Effect erasure; all descriptions
return existing Step/Once/Pending transitions. Roots, imported runs, worker child
graphs and race observation steps call the same `poll_drive`. No ordinary spawn,
boxed erased job, task-count quota, second evaluator or global provider is used.

## Grammar and actual admission

Existing `effect_future` / `effect_try_future` native roles retain Future,
Result and Exit distinctions. Existing `effect_cpu`, `effect_try_cpu`,
`effect_blocking` and `effect_try_blocking` lower to selected prepared workers.
Their bodies and error mappers remain synchronous native Rust on that worker;
context modes receive the prepared FutureContext companion plus TraceContext.
Instrumentation is entered on each actual worker poll, never carried across
submission/Pending. Its opaque allocations, like native waker/platform costs,
remain explicitly named native contracts, not region-layout proofs.

`effect_prepare` scheduled mode accepts a native Environment/FutureContext
factory and a provider_kind. HasPreparedProvider selects the actual submitter
and transitive budget from R. A construction_key instead claims the actual
PreparedProviderConstruction supplied by G26 prebusiness construction. This
works for unpublished projected/Layer R without calling its getter early.
CPU/blocking roles also accept that syntax-only construction_key. Type/const
selections are passed outside inference helper items as typed PhantomData
arguments, preserving outer generic scope and callback argument inference.
Keys are syntax, not late runtime field locals or authority by themselves.

Admission recursively holds concrete operation, pinned future/child, result,
continuation, observer, native context and origin-retirement frames; actual
scope rows/vacancies; provider queue/result/cancellation/execution storage; D;
and the three diagnostic leases. D enters the private unpublished provider
entry during native preflight, before the outer graph factory. The ordinary
provider submit path uses that same D installation and handoff. Refusal is a
native MemoryError/ProviderRefusal before business factories, not business E.
Failed admission drops the entire acquired vector. Submission uses only those
reservations; a violated private handoff invariant is not a normal capacity path.
Neither native factories nor native Future pointees become arena-accounted
merely because their handles have compiler layouts.

## Canonical scope and affine fork authority

Existing effect_scope bodies lower to the canonical PreparedScopePool and shared
Closing adapter. A construction field supplies the same finite Construction P
used by Layer: the body receives the actual scope and P's affine permits. Tuple
construction shapes compose. The complete body and protected close tree and P's
real entry promises precede the scope callback. The result remains the body's A;
there is no Sync/Clone bound added to A and no replacement error wrapper.

A ScopePermit's fork(program) or fork_on(program) consumes its actual retained
entry promise; the prepared fork mode exposes the same operation with an
explicit provider_kind/construction_key. There is no independently selected
admission scope, later target comparison, substitution, bank replenishment or
late checkout. A host/native G26 constructor can reserve the permit against an
actual foreign scope before business. The source program can likewise be a
real admitted G26 import. Every provider/foreign source keeps its own admitted
R, context and origins. No projected or foreign target family is excluded.

Each child runs an admitted ScopedProgram on its selected provider: its own
canonical scope closes before its result continuation. The owning target scope
receives an armed Submitting finalizer BEFORE queue handoff. A competing closer
requests child cancellation and waits for receipt publication; it cannot treat a
still-submitting job as an empty, settled record. A closed target before handoff
runs no child factory/submission and reports existing ScopeRegistrationClosed.
Accepted jobs remain provider-owned until actual return/destruction/settlement.
Abandonment requests cancellation, never asserts settlement or makes occupied
storage available. Uncooperative work remains physically charged.

## Join, cancellation and losing branches

PreparedFiber is a control handle; its consuming join description returns the
original child A/E, including non-Clone A and E = !. Checked sharing clones only
actual control owners. Only one actor holds the private receipt at a time.
Additional joins wait on pre-reserved intrusive observer records, not Event
allocation or overwritten provider waiters. An interrupted/dropped observer
publishes the still-unobserved receipt back as Running BEFORE unlink can destroy
a native waker; an unwind guard guarantees notification even if that destruction
panics. Successful-observation unlink also has stack-only receipt rollback if
native destruction unwinds before handoff. Dropping a pending canonical cleanup
driver likewise returns its unobserved receipt from Cleaning to Running and
notifies observers, rather than leaving an ownerless Cleaning state. The actual
task/entry/D remains retained through these native operations. These paths do not
allocate, clone owners late, swallow panic or run user Drop under task gates.
It does not cancel an unrelated foreign child merely because the
observing caller was cancelled. Canonical owning-scope cleanup requests child
cancellation and waits for actual settlement. Cleanup serializes with a join
already observing, and preserves observed/unobserved outcome semantics.

Observer self anchors are acquired before factories. The task gate protects
links; clone/drop/wake happen outside it. Notification advances/takes each node
before native wake, and an unwind-resuming guard drains remaining notifications
on one panic. Detached queued records remain anchored until notification ends.
Live observers reuse their pre-held anchor on another wait. There is no late
owner clone. Completion notification is guarded even if a result continuation
unwinds. Existing ordinary/double-panic behavior is retained; no diagnostic sink,
saturation or loss policy is chosen.

Existing race syntax/methods lower to two admitted provider children and one
canonical owning scope. Explicit race mode permits distinct left/right provider
kinds and construction keys (affine preparations require distinct sites).
Selection occurs at the child continuation, not whichever queue entry happens
to retire first. A driver lost before that continuation can also select. The
loser is cancelled and BOTH private joins are driven to actual final settlement
before returning. Winner protection flags and the original Exit are preserved;
loser primary/cleanup treatment reuses canonical outcome functions, including
native driver loss and caught loser-value destruction. No E is cloned.

## Actual result/source ownership, without self-consumer cycles

`poll_final` observes future destruction, result publication, execution-owner
and queue retirement. The worker retains entry/D through final notification.
Private graph receipts expose no public result aliases. Handoff installs the
actual entry/D in the receiving root before moving its unchanged Exit into the
existing Finish. A source child node is retained by its actual origin footer.

A job may outlive the awaiting root or leave an unobserved result behind. A raw
context/row reference alone is insufficient: that root's last output consumer
can detach its environment/recipe/prior-result origins. Each scheduled job
therefore pre-reserves an immutable origin-prefix branch and D-retirement frame.
At handoff the old origin head MOVES into that shared prefix; a pre-held root
anchor and the provider retirement footer share it. The prefix never captures
its new anchor, later result attachment or root output value. No permanent
counted output consumer is stored in a root's own result or origin records.

Final entry retirement first destroys future/result/waiter descendants. Its
pre-reserved retained frame then destroys actual D and D's descendants; only
then does its footer release the old origin prefix and actual child-node owner.
Thus environment, startup recipe and preceding result backing remain alive
through accepted work AND last result destruction, including abandonment,
foreign-root joins, active intrusive drains and one unwind. Joining attaches D
and actual task metadata before those owners can disappear. The private task
cell also retains a pre-admitted, non-observing entry lease, protecting D through
waiter operations concurrent with another join/closer. That guard cannot inspect
or extract result data.

## Deliberately unchanged boundaries

Canonical error aggregation still uses ordinary outcome Vec storage; physically
bounded cleanup-error accumulation is W05-17, not silently certified here.
Defect panic conversion, nonempty trace/instrumentation and native allocation
contracts retain their unresolved/separate requirements. W05-18 policy is not
selected. Arbitrary native Clone/slice/transport escape still requires actual
independent backing or the explicit owned-transfer contract in OUTPUT_OWNERS.md.
No server/cache/SQL/timer row is implemented by this source. Macro consumer,
concurrency, cancellation, pin/drop, allocation and runtime validation remain
globally deferred until FULL-13; library checks are not substitutes.
