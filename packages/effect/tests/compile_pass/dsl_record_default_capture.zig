const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
const Wrapped = fx.dsl.ownership.Capture(i64);
const codec = fx.PureCodec(Wrapped, std.json.Value).init(struct {
    fn decode(input: *const std.json.Value) fx.Result(Wrapped, fx.SchemaIssue.Issue) {
        return .{ .ok = .{ .value = input.integer } };
    }
}.decode, struct {
    fn encode(value: *const Wrapped) fx.Result(std.json.Value, fx.SchemaIssue.Issue) {
        return .{ .ok = .{ .integer = value.value } };
    }
}.encode);
fn default(_: *const Env) Wrapped {
    return .{ .value = 7 };
}
pub fn main() void {
    const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{ .answer = .{ .codec = codec, .presence = .defaulted, .default = default } }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
    _ = Record.decode;
}
