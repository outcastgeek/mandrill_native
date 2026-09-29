//! Recoverable deadline contract. Wall deadlines are data; local monotonic
//! sleeps remain the caller's runtime concern. Due work is claimed and fenced.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const workflow = @import("workflow_types.zig");

pub const Identity = struct {
    execution: workflow.ExecutionId,
    occurrence: []const u8,

    pub fn valid(self: Identity) bool {
        return self.execution.validate() == .ok and self.occurrence.len > 0 and
            std.unicode.utf8ValidateSlice(self.occurrence);
    }
};

pub const Schedule = struct { identity: Identity, due_at: deadlines.WallInstant };
pub const ScheduleResult = enum { scheduled, already_scheduled, conflicting_deadline, possibly_scheduled };
pub const CancelResult = enum { cancelled, already_cancelled, absent, possibly_cancelled };
pub const ClaimPolicy = struct {
    owner_id: []const u8,
    generation: u64,
    lease_until: deadlines.Deadline,
    max_items: usize,

    pub fn valid(self: ClaimPolicy) bool {
        return self.owner_id.len > 0 and self.max_items > 0 and self.lease_until != .never;
    }
};

pub const Claimed = struct {
    pub const effect_owned_payload = true;
    execution_key: workflow.Bytes,
    occurrence: workflow.Bytes,
    due_at: deadlines.WallInstant,
    owner_id: workflow.Bytes,
    generation: u64,
    attempt: u64,

    pub fn deinit(self: *Claimed) void {
        self.execution_key.deinit();
        self.occurrence.deinit();
        self.owner_id.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const Claimed) error{OutOfMemory}!Claimed {
        var execution_key = try self.execution_key.clone();
        errdefer execution_key.deinit();
        var occurrence = try self.occurrence.clone();
        errdefer occurrence.deinit();
        return .{ .execution_key = execution_key, .occurrence = occurrence, .due_at = self.due_at, .owner_id = try self.owner_id.clone(), .generation = self.generation, .attempt = self.attempt };
    }
};

pub const ClaimedBatch = outcomes.OwnedSlice(Claimed);
pub const ClaimResult = union(enum) {
    pub const effect_owned_payload = true;
    claimed: ClaimedBatch,
    none_due,
    ownership_lost,

    pub fn deinit(self: *@This()) void {
        if (self.* == .claimed) self.claimed.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .claimed => |value| .{ .claimed = try value.clone() },
            else => self.*,
        };
    }
};

pub const Token = struct {
    identity: Identity,
    owner_id: []const u8,
    generation: u64,
    attempt: u64,
};
pub const Acknowledgement = enum { acknowledged, already_acknowledged, stale_claim, uncertain };
pub const Failure = enum { malformed, unavailable, invalid_policy, attempt_exhausted };
pub const Guarantee = enum { @"volatile", recoverable };

pub fn Store(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        pub const storage_guarantee = guarantee;
        pub const Error = union(enum) {
            const Own = @This();
            pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
            operation: Failure,
            provider: ProviderError,

            pub fn deinit(self: *Own) void {
                if (self.* == .provider) outcomes.discardPayload(&self.provider);
                self.* = undefined;
            }

            pub fn clone(self: *const Own) error{OutOfMemory}!Own {
                return switch (self.*) {
                    .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                    else => self.*,
                };
            }
        };
        pub const ScheduleEffect = effects.Effect(ScheduleResult, Error, R);
        pub const CancelEffect = effects.Effect(CancelResult, Error, R);
        pub const ClaimEffect = effects.Effect(ClaimResult, Error, R);
        pub const AckEffect = effects.Effect(Acknowledgement, Error, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            schedule: *const fn (?*anyopaque, *const R, Schedule) ScheduleEffect,
            cancel: *const fn (?*anyopaque, *const R, Identity) CancelEffect,
            claim_due: *const fn (?*anyopaque, *const R, ClaimPolicy) ClaimEffect,
            acknowledge: *const fn (?*anyopaque, *const R, Token) AckEffect,
        };

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
