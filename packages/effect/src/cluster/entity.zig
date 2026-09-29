//! Typed logical entity definitions. An entity group names a handler family;
//! the physical owner and message store remain separate capabilities.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");

pub fn Entity(comptime RequestCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    comptime {
        for (.{ RequestCodec, SuccessCodec, ErrorCodec }) |C| codec_contract.require(C);
    }
    return struct {
        const Self = @This();
        pub const Request = RequestCodec.Decoded;
        pub const Success = SuccessCodec.Decoded;
        pub const DomainError = ErrorCodec.Decoded;

        group: []const u8,
        version: u32,
        request: RequestCodec,
        success: SuccessCodec,
        failure: ErrorCodec,

        pub fn define(comptime group: []const u8, comptime version: u32, comptime request: RequestCodec, comptime success: SuccessCodec, comptime failure: ErrorCodec) Self {
            const definition: Self = .{ .group = group, .version = version, .request = request, .success = success, .failure = failure };
            definition.validate();
            return definition;
        }

        pub fn validate(comptime self: Self) void {
            if (comptime self.group.len == 0 or !std.unicode.utf8ValidateSlice(self.group))
                @compileError("entity group must be stable nonempty UTF-8");
            if (comptime self.version == 0) @compileError("entity definition version must be nonzero");
        }

        pub fn handledBy(comptime self: Self, comptime R: type, comptime body: fn (*const Request, *const R) effects.Effect(Success, DomainError, R)) Handler(Self, R) {
            self.validate();
            return .{ .definition = self, .body = body };
        }
    };
}

pub fn Handler(comptime Definition: type, comptime R: type) type {
    return struct {
        const Self = @This();
        pub const EntityDefinition = Definition;
        pub const Environment = R;
        definition: Definition,
        body: *const fn (*const Definition.Request, *const R) effects.Effect(Definition.Success, Definition.DomainError, R),

        pub fn invoke(comptime self: Self, comptime select_request: fn (*const R) *const Definition.Request) effects.Effect(Definition.Success, Definition.DomainError, R) {
            self.definition.validate();
            const Program = effects.Effect(Definition.Success, Definition.DomainError, R);
            return Program.deferEffect(struct {
                fn run(env: *const R) Program {
                    return self.body(select_request(env), env);
                }
            }.run);
        }
    };
}

pub fn Registry(comptime handlers: anytype) type {
    const Info = @typeInfo(@TypeOf(handlers));
    if (Info != .@"struct" or !Info.@"struct".is_tuple or handlers.len == 0)
        @compileError("entity registry requires a nonempty tuple of handlers");
    inline for (handlers, 0..) |handler, index| {
        const H = @TypeOf(handler);
        if (@typeInfo(H) != .@"struct" or !@hasDecl(H, "EntityDefinition") or
            !@hasDecl(H, "Environment") or !@hasField(H, "definition") or
            H != Handler(H.EntityDefinition, H.Environment))
            @compileError("entity registry members must be completed handlers");
        handler.definition.validate();
        if (H.Environment != @TypeOf(handlers[0]).Environment)
            @compileError("entity registry needs one environment");
        inline for (handlers, 0..) |other, earlier| {
            if (earlier < index and std.mem.eql(u8, handler.definition.group, other.definition.group) and handler.definition.version == other.definition.version)
                @compileError("duplicate entity group and version");
        }
    }
    return struct {
        pub const Environment = @TypeOf(handlers[0]).Environment;

        pub fn resolve(group: []const u8, version: u32) outcomes.Result(usize, ResolveError) {
            var supported: ?u32 = null;
            inline for (handlers, 0..) |handler, index| {
                if (std.mem.eql(u8, group, handler.definition.group)) {
                    if (version == handler.definition.version) return .{ .ok = index };
                    if (supported == null) supported = handler.definition.version;
                }
            }
            if (supported) |known| return .{ .err = .{ .incompatible_version = .{ .requested = version, .supported = known } } };
            return .{ .err = .unknown_group };
        }
    };
}

pub const ResolveError = union(enum) {
    unknown_group,
    incompatible_version: struct { requested: u32, supported: u32 },
};
