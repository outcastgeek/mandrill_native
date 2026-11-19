# Checked dynamic imports and foreign construction (W05-13)

This is the checked prepared alternative to importing an arbitrary already-erased
ordinary Effect recipe. Ordinary Effect APIs remain unchanged. Their Arc/Box and
native dependency allocations are not retroactively certified by a byte count or
by wrapping their interpreter in a prepared Future. Here dynamic selection erases
an **actually admitted PreparedInvocation<A,E>**, whose Step/Once/Pending, context,
recipe, scope, output and native working-set owners already exist. Both concrete
native-root plans and generated-graph plans produce this same invocation type.
The dynamic source's concrete graph/callback layouts need not be known by its
importing continuation. No ordinary-erased/System kernel fallback is used.

## Finite physical construction before callbacks

`construction_site::<T, KEY>()` declares one typed site; unit and nested tuples
form finite shapes. Keys are explicit per-invocation configuration, not global
provider IDs. `effect_prepare! { graph; ... construction: shape, reserve: ... }`
produces a ConstructedGraphPlan. The native reserve callback receives the original
available Arc<R> and matching typed vacancies. It returns Result<(), MemoryError>
before business E exists, including E = !. Ordinary plans without construction
retain their original interface and behavior.

Preparation allocates one fixed registry lane per run and exactly its declared
site capacity in the managed region. Demand includes lane/array/callback storage,
each compiler-layout typed site record and the callback's actual retained recipe
owner, with checked arithmetic. Region allocator headers/padding remain visible
in memory usage rather than being inferred from these payload formulas. Execution
never grows the vector or allocates a HashMap, Box, Arc control block or fallback
registry. Direct provider preparations also use a real typed prepared frame,
not a large inline enum or a heap Box introduced to satisfy a lint.

Admission order is:

1. Acquire the outer root/output, recipe, start/store/terminal frames and the
   construction lane. Reserve every typed vacancy's stable frame, actual node
   footer and output consumer.
2. Recursively reserve the COMPLETE outer generated tree, including import
   drivers, cancellation listeners, native payload/continuation/output transfer
   cells, and actual environment/node origins. A dynamic site is claimed once
   by its actual typed counted record, even though T is not installed yet.
3. Run the explicit native pre-business callback. It constructs/checks out real
   foreign targets into the already-owned vacancies: for example a complete
   source PreparedInvocation, or actual PreparedProviderConstruction containing
   provider queue/result/cancellation preparation, D and diagnostic leases.
4. Verify that every declared vacancy is installed before invoking any outer
   description/projector/Layer-build/native-business factory.

No projector runs early to discover capabilities. There is no late provider
checkout hidden after a business prefix and no per-type global provider. Missing,
duplicate, already-claimed, type-mismatched or unbound sites refuse explicitly
before business callbacks. A registry key alone grants no authority: the entry
holds the actual shared typed region frame and originating node; TypeId protects
the private pointer/accessor pairing, and an affine claim prevents alias use.
The owning root/lane/slot generation stays live through all references, so a
stale numeric key cannot upgrade into another run's resource.

ConstructionVacancy::install consumes the vacancy and moves T once. It never
drops/replaces an old T under a metadata gate. The native callback is synchronous;
its ordinary unwind behavior is preserved (W05-19 is not silently completed).
The callback's real captured owner is retained through output consumption; the
runner may be dropped after admission without invalidating captures used by
constructed resources. Native callback code must acquire its own real declared
foreign/environment working set BEFORE expensive transitive construction; a
reserved inline T cell is not an accounting proof for arbitrary T pointees.

## Dynamic run import in the existing computation grammar

`effect_prepare! { import; key: KEY }` is a normal typed Description inside the
existing effect/bind program. Its matching construction site is
`ConstructionSite<PreparedInvocation<A,E>, KEY>`. The native pre-business callback
selects and fully admits the actual source run, then installs it. Selection can
vary per invocation without changing the importing graph's static Description
type, while each selected source admission owns its complete physical demand.
Different target recipes, native futures and generated graph shapes therefore
share a real checked dynamic boundary, not a refusal-only opaque-expression API.

The importer claims the actual source record during preflight and moves its
invocation only when that operation executes. Its driver invokes the SAME common
poll_drive routine used by ordinary and prepared roots; it does not duplicate the
Step interpreter. The foreign run retains its explicitly admitted context/R/scope
boundary and clock state. A pre-reserved parent cancellation observer requests
source cancellation, and the importer continues driving the source through its
actual completion/finalization; it does not mistake a request for settlement.
No async cleanup is attempted from Drop. Abandonment drops the source Step before
its own output owner, with the existing prepared destruction mechanism retaining
accepted native work independently until actual settlement.

On Ready, the source's original Exit<A,E> is transferred only through a private
engine continuation. The actual foreign PreparedOutput (with recipe, nodes,
environment/resource/provider origins and output consumer) is installed into a
pre-reserved outer origin cell BEFORE moving the source Exit. The importing live
invocation holds its output through drive/publication; the temporary transfer
consumer is not inserted into its own root outcome. A/E and all five Exit channel
families remain unchanged, without Clone or a wrapper business type. External
public output observers retain their existing borrowed/ownership-coupled rules.
Imported source scope cleanup remains the source's canonical Scope close engine.
This explicit foreign-run boundary is not claimed to be a transparent rewrite
of ordinary Effect::run or arbitrary NativeContext allocations.

## Foreign provider authority for an unpublished projected R

`PreparedProviderConstruction<A,E,D>::reserve` obtains an ACTUAL typed physical
provider submission before its budget callback, then owns D plus the pre-admitted
diagnostic events. It can be installed into a declared construction site without
reading the not-yet-published projected environment. `ConstructionProvider<K,KEY>`
is an explicit class selector for that per-invocation owned site, not an ambient
provider handle. The scheduled descriptor's preflight claims that actual typed
record and reserves its payload/continuation/output transfer working set.

The `effect_prepare! { scheduled; provider_kind: K, construction_key: KEY,
body: ... }` companion uses this explicit authority. Its native body gets the
same counted Environment/FutureContext companion as native prepared leaves and
returns a concrete Future<Output = Exit<A,E>>. That Future and its lazy factory
live in a compiler-layout region frame and run on the selected prepared worker,
with the configured reactor/native worker entry. A failed native preflight or
uninstalled site runs no projected-R/native-business callback. Without a declared
construction key, direct R selection is valid only for an already available R;
it does not pretend an unpublished projected value can supply authority.

The native callback selects the real target through available outer R services
and passes its actual owner into the typed vacancy; no provider is selected by a
bare TypeId or static type-global pointer. A construction-selected provider is an
explicit foreign authority, not an implicit assertion that a later R getter
selected the same instance. Thread/native allocation/diagnostic obligations remain
those of the actual provider and D. Counts/pointer layouts do not prove arbitrary
working-set or allocator overhead.

G27 scheduling now uses this foreign-provider authority for scheduled native
jobs and scoped fork/join/race, without early projection or late provider checkout;
see SCHEDULING.md for the production source contract and deferred certification.
Awaited provider observation waits
for actual execution/queue settlement before private result transfer; the worker
holds entry/D through its single final notification and never writes result data
after that flag. Its private graph receipt protocol exposes no public result
alias. Publication observation remains distinguishable from actual settlement;
no new spin loop, timeout, diagnostic sink or saturation/loss policy is selected.

## Rollback, last ownership and remaining certification

A failure at any outer/source/provider acquisition unwinds the whole accepted
construction vector before business construction. Unused or abandoned site values
own their real target and use existing prepared payload/descendant/footer
retirement; they are not naked IDs returned to a pool early. Last output release
takes the lane out of its root gate and explicitly clears every site BEFORE
returning the reusable lane (SlotPool return alone would preserve old values).
This breaks root -> registry -> record -> originating-node -> root links outside
metadata locks. Actual imported/provider backing survives transfer through the
outer output's last mapped/shared/Future consumer via the accepted G29 mechanism.
Forgotten native ownership can retain capacity, never authorize unsafe recycling.

PreparedReuse uses startup-prepared cells; BoundedAllocation may additionally use
its declared fallible native/region working set. Neither silently grows managed
backing. Native constructors/captures/opaque payloads still require actual named
allocation contracts, including diagnostic and waker storage. No all-family,
whole-process, zero-native-allocation, arbitrary ordinary-erased-Effect or runtime
certification claim follows from this source boundary. Other graph operation,
error/diagnostic/trace and provider scheduling rows retain their own requirements.
Tests, fixtures, probes, builds and runtime/concurrency/allocation measurement
remain deferred under the FULL-13 production-first instruction.
