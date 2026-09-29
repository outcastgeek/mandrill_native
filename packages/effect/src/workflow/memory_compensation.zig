//! Volatile compensation journal used by the independent memory engine.
//! Ordering, identity deduplication and result recording are atomic under
//! one short provider lock; no user action executes while it is held.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const types = @import("workflow_types.zig");
const compensation = @import("compensation.zig");

pub fn MemoryCompensation(comptime R: type) type {
    return struct {
        const Self = @This();
        const Journal = compensation.Journal(R, void, .@"volatile");
        const Failure = Journal.Failure;
        const Bytes = types.Bytes;
        const Item = struct {
            execution_key: Bytes,
            name: Bytes,
            version: u32,
            ordinal: u64,
            attempt: u64 = 0,
            encoded_exit: ?Bytes = null,

            fn deinit(self: *Item) void {
                self.execution_key.deinit();
                self.name.deinit();
                if (self.encoded_exit) |*value| value.deinit();
                self.* = undefined;
            }
        };
        const IdentityCapture = struct { owner: *Self, identity: compensation.Identity };
        const AttemptCapture = struct { owner: *Self, attempt: compensation.Attempt };
        const RecordCapture = struct { owner: *Self, attempt: compensation.Attempt, encoded: []const u8 };
        const ListCapture = struct { owner: *Self, execution: types.ExecutionId, before: ?u64, limit: usize };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        items: std.array_list.Managed(Item),
        closed: bool = false,

        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("memory compensation requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("memory compensation requires caller I/O in R");
            return .{ .allocator = env.allocator, .io = env.io, .items = std.array_list.Managed(Item).init(env.allocator) };
        }

        pub fn capability(self: *Self) Journal {
            return Journal.fromProvider(self, &.{ .register = registerEffect, .lookup = lookupEffect, .record = recordEffect, .pending_reverse = pendingEffect });
        }

        fn copy(allocator: std.mem.Allocator, value: []const u8) error{OutOfMemory}!Bytes {
            return .{ .items = try allocator.dupe(u8, value), .allocator = allocator };
        }

        fn matches(item: *const Item, identity: compensation.Identity) bool {
            return item.version == identity.version and
                std.mem.eql(u8, item.execution_key.items, identity.execution.encoded) and
                std.mem.eql(u8, item.name.items, identity.name);
        }

        fn find(self: *Self, identity: compensation.Identity) ?*Item {
            for (self.items.items) |*item| if (matches(item, identity)) return item;
            return null;
        }

        fn makeItem(self: *Self, identity: compensation.Identity, ordinal: u64) error{OutOfMemory}!Item {
            var execution_key = try copy(self.allocator, identity.execution.encoded);
            errdefer execution_key.deinit();
            return .{ .execution_key = execution_key, .name = try copy(self.allocator, identity.name), .version = identity.version, .ordinal = ordinal };
        }

        fn registerEffect(data: ?*anyopaque, env: *const R, identity: compensation.Identity) Journal.RegisterEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Journal.RegisterEffect.fromOwned(env, IdentityCapture{ .owner = owner, .identity = identity }, struct {
                fn run(capture: *const IdentityCapture, _: *const R) outcomes.Exit(compensation.Registration, Failure) {
                    return capture.owner.registerNow(capture.identity);
                }
            }.run);
        }

        fn registerNow(self: *Self, identity: compensation.Identity) outcomes.Exit(compensation.Registration, Failure) {
            if (!identity.valid()) return .{ .fail = .{ .operation = .invalid_identity } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            if (self.find(identity)) |item| return .{ .success = .{ .already_registered = item.ordinal } };
            var last: u64 = 0;
            for (self.items.items) |*item| {
                if (!std.mem.eql(u8, item.execution_key.items, identity.execution.encoded)) continue;
                if (std.mem.eql(u8, item.name.items, identity.name))
                    return .{ .success = .conflicting_definition };
                if (item.ordinal > last) last = item.ordinal;
            }
            const ordinal = std.math.add(u64, last, 1) catch
                return .{ .fail = .{ .operation = .ordinal_exhausted } };
            var item = self.makeItem(identity, ordinal) catch return .{ .die = outcomes.Defect.out_of_memory };
            self.items.append(item) catch {
                item.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            return .{ .success = .{ .registered = ordinal } };
        }

        fn lookupEffect(data: ?*anyopaque, env: *const R, attempt: compensation.Attempt) Journal.LookupEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Journal.LookupEffect.fromOwned(env, AttemptCapture{ .owner = owner, .attempt = attempt }, struct {
                fn run(capture: *const AttemptCapture, read_env: *const R) outcomes.Exit(?compensation.Recorded, Failure) {
                    return capture.owner.lookupNow(read_env.allocator, capture.attempt);
                }
            }.run);
        }

        fn lookupNow(self: *Self, allocator: std.mem.Allocator, attempt: compensation.Attempt) outcomes.Exit(?compensation.Recorded, Failure) {
            if (!attempt.identity.valid() or attempt.ordinal == 0 or attempt.attempt == 0)
                return .{ .fail = .{ .operation = .invalid_identity } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(attempt.identity) orelse return .{ .success = null };
            if (item.ordinal != attempt.ordinal or
                (item.encoded_exit != null and item.attempt != attempt.attempt))
                return .{ .fail = .{ .operation = .stale_attempt } };
            if (item.encoded_exit) |encoded| return .{ .success = .{
                .ordinal = item.ordinal,
                .attempt = item.attempt,
                .encoded_exit = copy(allocator, encoded.items) catch return .{ .die = outcomes.Defect.out_of_memory },
            } };
            return .{ .success = null };
        }

        fn recordEffect(data: ?*anyopaque, env: *const R, attempt: compensation.Attempt, encoded: []const u8) Journal.RecordEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Journal.RecordEffect.fromOwned(env, RecordCapture{ .owner = owner, .attempt = attempt, .encoded = encoded }, struct {
                fn run(capture: *const RecordCapture, _: *const R) outcomes.Exit(compensation.Recording, Failure) {
                    return capture.owner.recordNow(capture.attempt, capture.encoded);
                }
            }.run);
        }

        fn recordNow(self: *Self, attempt: compensation.Attempt, encoded: []const u8) outcomes.Exit(compensation.Recording, Failure) {
            if (!attempt.identity.valid() or attempt.ordinal == 0 or attempt.attempt == 0 or encoded.len == 0)
                return .{ .fail = .{ .operation = .invalid_identity } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(attempt.identity) orelse return .{ .success = .stale_attempt };
            if (item.ordinal != attempt.ordinal or attempt.attempt < item.attempt) {
                return .{ .success = .stale_attempt };
            }
            if (item.encoded_exit) |previous| {
                if (item.attempt != attempt.attempt) return .{ .success = .stale_attempt };
                return .{ .success = if (std.mem.eql(u8, previous.items, encoded))
                    .already_recorded
                else
                    .conflicting_outcome };
            }
            const candidate = copy(self.allocator, encoded) catch return .{ .die = outcomes.Defect.out_of_memory };
            item.attempt = attempt.attempt;
            item.encoded_exit = candidate;
            return .{ .success = .recorded };
        }

        fn pendingEffect(data: ?*anyopaque, env: *const R, execution: types.ExecutionId, before: ?u64, limit: usize) Journal.PendingEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Journal.PendingEffect.fromOwned(env, ListCapture{ .owner = owner, .execution = execution, .before = before, .limit = limit }, struct {
                fn run(capture: *const ListCapture, read_env: *const R) outcomes.Exit(compensation.Page, Failure) {
                    return capture.owner.pendingNow(read_env.allocator, capture.execution, capture.before, capture.limit);
                }
            }.run);
        }

        fn pendingNow(self: *Self, allocator: std.mem.Allocator, execution: types.ExecutionId, before: ?u64, limit: usize) outcomes.Exit(compensation.Page, Failure) {
            if (execution.validate() == .err or limit == 0)
                return .{ .fail = .{ .operation = .invalid_identity } };
            var result = std.array_list.Managed(compensation.Pending).init(allocator);
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) {
                result.deinit();
                return .{ .fail = .{ .operation = .unavailable } };
            }
            var ceiling = before;
            var more = false;
            while (true) {
                var best: ?*const Item = null;
                for (self.items.items) |*item| {
                    if (!std.mem.eql(u8, item.execution_key.items, execution.encoded) or
                        item.encoded_exit != null or (ceiling != null and item.ordinal >= ceiling.?)) continue;
                    if (best == null or item.ordinal > best.?.ordinal) best = item;
                }
                const item = best orelse break;
                if (result.items.len == limit) {
                    more = true;
                    break;
                }
                const name = copy(allocator, item.name.items) catch {
                    for (result.items) |*pending| pending.deinit();
                    result.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                result.append(.{ .name = name, .version = item.version, .ordinal = item.ordinal, .attempt = item.attempt }) catch {
                    var discarded = name;
                    discarded.deinit();
                    for (result.items) |*pending| pending.deinit();
                    result.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                ceiling = item.ordinal;
            }
            return .{ .success = .{ .items = .{ .items = result.toOwnedSlice() catch {
                for (result.items) |*pending| pending.deinit();
                result.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            }, .allocator = allocator }, .more = more } };
        }

        pub fn shutdown(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            self.mutex.unlock(self.io);
        }

        pub fn deinit(self: *Self) void {
            self.shutdown();
            for (self.items.items) |*item| item.deinit();
            self.items.deinit();
            self.* = undefined;
        }
    };
}
