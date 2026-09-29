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
        pub const durable_signal = true;
        pub const Result = Codec.Decoded;
        pub const ResultSchema = Codec;
        pub const schema = result_schema;
        pub const signal_name = name;
        pub const schema_version = version;

        pub fn at(execution: workflow.ExecutionId) Token {
            return .{ .identity = .{ .execution = execution, .name = name, .version = version } };
        }

        pub const Token = struct {
            pub const Value = Codec.Decoded;
            pub const Schema = Codec;
            identity: Identity,

            pub fn valid(self: @This()) bool {
                return self.identity.valid();
            }
        };
    };
}

pub const Identity = struct {
    execution: workflow.ExecutionId,
    name: []const u8,
    version: u32,

    pub fn valid(self: Identity) bool {
        return self.execution.validate() == .ok and self.name.len > 0 and
            std.unicode.utf8ValidateSlice(self.name) and self.version > 0;
    }
};

pub const Registration = struct {
    pub const effect_owned_payload = true;
    execution_key: workflow.Bytes,
    name: workflow.Bytes,
    version: u32,
    generation: u64,

    pub fn view(self: *const Registration) RegistrationView {
        return .{ .identity = .{ .execution = .{ .encoded = self.execution_key.items }, .name = self.name.items, .version = self.version }, .generation = self.generation };
    }

    pub fn deinit(self: *Registration) void {
        self.execution_key.deinit();
        self.name.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const Registration) error{OutOfMemory}!Registration {
        var execution_key = try self.execution_key.clone();
        errdefer execution_key.deinit();
        return .{ .execution_key = execution_key, .name = try self.name.clone(), .version = self.version, .generation = self.generation };
    }
};

pub const RegistrationView = struct { identity: Identity, generation: u64 };

pub const Observed = union(enum) {
    pub const effect_owned_payload = true;
    ready: workflow.Bytes,
    waiting: Registration,

    pub fn deinit(self: *@This()) void {
        if (self.* == .ready) self.ready.deinit();
        if (self.* == .waiting) self.waiting.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .ready => |value| .{ .ready = try value.clone() },
            .waiting => |value| .{ .waiting = try value.clone() },
        };
    }
};

pub const Completion = union(enum) {
    recorded: struct { wake_registered_execution: bool },
    already_recorded,
    conflicting_result,
    possibly_recorded,
};

pub const Error = union(enum) {
    invalid_identity,
    unavailable,
    generation_exhausted,
    malformed_outcome,
};

pub const Guarantee = enum { @"volatile", recoverable };

pub fn Store(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        pub const storage_guarantee = guarantee;
        pub const Failure = union(enum) {
            const Own = @This();
            pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
            operation: Error,
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
        pub const ObserveEffect = effects.Effect(Observed, Failure, R);
        pub const CompleteEffect = effects.Effect(Completion, Failure, R);
        pub const ReleaseEffect = effects.Effect(void, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            observe: *const fn (?*anyopaque, *const R, Identity) ObserveEffect,
            complete: *const fn (?*anyopaque, *const R, Identity, []const u8) CompleteEffect,
            release_wait: *const fn (?*anyopaque, *const R, RegistrationView) ReleaseEffect,
        };

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
