//! Replaceable replication, cursor, session and protection boundaries. None
//! constructs a transport, clock, crypto backend or persistence engine.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const journal = @import("event_journal.zig");
const message = @import("event_log_message.zig");

/// Specialize the typed eventlog error channel from its compile-time error type.
/// Parameters:
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): typed replica-operation error union retaining transport and domain failures.
pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
        provider: ProviderError,
        malformed,
        disconnected,
        rejected_session,
        rejected_protection,
        uncertain_delivery,
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

/// `Delivery` cases: `accepted`, `possibly_accepted`, `rejected`.
pub const Delivery = enum { accepted, possibly_accepted, rejected };
/// Replica transport receive outcome with a message or end-of-stream state.
pub const Received = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    batch: message.Batch,
    disconnected,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .batch) self.batch.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .batch => |value| .{ .batch = try value.clone() },
            .disconnected => .disconnected,
        };
    }
};

/// The provider retains no borrowed batch fields after send completes.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): replica message transport contract with explicit send ownership.
pub fn Transport(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = Error(ProviderError);
        /// Lazy Effect for send effect; success, error, and environment types are fixed by
        /// `effects.Effect(Delivery, Failure, R)`.
        pub const SendEffect = effects.Effect(Delivery, Failure, R);
        /// Lazy Effect for receive effect; success, error, and environment types are fixed by
        /// `effects.Effect(Received, Failure, R)`.
        pub const ReceiveEffect = effects.Effect(Received, Failure, R);
        /// Lazy Effect for ack effect; success, error, and environment types are fixed by
        /// `effects.Effect(Delivery, Failure, R)`.
        pub const AckEffect = effects.Effect(Delivery, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Send batch through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: *const message.Batch`: borrowed read-only typed input; keep it valid through deferred use.
            /// Returns (`SendEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            send_batch: *const fn (?*anyopaque, *const R, *const message.Batch) SendEffect,
            /// Receive batch through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// Returns (`ReceiveEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            receive_batch: *const fn (?*anyopaque, *const R) ReceiveEffect,
            /// Acknowledge completed work.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: message.Acknowledgement`: typed domain value consumed by this hook.
            /// Returns (`AckEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            acknowledge: *const fn (?*anyopaque, *const R, message.Acknowledgement) AckEffect,
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

/// Outcome of advancing a replica cursor or discovering a conflict.
pub const Advance = union(enum) {
    committed: journal.Cursor,
    already_advanced: journal.Cursor,
    conflict: journal.Cursor,
    possibly_committed,
};

/// Progress is keyed by source replica. Implementations must compare and
/// advance atomically. Import may append before advancing, but stable event
/// IDs make a retry deduplicate the append after an uncertain cursor update.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): provider for durable per-replica import progress.
pub fn CursorStore(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = Error(ProviderError);
        /// Lazy Effect for read effect; success, error, and environment types are fixed by
        /// `effects.Effect(journal.Cursor, Failure, R)`.
        pub const ReadEffect = effects.Effect(journal.Cursor, Failure, R);
        /// Lazy Effect for advance effect; success, error, and environment types are fixed by
        /// `effects.Effect(Advance, Failure, R)`.
        pub const AdvanceEffect = effects.Effect(Advance, Failure, R);
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
            /// Compare and advance through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `4: journal.Cursor`: typed domain value consumed by this hook.
            /// - `5: journal.Cursor`: typed domain value consumed by this hook.
            /// Returns (`AdvanceEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            compare_and_advance: *const fn (?*anyopaque, *const R, []const u8, journal.Cursor, journal.Cursor) AdvanceEffect,
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

/// Specialize a provider interface for replica session validation or payload protection.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): provider that validates replica session authority.
pub fn SessionAuthority(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Lazy Effect for validate effect; success, error, and environment types are fixed by
        /// `effects.Effect(bool, Error(ProviderError), R)`.
        pub const ValidateEffect = effects.Effect(bool, Error(ProviderError), R);
        userdata: ?*anyopaque,
        validate: *const fn (?*anyopaque, *const R, message.Identity) ValidateEffect,

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context; keep it alive until
        ///   every validation Effect produced by `callback` has completed.
        /// - `callback: *const fn (?*anyopaque, *const R, message.Identity) ValidateEffect`:
        ///   Receives provider state, borrowed `R`, and event identity; returns
        ///   a lazy Effect whose success value says whether the session is valid.
        /// Returns (`Self`): A non-owning authority adapter; no provider state is transferred.
        pub fn fromProvider(data: ?*anyopaque, callback: *const fn (?*anyopaque, *const R, message.Identity) ValidateEffect) Self {
            return .{ .userdata = data, .validate = callback };
        }
    };
}

/// Specialize a provider interface for replica session validation or payload protection.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): provider that authenticates or protects replicated payloads.
pub fn PayloadProtection(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Lazy Effect for transform effect; success, error, and environment types are fixed by
        /// `effects.Effect(outcomes.OwnedSlice(u8), Error(ProviderError), R)`.
        pub const TransformEffect = effects.Effect(outcomes.OwnedSlice(u8), Error(ProviderError), R);
        userdata: ?*anyopaque,
        seal: *const fn (?*anyopaque, *const R, message.Identity, []const u8) TransformEffect,
        open: *const fn (?*anyopaque, *const R, message.Identity, []const u8) TransformEffect,

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context retained through
        ///   every Effect built by these hooks.
        /// - `seal: *const fn (?*anyopaque, *const R, message.Identity, []const u8) TransformEffect`:
        ///   Receives context, environment, identity, and borrowed plaintext;
        ///   returns a lazy Effect producing owned protected bytes.
        /// - `open: *const fn (?*anyopaque, *const R, message.Identity, []const u8) TransformEffect`:
        ///   Receives the same typed inputs with borrowed protected bytes and
        ///   returns a lazy Effect producing owned opened bytes.
        /// Returns (`Self`): A non-owning protection adapter referencing the hooks.
        pub fn fromProvider(
            data: ?*anyopaque,
            seal: *const fn (?*anyopaque, *const R, message.Identity, []const u8) TransformEffect,
            open: *const fn (?*anyopaque, *const R, message.Identity, []const u8) TransformEffect,
        ) Self {
            return .{ .userdata = data, .seal = seal, .open = open };
        }
    };
}

/// Compaction requires a provider-defined replay proof. The core never
/// assumes a projection checkpoint can replace history for every consumer.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `Proof: comptime type`: Compile-time provider-defined replay proof required to authorize compaction.
/// Returns (`type`): provider contract requiring a replay proof before journal truncation.
pub fn Compaction(comptime R: type, comptime ProviderError: type, comptime Proof: type) type {
    return struct {
        const Self = @This();
        /// Lazy Effect for compact effect; success, error, and environment types are fixed by
        /// `effects.Effect(journal.Cursor, Error(ProviderError), R)`.
        pub const CompactEffect = effects.Effect(journal.Cursor, Error(ProviderError), R);
        userdata: ?*anyopaque,
        compact_through: *const fn (?*anyopaque, *const R, journal.Cursor, *const Proof) CompactEffect,
        oldest_available: *const fn (?*anyopaque, *const R) CompactEffect,

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context retained through
        ///   every compaction Effect built by these hooks.
        /// - `compact: *const fn (?*anyopaque, *const R, journal.Cursor, *const Proof) CompactEffect`:
        ///   Receives context, environment, target cursor, and borrowed replay
        ///   proof; returns a lazy Effect yielding the compacted cursor.
        /// - `oldest: *const fn (?*anyopaque, *const R) CompactEffect`:
        ///   Returns a lazy Effect yielding the oldest available cursor.
        /// Returns (`Self`): A non-owning compaction adapter referencing the hooks.
        pub fn fromProvider(
            data: ?*anyopaque,
            compact: *const fn (?*anyopaque, *const R, journal.Cursor, *const Proof) CompactEffect,
            oldest: *const fn (?*anyopaque, *const R) CompactEffect,
        ) Self {
            return .{ .userdata = data, .compact_through = compact, .oldest_available = oldest };
        }
    };
}

/// Conflict facts are borrowed for the duration of a handler Effect. A
/// handler may consult application history or request a snapshot, but the
/// importer never fabricates a universal event merge from timestamps.
pub const Conflict = struct {
    source: []const u8,
    destination: []const u8,
    source_cursor: journal.Cursor,
    observed_cursor: journal.Cursor,
    event_id: ?[]const u8,
    reason: enum { cursor_diverged, missing_event, incompatible_event, concurrent_append },

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Conflict`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when source and destination are nonempty UTF-8 replica IDs.
    pub fn valid(self: Conflict) bool {
        return self.source.len > 0 and self.destination.len > 0 and
            std.unicode.utf8ValidateSlice(self.source) and
            std.unicode.utf8ValidateSlice(self.destination);
    }
};

/// Resolution is an application-defined typed value. The core deliberately
/// does not interpret it as permission to rewrite journal history; any
/// replacement must pass the same authoritative journal contracts as append.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `Resolution: comptime type`: Compile-time typed conflict-resolution result.
/// - `HandlerError: comptime type`: Compile-time typed failure returned by the handler.
/// Returns (`type`): application-selected typed resolution of replica conflicts.
pub fn ConflictHandler(comptime R: type, comptime Resolution: type, comptime HandlerError: type) type {
    return struct {
        const Self = @This();
        /// Lazy Effect for provider effect; success, error, and environment types are fixed by
        /// `effects.Effect(Resolution, HandlerError, R)`.
        pub const ProviderEffect = effects.Effect(Resolution, HandlerError, R);
        /// Lazy Effect for resolve effect; success, error, and environment types are fixed by
        /// `effects.Effect(Resolution, Error(HandlerError), R)`.
        pub const ResolveEffect = effects.Effect(Resolution, Error(HandlerError), R);
        userdata: ?*anyopaque,
        resolve_fn: *const fn (?*anyopaque, *const R, Conflict) ProviderEffect,

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context retained through
        ///   every resolution Effect built by `callback`.
        /// - `callback: *const fn (?*anyopaque, *const R, Conflict) ProviderEffect`:
        ///   Receives context, environment, and conflict facts; returns a lazy
        ///   Effect yielding the typed `Resolution` or `HandlerError`.
        /// Returns (`Self`): A non-owning conflict-handler adapter.
        pub fn fromProvider(data: ?*anyopaque, callback: *const fn (?*anyopaque, *const R, Conflict) ProviderEffect) Self {
            return .{ .userdata = data, .resolve_fn = callback };
        }

        /// Resolve a request through the selected capability.
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `conflict: Conflict`: Conflict facts passed to the resolution handler.
        /// Returns (`ResolveEffect`): application-selected conflict resolution carried through a typed Effect.
        pub fn resolve(self: Self, env: *const R, conflict: Conflict) ResolveEffect {
            if (!conflict.valid()) return ResolveEffect.fail(.malformed);
            var delegated = self.resolve_fn(self.userdata, env, conflict);
            return delegated.mapErrorOwned(env, struct {
                fn provider(problem: HandlerError) Error(HandlerError) {
                    return .{ .provider = problem };
                }
            }.provider);
        }
    };
}
