//! A workflow definition is a versioned description plus an Effect body.
//! The body is registered in process; only its name, versions and encoded
//! payload/outcome cross the engine boundary.
const std = @import("std");
const effects = @import("../effect.zig");
const types = @import("workflow_types.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");

pub fn Workflow(comptime PayloadCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    comptime {
        for (.{ PayloadCodec, SuccessCodec, ErrorCodec }) |C| codec_contract.require(C);
    }
    return struct {
        const Self = @This();
        pub const Payload = PayloadCodec.Decoded;
        pub const Success = SuccessCodec.Decoded;
        pub const DomainError = ErrorCodec.Decoded;

        description: types.Definition,
        payload: PayloadCodec,
        success: SuccessCodec,
        failure: ErrorCodec,

        pub fn define(
            comptime name: []const u8,
            comptime version: u32,
            comptime payload_schema_version: u32,
            comptime success_schema_version: u32,
            comptime error_schema_version: u32,
            comptime payload: PayloadCodec,
            comptime success: SuccessCodec,
            comptime failure: ErrorCodec,
        ) Self {
            const definition: Self = .{
                .description = .{
                    .name = name,
                    .version = version,
                    .payload_schema_version = payload_schema_version,
                    .success_schema_version = success_schema_version,
                    .error_schema_version = error_schema_version,
                },
                .payload = payload,
                .success = success,
                .failure = failure,
            };
            definition.validate();
            return definition;
        }

        pub fn validate(comptime self: Self) void {
            if (comptime self.description.name.len == 0 or !std.unicode.utf8ValidateSlice(self.description.name))
                @compileError("workflow name must be stable nonempty UTF-8");
            if (comptime self.description.version == 0 or self.description.payload_schema_version == 0 or
                self.description.success_schema_version == 0 or self.description.error_schema_version == 0)
                @compileError("workflow and schema versions must be nonzero");
        }

        pub fn handledBy(
            comptime self: Self,
            comptime R: type,
            comptime body: fn (*const Payload, *const R) effects.Effect(Success, DomainError, R),
        ) Handler(Self, R) {
            self.validate();
            return .{ .definition = self, .body = body };
        }

        pub fn handle(comptime self: Self, execution: types.ExecutionId) Handle(Self) {
            self.validate();
            return .{ .description = self.description, .execution = execution };
        }
    };
}

/// The type carries payload, success and domain-error identity through client
/// code; the process-local handler remains separate from the durable handle.
pub fn Handle(comptime Definition: type) type {
    return struct {
        pub const Payload = Definition.Payload;
        pub const Success = Definition.Success;
        pub const DomainError = Definition.DomainError;
        description: types.Definition,
        execution: types.ExecutionId,

        pub fn submission(self: @This(), encoded_payload: []const u8) types.Submission {
            return .{ .definition = self.description, .execution = self.execution, .encoded_payload = encoded_payload };
        }
    };
}

pub fn Handler(comptime Definition: type, comptime R: type) type {
    return struct {
        const Self = @This();
        pub const WorkflowDefinition = Definition;
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
        @compileError("workflow registry needs a nonempty tuple of completed handlers");
    inline for (handlers, 0..) |handler, index| {
        const H = @TypeOf(handler);
        if (@typeInfo(H) != .@"struct" or !@hasDecl(H, "WorkflowDefinition") or
            !@hasDecl(H, "Environment") or !@hasField(H, "definition") or
            H != Handler(H.WorkflowDefinition, H.Environment))
            @compileError("workflow registry members must be completed handlers");
        handler.definition.validate();
        if (H.Environment != @TypeOf(handlers[0]).Environment)
            @compileError("workflow registry needs one environment");
        inline for (handlers, 0..) |other, earlier| {
            if (earlier < index and std.mem.eql(u8, handler.definition.description.name, other.definition.description.name) and
                handler.definition.description.version == other.definition.description.version)
                @compileError("duplicate workflow identity");
        }
    }
    return struct {
        pub const Environment = @TypeOf(handlers[0]).Environment;

        pub fn resolve(name: []const u8, version: u32) ?usize {
            inline for (handlers, 0..) |handler, index| {
                if (std.mem.eql(u8, name, handler.definition.description.name) and
                    version == handler.definition.description.version)
                    return index;
            }
            return null;
        }

        pub fn definition(comptime index: usize) types.Definition {
            if (index >= handlers.len) @compileError("workflow registry index is out of bounds");
            return handlers[index].definition.description;
        }
    };
}
