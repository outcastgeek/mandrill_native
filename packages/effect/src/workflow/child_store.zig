//! Parent/child completion authority. A completion is retained until its
//! parent has been woken and explicitly acknowledges it. A lost notification
//! can therefore be reconciled by reading pending completions again.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const types = @import("workflow_types.zig");
const workflow_engine = @import("workflow_engine.zig");

/// Durable parent/child execution relationship.
pub const Link = struct {
    parent: types.ExecutionId,
    child: types.ExecutionId,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Link`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when both execution IDs validate and parent differs from child.
    pub fn valid(self: Link) bool {
        return self.parent.validate() == .ok and self.child.validate() == .ok and
            !std.mem.eql(u8, self.parent.encoded, self.child.encoded);
    }
};

/// Child execution awaiting a recorded terminal outcome.
pub const Pending = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    child: outcomes.OwnedSlice(u8),
    encoded_exit: outcomes.OwnedSlice(u8),

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.child.deinit();
        self.encoded_exit.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var child = try self.child.clone();
        errdefer child.deinit();
        return .{ .child = child, .encoded_exit = try self.encoded_exit.clone() };
    }
};

/// `RegisterResult` cases: `registered`, `already_registered`, `conflicting_parent`.
pub const RegisterResult = enum { registered, already_registered, conflicting_parent };
/// `CompleteResult` cases: `recorded`, `already_recorded`, `conflicting_exit`, `missing_child`.
pub const CompleteResult = enum { recorded, already_recorded, conflicting_exit, missing_child };
/// `AcknowledgeResult` cases: `acknowledged`, `already_acknowledged`, `missing_child`, `not_completed`.
pub const AcknowledgeResult = enum { acknowledged, already_acknowledged, missing_child, not_completed };

/// Specialize the typed workflow error channel from its compile-time error type.
/// Parameters:
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): typed child-store failure union preserving its provider error.
pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
        invalid_identity,
        unavailable,
        provider: ProviderError,

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

/// A recoverable provider must commit link, completion and acknowledgement
/// transitions atomically within its own authority. The volatile memory
/// implementation exercises the same contract without claiming disk safety.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `guarantee: comptime workflow_engine.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// Returns (`type`): child-execution persistence capability with typed records and provider failures.
pub fn Store(comptime R: type, comptime ProviderError: type, comptime guarantee: workflow_engine.Guarantee) type {
    return struct {
        const Self = @This();
        /// Compile-time volatility or recoverability promised by this provider.
        pub const storage_guarantee = guarantee;
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = Error(ProviderError);
        /// Lazy Effect for register effect; success, error, and environment types are fixed by
        /// `effects.Effect(RegisterResult, Failure, R)`.
        pub const RegisterEffect = effects.Effect(RegisterResult, Failure, R);
        /// Lazy Effect for complete effect; success, error, and environment types are fixed by
        /// `effects.Effect(CompleteResult, Failure, R)`.
        pub const CompleteEffect = effects.Effect(CompleteResult, Failure, R);
        /// Lazy Effect for pending effect; success, error, and environment types are fixed by
        /// `effects.Effect(outcomes.OwnedSlice(Pending), Failure, R)`.
        pub const PendingEffect = effects.Effect(outcomes.OwnedSlice(Pending), Failure, R);
        /// Lazy Effect for acknowledge effect; success, error, and environment types are fixed by
        /// `effects.Effect(AcknowledgeResult, Failure, R)`.
        pub const AcknowledgeEffect = effects.Effect(AcknowledgeResult, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Register through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Link`: typed domain value consumed by this hook.
            /// Returns (`RegisterEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            register: *const fn (?*anyopaque, *const R, Link) RegisterEffect,
            /// Record completion.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Link`: typed domain value consumed by this hook.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`CompleteEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            complete: *const fn (?*anyopaque, *const R, Link, []const u8) CompleteEffect,
            /// Pending through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ExecutionId`: stable typed identity for the selected operation.
            /// Returns (`PendingEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            pending: *const fn (?*anyopaque, *const R, types.ExecutionId) PendingEffect,
            /// Acknowledge completed work.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Link`: typed domain value consumed by this hook.
            /// Returns (`AcknowledgeEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            acknowledge: *const fn (?*anyopaque, *const R, Link) AcknowledgeEffect,
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

        /// Register a definition or subscription with its owner.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_link: comptime fn (*const R) Link`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`RegisterEffect`): durable parent/child link or an existing matching registration.
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

        /// Record successful completion of the operation.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_link: comptime fn (*const R) Link`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_exit: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`CompleteEffect`): stored terminal child outcome accepted under its current identity.
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

        /// Read the count of currently pending operations.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_parent: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`PendingEffect`): bounded page of children still awaiting completion.
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

        /// Acknowledge processing through the selected capability.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_link: comptime fn (*const R) Link`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`AcknowledgeEffect`): typed acknowledgement that a completed child was observed.
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
