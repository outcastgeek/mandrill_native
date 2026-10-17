const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
const Object = struct {
    pub const Representation = std.json.Value;
    pub const Builder = struct { allocator: std.mem.Allocator, values: std.json.ObjectMap = .{} };
    pub fn count(input: *const Representation) usize {
        return if (input.* == .object) input.object.count() else 0;
    }
    pub fn nameAt(input: *const Representation, index: usize) []const u8 {
        return input.object.keys()[index];
    }
    pub fn find(input: *const Representation, name: []const u8) ?*const Representation {
        return if (input.* == .object) input.object.getPtr(name) else null;
    }
    pub fn init(env: *const Env, _: usize) fx.Exit(Builder, fx.SchemaIssue.Issue) {
        return .{ .success = .{ .allocator = env.allocator } };
    }
    pub fn put(_: *const Env, builder: *Builder, name: []const u8, value: *const Representation) fx.Exit(void, fx.SchemaIssue.Issue) {
        builder.values.put(builder.allocator, name, value.*) catch return .{ .die = fx.Defect.out_of_memory };
        return .{ .success = {} };
    }
    pub fn finish(_: *const Env, builder: *Builder) fx.Exit(Representation, fx.SchemaIssue.Issue) {
        const values = builder.values;
        builder.values = .{};
        return .{ .success = .{ .object = values } };
    }
    pub fn discard(builder: *Builder) void {
        builder.values.deinit(builder.allocator);
    }
};
fn default(_: *const Env) bool {
    return true;
}
pub fn main() void {
    const Record = fx.dsl.nativeRecord(Object, .{ .decode_env = Env, .fields = .{
        .answer = .{ .codec = fx.JsonCodec.signed_integer, .presence = .defaulted, .default = default },
    }, .reject_unknown = true });
    _ = Record.decode;
}
