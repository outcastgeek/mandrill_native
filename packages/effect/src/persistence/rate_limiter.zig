//! Provider-neutral atomic rate admission. A memory implementation is local
//! only; sharing a limit across processes requires a conforming shared store.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const deadlines = @import("../deadline.zig");
const keys = @import("../primary_key.zig");

/// Fixed-window limit and duration chosen by the caller.
pub const FixedWindow = struct {
    capacity: u64,
    window: deadlines.Duration,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: FixedWindow`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for positive fixed-window capacity and duration.
    pub fn valid(self: FixedWindow) bool {
        return self.capacity > 0 and self.window.nanoseconds > 0;
    }
};

/// Token capacity and refill policy chosen by the caller.
pub const TokenBucket = struct {
    capacity: u64,
    refill_tokens: u64,
    refill_every: deadlines.Duration,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: TokenBucket`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for positive bucket capacity, refill amount, and refill interval.
    pub fn valid(self: TokenBucket) bool {
        return self.capacity > 0 and self.refill_tokens > 0 and self.refill_every.nanoseconds > 0;
    }
};

/// Admission uses the current capacity in a fixed window. Feedback changes
/// that capacity under the same store gate as admissions. Every policy number
/// is chosen by the application; the core has no tuning constants.
pub const Adaptive = struct {
    minimum_capacity: u64,
    initial_capacity: u64,
    maximum_capacity: u64,
    increase_by: u64,
    decrease_by: u64,
    window: deadlines.Duration,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Adaptive`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when capacity bounds and all adjustment steps satisfy the adaptive policy.
    pub fn valid(self: Adaptive) bool {
        return self.minimum_capacity > 0 and
            self.minimum_capacity <= self.initial_capacity and
            self.initial_capacity <= self.maximum_capacity and
            self.increase_by > 0 and self.decrease_by > 0 and
            self.window.nanoseconds > 0;
    }
};

/// Selected fixed-window or token-bucket rate-limit algorithm.
pub const Rule = union(enum) {
    fixed_window: FixedWindow,
    token_bucket: TokenBucket,
    adaptive: Adaptive,
};

/// Versioned rate-limit rule used to reject incompatible stored state.
pub const Policy = struct {
    /// An intentional policy change selects new local/shared counter state.
    revision: u32,
    rule: Rule,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Policy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for a positive revision and valid selected rate-limit rule.
    pub fn valid(self: Policy) bool {
        if (self.revision == 0) return false;
        return switch (self.rule) {
            .fixed_window => |value| value.valid(),
            .token_bucket => |value| value.valid(),
            .adaptive => |value| value.valid(),
        };
    }

    /// Return the capacity allowed by this rate-limit policy.
    /// Parameters:
    /// - `self: Policy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`u64`): The current numeric capacity or revision.
    pub fn capacity(self: Policy) u64 {
        return switch (self.rule) {
            .fixed_window => |value| value.capacity,
            .token_bucket => |value| value.capacity,
            .adaptive => |value| value.maximum_capacity,
        };
    }
};

/// Admission result: allowed with remaining capacity or limited until a deadline.
pub const Decision = union(enum) {
    allowed: struct { remaining: u64 },
    limited: struct { retry_after: deadlines.Duration },
    possibly_applied,
};

/// The producer assigns increasing sequence numbers within one key/policy
/// revision. Duplicate or reordered observations never undo newer feedback.
pub const Feedback = struct {
    sequence: u64,
    signal: enum { healthy, overloaded },
};

/// Whether feedback updated current state or was ignored as stale.
pub const FeedbackResult = union(enum) {
    applied: u64,
    ignored_stale: u64,
    possibly_applied,
};

/// Specialize the typed persistence error channel from its compile-time error type.
/// Parameters:
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): typed rate-limit failure union retaining provider errors.
pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
        provider: ProviderError,
        invalid_policy,
        stale_policy,
        invalid_key,
        invalid_feedback,
        time_overflow,
        unavailable,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .provider) outcomes.discardPayload(&self.provider);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

/// Specialize the persistence provider interface and its typed Effect channels.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): rate-limit state provider interface for atomic policy decisions.
pub fn Store(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = Error(ProviderError);
        /// Lazy Effect for take effect; success, error, and environment types are fixed by
        /// `effects.Effect(Decision, Failure, R)`.
        pub const TakeEffect = effects.Effect(Decision, Failure, R);
        /// Lazy Effect for feedback effect; success, error, and environment types are fixed by
        /// `effects.Effect(FeedbackResult, Failure, R)`.
        pub const FeedbackEffect = effects.Effect(FeedbackResult, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Take through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `4: Policy`: caller-selected policy controlling this operation.
            /// - `5: u64`: typed counter or optional sequence value.
            /// - `6: deadlines.WallInstant`: typed input consumed by this hook.
            /// Returns (`TakeEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            take: *const fn (?*anyopaque, *const R, []const u8, Policy, u64, deadlines.WallInstant) TakeEffect,
            /// Feedback through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `4: Policy`: caller-selected policy controlling this operation.
            /// - `5: Feedback`: typed domain value consumed by this hook.
            /// - `6: deadlines.WallInstant`: typed input consumed by this hook.
            /// Returns (`FeedbackEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            feedback: *const fn (?*anyopaque, *const R, []const u8, Policy, Feedback, deadlines.WallInstant) FeedbackEffect,
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

        /// Take one item or permit from the selected capability.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_policy: comptime fn (*const R) Policy`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_cost: comptime fn (*const R) u64`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_now: comptime fn (*const R) deadlines.WallInstant`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`TakeEffect`): rate-limit decision after atomically applying the selected rule.
        pub fn take(
            comptime select_store: fn (*const R) *const Self,
            comptime select_key: fn (*const R) []const u8,
            comptime select_policy: fn (*const R) Policy,
            comptime select_cost: fn (*const R) u64,
            comptime select_now: fn (*const R) deadlines.WallInstant,
        ) TakeEffect {
            return TakeEffect.deferEffect(struct {
                fn start(env: *const R) TakeEffect {
                    const key = select_key(env);
                    if (keys.inspectEncoded(key) == .err) return TakeEffect.fail(.invalid_key);
                    const policy = select_policy(env);
                    const cost = select_cost(env);
                    if (!policy.valid() or cost == 0 or cost > policy.capacity()) return TakeEffect.fail(.invalid_policy);
                    const store = select_store(env);
                    return store.vtable.take(store.userdata, env, key, policy, cost, select_now(env));
                }
            }.start);
        }

        /// Record provider feedback for future rate decisions.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_policy: comptime fn (*const R) Policy`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_feedback: comptime fn (*const R) Feedback`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_now: comptime fn (*const R) deadlines.WallInstant`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`FeedbackEffect`): typed acknowledgement of feedback applied to the current policy revision.
        pub fn recordFeedback(
            comptime select_store: fn (*const R) *const Self,
            comptime select_key: fn (*const R) []const u8,
            comptime select_policy: fn (*const R) Policy,
            comptime select_feedback: fn (*const R) Feedback,
            comptime select_now: fn (*const R) deadlines.WallInstant,
        ) FeedbackEffect {
            return FeedbackEffect.deferEffect(struct {
                fn start(env: *const R) FeedbackEffect {
                    const key = select_key(env);
                    if (keys.inspectEncoded(key) == .err) return FeedbackEffect.fail(.invalid_key);
                    const policy = select_policy(env);
                    if (!policy.valid() or policy.rule != .adaptive)
                        return FeedbackEffect.fail(.invalid_policy);
                    const feedback = select_feedback(env);
                    if (feedback.sequence == 0) return FeedbackEffect.fail(.invalid_feedback);
                    const store = select_store(env);
                    return store.vtable.feedback(store.userdata, env, key, policy, feedback, select_now(env));
                }
            }.start);
        }
    };
}
