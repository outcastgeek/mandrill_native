//! Recoverable deadline contract. Wall deadlines are data; local monotonic
//! sleeps remain the caller's runtime concern. Due work is claimed and fenced.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const workflow = @import("workflow_types.zig");

/// Stable key of a durable workflow timer.
pub const Identity = struct {
    execution: workflow.ExecutionId,
    occurrence: []const u8,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Identity`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when execution identity validates and occurrence is nonempty UTF-8.
    pub fn valid(self: Identity) bool {
        return self.execution.validate() == .ok and self.occurrence.len > 0 and
            std.unicode.utf8ValidateSlice(self.occurrence);
    }
};

/// Persisted timer request and its wake deadline.
pub const Schedule = struct { identity: Identity, due_at: deadlines.WallInstant };
/// `ScheduleResult` cases: `scheduled`, `already_scheduled`, `conflicting_deadline`,
/// `possibly_scheduled`.
pub const ScheduleResult = enum { scheduled, already_scheduled, conflicting_deadline, possibly_scheduled };
/// `CancelResult` cases: `cancelled`, `already_cancelled`, `absent`, `possibly_cancelled`.
pub const CancelResult = enum { cancelled, already_cancelled, absent, possibly_cancelled };
/// Caller-selected timer claim count and lease deadline.
pub const ClaimPolicy = struct {
    owner_id: []const u8,
    generation: u64,
    lease_until: deadlines.Deadline,
    max_items: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: ClaimPolicy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when owner ID and item bound are nonempty/positive and lease deadline is finite.
    pub fn valid(self: ClaimPolicy) bool {
        return self.owner_id.len > 0 and self.max_items > 0 and self.lease_until != .never;
    }
};

/// Timer leased to a worker for one wake attempt.
pub const Claimed = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    execution_key: workflow.Bytes,
    occurrence: workflow.Bytes,
    due_at: deadlines.WallInstant,
    owner_id: workflow.Bytes,
    generation: u64,
    attempt: u64,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Claimed`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Claimed) void {
        self.execution_key.deinit();
        self.occurrence.deinit();
        self.owner_id.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const Claimed`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!Claimed`): A `Claimed` on success; `OutOfMemory` if required allocation fails.
    pub fn clone(self: *const Claimed) error{OutOfMemory}!Claimed {
        var execution_key = try self.execution_key.clone();
        errdefer execution_key.deinit();
        var occurrence = try self.occurrence.clone();
        errdefer occurrence.deinit();
        return .{ .execution_key = execution_key, .occurrence = occurrence, .due_at = self.due_at, .owner_id = try self.owner_id.clone(), .generation = self.generation, .attempt = self.attempt };
    }
};

/// Owned claimed batch buffer paired with its allocator; release it with `deinit`.
pub const ClaimedBatch = outcomes.OwnedSlice(Claimed);
/// Timer-claim outcome with leased work or no available timer.
pub const ClaimResult = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    claimed: ClaimedBatch,
    none_due,
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
            .claimed => |value| .{ .claimed = try value.clone() },
            else => self.*,
        };
    }
};

/// Fence identifying the worker allowed to complete a timer wake.
pub const Token = struct {
    identity: Identity,
    owner_id: []const u8,
    generation: u64,
    attempt: u64,
};
/// `Acknowledgement` cases: `acknowledged`, `already_acknowledged`, `stale_claim`, `uncertain`.
pub const Acknowledgement = enum { acknowledged, already_acknowledged, stale_claim, uncertain };
/// Typed failure channel combining domain and provider errors.
pub const Failure = enum { malformed, unavailable, invalid_policy, attempt_exhausted };
/// Storage or provider durability promised by this capability. `recoverable`
/// permits compositions that must survive process restart; `volatile` does not.
pub const Guarantee = enum { @"volatile", recoverable };

/// Specialize the workflow provider interface and its typed Effect channels.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// Returns (`type`): workflow clock provider interface with typed time and wake operations.
pub fn Store(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        /// Compile-time volatility or recoverability promised by this provider.
        pub const storage_guarantee = guarantee;
        /// Typed durable-timer failure; keep provider and schedule errors distinct.
        pub const Error = union(enum) {
            const Own = @This();
            /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
            /// `deinit` and must be released after use.
            pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
            operation: Failure,
            provider: ProviderError,

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *Own`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *Own) void {
                if (self.* == .provider) outcomes.discardPayload(&self.provider);
                self.* = undefined;
            }

            /// Duplicate this value; marked owning payloads are cloned, while ordinary
            /// borrowed fields retain their original lifetime.
            /// Parameters:
            /// - `self: *const Own`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
            /// Returns (`error{OutOfMemory}!Own`): A `Own` on success; `OutOfMemory` if required allocation fails.
            pub fn clone(self: *const Own) error{OutOfMemory}!Own {
                return switch (self.*) {
                    .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                    else => self.*,
                };
            }
        };
        /// Lazy Effect for schedule effect; success, error, and environment types are fixed by
        /// `effects.Effect(ScheduleResult, Error, R)`.
        pub const ScheduleEffect = effects.Effect(ScheduleResult, Error, R);
        /// Lazy Effect for cancel effect; success, error, and environment types are fixed by
        /// `effects.Effect(CancelResult, Error, R)`.
        pub const CancelEffect = effects.Effect(CancelResult, Error, R);
        /// Lazy Effect for claim effect; success, error, and environment types are fixed by
        /// `effects.Effect(ClaimResult, Error, R)`.
        pub const ClaimEffect = effects.Effect(ClaimResult, Error, R);
        /// Lazy Effect for ack effect; success, error, and environment types are fixed by
        /// `effects.Effect(Acknowledgement, Error, R)`.
        pub const AckEffect = effects.Effect(Acknowledgement, Error, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Schedule through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Schedule`: typed domain value consumed by this hook.
            /// Returns (`ScheduleEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            schedule: *const fn (?*anyopaque, *const R, Schedule) ScheduleEffect,
            /// Request cancellation.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Identity`: stable typed identity for the selected operation.
            /// Returns (`CancelEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            cancel: *const fn (?*anyopaque, *const R, Identity) CancelEffect,
            /// Claim due through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: ClaimPolicy`: caller-selected policy controlling this operation.
            /// Returns (`ClaimEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            claim_due: *const fn (?*anyopaque, *const R, ClaimPolicy) ClaimEffect,
            /// Acknowledge completed work.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Token`: stable typed identity for the selected operation.
            /// Returns (`AckEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            acknowledge: *const fn (?*anyopaque, *const R, Token) AckEffect,
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

        fn reject(comptime A: type, comptime problem: Failure) effects.Effect(A, Error, R) {
            return effects.Effect(A, Error, R).fromOutcome(struct {
                fn result(_: *const R) outcomes.Exit(A, Error) {
                    return .{ .fail = .{ .operation = problem } };
                }
            }.result);
        }

        /// Register a durable timer for the selected workflow.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_schedule: comptime fn (*const R) Schedule`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`ScheduleEffect`): durable timer registration under its stable identity and selected deadline.
        pub fn schedule(
            comptime select_store: fn (*const R) *const Self,
            comptime select_schedule: fn (*const R) Schedule,
        ) ScheduleEffect {
            return ScheduleEffect.deferEffect(struct {
                fn start(env: *const R) ScheduleEffect {
                    const value = select_schedule(env);
                    if (!value.identity.valid())
                        return reject(ScheduleResult, .malformed);
                    const store = select_store(env);
                    return store.vtable.schedule(store.userdata, env, value);
                }
            }.start);
        }

        /// Request cancellation of the selected operation.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_identity: comptime fn (*const R) Identity`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`CancelEffect`): typed acknowledgement after cancelling the selected timer.
        pub fn cancel(
            comptime select_store: fn (*const R) *const Self,
            comptime select_identity: fn (*const R) Identity,
        ) CancelEffect {
            return CancelEffect.deferEffect(struct {
                fn start(env: *const R) CancelEffect {
                    const identity = select_identity(env);
                    if (!identity.valid())
                        return reject(CancelResult, .malformed);
                    const store = select_store(env);
                    return store.vtable.cancel(store.userdata, env, identity);
                }
            }.start);
        }

        /// Claim timers whose deadlines have passed.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_policy: comptime fn (*const R) ClaimPolicy`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`ClaimEffect`): lease-fenced page of timers whose deadlines have passed.
        pub fn claimDue(
            comptime select_store: fn (*const R) *const Self,
            comptime select_policy: fn (*const R) ClaimPolicy,
        ) ClaimEffect {
            return ClaimEffect.deferEffect(struct {
                fn start(env: *const R) ClaimEffect {
                    const policy = select_policy(env);
                    if (!policy.valid())
                        return reject(ClaimResult, .invalid_policy);
                    const store = select_store(env);
                    return store.vtable.claim_due(store.userdata, env, policy);
                }
            }.start);
        }

        /// Acknowledge processing through the selected capability.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_token: comptime fn (*const R) Token`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`AckEffect`): typed acknowledgement of a claimed timer wake.
        pub fn acknowledge(
            comptime select_store: fn (*const R) *const Self,
            comptime select_token: fn (*const R) Token,
        ) AckEffect {
            return AckEffect.deferEffect(struct {
                fn start(env: *const R) AckEffect {
                    const token = select_token(env);
                    if (!token.identity.valid() or token.owner_id.len == 0 or token.attempt == 0)
                        return reject(Acknowledgement, .malformed);
                    const store = select_store(env);
                    return store.vtable.acknowledge(store.userdata, env, token);
                }
            }.start);
        }
    };
}
