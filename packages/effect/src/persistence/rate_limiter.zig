//! Provider-neutral atomic rate admission. A memory implementation is local
//! only; sharing a limit across processes requires a conforming shared store.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const deadlines = @import("../deadline.zig");
const keys = @import("../primary_key.zig");

pub const FixedWindow = struct {
    capacity: u64,
    window: deadlines.Duration,

    pub fn valid(self: FixedWindow) bool {
        return self.capacity > 0 and self.window.nanoseconds > 0;
    }
};

pub const TokenBucket = struct {
    capacity: u64,
    refill_tokens: u64,
    refill_every: deadlines.Duration,

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

    pub fn valid(self: Adaptive) bool {
        return self.minimum_capacity > 0 and
            self.minimum_capacity <= self.initial_capacity and
            self.initial_capacity <= self.maximum_capacity and
            self.increase_by > 0 and self.decrease_by > 0 and
            self.window.nanoseconds > 0;
    }
};

pub const Rule = union(enum) {
    fixed_window: FixedWindow,
    token_bucket: TokenBucket,
    adaptive: Adaptive,
};

pub const Policy = struct {
    /// An intentional policy change selects new local/shared counter state.
    revision: u32,
    rule: Rule,

    pub fn valid(self: Policy) bool {
        if (self.revision == 0) return false;
        return switch (self.rule) {
            .fixed_window => |value| value.valid(),
            .token_bucket => |value| value.valid(),
            .adaptive => |value| value.valid(),
        };
    }

    pub fn capacity(self: Policy) u64 {
        return switch (self.rule) {
            .fixed_window => |value| value.capacity,
            .token_bucket => |value| value.capacity,
            .adaptive => |value| value.maximum_capacity,
        };
    }
};

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

pub const FeedbackResult = union(enum) {
    applied: u64,
    ignored_stale: u64,
    possibly_applied,
};

pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
        provider: ProviderError,
        invalid_policy,
        stale_policy,
        invalid_key,
        invalid_feedback,
        time_overflow,
        unavailable,

        pub fn deinit(self: *Self) void {
            if (self.* == .provider) outcomes.discardPayload(&self.provider);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

pub fn Store(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const Failure = Error(ProviderError);
        pub const TakeEffect = effects.Effect(Decision, Failure, R);
        pub const FeedbackEffect = effects.Effect(FeedbackResult, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            take: *const fn (?*anyopaque, *const R, []const u8, Policy, u64, deadlines.WallInstant) TakeEffect,
            feedback: *const fn (?*anyopaque, *const R, []const u8, Policy, Feedback, deadlines.WallInstant) FeedbackEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

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
