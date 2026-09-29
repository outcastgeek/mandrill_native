//! Named encoded queues with at-least-once delivery. An acknowledged offer
//! is retained until the caller's retention rule permits cleanup. A worker's
//! external side effect can happen before acknowledgement; retry may repeat it.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const deadlines = @import("../deadline.zig");
const keys = @import("../primary_key.zig");
const PersistenceError = @import("persistence_error.zig").Error;

/// Owned bytes buffer paired with its allocator; release it with `deinit`.
pub const Bytes = outcomes.OwnedSlice(u8);
/// Storage or provider durability promised by this capability. `recoverable`
/// permits compositions that must survive process restart; `volatile` does not.
pub const Guarantee = enum { @"volatile", recoverable };
/// Queue position used for incremental reads; `before_first` starts from the beginning.
pub const Cursor = struct {
    sequence: u64,
    /// Cursor preceding the first persisted queue item.
    pub const before_first: Cursor = .{ .sequence = 0 };
};

/// Borrowed queue key, idempotent offer key, encoded payload, visibility instant, and retention
/// deadline.
pub const Submission = struct {
    queue_key: []const u8,
    offer_key: []const u8,
    encoded_payload: []const u8,
    visible_at: deadlines.WallInstant,
    retention_until: deadlines.Deadline,
};

/// Offer acknowledgement; `possibly_offered` requires reconciliation before retrying.
pub const OfferResult = enum { accepted, already_offered, possibly_offered };

/// Maximum batch size and lease deadline selected by a worker before claiming work.
pub const ClaimPolicy = struct {
    max_items: usize,
    lease_until: deadlines.Deadline,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: ClaimPolicy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for a positive item bound and finite claim lease.
    pub fn valid(self: ClaimPolicy) bool {
        return self.max_items > 0 and self.lease_until != .never;
    }
};

/// Borrowed key, owner, generation, and attempt identifying one fenced claim.
pub const ClaimToken = struct {
    queue_key: []const u8,
    offer_key: []const u8,
    owner_id: []const u8,
    generation: u64,
    attempt: u64,
};

/// Owned copy of a claimed queue item and its fencing identity; release buffers with `deinit`.
pub const ClaimedItem = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    queue_key: Bytes,
    offer_key: Bytes,
    encoded_payload: Bytes,
    owner_id: Bytes,
    generation: u64,
    attempt: u64,

    /// Expose the claim or cancellation token.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`ClaimToken`): A typed fenced claim token borrowing its owner’s identity bytes.
    pub fn token(self: *const @This()) ClaimToken {
        return .{
            .queue_key = self.queue_key.items,
            .offer_key = self.offer_key.items,
            .owner_id = self.owner_id.items,
            .generation = self.generation,
            .attempt = self.attempt,
        };
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.queue_key.deinit();
        self.offer_key.deinit();
        self.encoded_payload.deinit();
        self.owner_id.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
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

/// Owned claimed items, no available work, or lost ownership of the queue claim.
pub const ClaimResult = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    claimed: outcomes.OwnedSlice(ClaimedItem),
    none_available,
    ownership_lost,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .claimed) self.claimed.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .claimed => |items| .{ .claimed = try items.clone() },
            else => self.*,
        };
    }
};

/// Acknowledgement outcome, distinguishing stale claims and ambiguous writes.
pub const AckResult = enum { acknowledged, already_acknowledged, stale_claim, possibly_acknowledged };
/// Retry-scheduling outcome, distinguishing stale claims and ambiguous writes.
pub const RetryResult = enum { scheduled, already_scheduled, stale_claim, possibly_scheduled };
/// Dead-letter transition outcome, distinguishing stale claims and ambiguous writes.
pub const DeadLetterResult = enum { recorded, already_recorded, stale_claim, possibly_recorded };
/// Result of moving a dead-letter item back to active delivery.
pub const RequeueResult = enum { requeued, not_dead_letter, missing, possibly_requeued };
/// Number of retained queue records removed by cleanup.
pub const CleanupResult = struct { removed: usize };

/// Owned record of an item moved to dead-letter storage; release buffers with `deinit`.
pub const DeadLetter = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    sequence: Cursor,
    offer_key: Bytes,
    encoded_payload: Bytes,
    reason_code: Bytes,
    attempts: u64,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.offer_key.deinit();
        self.encoded_payload.deinit();
        self.reason_code.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
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

/// Caller-selected page bound for reading dead-letter entries.
pub const ReadPolicy = struct {
    max_items: usize,
    max_encoded_bytes: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: ReadPolicy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when both item and encoded-byte bounds are positive.
    pub fn valid(self: ReadPolicy) bool {
        return self.max_items > 0 and self.max_encoded_bytes > 0;
    }
};

/// Owned page of dead-letter records and continuation cursor; release it with `deinit`.
pub const DeadLetterPage = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    items: outcomes.OwnedSlice(DeadLetter),
    next: Cursor,
    more: bool,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.items.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .items = try self.items.clone(), .next = self.next, .more = self.more };
    }
};

/// A provider must atomically deduplicate an offer, fence claims by owner,
/// generation and attempt, and keep raw corrupt payloads inspectable. A plain
/// key/value get/set provider does not satisfy these transitions. Only a
/// recoverable implementation can promise restart-stable accepted work.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// Returns (`type`): persisted queue provider contract with deduplicated offers and fenced claims.
pub fn Store(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        /// Compile-time volatility or recoverability promised by this provider.
        pub const storage_guarantee = guarantee;
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = PersistenceError(ProviderError);
        /// Lazy Effect for offer effect; success, error, and environment types are fixed by
        /// `effects.Effect(OfferResult, Failure, R)`.
        pub const OfferEffect = effects.Effect(OfferResult, Failure, R);
        /// Lazy Effect for claim effect; success, error, and environment types are fixed by
        /// `effects.Effect(ClaimResult, Failure, R)`.
        pub const ClaimEffect = effects.Effect(ClaimResult, Failure, R);
        /// Lazy Effect for ack effect; success, error, and environment types are fixed by
        /// `effects.Effect(AckResult, Failure, R)`.
        pub const AckEffect = effects.Effect(AckResult, Failure, R);
        /// Lazy Effect for retry effect; success, error, and environment types are fixed by
        /// `effects.Effect(RetryResult, Failure, R)`.
        pub const RetryEffect = effects.Effect(RetryResult, Failure, R);
        /// Lazy Effect for dead letter effect; success, error, and environment types are fixed by
        /// `effects.Effect(DeadLetterResult, Failure, R)`.
        pub const DeadLetterEffect = effects.Effect(DeadLetterResult, Failure, R);
        /// Lazy Effect for read effect; success, error, and environment types are fixed by
        /// `effects.Effect(DeadLetterPage, Failure, R)`.
        pub const ReadEffect = effects.Effect(DeadLetterPage, Failure, R);
        /// Lazy Effect for requeue effect; success, error, and environment types are fixed by
        /// `effects.Effect(RequeueResult, Failure, R)`.
        pub const RequeueEffect = effects.Effect(RequeueResult, Failure, R);
        /// Lazy Effect for cleanup effect; success, error, and environment types are fixed by
        /// `effects.Effect(CleanupResult, Failure, R)`.
        pub const CleanupEffect = effects.Effect(CleanupResult, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Offer through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Submission`: typed domain value consumed by this hook.
            /// Returns (`OfferEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            offer: *const fn (?*anyopaque, *const R, Submission) OfferEffect,
            /// Claim available work.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `5: u64`: typed counter or optional sequence value.
            /// - `6: ClaimPolicy`: caller-selected policy controlling this operation.
            /// Returns (`ClaimEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            claim: *const fn (?*anyopaque, *const R, []const u8, []const u8, u64, ClaimPolicy) ClaimEffect,
            /// Acknowledge completed work.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: ClaimToken`: stable typed identity for the selected operation.
            /// Returns (`AckEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            acknowledge: *const fn (?*anyopaque, *const R, ClaimToken) AckEffect,
            /// Retry through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: ClaimToken`: stable typed identity for the selected operation.
            /// - `4: deadlines.WallInstant`: typed input consumed by this hook.
            /// Returns (`RetryEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            retry: *const fn (?*anyopaque, *const R, ClaimToken, deadlines.WallInstant) RetryEffect,
            /// Dead letter through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: ClaimToken`: stable typed identity for the selected operation.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`DeadLetterEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            dead_letter: *const fn (?*anyopaque, *const R, ClaimToken, []const u8) DeadLetterEffect,
            /// Read dead letters through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `4: Cursor`: typed domain value consumed by this hook.
            /// - `5: ReadPolicy`: caller-selected policy controlling this operation.
            /// Returns (`ReadEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            read_dead_letters: *const fn (?*anyopaque, *const R, []const u8, Cursor, ReadPolicy) ReadEffect,
            /// Requeue through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `5: deadlines.WallInstant`: typed input consumed by this hook.
            /// Returns (`RequeueEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            requeue: *const fn (?*anyopaque, *const R, []const u8, []const u8, deadlines.WallInstant) RequeueEffect,
            /// Cleanup through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `4: usize`: caller-selected count or bound.
            /// - `5: deadlines.WallInstant`: typed input consumed by this hook.
            /// Returns (`CleanupEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            cleanup: *const fn (?*anyopaque, *const R, []const u8, usize, deadlines.WallInstant) CleanupEffect,
        };

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context passed unchanged to every hook.
        ///   Its owner keeps it alive through any pending effect or operation.
        /// - `vtable: *const VTable`: Borrowed table of exactly typed callbacks; keep it
        ///   alive for every use of this capability, including deferred execution.
        /// Returns (`Self`): A non-owning adapter referencing `data` and `vtable`; no
        ///   provider state is allocated or transferred by this constructor.
        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        /// Offer the value to the selected queue or store.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_submission: comptime fn (*const R) Submission`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`OfferEffect`): deduplicated persisted offer acknowledgement for the supplied stable request key.
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

        /// Claim eligible work under the provider’s lease policy.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_queue: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_owner: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_generation: comptime fn (*const R) u64`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_policy: comptime fn (*const R) ClaimPolicy`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`ClaimEffect`): lease-fenced page of queue items eligible for processing.
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

        /// Acknowledge processing through the selected capability.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_token: comptime fn (*const R) ClaimToken`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`AckEffect`): completion acknowledgement accepted only for the current claim owner.
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

        /// Retry according to the declared policy.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_token: comptime fn (*const R) ClaimToken`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_visible_at: comptime fn (*const R) deadlines.WallInstant`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`RetryEffect`): item returned for a later attempt under the provider’s retry contract.
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

        /// Move an exhausted queue item to dead-letter storage.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_token: comptime fn (*const R) ClaimToken`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_reason: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`DeadLetterEffect`): item moved to dead-letter storage under the current claim.
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

        /// Read dead-letter items after the selected cursor.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_queue: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_after: comptime fn (*const R) Cursor`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_policy: comptime fn (*const R) ReadPolicy`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`ReadEffect`): bounded page of dead-letter records and its continuation cursor.
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

        /// Return a dead-letter item to the active queue.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_queue: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_offer: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_visible_at: comptime fn (*const R) deadlines.WallInstant`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`RequeueEffect`): dead-letter item returned to the pending queue with a stable identity.
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

        /// Remove retained queue data whose deadline has passed.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_queue: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_max_items: comptime fn (*const R) usize`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_now: comptime fn (*const R) deadlines.WallInstant`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`CleanupEffect`): bounded cleanup progress for expired or completed queue records.
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
