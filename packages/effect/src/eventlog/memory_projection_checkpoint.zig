//! Volatile atomic projection checkpoints. This is a concrete in-memory
//! provider for contract tests and explicitly ephemeral applications; it
//! makes no claim of durability across process restarts.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const contract = @import("projection_checkpoint.zig");
const journal = @import("event_journal.zig");
const EventError = @import("event_log_error.zig").Error;

/// Construct an in-memory projection checkpoint provider with caller-owned runtime resources.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): in-memory implementation of projection checkpoint storage.
pub fn MemoryProjectionCheckpoint(comptime R: type) type {
    return struct {
        const Self = @This();
        const Provider = contract.AtomicCheckpointStore(R, void);
        const Failure = EventError(void);
        const Stored = struct { key: []u8, checkpoint: contract.Checkpoint };
        const ReadCapture = struct { owner: *Self, id: []const u8 };
        const PublishCapture = struct {
            owner: *Self,
            id: []const u8,
            expected: contract.Revision,
            next: journal.Cursor,
            schema_version: u32,
            bytes: []const u8,
        };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        checkpoints: std.StringHashMap(Stored),
        closed: bool = false,

        /// Create an in-memory projection checkpoint store using the caller environment.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`Self`): a checkpoint store bound to the supplied environment.
        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("MemoryProjectionCheckpoint requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("MemoryProjectionCheckpoint requires caller io in R");
            return .{ .allocator = env.allocator, .io = env.io, .checkpoints = std.StringHashMap(Stored).init(env.allocator) };
        }

        /// Expose this implementation through its capability interface.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`Provider`): A capability adapter referencing the selected implementation; keep its backing state alive while in use.
        pub fn capability(self: *Self) Provider {
            return Provider.fromProvider(self, &.{ .read = readEffect, .compare_and_publish = publishEffect });
        }

        fn readEffect(data: ?*anyopaque, env: *const R, id: []const u8) Provider.ReadEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Provider.ReadEffect.fromOwned(env, ReadCapture{ .owner = owner, .id = id }, struct {
                fn run(capture: *const ReadCapture, read_env: *const R) outcomes.Exit(?contract.Checkpoint, Failure) {
                    return capture.owner.readNow(read_env.allocator, capture.id);
                }
            }.run);
        }

        fn readNow(self: *Self, allocator: std.mem.Allocator, id: []const u8) outcomes.Exit(?contract.Checkpoint, Failure) {
            if (id.len == 0) return .{ .fail = .invalid_identity };
            while (true) {
                self.mutex.lockUncancelable(self.io);
                if (self.closed) {
                    self.mutex.unlock(self.io);
                    return .{ .fail = .checkpoint_unavailable };
                }
                const size = if (self.checkpoints.get(id)) |stored| stored.checkpoint.encoded_state.items.len else {
                    self.mutex.unlock(self.io);
                    return .{ .success = null };
                };
                self.mutex.unlock(self.io);
                const bytes = allocator.alloc(u8, size) catch return .{ .die = outcomes.Defect.out_of_memory };
                self.mutex.lockUncancelable(self.io);
                if (self.closed) {
                    self.mutex.unlock(self.io);
                    allocator.free(bytes);
                    return .{ .fail = .checkpoint_unavailable };
                }
                const current = self.checkpoints.get(id) orelse {
                    self.mutex.unlock(self.io);
                    allocator.free(bytes);
                    return .{ .success = null };
                };
                if (current.checkpoint.encoded_state.items.len != size) {
                    self.mutex.unlock(self.io);
                    allocator.free(bytes);
                    continue;
                }
                std.mem.copyForwards(u8, bytes, current.checkpoint.encoded_state.items);
                const cursor = current.checkpoint.cursor;
                const version = current.checkpoint.state_schema_version;
                self.mutex.unlock(self.io);
                return .{ .success = .{
                    .cursor = cursor,
                    .state_schema_version = version,
                    .encoded_state = .{ .items = bytes, .allocator = allocator },
                } };
            }
        }

        fn publishEffect(
            data: ?*anyopaque,
            env: *const R,
            id: []const u8,
            expected: contract.Revision,
            next: journal.Cursor,
            schema_version: u32,
            bytes: []const u8,
        ) Provider.CommitEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Provider.CommitEffect.fromOwned(env, PublishCapture{
                .owner = owner,
                .id = id,
                .expected = expected,
                .next = next,
                .schema_version = schema_version,
                .bytes = bytes,
            }, struct {
                fn run(capture: *const PublishCapture, _: *const R) outcomes.Exit(contract.CommitResult, Failure) {
                    return capture.owner.publishNow(capture.*);
                }
            }.run);
        }

        fn publishNow(self: *Self, request: PublishCapture) outcomes.Exit(contract.CommitResult, Failure) {
            if (request.id.len == 0) return .{ .fail = .invalid_identity };
            if (request.next.sequence == journal.Cursor.before_first.sequence) return .{ .fail = .invalid_policy };
            const key = self.allocator.dupe(u8, request.id) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_key = false;
            defer if (!keep_key) self.allocator.free(key);
            const state = self.allocator.dupe(u8, request.bytes) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_state = false;
            defer if (!keep_state) self.allocator.free(state);
            var replaced: ?outcomes.OwnedSlice(u8) = null;
            defer if (replaced) |*old| old.deinit();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .checkpoint_unavailable };
            const existing = self.checkpoints.getPtr(request.id);
            const observed: contract.Revision = if (existing) |entry| .{ .present = entry.checkpoint.cursor } else .absent;
            const matches = switch (request.expected) {
                .absent => observed == .absent,
                .present => |cursor| observed == .present and cursor.sequence == observed.present.sequence,
            };
            if (!matches) return .{ .success = .{ .conflict = observed } };
            if (existing) |entry| {
                if (request.next.sequence <= entry.checkpoint.cursor.sequence) return .{ .fail = .invalid_policy };
                replaced = entry.checkpoint.encoded_state;
                entry.checkpoint = .{
                    .cursor = request.next,
                    .state_schema_version = request.schema_version,
                    .encoded_state = .{ .items = state, .allocator = self.allocator },
                };
                keep_state = true;
            } else {
                self.checkpoints.ensureUnusedCapacity(1) catch return .{ .die = outcomes.Defect.out_of_memory };
                self.checkpoints.putAssumeCapacity(key, .{ .key = key, .checkpoint = .{
                    .cursor = request.next,
                    .state_schema_version = request.schema_version,
                    .encoded_state = .{ .items = state, .allocator = self.allocator },
                } });
                keep_key = true;
                keep_state = true;
            }
            return .{ .success = .{ .committed = request.next } };
        }

        /// Stop the capability and release its retained resources.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn shutdown(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            self.mutex.unlock(self.io);
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            var saved = self.checkpoints;
            self.checkpoints = std.StringHashMap(Stored).init(self.allocator);
            self.mutex.unlock(self.io);
            var iterator = saved.iterator();
            while (iterator.next()) |entry| {
                self.allocator.free(entry.value_ptr.key);
                entry.value_ptr.checkpoint.deinit();
            }
            saved.deinit();
            self.checkpoints.deinit();
            self.* = undefined;
        }
    };
}
