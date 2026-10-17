//! Recursive metadata is lazy; the existing bounded structural engine owns
//! traversal and generated captures own ordinary composite node members.
const recursive = @import("../schema/recursive_schema.zig");
const roles = @import("roles.zig");
const inference = @import("inference.zig");

pub const Box = recursive.InferredOwnedBox;

/// Supply the recursive value, environment, stable reference and depth once.
/// The schema factory receives Value/Environment/Schema aliases when requested.
pub fn from(comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "recursiveSchema", &.{ "value", "max_depth", "reference", "schema" }, &.{});
    const T = roles.get("value", supplied);
    if (@TypeOf(T) != type) @compileError("recursiveSchema.value must be the recursive application type");
    const factory = roles.get("schema", supplied);
    const info = inference.info(factory, "recursiveSchema.schema");
    if (info.param_types.len > 1 or (info.param_types.len == 1 and info.param_types[0] != null and info.param_types[0].? != type)) @compileError("recursiveSchema.schema requires zero parameters or a comptime context type");
    if (info.return_type != type) @compileError("recursiveSchema.schema must return a structural schema type");
    const Bridge = struct {
        pub const Value = T;
        pub const Environment = R;
        pub const Schema = recursive.InferredBox(T, R, supplied.max_depth, supplied.reference, make);
        fn make() type {
            if (comptime info.param_types.len == 0) return factory();
            return factory(@This());
        }
    };
    return Bridge.Schema;
}
