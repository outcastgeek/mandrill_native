const std = @import("std");
const fx = @import("effect");

const Env = struct { allocator: std.mem.Allocator, io: std.Io };
const Record = struct { value: i64 };
const Incomplete = fx.SchemaRecordBuilder(Record, Env).begin()
    .fields(.{fx.SchemaBuilder.required("value", fx.SchemaRepresentation.signed_integer)});

pub fn main() void {
    _ = Incomplete.build();
}
