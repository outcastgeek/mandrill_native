# Production literal contract inventory

`source_policy_inventory.txt` records the exact multiset of source literals
reported by `source_policy.rs`. The test rejects any addition, removal or
changed count until this inventory is reviewed again. It is not a license to
introduce fixed runtime policies.

The reviewed integers are structural zero/one boundaries, indexing steps,
nonzero initial sequence values, protocol tag discriminants, UTF-8 and
length-prefix encoding dimensions, civil calendar field ranges, or exact SI
conversions. In particular, the `cron.rs` values describe seconds/minutes,
hours, days, months and weekdays; `duration.rs` and `rate_limiter.rs` use
exact nanoseconds per millisecond; `schema_binary/internal.rs` uses declared
binary tags and width limits. The `descriptor_expand.rs` zero index removes
the first checked handler when constructing a nonempty protocol. None selects
a default worker count, retry
budget, queue capacity, timeout, or scheduler quantum.

The EventLog `1` values advance a journal sequence by one entry when
validating a read or commit receipt. The transactional PubSub `1` places a
lagged subscriber immediately before the first retained sequence so its next
read can resume there. These are sequence arithmetic, not runtime policy.
The EventLog `0` values represent the cursor of an empty journal stream.

The reviewed strings and character literals are JSON/JSON Schema vocabulary,
wire field names or operation tags, parser grammar characters, Rust macro
field names and diagnostics, or typed defect labels. These are protocol or
presentation identities, not environment selectors or implicit policies.
The regex punctuation in `json_schema.rs` is escaped by the exporter; the
NDJSON characters are JSON line framing. Decimal sign, separator, zero and
empty-string literals are the exact base-ten grammar and canonical form.
`decimal_syntax` and `stable_key_encoding` are typed schema issue identities;
the empty-handler macro strings diagnose a protocol with no typed handlers.
The event-group diagnostic rejects a registry with no typed event definition.
The entity macro's `policy` field selects construction of a complete
dispatcher; its diagnostic rejects simultaneous construction and registration
forms.
A new format tag, schema keyword, or diagnostic still changes the inventory
and requires review.

Runtime capacities, attempts, leases, parse and representation budgets,
retention, clock policies, and concurrency limits remain caller-supplied
typed values or provider capabilities.

## Semantic roles

- `schema/exact_number.rs`: zero/one identities, decimal radix ten, decimal sign,
  decimal point and exponent grammar. These implement exact arithmetic; digit
  and expansion limits come from the caller. The constraint strings are JSON
  Schema vocabulary and the checked-number schema annotation.
- `schema/json_constraints.rs`: end-of-input adds one simulation position;
  regex compilation and execution bounds remain caller supplied. UUID lengths,
  hyphen offsets, hexadecimal radix, version nibble four and variant nibble
  range eight through eleven follow RFC 9562's textual version-four layout.
  Alternative indices, empty collections and unique-item comparisons are
  structural. Constraint names are JSON Schema keywords or typed issue labels.
- `schema/internal/parser.rs` and `schema/json_writer.rs`: JSON punctuation,
  whitespace, escapes, control-character boundary U+001F and scalar literals
  are JSON grammar. Zero/one counters describe empty containers and one emitted
  value; UTF-8 scratch storage follows `size_of::<char>()`. No literal sets a
  byte, collection or nesting policy.
- `schema/parse_context.rs`, `schema/native_codec.rs` and `schema/domain_codec.rs`:
  traversal starts at one root, counters start at zero, and a single issue is
  returned directly. Each resource bound is supplied through a typed policy.
  `direct_json_writer` identifies an unavailable representation witness.
- `schema/schema_ast.rs` and `schema/enum_builder.rs`: `discriminant` names the
  immutable tagged-union annotation; it does not choose an encoded wire tag.
- `workflow/memory_workflow_engine.rs`: execution ownership begins at zero and
  advances once on acquisition of a new running body. It is a checked identity
  counter, not a retry policy or a timing quantum.
- Trace diagnostic labels show only an entry count and never annotation
  values. Cache settlement uses the existing typed missing-outcome defect
  identity if an internal outcome is absent.
- `effect_macros/schema_expand.rs`: option/attribute names are Rust macro
  grammar; remaining string sites are compile-time type/layout diagnostics.
  The expansion's empty tuple and tuple positions are structural identities.

Structural zero/one counters account for shared issue trees, schema descent,
one-value writer boundaries, alternative cardinality, prefix absence, and
uniqueness. None selects a runtime capacity, retry count, or work quota.
Constraint strings use JSON Schema keywords or typed issue vocabulary in
type-side witnesses. `direct_json_writer` names an unsupported representation;
`SharedValue` is an opaque Debug label that neither inspects nor copies payloads.
The AST guard covers production files; this rationale does not exempt files or
authorize hidden runtime policies.

The callback-block lowering inventory also records native callback arities from
zero to five, checked positional argument slots, and tuple member indices. Its
strings are macro field/mode names, typed owner/action names, hygienic local
identifier recipes, and compiler diagnostics. SQL and route character literals
check identifier separators, NUL rejection, absolute paths, and capture/query
syntax; the semicolon character separates explicit constructor modes. These
values describe Rust syntax and native API contracts and select no runtime
capacity, deadline, or provider. Moving existing tuple access into macro input
makes its member indices visible to the token inventory.

The service environment adapter preserves the native caller’s explicit environment
type at the macro boundary. Its `environment` syntax and hygienic selector name
choose no runtime policy; the callback factory still evaluates once.
