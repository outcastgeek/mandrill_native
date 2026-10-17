const std = @import("std");
const fx = @import("effect");
const Cfg = struct { offset: i64 };
const Creation = struct { allocator: std.mem.Allocator };
const Env = struct { marker: bool };
const Functions = struct {
    fn decode(_: *const Cfg, input: *const std.json.Value) fx.Result(i64, fx.SchemaIssue.Issue) {
        return fx.JsonCodec.signed_integer.decodeDirect(input);
    }
    fn encode(_: *const Cfg, input: *const i64) fx.Result(std.json.Value, fx.SchemaIssue.Issue) {
        return fx.JsonCodec.signed_integer.encodeDirect(input);
    }
    fn write(cfg: *const Cfg, input: *const i64, output: *fx.JsonWriter.Writer) fx.Exit(void, fx.SchemaIssue.Issue) {
        const adjusted = input.* + cfg.offset;
        return fx.JsonCodec.signed_integer.writeJson(&adjusted, output);
    }
    fn invalid(_: *const Cfg, _: *const i64, _: *fx.JsonWriter.Writer) fx.Result(void, fx.SchemaIssue.Issue) {
        return .{ .ok = {} };
    }
    fn limits(_: *const Env) fx.JsonWriter.Limits {
        return .{ .max_output_bytes = 64, .max_json_depth = 4, .max_value_bytes = 32, .max_integer_digits = 16 };
    }
};
pub fn main() void {
    const creation: Creation = .{ .allocator = std.heap.page_allocator };
    const env: Env = .{ .marker = true };
    var description = fx.dsl.makeCodec(&creation, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.pure, .decode = Functions.decode, .encode = Functions.encode, .shape = fx.JsonCodec.signed_integer.encoded_shape, .writer = Functions.write }), .inputs = Cfg{ .offset = 3 } });
    defer description.deinit();
    var made = fx.run(&creation, description);
    defer made.deinit();
    var input = made.success.value(39);
    defer input.deinit();
    var program = made.success.writeWith(&input, Functions.limits);
    defer program.deinit();
    var output = fx.run(&env, program);
    defer output.deinit();
    std.debug.assert(output == .success and std.mem.eql(u8, output.success.items, "42"));
}
