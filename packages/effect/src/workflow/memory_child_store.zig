//! Volatile parent/child link and wake-reason store. Every state transition is
//! serialized under one caller-supplied I/O mutex; callbacks run outside it.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const types = @import("workflow_types.zig");
const contract = @import("child_store.zig");

pub fn MemoryChildStore(comptime R: type) type {
    return struct {
        const Self = @This();
        const Store = contract.Store(R, void, .@"volatile");
        const Failure = contract.Error(void);
        const Bytes = outcomes.OwnedSlice(u8);
        const Item = struct {
            parent: Bytes,
            child: Bytes,
            encoded_exit: ?Bytes = null,
            acknowledged: bool = false,

            fn deinit(self: *Item) void {
                self.parent.deinit();
                self.child.deinit();
                if (self.encoded_exit) |*value| value.deinit();
                self.* = undefined;
            }
        };
        const LinkCapture = struct { owner: *Self, link: contract.Link };
        const CompleteCapture = struct { owner: *Self, link: contract.Link, encoded_exit: []const u8 };
        const ParentCapture = struct { owner: *Self, parent: types.ExecutionId };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        items: std.array_list.Managed(Item),
        closed: bool = false,

        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or
                @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("memory child store requires R.allocator");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("memory child store requires R.io");
            return .{ .allocator = env.allocator, .io = env.io, .items = std.array_list.Managed(Item).init(env.allocator) };
        }

        pub fn capability(self: *Self) Store {
            return Store.fromProvider(self, &.{ .register = registerEffect, .complete = completeEffect, .pending = pendingEffect, .acknowledge = acknowledgeEffect });
        }

        fn copy(allocator: std.mem.Allocator, input: []const u8) error{OutOfMemory}!Bytes {
            return .{ .items = try allocator.dupe(u8, input), .allocator = allocator };
        }

        fn find(self: *Self, child: []const u8) ?*Item {
            for (self.items.items) |*item| {
                if (std.mem.eql(u8, item.child.items, child)) return item;
            }
            return null;
        }

        fn registerEffect(data: ?*anyopaque, env: *const R, link: contract.Link) Store.RegisterEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.RegisterEffect.fromOwned(env, LinkCapture{ .owner = owner, .link = link }, struct {
                fn run(capture: *const LinkCapture, _: *const R) outcomes.Exit(contract.RegisterResult, Failure) {
                    return capture.owner.registerNow(capture.link);
                }
            }.run);
        }

        fn registerNow(self: *Self, link: contract.Link) outcomes.Exit(contract.RegisterResult, Failure) {
            if (!link.valid()) return .{ .fail = .invalid_identity };
            var parent = copy(self.allocator, link.parent.encoded) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var keep_parent = false;
            defer if (!keep_parent) parent.deinit();
            var child = copy(self.allocator, link.child.encoded) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var keep_child = false;
            defer if (!keep_child) child.deinit();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            if (self.find(link.child.encoded)) |existing| {
                return .{ .success = if (std.mem.eql(u8, existing.parent.items, link.parent.encoded)) .already_registered else .conflicting_parent };
            }
            self.items.append(.{ .parent = parent, .child = child }) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            keep_parent = true;
            keep_child = true;
            return .{ .success = .registered };
        }

        fn completeEffect(data: ?*anyopaque, env: *const R, link: contract.Link, encoded_exit: []const u8) Store.CompleteEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.CompleteEffect.fromOwned(env, CompleteCapture{
                .owner = owner,
                .link = link,
                .encoded_exit = encoded_exit,
            }, struct {
                fn run(capture: *const CompleteCapture, _: *const R) outcomes.Exit(contract.CompleteResult, Failure) {
                    return capture.owner.completeNow(capture.link, capture.encoded_exit);
                }
            }.run);
        }

        fn completeNow(self: *Self, link: contract.Link, encoded_exit: []const u8) outcomes.Exit(contract.CompleteResult, Failure) {
            if (!link.valid()) return .{ .fail = .invalid_identity };
            var prepared = copy(self.allocator, encoded_exit) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var keep = false;
            defer if (!keep) prepared.deinit();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            const item = self.find(link.child.encoded) orelse
                return .{ .success = .missing_child };
            if (!std.mem.eql(u8, item.parent.items, link.parent.encoded))
                return .{ .success = .missing_child };
            if (item.encoded_exit) |existing| {
                return .{ .success = if (std.mem.eql(u8, existing.items, encoded_exit)) .already_recorded else .conflicting_exit };
            }
            item.encoded_exit = prepared;
            keep = true;
            return .{ .success = .recorded };
        }

        fn pendingEffect(data: ?*anyopaque, env: *const R, parent: types.ExecutionId) Store.PendingEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.PendingEffect.fromOwned(env, ParentCapture{
                .owner = owner,
                .parent = parent,
            }, struct {
                fn run(capture: *const ParentCapture, supplied: *const R) outcomes.Exit(outcomes.OwnedSlice(contract.Pending), Failure) {
                    return capture.owner.pendingNow(supplied.allocator, capture.parent);
                }
            }.run);
        }

        fn pendingNow(self: *Self, allocator: std.mem.Allocator, parent: types.ExecutionId) outcomes.Exit(outcomes.OwnedSlice(contract.Pending), Failure) {
            if (parent.validate() == .err) return .{ .fail = .invalid_identity };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            var count: usize = 0;
            for (self.items.items) |item| {
                if (!item.acknowledged and item.encoded_exit != null and
                    std.mem.eql(u8, item.parent.items, parent.encoded)) count += 1;
            }
            const pending = allocator.alloc(contract.Pending, count) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var filled: usize = 0;
            for (self.items.items) |item| {
                if (item.acknowledged or item.encoded_exit == null or
                    !std.mem.eql(u8, item.parent.items, parent.encoded)) continue;
                var child = copy(allocator, item.child.items) catch {
                    for (pending[0..filled]) |*entry| entry.deinit();
                    allocator.free(pending);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                const encoded = copy(allocator, item.encoded_exit.?.items) catch {
                    child.deinit();
                    for (pending[0..filled]) |*entry| entry.deinit();
                    allocator.free(pending);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                pending[filled] = .{ .child = child, .encoded_exit = encoded };
                filled += 1;
            }
            return .{ .success = .{ .items = pending, .allocator = allocator } };
        }

        fn acknowledgeEffect(data: ?*anyopaque, env: *const R, link: contract.Link) Store.AcknowledgeEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.AcknowledgeEffect.fromOwned(env, LinkCapture{
                .owner = owner,
                .link = link,
            }, struct {
                fn run(capture: *const LinkCapture, _: *const R) outcomes.Exit(contract.AcknowledgeResult, Failure) {
                    return capture.owner.acknowledgeNow(capture.link);
                }
            }.run);
        }

        fn acknowledgeNow(self: *Self, link: contract.Link) outcomes.Exit(contract.AcknowledgeResult, Failure) {
            if (!link.valid()) return .{ .fail = .invalid_identity };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            const item = self.find(link.child.encoded) orelse
                return .{ .success = .missing_child };
            if (!std.mem.eql(u8, item.parent.items, link.parent.encoded))
                return .{ .success = .missing_child };
            if (item.encoded_exit == null) return .{ .success = .not_completed };
            if (item.acknowledged) return .{ .success = .already_acknowledged };
            item.acknowledged = true;
            return .{ .success = .acknowledged };
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
