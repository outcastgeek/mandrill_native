//! Runtime codec configuration is retained by descriptions and returned values.
//! Directional work remains on the existing Effect trampoline and services.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const rt = @import("../internal/runtime.zig");
const storage = @import("../internal/capture_owner.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");
const declarations = @import("declarations.zig");
const codecs = @import("codecs.zig");
const json_writer = @import("../schema/json_writer.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

fn pointed(comptime P: type, comptime operation: []const u8) type {
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one or !@typeInfo(P).pointer.attrs.@"const") @compileError(operation ++ " requires a borrowed const pointer");
    return @typeInfo(P).pointer.child;
}

fn anchor(comptime supplied: anytype, comptime name: []const u8) ?type {
    if (!roles.has(roles.recordType(supplied), name)) return null;
    const T = roles.get(name, supplied);
    if (@TypeOf(T) != type) @compileError("makeCodec type anchor must be a type");
    return T;
}
fn pureChannels(comptime T: type) ?type {
    if (@typeInfo(T) != .@"union" or !@hasField(T, "ok") or !@hasField(T, "err")) return null;
    const A = @FieldType(T, "ok");
    const E = @FieldType(T, "err");
    if (T != outcomes.Result(A, E)) return null;
    return struct {
        const Success = A;
        const Error = E;
    };
}
fn resultValue(comptime callback: anytype, comptime pure: bool) ?type {
    const Returned = inference.info(callback, "makeCodec").return_type orelse return null;
    if (pure) return if (pureChannels(Returned)) |Ch| Ch.Success else null;
    return if (inference.isEffect(Returned)) Returned.Success else null;
}
fn evidence(comptime first: ?type, comptime second: ?type) ?type {
    return if (first) |T| T else second;
}
fn argumentValue(comptime callback: anytype, comptime pure: bool) ?type {
    const f = inference.info(callback, "makeCodec");
    if (pure and f.param_types.len != 2) @compileError("makeCodec pure callback requires configuration and input arguments");
    if (!pure and f.param_types.len != 3) @compileError("makeCodec callback requires configuration, input, and environment arguments");
    return if (f.param_types[1]) |P| pointed(P, "makeCodec input") else null;
}
fn environmentValue(comptime callback: anytype) ?type {
    const f = inference.info(callback, "makeCodec");
    if (f.param_types.len != 3) @compileError("makeCodec callback requires configuration, input, and environment arguments");
    if (f.param_types[2] != null) return inference.environment(callback, "makeCodec");
    if (f.return_type) |Returned| if (inference.isEffect(Returned)) return Returned.Environment;
    return null;
}
fn Direction(comptime C: type, comptime callback: anytype, comptime input_anchor: ?type, comptime env_anchor: ?type, comptime operation: []const u8, comptime pure: bool) type {
    const f = inference.info(callback, operation);
    if (pure and f.param_types.len != 2) @compileError("makeCodec pure callback requires configuration and input arguments");
    if (!pure and f.param_types.len != 3) @compileError(operation ++ " requires configuration, input, and environment arguments");
    const Input = if (input_anchor) |T| T else if (f.param_types[1]) |P| pointed(P, operation) else @compileError(operation ++ " needs input evidence from a direction or explicit type anchor");
    const R = if (pure) void else if (env_anchor) |T| T else if (f.param_types[2] != null) inference.environment(callback, operation) else if (f.return_type) |Returned| (if (inference.isEffect(Returned)) Returned.Environment else @compileError(operation ++ " needs directional environment evidence")) else @compileError(operation ++ " needs directional environment evidence");
    if (pure and env_anchor != null and env_anchor.? != void) @compileError("makeCodec pure direction cannot require an execution environment");
    const Returned = if (pure) inference.result(callback, &.{ *const C, *const Input }, operation) else inference.result(callback, &.{ *const C, *const Input, *const R }, operation);
    const P = if (pure) blk: {
        const Ch = pureChannels(Returned) orelse @compileError("makeCodec pure callback requires exact Result");
        break :blk effects.Effect(Ch.Success, Ch.Error, void);
    } else blk: {
        inference.requireEffect(Returned, operation);
        if (Returned.Environment != R) @compileError(operation ++ " result environment conflicts with its environment parameter");
        break :blk Returned;
    };
    return struct {
        const Program = P;
        const Argument = Input;
    };
}

fn Definition(comptime C: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "makeCodec", &.{ "kind", "decode", "encode" }, &.{ "shape", "writer", "value", "representation", "decode_env", "encode_env" });
    const kind: codecs.Kind = roles.get("kind", supplied);
    const decoding_pure = kind == .pure or kind == .pure_decode;
    const encoding_pure = kind == .pure or kind == .pure_encode;
    const decode_callback = roles.get("decode", supplied);
    const encode_callback = roles.get("encode", supplied);
    const Pair = blk: {
        var value_type: ?type = evidence(anchor(supplied, "value"), evidence(resultValue(decode_callback, decoding_pure), argumentValue(encode_callback, encoding_pure)));
        var representation_type: ?type = evidence(anchor(supplied, "representation"), evidence(resultValue(encode_callback, encoding_pure), argumentValue(decode_callback, decoding_pure)));
        const decode_env: ?type = evidence(anchor(supplied, "decode_env"), if (decoding_pure) void else environmentValue(decode_callback));
        const encode_env: ?type = evidence(anchor(supplied, "encode_env"), if (encoding_pure) void else environmentValue(encode_callback));
        var decoded: ?type = null;
        var encoded: ?type = null;
        if (representation_type != null and decode_env != null) {
            decoded = Direction(C, decode_callback, representation_type, decode_env, "makeCodec.decode", decoding_pure);
            if (value_type == null) value_type = decoded.?.Program.Success;
        }
        if (value_type != null and encode_env != null) {
            encoded = Direction(C, encode_callback, value_type, encode_env, "makeCodec.encode", encoding_pure);
            if (representation_type == null) representation_type = encoded.?.Program.Success;
        }
        if (decoded == null and representation_type != null and decode_env != null)
            decoded = Direction(C, decode_callback, representation_type, decode_env, "makeCodec.decode", decoding_pure);
        if (decoded == null or encoded == null) @compileError("makeCodec inference remains ambiguous after both directions; supply only the unresolved value, representation, or directional environment anchor");
        const Decode = decoded.?;
        const Encode = encoded.?;
        break :blk struct {
            const decoding = Decode;
            const encoding = Encode;
        };
    };
    const D = Pair.decoding;
    const W = Pair.encoding;
    const A = D.Program.Success;
    const Rep = W.Program.Success;
    const E = D.Program.Error;
    const has_writer = roles.has(roles.recordType(supplied), "writer");
    if (has_writer) {
        if (kind != .pure or E != Issue) @compileError("makeCodec.writer requires an explicitly pure codec with the native Issue channel");
        if (inference.result(roles.get("writer", supplied), &.{ *const C, *const A, *json_writer.Writer }, "makeCodec.writer") != outcomes.Exit(void, Issue)) @compileError("makeCodec.writer requires exact Exit(void,Issue) and the native Writer");
    }
    if (D.Argument != Rep or W.Argument != A or W.Program.Error != E) @compileError("codec directions disagree on value representation or error type; use an explicit typed mapper");
    inline for (.{ A, Rep, E }, .{ !decoding_pure, !encoding_pure, !(decoding_pure and encoding_pure) }) |T, effect_channel| {
        ownership.validate(T);
        if (effect_channel and ownership.containsOwned(T) and !outcomes.ownsPayload(T)) @compileError("contextual codec owning channels require declared payload ownership before Effect composition");
    }
    ownership.validate(C);
    const Storage = storage.Owner(C);
    return struct {
        pub const Result = struct {
            const Self = @This();
            pub const effect_owned_payload = true;
            pub const runtime_codec_shape = struct {
                pub const Configuration = C;
                pub const definition = supplied;
                pub const Payload: ?type = null;
            };
            pub const Decoded = Storage.Held(A);
            pub const Representation = Storage.Held(Rep);
            pub const Error = Storage.Held(E);
            pub const RawDecoded = A;
            pub const RawRepresentation = Rep;
            pub const DecodingEnvironment = D.Program.Environment;
            pub const EncodingEnvironment = W.Program.Environment;
            pub const pure_decoding = decoding_pure;
            pub const pure_encoding = encoding_pure;
            pub const has_explicit_writer = has_writer;
            encoded_shape: ?*const @import("../schema/schema_ast.zig").Node = if (roles.has(roles.recordType(supplied), "shape")) roles.get("shape", supplied) else null,
            capture: Storage,

            fn retained(self: Self) Self {
                var result = self;
                result.capture = self.capture.retain();
                return result;
            }
            pub fn clone(self: *const Self) error{OutOfMemory}!Self {
                return self.retained();
            }
            pub fn deinit(self: *Self) void {
                self.capture.deinit();
                self.* = undefined;
            }
            /// Move a representation into the codec's generated owner.
            pub fn representation(self: Self, value_: Rep) Representation {
                return .{ .value = value_, .owner = self.capture.retain() };
            }
            /// Move an application value into the codec's generated owner.
            pub fn value(self: Self, value_: A) Decoded {
                return .{ .value = value_, .owner = self.capture.retain() };
            }
            pub fn decode(self: Self, input: *const Representation) effects.Effect(Decoded, Error, DecodingEnvironment) {
                return Operation(true).make(self, &input.value);
            }
            pub fn encode(self: Self, input: *const Decoded) effects.Effect(Representation, Error, EncodingEnvironment) {
                return Operation(false).make(self, &input.value);
            }
            pub const decodeDirect = if (decoding_pure) struct {
                fn run(self: Self, input: *const Representation) outcomes.Result(Decoded, Error) {
                    return switch (decode_callback(self.capture.borrow(), &input.value)) {
                        .ok => |value_| .{ .ok = .{ .value = value_, .owner = self.capture.retain() } },
                        .err => |err| .{ .err = .{ .value = err, .owner = self.capture.retain() } },
                    };
                }
            }.run else struct {
                fn run(_: Self, _: *const Representation) @compileError("makeCodec.decodeDirect requires explicit pure decode evidence") {
                    unreachable;
                }
            }.run;
            pub const encodeDirect = if (encoding_pure) struct {
                fn run(self: Self, input: *const Decoded) outcomes.Result(Representation, Error) {
                    return switch (encode_callback(self.capture.borrow(), &input.value)) {
                        .ok => |value_| .{ .ok = .{ .value = value_, .owner = self.capture.retain() } },
                        .err => |err| .{ .err = .{ .value = err, .owner = self.capture.retain() } },
                    };
                }
            }.run else struct {
                fn run(_: Self, _: *const Decoded) @compileError("makeCodec.encodeDirect requires explicit pure encode evidence") {
                    unreachable;
                }
            }.run;
            /// Encode once through explicit pure evidence, dispose the owned
            /// representation, and retain configuration in a returned error.
            pub fn validateDirect(self: Self, input: *const Decoded) outcomes.Result(void, Error) {
                if (comptime !encoding_pure) @compileError("makeCodec.validateDirect requires explicit pure encode evidence");
                var encoded = self.encodeDirect(input);
                return switch (encoded) {
                    .ok => blk: {
                        encoded.ok.deinit();
                        break :blk .{ .ok = {} };
                    },
                    .err => |problem| .{ .err = problem },
                };
            }
            /// Execute encoding once and release its generated representation.
            pub fn validate(self: Self, input: *const Decoded) effects.Effect(void, Error, EncodingEnvironment) {
                var described = self.encode(input);
                return described.mapAllocated(self.capture.allocator(), struct {
                    fn discard(encoded: Representation) void {
                        var owned = encoded;
                        owned.deinit();
                    }
                }.discard);
            }
            pub fn decodeIn(self: Self, input: *const Representation, comptime project: anytype) effects.Effect(Decoded, Error, inference.environment(project, "codec.decodeIn")) {
                const R = inference.environment(project, "codec.decodeIn");
                var described = self.decode(input);
                return described.provideSomeAllocated(self.capture.allocator(), R, project);
            }
            pub fn encodeIn(self: Self, input: *const Decoded, comptime project: anytype) effects.Effect(Representation, Error, inference.environment(project, "codec.encodeIn")) {
                const R = inference.environment(project, "codec.encodeIn");
                var described = self.encode(input);
                return described.provideSomeAllocated(self.capture.allocator(), R, project);
            }
            /// Borrow input/output for this direct call; retain configuration in
            /// every returned writer failure and its cleanup metadata.
            pub fn writeJson(self: Self, input: *const Decoded, writer: *json_writer.Writer) outcomes.Exit(void, Error) {
                if (comptime !has_writer) @compileError("codec has no direct JSON writer evidence");
                const raw = roles.get("writer", supplied)(self.capture.borrow(), &input.value, writer);
                return outcomes.mapErrorCaptured(Error, raw, writer.output.allocator, self, struct {
                    fn wrap(owner: Self, problem: Issue) Error {
                        return .{ .value = problem, .owner = owner.capture.retain() };
                    }
                }.wrap);
            }
            /// Infer caller R from its explicit limits selector and use the
            /// native bounded JSON writer. Runtime configuration is retained
            /// automatically; the input remains borrowed until this Effect joins.
            pub fn writeWith(self: Self, input: *const Decoded, comptime select_limits: anytype) WriterOperation(select_limits).Out {
                return WriterOperation(select_limits).make(self, input);
            }
            fn WriterOperation(comptime select_limits: anytype) type {
                if (!has_writer) @compileError("codec has no direct JSON writer evidence");
                const R = inference.environment(select_limits, "makeCodec.writeWith");
                if (inference.unary(select_limits, *const R, "makeCodec.writeWith") != json_writer.Limits) @compileError("makeCodec.writeWith selector requires native JSON writer Limits");
                if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator) @compileError("makeCodec.writeWith requires caller environment allocator");
                const Bytes = outcomes.OwnedSlice(u8);
                const P = effects.Effect(Bytes, Issue, R);
                return struct {
                    const Out = effects.Effect(Bytes, Error, R);
                    const Capture = struct {
                        pub const effect_owned_payload = true;
                        owner: Self,
                        input: *const Decoded,
                        pub fn deinit(self: *@This()) void {
                            self.owner.deinit();
                        }
                    };
                    // This private adapter receives explicit writer evidence
                    // only after the declaration and native signature validate.
                    const Adapter = struct {
                        pub const supports_direct_writer = true;
                        pub const Decoded = Capture;
                        pub fn writeJson(_: *const R, stored: *const Capture, writer: *json_writer.Writer) outcomes.Exit(void, Issue) {
                            return roles.get("writer", supplied)(stored.owner.capture.borrow(), &stored.input.value, writer);
                        }
                    };
                    const Frame = struct {
                        allocator: std.mem.Allocator,
                        parent: *Out.RuntimeAdapter.Job,
                        capture: *const Capture,
                        child: P.RuntimeAdapter.Job = undefined,
                        result: outcomes.Exit(Bytes, Issue) = undefined,
                        fn wrap(owner: Self, problem: Issue) Error {
                            return .{ .value = problem, .owner = owner.capture.retain() };
                        }
                        fn complete(_: *rt.State, data: ?*anyopaque) rt.Step {
                            const frame: *@This() = @ptrCast(@alignCast(data.?));
                            const result = outcomes.mapErrorCaptured(Error, frame.result, frame.allocator, frame.capture.owner, wrap);
                            const parent = frame.parent;
                            frame.allocator.destroy(frame);
                            return parent.adapterFinish(result);
                        }
                    };
                    fn start(state: *rt.State, parent: *Out.RuntimeAdapter.Job) rt.Step {
                        const capture: *const Capture = @ptrCast(@alignCast(parent.program.value.borrowed));
                        const allocator = state.services.allocatorValue() orelse return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
                        const frame = allocator.create(Frame) catch return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
                        frame.* = .{ .allocator = allocator, .parent = parent, .capture = capture };
                        const program = json_writer.requireWriter(Adapter).writeWith(R, capture, select_limits);
                        frame.child = .{ .program = program, .owned = program.ownership, .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.result, .next_step = .{ .data = frame, .advance = Frame.complete } } };
                        return frame.child.adapterStep();
                    }
                    fn make(owner: Self, input: *const Decoded) Out {
                        var seed = effects.Effect(void, Error, R).succeed({});
                        return seed.flatMapCapturedAllocated(owner.capture.allocator(), Bytes, Capture{ .owner = owner.retained(), .input = input }, struct {
                            fn next(stored: *const Capture, _: void, _: *const R) Out {
                                return .{ .value = .{ .borrowed = stored }, .start = start };
                            }
                        }.next);
                    }
                };
            }
            fn Operation(comptime decoding: bool) type {
                const P = if (decoding) D.Program else W.Program;
                const Input = if (decoding) Rep else A;
                const Output = if (decoding) Decoded else Representation;
                const R = P.Environment;
                const Out = effects.Effect(Output, Error, R);
                return struct {
                    const Capture = struct {
                        pub const effect_owned_payload = true;
                        owner: Self,
                        input: *const Input,
                        pub fn deinit(self: *@This()) void {
                            self.owner.deinit();
                        }
                    };
                    const Frame = struct {
                        allocator: std.mem.Allocator,
                        parent: *Out.RuntimeAdapter.Job,
                        capture: *const Capture,
                        child: P.RuntimeAdapter.Job = undefined,
                        result: outcomes.Exit(P.Success, E) = undefined,
                        fn mapError(owner: Self, err: E) Error {
                            return .{ .value = err, .owner = owner.capture.retain() };
                        }
                        fn complete(_: *rt.State, data: ?*anyopaque) rt.Step {
                            const frame: *@This() = @ptrCast(@alignCast(data.?));
                            const mapped = outcomes.mapErrorCaptured(Error, frame.result, frame.allocator, frame.capture.owner, mapError);
                            const result: outcomes.Exit(Output, Error) = switch (mapped) {
                                .success => |raw| .{ .success = .{ .value = raw, .owner = frame.capture.owner.capture.retain() } },
                                else => outcomes.castFailure(Output, mapped),
                            };
                            const parent = frame.parent;
                            frame.allocator.destroy(frame);
                            return parent.adapterFinish(result);
                        }
                    };
                    fn start(state: *rt.State, parent: *Out.RuntimeAdapter.Job) rt.Step {
                        const capture: *const Capture = @ptrCast(@alignCast(parent.program.value.borrowed));
                        const callback = if (decoding) decode_callback else encode_callback;
                        if (comptime (if (decoding) decoding_pure else encoding_pure)) {
                            const result: outcomes.Exit(Output, Error) = switch (callback(capture.owner.capture.borrow(), capture.input)) {
                                .ok => |value_| .{ .success = .{ .value = value_, .owner = capture.owner.capture.retain() } },
                                .err => |err| .{ .fail = .{ .value = err, .owner = capture.owner.capture.retain() } },
                            };
                            return parent.adapterFinish(result);
                        } else {
                            const allocator = state.services.allocatorValue() orelse return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
                            const frame = allocator.create(Frame) catch return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
                            frame.* = .{ .allocator = allocator, .parent = parent, .capture = capture };
                            const program = callback(capture.owner.capture.borrow(), capture.input, parent.env);
                            frame.child = .{ .program = program, .owned = program.ownership, .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.result, .next_step = .{ .data = frame, .advance = Frame.complete } } };
                            return frame.child.adapterStep();
                        }
                    }
                    fn make(owner: Self, input: *const Input) Out {
                        var seed = effects.Effect(void, Error, R).succeed({});
                        return seed.flatMapCapturedAllocated(owner.capture.allocator(), Output, Capture{ .owner = owner.retained(), .input = input }, struct {
                            fn next(capture: *const Capture, _: void, _: *const R) Out {
                                return .{ .value = .{ .borrowed = capture }, .start = start };
                            }
                        }.next);
                    }
                };
            }
        };
        fn make(allocator: std.mem.Allocator, configuration: C) error{OutOfMemory}!Result {
            return .{ .capture = try Storage.create(allocator, configuration) };
        }
    };
}

fn Info(comptime Input: type) type {
    return struct {
        const Decl = declarations.Info(Input, "makeCodec");
        const Selected = Definition(Decl.Inputs, Decl.definition);
        const Result = Selected.Result;
    };
}

/// Inspect only this exact generated owner or payload wrapper. The public
/// metadata alone never grants a different record this ownership contract.
pub fn concurrencyShape(comptime T: type) ?type {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "runtime_codec_shape")) return null;
    const Shape = T.runtime_codec_shape;
    const Owner = Definition(Shape.Configuration, Shape.definition).Result;
    if (Shape.Payload != null or T != Owner) @compileError("concurrent codec capture rejects forged runtime ownership evidence");
    return Shape;
}

/// Capture one definition/configuration record. Construction remains lazy
/// with respect to codec work; each run returns a retained configuration owner.
pub fn make(env: anytype, input: anytype) effects.Effect(Info(if (@TypeOf(input) == type) input else @TypeOf(input)).Result, Info(if (@TypeOf(input) == type) input else @TypeOf(input)).Result.Error, @typeInfo(@TypeOf(env)).pointer.child) {
    const Carrier = if (@TypeOf(input) == type) input else @TypeOf(input);
    const Selected = Info(Carrier).Selected;
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const Out = effects.Effect(Selected.Result, Selected.Result.Error, R);
    const allocator = rt.services(R, env).allocatorValue() orelse {
        var abandoned = roles.get("inputs", input);
        ownership.discard(&abandoned);
        return Out.die(.{ .kind = .invalid_configuration, .message = "makeCodec requires construction environment allocator" });
    };
    const owner = Selected.make(allocator, roles.get("inputs", input)) catch return Out.die(outcomes.Defect.out_of_memory);
    return Out.fromOwnedAllocated(allocator, owner, struct {
        fn run(stored: *const Selected.Result, _: *const R) outcomes.Exit(Selected.Result, Selected.Result.Error) {
            return .{ .success = stored.retained() };
        }
    }.run);
}
