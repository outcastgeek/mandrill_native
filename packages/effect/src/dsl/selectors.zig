const effects = @import("../effect.zig");

pub fn borrow(comptime E: type, comptime R: type, comptime name: []const u8) effects.Effect(*const @FieldType(R, name), E, R) {
    return effects.Effect(*const @FieldType(R, name), E, R).sync(struct {
        fn read(env: *const R) *const @FieldType(R, name) {
            return &@field(env.*, name);
        }
    }.read);
}

pub fn value(comptime E: type, comptime R: type, comptime name: []const u8) effects.Effect(@FieldType(R, name), E, R) {
    if (@import("ownership.zig").containsOwned(@FieldType(R, name))) @compileError("value selector cannot copy an owning field; borrow or explicitly clone/transfer it");
    return effects.Effect(@FieldType(R, name), E, R).sync(struct {
        fn read(env: *const R) @FieldType(R, name) {
            return @field(env.*, name);
        }
    }.read);
}
