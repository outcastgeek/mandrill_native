//! Atomic acceptance, claim and reply contracts for logical entity delivery.
//! Plain persistence get/set cannot implement these guarantees by naming a
//! callback `transaction`; a provider must supply the specified transitions.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const types = @import("cluster_types.zig");
const ClusterError = @import("cluster_error.zig").Error;

pub const Bytes = outcomes.OwnedSlice(u8);
pub const Guarantee = enum { @"volatile", recoverable };

pub const Submission = struct {
    address: types.Address,
    definition_version: u32,
    encoded_request: []const u8,
    retention_until: deadlines.Deadline,
};

pub const Acceptance = union(enum) {
    accepted,
    already_accepted,
    already_completed,
    /// The provider cannot prove whether the request became authoritative.
    /// Read the saved reply or retry the same stable request identity.
    possibly_accepted,
};

pub const ClaimedMessage = struct {
    pub const effect_owned_payload = true;
    group: Bytes,
    entity_key: Bytes,
    request_key: Bytes,
    encoded_request: Bytes,
    definition_version: u32,
    owner_id: Bytes,
    generation: u64,
    delivery_attempt: u64,

    pub fn token(self: *const @This()) types.ClaimToken {
        return .{ .address = .{ .group = self.group.items, .entity_key = self.entity_key.items }, .request_key = self.request_key.items, .owner_id = self.owner_id.items, .generation = self.generation, .delivery_attempt = self.delivery_attempt };
    }

    pub fn deinit(self: *@This()) void {
        self.group.deinit();
        self.entity_key.deinit();
        self.request_key.deinit();
        self.encoded_request.deinit();
        self.owner_id.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var group = try self.group.clone();
        errdefer group.deinit();
        var entity_key = try self.entity_key.clone();
        errdefer entity_key.deinit();
        var request_key = try self.request_key.clone();
        errdefer request_key.deinit();
        var encoded_request = try self.encoded_request.clone();
        errdefer encoded_request.deinit();
        return .{
            .group = group,
            .entity_key = entity_key,
            .request_key = request_key,
            .encoded_request = encoded_request,
            .definition_version = self.definition_version,
            .owner_id = try self.owner_id.clone(),
            .generation = self.generation,
            .delivery_attempt = self.delivery_attempt,
        };
    }
};

pub const ClaimBatch = outcomes.OwnedSlice(ClaimedMessage);

pub const ClaimPolicy = struct {
    max_messages: usize,
    lease_until: deadlines.Deadline,

    pub fn valid(self: ClaimPolicy) bool {
        return self.max_messages > 0 and self.lease_until != .never;
    }
};

pub const ClaimResult = union(enum) {
    pub const effect_owned_payload = true;
    claimed: ClaimBatch,
    none_available,
    ownership_lost,

    pub fn deinit(self: *@This()) void {
        if (self.* == .claimed) self.claimed.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .claimed => |batch| .{ .claimed = try batch.clone() },
            else => self.*,
        };
    }
};

pub const Reply = union(enum) {
    pub const effect_owned_payload = true;
    pending,
    completed: Bytes,
    missing,
    expired,

    pub fn deinit(self: *@This()) void {
        if (self.* == .completed) self.completed.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .completed => |bytes| .{ .completed = try bytes.clone() },
            else => self.*,
        };
    }
};

pub const Completion = union(enum) {
    committed,
    already_committed,
    stale_claim,
    possibly_committed,
};

/// Chunk sequences start after zero and advance contiguously for each request.
/// A provider records a chunk before reporting it to a streaming observer.
pub const SavedChunk = struct {
    pub const effect_owned_payload = true;
    sequence: u64,
    encoded: Bytes,

    pub fn deinit(self: *@This()) void {
        self.encoded.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .sequence = self.sequence, .encoded = try self.encoded.clone() };
    }
};

pub const ChunkPolicy = struct {
    max_chunks: usize,
    max_encoded_bytes: usize,

    pub fn valid(self: ChunkPolicy) bool {
        return self.max_chunks > 0 and self.max_encoded_bytes > 0;
    }
};

pub const ChunkPage = struct {
    pub const effect_owned_payload = true;
    chunks: outcomes.OwnedSlice(SavedChunk),
    next_sequence: u64,
    more: bool,
    completed: bool,

    pub fn deinit(self: *@This()) void {
        self.chunks.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .chunks = try self.chunks.clone(), .next_sequence = self.next_sequence, .more = self.more, .completed = self.completed };
    }
};

pub const ChunkRead = union(enum) {
    pub const effect_owned_payload = true;
    available: ChunkPage,
    missing,
    expired,

    pub fn deinit(self: *@This()) void {
        if (self.* == .available) self.available.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .available => |value| .{ .available = try value.clone() },
            else => self.*,
        };
    }
};

pub const ChunkWrite = enum { recorded, already_recorded, stale_claim, possibly_recorded };
pub const ChunkAck = enum { acknowledged, already_acknowledged, missing, unknown_sequence, uncertain };

pub const Acknowledgement = union(enum) {
    acknowledged,
    already_acknowledged,
    reply_missing,
    uncertain,
};

/// A provider must atomically deduplicate acceptance by request identity and
/// reject a reused identity with different request bytes or definition. Claim
/// and completion are fenced by a current ownership proof for this exact
/// entity as well as generation and attempt. A claim lease cannot outlive
/// that ownership lease. Replacing or releasing ownership and publishing a
/// completion share one linearization boundary; a stale completion cannot
/// commit merely because nobody has claimed the message under a new owner.
/// Saved streaming chunks are ordered from sequence one and fenced by the
/// same claim as final completion. Duplicate sequence/bytes are idempotent;
/// conflicting bytes or a gap are rejected. Reads use a caller bound and
/// acknowledgement advances monotonically. Replies and unacknowledged chunks
/// remain inspectable until retention permits pruning; pruning never removes
/// unfinished or unacknowledged work. Observer cancel does not cancel accepted
/// execution. `recoverable` is a stronger provider promise tested by
/// conformance; the in-memory implementation is volatile.
pub fn MessageStore(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        pub const storage_guarantee = guarantee;
        pub const Failure = ClusterError(ProviderError);
        pub const AcceptEffect = effects.Effect(Acceptance, Failure, R);
        pub const ClaimEffect = effects.Effect(ClaimResult, Failure, R);
        pub const CompleteEffect = effects.Effect(Completion, Failure, R);
        pub const ChunkWriteEffect = effects.Effect(ChunkWrite, Failure, R);
        pub const ChunkReadEffect = effects.Effect(ChunkRead, Failure, R);
        pub const ChunkAckEffect = effects.Effect(ChunkAck, Failure, R);
        pub const ReplyEffect = effects.Effect(Reply, Failure, R);
        pub const AckEffect = effects.Effect(Acknowledgement, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            accept: *const fn (?*anyopaque, *const R, Submission) AcceptEffect,
            claim: *const fn (?*anyopaque, *const R, types.EntityAddress, types.Ownership, ClaimPolicy) ClaimEffect,
            complete: *const fn (?*anyopaque, *const R, types.ClaimToken, []const u8) CompleteEffect,
            append_chunk: *const fn (?*anyopaque, *const R, types.ClaimToken, u64, []const u8) ChunkWriteEffect,
            read_chunks: *const fn (?*anyopaque, *const R, types.Address, u64, ChunkPolicy) ChunkReadEffect,
            acknowledge_chunk: *const fn (?*anyopaque, *const R, types.Address, u64) ChunkAckEffect,
            read_reply: *const fn (?*anyopaque, *const R, types.Address) ReplyEffect,
            acknowledge_reply: *const fn (?*anyopaque, *const R, types.Address) AckEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn accept(comptime select_store: fn (*const R) *const Self, comptime select_submission: fn (*const R) Submission) AcceptEffect {
            return AcceptEffect.deferEffect(struct {
                fn start(env: *const R) AcceptEffect {
                    const submission = select_submission(env);
                    if (submission.address.validate() == .err) return AcceptEffect.fail(.malformed);
                    const store = select_store(env);
                    return store.vtable.accept(store.userdata, env, submission);
                }
            }.start);
        }

        pub fn claim(comptime select_store: fn (*const R) *const Self, comptime select_address: fn (*const R) types.EntityAddress, comptime select_owner: fn (*const R) types.Ownership, comptime select_policy: fn (*const R) ClaimPolicy) ClaimEffect {
            return ClaimEffect.deferEffect(struct {
                fn start(env: *const R) ClaimEffect {
                    const policy = select_policy(env);
                    if (!policy.valid()) return ClaimEffect.fail(.malformed);
                    const address = select_address(env);
                    if (!address.valid()) return ClaimEffect.fail(.malformed);
                    const owner = select_owner(env);
                    if (owner.owner_id.len == 0 or owner.generation == 0) return ClaimEffect.fail(.malformed);
                    const store = select_store(env);
                    return store.vtable.claim(store.userdata, env, address, owner, policy);
                }
            }.start);
        }

        pub fn complete(comptime select_store: fn (*const R) *const Self, comptime select_claim: fn (*const R) types.ClaimToken, comptime select_reply: fn (*const R) []const u8) CompleteEffect {
            return CompleteEffect.deferEffect(struct {
                fn start(env: *const R) CompleteEffect {
                    const claim_token = select_claim(env);
                    if ((types.Address{ .group = claim_token.address.group, .entity_key = claim_token.address.entity_key, .request_key = claim_token.request_key }).validate() == .err or
                        claim_token.owner_id.len == 0 or claim_token.generation == 0 or
                        claim_token.delivery_attempt == 0) return CompleteEffect.fail(.malformed);
                    const store = select_store(env);
                    return store.vtable.complete(store.userdata, env, claim_token, select_reply(env));
                }
            }.start);
        }

        pub fn appendChunk(comptime select_store: fn (*const R) *const Self, comptime select_claim: fn (*const R) types.ClaimToken, comptime select_sequence: fn (*const R) u64, comptime select_bytes: fn (*const R) []const u8) ChunkWriteEffect {
            return ChunkWriteEffect.deferEffect(struct {
                fn start(env: *const R) ChunkWriteEffect {
                    const claim_token = select_claim(env);
                    const sequence = select_sequence(env);
                    if ((types.Address{ .group = claim_token.address.group, .entity_key = claim_token.address.entity_key, .request_key = claim_token.request_key }).validate() == .err or
                        claim_token.owner_id.len == 0 or claim_token.generation == 0 or claim_token.delivery_attempt == 0 or sequence == 0)
                        return ChunkWriteEffect.fromOwnedOutcome(env, .{ .fail = .malformed });
                    const store = select_store(env);
                    return store.vtable.append_chunk(store.userdata, env, claim_token, sequence, select_bytes(env));
                }
            }.start);
        }

        pub fn readChunks(comptime select_store: fn (*const R) *const Self, comptime select_address: fn (*const R) types.Address, comptime select_after: fn (*const R) u64, comptime select_policy: fn (*const R) ChunkPolicy) ChunkReadEffect {
            return ChunkReadEffect.deferEffect(struct {
                fn start(env: *const R) ChunkReadEffect {
                    const address = select_address(env);
                    const policy = select_policy(env);
                    if (address.validate() == .err)
                        return ChunkReadEffect.fromOwnedOutcome(env, .{ .fail = .malformed });
                    if (!policy.valid())
                        return ChunkReadEffect.fromOwnedOutcome(env, .{ .fail = .invalid_policy });
                    const store = select_store(env);
                    return store.vtable.read_chunks(store.userdata, env, address, select_after(env), policy);
                }
            }.start);
        }

        pub fn acknowledgeChunk(comptime select_store: fn (*const R) *const Self, comptime select_address: fn (*const R) types.Address, comptime select_through: fn (*const R) u64) ChunkAckEffect {
            return ChunkAckEffect.deferEffect(struct {
                fn start(env: *const R) ChunkAckEffect {
                    const address = select_address(env);
                    const through = select_through(env);
                    if (address.validate() == .err or through == 0)
                        return ChunkAckEffect.fromOwnedOutcome(env, .{ .fail = .malformed });
                    const store = select_store(env);
                    return store.vtable.acknowledge_chunk(store.userdata, env, address, through);
                }
            }.start);
        }

        pub fn readReply(comptime select_store: fn (*const R) *const Self, comptime select_address: fn (*const R) types.Address) ReplyEffect {
            return ReplyEffect.deferEffect(struct {
                fn start(env: *const R) ReplyEffect {
                    const address = select_address(env);
                    if (address.validate() == .err) return ReplyEffect.fail(.malformed);
                    const store = select_store(env);
                    return store.vtable.read_reply(store.userdata, env, address);
                }
            }.start);
        }

        pub fn acknowledgeReply(comptime select_store: fn (*const R) *const Self, comptime select_address: fn (*const R) types.Address) AckEffect {
            return AckEffect.deferEffect(struct {
                fn start(env: *const R) AckEffect {
                    const address = select_address(env);
                    if (address.validate() == .err) return AckEffect.fail(.malformed);
                    const store = select_store(env);
                    return store.vtable.acknowledge_reply(store.userdata, env, address);
                }
            }.start);
        }
    };
}
