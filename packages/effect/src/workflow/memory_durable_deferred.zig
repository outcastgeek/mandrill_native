//! Volatile deferred records with atomic observe/complete transitions.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const deferred = @import("durable_deferred.zig");
const workflow = @import("workflow_types.zig");

/// Construct an in-memory durable deferred provider with caller-owned runtime resources.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): in-memory provider for durable deferred completion records.
pub fn MemoryDurableDeferred(comptime R: type) type {
    return struct {
        const Self = @This();
        const Store = deferred.Store(R, void, .@"volatile");
        const Failure = Store.Failure;
        const Bytes = workflow.Bytes;
        const Item = struct {
            execution_key: Bytes,
            name: Bytes,
            version: u32,
            result: ?Bytes = null,
            generation: u64 = 0,
            waiting: bool = false,

            fn deinit(self: *Item) void {
                self.execution_key.deinit();
                self.name.deinit();
                if (self.result) |*value| value.deinit();
                self.* = undefined;
            }
        };
        const IdentityCapture = struct { owner: *Self, identity: deferred.Identity };
        const CompleteCapture = struct { owner: *Self, identity: deferred.Identity, encoded: []const u8 };
        const ReleaseCapture = struct { owner: *Self, registration: deferred.RegistrationView };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        items: std.array_list.Managed(Item),
        closed: bool = false,

        /// Create an in-memory durable-deferred store using the caller environment.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`Self`): a durable-deferred store bound to the supplied environment.
        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("memory durable deferred requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("memory durable deferred requires caller I/O in R");
            return .{ .allocator = env.allocator, .io = env.io, .items = std.array_list.Managed(Item).init(env.allocator) };
        }

        /// Expose this implementation through its capability interface.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`Store`): A capability adapter referencing the selected implementation; keep its backing state alive while in use.
        pub fn capability(self: *Self) Store {
            return Store.fromProvider(self, &.{ .observe = observeEffect, .complete = completeEffect, .release_wait = releaseEffect });
        }

        fn copy(allocator: std.mem.Allocator, value: []const u8) error{OutOfMemory}!Bytes {
            return .{ .items = try allocator.dupe(u8, value), .allocator = allocator };
        }

        fn equal(item: *const Item, identity: deferred.Identity) bool {
            return item.version == identity.version and
                std.mem.eql(u8, item.execution_key.items, identity.execution.encoded) and
                std.mem.eql(u8, item.name.items, identity.name);
        }

        fn find(self: *Self, identity: deferred.Identity) ?*Item {
            for (self.items.items) |*item| if (equal(item, identity)) return item;
            return null;
        }

        fn prepared(self: *Self, identity: deferred.Identity) error{OutOfMemory}!Item {
            var execution_key = try copy(self.allocator, identity.execution.encoded);
            errdefer execution_key.deinit();
            return .{ .execution_key = execution_key, .name = try copy(self.allocator, identity.name), .version = identity.version };
        }

        fn registration(allocator: std.mem.Allocator, identity: deferred.Identity, generation: u64) error{OutOfMemory}!deferred.Registration {
            var execution_key = try copy(allocator, identity.execution.encoded);
            errdefer execution_key.deinit();
            return .{ .execution_key = execution_key, .name = try copy(allocator, identity.name), .version = identity.version, .generation = generation };
        }

        fn observeEffect(data: ?*anyopaque, env: *const R, identity: deferred.Identity) Store.ObserveEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ObserveEffect.fromOwned(env, IdentityCapture{ .owner = owner, .identity = identity }, struct {
                fn run(capture: *const IdentityCapture, read_env: *const R) outcomes.Exit(deferred.Observed, Failure) {
                    return capture.owner.observeNow(read_env.allocator, capture.identity);
                }
            }.run);
        }

        fn observeNow(self: *Self, allocator: std.mem.Allocator, identity: deferred.Identity) outcomes.Exit(deferred.Observed, Failure) {
            if (!identity.valid()) return .{ .fail = .{ .operation = .invalid_identity } };
            var candidate = self.prepared(identity) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_candidate = false;
            defer if (!keep_candidate) candidate.deinit();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            if (self.find(identity)) |item| {
                if (item.result) |*value| return .{ .success = .{ .ready = value.clone() catch
                    return .{ .die = outcomes.Defect.out_of_memory } } };
                const next_generation = std.math.add(u64, item.generation, 1) catch
                    return .{ .fail = .{ .operation = .generation_exhausted } };
                const token = registration(allocator, identity, next_generation) catch
                    return .{ .die = outcomes.Defect.out_of_memory };
                item.generation = next_generation;
                item.waiting = true;
                return .{ .success = .{ .waiting = token } };
            }
            const first_generation: u64 = 1;
            const token = registration(allocator, identity, first_generation) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            self.items.append(candidate) catch {
                var abandoned = token;
                abandoned.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            keep_candidate = true;
            const item = &self.items.items[self.items.items.len - 1];
            item.generation = first_generation;
            item.waiting = true;
            return .{ .success = .{ .waiting = token } };
        }

        fn completeEffect(data: ?*anyopaque, env: *const R, identity: deferred.Identity, encoded: []const u8) Store.CompleteEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.CompleteEffect.fromOwned(env, CompleteCapture{ .owner = owner, .identity = identity, .encoded = encoded }, struct {
                fn run(capture: *const CompleteCapture, _: *const R) outcomes.Exit(deferred.Completion, Failure) {
                    return capture.owner.completeNow(capture.identity, capture.encoded);
                }
            }.run);
        }

        fn completeNow(self: *Self, identity: deferred.Identity, encoded: []const u8) outcomes.Exit(deferred.Completion, Failure) {
            if (!identity.valid()) return .{ .fail = .{ .operation = .invalid_identity } };
            var candidate = self.prepared(identity) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_candidate = false;
            defer if (!keep_candidate) candidate.deinit();
            var result_bytes = copy(self.allocator, encoded) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_result = false;
            defer if (!keep_result) result_bytes.deinit();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            if (self.find(identity)) |item| {
                if (item.result) |value| {
                    if (!std.mem.eql(u8, value.items, encoded)) return .{ .success = .conflicting_result };
                    return .{ .success = .already_recorded };
                }
                const wake = item.waiting;
                item.result = result_bytes;
                item.waiting = false;
                keep_result = true;
                return .{ .success = .{ .recorded = .{ .wake_registered_execution = wake } } };
            }
            candidate.result = result_bytes;
            keep_result = true;
            self.items.append(candidate) catch {
                keep_result = false;
                candidate.result = null;
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            keep_candidate = true;
            return .{ .success = .{ .recorded = .{ .wake_registered_execution = false } } };
        }

        fn releaseEffect(data: ?*anyopaque, env: *const R, token: deferred.RegistrationView) Store.ReleaseEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ReleaseEffect.fromOwned(env, ReleaseCapture{ .owner = owner, .registration = token }, struct {
                fn run(capture: *const ReleaseCapture, _: *const R) outcomes.Exit(void, Failure) {
                    return capture.owner.releaseNow(capture.registration);
                }
            }.run);
        }

        fn releaseNow(self: *Self, token: deferred.RegistrationView) outcomes.Exit(void, Failure) {
            if (!token.identity.valid()) return .{ .fail = .{ .operation = .invalid_identity } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            if (self.find(token.identity)) |item| {
                if (item.generation == token.generation) item.waiting = false;
            }
            return .{ .success = {} };
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            for (self.items.items) |*item| item.deinit();
            self.items.deinit();
            self.mutex.unlock(self.io);
            self.* = undefined;
        }
    };
}
