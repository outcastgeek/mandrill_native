const std = @import("std");
const fx = @import("effect");
const Issue = fx.SchemaIssue.Issue;
fn decode(input: *const std.json.Value) fx.Result(i64, Issue) {
    return fx.JsonCodec.signed_integer.decodeDirect(input);
}
fn encode(input: *const i64) fx.Result(std.json.Value, Issue) {
    return fx.JsonCodec.signed_integer.encodeDirect(input);
}
fn write(input: *const i64, output: *fx.JsonWriter.Writer) fx.Exit(void, Issue) {
    return fx.JsonCodec.signed_integer.writeJson(input, output);
}
pub fn main() void {
    const Env = struct { allocator: std.mem.Allocator };
    const env: Env = .{ .allocator = std.heap.page_allocator };
    const codec = comptime fx.dsl.codec(.{ .kind = .pure, .decode = decode, .encode = encode });
    var output = fx.JsonWriter.Writer.init(&env, .{ .max_output_bytes = 64, .max_json_depth = 4, .max_value_bytes = 32, .max_integer_digits = 16 });
    defer output.deinit();
    var result = codec.writeJson(&@as(i64, 7), &output);
    result.deinit();
}
