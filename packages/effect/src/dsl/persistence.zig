//! Inferred persistence declarations reuse Persistable's wire/version/expiry
//! and provider authority. Runtime keys/configuration live in generated storage.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const storage = @import("../persistence/key_value_store.zig");
const persisted = @import("../persistence/persistable.zig");
const typed = @import("../schema/typed_outcome_codec.zig");
const wire = @import("../schema/outcome_codec.zig");
const deadlines = @import("../deadline.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");
const declarations = @import("declarations.zig");

fn directionEnvironment(comptime callback: anytype, comptime supplied: anytype, comptime anchor: []const u8) type {
    if (roles.has(roles.recordType(supplied), anchor)) {
        const R = roles.get(anchor, supplied);
        if (@TypeOf(R) != type) @compileError("payloadCodec environment anchor must be a type");
        return R;
    }
    return inference.environment(callback, "payloadCodec." ++ anchor);
}
fn Payload(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "payloadCodec", &.{ "encode_success", "encode_failure", "decode_success", "decode_failure" }, &.{ "decoding_environment", "encoding_environment" });
    const DecodeR = directionEnvironment(roles.get("decode_success", supplied), supplied, "decoding_environment");
    const EncodeR = directionEnvironment(roles.get("encode_success", supplied), supplied, "encoding_environment");
    const Success = inference.result(roles.get("decode_success", supplied), &.{ []const u8, *const DecodeR }, "payloadCodec.decode_success");
    const Failure = inference.result(roles.get("decode_failure", supplied), &.{ []const u8, *const DecodeR }, "payloadCodec.decode_failure");
    if (!inference.isExit(Success) or !inference.isExit(Failure)) @compileError("payloadCodec decoders require exact Exit(Value,Issue)");
    const Issue = @import("../schema/schema_issue.zig").Issue;
    if (Success.Error != Issue or Failure.Error != Issue) @compileError("payloadCodec decoders preserve the existing schema Issue channel");
    const A = Success.Success;
    const E = Failure.Success;
    const Encoded = outcomes.Exit(outcomes.OwnedSlice(u8), Issue);
    if (inference.result(roles.get("encode_success", supplied), &.{ *const A, *const EncodeR }, "payloadCodec.encode_success") != Encoded or inference.result(roles.get("encode_failure", supplied), &.{ *const E, *const EncodeR }, "payloadCodec.encode_failure") != Encoded) @compileError("payloadCodec encoders require exact Exit(OwnedSlice(u8),Issue)");
    if (@import("../cluster/entity_resource_plan.zig").containsResource(A) or @import("../cluster/entity_resource_plan.zig").containsResource(E)) @compileError("persisted outcomes cannot contain live activation resources");
    return struct {
        pub const Codec = typed.PayloadCodec(A, E, DecodeR, EncodeR);
        fn encodeSuccess(value: *const A, env: *const EncodeR) Encoded {
            return roles.get("encode_success", supplied)(value, env);
        }
        fn encodeFailure(value: *const E, env: *const EncodeR) Encoded {
            return roles.get("encode_failure", supplied)(value, env);
        }
        fn decodeSuccess(bytes: []const u8, env: *const DecodeR) Success {
            return roles.get("decode_success", supplied)(bytes, env);
        }
        fn decodeFailure(bytes: []const u8, env: *const DecodeR) Failure {
            return roles.get("decode_failure", supplied)(bytes, env);
        }
        fn make() Codec {
            return Codec.init(encodeSuccess, encodeFailure, decodeSuccess, decodeFailure);
        }
    };
}
/// Infer both application channels and directional environments from native
/// byte codec roles; only genuinely unresolved environments need anchors.
pub fn payloadCodec(comptime supplied: anytype) Payload(supplied).Codec {
    return Payload(supplied).make();
}

fn StoreContext(comptime select: anytype) type {
    const Function = inference.info(select, "persistence.store");
    const P = Function.return_type orelse return inference.environment(select, "persistence.store");
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one) @compileError("persistence.store requires a pointer to KeyValueStore");
    const Store = @typeInfo(P).pointer.child;
    if (@typeInfo(Store) != .@"struct" or !@hasDecl(Store, "WriteEffect")) @compileError("persistence.store requires the existing KeyValueStore");
    return Store.WriteEffect.Environment;
}
fn roleType(comptime callback: anytype, comptime C: type, comptime R: type, comptime name: []const u8) type {
    return if (inference.info(callback, name).param_types.len == 1) inference.unary(callback, *const R, name) else inference.result(callback, &.{ *const C, *const R }, name);
}
fn call(comptime callback: anytype, input: anytype, env: anytype) roleType(callback, @typeInfo(@TypeOf(input)).pointer.child, @typeInfo(@TypeOf(env)).pointer.child, "persistence.role") {
    return if (comptime inference.info(callback, "persistence.role").param_types.len == 1) callback(env) else callback(input, env);
}
fn Info(comptime C: type, comptime supplied: anytype, comptime is_scoped: bool) type {
    roles.validate(roles.recordType(supplied), "persistence", if (is_scoped) &.{ "codec", "schema_version", "limits", "store", "key", "expiry", "body" } else &.{ "codec", "schema_version", "limits", "store", "key", "expiry" }, &.{ "decode_environment", "encode_environment" });
    const codec = roles.get("codec", supplied);
    const Codec = @TypeOf(codec);
    if (@typeInfo(Codec) != .@"struct" or !@hasDecl(Codec, "Decoded") or @typeInfo(Codec.Decoded) != .@"struct" or !@hasField(Codec.Decoded, "value")) @compileError("persistence.codec requires the typed outcome codec");
    const Outcome = @FieldType(Codec.Decoded, "value");
    if (!inference.isExit(Outcome)) @compileError("persistence.codec requires an exact decoded Exit(A,E)");
    const A = Outcome.Success;
    const E = Outcome.Error;
    const DecodeR = Codec.DecodingEnvironment;
    const EncodeR = Codec.EncodingEnvironment;
    if (Codec != typed.PayloadCodec(A, E, DecodeR, EncodeR)) @compileError("persistence.codec rejects structural lookalikes");
    if (@import("../cluster/entity_resource_plan.zig").containsResource(A) or @import("../cluster/entity_resource_plan.zig").containsResource(E)) @compileError("persisted outcomes cannot contain live activation resources");
    const R = StoreContext(roles.get("store", supplied));
    const Pointer = roleType(roles.get("store", supplied), C, R, "persistence.store");
    const Store = @typeInfo(Pointer).pointer.child;
    const ProviderError = @FieldType(Store.WriteEffect.Error, "domain");
    if (Store != storage.KeyValueStore(R, ProviderError)) @compileError("persistence.store rejects a foreign provider/environment");
    if (roleType(roles.get("key", supplied), C, R, "persistence.key") != []const u8) @compileError("persistence.key requires encoded key bytes");
    if (roleType(roles.get("expiry", supplied), C, R, "persistence.expiry") != deadlines.Deadline) @compileError("persistence.expiry requires the existing Deadline policy");
    const supplied_limits = roles.get("limits", supplied);
    const limits: wire.Limits = if (@TypeOf(supplied_limits) == wire.Limits) supplied_limits else blk: {
        roles.validate(@TypeOf(supplied_limits), "persistence.limits", &.{ "max_payload_bytes", "max_cleanup_records", "max_total_bytes" }, &.{});
        break :blk .{ .max_payload_bytes = supplied_limits.max_payload_bytes, .max_cleanup_records = supplied_limits.max_cleanup_records, .max_total_bytes = supplied_limits.max_total_bytes };
    };
    const version: u32 = roles.get("schema_version", supplied);
    if (version == 0 or !limits.valid()) @compileError("persistence requires the existing nonzero schema version and valid outcome wire limits");
    return struct {
        pub const Value = A;
        pub const DomainError = E;
        pub const Environment = R;
        pub const Provider = Store;
        pub const Failure = Store.Failure;
        pub const Read = persisted.ReadResult(A, E);
        pub const OutcomeType = Outcome;
        pub const Native = persisted.Persistable(A, E, R, ProviderError, DecodeR, EncodeR);
        fn decodeEnvironment(input: *const C, env: *const R) *const DecodeR {
            if (comptime DecodeR == R) return env;
            if (comptime DecodeR == void) return &{};
            if (comptime !roles.has(roles.recordType(supplied), "decode_environment")) @compileError("persistence needs an explicit directional decode_environment projection");
            return call(roles.get("decode_environment", supplied), input, env);
        }
        fn encodeEnvironment(input: *const C, env: *const R) *const EncodeR {
            if (comptime EncodeR == R) return env;
            if (comptime EncodeR == void) return &{};
            if (comptime !roles.has(roles.recordType(supplied), "encode_environment")) @compileError("persistence needs an explicit directional encode_environment projection");
            return call(roles.get("encode_environment", supplied), input, env);
        }
        fn staticDefinition() Native {
            return Native.define(codec, version, limits, struct {
                fn select(env: *const R) *const Store {
                    return call(roles.get("store", supplied), &{}, env);
                }
            }.select, struct {
                fn select(env: *const R) []const u8 {
                    return call(roles.get("key", supplied), &{}, env);
                }
            }.select, struct {
                fn select(env: *const R) deadlines.Deadline {
                    return call(roles.get("expiry", supplied), &{}, env);
                }
            }.select, struct {
                fn select(env: *const R) *const DecodeR {
                    return decodeEnvironment(&{}, env);
                }
            }.select, struct {
                fn select(env: *const R) *const EncodeR {
                    return encodeEnvironment(&{}, env);
                }
            }.select);
        }
        const Bridge = struct {
            allocator: std.mem.Allocator,
            io: std.Io,
            original: *const R,
            input: *const C,
            provider: BridgeStore,
        };
        const BridgeStore = storage.KeyValueStore(Bridge, ProviderError);
        const BridgeNative = persisted.Persistable(A, E, Bridge, ProviderError, DecodeR, EncodeR);
        const Forward = struct {
            fn original(env: *const Bridge) *const R {
                return env.original;
            }
            fn read(_: ?*anyopaque, env: *const Bridge, key: []const u8) BridgeStore.ReadEffect {
                const provider = call(roles.get("store", supplied), env.input, env.original);
                var source = provider.vtable.read(provider.userdata, env.original, key);
                return source.provideSomeAllocated(env.allocator, Bridge, original);
            }
            fn write(_: ?*anyopaque, env: *const Bridge, key: []const u8, bytes: []const u8, expiry: deadlines.Deadline) BridgeStore.WriteEffect {
                const provider = call(roles.get("store", supplied), env.input, env.original);
                var source = provider.vtable.write(provider.userdata, env.original, key, bytes, expiry);
                return source.provideSomeAllocated(env.allocator, Bridge, original);
            }
            fn remove(_: ?*anyopaque, env: *const Bridge, key: []const u8) BridgeStore.WriteEffect {
                const provider = call(roles.get("store", supplied), env.input, env.original);
                var source = provider.vtable.remove(provider.userdata, env.original, key);
                return source.provideSomeAllocated(env.allocator, Bridge, original);
            }
            fn store(env: *const Bridge) *const BridgeStore {
                return &env.provider;
            }
            fn selectKey(env: *const Bridge) []const u8 {
                return call(roles.get("key", supplied), env.input, env.original);
            }
            fn selectExpiry(env: *const Bridge) deadlines.Deadline {
                return call(roles.get("expiry", supplied), env.input, env.original);
            }
            fn decoding(env: *const Bridge) *const DecodeR {
                return decodeEnvironment(env.input, env.original);
            }
            fn encoding(env: *const Bridge) *const EncodeR {
                return encodeEnvironment(env.input, env.original);
            }
        };
        const native = BridgeNative.define(codec, version, limits, Forward.store, Forward.selectKey, Forward.selectExpiry, Forward.decoding, Forward.encoding);
        pub const View = struct {
            const Self = @This();
            input: C,
            fn bridge(self: *const Self, env: *const R) Bridge {
                return .{ .allocator = env.allocator, .io = env.io, .original = env, .input = &self.input, .provider = BridgeStore.fromProvider(null, &.{ .read = Forward.read, .write = Forward.write, .remove = Forward.remove }) };
            }
            pub fn read(self: *const Self, env: *const R) effects.Effect(Read, Failure, R) {
                const Out = effects.Effect(Read, Failure, R);
                var seed = effects.Effect(void, Failure, R).succeed({});
                return seed.flatMapCapturedOwned(env, Read, self.bridge(env), struct {
                    fn start(context: *const Bridge, _: void, current: *const R) Out {
                        var source = native.read(context);
                        return source.provideOwned(context, R, current);
                    }
                }.start);
            }
            /// Move one outcome into generated storage; native encoding owns
            /// the request bytes through storage admission and completion.
            pub fn write(self: *const Self, env: *const R, value: Outcome) effects.Effect(Store.WriteEffect.Success, Failure, R) {
                const Out = effects.Effect(Store.WriteEffect.Success, Failure, R);
                const Inputs = struct {
                    pub const effect_owned_payload = true;
                    context: Bridge,
                    value: Outcome,
                    pub fn deinit(owned: *@This()) void {
                        owned.value.deinit();
                    }
                };
                var seed = effects.Effect(void, Failure, R).succeed({});
                return seed.flatMapCapturedOwned(env, Out.Success, Inputs{ .context = self.bridge(env), .value = value }, struct {
                    fn start(input: *const Inputs, _: void, current: *const R) Out {
                        var source = native.writeValue(&input.context, &input.value);
                        return source.provideOwned(&input.context, R, current);
                    }
                }.start);
            }
            pub fn remove(self: *const Self, env: *const R) effects.Effect(Store.WriteEffect.Success, Failure, R) {
                const Out = effects.Effect(Store.WriteEffect.Success, Failure, R);
                var seed = effects.Effect(void, Failure, R).succeed({});
                return seed.flatMapCapturedOwned(env, Out.Success, self.bridge(env), struct {
                    fn start(context: *const Bridge, _: void, current: *const R) Out {
                        var source = native.remove();
                        return source.provideOwned(context, R, current);
                    }
                }.start);
            }
        };
    };
}

pub fn from(comptime supplied: anytype) Info(void, supplied, false).Native {
    return Info(void, supplied, false).staticDefinition();
}
fn ScopedResult(comptime C: type, comptime supplied: anytype) type {
    const Selected = Info(C, supplied, true);
    const Out = inference.result(roles.get("body", supplied), &.{ *const Selected.View, *const Selected.Environment }, "persistence.body");
    inference.requireEffect(Out, "persistence.body");
    if (Out.Environment != Selected.Environment) @compileError("persistence.body environment mismatch; use explicit projection");
    return Out;
}
pub fn scoped(env: anytype, capture: anytype, comptime supplied: anytype) ScopedResult(@TypeOf(capture), supplied) {
    const C = @TypeOf(capture);
    const Selected = Info(C, supplied, true);
    const Out = ScopedResult(C, supplied);
    const Capture = ownership.Capture(Selected.View);
    return Out.scopedCaptured(env, Capture{ .value = .{ .input = capture } }, struct {
        fn body(stored: *const Capture, _: *@import("../scope.zig").Scope(Out.Error), current: *const Selected.Environment) Out {
            return roles.get("body", supplied)(&stored.value, current);
        }
    }.body);
}
pub fn declared(env: anytype, input: anytype) ScopedResult(declarations.Info(if (@TypeOf(input) == type) input else @TypeOf(input), "persistence.declared").Inputs, declarations.Info(if (@TypeOf(input) == type) input else @TypeOf(input), "persistence.declared").definition) {
    const Decl = declarations.Info(if (@TypeOf(input) == type) input else @TypeOf(input), "persistence.declared");
    return scoped(env, roles.get("inputs", input), Decl.definition);
}
