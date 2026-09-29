//! Named encoded queues with at-least-once delivery. An acknowledged offer
//! is retained until the caller's retention rule permits cleanup. A worker's
//! external side effect can happen before acknowledgement; retry may repeat it.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const deadlines = @import("../deadline.zig");
const keys = @import("../primary_key.zig");
const PersistenceError = @import("persistence_error.zig").Error;

pub const Bytes = outcomes.OwnedSlice(u8);
pub const Guarantee = enum { @"volatile", recoverable };
pub const Cursor = struct {
    sequence: u64,
    pub const before_first: Cursor = .{ .sequence = 0 };
};

pub const Submission = struct {
    queue_key: []const u8,
    offer_key: []const u8,
    encoded_payload: []const u8,
    visible_at: deadlines.WallInstant,
    retention_until: deadlines.Deadline,
};

pub const OfferResult = enum { accepted, already_offered, possibly_offered };

pub const ClaimPolicy = struct {
    max_items: usize,
    lease_until: deadlines.Deadline,

    pub fn valid(self: ClaimPolicy) bool {
        return self.max_items > 0 and self.lease_until != .never;
    }
};

pub const ClaimToken = struct {
    queue_key: []const u8,
    offer_key: []const u8,
    owner_id: []const u8,
    generation: u64,
    attempt: u64,
};

pub const ClaimedItem = struct {
    pub const effect_owned_payload = true;
    queue_key: Bytes,
    offer_key: Bytes,
    encoded_payload: Bytes,
    owner_id: Bytes,
    generation: u64,
    attempt: u64,

    pub fn token(self: *const @This()) ClaimToken {
        return .{
            .queue_key = self.queue_key.items,
            .offer_key = self.offer_key.items,
            .owner_id = self.owner_id.items,
            .generation = self.generation,
            .attempt = self.attempt,
        };
    }

    pub fn deinit(self: *@This()) void {
        self.queue_key.deinit();
        self.offer_key.deinit();
        self.encoded_payload.deinit();
        self.owner_id.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var queue_key = try self.queue_key.clone();
        errdefer queue_key.deinit();
        var offer_key = try self.offer_key.clone();
        errdefer offer_key.deinit();
        var encoded_payload = try self.encoded_payload.clone();
        errdefer encoded_payload.deinit();
        return .{
            .queue_key = queue_key,
            .offer_key = offer_key,
            .encoded_payload = encoded_payload,
            .owner_id = try self.owner_id.clone(),
            .generation = self.generation,
            .attempt = self.attempt,
        };
    }
};

pub const ClaimResult = union(enum) {
    pub const effect_owned_payload = true;
    claimed: outcomes.OwnedSlice(ClaimedItem),
    none_available,
    ownership_lost,

    pub fn deinit(self: *@This()) void {
        if (self.* == .claimed) self.claimed.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .claimed => |items| .{ .claimed = try items.clone() },
            else => self.*,
        };
    }
};

pub const AckResult = enum { acknowledged, already_acknowledged, stale_claim, possibly_acknowledged };
pub const RetryResult = enum { scheduled, already_scheduled, stale_claim, possibly_scheduled };
pub const DeadLetterResult = enum { recorded, already_recorded, stale_claim, possibly_recorded };
pub const RequeueResult = enum { requeued, not_dead_letter, missing, possibly_requeued };
pub const CleanupResult = struct { removed: usize };

pub const DeadLetter = struct {
    pub const effect_owned_payload = true;
    sequence: Cursor,
    offer_key: Bytes,
    encoded_payload: Bytes,
    reason_code: Bytes,
    attempts: u64,

    pub fn deinit(self: *@This()) void {
        self.offer_key.deinit();
        self.encoded_payload.deinit();
        self.reason_code.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var offer_key = try self.offer_key.clone();
        errdefer offer_key.deinit();
        var encoded_payload = try self.encoded_payload.clone();
        errdefer encoded_payload.deinit();
        return .{
            .sequence = self.sequence,
            .offer_key = offer_key,
            .encoded_payload = encoded_payload,
            .reason_code = try self.reason_code.clone(),
            .attempts = self.attempts,
        };
    }
};

pub const ReadPolicy = struct {
    max_items: usize,
    max_encoded_bytes: usize,

    pub fn valid(self: ReadPolicy) bool {
        return self.max_items > 0 and self.max_encoded_bytes > 0;
    }
};

pub const DeadLetterPage = struct {
    pub const effect_owned_payload = true;
    items: outcomes.OwnedSlice(DeadLetter),
    next: Cursor,
    more: bool,

    pub fn deinit(self: *@This()) void {
        self.items.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .items = try self.items.clone(), .next = self.next, .more = self.more };
    }
};

/// A provider must atomically deduplicate an offer, fence claims by owner,
/// generation and attempt, and keep raw corrupt payloads inspectable. A plain
/// key/value get/set provider does not satisfy these transitions. Only a
/// recoverable implementation can promise restart-stable accepted work.
pub fn Store(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        pub const storage_guarantee = guarantee;
        pub const Failure = PersistenceError(ProviderError);
        pub const OfferEffect = effects.Effect(OfferResult, Failure, R);
        pub const ClaimEffect = effects.Effect(ClaimResult, Failure, R);
        pub const AckEffect = effects.Effect(AckResult, Failure, R);
        pub const RetryEffect = effects.Effect(RetryResult, Failure, R);
        pub const DeadLetterEffect = effects.Effect(DeadLetterResult, Failure, R);
        pub const ReadEffect = effects.Effect(DeadLetterPage, Failure, R);
        pub const RequeueEffect = effects.Effect(RequeueResult, Failure, R);
        pub const CleanupEffect = effects.Effect(CleanupResult, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            offer: *const fn (?*anyopaque, *const R, Submission) OfferEffect,
            claim: *const fn (?*anyopaque, *const R, []const u8, []const u8, u64, ClaimPolicy) ClaimEffect,
            acknowledge: *const fn (?*anyopaque, *const R, ClaimToken) AckEffect,
            retry: *const fn (?*anyopaque, *const R, ClaimToken, deadlines.WallInstant) RetryEffect,
            dead_letter: *const fn (?*anyopaque, *const R, ClaimToken, []const u8) DeadLetterEffect,
            read_dead_letters: *const fn (?*anyopaque, *const R, []const u8, Cursor, ReadPolicy) ReadEffect,
            requeue: *const fn (?*anyopaque, *const R, []const u8, []const u8, deadlines.WallInstant) RequeueEffect,
            cleanup: *const fn (?*anyopaque, *const R, []const u8, usize, deadlines.WallInstant) CleanupEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn offer(comptime select_store: fn (*const R) *const Self, comptime select_submission: fn (*const R) Submission) OfferEffect {
            return OfferEffect.deferEffect(struct {
                fn start(env: *const R) OfferEffect {
                    const submission = select_submission(env);
                    if (keys.inspectEncoded(submission.queue_key) == .err or keys.inspectEncoded(submission.offer_key) == .err)
                        return OfferEffect.fail(.malformed);
                    const store = select_store(env);
                    return store.vtable.offer(store.userdata, env, submission);
                }
            }.start);
        }

        pub fn claim(
            comptime select_store: fn (*const R) *const Self,
            comptime select_queue: fn (*const R) []const u8,
            comptime select_owner: fn (*const R) []const u8,
            comptime select_generation: fn (*const R) u64,
            comptime select_policy: fn (*const R) ClaimPolicy,
        ) ClaimEffect {
            return ClaimEffect.deferEffect(struct {
                fn start(env: *const R) ClaimEffect {
                    const queue = select_queue(env);
                    const owner = select_owner(env);
                    const policy = select_policy(env);
                    if (keys.inspectEncoded(queue) == .err or owner.len == 0 or !policy.valid())
                        return ClaimEffect.fail(.invalid_policy);
                    const store = select_store(env);
                    return store.vtable.claim(store.userdata, env, queue, owner, select_generation(env), policy);
                }
            }.start);
        }

        pub fn acknowledge(comptime select_store: fn (*const R) *const Self, comptime select_token: fn (*const R) ClaimToken) AckEffect {
            return AckEffect.deferEffect(struct {
                fn start(env: *const R) AckEffect {
                    const token = select_token(env);
                    if (!validToken(token)) return AckEffect.fail(.malformed);
                    const store = select_store(env);
                    return store.vtable.acknowledge(store.userdata, env, token);
                }
            }.start);
        }

        pub fn retry(
            comptime select_store: fn (*const R) *const Self,
            comptime select_token: fn (*const R) ClaimToken,
            comptime select_visible_at: fn (*const R) deadlines.WallInstant,
        ) RetryEffect {
            return RetryEffect.deferEffect(struct {
                fn start(env: *const R) RetryEffect {
                    const token = select_token(env);
                    if (!validToken(token)) return RetryEffect.fail(.malformed);
                    const store = select_store(env);
                    return store.vtable.retry(store.userdata, env, token, select_visible_at(env));
                }
            }.start);
        }

        pub fn deadLetter(
            comptime select_store: fn (*const R) *const Self,
            comptime select_token: fn (*const R) ClaimToken,
            comptime select_reason: fn (*const R) []const u8,
        ) DeadLetterEffect {
            return DeadLetterEffect.deferEffect(struct {
                fn start(env: *const R) DeadLetterEffect {
                    const token = select_token(env);
                    const reason = select_reason(env);
                    if (!validToken(token) or reason.len == 0 or !std.unicode.utf8ValidateSlice(reason))
                        return DeadLetterEffect.fail(.malformed);
                    const store = select_store(env);
                    return store.vtable.dead_letter(store.userdata, env, token, reason);
                }
            }.start);
        }

        pub fn readDeadLetters(
            comptime select_store: fn (*const R) *const Self,
            comptime select_queue: fn (*const R) []const u8,
            comptime select_after: fn (*const R) Cursor,
            comptime select_policy: fn (*const R) ReadPolicy,
        ) ReadEffect {
            return ReadEffect.deferEffect(struct {
                fn start(env: *const R) ReadEffect {
                    const queue = select_queue(env);
                    const policy = select_policy(env);
                    if (keys.inspectEncoded(queue) == .err or !policy.valid()) return ReadEffect.fail(.invalid_policy);
                    const store = select_store(env);
                    return store.vtable.read_dead_letters(store.userdata, env, queue, select_after(env), policy);
                }
            }.start);
        }

        pub fn requeue(
            comptime select_store: fn (*const R) *const Self,
            comptime select_queue: fn (*const R) []const u8,
            comptime select_offer: fn (*const R) []const u8,
            comptime select_visible_at: fn (*const R) deadlines.WallInstant,
        ) RequeueEffect {
            return RequeueEffect.deferEffect(struct {
                fn start(env: *const R) RequeueEffect {
                    const queue = select_queue(env);
                    const offer_id = select_offer(env);
                    if (keys.inspectEncoded(queue) == .err or keys.inspectEncoded(offer_id) == .err)
                        return RequeueEffect.fail(.malformed);
                    const store = select_store(env);
                    return store.vtable.requeue(store.userdata, env, queue, offer_id, select_visible_at(env));
                }
            }.start);
        }

        pub fn cleanup(
            comptime select_store: fn (*const R) *const Self,
            comptime select_queue: fn (*const R) []const u8,
            comptime select_max_items: fn (*const R) usize,
            comptime select_now: fn (*const R) deadlines.WallInstant,
        ) CleanupEffect {
            return CleanupEffect.deferEffect(struct {
                fn start(env: *const R) CleanupEffect {
                    const queue = select_queue(env);
                    const max_items = select_max_items(env);
                    if (keys.inspectEncoded(queue) == .err or max_items == 0) return CleanupEffect.fail(.invalid_policy);
                    const store = select_store(env);
                    return store.vtable.cleanup(store.userdata, env, queue, max_items, select_now(env));
                }
            }.start);
        }

        fn validToken(token: ClaimToken) bool {
            return keys.inspectEncoded(token.queue_key) == .ok and
                keys.inspectEncoded(token.offer_key) == .ok and token.owner_id.len > 0 and token.attempt > 0;
        }
    };
}
