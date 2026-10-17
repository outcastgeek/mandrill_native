//! Canonical generated record/variant owners keep logical fields in value.
//! Plain application owners retain their existing direct field layout.
const ownership = @import("../dsl/ownership.zig");

pub fn Fields(comptime T: type) type {
    if (ownership.capturedType(T)) |Inner| if (@typeInfo(Inner) == .@"struct" or @typeInfo(Inner) == .@"union") return Inner;
    return T;
}
pub fn init(comptime T: type, value: Fields(T)) T {
    if (comptime Fields(T) == T) return value;
    return .{ .value = value };
}
fn Pointer(comptime P: type) type {
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one) @compileError("record fields require a borrowed record pointer");
    const info = @typeInfo(P).pointer;
    return if (info.attrs.@"const") *const Fields(info.child) else *Fields(info.child);
}
pub fn get(value: anytype) Pointer(@TypeOf(value)) {
    const T = @typeInfo(@TypeOf(value)).pointer.child;
    if (comptime Fields(T) == T) return value;
    return &value.value;
}
