//! Volatile encoded key/value storage. Conditional operations are linearized
//! with reads and writes under one local commit gate; they do not claim
//! cross-process durability or multi-key atomicity.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const contracts = @import("key_value_store.zig");
const batches = @import("batch_store.zig");
const PersistenceError = @import("persistence_error.zig").Error;

pub fn MemoryPersistence(comptime R: type, comptime now: fn (*const R) deadlines.WallInstant) type {
    return struct {
        const Self = @This();
        const Failure = PersistenceError(void);
        const Plain = contracts.KeyValueStore(R, void);
        const Conditional = contracts.ConditionalStore(R, void);
        const Batch = batches.BatchStore(R, void);
        const Entry = struct {
            key: []u8,
            value: []u8,
            expiry: deadlines.Deadline,
            revision: u64,
        };
        const ReadCapture = struct { owner: *Self, key: []const u8 };
        const WriteCapture = struct { owner: *Self, key: []const u8, value: []const u8, expiry: deadlines.Deadline };
        const CompareCapture = struct { owner: *Self, key: []const u8, expected: contracts.Revision, replacement: ?[]const u8, expiry: deadlines.Deadline };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        data: std.StringHashMap(Entry),
        next_revision: u64 = 0,

        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("MemoryPersistence requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("MemoryPersistence requires caller io in R");
            return .{ .allocator = env.allocator, .io = env.io, .data = std.StringHashMap(Entry).init(env.allocator) };
        }

        pub fn keyValue(self: *Self) Plain {
            return Plain.fromProvider(self, &.{ .read = plainRead, .write = plainWrite, .remove = plainRemove });
        }

        pub fn conditional(self: *Self) Conditional {
            return Conditional.fromProvider(self, &.{ .read_versioned = conditionalRead, .compare_exchange = compareExchange });
        }

        pub fn batch(self: *Self) Batch {
            return Batch.fromProvider(self, &.{ .read_many = batchRead, .write_many = batchWrite, .remove_many = batchRemove });
        }

        const BatchKeys = struct { owner: *Self, keys: []const []const u8 };
        const BatchWrites = struct { owner: *Self, writes: []const batches.Write };

        fn batchRead(data: ?*anyopaque, env: *const R, keys: []const []const u8) Batch.ReadEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Batch.ReadEffect.fromOwned(env, BatchKeys{ .owner = owner, .keys = keys }, struct {
                fn run(capture: *const BatchKeys, read_env: *const R) outcomes.Exit(Batch.ReadReport, Failure) {
                    const items = read_env.allocator.alloc(batches.Item(contracts.Read, Failure), capture.keys.len) catch
                        return .{ .die = outcomes.Defect.out_of_memory };
                    for (items) |*item| item.* = .not_attempted;
                    for (capture.keys, 0..) |key, index| {
                        const result = capture.owner.readNow(read_env.allocator, key, now(read_env));
                        items[index] = .{ .attempted = result };
                        if (result != .success) break;
                    }
                    return .{ .success = .{ .items = items, .allocator = read_env.allocator } };
                }
            }.run);
        }

        fn batchWrite(data: ?*anyopaque, env: *const R, writes: []const batches.Write) Batch.WriteEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Batch.WriteEffect.fromOwned(env, BatchWrites{ .owner = owner, .writes = writes }, struct {
                fn run(capture: *const BatchWrites, read_env: *const R) outcomes.Exit(Batch.WriteReport, Failure) {
                    const items = read_env.allocator.alloc(batches.Item(contracts.WriteAcknowledgement, Failure), capture.writes.len) catch
                        return .{ .die = outcomes.Defect.out_of_memory };
                    for (items) |*item| item.* = .not_attempted;
                    for (capture.writes, 0..) |write, index| {
                        const result = capture.owner.writeNow(write.key, write.bytes, write.expiry);
                        items[index] = .{ .attempted = result };
                        if (result != .success) break;
                    }
                    return .{ .success = .{ .items = items, .allocator = read_env.allocator } };
                }
            }.run);
        }

        fn batchRemove(data: ?*anyopaque, env: *const R, keys: []const []const u8) Batch.WriteEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Batch.WriteEffect.fromOwned(env, BatchKeys{ .owner = owner, .keys = keys }, struct {
                fn run(capture: *const BatchKeys, read_env: *const R) outcomes.Exit(Batch.WriteReport, Failure) {
                    const items = read_env.allocator.alloc(batches.Item(contracts.WriteAcknowledgement, Failure), capture.keys.len) catch
                        return .{ .die = outcomes.Defect.out_of_memory };
                    for (items) |*item| item.* = .not_attempted;
                    for (capture.keys, 0..) |key, index| {
                        const result = capture.owner.removeNow(key);
                        items[index] = .{ .attempted = result };
                        if (result != .success) break;
                    }
                    return .{ .success = .{ .items = items, .allocator = read_env.allocator } };
                }
            }.run);
        }

        fn plainRead(data: ?*anyopaque, env: *const R, key: []const u8) Plain.ReadEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Plain.ReadEffect.fromOwned(env, ReadCapture{ .owner = owner, .key = key }, struct {
                fn run(capture: *const ReadCapture, read_env: *const R) outcomes.Exit(contracts.Read, Failure) {
                    return capture.owner.readNow(read_env.allocator, capture.key, now(read_env));
                }
            }.run);
        }

        fn plainWrite(data: ?*anyopaque, env: *const R, key: []const u8, value: []const u8, expiry: deadlines.Deadline) Plain.WriteEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Plain.WriteEffect.fromOwned(env, WriteCapture{ .owner = owner, .key = key, .value = value, .expiry = expiry }, struct {
                fn run(capture: *const WriteCapture, _: *const R) outcomes.Exit(contracts.WriteAcknowledgement, Failure) {
                    return capture.owner.writeNow(capture.key, capture.value, capture.expiry);
                }
            }.run);
        }

        fn plainRemove(data: ?*anyopaque, env: *const R, key: []const u8) Plain.WriteEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Plain.WriteEffect.fromOwned(env, ReadCapture{ .owner = owner, .key = key }, struct {
                fn run(capture: *const ReadCapture, _: *const R) outcomes.Exit(contracts.WriteAcknowledgement, Failure) {
                    return capture.owner.removeNow(capture.key);
                }
            }.run);
        }

        fn conditionalRead(data: ?*anyopaque, env: *const R, key: []const u8) Conditional.ReadEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Conditional.ReadEffect.fromOwned(env, ReadCapture{ .owner = owner, .key = key }, struct {
                fn run(capture: *const ReadCapture, read_env: *const R) outcomes.Exit(contracts.ReadVersioned, Failure) {
                    return capture.owner.readVersionedNow(read_env.allocator, capture.key, now(read_env));
                }
            }.run);
        }

        fn compareExchange(data: ?*anyopaque, env: *const R, key: []const u8, expected: contracts.Revision, replacement: ?[]const u8, expiry: deadlines.Deadline) Conditional.CompareEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Conditional.CompareEffect.fromOwned(env, CompareCapture{ .owner = owner, .key = key, .expected = expected, .replacement = replacement, .expiry = expiry }, struct {
                fn run(capture: *const CompareCapture, read_env: *const R) outcomes.Exit(contracts.ConditionalAcknowledgement, Failure) {
                    return capture.owner.compareNow(capture.key, capture.expected, capture.replacement, capture.expiry, now(read_env));
                }
            }.run);
        }

        fn readNow(self: *Self, allocator: std.mem.Allocator, key: []const u8, observed_at: deadlines.WallInstant) outcomes.Exit(contracts.Read, Failure) {
            if (key.len == 0) return .{ .fail = .malformed };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            const entry = self.data.get(key) orelse return .{ .success = .missing };
            if (entry.expiry.isDue(observed_at)) {
                self.discardKey(key);
                return .{ .success = .expired };
            }
            const copy = allocator.dupe(u8, entry.value) catch return .{ .die = outcomes.Defect.out_of_memory };
            return .{ .success = .{ .found = .{ .items = copy, .allocator = allocator } } };
        }

        fn readVersionedNow(self: *Self, allocator: std.mem.Allocator, key: []const u8, observed_at: deadlines.WallInstant) outcomes.Exit(contracts.ReadVersioned, Failure) {
            if (key.len == 0) return .{ .fail = .malformed };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            const entry = self.data.get(key) orelse return .{ .success = .missing };
            if (entry.expiry.isDue(observed_at)) {
                self.discardKey(key);
                return .{ .success = .expired };
            }
            const copy = allocator.dupe(u8, entry.value) catch return .{ .die = outcomes.Defect.out_of_memory };
            return .{ .success = .{ .found = .{ .bytes = .{ .items = copy, .allocator = allocator }, .revision = .{ .present = entry.revision } } } };
        }

        fn writeNow(self: *Self, key: []const u8, value: []const u8, expiry: deadlines.Deadline) outcomes.Exit(contracts.WriteAcknowledgement, Failure) {
            if (key.len == 0) return .{ .fail = .malformed };
            const key_copy = self.allocator.dupe(u8, key) catch return .{ .die = outcomes.Defect.out_of_memory };
            var use_key = false;
            defer if (!use_key) self.allocator.free(key_copy);
            const value_copy = self.allocator.dupe(u8, value) catch return .{ .die = outcomes.Defect.out_of_memory };
            var use_value = false;
            defer if (!use_value) self.allocator.free(value_copy);

            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            const revision = std.math.add(u64, self.next_revision, 1) catch return .{ .fail = .revision_exhausted };
            if (self.data.getPtr(key)) |existing| {
                self.allocator.free(existing.value);
                existing.value = value_copy;
                existing.expiry = expiry;
                existing.revision = revision;
            } else {
                self.data.ensureUnusedCapacity(1) catch return .{ .die = outcomes.Defect.out_of_memory };
                self.data.putAssumeCapacity(key_copy, .{ .key = key_copy, .value = value_copy, .expiry = expiry, .revision = revision });
                use_key = true;
            }
            use_value = true;
            self.next_revision = revision;
            return .{ .success = .committed };
        }

        fn removeNow(self: *Self, key: []const u8) outcomes.Exit(contracts.WriteAcknowledgement, Failure) {
            if (key.len == 0) return .{ .fail = .malformed };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            self.discardKey(key);
            return .{ .success = .committed };
        }

        fn compareNow(self: *Self, key: []const u8, expected: contracts.Revision, replacement: ?[]const u8, expiry: deadlines.Deadline, observed_at: deadlines.WallInstant) outcomes.Exit(contracts.ConditionalAcknowledgement, Failure) {
            if (key.len == 0) return .{ .fail = .malformed };
            const key_copy = self.allocator.dupe(u8, key) catch return .{ .die = outcomes.Defect.out_of_memory };
            var use_key = false;
            defer if (!use_key) self.allocator.free(key_copy);
            const value_copy: ?[]u8 = if (replacement) |bytes| self.allocator.dupe(u8, bytes) catch return .{ .die = outcomes.Defect.out_of_memory } else null;
            var use_value = false;
            defer if (!use_value) if (value_copy) |bytes| self.allocator.free(bytes);

            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.data.get(key)) |entry| if (entry.expiry.isDue(observed_at)) self.discardKey(key);
            const actual: contracts.Revision = if (self.data.get(key)) |entry| .{ .present = entry.revision } else .absent;
            if (!std.meta.eql(expected, actual)) return .{ .success = .{ .conflict = actual } };
            if (value_copy) |bytes| {
                const revision = std.math.add(u64, self.next_revision, 1) catch return .{ .fail = .revision_exhausted };
                if (self.data.getPtr(key)) |existing| {
                    self.allocator.free(existing.value);
                    existing.value = bytes;
                    existing.expiry = expiry;
                    existing.revision = revision;
                } else {
                    self.data.ensureUnusedCapacity(1) catch return .{ .die = outcomes.Defect.out_of_memory };
                    self.data.putAssumeCapacity(key_copy, .{ .key = key_copy, .value = bytes, .expiry = expiry, .revision = revision });
                    use_key = true;
                }
                use_value = true;
                self.next_revision = revision;
                return .{ .success = .{ .applied = .{ .present = revision } } };
            }
            self.discardKey(key);
            return .{ .success = .{ .applied = .absent } };
        }

        /// Caller holds mutex. Stored keys and values have the construction
        /// allocator; result buffers use each invoking environment's allocator.
        fn discardKey(self: *Self, key: []const u8) void {
            const removed = self.data.fetchRemove(key) orelse return;
            self.allocator.free(removed.value.key);
            self.allocator.free(removed.value.value);
        }

        pub fn deinit(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            var iterator = self.data.iterator();
            while (iterator.next()) |entry| {
                self.allocator.free(entry.value_ptr.key);
                self.allocator.free(entry.value_ptr.value);
            }
            self.data.deinit();
            self.mutex.unlock(self.io);
            self.* = undefined;
        }
    };
}
