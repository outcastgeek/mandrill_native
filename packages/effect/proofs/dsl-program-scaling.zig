const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const Env = struct { input: u32 };
fn read(env: *const Env) u32 {
    return env.input;
}
fn next(value: *const u32, _: *const Env) u32 {
    return value.* +% 1;
}
const sixteen = p.describe(.{ p.native("s0", .{}, read), p.native("s1", .{"s0"}, next), p.native("s2", .{"s1"}, next), p.native("s3", .{"s2"}, next), p.native("s4", .{"s3"}, next), p.native("s5", .{"s4"}, next), p.native("s6", .{"s5"}, next), p.native("s7", .{"s6"}, next), p.native("s8", .{"s7"}, next), p.native("s9", .{"s8"}, next), p.native("s10", .{"s9"}, next), p.native("s11", .{"s10"}, next), p.native("s12", .{"s11"}, next), p.native("s13", .{"s12"}, next), p.native("s14", .{"s13"}, next), p.native("s15", .{"s14"}, next) });
const sixty_four = p.describe(.{ p.native("s0", .{}, read), p.native("s1", .{"s0"}, next), p.native("s2", .{"s1"}, next), p.native("s3", .{"s2"}, next), p.native("s4", .{"s3"}, next), p.native("s5", .{"s4"}, next), p.native("s6", .{"s5"}, next), p.native("s7", .{"s6"}, next), p.native("s8", .{"s7"}, next), p.native("s9", .{"s8"}, next), p.native("s10", .{"s9"}, next), p.native("s11", .{"s10"}, next), p.native("s12", .{"s11"}, next), p.native("s13", .{"s12"}, next), p.native("s14", .{"s13"}, next), p.native("s15", .{"s14"}, next), p.native("s16", .{"s15"}, next), p.native("s17", .{"s16"}, next), p.native("s18", .{"s17"}, next), p.native("s19", .{"s18"}, next), p.native("s20", .{"s19"}, next), p.native("s21", .{"s20"}, next), p.native("s22", .{"s21"}, next), p.native("s23", .{"s22"}, next), p.native("s24", .{"s23"}, next), p.native("s25", .{"s24"}, next), p.native("s26", .{"s25"}, next), p.native("s27", .{"s26"}, next), p.native("s28", .{"s27"}, next), p.native("s29", .{"s28"}, next), p.native("s30", .{"s29"}, next), p.native("s31", .{"s30"}, next), p.native("s32", .{"s31"}, next), p.native("s33", .{"s32"}, next), p.native("s34", .{"s33"}, next), p.native("s35", .{"s34"}, next), p.native("s36", .{"s35"}, next), p.native("s37", .{"s36"}, next), p.native("s38", .{"s37"}, next), p.native("s39", .{"s38"}, next), p.native("s40", .{"s39"}, next), p.native("s41", .{"s40"}, next), p.native("s42", .{"s41"}, next), p.native("s43", .{"s42"}, next), p.native("s44", .{"s43"}, next), p.native("s45", .{"s44"}, next), p.native("s46", .{"s45"}, next), p.native("s47", .{"s46"}, next), p.native("s48", .{"s47"}, next), p.native("s49", .{"s48"}, next), p.native("s50", .{"s49"}, next), p.native("s51", .{"s50"}, next), p.native("s52", .{"s51"}, next), p.native("s53", .{"s52"}, next), p.native("s54", .{"s53"}, next), p.native("s55", .{"s54"}, next), p.native("s56", .{"s55"}, next), p.native("s57", .{"s56"}, next), p.native("s58", .{"s57"}, next), p.native("s59", .{"s58"}, next), p.native("s60", .{"s59"}, next), p.native("s61", .{"s60"}, next), p.native("s62", .{"s61"}, next), p.native("s63", .{"s62"}, next) });
test "PERF-04 INF-11 sixteen and sixty four native bindings compile under the default quota and run without an environment allocator" {
    inline for (.{ sixteen, sixty_four }, .{ 16, 64 }) |described, count| {
        comptime if (@TypeOf(described) != fx.Effect(u32, void, Env)) @compileError("scaled native descriptions must retain exact Effect");
        for ([_]u32{ 0, 1, 17, std.math.maxInt(u32) }) |input| {
            const env: Env = .{ .input = input };
            var result = fx.run(&env, described);
            defer result.deinit();
            try std.testing.expectEqual(input +% @as(u32, count - 1), result.success);
        }
    }
}
