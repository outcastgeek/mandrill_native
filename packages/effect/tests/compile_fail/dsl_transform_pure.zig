const std = @import("std");
const fx = @import("effect");
const Issue = fx.SchemaIssue.Issue;
const Env = struct { allocator: std.mem.Allocator };
const source = fx.Codec(i64, std.json.Value, Env, Env).init(struct {
    fn decode(input: *const std.json.Value) fx.Effect(i64, Issue, Env) {
        return fx.Effect(i64, Issue, Env).fromBorrowed(std.json.Value, input, struct {
            fn run(value: *const std.json.Value, _: *const Env) fx.Exit(i64, Issue) {
                return .{ .success = value.integer };
            }
        }.run);
    }
}.decode, struct {
    fn encode(value: *const i64) fx.Effect(std.json.Value, Issue, Env) {
        return fx.Effect(std.json.Value, Issue, Env).fromBorrowed(i64, value, struct {
            fn run(input: *const i64, _: *const Env) fx.Exit(std.json.Value, Issue) {
                return .{ .success = .{ .integer = input.* } };
            }
        }.run);
    }
}.encode);
fn forward(value: i64) fx.Result(u16, Issue) {
    return .{ .ok = @intCast(value) };
}
fn backward(value: *const u16) fx.Result(i64, Issue) {
    return .{ .ok = value.* };
}
pub fn main() void {
    _ = fx.dsl.transformCodec(source, .{ .forward = forward, .backward = backward });
}
