//! Typed retained storage for generated runtime definitions. Retaining storage
//! never clones application configuration or adds execution services.
const std = @import("std");
const ownership = @import("../dsl/ownership.zig");

pub fn Owner(comptime C: type) type {
    comptime ownership.validate(C);
    return struct {
        const Self = @This();
        pub const effect_owned_payload = true;
        pub const generated_capture_storage = struct {
            pub const Configuration = C;
            pub const Payload: ?type = null;
        };
        const State = struct {
            refs: std.atomic.Value(usize) = .init(1),
            allocator: std.mem.Allocator,
            value: C,
        };
        state: *State,
        pub fn create(memory: std.mem.Allocator, value: C) error{OutOfMemory}!Self {
            const state = memory.create(State) catch |err| {
                var abandoned = value;
                ownership.discard(&abandoned);
                return err;
            };
            state.* = .{ .allocator = memory, .value = value };
            return .{ .state = state };
        }
        pub fn retain(self: Self) Self {
            _ = self.state.refs.fetchAdd(1, .monotonic);
            return self;
        }
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return self.retain();
        }
        pub fn borrow(self: Self) *const C {
            return &self.state.value;
        }
        pub fn allocator(self: Self) std.mem.Allocator {
            return self.state.allocator;
        }
        pub fn deinit(self: *Self) void {
            if (self.state.refs.fetchSub(1, .acq_rel) == 1) {
                const memory = self.state.allocator;
                ownership.discard(&self.state.value);
                memory.destroy(self.state);
            }
            self.* = undefined;
        }
        pub fn Held(comptime T: type) type {
            comptime ownership.validate(T);
            return struct {
                pub const effect_owned_payload = true;
                pub const generated_capture_storage = struct {
                    pub const Configuration = C;
                    pub const Payload: ?type = T;
                };
                value: T,
                owner: Self,
                pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                    return .{ .value = try ownership.cloneValue(self.value), .owner = self.owner.retain() };
                }
                pub fn deinit(self: *@This()) void {
                    ownership.discard(&self.value);
                    self.owner.deinit();
                }
            };
        }
        pub fn keep(self: Self, value: anytype) Held(@TypeOf(value)) {
            return .{ .value = value, .owner = self.retain() };
        }
    };
}

pub fn concurrencyShape(comptime T: type) ?type {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "generated_capture_storage")) return null;
    const Shape = T.generated_capture_storage;
    const Storage = Owner(Shape.Configuration);
    if (T != (if (Shape.Payload) |Payload| Storage.Held(Payload) else Storage)) @compileError("capture storage rejects forged generated ownership evidence");
    return Shape;
}
