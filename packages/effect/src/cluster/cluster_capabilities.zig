//! Logical ownership capability, independent of membership, placement and
//! transport. MessageStore remains the separate authority for requests and
//! replies; an expired or replaced lease cannot complete its old claims.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const messages = @import("message_store.zig");
const types = @import("cluster_types.zig");

/// Typed cluster address of the selected entity.
pub const EntityAddress = types.EntityAddress;

/// Caller-selected ownership lease duration and renewal policy.
pub const LeasePolicy = struct {
    until: deadlines.Deadline,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: LeasePolicy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` only when the ownership lease has a finite deadline.
    pub fn valid(self: LeasePolicy) bool {
        return self.until != .never;
    }
};

/// Opaque ownership proof with generation information used to fence writes.
pub const Token = struct {
    address: EntityAddress,
    owner_id: []const u8,
    generation: u64,
};

/// A provider can hold its ownership linearization gate across a message
/// claim/completion transition. The message authority must release this guard
/// on every exit path. A cross-process provider supplies an equivalent atomic
/// proof; a read followed by an unguarded write is insufficient.
pub const Fence = struct {
    userdata: ?*anyopaque,
    enter: *const fn (?*anyopaque, Token, deadlines.WallInstant) Enter,

    /// `Guard` is a callback interface with `userdata: ?*anyopaque`, `until: deadlines.Deadline`,
    /// `release_fn: *const fn (?*anyopaque) void`.
    pub const Guard = struct {
        userdata: ?*anyopaque,
        until: deadlines.Deadline,
        release_fn: *const fn (?*anyopaque) void,

        /// Return or release the owned resource.
        /// Parameters:
        /// - `self: Guard`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// Returns (`void`): no value.
        pub fn release(self: Guard) void {
            self.release_fn(self.userdata);
        }
    };

    /// Ownership acquisition result indicating whether this caller became owner.
    pub const Enter = union(enum) { held: Guard, stale, unavailable };
};

/// Active entity ownership lease; renew or release it before its proof expires.
pub const Lease = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    group: messages.Bytes,
    entity_key: messages.Bytes,
    owner_id: messages.Bytes,
    generation: u64,
    until: deadlines.Deadline,

    /// Expose the claim or cancellation token.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`Token`): A typed claim or cancellation token borrowed from its owner.
    pub fn token(self: *const @This()) Token {
        return .{ .address = .{ .group = self.group.items, .entity_key = self.entity_key.items }, .owner_id = self.owner_id.items, .generation = self.generation };
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.group.deinit();
        self.entity_key.deinit();
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
        return .{ .group = group, .entity_key = entity_key, .owner_id = try self.owner_id.clone(), .generation = self.generation, .until = self.until };
    }
};

/// Typed result of acquiring a lease or observing existing ownership.
pub const Acquisition = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    acquired: Lease,
    unavailable,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .acquired) self.acquired.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .acquired => |value| .{ .acquired = try value.clone() },
            .unavailable => .unavailable,
        };
    }
};

/// `Renewal` cases: `renewed`, `stale`, `possibly_renewed`.
pub const Renewal = enum { renewed, stale, possibly_renewed };
/// `Release` cases: `released`, `already_released`, `stale`, `possibly_released`.
pub const Release = enum { released, already_released, stale, possibly_released };
/// Ownership-provider failure for invalid identity or deadline, unavailable service, or exhausted generation.
pub const Error = enum { invalid_identity, invalid_deadline, unavailable, generation_exhausted };
/// Storage or provider durability promised by this capability. `recoverable`
/// permits compositions that must survive process restart; `volatile` does not.
pub const Guarantee = enum { @"volatile", recoverable };

/// A conforming implementation linearizes acquire, renew and release for
/// each entity; generations never repeat. It must not execute user handlers
/// while holding its ownership gate. An ambiguous acknowledgement is surfaced
/// for read-back rather than assumed successful.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// Returns (`type`): fenced ownership capability whose acquisition, renewal, and release are linearized by its provider.
pub fn Ownership(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        /// Compile-time volatility or recoverability promised by this provider.
        pub const storage_guarantee = guarantee;
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = union(enum) {
            const Own = @This();
            /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
            /// `deinit` and must be released after use.
            pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
            operation: Error,
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
        /// Lazy Effect for acquire effect; success, error, and environment types are fixed by
        /// `effects.Effect(Acquisition, Failure, R)`.
        pub const AcquireEffect = effects.Effect(Acquisition, Failure, R);
        /// Lazy Effect for renew effect; success, error, and environment types are fixed by
        /// `effects.Effect(Renewal, Failure, R)`.
        pub const RenewEffect = effects.Effect(Renewal, Failure, R);
        /// Lazy Effect for release effect; success, error, and environment types are fixed by
        /// `effects.Effect(Release, Failure, R)`.
        pub const ReleaseEffect = effects.Effect(Release, Failure, R);
        /// Lazy Effect for read effect; success, error, and environment types are fixed by
        /// `effects.Effect(?Lease, Failure, R)`.
        pub const ReadEffect = effects.Effect(?Lease, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Acquire through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: EntityAddress`: stable typed identity for the selected operation.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `5: LeasePolicy`: caller-selected policy controlling this operation.
            /// Returns (`AcquireEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            acquire: *const fn (?*anyopaque, *const R, EntityAddress, []const u8, LeasePolicy) AcquireEffect,
            /// Renew through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Token`: stable typed identity for the selected operation.
            /// - `4: LeasePolicy`: caller-selected policy controlling this operation.
            /// Returns (`RenewEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            renew: *const fn (?*anyopaque, *const R, Token, LeasePolicy) RenewEffect,
            /// Release through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Token`: stable typed identity for the selected operation.
            /// Returns (`ReleaseEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            release: *const fn (?*anyopaque, *const R, Token) ReleaseEffect,
            /// Read the selected value.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: EntityAddress`: stable typed identity for the selected operation.
            /// Returns (`ReadEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            read: *const fn (?*anyopaque, *const R, EntityAddress) ReadEffect,
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
    };
}
