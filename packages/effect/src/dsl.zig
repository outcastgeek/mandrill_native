//! Checked Zig-native role adapters. All effectful results are the existing
//! Effect(A,E,R), executed by the existing kernel and selected providers.
const effects = @import("effect.zig");
const outcomes = @import("outcome.zig");
pub const inference = @import("dsl/inference.zig");
pub const roles = @import("dsl/roles.zig");
pub const captures = @import("dsl/capture.zig");
pub const ownership = @import("dsl/ownership.zig");
pub const pipeline = @import("dsl/pipeline.zig");
pub const program = @import("dsl/program.zig");
pub const streams = @import("dsl/streams.zig");
pub const stream = streams.from;
pub const streamBorrowed = streams.borrowed;
pub const activation = @import("cluster/entity_resource_plan.zig");
pub const entities = @import("dsl/entities.zig");
pub const entity = entities.from;
pub const entityDefinition = entities.definition;
pub const events = @import("dsl/events.zig");
pub const event = events.from;
pub const eventDefinition = events.definition;
pub const eventCase = events.case;
pub const projection = events.projection;
pub const persistence = @import("dsl/persistence.zig");
pub const persistable = persistence.from;
pub const withPersistence = persistence.scoped;
pub const queues = @import("dsl/queues.zig");
pub const withQueue = queues.scoped;
pub const selectors = @import("dsl/selectors.zig");
pub const resources = @import("dsl/resources.zig");
pub const resourceBinding = resources.binding;
pub const using = resources.using;
pub const usingCaptured = resources.usingCaptured;
pub const scopeAcquire = resources.acquire;
pub const registerFinalizer = resources.finalizer;
pub const resourceCaptured = resources.bracket;
pub const scopedCaptured = resources.scoped;
pub const layers = @import("dsl/layers.zig");
pub const layer = layers.from;
pub const mergeLayers = layers.merge;
pub const provideLayer = layers.provide;
pub const composeLayers = layers.compose;
pub const permits = @import("dsl/permits.zig");
pub const withPermit = permits.with;
pub const withPermitCaptured = permits.captured;
pub const collections = @import("dsl/collections.zig");
pub const all = collections.all;
pub const allOf = collections.allOf;
pub const allBorrowed = collections.allBorrowed;
pub const allOwned = collections.allOwned;
pub const allConcurrent = collections.allConcurrent;
pub const allConcurrentStatic = collections.allConcurrentStatic;
pub const allConcurrentBorrowed = collections.allConcurrentBorrowed;
pub const forEachCaptured = collections.forEachCaptured;
pub const forEach = collections.forEach;
pub const forEachConcurrent = collections.forEachConcurrent;
pub const forEachOwned = collections.forEachOwned;
pub const forEachConcurrentCaptured = collections.forEachConcurrentCaptured;
pub const forEachConcurrentOwned = collections.forEachConcurrentOwned;
pub const forEachFactory = collections.forEachFactory;
pub const forEachFactoryConcurrent = collections.forEachFactoryConcurrent;
pub const policies = @import("dsl/policies.zig");
pub const retry = policies.retry;
pub const repeat = policies.repeat;
pub const retryOwned = policies.retryOwned;
pub const repeatOwned = policies.repeatOwned;
pub const race = policies.race;
pub const timeout = policies.timeout;
pub const uninterruptible = policies.uninterruptible;
pub const references = @import("dsl/references.zig");
pub const makeRef = references.make;
pub const modifyRefCaptured = references.modifyCaptured;
pub const modifyRefEffect = references.modifyEffect;
pub const RefChange = references.Change;
pub const refChange = references.change;
pub const caches = @import("dsl/caches.zig");
pub const makeCache = caches.make;
pub const makeDependencyCache = caches.makeDependent;
pub const pools = @import("dsl/pools.zig");
pub const makePool = pools.make;
pub const resolvers = @import("dsl/resolvers.zig");
pub const resolveBatch = resolvers.batch;
pub const resolveGrouped = resolvers.grouped;
pub const sockets = @import("dsl/sockets.zig");
pub const withSocket = sockets.withSocket;
pub const withConnection = sockets.withConnection;
pub const codecs = @import("dsl/codecs.zig");
pub const codec = codecs.from;
pub const transformCodec = codecs.transform;
pub const CodecKind = codecs.Kind;
pub const records = @import("dsl/records.zig");
pub const schemaRecord = records.schema;
pub const nativeRecord = records.nativeRecord;
pub const variants = @import("dsl/variants.zig");
pub const variant = variants.from;
pub const recursive = @import("dsl/recursive.zig");
pub const recursiveSchema = recursive.from;
pub const RecursiveBox = recursive.Box;
pub const parsing = @import("dsl/parsing.zig");
pub const parseSchema = parsing.parse;
pub const declarations = @import("dsl/declarations.zig");
pub const declareMetadata = declarations.metadata;
pub const parseDeclaration = parsing.declared;
pub const owned_codecs = @import("dsl/owned_codecs.zig");
pub const makeCodec = owned_codecs.make;
pub const rpc = @import("dsl/rpc.zig");
pub const activity = @import("dsl/workflow_declarations.zig").activity;
pub const workflow = @import("dsl/workflow_declarations.zig").workflow;
pub const rpcUnary = rpc.unary;
pub const rpcStream = rpc.streaming;
pub const rpcRegistry = rpc.registry;
pub const rpcUnaryDefinition = rpc.unaryDefinition;
pub const rpcStreamDefinition = rpc.streamDefinition;
pub const rpcHandled = rpc.handled;
pub const makeRpcHandler = rpc.make;
pub const rpcStatic = rpc.staticBinding;
pub const withRpcRegistry = rpc.scopedRegistry;
pub const withRpcMiddleware = @import("dsl/rpc_middleware.zig").scoped;
pub const withRpcSession = @import("dsl/rpc_middleware.zig").session;

fn callbackType(comptime supplied: anytype, comptime operation: []const u8) type {
    roles.validate(roles.recordType(supplied), operation, &.{"run"}, &.{});
    return @TypeOf(roles.get("run", supplied));
}

fn callback(comptime supplied: anytype, comptime operation: []const u8) callbackType(supplied, operation) {
    roles.validate(roles.recordType(supplied), operation, &.{"run"}, &.{});
    return roles.get("run", supplied);
}

/// Explicit E/R anchor for leaves whose result alone cannot determine them.
/// Source-driven composition infers this context from the supplied Effect.
pub fn Dsl(comptime E: type, comptime R: type) type {
    return struct {
        pub fn sync(comptime supplied: anytype) effects.Effect(inference.unary(callback(supplied, "sync"), *const R, "sync.run"), E, R) {
            const run = callback(supplied, "sync");
            const A = inference.unary(run, *const R, "sync.run");
            comptime rejectEffectValue(A, "sync.run");
            return effects.Effect(A, E, R).sync(run);
        }

        pub fn outcome(comptime supplied: anytype) effects.Effect(inference.unary(callback(supplied, "outcome"), *const R, "outcome.run").Success, E, R) {
            const run = callback(supplied, "outcome");
            const T = inference.unary(run, *const R, "outcome.run");
            if (T != outcomes.Exit(T.Success, E)) @compileError("outcome.run requires exact Exit(A,E)");
            return effects.Effect(T.Success, E, R).fromOutcome(run);
        }

        pub fn trySync(comptime supplied: anytype) effects.Effect(nativePayload(supplied, R), E, R) {
            comptime roles.validate(roles.recordType(supplied), "trySync", &.{ "run", "map_error" }, &.{});
            return effects.Effect(nativePayload(supplied, R), E, R).trySync(roles.get("run", supplied), roles.get("map_error", supplied));
        }
        pub fn async(comptime supplied: anytype) effects.Effect(asyncSuccess(supplied, R), E, R) {
            const run = callback(supplied, "async");
            const Out = effects.Effect(asyncSuccess(supplied, R), E, R);
            return Out.fromAsync(run);
        }
        pub fn cpu(comptime supplied: anytype) effects.Effect(workerSuccess(supplied, R), E, R) {
            const run = callback(supplied, "cpu");
            return effects.Effect(workerSuccess(supplied, R), E, R).onCpu(run);
        }
        pub fn blocking(comptime supplied: anytype) effects.Effect(workerSuccess(supplied, R), E, R) {
            const run = callback(supplied, "blocking");
            return effects.Effect(workerSuccess(supplied, R), E, R).onBlocking(run);
        }

        pub fn native(comptime supplied: anytype) effects.Effect(nativePayload(supplied, R), E, R) {
            comptime roles.validate(roles.recordType(supplied), "native", &.{ "run", "map_error" }, &.{});
            const run = roles.get("run", supplied);
            const on_error = roles.get("map_error", supplied);
            const Native = @typeInfo(inference.unary(run, *const R, "native.run")).error_union;
            if (inference.unary(on_error, Native.error_set, "native.map_error") != E) @compileError("native.map_error output must match the anchored E");
            return effects.Effect(Native.payload, E, R).tryNative(run, on_error);
        }

        pub fn factory(comptime supplied: anytype) inference.unary(callback(supplied, "factory"), *const R, "factory.run") {
            const run = callback(supplied, "factory");
            const Out = inference.unary(run, *const R, "factory.run");
            comptime checkContext(Out, E, R, "factory.run");
            return Out.deferEffect(run);
        }

        /// Move a runtime record into generated stable storage. Application
        /// fields are supplied once; ownership follows declared field evidence.
        pub fn owned(env: *const R, capture: anytype, comptime run: anytype) inference.result(run, &.{ *const @TypeOf(capture), *const R }, "owned.run") {
            const T = @TypeOf(capture);
            const Out = inference.result(run, &.{ *const T, *const R }, "owned.run");
            comptime checkContext(Out, E, R, "owned.run");
            const Stored = ownership.Capture(T);
            var seed = effects.Effect(void, E, R).succeed({});
            return seed.flatMapCapturedOwned(env, Out.Success, Stored{ .value = capture }, struct {
                fn next(stored: *const Stored, _: void, current: *const R) Out {
                    return run(&stored.value, current);
                }
            }.next);
        }

        pub fn syncOwned(env: *const R, capture: anytype, comptime run: anytype) effects.Effect(inference.result(run, &.{ *const @TypeOf(capture), *const R }, "syncOwned.run"), E, R) {
            const T = @TypeOf(capture);
            const A = inference.result(run, &.{ *const T, *const R }, "syncOwned.run");
            comptime rejectEffectValue(A, "syncOwned.run");
            const Stored = ownership.Capture(T);
            return effects.Effect(A, E, R).fromOwned(env, Stored{ .value = capture }, struct {
                fn invoke(stored: *const Stored, current: *const R) outcomes.Exit(A, E) {
                    return .{ .success = run(&stored.value, current) };
                }
            }.invoke);
        }

        /// Method objects use the same stable owned capture path; the receiver
        /// is never the address of the constructing function's parameter.
        pub fn object(env: *const R, supplied: anytype) inference.result(@TypeOf(supplied).call, &.{ *const @TypeOf(supplied), *const R }, "object.call") {
            return @This().owned(env, supplied, @TypeOf(supplied).call);
        }

        /// Infer the capture and Exit types of a borrowed native leaf.
        pub fn outcomeBorrowed(pointer: anytype, comptime run: anytype) effects.Effect(borrowedExit(run, @TypeOf(pointer), R).Success, E, R) {
            const P = @TypeOf(pointer);
            const Exit = borrowedExit(run, P, R);
            if (comptime Exit.Error != E) @compileError("outcomeBorrowed.run error type mismatch; use an explicit mapper");
            if (comptime @typeInfo(P) != .pointer or !@typeInfo(P).pointer.attrs.@"const") @compileError("outcomeBorrowed requires a stable *const capture pointer");
            return effects.Effect(Exit.Success, E, R).fromBorrowed(@typeInfo(P).pointer.child, pointer, run);
        }
        pub fn asyncBorrowed(pointer: anytype, comptime run: anytype) effects.Effect(borrowedAsync(run, @TypeOf(pointer), R).Success, E, R) {
            const P = @TypeOf(pointer);
            const Exit = borrowedAsync(run, P, R);
            if (comptime Exit.Error != E) @compileError("asyncBorrowed.run error type mismatch; use an explicit mapper");
            return effects.Effect(Exit.Success, E, R).fromAsyncBorrowed(@typeInfo(P).pointer.child, pointer, run);
        }

        pub fn borrowed(pointer: anytype, comptime run: anytype) inference.result(run, &.{ @TypeOf(pointer), *const R }, "borrowed.run") {
            const P = @TypeOf(pointer);
            if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one or !@typeInfo(P).pointer.attrs.@"const")
                @compileError("borrowed.run requires a stable *const capture; preserve qualifiers explicitly");
            const Out = inference.result(run, &.{ P, *const R }, "borrowed.run");
            comptime checkContext(Out, E, R, "borrowed.run");
            return Out.deferBorrowed(@typeInfo(P).pointer.child, pointer, run);
        }

        pub fn field(comptime name: []const u8) effects.Effect(*const @FieldType(R, name), E, R) {
            return selectors.borrow(E, R, name);
        }

        /// Named resource roles preserve the existing bracket's outcome and
        /// protected cleanup contracts. Result/resource types come from acquire.
        pub fn resource(comptime supplied: anytype) resourceResult(supplied) {
            const acquire = roles.get("acquire", supplied);
            comptime checkContext(@TypeOf(acquire), E, R, "resource.acquire");
            return acquire.acquireRelease(roles.get("body", supplied), ResourceRelease(supplied).release);
        }

        pub inline fn using(resource_value: anytype, comptime supplied: anytype) @TypeOf(resources.using(resource_value, supplied)) {
            const Out = @TypeOf(resources.using(resource_value, supplied));
            comptime checkContext(Out, E, R, "using.body");
            return resources.using(resource_value, supplied);
        }

        pub fn closeScope(scope: *@import("scope.zig").Scope(E)) effects.Effect(void, E, R) {
            return effects.Effect(void, E, R).closeScope(scope);
        }

        pub fn scoped(comptime supplied: anytype) inference.continuation(scopedBody(supplied), *@import("scope.zig").Scope(E), R, "scoped.body") {
            const Out = inference.continuation(scopedBody(supplied), *@import("scope.zig").Scope(E), R, "scoped.body");
            comptime checkContext(Out, E, R, "scoped.body");
            return Out.scoped(scopedBody(supplied));
        }

        pub fn getRef(reference: anytype) @TypeOf(references.get(E, R, reference)) {
            return references.get(E, R, reference);
        }
        pub fn modifyRef(reference: anytype, comptime supplied: anytype) @TypeOf(references.modify(E, R, reference, supplied)) {
            return references.modify(E, R, reference, supplied);
        }
    };
}

fn resourceResult(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "resource", &.{ "acquire", "body", "release" }, &.{});
    const Acquire = @TypeOf(roles.get("acquire", supplied));
    inference.requireEffect(Acquire, "resource.acquire");
    const Out = inference.continuation(roles.get("body", supplied), Acquire.Success, Acquire.Environment, "resource.body");
    checkContext(Out, Acquire.Error, Acquire.Environment, "resource.body");
    _ = ResourceRelease(supplied);
    return Out;
}
fn ResourceRelease(comptime supplied: anytype) type {
    const Acquire = @TypeOf(roles.get("acquire", supplied));
    const E = Acquire.Error;
    const R = Acquire.Environment;
    const A = Acquire.Success;
    const callback_release = roles.get("release", supplied);
    const Primary = inference.primaryInput(callback_release, 1, E);
    const with_env = inference.info(callback_release, "resource.release").param_types.len == 3;
    const Unit = effects.Effect(void, E, R);
    if (inference.result(callback_release, if (with_env) &.{ A, Primary, *const R } else &.{ A, Primary }, "resource.release") != Unit) @compileError("resource.release requires exact Effect(void,E,R)");
    return struct {
        fn release(value: A, primary: Primary, env: *const R) Unit {
            return if (comptime with_env) callback_release(value, primary, env) else callback_release(value, primary);
        }
    };
}

/// The acquisition determines resource, E and R; the body determines success.
pub fn resource(comptime supplied: anytype) resourceResult(supplied) {
    const Acquire = @TypeOf(roles.get("acquire", supplied));
    return Dsl(Acquire.Error, Acquire.Environment).resource(supplied);
}

fn scopedBody(comptime supplied: anytype) callbackTypeForScope(supplied) {
    return roles.get("body", supplied);
}
fn callbackTypeForScope(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "scoped", &.{"body"}, &.{});
    return @TypeOf(roles.get("body", supplied));
}

pub fn scoped(comptime supplied: anytype) inferredScopedType(supplied) {
    const Out = inferredScopedType(supplied);
    return Dsl(Out.Error, Out.Environment).scoped(supplied);
}
fn inferredScopedType(comptime supplied: anytype) type {
    const body = scopedBody(supplied);
    const Out = inference.info(body, "scoped.body").return_type orelse @compileError("scoped.body needs one explicit Dsl(E,R) anchor for a generic scope callback");
    inference.requireEffect(Out, "scoped.body");
    return Out;
}

fn nativePayload(comptime supplied: anytype, comptime R: type) type {
    const T = inference.unary(roles.get("run", supplied), *const R, "native.run");
    if (@typeInfo(T) != .error_union) @compileError("native.run requires a native error union; use sync, outcome, or factory for the matching result category");
    return @typeInfo(T).error_union.payload;
}

fn rejectEffectValue(comptime T: type, comptime operation: []const u8) void {
    if (inference.isEffect(T))
        @compileError(operation ++ " expects a native value; use factory for an Effect result");
    if (inference.isExit(T)) @compileError(operation ++ " expects a native value; use outcome for exact Exit(A,E)");
    if (@typeInfo(T) == .@"struct" and @hasDecl(T, "transaction_shape")) @compileError(operation ++ " expects a native value; execute transaction descriptions at their atomic Effect boundary");
    if (@typeInfo(T) == .@"struct" and @hasDecl(T, "Success") and @hasDecl(T, "Error") and @hasDecl(T, "Environment")) {
        if (T == @import("workflow/workflow_builder.zig").WorkflowBuilder(T.Error, T.Environment).Step(T.Success))
            @compileError(operation ++ " expects a native value; use the durable Step engine boundary");
    }
    if (@typeInfo(T) == .error_union) @compileError(operation ++ " expects a native value; use native for an error-union result");
}

pub fn checkContext(comptime T: type, comptime E: type, comptime R: type, comptime operation: []const u8) void {
    inference.requireEffect(T, operation);
    if (T.Error != E) @compileError(operation ++ " error type mismatch: expected " ++ @typeName(E) ++ "; actual " ++ @typeName(T.Error) ++ "; use explicit error mapping");
    if (T.Environment != R) @compileError(operation ++ " environment mismatch: expected " ++ @typeName(R) ++ "; actual " ++ @typeName(T.Environment) ++ "; use explicit provision/projection");
}

/// Infer both context parameters from an Effect-returning factory.
pub fn factory(comptime supplied: anytype) inference.unary(callback(supplied, "factory"), *const inference.environment(callback(supplied, "factory"), "factory"), "factory.run") {
    const run = callback(supplied, "factory");
    const R = inference.environment(run, "factory");
    const Out = inference.unary(run, *const R, "factory.run");
    inference.requireEffect(Out, "factory.run");
    return Dsl(Out.Error, R).factory(supplied);
}

fn asyncSuccess(comptime supplied: anytype, comptime R: type) type {
    const T = inference.unary(callback(supplied, "async"), *const R, "async.run");
    if (@typeInfo(T) != .@"union" or !@hasField(T, "completed")) @compileError("async.run requires the provider Async.Start result");
    return @FieldType(T, "completed").Success;
}
fn workerSuccess(comptime supplied: anytype, comptime R: type) type {
    const T = inference.result(callback(supplied, "worker"), &.{ *const R, *const @import("internal/runtime.zig").Cancellation }, "worker.run");
    if (@typeInfo(T) != .@"union" or !@hasDecl(T, "Success")) @compileError("worker.run requires exact Exit(A,E)");
    return T.Success;
}

/// Infer R from the callback; E is the sole unresolved leaf anchor.
pub fn sync(comptime E: type, comptime supplied: anytype) effects.Effect(inference.unary(callback(supplied, "sync"), *const inference.environment(callback(supplied, "sync"), "sync"), "sync.run"), E, inference.environment(callback(supplied, "sync"), "sync")) {
    return Dsl(E, inference.environment(callback(supplied, "sync"), "sync")).sync(supplied);
}
pub fn owned(env: anytype, supplied: anytype, comptime run: anytype) inference.result(run, &.{ *const @TypeOf(supplied), @TypeOf(env) }, "owned.run") {
    const Out = inference.result(run, &.{ *const @TypeOf(supplied), @TypeOf(env) }, "owned.run");
    inference.requireEffect(Out, "owned.run");
    return Dsl(Out.Error, Out.Environment).owned(env, supplied, run);
}
pub fn outcome(comptime supplied: anytype) effects.Effect(inference.unary(callback(supplied, "outcome"), *const inference.environment(callback(supplied, "outcome"), "outcome"), "outcome.run").Success, inference.unary(callback(supplied, "outcome"), *const inference.environment(callback(supplied, "outcome"), "outcome"), "outcome.run").Error, inference.environment(callback(supplied, "outcome"), "outcome")) {
    const R = inference.environment(callback(supplied, "outcome"), "outcome");
    const Exit = inference.unary(callback(supplied, "outcome"), *const R, "outcome.run");
    return Dsl(Exit.Error, R).outcome(supplied);
}

/// Store an explicitly dynamic, fully typed callback pointer as runtime data.
/// Only the static adapter is specialized; invocation selects the stored pointer.
pub fn syncPointer(comptime E: type, env: anytype, pointer: anytype) effects.Effect(pointerResult(@TypeOf(pointer), @typeInfo(@TypeOf(env)).pointer.child), E, @typeInfo(@TypeOf(env)).pointer.child) {
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const A = pointerResult(@TypeOf(pointer), R);
    comptime rejectEffectValue(A, "syncPointer.run");
    const Capture = struct { callback: @TypeOf(pointer) };
    return effects.Effect(A, E, R).fromOwned(env, Capture{ .callback = pointer }, struct {
        fn invoke(stored: *const Capture, current: *const R) outcomes.Exit(A, E) {
            return .{ .success = stored.callback(current) };
        }
    }.invoke);
}
fn pointerResult(comptime T: type, comptime R: type) type {
    if (@typeInfo(T) != .pointer or @typeInfo(@typeInfo(T).pointer.child) != .@"fn") @compileError("syncPointer.run requires a typed runtime function pointer");
    const f = @typeInfo(@typeInfo(T).pointer.child).@"fn";
    if (f.is_generic or f.return_type == null or f.param_types.len != 1 or f.param_types[0] != *const R) @compileError("syncPointer.run expected fully typed fn(*const R) A");
    return f.return_type.?;
}

/// Execute constructor factories in explicit tuple order, once each. Their
/// result types are discovered without executing their bodies at compilation.
pub fn sequence(comptime factories: anytype) Sequence(factories) {
    var values: Sequence(factories) = undefined;
    inline for (factories, 0..) |factory_function, index| values[index] = factory_function();
    return values;
}
fn Sequence(comptime factories: anytype) type {
    var types: [factories.len]type = undefined;
    inline for (factories, 0..) |factory_function, index| {
        types[index] = inference.result(factory_function, &.{}, "sequence.initializer");
        if (@typeInfo(types[index]) == .error_union) @compileError("sequence initializers must return values; adapt fallible initialization explicitly");
    }
    return @Tuple(&types);
}

fn borrowedExit(comptime run: anytype, comptime P: type, comptime R: type) type {
    const T = inference.result(run, &.{ P, *const R }, "outcomeBorrowed.run");
    if (!inference.isExit(T)) @compileError("outcomeBorrowed.run requires exact Exit(A,E)");
    return T;
}
fn borrowedAsync(comptime run: anytype, comptime P: type, comptime R: type) type {
    const T = inference.result(run, &.{ P, *const R }, "asyncBorrowed.run");
    if (@typeInfo(T) != .@"union" or !@hasField(T, "completed")) @compileError("asyncBorrowed.run requires exact Async.Start");
    const Exit = @FieldType(T, "completed");
    if (!inference.isExit(Exit) or T != @import("async.zig").Async(Exit.Success, Exit.Error).Start) @compileError("asyncBorrowed.run requires exact Async.Start");
    return Exit;
}
