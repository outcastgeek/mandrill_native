//! Resource syntax delegates acquisition, registration and close to the
//! existing bracket and Scope authorities.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const scopes = @import("../scope.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

fn acquisitionType(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "resourceBinding", &.{"acquire"}, &.{ "release", "adapter" });
    const Acquire = @TypeOf(roles.get("acquire", supplied));
    inference.requireEffect(Acquire, "resourceBinding.acquire");
    const Release = inference.result(releaseRole(supplied), &.{ Acquire.Success, outcomes.PrimaryOutcome(Acquire.Error) }, "resourceBinding.release");
    if (Release != effects.Effect(void, Acquire.Error, Acquire.Environment)) @compileError("resourceBinding.release requires exact Effect(void,E,R)");
    return Acquire;
}

fn releaseRole(comptime supplied: anytype) releaseRoleType(supplied) {
    const T = roles.recordType(supplied);
    const has_release = roles.has(T, "release");
    const has_adapter = roles.has(T, "adapter");
    if (has_release == has_adapter) @compileError("resourceBinding requires exactly one release role or explicit type adapter");
    if (has_release) return roles.get("release", supplied);
    const Adapter = roles.get("adapter", supplied);
    if (@TypeOf(Adapter) != type or @typeInfo(Adapter) != .@"struct" or !@hasDecl(Adapter, "release")) @compileError("resourceBinding.adapter requires a type with public release evidence");
    return Adapter.release;
}
fn releaseRoleType(comptime supplied: anytype) type {
    const T = roles.recordType(supplied);
    if (roles.has(T, "release") == roles.has(T, "adapter")) @compileError("resourceBinding requires exactly one release role or explicit type adapter");
    if (roles.has(T, "release")) return @TypeOf(roles.get("release", supplied));
    const Adapter = roles.get("adapter", supplied);
    if (@TypeOf(Adapter) != type or @typeInfo(Adapter) != .@"struct" or !@hasDecl(Adapter, "release")) @compileError("resourceBinding.adapter requires a type with public release evidence");
    return @TypeOf(Adapter.release);
}

/// An inert binding definition; acquisition starts only when using executes.
pub fn binding(comptime supplied: anytype) effects.ResourceBinding(acquisitionType(supplied).Success, acquisitionType(supplied).Error, acquisitionType(supplied).Environment) {
    const Acquire = acquisitionType(supplied);
    return effects.resource(roles.get("acquire", supplied), struct {
        fn release(value: Acquire.Success, primary: outcomes.PrimaryOutcome(Acquire.Error)) effects.Effect(void, Acquire.Error, Acquire.Environment) {
            return releaseRole(supplied)(value, primary);
        }
    }.release);
}

fn usingType(comptime B: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "using", &.{"body"}, &.{});
    if (@typeInfo(B) != .@"struct") @compileError("using requires a ResourceBinding or an already-owned ResourceValue");
    if (@hasDecl(B, "AlreadyOwnedResource")) {
        if (B != effects.ResourceValue(B.AlreadyOwnedResource)) @compileError("using rejects forged ResourceValue ownership evidence");
        const Out = inference.unary(roles.get("body", supplied), B, "using.body");
        inference.requireEffect(Out, "using.body");
        return Out;
    }
    if (!@hasDecl(B, "Resource") or !@hasField(B, "acquire")) @compileError("using requires a ResourceBinding or an already-owned ResourceValue");
    const Acquire = @FieldType(B, "acquire");
    inference.requireEffect(Acquire, "using.acquire");
    if (B != effects.ResourceBinding(Acquire.Success, Acquire.Error, Acquire.Environment)) @compileError("using rejects forged ResourceBinding ownership evidence");
    const Out = inference.unary(roles.get("body", supplied), effects.ResourceValue(B.Resource), "using.body");
    inference.requireEffect(Out, "using.body");
    if (Out != effects.Effect(Out.Success, Acquire.Error, Acquire.Environment)) @compileError("using.body must preserve acquisition E/R; map errors or project environments explicitly");
    return Out;
}

fn capturedUsingType(comptime B: type, comptime T: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "usingCaptured", &.{"body"}, &.{});
    if (@typeInfo(B) != .@"struct" or !@hasDecl(B, "AlreadyOwnedResource") or B != effects.ResourceValue(B.AlreadyOwnedResource)) @compileError("usingCaptured requires an already-owned ResourceValue; establish its outer resource owner first");
    const Out = inference.result(roles.get("body", supplied), &.{ *const T, B, *const R }, "usingCaptured.body");
    inference.requireEffect(Out, "usingCaptured.body");
    if (Out.Environment != R) @compileError("usingCaptured.body environment mismatch; use an explicit projection");
    return Out;
}

/// Capture application inputs for a nested body while borrowing the original
/// ResourceValue. Its enclosing resource owner must cover this body's run.
pub fn usingCaptured(env: anytype, resource: anytype, capture: anytype, comptime supplied: anytype) capturedUsingType(@TypeOf(resource), @TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied) {
    const T = @TypeOf(capture);
    const B = @TypeOf(resource);
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const Out = capturedUsingType(B, T, R, supplied);
    const Stored = struct {
        pub const effect_owned_payload = true;
        inputs: ownership.Capture(T),
        borrowed_resource: B,
        pub fn deinit(self: *@This()) void {
            self.inputs.deinit();
        }
    };
    var seed = effects.Effect(void, Out.Error, R).succeed({});
    return seed.flatMapCapturedOwned(env, Out.Success, Stored{ .inputs = .{ .value = capture }, .borrowed_resource = resource }, struct {
        fn body(stored: *const Stored, _: void, current: *const R) Out {
            return roles.get("body", supplied)(&stored.inputs.value, stored.borrowed_resource, current);
        }
    }.body);
}

/// Reusing an already-owned ResourceValue invokes its body without another
/// acquisition, registration or release. The surrounding owner stays in charge.
pub inline fn using(resource: anytype, comptime supplied: anytype) usingType(@TypeOf(resource), supplied) {
    const Out = usingType(@TypeOf(resource), supplied);
    return effects.EffectBuilder(Out.Error, Out.Environment).using(resource, roles.get("body", supplied));
}

fn capturedAcquireType(comptime T: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "scopeAcquire", &.{ "acquire", "release" }, &.{});
    const Acquire = inference.result(roles.get("acquire", supplied), &.{ *const T, *const R }, "scopeAcquire.acquire");
    inference.requireEffect(Acquire, "scopeAcquire.acquire");
    if (Acquire.Environment != R) @compileError("scopeAcquire.acquire environment mismatch; use an explicit projection");
    if (ownership.containsOwned(Acquire.Success)) @compileError("scopeAcquire must expose a borrowed resource handle; keep owning values inside their scope");
    const Release = inference.result(roles.get("release", supplied), &.{ *const T, Acquire.Success, *const R }, "scopeAcquire.release");
    if (Release != effects.Effect(void, Acquire.Error, R)) @compileError("scopeAcquire.release requires exact Effect(void,E,R)");
    return Acquire;
}

/// Move runtime inputs once into generated storage. Successful registrations
/// retain that storage independently of the returned description's lifetime.
pub fn acquire(env: anytype, scope: anytype, capture: anytype, comptime supplied: anytype) capturedAcquireType(@TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied) {
    const T = @TypeOf(capture);
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const Acquire = capturedAcquireType(T, R, supplied);
    if (@TypeOf(scope) != *scopes.Scope(Acquire.Error)) @compileError("scopeAcquire requires the existing Scope(E) owner");
    const Stored = ownership.Capture(T);
    return Acquire.scopeAcquireFactoryCapturedAllocated(env.allocator, scope, Stored{ .value = capture }, struct {
        fn start(stored: *const Stored, current: *const R) Acquire {
            return roles.get("acquire", supplied)(&stored.value, current);
        }
    }.start, struct {
        fn release(stored: *const Stored, value: Acquire.Success, current: *const R) effects.Effect(void, Acquire.Error, R) {
            return roles.get("release", supplied)(&stored.value, value, current);
        }
    }.release);
}

fn finalizerType(comptime T: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "registerFinalizer", &.{"run"}, &.{});
    const Unit = inference.result(roles.get("run", supplied), &.{ *const T, *const R }, "registerFinalizer.run");
    inference.requireEffect(Unit, "registerFinalizer.run");
    if (Unit.Success != void or Unit.Environment != R) @compileError("registerFinalizer.run requires exact Effect(void,E,R)");
    return Unit;
}

pub fn finalizer(env: anytype, scope: anytype, capture: anytype, comptime supplied: anytype) finalizerType(@TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied) {
    const T = @TypeOf(capture);
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const Unit = finalizerType(T, R, supplied);
    return acquire(env, scope, capture, .{
        .acquire = struct {
            fn start(_: *const T, _: *const R) Unit {
                return Unit.succeed({});
            }
        }.start,
        .release = struct {
            fn release(stored: *const T, _: void, current: *const R) Unit {
                return roles.get("run", supplied)(stored, current);
            }
        }.release,
    });
}

fn resourceType(comptime T: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "resourceCaptured", &.{ "acquire", "body", "release" }, &.{});
    const Acquire = inference.result(roles.get("acquire", supplied), &.{ *const T, *const R }, "resourceCaptured.acquire");
    inference.requireEffect(Acquire, "resourceCaptured.acquire");
    const Out = inference.result(roles.get("body", supplied), &.{ *const T, Acquire.Success, *const R }, "resourceCaptured.body");
    inference.requireEffect(Out, "resourceCaptured.body");
    if (Acquire.Environment != R or Out.Environment != R or Acquire.Error != Out.Error) @compileError("resourceCaptured acquisition/body E/R mismatch; map errors or project environments explicitly");
    if (inference.result(roles.get("release", supplied), &.{ *const T, Acquire.Success, inference.primaryInput(roles.get("release", supplied), 2, Out.Error), *const R }, "resourceCaptured.release") != effects.Effect(void, Out.Error, R)) @compileError("resourceCaptured.release requires exact Effect(void,E,R)");
    return Out;
}

pub fn bracket(env: anytype, capture: anytype, comptime supplied: anytype) resourceType(@TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied) {
    const T = @TypeOf(capture);
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const Out = resourceType(T, R, supplied);
    const Acquire = inference.result(roles.get("acquire", supplied), &.{ *const T, *const R }, "resourceCaptured.acquire");
    const Stored = ownership.Capture(T);
    const Adapter = struct {
        fn acquire(stored: *const Stored, current: *const R) Acquire {
            return roles.get("acquire", supplied)(&stored.value, current);
        }
        fn body(stored: *const Stored, value: Acquire.Success, current: *const R) Out {
            return roles.get("body", supplied)(&stored.value, value, current);
        }
        fn release(stored: *const Stored, value: Acquire.Success, primary: inference.primaryInput(roles.get("release", supplied), 2, Out.Error), current: *const R) effects.Effect(void, Out.Error, R) {
            return roles.get("release", supplied)(&stored.value, value, primary, current);
        }
    };
    return Acquire.acquireReleaseFactoryCapturedOwned(env, Stored{ .value = capture }, Adapter.acquire, Adapter.body, Adapter.release);
}

fn scopedType(comptime T: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "scopedCaptured", &.{"body"}, &.{});
    const Body = inference.info(roles.get("body", supplied), "scopedCaptured.body");
    const Out = Body.return_type orelse @compileError("scopedCaptured.body requires a concrete Effect return or a Dsl(E,R) anchor");
    inference.requireEffect(Out, "scopedCaptured.body");
    if (Out.Environment != R) @compileError("scopedCaptured.body environment mismatch; use an explicit projection");
    if (inference.result(roles.get("body", supplied), &.{ *const T, *scopes.Scope(Out.Error), *const R }, "scopedCaptured.body") != Out) @compileError("scopedCaptured.body result mismatch");
    return Out;
}

pub fn scoped(env: anytype, capture: anytype, comptime supplied: anytype) scopedType(@TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied) {
    const T = @TypeOf(capture);
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const Out = scopedType(T, R, supplied);
    const Stored = ownership.Capture(T);
    return Out.scopedCaptured(env, Stored{ .value = capture }, struct {
        fn body(stored: *const Stored, scope: *scopes.Scope(Out.Error), current: *const R) Out {
            return roles.get("body", supplied)(&stored.value, scope, current);
        }
    }.body);
}
