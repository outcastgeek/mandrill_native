//! Volatile atomic replica progress. Imported event history remains in the
//! separate journal; this store only tracks acknowledged source cursors.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const journal = @import("event_journal.zig");
const remote = @import("event_log_remote.zig");
const keys = @import("../primary_key.zig");

pub fn MemoryReplicaCursor(comptime R: type) type {
    return struct {
        const Self = @This();
        const Store = remote.CursorStore(R, void);
        const Failure = remote.Error(void);
        const ReadCapture = struct { owner: *Self, source: []const u8 };
        const AdvanceCapture = struct { owner: *Self, source: []const u8, expected: journal.Cursor, next: journal.Cursor };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        cursors: std.StringHashMap(journal.Cursor),
        closed: bool = false,

        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("memory replica cursor requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("memory replica cursor requires caller I/O in R");
            return .{ .allocator = env.allocator, .io = env.io, .cursors = std.StringHashMap(journal.Cursor).init(env.allocator) };
        }

        pub fn capability(self: *Self) Store {
            return Store.fromProvider(self, &.{ .read = readEffect, .compare_and_advance = advanceEffect });
        }

        fn readEffect(data: ?*anyopaque, env: *const R, source: []const u8) Store.ReadEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ReadEffect.fromOwned(env, ReadCapture{ .owner = owner, .source = source }, struct {
                fn run(capture: *const ReadCapture, _: *const R) outcomes.Exit(journal.Cursor, Failure) {
                    return capture.owner.readNow(capture.source);
                }
            }.run);
        }

        fn advanceEffect(data: ?*anyopaque, env: *const R, source: []const u8, expected: journal.Cursor, next: journal.Cursor) Store.AdvanceEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.AdvanceEffect.fromOwned(env, AdvanceCapture{
                .owner = owner,
                .source = source,
                .expected = expected,
                .next = next,
            }, struct {
                fn run(capture: *const AdvanceCapture, _: *const R) outcomes.Exit(remote.Advance, Failure) {
                    return capture.owner.advanceNow(capture.source, capture.expected, capture.next);
                }
            }.run);
        }

        fn readNow(self: *Self, source: []const u8) outcomes.Exit(journal.Cursor, Failure) {
            if (keys.inspectEncoded(source) == .err) return .{ .fail = .malformed };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            return .{ .success = self.cursors.get(source) orelse journal.Cursor.before_first };
        }

        fn advanceNow(self: *Self, source: []const u8, expected: journal.Cursor, next: journal.Cursor) outcomes.Exit(remote.Advance, Failure) {
            if (keys.inspectEncoded(source) == .err or next.sequence < expected.sequence)
                return .{ .fail = .malformed };
            const copied = self.allocator.dupe(u8, source) catch return .{ .die = outcomes.Defect.out_of_memory };
            var retain_key = false;
            defer if (!retain_key) self.allocator.free(copied);
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            const current = self.cursors.get(source) orelse journal.Cursor.before_first;
            if (current.sequence == next.sequence) return .{ .success = .{ .already_advanced = current } };
            if (current.sequence != expected.sequence) return .{ .success = .{ .conflict = current } };
            if (self.cursors.getPtr(source)) |existing| {
                existing.* = next;
            } else {
                self.cursors.ensureUnusedCapacity(1) catch return .{ .die = outcomes.Defect.out_of_memory };
                self.cursors.putAssumeCapacity(copied, next);
                retain_key = true;
            }
            return .{ .success = .{ .committed = next } };
        }

        pub fn deinit(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            var iterator = self.cursors.iterator();
            while (iterator.next()) |entry| self.allocator.free(entry.key_ptr.*);
            self.cursors.deinit();
            self.mutex.unlock(self.io);
            self.* = undefined;
        }
    };
}
