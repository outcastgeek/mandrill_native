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

Structured programs retain owning values in generated slots. When an unmarked
composite result contains owning members, the returned Effect uses a generated
payload owner; its business value is available through `result.success.value`.
Callbacks selecting an earlier binding still receive a pointer to the business
value. Loop `done`, normal loop completion, final bindings, and early returns
transfer only the selected value; cancellation releases initialized slots and
iteration resources through the existing Effect and Scope authorities. Executable
coverage lives in [program lifetime proofs](proofs/dsl-program-lifetime.zig).

`dsl.program.describe(nodes)` infers the error and environment from callback
signatures and earlier binding types, including native, branch, loop and resource
nodes. Native-only programs infer an empty error channel. Use
`describeWith(.{ .environment = R }, nodes)` when a fully generic callback graph
has no environment evidence; `error_type` selects an explicit error channel.
Conflicting callback context requires explicit mapping or projection. The original
`build(E, R, nodes)` entry remains available for explicitly anchored descriptions.

`dsl.program.captured(&env, runtime_inputs, nodes)` moves application inputs once
into generated storage and reserves the earlier binding name `"inputs"`. A node
selecting that name receives a pointer to the inferred input record. Named
bindings still retain earlier values, branches, loop state and lexical resources.
The returned value is the ordinary inferred Effect; compose it before running it.
Keep its description alive through execution and joined work, and deinitialize it
after its last run. Native pointers returned from callbacks retain their ordinary
borrow contract. See [captured program proofs](proofs/dsl-programs.zig).

Named stream factories use `dsl.streams.from(.{ .open = open, .pull = pull, .release = release })`;
`iterator` accepts a native Exit or Effect returning the next optional item.
`append`, `bind`, `branch`, `yieldEffect`, `unfold`, and `loop` preserve pull demand.
Use `scoped` for owning runtime configuration and `borrowed` for explicitly
borrowed configuration. `map`, `mapError`, `project`, and `consume` infer their
channels from the supplied roles. Fold sinks support both a static stream with
`run` and a runtime stream with `runWith`. See [stream proofs](proofs/dsl-streams.zig).

`dsl.activation` composes generated acquisition, use, bind, and branch plans over
`ClusterEntityResource`. Its `run` constructs an Effect with explicit token,
fence, clock, and error-mapping roles. The existing scope releases acquisitions
in reverse order after dispatch drains. Each run reacquires live resources; typed
persistence and projection declarations reject known activation resource wrappers.
See [activation proofs](proofs/dsl-activation.zig).

`dsl.entity` and `dsl.event` infer completed native definitions from named codecs
and handlers. `dsl.eventCase` associates an event definition with its pure fold;
`dsl.projection` derives dispatch and decoding from those cases. A contextual
payload decoder remains an Effect and retains its parse document through joined
work. Replay publishes a new state/cursor only after the complete page succeeds.
Use the native atomic checkpoint capability for state/cursor publication.

`dsl.persistence.payloadCodec` infers application channels and separate directional
environments from byte encoder/decoder roles. `dsl.persistence.scoped` and
`dsl.queues.scoped` retain runtime keys and configuration; `declared` accepts the
shared `declareMetadata`/`inputs` carrier. Persistence uses native outcome versions
and expiry. Queues require the native versioned binary codec, explicit claim and
retention policy, and the actual provider guarantee. Decoded claims share one
owner: their payload is disposed before its borrowed encoded representation.
Retry and acknowledgement consume that owner while retaining native fencing and
at-least-once delivery semantics. See [persistence proofs](proofs/dsl-events-persistence.zig)
and [queue proofs](proofs/dsl-queues.zig).

Run `zig build dsl-perf -Doptimize=fast` for allocation/runtime pairs and serial
stage, binding, branch, and field scaling. Raw commands, compiler/platform,
source hash, process compile times, executable sizes, and runtime results are
written to `zig-out/dsl-performance/measurements.json`. Timings use the normal
compiler cache and are measurements, not speedup guarantees.

`dsl.activity` takes named identity, codec and Effect-handler roles. `dsl.workflow`
takes an explicit `WorkflowTypes.Definition`, codec roles and a durable Step
handler. Both infer channel types and the handler environment through the same
directional evidence used by RPC declarations. Workflow and schema versions are
explicit business policy. These declarations retain the completed native handler
types and their existing engine boundaries; ordinary Effects belong inside
recorded activity handlers. Executable examples live in
[workflow declaration proofs](proofs/dsl-workflow-declarations.zig).

## Codecs with runtime configuration

`dsl.makeCodec` accepts one record containing `definition =
dsl.declareMetadata(...)` and `inputs = configuration`. Supply contextual
`decode(configuration, representation, environment)` and
`encode(configuration, value, environment)` callbacks returning ordinary
Effects. The declaration infers both value types, the shared error type, and
each direction's environment independently. Generic arguments specialize from
either direction's known evidence; a genuinely ambiguous declaration needs
only its unresolved `value`, `representation`, `decode_env`, or `encode_env`
type anchor.

Select `.pure` for two pure directions, `.pure_decode` for a pure decoder,
or `.pure_encode` for a pure encoder. Pure callbacks take configuration and
input pointers and return an exact `Result`; they do not take an environment.
Their `decodeDirect` or `encodeDirect` operation returns a retained Result.
The ordinary `decode` and `encode` operations still return exact Effects;
a pure direction uses `void` as its execution environment. Composite owning
values and errors receive generated cleanup, and retaining configuration
does not require a business `clone` method.

A pure declaration using the native `SchemaIssue.Issue` channel may also supply
`writer(configuration, value, output) -> Exit(void, Issue)`, where `output` is
the native `JsonWriter.Writer`. Use `codec.writeJson(&value, &writer)` for a
direct call or `codec.writeWith(&value, selectLimits)` for an ordinary Effect.
The limits selector determines the caller environment, which supplies the
output allocator. Each execution uses the native bounded writer with fresh
state. Generated storage retains configuration through writer errors and
cleanup metadata; successful output bytes own their storage independently.

```zig
var creation = fx.dsl.makeCodec(&construction_env, .{
    .definition = fx.dsl.declareMetadata(.{
        .kind = fx.dsl.CodecKind.contextual,
        .decode = decodeConfigured,
        .encode = encodeConfigured,
    }),
    .inputs = runtime_configuration,
});
defer creation.deinit();
var made = fx.run(&construction_env, creation);
defer made.deinit();
// Handle all Exit cases before accessing success.
if (made == .success) {
    var input = made.success.representation(wire_value);
    defer input.deinit();
    var decoding = made.success.decode(&input);
    defer decoding.deinit();
    var decoded = fx.run(&decode_env, decoding);
    defer decoded.deinit();
    // decoded.success.value is the application value after checking success.
}
```

Creating the description moves configuration into generated storage and does
not call either codec callback. Reusing the creation Effect retains the same
configuration; it does not clone application data. Each decode/encode invocation
has its own execution frame. Descriptions and returned value/error owners retain
configuration, including through suspended work and cancellation. Their `.value`
field exposes the application payload. `representation` and `value` move caller
payloads into those generated owners; borrowed inputs stay alive through join.
`decode`, `encode`, `validate`, `decodeIn`, and `encodeIn` return exact Effects
and compose with existing map/bind methods. Validation encodes once and releases
the representation. Direct-writer and pure evidence remain with the existing
checked codec APIs; an explicit `shape` is retained without granting purity.

Description and retention storage use the construction allocator; execution
frames and callback allocations use the selected directional environment.
The [public codec proofs](proofs/dsl-owned-codecs.zig) exercise separate allocation
domains, owner release, generic inference, pending work, provider rejection,
late cancellation, and allocation failures.

Runtime RPC middleware accepts one `withRpcMiddleware(&env, .{ .definition =
declareMetadata(.{ .acquire = acquire, .project = project, .use = use,
.release = release }), .inputs = configuration })` record. `acquire` takes
configuration and caller environment pointers and returns the service Effect.
`project` takes configuration, service and caller environment pointers and
returns the handler environment, `Result(HandlerEnv, MiddlewareError)`, or
`Effect(HandlerEnv, MiddlewareError, CallerEnv)`.
`use` takes configuration and projected environment pointers and returns its
ordinary Effect. The optional `release` takes configuration, service, primary
outcome and caller environment pointers and returns
`Effect(void, MiddlewareError, CallerEnv)`.

Configuration is captured once; acquisition and projection are fresh per run.
Generated storage holds service and environment addresses through suspension.
The existing resource bracket runs effectful release after projected-environment
cleanup, including after late acquisition during cancellation. Service native
cleanup follows release drain. Domain and middleware failures remain separate;
release failures retain the primary outcome. Native owning service, environment
and channel types must declare their ownership before Effect composition.
Returned borrowed data follows the native callback's lifetime contract. Streams
consumed by a server use the managed request scope described below; returned
borrowed values continue to follow their native lifetime contract. The executed cases are in
[the middleware proof](proofs/dsl-rpc-middleware.zig).

`withRpcSession` adds a `server` role naming the canonical `RpcStreamServer`
type to the middleware declaration. Its `use` callback receives configuration,
the generated `Scope(Server.DomainError)`, and the projected server environment.
The callback returns `Effect(A, Server.Failure, Server.Environment)`. The outer
result remains an ordinary Effect; middleware failures stay separate from the
server's domain, session/provider, serialization and protocol errors.

The generated request scope closes through the existing Scope authority before
middleware environment/service release. This covers reader EOF/early close,
reader cleanup failure, pending pull cancellation, async owner destruction, and
admitted child fibers after the serving body returns. Scope-close error mapping
runs after drain and introduces no allocation before cleanup starts. The
[session proofs](proofs/dsl-rpc-session.zig) execute canonical server opening and
reader consumption. The [serving proofs](proofs/dsl-rpc-serving.zig) add encoded
provider send/receive, owning chunk/terminal/error payloads, acknowledgement
ordering, cancellation, cleanup failures, and two admitted runtime registry calls.

`makeRpcHandler` also accepts streaming definitions. A captured streaming
callback returns the definition's exact `Stream(Event, Error, R)`; chunk and
terminal types come from that definition. `handler.openWith(scope, context,
request)` returns the existing `Effect(StreamReader(Event, Error, R), Error, R)`.
The generated context record and configuration stay live through native reader
finalizers. Request pointers and context entries remain native borrows through
scope close.

Runtime registries open direct declaration records, retained streaming handlers,
and `rpcStatic` bindings through `registry.open(scope, definition, request)` or
`openWith(scope, definition, context, request)`. Value requests move into generated
call storage; pointer requests remain borrows. Readers and their clones retain
configuration and call storage independently of the registry description. Reader
resource access and borrowed chunk contents keep the underlying Stream scope
contract: consumption occurs before that scope closes. The
[captured streaming proofs](proofs/dsl-rpc-stream-captures.zig) exercise these
paths, pending work, allocation failures and projected execution allocation.
Concurrent driving and remaining provider/session variants still need verification.
