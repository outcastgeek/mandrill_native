# Comptime DSL

Import `effect` and author ordinary typed Effects using `fx.dsl`. Static role
records contain callbacks and definitions; runtime inputs live in generated
capture storage. Functions, role namespaces, and callable objects normalize to
one implementation. Codec evidence and callback result types infer channels;
explicit E/R anchors resolve missing evidence or select a closed execution
context. Schema constructors also take the target environment once because
pure fields do not select a caller environment.

Captured codecs project environments with `codec.decodeIn(&wire, project)` and
`codec.encodeIn(&value, project)`; a typed projector supplies the caller type.
For a generic projector with no environment evidence, use the returned
`decode`/`encode` Effect's existing `provideSomeAllocated(allocator, R, project)`
boundary to supply the missing type explicitly.

Use `dsl.owned(env, inputs, body)` for reusable configuration and
`dsl.program.captured(env, inputs, nodes)` for several live named values. The
reserved `inputs` binding supplies the generated input slot. Bindings receive
borrowed pointers; an owning final value moves from its slot. Resource bindings
remain bracket-owned. A description must remain live until any asynchronous
run joins. Plain pointers are borrows and keep their native lifetime contract.

Use the examples as complete runnable starting points:

| Recipe | Source |
| --- | --- |
| File I/O, CPU parse, bounded mixed-provider lookup and save | `examples/dsl/native_io.zig` |
| Earlier values, nested reservation, branch, loop and early exit | `examples/dsl/order.zig` |
| Scoped cache with early/error fold consumption | `examples/dsl/cache_stream.zig` |
| Typed unary/streaming handlers and managed session lifetime | `examples/dsl/rpc_session.zig` |
| Business-key traversal, recorded activity replay and compensation | `examples/dsl/workflow.zig` |
| Captured transaction cells, retry and rollback | `examples/dsl/withdrawal.zig` |
| Branch-sensitive tracked reads and graph replacement | `examples/dsl/atoms.zig` |
| Activation reacquisition, typed replay and atomic checkpoint | `examples/dsl/activation_projection.zig` |
| Surrounding Effect composition with explicit E/R adapters | `examples/dsl/composition.zig` |

Transaction descriptions admit journal operations and immutable intermediates;
external Effects remain outside the transaction. The native callback escape
requires synchronous, repeatable code. Workflow descriptions admit their own
Steps and recorded activities; a matching field layout is insufficient. Stream
plans retain scoped readers and pull only on demand. Atom programs collect
actual reads in an evaluation-local context and publish only a valid snapshot.
Activation plans check the current token before protected work; the provider
must fence commits atomically. Live resources cannot enter persisted channels.

Collection concurrency, schema parse budgets, cache/pool configuration,
wire/version/expiry rules and provider guarantees remain explicit policies.
There is no universal bounds record and no separate DSL runner. Operations at
legal boundaries return the existing `Effect(A,E,R)` and compose with its native
mapping, error handling, projection, cancellation and resource semantics.

Run `zig build example` for the executable recipes, `zig build dsl-test` for
registered acceptance proofs, and `zig build dsl-perf -Doptimize=fast` for raw
measurements. Performance fixtures report compiler/options, inputs, iteration
counts, allocation/byte counts, elapsed time and executable size. Results are
measurements for the selected environment, not a general speedup promise.
