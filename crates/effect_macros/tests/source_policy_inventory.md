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
