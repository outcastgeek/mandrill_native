//! Shared directional environment evidence for static and captured declarations.
const codec_contract = @import("../schema/internal/codec_contract.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");

fn codecEnvironment(comptime selected: anytype, comptime decoding: bool, comptime operation: []const u8) ?type {
    const C = if (@TypeOf(selected) == type) selected else @TypeOf(selected);
    codec_contract.require(C);
    const direction = if (decoding) "DecodingEnvironment" else "EncodingEnvironment";
    const R = if (@hasDecl(C, direction)) @field(C, direction) else if (@hasDecl(C, "Environment")) C.Environment else return null;
    if (@TypeOf(R) != type) @compileError(operation ++ " codec environment evidence must be a type");
    return if (R == void) null else R;
}
pub fn environment(comptime callback: anytype, comptime supplied: anytype, comptime candidates: anytype, comptime operation: []const u8, comptime captured: bool) type {
    const f = inference.info(callback, operation);
    if (f.param_types.len != (if (captured) @as(usize, 3) else 2) and f.param_types.len != (if (captured) @as(usize, 4) else 3)) @compileError(operation ++ " requires " ++ (if (captured) "configuration/request/environment with optional context" else "request/environment or context/request/environment arguments"));
    if (roles.has(roles.recordType(supplied), "environment")) {
        const R = roles.get("environment", supplied);
        if (@TypeOf(R) != type) @compileError(operation ++ " environment anchor must be a type");
        return R;
    }
    if (f.param_types[f.param_types.len - 1] != null) return inference.environment(callback, operation);
    if (f.return_type) |T| {
        if (@typeInfo(T) == .@"struct" and @hasDecl(T, "Environment")) return T.Environment;
    }
    var selected: ?type = null;
    inline for (candidates) |candidate| {
        if (codecEnvironment(candidate.codec, candidate.decoding, operation)) |R| {
            if (selected) |Previous| {
                if (Previous != R) @compileError(operation ++ " codec environment evidence is ambiguous; supply the handler environment anchor");
            } else selected = R;
        }
    }
    return selected orelse @compileError(operation ++ " requires environment evidence from its parameter result codecs or an explicit environment anchor");
}
