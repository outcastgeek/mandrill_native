//! Atomic acceptance, claim and reply contracts for logical entity delivery.
//! Plain persistence get/set cannot implement these guarantees by naming a
//! callback `transaction`; a provider must supply the specified transitions.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const types = @import("cluster_types.zig");
const ClusterError = @import("cluster_error.zig").Error;

/// Owned bytes buffer paired with its allocator; release it with `deinit`.
pub const Bytes = outcomes.OwnedSlice(u8);
/// Storage or provider durability promised by this capability. `recoverable`
/// permits compositions that must survive process restart; `volatile` does not.
pub const Guarantee = enum { @"volatile", recoverable };

/// Encoded entity request awaiting durable acceptance by the message store.
pub const Submission = struct {
    address: types.Address,
    definition_version: u32,
    encoded_request: []const u8,
    retention_until: deadlines.Deadline,
};

/// `Acceptance` is a tagged choice.
pub const Acceptance = union(enum) {
    accepted,
    already_accepted,
    already_completed,
    /// The provider cannot prove whether the request became authoritative.
    /// Read the saved reply or retry the same stable request identity.
    possibly_accepted,
};

/// Accepted request leased to one worker; ownership and attempt tokens fence completion.
pub const ClaimedMessage = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    group: Bytes,
    entity_key: Bytes,
    request_key: Bytes,
    encoded_request: Bytes,
    definition_version: u32,
    owner_id: Bytes,
    generation: u64,
    delivery_attempt: u64,

    /// Expose the claim or cancellation token.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`types.ClaimToken`): A typed fenced claim token borrowing its owner’s identity bytes.
    pub fn token(self: *const @This()) types.ClaimToken {
        return .{ .address = .{ .group = self.group.items, .entity_key = self.entity_key.items }, .request_key = self.request_key.items, .owner_id = self.owner_id.items, .generation = self.generation, .delivery_attempt = self.delivery_attempt };
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.group.deinit();
        self.entity_key.deinit();
        self.request_key.deinit();
        self.encoded_request.deinit();
        self.owner_id.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
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

/// Owned claim batch buffer paired with its allocator; release it with `deinit`.
pub const ClaimBatch = outcomes.OwnedSlice(ClaimedMessage);

/// Caller-selected maximum claims and claim lease deadline.
pub const ClaimPolicy = struct {
    max_messages: usize,
    lease_until: deadlines.Deadline,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: ClaimPolicy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for a positive message bound and a finite claim lease.
    pub fn valid(self: ClaimPolicy) bool {
        return self.max_messages > 0 and self.lease_until != .never;
    }
};

/// Claim attempt result distinguishing leased work from no available work.
pub const ClaimResult = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    claimed: ClaimBatch,
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
            .claimed => |batch| .{ .claimed = try batch.clone() },
            else => self.*,
        };
    }
};

/// Stored entity reply, including encoded outcome and chunk progress.
pub const Reply = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    pending,
    completed: Bytes,
    missing,
    expired,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .completed) self.completed.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .completed => |bytes| .{ .completed = try bytes.clone() },
            else => self.*,
        };
    }
};

/// `Completion` is a tagged choice.
pub const Completion = union(enum) {
    committed,
    already_committed,
    stale_claim,
    possibly_committed,
};

/// Chunk sequences start after zero and advance contiguously for each request.
/// A provider records a chunk before reporting it to a streaming observer.
pub const SavedChunk = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    sequence: u64,
    encoded: Bytes,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.encoded.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .sequence = self.sequence, .encoded = try self.encoded.clone() };
    }
};

/// Caller-selected maximum saved chunks and encoded bytes.
pub const ChunkPolicy = struct {
    max_chunks: usize,
    max_encoded_bytes: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: ChunkPolicy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when both chunk count and encoded-byte limits are positive.
    pub fn valid(self: ChunkPolicy) bool {
        return self.max_chunks > 0 and self.max_encoded_bytes > 0;
    }
};

/// Bounded page of saved reply chunks with next sequence and continuation state.
pub const ChunkPage = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    chunks: outcomes.OwnedSlice(SavedChunk),
    next_sequence: u64,
    more: bool,
    completed: bool,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.chunks.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .chunks = try self.chunks.clone(), .next_sequence = self.next_sequence, .more = self.more, .completed = self.completed };
    }
};

/// Read result distinguishing saved chunks from an unavailable reply.
pub const ChunkRead = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    available: ChunkPage,
    missing,
    expired,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .available) self.available.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .available => |value| .{ .available = try value.clone() },
            else => self.*,
        };
    }
};

/// `ChunkWrite` cases: `recorded`, `already_recorded`, `stale_claim`, `possibly_recorded`.
pub const ChunkWrite = enum { recorded, already_recorded, stale_claim, possibly_recorded };
/// `ChunkAck` cases: `acknowledged`, `already_acknowledged`, `missing`, `unknown_sequence`,
/// `uncertain`.
pub const ChunkAck = enum { acknowledged, already_acknowledged, missing, unknown_sequence, uncertain };

/// `Acknowledgement` is a tagged choice.
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
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// Returns (`type`): message-store provider contract for atomic request deduplication, fenced claims, and typed replies.
pub fn MessageStore(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        /// Compile-time volatility or recoverability promised by this provider.
        pub const storage_guarantee = guarantee;
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = ClusterError(ProviderError);
        /// Lazy Effect for accept effect; success, error, and environment types are fixed by
        /// `effects.Effect(Acceptance, Failure, R)`.
        pub const AcceptEffect = effects.Effect(Acceptance, Failure, R);
        /// Lazy Effect for claim effect; success, error, and environment types are fixed by
        /// `effects.Effect(ClaimResult, Failure, R)`.
        pub const ClaimEffect = effects.Effect(ClaimResult, Failure, R);
        /// Lazy Effect for complete effect; success, error, and environment types are fixed by
        /// `effects.Effect(Completion, Failure, R)`.
        pub const CompleteEffect = effects.Effect(Completion, Failure, R);
        /// Lazy Effect for chunk write effect; success, error, and environment types are fixed by
        /// `effects.Effect(ChunkWrite, Failure, R)`.
        pub const ChunkWriteEffect = effects.Effect(ChunkWrite, Failure, R);
        /// Lazy Effect for chunk read effect; success, error, and environment types are fixed by
        /// `effects.Effect(ChunkRead, Failure, R)`.
        pub const ChunkReadEffect = effects.Effect(ChunkRead, Failure, R);
        /// Lazy Effect for chunk ack effect; success, error, and environment types are fixed by
        /// `effects.Effect(ChunkAck, Failure, R)`.
        pub const ChunkAckEffect = effects.Effect(ChunkAck, Failure, R);
        /// Lazy Effect for reply effect; success, error, and environment types are fixed by
        /// `effects.Effect(Reply, Failure, R)`.
        pub const ReplyEffect = effects.Effect(Reply, Failure, R);
        /// Lazy Effect for ack effect; success, error, and environment types are fixed by
        /// `effects.Effect(Acknowledgement, Failure, R)`.
        pub const AckEffect = effects.Effect(Acknowledgement, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Accept through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Submission`: typed domain value consumed by this hook.
            /// Returns (`AcceptEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            accept: *const fn (?*anyopaque, *const R, Submission) AcceptEffect,
            /// Claim available work.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.EntityAddress`: stable typed identity for the selected operation.
            /// - `4: types.Ownership`: typed domain value consumed by this hook.
            /// - `5: ClaimPolicy`: caller-selected policy controlling this operation.
            /// Returns (`ClaimEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            claim: *const fn (?*anyopaque, *const R, types.EntityAddress, types.Ownership, ClaimPolicy) ClaimEffect,
            /// Record completion.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ClaimToken`: stable typed identity for the selected operation.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`CompleteEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            complete: *const fn (?*anyopaque, *const R, types.ClaimToken, []const u8) CompleteEffect,
            /// Append chunk through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ClaimToken`: stable typed identity for the selected operation.
            /// - `4: u64`: typed counter or optional sequence value.
            /// - `5: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`ChunkWriteEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            append_chunk: *const fn (?*anyopaque, *const R, types.ClaimToken, u64, []const u8) ChunkWriteEffect,
            /// Read chunks through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.Address`: stable typed identity for the selected operation.
            /// - `4: u64`: typed counter or optional sequence value.
            /// - `5: ChunkPolicy`: caller-selected policy controlling this operation.
            /// Returns (`ChunkReadEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            read_chunks: *const fn (?*anyopaque, *const R, types.Address, u64, ChunkPolicy) ChunkReadEffect,
            /// Acknowledge chunk through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.Address`: stable typed identity for the selected operation.
            /// - `4: u64`: typed counter or optional sequence value.
            /// Returns (`ChunkAckEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            acknowledge_chunk: *const fn (?*anyopaque, *const R, types.Address, u64) ChunkAckEffect,
            /// Read reply through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.Address`: stable typed identity for the selected operation.
            /// Returns (`ReplyEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            read_reply: *const fn (?*anyopaque, *const R, types.Address) ReplyEffect,
            /// Acknowledge reply through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.Address`: stable typed identity for the selected operation.
            /// Returns (`AckEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            acknowledge_reply: *const fn (?*anyopaque, *const R, types.Address) AckEffect,
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

        /// Accept a message into the cluster store.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_submission: comptime fn (*const R) Submission`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`AcceptEffect`): durable acceptance or deduplication of a keyed entity request.
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

        /// Claim eligible work under the provider’s lease policy.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_address: comptime fn (*const R) types.EntityAddress`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_owner: comptime fn (*const R) types.Ownership`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_policy: comptime fn (*const R) ClaimPolicy`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`ClaimEffect`): lease-fenced batch of accepted messages eligible for processing.
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

        /// Record successful completion of the operation.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_claim: comptime fn (*const R) types.ClaimToken`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_reply: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`CompleteEffect`): stored terminal reply committed only under the current claim fence.
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

        /// Append a response chunk for a claimed message.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_claim: comptime fn (*const R) types.ClaimToken`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_sequence: comptime fn (*const R) u64`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_bytes: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`ChunkWriteEffect`): saved reply chunk accepted only at the next valid sequence.
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

        /// Read response chunks after the selected cursor.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_address: comptime fn (*const R) types.Address`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_after: comptime fn (*const R) u64`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_policy: comptime fn (*const R) ChunkPolicy`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`ChunkReadEffect`): bounded page of saved streaming reply chunks.
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

        /// Acknowledge response chunks through the selected cursor.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_address: comptime fn (*const R) types.Address`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_through: comptime fn (*const R) u64`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`ChunkAckEffect`): acknowledgement advancing the consumer’s saved-chunk cursor.
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

        /// Read the terminal reply for a routed request.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_address: comptime fn (*const R) types.Address`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`ReplyEffect`): stored terminal reply for the request, if available.
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

        /// Acknowledge receipt of a terminal reply.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_address: comptime fn (*const R) types.Address`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`AckEffect`): acknowledgement of the consumed terminal reply.
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
