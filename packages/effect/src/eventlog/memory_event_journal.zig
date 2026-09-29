//! Volatile ordered journal with atomic local append and duplicate detection.
//! It implements the same public Journal contract as an external provider but
//! makes no physical-durability claim.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const keys = @import("../primary_key.zig");
const journal = @import("event_journal.zig");
const EventError = @import("event_log_error.zig").Error;

/// Construct an in-memory event journal provider with caller-owned runtime resources.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): in-memory implementation of the event journal provider.
pub fn MemoryEventJournal(comptime R: type) type {
    return struct {
        const Self = @This();
        const Failure = EventError(void);
        const Store = journal.Journal(R, void);
        const AppendCapture = struct { owner: *Self, proposal: journal.Proposal };
        const ReadCapture = struct { owner: *Self, cursor: journal.Cursor, policy: journal.ReadPolicy };
        const FindCapture = struct { owner: *Self, event_id: []const u8 };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        entries: std.array_list.Managed(journal.Entry),
        by_id: std.StringHashMap(usize),

        /// Create an in-memory event journal using the caller environment.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`Self`): an event journal bound to the supplied environment.
        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("MemoryEventJournal requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("MemoryEventJournal requires caller io in R");
            return .{
                .allocator = env.allocator,
                .io = env.io,
                .entries = std.array_list.Managed(journal.Entry).init(env.allocator),
                .by_id = std.StringHashMap(usize).init(env.allocator),
            };
        }

        /// Expose this implementation through its capability interface.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`Store`): A capability adapter referencing the selected implementation; keep its backing state alive while in use.
        pub fn capability(self: *Self) Store {
            return Store.fromProvider(self, &.{
                .append = appendEffect,
                .read_after = readEffect,
                .find_event = findEffect,
            });
        }

        fn appendEffect(data: ?*anyopaque, env: *const R, proposal: journal.Proposal) Store.AppendEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.AppendEffect.fromOwned(env, AppendCapture{ .owner = owner, .proposal = proposal }, struct {
                fn run(capture: *const AppendCapture, _: *const R) outcomes.Exit(journal.AppendAcknowledgement, Failure) {
                    return capture.owner.appendNow(capture.proposal);
                }
            }.run);
        }

        fn readEffect(data: ?*anyopaque, env: *const R, cursor: journal.Cursor, policy: journal.ReadPolicy) Store.ReadEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ReadEffect.fromOwned(env, ReadCapture{ .owner = owner, .cursor = cursor, .policy = policy }, struct {
                fn run(capture: *const ReadCapture, read_env: *const R) outcomes.Exit(journal.ReadPage, Failure) {
                    return capture.owner.readNow(read_env.allocator, capture.cursor, capture.policy);
                }
            }.run);
        }

        fn findEffect(data: ?*anyopaque, env: *const R, event_id: []const u8) Store.LookupEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.LookupEffect.fromOwned(env, FindCapture{ .owner = owner, .event_id = event_id }, struct {
                fn run(capture: *const FindCapture, read_env: *const R) outcomes.Exit(?journal.Entry, Failure) {
                    return capture.owner.findNow(read_env.allocator, capture.event_id);
                }
            }.run);
        }

        fn appendNow(self: *Self, proposal: journal.Proposal) outcomes.Exit(journal.AppendAcknowledgement, Failure) {
            if (keys.inspectEncoded(proposal.event_id) == .err or proposal.tag.len == 0 or
                !std.unicode.utf8ValidateSlice(proposal.tag)) return .{ .fail = .invalid_identity };
            var owned_tag: journal.Bytes = .{ .items = self.allocator.dupe(u8, proposal.tag) catch return .{ .die = outcomes.Defect.out_of_memory }, .allocator = self.allocator };
            var keep_tag = false;
            defer if (!keep_tag) owned_tag.deinit();
            var owned_id: journal.Bytes = .{ .items = self.allocator.dupe(u8, proposal.event_id) catch return .{ .die = outcomes.Defect.out_of_memory }, .allocator = self.allocator };
            var keep_id = false;
            defer if (!keep_id) owned_id.deinit();
            var owned_encoded: journal.Bytes = .{ .items = self.allocator.dupe(u8, proposal.encoded) catch return .{ .die = outcomes.Defect.out_of_memory }, .allocator = self.allocator };
            var keep_encoded = false;
            defer if (!keep_encoded) owned_encoded.deinit();

            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.by_id.get(proposal.event_id)) |index| {
                const existing = &self.entries.items[index];
                if (existing.schema_version != proposal.schema_version or
                    !std.mem.eql(u8, existing.tag.items, proposal.tag) or
                    !std.mem.eql(u8, existing.encoded.items, proposal.encoded))
                    return .{ .fail = .identity_conflict };
                return .{ .success = .{ .committed = existing.cursor } };
            }
            const index = self.entries.items.len;
            const sequence_index = std.math.cast(u64, index) orelse return .{ .fail = .cursor_exhausted };
            const sequence = std.math.add(u64, sequence_index, 1) catch return .{ .fail = .cursor_exhausted };
            self.entries.ensureUnusedCapacity(1) catch return .{ .die = outcomes.Defect.out_of_memory };
            self.by_id.ensureUnusedCapacity(1) catch return .{ .die = outcomes.Defect.out_of_memory };

            self.entries.appendAssumeCapacity(.{
                .tag = owned_tag,
                .event_id = owned_id,
                .encoded = owned_encoded,
                .cursor = .{ .sequence = sequence },
                .schema_version = proposal.schema_version,
            });
            self.by_id.putAssumeCapacity(owned_id.items, index);
            keep_id = true;
            keep_tag = true;
            keep_encoded = true;
            return .{ .success = .{ .committed = .{ .sequence = sequence } } };
        }

        fn readNow(self: *Self, allocator: std.mem.Allocator, cursor: journal.Cursor, policy: journal.ReadPolicy) outcomes.Exit(journal.ReadPage, Failure) {
            if (!policy.valid()) return .{ .fail = .invalid_policy };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            const start = std.math.cast(usize, cursor.sequence) orelse return .{ .fail = .history_gap };
            if (start > self.entries.items.len) return .{ .fail = .history_gap };

            var count: usize = 0;
            var total_bytes: usize = 0;
            while (start + count < self.entries.items.len and count < policy.max_entries) {
                const entry = self.entries.items[start + count];
                const identity_bytes = std.math.add(usize, entry.tag.items.len, entry.event_id.items.len) catch return .{ .fail = .entry_exceeds_page_limit };
                const entry_bytes = std.math.add(usize, identity_bytes, entry.encoded.items.len) catch return .{ .fail = .entry_exceeds_page_limit };
                const next_bytes = std.math.add(usize, total_bytes, entry_bytes) catch return .{ .fail = .entry_exceeds_page_limit };
                if (next_bytes > policy.max_encoded_bytes) {
                    if (count == 0) return .{ .fail = .entry_exceeds_page_limit };
                    break;
                }
                total_bytes = next_bytes;
                count += 1;
            }

            const copied = allocator.alloc(journal.Entry, count) catch return .{ .die = outcomes.Defect.out_of_memory };
            var initialized: usize = 0;
            while (initialized < count) : (initialized += 1) {
                copied[initialized] = cloneEntry(allocator, &self.entries.items[start + initialized]) catch {
                    for (copied[0..initialized]) |*entry| entry.deinit();
                    allocator.free(copied);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
            }
            const next_sequence = if (count == 0) cursor.sequence else copied[count - 1].cursor.sequence;
            return .{ .success = .{
                .entries = .{ .items = copied, .allocator = allocator },
                .next = .{ .sequence = next_sequence },
                .more = start + count < self.entries.items.len,
            } };
        }

        fn findNow(self: *Self, allocator: std.mem.Allocator, event_id: []const u8) outcomes.Exit(?journal.Entry, Failure) {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            const index = self.by_id.get(event_id) orelse return .{ .success = null };
            return .{ .success = cloneEntry(allocator, &self.entries.items[index]) catch return .{ .die = outcomes.Defect.out_of_memory } };
        }

        fn cloneEntry(allocator: std.mem.Allocator, source: *const journal.Entry) error{OutOfMemory}!journal.Entry {
            const tag = try allocator.dupe(u8, source.tag.items);
            errdefer allocator.free(tag);
            const id = try allocator.dupe(u8, source.event_id.items);
            errdefer allocator.free(id);
            const encoded = try allocator.dupe(u8, source.encoded.items);
            return .{
                .tag = .{ .items = tag, .allocator = allocator },
                .event_id = .{ .items = id, .allocator = allocator },
                .encoded = .{ .items = encoded, .allocator = allocator },
                .cursor = source.cursor,
                .schema_version = source.schema_version,
            };
        }

        /// Scope closure joins all users before deinit. Keys in `by_id` borrow
        /// the owned IDs in `entries` and are not freed a second time.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            for (self.entries.items) |*entry| entry.deinit();
            self.by_id.deinit();
            self.entries.deinit();
            self.mutex.unlock(self.io);
            self.* = undefined;
        }
    };
}
