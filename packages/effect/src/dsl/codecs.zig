//! Codec declarations infer paired values and independent directional services.
//! Pure/effectful intent and writer/shape evidence remain explicit inputs.
const codecs = @import("../schema/codec.zig");
const json_codecs = @import("../schema/json_codec.zig");
const domains = @import("../schema/domain_codec.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;
const Ast = @import("../schema/schema_ast.zig");
const Writer = @import("../schema/json_writer.zig").Writer;
const outcomes = @import("../outcome.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

pub const Kind = enum { pure, contextual, pure_decode, pure_encode };
const Channels = struct { value: type, err: type, environment: type };

/// Shared admission for contextual channels, before any Effect composition.
pub fn validateContextualChannels(comptime A: type, comptime Rep: type, comptime E: type) void {
    inline for (.{ A, Rep, E }) |T| {
        ownership.validate(T);
        if (ownership.containsOwned(T) and !outcomes.ownsPayload(T)) @compileError("contextual codec owning channels require declared payload ownership before Effect composition");
    }
}

fn channels(comptime T: type, comptime kind: Kind) ?Channels {
    if (kind == .contextual) {
        if (!inference.isEffect(T)) return null;
        return .{ .value = T.Success, .err = T.Error, .environment = T.Environment };
    }
    if (@typeInfo(T) != .@"union" or !@hasField(T, "ok") or !@hasField(T, "err")) return null;
    const A = @FieldType(T, "ok");
    const E = @FieldType(T, "err");
    if (T != outcomes.Result(A, E)) return null;
    return .{ .value = A, .err = E, .environment = void };
}
fn pointerValue(comptime callback: anytype, comptime operation: []const u8) ?type {
    const info = inference.info(callback, operation);
    if (info.param_types.len != 1) @compileError(operation ++ " requires one borrowed input pointer");
    const P = info.param_types[0] orelse return null;
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one) @compileError(operation ++ " requires one borrowed input pointer");
    return @typeInfo(P).pointer.child;
}
fn anchor(comptime supplied: anytype, comptime name: []const u8) ?type {
    if (!roles.has(roles.recordType(supplied), name)) return null;
    const selected = roles.get(name, supplied);
    if (@TypeOf(selected) != type) @compileError("codec type anchor must be a type");
    return selected;
}
fn unwrapped(comptime Raw: type, value: *const ownership.Moved(Raw)) *const Raw {
    if (comptime ownership.Moved(Raw) == Raw) return value;
    return &value.value;
}

fn Definition(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "codec", &.{ "kind", "decode", "encode" }, &.{ "value", "representation", "shape", "writer" });
    const kind: Kind = roles.get("kind", supplied);
    const decoding_pure = kind == .pure or kind == .pure_decode;
    const encoding_pure = kind == .pure or kind == .pure_encode;
    const decode_kind: Kind = if (decoding_pure) .pure else .contextual;
    const encode_kind: Kind = if (encoding_pure) .pure else .contextual;
    const decode = roles.get("decode", supplied);
    const encode = roles.get("encode", supplied);
    var A: ?type = anchor(supplied, "value");
    var Rep: ?type = anchor(supplied, "representation");
    if (A == null) A = pointerValue(encode, "codec.encode");
    if (Rep == null) Rep = pointerValue(decode, "codec.decode");
    if (A == null) if (inference.info(decode, "codec.decode").return_type) |T| if (channels(T, decode_kind)) |selected| {
        A = selected.value;
    };
    if (Rep == null) if (inference.info(encode, "codec.encode").return_type) |T| if (channels(T, encode_kind)) |selected| {
        Rep = selected.value;
    };
    if (A == null and Rep == null) @compileError("codec inference needs one typed decode/encode input or an explicit value/representation anchor");
    if (A == null) {
        const Returned = inference.unary(decode, *const Rep.?, "codec.decode");
        A = (channels(Returned, decode_kind) orelse @compileError("codec.decode result category does not match its explicit kind")).value;
    }
    if (Rep == null) {
        const Returned = inference.unary(encode, *const A.?, "codec.encode");
        Rep = (channels(Returned, encode_kind) orelse @compileError("codec.encode result category does not match its explicit kind")).value;
    }
    const DecodeResult = inference.unary(decode, *const Rep.?, "codec.decode");
    const EncodeResult = inference.unary(encode, *const A.?, "codec.encode");
    const D = channels(DecodeResult, decode_kind) orelse @compileError("codec.decode requires exact Result for pure kind or exact Effect for contextual kind");
    const W = channels(EncodeResult, encode_kind) orelse @compileError("codec.encode requires exact Result for pure kind or exact Effect for contextual kind");
    if (D.value != A.? or W.value != Rep.? or D.err != W.err) @compileError("codec directions disagree on value representation or error type; use an explicit typed mapper");
    if (!decoding_pure and !encoding_pure) validateContextualChannels(A.?, Rep.?, D.err);
    inline for (.{ A.?, Rep.?, D.err }, .{ !decoding_pure, !encoding_pure, !(decoding_pure and encoding_pure) }) |T, must_be_declared| {
        ownership.validate(T);
        if (must_be_declared and ownership.containsOwned(T) and !outcomes.ownsPayload(T)) @compileError("contextual codec owning channels require declared payload ownership before Effect composition");
    }
    const RawValue = A.?;
    const RawRepresentation = Rep.?;
    const Value = if (decoding_pure) ownership.Moved(RawValue) else RawValue;
    const Representation = if (encoding_pure) ownership.Moved(RawRepresentation) else RawRepresentation;
    const E = if (decoding_pure and encoding_pure) ownership.Moved(D.err) else D.err;
    const has_writer = roles.has(roles.recordType(supplied), "writer");
    if (has_writer and (kind != .pure or E != Issue)) @compileError("codec.writer requires the existing pure schema codec Issue channel");
    const Selected = if (kind == .pure)
        (if (E == Issue) codecs.PureCodec(Value, Representation) else domains.Pure(Value, Representation, E))
    else if (kind == .contextual)
        (if (E == Issue) codecs.Codec(Value, Representation, D.environment, W.environment) else domains.DomainCodec(Value, Representation, E, D.environment, W.environment))
    else
        domains.Directed(Value, Representation, E, D.environment, W.environment, decoding_pure, encoding_pure);
    return struct {
        pub const Result = Selected;
        fn pureDecode(input: *const Representation) outcomes.Result(Value, E) {
            return switch (decode(unwrapped(RawRepresentation, input))) {
                .ok => |value| .{ .ok = ownership.move(value) },
                .err => |err| .{ .err = ownership.move(err) },
            };
        }
        fn pureEncode(input: *const Value) outcomes.Result(Representation, E) {
            return switch (encode(unwrapped(RawValue, input))) {
                .ok => |value| .{ .ok = ownership.move(value) },
                .err => |err| .{ .err = ownership.move(err) },
            };
        }
        fn contextualDecode(input: *const Representation) DecodeResult {
            return decode(unwrapped(RawRepresentation, input));
        }
        fn contextualEncode(input: *const Value) EncodeResult {
            return encode(unwrapped(RawValue, input));
        }
        fn write(input: *const Value, output: *Writer) outcomes.Exit(void, Issue) {
            const callback = roles.get("writer", supplied);
            if (inference.result(callback, &.{ *const RawValue, *Writer }, "codec.writer") != outcomes.Exit(void, Issue)) @compileError("codec.writer requires exact Exit(void,Issue) with the existing Writer");
            return callback(unwrapped(RawValue, input), output);
        }
        fn make() Selected {
            var result = if (comptime kind == .pure)
                (if (comptime has_writer) Selected.initWriter(pureDecode, pureEncode, write) else Selected.init(pureDecode, pureEncode))
            else
                Selected.init(if (decoding_pure) pureDecode else contextualDecode, if (encoding_pure) pureEncode else contextualEncode);
            if (comptime roles.has(roles.recordType(supplied), "shape")) result.encoded_shape = @as(?*const Ast.Node, roles.get("shape", supplied));
            return result;
        }
    };
}

pub fn from(comptime supplied: anytype) Definition(supplied).Result {
    return Definition(supplied).make();
}

fn Transform(comptime supplied_child: anytype, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "transformCodec", &.{ "forward", "backward" }, &.{});
    const child = codecs.requirePure(supplied_child);
    const C = @TypeOf(child);
    if (@TypeOf(child.decodeDirect(@as(*const C.Representation, undefined))) != outcomes.Result(C.Decoded, Issue) or @TypeOf(child.encodeDirect(@as(*const C.Decoded, undefined))) != outcomes.Result(C.Representation, Issue)) @compileError("transformCodec requires the schema Issue channel in both directions");
    if (ownership.containsOwned(C.Decoded)) @compileError("a pure JSON transformation cannot return a representation borrowing an owned temporary");
    const forward = roles.get("forward", supplied);
    const backward = roles.get("backward", supplied);
    const Returned = inference.unary(forward, C.Decoded, "transformCodec.forward");
    const Forward = channels(Returned, .pure) orelse @compileError("transformCodec.forward requires exact Result(A,Issue)");
    if (Forward.err != Issue) @compileError("transformCodec.forward requires the schema Issue channel");
    const A = Forward.value;
    if (inference.unary(backward, *const A, "transformCodec.backward") != outcomes.Result(C.Decoded, Issue)) @compileError("transformCodec.backward must return the selected child's decoded value in exact Result");
    const Mapping = struct {
        fn ahead(value: C.Decoded) outcomes.Result(A, Issue) {
            return forward(value);
        }
        fn behind(value: *const A) outcomes.Result(C.Decoded, Issue) {
            return backward(value);
        }
    };
    const transformed = json_codecs.transform(child, A, Mapping.ahead, Mapping.behind);
    const Callbacks = struct {
        fn decode(input: *const C.Representation) outcomes.Result(A, Issue) {
            return transformed.decodeDirect(input);
        }
        fn encode(value: *const A) outcomes.Result(C.Representation, Issue) {
            return transformed.encodeDirect(value);
        }
        fn write(value: *const A, output: *Writer) outcomes.Exit(void, Issue) {
            return transformed.writeJson(value, output);
        }
    };
    const normalized = if (transformed.writer != null)
        .{ .kind = Kind.pure, .decode = Callbacks.decode, .encode = Callbacks.encode, .writer = Callbacks.write, .shape = transformed.encoded_shape }
    else
        .{ .kind = Kind.pure, .decode = Callbacks.decode, .encode = Callbacks.encode, .shape = transformed.encoded_shape };
    const Selected = Definition(normalized);
    return struct {
        pub const Result = Selected.Result;
        fn make() Result {
            return Selected.make();
        }
    };
}

/// Infer the mapped value of an existing pure JSON transformation and retain
/// the source's actual representation/writer evidence and ownership rules.
pub fn transform(comptime child: anytype, comptime supplied: anytype) Transform(child, supplied).Result {
    return Transform(child, supplied).make();
}
