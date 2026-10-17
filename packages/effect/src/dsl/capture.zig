//! Checked consume-once inputs. Handles borrow a live explicit owner; copying
//! a handle never frees or duplicates its control storage. Join work before
//! destroying that owner, as with the package's other provider capabilities.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const rt = @import("../internal/runtime.zig");
const inference = @import("inference.zig");
const ownership = @import("ownership.zig");

pub fn OneShot(comptime T: type) type {
    comptime ownership.validate(T);
    return struct {
        const Self = @This();
        allocator: std.mem.Allocator,
        available: std.atomic.Value(bool) = .init(true),
        value: T,
        pub const Handle = struct {
            owner: *Self,
            pub fn effect(self: Handle, comptime run: anytype) effects.Effect(RunSuccess(run), RunError(run), inference.environment(run, "one-shot")) {
                const E = RunError(run);
                const R = inference.environment(run, "one-shot");
                const Out = effects.Effect(RunSuccess(run), E, R);
                return Out.fromBorrowed(Self, self.owner, struct {
                    fn invoke(borrowed: *const Self, env: *const R) outcomes.Exit(Out.Success, E) {
                        const owner = @constCast(borrowed);
                        if (!owner.available.swap(false, .acq_rel)) return .{ .die = .{ .kind = .invalid_configuration, .message = "one-shot input has already been consumed" } };
                        const moved = owner.value;
                        owner.value = undefined;
                        return run(moved, env);
                    }
                }.invoke);
            }
            /// Consumption occurs inside admitted CPU work. A rejected provider
            /// never invokes this callback and leaves the input available.
            pub fn onCpu(self: Handle, comptime run: anytype) effects.Effect(RunSuccess(run), RunError(run), inference.environment(run, "one-shot")) {
                const E = RunError(run);
                const R = inference.environment(run, "one-shot");
                comptime ownership.requireConcurrent(T, "OneShot.onCpu");
                const Out = effects.Effect(RunSuccess(run), E, R);
                return Out.onCpuBorrowed(Self, self.owner, struct {
                    fn invoke(borrowed: *const Self, env: *const R, _: *const rt.Cancellation) outcomes.Exit(Out.Success, E) {
                        const owner = @constCast(borrowed);
                        if (!owner.available.swap(false, .acq_rel)) return .{ .die = .{ .kind = .invalid_configuration, .message = "one-shot input has already been consumed" } };
                        const moved = owner.value;
                        owner.value = undefined;
                        return run(moved, env);
                    }
                }.invoke);
            }
        };
        pub fn create(allocator: std.mem.Allocator, value: T) error{OutOfMemory}!*Self {
            const owner = allocator.create(Self) catch |err| {
                var abandoned = value;
                ownership.discard(&abandoned);
                return err;
            };
            owner.* = .{ .allocator = allocator, .value = value };
            return owner;
        }
        pub fn handle(self: *Self) Handle {
            return .{ .owner = self };
        }
        pub fn isAvailable(self: *const Self) bool {
            return self.available.load(.acquire);
        }
        pub fn deinit(self: *Self) void {
            if (self.available.swap(false, .acq_rel)) ownership.discard(&self.value);
            self.allocator.destroy(self);
        }
    };
}
fn RunSuccess(comptime run: anytype) type {
    const T = @typeInfo(@TypeOf(run)).@"fn".return_type orelse @compileError("one-shot callback requires a typed Exit result");
    if (!@hasDecl(T, "Success")) @compileError("one-shot callback requires a typed Exit result");
    return T.Success;
}

fn RunError(comptime run: anytype) type {
    return @typeInfo(@TypeOf(run)).@"fn".return_type.?.Error;
}
