//! Explicitly volatile logical-message authority. Acceptance, claim fencing,
//! completion and reply acknowledgement are local atomic transitions. A
//! recoverable provider implements the same MessageStore contract separately.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const types = @import("cluster_types.zig");
const ownership_contract = @import("cluster_capabilities.zig");
const messages = @import("message_store.zig");
const ClusterError = @import("cluster_error.zig").Error;

pub fn MemoryMessageStore(
    comptime R: type,
    comptime now: fn (*const R) deadlines.WallInstant,
) type {
    return struct {
        const Self = @This();
        const Store = messages.MessageStore(R, void, .@"volatile");
        const Failure = ClusterError(void);
        const Bytes = outcomes.OwnedSlice(u8);
        const State = enum { pending, completed };
        const Item = struct {
            group: Bytes,
            entity_key: Bytes,
            request_key: Bytes,
            encoded_request: Bytes,
            definition_version: u32,
            retention_until: deadlines.Deadline,
            state: State = .pending,
            reply: ?Bytes = null,
            chunks: std.array_list.Managed(messages.SavedChunk),
            acknowledged_chunk_through: u64 = 0,
            acknowledged: bool = false,
            owner_id: ?Bytes = null,
            generation: u64 = 0,
            delivery_attempt: u64 = 0,
            lease_until: deadlines.Deadline = .never,

            fn deinit(self: *Item) void {
                self.group.deinit();
                self.entity_key.deinit();
                self.request_key.deinit();
                self.encoded_request.deinit();
                if (self.reply) |*value| value.deinit();
                for (self.chunks.items) |*chunk| chunk.deinit();
                self.chunks.deinit();
                if (self.owner_id) |*value| value.deinit();
                self.* = undefined;
            }

            fn addressEquals(self: *const Item, address: types.Address) bool {
                return std.mem.eql(u8, self.group.items, address.group) and
                    std.mem.eql(u8, self.entity_key.items, address.entity_key) and
                    std.mem.eql(u8, self.request_key.items, address.request_key);
            }
        };
        const AcceptCapture = struct { owner: *Self, submission: messages.Submission };
        const ClaimCapture = struct { owner: *Self, address: types.EntityAddress, ownership: types.Ownership, policy: messages.ClaimPolicy };
        const CompleteCapture = struct { owner: *Self, claim: types.ClaimToken, encoded_reply: []const u8 };
        const ChunkWriteCapture = struct { owner: *Self, claim: types.ClaimToken, sequence: u64, encoded: []const u8 };
        const ChunkReadCapture = struct { owner: *Self, address: types.Address, after: u64, policy: messages.ChunkPolicy };
        const ChunkAckCapture = struct { owner: *Self, address: types.Address, through: u64 };
        const AddressCapture = struct { owner: *Self, address: types.Address };

        allocator: std.mem.Allocator,
        io: std.Io,
        fence: ownership_contract.Fence,
        mutex: std.Io.Mutex = .init,
        items: std.array_list.Managed(Item),
        closed: bool = false,

        pub fn init(env: *const R, fence: ownership_contract.Fence) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("MemoryMessageStore requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("MemoryMessageStore requires caller io in R");
            return .{
                .allocator = env.allocator,
                .io = env.io,
                .fence = fence,
                .items = std.array_list.Managed(Item).init(env.allocator),
            };
        }

        pub fn capability(self: *Self) Store {
            return Store.fromProvider(self, &.{
                .accept = acceptEffect,
                .claim = claimEffect,
                .complete = completeEffect,
                .append_chunk = appendChunkEffect,
                .read_chunks = readChunksEffect,
                .acknowledge_chunk = acknowledgeChunkEffect,
                .read_reply = readReplyEffect,
                .acknowledge_reply = acknowledgeEffect,
            });
        }

        fn bytes(allocator: std.mem.Allocator, input: []const u8) error{OutOfMemory}!Bytes {
            return .{ .items = try allocator.dupe(u8, input), .allocator = allocator };
        }

        fn acceptedItem(self: *Self, submission: messages.Submission) error{OutOfMemory}!Item {
            var group = try bytes(self.allocator, submission.address.group);
            errdefer group.deinit();
            var entity_key = try bytes(self.allocator, submission.address.entity_key);
            errdefer entity_key.deinit();
            var request_key = try bytes(self.allocator, submission.address.request_key);
            errdefer request_key.deinit();
            return .{
                .group = group,
                .entity_key = entity_key,
                .request_key = request_key,
                .encoded_request = try bytes(self.allocator, submission.encoded_request),
                .chunks = std.array_list.Managed(messages.SavedChunk).init(self.allocator),
                .definition_version = submission.definition_version,
                .retention_until = submission.retention_until,
            };
        }

        fn acceptEffect(data: ?*anyopaque, env: *const R, submission: messages.Submission) Store.AcceptEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.AcceptEffect.fromOwned(env, AcceptCapture{ .owner = owner, .submission = submission }, struct {
                fn run(capture: *const AcceptCapture, read_env: *const R) outcomes.Exit(messages.Acceptance, Failure) {
                    return capture.owner.acceptNow(capture.submission, now(read_env));
                }
            }.run);
        }

        fn acceptNow(self: *Self, submission: messages.Submission, instant: deadlines.WallInstant) outcomes.Exit(messages.Acceptance, Failure) {
            if (submission.address.validate() == .err) return .{ .fail = .malformed };
            if (submission.retention_until.isDue(instant)) return .{ .fail = .rejected_before_acceptance };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            for (self.items.items) |*item| {
                if (!std.mem.eql(u8, item.request_key.items, submission.address.request_key)) continue;
                if (!item.addressEquals(submission.address) or
                    item.definition_version != submission.definition_version or
                    !std.mem.eql(u8, item.encoded_request.items, submission.encoded_request))
                    return .{ .fail = .conflicting_request };
                return .{ .success = if (item.state == .completed) .already_completed else .already_accepted };
            }
            var prepared = self.acceptedItem(submission) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep = false;
            defer if (!keep) prepared.deinit();
            self.items.append(prepared) catch return .{ .die = outcomes.Defect.out_of_memory };
            keep = true;
            return .{ .success = .accepted };
        }

        fn claimEffect(data: ?*anyopaque, env: *const R, address: types.EntityAddress, owner: types.Ownership, policy: messages.ClaimPolicy) Store.ClaimEffect {
            const store: *Self = @ptrCast(@alignCast(data.?));
            return Store.ClaimEffect.fromOwned(env, ClaimCapture{ .owner = store, .address = address, .ownership = owner, .policy = policy }, struct {
                fn run(capture: *const ClaimCapture, read_env: *const R) outcomes.Exit(messages.ClaimResult, Failure) {
                    return capture.owner.claimNow(read_env.allocator, capture.address, capture.ownership, capture.policy, now(read_env));
                }
            }.run);
        }

        fn belongsTo(item: *const Item, address: types.EntityAddress) bool {
            return std.mem.eql(u8, item.group.items, address.group) and
                std.mem.eql(u8, item.entity_key.items, address.entity_key);
        }

        fn eligible(item: *const Item, address: types.EntityAddress, ownership: types.Ownership, instant: deadlines.WallInstant) bool {
            if (!belongsTo(item, address)) return false;
            if (item.state != .pending) return false;
            const present = item.owner_id orelse return true;
            if (ownership.generation < item.generation) return false;
            if (ownership.generation > item.generation) return true;
            if (!std.mem.eql(u8, present.items, ownership.owner_id)) return false;
            return item.lease_until.isDue(instant);
        }

        fn cloneClaim(
            allocator: std.mem.Allocator,
            item: *const Item,
            ownership: types.Ownership,
            next_attempt: u64,
        ) error{OutOfMemory}!messages.ClaimedMessage {
            var group = try bytes(allocator, item.group.items);
            errdefer group.deinit();
            var entity_key = try bytes(allocator, item.entity_key.items);
            errdefer entity_key.deinit();
            var request_key = try bytes(allocator, item.request_key.items);
            errdefer request_key.deinit();
            var encoded_request = try bytes(allocator, item.encoded_request.items);
            errdefer encoded_request.deinit();
            return .{
                .group = group,
                .entity_key = entity_key,
                .request_key = request_key,
                .encoded_request = encoded_request,
                .definition_version = item.definition_version,
                .owner_id = try bytes(allocator, ownership.owner_id),
                .generation = ownership.generation,
                .delivery_attempt = next_attempt,
            };
        }

        fn claimNow(
            self: *Self,
            allocator: std.mem.Allocator,
            address: types.EntityAddress,
            ownership: types.Ownership,
            policy: messages.ClaimPolicy,
            instant: deadlines.WallInstant,
        ) outcomes.Exit(messages.ClaimResult, Failure) {
            if (!address.valid() or !policy.valid() or policy.lease_until.isDue(instant) or
                ownership.owner_id.len == 0 or ownership.generation == 0)
                return .{ .fail = .malformed };
            const entered = self.fence.enter(self.fence.userdata, .{ .address = address, .owner_id = ownership.owner_id, .generation = ownership.generation }, instant);
            const guard = switch (entered) {
                .held => |held| held,
                .stale => return .{ .success = .ownership_lost },
                .unavailable => return .{ .fail = .unavailable },
            };
            defer guard.release();
            const ownership_deadline = switch (guard.until) {
                .at => |value| value,
                .never => return .{ .fail = .invalid_policy },
            };
            const claim_deadline = switch (policy.lease_until) {
                .at => |value| value,
                .never => unreachable,
            };
            if (claim_deadline.unix_nanoseconds > ownership_deadline.unix_nanoseconds)
                return .{ .fail = .invalid_policy };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            var count: usize = 0;
            for (self.items.items) |*item| {
                if (eligible(item, address, ownership, instant)) count += 1;
                if (count == policy.max_messages) break;
            }
            if (count == 0) {
                for (self.items.items) |*item| {
                    if (!belongsTo(item, address) or item.state != .pending) continue;
                    if (item.owner_id) |owner_id| {
                        if (ownership.generation < item.generation or
                            (ownership.generation == item.generation and
                                !std.mem.eql(u8, owner_id.items, ownership.owner_id)))
                            return .{ .success = .ownership_lost };
                    }
                }
                return .{ .success = .none_available };
            }
            const Prepared = struct { index: usize, owner_id: Bytes };
            const staged = self.allocator.alloc(Prepared, count) catch return .{ .die = outcomes.Defect.out_of_memory };
            defer self.allocator.free(staged);
            const claimed = allocator.alloc(messages.ClaimedMessage, count) catch return .{ .die = outcomes.Defect.out_of_memory };
            var copied: usize = 0;
            var committed = false;
            defer if (!committed) {
                for (claimed[0..copied]) |*message| message.deinit();
                for (staged[0..copied]) |*entry| entry.owner_id.deinit();
                allocator.free(claimed);
            };
            for (self.items.items, 0..) |*item, index| {
                if (!eligible(item, address, ownership, instant)) continue;
                if (item.delivery_attempt == std.math.maxInt(u64)) return .{ .fail = .malformed };
                staged[copied] = .{
                    .index = index,
                    .owner_id = bytes(self.allocator, ownership.owner_id) catch return .{ .die = outcomes.Defect.out_of_memory },
                };
                claimed[copied] = cloneClaim(allocator, item, ownership, item.delivery_attempt + 1) catch {
                    staged[copied].owner_id.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                copied += 1;
                if (copied == count) break;
            }
            for (staged[0..count]) |entry| {
                const item = &self.items.items[entry.index];
                if (item.owner_id) |*old| old.deinit();
                item.owner_id = entry.owner_id;
                item.generation = ownership.generation;
                item.delivery_attempt += 1;
                item.lease_until = policy.lease_until;
            }
            committed = true;
            return .{ .success = .{ .claimed = .{ .items = claimed, .allocator = allocator } } };
        }

        fn completeEffect(data: ?*anyopaque, env: *const R, claim: types.ClaimToken, reply: []const u8) Store.CompleteEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.CompleteEffect.fromOwned(env, CompleteCapture{ .owner = owner, .claim = claim, .encoded_reply = reply }, struct {
                fn run(capture: *const CompleteCapture, read_env: *const R) outcomes.Exit(messages.Completion, Failure) {
                    return capture.owner.completeNow(capture.claim, capture.encoded_reply, now(read_env));
                }
            }.run);
        }

        fn completeNow(self: *Self, claim: types.ClaimToken, reply: []const u8, instant: deadlines.WallInstant) outcomes.Exit(messages.Completion, Failure) {
            if ((types.Address{ .group = claim.address.group, .entity_key = claim.address.entity_key, .request_key = claim.request_key }).validate() == .err or
                claim.owner_id.len == 0 or
                claim.generation == 0 or claim.delivery_attempt == 0) return .{ .fail = .malformed };
            const entered = self.fence.enter(self.fence.userdata, .{ .address = claim.address, .owner_id = claim.owner_id, .generation = claim.generation }, instant);
            const guard = switch (entered) {
                .held => |held| held,
                .stale => return .{ .success = .stale_claim },
                .unavailable => return .{ .fail = .unavailable },
            };
            defer guard.release();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            for (self.items.items) |*item| {
                if (!std.mem.eql(u8, item.request_key.items, claim.request_key)) continue;
                if (!belongsTo(item, claim.address)) return .{ .success = .stale_claim };
                const owner = item.owner_id orelse return .{ .success = .stale_claim };
                if (!std.mem.eql(u8, owner.items, claim.owner_id) or
                    item.generation != claim.generation or
                    item.delivery_attempt != claim.delivery_attempt or
                    item.lease_until.isDue(instant)) return .{ .success = .stale_claim };
                if (item.state == .completed) {
                    if (std.mem.eql(u8, item.reply.?.items, reply)) return .{ .success = .already_committed };
                    return .{ .fail = .conflicting_request };
                }
                const copied = bytes(self.allocator, reply) catch return .{ .die = outcomes.Defect.out_of_memory };
                item.reply = copied;
                item.state = .completed;
                return .{ .success = .committed };
            }
            return .{ .success = .stale_claim };
        }

        fn appendChunkEffect(data: ?*anyopaque, env: *const R, claim: types.ClaimToken, sequence: u64, encoded: []const u8) Store.ChunkWriteEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ChunkWriteEffect.fromOwned(env, ChunkWriteCapture{ .owner = owner, .claim = claim, .sequence = sequence, .encoded = encoded }, struct {
                fn run(capture: *const ChunkWriteCapture, read_env: *const R) outcomes.Exit(messages.ChunkWrite, Failure) {
                    return capture.owner.appendChunkNow(capture.claim, capture.sequence, capture.encoded, now(read_env));
                }
            }.run);
        }

        fn appendChunkNow(self: *Self, claim: types.ClaimToken, sequence: u64, encoded: []const u8, instant: deadlines.WallInstant) outcomes.Exit(messages.ChunkWrite, Failure) {
            if ((types.Address{ .group = claim.address.group, .entity_key = claim.address.entity_key, .request_key = claim.request_key }).validate() == .err or
                claim.owner_id.len == 0 or claim.generation == 0 or claim.delivery_attempt == 0 or sequence == 0)
                return .{ .fail = .malformed };
            const entered = self.fence.enter(self.fence.userdata, .{ .address = claim.address, .owner_id = claim.owner_id, .generation = claim.generation }, instant);
            const guard = switch (entered) {
                .held => |value| value,
                .stale => return .{ .success = .stale_claim },
                .unavailable => return .{ .fail = .unavailable },
            };
            defer guard.release();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            for (self.items.items) |*item| {
                if (!std.mem.eql(u8, item.request_key.items, claim.request_key)) continue;
                if (!belongsTo(item, claim.address)) return .{ .success = .stale_claim };
                const owner = item.owner_id orelse return .{ .success = .stale_claim };
                if (!std.mem.eql(u8, owner.items, claim.owner_id) or
                    item.generation != claim.generation or
                    item.delivery_attempt != claim.delivery_attempt or
                    item.lease_until.isDue(instant)) return .{ .success = .stale_claim };
                const existing_count = std.math.cast(u64, item.chunks.items.len) orelse return .{ .fail = .invalid_chunk_sequence };
                if (sequence <= existing_count) {
                    const previous = &item.chunks.items[@intCast(sequence - 1)];
                    return if (std.mem.eql(u8, previous.encoded.items, encoded))
                        .{ .success = .already_recorded }
                    else
                        .{ .fail = .conflicting_chunk };
                }
                if (item.state == .completed) return .{ .success = .stale_claim };
                const next = std.math.add(u64, existing_count, 1) catch return .{ .fail = .invalid_chunk_sequence };
                if (sequence != next) return .{ .fail = .invalid_chunk_sequence };
                var copied = bytes(self.allocator, encoded) catch return .{ .die = outcomes.Defect.out_of_memory };
                item.chunks.append(.{ .sequence = sequence, .encoded = copied }) catch {
                    copied.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                return .{ .success = .recorded };
            }
            return .{ .success = .stale_claim };
        }

        fn readChunksEffect(data: ?*anyopaque, env: *const R, address: types.Address, after: u64, policy: messages.ChunkPolicy) Store.ChunkReadEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ChunkReadEffect.fromOwned(env, ChunkReadCapture{ .owner = owner, .address = address, .after = after, .policy = policy }, struct {
                fn run(capture: *const ChunkReadCapture, read_env: *const R) outcomes.Exit(messages.ChunkRead, Failure) {
                    return capture.owner.readChunksNow(read_env.allocator, capture.address, capture.after, capture.policy, now(read_env));
                }
            }.run);
        }

        fn readChunksNow(self: *Self, allocator: std.mem.Allocator, address: types.Address, after: u64, policy: messages.ChunkPolicy, instant: deadlines.WallInstant) outcomes.Exit(messages.ChunkRead, Failure) {
            if (address.validate() == .err or !policy.valid()) return .{ .fail = .invalid_policy };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            for (self.items.items) |*item| {
                if (!item.addressEquals(address)) continue;
                const last = std.math.cast(u64, item.chunks.items.len) orelse return .{ .fail = .invalid_chunk_sequence };
                if (after > last) return .{ .fail = .invalid_chunk_sequence };
                if (item.state == .completed and item.acknowledged and
                    item.acknowledged_chunk_through == last and item.retention_until.isDue(instant))
                    return .{ .success = .expired };
                var selected = std.array_list.Managed(messages.SavedChunk).init(allocator);
                var keep = false;
                defer if (!keep) {
                    for (selected.items) |*chunk| chunk.deinit();
                    selected.deinit();
                };
                var encoded_bytes: usize = 0;
                var next = after;
                var more = false;
                for (item.chunks.items) |chunk| {
                    if (chunk.sequence <= after) continue;
                    if (chunk.encoded.items.len > policy.max_encoded_bytes)
                        return .{ .fail = .chunk_exceeds_page_limit };
                    const new_bytes = std.math.add(usize, encoded_bytes, chunk.encoded.items.len) catch
                        return .{ .fail = .chunk_exceeds_page_limit };
                    if (selected.items.len == policy.max_chunks or new_bytes > policy.max_encoded_bytes) {
                        more = true;
                        break;
                    }
                    var copied = chunk.clone() catch return .{ .die = outcomes.Defect.out_of_memory };
                    selected.append(copied) catch {
                        copied.deinit();
                        return .{ .die = outcomes.Defect.out_of_memory };
                    };
                    next = chunk.sequence;
                    encoded_bytes = new_bytes;
                }
                const owned = selected.toOwnedSlice() catch return .{ .die = outcomes.Defect.out_of_memory };
                keep = true;
                return .{ .success = .{ .available = .{
                    .chunks = .{ .items = owned, .allocator = allocator },
                    .next_sequence = next,
                    .more = more,
                    .completed = item.state == .completed,
                } } };
            }
            return .{ .success = .missing };
        }

        fn acknowledgeChunkEffect(data: ?*anyopaque, env: *const R, address: types.Address, through: u64) Store.ChunkAckEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ChunkAckEffect.fromOwned(env, ChunkAckCapture{ .owner = owner, .address = address, .through = through }, struct {
                fn run(capture: *const ChunkAckCapture, _: *const R) outcomes.Exit(messages.ChunkAck, Failure) {
                    return capture.owner.acknowledgeChunkNow(capture.address, capture.through);
                }
            }.run);
        }

        fn acknowledgeChunkNow(self: *Self, address: types.Address, through: u64) outcomes.Exit(messages.ChunkAck, Failure) {
            if (address.validate() == .err or through == 0) return .{ .fail = .malformed };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            for (self.items.items) |*item| {
                if (!item.addressEquals(address)) continue;
                if (through <= item.acknowledged_chunk_through) return .{ .success = .already_acknowledged };
                const last = std.math.cast(u64, item.chunks.items.len) orelse return .{ .fail = .invalid_chunk_sequence };
                if (through > last) return .{ .success = .unknown_sequence };
                item.acknowledged_chunk_through = through;
                return .{ .success = .acknowledged };
            }
            return .{ .success = .missing };
        }

        fn readReplyEffect(data: ?*anyopaque, env: *const R, address: types.Address) Store.ReplyEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ReplyEffect.fromOwned(env, AddressCapture{ .owner = owner, .address = address }, struct {
                fn run(capture: *const AddressCapture, read_env: *const R) outcomes.Exit(messages.Reply, Failure) {
                    return capture.owner.readReplyNow(read_env.allocator, capture.address, now(read_env));
                }
            }.run);
        }

        fn readReplyNow(self: *Self, allocator: std.mem.Allocator, address: types.Address, instant: deadlines.WallInstant) outcomes.Exit(messages.Reply, Failure) {
            if (address.validate() == .err) return .{ .fail = .malformed };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            for (self.items.items) |*item| {
                if (!item.addressEquals(address)) continue;
                if (item.state == .pending) return .{ .success = .pending };
                if (item.acknowledged and item.retention_until.isDue(instant))
                    return .{ .success = .expired };
                const copied = bytes(allocator, item.reply.?.items) catch
                    return .{ .die = outcomes.Defect.out_of_memory };
                return .{ .success = .{ .completed = copied } };
            }
            return .{ .success = .missing };
        }

        fn acknowledgeEffect(data: ?*anyopaque, env: *const R, address: types.Address) Store.AckEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.AckEffect.fromOwned(env, AddressCapture{ .owner = owner, .address = address }, struct {
                fn run(capture: *const AddressCapture, read_env: *const R) outcomes.Exit(messages.Acknowledgement, Failure) {
                    return capture.owner.acknowledgeNow(capture.address, now(read_env));
                }
            }.run);
        }

        fn acknowledgeNow(self: *Self, address: types.Address, instant: deadlines.WallInstant) outcomes.Exit(messages.Acknowledgement, Failure) {
            if (address.validate() == .err) return .{ .fail = .malformed };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            for (self.items.items) |*item| {
                if (!item.addressEquals(address)) continue;
                if (item.state != .completed or (item.acknowledged and item.retention_until.isDue(instant)))
                    return .{ .success = .reply_missing };
                if (item.acknowledged) return .{ .success = .already_acknowledged };
                item.acknowledged = true;
                return .{ .success = .acknowledged };
            }
            return .{ .success = .reply_missing };
        }

        /// Reclaim only terminal, acknowledged records after their declared
        /// retention deadline. An unfinished request or an unacknowledged
        /// streaming chunk remains claimable/readable even past that date.
        pub fn pruneExpired(self: *Self, env: *const R) outcomes.Exit(usize, Failure) {
            const instant = now(env);
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            var removed: usize = 0;
            var index: usize = 0;
            while (index < self.items.items.len) {
                const item = &self.items.items[index];
                const last_chunk = std.math.cast(u64, item.chunks.items.len) orelse
                    return .{ .fail = .invalid_chunk_sequence };
                if (item.state != .completed or !item.acknowledged or
                    item.acknowledged_chunk_through != last_chunk or
                    !item.retention_until.isDue(instant))
                {
                    index += 1;
                    continue;
                }
                var old = self.items.orderedRemove(index);
                old.deinit();
                removed += 1;
            }
            return .{ .success = removed };
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
