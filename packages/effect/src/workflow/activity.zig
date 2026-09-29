//! Durable activity definitions pair payload, result and error codecs with
//! one typed Effect body. Replay records the entire Exit for a stable attempt.
const std = @import("std");
const effects = @import("../effect.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");

pub fn Activity(comptime PayloadCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    inline for (.{ PayloadCodec, SuccessCodec, ErrorCodec }) |C| codec_contract.require(C);
    return struct {
        const Self = @This();
        pub const Payload = PayloadCodec.Decoded;
        pub const Success = SuccessCodec.Decoded;
        pub const DomainError = ErrorCodec.Decoded;

        name: []const u8,
        version: u32,
        payload: PayloadCodec,
        success: SuccessCodec,
        failure: ErrorCodec,

        pub fn define(
            comptime name: []const u8,
            comptime version: u32,
            comptime payload: PayloadCodec,
            comptime success: SuccessCodec,
            comptime failure: ErrorCodec,
        ) Self {
            const definition: Self = .{ .name = name, .version = version, .payload = payload, .success = success, .failure = failure };
            definition.validate();
            return definition;
        }

        pub fn validate(comptime self: Self) void {
            if (comptime self.name.len == 0 or !std.unicode.utf8ValidateSlice(self.name))
                @compileError("activity names must be stable nonempty UTF-8");
            if (comptime self.version == 0) @compileError("activity version zero is reserved for absent metadata");
        }

        pub fn handledBy(
            comptime self: Self,
            comptime R: type,
            comptime body: fn (*const Payload, *const R) effects.Effect(Success, DomainError, R),
        ) Handler(Self, R) {
            self.validate();
            return .{ .definition = self, .body = body };
        }
    };
}

pub fn Handler(comptime Definition: type, comptime R: type) type {
    return struct {
        const Self = @This();
        pub const ActivityDefinition = Definition;
        pub const Environment = R;
        definition: Definition,
        body: *const fn (*const Definition.Payload, *const R) effects.Effect(Definition.Success, Definition.DomainError, R),

        pub fn run(comptime self: Self, comptime select_payload: fn (*const R) *const Definition.Payload) effects.Effect(Definition.Success, Definition.DomainError, R) {
            self.definition.validate();
            const Program = effects.Effect(Definition.Success, Definition.DomainError, R);
            return Program.deferEffect(struct {
                fn start(env: *const R) Program {
                    return self.body(select_payload(env), env);
                }
            }.start);
        }
    };
}

pub fn Registry(comptime handlers: anytype) type {
    const Info = @typeInfo(@TypeOf(handlers));
    if (Info != .@"struct" or !Info.@"struct".is_tuple or handlers.len == 0)
        @compileError("activity registry needs a nonempty tuple of completed handlers");
    inline for (handlers, 0..) |handler, index| {
        const H = @TypeOf(handler);
        if (@typeInfo(H) != .@"struct" or !@hasDecl(H, "ActivityDefinition") or
            !@hasDecl(H, "Environment") or !@hasField(H, "definition") or
            H != Handler(H.ActivityDefinition, H.Environment))
            @compileError("activity registry members must be completed handlers");
        handler.definition.validate();
        if (H.Environment != @TypeOf(handlers[0]).Environment)
            @compileError("activity registry needs one environment");
        inline for (handlers, 0..) |other, earlier| {
            if (earlier < index and std.mem.eql(u8, handler.definition.name, other.definition.name) and
                handler.definition.version == other.definition.version)
                @compileError("duplicate activity identity");
        }
    }
    return struct {
        pub const Environment = @TypeOf(handlers[0]).Environment;

        pub fn resolve(name: []const u8, version: u32) ?usize {
            inline for (handlers, 0..) |handler, index| {
                if (std.mem.eql(u8, name, handler.definition.name) and version == handler.definition.version)
                    return index;
            }
            return null;
        }
    };
}
