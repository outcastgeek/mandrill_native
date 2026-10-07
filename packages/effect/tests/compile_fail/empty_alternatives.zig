const fx = @import("effect");
pub fn main() void {
    _ = fx.JsonAlternatives.oneOf(void, .{}, .{ .maximum_issues = 1, .limits = .{ .max_output_bytes = 1, .max_json_depth = 1, .max_value_bytes = 1, .max_integer_digits = 1 } });
}
