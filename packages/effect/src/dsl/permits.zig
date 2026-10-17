//! Permit syntax infers body channels and retains explicit admission mapping.
const gates = @import("../semaphore.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

fn validate(comptime R: type, comptime Out: type, comptime supplied: anytype) void {
    roles.validate(roles.recordType(supplied), "withPermit", &.{ "select", "map_error", "body" }, &.{"key"});
    inference.requireEffect(Out, "withPermit.body");
    if (Out.Environment != R) @compileError("withPermit.body environment mismatch; use an explicit projection");
    const Gate = inference.unary(roles.get("select", supplied), *const R, "withPermit.select");
    if (Gate != *gates.Semaphore and Gate != *gates.PartitionedSemaphore) @compileError("withPermit.select must select the existing Semaphore or PartitionedSemaphore");
    const partitioned = Gate == *gates.PartitionedSemaphore;
    if (partitioned != roles.has(roles.recordType(supplied), "key")) @compileError("withPermit.key is required exactly for a PartitionedSemaphore");
    if (partitioned and inference.unary(roles.get("key", supplied), *const R, "withPermit.key") != []const u8) @compileError("withPermit.key requires a borrowed []const u8");
    if (inference.unary(roles.get("map_error", supplied), if (partitioned) gates.PartitionError else gates.Error, "withPermit.map_error") != Out.Error) @compileError("withPermit.map_error must return the exact body E");
}

fn bodyType(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "withPermit", &.{ "select", "map_error", "body" }, &.{"key"});
    const R = inference.environment(roles.get("body", supplied), "withPermit.body");
    const Out = inference.unary(roles.get("body", supplied), *const R, "withPermit.body");
    validate(R, Out, supplied);
    return Out;
}
pub fn with(comptime supplied: anytype) bodyType(supplied) {
    const Out = bodyType(supplied);
    const R = Out.Environment;
    const Adapter = struct {
        fn body(_: void, env: *const R) Out {
            return roles.get("body", supplied)(env);
        }
    };
    if (comptime roles.has(roles.recordType(supplied), "key")) return gates.PartitionedSemaphore.withPermit(R, Out.Success, Out.Error, roles.get("select", supplied), roles.get("key", supplied), roles.get("map_error", supplied), Adapter.body);
    return gates.Semaphore.withPermit(R, Out.Success, Out.Error, roles.get("select", supplied), roles.get("map_error", supplied), Adapter.body);
}

fn capturedType(comptime T: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "withPermitCaptured", &.{ "select", "map_error", "body" }, &.{"key"});
    const Out = inference.result(roles.get("body", supplied), &.{ *const T, *const R }, "withPermitCaptured.body");
    validate(R, Out, supplied);
    return Out;
}
pub fn captured(env: anytype, capture: anytype, comptime supplied: anytype) capturedType(@TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied) {
    const T = @TypeOf(capture);
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const Out = capturedType(T, R, supplied);
    const Stored = ownership.Capture(T);
    const Adapter = struct {
        fn body(stored: *const Stored, current: *const R) Out {
            return roles.get("body", supplied)(&stored.value, current);
        }
    };
    if (comptime roles.has(roles.recordType(supplied), "key")) return gates.PartitionedSemaphore.withPermitCaptured(R, Out.Success, Out.Error, env, Stored{ .value = capture }, roles.get("select", supplied), roles.get("key", supplied), roles.get("map_error", supplied), Adapter.body);
    return gates.Semaphore.withPermitCaptured(R, Out.Success, Out.Error, env, Stored{ .value = capture }, roles.get("select", supplied), roles.get("map_error", supplied), Adapter.body);
}

pub fn semaphore(env: anytype, capacity: usize) @TypeOf(gates.Semaphore.init(env, capacity)) {
    return gates.Semaphore.init(env, capacity);
}
pub fn semaphoreStatic(env: anytype, comptime capacity: usize) @TypeOf(gates.Semaphore.init(env, capacity)) {
    if (capacity == 0) @compileError("semaphore capacity must be positive");
    return gates.Semaphore.init(env, capacity);
}
pub fn partitions(env: anytype, maximum: usize, permits_per_partition: usize) @TypeOf(gates.PartitionedSemaphore.init(env, maximum, permits_per_partition)) {
    return gates.PartitionedSemaphore.init(env, maximum, permits_per_partition);
}
pub fn partitionsStatic(env: anytype, comptime maximum: usize, comptime permits_per_partition: usize) @TypeOf(gates.PartitionedSemaphore.init(env, maximum, permits_per_partition)) {
    if (maximum == 0 or permits_per_partition == 0) @compileError("partitioned semaphore bounds must be positive");
    return gates.PartitionedSemaphore.init(env, maximum, permits_per_partition);
}
