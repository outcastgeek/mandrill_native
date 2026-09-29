//! Portable encoded storage contracts. Conditional updates are a stronger
//! capability, never inferred from ordinary get/set behavior.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const deadlines = @import("../deadline.zig");
const PersistenceError = @import("persistence_error.zig").Error;

/// Owned bytes buffer paired with its allocator; release it with `deinit`.
pub const Bytes = outcomes.OwnedSlice(u8);
/// Deadline after which the stored value may expire.
pub const Expiry = deadlines.Deadline;

/// Key lookup: owned found bytes or a missing value.
pub const Read = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    found: Bytes,
    missing,
    expired,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .found) self.found.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .found => |value| .{ .found = try value.clone() },
            else => self.*,
        };
    }
};

/// Version marker distinguishing absent from a present revision.
pub const Revision = union(enum) { absent, present: u64 };
/// Owned encoded bytes paired with their stored revision.
pub const Versioned = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    bytes: Bytes,
    revision: Revision,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.bytes.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .bytes = try self.bytes.clone(), .revision = self.revision };
    }
};

/// Versioned lookup result, including a missing key.
pub const ReadVersioned = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    found: Versioned,
    missing,
    expired,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .found) self.found.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .found => |value| .{ .found = try value.clone() },
            else => self.*,
        };
    }
};

/// `WriteAcknowledgement` cases: `committed`, `possibly_committed`.
pub const WriteAcknowledgement = enum { committed, possibly_committed };
/// Compare-and-exchange result indicating applied revision or conflict.
pub const ConditionalAcknowledgement = union(enum) {
    applied: Revision,
    conflict: Revision,
    possibly_applied,
};

/// Providers return owned read buffers allocated from R. They copy borrowed
/// write buffers before committing or retaining them. The callback constructs
/// an Effect; the existing kernel then owns any suspension/cancellation.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): encoded key/value provider returning owned buffers in the caller allocation domain.
pub fn KeyValueStore(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = PersistenceError(ProviderError);
        /// Lazy Effect for read effect; success, error, and environment types are fixed by
        /// `effects.Effect(Read, Failure, R)`.
        pub const ReadEffect = effects.Effect(Read, Failure, R);
        /// Lazy Effect for write effect; success, error, and environment types are fixed by
        /// `effects.Effect(WriteAcknowledgement, Failure, R)`.
        pub const WriteEffect = effects.Effect(WriteAcknowledgement, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Read the selected value.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`ReadEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            read: *const fn (?*anyopaque, *const R, []const u8) ReadEffect,
            /// Write the supplied value.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `5: Expiry`: caller-selected policy controlling this operation.
            /// Returns (`WriteEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            write: *const fn (?*anyopaque, *const R, []const u8, []const u8, Expiry) WriteEffect,
            /// Remove the selected value.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`WriteEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            remove: *const fn (?*anyopaque, *const R, []const u8) WriteEffect,
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

        /// Read through the selected capability.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`ReadEffect`): owned bytes for a present key, or a missing-key result.
        pub fn read(comptime select_store: fn (*const R) *const Self, comptime select_key: fn (*const R) []const u8) ReadEffect {
            return ReadEffect.deferEffect(struct {
                fn run(env: *const R) ReadEffect {
                    const store = select_store(env);
                    return store.vtable.read(store.userdata, env, select_key(env));
                }
            }.run);
        }

        /// Write through the selected capability.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_value: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_expiry: comptime fn (*const R) Expiry`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`WriteEffect`): typed acknowledgement after the provider stores the encoded bytes.
        pub fn write(
            comptime select_store: fn (*const R) *const Self,
            comptime select_key: fn (*const R) []const u8,
            comptime select_value: fn (*const R) []const u8,
            comptime select_expiry: fn (*const R) Expiry,
        ) WriteEffect {
            return WriteEffect.deferEffect(struct {
                fn run(env: *const R) WriteEffect {
                    const store = select_store(env);
                    return store.vtable.write(store.userdata, env, select_key(env), select_value(env), select_expiry(env));
                }
            }.run);
        }

        /// Remove the selected entry.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`WriteEffect`): typed acknowledgement after the provider removes the key.
        pub fn remove(comptime select_store: fn (*const R) *const Self, comptime select_key: fn (*const R) []const u8) WriteEffect {
            return WriteEffect.deferEffect(struct {
                fn run(env: *const R) WriteEffect {
                    const store = select_store(env);
                    return store.vtable.remove(store.userdata, env, select_key(env));
                }
            }.run);
        }
    };
}

/// A successful compareExchange is linearizable for this key, including its
/// revision and expiry update. Implementations unable to uphold that contract
/// cannot construct a conforming conditional capability.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): versioned compare-and-exchange provider for one-key linearizable updates.
pub fn ConditionalStore(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = PersistenceError(ProviderError);
        /// Lazy Effect for read effect; success, error, and environment types are fixed by
        /// `effects.Effect(ReadVersioned, Failure, R)`.
        pub const ReadEffect = effects.Effect(ReadVersioned, Failure, R);
        /// Lazy Effect for compare effect; success, error, and environment types are fixed by
        /// `effects.Effect(ConditionalAcknowledgement, Failure, R)`.
        pub const CompareEffect = effects.Effect(ConditionalAcknowledgement, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Read versioned through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`ReadEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            read_versioned: *const fn (?*anyopaque, *const R, []const u8) ReadEffect,
            /// Compare exchange through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `4: Revision`: typed domain value consumed by this hook.
            /// - `5: ?[]const u8`: optional typed input; null means absent.
            /// - `6: Expiry`: caller-selected policy controlling this operation.
            /// Returns (`CompareEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            compare_exchange: *const fn (?*anyopaque, *const R, []const u8, Revision, ?[]const u8, Expiry) CompareEffect,
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

        /// Read through the selected capability.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`ReadEffect`): owned bytes plus revision for a present key, or a missing-key result.
        pub fn read(comptime select_store: fn (*const R) *const Self, comptime select_key: fn (*const R) []const u8) ReadEffect {
            return ReadEffect.deferEffect(struct {
                fn run(env: *const R) ReadEffect {
                    const store = select_store(env);
                    return store.vtable.read_versioned(store.userdata, env, select_key(env));
                }
            }.run);
        }

        /// Conditionally write when the expected revision still matches.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_expected: comptime fn (*const R) Revision`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_replacement: comptime fn (*const R) ?[]const u8`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_expiry: comptime fn (*const R) Expiry`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`CompareEffect`): new revision when the expected revision matches, or an explicit conflict.
        pub fn compareExchange(
            comptime select_store: fn (*const R) *const Self,
            comptime select_key: fn (*const R) []const u8,
            comptime select_expected: fn (*const R) Revision,
            comptime select_replacement: fn (*const R) ?[]const u8,
            comptime select_expiry: fn (*const R) Expiry,
        ) CompareEffect {
            return CompareEffect.deferEffect(struct {
                fn run(env: *const R) CompareEffect {
                    const store = select_store(env);
                    return store.vtable.compare_exchange(store.userdata, env, select_key(env), select_expected(env), select_replacement(env), select_expiry(env));
                }
            }.run);
        }
    };
}
