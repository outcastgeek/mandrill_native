//! Volatile implementation of PersistedQueue's atomic local transitions.
//! It is suitable for scoped applications and conformance work, not for a
//! restart-stable queue; a recoverable provider must uphold the same contract.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const keys = @import("../primary_key.zig");
const queue = @import("persisted_queue.zig");
const PersistenceError = @import("persistence_error.zig").Error;

pub fn MemoryPersistedQueue(
    comptime R: type,
    comptime now: fn (*const R) deadlines.WallInstant,
) type {
    return struct {
        const Self = @This();
        const Store = queue.Store(R, void, .@"volatile");
        const Failure = PersistenceError(void);
        const Bytes = outcomes.OwnedSlice(u8);
        const State = enum { pending, claimed, acknowledged, dead_letter };
        const Item = struct {
            sequence: u64,
            queue_key: Bytes,
            offer_key: Bytes,
            payload: Bytes,
            visible_at: deadlines.WallInstant,
            retention_until: deadlines.Deadline,
            state: State = .pending,
            owner_id: ?Bytes = null,
            generation: u64 = 0,
            attempt: u64 = 0,
            lease_until: deadlines.Deadline = .never,
            reason_code: ?Bytes = null,

            fn deinit(self: *Item) void {
                self.queue_key.deinit();
                self.offer_key.deinit();
                self.payload.deinit();
                if (self.owner_id) |*value| value.deinit();
                if (self.reason_code) |*value| value.deinit();
                self.* = undefined;
            }
        };
        const OfferCapture = struct { owner: *Self, submission: queue.Submission };
        const ClaimCapture = struct {
            owner: *Self,
            queue_key: []const u8,
            owner_id: []const u8,
            generation: u64,
            policy: queue.ClaimPolicy,
        };
        const TokenCapture = struct { owner: *Self, token: queue.ClaimToken };
        const RetryCapture = struct { owner: *Self, token: queue.ClaimToken, visible_at: deadlines.WallInstant };
        const DeadCapture = struct { owner: *Self, token: queue.ClaimToken, reason_code: []const u8 };
        const ReadCapture = struct { owner: *Self, queue_key: []const u8, after: queue.Cursor, policy: queue.ReadPolicy };
        const RequeueCapture = struct { owner: *Self, queue_key: []const u8, offer_key: []const u8, visible_at: deadlines.WallInstant };
        const CleanupCapture = struct { owner: *Self, queue_key: []const u8, max_items: usize, instant: deadlines.WallInstant };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        items: std.array_list.Managed(Item),
        next_sequence: u64 = queue.Cursor.before_first.sequence + 1,
        closed: bool = false,

        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("MemoryPersistedQueue requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("MemoryPersistedQueue requires caller io in R");
            return .{
                .allocator = env.allocator,
                .io = env.io,
                .items = std.array_list.Managed(Item).init(env.allocator),
            };
        }

        pub fn capability(self: *Self) Store {
            return Store.fromProvider(self, &.{
                .offer = offerEffect,
                .claim = claimEffect,
                .acknowledge = ackEffect,
                .retry = retryEffect,
                .dead_letter = deadEffect,
                .read_dead_letters = readEffect,
                .requeue = requeueEffect,
                .cleanup = cleanupEffect,
            });
        }

        fn bytes(allocator: std.mem.Allocator, value: []const u8) error{OutOfMemory}!Bytes {
            return .{ .items = try allocator.dupe(u8, value), .allocator = allocator };
        }

        fn hasIdentity(item: *const Item, queue_key: []const u8, offer_key: []const u8) bool {
            return std.mem.eql(u8, item.queue_key.items, queue_key) and std.mem.eql(u8, item.offer_key.items, offer_key);
        }

        fn find(self: *Self, queue_key: []const u8, offer_key: []const u8) ?*Item {
            for (self.items.items) |*item| if (hasIdentity(item, queue_key, offer_key)) return item;
            return null;
        }

        fn validToken(token: queue.ClaimToken) bool {
            return keys.inspectEncoded(token.queue_key) == .ok and keys.inspectEncoded(token.offer_key) == .ok and
                token.owner_id.len > 0 and token.attempt > 0;
        }

        fn sameClaim(item: *const Item, token: queue.ClaimToken) bool {
            const owner = item.owner_id orelse return false;
            return item.generation == token.generation and item.attempt == token.attempt and
                std.mem.eql(u8, owner.items, token.owner_id);
        }

        fn offerEffect(data: ?*anyopaque, env: *const R, submission: queue.Submission) Store.OfferEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.OfferEffect.fromOwned(env, OfferCapture{ .owner = owner, .submission = submission }, struct {
                fn run(capture: *const OfferCapture, read_env: *const R) outcomes.Exit(queue.OfferResult, Failure) {
                    return capture.owner.offerNow(capture.submission, now(read_env));
                }
            }.run);
        }

        fn preparedOffer(self: *Self, submission: queue.Submission) error{OutOfMemory}!Item {
            var queue_key = try bytes(self.allocator, submission.queue_key);
            errdefer queue_key.deinit();
            var offer_key = try bytes(self.allocator, submission.offer_key);
            errdefer offer_key.deinit();
            return .{
                .sequence = undefined,
                .queue_key = queue_key,
                .offer_key = offer_key,
                .payload = try bytes(self.allocator, submission.encoded_payload),
                .visible_at = submission.visible_at,
                .retention_until = submission.retention_until,
            };
        }

        fn offerNow(self: *Self, submission: queue.Submission, instant: deadlines.WallInstant) outcomes.Exit(queue.OfferResult, Failure) {
            if (keys.inspectEncoded(submission.queue_key) == .err or keys.inspectEncoded(submission.offer_key) == .err)
                return .{ .fail = .malformed };
            if (submission.retention_until.isDue(instant)) return .{ .fail = .invalid_policy };
            var prepared = self.preparedOffer(submission) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep = false;
            defer if (!keep) prepared.deinit();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            if (self.find(submission.queue_key, submission.offer_key)) |existing| {
                if (!std.mem.eql(u8, existing.payload.items, submission.encoded_payload))
                    return .{ .fail = .identity_conflict };
                return .{ .success = .already_offered };
            }
            if (self.next_sequence == std.math.maxInt(u64)) return .{ .fail = .revision_exhausted };
            prepared.sequence = self.next_sequence;
            self.items.append(prepared) catch return .{ .die = outcomes.Defect.out_of_memory };
            self.next_sequence += 1;
            keep = true;
            return .{ .success = .accepted };
        }

        fn claimEffect(data: ?*anyopaque, env: *const R, queue_key: []const u8, owner_id: []const u8, generation: u64, policy: queue.ClaimPolicy) Store.ClaimEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ClaimEffect.fromOwned(env, ClaimCapture{
                .owner = owner,
                .queue_key = queue_key,
                .owner_id = owner_id,
                .generation = generation,
                .policy = policy,
            }, struct {
                fn run(capture: *const ClaimCapture, read_env: *const R) outcomes.Exit(queue.ClaimResult, Failure) {
                    return capture.owner.claimNow(read_env.allocator, capture.*, now(read_env));
                }
            }.run);
        }

        fn eligible(item: *const Item, request: ClaimCapture, instant: deadlines.WallInstant) bool {
            if (!std.mem.eql(u8, item.queue_key.items, request.queue_key)) return false;
            if (item.visible_at.unix_nanoseconds > instant.unix_nanoseconds) return false;
            if (item.state == .pending) return request.generation >= item.generation;
            if (item.state != .claimed) return false;
            const owner = item.owner_id orelse return true;
            if (request.generation < item.generation) return false;
            if (request.generation > item.generation) return true;
            if (!std.mem.eql(u8, owner.items, request.owner_id)) return false;
            return item.lease_until.isDue(instant);
        }

        fn claimedCopy(allocator: std.mem.Allocator, item: *const Item, request: ClaimCapture) error{OutOfMemory}!queue.ClaimedItem {
            var queue_key = try bytes(allocator, item.queue_key.items);
            errdefer queue_key.deinit();
            var offer_key = try bytes(allocator, item.offer_key.items);
            errdefer offer_key.deinit();
            var payload = try bytes(allocator, item.payload.items);
            errdefer payload.deinit();
            return .{
                .queue_key = queue_key,
                .offer_key = offer_key,
                .encoded_payload = payload,
                .owner_id = try bytes(allocator, request.owner_id),
                .generation = request.generation,
                .attempt = item.attempt + 1,
            };
        }

        fn claimNow(self: *Self, allocator: std.mem.Allocator, request: ClaimCapture, instant: deadlines.WallInstant) outcomes.Exit(queue.ClaimResult, Failure) {
            if (keys.inspectEncoded(request.queue_key) == .err or request.owner_id.len == 0 or !request.policy.valid() or
                request.policy.lease_until.isDue(instant)) return .{ .fail = .invalid_policy };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            var count: usize = 0;
            for (self.items.items) |*item| {
                if (eligible(item, request, instant)) count += 1;
                if (count == request.policy.max_items) break;
            }
            if (count == 0) {
                for (self.items.items) |*item| {
                    if (!std.mem.eql(u8, item.queue_key.items, request.queue_key) or
                        (item.state != .claimed and item.state != .pending)) continue;
                    const owner = item.owner_id orelse continue;
                    if (request.generation < item.generation or
                        (item.state == .claimed and request.generation == item.generation and
                            !std.mem.eql(u8, owner.items, request.owner_id)))
                        return .{ .success = .ownership_lost };
                }
                return .{ .success = .none_available };
            }
            const Prepared = struct { index: usize, owner: Bytes };
            const staged = self.allocator.alloc(Prepared, count) catch return .{ .die = outcomes.Defect.out_of_memory };
            defer self.allocator.free(staged);
            const claimed = allocator.alloc(queue.ClaimedItem, count) catch return .{ .die = outcomes.Defect.out_of_memory };
            var copied: usize = 0;
            var committed = false;
            defer if (!committed) {
                for (claimed[0..copied]) |*item| item.deinit();
                for (staged[0..copied]) |*entry| entry.owner.deinit();
                allocator.free(claimed);
            };
            for (self.items.items, 0..) |*item, index| {
                if (!eligible(item, request, instant)) continue;
                if (item.attempt == std.math.maxInt(u64)) return .{ .fail = .revision_exhausted };
                staged[copied] = .{
                    .index = index,
                    .owner = bytes(self.allocator, request.owner_id) catch return .{ .die = outcomes.Defect.out_of_memory },
                };
                claimed[copied] = claimedCopy(allocator, item, request) catch {
                    staged[copied].owner.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                copied += 1;
                if (copied == count) break;
            }
            for (staged[0..count]) |entry| {
                const item = &self.items.items[entry.index];
                if (item.owner_id) |*old| old.deinit();
                item.owner_id = entry.owner;
                item.generation = request.generation;
                item.attempt += 1;
                item.lease_until = request.policy.lease_until;
                item.state = .claimed;
            }
            committed = true;
            return .{ .success = .{ .claimed = .{ .items = claimed, .allocator = allocator } } };
        }

        fn ackEffect(data: ?*anyopaque, env: *const R, token: queue.ClaimToken) Store.AckEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.AckEffect.fromOwned(env, TokenCapture{ .owner = owner, .token = token }, struct {
                fn run(capture: *const TokenCapture, read_env: *const R) outcomes.Exit(queue.AckResult, Failure) {
                    return capture.owner.ackNow(capture.token, now(read_env));
                }
            }.run);
        }

        fn ackNow(self: *Self, token: queue.ClaimToken, instant: deadlines.WallInstant) outcomes.Exit(queue.AckResult, Failure) {
            if (!validToken(token)) return .{ .fail = .malformed };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            const item = self.find(token.queue_key, token.offer_key) orelse return .{ .success = .stale_claim };
            if (!sameClaim(item, token)) return .{ .success = .stale_claim };
            if (item.state == .acknowledged) return .{ .success = .already_acknowledged };
            if (item.state != .claimed or item.lease_until.isDue(instant)) return .{ .success = .stale_claim };
            item.state = .acknowledged;
            return .{ .success = .acknowledged };
        }

        fn retryEffect(data: ?*anyopaque, env: *const R, token: queue.ClaimToken, visible_at: deadlines.WallInstant) Store.RetryEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.RetryEffect.fromOwned(env, RetryCapture{ .owner = owner, .token = token, .visible_at = visible_at }, struct {
                fn run(capture: *const RetryCapture, read_env: *const R) outcomes.Exit(queue.RetryResult, Failure) {
                    return capture.owner.retryNow(capture.token, capture.visible_at, now(read_env));
                }
            }.run);
        }

        fn retryNow(self: *Self, token: queue.ClaimToken, visible_at: deadlines.WallInstant, instant: deadlines.WallInstant) outcomes.Exit(queue.RetryResult, Failure) {
            if (!validToken(token)) return .{ .fail = .malformed };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            const item = self.find(token.queue_key, token.offer_key) orelse return .{ .success = .stale_claim };
            if (!sameClaim(item, token)) return .{ .success = .stale_claim };
            if (item.state == .pending and item.visible_at.unix_nanoseconds == visible_at.unix_nanoseconds)
                return .{ .success = .already_scheduled };
            if (item.state != .claimed or item.lease_until.isDue(instant)) return .{ .success = .stale_claim };
            item.visible_at = visible_at;
            item.state = .pending;
            return .{ .success = .scheduled };
        }

        fn deadEffect(data: ?*anyopaque, env: *const R, token: queue.ClaimToken, reason_code: []const u8) Store.DeadLetterEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.DeadLetterEffect.fromOwned(env, DeadCapture{ .owner = owner, .token = token, .reason_code = reason_code }, struct {
                fn run(capture: *const DeadCapture, read_env: *const R) outcomes.Exit(queue.DeadLetterResult, Failure) {
                    return capture.owner.deadNow(capture.token, capture.reason_code, now(read_env));
                }
            }.run);
        }

        fn deadNow(self: *Self, token: queue.ClaimToken, reason_code: []const u8, instant: deadlines.WallInstant) outcomes.Exit(queue.DeadLetterResult, Failure) {
            if (!validToken(token) or reason_code.len == 0 or !std.unicode.utf8ValidateSlice(reason_code)) return .{ .fail = .malformed };
            var reason = bytes(self.allocator, reason_code) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep = false;
            defer if (!keep) reason.deinit();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            const item = self.find(token.queue_key, token.offer_key) orelse return .{ .success = .stale_claim };
            if (!sameClaim(item, token)) return .{ .success = .stale_claim };
            if (item.state == .dead_letter) {
                if (std.mem.eql(u8, item.reason_code.?.items, reason_code)) return .{ .success = .already_recorded };
                return .{ .fail = .identity_conflict };
            }
            if (item.state != .claimed or item.lease_until.isDue(instant)) return .{ .success = .stale_claim };
            item.reason_code = reason;
            item.state = .dead_letter;
            keep = true;
            return .{ .success = .recorded };
        }

        fn readEffect(data: ?*anyopaque, env: *const R, queue_key: []const u8, after: queue.Cursor, policy: queue.ReadPolicy) Store.ReadEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.ReadEffect.fromOwned(env, ReadCapture{ .owner = owner, .queue_key = queue_key, .after = after, .policy = policy }, struct {
                fn run(capture: *const ReadCapture, read_env: *const R) outcomes.Exit(queue.DeadLetterPage, Failure) {
                    return capture.owner.readNow(read_env.allocator, capture.queue_key, capture.after, capture.policy);
                }
            }.run);
        }

        fn deadCopy(allocator: std.mem.Allocator, item: *const Item) error{OutOfMemory}!queue.DeadLetter {
            var offer_key = try bytes(allocator, item.offer_key.items);
            errdefer offer_key.deinit();
            var payload = try bytes(allocator, item.payload.items);
            errdefer payload.deinit();
            return .{
                .sequence = .{ .sequence = item.sequence },
                .offer_key = offer_key,
                .encoded_payload = payload,
                .reason_code = try bytes(allocator, item.reason_code.?.items),
                .attempts = item.attempt,
            };
        }

        fn readNow(self: *Self, allocator: std.mem.Allocator, queue_key: []const u8, after: queue.Cursor, policy: queue.ReadPolicy) outcomes.Exit(queue.DeadLetterPage, Failure) {
            if (keys.inspectEncoded(queue_key) == .err or !policy.valid()) return .{ .fail = .invalid_policy };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            var count: usize = 0;
            var used: usize = 0;
            var more = false;
            for (self.items.items) |*item| {
                if (item.sequence <= after.sequence or item.state != .dead_letter or
                    !std.mem.eql(u8, item.queue_key.items, queue_key)) continue;
                const size = std.math.add(usize, item.payload.items.len, item.reason_code.?.items.len) catch return .{ .fail = .entry_exceeds_page_limit };
                if (size > policy.max_encoded_bytes and count == 0) return .{ .fail = .entry_exceeds_page_limit };
                if (count == policy.max_items or size > policy.max_encoded_bytes - used) {
                    more = true;
                    break;
                }
                used += size;
                count += 1;
            }
            const copies = allocator.alloc(queue.DeadLetter, count) catch return .{ .die = outcomes.Defect.out_of_memory };
            var copied: usize = 0;
            var keep = false;
            defer if (!keep) {
                for (copies[0..copied]) |*entry| entry.deinit();
                allocator.free(copies);
            };
            var next = after;
            for (self.items.items) |*item| {
                if (copied == count) break;
                if (item.sequence <= after.sequence or item.state != .dead_letter or
                    !std.mem.eql(u8, item.queue_key.items, queue_key)) continue;
                copies[copied] = deadCopy(allocator, item) catch return .{ .die = outcomes.Defect.out_of_memory };
                copied += 1;
                next = .{ .sequence = item.sequence };
            }
            keep = true;
            return .{ .success = .{ .items = .{ .items = copies, .allocator = allocator }, .next = next, .more = more } };
        }

        fn requeueEffect(data: ?*anyopaque, env: *const R, queue_key: []const u8, offer_key: []const u8, visible_at: deadlines.WallInstant) Store.RequeueEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.RequeueEffect.fromOwned(env, RequeueCapture{
                .owner = owner,
                .queue_key = queue_key,
                .offer_key = offer_key,
                .visible_at = visible_at,
            }, struct {
                fn run(capture: *const RequeueCapture, _: *const R) outcomes.Exit(queue.RequeueResult, Failure) {
                    return capture.owner.requeueNow(capture.queue_key, capture.offer_key, capture.visible_at);
                }
            }.run);
        }

        fn requeueNow(self: *Self, queue_key: []const u8, offer_key: []const u8, visible_at: deadlines.WallInstant) outcomes.Exit(queue.RequeueResult, Failure) {
            if (keys.inspectEncoded(queue_key) == .err or keys.inspectEncoded(offer_key) == .err) return .{ .fail = .malformed };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            const item = self.find(queue_key, offer_key) orelse return .{ .success = .missing };
            if (item.state != .dead_letter) return .{ .success = .not_dead_letter };
            if (item.reason_code) |*reason| reason.deinit();
            item.reason_code = null;
            item.visible_at = visible_at;
            item.state = .pending;
            return .{ .success = .requeued };
        }

        fn cleanupEffect(data: ?*anyopaque, env: *const R, queue_key: []const u8, max_items: usize, instant: deadlines.WallInstant) Store.CleanupEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.CleanupEffect.fromOwned(env, CleanupCapture{
                .owner = owner,
                .queue_key = queue_key,
                .max_items = max_items,
                .instant = instant,
            }, struct {
                fn run(capture: *const CleanupCapture, _: *const R) outcomes.Exit(queue.CleanupResult, Failure) {
                    return capture.owner.cleanupNow(capture.queue_key, capture.max_items, capture.instant);
                }
            }.run);
        }

        fn cleanupNow(self: *Self, queue_key: []const u8, max_items: usize, instant: deadlines.WallInstant) outcomes.Exit(queue.CleanupResult, Failure) {
            if (keys.inspectEncoded(queue_key) == .err or max_items == 0) return .{ .fail = .invalid_policy };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            var removed: usize = 0;
            var index: usize = 0;
            while (index < self.items.items.len and removed < max_items) {
                const item = &self.items.items[index];
                if (item.state == .acknowledged and item.retention_until.isDue(instant) and
                    std.mem.eql(u8, item.queue_key.items, queue_key))
                {
                    var departed = self.items.orderedRemove(index);
                    departed.deinit();
                    removed += 1;
                } else index += 1;
            }
            return .{ .success = .{ .removed = removed } };
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
