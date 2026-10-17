//! Static declaration evidence and runtime inputs share one caller record.
//! Domain builders retain responsibility for their own completeness rules.
const roles = @import("roles.zig");

/// Normalize a selected codec type/value once for named domain declarations.
pub fn codecType(comptime selected: anytype) type {
    const C = if (@TypeOf(selected) == type) selected else @TypeOf(selected);
    @import("../schema/internal/codec_contract.zig").require(C);
    return C;
}
pub fn codec(comptime selected: anytype) codecType(selected) {
    return if (@TypeOf(selected) == type) .{} else selected;
}

fn Static(comptime selected: anytype) type {
    return struct {
        pub const value = selected;
    };
}

/// Keep static business metadata in the declaration type. Runtime inputs stay
/// ordinary fields and are captured by the consuming operation exactly once.
pub fn metadata(comptime selected: anytype) type {
    return Static(selected);
}

/// Extract exact generated metadata without evaluating or copying runtime
/// inputs. Each operation validates the returned definition through its builder.
pub fn Info(comptime Input: type, comptime operation: []const u8) type {
    roles.validate(Input, operation, &.{ "definition", "inputs" }, &.{});
    const Definition = if (@hasField(Input, "definition")) @FieldType(Input, "definition") else @TypeOf(Input.definition);
    if (Definition != type) @compileError(operation ++ " definition requires declareMetadata with static definition data");
    const Box = if (@hasField(Input, "definition")) @field(@as(Input, undefined), "definition") else Input.definition;
    if (@typeInfo(Box) != .@"struct" or !@hasDecl(Box, "value")) @compileError(operation ++ " definition requires declareMetadata with static definition data");
    if (Box != Static(Box.value)) @compileError(operation ++ " rejects forged declaration metadata");
    return struct {
        pub const definition = Box.value;
        pub const Inputs = if (@hasField(Input, "inputs")) @FieldType(Input, "inputs") else @TypeOf(Input.inputs);
    };
}
