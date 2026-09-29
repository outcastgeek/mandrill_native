//! Durable-deferred data boundary. Registration and read are one atomic
//! provider operation; a lost wakeup is repaired by a later observe/reconcile.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const workflow = @import("workflow_types.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");

/// A signal's result schema is part of its compile-time identity. Its
/// execution-specific token cannot be constructed with an unrelated result
/// type or silently rebound to a different schema version.
///
/// Parameters:
/// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
/// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or replayed data.
/// - `result_schema: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded and encoded
///   value types.
/// Returns (`type`): versioned durable signal definition whose result schema is part of its identity.
pub fn Signal(comptime name: []const u8, comptime version: u32, comptime result_schema: anytype) type {
    const Codec = @TypeOf(result_schema);
    comptime {
        codec_contract.require(Codec);
        const byte_representation = @hasDecl(Codec, "Representation") and
            (Codec.Representation == []const u8 or Codec.Representation == []u8 or
                Codec.Representation == outcomes.OwnedSlice(u8));
        if (!byte_representation and !@hasDecl(Codec, "encodeBytesDirect") and
            !@hasDecl(Codec, "encodeBytes"))
            @compileError("durable signal result schema must encode to bytes");
        if (name.len == 0 or !std.unicode.utf8ValidateSlice(name) or version == 0)
            @compileError("durable signal requires a stable nonempty UTF-8 name and nonzero schema version");
    }
    return struct {
        /// Compile-time marker that this signal definition is durable.
        pub const durable_signal = true;
        /// Typed result channel described by `Codec.Decoded`.
        pub const Result = Codec.Decoded;
        /// ResultSchema contract selected by this definition; its concrete type is `Codec`.
        pub const ResultSchema = Codec;
        /// Schema selected for this definition’s state or result payload.
        pub const schema = result_schema;
        /// Stable durable-signal identity.
        pub const signal_name = name;
        /// Version of the durable-signal schema.
        pub const schema_version = version;

        /// Construct or access the value at the selected position.
        /// Parameters:
        /// - `execution: workflow.ExecutionId`: Workflow execution identity.
        /// Returns (`Token`): A typed claim or cancellation token borrowed from its owner.
        pub fn at(execution: workflow.ExecutionId) Token {
            return .{ .identity = .{ .execution = execution, .name = name, .version = version } };
        }

        /// Stable durable-signal token used to identify a completion.
        pub const Token = struct {
            /// Result value type decoded by this durable signal’s codec.
            pub const Value = Codec.Decoded;
            /// Schema contract selected by this definition; its concrete type is `Codec`.
            pub const Schema = Codec;
            identity: Identity,

            /// Check whether this value satisfies its structural invariants.
            /// Parameters:
            /// - `self: @This()`: Current value passed by value; ownership follows this operation’s
            ///   contract.
            /// Returns (`bool`): `true` when the contained durable-signal identity validates.
            pub fn valid(self: @This()) bool {
                return self.identity.valid();
            }
        };
    };
}

/// Versioned durable-signal identity and associated result schema.
pub const Identity = struct {
    execution: workflow.ExecutionId,
    name: []const u8,
    version: u32,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Identity`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when execution, nonempty UTF-8 signal name, and positive version validate.
    pub fn valid(self: Identity) bool {
        return self.execution.validate() == .ok and self.name.len > 0 and
            std.unicode.utf8ValidateSlice(self.name) and self.version > 0;
    }
};

/// Stored signal registration awaiting an outcome.
pub const Registration = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    execution_key: workflow.Bytes,
    name: workflow.Bytes,
    version: u32,
    generation: u64,

    /// Borrow a read-only view of the registration.
    /// Parameters:
    /// - `self: *const Registration`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`RegistrationView`): Borrowed registration state suitable for observation.
    pub fn view(self: *const Registration) RegistrationView {
        return .{ .identity = .{ .execution = .{ .encoded = self.execution_key.items }, .name = self.name.items, .version = self.version }, .generation = self.generation };
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Registration`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Registration) void {
        self.execution_key.deinit();
        self.name.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const Registration`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!Registration`): A `Registration` on success; `OutOfMemory` if required allocation fails.
    pub fn clone(self: *const Registration) error{OutOfMemory}!Registration {
        var execution_key = try self.execution_key.clone();
        errdefer execution_key.deinit();
        return .{ .execution_key = execution_key, .name = try self.name.clone(), .version = self.version, .generation = self.generation };
    }
};

/// Read-only view of a stored durable-signal registration.
pub const RegistrationView = struct { identity: Identity, generation: u64 };

/// Signal observation: pending or completed with the typed stored outcome.
pub const Observed = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    ready: workflow.Bytes,
    waiting: Registration,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .ready) self.ready.deinit();
        if (self.* == .waiting) self.waiting.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .ready => |value| .{ .ready = try value.clone() },
            .waiting => |value| .{ .waiting = try value.clone() },
        };
    }
};

/// Result of attempting to complete a durable signal once.
pub const Completion = union(enum) {
    recorded: struct { wake_registered_execution: bool },
    already_recorded,
    conflicting_result,
    possibly_recorded,
};

/// Typed durable-signal registration or completion failure.
pub const Error = union(enum) {
    invalid_identity,
    unavailable,
    generation_exhausted,
    malformed_outcome,
};

/// Storage or provider durability promised by this capability. `recoverable`
/// permits compositions that must survive process restart; `volatile` does not.
pub const Guarantee = enum { @"volatile", recoverable };

/// Specialize the workflow provider interface and its typed Effect channels.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// Returns (`type`): durable-signal provider interface for recording and resolving signals.
pub fn Store(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
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
        /// Lazy Effect for observe effect; success, error, and environment types are fixed by
        /// `effects.Effect(Observed, Failure, R)`.
        pub const ObserveEffect = effects.Effect(Observed, Failure, R);
        /// Lazy Effect for complete effect; success, error, and environment types are fixed by
        /// `effects.Effect(Completion, Failure, R)`.
        pub const CompleteEffect = effects.Effect(Completion, Failure, R);
        /// Lazy Effect for release effect; success, error, and environment types are fixed by
        /// `effects.Effect(void, Failure, R)`.
        pub const ReleaseEffect = effects.Effect(void, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Observe through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Identity`: stable typed identity for the selected operation.
            /// Returns (`ObserveEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            observe: *const fn (?*anyopaque, *const R, Identity) ObserveEffect,
            /// Record completion.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Identity`: stable typed identity for the selected operation.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`CompleteEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            complete: *const fn (?*anyopaque, *const R, Identity, []const u8) CompleteEffect,
            /// Release wait through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: RegistrationView`: typed domain value consumed by this hook.
            /// Returns (`ReleaseEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            release_wait: *const fn (?*anyopaque, *const R, RegistrationView) ReleaseEffect,
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

        fn invalid(comptime A: type) effects.Effect(A, Failure, R) {
            return effects.Effect(A, Failure, R).fromOutcome(struct {
                fn result(_: *const R) outcomes.Exit(A, Failure) {
                    return .{ .fail = .{ .operation = .invalid_identity } };
                }
            }.result);
        }

        /// Read the current observable state.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_identity: comptime fn (*const R) Identity`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`ObserveEffect`): pending or completed durable-signal state under its stable identity.
        pub fn observe(
            comptime select_store: fn (*const R) *const Self,
            comptime select_identity: fn (*const R) Identity,
        ) ObserveEffect {
            return ObserveEffect.deferEffect(struct {
                fn start(env: *const R) ObserveEffect {
                    const identity = select_identity(env);
                    if (!identity.valid())
                        return invalid(Observed);
                    const store = select_store(env);
                    return store.vtable.observe(store.userdata, env, identity);
                }
            }.start);
        }

        /// Record successful completion of the operation.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_identity: comptime fn (*const R) Identity`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_encoded_result: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment;
        ///   its result must remain valid through use.
        /// Returns (`CompleteEffect`): one-shot signal completion result, retaining any prior committed outcome.
        pub fn complete(
            comptime select_store: fn (*const R) *const Self,
            comptime select_identity: fn (*const R) Identity,
            comptime select_encoded_result: fn (*const R) []const u8,
        ) CompleteEffect {
            return CompleteEffect.deferEffect(struct {
                fn start(env: *const R) CompleteEffect {
                    const identity = select_identity(env);
                    if (!identity.valid())
                        return invalid(Completion);
                    const store = select_store(env);
                    return store.vtable.complete(store.userdata, env, identity, select_encoded_result(env));
                }
            }.start);
        }

        /// Release a durable-deferred waiter registration.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_registration: comptime fn (*const R) RegistrationView`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`ReleaseEffect`): typed acknowledgement after removing the selected durable waiter.
        pub fn releaseWait(
            comptime select_store: fn (*const R) *const Self,
            comptime select_registration: fn (*const R) RegistrationView,
        ) ReleaseEffect {
            return ReleaseEffect.deferEffect(struct {
                fn start(env: *const R) ReleaseEffect {
                    const registration = select_registration(env);
                    if (!registration.identity.valid() or registration.generation == 0)
                        return invalid(void);
                    const store = select_store(env);
                    return store.vtable.release_wait(store.userdata, env, registration);
                }
            }.start);
        }
    };
}
