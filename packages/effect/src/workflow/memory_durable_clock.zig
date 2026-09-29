//! Volatile deadline store with atomic local claim and acknowledgement.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const clock = @import("durable_clock.zig");
const workflow = @import("workflow_types.zig");

pub fn MemoryDurableClock(comptime R: type, comptime now: fn (*const R) deadlines.WallInstant) type {
    return struct {
        const Self = @This();
        const Store = clock.Store(R, void, .@"volatile");
        const Failure = Store.Error;
        const Bytes = workflow.Bytes;
        const State = enum { scheduled, claimed, acknowledged, cancelled };
        const Item = struct {
            execution_key: Bytes,
            occurrence: Bytes,
            due_at: deadlines.WallInstant,
            state: State = .scheduled,
            owner_id: ?Bytes = null,
            generation: u64 = 0,
            attempt: u64 = 0,
            lease_until: deadlines.Deadline = .never,

            fn deinit(self: *Item) void {
                self.execution_key.deinit();
                self.occurrence.deinit();
                if (self.owner_id) |*value| value.deinit();
                self.* = undefined;
            }
        };
        const ScheduleCapture = struct { owner: *Self, value: clock.Schedule };
        const IdentityCapture = struct { owner: *Self, identity: clock.Identity };
        const ClaimCapture = struct { owner: *Self, policy: clock.ClaimPolicy };
        const AckCapture = struct { owner: *Self, token: clock.Token };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        items: std.array_list.Managed(Item),
        closed: bool = false,

        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("memory durable clock requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("memory durable clock requires caller I/O in R");
            return .{ .allocator = env.allocator, .io = env.io, .items = std.array_list.Managed(Item).init(env.allocator) };
        }

        pub fn capability(self: *Self) Store {
            return Store.fromProvider(self, &.{ .schedule = scheduleEffect, .cancel = cancelEffect, .claim_due = claimEffect, .acknowledge = ackEffect });
        }

        fn copy(allocator: std.mem.Allocator, source: []const u8) error{OutOfMemory}!Bytes {
            return .{ .items = try allocator.dupe(u8, source), .allocator = allocator };
        }

        fn find(self: *Self, identity: clock.Identity) ?*Item {
            for (self.items.items) |*item| if (std.mem.eql(u8, item.execution_key.items, identity.execution.encoded) and
                std.mem.eql(u8, item.occurrence.items, identity.occurrence)) return item;
            return null;
        }

        fn scheduleEffect(data: ?*anyopaque, env: *const R, value: clock.Schedule) Store.ScheduleEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ScheduleEffect.fromOwned(env, ScheduleCapture{ .owner = owner, .value = value }, struct {
                fn run(capture: *const ScheduleCapture, _: *const R) outcomes.Exit(clock.ScheduleResult, Failure) {
                    return capture.owner.scheduleNow(capture.value);
                }
            }.run);
        }

        fn scheduleNow(self: *Self, value: clock.Schedule) outcomes.Exit(clock.ScheduleResult, Failure) {
            if (!value.identity.valid()) return .{ .fail = .{ .operation = .malformed } };
            var execution_key = copy(self.allocator, value.identity.execution.encoded) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_execution = false;
            defer if (!keep_execution) execution_key.deinit();
            var occurrence = copy(self.allocator, value.identity.occurrence) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_occurrence = false;
            defer if (!keep_occurrence) occurrence.deinit();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            if (self.find(value.identity)) |existing| {
                if (existing.due_at.unix_nanoseconds != value.due_at.unix_nanoseconds)
                    return .{ .success = .conflicting_deadline };
                if (existing.state == .cancelled) return .{ .success = .conflicting_deadline };
                return .{ .success = .already_scheduled };
            }
            self.items.append(.{ .execution_key = execution_key, .occurrence = occurrence, .due_at = value.due_at }) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            keep_execution = true;
            keep_occurrence = true;
            return .{ .success = .scheduled };
        }

        fn cancelEffect(data: ?*anyopaque, env: *const R, identity: clock.Identity) Store.CancelEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.CancelEffect.fromOwned(env, IdentityCapture{ .owner = owner, .identity = identity }, struct {
                fn run(capture: *const IdentityCapture, _: *const R) outcomes.Exit(clock.CancelResult, Failure) {
                    return capture.owner.cancelNow(capture.identity);
                }
            }.run);
        }

        fn cancelNow(self: *Self, identity: clock.Identity) outcomes.Exit(clock.CancelResult, Failure) {
            if (!identity.valid()) return .{ .fail = .{ .operation = .malformed } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(identity) orelse return .{ .success = .absent };
            if (item.state == .cancelled) return .{ .success = .already_cancelled };
            if (item.state == .acknowledged) return .{ .success = .absent };
            item.state = .cancelled;
            return .{ .success = .cancelled };
        }

        fn claimEffect(data: ?*anyopaque, env: *const R, policy: clock.ClaimPolicy) Store.ClaimEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ClaimEffect.fromOwned(env, ClaimCapture{ .owner = owner, .policy = policy }, struct {
                fn run(capture: *const ClaimCapture, read_env: *const R) outcomes.Exit(clock.ClaimResult, Failure) {
                    return capture.owner.claimNow(read_env.allocator, capture.policy, now(read_env));
                }
            }.run);
        }

        fn eligible(item: *const Item, policy: clock.ClaimPolicy, instant: deadlines.WallInstant) bool {
            if (item.due_at.unix_nanoseconds > instant.unix_nanoseconds) return false;
            if (item.state == .scheduled) return policy.generation >= item.generation;
            if (item.state != .claimed or policy.generation < item.generation) return false;
            if (policy.generation > item.generation) return true;
            const owner = item.owner_id orelse return false;
            return std.mem.eql(u8, owner.items, policy.owner_id) and item.lease_until.isDue(instant);
        }

        fn claimCopy(allocator: std.mem.Allocator, item: *const Item, policy: clock.ClaimPolicy) error{OutOfMemory}!clock.Claimed {
            var execution_key = try copy(allocator, item.execution_key.items);
            errdefer execution_key.deinit();
            var occurrence = try copy(allocator, item.occurrence.items);
            errdefer occurrence.deinit();
            return .{ .execution_key = execution_key, .occurrence = occurrence, .due_at = item.due_at, .owner_id = try copy(allocator, policy.owner_id), .generation = policy.generation, .attempt = item.attempt + 1 };
        }

        fn claimNow(self: *Self, allocator: std.mem.Allocator, policy: clock.ClaimPolicy, instant: deadlines.WallInstant) outcomes.Exit(clock.ClaimResult, Failure) {
            if (!policy.valid() or policy.lease_until.isDue(instant)) return .{ .fail = .{ .operation = .invalid_policy } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            var count: usize = 0;
            for (self.items.items) |*item| {
                if (eligible(item, policy, instant)) count += 1;
                if (count == policy.max_items) break;
            }
            if (count == 0) {
                for (self.items.items) |item| {
                    if (item.due_at.unix_nanoseconds > instant.unix_nanoseconds or
                        (item.state != .scheduled and item.state != .claimed)) continue;
                    if (policy.generation < item.generation) return .{ .success = .ownership_lost };
                    if (item.state == .claimed and policy.generation == item.generation) {
                        const owner = item.owner_id orelse continue;
                        if (!std.mem.eql(u8, owner.items, policy.owner_id)) return .{ .success = .ownership_lost };
                    }
                }
                return .{ .success = .none_due };
            }
            const Staged = struct { index: usize, owner_id: Bytes };
            const staged = self.allocator.alloc(Staged, count) catch return .{ .die = outcomes.Defect.out_of_memory };
            defer self.allocator.free(staged);
            const claimed = allocator.alloc(clock.Claimed, count) catch return .{ .die = outcomes.Defect.out_of_memory };
            var prepared: usize = 0;
            var committed = false;
            defer if (!committed) {
                for (staged[0..prepared]) |*item| item.owner_id.deinit();
                for (claimed[0..prepared]) |*item| item.deinit();
                allocator.free(claimed);
            };
            for (self.items.items, 0..) |*item, index| {
                if (!eligible(item, policy, instant)) continue;
                if (item.attempt == std.math.maxInt(u64)) return .{ .fail = .{ .operation = .attempt_exhausted } };
                const owner_id = copy(self.allocator, policy.owner_id) catch return .{ .die = outcomes.Defect.out_of_memory };
                claimed[prepared] = claimCopy(allocator, item, policy) catch {
                    var abandoned = owner_id;
                    abandoned.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                staged[prepared] = .{ .index = index, .owner_id = owner_id };
                prepared += 1;
                if (prepared == count) break;
            }
            for (staged[0..count]) |entry| {
                const item = &self.items.items[entry.index];
                if (item.owner_id) |*old| old.deinit();
                item.owner_id = entry.owner_id;
                item.generation = policy.generation;
                item.attempt += 1;
                item.lease_until = policy.lease_until;
                item.state = .claimed;
            }
            committed = true;
            return .{ .success = .{ .claimed = .{ .items = claimed, .allocator = allocator } } };
        }

        fn ackEffect(data: ?*anyopaque, env: *const R, token: clock.Token) Store.AckEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.AckEffect.fromOwned(env, AckCapture{ .owner = owner, .token = token }, struct {
                fn run(capture: *const AckCapture, read_env: *const R) outcomes.Exit(clock.Acknowledgement, Failure) {
                    return capture.owner.ackNow(capture.token, now(read_env));
                }
            }.run);
        }

        fn ackNow(self: *Self, token: clock.Token, instant: deadlines.WallInstant) outcomes.Exit(clock.Acknowledgement, Failure) {
            if (!token.identity.valid() or token.owner_id.len == 0 or token.attempt == 0)
                return .{ .fail = .{ .operation = .malformed } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(token.identity) orelse return .{ .success = .stale_claim };
            const owner = item.owner_id orelse return .{ .success = .stale_claim };
            if (item.generation != token.generation or item.attempt != token.attempt or
                !std.mem.eql(u8, owner.items, token.owner_id)) return .{ .success = .stale_claim };
            if (item.state == .acknowledged) return .{ .success = .already_acknowledged };
            if (item.state != .claimed or item.lease_until.isDue(instant)) return .{ .success = .stale_claim };
            item.state = .acknowledged;
            return .{ .success = .acknowledged };
        }

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
