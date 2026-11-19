# `effect_cms` — Rust port of Go `contentmgmt2`

Rust port implementation specified by
`scratch/plans/contentmgmt2-rust-effectful/IMPLEMENTATION_PLAN.md`. This file
records source/owner mappings and the implementation evidence for each completed
owner, including the WP20 public composition. It adds no requirements,
API designs or product behavior.

## Boundaries in force

- **Consumer-owned CRUD.** Consumers implement every database read and write.
  The CMS invokes supplied capabilities and owns no backend, SQL/SDK statements,
  schema, migration catalog, pool, connection, database lifecycle or backend
  selection. Identity persistence stays behind consumer-supplied identity
  services.
- **Effects.** Effectful behavior uses the workspace `Effect<A, E, R>` model and
  the required effectful macros; no imperative core wrapped once at the edge.
- **Architecture.** The non-actor owner/capability rules in
  `scratch/docs/architecture/rust-non-actor` are mandatory. Go `internal`
  privacy/facade layers are not reproduced as Rust owners.
- **HTML.** Every HTML-producing path uses Maud; manual HTML construction is
  forbidden, including Datastar/SSE fragments.
- **Sequencing.** No test writing or execution until 100% of implementation code
  is in place. The current implementation phase also defers routine builds, checks and lints.

## Prepared CMS consumers

The original service constructors and request helpers remain the ordinary path.
For a prepared application, calculate the common CMS payload and slot-table demand
with `PreparedCmsMemory::storage_bytes(CmsMemoryCapacity)`. Include that demand
and allocator overhead in a region of the application's existing memory plan.
Call `PreparedCmsMemory::prepare` with that region before sealing the plan.
Definition records, command and access work, and optional
`PreparedCmsContent` workspaces have separate sizes. Include the selected owners
in the same application plan and attach content workspaces with `with_content`.

Keep the resulting `Arc<PreparedCmsMemory>` in application services and implement
`HasCmsMemory`. `effect_server::RequestEnv` forwards that capability while
retaining the current request. `read_prepared_cms_form` uses both capabilities to
read retained decoded fields. Query, command and validation services use their
explicit prepared inputs. Build those inputs with `PreparedDefinitionServices`,
then use `new_prepared_definition_service_set` or
`new_prepared_definition_runtime_factory` with the selected
`DefinitionRegistryStorage`. An explicitly supplied registry keeps its authority
over declarations. These constructors preserve the application's validation,
access, persistence and provider callbacks.

Configure prepared body and reply buffers on Server endpoints that need them.
`ResourcePageRenderer::reply_prepared` selects the existing Maud page renderer.
`definition_reply_prepared` claims the output before its selected list, detail or
dashboard query and page producer. `reply_prepared_with` accepts the application's
own effectful page-model source and applies the same output lifetime.
`render_prepared_maud_reply` invokes a consumer renderer after taking the
response's output buffer. For direct encoding into a caller-owned buffer,
`write_maud_markup` appends native markup and preserves the output prefix on
refusal. These adapters account for the native Maud string and the full output
slot while both are live. Renderers own any intermediate work before returning
markup. `prepared_cms_search_stream` writes existing Maud components and
prepared signal text into leased Datastar frames from the CMS capability.

`LocalizedAuthoringService::with_prepared_memory` selects the shared decoded form
owner for the existing authoring handler. Where a Gemini provider is selected,
`PreparedCmsContent::gemini_translator` attaches the prepared provider workspace
to the application's existing translator.

Media runtimes expose `list_attachment_items_prepared`,
`preview_attachment_items_from_request_prepared` and
`replace_attachments_from_request_prepared` for applications that use selected
attachment storage. Validation and compatible media readers share one admitted
form snapshot, preserving repeated field values and the request's single body
claim. An incompatible storage owner or form policy yields a typed conflict.

Map memory and capacity refusals through the application's typed error callbacks.
Keep form, validation, draft and output handles only as long as a consumer needs
their backing. A published response body or event holds its lease through
transport. Any native provider and consumer memory belongs to its existing
owner.

## Baseline and preserved work

- Local workspace: branch `main`, HEAD `8fb68c23b9b18acd3853f92b6b34f6b318e94abb`.
- Pre-existing modified paths (`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`,
  `.zbscanhelper.zig`, `AGENTS.md`) and the untracked `crates/effect_cms`
  scaffold predate this work. They are preserved; no worktree cleanup, stage or
  revert was performed.
- Reference source: Go `contentmgmt2` at HEAD `93d35b54a6c2c9e4675bbf3e730dff5949b5632d`
  plus the inspected dirty W19–W22 relation work (source inventory §1.1). The Go
  checkout is reference material, not a build dependency.

The pre-existing `add` function and its test are preserved verbatim as a crate
scaffold; the root owner record lists `add` under `preserved_items`. It is not a
CMS contract. The only change to it is a `#[must_use]` attribute required by the
workspace's denied `clippy::must_use_candidate` lint.

## Current production source

| Module | Responsibility | Status |
| --- | --- | --- |
| `effect_cms` (`src/lib.rs`) | Composition root and root of the frozen owner graph | eleven-service composition implemented (WP20); compiler/runtime verification deferred |
| `effect_cms::definitions` | Resource vocabulary, model, builders, registry and relation declarations | implemented (WP03 definitions substep) |
| `effect_cms::localization` | Language normalization, localized drafts/selection, description JSON and the translation port | WP03-L4 implementation completed; compilation and static review passed; tests deferred |
| `add` | Pre-existing scaffold | preserved, not a CMS contract |

## WP01 — source snapshot and owner mapping

Complete inspection evidence remains in
[`SOURCE_INVENTORY.md`](../../scratch/plans/contentmgmt2-rust-effectful/SOURCE_INVENTORY.md):
§3.1 frozen owner topology, §4.1 W1–W22, §4.3 additional behavior families,
§5 public contract compatibility, §6 all 76 test files, §8 architecture
obligations. Plan §4 repeats the reference owners; plan §7 holds the WP mapping.
This file adds only the source-to-Rust owner mapping and first-boundary evidence.

### Intended source-owner → Rust module mapping

Module paths are derived from the reference owner graph (source inventory §3.1)
and plan §4; they are not new API designs. Only modules recorded in
`Registry::current()` are registered; every row marked `no-started` is **not**
implemented and is not inventory evidence.

| Reference owner | Intended Rust module | Semantic parent | WP | Status |
| --- | --- | --- | --- | --- |
| `.` (composition root) | `effect_cms` | — | WP20 | root composition implemented; compiler/runtime verification deferred |
| `internal/definitions` | `definitions` | crate root | WP03 | implemented: definitions vocabulary/model/builders/registry/relations; temporal still `no-started` |
| `internal/localization` | `localization` | crate root | WP03 | implemented: normalization/drafts/JSON/translation and byte-preserving form/query/multipart collection; tests deferred |
| `internal/temporal` | `temporal` | crate root | WP03 | implemented/check-green: temporal values, byte-preserving request transport and wide UNIX formatting; native future-zone caveat below |
| `internal/attachments` | `attachments` | crate root | WP03 | implemented: item/surface/manager records, role constants and the complete input algebra over a decoded form view; no store; native byte-preserving Effect request adapter (A2, recorded below) |
| `internal/content` | `content` | crate root | WP04 | source-reviewed implementation: formats/payloads/assets/text/shortcodes and sanitation policy; current correction/check evidence below |
| `internal/access` | `access` | crate root | WP05 | source-reviewed guard/policy/response implementation, compiler check evidence below |
| `internal/queries` | `queries` | crate root | WP06 | source-reviewed admission/filter/search/selection implementation; byte-preserving contracts and current check evidence below |
| `internal/validation` | `validation` | crate root | WP07 | implementation complete; checks only |
| `internal/commands` | `commands` | crate root | WP08 | existing implementation source-reviewed |
| `internal/mutations` | `mutations` | crate root | WP09 | implementation source-reviewed |
| `internal/forms` | `forms` | crate root | WP10 | implementation source-reviewed; runtime validation deferred |
| `internal/routing` | `routing` | crate root | WP11 | implementation source-reviewed |
| `internal/media` | `media` | crate root | WP12 | foundation implementation source-reviewed; runtime validation deferred |
| `internal/media/internal/assets` | `media::assets` | `media` | WP12 | foundation implementation source-reviewed; runtime validation deferred |
| `internal/media/internal/contentmedia` | `media::contentmedia` | `media` | WP12 | foundation implementation source-reviewed; runtime validation deferred |
| `internal/media/internal/surface` | `media::surface` | `media` | WP12 | foundation implementation source-reviewed; runtime validation deferred |
| `internal/media/internal/attachmentflow` | `media::attachmentflow` | `media` | WP13 | attachment workflow source-reviewed; runtime validation deferred |
| `internal/media/internal/runtime` | `media::runtime` | `media` | WP14 | runtime/management producers source-reviewed; concrete presentation tracked in WP15 |
| `internal/media/internal/management` | `media::management` | `media` | WP14 | runtime/management producers source-reviewed; concrete presentation tracked in WP15 |
| `internal/media/internal/flows` | `media::flows` | `media` | WP15 | source implementation and review complete; post-code runtime validation deferred |
| `internal/media/internal/demo` | `media::demo` | `media` | WP15 | source implementation and review complete; post-code runtime validation deferred |
| `internal/media/internal/presentation` | `media::presentation` | `media` | WP15 | source implementation and review complete; post-code runtime validation deferred |
| `internal/authoring` | `authoring` | crate root | WP16 | implementation complete; bounded source review, runtime verification deferred |
| `internal/rendering` | `rendering` | crate root | WP17 | existing list/detail producers reused; full dashboard composition and publication implemented |
| `internal/dashboards` | `dashboards` | crate root | WP17 | canonical data/builders reused; card/page/query-aware service implemented |
| `internal/resourcepages` | `resourcepages` | crate root | WP17 | Maud templates, native renderer/layout and ordered SSE publication implemented |
| `internal/seo` | `seo` | crate root | WP18 | head/structured/sitemap publication reused; native origin, formatting and effect projection complete |

Relations (W19–W22) have no standalone source owner: they live in
`definitions`, `queries`, `validation`, `mutations` and `forms` (WP19). The
eleven source services (plan §4, inventory §5.2) belong to the crate-root
composition surface (WP20); no placeholder module is declared for them.

### Next owner boundary (WP03) source evidence

Targeted read-only verification, Go HEAD above. `definitions`, `localization` and
`attachments` and `temporal` are ported (see their sections below).

| Owner | Reference files (observed line counts) | Public contract evidence |
| --- | --- | --- |
| `definitions` | `interfaces.go` (216), `platform.go` (109), `internal/builders.go` (122), `internal/model.go` (202), `internal/registry.go` (74), `internal/relation_validation.go` (98), `internal/relations.go` (119), `internal/vocabulary.go` (210) | `Registry` (`Definition`, `Definitions`); re-exported definition/route/filter/section/field types; `ResourceDefinition` model (inventory §5.3); vocabulary constants; constructors `NewResourceRelationDescriptor`, `ResolveResourceRelation`, `NewDefinitionRegistry`, `FindResourceDefinition`, `NewResourceFormDefinition`, `NewResourceFormSection`, `NewTitledResourceFormSection`, `NewResourceListDefinition`, `NewFeaturedResourceListDefinition`, `NewResultsResourceListDefinition`, `NewResourceDetailDefinition`, `NewResourceDashboardDefinition`, `NormalizeResourceDefinition`, `UnifiedManagementRouteSet`, `ResourceUsesUnifiedManagementRoutes`, `ResourceFormFieldStateKey` |
| `localization` | `interfaces.go` (27), `platform.go` (128), `internal/drafts.go` (261), `internal/normalizer.go` (61) | `LocalizedContentDraft`, `LocalizedTextDraft`, `LocalizedRowOptions[T]`, `ContractLanguageNormalizerOptions`, `TranslateKeyFunc`, localized field-key constants; `LocalizedFieldSuffix`, `LocalizedFieldName`, draft collection/clone/from-rows/find/metadata helpers, description JSON round-trip, completeness, `NewContractLanguageNormalizer`, `NormalizeAuthoringLanguage` |
| `temporal` | `interfaces.go` (33), `platform.go` (136), `internal/datetime.go` (262), `internal/duration.go` (91) | `DateTimeValidationCode`, message keys, translator; valid/required/invalid-date/invalid-zone and today/upcoming/past constants; `DateTimeDisplayLayouts`; `NormalizeDateTimeFormValues*`, `DateTimeValidationMessageKey`, date/time/ISO/localized formatters, display parsers, timezone normalization, `LocalDateInTimeZone`, `TemporalStatus`/`TemporalStatusAt`, duration normalization/request/format helpers |
| `attachments` | `interfaces.go` (30), `platform.go` (81), `internal/inputs.go` (324), `internal/items.go` (76) | `MediaAttachmentInput`, `MediaAttachmentFormItem`, `MediaAttachmentStoreRecord`, `MediaSurfaceMounts`, `MediaAttachmentManagerProps`, `MediaAttachmentRoleOptions`, role constants (hero/gallery/inline/download); collection, item conversion, dedupe, role normalization, asset-ID attachment, promote/remove, index lookup, selected IDs, `MediaShortcode`, `FormValuesWithFallback` |

Behavioral obligations for these owners remain plan §5 and inventory §§4–5; the
source tests for them are inventory §6 obligations deferred to WP21. The
`localization` request collectors consume the local `effect`/`effect_server` APIs;
the exact APIs used are recorded in the localization section below.

## WP02 — architecture enforcement

The user rejected and deleted the architecture-scanner subsystem. Its source,
owner/module metadata, re-exports and direct `syn` dependency are removed; the
non-actor ownership/privacy/capability rules stay applied in typed domain code and
no replacement runtime-inspection framework is introduced. The ledger records the
deletion and the remaining production gaps.

## WP03 — definitions owner (first owner substep)

Source: `internal/definitions/{interfaces.go, platform.go}` and
`internal/definitions/internal/{vocabulary,model,builders,registry,relations,relation_validation}.go`
at Go HEAD `93d35b54a` plus the inspected dirty relation work. Read-only; the Go
checkout is reference material.

### Source → Rust mapping

| Reference file | Rust module | Content |
| --- | --- | --- |
| `interfaces.go`, `platform.go` | `definitions/mod.rs` (owner API) | `Registry` trait, the 16 public constructors/builders/normalizers, enumerated re-exports of every definition type and vocabulary value |
| `internal/vocabulary.go` | `definitions/vocabulary` | 16 open named-string types and all 105 reference constants, including `ResourceManagementRouteModeSplit = ""` and the four canonical path parameters |
| `internal/model.go` | `definitions/model` | Complete `ResourceDefinition` model (22 types, every reference field) and `ResourceFormFieldStateKey` |
| `internal/builders.go` | `definitions/builders` | All builders, `NormalizeResourceDefinition`, `UnifiedManagementRouteSet`, `ResourceUsesUnifiedManagementRoutes`, `firstNonBlank` |
| `internal/registry.go` | `definitions/registry` | Concrete registry, empty-kind skip, insertion order with duplicates, unknown-kind error |
| `internal/relations.go` | `definitions/relations` | Relation declaration types, checked descriptor, detached resolution |
| `internal/relation_validation.go` | `definitions/relation_validation` | Cardinality/target/key/operation/through/bound/save-mode/removal checks in reference precedence |
| — | `definitions/sharing` | Shared collection handles implementing the reference's reference semantics |

The Go `internal` package is technical privacy scaffolding, not a semantic child
owner: `definitions` is one owner with seven private implementation modules (plan
§4). The modules are declared privately, so no other owner can name an
implementation path. No empty future owner or stub module was created.

### Preserved vocabulary

Named strings are **open** values, not closed enums: kinds, actions, filter keys,
field keys and section keys accept consumer-defined values, and no owner code
validates a vocabulary value. Only the declared relation cardinality, operation,
key-type and save-mode sets are matched, exactly as the reference matches them.
The empty split mode (`""`) is preserved as the zero value and as an explicit
constant. Spellings were verified mechanically against the Go constant set (105 of
105 constants, identical names by conversion and identical values).

### Preserved registry and sharing semantics

- Empty kind skipped; no trimming of the kind.
- Duplicate kinds keep their insertion position and each occurrence resolves to the
  last stored definition; the returned outer list is independent per call.
- The reference's nested reference semantics are preserved with owner-local
  `SharedList`/`SharedMap` handles rather than a deep copy: an **in-place element**
  mutation (a filter field such as `Param`/`DefaultValue`, or an aux route value)
  made by the caller after registration is observable through lookup, one made
  through a lookup result is observable in the caller's collection and in later
  lookups, and the caller's page callback filter mutation therefore stays observable
  downstream. `Arc<RwLock<…>>` keeps the handles `Send + Sync` without `unsafe`;
  each accessor callback runs while the lock is held; no global, locator or runtime
  discovery exists.
- `SharedList` models a Go slice header — a shared backing store **plus a per-handle
  length and capacity**: cloning copies the header (same store, that handle's length
  and capacity), an element mutation is shared in place, an append inside the
  capacity writes at that handle's own length so no other handle's length changes
  (a longer handle whose view reaches the slot observes the new value there), and an
  append that exhausts the capacity copies this handle's visible prefix into a fresh
  store and moves this handle to it, leaving older handles on the previous store —
  the reference's reallocation independence. Nested collection handles inside copied
  elements stay shared, exactly as copying a Go element copies its nested headers.
  Builders take owned vectors, so their outer collections are detached. Three details
  remain deliberately unclaimed (module docs + ledger): the allocator size-class
  rounding of Go's capacity growth, spare capacity a source slice may have had at the
  wrap point (a wrapped collection is modelled as covering its whole array), and
  mid-array headers. Exact byte-for-byte native allocation parity is **not** claimed;
  the sharing behavior itself is recorded as deferred test obligations below.
- The reference's absent-registry branch of `FindResourceDefinition` is preserved:
  `find_resource_definition` takes an explicit optional borrowed registry and yields
  no definition when it is absent. This is a helper's optional input, not an
  optional required capability: the owner's required production registry remains a
  required typed input (`new_definition_registry` constructs it, present lookups
  borrow it).
- Relation declarations are the deliberate exception: detached on normalization,
  on construction and on every lookup, including key components, targets' keys,
  through steps, operations, editor columns and minimum/maximum bounds.

### Preserved relation contract

`ResourceRelationDescriptor` keeps a private representation with no public field
and no unchecked constructor; the zero descriptor grants no operation. Only the
reference's checks are applied, in the reference's precedence and with its message
text: trimmed source/name/cardinality, source and relation identity required,
explicit cardinality from the fixed set, one of single target or finite tagged
targets, discriminator for finite targets, declared keys when no through path,
inverse declaration or finite targets exist, distinct source/target key fields
with a supported scalar type, distinct tags/targets/keys, the supported operation
set with no repetitions, non-blank through steps, nonnegative ordered bounds,
patch or replacement save mode, and a declared removal operation. Arbitrary
operation strings and through steps are not trimmed, and relation resolution
trims only the compared names.

### Effects

The owner contains no effectful operation: definitions are caller-supplied data
and the owner performs no I/O, so nothing here is written as an imperative core
awaiting an `Effect` wrapper. Wrapping `Registry` lookups or the pure builders in
`Effect<A, E, R>` inside this owner would be an effectless stand-in. Consumers
(queries, validation, commands, forms, rendering) lift these pure functions with
the required macros at their own workflow boundary, which is where `R`, `E` and
interruption semantics actually exist. No CMS database, SQL, schema, migration,
pool or backend exists or is required by this owner.

### Inventory registration

The definitions owner is registered in `Registry::current()` with its real
compiler-checked identity (`module_path!()`, `file!()`, `include_str!`), parent
`effect_cms`, its 17 declared API items, seven private implementation modules with
their own compiler-checked identities and source, and five deferred test obligations
(`source:definition_builders_test.go`, `source:registry_alias_compatibility_test.go`
and the three Rust-side proofs `shared_list_shared_element_and_per_handle_length`,
`shared_map_mutation_visibility`, `find_resource_definition_absent_registry`).
The root record now lists `definitions` as its child and as an API item. No
publication or capability record was added: the owner publishes no
immediate-child-owner contract (it exposes its own API and, at crate scope, only
private implementation identities) and holds no capability. No `Effect`, web, OTel
or SQL dependency was added, so `crates/effect_cms/Cargo.toml` and `Cargo.lock`
are unchanged by this substep.

## Permitted validation performed at the WP02 checkpoint

| Command | Result |
| --- | --- |
| `cargo check -p effect_cms --lib` | passed |
| `cargo clippy -p effect_cms --lib` | passed (no warnings) |
| `cargo fmt -p effect_cms -- --check` | passed |

No test, doctest, `--all-targets` or workspace-wide build was run; no validator
was executed. Disk: 63 GiB available on the volume (93% used) with `target/` at
9.5 GiB before this correction and unchanged after it. The only dependency
change is the `effect_cms` production dependency on the workspace's existing
`syn 3.0.10` (`features = ["full", "visit"]`), which is already resolved for
`effect_macros`; `Cargo.lock` gained only the `effect_cms` package entry naming
that dependency. No toolchain or other dependency change was made and no
cleanup was needed.

## Permitted validation performed at the WP03 definitions checkpoint

| Command | Result |
| --- | --- |
| `cargo check -p effect_cms --lib` | passed |
| `cargo clippy -p effect_cms --lib` | passed (no warnings) |
| `cargo fmt -p effect_cms -- --check` | passed |

No test, doctest, `--all-targets` or workspace-wide build was run, and the
architecture validators were **not** executed: the definitions owner record is
check-compiled production inventory only. Source tests were read for behaviour
only and neither executed nor rewritten; no test, fixture, doctest or test-only
helper was written. Disk: 63 GiB available (93% used) and `target/` 9.5 GiB before
the substep, 62 GiB available and `target/` 9.5 GiB after it (the 1 GiB `df`
difference is not attributable to the measured `effect_cms` artifacts, which grew
from about 176 KiB to about 268 KiB); only incremental build artifacts for the
affected package were produced, nothing was deleted and no new target tree was
created. `crates/effect_cms/Cargo.toml` and `Cargo.lock` are untouched by this
substep; the pre-existing modified/untracked paths listed above are preserved and
nothing was staged or committed.

## WP03 correction — definitions compatibility review findings

Two source-grounded review findings were corrected inside this same owner. The
watchdog's "move after insert" allegation against `registry.rs` was stale: the
actual source takes `let kind = definition.kind.clone();` before moving the
definition into the map, the crate checks clean, and nothing in that code was
changed here. The package format check also passes (see below).

### Finding 1 — the reference's absent-registry branch is preserved

The reference declares `FindResourceDefinition(resources Registry, kind)` and
returns `(zero, false)` when `resources` is nil. The earlier Rust form took a
required `&R` and claimed that absence had no Rust counterpart, which removed an
observable source branch. `find_resource_definition` now takes an explicit optional
borrowed registry (`Option<&R>` with `R: Registry + ?Sized`) and yields no
definition when it is absent, preserving the branch without inventing behaviour.
Absence is a property of this helper's input only: the owner's required production
registry stays a required typed input (`new_definition_registry` constructs it, and
present lookups borrow it), so no required dependency became optional. The doc
comment states the absent-call spelling
(`find_resource_definition::<DefinitionRegistry>(None, &kind)`) and the present-call
form for concrete and `dyn` registries, and the obligation is recorded as a
deferred WP21 proof (`rust:find_resource_definition_absent_registry`). No in-crate
caller of either form exists yet — no consumer owner has been ported — so the
absent case is exercised only by that deferred proof.

### Finding 2 — the list handle no longer implies shared length or reallocation

A Go slice value is a header (backing-array pointer, length, capacity); copying the
containing value copies the header. The previous `Arc<RwLock<Vec<T>>>` handle shared
the whole vector, including its length, so an append through one handle would have
silently become visible through every other handle. `SharedList` now holds shared
backing storage **plus a per-handle length**:

- `clone` copies the header — same backing storage, that handle's length;
- `with_element_mut` performs the required in-place element mutation (filter
  `Param`/`DefaultValue`, aux route values) visible through every handle, and cannot
  change any handle's length;
- `push` writes at the issuing handle's own length and extends only that handle, so
  no other handle's length changes; a copy can, however, observe the written value,
  because the write lands in the shared backing storage and overwrites the element a
  longer handle's view already reaches (the documentation overstated this before the
  correction below);
- `with`/`to_vec`/`len`/`is_empty` expose only this handle's view.

Builders take owned vectors and construct detached outer collections; relation
declarations also remain detached. A handle carries its own start, length and
capacity: an in-capacity append writes into shared storage, while an exhausted append
copies the visible window into fresh storage. Supplied capacity and overlapping
initialized views are retained, and detached storage uses native allocation capacity.
`SharedMap` keeps whole-map sharing. The share-visibility and reallocation obligations are recorded as
deferred WP21 proofs
(`rust:shared_list_shared_element_and_per_handle_length`,
`rust:shared_map_mutation_visibility`).

The locking documentation was also corrected: every accessor runs its callback while
the lock is held, so the earlier "no user policy runs under it" sentence was wrong.
The documentation now states that callbacks borrow the contents, cannot retain the
borrow, and must not re-enter the same handle, and that poisoning is not surfaced as
an error by these plain-data handles. No panic, fallback or recovery rule was
invented.

### Checks performed for this correction (lints/checks only, H09)

| Command | Result |
| --- | --- |
| `cargo check -p effect_cms --lib` | passed |
| `cargo clippy -p effect_cms --lib` | passed (no warnings) |
| `cargo fmt -p effect_cms -- --check` | passed |

No test, doctest, fixture, helper script, `--all-targets` or workspace-wide build was
written or run, and the architecture validators were not executed. Both corrected
behaviours (per-handle length with shared element mutation; the absent-registry
branch) are known only by construction and review until the deferred WP21 tests
exist, and neither is claimed as proven. Disk: 61.7 GiB available (93% used) and
`target/` 9.5 GiB (9,939,252 KiB) after the correction; the volume was measured at
61.6 GiB available with `target/` 9.5 GiB when the correction started, and
`crates/effect_cms` went from about 268 KiB to about 284 KiB. Only
incremental artifacts for the affected package were produced, nothing was deleted,
no new target tree was created and no cleanup was needed. `Cargo.toml`,
`crates/effect_cms/Cargo.toml`, `Cargo.lock`, the toolchain and every other crate are
unchanged by this correction, and nothing was staged or committed.

## WP03 sharing-documentation correction (definitions docs only)

The definitions documentation promised that appending through one `SharedList`
handle leaves every other handle's *view* unchanged. That overstated the model.
`push` writes at the issuing handle's own length into the shared backing storage, so
when another handle's view already reaches that slot the append replaces the element
that handle reads — for example a shorter handle appended into storage a longer
handle still reaches, or a handle appended after a copy grew the storage. Only
*lengths* stay unchanged. `definitions/sharing.rs` (module model, `push` doc
comment), `definitions/mod.rs` (data-sharing summary) and this README now say exactly
that, and the `push` doc comment no longer contradicts its own module
documentation. Capacity and reallocation are described in the following section.
At this documentation correction no collection implementation changed: `SharedList`, `SharedMap`, their
`push`/`with`/`with_element_mut`/`len` behavior and the builders are byte-for-byte
the same code. No test was written or executed.

## WP03 sharing capacity/reallocation implementation (definitions production gap)

`SharedList` has a shared backing store and a per-handle start, length, capacity and
nil state.

- `new(values)` moves nonempty values into detached storage with native allocation
  capacity; empty input and `default()` produce nil headers. `wrap(values)` retains
  supplied capacity and produces a nonnil header, including for empty input.
- `view(start, end)` creates an overlapping header over initialized backing elements,
  including initialized elements beyond the current view's length. Native spare
  capacity is not read as initialized data.
- `clone` copies the header, so in-place element mutation stays visible through
  every handle whose view reaches it.
- `push` inside the capacity writes at this handle's own length into the shared store:
  no other length changes, and a longer handle whose view already reaches that slot
  reads the new value.
- `push` that exhausts the capacity allocates a fresh store for this handle, copies
  this handle's visible window (cloning elements, so nested collection handles stay
  shared) and appends there. Older handles keep the previous store, so later outer
  writes are no longer mutually visible.
- Detached storage records the capacity of its native allocation.

Exact allocation growth across runtimes is not claimed. The reallocation behaviour is a deferred WP21 proof
(`rust:shared_list_capacity_and_reallocation_detach`), separate from this code
completion. No other owner, builder signature or bound changed: `push` requires the
element types the definitions already derive as `Clone`, and the row APIs, map handle
and relation detachment are untouched.

## WP03-L4 — current localization implementation

The five localization transport categories are implemented. Compilation and static
review passed, including the final shared-header correction. This is implementation
closure, not behavioral or full-port acceptance. The current contract supersedes
the historical notes below.

- Draft descriptions, content and format values use `InputBytes`. Value accessors
  and format callbacks retain bytes; language metadata and language normalizers
  remain textual. Payload maps support serialization without early data conversion.
- `LocalizedFieldValues` supports explicitly initialized forms, including empty
  forms. Empty language lists do not access the request. Field-name callbacks run
  before the first body read; nil data retains its distinct callback behavior.
- Request collectors are lazy Effects using `LocalizedFormPolicy` and the native
  opt-in partial-byte and accounted multipart readers. URL-encoded body values
  precede query values; multipart query values precede body fields. Repeated values
  retain order, including an empty first value. Malformed URL segments do not erase
  valid neighboring values.
- Policies retain their native units. The ordinary URL body cap, prebounded branch,
  segment count, multipart memory/file allowance, shared header count and explicit
  caller limits are enforced without treating file memory as a total upload cap.
  Files are drained, not exposed as an upload-storage feature.
- Readers reuse the request's existing body/cache owner. Compatible reads share
  outcomes; abandoned consuming reads leave a terminal acknowledgment. Typed parse
  failures do not flatten defects, interruption or finalization failures.
- Description JSON accepts byte input, adapts string tokens and unmatched
  surrogates, and retains iterative skipping, structural-depth and trailing-content
  validation. Token processing is linear; no recursive value tree is constructed.
- JSON conversion occurs at serialization. Standard Maud projects values to UTF8
  only for presentation and retains HTML escaping; stored values and callback
  inputs are not normalized by rendering. No renderer fork is used.

Native multipart admission and transfer-decoding behavior, physical file-spill
failures and malformed HTML wire-byte identity are not claims of this CMS port.
Normal CMS collection, bounds, serialization and callback behavior remain required.
The final library check is `/tmp/cms-l4-header-bound-check.log` (exit file `0`);
review `4129bf02` verified the final correction after full review `1829ef4e` and
scope audit `e312ce23`. See the progress ledger for evidence and scope details.
All runtime, browser and accessibility validation remains deferred under the global
code-before-tests instruction; no conformance claim is made.

## Historical initial localization checkpoint — superseded by WP03-L4

`crates/effect_cms/src/localization/` is the Rust counterpart of the reference
`internal/localization` package (`interfaces.go`, `platform.go`,
`internal/normalizer.go`, `internal/drafts.go`). It is a crate-level semantic owner
under the composition root, sibling of `definitions`, and holds four private
implementation modules.

**Historical status at this earlier checkpoint: `in-progress`, PARTIAL.** The concrete missing production behaviors are listed under
"Open production parity before CODE-100" below; they are production work to finish
before CODE-100, not WP21 test deferrals, and nothing here is claimed as full
source parity.

### Source → Rust mapping

| Reference | Rust |
| --- | --- |
| `LocalizedContentDraft` / `LocalizedTextDraft` | `LocalizedContentDraft` / `LocalizedTextDraft` in `localization/drafts.rs` |
| `LocalizedRowOptions[T]` (all accessors optional, default content format) | `LocalizedRowOptions<T>` with `Option<Normalizer>` / `Option<RowAccessor<T>>` fields and `default_content_format` |
| `ContractLanguageNormalizerOptions`, `NewContractLanguageNormalizer` | `ContractLanguageNormalizerOptions`, `new_contract_language_normalizer` |
| `TranslateKeyFunc` (`func(context.Context, string) string`) | `TranslateKeyFunc<C>` = `Arc<dyn Fn(&C, &str) -> String + Send + Sync>` over the caller-selected context, plus the borrowed `TranslateKeyFuncRef<'a, C>`; `C = effect_server::Request` for HTTP callers and `C = ()` for the reference's request-free `context.Background()` callers |
| `LocalizedFieldSuffix` / `LocalizedFieldName` | `localized_field_suffix` / `localized_field_name` |
| `LocalizedDescriptionFieldKey` / `LocalizedContentFieldKey` / `LocalizedContentFormatFieldKey` | `LOCALIZED_DESCRIPTION_FIELD_KEY` / `LOCALIZED_CONTENT_FIELD_KEY` / `LOCALIZED_CONTENT_FORMAT_FIELD_KEY` (`description`, `content`, `content_format`) |
| `NormalizeAuthoringLanguage` | `normalize_authoring_language` |
| `CollectLocalizedContentDraftsFromRequest` / `CollectLocalizedTextDraftsFromRequest` | `collect_localized_content_drafts_from_request` / `collect_localized_text_drafts_from_request` (lazy `Effect`) |
| `CloneLocalizedContentDrafts` / `CloneLocalizedTextDrafts` | `clone_localized_content_drafts` / `clone_localized_text_drafts` |
| `LocalizedContentDraftsFromRows` / `LocalizedTextDraftsFromRows` | `localized_content_drafts_from_rows` / `localized_text_drafts_from_rows` |
| `FindLocalizedRowForRouteLang` | `find_localized_row_for_route_lang` |
| `MetadataLocalizedRow` | `metadata_localized_row` |
| `FindLocalizedContentDraftForRouteLang` / `FindLocalizedTextDraftForRouteLang` | `find_localized_content_draft_for_route_lang` / `find_localized_text_draft_for_route_lang` |
| `LocalizedTextDraftsFromDescriptionJSON` | `localized_text_drafts_from_description_json` |
| `SerializeLocalizedTextDraftDescriptions` | `serialize_localized_text_draft_descriptions` |
| `LocalizedTextDraftsComplete` | `localized_text_drafts_complete` |

Rust names for the reference's func types are type aliases only (`Normalizer`,
`NormalizerRef`, `RowAccessor`, `TranslateKeyFunc`, `TranslateKeyFuncRef`); no source
responsibility was renamed, split or merged.

Reviewer-accepted correction: the translation port no longer fixes an HTTP request.
The reference's `TranslateKeyFunc` takes `context.Context`, and source
`validation/internal/issue_localization.go` calls it with a context that carries no
request, so the port is generic over the caller's context type: HTTP is supported
through `effect_server::Request` and request-free callers through their own context
value. A compile-time witness type-checks both instantiations without constructing a
request, a context value or a locator. No other owner's source or behavior changed.

### Preserved behavior

- **Normalizer.** The configured default language is trimmed and a blank result
  becomes `en`; preserved entries are trimmed, blank entries dropped and matched
  exactly against the *normalized* value; the caller's normalizer runs first and the
  result is trimmed afterwards (an absent normalizer only trims); the fallback key
  is consulted only for a nonempty, not-preserved normalized value, and only a
  nonblank fallback replaces it. No lowercasing, locale chain or separator rule was
  invented.
- **Field names.** The suffix normalizes the language and replaces `-` with `_`; the
  name trims the prefix and always joins with `_`, including for an empty suffix.
- **Content/text drafts from a request.** Descriptions, content and content format
  are trimmed, the optional format normalizer runs on the trimmed format, the
  requested language string is kept verbatim and `required` is always true.
- **Absent request versus present empty request.** The reference's nil-request branch
  reads no field values *and never builds a field name*, so the caller's language
  normalizer is never invoked; a present request with no matching fields does build
  each field name and does invoke the normalizer. Both cases are reachable from this
  owner's public contract: `collect_localized_content_drafts_from_values` and
  `collect_localized_text_drafts_from_values` take `Option<&LocalizedFieldValues>`,
  where `None` is the absent request and `Some` is decoded request data (possibly
  empty), and `LocalizedFieldValues::from_request_parts` builds the merged
  body-then-query values. The transport variants keep the required `R: HasRequest`
  environment and delegate to exactly that logic, so the required runtime request
  capability stays required and no fake request object is needed to exercise the
  absent branch.
- **Rows.** Drafts keep the requested language, start from the trimmed default
  content format and are required; a matching row supplies the trimmed description,
  the **untrimmed** content and the trimmed content format. A present row content
  format overrides the default even when it is empty — the default is not a
  fallback. Row selection normalizes both sides and never matches a row when the
  caller supplied no row-language accessor. Rows are borrowed: the source declares
  `LocalizedRowOptions[T any]`, `FindLocalizedRowForRouteLang[T any]` and
  `MetadataLocalizedRow[T any]` with no constraint on `T` (Go copies any `T` by
  value), so the Rust contract requires **no** `Clone`, `Default` or `PartialEq` of
  the caller's row type and no runtime type registry.
- **Draft selection.** The first matching *original* draft is returned by mutable
  reference, so a caller that mutates the result mutates the collection it passed
  in, exactly as the reference returns a pointer into its slice.
- **Metadata row.** The reference's four branches are preserved in order —
  default-language row, then the caller's present fallback, then the first row, then
  absence. The reference takes a `fallback T` and skips it when reflection reports
  the zero value; that is genuinely optional *data*, so the Rust contract takes
  `Option<&T>` for the fallback (`None` = no usable fallback) and returns
  `Option<&T>` (`None` = the reference's zero row). The zero decision therefore
  belongs to the caller, where the row's zero value is actually expressible, and the
  owner does not compare Rust values with `PartialEq` (which is not Go's
  `reflect.IsZero` — for example an empty non-nil collection is non-zero to Go but
  equal to an empty collection value in Rust) nor require `Default`/`Clone` of an
  opaque caller row.
- **Description JSON.** Blank/whitespace input yields no values; a document that is
  not valid JSON yields no values (Go validates the whole document before decoding
  and the reference discards the error); a valid non-object top level yields no
  values; a member whose value is not a JSON string is stored as an empty string,
  which is what Go's decoder writes for a member it cannot type-check into
  `map[string]string`, with the error discarded and decoding continued; duplicate
  member names keep the last value. Decoding uses the normalized language and falls
  back to the stored value for the trimmed language only when that value is empty or
  absent, and stored descriptions are not trimmed. Go's `maxNestingDepth` (10,000,
  verified in the reference toolchain's `encoding/json/scanner.go`) is preserved
  explicitly: a deeper document yields no values, exactly as Go's validation error
  does once the reference discards it. Decoding **never builds a recursive value
  tree**: the top-level object is streamed member by member through a `serde` map
  visitor, kept values are flat `String`s, and every other member value (and any
  non-string top-level shape, rejected up front as Go's type error) is skipped by
  `serde_json`'s iterative `deserialize_ignored_any`/`ignore_value` scanner, which
  uses its own byte stack and neither recurses nor materializes the skipped data.
  Parsing, skipping, and cleanup are therefore independent of document depth: no
  deep recursive drop exists on the success, parse-error, trailing-content or
  wrong-shape path, and no stack-growing deserializer or unbounded-recursion mode is
  used or needed.
- **Description JSON encoding.** Only trimmed-nonempty descriptions are written,
  keyed by the normalized language, later duplicates replacing earlier ones, and no
  usable data encodes as `{}`. Keys are ordered as Go orders map keys (byte order)
  and strings use Go `encoding/json` escaping, including the default HTML-safe
  escapes for `<`, `>`, `&`, U+2028 and U+2029 and `\u00XX` for the remaining control
  characters. This is JSON encoding; no HTML is constructed anywhere in this owner.
- **Completeness.** Only a trimmed-empty description is incomplete; an empty
  collection is complete.

### Effects and dependencies

The pure helpers stay pure functions. The two request-collection helpers perform
native I/O, so each returns a lazy `Effect<Vec<…>, E, R>` built with the workspace
`effect!`/`service!` macros: the request is selected from `R: HasRequest`, the body
form is read through `effect_server::RequestInput::<Arc<FormData>, E, R>::form`
under the caller's explicit `FormPolicy`, and a form-read failure is mapped to "no
body values" (`Effect::either`) because the reference's `FormValue` has no error
channel. That mapping is **not** claimed as exact parity — see gap 2 below. Nothing
is eagerly read, no runtime is created, no provider is selected and no hidden
byte/field/repetition bound exists: the policy is the caller's. Query/body
precedence, the method gate, the query parsing rules and Go 1.27's
`url.ParseQuery` parameter-count limit (10,000 segments, the reference's own
constant) are documented in `localization/request.rs`.

`crates/effect_cms/Cargo.toml` gained four normal dependencies for this owner:
`effect` (kernel and macros), `effect-server` (`Request`, `FormData`, `FormPolicy`,
`RequestInput`, `HasRequest`, `ServerError`), `serde` (streaming visitors for the
description decoder) and `serde_json` (JSON scanner/reader only). `Cargo.lock` gained
the matching `effect_cms` package entry. No database, SQL adapter, schema, migration,
pool, backend or credential handling is introduced; all CRUD remains consumer-owned.

### Historical inventory registration — deleted subsystem, not current code

The earlier checkpoint reported registration in `Registry::current()` with
identity, parent `effect_cms`, 20 locally declared API items, four implementation
modules (`drafts`, `json`, `normalizer`, `request`) with their own identities and
source, and seven contract-test obligations (five Rust-side proofs and two source
test files). The recorded API items follow the inventory convention already used by
`definitions`: the names declared directly in the owner module. The owner's
re-exported draft types, field-key constants and normalizer type aliases are
publications of this owner's own implementation modules, so the publication check
covers them instead. The
crate root record now lists `localization` as a child and an API item. No publication
or capability record was added: the owner publishes no immediate-child-owner contract
and holds no supplied capability — the request environment belongs to the caller's
`R`, and the translation port is declared for its consumers.

### Historical production-gap register — superseded by completed D6 and L4

1. **Multipart body forms.** Go's `FormValue` also parses `multipart/form-data`
   through `ParseMultipartForm(defaultMaxMemory)`; these request helpers read only
   URL-encoded bodies. A faithful multipart read needs an explicit caller-supplied
   multipart policy rather than an invented hidden bound, so it is unwritten. No
   inspected source call site posts multipart to either helper (both are public
   surface with no in-source caller).
2. **Partial retention on a malformed URL-encoded body.** Go's `url.ParseQuery`
   fills its map with every well-formed pair and returns the error for a malformed
   one, so `ParseForm` (and therefore `FormValue`) still reads the good body pairs
   from a partially malformed body. The native `Request::read_form` result used here
   is all-or-nothing: any parse failure removes all body values and only the query
   values remain. This production difference is recorded rather than presented as
   parity.
3. **Body parameter/length bounds versus Go's rule.** Go's body parse goes through
   the same `url.ParseQuery`, including its 10,000-segment limit; the native body
   read enforces the caller's explicit `FormPolicy` (distinct field names, per-value
   length and repetition) instead. The URL *query* path does preserve the
   10,000-segment rule.
4. **JSON unpaired-surrogate escapes.** A *kept string member* containing an
   unpaired surrogate escape makes the whole document invalid here, while Go
   substitutes U+FFFD and continues. The substitution is **not** implemented and is
   not claimed. (Skipped, non-string member values are not code-point-validated,
   matching Go's skip path.)
5. **Non-UTF-8 percent-decoded query values.** Go's `url.ParseQuery` tolerates
   percent-decoded bytes that are not UTF-8; a Rust `String` cannot hold them, so
   such a query segment is skipped (the same treatment as a malformed escape). This
   is a narrowing of the accepted input, not a user-approved change.
6. **Definitions slice capacity/reallocation.** Implemented in the primary tree (see
   the sharing capacity/reallocation section above): per-handle length and capacity,
   in-place appends inside capacity, reallocation detach outside it. The three
   unclaimed details (size-class rounding, wrap-point spare capacity, mid-array
   headers) are recorded there; this item no longer blocks CODE-100 as a missing
   behaviour.

The nil-request branch is now part of the public contract (see "Absent request
versus present empty request" above), not a private helper: `None` is the absent
request and `Some` is decoded request data. It remains recorded as the deferred proof
`rust:localized_request_absent_request_branch` for the test stage.

### Stack-safety correction (JSON description decode)

The first version of the source-limit decoder deserialized the document into a
`serde_json::Value` and then filtered its members. That kept `serde_json`'s 128-level
guard out of the way, but the *value tree* was still built and then recursively
dropped — on the success path for ignored nested members, on the `end()` trailing-error
path, and inside `serde_json` for a partially built tree on a parse error, and for a
non-object top-level shape. `serde_stacker` only protects the deserialization call
stack, so a 10,000-deep ignored tree could still overflow while being destroyed; the
stack-safety claim was therefore wrong and is withdrawn. The decoder now streams: a
`serde` map visitor collects only string members (`BTreeMap<String, String>`), every
other value is skipped through `serde::de::IgnoredAny` → `serde_json`'s iterative
`deserialize_ignored_any`/`ignore_value` scanner (its own `Vec<u8>` closing-delimiter
stack), a non-object top level is rejected before decoding, and the trailing-content
check runs after the visitor. No `Value`, no recursion, and consequently no recursive
drop on any path — success, parse error, trailing error and wrong shape all drop flat
strings only. `serde_stacker` and the unbounded-recursion mode are no longer used and
the dependency was removed; `serde_json`'s own 128-level recursion guard is never
reached because nested data only reaches the scanner through the iterative ignore
path. A kept string member with an unpaired surrogate escape still invalidates the
document (recorded gap 4, not implemented); skipped values are not code-point
validated, matching Go's skip path.

### Historical checks for the initial substep (not rerun for L4, H09)

| Command | Result |
| --- | --- |
| `cargo check -p effect_cms --lib` | passed |
| `cargo clippy -p effect_cms --lib` | passed (no warnings) |
| `cargo fmt -p effect_cms -- --check` | passed |

No test, doctest, fixture, helper script, `--all-targets` or workspace-wide build was
written or run, and the architecture validators were not executed: the new owner
record is check-compiled production inventory only, so its acceptance by `validate()`
is a reviewed expectation, not execution evidence. The reference files listed above
were read for behavior only and were not modified, executed or rewritten. The three
source test files named in the assignment were read for consumer behavior evidence
only (`contract_payload_mapper_test.go`, `localized_tab_props_test.go`,
`localized_tabs_authoring_test.go`); the two used as this owner's source obligations
are `contract_payload_mapper_test.go` (normalizer preserve/fallback) and
`content_link_shortcodes_test.go` (row language selection). The JSON depth rule and
the query parameter limit were confirmed read-only in the reference toolchain's
Go source (`encoding/json/scanner.go` `maxNestingDepth`, `net/url/url.go`
`parseQuery`/`defaultMaxParams`) for the module's pinned `go 1.27.0`; no Go program
was built or run.

Disk: `target/` 9,939,252 KiB with `crates/effect_cms` about 284 KiB before the
localization substep; 10,060,688 KiB / about 364 KiB after it; 10,062,940 KiB /
about 376 KiB after the public-contract correction described above; 10,065,252 KiB /
about 380 KiB after the stack-safety correction described above. The volume reported
62 GiB available before the localization substep, 61 GiB during these corrections and
64 GiB at the final reading (93% used); that spread is larger than the produced
artifacts, it moves in both directions, its cause was not verified and none of it is
attributed to this work, because only incremental build artifacts were produced and
nothing was deleted or cleaned here. The about 123 MiB `target/` growth matches
compiling the affected package's new dependency graph (`serde`, `serde_json`,
`effect-server`) plus the incremental recompiles of these corrections. Only generated
build artifacts were produced: nothing was deleted,
no source or user data was removed, no unrelated cache was cleared and no new target
tree was created.

## WP03 — attachments owner (third owner substep)

`crates/effect_cms/src/attachments/` is the Rust counterpart of the reference
`internal/attachments` package (`interfaces.go`, `platform.go`,
`internal/items.go`, `internal/inputs.go`). It is a crate-level semantic owner under
the composition root, sibling of `definitions` and `localization`, and holds two
private implementation modules — the reference's own item/input split, not new
semantic children.

**Status: implemented public contract for this owner.** Every declared type,
constant and public function of the inspected source is present, and the reference's
observable input algebra is reproduced. The recorded request/transport integration
obligations below are consumer-adapter responsibilities that the source also assigns
upstream; they are not claimed as done end-to-end, because no consumer owner that
reads a real request has been ported yet.

### Source → Rust mapping

| Reference | Rust |
| --- | --- |
| `MediaAttachmentInput` | `MediaAttachmentInput` (`attachments/items.rs`) |
| `MediaAttachmentStoreRecord` | `MediaAttachmentStoreRecord` (`attachments/items.rs`) |
| `MediaAttachmentFormItem` (all 19 fields) | `MediaAttachmentFormItem` (`attachments/items.rs`) |
| `MediaSurfaceMounts` | `MediaSurfaceMounts` (`attachments/items.rs`) |
| `MediaAttachmentManagerProps` (incl. value `Mounts`) | `MediaAttachmentManagerProps` (`attachments/items.rs`) |
| `MediaAttachmentRoleHero/Gallery/Inline/Download` | `MEDIA_ATTACHMENT_ROLE_HERO/GALLERY/INLINE/DOWNLOAD` (`attachments/items.rs`) |
| `MediaAttachmentRoleOptions` | `MediaAttachmentRoleOptions` (`attachments/inputs.rs`) |
| `func(string) string` role normalizer | `RoleNormalizer` (stored, `Arc`) / `RoleNormalizerRef` (borrowed, and the absent spelling) |
| `CollectMediaAttachmentInputs` | `collect_media_attachment_inputs` |
| `MediaAttachmentInputsFromItems` | `media_attachment_inputs_from_items` |
| `DedupeMediaAttachmentInputs` | `dedupe_media_attachment_inputs` |
| `NormalizeMediaAttachmentRole` | `normalize_media_attachment_role` |
| `AttachAssetIDsToAttachmentInputs` | `attach_asset_ids_to_attachment_inputs` |
| `PromoteAttachmentInput` | `promote_attachment_input` |
| `RemoveAttachmentInput` | `remove_attachment_input` |
| `MediaAttachmentInputIndex` (`-1` sentinel) | `media_attachment_input_index` (`Option<usize>`) |
| `MediaSelectedAssetIDs` | `media_selected_asset_ids` |
| `MediaShortcode` | `media_shortcode` |
| `FormValuesWithFallback` | `form_values_with_fallback` |

Two representational notes: `MediaAttachmentFormItem.Position` is Go's `int`
(`i64` here, which is what the reference's supported 64-bit platforms use), and
`MediaAttachmentInputIndex` reports absence as `None` rather than `-1`. No source
responsibility was renamed, split or merged, and no `Default`/`PartialEq`/`Clone`
bound was imposed on caller data: the records are owned plain data and the role
options carry an optional callback.

### Preserved behavior

- **Repeated form value.** The primary name wins whenever it carries **any** entry,
  even when every entry is blank; the legacy fallback name applies only when the
  primary column has no entries. Stored order is preserved and no value is trimmed
  by the fallback helper itself.
- **Collection.** Six repeated columns (asset identities, positions, roles, title
  overrides, captions, alternative-text overrides) with the reference's primary and
  legacy names. One row is built per position up to the length of the longest
  column, so a row with a blank asset identity is fully built first — the caller's
  role normalizer runs for it — and only then skipped. Nothing is deduplicated here.
  The position is parsed from the trimmed value as a signed integer, and a
  malformed or negative position falls back to that row's original column index. The
  result is stable-sorted by position and then by original index.
- **Item conversion.** A blank trimmed asset identity is skipped **before** the role
  normalizer runs; kept rows trim their title override, caption and
  alternative-text override, normalize the role, and pass through the same dedupe, so
  a kept row's normalizer runs twice — exactly the reference's call sequence.
- **Dedupe.** A blank trimmed asset identity is dropped, the first entry for each
  trimmed identity wins, the identity is stored trimmed, and only the role is
  normalized. Title override, caption and alternative-text override are deliberately
  not touched here; the offering paths trim their own metadata first.
- **Role defaulting.** `normalize_media_attachment_role` trims and lowercases and
  keeps only the exact `hero`, `inline` and `download` spellings; everything else
  becomes `gallery`. The generic normalizer used by collection, dedupe, attach,
  promote and remove never imposes `gallery`: it applies the caller's callback and
  trims the result, or only trims when no callback is supplied, so consumer-defined
  roles and the caller's configured hero/gallery names and their case are preserved.
- **Configured role names.** The hero and gallery names come from the role options
  through the neutral first-non-blank primitive with the declared `hero`/`gallery`
  values as fallback, and comparisons against them are exact on the trimmed names.
  That primitive is kept local to this owner: it carries no attachment policy and no
  other owner's private code is reached.
- **Attach.** The current inputs are deduped and the requested role normalized
  first. For the configured hero role, the first trimmed nonblank supplied identity
  becomes a **fresh** entry (the previous entry's metadata is not retained), any
  previous entry with that identity is omitted, and every older hero entry is
  downgraded to the configured gallery role. Every other role appends each new
  identity once, in the supplied order, when it is not already present. Both branches
  finish with the same dedupe.
- **Promote.** A blank identity only dedupes; an unknown identity leaves the deduped
  inputs unchanged; otherwise the selected entry keeps **its own** metadata, receives
  the configured hero role, moves to the front, and every other hero entry is
  downgraded to the configured gallery role. The result is deduped once more.
- **Remove.** A blank identity only dedupes; otherwise the deduped inputs keep their
  order with the matching trimmed identity removed.
- **Index lookup.** Each stored asset identity is trimmed before comparison, the
  supplied identity is compared as given, and the first match wins.
- **Selected identities.** The `selected_asset_ids[]` column wins whenever it carries
  any entry and only otherwise does `selected_asset_id` apply; entries are trimmed,
  blank entries dropped and the first occurrence of each identity fixes its position.
- **Shortcode.** Both inputs are trimmed, a blank identity yields no shortcode, a
  blank role becomes `inline`, and the result is the reference's exact
  `[media id="<asset>" role=<role>]` text — the role is unquoted, and nothing is
  escaped or whitelisted, because the reference does neither.

### Decoded-form input and transport adapter (A2)

The attachment algebra reads `Option<&effect_server::ByteFormData>` without I/O.
`None` represents an absent/uninitialized form. The native adapter
`initialize_media_form` returns a lazy `Effect<(ByteFormData, Option<ServerError>), E, R>`.
URL-encoded body values precede query values; multipart fields follow query values.
Partial URL-encoded values survive parse diagnostics. Consumer-provided policies
bound native input reads; initialized forms bypass parsing.

Input IDs, role callback arguments/results, overrides, captions and selected IDs
retain bytes through `InputBytes`. The attachment store records, loader keys and
necessary direct consumer fields preserve those bytes. The role callback sees the
original bytes before its returned value is trimmed. Asset input metadata and
surface/query data likewise retain bytes; built-in normalization performs only
its source-defined transformations. JSON serialization uses the existing explicit
`InputBytes` serialization boundary. URL query construction percent-encodes bytes;
no HTML is constructed by concatenation or by the attachment input algebra.

Current manager, picker and management form consumers reuse initialized forms,
including manager mounts. Management GET preserves new-asset defaults. Public
request I/O uses Effects. The shared media-type recognizer excludes conflicting
duplicate parameters without discarding ordinary malformed parameter tails.
The query fallback respects the same segment limit as initialization.

All persistence remains consumer-owned. No backend, schema, migration, pool or
runtime was added. This transport integration does not claim later media handlers
or whole-port completion. Tests remain deferred until CODE-100; compilation is
not behavioral acceptance. Native caller-selected bounds and multipart parser
admission/spill differences remain transport distinctions, not a new Go runtime.

### Inventory registration

The owner is registered in `Registry::current()` with its real compiler-checked
identity (`module_path!() == "effect_cms::attachments"`, file
`src/attachments/mod.rs`, `include_str!` source), parent `effect_cms`, its 11
locally declared API items (the reference's `platform.go` functions), two private
implementation modules (`inputs`, `items`) with their own identities and source, and
eight deferred contract-test obligations (five Rust-side proofs and three source
consumer test files). The re-exported records, role constants and role-option type
are this owner's own implementation-module publications, so the publication check
covers them; no publication or capability record was added, because the owner
publishes no immediate-child-owner contract and holds no supplied capability. The
crate-root record now lists `attachments` as a child and an API item.

### Checks performed for this substep (lints/checks only, H09)

| Command | Result |
| --- | --- |
| `cargo check -p effect_cms --lib` | passed |
| `cargo clippy -p effect_cms --lib` | passed (no warnings) |
| `cargo fmt -p effect_cms -- --check` | passed |

No test, doctest, fixture, helper script, `--all-targets` or workspace-wide build was
written or run, and the architecture validators were not executed: the new owner
record is check-compiled production inventory only, so its acceptance by `validate()`
is a reviewed expectation, not execution evidence. The four reference files named in
the assignment were read for behavior only, and the three consumer test files named
as obligations were located read-only by reference search; neither was executed,
modified or rewritten. Every behaviour listed above is a review claim about compiled
production code, not an executed result.

Disk: the nearest prior ledger reading (§9.2) was 64 GiB available (93% used),
`target/` 10,065,252 KiB and `crates/effect_cms` about 380 KiB. After this substep's
checks the volume reports 63 GiB available (93% used), `target/` 10,065,624 KiB
(+372 KiB) and `crates/effect_cms` about 428 KiB; a fresh before-reading was not
taken when the substep started, so these are the closest measured values. Only
incremental artifacts for the affected package were produced; nothing was deleted,
no source or user data or unrelated cache was removed, and no new target tree was
created.

## WP03 — temporal owner (fourth owner substep)

`crates/effect_cms/src/temporal/` is the Rust counterpart of the reference
`internal/temporal` package (`interfaces.go`, `platform.go`,
`internal/datetime.go`, `internal/duration.go`) plus the `utils` date/time helpers and
the duration input/format rules of `components/unstyled/durationpicker` it delegates
to. It is a crate-level semantic owner under the composition root, sibling of
`definitions`, `localization` and `attachments`, with six private implementation
modules: `zone`, `layout`, `translate`, `datetime`, `duration`, `request`.

**Status: implementation and production compiler check complete for the duration
request and wide-UNIX paths (T6).** Byte-preserving native form integration is in place.
The native future-zone limitation is disclosed below; behavioral tests remain deferred
until all implementation code is in place.

### Source → Rust mapping

| Reference | Rust |
| --- | --- |
| `DateTimeValidationCode` (+ `Valid/Required/InvalidDateTime/InvalidTimeZone`) | `DateTimeValidationCode` (open string newtype) and `DATE_TIME_VALIDATION_VALID/REQUIRED/INVALID_DATE_TIME/INVALID_TIME_ZONE` |
| `TemporalStatusToday/Upcoming/Past` | `TEMPORAL_STATUS_TODAY/UPCOMING/PAST` |
| `DateTimeValidationMessageKeys` | `DateTimeValidationMessageKeys` (`required`, `invalid_date_time`, `invalid_time_zone`) |
| `DateTimeValidationTranslator` | `DateTimeValidationTranslator` (`Fn(&str) -> String`, no args in the source) |
| `DateTimeDisplayLayouts` | `DATE_TIME_DISPLAY_LAYOUTS` |
| `NormalizeDateTimeFormValues` | `normalize_date_time_form_values` |
| `NormalizeDateTimeFormValuesWithMessageKey` | `normalize_date_time_form_values_with_message_key` |
| `NormalizeDateTimeFormValuesWithTranslatedMessage` | `normalize_date_time_form_values_with_translated_message` (`None` = nil translator) |
| `DateTimeValidationMessageKey` | `date_time_validation_message_key` |
| `FormatDate/FormatDateTime/FormatDateISO` | `format_date/format_date_time/format_date_iso` (explicit `&TemporalZone` local zone) |
| `FormatLocalizedDateTime/FormatLocalizedDateShort` | `format_localized_date_time/format_localized_date_short` (`&C: TemporalContext`, `BackgroundContext`) |
| `FormatLocalizedDateTimeCtx/FormatLocalizedDateShortCtx` | `format_localized_date_time_ctx/format_localized_date_short_ctx` |
| `ParseDisplayDate/ParseDisplayDateTime` | `parse_display_date/parse_display_date_time` (`DisplayDateTime { value, has_time }`) |
| `NormalizedTimeZoneName` / `LocalDateInTimeZone` | `normalized_time_zone_name` / `local_date_in_time_zone` |
| `TemporalStatus` / `TemporalStatusAt` | `temporal_status` (effect, clock from `R: HasWallClock`) / `temporal_status_at` (explicit `&TemporalInstant`) |
| `NormalizeDurationFormValues` | `normalize_duration_form_values` |
| `NormalizeDurationFormRequest` | `normalize_duration_form_request` (effect) / `normalize_duration_form_request_from_decoded` |
| `RequestedDurationFormValue` | `requested_duration_form_value` (effect) / `requested_duration_form_value_from_decoded` |
| `InvalidDurationTextFromRequest` | `invalid_duration_text_from_request` (effect) / `invalid_duration_text_from_decoded` |
| `FormatDurationCtx` | `format_duration` (`&C: TemporalContext`) |
| `durationpicker` parse/normalize/breakdown | `parse_duration`, `parse_duration_minutes`, `normalize_duration_value`, `canonical_minutes_from_structured_parts`, `canonical_minutes_from_parts`, `breakdown_duration`, `DEFAULT_STEP_MINUTES` (1), `DEFAULT_MAX_MINUTES` (0 = unlimited) |

### Preserved behaviour

- **Required before parsing, zone before value.** A trimmed-empty date/time *or* zone is
  `required` first; a nonblank but unresolvable zone is `invalid-time-zone` even when the
  date/time is also unparseable, because the reference normalizes the zone first. A
  blank zone with a parseable date/time reaches the reference's `time zone is required`
  branch and is classified `invalid-time-zone` (its `strings.Contains(err, "time zone")`
  rule).
- **Storage.** An RFC3339/RFC3339Nano instant is stored as `instant.UTC()` in Go RFC3339
  text (`2026-04-01T18:00:00` + `America/Toronto` → `2026-04-01T22:00:00Z`) with the
  declared zone name kept (blank zone → `UTC`); otherwise the value is a zoned wall
  clock interpreted in the declared zone. Zoned legacy values are never assumed UTC.
- **Go layouts, not `strftime`.** `layout` implements Go's own reference-time scan for
  formatting (every chunk of the layout language, including `MST`, all numeric/`Z` zone
  forms and fractional seconds) and parsing for the layouts the reference parses (the
  five `utils` local layouts and RFC3339), including `skip`'s space handling, fixed vs
  free-width numeric fields, month/weekday name matching, fractional seconds accepted
  even when the layout omits them, and `time.Date`'s two-lookup DST gap/overlap
  normalization. No reject/earliest/latest policy was added.
- **Display.** An instant is projected into the declared zone (blank/unresolvable →
  `UTC`); `hasTime` is true only for the datetime and minute layouts, so a date-only
  value formats without a time or zone suffix; an invalid stored value formats as its
  own raw text; the zone label is the abbreviation in effect (`EDT`), then the normalized
  zone name, then `UTC`.
- **Localized text.** Month name, `time.date_format`, `time.date_time_format`,
  `time.date_time_short_format`, `time.time_format` (Go layout, default `15:04`, or
  `3:04 PM` when `component.datetimepicker.use_24_hour` is `false`), `time.with_time_zone`
  and the reference's own fallbacks are preserved verbatim, including the `" · "`/`" "`
  fallback punctuation (`18 mars 2026 à 09:35 · EDT`).
- **Message keys.** A valid value returns a blank message; a blank configured key stays
  blank; no key is invented; an absent translator returns the key.
- **Status.** `TemporalStatusAt` parses the source value first, projects `now` into the
  declared zone when it resolves, compares the two local midnights and reports
  `today`/`upcoming`/`past`; an unparseable value has no status (empty).
- **Duration.** Typed text first (an invalid typed text fails even when the canonical
  value is valid), then any nonblank structured part, then the canonical value, then a
  valid empty result. Structured parts are nonnegative integers, minutes `0..=59`, the
  step default is 1 and a positive maximum rejects larger values. The input language is
  the reference's: signed integral minutes, ISO, `H:MM`, compact `2h30`, and the
  multilingual tokenized form with its units, fractional rounding and separators;
  canonical zero/negative normalizes to empty. The localized format decomposes
  weeks/days/hours/minutes, uses the singular/plural keys (plural looked up first, as
  the reference does) and the `part_joiner` fallback (`space`/blank/absent → `" "`),
  returns trimmed raw text for an unparseable value and empty for zero.

### Effects, zones, clocks and translation

`TemporalZone` is an IANA zone resolved through the real time-zone database
(`chrono-tz`, DST rules and abbreviations) or a caller-supplied fixed offset (the
`time.Local` case); there is no fixed-offset approximation of an IANA zone. The UNIX
formatters take the selected local zone as an argument because `time.Unix(...)` formats
in `time.Local`. `temporal_status` is an `Effect<A, E, R>` built with the workspace
effect macros: the request-free path selects the clock from `R: HasWallClock`, maps its
typed error through the caller's `on_error`, and never calls a system clock in domain
code. Translation is the caller's `TemporalContext` (typed `FormatArg` values), with
`BackgroundContext` as the `context.Background()` counterpart, so background, non-HTTP
callers work and no `effect_server::Request` is fabricated; `NormalizeDurationFormRequest`
and friends read the current request from `R: HasRequest` through the shared native merged-form adapter under explicit `DurationFormPolicy` URL-encoded/multipart bounds.

### T6 request and UNIX implementation

Duration requests use native cached partial byte-form and multipart reads. Valid pairs
survive malformed URL-encoded segments; URL-encoded body values precede query values,
and multipart values follow query values, matching FormValue. Parse diagnostics do
not discard retained values. `DurationFormValues` borrows byte maps; normalization
selects typed text, structured parts, then canonical input before decoding syntax.
Raw requested-value fallback and invalid typed text return `InputBytes`, preserving
malformed bytes. Nil decoded views remain valid empty input. No policy limits are
invented: the caller supplies URL-encoded and multipart bounds/file-memory allowance.

UNIX formatters now perform wide civil arithmetic using the source absolute epoch
and unsigned wrapping instead of returning empty outside chrono's civil range. UTC
and fixed-offset inputs cover the full i64 timestamp range. IANA lookup retains the
native database's unbounded first/last spans. `chrono-tz` supplies no Go-style POSIX
future-rule evaluator, so its terminal offset remains in force for far-future dates;
that native database difference is explicit, not an invented UTC fallback. Zero UNIX
seconds still formats as empty. No new timezone runtime, parser framework or database
was introduced. Behavioral tests remain deferred until CODE-100.

### Inventory registration

Registered in `Registry::current()` with its real compiler-checked identity
(`module_path!() == "effect_cms::temporal"`, file `src/temporal/mod.rs`,
`include_str!` source), parent `effect_cms`, its locally declared API items, six private
implementation modules with their own identities and source, and nine deferred
contract-test obligations (seven Rust-side, plus `datetime_helpers_test.go` and
`duration_helpers_test.go`). No publication or capability record was added: the owner
publishes no immediate-child-owner contract and holds no supplied capability — the clock
is the caller's `R`, the zone is an explicit argument and translation is a caller trait.
`chrono` (workspace version) and `chrono-tz 0.10` (default features off, `std` only) are
the only new dependencies; no root manifest, toolchain, other crate or database change.

### Checks performed for this substep (lints/checks only, H09)

| Command | Result |
| --- | --- |
| `cargo check -p effect_cms --lib` | passed |
| `cargo clippy -p effect_cms --lib` | passed (no warnings) |
| `cargo fmt -p effect_cms -- --check` | passed |

No test, doctest, fixture, helper script, `--all-targets` or workspace-wide build was
written or run, and the architecture validators were not executed, so acceptance of the
new record by `validate()` is a reviewed expectation rather than execution evidence. The
four reference files and the two source test files were read read-only; none was
executed, modified or rewritten. Disk: 61 GiB available (94% used) after this substep's
checks, `target/` 10,114,544 KiB and `crates/effect_cms` about 568 KiB; only incremental
artifacts for the affected package were produced, nothing was deleted and no second
target tree was created.

## WP04 content implementation checkpoint (2026-11-13)

Earlier seven-finding integration notes are historical: those corrections are present.
Current source review covers format precedence/storage, localized payload callbacks and
completeness, CDN joins, link lookup/row selection, presentation shortcode decisions,
and sanitizer policy. New corrections preserve simple Unicode lowercase in slugs,
Go JSON replacement for unpaired surrogate escapes in tag/object/pair decoding, and
short backspace/form-feed JSON escapes. JSON string-token normalization is shared
with localization through a neutral private helper; no second parser is introduced.

Sanitizer URLs now validate authority, IPv6/zone and port syntax, lowercase schemes,
and serialize path/fragment components while retaining valid original escapes.
Raw queries and opaque URLs retain their source handling. Existing supported empty-host,
relative and userinfo cases are retained. HTML remains produced by Maud for authored
components and html5ever for sanitized content. Native serializer spelling and malformed
HTML token recovery may differ from Go's token renderer; no manual HTML construction,
dependency fork or replacement tokenizer is introduced.

Production `cargo check -p effect_cms --lib` passed during implementation; final review
and any subsequent correction evidence is in the progress ledger. Tests/probes remain
unwritten and unrun until global CODE-100.

## WP05 access implementation checkpoint (2026-11-13)

W15 source review confirms public bypass, explicit-principal precedence, lazy resolver
and rule invocation, authenticated absent-rule admission, missing/forbidden response
selection, consumed denial outcomes, role/default-key policy and translated responses.
The access owner invokes the consumer identity resolver and does not own authentication
or persistence. Redirects retain invalid raw-userinfo targets instead of cleaning them;
the raw userinfo predicate is shared with content URL validation without moving response
policy or constructing a second URL parser.

The older selection-token decoding, Unicode search-token and ParseQuery gap labels were
misattributed to WP05: their actual implementations and reference owners are `queries`
(WP06). Access only uses the pure `first_non_blank` helper. Those query obligations were assigned
to WP06; no query implementation is credited to this access checkpoint.
Tests remain deferred until global CODE-100. Current check/review evidence is in the ledger.

## WP06 query implementation checkpoint (2026-11-13)

Query parsing now reuses the native partial byte decoder: malformed segments are
ignored individually, decoded bytes survive, and the source 10,000 encoded-segment
limit applies. `QueryValues` has byte-backed keys and repeated values; list search,
sort and filters, detail identity/slug/parent/locale, relation query input/revision and
string keys retain `InputBytes` through consumer callbacks and URL encoding. The
request-language callback also returns bytes. Configured definitions/route names and
generated asset record IDs remain text. Incoming asset lookup/delete IDs and form
slug callbacks preserve submitted bytes; presentation-only rendering and textual
error diagnostics project to Unicode explicitly.

Filter normalization preserves bytewise trim/dedupe/sort/cap order. Lowercasing uses
simple Unicode scalar mapping with source malformed-byte replacement behavior; search
keeps only Unicode Letter and Decimal_Number categories, not broader numeric or
alphabetic properties. Display and resubmission data remain distinct at consumers.

Relation selection decoding preserves source field-name folding, duplicate member
order, scalar-null no-ops, Keys null/empty resets and repeated-array backing-slot reuse.
Lone surrogates use the shared JSON string normalizer; lexical integer -0 is accepted
without admitting decimal/exponent spellings to integer keys. Unknown fields, type
errors, mixed key types and extra top-level values remain rejected. Native JSON syntax
error wording is retained under the source refusal prefix. Admitted identity and
callback ordering remain unchanged. Production compiler checks passed; tests/probes
remain deferred until global CODE-100, with final review evidence in the ledger.

### WP07 native validation contracts

Validation retains the existing ten native builtin schema contracts and supplies
caller-owned `PureCodec` selector sets through `NativeContractValidator`. Platform
and project handles preserve setup-error identity and routing precedence. Typed
preparation uses a declared native codec; canonical command, validation-result and
mutation-result document adapters preserve source JSON field names. Optional
validation accepts an absent validator, while absent command input remains an error.
Borrowed, serialized and effect entry points propagate explicit schema policies and
invocation budgets. Setup and selector failures expose form-scoped schema issues.

W6 preserves callback identity and sequencing. Request parsing retains MIME errors,
the 10,000 query-segment bound, exact submitted bytes, and acknowledged native body
fields across parser policies. That body snapshot is not a mutable merged request
form cache: query values are merged by the current invocation. `build_command_input_from_form` and `form_values_from_form` accept a caller-owned
initialized merged map, retaining modifications and partial values without reparsing. Native schema diagnostics and Unicode field-label presentation retain
native wording; no CUE runtime or foreign bridge is introduced. Production library
checks cover these changes; tests remain deferred until the full implementation gate.

### WP08 command availability and bindings

Source review retained the existing command implementation and corrected action
iteration to read each shared element after the preceding label callback. Actions keep declaration
order and append translation when widgets exist; support handles translation by
widget presence and unknown kinds as unsupported. Label keys and translated labels
retain trimmed fallback behavior. Binder collection remains ordered and shared
binding maps are resolved after input validation, so changes are visible to execution.
The same mutable input reaches the executor and supplies missing result identity
after execution. Executor failures stop before result validation; result validators
receive copied scalar fields with shared redirect-parameter map contents.

The native executor success type requires a mutation result, making the source nil
success payload unrepresentable. Private availability stages do not expose replay or
forgery states. No compiler check, lint or test was run for this closure; verification was source
inspection of the existing implementation and the bounded action-iteration correction.

### WP09 mutation progression and recovery

The existing W7 flow preserves action-specific validation dispatch, the mutable
input through the before callback and executor, and classification by cause plus
validation code. A successful before callback does not replace the accepted
validation result. Recovery preserves result/message callback precedence and
fallback trimming; failures are handled even without a callback. Supplied attempts
bypass standard services and preserve optional result projections.

The redirect flow now observes a missing command service only after successful
validation, preserving rejection recovery when no executor is needed. Missing
validation is still a defect when the standard flow is selected. Redirect responses
remain inside the lazy effect. Source review verified these paths; no compiler check,
lint, test or probe was run for this row.

### WP10 forms and localized tabs

Existing W5 modes, draft preparation, rehydration, action filtering and page validation
remain in place. The page now copies widgets before status translation callbacks,
and section iteration observes shared section/field edits at the source callback
boundaries. Localized labels format their single language-name operand with string
verbs, width, precision, indexing, quoting and format diagnostics. This helper serves
that label contract only.

Localized controls now publish the source rich-editor enhancement attributes, retain
the separate authoring wrapper around tabs and transport controls, and use the
localized tab-list fallback key. Maud owns all markup. The localized widget's maps
have fixed internal key sets, forwarded explicitly; arbitrary generic field maps
remain part of WP17 resource-page rendering, not a missing localized-tabs API.
No renderer fork or manual HTML assembly was introduced.

UI source baseline: reference localized_tabs.templ/localized_field_controls.templ
and their unstyled input/tabs components. Applicable skill rules include UX-01/08/10/11,
A11Y-03/04/08/09/16 and VAL-01/08/13. Existing source layout, classes, controls and
relationships are preserved. Browser/keyboard/visual/contrast and assistive-technology
evidence remain deferred until CODE-100 under the user instruction; no accessibility
conformance is claimed. No compiler check, lint, test or probe ran for this row.

### WP11 routing, permissions and metadata

Existing whole-route-set selection, canonical/form paths, query-preserving redirects,
SSE URLs, sorted mutation parameters, permission protection and metadata reconciliation
remain in place. Management-path detection now retains decoded bytes through its
optional language stripper and prefix comparison. Route methods use simple Unicode
uppercase without expanding a scalar into multiple characters.

Operational registration now starts each admitted route's metadata task before its
middleware wrapping and mount. A later registration failure leaves earlier work owned
by the application scope. The existing shared service semaphore, retry schedule,
reconciliation, typed failures and terminal logging are reused; the returned fiber
observes the started tasks. Source review is the verification evidence for this row;
no compiler check, lint, test or probe was run.

### WP12 media foundation and consumer stores

The existing asset records, resource defaults, form/query normalization and mutable
consumer store contracts are preserved. Asset and attachment operations remain lazy
`Effect` capabilities supplied by the consuming application. Record preparation runs
ID/time callbacks in source order; absent or unusable callback values use UUID/clock
fallbacks inside the effect, and attachment replacement retains original positions.
No database implementation is part of the CMS.

Stored media parsing now preserves ordered duplicate fields, folded field names,
null scalar/object behavior, nested provider/metadata merges, null item slots and
reused tag-array slots. The byte-oriented video entrypoint and media parser repair
invalid JSON string bytes and lone surrogates through the existing shared decoder.
Integer `-0` shares the selection decoder's normalization; integer decimal/exponent
spellings remain invalid. Serialization retains HTML-safe JSON escaping.

Provider inference and image classification use only the URL host/path components
required by those helpers. Relative/network references, opaque references, raw path
bytes and percent-decoded extensions are retained without path normalization;
malformed query escapes remain query text. Surface fragments encode sorted keys,
and page offsets retain fixed-width arithmetic. Existing resource-default definitions
and ID/time preparation were reviewed without rewriting them. Current corrections
were verified by source review; no compiler check, lint, test or probe was run.
Media runtime, management, attachment flows and presentation remain their own rows.

### WP13 attachment replacement

The existing W10 workflow and consumer-store adapters are retained: position order,
normalizer observations, accepted-row filtering, raw owner arguments, one replacement
including an empty clearing batch, and final loading only after replacement succeeds.
Errors and native effect exits retain their original channels. The private consumed
row stage prevents replay within a run; reusable effects construct fresh stages.

Request preview now collects the initialized form selection before checking optional
asset/builder callbacks, preserving role-normalizer calls even when preview cannot
load items. Collection reads the supplied byte-form snapshot and never parses a
body. Existing A2 initialization already carries partial URL-encoded and multipart
fields without a lossy text conversion; the old owner-level transport-gap claims
were stale. `attachmentflow/request.rs` is an unlinked historical file, not a request
path or validation receipt. No compiler check, lint, test or probe ran for this row;
verification is source review of the existing flow and bounded callback-order fix.

### WP14 media runtime and management producers

Existing consumer-store runtime operations, asset create/update/delete commands,
resource form state/pages, attachment integration and management route registration
are preserved. The update path skips a missing asset, executors reject unavailable
stores, edit forms retain their defaults when an asset is absent, and shortcode
projection retains its source IDs and callback order. Management registration already
uses the WP11 scope-owned metadata startup path; it does not need another startup
implementation.

Media validators now use the canonical A2 byte-form adapter, honor a supplied
initialized form and preserve a base validation error separately from contextual
command errors. Runtime surface normalization retains default-scope fallback;
request URLs and picker root defaults are trimmed, and role-default observations
retain their source order. The public context-label options/helper preserve the
request-aware translation port and source scope/hero precedence. Picker publication
retains its image kind, hero role, request-bound resolver and page-count callback for
the concrete presentation producer.

The management list factory now invokes its optional Maud renderer after assembling
the canonical list page. Construction remains lazy. Its direct item projection skips
blank IDs, trims links, uses the source URL summary and preview, and includes translated
kind/status metadata plus edit/delete actions and delete confirmation. This projection
maps the management list-page dependency on `presentation/internal/picker.go` and
`media_labels.go`; concrete templates/controllers remain WP15. Consumer callbacks
continue to own rendering and all actual persistence. No compiler check, lint, test
or probe ran for this row; current corrections are source-reviewed and have not been
freshly compiler-verified.

### WP15 media flows, presentation, experience and demo

The preserved presentation checkpoint is now integrated with the current byte-form,
asset, content and rendering contracts. The complete picker, library, attachment
manager, inline-create/preview, shortcode, fragment-action and content-media template
set uses Maud and imports the canonical `web_components` controls directly. Missing
standard HTML, ARIA and Datastar attributes are implemented in the existing shared
renderers (button, input, select, form, collapsible, dialog, drawer, checkbox, stepper
and chevron icon). The generic CMS adapter layer has been removed. Only named
picker-create, shortcode-action and media-mode compositions retain media-specific
markers and theme state, using shared button class builders. Native
UTF-8 text projection occurs at presentation/JSON boundaries; original store values,
attachment selections and consumer URL callbacks retain their bytes.

Six lazy fragment handlers preserve request admission, callback availability,
current-input/preview/validation/create order, rejection/error branches, HTML status
versus Datastar modes, empty successful asset identities and ordered patch delivery.
Their factories bind the actual runtime picker producer and default components.
Separate HTML and status-HTML callbacks retain inline-preview versus create behavior.
Native reply delivery owns transport failure; cancellation, defects and finalizer
failure stay in their effect channels. The protected controller implements list,
new/edit forms, create/update/delete and the six fragment endpoints over existing
access, W7 mutation and route services. Experience/from-bindings constructors retain
consumer callbacks and scoped metadata startup; no application host is created.

The asset shortcode catalog delegates lookups to the supplied store. Media dashboard
page and HTTP handler production use the preserved canonical dashboard values,
including filtered counts, ordered recent/stat/summary queries, row actions,
translation/date/report callbacks and shell/search/render integration. The canonical
`dashboards` dependency data/search/helpers were reused for this direct consumer;
its remaining service/page work stays in WP17. Existing URL serialization and the
single-string formatter are reused at the dashboard boundary.

Demo plans now provide deterministic namespace IDs, mutable consumer-store upserts,
hero-first asset/attachment seeding, shortcode insertion through Maud, featured-video
and collection JSON, and shared-spec lookup. They create no CMS persistence backend.
Production dependencies reuse resolved SHA-1 and UUID-v5 support for deterministic IDs.

Source mapping: media `flows` request/factory and inline stages; management controller,
management and experience producers; presentation components, picker/media views,
asset shortcodes, dashboard helpers and dashboard page; demo asset/plan operations.
Code was reviewed against these sources. No compilation, lint, tests, probes or
fixtures were run or authored for this row. These changes have not been freshly
compiler-verified; the historical checkpoint check does not cover current code.

UI baseline is the supplied media template set and its unstyled shared components.
The shared-control correction preserves source interaction attributes and removes
duplicated generic renderers. Runtime validation remains deferred until CODE-100
by user instruction; no runtime acceptance is claimed.

### WP16 authoring and translation

Existing translation/provider, parallel target execution, per-target deadlines and
cancellation remain in place. The request boundary now reuses the canonical byte
form initializer: normal translation initializes ParseForm values; unavailable
translation uses FormValue initialization, including multipart fields. Supplied
initialized forms bypass body reads. Partial URL-encoded/query values and their
parse diagnostic remain available for the failed-status/widget branch.

Authoring request, field, translator input/result and language-policy values use
`InputBytes`; exact field bytes reach consumer callbacks. Provider JSON and signal
JSON apply the existing explicit text encoding boundary. Localized signal builders
remain canonical forms helpers. Field normalization retains value bytes, trims
name/format bytes and preserves requested order.

Failure logging uses the existing logger and trace annotations for error, scope,
widget, languages, field-byte summary, counts and elapsed milliseconds. Status-text
callbacks now run at the source write stage. An optional `AuthoringPatchDelivery`
consumer boundary can report an expected precommit write failure; that branch uses
`internal_error_status_code` (zero selects 500). Other exits, including cleanup
failures, retain their native channels. Completed-reply delivery remains with the
existing host; committed responses cannot rewrite their sent status.

`AuthoringRouteService::register_and_start_routes` delegates operational metadata
synchronization to the existing routing service under the application scope and
shares its synchronization capacity across registrations. An absent router, service,
route name or path resolves no handler and starts no work. The synchronous
prepare/mount entry remains available.

Source evidence: authoring request decoding/field pairing, translation handler and
write helper in `authoring_helpers.go`; localized handler/status policies and route
registration in `authoring_translation_runtime.go`; existing provider and signal
patch implementations. No new runtime, provider, host or persistence implementation
was introduced. No build/check/lint, test, fixture or probe was run or authored;
these changes are not freshly compiler/runtime verified.

### WP17 rendering, dashboards and resource pages

The existing thirteen-file list/detail rendering core remains canonical. Preserved
R3 dashboard section/page builders and the bounded R3 list collection correction
were integrated over current byte-valued records. List item metadata, actions,
children and pagination links retain their shared collection handles and explicit
source detachment points. The dashboard service resolves registered cards, ordinary
pages and query-aware pages, including action/search callback order and mutable page
validation. Full rendering composition retains list, detail and dashboard capabilities;
full frame/page options project into existing list/detail builders without replacing
their query, SEO or stage logic.

Resource pages now render the complete supplied form/list/detail/dashboard template
set with Maud and direct shared controls. Field rendering consumes the canonical
`ResourceFieldVM.attrs`, relation controls, localized tabs, media managers, prepared
editors and declared options. A missing required typed editor returns its named
native render error at the field-render step. Form publication carries that failure
through the caller's error adapter; later fields and layout callbacks are not run.
Optional page overrides, shell callbacks, CSRF values, section-container IDs and
layout fallback behavior remain explicit. List/detail layout props retain the shared
`SEOView` produced by the existing SEO integration.

Ordinary templates import `maud::html` from stock Maud. Templates that use
`..(&attributes)` import `web_components::html` or call `web_components::html!`
directly. Shared controls use the same macro through `crate::html!`. Both use
the stock Maud runtime. Attribute spreads use Maud escaping for names and
values and reject invalid attribute names. Shared input, textarea,
select/options, form, button, fieldset and resource layout controls now retain complete
attribute maps, including caller-defined option synchronization data. Generic resource
layouts, command-search input, icon-button compositions and date/time locale preference
resolution live in `web_components`; CMS owns resource-specific composition. There is
no duplicated control adapter layer or manually assembled fragment HTML.

Search replacement and incremental pagination reuse shared Datastar framing. Native
body programs write and flush events in source order; terminal delivery, cancellation,
defects and cleanup remain owned by the existing request lifecycle. URL patch-mode
updates reuse existing URL validation/serialization and byte query encoding. Request
form helpers use the canonical initialized byte form; explicit mutable merged/post
snapshots retain source update behavior without mutating an immutable transport object.

Source mapping: rendering list/detail producers and full capability contracts;
dashboards builders/page builders/card/search/service; resourcepages renderer/shell,
render helpers, management actions, both template files and search SSE. Existing
provider, SEO, form, route, media, effect and native server owners are reused. No new
runtime, host, provider or persistence backend was introduced. No build/check/lint,
test, fixture or probe was run or authored; current code is not freshly compiler or
runtime verified. Historical private checks cover only their original snapshots.

### WP18 SEO, head and sitemap publication

The existing SEO head, structured JSON-LD/video metadata, sitemap document/index,
lastmod normalization and private validated publication stages remain canonical.
Page/document/index validators mutate the value that is published and preserve the
original validation cause. Head fragments continue through Maud, and structured data
uses the existing HTML-safe JSON serializer.

Three actual remaining gaps were corrected. Hreflang origin selection now uses the
consumer-supplied `SEORequestOrigin` extension: direct TLS selects HTTPS before any
forwarded-protocol value; otherwise recognized forwarded protocol applies, with HTTP
as the fallback. URI scheme alone does not assert a TLS connection. The host remains
the native request authority/Host value. A host can install its actual transport fact
with `request.insert_extension(SEORequestOrigin { direct_tls: true })`; CMS constructs
no TLS listener or host.

Filtered title/description patterns now use the neutral string formatter with both
supplied operands. Width, precision, indexing, quoting/hex verbs and diagnostics share
the same implementation as localized single-string labels; the one-string entry
remains a wrapper over that implementation.

Native list/detail SEO projection executes query admission, rendering, view extraction
and props callbacks lazily in order. Missing capabilities, absent pages and ordinary
render failure retain the empty-props branch; interruption, defects and cleanup
failures remain native exits. The canonical `ResourceSEOPropsResolver<E,R>` now returns
an Effect, and existing definition-aware list/detail builders await that resolver only
when render props are absent. Full rendering options retain those same resolver types.
`new_resource_seo_props_resolvers` binds the WP17 rendering callbacks, and the default
binder fills only absent caller options. Existing shared SEOView updates, hreflang
fallbacks and layout publication are reused.

Source evidence: SEO `props.go` origin/filter projection, `resource_projection.go` and
root list/detail projection wrappers; existing head/structured/documents/sitemap,
service and publication implementations. Changes are confined to the SEO owner,
neutral string formatting and direct rendering resolver call sites. No new runtime,
provider, host or persistence owner was introduced. No builds/checks/lints, tests,
fixtures or probes were run or authored; current changes are not freshly compiler
or runtime verified.

### WP19 relations: lookup, validation, mutation and forms

The existing canonical relation descriptors/selections, byte-valued lookup query and
five-file validation producer remain in place. The four preserved W21 consumer files
are now integrated under `mutations::relation` and re-exported by mutations. They bind
one source/relation, admit input identity against the owner-issued validation draft,
consume its claim once, run the before hook, then run/classify the consumer executor.
The preserved cancellation-versus-executor-failure precedence and applied/rejected/
failed/unresolved outcomes are retained. Actual writes remain consumer-owned.

Lookup already retained search, source identity and revision bytes; no replacement
parser was needed. Its actual transport-cause gap is corrected with
`RelationLookupDecodeError<E>`: cloned inputs share the original typed error behind
one lock, accessible through `with_error`, without requiring E to be Clone or Sync.
Decode admission still precedes mode/page/scope lookup, and the rejected outcome keeps
that typed cause. The checked constructor retains required registry/admission/lookup
capabilities. An unconstructed default service now repeatedly yields no outcome;
constructed lookup yields Some of its closed result. No fabricated successful result
or write authority was added.

The validation producer's zero-service None effect is reusable too; real validated
claims retain their one-shot consumption. Form preparation consumes the optional
lookup result and preserves its missing/rejected/unavailable/failed branches, scoped
selection availability, canonical transport collision checks, rejected transport,
row identity/actions and supplied row renderer. The relation control now delegates
input/select/button rendering to `web_components`, retaining complete admitted client
attribute maps. Source-generated search/previous/next IDs and localized submit attrs
remain on the actual controls, including the pagination focus targets.

Native interruption, typed consumer timeout failures and cleanup exits remain intact.
The request cancellation token itself is boolean and does not establish whether an
external caller requested cancellation because of a deadline; no deadline-error
identity is inferred from that token and no new cancellation runtime was introduced.

Source evidence: queries relation query/admission/observation/publication; validation
relation decoding/rows/bindings/workflow; mutations relation bindings/execution/
observation; forms relation preparation/helpers/controls/request. Preserved W21 source
and its earlier private check are reused only as historical evidence. No build/check/
lint, test, fixture or probe was run or authored for this row; current integration is
not freshly compiler/runtime verified. No CMS backend, database or CRUD implementation
was introduced.

### WP20 root composition and native integration

The root now exports `Dependencies`, `Platform`, `compose_platform`,
`DefinitionServiceSet`, `DefinitionServiceRuntime`, `DefinitionRuntimeFactory` and
its options, and the ordered `RouteService`. The private `composition` files are
root implementation modules, not additional policy owners.

`new_definition_service_set` constructs actual owners in source order: registry,
queries, validation, commands, SEO, forms, full rendering, dashboards. The same
input validator is supplied to validation and commands. Consumer validator slots
cover all ten source contracts, including dashboard, sitemap document and index.
Validation and command bindings are canonical live maps. The native byte-form
policy is explicit; no transport limits are fabricated by the composition root.

The factory supplies the source translation/action-label/localized-route/
management-detector defaults, access-policy translation fallback, per-field
canonical parameter defaults, and route-set defaults. Optional existing validation
or command services return unchanged before binder collection or reconstruction.
Required full rendering retains list, detail **and dashboard** capabilities.
Runtime page options connect the actual query/rendering/dashboard services;
missing list/detail SEO callbacks are filled with the effectful SEO projections.
Supplied callbacks remain intact. `CMSRequestContext` retains the native request
and the optional typed validation-message translator for canonical form workflows.

All eleven platform slots are copied and exposed individually: Routes, Resources,
Queries, Validation, Forms, Rendering, Commands, Dashboards, SEO, Authoring and
MediaAssets. Like the source, `DefinitionServiceSet::dependencies` supplies the eight
definition services plus supplied Routes/Authoring; callers can set MediaAssets on
that dependency value before `compose_platform`. `compose_platform` does not
invent absent capabilities or mount an application host. `MediaAssetServiceBinder`
delegates both canonical binder traits to the supplied media service, so its actual
validation/executors can populate the same maps used by definition services.

`new_route_service` retains registrar order and absent entries. Its returned Effect
**invokes and awaits** each supplied registrar in order against the native router;
`managed_route_registrar` delegates registration/protection and metadata startup to
`routing::Service::register_and_start_routes` in the application's existing Scope.
The resulting registrations and synchronization fibers are retained. No router
means no registrar invocation. Startup failure, interruption and finalization
retain native exit semantics; the root starts no independent runtime.

Public source-to-native entry mapping:

| Source entry/family | Native entry |
| --- | --- |
| `ComposePlatform` / eleven getters | root `compose_platform` / `Platform` getters |
| `NewDefinitionServiceSet` / `Dependencies` | root `new_definition_service_set` / `DefinitionServiceSet::dependencies` |
| `NewDefinitionRuntimeFactory` | root `new_definition_runtime_factory` (`services` contains the shared set-option fields and is exposed by Deref) |
| factory service-set/default/runtime methods | `service_set_options`, `new_service_set`, `new_runtime` |
| factory/current validation and command resolution | factory resolver methods / root `resolve_definition_*_service` |
| `NewDefinitionServiceRuntime` / frame/page/resource/SEO options | root runtime constructor and corresponding set/runtime methods |
| definition query/validation/command/form/SEO/dashboard constructors | canonical `queries::new_query_service`, `validation::ActionValidationService::new`, `commands::CommandService::new`, `forms::Service::new`, `seo::Service::new`, `dashboards::DashboardService::new` |
| definition full rendering constructor | `rendering::new_definition_rendering_service` |
| `NewRouteService` / route registration | root `new_route_service` / `RouteService::register_routes` |
| authoring/media factories, handlers and supplied persistence | `authoring` and `media` canonical owner APIs, retained by the platform |
| principal/access adaptation | existing `access::PrincipalResolver` supplied to factory options; `access` owns the policy |
| web/context/telemetry boundary | existing native Request, Router, Scope and Effect environment; original callbacks and typed failure channels |

The identity resolver remains application-supplied and does not assert authentication
from a fabricated principal. Root callbacks receive the existing Request/extensions
and effects execute with the caller's environment, cancellation and supervision;
no new identity store, tracing provider or request runtime is introduced. Authoring
provider/translation errors and managed startup errors remain distinct native type
parameters rather than being flattened into the resource error family.

Source evidence: `internal/platform.go` service constructors and construction order
(224–525), set/runtime option adapters (527–648), runtime factory/defaults/resolvers
(665–824), dependency/service/factory contracts (858–971), ordered route service
(149–171), and `ComposePlatform` (1677–1691). Existing producer implementations are
reused directly. No tests, probes, fixtures, builds, checks or lints were authored or
run in this row. Production source completion is not compiler or runtime validation;
the coordinator owns the CODE100 transition and deferred verification phase.

### CODE100 production compiler integration

After the coordinator confirmed all production rows closed and authorized the compiler
phase, `cargo check -p effect_cms --lib` completed successfully (exit 0). The captured
receipt is `/private/tmp/cms-code100-check.log` (`Finished dev profile`, 10.02s).
This supersedes the earlier deferred-compiler statements for the current library
source, without turning historical source-only evidence into runtime evidence.

Compiler corrections moved the SHA-1 dependency into the dependency table, repaired
one Maud delimiter and duplicate declarations, supplied native Effect Send/environment
bounds, matched existing shared component signatures, and retained owned callback/form
values across effect boundaries. The demo seed still processes hero first and the
first occurrence of each remaining key in order, now through native sequential
Effect traversal. Logging annotations and streamed Datastar bodies use the required
`span!` and native `effect_server::stream!` macros. No tests/probes/fixtures or lints
were authored or executed during this compiler integration batch.
