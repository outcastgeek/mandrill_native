//! Parent/child completion authority. A completion is retained until its
//! parent has been woken and explicitly acknowledges it. A lost notification
//! can therefore be reconciled by reading pending completions again.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const types = @import("workflow_types.zig");
const workflow_engine = @import("workflow_engine.zig");

pub const Link = struct {
    parent: types.ExecutionId,
    child: types.ExecutionId,

    pub fn valid(self: Link) bool {
        return self.parent.validate() == .ok and self.child.validate() == .ok and
            !std.mem.eql(u8, self.parent.encoded, self.child.encoded);
    }
};

pub const Pending = struct {
    pub const effect_owned_payload = true;
    child: outcomes.OwnedSlice(u8),
    encoded_exit: outcomes.OwnedSlice(u8),

    pub fn deinit(self: *@This()) void {
        self.child.deinit();
        self.encoded_exit.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var child = try self.child.clone();
        errdefer child.deinit();
        return .{ .child = child, .encoded_exit = try self.encoded_exit.clone() };
    }
};

pub const RegisterResult = enum { registered, already_registered, conflicting_parent };
pub const CompleteResult = enum { recorded, already_recorded, conflicting_exit, missing_child };
pub const AcknowledgeResult = enum { acknowledged, already_acknowledged, missing_child, not_completed };

pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
        invalid_identity,
        unavailable,
        provider: ProviderError,

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

/// A recoverable provider must commit link, completion and acknowledgement
/// transitions atomically within its own authority. The volatile memory
/// implementation exercises the same contract without claiming disk safety.
pub fn Store(comptime R: type, comptime ProviderError: type, comptime guarantee: workflow_engine.Guarantee) type {
    return struct {
        const Self = @This();
        pub const storage_guarantee = guarantee;
        pub const Failure = Error(ProviderError);
        pub const RegisterEffect = effects.Effect(RegisterResult, Failure, R);
        pub const CompleteEffect = effects.Effect(CompleteResult, Failure, R);
        pub const PendingEffect = effects.Effect(outcomes.OwnedSlice(Pending), Failure, R);
        pub const AcknowledgeEffect = effects.Effect(AcknowledgeResult, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            register: *const fn (?*anyopaque, *const R, Link) RegisterEffect,
            complete: *const fn (?*anyopaque, *const R, Link, []const u8) CompleteEffect,
            pending: *const fn (?*anyopaque, *const R, types.ExecutionId) PendingEffect,
            acknowledge: *const fn (?*anyopaque, *const R, Link) AcknowledgeEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn register(comptime select_store: fn (*const R) *const Self, comptime select_link: fn (*const R) Link) RegisterEffect {
            return RegisterEffect.deferEffect(struct {
                fn start(env: *const R) RegisterEffect {
                    const link = select_link(env);
                    if (!link.valid()) return RegisterEffect.fail(.invalid_identity);
                    const store = select_store(env);
                    return store.vtable.register(store.userdata, env, link);
                }
            }.start);
        }

        pub fn complete(comptime select_store: fn (*const R) *const Self, comptime select_link: fn (*const R) Link, comptime select_exit: fn (*const R) []const u8) CompleteEffect {
            return CompleteEffect.deferEffect(struct {
                fn start(env: *const R) CompleteEffect {
                    const link = select_link(env);
                    if (!link.valid()) return CompleteEffect.fail(.invalid_identity);
                    const store = select_store(env);
                    return store.vtable.complete(store.userdata, env, link, select_exit(env));
                }
            }.start);
        }

        pub fn pending(comptime select_store: fn (*const R) *const Self, comptime select_parent: fn (*const R) types.ExecutionId) PendingEffect {
            return PendingEffect.deferEffect(struct {
                fn start(env: *const R) PendingEffect {
                    const parent = select_parent(env);
                    if (parent.validate() == .err) return PendingEffect.fail(.invalid_identity);
                    const store = select_store(env);
                    return store.vtable.pending(store.userdata, env, parent);
                }
            }.start);
        }

        pub fn acknowledge(comptime select_store: fn (*const R) *const Self, comptime select_link: fn (*const R) Link) AcknowledgeEffect {
            return AcknowledgeEffect.deferEffect(struct {
                fn start(env: *const R) AcknowledgeEffect {
                    const link = select_link(env);
                    if (!link.valid()) return AcknowledgeEffect.fail(.invalid_identity);
                    const store = select_store(env);
                    return store.vtable.acknowledge(store.userdata, env, link);
                }
            }.start);
        }
    };
}
