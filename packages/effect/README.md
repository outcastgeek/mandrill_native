# Effect for Zig

A Zig 0.17 port of the Mandrill F# Effect library. `Effect(A, E, R)` describes
lazy work. `R` supplies application services, allocator, I/O, cancellation, and
execution providers. `run` and `runAsync().join()` return `Exit(A, E)` directly:
success, typed failure, interruption, defect, or failed finalization. Consumers
handle that result with a switch; no `try` or `catch` is required.

The package implements the typed continuation kernel, comptime builder,
resources/scopes, native async adapters, fibers, races, bounded traversal,
layers, schema validation, scheduling, tracing/logging, deferred/latch/ref/queue,
streams, managed HTTP/WebSocket, cache, pool, and console adapters. WeakCache
explicitly rejects the reference's GC ephemeron contract, which Zig does not
provide. This is an initial port; the executed validation and deliberate
adaptations are recorded in [docs/port-audit.md](docs/port-audit.md).

## Use the ordinary typed API

```zig
const std = @import("std");
const fx = @import("effect");
const AppError = enum { invalid_port };
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    port: u16,
};
const Port = fx.Effect(u16, AppError, Env);
const Build = fx.EffectBuilder(AppError, Env);

fn readPort(env: *const Env) u16 { return env.port; }
fn validate(port: u16) Port {
    return if (port == 0) Port.fail(.invalid_port) else Port.succeed(port);
}

const program = Build.bind(Port.service(readPort), validate);
// Supply an Env once at the application boundary:
// var result = fx.run(&env, program);
// defer result.deinit();
// switch (result) { .success => ..., .fail => ..., .interrupted => ...,
//                   .die => ..., .finalization_failed => ... }
```

`map` infers its output type; `flatMap`/builder binding infer the returned Effect.
Callbacks, environment/error compatibility, and finalizer signatures are checked
at comptime. The builder also supplies delay, combination, loops, finally, and
resource-aware binding/Using through the same iterative execution kernel.
Its `using` accepts the `ResourceValue` already owned by resource binding;
it does not register that resource a second time.

Dependent callbacks for `flatMap`, `flatMapOwned`, resource bodies, and builder
binding may accept either `fn (A) Out` or `fn (A, *const R) Out`. The latter gets
the current environment automatically, for constructing runtime captures or
descriptions through its allocator. It introduces no public execution context
and no second runtime configuration. The compiler selects and checks the form;
there is no runtime arity dispatch. See the executable
[mixed pipeline](proofs/mixed-pipeline.zig) for a scoped native I/O fetch, CPU
parse, bounded concurrent I/O/CPU/blocking lookup, and native I/O save.

Static composition requires a comptime-known description. Runtime input remains
runtime data. Use `fromBorrowed` for a capture that outlives the run, or
`fromOwned` and `flatMapOwned` for an allocated description owned by its consumer.
`Ref.modify` infers its result from the callback's `result` field and verifies
that its replacement `value` has the stored type.

## Select runtime providers through R

For concurrent workflows, add `async_engine: fx.Executor` to R. CPU and blocking
work select `parallel_engine` and `blocking_engine` respectively. The three
providers may be independent implementations. Consumer-owned
`NativeIoExecutor` adapters use the supplied allocator and `std.Io`; their
capabilities and `maximum_owned` admission limit are explicit configuration.
An unsupported or saturated provider returns a defect without starting the job.

`onCpu`/`onBlocking` take environment callbacks. `onCpuBorrowed` and
`onBlockingBorrowed` additionally accept runtime captures, retaining static
callback/type information without allocating a capture box. For example:

```zig
const Work = struct { input: u32 };
const P = fx.Effect(u32, AppError, RuntimeEnv);
fn compute(work: *const Work, _: *const RuntimeEnv,
           token: *const fx.Cancellation) fx.Exit(u32, AppError) {
    if (token.isRequested()) return .interrupted;
    return .{ .success = work.input * 2 };
}
fn job(work: *const Work) P { return P.onCpuBorrowed(Work, work, compute); }
// RuntimeEnv supplies allocator, io, async_engine, and parallel_engine.
// P.forEachConcurrent(*const Work, caller_limit, work_pointers, job)
// returns ordered results and drains every admitted worker before publishing.
```

Construction starts no user work. Native async factories start when execution
reaches them. `runAsync` executes the immediate prefix and returns an immediate
result or an owned pending handle. `join` observes an independently owned result;
handle `deinit` cancels and drains pending work. Scoped fibers remain owned by
their scope. R/captures/providers must outlive the joined run and scope cleanup.

## Ownership and policies

Descriptions, async handles, readers, leases, and owning results must be moved
and destroyed once. Zig does not enforce linear ownership or borrowed lifetimes.
`Exit.deinit()` releases cleanup metadata and payloads declaring the
`effect_owned_payload` clone/deinit contract; unmarked application values retain
their ordinary Zig ownership rules. Reusable owning successes/errors come from
lazy factories rather than `succeed`/`fail`. Use `borrow()` when returning a
consumer-owned description from a factory without transferring it.

Scopes install successful acquisitions before a late cancellation check and
run cleanup under protection. Race/timeout losers and rejected partial
traversals are cancelled, drained, and joined. Queue capacity, traversal limits,
cache capacity/TTL/inflight limits, pool capacity/idle lifetime, HTTP limits,
and WebSocket limits are consumer policies. No production tuning defaults are
invented. Concurrent providers require a suitable thread-safe allocator.
See [docs/api-contracts.md](docs/api-contracts.md) for the operation contracts.

## Verify

From this package directory:

```sh
zig build test example compile-fail fmt codegen --summary all
zig build test example compile-fail fmt -Doptimize=safe --summary all
zig build provider-test
```

The external consumer [examples/port.zig](examples/port.zig) handles all five
Exit cases. [proofs/facilities.zig](proofs/facilities.zig) demonstrates scoped
services, schema validation, tracing, retry, mixed CPU/blocking engines, native
HTTP, WebSocket cleanup, and allocation failures. `codegen` compares five static
compositions with equivalent handwritten Zig and writes assembly/LLVM IR under
`zig-out/proofs/`. These comparisons pass through the ordinary API.

Allocating tests use `std.testing.allocator`, including beneath tracking and
failure-injection allocators. The test target also proves that deliberately
leaking fixtures fail Zig's native test runner in debug and safe.
Python 3 runs the verification scripts; the library has no Python dependency.

See [the 2026-10-06 reference update](docs/reference-update-2026-10-06.md) for native/domain codec contracts, explicit JSON limits, dependency-cache ownership, and fixed reference dispositions.

For UTF-8 parsing, select `JsonSurface.Limits` with independent `input`, `schema`, `maximum_issues`, `maximum_integer_digits`, and `output` fields. `JsonConstraints.withLimits(Schema, selectLimits).parseBytesDirect(&env, bytes, .reject)` returns `Exit(Parsed(Schema.Decoded), SchemaIssue.Issue)`. Keep the Parsed owner until its borrowed fields are no longer used. The independent consumer includes a complete exact-number example using these limits and its caller SafeAllocator.

`JsonCodec.requireWriter(Schema)` requires child-checked public evidence. Records and variants with eligible children write directly; variant `encodeBytesWith` also accepts explicit bounds and retains a checked fallback. `DomainCodec.Directed` preserves each direction's pure evidence independently; `decodeIn` and `encodeIn` project only that direction's environment.

Workflow providers accept `WorkflowTypes.ResumeRequest` and compare `expected_revision` at mutation time. Failed-run retry supplies the observed `expected_terminal_reply`; completed poll replies include their revision. RPC receive drivers handle `end_of_input` by stopping intake and allowing pending stream consumers to drain.
