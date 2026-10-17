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
const Bytes = fx.OwnedSlice(u8);
const codec = fx.dsl.codec(.{ .kind = .contextual, .decode = struct {
    fn decode(input: *const std.json.Value) fx.Effect(Bytes, fx.SchemaIssue.Issue, Env) {
        return fx.Effect(Bytes, fx.SchemaIssue.Issue, Env).fromBorrowed(std.json.Value, input, struct {
            fn run(value: *const std.json.Value, env: *const Env) fx.Exit(Bytes, fx.SchemaIssue.Issue) {
                const bytes = env.allocator.dupe(u8, value.string) catch return .{ .die = fx.Defect.out_of_memory };
                return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
            }
        }.run);
    }
}.decode, .encode = struct {
    fn encode(input: *const Bytes) fx.Effect(std.json.Value, fx.SchemaIssue.Issue, Env) {
        return fx.Effect(std.json.Value, fx.SchemaIssue.Issue, Env).fromBorrowed(Bytes, input, struct {
            fn run(value: *const Bytes, _: *const Env) fx.Exit(std.json.Value, fx.SchemaIssue.Issue) {
                return .{ .success = .{ .string = value.items } };
            }
        }.run);
    }
}.encode });
fn default() std.json.Value {
    return .{ .string = "copied" };
}
pub fn main() void {
    const Record = fx.dsl.nativeRecord(Object, .{ .fields = .{
        .answer = .{ .codec = codec, .presence = .defaulted, .default = default },
    }, .reject_unknown = true });
    const env: Env = .{ .allocator = std.heap.page_allocator };
    const input: std.json.Value = .{ .object = .{} };
    var result = fx.run(&env, Record.decode(&input));
    result.deinit();
}
