# Schema

`PureCodec<A, Encoded>` keeps decoding, encoding and encoded-side structure together. `SchemaAst` describes representation, not an unchecked constructor for `A`. Parsing requires a consumer-selected `ParsePolicy` with explicit byte, depth, node, collection and issue limits. Decode and encode return typed `SchemaIssue` values with paths; direct type-side validation may be supplied when constructing an encoded value would be unnecessary.

The `schema!` procedural macro derives a typed record or tagged-enum codec from application fields and their codecs. Builders reject incomplete or duplicate definitions. Recursive codecs use weak schema references so a dropped schema graph does not retain itself. Representation export reports unsupported or lossy structures explicitly rather than silently changing validation semantics.

See the [provider-neutral example](../../tests/fixtures/shared/provider_program.rs),
`schema_cpu`, for a `schema!` record and an effectful codec using the CPU
provider supplied through `R`.

### Prepared schema and configuration backing

The existing `memory!` grammar can prepare one shared set of schema pools and a
separate managed configuration store. Capacities below are application choices;
`memory`, `region`, and `sink_frames` are caller-prepared owners.

```rust,ignore
let capacity = PreparedSchemaMemoryCapacity {
    writers: PreparedJsonWriterCapacity {
        writers: 4, output_bytes: 1024, name_bytes: 512,
        names: 16, depth: 8, tree_frames: 8, boundaries: 8,
    },
    validation: PreparedJsonValidationCapacity { checks: 4, depth: 8 },
    parser: PreparedJsonParserCapacity {
        documents: 8, nodes_per_document: 64,
        text_bytes_per_document: 1024, depth_per_document: 8,
    },
    contexts: PreparedParseContextCapacity { roots: 4, paths: 16, segments_per_path: 8 },
};
let schema = memory! { schema; memory: memory, region: region,
    capacity: capacity, frames: sink_frames }?;
let store = memory! { config_store; memory: memory, region: region,
    capacity: PreparedMemoryConfigCapacity {
        entries: 8, segments_per_path: 4, key_bytes_per_path: 128,
    } }?;
let path = ConfigPath::new([Arc::<str>::from("service"), Arc::<str>::from("name")])?;
let config = memory! { config; path: path.clone(), codec: json_prepared_string() };
let document = schema.parser().parse("\"example\"", schema_policy.parse)?;
store.try_set(&path, document.share()).map_err(|(error, _value)| error)?;
let context = schema.contexts().try_json_context(schema_policy)?;
let name = config.try_load(&store, &context)?;
```

`PreparedConfig::try_load` selects `HasPreparedConfigStore` from the supplied
`R`; `PreparedMemoryConfigSource` implements it directly. Defaults are optional,
used only for absence, and require the codec's independent contextual validator.
Shared values and decoded prepared strings retain the complete document slot.
The four schema pools share one writer pool; frame backing is supplied separately.
`PreparedJsonWriterPool::with_admission` optionally couples each checked-out
writer storage slot to one Slots work unit. Sink admission claims its frame first;
the writer or completed output retains the charge. Frames, snapshots and native
sink allocations are separate.
Demand reports region payload and startup control counts, excluding allocator
rounding and caller-owned Arc wrappers. The ordinary async `ConfigSource`, native
`SchemaIssue` payloads, and ordinary JSON parsing are separate obligations;
this example does not establish whole-application allocation bounds.

Managed scalar constructors `json_prepared_null`, `json_prepared_bool`,
`json_prepared_i64`, `json_prepared_u64`, and `json_prepared_f64` take the
caller-selected parser pool and `ParsePolicy`. Encoding uses the existing direct
writer witness, retaining one document and two simultaneous writer records plus
its codec-boundary snapshot before invoking that witness. It writes to managed
buffered output and parses that retained slice into the admitted document; this
is a bounded double pass, not a zero-copy conversion. The prepared string codec
instead retains its existing document without copying. Native `SchemaIssue`
contents and ordinary serde_json number parsing remain separate. Managed number tokens use borrowed validation and normalized output without a Number allocation. Demand for these pools excludes the startup codec/AST/default and
`ConfigPath` descriptions, allocator overhead, and caller-owned Arc wrappers.

Generic `Codec::decode_in` and `encode_in` recognize a managed shared JSON value
when given a context from `try_json_context`; missing prepared validation returns
a typed unsupported-contract memory error. Low-level `PureCodec` borrowed
`*_ref_in` methods leave representation checking to their caller. An unselected
`ConfiguredCodec` continues to create ordinary contexts; `.with_context_pool(...)`
selects fresh prepared contexts without making the surrounding Effect allocation-free.

Managed nullable, pair, and homogeneous-array codecs compose existing witnesses.
The typed array owns a real fixed vector and a destruction-ordered frame; frame
backing is separate from `PreparedJsonArrayCapacity::demand::<A>().region_bytes`.
The result type annotation supplies `A` to the existing `memory!` helper:

```rust,ignore
let array_capacity = PreparedJsonArrayCapacity { arrays: 4, elements: 16 };
let demand = array_capacity.demand::<i64>()?;
let frames = memory.frames(region, demand.frames, demand.frame_layout)?;
let arrays: PreparedJsonArrayPool<i64> = memory! { schema_array;
    memory: memory, region: region, capacity: array_capacity, frames: frames
}?;
let integer = json_prepared_i64(schema.shared_parser(), schema_policy.parse);
let numbers = json_prepared_array(integer.clone(), Arc::new(arrays),
    schema.shared_parser(), schema_policy.parse, None);
let pair = json_prepared_tuple2(integer.clone(), json_prepared_string(),
    schema.shared_parser(), schema_policy.parse);
let optional_pair = json_prepared_nullable(pair, schema.shared_parser(), schema_policy.parse);
```

Array encoding invokes each element's managed encoder and copies the selected
subtree into one admitted output document; it does not require an element writer
witness. Array and tuple validation/direct writing are supplied only when all
children provide the corresponding genuine witnesses. Native issue paths and
application values reachable from array elements remain separately accounted.

A managed configured codec can parse reusable shared text with the existing
parse grammar; this still uses the ordinary Effect interpreter/continuations:

```rust,ignore
let configured = json_prepared_string().into_effect::<AppEnv, AppEnv>()
    .configured(schema_policy)?
    .with_context_pool(schema.shared_contexts());
let program = effect_schema_parse! { prepared; codec: configured,
    parser: schema.shared_parser(), text: input_text };
```

A managed pure record uses the same `schema!` grammar with explicit owners:

```rust,ignore
struct Service { name: PreparedJsonString, retries: u64 }
let spare = usize::from(schema_policy.parse.unknown_fields == UnknownFields::Reject
    && schema_policy.parse.errors == SchemaErrorMode::Accumulate);
let issue_capacity = PreparedJsonArrayCapacity {
    arrays: concurrent_records,
    elements: schema_policy.parse.limits.issues.get().checked_add(spare)
        .ok_or(MemoryError::ArithmeticOverflow)?,
};
let issue_demand = issue_capacity.demand::<SchemaIssue>()?;
let issue_frames = memory.frames(region, issue_demand.frames, issue_demand.frame_layout)?;
let issue_array_pool = Arc::new(PreparedJsonArrayPool::<SchemaIssue>::prepare(
    &memory, region, issue_capacity, issue_frames)?);
let record = schema! { prepared;
    struct Service {
        name: json_prepared_string(),
        retries: json_prepared_u64(schema.shared_parser(), schema_policy.parse),
    }
    policy: schema_policy.parse,
    output: schema.shared_parser(),
    issues: issue_array_pool,
    validation: schema.shared_validation(),
}?;
```

The issue array pool is caller prepared. When unknown fields are rejected in
accumulate mode, each admitted record needs one more physical issue entry than
the logical issue limit, for the overflow-detecting unknown. Managed documents,
issue paths and startup codec/AST descriptions have distinct backing and must
be accounted separately. Ordinary schema and Effect paths retain their existing
allocation behavior. Native `SchemaIssue` paths and Arc text remain ordinary,
as does async `Codec` execution. Native `ObjectRepresentation` and
`SequenceRepresentation` still require `Vec` backing.

Managed tagged enums use the same macro, with caller-selected output and
validation owners. Unit, tuple and named payloads use managed codecs:

```rust,ignore
enum Event { Ready, Count(u64), Named { name: PreparedJsonString } }
let event = schema! { prepared;
    enum Event {
        Ready => "ready",
        Count(json_prepared_u64(schema.shared_parser(), schema_policy.parse)) => "count",
        Named { name: json_prepared_string() } => "named",
    }
    tag_field: "type", content_field: "data", policy: schema_policy.parse,
    output: schema.shared_parser(), validation: schema.shared_validation(),
}?;
```

The managed output keeps discriminator then content insertion order; ordinary
serde maps may use key order. Native issue paths/text and ordinary Effect
execution remain outside this prepared representation backing.

Representation-independent pure combinators also accept managed codecs. For
example, refinement and transformation retain the managed numeric representation:

```rust,ignore
struct RetryCount(u64);
let retries = json_prepared_u64(schema.shared_parser(), schema_policy.parse)
    .refine("retry-limit", |n| *n <= 10)
    .transform(|n| Ok(RetryCount(n)), |count: &RetryCount| Ok(count.0));
```

A supplied parse context selects the active output limits; without one the
explicit construction policy remains the fallback. These combinators retain
startup codec metadata and do not make ordinary Effect execution prepared.

Managed JSON bytes use a real retained `u8` array slot and a separate output
parser pool. The frame pool uses the exact layout in the array demand:

```rust,ignore
let byte_capacity = PreparedJsonArrayCapacity { arrays: 4, elements: 256 };
let byte_demand = byte_capacity.demand::<u8>()?;
let byte_frames = memory.frames(region, byte_demand.frames, byte_demand.frame_layout)?;
let byte_arrays: PreparedJsonArrayPool<u8> = memory! { schema_array;
    memory: memory, region: region, capacity: byte_capacity, frames: byte_frames
}?;
let bytes_codec = json_prepared_bytes(Arc::new(byte_arrays), schema.shared_parser(),
    schema_policy.parse, NonZeroUsize::new(256));
```

Decoded byte arrays retain the actual vector and frame. Encoding checks logical
limits and physical arena/text capacity before filling a single admitted output
document; it does not allocate per-byte documents. Native diagnostic paths and
ordinary Effect execution remain separate.

`json_string_constraints` prepares the fixed length and UUID diagnostic text
when the codec is defined. UUID v4 validation checks the 36-byte layout without
building a temporary nibble vector. An unprepared regex pattern still creates its ordinary PikeVM search cache per
search. Prepared cache selection has the separate native backing described below.

Prepared pattern caches can be selected explicitly for managed constrained
strings. `pattern_policy`, `memory`, `region`, and the concurrency count are
application choices:

```rust,ignore
let ordinary = CheckedPattern::new(r"^[a-z]+$", pattern_policy)?;
let cache_demand = ordinary.prepared_cache_demand(concurrent_searches)?;
let selected = memory! { pattern; memory: memory, region: region,
    pattern: ordinary, caches: concurrent_searches }?;
let constrained = json_prepared_string_constraints(StringConstraints {
    minimum: 1, maximum: 64, pattern: Some(selected), uuid_v4: false,
})?;
```

The cache slots are region-backed; PikeVM cache arrays and the temporary
startup NFA have native backing. The demand reports logical stack, active-state
and capture entries plus seven cache Vec buffers per slot, separately from the
region bytes. The stack bound counts initial exploration, union branches and
capture restore entries, including the single-pattern UTF-8 empty-match path.
Native retained capacities, allocator rounding and byte totals remain unmeasured;
runtime allocation and concurrency validation are deferred. An unselected
pattern still uses ordinary per-search cache allocation.

A prepared literal copies its startup JSON template into one admitted managed
parser document and retains that entire document for the codec's lifetime:

```rust,ignore
let literal = Arc::new(json!({"kind": "ready", "version": 1}));
let ready = json_prepared_literal(literal, schema.parser(), schema_policy.parse)?;
```

Size the parser document pool for those retained templates as well as concurrent
work, and supply a counting-writer slot during each definition. The original
`Arc<JsonValue>` and schema AST are separate startup metadata. Native diagnostic
paths and the recursive comparison/copy call stack remain separate costs.

Managed scalar literal enums retain one prepared document for each definition
literal for the codec lifetime. Account for these template slots separately from
concurrent parsing and startup `Vec`/AST metadata:

```rust,ignore
let accepted = json_prepared_literal_enum(
    vec![json!(1), json!(2)], UnknownLiteralPolicy::Preserve,
    schema.parser(), schema_policy.parse,
)?;
```

Numeric membership compares exact decimal values using borrowed tokens, including
signed zero and arbitrarily long exponent spellings; it does not convert through
`f64`. An unknown scalar retains its complete managed document. Existing native
diagnostics and recursive work remain separate costs.

Managed exact numbers retain their complete numeric document. Divisibility uses
caller-prepared region workspaces with two digit vectors per concurrent operation:

```rust,ignore
let scratch_capacity = PreparedNumericScratchCapacity { operations: 4, digits: 128 };
let scratch_demand = scratch_capacity.demand()?;
let scratch = Arc::new(memory! { numeric; memory: memory, region: region,
    capacity: scratch_capacity }?);
let divisor = JsonExactNumber::parse("0.125", NonZeroUsize::new(128).unwrap())?;
let exact = json_prepared_exact_number(
    vec![NumericConstraint::MultipleOf(divisor), NumericConstraint::Integer],
    NonZeroUsize::new(128).unwrap(), scratch,
)?;
let input = schema.parser().parse("1e0", schema_policy.parse)?.share();
let retained = exact.decode_ref(&input)?;
let original_spelling = retained.raw_token();
let mut writer = schema.writers().admit_buffered(JsonWriterPolicy {
    bytes: schema_policy.parse.limits.bytes, depth: schema_policy.parse.limits.depth,
})?;
exact.write_into(&retained, &mut writer)?;
let encoded = writer.finish_prepared()?;
```

The capacity must cover every normalized divisor coefficient that needs actual
arithmetic. Startup constraint/AST metadata remains native; document slots and
allocator overhead are separate. Runtime comparison borrows tokens, while
MultipleOf strips coefficient zeros, compares the adjusted scale, reduces the
divisor's powers of two and five, and uses bounded decimal long division without
expanding powers of ten. Zero, negative scale and a coefficient-one divisor
require no scratch lease. `raw_token()` preserves the input spelling; ordinary
`JsonExactNumber` exposes normalized text. Runtime behavior and allocation
validation remain deferred.

`json_prepared_array_constraints` applies length and optional encoded uniqueness
to managed arrays. Supply the typed `PreparedJsonArrayPool<A>` and its separately
prepared frames, plus the parser and validation pools; their capacities must
cover the concurrent array, output-document and representation-check owners.
For uniqueness, typed validation and writing are exposed only when the element
codec provides both the respective witness and semantic equality. Encoded
uniqueness compares numeric values exactly and object names independent of order.
Native diagnostic paths and recursive comparison stack remain separate costs.

Managed prefix/rest values use separate retained arrays for declared optional
prefix slots and rest elements. Each pool needs its own capacity and exact
frame layout; the output parser and validation pools are supplied separately:

```rust,ignore
let prefix_capacity = PreparedJsonArrayCapacity { arrays: 4, elements: 2 };
let rest_capacity = PreparedJsonArrayCapacity { arrays: 4, elements: 16 };
let prefix_demand = prefix_capacity.demand::<Option<u64>>()?;
let rest_demand = rest_capacity.demand::<u64>()?;
let prefix_frames = memory.frames(region, prefix_demand.frames, prefix_demand.frame_layout)?;
let rest_frames = memory.frames(region, rest_demand.frames, rest_demand.frame_layout)?;
let prefixes: PreparedJsonArrayPool<Option<u64>> = memory! { schema_array;
    memory: memory, region: region, capacity: prefix_capacity, frames: prefix_frames }?;
let rests: PreparedJsonArrayPool<u64> = memory! { schema_array;
    memory: memory, region: region, capacity: rest_capacity, frames: rest_frames }?;
let number = json_prepared_u64(schema.shared_parser(), schema_policy.parse);
let codec = json_prepared_prefix_rest(
    vec![PreparedPrefixElement { codec: number.clone(), required: true }],
    Some(number), schema_policy.parse, Arc::new(prefixes), Arc::new(rests),
    schema.shared_parser(), schema.shared_validation(),
)?;
```

Both result slots and their needed element capacity are admitted before element
codecs run. `None` denotes a trailing absent prefix slot, not JSON null; a hole
followed by a value is rejected. Output holds one admitted document and counting
writer. Native diagnostic paths and recursive subtree copy remain separate costs.

Prepared diagnostic backing is opt-in. `PreparedIssueMemoryCapacity` selects
separate array counts and element capacities for issue paths and child arrays;
its `demand()` reports each exact retained frame layout and the combined region
bytes. Prepare `PreparedIssueMemory` with the corresponding path and child
frame pools, sized for those layouts and counts. These owners do not prepare
native diagnostic text or automatically change existing codec, context, writer,
`SchemaIssue::at`, or `SchemaIssue::clone` operations.

```rust,ignore
let capacity = PreparedIssueMemoryCapacity {
    paths: PreparedJsonArrayCapacity { arrays: live_paths, elements: path_segments },
    children: PreparedJsonArrayCapacity { arrays: live_child_lists, elements: children_per_list },
    text: PreparedJsonArrayCapacity { arrays: live_texts, elements: utf8_bytes_per_text },
    match_lists: live_ambiguous_lists,
    matches_per_list: alternatives_per_list,
    records: live_issues_with_backing,
};
let demand = capacity.demand()?;
let path_frames = memory.frames(region, demand.paths.frames, demand.paths.frame_layout)?;
let child_frames = memory.frames(region, demand.children.frames, demand.children.frame_layout)?;
let text_frames = memory.frames(region, demand.text.frames, demand.text.frame_layout)?;
let issue_memory = Arc::new(memory! { issues;
    memory: memory, region: region, capacity: capacity,
    path_frames: path_frames, child_frames: child_frames, text_frames: text_frames,
}?);
let mut text_builder = issue_memory.reserve_text()?;
text_builder.write_text("invalid field: ")?;
text_builder.write_format(format_args!("{}", field_name))?;
let diagnostic_text = text_builder.finish()?;
let borrowed: &str = diagnostic_text.as_str();
let actual_native_match_bytes = issue_memory.actual_match_bytes();
let copied = issue.try_clone_prepared(&issue_memory)?;
let prefixed = copied.try_at_prepared(IssueSegment::Index(0), &issue_memory);
let schema = memory! { schema;
    memory: memory, region: region, capacity: schema_capacity,
    frames: writer_frames, issues: Arc::clone(&issue_memory),
}?;
```

Nonempty ambiguous-alternative lists use startup-reserved native `Vec<usize>`
backing. Demand reports requested bytes and the prepared owner reports actual
vector capacity bytes; neither includes allocator overhead. Each prepared issue
returns its actual list buffer when dropped. A failed prefix returns the original
issue. Empty paths, child lists, and alternative lists need no array owner.
The text builder reserves its vector and frame before formatting, checks each
complete UTF-8 write, and rejects publication after a failed write or a formatter
error reported through `write_format` or `fmt::Write::write_fmt`. A direct call
to the free `std::fmt::write` returns a formatter error to its caller; the caller
must handle that result because it is not passed back to the builder. Published
immutable text retains that array through fallible clones.
Prepared unknown/duplicate field names, JSON syntax/encoding details, and
field/property path segments can carry this text owner. `diagnostic_text()` and
`IssueSegment::text()` borrow text in either representation; equality and debug
formatting use the same logical condition and text. Ordinary `Clone` of a
prepared text case materializes its native Arc representation. Prepared issue
copying and `IssueSegment::try_clone_prepared` retain actual text backing
fallibly instead. Managed parser syntax, UTF-8, duplicate-key, selected locator
names, record and enum unknown fields, and prepared writer duplicate properties
use this backing. A managed record selects its context service first and then
its directly supplied issue service; managed enums can use their output parser
service. Missing selection yields a typed memory refusal.
The existing four-pool `schema` form without `issues:` remains available but
does not select diagnostic backing. A separately prepared context pool can
select it with `with_issue_memory`; `ParseContext::issue_memory` reports its
presence. The `schema` form with `issues:` selects the same service for its
contexts and admitted writers. The prepared `schema!` record option `issues:`
can also receive `Arc::clone(&issue_memory)` instead of a separate issue-array
pool; record accumulation and path/child decoration then contend for the same
physical child slots. An unselected prepared context or writer reports a typed
unsupported-contract error if a managed child issue needs prefixing; ordinary
writers and calls without a context retain ordinary prefix behavior. Managed
parser and subtree-copy locators also use the admitted writer service for path
ownership and dynamic field-name text. Other native diagnostic text remains
separate work. Selecting these pools alone does not certify all schema
operations or ordinary Effect execution as allocation-free.

Managed `json_prepared_one_of` and `json_prepared_any_of` take the same explicit
parse policy, a typed `PreparedJsonArrayPool<(usize, A)>`, a validation pool,
and the issue service. Size each array to retain every possible matching branch
and the issue service's child arrays to retain the smaller of the branch count
and active issue-node limit;
one-of and typed matching additionally reserve one match-index vector and issue
record. Supply the typed array's frame layout from
`PreparedJsonArrayCapacity::demand::<(usize, A)>()`. Any-of retains the complete
input document and all successful values; its writer reuses that input and needs
child validators, not child writers. One-of encodes its selected branch once
and probes the encoded document. Branch order and issue-budget accounting are
preserved; exhausted physical backing returns a memory issue. The startup AST,
callback owners and any ordinary codec/effect work remain separately classified.

`json_prepared_all_of` uses the selected parser, validation pool and issue
service. Accumulating decode admits child-error backing; First mode returns the
first prefixed failure without an aggregation array. Encoding retains one merge
document and counting writer, checks each branch, and merges in branch order;
its completed merge receives the original final logical limit check. The direct
writer admits the merge owner first, then for each branch admits a staging
document plus buffered and counting writers before invoking that branch's
writer once. Size those physical owners and codec-boundary snapshots alongside
any documents retained by branch codecs. Object output follows managed insertion
order; merge compares numbers by value and reports conflicting field paths.
Physical refusal can stop an operation. These owners do not qualify the recursive
merge stack, startup codec descriptions, or ordinary Effect execution.

`json_prepared_object_constraints` accepts explicit managed name and value
codecs, startup declared and prepared-pattern routes, and caller-supplied entry,
route-index, parser, validation, and issue owners. Each decoded entry retains
its original property name in a separate managed string document and its first
successful decoded value in the prepared entry array. Size the parser for one
retained name document per entry plus transient name, merge, and branch staging
documents; size the entry array for the object width and each per-property route
array for the maximum simultaneous declared and pattern routes (or one fallback
when no route matches). A definition with no routes does not check out a route
array. Pattern routes require explicitly prepared caches. Name decoding runs in
decode and encode selection; typed validation and writing use the independent
name validator. Multiple routes run in declared-then-pattern order and merge
sequentially; managed entries retain input order. Startup route metadata, native
name-independent codec work, and the recursive merge stack remain separate.
For typed construction, `PreparedJsonString::copy_from(name, parser, policy)`
admits a name document without serializing JSON text. Pass owned `(name, value)`
pairs to `PreparedObjectValue::try_from_iter(&entry_pool, pairs)`; it admits the
entry array before invoking the iterator, preserves the first name position, and
replaces the value on duplicate names. Refusal retires partial entries and the
iterator normally. The policy does not add a whole-string JSON byte check to
name construction; the physical name document remains caller-sized.

```rust,ignore
let name = PreparedJsonString::copy_from("region", schema.parser(), policy)?;
let values = PreparedObjectValue::try_from_iter(&entry_pool, [(name, region_value)])?;
let encoded = object_codec.encode_ref_in(&values, &context)?;
```

Prepared frame and NDJSON decoders in `effect::encoding` use the same framing
loops as their ordinary counterparts with a `PreparedBytePool` for payloads and
`PreparedJsonArrayPool<PreparedBytes>` for each push's completed results. Size
byte cells for both an in-progress payload and all escaped results, and use the
array capacity's typed `demand()` to prepare its retained frames. Result-array
admission failure consumes no input and does not mutate the decoder. A physical
failure after processing begins poisons that prepared decoder, returns the exact
memory cause and preserves earlier completed payload owners; the caller creates
a new decoder for a new stream. `encode_frame_into` and
`encode_prepared_ndjson_into` append to a caller Buffer and preserve its prefix
on failure. The latter uses an admitted prepared serde-compatible writer over a
managed document. Its line-size `observed_at_least` is the actual prepared-writer
chunk crossing the limit, which may differ from the ordinary serde serializer's
chunk boundary while retaining the same accepted byte limit. These are explicit
prepared APIs; ordinary frame/NDJSON methods keep their existing Vec behavior.

```rust,ignore
let payload_capacity = encoding::PreparedByteCapacity {
    buffers: live_payloads, bytes_per_buffer: payload_bytes,
};
let payloads = encoding::PreparedBytePool::prepare(&memory, region, payload_capacity)?;
let results_capacity = PreparedJsonArrayCapacity { arrays: live_batches, elements: frames_per_batch };
let result_demand = results_capacity.demand::<encoding::PreparedBytes>()?;
let result_frames = memory.frames(region, result_demand.frames, result_demand.frame_layout)?;
let results: PreparedJsonArrayPool<encoding::PreparedBytes> = memory! { schema_array;
    memory: memory, region: region, capacity: results_capacity, frames: result_frames,
}?;
let mut decoder = encoding::PreparedFrameDecoder::prepare(frame_policy, &payloads)?;
let completed = decoder.push(input, &results);
```

`PreparedCodec` represents owned decode, encode, independent validation and
writing as lazy descriptions on the existing prepared graph. For example, with
an already selected writer and representation:

```rust,ignore
use effect::{json_bool, PreparedCodec, PureCodec};
use std::sync::Arc;

type BoolCodec = PureCodec<bool, serde_json::Value>;
let codec = Arc::new(json_bool());
let decode = <BoolCodec as PreparedCodec<bool, serde_json::Value, (), ()>>::decode(
    Arc::clone(&codec), serde_json::Value::Bool(true), None,
);
let program = effect_prepare! { description; program: effect! {
    let value = decode?;
    let value = <BoolCodec as PreparedCodec<bool, serde_json::Value, (), ()>>::validate(
        Arc::clone(&codec), value, None,
    )?;
    <BoolCodec as PreparedCodec<bool, serde_json::Value, (), ()>>::write(codec, value, writer)
}, };
```

Here `writer` is an admitted `JsonWriter`; the example uses `()` for both
codec environments and `SchemaIssue` as its associated error. Missing genuine
writer/validator witnesses produce `MissingWriter`/`MissingValidator` through
this interface. The legacy `PureCodec::write_into` behavior is unchanged.
Prepared control flow does not by itself certify payload or callback storage.

Owned effectful managed arrays use the same `PreparedCodec` directions as the
child. Given already prepared shared arrays, parser, issue and validation
pools, select the independently bounded operation records and configure the
codec through the existing macros:

```rust,ignore
let operations: Arc<PreparedArrayOperationPool<Item>> = Arc::new(memory! {
    schema_array_operations;
    memory: memory, region: region,
    capacity: PreparedArrayOperationCapacity { operations: concurrent_arrays },
}?);
let array_codec: PreparedJsonArrayCodec<_, Item, DecodeEnv, EncodeEnv> = schema! {
    prepared_effect_array;
    codec: child_codec, policy: parse_policy, maximum: None,
    arrays: item_arrays, output: parser, operations: operations,
    issues: issues, validation: validation,
};
```

The operation demand covers the actual region records; the typed array's
`PreparedJsonArrayCapacity::demand::<Item>()` separately reports its retained
frame layout and element backing. The output parser and counting writer,
validation scratch, issue owners and child graph demand are separate. Each
operation admits its record and loop handles before callbacks; encode also
admits the output document before invoking a child. Validation rotates the
same owned values in order, and direct writing requires a genuine child writer.
Expected child failures receive their index, including failures retained in
finalization. Ordinary compatibility paths and user callbacks retain their own
storage requirements; these controls do not certify their transitive backing.

A pure codec can select the expected error family for prepared descriptions
before graph construction. For example, an owned managed boolean decoder can
join a graph whose expected error is `BinaryError`:

```rust,ignore
use effect::{json_prepared_bool, PreparedCodec, SharedPreparedJsonValue};
use effect::encoding::BinaryError;
use std::sync::Arc;

let codec = Arc::new(
    json_prepared_bool(parser, parse_policy).with_prepared_error_map(BinaryError::Schema),
);
let decode = <_ as PreparedCodec<bool, SharedPreparedJsonValue, (), ()>>::decode(
    codec, managed_value, None,
);
```

The selected mapper receives the actual `SchemaIssue` inside the invoked lazy
leaf, including missing validator or writer issues. Validation returns its
original owned value on success. This chooses the root error type up front;
it does not convert an already constructed graph or its cleanup errors between
error families. Callback and payload storage retain their own contracts.

`PreparedBinaryCodec` composes an owned managed `PreparedCodec` with the existing
schema-binary traversal. The child selects `BinaryError` as its expected error
family before construction, for example with
`pure.with_prepared_error_map(BinaryError::Schema)`. Given explicitly prepared
byte, traversal, parser and issue-array pools, prepare the separate union bank
and configure the adapter with the same schema AST identity:

```rust,ignore
use effect::encoding::{PreparedBinaryCodec, PreparedBinaryUnionCapacity};
use std::sync::Arc;

let banks = Arc::new(memory! { binary_unions;
    memory: memory, region: region,
    capacity: PreparedBinaryUnionCapacity { banks: concurrent_encodes, builders_per_bank: live_unions },
}?);
let codec: PreparedBinaryCodec<_, Item, DecodeEnv, EncodeEnv> = schema! { prepared_binary;
    binary: binary, codec: child, bytes: byte_pool, errors: union_error_arrays,
    banks: banks, output: parser, policy: parse_policy,
}?;
```

Account `PreparedByteCapacity::storage_bytes()` for output cells,
`PreparedBinaryTraversalCapacity::storage_bytes()` for traversal cells, and
`PreparedBinaryUnionCapacity::storage_bytes()` for bank controls and fixed
builder slots separately. The supplied `PreparedJsonArrayPool<BinaryError>` separately needs
sufficient arrays, element capacity and the exact retained-frame backing; a
published union diagnostic retains its own array until retirement. An encode
claims its actual byte cell, uninitialized traversal and bank builders before
calling the child encoder. A successful or discarded candidate returns its
cleared builder to that operation's bank; nested unions need simultaneous
builders. Decode holds the input bytes through managed parsing and then invokes
the genuine child decoder once. This adapter does not provide a JSON writer
witness, and its separate provider, native and diagnostic envelopes remain
subject to their own contracts.

Prepared effectful configuration keeps lookup in the original decoding
environment. A `PreparedConfigSource<R>` returns an owned prepared description;
it can use that environment's selected providers and preserves their defect,
interruption and cleanup channels. For an environment whose source error is
`MemoryError`, configure a managed pure codec with the path-aware expected error
family and select the actual context pool:

```rust,ignore
let error_path = path.clone();
let codec = Arc::new(managed_codec.with_prepared_error_map(move |issue| {
    ConfigError::Invalid { path: error_path.clone(), issue }
}));
let config: PreparedEffectConfig<_, Value, DecodeEnv, EncodeEnv> = memory! {
    config_effect;
    path: path, codec: codec, contexts: contexts, policy: schema_policy,
};
let load = config.load();
```

The environment implements `HasPreparedConfigSource`; the existing
`PreparedMemoryConfigSource` also implements the source trait using its actual
fallibly retained store handle. A fresh selected JSON context is admitted before
source work. Present values, including null, are checked in that context and
then decoded once; absence returns `ConfigError::Missing`. To use a default only for absence, add a prepared factory to the same
`memory! { config_effect; ... }` form:

```rust,ignore
let config = memory! { config_effect;
    path: path, codec: codec, contexts: contexts, policy: schema_policy,
    default: move || effect_try_sync! { once;
        body: || Ok::<_, ConfigError<MemoryError>>(make_default()),
    },
};
let load = config.load();
```

The factory runs once for an absent lookup, then the codec's independent
borrowed default witness checks the same owned value in the active context.
An owned asynchronous validator alone does not provide that witness. A present
null remains present and goes through decoding. Context, source, codec and
provider backing remain separately selected and accounted.

To choose an application error family before constructing the binary graph,
configure the managed child with that same family and supply `on_error` for
binary admission and format failures:

```rust,ignore
use effect::encoding::{BinaryError, PreparedBinaryCodec};
use std::sync::Arc;

let child = Arc::new(pure.with_prepared_error_map(|issue| {
    AppError::Binary(BinaryError::Schema(issue))
}));
let codec: PreparedBinaryCodec<_, Item, DecodeEnv, EncodeEnv, _> = schema! { prepared_binary;
    binary: binary, codec: child, bytes: byte_pool, errors: union_error_arrays,
    banks: banks, output: parser, policy: parse_policy,
    on_error: AppError::Binary,
}?;
```

The mapper runs only on an internal binary error in its prepared leaf. The child
already returns `AppError`, so its expected errors, defects, interruptions and
cleanup outcomes are forwarded without converting an existing graph or copying
cleanup storage. AST identity mismatch remains the constructor's `BinaryError`.

A source and configuration can instead select one application error family
before constructing any prepared graph. For an actual prepared memory store,
`config_source` retains the supplied store handle and maps source failures;
`config_effect` maps its own missing/context/representation failures. The
managed codec and default already return the same application error type:

```rust,ignore
use effect::{ConfigError, PreparedConfigStoreSource, PreparedEffectConfigWithDefault, PreparedMemoryConfigSource};
use effect::memory::MemoryError;
use std::sync::Arc;

type DecodeEnv = PreparedConfigStoreSource<
    PreparedMemoryConfigSource, AppError, fn(ConfigError<MemoryError>) -> AppError,
>;
let source: DecodeEnv = memory! { config_source;
    store: Arc::clone(&prepared_memory_store),
    on_error: AppError::Config as fn(ConfigError<MemoryError>) -> AppError,
};
let invalid_path = path.clone();
let codec = Arc::new(managed_codec.with_prepared_error_map(move |issue| {
    AppError::Config(ConfigError::Invalid { path: invalid_path.clone(), issue })
}));
let config: PreparedEffectConfigWithDefault<_, Item, DecodeEnv, EncodeEnv, _, _, _> = memory! {
    config_effect;
    path: path, codec: codec, contexts: contexts, policy: schema_policy,
    on_error: AppError::Config,
    default: move || effect_try_sync! { once; body: || Ok::<_, AppError>(make_default()), },
};
let load = config.load();
// Run `load` with `source` as the selected decoding environment.
```

A custom effectful source similarly selects its `Error` family when it builds
its own descriptions. Neither helper converts an already constructed source or
codec graph. The synchronous source wrapper has one startup native `Arc<F>`
for its mapper, in addition to the caller's supplied store `Arc`; configuration
has one startup native `Arc<M>` for its metadata mapper and an optional
`Arc<F>` for its default factory. Their payloads are the respective closure
values; standard Arc headers, allocator overhead and any captures' backing are
separate. No new Arc control is created by a load.

An effectful managed array can share an application root error family with
prepared binary and configuration descriptions. Supply both the schema-error
constructor and an explicit classifier:

```rust,ignore
let array: PreparedJsonArrayCodec<_, Item, DecodeEnv, EncodeEnv, _, _> = schema! {
    prepared_effect_array;
    codec: child, policy: parse_policy, maximum: None,
    arrays: item_arrays, output: parser, operations: operations,
    issues: issues, validation: validation,
    on_error: AppError::Schema,
    into_schema_error: |error| match error {
        AppError::Schema(issue) => Ok(issue),
        other => Err(other), // e.g. AppError::Disconnected remains unchanged
    },
};
```

The classifier is caller-supplied: only an extracted `SchemaIssue` receives an
index and participates in issue aggregation. Other application errors return
immediately. A borrowed array default validator is available only when the child
has its independent borrowed witness; it checks width and each original value
in order without cloning or replaying an encoder.

The managed effectful map keeps both each original name document and each
owned child value. Prepare separate typed result and input-field arrays, a name
document pool, an output parser, validation and issue backing, and operation
records. Given those already prepared shared pools, configure it through the
existing grammars:

```rust,ignore
let operations: Arc<PreparedMapOperationPool<Item>> = Arc::new(memory! {
    schema_map_operations;
    memory: memory, region: region,
    capacity: PreparedMapOperationCapacity { operations: concurrent_maps },
}?);
let map: PreparedJsonMapCodec<_, Item, DecodeEnv, EncodeEnv, _, _> = schema! {
    prepared_effect_map;
    codec: child, policy: parse_policy, entries: entry_arrays, fields: field_arrays,
    names: name_parser, output: output_parser, operations: operations,
    issues: issues, validation: validation,
    on_error: AppError::Schema,
    into_schema_error: |error| match error {
        AppError::Schema(issue) => Ok(issue),
        other => Err(other),
    },
};
```

The operation demand covers real region state records; each typed array pool
separately reports its entries and retained-frame layout. Decode reserves both
arrays and a name document per input field before invoking the child. The name
pool must account for those simultaneous documents and its own counting-writer
backing. Encode reserves an output document before its first child encoder;
validation rotates the same entry owners, and a direct writer requires the
genuine child writer. The managed representation keeps retained input order,
which can differ from ordinary native `BTreeMap` sorting. It does not certify
unrelated native or provider allocation envelopes.

`PreparedEffectRecordBuilder` composes required fields as a typed nested tuple.
Add fields in reverse declaration order; the constructor and splitter consume
and return that tuple without cloning field values. For example, with the
explicit operation, output, issue and validation owners already prepared:

```rust,ignore
let codec = PreparedEffectRecordBuilder::<Order, DecodeEnv, EncodeEnv, AppError>::new()
    .field("quantity", |order: &Order| &order.quantity, quantity_codec)
    .field("name", |order: &Order| &order.name, name_codec)
    .finish(policy, resources,
        |(name, (quantity, ()))| Order { name, quantity },
        |order: Order| (order.name, (order.quantity, ())),
        AppError::Schema,
        |error| match error {
            AppError::Schema(issue) => Ok(issue),
            other => Err(other),
        })?;
```

For accumulation, each simultaneous operation needs at least one actual
two-child issue builder for each pair of fields (`fields - 1`, saturating at
zero). The operation capacity reports its region records and builder indexes;
issue arrays and frames, output documents, and validation scratch are separate.
Definition-time names, AST and callback `Arc` controls are native startup
metadata. A separate startup name slice holds only fields guaranteed to be
emitted, so an omitted optional field does not consume output admission.
Runtime qualification remains deferred.

`optional` takes a lazy prepared missing-value factory, an omission predicate,
and an optional explicit JSON metadata annotation. It does not validate the
missing value as a default. `default_with` instead validates the produced value
with the child's independent borrowed default witness; `default_with_context`
accepts a separate contextual borrowed validator. A present value, including
null, always goes to the child decoder. An annotation is startup metadata and
is never derived by running an encoder. Factories build descriptions in the
decode environment, and omitted values are skipped by encoding, validation,
and direct writing. Encode and owned validation compute omission decisions in
field order before any child codec starts, retaining each decision with the
owned value and its child context. A context failure stops the prepass at that
field; earlier fields can still execute before the failure reaches their pair.
Direct writing evaluates each omission immediately before that field's writer.
Each selected child still requires its genuine witness.

The same typed builder is available through `schema!`. With explicitly prepared
shared pools, a distinct decode and encode environment can be selected without
cloning the record or its fields:

```rust,ignore
let operations = Arc::new(memory! { schema_record_operations;
    memory: memory, region: region,
    capacity: PreparedRecordOperationCapacity {
        operations: concurrent_records, issue_builders: pair_builders_per_record,
    },
}?);
let codec = schema! { prepared_effect; struct Order {
    id: id_codec,
    #[schema(optional)] note: optional_note_codec,
    #[schema(default_with = || { effect_sync! { once; body: || 1, } })]
    quantity: quantity_codec,
}
    policy: parse_policy, operations: operations, output: output_parser,
    issues: issues, validation: validation,
    decode_env: DecodeEnv, encode_env: EncodeEnv,
}?;
```

Codec expressions are `Arc` owners. For accumulating a record with three
fields, each operation needs two actual pair-builder entries and their separate
issue arrays and frames. The default factory is invoked only for absence and
its value needs the child's independent borrowed default validator. This form
does not create an environment, pool, capacity, or error-family conversion.

A generic encoded representation uses the same typed record fields and an
explicit `PreparedRecordRepresentation<Encoded>`. The representation supplies
an owned input cursor and actual output owner; `fields` supplies the separate
prepared array of owned input entries. With both already prepared:

```rust,ignore
let operations = Arc::new(memory! { schema_record_operations;
    memory: memory, region: region,
    capacity: PreparedRecordOperationCapacity {
        operations: concurrent_records, issue_builders: pair_builders_per_record,
    },
}?);
let codec = schema! { prepared_effect; struct Order {
    id: id_codec,
    #[schema(optional)] note: optional_note_codec,
}
    policy: parse_policy, operations: operations,
    representation: object_representation, fields: input_entry_pool,
    issues: issues, validation: validation,
    decode_env: DecodeEnv, encode_env: EncodeEnv,
    on_error: AppError::Schema,
    into_schema_error: |error| match error {
        AppError::Schema(issue) => Ok(issue),
        other => Err(other),
    },
}?;
```

`PreparedRecordOperationCapacity::demand_for::<AppError, Backing>()` reports
the actual generic input and output owner layout. The representation, input
entry array/frame, diagnostics and any provider-owned payloads have separate
demands. The macro creates one startup `Arc` for its generic backing; it does
not certify arbitrary provider allocations. Only guaranteed-emitted names are
checked when output is admitted, and each conditional name is preflighted if
it is emitted.

`PreparedSequenceRepresentation<Encoded>` supplies an owned cursor and an
owned write admission for a generic representation. Its cursor reports the
exact remaining count and moves each child out; a cursor error stops immediately,
and each successful take must reduce that count by one before child dispatch.
The write owner is admitted before child encoders and preflights each placement. Implementations must retain
actual prepared backing or a separately bounded, accounted envelope. They must
not substitute an ordinary Vec-returning representation. An owned
`PreparedJsonArray<Encoded>` itself implements the cursor contract.

For managed JSON, `PreparedJsonSequenceRepresentation` uses the selected
validation and parser pools, retains the document through sibling owners, and
owns an admitted output document and counting writer. Given the already
prepared shared pools:

```rust,ignore
type Operations = PreparedSequenceOperationPool<
    Item, PreparedJsonSequenceCursor, PreparedJsonSequenceWriteAdmission,
>;
let operations: Arc<Operations> = Arc::new(memory! { schema_sequence_operations;
    memory: memory, region: region,
    capacity: PreparedSequenceOperationCapacity { operations: concurrent_sequences },
}?);
let representation = Arc::new(PreparedJsonSequenceRepresentation::new(parser, validation.clone()));
let codec: PreparedSequenceCodec<_, _, Item, SharedPreparedJsonValue, DecodeEnv, EncodeEnv> = schema! {
    prepared_sequence;
    codec: child, representation: representation, policy: parse_policy,
    arrays: item_arrays, operations: operations, issues: issues, validation: validation,
};
```

`PreparedSequenceOperationCapacity::demand::<Item, Cursor, WriteAdmission>()`
reports real region state records including the inline cursor and write owner.
The decoded array/frame, representation backing, issue scratch and provider
working sets are separate. This generic contract does not convert custom
representations through JSON or certify their transitive native storage.

`PreparedObjectRepresentation<Encoded>` is the corresponding owned object
contract for custom representations. Its cursor returns actual owned
`(PreparedJsonString, Encoded)` pairs and an exact remaining count. The codec
fully consumes that cursor into an admitted input-field array, checks each
one-step decrement and rejects duplicates before invoking child codecs. A
cursor error returns immediately. For output, the representation consumes an
iterator of borrowed original names during admission, then owns its actual
write state through the child encoders. Implementations must retain prepared
backing or a separately bounded, accounted envelope; the legacy Vec-returning
object representation is not a prepared fallback. `PreparedObjectValue<T>`
also implements the cursor contract without cloning its entries.

The managed JSON implementation selects separate name-document, output and
validation pools. Given those and the actual result/input entry arrays:

```rust,ignore
type Operations = PreparedObjectOperationPool<
    Item, SharedPreparedJsonValue, PreparedJsonObjectWriteAdmission,
>;
let operations: Arc<Operations> = Arc::new(memory! { schema_object_operations;
    memory: memory, region: region,
    capacity: PreparedObjectOperationCapacity { operations: concurrent_objects },
}?);
let representation = Arc::new(PreparedJsonObjectRepresentation::new(
    name_parser, output_parser, validation.clone(),
));
let codec: PreparedObjectCodec<_, _, Item, SharedPreparedJsonValue, DecodeEnv, EncodeEnv> = schema! {
    prepared_object;
    codec: child, representation: representation, policy: parse_policy,
    entries: entry_arrays, fields: field_arrays, operations: operations,
    issues: issues, validation: validation,
};
```

Output admission checks the known names' total bytes and the minimum physical
node, width and depth requirements before child callbacks. Name documents,
input and result arrays/frames, output document/writer, issue scratch and
operation records are separately accounted; later actual child contents also
consume the output backing. Managed input order is retained, unlike the
ordinary `BTreeMap` sorting path. Custom representation and provider pointees
remain subject to their own declared storage contracts.
